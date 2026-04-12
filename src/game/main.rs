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
use rust_engine::engine::editor::{SecondaryWindow, SecondaryWindowKind};
#[cfg(feature = "editor")]
use rust_engine::engine::input::action::{GamepadAxisType, GamepadButton, InputSource};
#[cfg(feature = "editor")]
use rust_engine::engine::input::GamepadState;
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
                        match sec.kind {
                            SecondaryWindowKind::Mesh => {
                                if let Some(data) = app.editor.scene.mesh_editors.get_mut(&sec.editor_key) {
                                    data.open = false;
                                }
                            }
                            SecondaryWindowKind::InputAction => {
                                if let Some(data) = app.editor.scene.input_action_editor.open_actions.get_mut(&sec.editor_key) {
                                    data.open = false;
                                }
                            }
                            SecondaryWindowKind::InputContext => {
                                if let Some(data) = app.editor.scene.input_context_editor.open_contexts.get_mut(&sec.editor_key) {
                                    data.open = false;
                                }
                            }
                            // Built-in panels: closing the window just closes it, no state cleanup needed
                            _ => {}
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
            if let Some(im) = app.core.game_world.resource_mut::<rust_engine::InputManager>() {
                im.handle_raw_mouse_motion(delta.0, delta.1);
            }
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
                // Skip if a window for this key+kind already exists
                let already_exists = secondary_windows.values().any(|s| s.editor_key == req.editor_key && s.kind == req.kind);
                if already_exists {
                    continue;
                }
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
                            req.editor_key,
                            req.kind,
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

        // Clean up closed editor entries so double-click can reopen them.
        // Must run BEFORE the is_empty() early-return, otherwise closed entries
        // persist forever once all secondary windows are gone.
        app.editor.scene.mesh_editors.retain(|_, data| data.open);
        app.editor.scene.input_action_editor.open_actions.retain(|_, data| data.open);
        app.editor.scene.input_context_editor.open_contexts.retain(|_, data| data.open);

        if secondary_windows.is_empty() {
            return;
        }

        // 2. Remove secondary windows for closed/missing editor data.
        secondary_windows.retain(|_, sec| {
            match sec.kind {
                SecondaryWindowKind::Mesh => {
                    app.editor.scene.mesh_editors.get(&sec.editor_key).is_some_and(|d| d.open)
                }
                SecondaryWindowKind::InputAction => {
                    app.editor.scene.input_action_editor.open_actions.get(&sec.editor_key).is_some_and(|d| d.open)
                }
                SecondaryWindowKind::InputContext => {
                    app.editor.scene.input_context_editor.open_contexts.get(&sec.editor_key).is_some_and(|d| d.open)
                }
                // Built-in panels: keep alive (removed only when docked back or closed)
                _ => !sec.dock_requested,
            }
        });

        // 3. Feed gamepad input to any mapping context editor in listening mode.
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
                for data in app.editor.scene.input_context_editor.open_contexts.values_mut() {
                    if data.listening_binding.is_some() {
                        data.pending_external_input = Some(source);
                        break;
                    }
                }
            }
        }

        // 4. Build mesh-preview command buffers (lazy-init, resize, render).
        let preview_cbs = app.build_mesh_preview_cbs();

        // 5. Render each secondary window.
        // Snapshot values before mutable borrows.
        let action_set_snapshot = app.core.game_world
            .resource::<rust_engine::engine::input::subsystem::InputSubsystem>()
            .map(|s| s.action_set.clone());
        let play_mode = app.core.game_world
            .resource::<rust_engine::engine::ecs::resources::PlayMode>()
            .copied()
            .unwrap_or(rust_engine::engine::ecs::resources::PlayMode::Edit);

        let device = app.core.renderer.device.clone();
        let queue = app.core.renderer.queue.clone();

        // Collect dock requests from secondary windows
        let dock_requests: Vec<(String, SecondaryWindowKind)> = {
        let mesh_editors = &mut app.editor.scene.mesh_editors;
        let asset_browser = &mut app.editor.scene.asset_browser;
        let ia_editor = &mut app.editor.scene.input_action_editor;
        let ic_editor = &mut app.editor.scene.input_context_editor;
        let profiler_panel = &mut app.editor.ui.profiler_panel;
        let input_settings_panel = &mut app.editor.ui.input_settings_panel;
        let hierarchy_panel = &mut app.editor.scene.hierarchy_panel;
        let inspector_panel = &mut app.editor.scene.inspector_panel;
        let selection = &mut app.editor.scene.selection;
        let console_messages = &mut app.editor.console.messages;
        let log_filter = &mut app.editor.console.log_filter;
        let world = app.core.game_world.hecs_mut();

        let mut dock_requests: Vec<(String, SecondaryWindowKind)> = Vec::new();

        for sec in secondary_windows.values_mut() {
            let dock_requested = std::cell::Cell::new(false);
            let sec_key = sec.editor_key.clone();
            let sec_kind = sec.kind;

            match sec.kind {
                SecondaryWindowKind::Mesh => {
                    if let Some(data) = mesh_editors.get_mut(&sec.editor_key) {
                        let preview_cb = preview_cbs
                            .iter()
                            .find(|(k, _)| k == &sec.editor_key)
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

                        if let Err(e) = sec.render(device.clone(), queue.clone(), preview_cb, |ctx| {
                            egui::CentralPanel::default().show(ctx, |ui| {
                                render_dock_button(ui, &dock_requested);
                                rust_engine::engine::editor::mesh_editor::MeshEditorPanel::show(
                                    ui,
                                    data,
                                    asset_browser,
                                );
                            });
                        }) {
                            log::error!("Mesh editor window render error: {}", e);
                        }
                    }
                }
                SecondaryWindowKind::InputAction => {
                    if let Some(data) = ia_editor.open_actions.get_mut(&sec.editor_key) {
                        if let Err(e) = sec.render(device.clone(), queue.clone(), None, |ctx| {
                            egui::CentralPanel::default().show(ctx, |ui| {
                                render_dock_button(ui, &dock_requested);
                                rust_engine::engine::editor::InputActionEditor::show_ui(ui, data);
                            });
                        }) {
                            log::error!("Input action editor window render error: {}", e);
                        }
                    }
                }
                SecondaryWindowKind::InputContext => {
                    if let Some(data) = ic_editor.open_contexts.get_mut(&sec.editor_key) {
                        let available = &ic_editor.available_actions;
                        if let Err(e) = sec.render(device.clone(), queue.clone(), None, |ctx| {
                            egui::CentralPanel::default().show(ctx, |ui| {
                                render_dock_button(ui, &dock_requested);
                                rust_engine::engine::editor::InputContextEditor::show_ui(ui, data, available);
                            });
                        }) {
                            log::error!("Mapping context editor window render error: {}", e);
                        }
                    }
                }
                SecondaryWindowKind::Profiler => {
                    if let Err(e) = sec.render(device.clone(), queue.clone(), None, |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            render_dock_button(ui, &dock_requested);
                            profiler_panel.show_contents(ui);
                        });
                    }) {
                        log::error!("Profiler window render error: {}", e);
                    }
                }
                SecondaryWindowKind::AssetBrowser => {
                    // Pass None for icon_manager — icons are TextureHandles bound to the
                    // main window's egui Context and invalid in secondary windows.
                    // The asset browser falls back to text labels gracefully.
                    if let Err(e) = sec.render(device.clone(), queue.clone(), None, |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            render_dock_button(ui, &dock_requested);
                            asset_browser.show(ui, None);
                        });
                    }) {
                        log::error!("Asset browser window render error: {}", e);
                    }
                }
                SecondaryWindowKind::InputSettings => {
                    let snapshot_ref = action_set_snapshot.as_ref();
                    if let Err(e) = sec.render(device.clone(), queue.clone(), None, |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            render_dock_button(ui, &dock_requested);
                            input_settings_panel.show_contents(ui, snapshot_ref);
                        });
                    }) {
                        log::error!("Input settings window render error: {}", e);
                    }
                }
                SecondaryWindowKind::Hierarchy => {
                    if let Err(e) = sec.render(device.clone(), queue.clone(), None, |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            render_dock_button(ui, &dock_requested);
                            hierarchy_panel.show_contents(ui, world, selection, play_mode);
                        });
                    }) {
                        log::error!("Hierarchy window render error: {}", e);
                    }
                }
                SecondaryWindowKind::Inspector => {
                    if let Err(e) = sec.render(device.clone(), queue.clone(), None, |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            render_dock_button(ui, &dock_requested);
                            inspector_panel.show_contents(ui, world, selection, play_mode, asset_browser);
                        });
                    }) {
                        log::error!("Inspector window render error: {}", e);
                    }
                }
                SecondaryWindowKind::Console => {
                    if let Err(e) = sec.render(device.clone(), queue.clone(), None, |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            render_dock_button(ui, &dock_requested);
                            render_console_panel(ui, console_messages, log_filter);
                        });
                    }) {
                        log::error!("Console window render error: {}", e);
                    }
                }
            }

            if dock_requested.get() {
                sec.dock_requested = true;
                dock_requests.push((sec_key, sec_kind));
            }
        }

        dock_requests
        }; // end borrow scope — all &mut refs to app fields are released

        // Process dock requests (re-dock secondary windows as tabs)
        for (key, kind) in dock_requests {
            app.dock_tab(&key, kind);
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

/// Render a "Dock to Editor" button at the top of a secondary window.
#[cfg(feature = "editor")]
fn render_dock_button(ui: &mut egui::Ui, dock_requested: &std::cell::Cell<bool>) {
    ui.horizontal(|ui| {
        if ui
            .button(
                egui::RichText::new("\u{2B05} Dock to Editor")
                    .small()
                    .color(egui::Color32::from_rgb(180, 200, 220)),
            )
            .on_hover_text("Move this panel back into the main editor window")
            .clicked()
        {
            dock_requested.set(true);
        }
    });
    ui.separator();
}

/// Render a simplified console panel for use in a secondary window.
/// Does not include command execution (requires World access).
#[cfg(feature = "editor")]
fn render_console_panel(
    ui: &mut egui::Ui,
    messages: &mut rust_engine::engine::editor::ConsoleLog,
    filter: &mut rust_engine::engine::editor::LogFilter,
) {
    use rust_engine::engine::editor::LogLevel;

    let (info_count, warn_count, error_count) = messages.counts();

    ui.horizontal(|ui| {
        ui.heading("Console");
        ui.separator();

        let error_fill = if filter.show_error {
            egui::Color32::from_rgba_unmultiplied(100, 50, 50, 180)
        } else {
            egui::Color32::from_gray(45)
        };
        if ui
            .add(
                egui::Button::new(
                    egui::RichText::new(format!("Errors ({})", error_count))
                        .color(if filter.show_error { LogLevel::Error.color() } else { egui::Color32::GRAY }),
                )
                .fill(error_fill)
                .corner_radius(3.0),
            )
            .clicked()
        {
            filter.show_error = !filter.show_error;
        }

        let warn_fill = if filter.show_warning {
            egui::Color32::from_rgba_unmultiplied(100, 80, 40, 180)
        } else {
            egui::Color32::from_gray(45)
        };
        if ui
            .add(
                egui::Button::new(
                    egui::RichText::new(format!("Warnings ({})", warn_count))
                        .color(if filter.show_warning { LogLevel::Warning.color() } else { egui::Color32::GRAY }),
                )
                .fill(warn_fill)
                .corner_radius(3.0),
            )
            .clicked()
        {
            filter.show_warning = !filter.show_warning;
        }

        let info_fill = if filter.show_info {
            egui::Color32::from_rgba_unmultiplied(60, 70, 90, 180)
        } else {
            egui::Color32::from_gray(45)
        };
        if ui
            .add(
                egui::Button::new(
                    egui::RichText::new(format!("Info ({})", info_count))
                        .color(if filter.show_info { LogLevel::Info.color() } else { egui::Color32::GRAY }),
                )
                .fill(info_fill)
                .corner_radius(3.0),
            )
            .clicked()
        {
            filter.show_info = !filter.show_info;
        }
    });
    ui.separator();

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .stick_to_bottom(true)
        .show(ui, |ui| {
            ui.style_mut().interaction.selectable_labels = true;
            let mut shown = 0;
            for message in messages.iter() {
                if filter.should_show(message) {
                    ui.label(message.rich_text());
                    shown += 1;
                }
            }
            if shown == 0 {
                ui.label(egui::RichText::new("No messages").weak().italics());
            }
        });
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
