// Swapchain - manages images for double/triple buffering to prevent tearing

use std::sync::Arc;
use vulkano::device::Device;
use vulkano::image::{Image, ImageUsage};
use vulkano::swapchain::{PresentMode, Surface, Swapchain, SwapchainCreateInfo};
use winit::window::Window;

/// Result type for swapchain creation operations.
type SwapchainResult = Result<(Arc<Swapchain>, Vec<Arc<Image>>), Box<dyn std::error::Error>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwapchainPresentModePreference {
    Default,
    Immediate,
}

/// Creates swapchain with at least 2 images for smooth rendering
pub fn create_swapchain(device: Arc<Device>, surface: Arc<Surface>) -> SwapchainResult {
    create_swapchain_with_present_mode(device, surface, SwapchainPresentModePreference::Default)
}

/// Creates a swapchain with an explicit present-mode preference.
pub fn create_swapchain_with_present_mode(
    device: Arc<Device>,
    surface: Arc<Surface>,
    present_mode_preference: SwapchainPresentModePreference,
) -> SwapchainResult {
    // Get window dimensions first for validation
    let window = surface.object().unwrap().downcast_ref::<Window>().unwrap();
    let window_size = window.inner_size();

    // Guard against 0x0 window (minimized or not yet visible)
    if window_size.width == 0 || window_size.height == 0 {
        return Err("Window has zero dimensions - cannot create swapchain".into());
    }

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

    let composite_alpha = surface_capabilities
        .supported_composite_alpha
        .into_iter()
        .next()
        .ok_or("No composite alpha modes supported")?;

    // Use the requested present mode when possible. Immediate is useful for
    // uncapped benchmark runs; the default path preserves existing behavior.
    let present_modes = device
        .physical_device()
        .surface_present_modes(&surface, Default::default())?;
    let present_mode = choose_present_mode(&present_modes, present_mode_preference);

    // Create swapchain with double buffering (min 2 images)
    let (swapchain, images) = Swapchain::new(
        device.clone(),
        surface.clone(),
        SwapchainCreateInfo {
            min_image_count: surface_capabilities.min_image_count.max(2),
            image_format: image_format.0,
            image_extent: [window_size.width, window_size.height],
            image_usage: ImageUsage::COLOR_ATTACHMENT,
            composite_alpha,
            present_mode,
            ..Default::default()
        },
    )?;

    println!(
        "✓ Swapchain: {}x{}, {:?}",
        window_size.width, window_size.height, present_mode
    );

    Ok((swapchain, images))
}

fn choose_present_mode(
    present_modes: &[PresentMode],
    preference: SwapchainPresentModePreference,
) -> PresentMode {
    match preference {
        SwapchainPresentModePreference::Immediate => {
            if present_modes.contains(&PresentMode::Immediate) {
                PresentMode::Immediate
            } else if present_modes.contains(&PresentMode::Mailbox) {
                PresentMode::Mailbox
            } else {
                PresentMode::Fifo
            }
        }
        SwapchainPresentModePreference::Default => {
            if present_modes.contains(&PresentMode::Mailbox) {
                PresentMode::Mailbox
            } else {
                PresentMode::Fifo
            }
        }
    }
}

/// Recreates swapchain when window is resized
pub fn recreate_swapchain(
    device: Arc<Device>,
    surface: Arc<Surface>,
    old_swapchain: Arc<Swapchain>,
) -> SwapchainResult {
    let _surface_capabilities = device
        .physical_device()
        .surface_capabilities(&surface, Default::default())?;

    let _image_format = device
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

    Ok((swapchain, images))
}

#[cfg(test)]
mod tests {
    use super::{choose_present_mode, SwapchainPresentModePreference};
    use vulkano::swapchain::PresentMode;

    #[test]
    fn default_preference_keeps_mailbox_behavior() {
        let modes = [PresentMode::Fifo, PresentMode::Mailbox];
        assert_eq!(
            choose_present_mode(&modes, SwapchainPresentModePreference::Default),
            PresentMode::Mailbox
        );
    }

    #[test]
    fn immediate_preference_uses_immediate_when_supported() {
        let modes = [PresentMode::Fifo, PresentMode::Immediate];
        assert_eq!(
            choose_present_mode(&modes, SwapchainPresentModePreference::Immediate),
            PresentMode::Immediate
        );
    }

    #[test]
    fn immediate_preference_falls_back_cleanly() {
        let modes = [PresentMode::Fifo, PresentMode::Mailbox];
        assert_eq!(
            choose_present_mode(&modes, SwapchainPresentModePreference::Immediate),
            PresentMode::Mailbox
        );
    }
}
