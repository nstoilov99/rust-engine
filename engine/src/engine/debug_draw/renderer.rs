//! Debug draw GPU pipeline and data types.
//!
//! Provides `DebugDrawPass` (two Vulkan line-list pipelines) and
//! `DebugDrawData` (vertex buffers ready for rendering).

use smallvec::smallvec;
use std::sync::Arc;
use vulkano::buffer::Subbuffer;
use vulkano::device::Device;
use vulkano::pipeline::graphics::{
    color_blend::{AttachmentBlend, ColorBlendAttachmentState, ColorBlendState},
    depth_stencil::{CompareOp, DepthState, DepthStencilState},
    input_assembly::{InputAssemblyState, PrimitiveTopology},
    multisample::MultisampleState,
    rasterization::{CullMode, RasterizationState},
    vertex_input::{Vertex as VertexTrait, VertexDefinition},
    viewport::ViewportState,
    GraphicsPipelineCreateInfo,
};
use vulkano::pipeline::layout::PipelineDescriptorSetLayoutCreateInfo;
use vulkano::pipeline::{GraphicsPipeline, PipelineLayout, PipelineShaderStageCreateInfo};
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
}

impl DebugDrawData {
    /// Create empty debug draw data (no lines to render).
    pub fn empty() -> Self {
        Self {
            depth_buffer: None,
            depth_vertex_count: 0,
            overlay_buffer: None,
            overlay_vertex_count: 0,
        }
    }

    /// Returns true if there is nothing to draw.
    pub fn is_empty(&self) -> bool {
        self.depth_buffer.is_none() && self.overlay_buffer.is_none()
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
        let vertex_input_state =
            DebugLineVertex::per_vertex().definition(&vs)?;

        let stages = [
            PipelineShaderStageCreateInfo::new(vs),
            PipelineShaderStageCreateInfo::new(fs),
        ];

        // Pipeline layout with push constants only
        let layout = PipelineLayout::new(
            device.clone(),
            PipelineDescriptorSetLayoutCreateInfo::from_stages(&stages)
                .into_pipeline_layout_create_info(device.clone())?,
        )?;

        let subpass = vulkano::render_pass::Subpass::from(render_pass.clone(), 0)
            .ok_or("Invalid subpass for debug draw")?;

        // Depth-tested pipeline: CompareOp::LessOrEqual, write_enable: false
        let depth_pipeline = GraphicsPipeline::new(
            device.clone(),
            None,
            GraphicsPipelineCreateInfo {
                stages: smallvec![stages[0].clone(), stages[1].clone()],
                vertex_input_state: Some(vertex_input_state.clone()),
                input_assembly_state: Some(InputAssemblyState {
                    topology: PrimitiveTopology::LineList,
                    ..Default::default()
                }),
                viewport_state: Some(ViewportState::default()),
                rasterization_state: Some(RasterizationState {
                    cull_mode: CullMode::None,
                    ..Default::default()
                }),
                multisample_state: Some(MultisampleState::default()),
                depth_stencil_state: Some(DepthStencilState {
                    depth: Some(DepthState {
                        compare_op: CompareOp::LessOrEqual,
                        write_enable: false,
                    }),
                    ..Default::default()
                }),
                color_blend_state: Some(ColorBlendState::with_attachment_states(
                    subpass.num_color_attachments(),
                    ColorBlendAttachmentState {
                        blend: Some(AttachmentBlend::alpha()),
                        ..Default::default()
                    },
                )),
                dynamic_state: [
                    vulkano::pipeline::DynamicState::Viewport,
                    vulkano::pipeline::DynamicState::Scissor,
                ]
                .into_iter()
                .collect(),
                subpass: Some(subpass.clone().into()),
                ..GraphicsPipelineCreateInfo::layout(layout.clone())
            },
        )?;

        // Overlay pipeline: no depth test
        let overlay_pipeline = GraphicsPipeline::new(
            device.clone(),
            None,
            GraphicsPipelineCreateInfo {
                stages: smallvec![stages[0].clone(), stages[1].clone()],
                vertex_input_state: Some(vertex_input_state),
                input_assembly_state: Some(InputAssemblyState {
                    topology: PrimitiveTopology::LineList,
                    ..Default::default()
                }),
                viewport_state: Some(ViewportState::default()),
                rasterization_state: Some(RasterizationState {
                    cull_mode: CullMode::None,
                    ..Default::default()
                }),
                multisample_state: Some(MultisampleState::default()),
                // Render pass has a depth attachment so we must declare state,
                // but use Always compare + no writes to skip depth testing.
                depth_stencil_state: Some(DepthStencilState {
                    depth: Some(DepthState {
                        compare_op: CompareOp::Always,
                        write_enable: false,
                    }),
                    ..Default::default()
                }),
                color_blend_state: Some(ColorBlendState::with_attachment_states(
                    subpass.num_color_attachments(),
                    ColorBlendAttachmentState {
                        blend: Some(AttachmentBlend::alpha()),
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
                ..GraphicsPipelineCreateInfo::layout(layout.clone())
            },
        )?;

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
