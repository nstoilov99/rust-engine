//! Viewport Texture for Render-to-Texture
//!
//! Creates a Vulkan image that can be used as both a render target (COLOR_ATTACHMENT)
//! and sampled in egui (SAMPLED). This enables rendering the 3D scene to a texture
//! that can then be displayed inside an egui panel.

use std::sync::Arc;
use vulkano::device::Device;
use vulkano::format::Format;
use vulkano::image::view::ImageView;
use vulkano::image::{Image, ImageCreateInfo, ImageType, ImageUsage};
use vulkano::memory::allocator::{AllocationCreateInfo, StandardMemoryAllocator};

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
        // Create image with both COLOR_ATTACHMENT (for rendering) and SAMPLED (for egui)
        let image = Image::new(
            allocator.clone(),
            ImageCreateInfo {
                image_type: ImageType::Dim2d,
                format,
                extent: [width, height, 1],
                usage: ImageUsage::COLOR_ATTACHMENT
                    | ImageUsage::SAMPLED
                    | ImageUsage::TRANSFER_SRC,
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
