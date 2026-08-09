use super::pass_pipeline::PassPipelineBuilder;
use crate::engine::rendering::error::RenderError;
use std::sync::Arc;
use vulkano::descriptor_set::allocator::StandardDescriptorSetAllocator;
use vulkano::descriptor_set::{DescriptorSet, WriteDescriptorSet};
use vulkano::device::Device;
use vulkano::image::sampler::{Filter, Sampler, SamplerAddressMode, SamplerCreateInfo};
use vulkano::image::view::ImageView;
use vulkano::pipeline::{GraphicsPipeline, PipelineLayout};
use vulkano::render_pass::RenderPass;

pub mod composite_vs {
    vulkano_shaders::shader! {
        ty: "vertex",
        path: "src/engine/rendering/shaders/deferred/composite.vert",
    }
}

pub mod composite_fs {
    vulkano_shaders::shader! {
        ty: "fragment",
        path: "src/engine/rendering/shaders/deferred/composite.frag",
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct CompositePushConstants {
    pub exposure: f32,
    pub bloom_intensity: f32,
    pub vignette_intensity: f32,
    pub tone_map_mode: f32,
    pub exposure_mode: f32,
    pub _pad0: f32,
    pub _pad1: f32,
    pub _pad2: f32,
}

unsafe impl bytemuck::Pod for CompositePushConstants {}
unsafe impl bytemuck::Zeroable for CompositePushConstants {}

pub struct CompositePass {
    pipeline: Arc<GraphicsPipeline>,
    layout: Arc<PipelineLayout>,
    render_pass: Arc<RenderPass>,
    sampler: Arc<Sampler>,
    /// Input set (HDR + bloom + luminance) — populated by `rebind()`.
    descriptor_set: Option<Arc<DescriptorSet>>,
}

impl CompositePass {
    pub fn new(
        device: Arc<Device>,
        render_pass: Arc<RenderPass>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let sampler = Sampler::new(
            device.clone(),
            SamplerCreateInfo {
                mag_filter: Filter::Linear,
                min_filter: Filter::Linear,
                address_mode: [SamplerAddressMode::ClampToEdge; 3],
                ..Default::default()
            },
        )?;

        let (pipeline, layout) = PassPipelineBuilder::new(
            device.clone(),
            render_pass.clone(),
            composite_vs::load(device.clone())?
                .entry_point("main")
                .unwrap(),
            composite_fs::load(device)?.entry_point("main").unwrap(),
        )
        .build()?;

        Ok(Self {
            pipeline,
            layout,
            render_pass,
            sampler,
            descriptor_set: None,
        })
    }

    pub fn create_descriptor_set(
        &self,
        descriptor_set_allocator: Arc<StandardDescriptorSetAllocator>,
        hdr_target: Arc<ImageView>,
        bloom_texture: Arc<ImageView>,
        luminance_texture: Arc<ImageView>,
    ) -> Result<Arc<DescriptorSet>, Box<dyn std::error::Error>> {
        let layout = self
            .layout
            .set_layouts()
            .first()
            .ok_or("Missing Set 0 layout")?;
        let set = DescriptorSet::new(
            descriptor_set_allocator,
            layout.clone(),
            [
                WriteDescriptorSet::image_view_sampler(0, hdr_target, self.sampler.clone()),
                WriteDescriptorSet::image_view_sampler(1, bloom_texture, self.sampler.clone()),
                WriteDescriptorSet::image_view_sampler(2, luminance_texture, self.sampler.clone()),
            ],
            [],
        )?;
        Ok(set)
    }

    pub fn pipeline(&self) -> Arc<GraphicsPipeline> {
        self.pipeline.clone()
    }

    pub fn layout(&self) -> Arc<PipelineLayout> {
        self.layout.clone()
    }

    pub fn render_pass(&self) -> Arc<RenderPass> {
        self.render_pass.clone()
    }

    /// Input descriptor set. `None` until `rebind()` runs.
    pub fn descriptor_set(&self) -> Option<&Arc<DescriptorSet>> {
        self.descriptor_set.as_ref()
    }
}

impl super::pass::DeferredPass for CompositePass {
    fn name(&self) -> &'static str {
        "composite"
    }

    // No `resize`: renders into externally provided target framebuffers
    // (swapchain / viewport texture), cached and cleared by the renderer.

    fn rebind(&mut self, inputs: &super::pass::PassInputs) -> Result<(), RenderError> {
        self.descriptor_set = Some(self.create_descriptor_set(
            inputs.descriptor_set_allocator.clone(),
            inputs.hdr_target.clone(),
            inputs.bloom_result.clone(),
            inputs.luminance_1x1.clone(),
        )?);
        Ok(())
    }
}
