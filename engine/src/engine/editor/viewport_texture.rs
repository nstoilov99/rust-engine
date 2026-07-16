//! Viewport Texture for Render-to-Texture
//!
//! Creates a Vulkan image that can be used as both a render target (COLOR_ATTACHMENT)
//! and sampled in the UI (SAMPLED). This enables rendering the 3D scene to a texture
//! that can then be displayed inside a UI panel.

use std::sync::Arc;
use vulkano::command_buffer::allocator::StandardCommandBufferAllocator;
use vulkano::command_buffer::{
    AutoCommandBufferBuilder, ClearColorImageInfo, CommandBufferUsage, PrimaryCommandBufferAbstract,
};
use vulkano::device::{Device, Queue};
use vulkano::format::Format;
use vulkano::image::view::ImageView;
use vulkano::image::{Image, ImageCreateInfo, ImageType, ImageUsage};
use vulkano::memory::allocator::{AllocationCreateInfo, StandardMemoryAllocator};
use vulkano::sync::GpuFuture;

/// Manages a render target texture for the viewport
pub struct ViewportTexture {
    image: Arc<Image>,
    image_view: Arc<ImageView>,
    width: u32,
    height: u32,
    format: Format,
    allocator: Arc<StandardMemoryAllocator>,
    device: Arc<Device>,
}

impl ViewportTexture {
    /// Create a new viewport texture with the given dimensions
    pub fn new(
        device: Arc<Device>,
        allocator: Arc<StandardMemoryAllocator>,
        width: u32,
        height: u32,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        // Use the same format as the lighting pass output (B8G8R8A8_SRGB)
        let format = Format::B8G8R8A8_SRGB;

        let (image, image_view) = Self::create_image(&allocator, width, height, format)?;

        Ok(Self {
            image,
            image_view,
            width,
            height,
            format,
            allocator,
            device,
        })
    }

    fn create_image(
        allocator: &Arc<StandardMemoryAllocator>,
        width: u32,
        height: u32,
        format: Format,
    ) -> Result<(Arc<Image>, Arc<ImageView>), Box<dyn std::error::Error>> {
        // Create image with both COLOR_ATTACHMENT (for rendering) and SAMPLED (for the UI)
        let image = Image::new(
            allocator.clone(),
            ImageCreateInfo {
                image_type: ImageType::Dim2d,
                format,
                extent: [width, height, 1],
                usage: ImageUsage::COLOR_ATTACHMENT
                    | ImageUsage::SAMPLED
                    | ImageUsage::TRANSFER_SRC
                    | ImageUsage::TRANSFER_DST,
                ..Default::default()
            },
            AllocationCreateInfo::default(),
        )?;

        let image_view = ImageView::new_default(image.clone())?;

        Ok((image, image_view))
    }

    /// Resize the viewport texture if dimensions have changed
    /// Returns true if the texture was recreated
    pub fn resize(
        &mut self,
        new_width: u32,
        new_height: u32,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        // Don't resize if dimensions are the same or invalid
        if new_width == self.width && new_height == self.height {
            return Ok(false);
        }

        if new_width == 0 || new_height == 0 {
            return Ok(false);
        }

        let (image, image_view) =
            Self::create_image(&self.allocator, new_width, new_height, self.format)?;

        self.image = image;
        self.image_view = image_view;
        self.width = new_width;
        self.height = new_height;

        Ok(true)
    }

    /// Clear the image synchronously so its layout is initialized before any
    /// UI pass samples it. A recorded-but-unsubmitted frame would otherwise
    /// leave the image Undefined while later command buffers expect General.
    pub fn clear(
        &self,
        queue: Arc<Queue>,
        cb_allocator: Arc<StandardCommandBufferAllocator>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut builder = AutoCommandBufferBuilder::primary(
            cb_allocator,
            queue.queue_family_index(),
            CommandBufferUsage::OneTimeSubmit,
        )?;
        builder.clear_color_image(ClearColorImageInfo::image(self.image.clone()))?;
        let fence = {
            let _submit_guard = crate::engine::rendering::common::gpu_context::lock_queue_submit();
            builder
                .build()?
                .execute(queue)?
                .then_signal_fence_and_flush()?
        };
        fence.wait(None)?;
        Ok(())
    }

    /// Get the image for use as a render target
    pub fn image(&self) -> Arc<Image> {
        self.image.clone()
    }

    /// Get the image view for use as a sampled texture
    pub fn image_view(&self) -> Arc<ImageView> {
        self.image_view.clone()
    }

    /// Get current width
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Get current height
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Get the device
    pub fn device(&self) -> Arc<Device> {
        self.device.clone()
    }
}
