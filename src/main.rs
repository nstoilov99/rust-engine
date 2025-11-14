pub mod engine;

use engine::Renderer;
use vulkano::descriptor_set;
use std::sync::Arc;
use winit::event::{Event, VirtualKeyCode, WindowEvent};
use winit::event_loop::{ControlFlow, EventLoop};
use crate::engine::Transform2D;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🎮 Rust Game Engine - Starting up...\n");

    let event_loop = EventLoop::new();
    let window = Arc::new(
        winit::window::WindowBuilder::new()
            .with_title("Rust Game Engine")
            .with_inner_size(winit::dpi::LogicalSize::new(800, 600))
            .build(&event_loop)?,
    );

    let mut renderer = Renderer::new(window.clone())?;

    let (texture_view, sampler) = engine::load_texture(
        renderer.device.clone(),
        renderer.queue.clone(),
        &renderer.command_buffer_allocator,
        renderer.memory_allocator.clone(),
        "assets/sprite.png",  // Put a test image here
    )?;
    println!("Texture loaded successfully!");

        // Create transforms for 3 sprites
    let sprites = vec![
        // Sprite 1: Center, no rotation
        (Transform2D::at_position(0.0, 0.0), renderer.descriptor_set.clone()),

        // Sprite 2: Right, rotated 45 degrees
        (Transform2D::new([0.5, 0.0], std::f32::consts::PI / 4.0, [0.5, 0.5]), renderer.descriptor_set.clone()),

        // Sprite 3: Left, scaled 2x
        (Transform2D::new([-0.5, 0.0], 0.0, [2.0, 2.0]), renderer.descriptor_set.clone()),
    ];

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Poll;

        match event {
            Event::WindowEvent { event, .. } => match event {
                WindowEvent::CloseRequested => {
                    println!("👋 Closing...");
                    *control_flow = ControlFlow::Exit;
                }
                WindowEvent::Resized(_) => {
                    println!("Window resized");
                }
                WindowEvent::KeyboardInput { input, .. } => {
                    if let Some(keycode) = input.virtual_keycode {
                        if keycode == VirtualKeyCode::Escape {
                            println!("👋 ESC pressed");
                            *control_flow = ControlFlow::Exit;
                        }
                    }
                }
                _ => {}
            },
            Event::RedrawRequested(_) => {
                if let Err(e) = renderer.render_sprites(&sprites) {
                    eprintln!("❌ Render error: {:?}", e);
                }
            }
            Event::MainEventsCleared => {
                window.request_redraw();
            }
            _ => {}
        }
    });
}