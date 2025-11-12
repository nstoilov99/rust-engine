use std::sync::Arc;
use vulkano::instance::{Instance, InstanceCreateInfo};
use vulkano::VulkanLibrary;

pub struct VulkanContext {
    pub instance: Arc<Instance>,
}

impl VulkanContext {
    pub fn new(app_name: &str) -> Result<Self, Box<dyn std::error::Error>> {
        // Load Vulkan library
        let library = VulkanLibrary::new()?;

        // Create instance
        let instance = Instance::new(
            library,
            InstanceCreateInfo {
                application_name: Some(app_name.to_string()),
                application_version: vulkano::Version::V1_0,
                engine_name: Some("Rust Engine".to_string()),
                engine_version: vulkano::Version::V1_0,
                ..Default::default()
            },
        )?;

        println!("✓ Vulkan instance created");

        Ok(Self { instance })
    }
}