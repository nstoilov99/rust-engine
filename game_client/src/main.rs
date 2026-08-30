//! Game entry point
//!
//! This is a minimal entry point that delegates to the App struct.
//! All game logic is organized in the game modules.

mod anim_bridge;
#[cfg(feature = "editor")]
mod app;
mod asset_resolve;
mod benchmark_runner;
mod game_setup;
#[cfg(all(not(feature = "editor"), feature = "hud"))]
mod hud;
mod input_handler;
mod interp;
#[cfg(feature = "editor")]
mod listen_server;
mod net;
mod plugin;
mod prediction;
mod render_loop;
mod replication;
mod systems;
mod world_population;

#[cfg(feature = "editor")]
use app::{App, EditorRuntimeFlags};
#[cfg(feature = "editor")]
use rust_engine::engine::input::action::{GamepadAxisType, GamepadButton, InputSource};
#[cfg(feature = "editor")]
use rust_engine::engine::input::GamepadState;
use rust_engine::engine::utils::WindowConfig;
use std::sync::Arc;
use winit::application::ApplicationHandler;
#[cfg(feature = "editor")]
use winit::dpi::LogicalPosition;
use winit::dpi::LogicalSize;
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
    runtime_flags: EditorRuntimeFlags,
    should_exit: bool,
    is_minimized: bool,
    /// When the last frame was pumped — lets window events re-drive frames
    /// while an OS modal move/resize loop starves `about_to_wait`.
    last_frame: std::time::Instant,
}

#[cfg(feature = "editor")]
impl GameApp {
    fn new(window_config: WindowConfig, runtime_flags: EditorRuntimeFlags) -> Self {
        Self {
            window: None,
            app: None,
            window_config,
            runtime_flags,
            should_exit: false,
            is_minimized: false,
            last_frame: std::time::Instant::now(),
        }
    }

    /// Pump one frame from a window event if the normal `about_to_wait`
    /// path is starved (OS modal move/resize loop blocks it on Windows —
    /// without this the whole engine freezes while a window is dragged
    /// or resized).
    fn pump_if_starved(&mut self, event_loop: &ActiveEventLoop) {
        if self.last_frame.elapsed() >= std::time::Duration::from_millis(8) {
            self.run_frame(event_loop);
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

        match App::new(
            window.clone(),
            self.runtime_flags,
            plugin::client_plugin_set(),
        ) {
            Ok(app) => {
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
        window_id: WindowId,
        event: WindowEvent,
    ) {
        // Route torn-off crusty float window events
        {
            let consumed = self
                .app
                .as_mut()
                .is_some_and(|app| app.crusty_float_event(window_id, &event));
            if consumed {
                // A float inside an OS move/resize modal loop starves
                // `about_to_wait`; keep the whole engine rendering from here.
                if matches!(event, WindowEvent::Resized(_) | WindowEvent::Moved(_)) {
                    self.pump_if_starved(event_loop);
                }
                return;
            }
        }

        // Main window events. Gate on the id: events still queued for a
        // just-destroyed float window must not reach the main gui.
        let Some(app) = &mut self.app else { return };
        let Some(window) = &self.window else { return };
        if window_id != window.id() {
            return;
        }

        match &event {
            WindowEvent::CloseRequested => {
                app.save_layout_on_exit();
                println!("Closing...");
                self.should_exit = true;
                event_loop.exit();
                return;
            }
            WindowEvent::RedrawRequested => {
                // Render directly from `about_to_wait` instead — see comment there.
                return;
            }
            WindowEvent::Resized(new_size) => {
                self.is_minimized = new_size.width == 0 || new_size.height == 0;
            }
            _ => {}
        }

        app.handle_window_event(&event, event_loop);

        // Keep rendering while the main window's own OS modal move/resize
        // loop blocks `about_to_wait`.
        if matches!(event, WindowEvent::Resized(_) | WindowEvent::Moved(_)) && !self.is_minimized {
            self.pump_if_starved(event_loop);
        }

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
            if let Some(im) = app
                .core
                .game_world
                .resource_mut::<rust_engine::InputManager>()
            {
                im.handle_raw_mouse_motion(delta.0, delta.1);
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.run_frame(event_loop);

        // Plugin Manager ▸ Relaunch now (39.8 §5.6). Checked after the frame
        // that drew the button: it saves, spawns the replacement, and then we
        // take the ordinary exit path so `exiting` still runs.
        if self
            .app
            .as_mut()
            .is_some_and(|app| app.take_relaunch_request())
        {
            self.should_exit = true;
            event_loop.exit();
        }
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        println!("Application suspended");
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(app) = &mut self.app {
            app.save_layout_on_exit();
        }
        println!("Application exiting");
    }
}

#[cfg(feature = "editor")]
impl GameApp {
    /// One full engine frame: update + render the main window and crusty
    /// float windows. Normally driven by `about_to_wait`; also pumped from
    /// window events during OS modal loops (see `pump_if_starved`).
    fn run_frame(&mut self, event_loop: &ActiveEventLoop) {
        self.last_frame = std::time::Instant::now();
        if self.is_minimized {
            return;
        }

        let Some(app) = self.app.as_mut() else { return };
        let Some(window) = self.window.as_ref() else {
            return;
        };

        app.begin_frame();
        app.update();

        // Build mesh-preview command buffers before rendering so docked
        // crusty tabs ship theirs with this frame's packet. Each key's CB
        // has exactly one consumer: crusty float window, or the frame packet
        // (docked crusty tab).
        for (key, cb) in app.build_mesh_preview_cbs() {
            if app.crusty_float_hosts_tab(&format!("mesh:{key}")) {
                app.crusty_float_preview_cbs.push((key, cb));
            } else {
                app.crusty_docked_preview_cbs.push((key, cb));
            }
        }
        // Blend space tab previews (ticket 08): keyed by the tab id itself.
        for (tab, cb) in app.build_blend_space_preview_cbs() {
            if app.crusty_float_hosts_tab(&tab) {
                app.crusty_float_preview_cbs.push((tab, cb));
            } else {
                app.crusty_docked_preview_cbs.push((tab, cb));
            }
        }

        // Render here directly — going via request_redraw → DWM gates to
        // vblank on Windows (cap ≈ refresh-1 with G-Sync). Bypasses that.
        let render_result = app.render(window);
        app.end_frame();
        if let Err(e) = render_result {
            eprintln!("Render error: {}", e);
        }

        // Torn-off crusty float windows (own swapchains, main thread).
        app.crusty_spawn_floats(event_loop);
        app.crusty_float_frames();

        // Clean up closed editor entries so double-click can reopen them.
        app.editor.scene.mesh_editors.retain(|_, data| data.open);
        app.editor
            .scene
            .input_action_editor
            .open_actions
            .retain(|_, data| data.open);
        app.editor
            .scene
            .input_context_editor
            .open_contexts
            .retain(|_, data| data.open);

        // Feed gamepad input to any mapping context editor in listening mode.
        {
            let gamepad_source: Option<InputSource> = app
                .core
                .game_world
                .resource::<GamepadState>()
                .and_then(|gs| {
                    // Check buttons
                    const BUTTONS: &[GamepadButton] = &[
                        GamepadButton::South,
                        GamepadButton::East,
                        GamepadButton::West,
                        GamepadButton::North,
                        GamepadButton::LeftBumper,
                        GamepadButton::RightBumper,
                        GamepadButton::LeftTrigger,
                        GamepadButton::RightTrigger,
                        GamepadButton::Select,
                        GamepadButton::Start,
                        GamepadButton::LeftStick,
                        GamepadButton::RightStick,
                        GamepadButton::DPadUp,
                        GamepadButton::DPadDown,
                        GamepadButton::DPadLeft,
                        GamepadButton::DPadRight,
                    ];
                    for &btn in BUTTONS {
                        if gs.is_pressed(btn) {
                            return Some(InputSource::GamepadButton(btn));
                        }
                    }
                    // Check axes (threshold > 0.5)
                    const AXES: &[GamepadAxisType] = &[
                        GamepadAxisType::LeftStickX,
                        GamepadAxisType::LeftStickY,
                        GamepadAxisType::RightStickX,
                        GamepadAxisType::RightStickY,
                        GamepadAxisType::LeftTrigger,
                        GamepadAxisType::RightTrigger,
                    ];
                    for &axis in AXES {
                        if gs.axis_value(axis).abs() > 0.5 {
                            return Some(InputSource::GamepadAxis(axis));
                        }
                    }
                    None
                });

            if let Some(source) = gamepad_source {
                for data in app
                    .editor
                    .scene
                    .input_context_editor
                    .open_contexts
                    .values_mut()
                {
                    if data.listening_binding.is_some() {
                        data.pending_external_input = Some(source);
                        break;
                    }
                }
            }
        }
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

        match standalone::StandaloneApp::new(window.clone(), plugin::client_plugin_set()) {
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
        let Some(_window) = &self.window else { return };

        match &event {
            WindowEvent::CloseRequested => {
                println!("Closing...");
                self.should_exit = true;
                event_loop.exit();
                return;
            }
            WindowEvent::RedrawRequested => {
                // Render directly from `about_to_wait` instead — see comment there.
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
    ) {
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if self.is_minimized {
            return;
        }

        let Some(app) = &mut self.app else { return };
        let Some(window) = &self.window else { return };

        app.begin_frame();
        app.update();
        // Render here directly — going via request_redraw → DWM gates to
        // vblank on Windows (cap ≈ refresh-1 with G-Sync). Bypasses that.
        let render_result = app.render(window);
        app.end_frame();
        if let Err(e) = render_result {
            eprintln!("Render error: {}", e);
        }
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

    let args: Vec<String> = std::env::args().collect();

    // A relaunched editor (39.8 §5.6) waits on the *process handle* of the
    // instance that spawned it before touching any config. Must happen before
    // `WindowConfig::load_or_default()` below — the parent is still writing.
    #[cfg(feature = "editor")]
    if let Some(parent) = rust_engine::engine::editor::relaunch::parse_wait_parent(&args) {
        println!("relaunch: waiting for parent {parent} to exit");
        rust_engine::engine::editor::relaunch::wait_for_parent(parent);
    }

    let event_loop = EventLoop::new()?;
    // Poll keeps the event loop spinning continuously instead of sleeping between
    // events. On Windows the DWM delivers RedrawRequested at vblank, so Wait caps
    // the app to display refresh even when the swapchain itself isn't VSync-bound.
    event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);

    if let Some(config) = benchmark_runner::parse_benchmark_config(&args) {
        let mut benchmark_app = benchmark_runner::BenchmarkApp::new(config);
        event_loop.run_app(&mut benchmark_app)?;
        return Ok(());
    }

    #[cfg(feature = "editor")]
    let mut game_app = {
        let window_config = WindowConfig::load_or_default();
        let runtime_flags = EditorRuntimeFlags::from_args(&args);
        GameApp::new(window_config, runtime_flags)
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
