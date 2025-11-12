// Main entry point for the Rust Game Engine
// This file demonstrates basic window creation and event handling

mod engine;

use engine::Window;
use std::sync::Arc;
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::ActiveEventLoop,
    window::{Window as WinitWindow, WindowId},
};

/// Our game application structure
/// This holds the game state and handles events
struct GameApp {
    window: Option<Arc<WinitWindow>>,
}

impl GameApp {
    fn new() -> Self {
        Self { window: None }
    }
}

/// Implement ApplicationHandler to receive window events
/// This is the core trait for winit 0.30+ applications
impl ApplicationHandler for GameApp {
    /// Called when the application resumes (or starts for the first time)
    /// This is where we get access to the window
    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {
        // Store the window if we don't have it yet
        // In a real game, you'd initialize graphics context here
        if self.window.is_none() {
            println!("🎮 Application resumed - ready to receive events");
        }
    }

    /// Called for window-specific events (keyboard, mouse, resize, etc.)
    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            // User clicked the close button (X)
            WindowEvent::CloseRequested => {
                println!("👋 Close button pressed, shutting down...");
                event_loop.exit();
            }

            // User pressed a key
            WindowEvent::KeyboardInput { event, .. } => {
                // Check if the key was pressed (not released)
                if event.state.is_pressed() {
                    // Check if ESC was pressed
                    if let winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape) =
                        event.logical_key
                    {
                        println!("👋 ESC pressed, shutting down...");
                        event_loop.exit();
                    } else {
                        println!("⌨️  Key pressed: {:?}", event.logical_key);
                    }
                }
            }

            // Window was resized
            WindowEvent::Resized(new_size) => {
                println!(
                    "📐 Window resized to: {}x{}",
                    new_size.width, new_size.height
                );
            }

            // Window gained or lost focus
            WindowEvent::Focused(focused) => {
                if focused {
                    println!("👁️  Window gained focus");
                } else {
                    println!("💤 Window lost focus");
                }
            }

            // Window needs to be redrawn
            WindowEvent::RedrawRequested => {
                // This is where we'd call our render function
                // For now, we don't have a renderer yet
            }

            // Catch-all for other window events we don't handle yet
            _ => {}
        }
    }

    /// Called when the event loop is about to wait for new events
    /// This is the perfect place to update game logic and request redraws
    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        // Request a redraw on every frame
        // This keeps the game loop running continuously
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🎮 Rust Game Engine - Starting up...\n");

    let vulkan_context = engine::VulkanContext::new("Rust Engine")?;

    let physical_device = engine::select_physical_device(vulkan_context.instance.clone())?;

    let window = Window::new("Rust Game Engine", 800, 600)?;

    println!("✓ Event loop starting");
    println!("📝 Press ESC or close window to exit\n");

    // Create our game app and run it
    let app = GameApp::new();
    window.run(app)?;

    println!("✓ Engine shut down successfully");
    Ok(())
}
