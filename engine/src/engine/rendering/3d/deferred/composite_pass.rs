use smallvec::smallvec;
use std::sync::Arc;
use vulkano::descriptor_set::allocator::StandardDescriptorSetAllocator;
use vulkano::descriptor_set::{DescriptorSet, WriteDescriptorSet};
use vulkano::device::Device;
use vulkano::image::sampler::{Filter, Sampler, SamplerAddressMode, SamplerCreateInfo};
use vulkano::image::view::ImageView;
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
use vulkano::render_pass::RenderPass;

pub mod composite_vs {
    vulkano_shaders::shader! {
        ty: "vertex",
        path: "src/engine/rendering/shaders/deferred/composite.vert",
    }
}

pub mod composite_fs {
    vulkano_shaders::shader! {
        ty: "fragment",
        path: "src/engine/rendering/shaders/deferred/composite.frag",
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct CompositePushConstants {
    pub exposure: f32,
    pub bloom_intensity: f32,
    pub vignette_intensity: f32,
    pub tone_map_mode: f32,
    pub exposure_mode: f32,
    pub _pad0: f32,
    pub _pad1: f32,
    pub _pad2: f32,
}

unsafe impl bytemuck::Pod for CompositePushConstants {}
unsafe impl bytemuck::Zeroable for CompositePushConstants {}

pub struct CompositePass {
    pipeline: Arc<GraphicsPipeline>,
    layout: Arc<PipelineLayout>,
    render_pass: Arc<RenderPass>,
    sampler: Arc<Sampler>,
}

impl CompositePass {
    pub fn new(
        device: Arc<Device>,
        render_pass: Arc<RenderPass>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let vs = composite_vs::load(device.clone())?
            .entry_point("main")
            .unwrap();
        let fs = composite_fs::load(device.clone())?
            .entry_point("main")
            .unwrap();

        let stages = [
            PipelineShaderStageCreateInfo::new(vs),
            PipelineShaderStageCreateInfo::new(fs),
        ];

        let sampler = Sampler::new(
            device.clone(),
            SamplerCreateInfo {
                mag_filter: Filter::Linear,
                min_filter: Filter::Linear,
                address_mode: [SamplerAddressMode::ClampToEdge; 3],
                ..Default::default()
            },
        )?;

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
                    subpass.num_color_attachments(),
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
            render_pass,
            sampler,
        })
    }

    pub fn create_descriptor_set(
        &self,
        descriptor_set_allocator: Arc<StandardDescriptorSetAllocator>,
        hdr_target: Arc<ImageView>,
        bloom_texture: Arc<ImageView>,
        luminance_texture: Arc<ImageView>,
    ) -> Result<Arc<DescriptorSet>, Box<dyn std::error::Error>> {
        let layout = self.layout.set_layouts().first().ok_or("Missing Set 0 layout")?;
        let set = DescriptorSet::new(
            descriptor_set_allocator,
            layout.clone(),
            [
                WriteDescriptorSet::image_view_sampler(0, hdr_target, self.sampler.clone()),
                WriteDescriptorSet::image_view_sampler(1, bloom_texture, self.sampler.clone()),
                WriteDescriptorSet::image_view_sampler(2, luminance_texture, self.sampler.clone()),
            ],
            [],
        )?;
        Ok(set)
    }

    pub fn pipeline(&self) -> Arc<GraphicsPipeline> {
        self.pipeline.clone()
    }

    pub fn layout(&self) -> Arc<PipelineLayout> {
        self.layout.clone()
    }

    pub fn render_pass(&self) -> Arc<RenderPass> {
        self.render_pass.clone()
    }
}
