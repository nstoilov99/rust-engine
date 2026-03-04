use std::sync::Arc;
use vulkano::image::view::ImageView;
use vulkano::image::Image;
use vulkano::render_pass::{Framebuffer, FramebufferCreateInfo, RenderPass};

/// Creates one framebuffer per swapchain image
pub fn create_framebuffers(
    images: &[Arc<Image>],
    render_pass: Arc<RenderPass>,
) -> Result<Vec<Arc<Framebuffer>>, Box<dyn std::error::Error>> {
    let framebuffers = images
        .iter()
        .map(|image| {
            let view = ImageView::new_default(image.clone())?;
            Framebuffer::new(
                render_pass.clone(),
                FramebufferCreateInfo {
                    attachments: vec![view],
                    ..Default::default()
                },
            )
        })
        .collect::<Result<Vec<_>, _>>()?;

    println!("✓ Created {} framebuffers", framebuffers.len());

    Ok(framebuffers)
}

/// Creates framebuffers with depth attachment for 3D rendering
pub fn create_framebuffers_3d(
    images: &[Arc<Image>],
    render_pass: Arc<RenderPass>,
    depth_view: Arc<ImageView>,
) -> Result<Vec<Arc<Framebuffer>>, Box<dyn std::error::Error>> {
    images
        .iter()
        .map(|image| {
            let view = ImageView::new_default(image.clone())?;

            // Create framebuffer with BOTH color and depth attachments
            Framebuffer::new(
                render_pass.clone(),
                FramebufferCreateInfo {
                    attachments: vec![view, depth_view.clone()], // Color + Depth
                    ..Default::default()
                },
            )
            .map_err(|e| e.into())
        })
        .collect::<Result<Vec<_>, _>>()
}
