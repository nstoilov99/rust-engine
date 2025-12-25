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
use rust_engine::engine::editor::WindowConfig;
use std::sync::Arc;
use winit::event::Event;
use winit::event_loop::{ControlFlow, EventLoop};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize profiler (1ns overhead when disabled, ~50-200ns when enabled)
    puffin::set_scopes_on(true);

    // Load saved window configuration
    let window_config = WindowConfig::load_or_default();

    // Create window with saved size/position
    let event_loop = EventLoop::new();
    let mut window_builder = winit::window::WindowBuilder::new()
        .with_title("Rust Game Engine")
        .with_inner_size(winit::dpi::LogicalSize::new(
            window_config.width,
            window_config.height,
        ))
        .with_position(winit::dpi::LogicalPosition::new(
            window_config.x,
            window_config.y,
        ));

    // Apply fullscreen or maximized state
    if window_config.fullscreen {
        window_builder =
            window_builder.with_fullscreen(Some(winit::window::Fullscreen::Borderless(None)));
    } else if window_config.maximized {
        window_builder = window_builder.with_maximized(true);
    }

    let window = Arc::new(window_builder.build(&event_loop)?);

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
