//! Shared `GraphicsPipeline` recipe for deferred passes.
//!
//! Every deferred pass builds its pipeline the same way: two shader stages,
//! layout derived from the stages, subpass 0 of a given render pass, dynamic
//! viewport/scissor, default multisampling. [`PassPipelineBuilder`] owns that
//! recipe; passes only state how they diverge:
//!
//! - **Fullscreen post-process** (the default): no vertex input, triangle
//!   list, no depth, one opaque color attachment — `new(..).build()`.
//! - **Mesh passes** add `.vertex_input(..)` and `.depth(..)`.
//! - **Depth-only** (shadow) adds `.no_color_attachments()`.
//! - **Blended overlays** (grid, debug lines, bloom upsample, plankton) add
//!   `.blend(..)` and/or `.topology(..)` / `.rasterization(..)`.
//!
//! This is a pure packaging of the previous hand-rolled
//! `GraphicsPipelineCreateInfo` blocks — the produced GPU state is identical.

use std::sync::Arc;
use vulkano::device::Device;
use vulkano::pipeline::graphics::{
    color_blend::{AttachmentBlend, ColorBlendAttachmentState, ColorBlendState},
    depth_stencil::{DepthState, DepthStencilState},
    input_assembly::{InputAssemblyState, PrimitiveTopology},
    multisample::MultisampleState,
    rasterization::RasterizationState,
    vertex_input::VertexInputState,
    viewport::ViewportState,
    GraphicsPipelineCreateInfo,
};
use vulkano::pipeline::layout::PipelineDescriptorSetLayoutCreateInfo;
use vulkano::pipeline::{
    DynamicState, GraphicsPipeline, PipelineLayout, PipelineShaderStageCreateInfo,
};
use vulkano::render_pass::{RenderPass, Subpass};
use vulkano::shader::EntryPoint;

pub struct PassPipelineBuilder {
    device: Arc<Device>,
    render_pass: Arc<RenderPass>,
    vs: EntryPoint,
    fs: EntryPoint,
    vertex_input: VertexInputState,
    input_assembly: InputAssemblyState,
    rasterization: RasterizationState,
    depth_stencil: Option<DepthStencilState>,
    /// Blend/write state applied to every color attachment of the subpass.
    /// `None` = depth-only pass (no color blend state at all).
    color_attachment: Option<ColorBlendAttachmentState>,
}

impl PassPipelineBuilder {
    /// Start from the fullscreen post-process defaults: no vertex input,
    /// triangle list, default rasterization, no depth/stencil, one opaque
    /// (non-blended) state per color attachment.
    pub fn new(
        device: Arc<Device>,
        render_pass: Arc<RenderPass>,
        vs: EntryPoint,
        fs: EntryPoint,
    ) -> Self {
        Self {
            device,
            render_pass,
            vs,
            fs,
            vertex_input: VertexInputState::default(),
            input_assembly: InputAssemblyState::default(),
            rasterization: RasterizationState::default(),
            depth_stencil: None,
            color_attachment: Some(ColorBlendAttachmentState::default()),
        }
    }

    /// Vertex buffer layout (mesh passes). Default: no vertex input.
    pub fn vertex_input(mut self, state: VertexInputState) -> Self {
        self.vertex_input = state;
        self
    }

    /// Primitive topology. Default: triangle list.
    pub fn topology(mut self, topology: PrimitiveTopology) -> Self {
        self.input_assembly.topology = topology;
        self
    }

    /// Full rasterization state (cull mode, depth bias). Default: vulkano's.
    pub fn rasterization(mut self, state: RasterizationState) -> Self {
        self.rasterization = state;
        self
    }

    /// Enable depth testing with the given state (no stencil). Default: none.
    pub fn depth(mut self, depth: DepthState) -> Self {
        self.depth_stencil = Some(DepthStencilState {
            depth: Some(depth),
            ..Default::default()
        });
        self
    }

    /// Enable blending on all color attachments. Default: opaque.
    pub fn blend(mut self, blend: AttachmentBlend) -> Self {
        self.color_attachment = Some(ColorBlendAttachmentState {
            blend: Some(blend),
            ..Default::default()
        });
        self
    }

    /// Depth-only pass: emit no color blend state (shadow map).
    pub fn no_color_attachments(mut self) -> Self {
        self.color_attachment = None;
        self
    }

    /// Create the pipeline layout (derived from the shader stages) and the
    /// pipeline against subpass 0, with dynamic viewport/scissor.
    pub fn build(
        self,
    ) -> Result<(Arc<GraphicsPipeline>, Arc<PipelineLayout>), Box<dyn std::error::Error>> {
        let stages = [
            PipelineShaderStageCreateInfo::new(self.vs),
            PipelineShaderStageCreateInfo::new(self.fs),
        ];

        let layout = PipelineLayout::new(
            self.device.clone(),
            PipelineDescriptorSetLayoutCreateInfo::from_stages(&stages)
                .into_pipeline_layout_create_info(self.device.clone())?,
        )?;

        let subpass = Subpass::from(self.render_pass.clone(), 0)
            .ok_or("render pass has no subpass 0")?;

        let color_blend_state = self.color_attachment.map(|state| {
            ColorBlendState::with_attachment_states(subpass.num_color_attachments(), state)
        });

        let pipeline = GraphicsPipeline::new(
            self.device,
            None,
            GraphicsPipelineCreateInfo {
                stages: stages.into_iter().collect(),
                vertex_input_state: Some(self.vertex_input),
                input_assembly_state: Some(self.input_assembly),
                viewport_state: Some(ViewportState::default()),
                rasterization_state: Some(self.rasterization),
                multisample_state: Some(MultisampleState::default()),
                depth_stencil_state: self.depth_stencil,
                color_blend_state,
                dynamic_state: [DynamicState::Viewport, DynamicState::Scissor]
                    .into_iter()
                    .collect(),
                subpass: Some(subpass.into()),
                ..GraphicsPipelineCreateInfo::layout(layout.clone())
            },
        )?;

        Ok((pipeline, layout))
    }
}
