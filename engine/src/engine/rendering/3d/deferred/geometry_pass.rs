//! Geometry pass - renders scene to G-Buffer

use std::sync::Arc;
use vulkano::device::Device;
use vulkano::pipeline::graphics::{
    depth_stencil::DepthState,
    vertex_input::{Vertex as VertexTrait, VertexDefinition},
};
use vulkano::pipeline::layout::PipelineLayout;
use vulkano::pipeline::GraphicsPipeline;
use vulkano::render_pass::RenderPass;

use super::pass_pipeline::PassPipelineBuilder;
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

        let (pipeline, layout) = PassPipelineBuilder::new(device, render_pass, vs, fs)
            .vertex_input(vertex_input_state)
            .depth(DepthState::simple())
            .build()?;

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
            ShaderModule::new(
                device.clone(),
                vulkano::shader::ShaderModuleCreateInfo::new(vs_spirv),
            )?
        };
        let fs_module = unsafe {
            ShaderModule::new(
                device.clone(),
                vulkano::shader::ShaderModuleCreateInfo::new(fs_spirv),
            )?
        };

        let vs = vs_module
            .entry_point("main")
            .ok_or("Missing vertex entry point 'main'")?;
        let fs = fs_module
            .entry_point("main")
            .ok_or("Missing fragment entry point 'main'")?;

        let vertex_input_state = Vertex3D::per_vertex().definition(&vs)?;

        let (pipeline, _layout) = PassPipelineBuilder::new(device, render_pass, vs, fs)
            .vertex_input(vertex_input_state)
            .depth(DepthState::simple())
            .build()?;

        Ok(pipeline)
    }

    pub fn pipeline(&self, registry: &PipelineRegistry) -> Arc<GraphicsPipeline> {
        registry.get(self.pipeline_id)
    }

    pub fn layout(&self) -> Arc<PipelineLayout> {
        self.layout.clone()
    }
}

impl super::pass::DeferredPass for GeometryPass {
    fn name(&self) -> &'static str {
        "geometry"
    }
    // No resize/rebind: renders into the renderer-owned G-buffer framebuffer,
    // which `DeferredRenderer::resize` recreates before the pass loop runs.
}
