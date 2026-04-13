//! Unified render target abstraction
//!
//! Allows the deferred renderer to output to either a swapchain image
//! (standalone build) or a texture (editor viewport).

use std::sync::Arc;
use vulkano::image::Image;

pub enum RenderTarget {
    /// Direct swapchain output (standalone game)
    Swapchain { image: Arc<Image> },
    /// Offscreen texture output (editor viewport)
    #[cfg(feature = "editor")]
    Texture { image: Arc<Image> },
}

impl RenderTarget {
    pub fn image(&self) -> &Arc<Image> {
        match self {
            RenderTarget::Swapchain { image } => image,
            #[cfg(feature = "editor")]
            RenderTarget::Texture { image } => image,
        }
    }
}
