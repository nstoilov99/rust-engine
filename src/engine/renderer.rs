use std::sync::Arc;
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
use vulkano::sync::{self, GpuFuture};
use vulkano::buffer::{Buffer, BufferCreateInfo, BufferUsage, Subbuffer};
use vulkano::memory::allocator::{AllocationCreateInfo, MemoryTypeFilter, StandardMemoryAllocator};
use winit::window::Window;
use crate::engine::render_pass::create_render_pass;
use crate::engine::framebuffer::create_framebuffers;
use crate::engine::pipeline::{create_pipeline, create_textured_pipeline, create_texture_descriptor_set, create_quad_vertices, Vertex, TexturedVertex};
use crate::engine::texture::load_texture;
use vulkano::pipeline::graphics::viewport::Viewport;
use vulkano::pipeline::{GraphicsPipeline, Pipeline, PipelineBindPoint};
use vulkano::descriptor_set::{PersistentDescriptorSet, allocator::StandardDescriptorSetAllocator};
use vulkano::image::view::ImageView;
use vulkano::image::sampler::Sampler;

use crate::engine::{
    create_logical_device, select_physical_device, VulkanContext,
};
use crate::engine::swapchain::{recreate_swapchain, create_swapchain};

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
    render_pass: Arc<vulkano::render_pass::RenderPass>,
    framebuffers: Vec<Arc<Framebuffer>>,
    pipeline: Arc<GraphicsPipeline>,
    descriptor_set_allocator: Arc<StandardDescriptorSetAllocator>,
    descriptor_set: Arc<PersistentDescriptorSet>, 
    // Frame-in-flight tracking
    frames_in_flight: usize,
    current_frame: usize,
    previous_frame_end: Option<Box<dyn GpuFuture>>,
    recreate_swapchain: bool,
}

impl Renderer {
    pub fn new(window: Arc<Window>) -> Result<Self, Box<dyn std::error::Error>> {
        println!("🎨 Initializing Vulkan renderer...\n");

        let vulkan_context = VulkanContext::new("Rust Engine")?;

        let surface = Surface::from_window(vulkan_context.instance.clone(), window.clone())?;
        println!("✓ Vulkan surface created");

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

        println!("✓ Vertex buffer created (6 vertices for textured quad)");

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

        // Create textured pipeline
        let pipeline = create_textured_pipeline(
        device_context.device.clone(),
        render_pass.clone(),
        viewport,
        )?;
        println!("✓ Graphics pipeline created (textured)");
        
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
            render_pass,
            framebuffers,
            pipeline,
            descriptor_set_allocator,  // NEW
            descriptor_set, 
            frames_in_flight: 2,  // 2 frames in flight (common choice)
            current_frame: 0,
            previous_frame_end: None,  // Will be set after first frame
            recreate_swapchain: false,
        })
    }

    pub fn render(&mut self) -> Result<(), Box<dyn std::error::Error>> {
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

            // Recreate framebuffers for new images
            self.framebuffers = create_framebuffers(&self.images, self.render_pass.clone())?;

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
            .draw(6, 1, 0, 0)?  // Draw 6 vertices (was 3)
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
}

/// Renders a frame: acquire image, clear to color, present
pub fn render_frame(
    command_buffer_allocator: &StandardCommandBufferAllocator,
    queue: Arc<Queue>,
    swapchain: Arc<Swapchain>,
    framebuffers: &[Arc<Framebuffer>],
) -> Result<(), Box<dyn std::error::Error>> {
    // 1. Acquire next image from swapchain
    let (image_index, _suboptimal, acquire_future) =
        vulkano::swapchain::acquire_next_image(swapchain.clone(), None)?;

    // 2. Create command buffer
    let mut builder = AutoCommandBufferBuilder::primary(
        command_buffer_allocator,
        queue.queue_family_index(),
        CommandBufferUsage::OneTimeSubmit,
    )?;

    // 3. Begin render pass
    builder.begin_render_pass(
        RenderPassBeginInfo {
            clear_values: vec![Some([0.0, 0.5, 1.0, 1.0].into())], // Sky blue color
            ..RenderPassBeginInfo::framebuffer(framebuffers[image_index as usize].clone())
        },
        SubpassBeginInfo::default(),
    )?;

    // 4. End render pass
    builder.end_render_pass(SubpassEndInfo::default())?;

    // 5. Build command buffer
    let command_buffer = builder.build()?;

    // 6. Submit to GPU
    let future = acquire_future
        .then_execute(queue.clone(), command_buffer)?
        .then_swapchain_present(
            queue.clone(),
            vulkano::swapchain::SwapchainPresentInfo::swapchain_image_index(
                swapchain.clone(),
                image_index,
            ),
        )
        .then_signal_fence_and_flush()?;

    // 7. Wait for GPU
    future.wait(None)?;

    Ok(())
}

/// Renders a triangle: acquire image, draw triangle, present
pub fn render_triangle(
    command_buffer_allocator: &StandardCommandBufferAllocator,
    queue: Arc<Queue>,
    swapchain: Arc<Swapchain>,
    framebuffers: &[Arc<Framebuffer>],
    pipeline: Arc<vulkano::pipeline::GraphicsPipeline>,
    vertex_buffer: Subbuffer<[Vertex]>,
) -> Result<(), Box<dyn std::error::Error>> {
    // 1. Acquire image
    let (image_index, _, acquire_future) =
        vulkano::swapchain::acquire_next_image(swapchain.clone(), None)?;

    // 2. Build command buffer
    let mut builder = AutoCommandBufferBuilder::primary(
        command_buffer_allocator,
        queue.queue_family_index(),
        CommandBufferUsage::OneTimeSubmit,
    )?;

    // 3. Begin render pass
    builder
        .begin_render_pass(
            RenderPassBeginInfo {
                clear_values: vec![Some([0.0, 0.0, 0.0, 1.0].into())], // Black background
                ..RenderPassBeginInfo::framebuffer(framebuffers[image_index as usize].clone())
            },
            SubpassBeginInfo::default(),
        )?
        // 4. Bind pipeline
        .bind_pipeline_graphics(pipeline.clone())?
        // 5. Bind vertex buffer
        .bind_vertex_buffers(0, vertex_buffer.clone())?
        // 6. Draw!
        .draw(3, 1, 0, 0)?  // 3 vertices, 1 instance, first vertex 0, first instance 0
        // 7. End render pass
        .end_render_pass(SubpassEndInfo::default())?;

    // 8. Build and submit
    let command_buffer = builder.build()?;

    let future = acquire_future
        .then_execute(queue.clone(), command_buffer)?
        .then_swapchain_present(
            queue.clone(),
            vulkano::swapchain::SwapchainPresentInfo::swapchain_image_index(
                swapchain.clone(),
                image_index,
            ),
        )
        .then_signal_fence_and_flush()?;

    future.wait(None)?;

    Ok(())
}