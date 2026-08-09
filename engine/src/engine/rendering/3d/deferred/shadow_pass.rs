use std::sync::Arc;
use vulkano::descriptor_set::allocator::StandardDescriptorSetAllocator;
use vulkano::device::Device;
use vulkano::image::sampler::Sampler;
use vulkano::image::view::ImageView;
use vulkano::memory::allocator::StandardMemoryAllocator;
use vulkano::pipeline::graphics::{
    depth_stencil::{CompareOp, DepthState},
    rasterization::{DepthBiasState, RasterizationState},
    vertex_input::{Vertex as VertexTrait, VertexDefinition},
};
use vulkano::pipeline::layout::PipelineLayout;
use vulkano::pipeline::GraphicsPipeline;
use vulkano::render_pass::{Framebuffer, FramebufferCreateInfo, RenderPass};

use super::pass_pipeline::PassPipelineBuilder;
use crate::engine::rendering::rendering_3d::shadow;
use crate::engine::rendering::rendering_3d::Vertex3D;

pub mod shadow_vs {
    vulkano_shaders::shader! {
        ty: "vertex",
        path: "src/engine/rendering/shaders/3d/shadow_vs.glsl",
    }
}

pub mod shadow_fs {
    vulkano_shaders::shader! {
        ty: "fragment",
        path: "src/engine/rendering/shaders/3d/shadow_fs.glsl",
    }
}

pub struct ShadowPass {
    pipeline: Arc<GraphicsPipeline>,
    layout: Arc<PipelineLayout>,
    render_pass: Arc<RenderPass>,
    shadow_map: Arc<ImageView>,
    shadow_sampler: Arc<Sampler>,
    framebuffer: Arc<Framebuffer>,
}

impl ShadowPass {
    pub fn new(
        device: Arc<Device>,
        allocator: Arc<StandardMemoryAllocator>,
        _descriptor_set_allocator: Arc<StandardDescriptorSetAllocator>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let render_pass = shadow::create_shadow_render_pass(device.clone())?;
        let shadow_map = shadow::create_shadow_map(device.clone(), allocator, 2048)?;
        let shadow_sampler = shadow::create_shadow_sampler(device.clone())?;

        let framebuffer = Framebuffer::new(
            render_pass.clone(),
            FramebufferCreateInfo {
                attachments: vec![shadow_map.clone()],
                ..Default::default()
            },
        )?;

        let vs = shadow_vs::load(device.clone())?
            .entry_point("main")
            .unwrap();
        let fs = shadow_fs::load(device.clone())?
            .entry_point("main")
            .unwrap();

        let vertex_input_state = Vertex3D::per_vertex().definition(&vs)?;

        let (pipeline, layout) = PassPipelineBuilder::new(device, render_pass.clone(), vs, fs)
            .vertex_input(vertex_input_state)
            // Hardware depth bias to prevent shadow acne — slope_factor scales
            // with surface angle, constant_factor handles precision noise.
            .rasterization(RasterizationState {
                depth_bias: Some(DepthBiasState {
                    constant_factor: 2.0,
                    clamp: 0.0,
                    slope_factor: 2.5,
                }),
                ..Default::default()
            })
            .depth(DepthState {
                compare_op: CompareOp::LessOrEqual,
                write_enable: true,
            })
            .no_color_attachments()
            .build()?;

        Ok(Self {
            pipeline,
            layout,
            render_pass,
            shadow_map,
            shadow_sampler,
            framebuffer,
        })
    }

    pub fn shadow_map(&self) -> Arc<ImageView> {
        self.shadow_map.clone()
    }

    pub fn shadow_sampler(&self) -> Arc<Sampler> {
        self.shadow_sampler.clone()
    }

    pub fn pipeline(&self) -> Arc<GraphicsPipeline> {
        self.pipeline.clone()
    }

    pub fn layout(&self) -> Arc<PipelineLayout> {
        self.layout.clone()
    }

    pub fn framebuffer(&self) -> Arc<Framebuffer> {
        self.framebuffer.clone()
    }

    pub fn render_pass(&self) -> Arc<RenderPass> {
        self.render_pass.clone()
    }
}

impl super::pass::DeferredPass for ShadowPass {
    fn name(&self) -> &'static str {
        "shadow"
    }
    // No resize/rebind: the shadow map is a fixed-size target independent of
    // the window/viewport extent.
}
