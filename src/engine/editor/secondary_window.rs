//! Secondary OS windows for mesh editors.
//!
//! Each secondary window has its own winit Window, Vulkan Surface/Swapchain,
//! and Gui instance. GPU resources (Device, Queue, allocators) are shared
//! with the main renderer.

use std::sync::Arc;

use vulkano::command_buffer::PrimaryAutoCommandBuffer;
use vulkano::device::{Device, Queue};
use vulkano::image::Image;
use vulkano::swapchain::{self, Surface, Swapchain, SwapchainPresentInfo};
use vulkano::sync::GpuFuture;
use vulkano::{Validated, VulkanError};
use winit::window::Window;

use crate::engine::core::swapchain::{create_swapchain, recreate_swapchain};
use crate::engine::gui::Gui;

/// Request to create a secondary OS window for a mesh editor.
pub struct PendingWindowRequest {
    pub mesh_key: String,
    pub title: String,
    pub width: u32,
    pub height: u32,
}

/// A secondary OS window with its own rendering infrastructure.
pub struct SecondaryWindow {
    pub window: Arc<Window>,
    surface: Arc<Surface>,
    swapchain: Arc<Swapchain>,
    images: Vec<Arc<Image>>,
    pub gui: Gui,
    recreate_swapchain: bool,
    pub is_minimized: bool,
    pub mesh_key: String,
    previous_frame_end: Option<Box<dyn GpuFuture>>,
    pub preview_texture_id: Option<egui::TextureId>,
    pub preview_texture_size: (u32, u32),
}

impl SecondaryWindow {
    pub fn new(
        window: Arc<Window>,
        device: Arc<Device>,
        queue: Arc<Queue>,
        mesh_key: String,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let instance = device.physical_device().instance().clone();
        let surface = Surface::from_window(instance, window.clone())?;
        let (swapchain, images) = create_swapchain(device.clone(), surface.clone())?;
        let swapchain_format = images[0].format();
        let gui = Gui::new(device, queue, swapchain_format, &window)?;

        Ok(Self {
            window,
            surface,
            swapchain,
            images,
            gui,
            recreate_swapchain: false,
            is_minimized: false,
            mesh_key,
            previous_frame_end: None,
            preview_texture_id: None,
            preview_texture_size: (0, 0),
        })
    }

    /// Update state when the window is resized.
    pub fn handle_resize(&mut self) {
        self.recreate_swapchain = true;
        let size = self.window.inner_size();
        self.is_minimized = size.width == 0 || size.height == 0;
        if !self.is_minimized {
            self.gui
                .set_screen_size(size.width as f32, size.height as f32);
        }
    }

    /// Render the secondary window's GUI content.
    ///
    /// When `preview_cb` is provided it is chained **before** the egui
    /// command buffer so that the preview texture is rendered and
    /// transitioned to `ShaderReadOnlyOptimal` within the same Vulkan
    /// submission — guaranteeing correct layout and memory visibility
    /// when egui samples it.
    pub fn render(
        &mut self,
        device: Arc<Device>,
        queue: Arc<Queue>,
        preview_cb: Option<Arc<PrimaryAutoCommandBuffer>>,
        ui_fn: impl FnMut(&egui::Context),
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Refresh screen_size from actual window dimensions every frame.
        // On Windows, inner_size() may return 0x0 at creation time before
        // the first Resized event is delivered.
        let size = self.window.inner_size();
        if size.width == 0 || size.height == 0 {
            self.is_minimized = true;
            return Ok(());
        }
        self.is_minimized = false;
        self.gui
            .set_screen_size(size.width as f32, size.height as f32);

        // Reclaim resources from previous frame
        if let Some(mut prev) = self.previous_frame_end.take() {
            prev.cleanup_finished();
        }

        // Recreate swapchain if needed (window resized, suboptimal, etc.)
        if self.recreate_swapchain {
            match recreate_swapchain(
                device.clone(),
                self.surface.clone(),
                self.swapchain.clone(),
            ) {
                Ok((new_swapchain, new_images)) => {
                    if new_images.is_empty() {
                        self.is_minimized = true;
                        return Ok(());
                    }
                    self.swapchain = new_swapchain;
                    self.images = new_images;
                    self.gui.clear_framebuffer_cache();
                    self.recreate_swapchain = false;
                }
                Err(e) => {
                    log::warn!("Secondary window swapchain recreation failed: {}", e);
                    return Ok(());
                }
            }
        }

        // Acquire swapchain image
        let (image_index, suboptimal, acquire_future) =
            match swapchain::acquire_next_image(self.swapchain.clone(), None) {
                Ok(r) => r,
                Err(Validated::Error(VulkanError::OutOfDate)) => {
                    self.recreate_swapchain = true;
                    return Ok(());
                }
                Err(e) => {
                    log::warn!("Secondary window acquire failed: {:?}", e);
                    self.recreate_swapchain = true;
                    return Ok(());
                }
            };

        if suboptimal {
            self.recreate_swapchain = true;
        }

        let target_image = self.images[image_index as usize].clone();

        // Run egui with clear — secondary windows have no 3D scene underneath,
        // so the swapchain image must be cleared before GUI rendering.
        let gui_result = self.gui.render_with_clear(
            &self.window,
            target_image,
            None,
            ui_fn,
            [0.11, 0.11, 0.12, 1.0],
        )?;

        // Submit: acquire → [preview if any] → GUI (with clear) → present
        let mut future: Box<dyn GpuFuture> = Box::new(acquire_future);
        if let Some(cb) = preview_cb {
            future = Box::new(
                future
                    .then_execute(queue.clone(), cb)
                    .map_err(|e| format!("Preview execute failed: {:?}", e))?,
            );
        }
        let future = future
            .then_execute(queue.clone(), gui_result.command_buffer)
            .map_err(|e| format!("Secondary window execute failed: {:?}", e))?;

        // Flush and mark finished (same pattern as main window to prevent
        // OneTimeSubmit panic on present failure).
        if let Err(e) = future.flush() {
            log::error!("Secondary window flush failed: {:?}", e);
            // SAFETY: flush attempted — mark finished to prevent CBEF Drop
            // from re-submitting OneTimeSubmit command buffers.
            unsafe { future.signal_finished() };
            queue.with(|mut q| q.wait_idle()).ok();
            return Ok(());
        }
        // SAFETY: flush succeeded — all CBs submitted. Marking finished
        // prevents inner CBEF drops from re-submitting.
        unsafe { future.signal_finished() };

        // Present
        let future = future
            .then_swapchain_present(
                queue.clone(),
                SwapchainPresentInfo::swapchain_image_index(
                    self.swapchain.clone(),
                    image_index,
                ),
            )
            .then_signal_fence_and_flush();

        match future {
            Ok(future) => {
                self.previous_frame_end = Some(future.boxed());
            }
            Err(e) => {
                log::warn!("Secondary window present failed: {:?}", e);
                self.recreate_swapchain = true;
                queue.with(|mut q| q.wait_idle()).ok();
            }
        }

        Ok(())
    }
}
