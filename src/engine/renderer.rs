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
use vulkano::swapchain::{Surface, Swapchain};
use vulkano::sync::GpuFuture;
use vulkano::buffer::{Buffer, BufferCreateInfo, BufferUsage, Subbuffer};
use vulkano::memory::allocator::{AllocationCreateInfo, MemoryTypeFilter, StandardMemoryAllocator};
use winit::window::Window;
use crate::engine::render_pass::create_render_pass;
use crate::engine::framebuffer::create_framebuffers;
use crate::engine::pipeline::{create_pipeline, Vertex};

use crate::engine::{
    create_logical_device, create_swapchain, select_physical_device, VulkanContext,
};

pub struct Renderer {
    _instance: Arc<Instance>,
    pub device: Arc<Device>,
    pub queue: Arc<Queue>,
    pub surface: Arc<Surface>,
    pub swapchain: Arc<Swapchain>,
    pub images: Vec<Arc<Image>>,
    pub command_buffer_allocator: Arc<StandardCommandBufferAllocator>,
    pub memory_allocator: Arc<StandardMemoryAllocator>,
    pub vertex_buffer: Subbuffer<[Vertex]>,
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
        let vertices = [
            Vertex { position: [0.0, -0.5], color: [1.0, 0.0, 0.0] },  // Top - Red
            Vertex { position: [0.5, 0.5], color: [0.0, 1.0, 0.0] },   // Right - Green
            Vertex { position: [-0.5, 0.5], color: [0.0, 0.0, 1.0] },  // Left - Blue
        ];

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

        println!("✓ Vertex buffer created (3 vertices)");

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
        })
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