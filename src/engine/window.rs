// Window management system
// Handles window creation, event processing, and lifecycle

use std::sync::Arc;
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop},
    window::{Window as WinitWindow, WindowAttributes, WindowId},
};

/// Represents the application window and event handler
///
/// In winit 0.30+, we use the ApplicationHandler trait which provides
/// lifecycle callbacks for window management
pub struct Window {
    event_loop: EventLoop<()>,
    window_attrs: WindowAttributes,
}

impl Window {
    /// Creates a new window configuration with the specified title and dimensions
    ///
    /// # Arguments
    /// * `title` - The window title (shown in title bar)
    /// * `width` - Window width in pixels
    /// * `height` - Window height in pixels
    ///
    /// # Returns
    /// A Result containing the Window or an error if creation fails
    pub fn new(title: &str, width: u32, height: u32) -> Result<Self, Box<dyn std::error::Error>> {
        // Create the event loop - this is the core of the application
        // It receives events from the operating system (keyboard, mouse, window events)
        let event_loop = EventLoop::new()?;

        // Build the window attributes
        let window_attrs = WindowAttributes::default()
            .with_title(title)
            .with_inner_size(winit::dpi::LogicalSize::new(width, height));

        println!("✓ Window configuration created: {}x{}", width, height);

        Ok(Self {
            event_loop,
            window_attrs,
        })
    }

    /// Runs the main event loop with the provided app handler
    ///
    /// This function takes ownership of self and runs the event loop.
    /// It creates the window when the event loop starts.
    ///
    /// # Arguments
    /// * `app` - An implementation of ApplicationHandler that processes events
    pub fn run<A>(self, app: A) -> Result<(), Box<dyn std::error::Error>>
    where
        A: ApplicationHandler + 'static,
    {
        // Create a wrapper to handle window creation on startup
        let window_attrs = self.window_attrs;
        let mut wrapper = AppWrapper {
            app,
            window_attrs: Some(window_attrs),
            window: None,
        };

        self.event_loop.run_app(&mut wrapper)?;
        Ok(())
    }
}

/// Internal wrapper that creates the window when the event loop starts
struct AppWrapper<A> {
    app: A,
    window_attrs: Option<WindowAttributes>,
    window: Option<Arc<WinitWindow>>,
}

impl<A: ApplicationHandler> ApplicationHandler for AppWrapper<A> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        // Create window if not already created
        if self.window.is_none() {
            if let Some(attrs) = self.window_attrs.take() {
                match event_loop.create_window(attrs) {
                    Ok(window) => {
                        println!("✓ Window created successfully");
                        self.window = Some(Arc::new(window));
                    }
                    Err(e) => {
                        eprintln!("❌ Failed to create window: {}", e);
                        event_loop.exit();
                        return;
                    }
                }
            }
        }

        self.app.resumed(event_loop);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, window_id: WindowId, event: WindowEvent) {
        self.app.window_event(event_loop, window_id, event);
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.app.about_to_wait(event_loop);
    }

    fn suspended(&mut self, event_loop: &ActiveEventLoop) {
        self.app.suspended(event_loop);
    }

    fn exiting(&mut self, event_loop: &ActiveEventLoop) {
        self.app.exiting(event_loop);
    }
}
