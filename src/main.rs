mod engine;

use engine::Renderer;
use std::sync::Arc;
use winit::event::{Event, WindowEvent};
use winit::event_loop::{ControlFlow, EventLoop};

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

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

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
            Event::MainEventsCleared => {
                // TODO: Render frame here (Next tutorial!)
            }
            _ => {}
        }
    });
}