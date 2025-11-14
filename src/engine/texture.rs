use std::sync::Arc;
use vulkano::device::{Device, Queue};
use vulkano::image::{Image, ImageCreateInfo, ImageType, ImageUsage};
use vulkano::image::view::ImageView;
use vulkano::format::Format;
use vulkano::memory::allocator::{AllocationCreateInfo, MemoryTypeFilter, StandardMemoryAllocator};
use vulkano::buffer::{Buffer, BufferCreateInfo, BufferUsage};
use vulkano::command_buffer::{
    AutoCommandBufferBuilder, CommandBufferUsage, CopyBufferToImageInfo,
    PrimaryCommandBufferAbstract,
};
use vulkano::command_buffer::allocator::StandardCommandBufferAllocator;
use vulkano::sync::{self, GpuFuture};
use vulkano::image::sampler::{Sampler, SamplerCreateInfo, Filter, SamplerAddressMode};

/// Loads image from file, uploads to GPU, and returns ImageView + Sampler
pub fn load_texture(
    device: Arc<Device>,
    queue: Arc<Queue>,
    command_buffer_allocator: &StandardCommandBufferAllocator,
    memory_allocator: Arc<StandardMemoryAllocator>,
    path: &str,
) -> Result<(Arc<ImageView>, Arc<Sampler>), Box<dyn std::error::Error>> {
    // Load image with image crate
    let image_data = image::open(path)?.to_rgba8();
    let (width, height) = image_data.dimensions();
    let image_bytes = image_data.into_raw();

    println!("📷 Loaded texture: {} ({}x{})", path, width, height);

    // Create Vulkan image
    let image = Image::new(
        memory_allocator.clone(),
        ImageCreateInfo {
            image_type: ImageType::Dim2d,
            format: Format::R8G8B8A8_SRGB,  // RGBA, 8 bits per channel
            extent: [width, height, 1],
            usage: ImageUsage::TRANSFER_DST | ImageUsage::SAMPLED,  // Can upload data + sample in shader
            ..Default::default()
        },
        AllocationCreateInfo {
            memory_type_filter: MemoryTypeFilter::PREFER_DEVICE,
            ..Default::default()
        },
    )?;

    // Upload image data to GPU
    upload_texture_data(
        command_buffer_allocator,
        queue.clone(),
        memory_allocator.clone(),
        image.clone(),
        &image_bytes,
    )?;

    println!("✓ Texture uploaded to GPU");

    // Create image view (how shaders access the image)
    let image_view = ImageView::new_default(image)?;
    println!("✓ Image view created");

    // Create sampler (texture filtering settings)
    let sampler = create_sampler(device)?;

    Ok((image_view, sampler))
}

/// Uploads image data from CPU to GPU
pub fn upload_texture_data(
    command_buffer_allocator: &StandardCommandBufferAllocator,
    queue: Arc<Queue>,
    memory_allocator: Arc<StandardMemoryAllocator>,
    image: Arc<Image>,
    data: &[u8],
) -> Result<(), Box<dyn std::error::Error>> {
    // Create staging buffer (CPU-accessible)
    let staging_buffer = Buffer::from_iter(
        memory_allocator,
        BufferCreateInfo {
            usage: BufferUsage::TRANSFER_SRC,  // Source for transfer
            ..Default::default()
        },
        AllocationCreateInfo {
            memory_type_filter: MemoryTypeFilter::PREFER_HOST
                | MemoryTypeFilter::HOST_SEQUENTIAL_WRITE,
            ..Default::default()
        },
        data.iter().copied(),
    )?;

    // Create command buffer to copy data
    let mut builder = AutoCommandBufferBuilder::primary(
        command_buffer_allocator,
        queue.queue_family_index(),
        CommandBufferUsage::OneTimeSubmit,
    )?;

    builder.copy_buffer_to_image(CopyBufferToImageInfo::buffer_image(
        staging_buffer,
        image,
    ))?;

    let command_buffer = builder.build()?;

    // Execute and wait for completion
    command_buffer
        .execute(queue.clone())?
        .then_signal_fence_and_flush()?
        .wait(None)?;

    Ok(())
}

/// Creates sampler with pixel-art friendly settings
pub fn create_sampler(device: Arc<Device>) -> Result<Arc<Sampler>, Box<dyn std::error::Error>> {
    let sampler = Sampler::new(
        device,
        SamplerCreateInfo {
            mag_filter: Filter::Nearest,  // Pixelated when zoomed in
            min_filter: Filter::Nearest,  // Pixelated when zoomed out
            address_mode: [SamplerAddressMode::Repeat; 3],  // Repeat texture
            ..Default::default()
        },
    )?;

    println!("✓ Sampler created");

    Ok(sampler)
}