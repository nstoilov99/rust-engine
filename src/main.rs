mod engine;

use engine::Renderer;
use std::sync::Arc;
use winit::event::{Event, WindowEvent};
use winit::event_loop::{ControlFlow, EventLoop};
use engine::render_pass::create_render_pass;
use engine::framebuffer::create_framebuffers;
use engine::pipeline::create_pipeline;
use vulkano::pipeline::graphics::viewport::Viewport;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🎮 Rust Game Engine - Starting up...\n");

    let event_loop = EventLoop::new();
    let window = Arc::new(
        winit::window::WindowBuilder::new()
            .with_title("Rust Game Engine")
            .with_inner_size(winit::dpi::LogicalSize::new(800, 600))
            .build(&event_loop)
            .unwrap(),
    );

    let renderer = Renderer::new(window.clone())?;

    // Create render pass
    let render_pass = create_render_pass(
        renderer.device.clone(),
        renderer.swapchain.clone(),
    )?;

    // Create framebuffers
    let framebuffers = create_framebuffers(
        &renderer.images,
        render_pass.clone(),
    )?;

    // Create pipeline
    let viewport = Viewport {
        offset: [0.0, 0.0],
        extent: [800.0, 600.0],
        depth_range: 0.0..=1.0,
    };
    let pipeline = create_pipeline(
        renderer.device.clone(),
        render_pass.clone(),
        viewport,
    )?;

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Poll;  // Changed to Poll for continuous rendering

        match event {
            Event::WindowEvent { event, .. } => match event {
                WindowEvent::CloseRequested => {
                    println!("👋 Closing...");
                    *control_flow = ControlFlow::Exit;
                }
                WindowEvent::KeyboardInput { input, .. } => {
                    if let Some(keycode) = input.virtual_keycode {
                        if keycode == winit::event::VirtualKeyCode::Escape {
                            println!("👋 ESC pressed");
                            *control_flow = ControlFlow::Exit;
                        }
                    }
                }
                _ => {}
            },
            Event::RedrawRequested(_) => {
                    engine::renderer::render_triangle(
                    &renderer.command_buffer_allocator,
                    renderer.queue.clone(),
                    renderer.swapchain.clone(),
                    &framebuffers,
                    pipeline.clone(),
                    renderer.vertex_buffer.clone(),
                ).expect("Failed to render");
            }
            Event::MainEventsCleared => {
                // Request redraw every frame
                window.request_redraw();
            }
            _ => {}
        }
    });
}