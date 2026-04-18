use super::pool::PlanktonPool;
use smallvec::smallvec;
use std::collections::HashMap;
use std::sync::Arc;
use vulkano::command_buffer::AutoCommandBufferBuilder;
use vulkano::descriptor_set::allocator::StandardDescriptorSetAllocator;
use vulkano::descriptor_set::{DescriptorSet, WriteDescriptorSet};
use vulkano::device::Device;
use vulkano::format::Format;
use vulkano::image::sampler::{Filter, Sampler, SamplerAddressMode, SamplerCreateInfo};
use vulkano::image::view::ImageView;
use vulkano::image::{Image, ImageCreateInfo, ImageType, ImageUsage};
use vulkano::memory::allocator::{AllocationCreateInfo, MemoryTypeFilter, StandardMemoryAllocator};
use vulkano::pipeline::graphics::color_blend::{
    AttachmentBlend, BlendFactor, BlendOp, ColorBlendAttachmentState, ColorBlendState,
};
use vulkano::pipeline::graphics::input_assembly::{InputAssemblyState, PrimitiveTopology};
use vulkano::pipeline::graphics::multisample::MultisampleState;
use vulkano::pipeline::graphics::rasterization::RasterizationState;
use vulkano::pipeline::graphics::vertex_input::VertexInputState;
use vulkano::pipeline::graphics::viewport::ViewportState;
use vulkano::pipeline::graphics::GraphicsPipelineCreateInfo;
use vulkano::pipeline::layout::PipelineDescriptorSetLayoutCreateInfo;
use vulkano::pipeline::{
    DynamicState, GraphicsPipeline, PipelineBindPoint, PipelineLayout,
    PipelineShaderStageCreateInfo,
};
use vulkano::render_pass::RenderPass;

mod plankton_vs {
    vulkano_shaders::shader! {
        ty: "vertex",
        path: "src/engine/rendering/shaders/plankton/plankton_render.vert",
    }
}

mod plankton_fs {
    vulkano_shaders::shader! {
        ty: "fragment",
        path: "src/engine/rendering/shaders/plankton/plankton_render.frag",
    }
}

/// Push constants for the plankton billboard render pass.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct PlanktonRenderPushConstants {
    pub view_projection: [[f32; 4]; 4],
    pub camera_right: [f32; 4],       // .xyz = right, .w = 0
    pub camera_up: [f32; 4],          // .xyz = up, .w = soft_fade_distance
    pub camera_near_far_pad: [f32; 4], // .x = near, .y = far, .zw = 0
}

unsafe impl bytemuck::Pod for PlanktonRenderPushConstants {}
unsafe impl bytemuck::Zeroable for PlanktonRenderPushConstants {}

pub struct PlanktonRenderPass {
    pipeline: Arc<GraphicsPipeline>,
    layout: Arc<PipelineLayout>,
    render_pass: Arc<RenderPass>,
    /// Per-emitter descriptor sets (Set 0: particle buffer + texture)
    emitter_descriptor_sets: HashMap<uuid::Uuid, Arc<DescriptorSet>>,
    /// Shared descriptor set (Set 1: gbuffer depth)
    depth_descriptor_set: Option<Arc<DescriptorSet>>,
    /// 1x1 white fallback texture for emitters without a texture
    fallback_texture: Arc<ImageView>,
    texture_sampler: Arc<Sampler>,
    depth_sampler: Arc<Sampler>,
}

impl PlanktonRenderPass {
    pub fn new(
        device: Arc<Device>,
        allocator: Arc<StandardMemoryAllocator>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        // HDR render pass with load_op: Load (preserves lighting output)
        let render_pass = vulkano::single_pass_renderpass!(
            device.clone(),
            attachments: {
                color: {
                    format: Format::R16G16B16A16_SFLOAT,
                    samples: 1,
                    load_op: Load,
                    store_op: Store,
                }
            },
            pass: {
                color: [color],
                depth_stencil: {}
            }
        )?;

        let vs = plankton_vs::load(device.clone())?
            .entry_point("main")
            .ok_or("missing main entry point in plankton_render.vert")?;
        let fs = plankton_fs::load(device.clone())?
            .entry_point("main")
            .ok_or("missing main entry point in plankton_render.frag")?;

        let stages = [
            PipelineShaderStageCreateInfo::new(vs),
            PipelineShaderStageCreateInfo::new(fs),
        ];

        let layout = PipelineLayout::new(
            device.clone(),
            PipelineDescriptorSetLayoutCreateInfo::from_stages(&stages)
                .into_pipeline_layout_create_info(device.clone())?,
        )?;

        let subpass = vulkano::render_pass::Subpass::from(render_pass.clone(), 0)
            .ok_or("failed to get subpass 0 for plankton render pass")?;

        // Additive blending: src * ONE + dst * ONE
        let blend = AttachmentBlend {
            src_color_blend_factor: BlendFactor::One,
            dst_color_blend_factor: BlendFactor::One,
            color_blend_op: BlendOp::Add,
            src_alpha_blend_factor: BlendFactor::One,
            dst_alpha_blend_factor: BlendFactor::One,
            alpha_blend_op: BlendOp::Add,
        };

        let pipeline = GraphicsPipeline::new(
            device.clone(),
            None,
            GraphicsPipelineCreateInfo {
                stages: smallvec![stages[0].clone(), stages[1].clone()],
                vertex_input_state: Some(VertexInputState::default()),
                input_assembly_state: Some(InputAssemblyState {
                    topology: PrimitiveTopology::TriangleStrip,
                    ..Default::default()
                }),
                viewport_state: Some(ViewportState::default()),
                rasterization_state: Some(RasterizationState::default()),
                multisample_state: Some(MultisampleState::default()),
                depth_stencil_state: None, // No depth buffer in HDR-only pass
                color_blend_state: Some(ColorBlendState::with_attachment_states(
                    1,
                    ColorBlendAttachmentState {
                        blend: Some(blend),
                        ..Default::default()
                    },
                )),
                dynamic_state: [DynamicState::Viewport, DynamicState::Scissor]
                    .into_iter()
                    .collect(),
                subpass: Some(subpass.into()),
                ..GraphicsPipelineCreateInfo::layout(layout.clone())
            },
        )?;

        // Create 1x1 white fallback texture
        let fallback_texture = Self::create_fallback_texture(allocator)?;

        let texture_sampler = Sampler::new(
            device.clone(),
            SamplerCreateInfo {
                mag_filter: Filter::Linear,
                min_filter: Filter::Linear,
                address_mode: [SamplerAddressMode::ClampToEdge; 3],
                ..Default::default()
            },
        )?;

        let depth_sampler = Sampler::new(
            device,
            SamplerCreateInfo {
                mag_filter: Filter::Nearest,
                min_filter: Filter::Nearest,
                address_mode: [SamplerAddressMode::ClampToEdge; 3],
                ..Default::default()
            },
        )?;

        Ok(Self {
            pipeline,
            layout,
            render_pass,
            emitter_descriptor_sets: HashMap::new(),
            depth_descriptor_set: None,
            fallback_texture,
            texture_sampler,
            depth_sampler,
        })
    }

    fn create_fallback_texture(
        allocator: Arc<StandardMemoryAllocator>,
    ) -> Result<Arc<ImageView>, Box<dyn std::error::Error>> {
        let image = Image::new(
            allocator,
            ImageCreateInfo {
                image_type: ImageType::Dim2d,
                format: Format::R8G8B8A8_UNORM,
                extent: [1, 1, 1],
                usage: ImageUsage::SAMPLED | ImageUsage::TRANSFER_DST,
                ..Default::default()
            },
            AllocationCreateInfo {
                memory_type_filter: MemoryTypeFilter::PREFER_DEVICE,
                ..Default::default()
            },
        )?;
        ImageView::new_default(image).map_err(|e| e.into())
    }

    pub fn render_pass(&self) -> Arc<RenderPass> {
        self.render_pass.clone()
    }

    /// Set or update the gbuffer depth descriptor set (Set 1).
    /// Called at construction and on resize.
    pub fn set_gbuffer_depth(
        &mut self,
        descriptor_set_allocator: Arc<StandardDescriptorSetAllocator>,
        depth_view: Arc<ImageView>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let set_layout = self
            .layout
            .set_layouts()
            .get(1)
            .ok_or("missing set 1 layout in plankton render pass")?
            .clone();

        let set = DescriptorSet::new(
            descriptor_set_allocator,
            set_layout,
            [WriteDescriptorSet::image_view_sampler(
                0,
                depth_view,
                self.depth_sampler.clone(),
            )],
            [],
        )?;

        self.depth_descriptor_set = Some(set);
        Ok(())
    }

    /// Cache a per-emitter descriptor set (Set 0: particle buffer + texture).
    pub fn prepare_pool(
        &mut self,
        descriptor_set_allocator: Arc<StandardDescriptorSetAllocator>,
        pool_guid: uuid::Uuid,
        pool: &PlanktonPool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let set_layout = self
            .layout
            .set_layouts()
            .first()
            .ok_or("missing set 0 layout in plankton render pass")?
            .clone();

        let set = DescriptorSet::new(
            descriptor_set_allocator,
            set_layout,
            [
                WriteDescriptorSet::buffer(0, pool.particle_buffer.clone()),
                WriteDescriptorSet::image_view_sampler(
                    1,
                    self.fallback_texture.clone(),
                    self.texture_sampler.clone(),
                ),
            ],
            [],
        )?;

        self.emitter_descriptor_sets.insert(pool_guid, set);
        Ok(())
    }

    /// Draw all emitters' particles as billboards into the HDR framebuffer.
    pub fn draw<L>(
        &self,
        builder: &mut AutoCommandBufferBuilder<L>,
        emitter_guids_and_capacities: &[(uuid::Uuid, u32, PlanktonRenderPushConstants)],
    ) -> Result<(), Box<dyn std::error::Error>> {
        let depth_set = self
            .depth_descriptor_set
            .as_ref()
            .ok_or("plankton render: depth descriptor set not initialized")?;

        for (guid, capacity, push) in emitter_guids_and_capacities {
            let emitter_set = match self.emitter_descriptor_sets.get(guid) {
                Some(s) => s,
                None => continue,
            };

            builder
                .bind_pipeline_graphics(self.pipeline.clone())?
                .bind_descriptor_sets(
                    PipelineBindPoint::Graphics,
                    self.layout.clone(),
                    0,
                    emitter_set.clone(),
                )?
                .bind_descriptor_sets(
                    PipelineBindPoint::Graphics,
                    self.layout.clone(),
                    1,
                    depth_set.clone(),
                )?
                .push_constants(self.layout.clone(), 0, *push)?;

            // 4 vertices (triangle strip quad) x capacity instances
            unsafe {
                builder.draw(4, *capacity, 0, 0)?;
            }
        }

        Ok(())
    }

    pub fn remove_pool(&mut self, pool_guid: &uuid::Uuid) {
        self.emitter_descriptor_sets.remove(pool_guid);
    }
}
