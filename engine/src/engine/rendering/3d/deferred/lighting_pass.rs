//! Lighting pass - reads G-Buffer and calculates lighting

use std::sync::Arc;
use vulkano::descriptor_set::allocator::StandardDescriptorSetAllocator;
use vulkano::descriptor_set::{DescriptorSet, WriteDescriptorSet};
use vulkano::device::Device;
use vulkano::image::sampler::{Filter, Sampler, SamplerAddressMode, SamplerCreateInfo};
use vulkano::image::view::ImageView;
use vulkano::pipeline::{GraphicsPipeline, PipelineLayout};
use vulkano::render_pass::{Framebuffer, FramebufferCreateInfo, RenderPass};

use super::pass::{DeferredPass, PassInputs};
use super::pass_pipeline::PassPipelineBuilder;
use crate::engine::rendering::error::RenderError;
use crate::engine::rendering::pipeline_registry::{PipelineId, PipelineRegistry};

// Lighting shaders
pub mod lighting_vs {
    vulkano_shaders::shader! {
        ty: "vertex",
        path: "src/engine/rendering/shaders/deferred/lighting.vert",
    }
}

pub mod lighting_fs {
    vulkano_shaders::shader! {
        ty: "fragment",
        path: "src/engine/rendering/shaders/deferred/lighting.frag",
    }
}

/// Lighting pass pipeline (fullscreen quad, reads G-Buffer)
pub struct LightingPass {
    pipeline_id: PipelineId,
    sampler: Arc<Sampler>,
    layout: Arc<PipelineLayout>,
    render_pass: Arc<RenderPass>,
    // Bind state owned by the pass — populated by `rebind()` after
    // construction and after each resize.
    gbuffer_set: Option<Arc<DescriptorSet>>,
    shadow_set: Option<Arc<DescriptorSet>>,
    ssao_set: Option<Arc<DescriptorSet>>,
    ssao_fallback_set: Option<Arc<DescriptorSet>>,
    hdr_framebuffer: Option<Arc<Framebuffer>>,
}

impl LightingPass {
    pub fn new(
        device: Arc<Device>,
        render_pass: Arc<RenderPass>,
    ) -> Result<(Self, Arc<GraphicsPipeline>), Box<dyn std::error::Error>> {
        let (pipeline, layout) = Self::create_pipeline(device.clone(), render_pass.clone())?;

        // Create sampler for G-Buffer textures
        let sampler = Sampler::new(
            device,
            SamplerCreateInfo {
                mag_filter: Filter::Nearest,
                min_filter: Filter::Nearest,
                address_mode: [SamplerAddressMode::ClampToEdge; 3],
                ..Default::default()
            },
        )?;

        Ok((
            Self {
                pipeline_id: PipelineId::Lighting,
                sampler,
                layout,
                render_pass,
                gbuffer_set: None,
                shadow_set: None,
                ssao_set: None,
                ssao_fallback_set: None,
                hdr_framebuffer: None,
            },
            pipeline,
        ))
    }

    /// Create the lighting pipeline from compile-time shaders.
    fn create_pipeline(
        device: Arc<Device>,
        render_pass: Arc<RenderPass>,
    ) -> Result<(Arc<GraphicsPipeline>, Arc<PipelineLayout>), Box<dyn std::error::Error>> {
        let (pipeline, layout) = PassPipelineBuilder::new(
            device.clone(),
            render_pass,
            lighting_vs::load(device.clone())?
                .entry_point("main")
                .unwrap(),
            lighting_fs::load(device)?.entry_point("main").unwrap(),
        )
        .build()?;

        Ok((pipeline, layout))
    }

    /// Create a lighting pipeline from runtime-compiled SPIR-V (for hot-reload).
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

        let (pipeline, _layout) = PassPipelineBuilder::new(device, render_pass, vs, fs).build()?;

        Ok(pipeline)
    }

    /// Create descriptor set for G-Buffer sampling
    pub fn create_descriptor_set(
        &self,
        descriptor_set_allocator: Arc<StandardDescriptorSetAllocator>,
        position: Arc<ImageView>,
        normal: Arc<ImageView>,
        albedo: Arc<ImageView>,
        material: Arc<ImageView>,
        emissive: Arc<ImageView>,
    ) -> Result<Arc<DescriptorSet>, Box<dyn std::error::Error>> {
        let layout = self.layout.set_layouts().first().unwrap();

        let set = DescriptorSet::new(
            descriptor_set_allocator,
            layout.clone(),
            [
                WriteDescriptorSet::image_view_sampler(0, position, self.sampler.clone()),
                WriteDescriptorSet::image_view_sampler(1, normal, self.sampler.clone()),
                WriteDescriptorSet::image_view_sampler(2, albedo, self.sampler.clone()),
                WriteDescriptorSet::image_view_sampler(3, material, self.sampler.clone()),
                WriteDescriptorSet::image_view_sampler(4, emissive, self.sampler.clone()),
            ],
            [],
        )?;

        Ok(set)
    }

    pub fn create_shadow_descriptor_set(
        &self,
        descriptor_set_allocator: Arc<StandardDescriptorSetAllocator>,
        shadow_map: Arc<ImageView>,
        shadow_sampler: Arc<Sampler>,
    ) -> Result<Arc<DescriptorSet>, Box<dyn std::error::Error>> {
        let layout = self
            .layout
            .set_layouts()
            .get(1)
            .ok_or("Missing Set 1 layout")?;
        let set = DescriptorSet::new(
            descriptor_set_allocator,
            layout.clone(),
            [WriteDescriptorSet::image_view_sampler(
                0,
                shadow_map,
                shadow_sampler,
            )],
            [],
        )?;
        Ok(set)
    }

    pub fn create_ssao_descriptor_set(
        &self,
        descriptor_set_allocator: Arc<StandardDescriptorSetAllocator>,
        ssao_texture: Arc<ImageView>,
        ssao_sampler: Arc<Sampler>,
    ) -> Result<Arc<DescriptorSet>, Box<dyn std::error::Error>> {
        let layout = self
            .layout
            .set_layouts()
            .get(2)
            .ok_or("Missing Set 2 layout")?;
        let set = DescriptorSet::new(
            descriptor_set_allocator,
            layout.clone(),
            [WriteDescriptorSet::image_view_sampler(
                0,
                ssao_texture,
                ssao_sampler,
            )],
            [],
        )?;
        Ok(set)
    }

    pub fn pipeline(&self, registry: &PipelineRegistry) -> Arc<GraphicsPipeline> {
        registry.get(self.pipeline_id)
    }

    pub fn layout(&self) -> Arc<PipelineLayout> {
        self.layout.clone()
    }

    pub fn render_pass(&self) -> Arc<RenderPass> {
        self.render_pass.clone()
    }

    /// G-buffer sampling set (Set 0). `None` until `rebind()` runs.
    pub fn gbuffer_set(&self) -> Option<&Arc<DescriptorSet>> {
        self.gbuffer_set.as_ref()
    }

    /// Shadow map set (Set 1). `None` until `rebind()` runs.
    pub fn shadow_set(&self) -> Option<&Arc<DescriptorSet>> {
        self.shadow_set.as_ref()
    }

    /// SSAO set (Set 2). `None` until `rebind()` runs.
    pub fn ssao_set(&self) -> Option<&Arc<DescriptorSet>> {
        self.ssao_set.as_ref()
    }

    /// SSAO fallback set (Set 2, 1x1 white). `None` until `rebind()` runs.
    pub fn ssao_fallback_set(&self) -> Option<&Arc<DescriptorSet>> {
        self.ssao_fallback_set.as_ref()
    }

    /// HDR output framebuffer. `None` until `rebind()` runs.
    pub fn hdr_framebuffer(&self) -> Option<&Arc<Framebuffer>> {
        self.hdr_framebuffer.as_ref()
    }
}

impl DeferredPass for LightingPass {
    fn name(&self) -> &'static str {
        "lighting"
    }

    // No `resize`: the pass owns no size-dependent images — its output target
    // (`hdr_target`) is a shared resource recreated by `DeferredRenderer`.

    fn rebind(&mut self, inputs: &PassInputs) -> Result<(), RenderError> {
        let dsa = inputs.descriptor_set_allocator.clone();
        self.gbuffer_set = Some(self.create_descriptor_set(
            dsa.clone(),
            inputs.gbuffer_position.clone(),
            inputs.gbuffer_normal.clone(),
            inputs.gbuffer_albedo.clone(),
            inputs.gbuffer_material.clone(),
            inputs.gbuffer_emissive.clone(),
        )?);
        self.shadow_set = Some(self.create_shadow_descriptor_set(
            dsa.clone(),
            inputs.shadow_map.clone(),
            inputs.shadow_sampler.clone(),
        )?);
        self.ssao_set = Some(self.create_ssao_descriptor_set(
            dsa.clone(),
            inputs.ssao_blurred.clone(),
            inputs.ssao_sampler.clone(),
        )?);
        self.ssao_fallback_set = Some(self.create_ssao_descriptor_set(
            dsa,
            inputs.ssao_fallback.clone(),
            inputs.ssao_sampler.clone(),
        )?);
        self.hdr_framebuffer = Some(Framebuffer::new(
            self.render_pass.clone(),
            FramebufferCreateInfo {
                attachments: vec![inputs.hdr_target.clone()],
                ..Default::default()
            },
        )?);
        Ok(())
    }
}
