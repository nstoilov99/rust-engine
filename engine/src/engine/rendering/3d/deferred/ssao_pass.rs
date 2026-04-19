use smallvec::smallvec;
use std::sync::Arc;
use vulkano::buffer::{Buffer, BufferCreateInfo, BufferUsage, Subbuffer};
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

pub mod ssao_fullscreen_vs {
    vulkano_shaders::shader! {
        ty: "vertex",
        path: "src/engine/rendering/shaders/deferred/composite.vert",
    }
}

pub mod ssao_fs {
    vulkano_shaders::shader! {
        ty: "fragment",
        path: "src/engine/rendering/shaders/deferred/ssao.frag",
    }
}

pub mod ssao_blur_fs {
    vulkano_shaders::shader! {
        ty: "fragment",
        path: "src/engine/rendering/shaders/deferred/ssao_blur.frag",
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SsaoPushConstants {
    pub view_projection: [[f32; 4]; 4],
    pub screen_size: [f32; 2],
    pub radius: f32,
    pub bias: f32,
}

unsafe impl bytemuck::Pod for SsaoPushConstants {}
unsafe impl bytemuck::Zeroable for SsaoPushConstants {}

pub struct SsaoPass {
    ssao_pipeline: Arc<GraphicsPipeline>,
    ssao_layout: Arc<PipelineLayout>,
    blur_pipeline: Arc<GraphicsPipeline>,
    blur_layout: Arc<PipelineLayout>,
    ssao_render_pass: Arc<RenderPass>,
    blur_render_pass: Arc<RenderPass>,
    sampler: Arc<Sampler>,
    kernel_buffer: Subbuffer<[[f32; 4]; 64]>,
    noise_texture: Arc<ImageView>,
    ssao_raw: Arc<ImageView>,
    ssao_blurred: Arc<ImageView>,
    ssao_raw_framebuffer: Arc<Framebuffer>,
    ssao_blur_framebuffer: Arc<Framebuffer>,
    // Cached descriptor sets — populated by prepare_sets() after init/resize.
    ssao_gbuffer_set: Option<Arc<DescriptorSet>>,
    ssao_kernel_set: Option<Arc<DescriptorSet>>,
    blur_set: Option<Arc<DescriptorSet>>,
}

impl SsaoPass {
    pub fn new(
        device: Arc<Device>,
        allocator: Arc<StandardMemoryAllocator>,
        _descriptor_set_allocator: Arc<StandardDescriptorSetAllocator>,
        width: u32,
        height: u32,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let sampler = Sampler::new(
            device.clone(),
            SamplerCreateInfo {
                mag_filter: Filter::Nearest,
                min_filter: Filter::Nearest,
                address_mode: [SamplerAddressMode::Repeat; 3],
                ..Default::default()
            },
        )?;

        let kernel_data = Self::generate_kernel();
        let kernel_buffer = Buffer::from_data(
            allocator.clone(),
            BufferCreateInfo {
                usage: BufferUsage::UNIFORM_BUFFER,
                ..Default::default()
            },
            AllocationCreateInfo {
                memory_type_filter: MemoryTypeFilter::PREFER_DEVICE
                    | MemoryTypeFilter::HOST_SEQUENTIAL_WRITE,
                ..Default::default()
            },
            kernel_data,
        )?;

        let noise_texture = Self::create_noise_texture(allocator.clone())?;

        let ssao_render_pass = vulkano::single_pass_renderpass!(
            device.clone(),
            attachments: {
                color: {
                    format: Format::R8_UNORM,
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

        let blur_render_pass = vulkano::single_pass_renderpass!(
            device.clone(),
            attachments: {
                color: {
                    format: Format::R8_UNORM,
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

        let (ssao_raw, ssao_blurred, ssao_raw_framebuffer, ssao_blur_framebuffer) =
            Self::create_targets(
                allocator.clone(),
                &ssao_render_pass,
                &blur_render_pass,
                width,
                height,
            )?;

        let ssao_pipeline;
        let ssao_layout;
        {
            let vs = ssao_fullscreen_vs::load(device.clone())?
                .entry_point("main")
                .unwrap();
            let fs = ssao_fs::load(device.clone())?
                .entry_point("main")
                .unwrap();
            let stages = [
                PipelineShaderStageCreateInfo::new(vs),
                PipelineShaderStageCreateInfo::new(fs),
            ];
            ssao_layout = PipelineLayout::new(
                device.clone(),
                PipelineDescriptorSetLayoutCreateInfo::from_stages(&stages)
                    .into_pipeline_layout_create_info(device.clone())?,
            )?;
            let subpass =
                vulkano::render_pass::Subpass::from(ssao_render_pass.clone(), 0).unwrap();
            ssao_pipeline = GraphicsPipeline::new(
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
                    ..GraphicsPipelineCreateInfo::layout(ssao_layout.clone())
                },
            )?;
        }

        let blur_pipeline;
        let blur_layout;
        {
            let vs = ssao_fullscreen_vs::load(device.clone())?
                .entry_point("main")
                .unwrap();
            let fs = ssao_blur_fs::load(device.clone())?
                .entry_point("main")
                .unwrap();
            let stages = [
                PipelineShaderStageCreateInfo::new(vs),
                PipelineShaderStageCreateInfo::new(fs),
            ];
            blur_layout = PipelineLayout::new(
                device.clone(),
                PipelineDescriptorSetLayoutCreateInfo::from_stages(&stages)
                    .into_pipeline_layout_create_info(device.clone())?,
            )?;
            let subpass =
                vulkano::render_pass::Subpass::from(blur_render_pass.clone(), 0).unwrap();
            blur_pipeline = GraphicsPipeline::new(
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
                    ..GraphicsPipelineCreateInfo::layout(blur_layout.clone())
                },
            )?;
        }

        Ok(Self {
            ssao_pipeline,
            ssao_layout,
            blur_pipeline,
            blur_layout,
            ssao_render_pass,
            blur_render_pass,
            sampler,
            kernel_buffer,
            noise_texture,
            ssao_raw,
            ssao_blurred,
            ssao_raw_framebuffer,
            ssao_blur_framebuffer,
            ssao_gbuffer_set: None,
            ssao_kernel_set: None,
            blur_set: None,
        })
    }

    /// Build (or rebuild) the cached descriptor sets for SSAO dispatches.
    /// Call after construction and after each `resize()`. The gbuffer set
    /// references external position/normal textures; kernel+blur sets reference
    /// internal buffers/images.
    pub fn prepare_sets(
        &mut self,
        descriptor_set_allocator: Arc<StandardDescriptorSetAllocator>,
        gbuffer_position: Arc<ImageView>,
        gbuffer_normal: Arc<ImageView>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let ssao_set_0 = self
            .ssao_layout
            .set_layouts()
            .first()
            .ok_or("Missing SSAO Set 0")?
            .clone();
        self.ssao_gbuffer_set = Some(DescriptorSet::new(
            descriptor_set_allocator.clone(),
            ssao_set_0,
            [
                WriteDescriptorSet::image_view_sampler(0, gbuffer_position, self.sampler.clone()),
                WriteDescriptorSet::image_view_sampler(1, gbuffer_normal, self.sampler.clone()),
                WriteDescriptorSet::image_view_sampler(
                    2,
                    self.noise_texture.clone(),
                    self.sampler.clone(),
                ),
            ],
            [],
        )?);

        let ssao_set_1 = self
            .ssao_layout
            .set_layouts()
            .get(1)
            .ok_or("Missing SSAO Set 1")?
            .clone();
        self.ssao_kernel_set = Some(DescriptorSet::new(
            descriptor_set_allocator.clone(),
            ssao_set_1,
            [WriteDescriptorSet::buffer(0, self.kernel_buffer.clone())],
            [],
        )?);

        let blur_set_0 = self
            .blur_layout
            .set_layouts()
            .first()
            .ok_or("Missing blur Set 0")?
            .clone();
        self.blur_set = Some(DescriptorSet::new(
            descriptor_set_allocator,
            blur_set_0,
            [WriteDescriptorSet::image_view_sampler(
                0,
                self.ssao_raw.clone(),
                self.sampler.clone(),
            )],
            [],
        )?);

        Ok(())
    }

    pub fn ssao_gbuffer_set(&self) -> Option<&Arc<DescriptorSet>> {
        self.ssao_gbuffer_set.as_ref()
    }

    pub fn ssao_kernel_set(&self) -> Option<&Arc<DescriptorSet>> {
        self.ssao_kernel_set.as_ref()
    }

    pub fn blur_set(&self) -> Option<&Arc<DescriptorSet>> {
        self.blur_set.as_ref()
    }

    fn generate_kernel() -> [[f32; 4]; 64] {
        let mut kernel = [[0.0f32; 4]; 64];
        for (i, sample) in kernel.iter_mut().enumerate() {
            let t = i as f32 / 64.0;
            let phi = t * std::f32::consts::PI * 2.0 * 7.0;
            let cos_theta = 1.0 - t;
            let sin_theta = (1.0 - cos_theta * cos_theta).sqrt();

            let mut x = sin_theta * phi.cos();
            let mut y = sin_theta * phi.sin();
            let mut z = cos_theta.abs();

            let scale_factor = i as f32 / 64.0;
            let scale = 0.1 + 0.9 * scale_factor * scale_factor;
            x *= scale;
            y *= scale;
            z *= scale;

            *sample = [x, y, z, 0.0];
        }
        kernel
    }

    fn create_noise_texture(
        allocator: Arc<StandardMemoryAllocator>,
    ) -> Result<Arc<ImageView>, Box<dyn std::error::Error>> {
        let image = Image::new(
            allocator,
            ImageCreateInfo {
                image_type: ImageType::Dim2d,
                format: Format::R8G8_UNORM,
                extent: [4, 4, 1],
                usage: ImageUsage::SAMPLED | ImageUsage::TRANSFER_DST,
                ..Default::default()
            },
            AllocationCreateInfo {
                memory_type_filter: MemoryTypeFilter::PREFER_DEVICE,
                ..Default::default()
            },
        )?;

        ImageView::new_default(image).map_err(|e| e.into())
    }

    #[allow(clippy::type_complexity)]
    fn create_targets(
        allocator: Arc<StandardMemoryAllocator>,
        ssao_render_pass: &Arc<RenderPass>,
        blur_render_pass: &Arc<RenderPass>,
        width: u32,
        height: u32,
    ) -> Result<
        (
            Arc<ImageView>,
            Arc<ImageView>,
            Arc<Framebuffer>,
            Arc<Framebuffer>,
        ),
        Box<dyn std::error::Error>,
    > {
        let ssao_raw_image = Image::new(
            allocator.clone(),
            ImageCreateInfo {
                image_type: ImageType::Dim2d,
                format: Format::R8_UNORM,
                extent: [width, height, 1],
                usage: ImageUsage::COLOR_ATTACHMENT | ImageUsage::SAMPLED,
                ..Default::default()
            },
            AllocationCreateInfo::default(),
        )?;
        let ssao_raw = ImageView::new_default(ssao_raw_image)?;

        let ssao_blurred_image = Image::new(
            allocator,
            ImageCreateInfo {
                image_type: ImageType::Dim2d,
                format: Format::R8_UNORM,
                extent: [width, height, 1],
                usage: ImageUsage::COLOR_ATTACHMENT | ImageUsage::SAMPLED,
                ..Default::default()
            },
            AllocationCreateInfo::default(),
        )?;
        let ssao_blurred = ImageView::new_default(ssao_blurred_image)?;

        let ssao_raw_fb = Framebuffer::new(
            ssao_render_pass.clone(),
            FramebufferCreateInfo {
                attachments: vec![ssao_raw.clone()],
                ..Default::default()
            },
        )?;

        let ssao_blur_fb = Framebuffer::new(
            blur_render_pass.clone(),
            FramebufferCreateInfo {
                attachments: vec![ssao_blurred.clone()],
                ..Default::default()
            },
        )?;

        Ok((ssao_raw, ssao_blurred, ssao_raw_fb, ssao_blur_fb))
    }

    pub fn resize(
        &mut self,
        allocator: Arc<StandardMemoryAllocator>,
        width: u32,
        height: u32,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (raw, blurred, raw_fb, blur_fb) = Self::create_targets(
            allocator,
            &self.ssao_render_pass,
            &self.blur_render_pass,
            width,
            height,
        )?;
        self.ssao_raw = raw;
        self.ssao_blurred = blurred;
        self.ssao_raw_framebuffer = raw_fb;
        self.ssao_blur_framebuffer = blur_fb;
        // Caller must call prepare_sets() after resize — clear stale references.
        self.ssao_gbuffer_set = None;
        self.ssao_kernel_set = None;
        self.blur_set = None;
        Ok(())
    }

    pub fn ssao_blurred(&self) -> Arc<ImageView> {
        self.ssao_blurred.clone()
    }

    pub fn ssao_pipeline(&self) -> Arc<GraphicsPipeline> {
        self.ssao_pipeline.clone()
    }

    pub fn ssao_layout(&self) -> &Arc<PipelineLayout> {
        &self.ssao_layout
    }

    pub fn blur_pipeline(&self) -> Arc<GraphicsPipeline> {
        self.blur_pipeline.clone()
    }

    pub fn blur_layout(&self) -> &Arc<PipelineLayout> {
        &self.blur_layout
    }

    pub fn sampler(&self) -> Arc<Sampler> {
        self.sampler.clone()
    }

    pub fn kernel_buffer(&self) -> &Subbuffer<[[f32; 4]; 64]> {
        &self.kernel_buffer
    }

    pub fn noise_texture(&self) -> Arc<ImageView> {
        self.noise_texture.clone()
    }

    pub fn ssao_raw_framebuffer(&self) -> Arc<Framebuffer> {
        self.ssao_raw_framebuffer.clone()
    }

    pub fn ssao_blur_framebuffer(&self) -> Arc<Framebuffer> {
        self.ssao_blur_framebuffer.clone()
    }

    pub fn ssao_raw(&self) -> Arc<ImageView> {
        self.ssao_raw.clone()
    }
}
