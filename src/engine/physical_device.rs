use std::sync::Arc;
use vulkano::device::physical::PhysicalDevice;
use vulkano::device::{DeviceExtensions, QueueFlags};
use vulkano::instance::Instance;

pub fn select_physical_device(
    instance: Arc<Instance>,
) -> Result<Arc<PhysicalDevice>, String> {
    // Get all physical devices and collect into Vec
    let devices: Vec<_> = instance
        .enumerate_physical_devices()
        .map_err(|e| format!("Failed to enumerate devices: {}", e))?
        .collect();

    println!("📊 Available GPUs:");
    for device in &devices {
        let props = device.properties();
        println!("  - {}", props.device_name);
    }

    // Select first device with graphics queue and swapchain support
    let device = devices
        .into_iter()
        .filter(|d| {
            // Must have graphics queue
            d.queue_family_properties()
                .iter()
                .any(|q| q.queue_flags.intersects(QueueFlags::GRAPHICS))
        })
        .filter(|d| {
            // Must support swapchain extension
            d.supported_extensions()
                .contains(&DeviceExtensions {
                    khr_swapchain: true,
                    ..DeviceExtensions::empty()
                })
        })
        .min_by_key(|d| {
            // Prefer discrete GPU over integrated
            match d.properties().device_type {
                vulkano::device::physical::PhysicalDeviceType::DiscreteGpu => 0,
                vulkano::device::physical::PhysicalDeviceType::IntegratedGpu => 1,
                _ => 2,
            }
        })
        .ok_or("No suitable GPU found")?;

    let props = device.properties();
    println!("✓ Selected GPU: {}", props.device_name);

    Ok(device)
}