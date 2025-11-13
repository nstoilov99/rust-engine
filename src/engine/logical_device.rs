// Logical device - interface to GPU for creating resources and submitting commands

use std::sync::Arc;
use vulkano::device::physical::PhysicalDevice;
use vulkano::device::{Device, DeviceCreateInfo, DeviceExtensions, Queue, QueueCreateInfo, QueueFlags};
use vulkano::swapchain::Surface;

pub struct LogicalDeviceContext {
    pub device: Arc<Device>,  // For creating resources
    pub queue: Arc<Queue>,    // For submitting commands
}

/// Creates logical device with queue that supports graphics + present
pub fn create_logical_device(
    physical_device: Arc<PhysicalDevice>,
    surface: Arc<Surface>,
) -> Result<LogicalDeviceContext, Box<dyn std::error::Error>> {
    // Find queue family that supports graphics AND present
    let queue_family_index = physical_device
        .queue_family_properties()
        .iter()
        .enumerate()
        .position(|(i, q)| {
            let has_graphics = q.queue_flags.intersects(QueueFlags::GRAPHICS);
            let has_present = physical_device
                .surface_support(i as u32, &surface)
                .unwrap_or(false);
            has_graphics && has_present
        })
        .ok_or("No suitable queue family found")? as u32;

    let (device, mut queues) = Device::new(
        physical_device,
        DeviceCreateInfo {
            queue_create_infos: vec![QueueCreateInfo {
                queue_family_index,
                ..Default::default()
            }],
            enabled_extensions: DeviceExtensions {
                khr_swapchain: true,  // Required for presenting to screen
                ..DeviceExtensions::empty()
            },
            ..Default::default()
        },
    )?;

    let queue = queues.next().ok_or("No queue created")?;

    println!("✓ Logical device created");
    println!("  Queue family index: {}", queue_family_index);

    Ok(LogicalDeviceContext { device, queue })
}