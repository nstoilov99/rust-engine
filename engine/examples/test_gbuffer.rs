//! Test G-Buffer generation and validation

use rust_engine::engine::rendering::rendering_3d::GBuffer;
use rust_engine::Renderer;
use std::sync::Arc;
use winit::event_loop::EventLoop;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🧪 Testing G-Buffer generation...\n");

    let event_loop = EventLoop::new();
    let window = Arc::new(
        winit::window::WindowBuilder::new()
            .with_title("G-Buffer Test")
            .with_inner_size(winit::dpi::LogicalSize::new(800, 600))
            .build(&event_loop)?,
    );

    let renderer = Renderer::new(window.clone())?;

    // Create G-Buffer
    println!("📦 Creating G-Buffer (800x600)...");
    let gbuffer = GBuffer::new(
        renderer.gpu.device.clone(),
        renderer.gpu.memory_allocator.clone(),
        800,
        600,
    )?;

    println!("✅ G-Buffer created successfully!\n");
    println!("📊 G-Buffer Layout:");
    println!("   RT0 (Position): {:?}", gbuffer.position.format());
    println!("   RT1 (Normal):   {:?}", gbuffer.normal.format());
    println!("   RT2 (Albedo):   {:?}", gbuffer.albedo.format());
    println!("   RT3 (Material): {:?}", gbuffer.material.format());
    println!("   Depth:          {:?}", gbuffer.depth.format());

    // Verify render pass
    println!("\n🔍 Render Pass Validation:");
    let attachments = gbuffer.render_pass.attachments();
    println!("   Attachments: {}", attachments.len());
    assert_eq!(
        attachments.len(),
        5,
        "Should have 5 attachments (4 color + 1 depth)"
    );

    // Verify framebuffer
    println!("\n🖼️  Framebuffer Validation:");
    println!("   Extent: {:?}", gbuffer.framebuffer.extent());
    assert_eq!(gbuffer.framebuffer.extent()[0], 800);
    assert_eq!(gbuffer.framebuffer.extent()[1], 600);

    println!("\n✅ All tests passed!");

    Ok(())
}
