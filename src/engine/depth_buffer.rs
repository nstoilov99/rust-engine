use std::sync::Arc;
use vulkano::device::Device;
use vulkano::image::{Image, ImageCreateInfo, ImageType, ImageUsage};
use vulkano::image::view::{ImageView, ImageViewCreateInfo};
use vulkano::format::Format;
use vulkano::memory::allocator::StandardMemoryAllocator;

/// Creates a depth buffer image and view
pub fn create_depth_buffer(
    device: Arc<Device>,
    memory_allocator: Arc<StandardMemoryAllocator>,
    width: u32,
    height: u32,
) -> Result<Arc<ImageView>, Box<dyn std::error::Error>> {
    // Create depth image
    let depth_image = Image::new(
        memory_allocator,
        ImageCreateInfo {
            image_type: ImageType::Dim2d,
            format: Format::D32_SFLOAT,  // 32-bit float depth
            extent: [width, height, 1],
            usage: ImageUsage::DEPTH_STENCIL_ATTACHMENT,
            ..Default::default()
        },
        Default::default(),
    )?;

    // Create image view for rendering
    let depth_view = ImageView::new(
        depth_image.clone(),
        ImageViewCreateInfo::from_image(&depth_image),
    )?;

    Ok(depth_view)
}