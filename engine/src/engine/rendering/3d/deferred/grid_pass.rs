//! Grid pass - renders infinite grid on XY plane (Z=0 in Z-up game space)
//!
//! Uses Unreal-style approach: camera-centered ground plane quad with hardware depth testing.
//! No manual depth sampling needed - GPU depth test handles occlusion automatically.

use glam::{Mat4, Vec3};
use std::sync::Arc;
use vulkano::device::Device;
use vulkano::pipeline::graphics::{
    color_blend::AttachmentBlend,
    depth_stencil::{CompareOp, DepthState},
    input_assembly::PrimitiveTopology,
    rasterization::{CullMode, DepthBiasState, RasterizationState},
};
use vulkano::pipeline::{GraphicsPipeline, PipelineLayout};
use vulkano::render_pass::RenderPass;

use super::pass_pipeline::PassPipelineBuilder;

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
    /// Grid parameters: base_spacing, unused, fade_start, fade_end
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
                1.0,                 // base spacing (1 unit) — LOD picks 1/10/100m
                0.0,                 // reserved
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
        // Camera-centered quad, alpha-blended, depth-tested but not depth-written.
        let (pipeline, layout) = PassPipelineBuilder::new(
            device.clone(),
            render_pass.clone(),
            grid_vs::load(device.clone())?.entry_point("main").unwrap(),
            grid_fs::load(device)?.entry_point("main").unwrap(),
        )
        .topology(PrimitiveTopology::TriangleStrip) // 4 vertices as triangle strip
        .rasterization(RasterizationState {
            cull_mode: CullMode::None,
            // Push grid fragments slightly behind coplanar geometry
            // so a floor mesh sitting on Z=0 reliably wins the depth
            // test instead of flickering against the infinite grid.
            depth_bias: Some(DepthBiasState {
                constant_factor: 1.0,
                clamp: 0.0,
                slope_factor: 1.0,
            }),
            ..Default::default()
        })
        .depth(DepthState {
            compare_op: CompareOp::Less,
            write_enable: false, // Don't write to depth buffer
        })
        .blend(AttachmentBlend::alpha())
        .build()?;

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

impl super::pass::DeferredPass for GridPass {
    fn name(&self) -> &'static str {
        "grid"
    }
    // No resize/rebind: draws into renderer-cached target framebuffers,
    // which are invalidated by `DeferredRenderer::resize`.
}
