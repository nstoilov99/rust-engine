// Swapchain - manages images for double/triple buffering to prevent tearing

use std::sync::Arc;
use vulkano::device::Device;
use vulkano::image::{Image, ImageUsage};
use vulkano::swapchain::{Surface, Swapchain, SwapchainCreateInfo};
use winit::window::Window;

/// Creates swapchain with at least 2 images for smooth rendering
pub fn create_swapchain(
    device: Arc<Device>,
    surface: Arc<Surface>,
) -> Result<(Arc<Swapchain>, Vec<Arc<Image>>), Box<dyn std::error::Error>> {
    let surface_capabilities = device
        .physical_device()
        .surface_capabilities(&surface, Default::default())?;

    // Prefer SRGB format (B8G8R8A8 or R8G8B8A8)
    let image_format = device
        .physical_device()
        .surface_formats(&surface, Default::default())?
        .into_iter()
        .find(|(format, _)| {
            matches!(
                format,
                vulkano::format::Format::B8G8R8A8_SRGB | vulkano::format::Format::R8G8B8A8_SRGB
            )
        })
        .unwrap_or_else(|| {
            device
                .physical_device()
                .surface_formats(&surface, Default::default())
                .unwrap()[0]
        });

    let window = surface.object().unwrap().downcast_ref::<Window>().unwrap();
    let window_size = window.inner_size();

    // Create swapchain with double buffering (min 2 images)
    let (swapchain, images) = Swapchain::new(
        device.clone(),
        surface.clone(),
        SwapchainCreateInfo {
            min_image_count: surface_capabilities.min_image_count.max(2),
            image_format: image_format.0,
            image_extent: [window_size.width, window_size.height],
            image_usage: ImageUsage::COLOR_ATTACHMENT,
            composite_alpha: surface_capabilities
                .supported_composite_alpha
                .into_iter()
                .next()
                .ok_or("No composite alpha modes supported")?,
            ..Default::default()
        },
    )?;

    println!("✓ Swapchain created");
    println!("  Format: {:?}", image_format.0);
    println!("  Size: {}x{}", window_size.width, window_size.height);
    println!("  Images: {}", images.len());

    Ok((swapchain, images))
}

/// Recreates swapchain when window is resized
pub fn recreate_swapchain(
    device: Arc<Device>,
    surface: Arc<Surface>,
    old_swapchain: Arc<Swapchain>,
) -> Result<(Arc<Swapchain>, Vec<Arc<Image>>), Box<dyn std::error::Error>> {
    let surface_capabilities = device
        .physical_device()
        .surface_capabilities(&surface, Default::default())?;

    let image_format = device
        .physical_device()
        .surface_formats(&surface, Default::default())?
        .into_iter()
        .find(|(format, _)| {
            matches!(
                format,
                vulkano::format::Format::B8G8R8A8_SRGB | vulkano::format::Format::R8G8B8A8_SRGB
            )
        })
        .unwrap_or_else(|| {
            device
                .physical_device()
                .surface_formats(&surface, Default::default())
                .unwrap()[0]
        });

    let window = surface.object().unwrap().downcast_ref::<Window>().unwrap();
    let window_size = window.inner_size();

    // Check if window is minimized (zero size)
    if window_size.width == 0 || window_size.height == 0 {
        // Return the old swapchain unchanged - we can't create a 0x0 swapchain
        return Ok((old_swapchain, vec![]));
    }

    // Reuse old swapchain for efficiency
    let (swapchain, images) = old_swapchain.recreate(SwapchainCreateInfo {
        image_extent: [window_size.width, window_size.height],
        ..old_swapchain.create_info()
    })?;

    println!("✓ Swapchain recreated: {}x{}", window_size.width, window_size.height);

    Ok((swapchain, images))
}