//! Debug draw GPU pipeline and data types.
//!
//! Provides `DebugDrawPass` (two Vulkan line-list pipelines) and
//! `DebugDrawData` (vertex buffers ready for rendering).

use crate::engine::rendering::rendering_3d::deferred::pass_pipeline::PassPipelineBuilder;
use std::sync::Arc;
use vulkano::buffer::Subbuffer;
use vulkano::device::Device;
use vulkano::pipeline::graphics::{
    color_blend::AttachmentBlend,
    depth_stencil::{CompareOp, DepthState},
    input_assembly::PrimitiveTopology,
    rasterization::{CullMode, RasterizationState},
    vertex_input::{Vertex as VertexTrait, VertexDefinition},
};
use vulkano::pipeline::{GraphicsPipeline, PipelineLayout};
use vulkano::render_pass::RenderPass;

/// Debug line vertex: position (vec3) + color (vec4) = 28 bytes.
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    vulkano::buffer::BufferContents,
    vulkano::pipeline::graphics::vertex_input::Vertex,
)]
#[repr(C)]
pub struct DebugLineVertex {
    #[format(R32G32B32_SFLOAT)]
    pub position: [f32; 3],
    #[format(R32G32B32A32_SFLOAT)]
    pub color: [f32; 4],
}

/// Push constants for debug line shaders (just view_proj mat4 = 64 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct DebugLinePushConstants {
    pub view_proj: [[f32; 4]; 4],
}

unsafe impl bytemuck::Pod for DebugLinePushConstants {}
unsafe impl bytemuck::Zeroable for DebugLinePushConstants {}

// Debug line shaders
pub mod debug_lines_vs {
    vulkano_shaders::shader! {
        ty: "vertex",
        path: "src/engine/rendering/shaders/deferred/debug_lines.vert",
    }
}

pub mod debug_lines_fs {
    vulkano_shaders::shader! {
        ty: "fragment",
        path: "src/engine/rendering/shaders/deferred/debug_lines.frag",
    }
}

/// GPU data ready for debug line rendering.
pub struct DebugDrawData {
    /// Vertex buffer for depth-tested lines (may be None if no lines).
    pub depth_buffer: Option<Subbuffer<[DebugLineVertex]>>,
    /// Number of vertices in the depth buffer.
    pub depth_vertex_count: u32,
    /// Vertex buffer for overlay lines (may be None if no lines).
    pub overlay_buffer: Option<Subbuffer<[DebugLineVertex]>>,
    /// Number of vertices in the overlay buffer.
    pub overlay_vertex_count: u32,
    /// Pre-uploaded depth-tested lines reused across frames (e.g. cached
    /// collision wireframes) — same pipeline as `depth_buffer`, but the
    /// caller keeps the buffer alive instead of rebuilding it per frame.
    pub static_depth_buffer: Option<Subbuffer<[DebugLineVertex]>>,
    /// Number of vertices in the static depth buffer.
    pub static_depth_vertex_count: u32,
}

impl DebugDrawData {
    /// Create empty debug draw data (no lines to render).
    pub fn empty() -> Self {
        Self {
            depth_buffer: None,
            depth_vertex_count: 0,
            overlay_buffer: None,
            overlay_vertex_count: 0,
            static_depth_buffer: None,
            static_depth_vertex_count: 0,
        }
    }

    /// Returns true if there is nothing to draw.
    pub fn is_empty(&self) -> bool {
        self.depth_buffer.is_none()
            && self.overlay_buffer.is_none()
            && self.static_depth_buffer.is_none()
    }
}

/// Debug draw rendering pass with two pipelines (depth-tested + overlay).
pub struct DebugDrawPass {
    depth_pipeline: Arc<GraphicsPipeline>,
    overlay_pipeline: Arc<GraphicsPipeline>,
    layout: Arc<PipelineLayout>,
    #[allow(dead_code)]
    render_pass: Arc<RenderPass>,
}

impl DebugDrawPass {
    pub fn new(
        device: Arc<Device>,
        render_pass: Arc<RenderPass>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        // Load shaders
        let vs = debug_lines_vs::load(device.clone())?
            .entry_point("main")
            .ok_or("Missing vertex shader entry point")?;
        let fs = debug_lines_fs::load(device.clone())?
            .entry_point("main")
            .ok_or("Missing fragment shader entry point")?;

        // Vertex input from DebugLineVertex
        let vertex_input_state = DebugLineVertex::per_vertex().definition(&vs)?;

        let line_pipeline = |depth: DepthState| {
            PassPipelineBuilder::new(device.clone(), render_pass.clone(), vs.clone(), fs.clone())
                .vertex_input(vertex_input_state.clone())
                .topology(PrimitiveTopology::LineList)
                .rasterization(RasterizationState {
                    cull_mode: CullMode::None,
                    ..Default::default()
                })
                .depth(depth)
                .blend(AttachmentBlend::alpha())
                .build()
        };

        // Depth-tested pipeline: CompareOp::LessOrEqual, write_enable: false
        let (depth_pipeline, layout) = line_pipeline(DepthState {
            compare_op: CompareOp::LessOrEqual,
            write_enable: false,
        })?;

        // Overlay pipeline: the render pass has a depth attachment so state
        // must be declared, but Always compare + no writes skips depth testing.
        let (overlay_pipeline, _overlay_layout) = line_pipeline(DepthState {
            compare_op: CompareOp::Always,
            write_enable: false,
        })?;

        Ok(Self {
            depth_pipeline,
            overlay_pipeline,
            layout,
            render_pass,
        })
    }

    pub fn depth_pipeline(&self) -> Arc<GraphicsPipeline> {
        self.depth_pipeline.clone()
    }

    pub fn overlay_pipeline(&self) -> Arc<GraphicsPipeline> {
        self.overlay_pipeline.clone()
    }

    pub fn layout(&self) -> Arc<PipelineLayout> {
        self.layout.clone()
    }
}

impl crate::engine::rendering::rendering_3d::deferred::pass::DeferredPass for DebugDrawPass {
    fn name(&self) -> &'static str {
        "debug_draw"
    }
    // No resize/rebind: draws into renderer-cached target framebuffers,
    // which are invalidated by `DeferredRenderer::resize`.
}
