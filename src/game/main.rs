//! Game entry point
//!
//! This is a minimal entry point that delegates to the App struct.
//! All game logic is organized in the game modules.

#[cfg(feature = "editor")]
mod app;
mod game_setup;
mod input_handler;
mod render_loop;

#[cfg(feature = "editor")]
use app::App;
use rust_engine::engine::utils::WindowConfig;
use std::sync::Arc;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalPosition, LogicalSize};
use winit::event::{DeviceEvent, DeviceId, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowAttributes, WindowId};

// ============================================================================
// Editor build: full editor with GUI, panels, docking, gizmos
// ============================================================================

#[cfg(feature = "editor")]
struct GameApp {
    window: Option<Arc<Window>>,
    app: Option<App>,
    window_config: WindowConfig,
    should_exit: bool,
    is_minimized: bool,
}

#[cfg(feature = "editor")]
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

#[cfg(feature = "editor")]
impl ApplicationHandler for GameApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

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

        if self.window_config.fullscreen {
            window_attrs =
                window_attrs.with_fullscreen(Some(winit::window::Fullscreen::Borderless(None)));
        } else if self.window_config.maximized {
            window_attrs = window_attrs.with_maximized(true);
        }

        let window = match event_loop.create_window(window_attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                eprintln!("Failed to create window: {}", e);
                event_loop.exit();
                return;
            }
        };

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
                if self.is_minimized {
                    return;
                }
                if let Err(e) = app.render(window) {
                    eprintln!("Render error: {}", e);
                }
                return;
            }
            WindowEvent::Resized(new_size) => {
                self.is_minimized = new_size.width == 0 || new_size.height == 0;
            }
            _ => {}
        }

        app.handle_window_event(&event, event_loop);

        if self.should_exit {
            event_loop.exit();
        }
    }

    fn device_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _device_id: DeviceId,
        event: DeviceEvent,
    ) {
        let Some(app) = &mut self.app else { return };

        if let DeviceEvent::MouseMotion { delta } = event {
            app.core.input_manager.handle_raw_mouse_motion(delta.0, delta.1);
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if self.is_minimized {
            return;
        }

        let Some(app) = &mut self.app else { return };
        let Some(window) = &self.window else { return };

        app.begin_frame();
        app.update();
        window.request_redraw();
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        println!("Application suspended");
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(app) = &self.app {
            app.save_layout_on_exit();
        }
        println!("Application exiting");
    }
}

// ============================================================================
// Standalone build: game-only, no editor UI
// ============================================================================

#[cfg(not(feature = "editor"))]
mod standalone;

#[cfg(not(feature = "editor"))]
struct GameApp {
    window: Option<Arc<Window>>,
    app: Option<standalone::StandaloneApp>,
    should_exit: bool,
    is_minimized: bool,
}

#[cfg(not(feature = "editor"))]
impl GameApp {
    fn new() -> Self {
        Self {
            window: None,
            app: None,
            should_exit: false,
            is_minimized: false,
        }
    }
}

#[cfg(not(feature = "editor"))]
impl ApplicationHandler for GameApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let window_config = WindowConfig::default();
        let window_attrs = WindowAttributes::default()
            .with_title("Rust Game Engine")
            .with_inner_size(LogicalSize::new(window_config.width, window_config.height));

        let window = match event_loop.create_window(window_attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                eprintln!("Failed to create window: {}", e);
                event_loop.exit();
                return;
            }
        };

        match standalone::StandaloneApp::new(window.clone()) {
            Ok(app) => {
                println!("Game ready!");
                self.app = Some(app);
            }
            Err(e) => {
                eprintln!("Failed to initialize game: {}", e);
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
                println!("Closing...");
                self.should_exit = true;
                event_loop.exit();
                return;
            }
            WindowEvent::RedrawRequested => {
                if self.is_minimized {
                    return;
                }
                if let Err(e) = app.render(window) {
                    eprintln!("Render error: {}", e);
                }
                return;
            }
            WindowEvent::Resized(new_size) => {
                self.is_minimized = new_size.width == 0 || new_size.height == 0;
            }
            _ => {}
        }

        app.handle_window_event(&event);

        if self.should_exit {
            event_loop.exit();
        }
    }

    fn device_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _device_id: DeviceId,
        _event: DeviceEvent,
    ) {}

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if self.is_minimized {
            return;
        }

        let Some(app) = &mut self.app else { return };
        let Some(window) = &self.window else { return };

        app.begin_frame();
        app.update();
        window.request_redraw();
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        println!("Application suspended");
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        println!("Application exiting");
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    puffin::set_scopes_on(true);

    #[cfg(feature = "tracy")]
    let _tracy_client = tracy_client::Client::start();

    // Initialize the asset source before anything else.
    init_asset_source();

    let event_loop = EventLoop::new()?;

    #[cfg(feature = "editor")]
    let mut game_app = {
        let window_config = WindowConfig::load_or_default();
        GameApp::new(window_config)
    };

    #[cfg(not(feature = "editor"))]
    let mut game_app = GameApp::new();

    event_loop.run_app(&mut game_app)?;

    Ok(())
}

fn init_asset_source() {
    use rust_engine::engine::assets::{asset_source, content_root};

    // In standalone builds, prefer game.pak next to the executable.
    #[cfg(not(feature = "editor"))]
    {
        let pak_candidates = [
            std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|d| d.join("game.pak"))),
            Some(std::path::PathBuf::from("game.pak")),
        ];

        for candidate in pak_candidates.iter().flatten() {
            if candidate.is_file() {
                println!("Using pak: {}", candidate.display());
                asset_source::init_pak(candidate);
                return;
            }
        }
    }

    // Fallback (and editor): use loose filesystem content directory.
    let root = content_root::content_root();
    println!("Using content dir: {}", root.display());
    asset_source::init_filesystem(root);
}
