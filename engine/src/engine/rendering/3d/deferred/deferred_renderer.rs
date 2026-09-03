use super::bloom_pass::{BloomPass, BloomSamplePush, BloomThresholdPush};
use super::composite_pass::{CompositePass, CompositePushConstants};
use super::gbuffer::GBuffer;
use super::geometry_pass::GeometryPass;
use super::grid_pass::{GridPass, GridPushConstants};
use super::lighting_pass::LightingPass;
use super::luminance_pass::{LuminancePass, LuminancePush};
use super::pass::{DeferredPass, PassInputs, PassResizeContext};
use super::plankton::PlanktonSystem;
use super::shadow_pass::ShadowPass;
use super::ssao_pass::{SsaoPass, SsaoPushConstants};
use crate::engine::debug_draw::{DebugDrawData, DebugDrawPass, DebugLinePushConstants};
use crate::engine::rendering::counters::RenderCounters;
use crate::engine::rendering::error::RenderError;
use crate::engine::rendering::graph::RenderGraph;
use crate::engine::rendering::pipeline_registry::PipelineRegistry;
use crate::engine::rendering::render_target::RenderTarget;
use crate::engine::rendering::rendering_3d::skinning::{InstanceData, SkinnedPaletteFrame};
use crate::engine::rendering::rendering_3d::material::{
    create_default_texture, create_default_texture_with_format, PbrMaterial, DEFAULT_ALBEDO_RGBA,
    DEFAULT_AO_RGBA, DEFAULT_METALLIC_ROUGHNESS_RGBA, DEFAULT_NORMAL_RGBA,
};
use glam::{Mat4, Vec3, Vec4};
use smallvec::smallvec;
use std::collections::HashMap;
use std::sync::Arc;
use vulkano::buffer::Subbuffer;
use vulkano::command_buffer::{
    allocator::StandardCommandBufferAllocator, AutoCommandBufferBuilder, CommandBufferUsage,
    PrimaryAutoCommandBuffer, RenderPassBeginInfo, SubpassBeginInfo, SubpassContents,
    SubpassEndInfo,
};
use vulkano::descriptor_set::allocator::StandardDescriptorSetAllocator;
use vulkano::descriptor_set::DescriptorSet;
use vulkano::device::{Device, Queue};
use vulkano::format::Format;
use vulkano::image::sampler::{Filter, Sampler, SamplerAddressMode, SamplerCreateInfo};
use vulkano::image::view::ImageView;
use vulkano::image::{Image, ImageCreateInfo, ImageType, ImageUsage};
use vulkano::memory::allocator::{AllocationCreateInfo, MemoryTypeFilter, StandardMemoryAllocator};
use vulkano::pipeline::{Pipeline, PipelineBindPoint};
use vulkano::render_pass::{Framebuffer, FramebufferCreateInfo, RenderPass};

pub struct DeferredRenderer {
    gbuffer: GBuffer,
    geometry_pass: GeometryPass,
    lighting_pass: LightingPass,
    pipeline_registry: PipelineRegistry,
    shadow_pass: ShadowPass,
    ssao_pass: SsaoPass,
    bloom_pass: BloomPass,
    luminance_pass: LuminancePass,
    composite_pass: CompositePass,
    grid_pass: GridPass,
    debug_draw_pass: DebugDrawPass,
    device: Arc<Device>,
    queue: Arc<Queue>,
    allocator: Arc<StandardMemoryAllocator>,
    command_buffer_allocator: Arc<StandardCommandBufferAllocator>,
    descriptor_set_allocator: Arc<StandardDescriptorSetAllocator>,
    debug_view: DebugView,
    render_counters: RenderCounters,
    ssao_sampler: Arc<Sampler>,
    ssao_fallback: Arc<ImageView>,
    default_material_set: Arc<DescriptorSet>,
    hdr_target: Arc<ImageView>,
    composite_render_pass: Arc<RenderPass>,
    composite_present_render_pass: Arc<RenderPass>,
    framebuffer_cache: HashMap<usize, Arc<Framebuffer>>,
    grid_framebuffer_cache: HashMap<usize, Arc<Framebuffer>>,
    grid_render_pass: Arc<RenderPass>,
    grid_present_render_pass: Arc<RenderPass>,
    plankton_system: PlanktonSystem,
    skin_binds: SkinBindCache,
}

/// Per-pass set-0 state for the skinning SSBO rings (Task 41.5 P1 + P7): one
/// tiny view-projection UBO per ring region (rewritten each frame — safe
/// because the region's previous frame's fence was reclaimed before `render`
/// runs) and a cached descriptor set per region binding
/// `{ palette region, vp ubo, instance-metadata region }`.
/// Sets are rebuilt only when a region's identity changes (ring growth) —
/// no per-frame descriptor allocation.
struct PassSkinBind {
    vp_ubos: [Subbuffer<[[f32; 4]; 4]>; 4],
    /// (buffer ptr, byte offset) of the palette + instance regions each
    /// cached set binds.
    sets: [Option<([(usize, u64); 2], Arc<DescriptorSet>)>; 4],
}

impl PassSkinBind {
    fn new(allocator: &Arc<StandardMemoryAllocator>) -> Result<Self, Box<dyn std::error::Error>> {
        let make = || -> Result<Subbuffer<[[f32; 4]; 4]>, Box<dyn std::error::Error>> {
            Ok(vulkano::buffer::Buffer::from_data(
                allocator.clone(),
                vulkano::buffer::BufferCreateInfo {
                    usage: vulkano::buffer::BufferUsage::UNIFORM_BUFFER,
                    ..Default::default()
                },
                AllocationCreateInfo {
                    memory_type_filter: MemoryTypeFilter::PREFER_DEVICE
                        | MemoryTypeFilter::HOST_SEQUENTIAL_WRITE,
                    ..Default::default()
                },
                Mat4::IDENTITY.to_cols_array_2d(),
            )?)
        };
        Ok(Self {
            vp_ubos: [make()?, make()?, make()?, make()?],
            sets: [None, None, None, None],
        })
    }

    /// Write this slot's VP and return the (cached) set-0 descriptor set for
    /// `region` + `instances`.
    fn bind(
        &mut self,
        descriptor_set_allocator: &Arc<StandardDescriptorSetAllocator>,
        layout: Arc<vulkano::descriptor_set::layout::DescriptorSetLayout>,
        slot: usize,
        region: &Subbuffer<[[f32; 16]]>,
        instances: &Subbuffer<[InstanceData]>,
        view_projection: [[f32; 4]; 4],
    ) -> Result<Arc<DescriptorSet>, Box<dyn std::error::Error>> {
        *self.vp_ubos[slot].write()? = view_projection;
        let key = [
            (Arc::as_ptr(region.buffer()) as usize, region.offset()),
            (Arc::as_ptr(instances.buffer()) as usize, instances.offset()),
        ];
        if let Some((cached, set)) = &self.sets[slot] {
            if *cached == key {
                return Ok(set.clone());
            }
        }
        let set = DescriptorSet::new(
            descriptor_set_allocator.clone(),
            layout,
            [
                vulkano::descriptor_set::WriteDescriptorSet::buffer(0, region.clone()),
                vulkano::descriptor_set::WriteDescriptorSet::buffer(
                    1,
                    self.vp_ubos[slot].clone(),
                ),
                vulkano::descriptor_set::WriteDescriptorSet::buffer(2, instances.clone()),
            ],
            [],
        )?;
        self.sets[slot] = Some((key, set.clone()));
        Ok(set)
    }
}

struct SkinBindCache {
    geometry: PassSkinBind,
    shadow: PassSkinBind,
    /// One identity matrix — bound when a frame carries no palette ring
    /// (tests, tools); static meshes still render via `palette_base = 0`.
    identity_region: Subbuffer<[[f32; 16]]>,
    /// One identity instance — the matching instance-metadata fallback for
    /// palette-less frames (draws read instance 0: identity model, base 0).
    identity_instances: Subbuffer<[InstanceData]>,
    /// Slot rotation for the identity fallback (no packet slot available).
    fallback_slot: usize,
    /// Set once a palette-bearing frame is seen; guards against a host
    /// interleaving `Some`/`None` palettes (see `prepare_skinning_binds`).
    saw_palette_frame: bool,
}

impl SkinBindCache {
    fn new(allocator: &Arc<StandardMemoryAllocator>) -> Result<Self, Box<dyn std::error::Error>> {
        let identity_region = vulkano::buffer::Buffer::from_iter(
            allocator.clone(),
            vulkano::buffer::BufferCreateInfo {
                usage: vulkano::buffer::BufferUsage::STORAGE_BUFFER,
                ..Default::default()
            },
            AllocationCreateInfo {
                memory_type_filter: MemoryTypeFilter::PREFER_DEVICE
                    | MemoryTypeFilter::HOST_SEQUENTIAL_WRITE,
                ..Default::default()
            },
            [Mat4::IDENTITY.to_cols_array()],
        )?;
        let identity_instances = vulkano::buffer::Buffer::from_iter(
            allocator.clone(),
            vulkano::buffer::BufferCreateInfo {
                usage: vulkano::buffer::BufferUsage::STORAGE_BUFFER,
                ..Default::default()
            },
            AllocationCreateInfo {
                memory_type_filter: MemoryTypeFilter::PREFER_DEVICE
                    | MemoryTypeFilter::HOST_SEQUENTIAL_WRITE,
                ..Default::default()
            },
            [InstanceData {
                model: Mat4::IDENTITY.to_cols_array_2d(),
                palette_base: 0,
                _pad: [0; 3],
            }],
        )?;
        Ok(Self {
            geometry: PassSkinBind::new(allocator)?,
            shadow: PassSkinBind::new(allocator)?,
            identity_region,
            identity_instances,
            fallback_slot: 0,
            saw_palette_frame: false,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DebugView {
    None,
    Position,
    Normal,
    Albedo,
    Material,
    Depth,
}

fn create_hdr_target(
    allocator: Arc<StandardMemoryAllocator>,
    width: u32,
    height: u32,
) -> Result<Arc<ImageView>, Box<dyn std::error::Error>> {
    let image = Image::new(
        allocator,
        ImageCreateInfo {
            image_type: ImageType::Dim2d,
            format: Format::R16G16B16A16_SFLOAT,
            extent: [width, height, 1],
            usage: ImageUsage::COLOR_ATTACHMENT | ImageUsage::SAMPLED,
            ..Default::default()
        },
        AllocationCreateInfo::default(),
    )?;
    ImageView::new_default(image).map_err(|e| e.into())
}

fn create_ssao_fallback(
    allocator: Arc<StandardMemoryAllocator>,
    command_buffer_allocator: Arc<StandardCommandBufferAllocator>,
    queue: Arc<Queue>,
) -> Result<Arc<ImageView>, Box<dyn std::error::Error>> {
    use vulkano::buffer::{Buffer, BufferCreateInfo, BufferUsage};
    use vulkano::command_buffer::CopyBufferToImageInfo;

    let image = Image::new(
        allocator.clone(),
        ImageCreateInfo {
            image_type: ImageType::Dim2d,
            format: Format::R8_UNORM,
            extent: [1, 1, 1],
            usage: ImageUsage::SAMPLED | ImageUsage::TRANSFER_DST,
            ..Default::default()
        },
        AllocationCreateInfo {
            memory_type_filter: MemoryTypeFilter::PREFER_DEVICE,
            ..Default::default()
        },
    )?;

    // Upload a single white pixel (0xFF) so ambient multiplication passes through
    // unchanged when SSAO is disabled and this fallback is bound.
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
        [255u8],
    )?;

    let mut builder = AutoCommandBufferBuilder::primary(
        command_buffer_allocator,
        queue.queue_family_index(),
        CommandBufferUsage::OneTimeSubmit,
    )?;
    builder.copy_buffer_to_image(CopyBufferToImageInfo::buffer_image(staging, image.clone()))?;
    use vulkano::command_buffer::PrimaryCommandBufferAbstract;
    use vulkano::sync::GpuFuture;
    builder
        .build()?
        .execute(queue)?
        .then_signal_fence_and_flush()?
        .wait(None)?;

    ImageView::new_default(image).map_err(|e| e.into())
}

impl DeferredRenderer {
    pub fn new(
        device: Arc<Device>,
        queue: Arc<Queue>,
        allocator: Arc<StandardMemoryAllocator>,
        command_buffer_allocator: Arc<StandardCommandBufferAllocator>,
        descriptor_set_allocator: Arc<StandardDescriptorSetAllocator>,
        width: u32,
        height: u32,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let gbuffer = GBuffer::new(device.clone(), allocator.clone(), width, height)?;
        let (geometry_pass, geometry_pipeline) =
            GeometryPass::new(device.clone(), gbuffer.render_pass.clone())?;

        let hdr_render_pass = vulkano::single_pass_renderpass!(
            device.clone(),
            attachments: {
                color: {
                    format: Format::R16G16B16A16_SFLOAT,
                    samples: 1,
                    load_op: Clear,
                    store_op: Store,
                }
            },
            pass: {
                color: [color],
                depth_stencil: {}
            }
        )?;

        let (lighting_pass, lighting_pipeline) =
            LightingPass::new(device.clone(), hdr_render_pass)?;

        let shadow_pass = ShadowPass::new(
            device.clone(),
            allocator.clone(),
            descriptor_set_allocator.clone(),
        )?;

        // final_layout = ShaderReadOnlyOptimal so the editor's UI pass (separate
        // submission, no auto layout tracking across CBs) samples in the right layout.
        let composite_render_pass = vulkano::single_pass_renderpass!(
            device.clone(),
            attachments: {
                color: {
                    format: Format::B8G8R8A8_SRGB,
                    samples: 1,
                    load_op: Clear,
                    store_op: Store,
                    final_layout: vulkano::image::ImageLayout::ShaderReadOnlyOptimal,
                }
            },
            pass: {
                color: [color],
                depth_stencil: {}
            }
        )?;

        // Swapchain variant: swapchain images lack SAMPLED usage, so the
        // ShaderReadOnlyOptimal final layout above is illegal there — presenting
        // needs PresentSrc instead. Pipelines are shared (render-pass
        // compatibility ignores initial/final layouts).
        let composite_present_render_pass = vulkano::single_pass_renderpass!(
            device.clone(),
            attachments: {
                color: {
                    format: Format::B8G8R8A8_SRGB,
                    samples: 1,
                    load_op: Clear,
                    store_op: Store,
                    final_layout: vulkano::image::ImageLayout::PresentSrc,
                }
            },
            pass: {
                color: [color],
                depth_stencil: {}
            }
        )?;

        let composite_pass = CompositePass::new(device.clone(), composite_render_pass.clone())?;

        let bloom_pass = BloomPass::new(device.clone(), allocator.clone(), width, height)?;

        let luminance_pass = LuminancePass::new(device.clone(), allocator.clone())?;

        let hdr_target = create_hdr_target(allocator.clone(), width, height)?;

        // Grid/debug-draw render pass — must match the G-buffer depth's
        // final_layout (DepthStencilReadOnlyOptimal) as initial_layout so the
        // depth values survive the layout transition for correct depth testing.
        let make_grid_render_pass = |color_layout: vulkano::image::ImageLayout| {
            use vulkano::image::{ImageLayout, SampleCount};
            use vulkano::render_pass::{
                AttachmentDescription, AttachmentLoadOp, AttachmentReference, AttachmentStoreOp,
                RenderPassCreateInfo, SubpassDescription,
            };
            RenderPass::new(
                device.clone(),
                RenderPassCreateInfo {
                    attachments: vec![
                        // Color (composite output): in/out layout matches what the
                        // preceding composite variant left the target in
                        // (ShaderReadOnlyOptimal for editor textures, PresentSrc
                        // for swapchain images).
                        AttachmentDescription {
                            format: Format::B8G8R8A8_SRGB,
                            samples: SampleCount::Sample1,
                            load_op: AttachmentLoadOp::Load,
                            store_op: AttachmentStoreOp::Store,
                            initial_layout: color_layout,
                            final_layout: color_layout,
                            ..Default::default()
                        },
                        // Attachment 1: depth (G-buffer depth, read-only for depth testing)
                        // Store so the depth survives between grid and debug-draw
                        // render passes that share this framebuffer.
                        AttachmentDescription {
                            format: Format::D32_SFLOAT,
                            samples: SampleCount::Sample1,
                            load_op: AttachmentLoadOp::Load,
                            store_op: AttachmentStoreOp::Store,
                            initial_layout: ImageLayout::DepthStencilReadOnlyOptimal,
                            final_layout: ImageLayout::DepthStencilReadOnlyOptimal,
                            ..Default::default()
                        },
                    ],
                    subpasses: vec![SubpassDescription {
                        color_attachments: vec![Some(AttachmentReference {
                            attachment: 0,
                            layout: ImageLayout::ColorAttachmentOptimal,
                            ..Default::default()
                        })],
                        depth_stencil_attachment: Some(AttachmentReference {
                            attachment: 1,
                            layout: ImageLayout::DepthStencilAttachmentOptimal,
                            ..Default::default()
                        }),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            )
        };
        let grid_render_pass =
            make_grid_render_pass(vulkano::image::ImageLayout::ShaderReadOnlyOptimal)?;
        let grid_present_render_pass =
            make_grid_render_pass(vulkano::image::ImageLayout::PresentSrc)?;

        let grid_pass = GridPass::new(device.clone(), grid_render_pass.clone())?;
        let debug_draw_pass = DebugDrawPass::new(device.clone(), grid_render_pass.clone())?;

        let ssao_pass = SsaoPass::new(
            device.clone(),
            allocator.clone(),
            descriptor_set_allocator.clone(),
            command_buffer_allocator.clone(),
            queue.clone(),
            width,
            height,
        )?;

        let ssao_sampler = Sampler::new(
            device.clone(),
            SamplerCreateInfo {
                mag_filter: Filter::Nearest,
                min_filter: Filter::Nearest,
                address_mode: [SamplerAddressMode::ClampToEdge; 3],
                ..Default::default()
            },
        )?;

        let ssao_fallback = create_ssao_fallback(
            allocator.clone(),
            command_buffer_allocator.clone(),
            queue.clone(),
        )?;

        let plankton_system = PlanktonSystem::new(
            device.clone(),
            queue.clone(),
            allocator.clone(),
            command_buffer_allocator.clone(),
            descriptor_set_allocator.clone(),
        )?;

        // --- Pipeline Registry ---
        let mut pipeline_registry = PipelineRegistry::new();

        {
            #[cfg(feature = "editor")]
            let gbuffer_rp = gbuffer.render_pass.clone();

            pipeline_registry.register(
                crate::engine::rendering::pipeline_registry::PipelineId::Geometry,
                geometry_pipeline,
                #[cfg(feature = "editor")]
                vec![
                    std::path::PathBuf::from("src/engine/rendering/shaders/deferred/gbuffer.vert"),
                    std::path::PathBuf::from("src/engine/rendering/shaders/deferred/gbuffer.frag"),
                ],
                #[cfg(feature = "editor")]
                Box::new(move |compiler, device| {
                    use crate::engine::rendering::pipeline_registry::PipelineError;
                    use crate::engine::rendering::shader_compiler::ShaderKind;
                    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
                    let vs_spirv = compiler
                        .compile(
                            &base.join("src/engine/rendering/shaders/deferred/gbuffer.vert"),
                            ShaderKind::Vertex,
                        )
                        .map_err(PipelineError::Shader)?;
                    let fs_spirv = compiler
                        .compile(
                            &base.join("src/engine/rendering/shaders/deferred/gbuffer.frag"),
                            ShaderKind::Fragment,
                        )
                        .map_err(PipelineError::Shader)?;
                    GeometryPass::create_pipeline_from_spirv(
                        device.clone(),
                        gbuffer_rp.clone(),
                        &vs_spirv,
                        &fs_spirv,
                    )
                    .map_err(|e| PipelineError::Vulkan(e.to_string()))
                }),
            );
        }

        {
            #[cfg(feature = "editor")]
            let lighting_rp = lighting_pass.render_pass();

            pipeline_registry.register(
                crate::engine::rendering::pipeline_registry::PipelineId::Lighting,
                lighting_pipeline,
                #[cfg(feature = "editor")]
                vec![
                    std::path::PathBuf::from("src/engine/rendering/shaders/deferred/lighting.vert"),
                    std::path::PathBuf::from("src/engine/rendering/shaders/deferred/lighting.frag"),
                ],
                #[cfg(feature = "editor")]
                Box::new(move |compiler, device| {
                    use crate::engine::rendering::pipeline_registry::PipelineError;
                    use crate::engine::rendering::shader_compiler::ShaderKind;
                    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
                    let vs_spirv = compiler
                        .compile(
                            &base.join("src/engine/rendering/shaders/deferred/lighting.vert"),
                            ShaderKind::Vertex,
                        )
                        .map_err(PipelineError::Shader)?;
                    let fs_spirv = compiler
                        .compile(
                            &base.join("src/engine/rendering/shaders/deferred/lighting.frag"),
                            ShaderKind::Fragment,
                        )
                        .map_err(PipelineError::Shader)?;
                    LightingPass::create_pipeline_from_spirv(
                        device.clone(),
                        lighting_rp.clone(),
                        &vs_spirv,
                        &fs_spirv,
                    )
                    .map_err(|e| PipelineError::Vulkan(e.to_string()))
                }),
            );
        }

        // --- Default material descriptor set (fallback for meshes without a material) ---
        let mat_sampler = Sampler::new(
            device.clone(),
            SamplerCreateInfo {
                mag_filter: Filter::Linear,
                min_filter: Filter::Linear,
                address_mode: [SamplerAddressMode::Repeat; 3],
                ..Default::default()
            },
        )?;
        // Albedo is a color texture → SRGB for automatic gamma decoding.
        let default_albedo = create_default_texture(
            device.clone(),
            allocator.clone(),
            command_buffer_allocator.clone(),
            queue.clone(),
            DEFAULT_ALBEDO_RGBA,
        )?;
        // Normal, metallic-roughness, and AO are data textures → UNORM (no gamma).
        let default_normal = create_default_texture_with_format(
            allocator.clone(),
            command_buffer_allocator.clone(),
            queue.clone(),
            DEFAULT_NORMAL_RGBA,
            Format::R8G8B8A8_UNORM,
        )?;
        let default_mr = create_default_texture_with_format(
            allocator.clone(),
            command_buffer_allocator.clone(),
            queue.clone(),
            DEFAULT_METALLIC_ROUGHNESS_RGBA,
            Format::R8G8B8A8_UNORM,
        )?;
        let default_ao = create_default_texture_with_format(
            allocator.clone(),
            command_buffer_allocator.clone(),
            queue.clone(),
            DEFAULT_AO_RGBA,
            Format::R8G8B8A8_UNORM,
        )?;
        let geom_pipeline_layout = pipeline_registry
            .get(crate::engine::rendering::pipeline_registry::PipelineId::Geometry)
            .layout()
            .clone();
        let default_material =
            PbrMaterial::builder(default_albedo, default_normal, default_mr, default_ao, mat_sampler)
                .build(
                    allocator.clone(),
                    descriptor_set_allocator.clone(),
                    geom_pipeline_layout,
                )?;
        let default_material_set = default_material.descriptor_set.clone();

        let skin_binds = SkinBindCache::new(&allocator)?;

        let mut renderer = Self {
            gbuffer,
            geometry_pass,
            lighting_pass,
            pipeline_registry,
            shadow_pass,
            ssao_pass,
            bloom_pass,
            luminance_pass,
            composite_pass,
            grid_pass,
            debug_draw_pass,
            device,
            queue,
            allocator,
            command_buffer_allocator,
            descriptor_set_allocator,
            debug_view: DebugView::None,
            render_counters: RenderCounters::default(),
            ssao_sampler,
            ssao_fallback,
            default_material_set,
            hdr_target,
            composite_render_pass,
            composite_present_render_pass,
            framebuffer_cache: HashMap::new(),
            grid_framebuffer_cache: HashMap::new(),
            grid_render_pass,
            grid_present_render_pass,
            plankton_system,
            skin_binds,
        };

        // Populate every pass's descriptor sets / framebuffers (same loop
        // that runs after each resize).
        renderer.rebind_passes()?;

        Ok(renderer)
    }

    /// All deferred passes, in frame execution order. This is the single
    /// registration point the resize/rebind loops iterate — adding a pass
    /// means: implement `DeferredPass` on it, construct it in `new`, and
    /// list it here.
    ///
    /// Execution order for reference (resize correctness does not depend on
    /// it — see `pass.rs`): shadow → geometry → ssao (+blur) → lighting →
    /// plankton → bloom → luminance → composite → grid → debug_draw.
    fn passes_mut(&mut self) -> [&mut dyn DeferredPass; 10] {
        [
            &mut self.shadow_pass,
            &mut self.geometry_pass,
            &mut self.ssao_pass,
            &mut self.lighting_pass,
            &mut self.plankton_system,
            &mut self.bloom_pass,
            &mut self.luminance_pass,
            &mut self.composite_pass,
            &mut self.grid_pass,
            &mut self.debug_draw_pass,
        ]
    }

    /// Snapshot the shared resources and cross-pass outputs that `rebind`
    /// implementations consume. Must be taken *after* all passes resized.
    fn pass_inputs(&self) -> PassInputs {
        PassInputs {
            descriptor_set_allocator: self.descriptor_set_allocator.clone(),
            gbuffer_position: self.gbuffer.position.clone(),
            gbuffer_normal: self.gbuffer.normal.clone(),
            gbuffer_albedo: self.gbuffer.albedo.clone(),
            gbuffer_material: self.gbuffer.material.clone(),
            gbuffer_emissive: self.gbuffer.emissive.clone(),
            gbuffer_depth: self.gbuffer.depth.clone(),
            hdr_target: self.hdr_target.clone(),
            shadow_map: self.shadow_pass.shadow_map(),
            shadow_sampler: self.shadow_pass.shadow_sampler(),
            ssao_blurred: self.ssao_pass.ssao_blurred(),
            ssao_fallback: self.ssao_fallback.clone(),
            ssao_sampler: self.ssao_sampler.clone(),
            bloom_result: self.bloom_pass.bloom_result(),
            luminance_1x1: self.luminance_pass.persistent_1x1(),
        }
    }

    /// Rebind phase: every pass recreates the descriptor sets / framebuffers
    /// that reference shared resources or other passes' outputs.
    fn rebind_passes(&mut self) -> Result<(), RenderError> {
        let inputs = self.pass_inputs();
        for pass in self.passes_mut() {
            let name = pass.name();
            pass.rebind(&inputs).map_err(|e| RenderError::PassRebind {
                pass: name,
                message: e.to_string(),
            })?;
        }
        Ok(())
    }

    fn get_or_create_framebuffer(
        &mut self,
        target_image: Arc<Image>,
        present: bool,
    ) -> Result<Arc<Framebuffer>, Box<dyn std::error::Error>> {
        let cache_key = Arc::as_ptr(&target_image) as usize;

        if let Some(fb) = self.framebuffer_cache.get(&cache_key) {
            return Ok(fb.clone());
        }

        let render_pass = if present {
            self.composite_present_render_pass.clone()
        } else {
            self.composite_render_pass.clone()
        };
        let target_view = ImageView::new_default(target_image)?;
        let framebuffer = Framebuffer::new(
            render_pass,
            FramebufferCreateInfo {
                attachments: vec![target_view],
                ..Default::default()
            },
        )?;

        self.framebuffer_cache
            .insert(cache_key, framebuffer.clone());
        Ok(framebuffer)
    }

    fn get_or_create_grid_framebuffer(
        &mut self,
        target_image: Arc<Image>,
        present: bool,
    ) -> Result<Arc<Framebuffer>, Box<dyn std::error::Error>> {
        let cache_key = Arc::as_ptr(&target_image) as usize;

        if let Some(fb) = self.grid_framebuffer_cache.get(&cache_key) {
            return Ok(fb.clone());
        }

        let render_pass = if present {
            self.grid_present_render_pass.clone()
        } else {
            self.grid_render_pass.clone()
        };
        let target_view = ImageView::new_default(target_image)?;
        let framebuffer = Framebuffer::new(
            render_pass,
            FramebufferCreateInfo {
                attachments: vec![target_view, self.gbuffer.depth.clone()],
                ..Default::default()
            },
        )?;

        self.grid_framebuffer_cache
            .insert(cache_key, framebuffer.clone());
        Ok(framebuffer)
    }

    pub fn clear_framebuffer_cache(&mut self) {
        self.framebuffer_cache.clear();
        self.grid_framebuffer_cache.clear();
    }

    /// Recreate all size-dependent GPU state. Runs in three steps:
    /// shared resources (G-buffer, HDR target) → per-pass `resize` →
    /// per-pass `rebind` against a fresh input snapshot. Errors carry the
    /// failing pass's name and propagate to the caller (the render thread
    /// forwards them to the main thread as `RenderEvent::RenderError`).
    pub fn resize(&mut self, width: u32, height: u32) -> Result<(), RenderError> {
        if width == 0 || height == 0 {
            return Ok(());
        }

        let current_extent = self.gbuffer.position.image().extent();
        if current_extent[0] == width && current_extent[1] == height {
            return Ok(());
        }

        // Shared resources every pass samples from: recreate first.
        self.gbuffer = GBuffer::new(self.device.clone(), self.allocator.clone(), width, height)?;
        self.hdr_target = create_hdr_target(self.allocator.clone(), width, height)?;

        // Phase 1: each pass recreates its own size-dependent targets.
        let ctx = PassResizeContext {
            allocator: self.allocator.clone(),
            width,
            height,
        };
        for pass in self.passes_mut() {
            let name = pass.name();
            pass.resize(&ctx).map_err(|e| RenderError::PassResize {
                pass: name,
                message: e.to_string(),
            })?;
        }

        // Phase 2: rebind cross-pass references against the new resources.
        self.rebind_passes()?;

        // Composite/grid framebuffers are keyed to external target images
        // and reference the old G-buffer depth — drop them.
        self.framebuffer_cache.clear();
        self.grid_framebuffer_cache.clear();

        Ok(())
    }

    /// Write this frame slot's per-pass view-projection UBOs and resolve the
    /// set-0 descriptor sets (palette region + VP) for the geometry and
    /// shadow passes. Safe to write: the caller's fence discipline guarantees
    /// the slot's previous GPU use was reclaimed (render thread reclaims the
    /// slot fence before calling `render`).
    fn prepare_skinning_binds(
        &mut self,
        palette: Option<&SkinnedPaletteFrame>,
        camera_vp: Mat4,
        light_vp: [[f32; 4]; 4],
    ) -> Result<(Arc<DescriptorSet>, Arc<DescriptorSet>), Box<dyn std::error::Error>> {
        let (region, instances, slot) = match palette {
            Some(p) => {
                self.skin_binds.saw_palette_frame = true;
                (p.region.clone(), p.instances.clone(), p.slot % 4)
            }
            None => {
                // Tests/tools only — shipping hosts always send `Some`.
                // `fallback_slot` rotates independently of the fence ring, so
                // a `None` frame after palette frames could rewrite a VP UBO
                // still referenced by an in-flight frame 1-3 slots back.
                debug_assert!(
                    !self.skin_binds.saw_palette_frame,
                    "palette-less frame after palette-bearing frames: \
                     fallback slot rotation is not fence-synchronized"
                );
                self.skin_binds.fallback_slot = (self.skin_binds.fallback_slot + 1) % 4;
                (
                    self.skin_binds.identity_region.clone(),
                    self.skin_binds.identity_instances.clone(),
                    self.skin_binds.fallback_slot,
                )
            }
        };
        let geom_layout = self.geometry_pass.layout().set_layouts()[0].clone();
        let shadow_layout = self.shadow_pass.layout().set_layouts()[0].clone();
        let geom_set = self.skin_binds.geometry.bind(
            &self.descriptor_set_allocator,
            geom_layout,
            slot,
            &region,
            &instances,
            camera_vp.to_cols_array_2d(),
        )?;
        let shadow_set = self.skin_binds.shadow.bind(
            &self.descriptor_set_allocator,
            shadow_layout,
            slot,
            &region,
            &instances,
            light_vp,
        )?;
        Ok((geom_set, shadow_set))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        mesh_data: &[MeshRenderData],
        shadow_caster_data: &[MeshRenderData],
        light_data: &LightUniformData,
        target: RenderTarget,
        grid_visible: bool,
        view_proj: Mat4,
        camera_pos: Vec3,
        camera_near: f32,
        camera_far: f32,
        debug_draw: &DebugDrawData,
        settings: &PostProcessingSettings,
        plankton_emitters: &[crate::engine::rendering::frame_packet::PlanktonEmitterFrameData],
        palette: Option<&SkinnedPaletteFrame>,
    ) -> Result<Arc<PrimaryAutoCommandBuffer>, RenderError> {
        crate::profile_function!();

        self.render_counters.reset();

        let (geom_skin_set, shadow_skin_set) =
            self.prepare_skinning_binds(palette, view_proj, light_data.light_vp)?;

        let needs_depth_framebuffer = grid_visible || !debug_draw.is_empty();

        let (target_framebuffer, depth_framebuffer, mut builder) = {
            crate::profile_scope!("command_buffer_setup");
            let present = matches!(target, RenderTarget::Swapchain { .. });
            let target_image = target.image().clone();
            let target_framebuffer = self.get_or_create_framebuffer(target_image.clone(), present)?;
            let depth_framebuffer = if needs_depth_framebuffer {
                Some(self.get_or_create_grid_framebuffer(target_image, present)?)
            } else {
                None
            };
            let builder = AutoCommandBufferBuilder::primary(
                self.command_buffer_allocator.clone(),
                self.queue.queue_family_index(),
                CommandBufferUsage::OneTimeSubmit,
            )?;
            (target_framebuffer, depth_framebuffer, builder)
        };

        // Run plankton compute passes (init/emit/simulate) before graph execution.
        // Skip expensive camera-vector math when no emitters are active this frame —
        // update_frame still runs so pool eviction/absence counters keep progressing.
        if !plankton_emitters.is_empty() {
            let vp_inverse = view_proj.inverse();
            let cam_right = (vp_inverse * Vec4::new(1.0, 0.0, 0.0, 0.0))
                .truncate()
                .normalize();
            let cam_up = (vp_inverse * Vec4::new(0.0, 1.0, 0.0, 0.0))
                .truncate()
                .normalize();
            let vp_cols = view_proj.to_cols_array_2d();
            self.plankton_system.update_frame(
                &mut builder,
                plankton_emitters,
                &vp_cols,
                cam_right.into(),
                cam_up.into(),
                camera_near,
                camera_far,
            )?;
        } else {
            self.plankton_system.update_frame(
                &mut builder,
                &[],
                &[[0.0; 4]; 4],
                [0.0; 3],
                [0.0; 3],
                camera_near,
                camera_far,
            )?;
        }

        // Counters move into a RefCell during recording: multiple pass
        // executors (geometry, shadow) bump them while the graph's closures
        // hold shared borrows of `self`. Restored right after execution.
        let counters = std::cell::RefCell::new(std::mem::take(&mut self.render_counters));
        let counters_ref = &counters;
        // Executor closures are `move`: the ResourceIds they capture are
        // locals of the setup scope below, so by-ref capture would dangle.
        // Shared state enters as Copy references instead.
        let this: &Self = &*self;
        let target_fb_ref = &target_framebuffer;
        let depth_fb_ref = &depth_framebuffer;
        let geom_skin_set_ref = &geom_skin_set;
        let shadow_skin_set_ref = &shadow_skin_set;

        let mut graph = RenderGraph::new();
        {
            crate::profile_scope!("graph_setup");

            let gbuffer_position = graph.declare_virtual("gbuffer_position");
            let gbuffer_normal = graph.declare_virtual("gbuffer_normal");
            let gbuffer_albedo = graph.declare_virtual("gbuffer_albedo");
            let gbuffer_material = graph.declare_virtual("gbuffer_material");
            let gbuffer_emissive = graph.declare_virtual("gbuffer_emissive");
            let gbuffer_depth = graph.declare_virtual("gbuffer_depth");
            let target_res = graph.declare_virtual("target");
            let ssao_raw_res = graph.declare_virtual("ssao_raw");
            let ssao_blurred_res = graph.declare_virtual("ssao_blurred");
            let bloom_res = graph.declare_virtual("bloom_result");
            let lum_res = graph.declare_virtual("luminance_result");

            let shadow_map_res = graph.import_image("shadow_map", self.shadow_pass.shadow_map());
            let hdr_res = graph.import_image("hdr_target", self.hdr_target.clone());

            let gbuffer_color = [
                gbuffer_position,
                gbuffer_normal,
                gbuffer_albedo,
                gbuffer_material,
                gbuffer_emissive,
            ];
            let ssao_enabled = settings.ssao_enabled;

            // Each pass is registered with its declaration AND its executor in
            // one place — the graph owns dispatch (no name-based match), and
            // an executor-less pass is a hard error at execute time.
            graph.add_pass_with(
                "geometry",
                |b| {
                    for id in gbuffer_color {
                        b.write(id);
                    }
                    b.write(gbuffer_depth);
                },
                move |ctx| {
                    for id in gbuffer_color {
                        ctx.mark_write(id);
                    }
                    ctx.mark_write(gbuffer_depth);
                    this.record_geometry_pass(ctx.builder, mesh_data, geom_skin_set_ref, counters_ref)
                },
            );

            if light_data.shadow_enabled > 0.5 {
                graph.add_pass_with(
                    "shadow",
                    |b| {
                        b.write(shadow_map_res);
                    },
                    move |ctx| {
                        ctx.mark_write(shadow_map_res);
                        this.record_shadow_pass(
                            ctx.builder,
                            shadow_caster_data,
                            shadow_skin_set_ref,
                            counters_ref,
                        )
                    },
                );
            }

            if ssao_enabled {
                graph.add_pass_with(
                    "ssao",
                    |b| {
                        b.read(gbuffer_position);
                        b.read(gbuffer_normal);
                        b.write(ssao_raw_res);
                    },
                    move |ctx| {
                        ctx.mark_read(gbuffer_position);
                        ctx.mark_read(gbuffer_normal);
                        ctx.mark_write(ssao_raw_res);
                        this.record_ssao_pass(ctx.builder, view_proj, settings)
                    },
                );

                graph.add_pass_with(
                    "ssao_blur",
                    |b| {
                        b.read(ssao_raw_res);
                        b.write(ssao_blurred_res);
                    },
                    move |ctx| {
                        ctx.mark_read(ssao_raw_res);
                        ctx.mark_write(ssao_blurred_res);
                        this.record_ssao_blur_pass(ctx.builder)
                    },
                );
            }

            graph.add_pass_with(
                "lighting",
                |b| {
                    for id in gbuffer_color {
                        b.read(id);
                    }
                    b.read(shadow_map_res);
                    // The lighting shader samples the blurred SSAO texture when
                    // SSAO is on. This read was previously undeclared — the
                    // culler saw no consumer of ssao_blurred and removed the
                    // SSAO passes entirely, and ordering relied on the
                    // insertion-order tiebreak.
                    if ssao_enabled {
                        b.read(ssao_blurred_res);
                    }
                    b.write(hdr_res);
                },
                move |ctx| {
                    for id in gbuffer_color {
                        ctx.mark_read(id);
                    }
                    ctx.mark_read(shadow_map_res);
                    if ssao_enabled {
                        ctx.mark_read(ssao_blurred_res);
                    }
                    ctx.mark_write(hdr_res);
                    this.record_lighting_pass(ctx.builder, light_data, ssao_enabled)
                },
            );

            if self.plankton_system.has_pending_draws() {
                graph.add_pass_with(
                    "plankton",
                    |b| {
                        b.read(gbuffer_depth);
                        b.modify(hdr_res);
                    },
                    move |ctx| {
                        crate::profile_scope!("plankton_pass");
                        ctx.mark_read(gbuffer_depth);
                        // Blends over existing HDR content: read + write = modify.
                        ctx.mark_read(hdr_res);
                        ctx.mark_write(hdr_res);
                        this.plankton_system.render_particles(ctx.builder)
                    },
                );
            }

            if settings.bloom_enabled {
                graph.add_pass_with(
                    "bloom",
                    |b| {
                        b.read(hdr_res);
                        b.write(bloom_res);
                    },
                    move |ctx| {
                        ctx.mark_read(hdr_res);
                        ctx.mark_write(bloom_res);
                        this.record_bloom_pass(ctx.builder, settings)
                    },
                );
            }

            if matches!(settings.exposure_mode, ExposureMode::Auto) {
                graph.add_pass_with(
                    "luminance",
                    |b| {
                        b.read(hdr_res);
                        b.write(lum_res);
                    },
                    move |ctx| {
                        ctx.mark_read(hdr_res);
                        ctx.mark_write(lum_res);
                        this.record_luminance_pass(ctx.builder)
                    },
                );
            }

            graph.add_pass_with(
                "composite",
                |b| {
                    b.read(hdr_res);
                    b.read(bloom_res);
                    b.read(lum_res);
                    b.write(target_res);
                },
                move |ctx| {
                    ctx.mark_read(hdr_res);
                    ctx.mark_read(bloom_res);
                    ctx.mark_read(lum_res);
                    ctx.mark_write(target_res);
                    this.record_composite_pass(ctx.builder, target_fb_ref, settings)
                },
            );

            if grid_visible {
                graph.add_pass_with(
                    "grid",
                    |b| {
                        b.read(gbuffer_depth);
                        b.modify(target_res);
                    },
                    move |ctx| {
                        ctx.mark_read(gbuffer_depth);
                        // Alpha-blends over the composited target: modify.
                        ctx.mark_read(target_res);
                        ctx.mark_write(target_res);
                        let fb = depth_fb_ref
                            .as_ref()
                            .ok_or("grid pass requires the depth framebuffer")?;
                        this.record_grid_pass(ctx.builder, fb, view_proj, camera_pos)
                    },
                );
            }

            if !debug_draw.is_empty() {
                graph.add_pass_with(
                    "debug_draw",
                    |b| {
                        b.read(gbuffer_depth);
                        b.modify(target_res);
                    },
                    move |ctx| {
                        ctx.mark_read(gbuffer_depth);
                        // Alpha-blends over the composited target: modify.
                        ctx.mark_read(target_res);
                        ctx.mark_write(target_res);
                        let fb = depth_fb_ref
                            .as_ref()
                            .ok_or("debug_draw pass requires the depth framebuffer")?;
                        this.record_debug_draw_pass(ctx.builder, fb, debug_draw, view_proj)
                    },
                );
            }

            graph.mark_output(target_res);
            graph.enable_culling();
            graph.compile()?;
        }

        graph.execute(&mut builder)?;
        drop(graph);
        self.render_counters = counters.into_inner();

        let command_buffer = {
            crate::profile_scope!("command_buffer_build");
            builder.build()?
        };

        Ok(command_buffer)
    }

    pub fn set_debug_view(&mut self, view: DebugView) {
        self.debug_view = view;
    }

    pub fn render_counters(&self) -> &RenderCounters {
        &self.render_counters
    }

    pub fn geometry_pipeline(&self) -> Arc<vulkano::pipeline::GraphicsPipeline> {
        self.geometry_pass.pipeline(&self.pipeline_registry)
    }

    /// Access the pipeline registry (for debug menu rebuild triggers).
    pub fn pipeline_registry(&self) -> &PipelineRegistry {
        &self.pipeline_registry
    }

    /// Default material descriptor set (Set 1) for meshes without a material.
    pub fn default_material_set(&self) -> &Arc<DescriptorSet> {
        &self.default_material_set
    }

    fn record_shadow_pass(
        &self,
        builder: &mut AutoCommandBufferBuilder<PrimaryAutoCommandBuffer>,
        mesh_data: &[MeshRenderData],
        skin_set: &Arc<DescriptorSet>,
        counters: &std::cell::RefCell<RenderCounters>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        crate::profile_scope!("shadow_pass");

        let shadow_extent = self.shadow_pass.shadow_map().image().extent();
        let shadow_w = shadow_extent[0];
        let shadow_h = shadow_extent[1];
        let shadow_viewport = vulkano::pipeline::graphics::viewport::Viewport {
            offset: [0.0, 0.0],
            extent: [shadow_w as f32, shadow_h as f32],
            depth_range: 0.0..=1.0,
        };
        let shadow_scissor = vulkano::pipeline::graphics::viewport::Scissor {
            offset: [0, 0],
            extent: [shadow_w, shadow_h],
        };

        builder
            .begin_render_pass(
                RenderPassBeginInfo {
                    clear_values: vec![Some(1.0.into())],
                    ..RenderPassBeginInfo::framebuffer(self.shadow_pass.framebuffer())
                },
                SubpassBeginInfo {
                    contents: SubpassContents::Inline,
                    ..Default::default()
                },
            )?
            .bind_pipeline_graphics(self.shadow_pass.pipeline())?
            .set_viewport(0, smallvec![shadow_viewport])?
            .set_scissor(0, smallvec![shadow_scissor])?;

        let shadow_layout = self.shadow_pass.layout();
        // One palette/VP/instance bind for the whole pass (SSBO rings,
        // S-D1 + S-D5); per-draw data lives in the instance-metadata SSBO,
        // addressed by gl_InstanceIndex (= first_instance + i in Vulkan).
        builder.bind_descriptor_sets(
            PipelineBindPoint::Graphics,
            shadow_layout.clone(),
            0,
            skin_set.clone(),
        )?;
        let mut counters = counters.borrow_mut();
        for mesh in mesh_data {
            builder
                .bind_vertex_buffers(0, mesh.vertex_buffer.clone())?
                .bind_index_buffer(mesh.index_buffer.clone())?;
            unsafe {
                builder.draw_indexed(
                    mesh.index_count,
                    mesh.instance_count,
                    0,
                    0,
                    mesh.first_instance,
                )?;
            }
            counters.draw_calls += 1;
        }

        builder.end_render_pass(SubpassEndInfo::default())?;
        Ok(())
    }

    fn record_geometry_pass(
        &self,
        builder: &mut AutoCommandBufferBuilder<PrimaryAutoCommandBuffer>,
        mesh_data: &[MeshRenderData],
        skin_set: &Arc<DescriptorSet>,
        counters: &std::cell::RefCell<RenderCounters>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        crate::profile_scope!("geometry_pass");

        let gbuffer_extent = self.gbuffer.framebuffer.extent();
        let viewport = vulkano::pipeline::graphics::viewport::Viewport {
            offset: [0.0, 0.0],
            extent: [gbuffer_extent[0] as f32, gbuffer_extent[1] as f32],
            depth_range: 0.0..=1.0,
        };
        let scissor = vulkano::pipeline::graphics::viewport::Scissor {
            offset: [0, 0],
            extent: [gbuffer_extent[0], gbuffer_extent[1]],
        };

        builder
            .begin_render_pass(
                RenderPassBeginInfo {
                    clear_values: vec![
                        Some([0.0, 0.0, 0.0, 1.0].into()),
                        Some([0.0, 0.0, 0.0, 1.0].into()),
                        Some([0.0, 0.0, 0.0, 1.0].into()),
                        Some([0.0, 0.0, 0.0, 1.0].into()),
                        Some([0.0, 0.0, 0.0, 0.0].into()), // emissive
                        Some(1.0.into()),
                    ],
                    ..RenderPassBeginInfo::framebuffer(self.gbuffer.framebuffer.clone())
                },
                SubpassBeginInfo {
                    contents: SubpassContents::Inline,
                    ..Default::default()
                },
            )?
            .bind_pipeline_graphics(self.geometry_pass.pipeline(&self.pipeline_registry))?
            .set_viewport(0, smallvec![viewport])?
            .set_scissor(0, smallvec![scissor])?;

        {
            crate::profile_scope!("mesh_loop");
            let mut counters = counters.borrow_mut();
            let mut last_material_ptr: Option<usize> = None;
            let geom_layout = self.geometry_pass.layout();
            // One palette/VP/instance bind for the whole pass (SSBO rings,
            // S-D1 + S-D5); per-draw model + palette_base come from the
            // instance-metadata SSBO via gl_InstanceIndex.
            builder.bind_descriptor_sets(
                PipelineBindPoint::Graphics,
                geom_layout.clone(),
                0,
                skin_set.clone(),
            )?;
            for mesh in mesh_data {
                counters.visible_entities += mesh.instance_count;
                counters.draw_calls += 1;
                counters.triangles += (mesh.index_count / 3) * mesh.instance_count;

                let mat_set = mesh
                    .material_descriptor_set
                    .as_ref()
                    .unwrap_or(&self.default_material_set);
                let mat_ptr = Arc::as_ptr(mat_set) as usize;
                if last_material_ptr != Some(mat_ptr) {
                    builder.bind_descriptor_sets(
                        PipelineBindPoint::Graphics,
                        geom_layout.clone(),
                        1,
                        mat_set.clone(),
                    )?;
                    last_material_ptr = Some(mat_ptr);
                    counters.material_changes += 1;
                }

                builder
                    .bind_vertex_buffers(0, mesh.vertex_buffer.clone())?
                    .bind_index_buffer(mesh.index_buffer.clone())?;
                unsafe {
                    builder.draw_indexed(
                        mesh.index_count,
                        mesh.instance_count,
                        0,
                        0,
                        mesh.first_instance,
                    )?;
                }
            }
        }

        builder.end_render_pass(SubpassEndInfo::default())?;
        Ok(())
    }

    fn record_lighting_pass(
        &self,
        builder: &mut AutoCommandBufferBuilder<PrimaryAutoCommandBuffer>,
        light_data: &LightUniformData,
        ssao_enabled: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        crate::profile_scope!("lighting_pass");

        let hdr_framebuffer = self
            .lighting_pass
            .hdr_framebuffer()
            .ok_or("lighting hdr framebuffer not prepared")?
            .clone();
        let gbuffer_set = self
            .lighting_pass
            .gbuffer_set()
            .ok_or("lighting gbuffer set not prepared")?
            .clone();
        let shadow_set = self
            .lighting_pass
            .shadow_set()
            .ok_or("lighting shadow set not prepared")?
            .clone();
        let ssao_set = if ssao_enabled {
            self.lighting_pass.ssao_set()
        } else {
            self.lighting_pass.ssao_fallback_set()
        }
        .ok_or("lighting ssao set not prepared")?
        .clone();

        let hdr_extent = hdr_framebuffer.extent();
        let hdr_viewport = vulkano::pipeline::graphics::viewport::Viewport {
            offset: [0.0, 0.0],
            extent: [hdr_extent[0] as f32, hdr_extent[1] as f32],
            depth_range: 0.0..=1.0,
        };
        let hdr_scissor = vulkano::pipeline::graphics::viewport::Scissor {
            offset: [0, 0],
            extent: [hdr_extent[0], hdr_extent[1]],
        };

        builder
            .begin_render_pass(
                RenderPassBeginInfo {
                    clear_values: vec![Some([0.0, 0.0, 0.0, 1.0].into())],
                    ..RenderPassBeginInfo::framebuffer(hdr_framebuffer)
                },
                SubpassBeginInfo {
                    contents: SubpassContents::Inline,
                    ..Default::default()
                },
            )?
            .bind_pipeline_graphics(self.lighting_pass.pipeline(&self.pipeline_registry))?
            .set_viewport(0, smallvec![hdr_viewport])?
            .set_scissor(0, smallvec![hdr_scissor])?
            .bind_descriptor_sets(
                PipelineBindPoint::Graphics,
                self.lighting_pass.layout(),
                0,
                gbuffer_set,
            )?
            .bind_descriptor_sets(
                PipelineBindPoint::Graphics,
                self.lighting_pass.layout(),
                1,
                shadow_set,
            )?
            .bind_descriptor_sets(
                PipelineBindPoint::Graphics,
                self.lighting_pass.layout(),
                2,
                ssao_set,
            )?
            .push_constants(self.lighting_pass.layout(), 0, *light_data)?;
        unsafe {
            builder.draw(3, 1, 0, 0)?;
        }
        builder.end_render_pass(SubpassEndInfo::default())?;
        Ok(())
    }

    fn record_composite_pass(
        &self,
        builder: &mut AutoCommandBufferBuilder<PrimaryAutoCommandBuffer>,
        target_framebuffer: &Arc<Framebuffer>,
        settings: &PostProcessingSettings,
    ) -> Result<(), Box<dyn std::error::Error>> {
        crate::profile_scope!("composite_pass");

        let composite_set = self
            .composite_pass
            .descriptor_set()
            .ok_or("composite set not prepared")?
            .clone();

        let target_extent = target_framebuffer.extent();
        let target_viewport = vulkano::pipeline::graphics::viewport::Viewport {
            offset: [0.0, 0.0],
            extent: [target_extent[0] as f32, target_extent[1] as f32],
            depth_range: 0.0..=1.0,
        };
        let target_scissor = vulkano::pipeline::graphics::viewport::Scissor {
            offset: [0, 0],
            extent: [target_extent[0], target_extent[1]],
        };

        let exposure_val = match settings.exposure_mode {
            ExposureMode::Manual(v) => v,
            ExposureMode::Auto => 1.0,
        };
        let composite_push = CompositePushConstants {
            exposure: exposure_val,
            bloom_intensity: if settings.bloom_enabled {
                settings.bloom_intensity
            } else {
                0.0
            },
            vignette_intensity: settings.vignette_intensity,
            tone_map_mode: match settings.tone_map_mode {
                ToneMapMode::Reinhard => 0.0,
                ToneMapMode::AcesFilmic => 1.0,
            },
            exposure_mode: match settings.exposure_mode {
                ExposureMode::Auto => 0.0,
                ExposureMode::Manual(_) => 1.0,
            },
            _pad0: 0.0,
            _pad1: 0.0,
            _pad2: 0.0,
        };

        builder
            .begin_render_pass(
                RenderPassBeginInfo {
                    clear_values: vec![Some([0.0, 0.0, 0.0, 1.0].into())],
                    ..RenderPassBeginInfo::framebuffer(target_framebuffer.clone())
                },
                SubpassBeginInfo {
                    contents: SubpassContents::Inline,
                    ..Default::default()
                },
            )?
            .bind_pipeline_graphics(self.composite_pass.pipeline())?
            .set_viewport(0, smallvec![target_viewport])?
            .set_scissor(0, smallvec![target_scissor])?
            .bind_descriptor_sets(
                PipelineBindPoint::Graphics,
                self.composite_pass.layout(),
                0,
                composite_set,
            )?
            .push_constants(self.composite_pass.layout(), 0, composite_push)?;
        unsafe {
            builder.draw(3, 1, 0, 0)?;
        }
        builder.end_render_pass(SubpassEndInfo::default())?;
        Ok(())
    }

    fn record_grid_pass(
        &self,
        builder: &mut AutoCommandBufferBuilder<PrimaryAutoCommandBuffer>,
        grid_fb: &Arc<Framebuffer>,
        view_proj: Mat4,
        camera_pos: Vec3,
    ) -> Result<(), Box<dyn std::error::Error>> {
        crate::profile_scope!("grid_pass");

        let grid_extent = grid_fb.extent();
        let grid_viewport = vulkano::pipeline::graphics::viewport::Viewport {
            offset: [0.0, 0.0],
            extent: [grid_extent[0] as f32, grid_extent[1] as f32],
            depth_range: 0.0..=1.0,
        };
        let grid_scissor = vulkano::pipeline::graphics::viewport::Scissor {
            offset: [0, 0],
            extent: [grid_extent[0], grid_extent[1]],
        };

        let grid_extent_size = 500.0;
        let grid_push = GridPushConstants::new(view_proj, camera_pos, grid_extent_size, 100.0);

        builder
            .begin_render_pass(
                RenderPassBeginInfo {
                    clear_values: vec![None, None],
                    ..RenderPassBeginInfo::framebuffer(grid_fb.clone())
                },
                SubpassBeginInfo {
                    contents: SubpassContents::Inline,
                    ..Default::default()
                },
            )?
            .bind_pipeline_graphics(self.grid_pass.pipeline())?
            .set_viewport(0, smallvec![grid_viewport])?
            .set_scissor(0, smallvec![grid_scissor])?
            .push_constants(self.grid_pass.layout(), 0, grid_push)?;

        unsafe {
            builder.draw(4, 1, 0, 0)?;
        }
        builder.end_render_pass(SubpassEndInfo::default())?;
        Ok(())
    }

    fn record_debug_draw_pass(
        &self,
        builder: &mut AutoCommandBufferBuilder<PrimaryAutoCommandBuffer>,
        debug_fb: &Arc<Framebuffer>,
        debug_draw: &DebugDrawData,
        view_proj: Mat4,
    ) -> Result<(), Box<dyn std::error::Error>> {
        crate::profile_scope!("debug_draw_pass");

        let debug_extent = debug_fb.extent();
        let debug_viewport = vulkano::pipeline::graphics::viewport::Viewport {
            offset: [0.0, 0.0],
            extent: [debug_extent[0] as f32, debug_extent[1] as f32],
            depth_range: 0.0..=1.0,
        };
        let debug_scissor = vulkano::pipeline::graphics::viewport::Scissor {
            offset: [0, 0],
            extent: [debug_extent[0], debug_extent[1]],
        };

        let debug_push = DebugLinePushConstants {
            view_proj: view_proj.to_cols_array_2d(),
        };

        builder.begin_render_pass(
            RenderPassBeginInfo {
                clear_values: vec![None, None],
                ..RenderPassBeginInfo::framebuffer(debug_fb.clone())
            },
            SubpassBeginInfo {
                contents: SubpassContents::Inline,
                ..Default::default()
            },
        )?;

        if let Some(ref depth_buf) = debug_draw.depth_buffer {
            builder
                .bind_pipeline_graphics(self.debug_draw_pass.depth_pipeline())?
                .set_viewport(0, smallvec![debug_viewport.clone()])?
                .set_scissor(0, smallvec![debug_scissor])?
                .push_constants(self.debug_draw_pass.layout(), 0, debug_push)?
                .bind_vertex_buffers(0, depth_buf.clone())?;
            unsafe {
                builder.draw(debug_draw.depth_vertex_count, 1, 0, 0)?;
            }
        }

        if let Some(ref static_buf) = debug_draw.static_depth_buffer {
            builder
                .bind_pipeline_graphics(self.debug_draw_pass.depth_pipeline())?
                .set_viewport(0, smallvec![debug_viewport.clone()])?
                .set_scissor(0, smallvec![debug_scissor])?
                .push_constants(self.debug_draw_pass.layout(), 0, debug_push)?
                .bind_vertex_buffers(0, static_buf.clone())?;
            unsafe {
                builder.draw(debug_draw.static_depth_vertex_count, 1, 0, 0)?;
            }
        }

        if let Some(ref overlay_buf) = debug_draw.overlay_buffer {
            builder
                .bind_pipeline_graphics(self.debug_draw_pass.overlay_pipeline())?
                .set_viewport(0, smallvec![debug_viewport])?
                .set_scissor(0, smallvec![debug_scissor])?
                .push_constants(self.debug_draw_pass.layout(), 0, debug_push)?
                .bind_vertex_buffers(0, overlay_buf.clone())?;
            unsafe {
                builder.draw(debug_draw.overlay_vertex_count, 1, 0, 0)?;
            }
        }

        builder.end_render_pass(SubpassEndInfo::default())?;
        Ok(())
    }

    fn record_ssao_pass(
        &self,
        builder: &mut AutoCommandBufferBuilder<PrimaryAutoCommandBuffer>,
        view_proj: Mat4,
        settings: &PostProcessingSettings,
    ) -> Result<(), Box<dyn std::error::Error>> {
        crate::profile_scope!("ssao_pass");

        let fb = self.ssao_pass.ssao_raw_framebuffer();
        let extent = fb.extent();
        let viewport = vulkano::pipeline::graphics::viewport::Viewport {
            offset: [0.0, 0.0],
            extent: [extent[0] as f32, extent[1] as f32],
            depth_range: 0.0..=1.0,
        };
        let scissor = vulkano::pipeline::graphics::viewport::Scissor {
            offset: [0, 0],
            extent: [extent[0], extent[1]],
        };

        let ssao_desc = self
            .ssao_pass
            .ssao_gbuffer_set()
            .ok_or("ssao gbuffer set not prepared")?
            .clone();
        let kernel_desc = self
            .ssao_pass
            .ssao_kernel_set()
            .ok_or("ssao kernel set not prepared")?
            .clone();

        let push = SsaoPushConstants {
            view_projection: view_proj.to_cols_array_2d(),
            screen_size: [extent[0] as f32, extent[1] as f32],
            radius: settings.ssao_radius,
            bias: settings.ssao_bias,
        };

        builder
            .begin_render_pass(
                RenderPassBeginInfo {
                    clear_values: vec![None],
                    ..RenderPassBeginInfo::framebuffer(fb)
                },
                SubpassBeginInfo {
                    contents: SubpassContents::Inline,
                    ..Default::default()
                },
            )?
            .bind_pipeline_graphics(self.ssao_pass.ssao_pipeline())?
            .set_viewport(0, smallvec![viewport])?
            .set_scissor(0, smallvec![scissor])?
            .bind_descriptor_sets(
                PipelineBindPoint::Graphics,
                self.ssao_pass.ssao_layout().clone(),
                0,
                ssao_desc,
            )?
            .bind_descriptor_sets(
                PipelineBindPoint::Graphics,
                self.ssao_pass.ssao_layout().clone(),
                1,
                kernel_desc,
            )?
            .push_constants(self.ssao_pass.ssao_layout().clone(), 0, push)?;
        unsafe {
            builder.draw(3, 1, 0, 0)?;
        }
        builder.end_render_pass(SubpassEndInfo::default())?;
        Ok(())
    }

    fn record_ssao_blur_pass(
        &self,
        builder: &mut AutoCommandBufferBuilder<PrimaryAutoCommandBuffer>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        crate::profile_scope!("ssao_blur_pass");

        let fb = self.ssao_pass.ssao_blur_framebuffer();
        let extent = fb.extent();
        let viewport = vulkano::pipeline::graphics::viewport::Viewport {
            offset: [0.0, 0.0],
            extent: [extent[0] as f32, extent[1] as f32],
            depth_range: 0.0..=1.0,
        };
        let scissor = vulkano::pipeline::graphics::viewport::Scissor {
            offset: [0, 0],
            extent: [extent[0], extent[1]],
        };

        let blur_desc = self
            .ssao_pass
            .blur_set()
            .ok_or("ssao blur set not prepared")?
            .clone();

        builder
            .begin_render_pass(
                RenderPassBeginInfo {
                    clear_values: vec![None],
                    ..RenderPassBeginInfo::framebuffer(fb)
                },
                SubpassBeginInfo {
                    contents: SubpassContents::Inline,
                    ..Default::default()
                },
            )?
            .bind_pipeline_graphics(self.ssao_pass.blur_pipeline())?
            .set_viewport(0, smallvec![viewport])?
            .set_scissor(0, smallvec![scissor])?
            .bind_descriptor_sets(
                PipelineBindPoint::Graphics,
                self.ssao_pass.blur_layout().clone(),
                0,
                blur_desc,
            )?;
        unsafe {
            builder.draw(3, 1, 0, 0)?;
        }
        builder.end_render_pass(SubpassEndInfo::default())?;
        Ok(())
    }

    fn record_luminance_pass(
        &self,
        builder: &mut AutoCommandBufferBuilder<PrimaryAutoCommandBuffer>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        crate::profile_scope!("luminance_pass");

        let level_count = self.luminance_pass.level_count();
        let level_fbs = self.luminance_pass.level_framebuffers();
        let level_sizes = self.luminance_pass.level_sizes();

        for i in 0..level_count {
            let size = level_sizes[i];
            let viewport = vulkano::pipeline::graphics::viewport::Viewport {
                offset: [0.0, 0.0],
                extent: [size as f32, size as f32],
                depth_range: 0.0..=1.0,
            };
            let scissor = vulkano::pipeline::graphics::viewport::Scissor {
                offset: [0, 0],
                extent: [size, size],
            };

            let desc = self
                .luminance_pass
                .level_set(i)
                .ok_or("luminance level set not prepared")?
                .clone();

            let push = LuminancePush {
                is_first_pass: if i == 0 { 1.0 } else { 0.0 },
                _pad0: 0.0,
                _pad1: 0.0,
                _pad2: 0.0,
            };

            builder
                .begin_render_pass(
                    RenderPassBeginInfo {
                        clear_values: vec![None],
                        ..RenderPassBeginInfo::framebuffer(level_fbs[i].clone())
                    },
                    SubpassBeginInfo {
                        contents: SubpassContents::Inline,
                        ..Default::default()
                    },
                )?
                .bind_pipeline_graphics(self.luminance_pass.pipeline())?
                .set_viewport(0, smallvec![viewport])?
                .set_scissor(0, smallvec![scissor])?
                .bind_descriptor_sets(
                    PipelineBindPoint::Graphics,
                    self.luminance_pass.layout().clone(),
                    0,
                    desc,
                )?
                .push_constants(self.luminance_pass.layout().clone(), 0, push)?;
            unsafe {
                builder.draw(3, 1, 0, 0)?;
            }
            builder.end_render_pass(SubpassEndInfo::default())?;
        }

        Ok(())
    }

    fn record_bloom_pass(
        &self,
        builder: &mut AutoCommandBufferBuilder<PrimaryAutoCommandBuffer>,
        settings: &PostProcessingSettings,
    ) -> Result<(), Box<dyn std::error::Error>> {
        crate::profile_scope!("bloom_pass");

        // The mip chain is allocated at its full depth; the setting only
        // limits how far down the downsample/upsample walk goes.
        let mip_count = (settings.bloom_mip_count as usize).clamp(1, self.bloom_pass.mip_count());
        let mip_sizes = self.bloom_pass.mip_sizes();
        let mip_fbs = self.bloom_pass.mip_framebuffers();
        let additive_fbs = self.bloom_pass.additive_framebuffers();

        // Threshold: HDR → mip[0]
        {
            let size = mip_sizes[0];
            let viewport = vulkano::pipeline::graphics::viewport::Viewport {
                offset: [0.0, 0.0],
                extent: [size[0] as f32, size[1] as f32],
                depth_range: 0.0..=1.0,
            };
            let scissor = vulkano::pipeline::graphics::viewport::Scissor {
                offset: [0, 0],
                extent: [size[0], size[1]],
            };

            let desc = self
                .bloom_pass
                .threshold_set()
                .ok_or("bloom threshold set not prepared")?
                .clone();

            builder
                .begin_render_pass(
                    RenderPassBeginInfo {
                        clear_values: vec![None],
                        ..RenderPassBeginInfo::framebuffer(mip_fbs[0].clone())
                    },
                    SubpassBeginInfo {
                        contents: SubpassContents::Inline,
                        ..Default::default()
                    },
                )?
                .bind_pipeline_graphics(self.bloom_pass.threshold_pipeline())?
                .set_viewport(0, smallvec![viewport])?
                .set_scissor(0, smallvec![scissor])?
                .bind_descriptor_sets(
                    PipelineBindPoint::Graphics,
                    self.bloom_pass.threshold_layout().clone(),
                    0,
                    desc,
                )?
                .push_constants(
                    self.bloom_pass.threshold_layout().clone(),
                    0,
                    BloomThresholdPush {
                        threshold: settings.bloom_threshold,
                        _pad0: 0.0,
                        _pad1: 0.0,
                        _pad2: 0.0,
                    },
                )?;
            unsafe {
                builder.draw(3, 1, 0, 0)?;
            }
            builder.end_render_pass(SubpassEndInfo::default())?;
        }

        // Downsample: mip[i-1] → mip[i]
        for i in 1..mip_count {
            let size = mip_sizes[i];
            let src_size = mip_sizes[i - 1];
            let viewport = vulkano::pipeline::graphics::viewport::Viewport {
                offset: [0.0, 0.0],
                extent: [size[0] as f32, size[1] as f32],
                depth_range: 0.0..=1.0,
            };
            let scissor = vulkano::pipeline::graphics::viewport::Scissor {
                offset: [0, 0],
                extent: [size[0], size[1]],
            };

            let desc = self
                .bloom_pass
                .downsample_set(i - 1)
                .ok_or("bloom downsample set not prepared")?
                .clone();

            builder
                .begin_render_pass(
                    RenderPassBeginInfo {
                        clear_values: vec![None],
                        ..RenderPassBeginInfo::framebuffer(mip_fbs[i].clone())
                    },
                    SubpassBeginInfo {
                        contents: SubpassContents::Inline,
                        ..Default::default()
                    },
                )?
                .bind_pipeline_graphics(self.bloom_pass.downsample_pipeline())?
                .set_viewport(0, smallvec![viewport])?
                .set_scissor(0, smallvec![scissor])?
                .bind_descriptor_sets(
                    PipelineBindPoint::Graphics,
                    self.bloom_pass.downsample_layout().clone(),
                    0,
                    desc,
                )?
                .push_constants(
                    self.bloom_pass.downsample_layout().clone(),
                    0,
                    BloomSamplePush {
                        texel_size: [1.0 / src_size[0] as f32, 1.0 / src_size[1] as f32],
                        is_first_pass: if i == 1 { 1.0 } else { 0.0 },
                        _pad: 0.0,
                    },
                )?;
            unsafe {
                builder.draw(3, 1, 0, 0)?;
            }
            builder.end_render_pass(SubpassEndInfo::default())?;
        }

        // Upsample: mip[i] → mip[i-1] (additive)
        for i in (0..mip_count - 1).rev() {
            let size = mip_sizes[i];
            let src_size = mip_sizes[i + 1];
            let viewport = vulkano::pipeline::graphics::viewport::Viewport {
                offset: [0.0, 0.0],
                extent: [size[0] as f32, size[1] as f32],
                depth_range: 0.0..=1.0,
            };
            let scissor = vulkano::pipeline::graphics::viewport::Scissor {
                offset: [0, 0],
                extent: [size[0], size[1]],
            };

            let desc = self
                .bloom_pass
                .upsample_set(i)
                .ok_or("bloom upsample set not prepared")?
                .clone();

            builder
                .begin_render_pass(
                    RenderPassBeginInfo {
                        clear_values: vec![None],
                        ..RenderPassBeginInfo::framebuffer(additive_fbs[i].clone())
                    },
                    SubpassBeginInfo {
                        contents: SubpassContents::Inline,
                        ..Default::default()
                    },
                )?
                .bind_pipeline_graphics(self.bloom_pass.upsample_pipeline())?
                .set_viewport(0, smallvec![viewport])?
                .set_scissor(0, smallvec![scissor])?
                .bind_descriptor_sets(
                    PipelineBindPoint::Graphics,
                    self.bloom_pass.upsample_layout().clone(),
                    0,
                    desc,
                )?
                .push_constants(
                    self.bloom_pass.upsample_layout().clone(),
                    0,
                    BloomSamplePush {
                        texel_size: [1.0 / src_size[0] as f32, 1.0 / src_size[1] as f32],
                        is_first_pass: 0.0,
                        _pad: 0.0,
                    },
                )?;
            unsafe {
                builder.draw(3, 1, 0, 0)?;
            }
            builder.end_render_pass(SubpassEndInfo::default())?;
        }

        Ok(())
    }
}

/// One instanced draw batch (Task 41.5 P7): all instances sharing this
/// submesh + material, occupying the contiguous instance-metadata span
/// `first_instance..first_instance + instance_count` in the frame's ring
/// region. The camera list and the shadow list carry entries over the same
/// span with different counts — instances are written camera-visible-first,
/// so the camera entry's count is a prefix of the shadow entry's (shadow
/// draws all casters unculled).
#[derive(Clone)]
pub struct MeshRenderData {
    pub vertex_buffer: Subbuffer<[crate::engine::rendering::rendering_3d::Vertex3D]>,
    pub index_buffer: Subbuffer<[u32]>,
    pub index_count: u32,
    pub mesh_index: usize,
    pub material_index: usize,
    /// First index of this batch's span in the frame's instance-metadata
    /// region; passed to `draw_indexed` so `gl_InstanceIndex` starts here.
    pub first_instance: u32,
    pub instance_count: u32,
    /// Pre-resolved material descriptor set (Set 1 for geometry pass).
    /// Resolved at `prepare_mesh_data` time — the render thread does no manager lookups.
    pub material_descriptor_set: Option<Arc<DescriptorSet>>,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LightUniformData {
    pub camera_position: [f32; 3],
    pub shadow_bias: f32,
    pub directional_light_dir: [f32; 3],
    pub shadow_enabled: f32,
    pub directional_light_color: [f32; 3],
    pub directional_light_intensity: f32,
    pub ambient_color: [f32; 3],
    pub ambient_intensity: f32,
    pub light_vp: [[f32; 4]; 4],
}

unsafe impl bytemuck::Pod for LightUniformData {}
unsafe impl bytemuck::Zeroable for LightUniformData {}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ExposureMode {
    Auto,
    Manual(f32),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ToneMapMode {
    Reinhard,
    AcesFilmic,
}

/// Post-processing knobs consumed by the deferred renderer each frame.
///
/// Reaches the render thread as `FramePacket::post_processing` — today the
/// packet builders always use `Default`, so a future settings page (Editor
/// Preferences / Project Settings) plugs in by writing these fields on the
/// packet before send.
pub struct PostProcessingSettings {
    pub bloom_enabled: bool,
    pub bloom_intensity: f32,
    pub bloom_threshold: f32,
    /// How many mips of the bloom chain to walk (clamped to the allocated
    /// chain depth, currently 6). Fewer mips = tighter, cheaper bloom.
    pub bloom_mip_count: u32,
    pub ssao_enabled: bool,
    pub ssao_radius: f32,
    /// Depth-compare bias that suppresses self-occlusion acne.
    pub ssao_bias: f32,
    pub ssao_intensity: f32,
    pub exposure_mode: ExposureMode,
    pub vignette_intensity: f32,
    pub tone_map_mode: ToneMapMode,
}

impl Default for PostProcessingSettings {
    fn default() -> Self {
        Self {
            bloom_enabled: true,
            bloom_intensity: 0.04,
            bloom_threshold: 1.0,
            bloom_mip_count: 6,
            // Disabled by default: current blur/noise produces visible
            // tile-aligned banding. Re-enable once blur kernel is widened.
            ssao_enabled: false,
            ssao_radius: 0.5,
            ssao_bias: 0.025,
            ssao_intensity: 1.0,
            // Manual default avoids the "darker when close" snap from the
            // instant auto-exposure path. Scene brightness should be driven by
            // light intensity; users can opt into Auto for adaptive behavior.
            exposure_mode: ExposureMode::Manual(1.0),
            vignette_intensity: 0.0,
            tone_map_mode: ToneMapMode::Reinhard,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn light_uniform_data_size() {
        assert_eq!(std::mem::size_of::<LightUniformData>(), 128);
    }
}
