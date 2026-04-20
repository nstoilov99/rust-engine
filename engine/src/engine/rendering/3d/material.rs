use std::sync::Arc;
use uuid::Uuid;
use vulkano::buffer::Subbuffer;
use vulkano::command_buffer::allocator::StandardCommandBufferAllocator;
use vulkano::descriptor_set::allocator::StandardDescriptorSetAllocator;
use vulkano::descriptor_set::{DescriptorSet, WriteDescriptorSet};
use vulkano::device::{Device, Queue};
use vulkano::image::sampler::Sampler;
use vulkano::image::view::ImageView;
use vulkano::memory::allocator::StandardMemoryAllocator;
use vulkano::pipeline::PipelineLayout;

/// Default 1×1 fallback colors (RGBA8).
pub const DEFAULT_ALBEDO_RGBA: [u8; 4] = [255, 255, 255, 255];
/// Tangent-space "flat" normal (pointing straight out).
pub const DEFAULT_NORMAL_RGBA: [u8; 4] = [128, 128, 255, 255];
/// glTF convention: G = roughness = 1, B = metallic = 0.
pub const DEFAULT_METALLIC_ROUGHNESS_RGBA: [u8; 4] = [0, 255, 0, 255];
/// Full ambient occlusion (no darkening).
pub const DEFAULT_AO_RGBA: [u8; 4] = [255, 255, 255, 255];
/// Black emissive (reserved for completeness; not used as a binding in v1).
pub const DEFAULT_EMISSIVE_RGBA: [u8; 4] = [0, 0, 0, 255];

/// GPU-side material parameters, uploaded as a uniform buffer (Set 1, binding 4).
///
/// Layout matches `MaterialParams` in `gbuffer.frag`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct MaterialParamsGpu {
    pub base_color_factor: [f32; 4],   // 16
    pub metallic_factor: f32,          //  4
    pub roughness_factor: f32,         //  4
    pub _pad0: [f32; 2],              //  8 (align emissive_factor to 16)
    pub emissive_factor: [f32; 3],     // 12
    pub _pad1: f32,                   //  4  (total 48 B)
}

const _: () = assert!(std::mem::size_of::<MaterialParamsGpu>() == 48);

/// PBR material with all texture maps, per-material UBO, and pre-built descriptor set.
pub struct PbrMaterial {
    pub albedo: Arc<ImageView>,
    pub normal: Arc<ImageView>,
    pub metallic_roughness: Arc<ImageView>,
    pub ao: Arc<ImageView>,
    pub params_buffer: Subbuffer<MaterialParamsGpu>,
    pub descriptor_set: Arc<DescriptorSet>,
}

impl PbrMaterial {
    /// Create a new PBR material with textures, factors, and a pre-built Set 1 descriptor set.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        albedo: Arc<ImageView>,
        normal: Arc<ImageView>,
        metallic_roughness: Arc<ImageView>,
        ao: Arc<ImageView>,
        sampler: Arc<Sampler>,
        base_color_factor: [f32; 4],
        metallic_factor: f32,
        roughness_factor: f32,
        emissive_factor: [f32; 3],
        allocator: Arc<StandardMemoryAllocator>,
        descriptor_set_allocator: Arc<StandardDescriptorSetAllocator>,
        geom_pipeline_layout: Arc<PipelineLayout>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        use vulkano::buffer::{Buffer, BufferCreateInfo, BufferUsage};
        use vulkano::memory::allocator::{AllocationCreateInfo, MemoryTypeFilter};

        let params = MaterialParamsGpu {
            base_color_factor,
            metallic_factor,
            roughness_factor,
            _pad0: [0.0; 2],
            emissive_factor,
            _pad1: 0.0,
        };

        let params_buffer = Buffer::from_data(
            allocator,
            BufferCreateInfo {
                usage: BufferUsage::UNIFORM_BUFFER,
                ..Default::default()
            },
            AllocationCreateInfo {
                memory_type_filter: MemoryTypeFilter::PREFER_DEVICE
                    | MemoryTypeFilter::HOST_SEQUENTIAL_WRITE,
                ..Default::default()
            },
            params,
        )?;

        // Set 1 layout: 4 textures + 1 UBO
        let set_layout = geom_pipeline_layout
            .set_layouts()
            .get(1)
            .ok_or("Missing Set 1 layout on geometry pipeline")?;

        let descriptor_set = DescriptorSet::new(
            descriptor_set_allocator,
            set_layout.clone(),
            [
                WriteDescriptorSet::image_view_sampler(0, albedo.clone(), sampler.clone()),
                WriteDescriptorSet::image_view_sampler(1, normal.clone(), sampler.clone()),
                WriteDescriptorSet::image_view_sampler(
                    2,
                    metallic_roughness.clone(),
                    sampler.clone(),
                ),
                WriteDescriptorSet::image_view_sampler(3, ao.clone(), sampler),
                WriteDescriptorSet::buffer(4, params_buffer.clone()),
            ],
            [],
        )?;

        Ok(Self {
            albedo,
            normal,
            metallic_roughness,
            ao,
            params_buffer,
            descriptor_set,
        })
    }
}

// ---------------------------------------------------------------------------
// MaterialBase / MaterialInstance split
// ---------------------------------------------------------------------------

/// Unique identifier for a material base (shared texture set).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct MaterialBaseId(pub Uuid);

impl Default for MaterialBaseId {
    fn default() -> Self {
        Self::new()
    }
}

impl MaterialBaseId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

/// Shared, immutable texture set. Multiple `MaterialInstance`s can reference
/// the same base to share GPU texture memory.
pub struct MaterialBase {
    pub id: MaterialBaseId,
    pub albedo: Arc<ImageView>,
    pub normal: Arc<ImageView>,
    pub metallic_roughness: Arc<ImageView>,
    pub ao: Arc<ImageView>,
    pub sampler: Arc<Sampler>,
}

/// Per-instance material overrides.  Owns a UBO with factor values and a
/// descriptor set that combines the base's textures with this instance's UBO.
pub struct MaterialInstance {
    pub base_id: MaterialBaseId,
    pub params: MaterialParamsGpu,
    pub params_buffer: Subbuffer<MaterialParamsGpu>,
    pub descriptor_set: Arc<DescriptorSet>,
}

impl MaterialInstance {
    /// Create a new instance from a `MaterialBase` and per-instance factor overrides.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        base: &MaterialBase,
        base_color_factor: [f32; 4],
        metallic_factor: f32,
        roughness_factor: f32,
        emissive_factor: [f32; 3],
        allocator: Arc<StandardMemoryAllocator>,
        descriptor_set_allocator: Arc<StandardDescriptorSetAllocator>,
        geom_pipeline_layout: Arc<PipelineLayout>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        use vulkano::buffer::{Buffer, BufferCreateInfo, BufferUsage};
        use vulkano::memory::allocator::{AllocationCreateInfo, MemoryTypeFilter};

        let params = MaterialParamsGpu {
            base_color_factor,
            metallic_factor,
            roughness_factor,
            _pad0: [0.0; 2],
            emissive_factor,
            _pad1: 0.0,
        };

        let params_buffer = Buffer::from_data(
            allocator,
            BufferCreateInfo {
                usage: BufferUsage::UNIFORM_BUFFER,
                ..Default::default()
            },
            AllocationCreateInfo {
                memory_type_filter: MemoryTypeFilter::PREFER_DEVICE
                    | MemoryTypeFilter::HOST_SEQUENTIAL_WRITE,
                ..Default::default()
            },
            params,
        )?;

        let set_layout = geom_pipeline_layout
            .set_layouts()
            .get(1)
            .ok_or("Missing Set 1 layout on geometry pipeline")?;

        let descriptor_set = DescriptorSet::new(
            descriptor_set_allocator,
            set_layout.clone(),
            [
                WriteDescriptorSet::image_view_sampler(
                    0,
                    base.albedo.clone(),
                    base.sampler.clone(),
                ),
                WriteDescriptorSet::image_view_sampler(
                    1,
                    base.normal.clone(),
                    base.sampler.clone(),
                ),
                WriteDescriptorSet::image_view_sampler(
                    2,
                    base.metallic_roughness.clone(),
                    base.sampler.clone(),
                ),
                WriteDescriptorSet::image_view_sampler(
                    3,
                    base.ao.clone(),
                    base.sampler.clone(),
                ),
                WriteDescriptorSet::buffer(4, params_buffer.clone()),
            ],
            [],
        )?;

        Ok(Self {
            base_id: base.id,
            params,
            params_buffer,
            descriptor_set,
        })
    }
}

/// Creates a 1×1 solid-color texture with proper pixel data upload.
///
/// Uses a staging buffer + one-shot command buffer, matching the pattern
/// in `TextureManager::load_texture_from_disk` and `create_ssao_fallback`.
pub fn create_default_texture(
    _device: Arc<Device>,
    allocator: Arc<StandardMemoryAllocator>,
    command_buffer_allocator: Arc<StandardCommandBufferAllocator>,
    queue: Arc<Queue>,
    color: [u8; 4],
) -> Result<Arc<ImageView>, Box<dyn std::error::Error>> {
    use vulkano::buffer::{Buffer, BufferCreateInfo, BufferUsage};
    use vulkano::command_buffer::{
        AutoCommandBufferBuilder, CopyBufferToImageInfo,
        PrimaryCommandBufferAbstract,
    };
    use vulkano::format::Format;
    use vulkano::image::{Image, ImageCreateInfo, ImageType, ImageUsage};
    use vulkano::memory::allocator::{AllocationCreateInfo, MemoryTypeFilter};
    use vulkano::sync::GpuFuture;

    let image = Image::new(
        allocator.clone(),
        ImageCreateInfo {
            image_type: ImageType::Dim2d,
            format: Format::R8G8B8A8_SRGB,
            extent: [1, 1, 1],
            usage: ImageUsage::TRANSFER_DST | ImageUsage::SAMPLED,
            ..Default::default()
        },
        AllocationCreateInfo {
            memory_type_filter: MemoryTypeFilter::PREFER_DEVICE,
            ..Default::default()
        },
    )?;

    let staging = Buffer::from_iter(
        allocator,
        BufferCreateInfo {
            usage: BufferUsage::TRANSFER_SRC,
            ..Default::default()
        },
        AllocationCreateInfo {
            memory_type_filter: MemoryTypeFilter::HOST_SEQUENTIAL_WRITE,
            ..Default::default()
        },
        color,
    )?;

    let mut builder = AutoCommandBufferBuilder::primary(
        command_buffer_allocator,
        queue.queue_family_index(),
        vulkano::command_buffer::CommandBufferUsage::OneTimeSubmit,
    )?;
    builder.copy_buffer_to_image(CopyBufferToImageInfo::buffer_image(staging, image.clone()))?;
    builder
        .build()?
        .execute(queue)?
        .then_signal_fence_and_flush()?
        .wait(None)?;

    let view = ImageView::new_default(image)?;
    Ok(view)
}
