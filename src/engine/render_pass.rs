// Render pass creation - defines rendering structure (attachments, subpasses, operations)

use std::sync::Arc;
use vulkano::device::Device;
use vulkano::render_pass::RenderPass;
use vulkano::swapchain::Swapchain;

/// Creates render pass for clearing screen to a color
pub fn create_render_pass(
    device: Arc<Device>,
    swapchain: Arc<Swapchain>,
) -> Result<Arc<RenderPass>, Box<dyn std::error::Error>> {
    // Use macro to create render pass (much simpler than manual creation)
    let render_pass = vulkano::single_pass_renderpass!(
        device,
        attachments: {
            color: {
                format: swapchain.image_format(),  // Match swapchain format
                samples: 1,                         // No multisampling
                load_op: Clear,                     // Clear before rendering
                store_op: Store,                    // Keep result after rendering
            },
        },
        pass: {
            color: [color],        // Use color attachment
            depth_stencil: {},     // No depth buffer
        },
    )?;

    println!("✓ Render pass created");

    Ok(render_pass)
}