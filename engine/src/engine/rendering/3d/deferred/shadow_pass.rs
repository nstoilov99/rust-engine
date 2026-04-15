use smallvec::smallvec;
use std::sync::Arc;
use vulkano::descriptor_set::allocator::StandardDescriptorSetAllocator;
use vulkano::device::Device;
use vulkano::image::sampler::Sampler;
use vulkano::image::view::ImageView;
use vulkano::memory::allocator::StandardMemoryAllocator;
use vulkano::pipeline::graphics::{
    depth_stencil::{CompareOp, DepthState, DepthStencilState},
    input_assembly::InputAssemblyState,
    multisample::MultisampleState,
    rasterization::RasterizationState,
    vertex_input::{Vertex as VertexTrait, VertexDefinition},
    viewport::ViewportState,
    GraphicsPipelineCreateInfo,
};
use vulkano::pipeline::layout::{PipelineDescriptorSetLayoutCreateInfo, PipelineLayout};
use vulkano::pipeline::{GraphicsPipeline, PipelineShaderStageCreateInfo};
use vulkano::render_pass::{Framebuffer, FramebufferCreateInfo, RenderPass};

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
            device,
            None,
            GraphicsPipelineCreateInfo {
                stages: smallvec![stages[0].clone(), stages[1].clone()],
                vertex_input_state: Some(vertex_input_state),
                input_assembly_state: Some(InputAssemblyState::default()),
                viewport_state: Some(ViewportState::default()),
                rasterization_state: Some(RasterizationState::default()),
                multisample_state: Some(MultisampleState::default()),
                depth_stencil_state: Some(DepthStencilState {
                    depth: Some(DepthState {
                        compare_op: CompareOp::LessOrEqual,
                        write_enable: true,
                    }),
                    ..Default::default()
                }),
                color_blend_state: None,
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
