//! Main application state and orchestration
//!
//! The App struct holds all engine state and provides methods for
//! initialization, update, and rendering.

use super::{game_setup, gui_panel, input_handler, render_loop};
use hecs::World;
use rust_engine::assets::{AssetManager, HotReloadWatcher, ReloadEvent};
use egui_dock::DockArea;
use rust_engine::engine::editor::{
    create_editor_dock_style, render_menu_bar, CommandHistory, EditorContext, EditorDockState,
    EditorTabViewer, HierarchyPanel, InspectorPanel, LogFilter, LogMessage, MenuAction, Selection,
    ViewportTexture, WindowConfig,
};
use rust_engine::engine::gui::Gui;
use rust_engine::engine::physics::PhysicsWorld;
use rust_engine::engine::rendering::rendering_3d::deferred_renderer::DebugView;
use rust_engine::engine::rendering::rendering_3d::DeferredRenderer;
use rust_engine::engine::scene::save_scene;
use rust_engine::{GameLoop, InputManager, Renderer};
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use vulkano::descriptor_set::DescriptorSet;
use vulkano::sync::GpuFuture;
use winit::event::{MouseScrollDelta, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::Window;

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
    pub selection: Selection,
    pub command_history: CommandHistory,
    pub dock_state: EditorDockState,
    pub console_messages: Vec<LogMessage>,
    pub log_filter: LogFilter,
    // Viewport rendering
    pub viewport_texture: ViewportTexture,
    pub viewport_texture_id: Option<egui::TextureId>,
    pub viewport_size: (u32, u32),
    /// Flag to force viewport/G-Buffer sync on next frame (after swapchain recreation)
    pub pending_viewport_sync: bool,
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
            selection: Selection::new(),
            command_history: CommandHistory::new(100),
            dock_state: EditorDockState::load_or_default(),
            console_messages: vec![
                LogMessage::info("Engine initialized successfully"),
                LogMessage::info("Scene loaded"),
            ],
            log_filter: LogFilter::default(),
            viewport_texture,
            viewport_texture_id: None, // Registered on first render
            viewport_size: (800, 600),
            pending_viewport_sync: false,
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
        self.input_manager.new_frame();
    }

    /// Update game state (physics, hot-reload)
    pub fn update(&mut self) {
        puffin::profile_function!();

        // Process hot-reload events
        self.process_hot_reload();

        // Update delta time and step physics
        let delta_time = self.game_loop.tick();

        {
            puffin::profile_scope!("physics_step");
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
        puffin::profile_function!();

        // Register viewport texture with egui if not done yet
        if self.viewport_texture_id.is_none() {
            let texture_id = self
                .gui
                .register_native_texture(self.viewport_texture.image_view());
            self.viewport_texture_id = Some(texture_id);
        }

        // Prepare render data
        let mesh_data =
            render_loop::prepare_mesh_data(&self.world, &self.asset_manager, &self.renderer);
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
        let deferred_cb = match self
            .deferred_renderer
            .render(&mesh_data, &light_data, self.viewport_texture.image())
        {
            Ok(cb) => cb,
            Err(e) => {
                eprintln!("Render error: {}", e);
                self.previous_frame_end = Some(render_loop::create_now_future(&self.renderer));
                return Ok(());
            }
        };

        // Render GUI with dock layout
        let entity_count = self.world.len() as usize;
        let game_loop = &self.game_loop;
        let camera_distance = self.camera_distance;
        let renderer = &self.renderer;
        let show_profiler = &mut self.show_profiler;
        let hierarchy_panel = &mut self.hierarchy_panel;
        let inspector_panel = &mut self.inspector_panel;
        let world = &mut self.world;
        let selection = &mut self.selection;
        let command_history = &mut self.command_history;
        let dock_state = &mut self.dock_state;
        let console_messages = &self.console_messages;
        let log_filter = &mut self.log_filter;
        let viewport_texture_id = self.viewport_texture_id;
        let viewport_size = &mut self.viewport_size;

        // We need to capture menu action outside the closure
        let mut menu_action = MenuAction::None;

        let gui_result = match self.gui.render(window, target_image, |ctx| {
            // Render menu bar first (at top)
            menu_action = render_menu_bar(ctx, dock_state, command_history);

            // Stats window (floating, always visible)
            gui_panel::create_stats_window(ctx, entity_count, game_loop, camera_distance, renderer);

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

        // Only process game keyboard input if GUI didn't consume it
        if !gui_result.wants_keyboard {
            // Camera movement
            input_handler::handle_camera_movement(&mut self.renderer, &self.input_manager, 0.1);

            // Camera rotation
            input_handler::handle_camera_rotation(&mut self.renderer, &self.input_manager, 0.05);

            // Debug view toggles
            input_handler::handle_debug_views(
                &self.input_manager,
                &mut self.deferred_renderer,
                &mut self.current_debug_view,
            );

            // Asset management controls
            if self.input_manager.is_key_pressed(KeyCode::KeyR) {
                println!("\nManual reload requested...");
                match self
                    .asset_manager
                    .reload_model_gpu("assets/models/Duck.glb")
                {
                    Ok((new_indices, _)) => {
                        self.mesh_indices = new_indices;
                        println!("Duck model reloaded");
                    }
                    Err(e) => eprintln!("Reload failed: {}", e),
                }
            }

            if self.input_manager.is_key_pressed(KeyCode::KeyC) {
                let stats = self.asset_manager.cache_stats();
                println!("\nAsset Cache Stats: {}", stats);
            }
        }

        // Only process mouse input if GUI didn't consume it
        if !gui_result.wants_pointer {
            input_handler::handle_zoom(
                &mut self.renderer,
                &self.input_manager,
                &mut self.camera_distance,
            );
        }
    }
}
