//! Geometry pass - renders scene to G-Buffer

use smallvec::smallvec;
use std::sync::Arc;
use vulkano::device::Device;
use vulkano::pipeline::graphics::{
    color_blend::{ColorBlendAttachmentState, ColorBlendState},
    depth_stencil::{DepthState, DepthStencilState},
    input_assembly::InputAssemblyState,
    multisample::MultisampleState,
    rasterization::RasterizationState,
    vertex_input::{Vertex as VertexTrait, VertexDefinition},
    viewport::ViewportState,
    GraphicsPipelineCreateInfo,
};
use vulkano::pipeline::layout::{PipelineDescriptorSetLayoutCreateInfo, PipelineLayout};
use vulkano::pipeline::{GraphicsPipeline, PipelineShaderStageCreateInfo};
use vulkano::render_pass::RenderPass;

use crate::engine::rendering::pipeline_registry::{PipelineId, PipelineRegistry};
use crate::engine::rendering::rendering_3d::Vertex3D;

// G-Buffer shaders (compile-time SPIR-V for initial pipeline creation)
pub mod gbuffer_vs {
    vulkano_shaders::shader! {
        ty: "vertex",
        path: "src/engine/rendering/shaders/deferred/gbuffer.vert",
    }
}

pub mod gbuffer_fs {
    vulkano_shaders::shader! {
        ty: "fragment",
        path: "src/engine/rendering/shaders/deferred/gbuffer.frag",
    }
}

/// Geometry pass pipeline (writes to G-Buffer)
pub struct GeometryPass {
    pipeline_id: PipelineId,
    layout: Arc<PipelineLayout>,
}

impl GeometryPass {
    pub fn new(
        device: Arc<Device>,
        render_pass: Arc<RenderPass>,
    ) -> Result<(Self, Arc<GraphicsPipeline>), Box<dyn std::error::Error>> {
        let (pipeline, layout) = Self::create_pipeline(device, render_pass)?;

        Ok((
            Self {
                pipeline_id: PipelineId::Geometry,
                layout,
            },
            pipeline,
        ))
    }

    /// Create the geometry pipeline from compile-time shaders.
    fn create_pipeline(
        device: Arc<Device>,
        render_pass: Arc<RenderPass>,
    ) -> Result<(Arc<GraphicsPipeline>, Arc<PipelineLayout>), Box<dyn std::error::Error>> {
        let vs = gbuffer_vs::load(device.clone())?
            .entry_point("main")
            .unwrap();
        let fs = gbuffer_fs::load(device.clone())?
            .entry_point("main")
            .unwrap();

        let vertex_input_state = Vertex3D::per_vertex().definition(&vs)?;

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
                vertex_input_state: Some(vertex_input_state),
                input_assembly_state: Some(InputAssemblyState::default()),
                viewport_state: Some(ViewportState::default()),
                rasterization_state: Some(RasterizationState::default()),
                multisample_state: Some(MultisampleState::default()),
                depth_stencil_state: Some(DepthStencilState {
                    depth: Some(DepthState::simple()),
                    ..Default::default()
                }),
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

    /// Create a geometry pipeline from runtime-compiled SPIR-V (for hot-reload).
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

        let vertex_input_state = Vertex3D::per_vertex().definition(&vs)?;

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
                vertex_input_state: Some(vertex_input_state),
                input_assembly_state: Some(InputAssemblyState::default()),
                viewport_state: Some(ViewportState::default()),
                rasterization_state: Some(RasterizationState::default()),
                multisample_state: Some(MultisampleState::default()),
                depth_stencil_state: Some(DepthStencilState {
                    depth: Some(DepthState::simple()),
                    ..Default::default()
                }),
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

    pub fn pipeline(&self, registry: &PipelineRegistry) -> Arc<GraphicsPipeline> {
        registry.get(self.pipeline_id)
    }

    pub fn layout(&self) -> Arc<PipelineLayout> {
        self.layout.clone()
    }
}
