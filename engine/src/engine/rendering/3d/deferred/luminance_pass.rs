use smallvec::smallvec;
use std::sync::Arc;
use vulkano::descriptor_set::allocator::StandardDescriptorSetAllocator;
use vulkano::descriptor_set::{DescriptorSet, WriteDescriptorSet};
use vulkano::device::Device;
use vulkano::format::Format;
use vulkano::image::sampler::{Filter, Sampler, SamplerAddressMode, SamplerCreateInfo};
use vulkano::image::view::ImageView;
use vulkano::image::{Image, ImageCreateInfo, ImageType, ImageUsage};
use vulkano::memory::allocator::{AllocationCreateInfo, MemoryTypeFilter, StandardMemoryAllocator};
use vulkano::pipeline::graphics::{
    color_blend::{ColorBlendAttachmentState, ColorBlendState},
    input_assembly::InputAssemblyState,
    multisample::MultisampleState,
    rasterization::RasterizationState,
    vertex_input::VertexInputState,
    viewport::ViewportState,
    GraphicsPipelineCreateInfo,
};
use vulkano::pipeline::layout::PipelineDescriptorSetLayoutCreateInfo;
use vulkano::pipeline::{GraphicsPipeline, PipelineLayout, PipelineShaderStageCreateInfo};
use vulkano::render_pass::{Framebuffer, FramebufferCreateInfo, RenderPass};

pub mod lum_fullscreen_vs {
    vulkano_shaders::shader! {
        ty: "vertex",
        path: "src/engine/rendering/shaders/deferred/composite.vert",
    }
}

pub mod lum_downsample_fs {
    vulkano_shaders::shader! {
        ty: "fragment",
        path: "src/engine/rendering/shaders/deferred/luminance.frag",
    }
}

const LUM_LEVELS: usize = 9;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LuminancePush {
    pub is_first_pass: f32,
    pub _pad0: f32,
    pub _pad1: f32,
    pub _pad2: f32,
}

unsafe impl bytemuck::Pod for LuminancePush {}
unsafe impl bytemuck::Zeroable for LuminancePush {}

pub struct LuminancePass {
    pipeline: Arc<GraphicsPipeline>,
    layout: Arc<PipelineLayout>,
    sampler: Arc<Sampler>,
    render_pass: Arc<RenderPass>,
    level_images: Vec<Arc<ImageView>>,
    level_framebuffers: Vec<Arc<Framebuffer>>,
    level_sizes: Vec<u32>,
    persistent_1x1: Arc<ImageView>,
    // One cached set per level — source is hdr_target for level 0 and
    // level_images[i - 1] otherwise. Populated by prepare_sets().
    level_sets: Vec<Arc<DescriptorSet>>,
}

impl LuminancePass {
    pub fn new(
        device: Arc<Device>,
        allocator: Arc<StandardMemoryAllocator>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let sampler = Sampler::new(
            device.clone(),
            SamplerCreateInfo {
                mag_filter: Filter::Linear,
                min_filter: Filter::Linear,
                address_mode: [SamplerAddressMode::ClampToEdge; 3],
                ..Default::default()
            },
        )?;

        let render_pass = vulkano::single_pass_renderpass!(
            device.clone(),
            attachments: {
                color: {
                    format: Format::R16_SFLOAT,
                    samples: 1,
                    load_op: DontCare,
                    store_op: Store,
                }
            },
            pass: {
                color: [color],
                depth_stencil: {}
            }
        )?;

        let persistent_1x1 = Image::new(
            allocator.clone(),
            ImageCreateInfo {
                image_type: ImageType::Dim2d,
                format: Format::R16_SFLOAT,
                extent: [1, 1, 1],
                usage: ImageUsage::COLOR_ATTACHMENT | ImageUsage::SAMPLED,
                ..Default::default()
            },
            AllocationCreateInfo {
                memory_type_filter: MemoryTypeFilter::PREFER_DEVICE,
                ..Default::default()
            },
        )?;
        let persistent_1x1 = ImageView::new_default(persistent_1x1)?;

        let (level_images, level_framebuffers, level_sizes) =
            Self::create_levels(allocator, &render_pass, &persistent_1x1)?;

        let vs = lum_fullscreen_vs::load(device.clone())?
            .entry_point("main")
            .unwrap();
        let fs = lum_downsample_fs::load(device.clone())?
            .entry_point("main")
            .unwrap();
        let stages = [
            PipelineShaderStageCreateInfo::new(vs),
            PipelineShaderStageCreateInfo::new(fs),
        ];
        let layout = PipelineLayout::new(
            device.clone(),
            PipelineDescriptorSetLayoutCreateInfo::from_stages(&stages)
                .into_pipeline_layout_create_info(device.clone())?,
        )?;
        let subpass = vulkano::render_pass::Subpass::from(render_pass.clone(), 0).unwrap();
        let pipeline = GraphicsPipeline::new(
            device,
            None,
            GraphicsPipelineCreateInfo {
                stages: smallvec![stages[0].clone(), stages[1].clone()],
                vertex_input_state: Some(VertexInputState::default()),
                input_assembly_state: Some(InputAssemblyState::default()),
                viewport_state: Some(ViewportState::default()),
                rasterization_state: Some(RasterizationState::default()),
                multisample_state: Some(MultisampleState::default()),
                depth_stencil_state: None,
                color_blend_state: Some(ColorBlendState::with_attachment_states(
                    1,
                    ColorBlendAttachmentState::default(),
                )),
                dynamic_state: [
                    vulkano::pipeline::DynamicState::Viewport,
                    vulkano::pipeline::DynamicState::Scissor,
                ]
                .into_iter()
                .collect(),
                subpass: Some(subpass.into()),
                ..GraphicsPipelineCreateInfo::layout(layout.clone())
            },
        )?;

        Ok(Self {
            pipeline,
            layout,
            sampler,
            render_pass,
            level_images,
            level_framebuffers,
            level_sizes,
            persistent_1x1,
            level_sets: Vec::new(),
        })
    }

    /// Build (or rebuild) the cached descriptor sets for every luminance level.
    /// Call after construction and after each `resize()`. Level 0 reads from the
    /// external HDR target; higher levels read from internal chain images.
    pub fn prepare_sets(
        &mut self,
        descriptor_set_allocator: Arc<StandardDescriptorSetAllocator>,
        hdr_target: Arc<ImageView>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let set_layout = self
            .layout
            .set_layouts()
            .first()
            .ok_or("Missing Set 0 layout")?
            .clone();
        let level_count = self.level_images.len();
        self.level_sets = Vec::with_capacity(level_count);
        for i in 0..level_count {
            let source = if i == 0 {
                hdr_target.clone()
            } else {
                self.level_images[i - 1].clone()
            };
            let set = DescriptorSet::new(
                descriptor_set_allocator.clone(),
                set_layout.clone(),
                [WriteDescriptorSet::image_view_sampler(
                    0,
                    source,
                    self.sampler.clone(),
                )],
                [],
            )?;
            self.level_sets.push(set);
        }
        Ok(())
    }

    pub fn level_set(&self, idx: usize) -> Option<&Arc<DescriptorSet>> {
        self.level_sets.get(idx)
    }

    #[allow(clippy::type_complexity)]
    fn create_levels(
        allocator: Arc<StandardMemoryAllocator>,
        render_pass: &Arc<RenderPass>,
        persistent_1x1: &Arc<ImageView>,
    ) -> Result<
        (Vec<Arc<ImageView>>, Vec<Arc<Framebuffer>>, Vec<u32>),
        Box<dyn std::error::Error>,
    > {
        let sizes: Vec<u32> = (0..LUM_LEVELS)
            .map(|i| 256 >> i)
            .collect();

        let mut images = Vec::with_capacity(LUM_LEVELS);
        let mut framebuffers = Vec::with_capacity(LUM_LEVELS);

        for &size in sizes.iter() {
            if size == 1 {
                images.push(persistent_1x1.clone());
                let fb = Framebuffer::new(
                    render_pass.clone(),
                    FramebufferCreateInfo {
                        attachments: vec![persistent_1x1.clone()],
                        ..Default::default()
                    },
                )?;
                framebuffers.push(fb);
            } else {
                let image = Image::new(
                    allocator.clone(),
                    ImageCreateInfo {
                        image_type: ImageType::Dim2d,
                        format: Format::R16_SFLOAT,
                        extent: [size, size, 1],
                        usage: ImageUsage::COLOR_ATTACHMENT | ImageUsage::SAMPLED,
                        ..Default::default()
                    },
                    AllocationCreateInfo::default(),
                )?;
                let view = ImageView::new_default(image)?;
                let fb = Framebuffer::new(
                    render_pass.clone(),
                    FramebufferCreateInfo {
                        attachments: vec![view.clone()],
                        ..Default::default()
                    },
                )?;
                images.push(view);
                framebuffers.push(fb);
            }
        }

        Ok((images, framebuffers, sizes))
    }

    pub fn resize(
        &mut self,
        allocator: Arc<StandardMemoryAllocator>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (images, framebuffers, sizes) =
            Self::create_levels(allocator, &self.render_pass, &self.persistent_1x1)?;
        self.level_images = images;
        self.level_framebuffers = framebuffers;
        self.level_sizes = sizes;
        // Caller must call prepare_sets() after resize — clear stale references.
        self.level_sets.clear();
        Ok(())
    }

    pub fn create_descriptor_set(
        &self,
        descriptor_set_allocator: Arc<StandardDescriptorSetAllocator>,
        source: Arc<ImageView>,
    ) -> Result<Arc<DescriptorSet>, Box<dyn std::error::Error>> {
        let layout = self.layout.set_layouts().first().ok_or("Missing Set 0 layout")?;
        let set = DescriptorSet::new(
            descriptor_set_allocator,
            layout.clone(),
            [WriteDescriptorSet::image_view_sampler(0, source, self.sampler.clone())],
            [],
        )?;
        Ok(set)
    }

    pub fn pipeline(&self) -> Arc<GraphicsPipeline> {
        self.pipeline.clone()
    }

    pub fn layout(&self) -> &Arc<PipelineLayout> {
        &self.layout
    }

    pub fn level_count(&self) -> usize {
        LUM_LEVELS
    }

    pub fn level_images(&self) -> &[Arc<ImageView>] {
        &self.level_images
    }

    pub fn level_framebuffers(&self) -> &[Arc<Framebuffer>] {
        &self.level_framebuffers
    }

    pub fn level_sizes(&self) -> &[u32] {
        &self.level_sizes
    }

    pub fn persistent_1x1(&self) -> Arc<ImageView> {
        self.persistent_1x1.clone()
    }
}
