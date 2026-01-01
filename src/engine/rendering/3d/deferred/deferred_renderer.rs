//! Deferred rendering orchestration
//!
//! Performance optimizations:
//! - Cached G-Buffer descriptor set (created once on init/resize)
//! - Cached swapchain framebuffers (one per swapchain image)

use super::gbuffer::GBuffer;
use super::geometry_pass::GeometryPass;
use super::lighting_pass::LightingPass;
use smallvec::smallvec;
use std::collections::HashMap;
use std::sync::Arc;
use vulkano::buffer::Subbuffer;
use vulkano::command_buffer::{
    allocator::StandardCommandBufferAllocator, AutoCommandBufferBuilder, CommandBufferUsage,
    PrimaryAutoCommandBuffer, RenderPassBeginInfo, SubpassBeginInfo, SubpassContents,
    SubpassEndInfo,
};
use vulkano::descriptor_set::allocator::StandardDescriptorSetAllocator;
use vulkano::descriptor_set::DescriptorSet;
use vulkano::device::{Device, Queue};
use vulkano::memory::allocator::StandardMemoryAllocator;
use vulkano::pipeline::PipelineBindPoint;
use vulkano::render_pass::{Framebuffer, FramebufferCreateInfo, RenderPass};
use vulkano::image::Image;
use vulkano::image::view::ImageView;

pub struct DeferredRenderer {
    gbuffer: GBuffer,
    geometry_pass: GeometryPass,
    lighting_pass: LightingPass,
    device: Arc<Device>,
    queue: Arc<Queue>,
    allocator: Arc<StandardMemoryAllocator>,
    command_buffer_allocator: Arc<StandardCommandBufferAllocator>,
    descriptor_set_allocator: Arc<StandardDescriptorSetAllocator>,
    debug_view: DebugView,
    // Cached resources for performance
    gbuffer_descriptor_set: Arc<DescriptorSet>,
    framebuffer_cache: HashMap<usize, Arc<Framebuffer>>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DebugView {
    None,     // Normal rendering
    Position, // Show world positions (colorful = far from origin)
    Normal,   // Show normals (RGB = XYZ)
    Albedo,   // Show base color
    Material, // Show roughness/metallic
    Depth,    // Show depth (white = near, black = far)
}

impl DeferredRenderer {
    pub fn new(
        device: Arc<Device>,
        queue: Arc<Queue>,
        allocator: Arc<StandardMemoryAllocator>,
        command_buffer_allocator: Arc<StandardCommandBufferAllocator>,
        descriptor_set_allocator: Arc<StandardDescriptorSetAllocator>,
        width: u32,
        height: u32,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let gbuffer = GBuffer::new(device.clone(), allocator.clone(), width, height)?;
        let geometry_pass = GeometryPass::new(device.clone(), gbuffer.render_pass.clone())?;

        // Create a separate render pass for lighting (outputs to swapchain, no depth)
        use vulkano::format::Format;

        let lighting_render_pass = vulkano::single_pass_renderpass!(
            device.clone(),
            attachments: {
                color: {
                    format: Format::B8G8R8A8_SRGB,
                    samples: 1,
                    load_op: Clear,
                    store_op: Store,
                }
            },
            pass: {
                color: [color],
                depth_stencil: {}
            }
        )?;

        let lighting_pass = LightingPass::new(device.clone(), lighting_render_pass)?;

        // Cache G-Buffer descriptor set (created once, reused every frame)
        let gbuffer_descriptor_set = lighting_pass.create_descriptor_set(
            descriptor_set_allocator.clone(),
            gbuffer.position.clone(),
            gbuffer.normal.clone(),
            gbuffer.albedo.clone(),
            gbuffer.material.clone(),
        )?;

        Ok(Self {
            gbuffer,
            geometry_pass,
            lighting_pass,
            device,
            queue,
            allocator,
            command_buffer_allocator,
            descriptor_set_allocator,
            debug_view: DebugView::None,
            gbuffer_descriptor_set,
            framebuffer_cache: HashMap::new(),
        })
    }

    /// Get or create a cached framebuffer for the given swapchain image
    fn get_or_create_framebuffer(
        &mut self,
        target_image: Arc<Image>,
    ) -> Result<Arc<Framebuffer>, Box<dyn std::error::Error>> {
        // Use image pointer as cache key
        let cache_key = Arc::as_ptr(&target_image) as usize;

        if let Some(fb) = self.framebuffer_cache.get(&cache_key) {
            return Ok(fb.clone());
        }

        // Create new framebuffer and cache it
        let target_view = ImageView::new_default(target_image)?;
        let framebuffer = Framebuffer::new(
            self.lighting_pass.render_pass(),
            FramebufferCreateInfo {
                attachments: vec![target_view],
                ..Default::default()
            },
        )?;

        self.framebuffer_cache.insert(cache_key, framebuffer.clone());
        Ok(framebuffer)
    }

    /// Clear the framebuffer cache (call on swapchain recreation)
    pub fn clear_framebuffer_cache(&mut self) {
        self.framebuffer_cache.clear();
    }

    /// Resize the G-Buffer (call when viewport size changes)
    pub fn resize(&mut self, width: u32, height: u32) -> Result<(), Box<dyn std::error::Error>> {
        // Skip if dimensions are invalid
        if width == 0 || height == 0 {
            return Ok(());
        }

        // Check current G-Buffer size - skip if already the right size
        let current_extent = self.gbuffer.position.image().extent();
        if current_extent[0] == width && current_extent[1] == height {
            return Ok(());
        }

        // Recreate G-Buffer with new dimensions
        self.gbuffer = GBuffer::new(
            self.device.clone(),
            self.allocator.clone(),
            width,
            height,
        )?;

        // Recreate the G-Buffer descriptor set for lighting pass
        self.gbuffer_descriptor_set = self.lighting_pass.create_descriptor_set(
            self.descriptor_set_allocator.clone(),
            self.gbuffer.position.clone(),
            self.gbuffer.normal.clone(),
            self.gbuffer.albedo.clone(),
            self.gbuffer.material.clone(),
        )?;

        // Clear framebuffer cache as well
        self.framebuffer_cache.clear();

        Ok(())
    }

    /// Render scene using deferred pipeline
    pub fn render(
        &mut self,
        mesh_data: &[MeshRenderData],         // Your mesh data structure
        light_data: &LightUniformData,        // Your light data structure
        target_image: Arc<Image>, // Swapchain image
    ) -> Result<Arc<PrimaryAutoCommandBuffer>, Box<dyn std::error::Error>> {
        crate::profile_function!();

        // Get cached framebuffer for this swapchain image
        let target_framebuffer = self.get_or_create_framebuffer(target_image)?;
        let mut builder = AutoCommandBufferBuilder::primary(
            self.command_buffer_allocator.clone(),
            self.queue.queue_family_index(),
            CommandBufferUsage::OneTimeSubmit,
        )?;

        // ========== PASS 1: Geometry Pass (Render to G-Buffer) ==========
        {
            crate::profile_scope!("geometry_pass");

            // Get G-Buffer dimensions for viewport
            let gbuffer_extent = self.gbuffer.framebuffer.extent();
            let viewport = vulkano::pipeline::graphics::viewport::Viewport {
                offset: [0.0, 0.0],
                extent: [gbuffer_extent[0] as f32, gbuffer_extent[1] as f32],
                depth_range: 0.0..=1.0,
            };
            let scissor = vulkano::pipeline::graphics::viewport::Scissor {
                offset: [0, 0],
                extent: [gbuffer_extent[0], gbuffer_extent[1]],
            };

            builder
                .begin_render_pass(
                    RenderPassBeginInfo {
                        clear_values: vec![
                            Some([0.0, 0.0, 0.0, 1.0].into()), // Position
                            Some([0.0, 0.0, 0.0, 1.0].into()), // Normal
                            Some([0.0, 0.0, 0.0, 1.0].into()), // Albedo
                            Some([0.0, 0.0, 0.0, 1.0].into()), // Material
                            Some(1.0.into()),                  // Depth
                        ],
                        ..RenderPassBeginInfo::framebuffer(self.gbuffer.framebuffer.clone())
                    },
                    SubpassBeginInfo {
                        contents: SubpassContents::Inline,
                        ..Default::default()
                    },
                )?
                .bind_pipeline_graphics(self.geometry_pass.pipeline())?
                .set_viewport(0, smallvec![viewport.clone()])?
                .set_scissor(0, smallvec![scissor.clone()])?;

            // Render all meshes to G-Buffer
            {
                crate::profile_scope!("mesh_loop");
                for mesh in mesh_data {
                    builder
                        .bind_vertex_buffers(0, mesh.vertex_buffer.clone())?
                        .bind_index_buffer(mesh.index_buffer.clone())?
                        .push_constants(
                            self.geometry_pass.layout(),
                            0,
                            mesh.push_constants, // Model + view-projection matrices
                        )?;
                    unsafe { builder.draw_indexed(mesh.index_count, 1, 0, 0, 0)?; }
                }
            }

            builder.end_render_pass(SubpassEndInfo::default())?;
        }

        // ========== PASS 2: Lighting Pass (Read G-Buffer, Output to Screen) ==========
        {
            crate::profile_scope!("lighting_pass");

            // Use cached G-Buffer descriptor set (no per-frame allocation)
            let gbuffer_descriptor_set = self.gbuffer_descriptor_set.clone();

            // Get target framebuffer dimensions for lighting pass viewport
            let target_extent = target_framebuffer.extent();
            let target_viewport = vulkano::pipeline::graphics::viewport::Viewport {
                offset: [0.0, 0.0],
                extent: [target_extent[0] as f32, target_extent[1] as f32],
                depth_range: 0.0..=1.0,
            };
            let target_scissor = vulkano::pipeline::graphics::viewport::Scissor {
                offset: [0, 0],
                extent: [target_extent[0], target_extent[1]],
            };

            builder
                .begin_render_pass(
                    RenderPassBeginInfo {
                        clear_values: vec![Some([0.0, 0.0, 0.0, 1.0].into())],
                        ..RenderPassBeginInfo::framebuffer(target_framebuffer)
                    },
                    SubpassBeginInfo {
                        contents: SubpassContents::Inline,
                        ..Default::default()
                    },
                )?
                .bind_pipeline_graphics(self.lighting_pass.pipeline())?
                .set_viewport(0, smallvec![target_viewport.clone()])?
                .set_scissor(0, smallvec![target_scissor.clone()])?
                .bind_descriptor_sets(
                    PipelineBindPoint::Graphics,
                    self.lighting_pass.layout(),
                    0,
                    gbuffer_descriptor_set,
                )?
                .push_constants(
                    self.lighting_pass.layout(),
                    0,
                    *light_data,
                )?;
            // Draw fullscreen triangle (no vertex buffer - generated in shader)
            unsafe { builder.draw(3, 1, 0, 0)?; }
            builder.end_render_pass(SubpassEndInfo::default())?;
        }

        // Build command buffer
        let command_buffer = builder.build()?;

        Ok(command_buffer)
    }

    pub fn set_debug_view(&mut self, view: DebugView) {
        self.debug_view = view;
    }
}

// Helper structures (define these based on your engine)
pub struct MeshRenderData {
    pub vertex_buffer: Subbuffer<[crate::engine::rendering::rendering_3d::Vertex3D]>,
    pub index_buffer: Subbuffer<[u32]>,
    pub index_count: u32,
    pub push_constants: PushConstantData,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PushConstantData {
    pub model: [[f32; 4]; 4],
    pub view_projection: [[f32; 4]; 4],
}

unsafe impl bytemuck::Pod for PushConstantData {}
unsafe impl bytemuck::Zeroable for PushConstantData {}

/// Light data for deferred lighting pass (push constants).
///
/// Note: This layout matches the `lighting.frag` shader's push_constant block.
/// It differs from `LightingUniformData` in pipeline_3d.rs which is used for
/// forward rendering with additional metallic/roughness fields.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct LightUniformData {
    pub camera_position: [f32; 3],
    pub _pad0: f32,
    pub directional_light_dir: [f32; 3],
    pub _pad1: f32,
    pub directional_light_color: [f32; 3],
    pub directional_light_intensity: f32,
    pub ambient_color: [f32; 3],
    pub ambient_intensity: f32,
}

unsafe impl bytemuck::Pod for LightUniformData {}
unsafe impl bytemuck::Zeroable for LightUniformData {}
