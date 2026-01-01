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
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalPosition, LogicalSize};
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowAttributes, WindowId};

/// Main application wrapper that implements the winit 0.30 ApplicationHandler trait
struct GameApp {
    /// The window (created in resumed())
    window: Option<Arc<Window>>,
    /// The actual game application (created after window)
    app: Option<App>,
    /// Saved window configuration
    window_config: WindowConfig,
    /// Whether the app should exit
    should_exit: bool,
    /// Whether the window is minimized (0x0 size)
    is_minimized: bool,
}

impl GameApp {
    fn new(window_config: WindowConfig) -> Self {
        Self {
            window: None,
            app: None,
            window_config,
            should_exit: false,
            is_minimized: false,
        }
    }
}

impl ApplicationHandler for GameApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        // Only create window if we don't have one yet
        if self.window.is_some() {
            return;
        }

        // Build window attributes from saved config
        let mut window_attrs = WindowAttributes::default()
            .with_title("Rust Game Engine")
            .with_inner_size(LogicalSize::new(
                self.window_config.width,
                self.window_config.height,
            ))
            .with_position(LogicalPosition::new(
                self.window_config.x,
                self.window_config.y,
            ));

        // Apply fullscreen or maximized state
        if self.window_config.fullscreen {
            window_attrs =
                window_attrs.with_fullscreen(Some(winit::window::Fullscreen::Borderless(None)));
        } else if self.window_config.maximized {
            window_attrs = window_attrs.with_maximized(true);
        }

        // Create window
        let window = match event_loop.create_window(window_attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                eprintln!("Failed to create window: {}", e);
                event_loop.exit();
                return;
            }
        };

        // Initialize application
        match App::new(window.clone()) {
            Ok(mut app) => {
                app.print_controls();
                println!("Engine ready!\n");
                self.app = Some(app);
            }
            Err(e) => {
                eprintln!("Failed to initialize application: {}", e);
                event_loop.exit();
                return;
            }
        }

        self.window = Some(window);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(app) = &mut self.app else { return };
        let Some(window) = &self.window else { return };

        match &event {
            WindowEvent::CloseRequested => {
                app.save_layout_on_exit();
                println!("Closing...");
                self.should_exit = true;
                event_loop.exit();
                return;
            }
            WindowEvent::RedrawRequested => {
                // Don't render when minimized
                if self.is_minimized {
                    return;
                }
                if let Err(e) = app.render(window) {
                    eprintln!("Render error: {}", e);
                }
                return;
            }
            WindowEvent::Resized(new_size) => {
                // Detect minimized state (0x0 window size)
                self.is_minimized = new_size.width == 0 || new_size.height == 0;
            }
            _ => {}
        }

        // Pass event to app for handling
        app.handle_window_event(&event, event_loop);

        // Check if app requested exit
        if self.should_exit {
            event_loop.exit();
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        // Skip all updates when minimized to avoid:
        // - CPU spinning (busy loop)
        // - Profiler accumulating unbounded data
        // - Physics stepping needlessly
        if self.is_minimized {
            return;
        }

        let Some(app) = &mut self.app else { return };
        let Some(window) = &self.window else { return };

        // Begin new frame
        app.begin_frame();

        // Update game state
        app.update();

        // Request redraw
        window.request_redraw();
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        // On suspend, we could release resources if needed
        // For now, just log
        println!("Application suspended");
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        // Final cleanup before exit
        if let Some(app) = &self.app {
            app.save_layout_on_exit();
        }
        println!("Application exiting");
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize profiler (1ns overhead when disabled, ~50-200ns when enabled)
    puffin::set_scopes_on(true);

    // Start Tracy client FIRST, before any profiling calls
    // The client must be started before any span!() macros are used
    #[cfg(feature = "tracy")]
    let _tracy_client = tracy_client::Client::start();

    // Load saved window configuration
    let window_config = WindowConfig::load_or_default();

    // Create event loop
    let event_loop = EventLoop::new()?;

    // Create game app
    let mut game_app = GameApp::new(window_config);

    // Run the event loop with our application handler
    event_loop.run_app(&mut game_app)?;

    Ok(())
}
