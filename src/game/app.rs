//! Main application state and orchestration
//!
//! The App struct holds all engine state and provides methods for
//! initialization, update, and rendering.

use super::{game_setup, input_handler, render_loop};
use rust_engine::assets::{AssetManager, HotReloadWatcher, ReloadEvent};
use rust_engine::engine::ecs::game_world::GameWorld;
use rust_engine::engine::ecs::resources::Time;
use rust_engine::engine::ecs::schedule::Schedule;
use egui_dock::DockArea;
use rust_engine::engine::editor::{
    create_editor_dock_style, render_menu_bar, AssetBrowserEvent, AssetBrowserPanel, CommandHistory,
    ConsoleCommandSystem, EditorCamera, EditorContext, EditorDockState, EditorTabViewer,
    GizmoHandler, HierarchyPanel, IconManager, InspectorPanel, LogFilter, LogMessage, MenuAction,
    ProfilerPanel, RenameTarget, Selection, ViewportSettings, ViewportTexture, WindowConfig,
};
use rust_engine::engine::gui::Gui;
use rust_engine::engine::physics::PhysicsWorld;
use rust_engine::engine::rendering::rendering_3d::deferred_renderer::DebugView;
use rust_engine::engine::rendering::rendering_3d::{DeferredRenderer, MeshRenderData};
use rust_engine::engine::scene::{save_scene, load_scene};
use rust_engine::assets::AssetType;
use rust_engine::{GameLoop, InputManager, Renderer};
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use vulkano::descriptor_set::DescriptorSet;
use vulkano::sync::GpuFuture;
use winit::event::{MouseScrollDelta, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorGrabMode, Window};

/// Minimum viewport dimension (pixels) for camera control to be enabled.
/// Below this size, camera interactions are disabled to prevent erratic behavior.
const MIN_VIEWPORT_SIZE_FOR_CAMERA: u32 = 50;

/// Main application state
pub struct App {
    pub window: Arc<Window>,
    pub renderer: Renderer,
    pub gui: Gui,
    pub asset_manager: Arc<AssetManager>,
    pub hot_reload: HotReloadWatcher,
    pub reload_rx: Receiver<ReloadEvent>,
    pub game_world: GameWorld,
    pub schedule: Schedule,
    pub physics_world: PhysicsWorld,
    pub input_manager: InputManager,
    pub deferred_renderer: DeferredRenderer,
    pub game_loop: GameLoop,
    pub current_debug_view: DebugView,
    pub camera_distance: f32,
    pub mesh_indices: Vec<usize>,
    pub descriptor_set: Arc<DescriptorSet>,
    pub previous_frame_end: Option<Box<dyn GpuFuture>>,
    pub show_profiler: bool,
    // Editor state
    pub hierarchy_panel: HierarchyPanel,
    pub inspector_panel: InspectorPanel,
    pub profiler_panel: ProfilerPanel,
    pub selection: Selection,
    pub command_history: CommandHistory,
    pub dock_state: EditorDockState,
    pub console_messages: Vec<LogMessage>,
    pub log_filter: LogFilter,
    pub console_command_system: ConsoleCommandSystem,
    pub console_input: String,
    /// Toggle for stat fps overlay (Unreal-style)
    pub show_stat_fps: bool,
    // Viewport rendering
    pub viewport_texture: ViewportTexture,
    /// Reusable buffer for mesh render data (avoids per-frame allocation)
    mesh_data_buffer: Vec<MeshRenderData>,
    pub viewport_texture_id: Option<egui::TextureId>,
    pub viewport_size: (u32, u32),
    /// Flag to force viewport/G-Buffer sync on next frame (after swapchain recreation)
    pub pending_viewport_sync: bool,
    // Viewport controls
    /// Editor camera with Unreal-style controls
    pub editor_camera: EditorCamera,
    /// Transform gizmo handler
    pub gizmo_handler: GizmoHandler,
    /// Grid visibility toggle
    pub grid_visible: bool,
    /// Viewport hover state (set by tab_viewer during render)
    pub viewport_hovered: bool,
    /// Previous frame's viewport rect for input blocking (egui screen coordinates)
    pub viewport_rect: egui::Rect,
    /// Track if cursor is locked/hidden during camera drag
    camera_cursor_locked: bool,
    /// Saved cursor position when camera drag starts (for restore on release)
    drag_start_cursor_pos: Option<(f32, f32)>,
    /// Viewport settings (tool mode, snapping, camera speed)
    pub viewport_settings: ViewportSettings,
    /// Icon manager for toolbar icons
    pub icon_manager: IconManager,
    /// Whether icons have been loaded (deferred until first render)
    icons_loaded: bool,
    /// Asset browser panel
    pub asset_browser: AssetBrowserPanel,
}

impl App {
    /// Create and initialize the application
    pub fn new(window: Arc<Window>) -> Result<Self, Box<dyn std::error::Error>> {
        println!("Rust Game Engine - Starting up...");

        // Initialize renderer
        let mut renderer = Renderer::new(window.clone())?;

        // Setup GUI
        let swapchain_format = renderer.images[0].format();
        let gui = Gui::new(
            renderer.device.clone(),
            renderer.queue.clone(),
            swapchain_format,
            &window,
        )?;

        // Setup asset system
        let (asset_manager, hot_reload, reload_rx) = game_setup::setup_asset_system(&renderer)?;

        // Load assets
        let (mesh_indices, plane_mesh_index, cube_mesh_index) =
            game_setup::load_assets(&asset_manager)?;

        // Setup ECS World
        let mut game_world = GameWorld::new();

        // Load or create scene
        let (scene_loaded, root_entities) =
            game_setup::load_or_create_scene(game_world.hecs_mut(), mesh_indices[0])?;

        // Only spawn physics test objects for new scenes (not loaded ones)
        // This prevents duplicates when the scene file already contains these entities
        if !scene_loaded {
            game_setup::spawn_physics_test_objects(game_world.hecs_mut(), plane_mesh_index, cube_mesh_index);
        }

        // Initialize hierarchy panel with root entity order from loaded scene
        let mut hierarchy_panel = HierarchyPanel::new();
        if !root_entities.is_empty() {
            hierarchy_panel.set_root_order(root_entities);
        }

        // Setup physics
        let mut physics_world = PhysicsWorld::new();
        game_setup::register_physics_entities(&mut physics_world, game_world.hecs_mut());

        // Upload model texture
        let descriptor_set = game_setup::upload_model_texture(&renderer, &asset_manager)?;

        // Create deferred renderer
        let deferred_renderer = DeferredRenderer::new(
            renderer.device.clone(),
            renderer.queue.clone(),
            renderer.memory_allocator.clone(),
            renderer.command_buffer_allocator.clone(),
            renderer.descriptor_set_allocator.clone(),
            800,
            600,
        )?;

        // Create viewport texture for rendering scene to egui panel
        let viewport_texture = ViewportTexture::new(
            renderer.device.clone(),
            renderer.memory_allocator.clone(),
            800,
            600,
        )?;

        // Frame synchronization
        let previous_frame_end: Option<Box<dyn GpuFuture>> =
            Some(vulkano::sync::now(renderer.device.clone()).boxed());

        // Create and register profiler panel
        let mut profiler_panel = ProfilerPanel::new();
        profiler_panel.register_sink();

        Ok(Self {
            renderer,
            gui,
            window,
            asset_manager,
            hot_reload,
            reload_rx,
            game_world,
            schedule: Schedule::new(),
            physics_world,
            input_manager: InputManager::new(),
            deferred_renderer,
            game_loop: GameLoop::new(),
            current_debug_view: DebugView::None,
            camera_distance: 5.0,
            mesh_indices,
            descriptor_set,
            previous_frame_end,
            show_profiler: false,
            hierarchy_panel,
            inspector_panel: InspectorPanel::new(),
            profiler_panel,
            selection: Selection::new(),
            command_history: CommandHistory::new(100),
            dock_state: EditorDockState::load_or_default(),
            console_messages: vec![
                LogMessage::info("Engine initialized successfully"),
                LogMessage::info("Scene loaded"),
            ],
            log_filter: LogFilter::default(),
            console_command_system: ConsoleCommandSystem::new(),
            console_input: String::new(),
            show_stat_fps: false,
            viewport_texture,
            mesh_data_buffer: Vec::with_capacity(64), // Pre-allocate for typical scene
            viewport_texture_id: None, // Registered on first render
            viewport_size: (800, 600),
            pending_viewport_sync: false,
            editor_camera: EditorCamera::new(800.0, 600.0),
            gizmo_handler: GizmoHandler::new(),
            grid_visible: true,
            viewport_hovered: false,
            viewport_rect: egui::Rect::NOTHING,
            camera_cursor_locked: false,
            drag_start_cursor_pos: None,
            viewport_settings: ViewportSettings::default(),
            icon_manager: IconManager::new(20, egui::Color32::WHITE),
            icons_loaded: false,
            asset_browser: AssetBrowserPanel::new(std::path::PathBuf::from("content")),
        })
    }

    /// Print control instructions
    pub fn print_controls(&self) {
        game_setup::print_controls();
    }

    /// Save the layout and window state on exit (silently fails on error)
    pub fn save_layout_on_exit(&self) {
        // Save dock layout
        if let Err(e) = self.dock_state.save_to_default() {
            eprintln!("Warning: Failed to save layout on exit: {}", e);
        }

        // Save window state (size, position, fullscreen)
        let size = self.window.inner_size();
        let position = self.window.outer_position().unwrap_or_default();
        let is_fullscreen = self.window.fullscreen().is_some();
        let is_maximized = self.window.is_maximized();

        let window_config = WindowConfig {
            width: size.width,
            height: size.height,
            x: position.x,
            y: position.y,
            maximized: is_maximized,
            fullscreen: is_fullscreen,
        };

        if let Err(e) = window_config.save_to_default() {
            eprintln!("Warning: Failed to save window config on exit: {}", e);
        }
    }

    /// Begin a new frame (call at start of MainEventsCleared)
    pub fn begin_frame(&mut self) {
        puffin::GlobalProfiler::lock().new_frame();
        #[cfg(feature = "tracy")]
        tracy_client::Client::running().map(|c| c.frame_mark());
        self.input_manager.new_frame();
        self.game_world.begin_frame();
    }

    /// Update game state (physics, hot-reload, schedule)
    pub fn update(&mut self) {
        rust_engine::profile_function!();

        // Process hot-reload events
        self.process_hot_reload();

        // Update delta time
        let delta_time = self.game_loop.tick();

        // Advance Time resource
        if let Some(time) = self.game_world.resource_mut::<Time>() {
            time.advance(delta_time);
        }

        // Run scheduled systems
        self.game_world.run_schedule(&mut self.schedule);

        // Step physics
        {
            rust_engine::profile_scope!("physics_step");
            self.physics_world.step(delta_time, self.game_world.hecs_mut());
        }
    }

    /// Process hot-reload events
    fn process_hot_reload(&mut self) {
        while let Ok(event) = self.reload_rx.try_recv() {
            match event {
                ReloadEvent::ModelChanged {
                    path,
                    mesh_indices: new_indices,
                    model: _,
                } => {
                    // Update mesh indices in ECS entities
                    for (_entity, mesh_renderer) in self
                        .game_world
                        .hecs_mut()
                        .query_mut::<&mut rust_engine::engine::ecs::components::MeshRenderer>()
                    {
                        if !new_indices.is_empty() {
                            mesh_renderer.mesh_index = new_indices[0];
                        }
                    }
                    println!("Auto-reload complete: {}", path);
                }
                ReloadEvent::TextureChanged { path } => {
                    println!("Texture auto-reloaded: {}", path);
                }
                ReloadEvent::ReloadFailed { path, error } => {
                    eprintln!("Auto-reload failed for {}: {}", path, error);
                }
            }
        }
    }

    /// Handle window events (winit 0.30 API)
    pub fn handle_window_event(&mut self, event: &WindowEvent, _event_loop: &ActiveEventLoop) {
        self.gui.handle_event(event);

        match event {
            // CloseRequested is handled in main.rs
            WindowEvent::Resized(new_size) => {
                self.renderer.recreate_swapchain = true;
                self.gui
                    .set_screen_size(new_size.width as f32, new_size.height as f32);
            }
            WindowEvent::KeyboardInput { event: key_event, .. } => {
                // Extract physical key code
                let keycode = match key_event.physical_key {
                    PhysicalKey::Code(code) => Some(code),
                    _ => None,
                };

                // Handle F12 immediately (before begin_frame clears keys_just_pressed)
                if key_event.state.is_pressed() {
                    if keycode == Some(KeyCode::F12) {
                        self.show_profiler = !self.show_profiler;
                        println!("Profiler: {}", if self.show_profiler { "ON" } else { "OFF" });
                    }

                    // Handle Ctrl+S immediately for scene save
                    if keycode == Some(KeyCode::KeyS)
                        && self.input_manager.is_key_pressed(KeyCode::ControlLeft)
                    {
                        match save_scene(
                            self.game_world.hecs(),
                            "assets/scenes/main.scene.ron",
                            "Main Scene",
                            self.hierarchy_panel.root_order(),
                        ) {
                            Ok(_) => println!("Scene saved!"),
                            Err(e) => eprintln!("Save failed: {}", e),
                        }
                    }
                }
                self.input_manager.handle_keyboard(keycode, key_event.state);
            }
            WindowEvent::MouseInput { button, state, .. } => {
                self.input_manager.handle_mouse_button(*button, *state);
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.input_manager
                    .handle_mouse_move(position.x as f32, position.y as f32);
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let scroll = match delta {
                    MouseScrollDelta::LineDelta(_x, y) => *y,
                    MouseScrollDelta::PixelDelta(pos) => pos.y as f32 * 0.01,
                };
                self.input_manager.handle_mouse_wheel(scroll);
            }
            WindowEvent::Focused(false) => {
                // Reset camera drag state when window loses focus
                // This prevents cursor from staying locked after Alt+Tab
                self.editor_camera.reset_active_drag();
                if self.camera_cursor_locked {
                    let _ = self.window.set_cursor_grab(CursorGrabMode::None);
                    self.window.set_cursor_visible(true);
                    self.camera_cursor_locked = false;
                    self.input_manager.set_use_raw_mouse(false);
                    self.drag_start_cursor_pos = None;
                }
            }
            _ => {}
        }
    }

    /// Render a frame
    pub fn render(&mut self, window: &Window) -> Result<(), Box<dyn std::error::Error>> {
        rust_engine::profile_function!();

        // Register viewport texture with egui if not done yet
        if self.viewport_texture_id.is_none() {
            let texture_id = self
                .gui
                .register_native_texture(self.viewport_texture.image_view());
            self.viewport_texture_id = Some(texture_id);
        }

        // Load toolbar icons if not loaded yet
        if !self.icons_loaded {
            let engine_path = std::path::Path::new("engine");
            self.icon_manager.load_toolbar_icons(self.gui.context(), engine_path);
            self.icon_manager.load_asset_browser_icons(self.gui.context(), engine_path);
            self.icons_loaded = true;
        }

        // Sync EditorCamera state to renderer.camera_3d for rendering
        // The EditorCamera is the authoritative source for camera position/target
        self.renderer.camera_3d.position = self.editor_camera.position;
        self.renderer.camera_3d.target = self.editor_camera.target;
        self.renderer.camera_3d.up = self.editor_camera.up;
        self.renderer.camera_3d.fov = self.editor_camera.fov;
        self.renderer.camera_3d.aspect_ratio = self.editor_camera.aspect_ratio;

        // Prepare render data (reuses mesh_data_buffer to avoid allocation)
        render_loop::prepare_mesh_data(
            self.game_world.hecs(),
            &self.asset_manager,
            &self.renderer,
            &mut self.mesh_data_buffer,
        );
        let light_data = render_loop::prepare_light_data(self.game_world.hecs(), &self.renderer);

        // Clean up previous frame
        if let Some(mut prev_future) = self.previous_frame_end.take() {
            prev_future.cleanup_finished();
        }

        // FIRST: Apply pending viewport sync from PREVIOUS frame's swapchain recreation
        // This runs BEFORE swapchain recreation check, so the flag set this frame
        // won't be processed until NEXT frame (when viewport_size is fresh from GUI)
        if self.pending_viewport_sync {
            let (vp_width, vp_height) = self.viewport_size;
            if vp_width > 0 && vp_height > 0 {
                // Force viewport texture to match panel size
                if let Ok(true) = self.viewport_texture.resize(vp_width, vp_height) {
                    if let Some(texture_id) = self.viewport_texture_id {
                        self.gui
                            .update_native_texture(texture_id, self.viewport_texture.image_view());
                    }
                    self.editor_camera.set_viewport_size(vp_width as f32, vp_height as f32);
                    self.renderer
                        .camera_3d
                        .set_viewport_size(vp_width as f32, vp_height as f32);
                }
                // Force G-Buffer to match panel size
                if let Err(e) = self.deferred_renderer.resize(vp_width, vp_height) {
                    eprintln!("Failed to sync deferred renderer after swapchain recreation: {}", e);
                }
            }
            self.pending_viewport_sync = false;
        }

        // THEN: Handle swapchain recreation (may set flag for NEXT frame)
        if self.renderer.recreate_swapchain {
            match render_loop::handle_swapchain_recreation(
                &mut self.renderer,
                &mut self.deferred_renderer,
            ) {
                Ok(false) => {
                    // Window minimized
                    self.previous_frame_end = Some(render_loop::create_now_future(&self.renderer));
                    return Ok(());
                }
                Ok(true) => {
                    // Swapchain recreated successfully - schedule viewport/G-Buffer sync for next frame
                    // The flag will be processed at the START of the next frame (above)
                    // By then, viewport_size will be fresh from THIS frame's GUI render
                    self.pending_viewport_sync = true;
                    // Also clear GUI framebuffer cache (swapchain images changed)
                    self.gui.clear_framebuffer_cache();
                }
                Err(e) => {
                    self.previous_frame_end = Some(render_loop::create_now_future(&self.renderer));
                    return Err(e);
                }
            }
        }

        // Acquire swapchain image
        let (image_index, target_image, acquire_future) =
            match render_loop::acquire_swapchain_image(&mut self.renderer) {
                Ok(result) => result,
                Err(_) => {
                    self.previous_frame_end = Some(render_loop::create_now_future(&self.renderer));
                    return Ok(());
                }
            };

        // Handle viewport resize if needed (normal case when panel is resized)
        let (vp_width, vp_height) = self.viewport_size;
        if vp_width != self.viewport_texture.width() || vp_height != self.viewport_texture.height() {
            if vp_width > 0 && vp_height > 0 {
                if let Ok(resized) = self.viewport_texture.resize(vp_width, vp_height) {
                    if resized {
                        // Update the egui texture registration with new image view
                        if let Some(texture_id) = self.viewport_texture_id {
                            self.gui
                                .update_native_texture(texture_id, self.viewport_texture.image_view());
                        }
                        // Update camera aspect ratio to match new viewport dimensions
                        self.editor_camera.set_viewport_size(vp_width as f32, vp_height as f32);
                        self.renderer.camera_3d.set_viewport_size(vp_width as f32, vp_height as f32);
                        // Resize deferred renderer G-Buffer to match new viewport dimensions
                        if let Err(e) = self.deferred_renderer.resize(vp_width, vp_height) {
                            eprintln!("Failed to resize deferred renderer: {}", e);
                        }
                    }
                }
            }
        }

        // Render with deferred pipeline to the VIEWPORT TEXTURE (not swapchain)
        // Calculate view-projection and camera position for grid rendering
        let view_proj = self.editor_camera.view_projection_matrix();
        let camera_pos = self.editor_camera.position;

        let deferred_cb = match self.deferred_renderer.render(
            &self.mesh_data_buffer,
            &light_data,
            self.viewport_texture.image(),
            self.grid_visible,
            view_proj,
            camera_pos,
        ) {
            Ok(cb) => cb,
            Err(e) => {
                eprintln!("Render error: {}", e);
                self.previous_frame_end = Some(render_loop::create_now_future(&self.renderer));
                return Ok(());
            }
        };

        // Render GUI with dock layout
        let show_profiler = &mut self.show_profiler;
        let hierarchy_panel = &mut self.hierarchy_panel;
        let inspector_panel = &mut self.inspector_panel;
        let profiler_panel = &mut self.profiler_panel;
        let world = self.game_world.hecs_mut();
        let selection = &mut self.selection;
        let command_history = &mut self.command_history;
        let dock_state = &mut self.dock_state;
        let console_messages = &mut self.console_messages;
        let log_filter = &mut self.log_filter;
        let console_command_system = &mut self.console_command_system;
        let console_input = &mut self.console_input;
        let show_stat_fps = &mut self.show_stat_fps;
        let fps = self.game_loop.fps();
        let delta_ms = self.game_loop.delta_ms();
        let viewport_texture_id = self.viewport_texture_id;
        let viewport_size = &mut self.viewport_size;
        let editor_camera = &mut self.editor_camera;
        let gizmo_handler = &mut self.gizmo_handler;
        let grid_visible = &mut self.grid_visible;
        let viewport_hovered = &mut self.viewport_hovered;
        // Store previous frame's viewport rect for input blocking, will be updated by render
        let prev_viewport_rect = self.viewport_rect;
        let mut new_viewport_rect = self.viewport_rect;
        let viewport_settings = &mut self.viewport_settings;

        // Icon manager reference (icons are loaded lazily on first frame)
        let icon_manager = if self.icon_manager.has_any_icons() {
            Some(&self.icon_manager)
        } else {
            None
        };

        // Asset browser panel
        let asset_browser = &mut self.asset_browser;

        // We need to capture menu action outside the closure
        let mut menu_action = MenuAction::None;

        let gui_result = match self.gui.render(window, target_image, Some(prev_viewport_rect), |ctx| {
            // Render menu bar first (at top)
            menu_action = render_menu_bar(ctx, dock_state, command_history);

            // Create editor context for tab viewer
            let editor_ctx = EditorContext {
                world,
                selection,
                hierarchy_panel,
                inspector_panel,
                command_history,
                show_profiler,
                console_messages,
                log_filter,
                viewport_texture_id,
                viewport_size,
                profiler_panel,
                console_command_system,
                console_input,
                show_stat_fps,
                fps,
                delta_ms,
                editor_camera,
                gizmo_handler,
                grid_visible,
                viewport_hovered,
                viewport_rect: &mut new_viewport_rect,
                viewport_settings,
                icon_manager,
                asset_browser,
            };

            // Create tab viewer
            let mut tab_viewer = EditorTabViewer { editor: editor_ctx };

            // Render dock area with all panels
            DockArea::new(&mut dock_state.dock_state)
                .style(create_editor_dock_style(ctx))
                .show(ctx, &mut tab_viewer);
        }) {
            Ok(result) => result,
            Err(e) => {
                eprintln!("GUI render error: {}", e);
                self.previous_frame_end = Some(render_loop::create_now_future(&self.renderer));
                return Ok(());
            }
        };

        // Store updated viewport rect for next frame's input blocking
        self.viewport_rect = new_viewport_rect;

        // Handle menu actions
        match menu_action {
            MenuAction::None => {}
            MenuAction::SaveScene => {
                match save_scene(
                    self.game_world.hecs(),
                    "assets/scenes/main.scene.ron",
                    "Main Scene",
                    self.hierarchy_panel.root_order(),
                ) {
                    Ok(_) => println!("Scene saved!"),
                    Err(e) => eprintln!("Save failed: {}", e),
                }
            }
            MenuAction::Exit => {
                self.save_layout_on_exit();
                println!("Closing...");
                std::process::exit(0);
            }
            MenuAction::Undo => {
                if let Some(desc) = self.command_history.undo(self.game_world.hecs_mut()) {
                    println!("Undo: {}", desc);
                }
            }
            MenuAction::Redo => {
                if let Some(desc) = self.command_history.redo(self.game_world.hecs_mut()) {
                    println!("Redo: {}", desc);
                }
            }
            MenuAction::SaveLayout => {
                match self.dock_state.save_to_default() {
                    Ok(()) => println!("Layout saved to {}", EditorDockState::default_layout_path().display()),
                    Err(e) => eprintln!("Failed to save layout: {}", e),
                }
            }
            MenuAction::ResetLayout => {
                self.dock_state = EditorDockState::new();
                // Also save the reset layout so it persists
                let _ = self.dock_state.save_to_default();
                println!("Layout reset to default");
            }
        }

        // Process asset browser events (collect first to avoid borrow conflicts)
        let asset_events: Vec<_> = self.asset_browser.events.drain().collect();
        for event in asset_events {
            match event {
                AssetBrowserEvent::AssetOpened { id } => {
                    if let Some(metadata) = self.asset_browser.registry.get(id) {
                        match metadata.asset_type {
                            AssetType::Scene => {
                                // Build full path to scene file
                                let scene_path = self.asset_browser.registry.root_path()
                                    .join(&metadata.path);
                                let scene_path_str = scene_path.to_string_lossy();

                                // Clear existing world
                                self.game_world.hecs_mut().clear();
                                self.selection.clear();

                                // Load the scene
                                match load_scene(self.game_world.hecs_mut(), &scene_path_str) {
                                    Ok((scene_name, root_entities)) => {
                                        self.hierarchy_panel.set_root_order(root_entities);
                                        self.console_messages.push(LogMessage::info(
                                            format!("Loaded scene: {}", scene_name)
                                        ));
                                        println!("Scene loaded: {}", metadata.display_name);
                                    }
                                    Err(e) => {
                                        self.console_messages.push(LogMessage::error(
                                            format!("Failed to load scene: {}", e)
                                        ));
                                        eprintln!("Failed to load scene: {}", e);
                                    }
                                }
                            }
                            _ => {
                                // Other asset types - could open in inspector or external app
                            }
                        }
                    }
                }
                AssetBrowserEvent::AssetDroppedInViewport { id, position, .. } => {
                    // Will be implemented when viewport drop detection is added
                    println!("Asset {} dropped at {:?}", id.0, position);
                }
                AssetBrowserEvent::RevealInExplorer { path } => {
                    // Open file explorer to the asset's location
                    let full_path = self.asset_browser.registry.root_path().join(&path);
                    #[cfg(target_os = "windows")]
                    {
                        let _ = std::process::Command::new("explorer")
                            .arg("/select,")
                            .arg(&full_path)
                            .spawn();
                    }
                    #[cfg(target_os = "macos")]
                    {
                        let _ = std::process::Command::new("open")
                            .arg("-R")
                            .arg(&full_path)
                            .spawn();
                    }
                    #[cfg(target_os = "linux")]
                    {
                        let _ = std::process::Command::new("xdg-open")
                            .arg(full_path.parent().unwrap_or(&full_path))
                            .spawn();
                    }
                }
                AssetBrowserEvent::AssetDeleted { id, path } => {
                    // Delete the file from disk
                    let full_path = self.asset_browser.registry.root_path().join(&path);
                    match std::fs::remove_file(&full_path) {
                        Ok(()) => {
                            self.console_messages.push(LogMessage::info(
                                format!("Deleted: {}", path.display())
                            ));
                            // Clear selection if deleted asset was selected
                            if self.asset_browser.selection.is_selected(id) {
                                self.asset_browser.selection.remove(id);
                            }
                            // Request rescan to update the registry
                            self.asset_browser.request_rescan();
                        }
                        Err(e) => {
                            self.console_messages.push(LogMessage::error(
                                format!("Failed to delete {}: {}", path.display(), e)
                            ));
                            eprintln!("Failed to delete file: {}", e);
                        }
                    }
                }
                AssetBrowserEvent::AssetRenamed { id, old_name, new_name } => {
                    // Rename the file on disk
                    if let Some(metadata) = self.asset_browser.registry.get(id) {
                        let old_path = self.asset_browser.registry.root_path().join(&metadata.path);

                        // Build new path with new name but preserve extension
                        let extension = old_path.extension()
                            .map(|e| format!(".{}", e.to_string_lossy()))
                            .unwrap_or_default();
                        let new_filename = format!("{}{}", new_name, extension);
                        let new_path = old_path.parent()
                            .map(|p| p.join(&new_filename))
                            .unwrap_or_else(|| std::path::PathBuf::from(&new_filename));

                        // Check if target already exists
                        if new_path.exists() && new_path != old_path {
                            self.console_messages.push(LogMessage::error(
                                format!("Cannot rename: '{}' already exists", new_filename)
                            ));
                        } else if new_name.is_empty() || new_name.trim().is_empty() {
                            self.console_messages.push(LogMessage::error(
                                "Cannot rename: name cannot be empty".to_string()
                            ));
                        } else if new_name.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|']) {
                            self.console_messages.push(LogMessage::error(
                                "Cannot rename: name contains invalid characters".to_string()
                            ));
                        } else {
                            match std::fs::rename(&old_path, &new_path) {
                                Ok(()) => {
                                    self.console_messages.push(LogMessage::info(
                                        format!("Renamed '{}' to '{}'", old_name, new_name)
                                    ));
                                    // Request rescan to update the registry
                                    self.asset_browser.request_rescan();
                                }
                                Err(e) => {
                                    self.console_messages.push(LogMessage::error(
                                        format!("Failed to rename '{}': {}", old_name, e)
                                    ));
                                    eprintln!("Failed to rename file: {}", e);
                                }
                            }
                        }
                    }
                }
                AssetBrowserEvent::CreateFolder { parent_path } => {
                    // Create a new folder inside the specified parent
                    let full_parent = self.asset_browser.registry.root_path().join(&parent_path);

                    // Generate a unique folder name
                    let base_name = "New Folder";
                    let mut new_name = base_name.to_string();
                    let mut counter = 1;

                    while full_parent.join(&new_name).exists() {
                        new_name = format!("{} {}", base_name, counter);
                        counter += 1;
                    }

                    let new_folder_path = full_parent.join(&new_name);
                    match std::fs::create_dir(&new_folder_path) {
                        Ok(()) => {
                            self.console_messages.push(LogMessage::info(
                                format!("Created folder: {}", new_name)
                            ));
                            // Expand parent in folder tree
                            if !parent_path.as_os_str().is_empty() {
                                self.asset_browser.folder_expanded.insert(parent_path.clone());
                            }
                            // Request rescan to show new folder
                            self.asset_browser.request_rescan();

                            // Automatically enter rename mode for the new folder
                            let relative_new_folder_path = parent_path.join(&new_name);
                            self.asset_browser.renaming = Some(RenameTarget::Folder {
                                path: relative_new_folder_path,
                                current_name: new_name.clone(),
                            });
                        }
                        Err(e) => {
                            self.console_messages.push(LogMessage::error(
                                format!("Failed to create folder: {}", e)
                            ));
                            eprintln!("Failed to create folder: {}", e);
                        }
                    }
                }
                AssetBrowserEvent::FolderDeleted { path } => {
                    // Delete the folder from disk
                    let full_path = self.asset_browser.registry.root_path().join(&path);

                    // Try to delete empty folder first, fall back to recursive delete
                    let result = std::fs::remove_dir(&full_path)
                        .or_else(|_| std::fs::remove_dir_all(&full_path));

                    match result {
                        Ok(()) => {
                            self.console_messages.push(LogMessage::info(
                                format!("Deleted folder: {}", path.display())
                            ));
                            // If current folder was the deleted one (or inside it), navigate up
                            if self.asset_browser.current_folder == path ||
                               self.asset_browser.current_folder.starts_with(&path) {
                                if let Some(parent) = path.parent() {
                                    self.asset_browser.current_folder = parent.to_path_buf();
                                } else {
                                    self.asset_browser.current_folder = std::path::PathBuf::new();
                                }
                            }
                            // Request rescan
                            self.asset_browser.request_rescan();
                        }
                        Err(e) => {
                            self.console_messages.push(LogMessage::error(
                                format!("Failed to delete folder: {}", e)
                            ));
                            eprintln!("Failed to delete folder: {}", e);
                        }
                    }
                }
                AssetBrowserEvent::RevealFolderInExplorer { path } => {
                    // Open file explorer to the folder's location
                    let full_path = self.asset_browser.registry.root_path().join(&path);
                    #[cfg(target_os = "windows")]
                    {
                        let _ = std::process::Command::new("explorer")
                            .arg(&full_path)
                            .spawn();
                    }
                    #[cfg(target_os = "macos")]
                    {
                        let _ = std::process::Command::new("open")
                            .arg(&full_path)
                            .spawn();
                    }
                    #[cfg(target_os = "linux")]
                    {
                        let _ = std::process::Command::new("xdg-open")
                            .arg(&full_path)
                            .spawn();
                    }
                }
                AssetBrowserEvent::AssetMoved { id: _, old_path, new_path } => {
                    // Move asset to new folder
                    let full_old_path = self.asset_browser.registry.root_path().join(&old_path);
                    let full_new_path = self.asset_browser.registry.root_path().join(&new_path);

                    // Ensure target directory exists
                    if let Some(parent) = full_new_path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }

                    // Check if target already exists
                    if full_new_path.exists() {
                        self.console_messages.push(LogMessage::error(
                            format!("Cannot move: '{}' already exists in target folder",
                                new_path.file_name()
                                    .map(|n| n.to_string_lossy().to_string())
                                    .unwrap_or_else(|| new_path.display().to_string()))
                        ));
                    } else {
                        match std::fs::rename(&full_old_path, &full_new_path) {
                            Ok(()) => {
                                self.console_messages.push(LogMessage::info(
                                    format!("Moved '{}' to '{}'",
                                        old_path.file_name()
                                            .map(|n| n.to_string_lossy().to_string())
                                            .unwrap_or_else(|| old_path.display().to_string()),
                                        new_path.parent()
                                            .map(|p| p.display().to_string())
                                            .unwrap_or_else(|| "root".to_string()))
                                ));
                                self.asset_browser.request_rescan();
                            }
                            Err(e) => {
                                self.console_messages.push(LogMessage::error(
                                    format!("Failed to move file: {}", e)
                                ));
                                eprintln!("Failed to move file: {}", e);
                            }
                        }
                    }
                }
                AssetBrowserEvent::FolderMoved { old_path, new_path } => {
                    // Move folder to new location
                    let full_old_path = self.asset_browser.registry.root_path().join(&old_path);
                    let mut full_new_path = self.asset_browser.registry.root_path().join(&new_path);
                    let mut final_new_path = new_path.clone();
                    let mut was_renamed = false;

                    // If target already exists, find a unique name with suffix
                    if full_new_path.exists() {
                        let base_name = new_path.file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default();
                        let parent = new_path.parent().unwrap_or(std::path::Path::new(""));

                        let mut counter = 1;
                        loop {
                            let new_name = format!("{} ({})", base_name, counter);
                            let candidate = parent.join(&new_name);
                            let full_candidate = self.asset_browser.registry.root_path().join(&candidate);
                            if !full_candidate.exists() {
                                final_new_path = candidate;
                                full_new_path = full_candidate;
                                was_renamed = true;
                                break;
                            }
                            counter += 1;
                            if counter > 100 {
                                // Safety limit - show error and abort
                                self.console_messages.push(LogMessage::error(
                                    format!("Cannot move: too many folders named '{}' in target location", base_name)
                                ));
                                continue;
                            }
                        }
                    }

                    match std::fs::rename(&full_old_path, &full_new_path) {
                        Ok(()) => {
                            let original_name = old_path.file_name()
                                .map(|n| n.to_string_lossy().to_string())
                                .unwrap_or_else(|| old_path.display().to_string());
                            let target_dir = final_new_path.parent()
                                .map(|p| p.display().to_string())
                                .unwrap_or_else(|| "root".to_string());

                            if was_renamed {
                                let new_name = final_new_path.file_name()
                                    .map(|n| n.to_string_lossy().to_string())
                                    .unwrap_or_default();
                                self.console_messages.push(LogMessage::info(
                                    format!("Moved folder '{}' to '{}' (renamed to '{}')",
                                        original_name, target_dir, new_name)
                                ));
                            } else {
                                self.console_messages.push(LogMessage::info(
                                    format!("Moved folder '{}' to '{}'", original_name, target_dir)
                                ));
                            }

                            // Update current folder if we were inside the moved folder
                            if self.asset_browser.current_folder.starts_with(&old_path) {
                                // Calculate relative path and apply to new location
                                if let Ok(relative) = self.asset_browser.current_folder.strip_prefix(&old_path) {
                                    self.asset_browser.current_folder = final_new_path.join(relative);
                                } else {
                                    self.asset_browser.current_folder = final_new_path.clone();
                                }
                            }
                            self.asset_browser.request_rescan();
                        }
                        Err(e) => {
                            self.console_messages.push(LogMessage::error(
                                format!("Failed to move folder: {}", e)
                            ));
                            eprintln!("Failed to move folder: {}", e);
                        }
                    }
                }
                AssetBrowserEvent::FolderRenamed { old_path, new_path } => {
                    // Rename folder on disk
                    let full_old_path = self.asset_browser.registry.root_path().join(&old_path);
                    let full_new_path = self.asset_browser.registry.root_path().join(&new_path);

                    // Validate new name
                    let new_name = new_path.file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();

                    if new_name.is_empty() || new_name.trim().is_empty() {
                        self.console_messages.push(LogMessage::error(
                            "Cannot rename: folder name cannot be empty".to_string()
                        ));
                    } else if new_name.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|']) {
                        self.console_messages.push(LogMessage::error(
                            "Cannot rename: folder name contains invalid characters".to_string()
                        ));
                    } else if full_new_path.exists() && full_new_path != full_old_path {
                        self.console_messages.push(LogMessage::error(
                            format!("Cannot rename: folder '{}' already exists", new_name)
                        ));
                    } else {
                        match std::fs::rename(&full_old_path, &full_new_path) {
                            Ok(()) => {
                                let old_name = old_path.file_name()
                                    .map(|n| n.to_string_lossy().to_string())
                                    .unwrap_or_else(|| old_path.display().to_string());
                                self.console_messages.push(LogMessage::info(
                                    format!("Renamed folder '{}' to '{}'", old_name, new_name)
                                ));
                                // Update current folder if we were inside the renamed folder
                                if self.asset_browser.current_folder == old_path ||
                                   self.asset_browser.current_folder.starts_with(&old_path) {
                                    if let Ok(relative) = self.asset_browser.current_folder.strip_prefix(&old_path) {
                                        self.asset_browser.current_folder = new_path.join(relative);
                                    } else {
                                        self.asset_browser.current_folder = new_path.clone();
                                    }
                                }
                                self.asset_browser.request_rescan();
                            }
                            Err(e) => {
                                self.console_messages.push(LogMessage::error(
                                    format!("Failed to rename folder: {}", e)
                                ));
                                eprintln!("Failed to rename folder: {}", e);
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        // Handle input
        self.handle_frame_input(&gui_result);

        // Submit command buffers and present
        {
            rust_engine::profile_scope!("gpu_submit");
            let future = acquire_future
                .then_execute(self.renderer.queue.clone(), deferred_cb)
                .unwrap()
                .then_execute(self.renderer.queue.clone(), gui_result.command_buffer)
                .unwrap()
                .then_swapchain_present(
                    self.renderer.queue.clone(),
                    vulkano::swapchain::SwapchainPresentInfo::swapchain_image_index(
                        self.renderer.swapchain.clone(),
                        image_index,
                    ),
                )
                .then_signal_fence_and_flush();

            match future {
                Ok(future) => {
                    self.previous_frame_end = Some(future.boxed());
                }
                Err(_) => {
                    // Expected during minimize/restore - surface becomes incompatible
                    // Swapchain will be recreated on next frame
                    self.previous_frame_end = Some(render_loop::create_now_future(&self.renderer));
                }
            }
        }

        Ok(())
    }

    /// Handle input during frame rendering
    fn handle_frame_input(&mut self, gui_result: &rust_engine::engine::gui::GuiRenderResult) {
        // Note: Escape is handled by individual UI components (asset browser, dialogs, etc.)
        // to provide context-aware behavior (cancel rename, close dialogs, etc.)
        // Use File > Exit menu or Alt+F4 to close the application

        // F12 and Ctrl+S are handled in handle_window_event() to avoid timing issues
        // (begin_frame clears keys_just_pressed before render is called)

        // Undo/Redo shortcuts (work even when GUI has focus for editor workflow)
        if self.input_manager.is_key_pressed(KeyCode::ControlLeft) {
            if self.input_manager.is_key_just_pressed(KeyCode::KeyZ) {
                if let Some(desc) = self.command_history.undo(self.game_world.hecs_mut()) {
                    println!("Undo: {}", desc);
                }
            }
            if self.input_manager.is_key_just_pressed(KeyCode::KeyY) {
                if let Some(desc) = self.command_history.redo(self.game_world.hecs_mut()) {
                    println!("Redo: {}", desc);
                }
            }
        }

        // Update EditorCamera with Unreal-style controls
        // This is done after GUI render so we know viewport hover state
        let gizmo_active = self.gizmo_handler.is_dragging();
        let delta_time = self.game_loop.delta();

        // Sync mouse sensitivity from viewport settings
        self.editor_camera.mouse_sensitivity = self.viewport_settings.mouse_sensitivity;

        // Check if camera should process input
        // Block camera when:
        // - A popup is open AND user is interacting (dragging slider, etc)
        // - Viewport is too small (< MIN_VIEWPORT_SIZE_FOR_CAMERA pixels)
        // Allow camera to continue if already in an active drag (prevents flickering)
        let (vp_w, vp_h) = self.viewport_size;
        let viewport_usable = vp_w >= MIN_VIEWPORT_SIZE_FOR_CAMERA && vp_h >= MIN_VIEWPORT_SIZE_FOR_CAMERA;

        // If viewport becomes unusable during active drag, force end the drag
        // This prevents cursor staying locked when viewport is resized very small
        if !viewport_usable && self.editor_camera.is_active_drag() {
            self.editor_camera.reset_active_drag();
        }

        let viewport_available = (self.viewport_hovered || self.editor_camera.is_active_drag())
            && !gui_result.is_using_pointer
            && viewport_usable;

        self.editor_camera.update(
            &self.input_manager,
            delta_time,
            viewport_available,
            gizmo_active,
            self.viewport_settings.camera_speed,
        );

        // If camera adjusted speed via scroll (RMB+scroll), sync back to viewport_settings
        // The camera's process_fly_mode modifies fly_speed_multiplier when scrolling
        if (self.editor_camera.fly_speed_multiplier - 1.0).abs() > 0.001 {
            // The multiplier changed from 1.0, apply the change to camera_speed
            let new_speed = (self.viewport_settings.camera_speed * self.editor_camera.fly_speed_multiplier)
                .clamp(0.03, 8.0);
            self.viewport_settings.camera_speed = new_speed;
            // Reset multiplier since we absorbed the change into camera_speed
            self.editor_camera.fly_speed_multiplier = 1.0;
        }

        // Lock/hide cursor during camera drag for unlimited rotation
        // Use is_active_drag() instead of just current_mode() for stable cursor locking.
        // This prevents cursor flickering when viewport_hovered toggles rapidly on small viewports.
        let camera_dragging = self.editor_camera.is_active_drag();

        if camera_dragging && !self.camera_cursor_locked {
            // Save cursor position before hiding for restore on release
            self.drag_start_cursor_pos = Some(self.input_manager.mouse_position());
            // Starting camera drag - lock and hide cursor
            if self.window.set_cursor_grab(CursorGrabMode::Confined).is_err() {
                // Fall back to None if Confined not supported
                let _ = self.window.set_cursor_grab(CursorGrabMode::None);
            }
            self.window.set_cursor_visible(false);
            self.camera_cursor_locked = true;
            // Switch to raw mouse input for unlimited rotation
            self.input_manager.set_use_raw_mouse(true);
        } else if !camera_dragging && self.camera_cursor_locked {
            // Ending camera drag - unlock and show cursor
            let _ = self.window.set_cursor_grab(CursorGrabMode::None);
            // Restore cursor position before showing
            if let Some((x, y)) = self.drag_start_cursor_pos.take() {
                let pos = winit::dpi::PhysicalPosition::new(x as f64, y as f64);
                let _ = self.window.set_cursor_position(pos);
            }
            self.window.set_cursor_visible(true);
            self.camera_cursor_locked = false;
            // Switch back to cursor-based input
            self.input_manager.set_use_raw_mouse(false);
        }

        // Only process game keyboard input if GUI didn't consume it
        if !gui_result.wants_keyboard {
            // Debug view toggles
            input_handler::handle_debug_views(
                &self.input_manager,
                &mut self.deferred_renderer,
                &mut self.current_debug_view,
            );
        }

        // Mouse input for non-viewport interactions
        if !gui_result.wants_pointer && !self.viewport_hovered {
            // Legacy zoom control (only when not over viewport)
            input_handler::handle_zoom(
                &mut self.renderer,
                &self.input_manager,
                &mut self.camera_distance,
            );
        }
    }
}
