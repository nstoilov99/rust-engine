use std::sync::Arc;
use vulkano::device::{Device, Queue};
use vulkano::image::Image;
use vulkano::instance::Instance;
use vulkano::swapchain::{Surface, Swapchain};
use winit::window::Window;

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

        println!("\n✅ Renderer initialized successfully!\n");

        Ok(Self {
            _instance: vulkan_context.instance,
            device: device_context.device,
            queue: device_context.queue,
            surface,
            swapchain,
            images,
        })
    }
}