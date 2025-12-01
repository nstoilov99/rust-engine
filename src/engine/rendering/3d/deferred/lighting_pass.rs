//! Lighting pass - reads G-Buffer and calculates lighting

use std::sync::Arc;
use smallvec::smallvec;
use vulkano::descriptor_set::{PersistentDescriptorSet, WriteDescriptorSet};
use vulkano::descriptor_set::allocator::StandardDescriptorSetAllocator;
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
use vulkano::pipeline::{GraphicsPipeline, PipelineShaderStageCreateInfo, PipelineLayout};
use vulkano::pipeline::layout::PipelineDescriptorSetLayoutCreateInfo;
use vulkano::render_pass::RenderPass;

// Lighting shaders
pub mod lighting_vs {
    vulkano_shaders::shader! {
        ty: "vertex",
        path: "src/engine/rendering/shaders/deferred/lighting.vert",
    }
}

pub mod lighting_fs {
    vulkano_shaders::shader! {
        ty: "fragment",
        path: "src/engine/rendering/shaders/deferred/lighting.frag",
    }
}

/// Lighting pass pipeline (fullscreen quad, reads G-Buffer)
pub struct LightingPass {
    pipeline: Arc<GraphicsPipeline>,
    sampler: Arc<Sampler>,
    layout: Arc<PipelineLayout>,
    render_pass: Arc<RenderPass>,
}

impl LightingPass {
    pub fn new(
        device: Arc<Device>,
        render_pass: Arc<RenderPass>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        // Load shaders
        let vs = lighting_vs::load(device.clone())?.entry_point("main").unwrap();
        let fs = lighting_fs::load(device.clone())?.entry_point("main").unwrap();

        let stages = [
            PipelineShaderStageCreateInfo::new(vs),
            PipelineShaderStageCreateInfo::new(fs),
        ];

        // Create sampler for G-Buffer textures
        let sampler = Sampler::new(
            device.clone(),
            SamplerCreateInfo {
                mag_filter: Filter::Nearest,
                min_filter: Filter::Nearest,
                address_mode: [SamplerAddressMode::ClampToEdge; 3],
                ..Default::default()
            },
        )?;

        // Pipeline layout
        let layout = PipelineLayout::new(
            device.clone(),
            PipelineDescriptorSetLayoutCreateInfo::from_stages(&stages)
                .into_pipeline_layout_create_info(device.clone())?,
        )?;

        // Create pipeline (no vertex buffers - fullscreen triangle generated in shader)
        let subpass = vulkano::render_pass::Subpass::from(render_pass.clone(), 0).unwrap();

        let pipeline = GraphicsPipeline::new(
            device.clone(),
            None,
            GraphicsPipelineCreateInfo {
                stages: smallvec![stages[0].clone(), stages[1].clone()],
                vertex_input_state: Some(VertexInputState::default()), // No vertex input
                input_assembly_state: Some(InputAssemblyState::default()),
                viewport_state: Some(ViewportState::default()),
                rasterization_state: Some(RasterizationState::default()),
                multisample_state: Some(MultisampleState::default()),
                depth_stencil_state: None, // No depth test
                color_blend_state: Some(ColorBlendState::with_attachment_states(
                    subpass.num_color_attachments(),
                    ColorBlendAttachmentState::default(),
                )),
                dynamic_state: [
                    vulkano::pipeline::DynamicState::Viewport,
                    vulkano::pipeline::DynamicState::Scissor,
                ].into_iter().collect(),
                subpass: Some(subpass.into()),
                ..GraphicsPipelineCreateInfo::layout(layout.clone())
            },
        )?;

        Ok(Self { pipeline, sampler, layout, render_pass })
    }

    /// Create descriptor set for G-Buffer sampling
    pub fn create_descriptor_set(
        &self,
        descriptor_set_allocator: &StandardDescriptorSetAllocator,
        position: Arc<ImageView>,
        normal: Arc<ImageView>,
        albedo: Arc<ImageView>,
        material: Arc<ImageView>,
    ) -> Result<Arc<PersistentDescriptorSet>, Box<dyn std::error::Error>> {
        let layout = self.layout.set_layouts().get(0).unwrap();

        let set = PersistentDescriptorSet::new(
            descriptor_set_allocator,
            layout.clone(),
            [
                WriteDescriptorSet::image_view_sampler(0, position, self.sampler.clone()),
                WriteDescriptorSet::image_view_sampler(1, normal, self.sampler.clone()),
                WriteDescriptorSet::image_view_sampler(2, albedo, self.sampler.clone()),
                WriteDescriptorSet::image_view_sampler(3, material, self.sampler.clone()),
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
