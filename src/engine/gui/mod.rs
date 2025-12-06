//! Custom egui-vulkano integration for Vulkano 0.34
//!
//! This module provides a minimal egui integration tailored for our engine.
//! Based on patterns from egui_winit_vulkano but adapted for Vulkano 0.34.

mod renderer;

pub use renderer::EguiRenderer;

use egui::Context;
use std::sync::Arc;
use winit::window::Window;

/// Main GUI integration struct
pub struct Gui {
    /// egui context
    context: Context,
    /// Vulkan renderer for egui
    renderer: EguiRenderer,
    /// Screen size for calculating input coordinates
    screen_size: [f32; 2],
}

impl Gui {
    /// Create new GUI integration
    pub fn new(
        device: Arc<vulkano::device::Device>,
        queue: Arc<vulkano::device::Queue>,
        swapchain_format: vulkano::format::Format,
        window: &Window,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let context = Context::default();

        // Create Vulkan renderer
        let renderer = EguiRenderer::new(device, queue, swapchain_format)?;

        let size = window.inner_size();
        let screen_size = [size.width as f32, size.height as f32];

        Ok(Self {
            context,
            renderer,
            screen_size,
        })
    }

    /// Run GUI and render - call this once per frame
    pub fn render(
        &mut self,
        _window: &Window,
        swapchain_image: Arc<vulkano::image::Image>,
        mut ui_fn: impl FnMut(&egui::Context),
    ) -> Result<Arc<vulkano::command_buffer::PrimaryAutoCommandBuffer<Arc<vulkano::command_buffer::allocator::StandardCommandBufferAllocator>>>, Box<dyn std::error::Error>> {
        // Create simple raw input (no real input for now, just for testing)
        let raw_input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(self.screen_size[0], self.screen_size[1]),
            )),
            ..Default::default()
        };

        // Run egui with UI code
        let full_output = self.context.run(raw_input, &mut ui_fn);

        // Tessellate and render
        let clipped_primitives = self.context.tessellate(full_output.shapes, full_output.pixels_per_point);

        // Create screen rect from our stored size
        let screen_rect = egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(self.screen_size[0], self.screen_size[1]),
        );

        self.renderer.render(
            swapchain_image,
            clipped_primitives,
            full_output.textures_delta,
            screen_rect,
        )
    }

    /// Update screen size (call when window is resized)
    pub fn set_screen_size(&mut self, width: f32, height: f32) {
        self.screen_size = [width, height];
    }

    /// Get egui context for custom usage
    pub fn context(&self) -> &Context {
        &self.context
    }
}
