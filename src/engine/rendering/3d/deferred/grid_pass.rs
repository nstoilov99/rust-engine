//! Grid pass - renders infinite grid on XY plane (Z=0 in Z-up game space)
//!
//! Uses Unreal-style approach: camera-centered ground plane quad with hardware depth testing.
//! No manual depth sampling needed - GPU depth test handles occlusion automatically.

use glam::{Mat4, Vec3};
use smallvec::smallvec;
use std::sync::Arc;
use vulkano::device::Device;
use vulkano::pipeline::graphics::{
    color_blend::{AttachmentBlend, ColorBlendAttachmentState, ColorBlendState},
    depth_stencil::{CompareOp, DepthState, DepthStencilState},
    input_assembly::{InputAssemblyState, PrimitiveTopology},
    multisample::MultisampleState,
    rasterization::{CullMode, RasterizationState},
    vertex_input::VertexInputState,
    viewport::ViewportState,
    GraphicsPipelineCreateInfo,
};
use vulkano::pipeline::layout::PipelineDescriptorSetLayoutCreateInfo;
use vulkano::pipeline::{GraphicsPipeline, PipelineLayout, PipelineShaderStageCreateInfo};
use vulkano::render_pass::RenderPass;

// Grid shaders
pub mod grid_vs {
    vulkano_shaders::shader! {
        ty: "vertex",
        path: "src/engine/rendering/shaders/deferred/grid.vert",
    }
}

pub mod grid_fs {
    vulkano_shaders::shader! {
        ty: "fragment",
        path: "src/engine/rendering/shaders/deferred/grid.frag",
    }
}

/// Push constants for grid rendering
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct GridPushConstants {
    /// View-projection matrix
    pub view_proj: [[f32; 4]; 4],
    /// Camera position (xyz) and grid extent (w)
    pub camera_pos: [f32; 4],
    /// Grid parameters: grid_size1, grid_size2, fade_start, fade_end
    pub grid_params: [f32; 4],
}

unsafe impl bytemuck::Pod for GridPushConstants {}
unsafe impl bytemuck::Zeroable for GridPushConstants {}

impl GridPushConstants {
    pub fn new(view_proj: Mat4, camera_pos: Vec3, grid_extent: f32, fade_distance: f32) -> Self {
        Self {
            view_proj: view_proj.to_cols_array_2d(),
            camera_pos: [camera_pos.x, camera_pos.y, camera_pos.z, grid_extent],
            grid_params: [
                1.0,                 // grid_size1: fine grid (1 unit)
                10.0,                // grid_size2: coarse grid (10 units)
                fade_distance * 0.5, // fade_start
                fade_distance,       // fade_end
            ],
        }
    }
}

/// Grid rendering pass
pub struct GridPass {
    pipeline: Arc<GraphicsPipeline>,
    layout: Arc<PipelineLayout>,
    #[allow(dead_code)]
    render_pass: Arc<RenderPass>,
}

impl GridPass {
    pub fn new(
        device: Arc<Device>,
        render_pass: Arc<RenderPass>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        // Load shaders
        let vs = grid_vs::load(device.clone())?.entry_point("main").unwrap();
        let fs = grid_fs::load(device.clone())?.entry_point("main").unwrap();

        let stages = [
            PipelineShaderStageCreateInfo::new(vs),
            PipelineShaderStageCreateInfo::new(fs),
        ];

        // Pipeline layout with push constants only (no descriptor sets needed)
        let layout = PipelineLayout::new(
            device.clone(),
            PipelineDescriptorSetLayoutCreateInfo::from_stages(&stages)
                .into_pipeline_layout_create_info(device.clone())?,
        )?;

        // Create pipeline with alpha blending and depth testing
        let subpass = vulkano::render_pass::Subpass::from(render_pass.clone(), 0).unwrap();

        let pipeline = GraphicsPipeline::new(
            device.clone(),
            None,
            GraphicsPipelineCreateInfo {
                stages: smallvec![stages[0].clone(), stages[1].clone()],
                vertex_input_state: Some(VertexInputState::default()), // No vertex buffer
                input_assembly_state: Some(InputAssemblyState {
                    topology: PrimitiveTopology::TriangleStrip, // 4 vertices as triangle strip
                    ..Default::default()
                }),
                viewport_state: Some(ViewportState::default()),
                rasterization_state: Some(RasterizationState {
                    cull_mode: CullMode::None, // See from both sides
                    ..Default::default()
                }),
                multisample_state: Some(MultisampleState::default()),
                // Enable depth testing but disable depth writes
                depth_stencil_state: Some(DepthStencilState {
                    depth: Some(DepthState {
                        compare_op: CompareOp::Less,
                        write_enable: false, // Don't write to depth buffer
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
            pipeline,
            layout,
            render_pass,
        })
    }

    pub fn pipeline(&self) -> Arc<GraphicsPipeline> {
        self.pipeline.clone()
    }

    pub fn layout(&self) -> Arc<PipelineLayout> {
        self.layout.clone()
    }
}
