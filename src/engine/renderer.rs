use std::sync::Arc;
use winit::window::Window;
use vulkano::command_buffer::{
    AutoCommandBufferBuilder, CommandBufferUsage, RenderPassBeginInfo,
    SubpassBeginInfo, SubpassEndInfo,
};
use vulkano::command_buffer::allocator::StandardCommandBufferAllocator;
use vulkano::device::{Device, Queue};
use vulkano::image::Image;
use vulkano::instance::Instance;
use vulkano::render_pass::Framebuffer;
use vulkano::swapchain::{self as vk_swapchain, Surface, Swapchain};
use vulkano::{Validated, VulkanError};
use vulkano::sync::{GpuFuture};
use vulkano::buffer::{Buffer, BufferCreateInfo, BufferUsage, Subbuffer};
use vulkano::memory::allocator::{AllocationCreateInfo, MemoryTypeFilter, StandardMemoryAllocator};
use vulkano::pipeline::graphics::viewport::Viewport;
use vulkano::pipeline::{GraphicsPipeline, Pipeline, PipelineLayout, PipelineBindPoint};
use vulkano::descriptor_set::{PersistentDescriptorSet, allocator::StandardDescriptorSetAllocator};
use vulkano::image::view::ImageView;
use vulkano::image::sampler::Sampler;
use crate::engine::render_pass::create_render_pass;
use crate::engine::framebuffer::create_framebuffers;
use crate::engine::pipeline::{
    create_pipeline, create_textured_pipeline, create_texture_descriptor_set, create_transform_pipeline,
    create_camera_pipeline, create_quad_vertices, create_quad_indices, transform_vs, camera_vs,
    Vertex, TexturedVertex
};
use crate::engine::texture::load_texture;
use crate::engine::components::Transform2D;
use crate::engine::{
    create_logical_device, select_physical_device, VulkanContext,
};
use crate::engine::swapchain::{recreate_swapchain, create_swapchain};
use crate::engine::camera::{Camera2D, CameraPushConstants};
use crate::engine::SpriteBatch;

pub struct Renderer {
    _instance: Arc<Instance>,
    pub device: Arc<Device>,
    pub queue: Arc<Queue>,
    pub surface: Arc<Surface>,
    pub swapchain: Arc<Swapchain>,
    pub images: Vec<Arc<Image>>,
    pub command_buffer_allocator: Arc<StandardCommandBufferAllocator>,
    pub memory_allocator: Arc<StandardMemoryAllocator>,
    pub vertex_buffer: Subbuffer<[TexturedVertex]>,
    pub index_buffer: Subbuffer<[u32]>,
    render_pass: Arc<vulkano::render_pass::RenderPass>,
    pub framebuffers: Vec<Arc<Framebuffer>>,
    pub pipeline: Arc<GraphicsPipeline>,
    descriptor_set_allocator: Arc<StandardDescriptorSetAllocator>,
    pub descriptor_set: Arc<PersistentDescriptorSet>,
    // Frame-in-flight tracking
    frames_in_flight: usize,
    current_frame: usize,
    previous_frame_end: Option<Box<dyn GpuFuture>>,
    pub recreate_swapchain: bool,
    pub camera: Camera2D, 
}

impl Renderer {
    pub fn new(window: Arc<Window>) -> Result<Self, Box<dyn std::error::Error>> {
        println!("🎨 Initializing Vulkan renderer...\n");

        let vulkan_context = VulkanContext::new("Rust Engine")?;

        let surface = Surface::from_window(vulkan_context.instance.clone(), window.clone())?;
        println!("✓ Vulkan surface created");
        
        let window_size = window.inner_size();
        let camera = Camera2D::new(window_size.width as f32, window_size.height as f32);

        let physical_device = select_physical_device(vulkan_context.instance.clone())?;

        let device_context = create_logical_device(physical_device, surface.clone())?;

        let (swapchain, images) = create_swapchain(device_context.device.clone(), surface.clone())?;

        let command_buffer_allocator = Arc::new(StandardCommandBufferAllocator::new(
            device_context.device.clone(),
            Default::default(),
        ));

        
        // Create memory allocator
        let memory_allocator = Arc::new(StandardMemoryAllocator::new_default(
            device_context.device.clone(),
        ));

        // Create triangle vertices
        // let vertices = [
        //     Vertex { position: [0.0, -0.5], color: [1.0, 0.0, 0.0] },  // Top - Red
        //     Vertex { position: [0.5, 0.5], color: [0.0, 1.0, 0.0] },   // Right - Green
        //     Vertex { position: [-0.5, 0.5], color: [0.0, 0.0, 1.0] },  // Left - Blue
        // ];

        // Create textured quad vertices
        let vertices = create_quad_vertices();

        // Create quad vertices (indexed version - only 4 vertices)
        let vertices = create_quad_vertices();

        // Create vertex buffer and upload to GPU
        let vertex_buffer = Buffer::from_iter(
            memory_allocator.clone(),
            BufferCreateInfo {
                usage: BufferUsage::VERTEX_BUFFER,
                ..Default::default()
            },
            AllocationCreateInfo {
                memory_type_filter: MemoryTypeFilter::PREFER_DEVICE
                    | MemoryTypeFilter::HOST_SEQUENTIAL_WRITE,
                ..Default::default()
            },
            vertices.iter().cloned(),
        )?;

                // Create index buffer
        let indices = create_quad_indices();

        let index_buffer = Buffer::from_iter(
            memory_allocator.clone(),
            BufferCreateInfo {
                usage: BufferUsage::INDEX_BUFFER,  // Note: INDEX_BUFFER
                ..Default::default()
            },
            AllocationCreateInfo {
                memory_type_filter: MemoryTypeFilter::PREFER_DEVICE
                    | MemoryTypeFilter::HOST_SEQUENTIAL_WRITE,
                ..Default::default()
            },
            indices.iter().cloned(),
        )?;

        println!("✓ Vertex buffer created (4 vertices)");
        println!("✓ Index buffer created (6 indices)");

        // Create render pass
        let render_pass = create_render_pass(device_context.device.clone(), swapchain.clone())?;
        println!("✓ Render pass created");

        // Create framebuffers
        let framebuffers = create_framebuffers(&images, render_pass.clone())?;
        println!("✓ Framebuffers created ({} framebuffers)", framebuffers.len());

        // Create pipeline
        let viewport = Viewport {
            offset: [0.0, 0.0],
            extent: window.inner_size().into(),
            depth_range: 0.0..=1.0,
        };

        // Create camera pipeline with view-projection support
        let pipeline = create_camera_pipeline(
            device_context.device.clone(),
            render_pass.clone(),
            viewport,
        )?;
        
        // let pipeline = create_pipeline(device_context.device.clone(), render_pass.clone(), viewport)?;
        // println!("✓ Graphics pipeline created");

        // Load texture
        let (texture_view, sampler) = load_texture(
            device_context.device.clone(),
            device_context.queue.clone(),
            &command_buffer_allocator,
            memory_allocator.clone(),
            "assets/sprite.png",
        )?;
                
        // Create descriptor set allocator
        let descriptor_set_allocator = Arc::new(StandardDescriptorSetAllocator::new(
            device_context.device.clone(),
            Default::default(),
        ));

        // Create descriptor set for texture
        let descriptor_set = create_texture_descriptor_set(
            descriptor_set_allocator.clone(),
            pipeline.clone(),
            texture_view,
            sampler,
        )?;

        println!("\n✅ Renderer initialized successfully!\n");

        Ok(Self {
            _instance: vulkan_context.instance,
            device: device_context.device,
            queue: device_context.queue,
            surface,
            swapchain,
            images,
            command_buffer_allocator,
            memory_allocator,
            vertex_buffer,
            index_buffer,
            render_pass,
            framebuffers,
            pipeline,
            descriptor_set_allocator,
            descriptor_set,
            frames_in_flight: 2,
            current_frame: 0,
            previous_frame_end: None,
            recreate_swapchain: false,
            camera
        })
    }

    pub fn render(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        // Wait for previous frame and cleanup
        if let Some(mut previous) = self.previous_frame_end.take() {
            previous.cleanup_finished();
        }

        // Check if window is minimized (zero size) - skip rendering
        let extent = self.swapchain.image_extent();
        if extent[0] == 0 || extent[1] == 0 {
            println!("⏸️  Window minimized ({}x{}), skipping render", extent[0], extent[1]);
            // Window is minimized, skip rendering but keep the future alive
            self.previous_frame_end = Some(vulkano::sync::now(self.device.clone()).boxed());
            return Ok(());
        }

        // Check if we need to recreate swapchain
        if self.recreate_swapchain {
            let (new_swapchain, new_images) = recreate_swapchain(
                self.device.clone(),
                self.surface.clone(),
                self.swapchain.clone(),
            )?;

            // If images is empty, window is minimized - keep old swapchain and skip rendering
            if new_images.is_empty() {
                self.recreate_swapchain = false; // Reset flag but keep old swapchain
                self.previous_frame_end = Some(vulkano::sync::now(self.device.clone()).boxed());
                return Ok(());
            }

            self.swapchain = new_swapchain;
            self.images = new_images;

            // Recreate framebuffers for new images
            self.framebuffers = create_framebuffers(&self.images, self.render_pass.clone())?;

            // Update camera viewport to match new swapchain size
            let extent = self.images[0].extent();
            self.camera.set_viewport_size(extent[0] as f32, extent[1] as f32);

            self.recreate_swapchain = false;
        }

        // Acquire next image
        let (image_index, suboptimal, acquire_future) =
            match vk_swapchain::acquire_next_image(self.swapchain.clone(), None) {
                Ok(r) => r,
                Err(Validated::Error(VulkanError::OutOfDate)) => {
                    // Swapchain is out of date (window resized)
                    self.recreate_swapchain = true;
                    return Ok(());
                }
                Err(e) => return Err(e.into()),
            };

        // If suboptimal, recreate on next frame
        if suboptimal {
            self.recreate_swapchain = true;
        }

        // Build command buffer to draw triangle
        let mut builder = AutoCommandBufferBuilder::primary(
            &self.command_buffer_allocator,
            self.queue.queue_family_index(),
            CommandBufferUsage::OneTimeSubmit,
        )?;

        // Begin render pass
        builder
            .begin_render_pass(
                RenderPassBeginInfo {
                    clear_values: vec![Some([0.0, 0.0, 0.0, 1.0].into())], // Black background
                    ..RenderPassBeginInfo::framebuffer(
                        self.framebuffers[image_index as usize].clone()
                    )
                },
                SubpassBeginInfo::default(),
            )?
            // Bind pipeline
            .bind_pipeline_graphics(self.pipeline.clone())?
            .bind_descriptor_sets(
            PipelineBindPoint::Graphics,
            self.pipeline.layout().clone(),
            0,
            self.descriptor_set.clone(),
            )?
            // Bind vertex buffer
            .bind_vertex_buffers(0, self.vertex_buffer.clone())?
            .bind_index_buffer(self.index_buffer.clone())? 
            .draw_indexed(6, 1, 0, 0, 0)?
            // End render pass
            .end_render_pass(SubpassEndInfo::default())?;

        let command_buffer = builder.build()?;

        // Execute command buffer and present
        let future = acquire_future
            .then_execute(self.queue.clone(), command_buffer)?
            .then_swapchain_present(
                self.queue.clone(),
                vk_swapchain::SwapchainPresentInfo::swapchain_image_index(
                    self.swapchain.clone(),
                    image_index,
                ),
            )
            .then_signal_fence_and_flush();

        match future {
            Ok(future) => {
                self.previous_frame_end = Some(future.boxed());
            }
            Err(e) => {
                println!("Failed to flush future: {:?}", e);
                self.previous_frame_end = None;
            }
        }

        Ok(())
    }

        
    /// Renders a sprite with 2D transform
    pub fn render_sprite(
        &mut self,
        transform: Transform2D,
        descriptor_set: Arc<PersistentDescriptorSet>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // 1. Acquire image
        let (image_index, _, acquire_future) =
            vulkano::swapchain::acquire_next_image(self.swapchain.clone(), None)?;

        // 2. Build command buffer
        let mut builder = AutoCommandBufferBuilder::primary(
            &self.command_buffer_allocator,
            self.queue.queue_family_index(),
            CommandBufferUsage::OneTimeSubmit,
        )?;

        // 3. Create push constants data
        let push_constants = transform_vs::PushConstants {
            pos: transform.position,
            rotation: transform.rotation.into(),
            scale: transform.scale,
        };

        // 4. Begin render pass and draw
        builder
            .begin_render_pass(
                RenderPassBeginInfo {
                    clear_values: vec![Some([0.0, 0.0, 0.0, 1.0].into())],
                    ..RenderPassBeginInfo::framebuffer(self.framebuffers[image_index as usize].clone())
                },
                SubpassBeginInfo::default(),
            )?
            .bind_pipeline_graphics(self.pipeline.clone())?
            .bind_descriptor_sets(
                vulkano::pipeline::PipelineBindPoint::Graphics,
                self.pipeline.layout().clone(),
                0,
                descriptor_set.clone(),
            )?
            .push_constants(self.pipeline.layout().clone(), 0, push_constants)?  // NEW: Push constants
            .bind_vertex_buffers(0, self.vertex_buffer.clone())?
            .bind_index_buffer(self.index_buffer.clone())?
            .draw_indexed(6, 1, 0, 0, 0)?
            .end_render_pass(SubpassEndInfo::default())?;

        // 5. Build and submit
        let command_buffer = builder.build()?;

        let future = acquire_future
            .then_execute(self.queue.clone(), command_buffer)?
            .then_swapchain_present(
                self.queue.clone(),
                vulkano::swapchain::SwapchainPresentInfo::swapchain_image_index(
                    self.swapchain.clone(),
                    image_index,
                ),
            )
            .then_signal_fence_and_flush()?;

        future.wait(None)?;

        Ok(())
    }

    /// Renders multiple sprites in one frame
    pub fn render_sprites(
        &mut self,
        sprites: &[(Transform2D, Arc<PersistentDescriptorSet>)],
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Wait for previous frame and cleanup
        if let Some(mut previous) = self.previous_frame_end.take() {
            previous.cleanup_finished();
        }

        // Check if window is minimized (zero size) - skip rendering
        let extent = self.swapchain.image_extent();
        if extent[0] == 0 || extent[1] == 0 {
            println!("⏸️  Window minimized ({}x{}), skipping render", extent[0], extent[1]);
            // Window is minimized, skip rendering but keep the future alive
            self.previous_frame_end = Some(vulkano::sync::now(self.device.clone()).boxed());
            return Ok(());
        }

        // Check if we need to recreate swapchain
        if self.recreate_swapchain {
            let (new_swapchain, new_images) = recreate_swapchain(
                self.device.clone(),
                self.surface.clone(),
                self.swapchain.clone(),
            )?;

            // If images is empty, window is minimized - keep old swapchain and skip rendering
            if new_images.is_empty() {
                self.recreate_swapchain = false; // Reset flag but keep old swapchain
                self.previous_frame_end = Some(vulkano::sync::now(self.device.clone()).boxed());
                return Ok(());
            }

            self.swapchain = new_swapchain;
            self.images = new_images;

            // Recreate framebuffers for new images
            self.framebuffers = create_framebuffers(&self.images, self.render_pass.clone())?;

            // Update camera viewport to match new swapchain size
            let extent = self.images[0].extent();
            self.camera.set_viewport_size(extent[0] as f32, extent[1] as f32);

            self.recreate_swapchain = false;
        }

        // Acquire next image
        let (image_index, suboptimal, acquire_future) =
            match vk_swapchain::acquire_next_image(self.swapchain.clone(), None) {
                Ok(r) => r,
                Err(Validated::Error(VulkanError::OutOfDate)) => {
                    self.recreate_swapchain = true;
                    return Ok(());
                }
                Err(e) => return Err(e.into()),
            };

        // If suboptimal, recreate on next frame
        if suboptimal {
            self.recreate_swapchain = true;
        }

        // Build command buffer
        let mut builder = AutoCommandBufferBuilder::primary(
            &self.command_buffer_allocator,
            self.queue.queue_family_index(),
            CommandBufferUsage::OneTimeSubmit,
        )?;

        // Begin render pass
        builder.begin_render_pass(
            RenderPassBeginInfo {
                clear_values: vec![Some([0.1, 0.1, 0.1, 1.0].into())],
                ..RenderPassBeginInfo::framebuffer(
                    self.framebuffers[image_index as usize].clone()
                )
            },
            SubpassBeginInfo::default(),
        )?;

        builder.bind_pipeline_graphics(self.pipeline.clone())?;

        // Set dynamic viewport (updates with window resize)
        let extent = self.swapchain.image_extent();
        builder.set_viewport(0, [Viewport {
            offset: [0.0, 0.0],
            extent: [extent[0] as f32, extent[1] as f32],
            depth_range: 0.0..=1.0,
        }].into_iter().collect())?;

        builder.bind_vertex_buffers(0, self.vertex_buffer.clone())?;
        builder.bind_index_buffer(self.index_buffer.clone())?;

        // Get camera view-projection matrix
        let camera_vp = self.camera.view_projection_matrix();

        // Draw each sprite
        for (transform, texture_descriptor) in sprites {
            let push_constants = camera_vs::PushConstants {
                view_projection: camera_vp.to_cols_array_2d(),
                pos: transform.position,
                rotation: transform.rotation.into(),
                scale: transform.scale,
            };

            builder
                .bind_descriptor_sets(
                    PipelineBindPoint::Graphics,
                    self.pipeline.layout().clone(),
                    0,
                    texture_descriptor.clone(),
                )?
                .push_constants(self.pipeline.layout().clone(), 0, push_constants)?
                .draw_indexed(6, 1, 0, 0, 0)?;
        }

        // End render pass
        builder.end_render_pass(SubpassEndInfo::default())?;

        let command_buffer = builder.build()?;

        // Execute command buffer and present (DO NOT JOIN - just like render())
        let future = acquire_future
            .then_execute(self.queue.clone(), command_buffer)?
            .then_swapchain_present(
                self.queue.clone(),
                vk_swapchain::SwapchainPresentInfo::swapchain_image_index(
                    self.swapchain.clone(),
                    image_index,
                ),
            )
            .then_signal_fence_and_flush();

        match future {
            Ok(future) => {
                self.previous_frame_end = Some(future.boxed());
            }
            Err(e) => {
                println!("Failed to flush future: {:?}", e);
                self.previous_frame_end = None;
            }
        }

        Ok(())
    }

    /// Renders sprites using batching (more efficient)
    pub fn render_sprite_batch(
        &mut self,
        batch: &SpriteBatch,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Wait for previous frame and cleanup
        if let Some(mut previous) = self.previous_frame_end.take() {
            previous.cleanup_finished();
        }

        // Check if we need to recreate swapchain
        if self.recreate_swapchain {
            let (new_swapchain, new_images) = recreate_swapchain(
                self.device.clone(),
                self.surface.clone(),
                self.swapchain.clone(),
            )?;
            self.swapchain = new_swapchain;
            self.images = new_images;
            self.framebuffers = create_framebuffers(&self.images, self.render_pass.clone())?;
            self.recreate_swapchain = false;

            // Update camera viewport
            let extent = self.images[0].extent();
            self.camera.set_viewport_size(extent[0] as f32, extent[1] as f32);
        }

        // Acquire next image
        let (image_index, suboptimal, acquire_future) =
            match vk_swapchain::acquire_next_image(self.swapchain.clone(), None) {
                Ok(r) => r,
                Err(Validated::Error(VulkanError::OutOfDate)) => {
                    self.recreate_swapchain = true;
                    return Ok(());
                }
                Err(e) => return Err(e.into()),
            };

        if suboptimal {
            self.recreate_swapchain = true;
        }

        // Build command buffer
        let mut builder = AutoCommandBufferBuilder::primary(
            &self.command_buffer_allocator,
            self.queue.queue_family_index(),
            CommandBufferUsage::OneTimeSubmit,
        )?;

        // Begin render pass
        builder.begin_render_pass(
            RenderPassBeginInfo {
                clear_values: vec![Some([0.1, 0.1, 0.1, 1.0].into())],
                ..RenderPassBeginInfo::framebuffer(
                    self.framebuffers[image_index as usize].clone()
                )
            },
            SubpassBeginInfo::default(),
        )?;

        // Bind pipeline and buffers (once for all sprites)
        builder.bind_pipeline_graphics(self.pipeline.clone())?;

        // Set dynamic viewport (updates with window resize)
        let extent = self.swapchain.image_extent();
        builder.set_viewport(0, [Viewport {
            offset: [0.0, 0.0],
            extent: [extent[0] as f32, extent[1] as f32],
            depth_range: 0.0..=1.0,
        }].into_iter().collect())?;

        builder.bind_vertex_buffers(0, self.vertex_buffer.clone())?;
        builder.bind_index_buffer(self.index_buffer.clone())?;

        // Get camera view-projection matrix
        let camera_vp = self.camera.view_projection_matrix();

        // Draw batched sprites
        for (descriptor_set, transforms) in batch.iter_batches() {
            // Bind texture once per batch
            builder.bind_descriptor_sets(
                PipelineBindPoint::Graphics,
                self.pipeline.layout().clone(),
                0,
                descriptor_set,
            )?;

            // Draw all sprites with this texture
            for transform in transforms {
                let push_constants = camera_vs::PushConstants {
                    view_projection: camera_vp.to_cols_array_2d(),
                    pos: transform.position,
                    rotation: transform.rotation.into(),
                    scale: transform.scale,
                };

                builder
                    .push_constants(self.pipeline.layout().clone(), 0, push_constants)?
                    .draw_indexed(6, 1, 0, 0, 0)?;
            }
        }

        // End render pass
        builder.end_render_pass(SubpassEndInfo::default())?;

        let command_buffer = builder.build()?;

        // Execute and present
        let future = acquire_future
            .then_execute(self.queue.clone(), command_buffer)?
            .then_swapchain_present(
                self.queue.clone(),
                vk_swapchain::SwapchainPresentInfo::swapchain_image_index(
                    self.swapchain.clone(),
                    image_index,
                ),
            )
            .then_signal_fence_and_flush();

        match future {
            Ok(future) => {
                self.previous_frame_end = Some(future.boxed());
            }
            Err(e) => {
                println!("Failed to flush future: {:?}", e);
                self.previous_frame_end = None;
            }
        }

        Ok(())
    }
}
