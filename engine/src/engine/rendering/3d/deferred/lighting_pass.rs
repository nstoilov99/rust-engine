//! Lighting pass - reads G-Buffer and calculates lighting

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

use crate::engine::rendering::pipeline_registry::{PipelineId, PipelineRegistry};

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
    pipeline_id: PipelineId,
    sampler: Arc<Sampler>,
    layout: Arc<PipelineLayout>,
    render_pass: Arc<RenderPass>,
}

impl LightingPass {
    pub fn new(
        device: Arc<Device>,
        render_pass: Arc<RenderPass>,
    ) -> Result<(Self, Arc<GraphicsPipeline>), Box<dyn std::error::Error>> {
        let (pipeline, layout) = Self::create_pipeline(device.clone(), render_pass.clone())?;

        // Create sampler for G-Buffer textures
        let sampler = Sampler::new(
            device,
            SamplerCreateInfo {
                mag_filter: Filter::Nearest,
                min_filter: Filter::Nearest,
                address_mode: [SamplerAddressMode::ClampToEdge; 3],
                ..Default::default()
            },
        )?;

        Ok((
            Self {
                pipeline_id: PipelineId::Lighting,
                sampler,
                layout,
                render_pass,
            },
            pipeline,
        ))
    }

    /// Create the lighting pipeline from compile-time shaders.
    fn create_pipeline(
        device: Arc<Device>,
        render_pass: Arc<RenderPass>,
    ) -> Result<(Arc<GraphicsPipeline>, Arc<PipelineLayout>), Box<dyn std::error::Error>> {
        let vs = lighting_vs::load(device.clone())?
            .entry_point("main")
            .unwrap();
        let fs = lighting_fs::load(device.clone())?
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

        Ok((pipeline, layout))
    }

    /// Create a lighting pipeline from runtime-compiled SPIR-V (for hot-reload).
    #[cfg(feature = "editor")]
    pub fn create_pipeline_from_spirv(
        device: Arc<Device>,
        render_pass: Arc<RenderPass>,
        vs_spirv: &[u32],
        fs_spirv: &[u32],
    ) -> Result<Arc<GraphicsPipeline>, Box<dyn std::error::Error>> {
        use vulkano::shader::ShaderModule;

        let vs_module = unsafe {
            ShaderModule::new(device.clone(), vulkano::shader::ShaderModuleCreateInfo::new(vs_spirv))?
        };
        let fs_module = unsafe {
            ShaderModule::new(device.clone(), vulkano::shader::ShaderModuleCreateInfo::new(fs_spirv))?
        };

        let vs = vs_module.entry_point("main").ok_or("Missing vertex entry point 'main'")?;
        let fs = fs_module.entry_point("main").ok_or("Missing fragment entry point 'main'")?;

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
                ..GraphicsPipelineCreateInfo::layout(layout)
            },
        )?;

        Ok(pipeline)
    }

    /// Create descriptor set for G-Buffer sampling
    pub fn create_descriptor_set(
        &self,
        descriptor_set_allocator: Arc<StandardDescriptorSetAllocator>,
        position: Arc<ImageView>,
        normal: Arc<ImageView>,
        albedo: Arc<ImageView>,
        material: Arc<ImageView>,
        emissive: Arc<ImageView>,
    ) -> Result<Arc<DescriptorSet>, Box<dyn std::error::Error>> {
        let layout = self.layout.set_layouts().first().unwrap();

        let set = DescriptorSet::new(
            descriptor_set_allocator,
            layout.clone(),
            [
                WriteDescriptorSet::image_view_sampler(0, position, self.sampler.clone()),
                WriteDescriptorSet::image_view_sampler(1, normal, self.sampler.clone()),
                WriteDescriptorSet::image_view_sampler(2, albedo, self.sampler.clone()),
                WriteDescriptorSet::image_view_sampler(3, material, self.sampler.clone()),
                WriteDescriptorSet::image_view_sampler(4, emissive, self.sampler.clone()),
            ],
            [],
        )?;

        Ok(set)
    }

    pub fn create_shadow_descriptor_set(
        &self,
        descriptor_set_allocator: Arc<StandardDescriptorSetAllocator>,
        shadow_map: Arc<ImageView>,
        shadow_sampler: Arc<Sampler>,
    ) -> Result<Arc<DescriptorSet>, Box<dyn std::error::Error>> {
        let layout = self.layout.set_layouts().get(1).ok_or("Missing Set 1 layout")?;
        let set = DescriptorSet::new(
            descriptor_set_allocator,
            layout.clone(),
            [WriteDescriptorSet::image_view_sampler(0, shadow_map, shadow_sampler)],
            [],
        )?;
        Ok(set)
    }

    pub fn create_ssao_descriptor_set(
        &self,
        descriptor_set_allocator: Arc<StandardDescriptorSetAllocator>,
        ssao_texture: Arc<ImageView>,
        ssao_sampler: Arc<Sampler>,
    ) -> Result<Arc<DescriptorSet>, Box<dyn std::error::Error>> {
        let layout = self.layout.set_layouts().get(2).ok_or("Missing Set 2 layout")?;
        let set = DescriptorSet::new(
            descriptor_set_allocator,
            layout.clone(),
            [WriteDescriptorSet::image_view_sampler(0, ssao_texture, ssao_sampler)],
            [],
        )?;
        Ok(set)
    }

    pub fn pipeline(&self, registry: &PipelineRegistry) -> Arc<GraphicsPipeline> {
        registry.get(self.pipeline_id)
    }

    pub fn layout(&self) -> Arc<PipelineLayout> {
        self.layout.clone()
    }

    pub fn render_pass(&self) -> Arc<RenderPass> {
        self.render_pass.clone()
    }
}
