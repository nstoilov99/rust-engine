use std::sync::Arc;
use vulkano::image::Image;
use vulkano::image::view::ImageView;
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