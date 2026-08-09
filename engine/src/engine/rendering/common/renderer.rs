//! GPU bootstrap: instance/device/swapchain creation and present helper.
//!
//! `Renderer` is intentionally thin — it owns the Vulkan bootstrap
//! (`GpuContext`: device, queue, allocators) and the window swapchain
//! (`SwapchainState`), plus the editor/game camera state that call sites
//! mutate on the main thread. All actual rendering happens on the render
//! thread (`DeferredRenderer` via `RenderThread`); the legacy immediate-mode
//! `render_*` methods were removed in Refactor Checkpoint #6.

use crate::engine::camera::Camera3D;
use crate::engine::core::swapchain::{
    create_swapchain, create_swapchain_with_present_mode, SwapchainPresentModePreference,
};
use crate::engine::core::{create_logical_device, select_physical_device, VulkanContext};
use std::sync::Arc;
use vulkano::command_buffer::allocator::StandardCommandBufferAllocator;
use vulkano::descriptor_set::allocator::StandardDescriptorSetAllocator;
use vulkano::image::Image;
use vulkano::instance::Instance;
use vulkano::memory::allocator::StandardMemoryAllocator;
use vulkano::swapchain::{self as vk_swapchain, Surface, Swapchain};
use vulkano::sync::GpuFuture;
use vulkano::{Validated, VulkanError};
use winit::window::Window;

/// Swapchain and associated resources (transferred to the render thread at
/// startup; the main thread keeps this copy for recreation bookkeeping).
pub struct SwapchainState {
    pub swapchain: Arc<Swapchain>,
    pub images: Vec<Arc<Image>>,
    pub surface: Arc<Surface>,
    pub recreate_swapchain: bool,
}

pub struct Renderer {
    #[allow(dead_code)] // Kept alive for Vulkan instance lifetime
    instance: Arc<Instance>,
    pub gpu: Arc<super::gpu_context::GpuContext>,
    pub swapchain_state: SwapchainState,
    pub camera_3d: Camera3D,
    previous_frame_end: Option<Box<dyn GpuFuture>>,
}

impl Renderer {
    pub fn new(window: Arc<Window>) -> Result<Self, Box<dyn std::error::Error>> {
        Self::new_with_present_mode(window, SwapchainPresentModePreference::Default)
    }

    pub fn new_with_present_mode(
        window: Arc<Window>,
        present_mode_preference: SwapchainPresentModePreference,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let vulkan_context = VulkanContext::new("Rust Engine")?;

        let surface = Surface::from_window(vulkan_context.instance.clone(), window.clone())?;

        let physical_device = select_physical_device(vulkan_context.instance.clone())?;
        let device_context = create_logical_device(physical_device, surface.clone())?;
        let (swapchain, images) = if matches!(
            present_mode_preference,
            SwapchainPresentModePreference::Default
        ) {
            create_swapchain(device_context.device.clone(), surface.clone())?
        } else {
            create_swapchain_with_present_mode(
                device_context.device.clone(),
                surface.clone(),
                present_mode_preference,
            )?
        };

        let command_buffer_allocator = Arc::new(StandardCommandBufferAllocator::new(
            device_context.device.clone(),
            Default::default(),
        ));
        let memory_allocator = Arc::new(StandardMemoryAllocator::new_default(
            device_context.device.clone(),
        ));
        let descriptor_set_allocator = Arc::new(StandardDescriptorSetAllocator::new(
            device_context.device.clone(),
            Default::default(),
        ));

        let window_size = window.inner_size();
        let camera_3d = Camera3D::new(window_size.width as f32, window_size.height as f32);

        let gpu = Arc::new(super::gpu_context::GpuContext {
            device: device_context.device,
            queue: device_context.queue,
            memory_allocator,
            command_buffer_allocator,
            descriptor_set_allocator,
        });

        let swapchain_state = SwapchainState {
            swapchain,
            images,
            surface,
            recreate_swapchain: false,
        };

        Ok(Self {
            instance: vulkan_context.instance,
            gpu,
            swapchain_state,
            camera_3d,
            previous_frame_end: None,
        })
    }

    /// Submit a command buffer, present to the swapchain, and store the
    /// resulting fence future for the next frame's cleanup.
    pub fn submit_and_present(
        &mut self,
        acquire_future: impl GpuFuture + 'static,
        command_buffer: Arc<impl vulkano::command_buffer::PrimaryCommandBufferAbstract + 'static>,
        image_index: u32,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(mut previous) = self.previous_frame_end.take() {
            previous.cleanup_finished();
        }

        let _submit_guard = super::gpu_context::lock_queue_submit();
        let future = acquire_future
            .then_execute(self.gpu.queue.clone(), command_buffer)?
            .then_swapchain_present(
                self.gpu.queue.clone(),
                vk_swapchain::SwapchainPresentInfo::swapchain_image_index(
                    self.swapchain_state.swapchain.clone(),
                    image_index,
                ),
            )
            .then_signal_fence_and_flush();

        match future {
            Ok(future) => {
                self.previous_frame_end = Some(future.boxed());
            }
            Err(Validated::Error(VulkanError::DeviceLost)) => {
                return Err("GPU device lost".into());
            }
            Err(Validated::Error(VulkanError::OutOfDate)) => {
                self.swapchain_state.recreate_swapchain = true;
                self.previous_frame_end = Some(vulkano::sync::now(self.gpu.device.clone()).boxed());
            }
            Err(e) => {
                eprintln!("Failed to flush future: {:?}", e);
                self.previous_frame_end = Some(vulkano::sync::now(self.gpu.device.clone()).boxed());
            }
        }

        Ok(())
    }
}
