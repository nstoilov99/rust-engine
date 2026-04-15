use smallvec::smallvec;
use std::sync::Arc;
use vulkano::descriptor_set::allocator::StandardDescriptorSetAllocator;
use vulkano::descriptor_set::{DescriptorSet, WriteDescriptorSet};
use vulkano::device::Device;
use vulkano::format::Format;
use vulkano::image::sampler::{Filter, Sampler, SamplerAddressMode, SamplerCreateInfo};
use vulkano::image::view::ImageView;
use vulkano::image::{Image, ImageCreateInfo, ImageType, ImageUsage};
use vulkano::memory::allocator::{AllocationCreateInfo, StandardMemoryAllocator};
use vulkano::pipeline::graphics::{
    color_blend::{
        AttachmentBlend, BlendFactor, BlendOp, ColorBlendAttachmentState, ColorBlendState,
    },
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

pub mod bloom_fullscreen_vs {
    vulkano_shaders::shader! {
        ty: "vertex",
        path: "src/engine/rendering/shaders/deferred/composite.vert",
    }
}

pub mod bloom_threshold_fs {
    vulkano_shaders::shader! {
        ty: "fragment",
        path: "src/engine/rendering/shaders/deferred/bloom_threshold.frag",
    }
}

pub mod bloom_downsample_fs {
    vulkano_shaders::shader! {
        ty: "fragment",
        path: "src/engine/rendering/shaders/deferred/bloom_downsample.frag",
    }
}

pub mod bloom_upsample_fs {
    vulkano_shaders::shader! {
        ty: "fragment",
        path: "src/engine/rendering/shaders/deferred/bloom_upsample.frag",
    }
}

const MIP_COUNT: usize = 6;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct BloomThresholdPush {
    pub threshold: f32,
    pub _pad0: f32,
    pub _pad1: f32,
    pub _pad2: f32,
}

unsafe impl bytemuck::Pod for BloomThresholdPush {}
unsafe impl bytemuck::Zeroable for BloomThresholdPush {}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct BloomSamplePush {
    pub texel_size: [f32; 2],
    pub is_first_pass: f32,
    pub _pad: f32,
}

unsafe impl bytemuck::Pod for BloomSamplePush {}
unsafe impl bytemuck::Zeroable for BloomSamplePush {}

pub struct BloomPass {
    threshold_pipeline: Arc<GraphicsPipeline>,
    threshold_layout: Arc<PipelineLayout>,
    downsample_pipeline: Arc<GraphicsPipeline>,
    downsample_layout: Arc<PipelineLayout>,
    upsample_pipeline: Arc<GraphicsPipeline>,
    upsample_layout: Arc<PipelineLayout>,
    sampler: Arc<Sampler>,
    mip_render_pass: Arc<RenderPass>,
    additive_render_pass: Arc<RenderPass>,
    mip_images: Vec<Arc<ImageView>>,
    mip_framebuffers: Vec<Arc<Framebuffer>>,
    additive_framebuffers: Vec<Arc<Framebuffer>>,
    mip_sizes: Vec<[u32; 2]>,
}

impl BloomPass {
    pub fn new(
        device: Arc<Device>,
        allocator: Arc<StandardMemoryAllocator>,
        width: u32,
        height: u32,
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

        let mip_render_pass = vulkano::single_pass_renderpass!(
            device.clone(),
            attachments: {
                color: {
                    format: Format::R16G16B16A16_SFLOAT,
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

        let additive_render_pass = vulkano::single_pass_renderpass!(
            device.clone(),
            attachments: {
                color: {
                    format: Format::R16G16B16A16_SFLOAT,
                    samples: 1,
                    load_op: Load,
                    store_op: Store,
                }
            },
            pass: {
                color: [color],
                depth_stencil: {}
            }
        )?;

        let (mip_images, mip_framebuffers, additive_framebuffers, mip_sizes) =
            Self::create_mip_chain(
                allocator.clone(),
                &mip_render_pass,
                &additive_render_pass,
                width,
                height,
            )?;

        let threshold_pipeline;
        let threshold_layout;
        {
            let vs = bloom_fullscreen_vs::load(device.clone())?
                .entry_point("main")
                .unwrap();
            let fs = bloom_threshold_fs::load(device.clone())?
                .entry_point("main")
                .unwrap();
            let stages = [
                PipelineShaderStageCreateInfo::new(vs),
                PipelineShaderStageCreateInfo::new(fs),
            ];
            threshold_layout = PipelineLayout::new(
                device.clone(),
                PipelineDescriptorSetLayoutCreateInfo::from_stages(&stages)
                    .into_pipeline_layout_create_info(device.clone())?,
            )?;
            let subpass =
                vulkano::render_pass::Subpass::from(mip_render_pass.clone(), 0).unwrap();
            threshold_pipeline = GraphicsPipeline::new(
                device.clone(),
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
                    ..GraphicsPipelineCreateInfo::layout(threshold_layout.clone())
                },
            )?;
        }

        let downsample_pipeline;
        let downsample_layout;
        {
            let vs = bloom_fullscreen_vs::load(device.clone())?
                .entry_point("main")
                .unwrap();
            let fs = bloom_downsample_fs::load(device.clone())?
                .entry_point("main")
                .unwrap();
            let stages = [
                PipelineShaderStageCreateInfo::new(vs),
                PipelineShaderStageCreateInfo::new(fs),
            ];
            downsample_layout = PipelineLayout::new(
                device.clone(),
                PipelineDescriptorSetLayoutCreateInfo::from_stages(&stages)
                    .into_pipeline_layout_create_info(device.clone())?,
            )?;
            let subpass =
                vulkano::render_pass::Subpass::from(mip_render_pass.clone(), 0).unwrap();
            downsample_pipeline = GraphicsPipeline::new(
                device.clone(),
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
                    ..GraphicsPipelineCreateInfo::layout(downsample_layout.clone())
                },
            )?;
        }

        let upsample_pipeline;
        let upsample_layout;
        {
            let vs = bloom_fullscreen_vs::load(device.clone())?
                .entry_point("main")
                .unwrap();
            let fs = bloom_upsample_fs::load(device.clone())?
                .entry_point("main")
                .unwrap();
            let stages = [
                PipelineShaderStageCreateInfo::new(vs),
                PipelineShaderStageCreateInfo::new(fs),
            ];
            upsample_layout = PipelineLayout::new(
                device.clone(),
                PipelineDescriptorSetLayoutCreateInfo::from_stages(&stages)
                    .into_pipeline_layout_create_info(device.clone())?,
            )?;
            let subpass =
                vulkano::render_pass::Subpass::from(additive_render_pass.clone(), 0).unwrap();
            upsample_pipeline = GraphicsPipeline::new(
                device.clone(),
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
                        ColorBlendAttachmentState {
                            blend: Some(AttachmentBlend {
                                src_color_blend_factor: BlendFactor::One,
                                dst_color_blend_factor: BlendFactor::One,
                                color_blend_op: BlendOp::Add,
                                src_alpha_blend_factor: BlendFactor::One,
                                dst_alpha_blend_factor: BlendFactor::One,
                                alpha_blend_op: BlendOp::Add,
                            }),
                            ..Default::default()
                        },
                    )),
                    dynamic_state: [
                        vulkano::pipeline::DynamicState::Viewport,
                        vulkano::pipeline::DynamicState::Scissor,
                    ]
                    .into_iter()
                    .collect(),
                    subpass: Some(subpass.into()),
                    ..GraphicsPipelineCreateInfo::layout(upsample_layout.clone())
                },
            )?;
        }

        Ok(Self {
            threshold_pipeline,
            threshold_layout,
            downsample_pipeline,
            downsample_layout,
            upsample_pipeline,
            upsample_layout,
            sampler,
            mip_render_pass,
            additive_render_pass,
            mip_images,
            mip_framebuffers,
            additive_framebuffers,
            mip_sizes,
        })
    }

    #[allow(clippy::type_complexity)]
    fn create_mip_chain(
        allocator: Arc<StandardMemoryAllocator>,
        mip_render_pass: &Arc<RenderPass>,
        additive_render_pass: &Arc<RenderPass>,
        width: u32,
        height: u32,
    ) -> Result<
        (
            Vec<Arc<ImageView>>,
            Vec<Arc<Framebuffer>>,
            Vec<Arc<Framebuffer>>,
            Vec<[u32; 2]>,
        ),
        Box<dyn std::error::Error>,
    > {
        let mut images = Vec::with_capacity(MIP_COUNT);
        let mut framebuffers = Vec::with_capacity(MIP_COUNT);
        let mut additive_fbs = Vec::with_capacity(MIP_COUNT);
        let mut sizes = Vec::with_capacity(MIP_COUNT);

        let mut w = (width / 2).max(1);
        let mut h = (height / 2).max(1);

        for _ in 0..MIP_COUNT {
            let image = Image::new(
                allocator.clone(),
                ImageCreateInfo {
                    image_type: ImageType::Dim2d,
                    format: Format::R16G16B16A16_SFLOAT,
                    extent: [w, h, 1],
                    usage: ImageUsage::COLOR_ATTACHMENT | ImageUsage::SAMPLED,
                    ..Default::default()
                },
                AllocationCreateInfo::default(),
            )?;
            let view = ImageView::new_default(image)?;

            let fb = Framebuffer::new(
                mip_render_pass.clone(),
                FramebufferCreateInfo {
                    attachments: vec![view.clone()],
                    ..Default::default()
                },
            )?;

            let additive_fb = Framebuffer::new(
                additive_render_pass.clone(),
                FramebufferCreateInfo {
                    attachments: vec![view.clone()],
                    ..Default::default()
                },
            )?;

            images.push(view);
            framebuffers.push(fb);
            additive_fbs.push(additive_fb);
            sizes.push([w, h]);

            w = (w / 2).max(1);
            h = (h / 2).max(1);
        }

        Ok((images, framebuffers, additive_fbs, sizes))
    }

    pub fn resize(
        &mut self,
        allocator: Arc<StandardMemoryAllocator>,
        width: u32,
        height: u32,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (images, framebuffers, additive_fbs, sizes) = Self::create_mip_chain(
            allocator,
            &self.mip_render_pass,
            &self.additive_render_pass,
            width,
            height,
        )?;
        self.mip_images = images;
        self.mip_framebuffers = framebuffers;
        self.additive_framebuffers = additive_fbs;
        self.mip_sizes = sizes;
        Ok(())
    }

    pub fn create_descriptor_set(
        &self,
        descriptor_set_allocator: Arc<StandardDescriptorSetAllocator>,
        layout: &Arc<PipelineLayout>,
        source: Arc<ImageView>,
    ) -> Result<Arc<DescriptorSet>, Box<dyn std::error::Error>> {
        let set_layout = layout.set_layouts().first().ok_or("Missing Set 0 layout")?;
        let set = DescriptorSet::new(
            descriptor_set_allocator,
            set_layout.clone(),
            [WriteDescriptorSet::image_view_sampler(
                0,
                source,
                self.sampler.clone(),
            )],
            [],
        )?;
        Ok(set)
    }

    pub fn bloom_result(&self) -> Arc<ImageView> {
        self.mip_images[0].clone()
    }

    pub fn threshold_pipeline(&self) -> Arc<GraphicsPipeline> {
        self.threshold_pipeline.clone()
    }

    pub fn threshold_layout(&self) -> &Arc<PipelineLayout> {
        &self.threshold_layout
    }

    pub fn downsample_pipeline(&self) -> Arc<GraphicsPipeline> {
        self.downsample_pipeline.clone()
    }

    pub fn downsample_layout(&self) -> &Arc<PipelineLayout> {
        &self.downsample_layout
    }

    pub fn upsample_pipeline(&self) -> Arc<GraphicsPipeline> {
        self.upsample_pipeline.clone()
    }

    pub fn upsample_layout(&self) -> &Arc<PipelineLayout> {
        &self.upsample_layout
    }

    pub fn mip_images(&self) -> &[Arc<ImageView>] {
        &self.mip_images
    }

    pub fn mip_framebuffers(&self) -> &[Arc<Framebuffer>] {
        &self.mip_framebuffers
    }

    pub fn additive_framebuffers(&self) -> &[Arc<Framebuffer>] {
        &self.additive_framebuffers
    }

    pub fn mip_sizes(&self) -> &[[u32; 2]] {
        &self.mip_sizes
    }

    pub fn mip_count(&self) -> usize {
        MIP_COUNT
    }
}
