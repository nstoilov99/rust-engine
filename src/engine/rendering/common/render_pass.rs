// Render pass creation - defines rendering structure (attachments, subpasses, operations)

use std::sync::Arc;
use vulkano::format::Format;
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

/// Creates render pass with depth attachment for 3D rendering
pub fn create_render_pass_3d(
    device: Arc<Device>,
    color_format: Format,
) -> Result<Arc<RenderPass>, Box<dyn std::error::Error>> {
    let render_pass = vulkano::single_pass_renderpass!(
        device,
        attachments: {
            // Color attachment (same as 2D)
            color: {
                format: color_format,
                samples: 1,
                load_op: Clear,
                store_op: Store,
            },
            // NEW: Depth attachment
            depth: {
                format: Format::D32_SFLOAT,
                samples: 1,
                load_op: Clear,      // Clear to 1.0 (far plane)
                store_op: DontCare,  // Don't need depth after rendering
            }
        },
        pass: {
            color: [color],
            depth_stencil: {depth},  // Enable depth testing
        }
    )?;

    Ok(render_pass)
}