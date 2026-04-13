// Vulkan instance - entry point to Vulkan API

use std::sync::Arc;
use vulkano::instance::{Instance, InstanceCreateInfo};
use vulkano::VulkanLibrary;

pub struct VulkanContext {
    pub instance: Arc<Instance>,
}

impl VulkanContext {
    pub fn new(app_name: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let library = VulkanLibrary::new()?;

        // Enable all supported extensions (surface, win32_surface, etc.)
        let required_extensions = *library.supported_extensions();

        let instance = Instance::new(
            library,
            InstanceCreateInfo {
                application_name: Some(app_name.to_string()),
                application_version: vulkano::Version::V1_0,
                engine_name: Some("Rust Engine".to_string()),
                engine_version: vulkano::Version::V1_0,
                enabled_extensions: required_extensions,
                ..Default::default()
            },
        )?;

        Ok(Self { instance })
    }
}
