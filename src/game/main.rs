//! Game entry point
//!
//! This is a minimal entry point that delegates to the App struct.
//! All game logic is organized in the game modules.

#[cfg(feature = "editor")]
mod app;
mod benchmark_runner;
mod game_setup;
mod input_handler;
mod render_loop;

#[cfg(feature = "editor")]
use app::{App, EditorRuntimeFlags};
use rust_engine::engine::utils::WindowConfig;
#[cfg(feature = "editor")]
use std::collections::HashMap;
use std::sync::Arc;
use winit::application::ApplicationHandler;
#[cfg(feature = "editor")]
use winit::dpi::LogicalPosition;
use winit::dpi::LogicalSize;
use winit::event::{DeviceEvent, DeviceId, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
#[cfg(feature = "editor")]
use rust_engine::engine::editor::SecondaryWindow;
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
    secondary_windows: HashMap<WindowId, SecondaryWindow>,
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
            secondary_windows: HashMap::new(),
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

        match App::new(window.clone(), self.runtime_flags) {
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
        // Route secondary window events
        if self.secondary_windows.contains_key(&window_id) {
            if matches!(&event, WindowEvent::CloseRequested) {
                if let Some(sec) = self.secondary_windows.remove(&window_id) {
                    if let Some(ref mut app) = self.app {
                        if let Some(data) =
                            app.editor.scene.mesh_editors.get_mut(&sec.mesh_key)
                        {
                            data.open = false;
                        }
                    }
                }
            } else if let Some(sec) = self.secondary_windows.get_mut(&window_id) {
                match &event {
                    WindowEvent::Resized(_) => sec.handle_resize(),
                    WindowEvent::RedrawRequested => {}
                    _ => {
                        sec.gui.handle_event(&event);
                    }
                }
            }
            return;
        }

        // Main window events
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
            app.core
                .input_manager
                .handle_raw_mouse_motion(delta.0, delta.1);
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // Destructure self to allow split borrows across fields.
        let Self {
            app: ref mut app_opt,
            window: ref window_opt,
            secondary_windows,
            is_minimized,
            ..
        } = self;

        if *is_minimized {
            return;
        }

        let Some(app) = app_opt.as_mut() else { return };
        let Some(window) = window_opt.as_ref() else { return };

        app.begin_frame();
        app.update();
        window.request_redraw();

        // --- Secondary window lifecycle ---

        // 1. Create windows for pending requests (requires ActiveEventLoop).
        let pending = app.drain_pending_window_requests();
        if !pending.is_empty() {
            let device = app.core.renderer.device.clone();
            let queue = app.core.renderer.queue.clone();
            for req in pending {
                let win_attrs = WindowAttributes::default()
                    .with_title(&req.title)
                    .with_inner_size(LogicalSize::new(req.width, req.height));
                match event_loop.create_window(win_attrs) {
                    Ok(win) => {
                        let win = Arc::new(win);
                        let win_id = win.id();
                        match SecondaryWindow::new(
                            win,
                            device.clone(),
                            queue.clone(),
                            req.mesh_key,
                        ) {
                            Ok(sec) => {
                                secondary_windows.insert(win_id, sec);
                            }
                            Err(e) => log::error!("Failed to create secondary window: {}", e),
                        }
                    }
                    Err(e) => log::error!("Failed to create window: {}", e),
                }
            }
        }

        // Clean up closed mesh editor entries so double-click can reopen them.
        // Must run BEFORE the is_empty() early-return, otherwise closed entries
        // persist forever once all secondary windows are gone.
        app.editor.scene.mesh_editors.retain(|_, data| data.open);

        if secondary_windows.is_empty() {
            return;
        }

        // 2. Remove secondary windows for closed/missing mesh editors.
        secondary_windows.retain(|_, sec| {
            app.editor
                .scene
                .mesh_editors
                .get(&sec.mesh_key)
                .is_some_and(|d| d.open)
        });

        // 3. Build mesh-preview command buffers (lazy-init, resize, render).
        //    These are chained into each secondary window's own Vulkan
        //    submission so that the preview texture is rendered and
        //    transitioned within the same chain as the egui sampling —
        //    eliminating cross-submission layout/memory visibility issues.
        let preview_cbs = app.build_mesh_preview_cbs();

        // 4. Render each secondary window with its mesh editor UI.
        let device = app.core.renderer.device.clone();
        let queue = app.core.renderer.queue.clone();
        let mesh_editors = &mut app.editor.scene.mesh_editors;
        let asset_browser = &mut app.editor.scene.asset_browser;

        for sec in secondary_windows.values_mut() {
            if let Some(data) = mesh_editors.get_mut(&sec.mesh_key) {
                // Find a preview CB for this editor (if one was built).
                let preview_cb = preview_cbs
                    .iter()
                    .find(|(k, _)| k == &sec.mesh_key)
                    .map(|(_, cb)| cb.clone());

                // Register/update preview texture with this window's Gui.
                if let Some(ref preview) = data.preview {
                    if !preview.mesh_indices.is_empty() {
                        let iv = preview.texture.image_view();
                        let size = (preview.texture.width(), preview.texture.height());
                        if sec.preview_texture_id.is_none() {
                            if !data.preview_dirty {
                                sec.preview_texture_id =
                                    Some(sec.gui.register_native_texture(iv));
                                sec.preview_texture_size = size;
                            }
                        } else if size != sec.preview_texture_size {
                            if let Some(tid) = sec.preview_texture_id {
                                sec.gui.update_native_texture(tid, iv);
                            }
                            sec.preview_texture_size = size;
                        }
                        data.preview.as_mut().unwrap().texture_id = sec.preview_texture_id;
                    }
                }

                // Render the secondary window (preview CB chained first).
                if let Err(e) = sec.render(device.clone(), queue.clone(), preview_cb, |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        rust_engine::engine::editor::mesh_editor::MeshEditorPanel::show(
                            ui,
                            data,
                            asset_browser,
                        );
                    });
                }) {
                    log::error!("Secondary window render error: {}", e);
                }
            }
        }
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

    let args: Vec<String> = std::env::args().collect();
    let event_loop = EventLoop::new()?;

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
