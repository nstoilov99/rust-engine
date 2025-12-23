//! Game entry point
//!
//! This is a minimal entry point that delegates to the App struct.
//! All game logic is organized in the game modules.

mod app;
mod game_setup;
mod gui_panel;
mod input_handler;
mod render_loop;

use app::App;
use std::sync::Arc;
use winit::event::Event;
use winit::event_loop::{ControlFlow, EventLoop};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize profiler (1ns overhead when disabled, ~50-200ns when enabled)
    puffin::set_scopes_on(true);

    // Create window
    let event_loop = EventLoop::new();
    let window = Arc::new(
        winit::window::WindowBuilder::new()
            .with_title("Rust Game Engine")
            .with_inner_size(winit::dpi::LogicalSize::new(800, 600))
            .build(&event_loop)?,
    );

    // Initialize application
    let mut app = App::new(window.clone())?;
    app.print_controls();

    println!("Engine ready!\n");

    // Run event loop
    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Poll;

        match event {
            Event::WindowEvent { event, .. } => {
                app.handle_window_event(&event, control_flow);
            }
            Event::MainEventsCleared => {
                app.begin_frame();
                app.update();
                window.request_redraw();
            }
            Event::RedrawRequested(_) => {
                if let Err(e) = app.render(&window) {
                    eprintln!("Render error: {}", e);
                }
            }
            _ => {}
        }
    });
}
