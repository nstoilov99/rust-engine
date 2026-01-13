//! Main application state and orchestration
//!
//! The App struct holds all engine state and provides methods for
//! initialization, update, and rendering.

use super::{game_setup, input_handler, render_loop};
use hecs::World;
use rust_engine::assets::{AssetManager, HotReloadWatcher, ReloadEvent};
use egui_dock::DockArea;
use rust_engine::engine::editor::{
    create_editor_dock_style, render_menu_bar, CommandHistory, ConsoleCommandSystem,
    EditorCamera, EditorContext, EditorDockState, EditorTabViewer, GizmoHandler, HierarchyPanel,
    IconManager, InspectorPanel, LogFilter, LogMessage, MenuAction, ProfilerPanel, Selection,
    ViewportSettings, ViewportTexture, WindowConfig,
};
use rust_engine::engine::gui::Gui;
use rust_engine::engine::physics::PhysicsWorld;
use rust_engine::engine::rendering::rendering_3d::deferred_renderer::DebugView;
use rust_engine::engine::rendering::rendering_3d::{DeferredRenderer, MeshRenderData};
use rust_engine::engine::scene::save_scene;
use rust_engine::{GameLoop, InputManager, Renderer};
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use vulkano::descriptor_set::DescriptorSet;
use vulkano::sync::GpuFuture;
use winit::event::{MouseScrollDelta, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorGrabMode, Window};

use rust_engine::engine::editor::CameraControlMode;

/// Main application state
pub struct App {
    pub window: Arc<Window>,
    pub renderer: Renderer,
    pub gui: Gui,
    pub asset_manager: Arc<AssetManager>,
    pub hot_reload: HotReloadWatcher,
    pub reload_rx: Receiver<ReloadEvent>,
    pub world: World,
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
        let mut world = World::new();

        // Load or create scene
        let (scene_loaded, root_entities) =
            game_setup::load_or_create_scene(&mut world, mesh_indices[0])?;

        // Only spawn physics test objects for new scenes (not loaded ones)
        // This prevents duplicates when the scene file already contains these entities
        if !scene_loaded {
            game_setup::spawn_physics_test_objects(&mut world, plane_mesh_index, cube_mesh_index);
        }

        // Initialize hierarchy panel with root entity order from loaded scene
        let mut hierarchy_panel = HierarchyPanel::new();
        if !root_entities.is_empty() {
            hierarchy_panel.set_root_order(root_entities);
        }

        // Setup physics
        let mut physics_world = PhysicsWorld::new();
        game_setup::register_physics_entities(&mut physics_world, &mut world);

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
            world,
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
            camera_cursor_locked: false,
            drag_start_cursor_pos: None,
            viewport_settings: ViewportSettings::default(),
            icon_manager: IconManager::new(20, egui::Color32::WHITE),
            icons_loaded: false,
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
    }

    /// Update game state (physics, hot-reload)
    pub fn update(&mut self) {
        rust_engine::profile_function!();

        // Process hot-reload events
        self.process_hot_reload();

        // Update delta time and step physics
        let delta_time = self.game_loop.tick();

        {
            rust_engine::profile_scope!("physics_step");
            self.physics_world.step(delta_time, &mut self.world);
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
                        .world
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
                            &self.world,
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
            let assets_path = std::path::Path::new("assets");
            self.icon_manager.load_toolbar_icons(self.gui.context(), assets_path);
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
            &self.world,
            &self.asset_manager,
            &self.renderer,
            &mut self.mesh_data_buffer,
        );
        let light_data = render_loop::prepare_light_data(&self.world, &self.renderer);

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
        let world = &mut self.world;
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
        let viewport_settings = &mut self.viewport_settings;

        // Icon manager reference (icons are loaded lazily on first frame)
        let icon_manager = if self.icon_manager.has_any_icons() {
            Some(&self.icon_manager)
        } else {
            None
        };

        // We need to capture menu action outside the closure
        let mut menu_action = MenuAction::None;

        let gui_result = match self.gui.render(window, target_image, |ctx| {
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
                viewport_settings,
                icon_manager,
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

        // Handle menu actions
        match menu_action {
            MenuAction::None => {}
            MenuAction::SaveScene => {
                match save_scene(
                    &self.world,
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
                if let Some(desc) = self.command_history.undo(&mut self.world) {
                    println!("Undo: {}", desc);
                }
            }
            MenuAction::Redo => {
                if let Some(desc) = self.command_history.redo(&mut self.world) {
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
        // ESC always works
        if self.input_manager.is_key_just_pressed(KeyCode::Escape) {
            std::process::exit(0);
        }

        // F12 and Ctrl+S are handled in handle_window_event() to avoid timing issues
        // (begin_frame clears keys_just_pressed before render is called)

        // Undo/Redo shortcuts (work even when GUI has focus for editor workflow)
        if self.input_manager.is_key_pressed(KeyCode::ControlLeft) {
            if self.input_manager.is_key_just_pressed(KeyCode::KeyZ) {
                if let Some(desc) = self.command_history.undo(&mut self.world) {
                    println!("Undo: {}", desc);
                }
            }
            if self.input_manager.is_key_just_pressed(KeyCode::KeyY) {
                if let Some(desc) = self.command_history.redo(&mut self.world) {
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
        // Block camera when a popup is open AND user is interacting (dragging slider, etc)
        // This prevents camera movement when dragging popup widgets over viewport
        let viewport_available = self.viewport_hovered && !gui_result.is_using_pointer;

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
        let camera_mode = self.editor_camera.current_mode();
        let camera_dragging = matches!(
            camera_mode,
            CameraControlMode::Fly | CameraControlMode::Orbit | CameraControlMode::Pan | CameraControlMode::LookDrag
        );

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
