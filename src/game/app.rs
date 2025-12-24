//! Main application state and orchestration
//!
//! The App struct holds all engine state and provides methods for
//! initialization, update, and rendering.

use super::{game_setup, gui_panel, input_handler, render_loop};
use hecs::World;
use rust_engine::assets::{AssetManager, HotReloadWatcher, ReloadEvent};
use rust_engine::engine::editor::{HierarchyPanel, Selection};
use rust_engine::engine::gui::Gui;
use rust_engine::engine::physics::PhysicsWorld;
use rust_engine::engine::rendering::rendering_3d::deferred_renderer::DebugView;
use rust_engine::engine::rendering::rendering_3d::DeferredRenderer;
use rust_engine::engine::scene::save_scene;
use rust_engine::{GameLoop, InputManager, Renderer};
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use vulkano::descriptor_set::PersistentDescriptorSet;
use vulkano::sync::GpuFuture;
use winit::event::{MouseScrollDelta, VirtualKeyCode, WindowEvent};
use winit::event_loop::ControlFlow;
use winit::window::Window;

/// Main application state
pub struct App {
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
    pub descriptor_set: Arc<PersistentDescriptorSet>,
    pub previous_frame_end: Option<Box<dyn GpuFuture>>,
    pub show_profiler: bool,
    // Editor state
    pub hierarchy_panel: HierarchyPanel,
    pub selection: Selection,
}

impl App {
    /// Create and initialize the application
    pub fn new(window: Arc<Window>) -> Result<Self, Box<dyn std::error::Error>> {
        println!("Rust Game Engine - Starting up...\n");

        // Initialize renderer
        let mut renderer = Renderer::new(window.clone())?;

        // Setup GUI
        println!("Setting up GUI...");
        let swapchain_format = renderer.images[0].format();
        let gui = Gui::new(
            renderer.device.clone(),
            renderer.queue.clone(),
            swapchain_format,
            &window,
        )?;
        println!("GUI initialized");

        // Setup asset system
        let (asset_manager, hot_reload, reload_rx) = game_setup::setup_asset_system(&renderer)?;

        // Load assets
        let (mesh_indices, plane_mesh_index, cube_mesh_index) =
            game_setup::load_assets(&asset_manager)?;

        // Setup ECS World
        println!("Setting up ECS World...");
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
        println!("Creating deferred renderer...");
        let deferred_renderer = DeferredRenderer::new(
            renderer.device.clone(),
            renderer.queue.clone(),
            renderer.memory_allocator.clone(),
            renderer.command_buffer_allocator.clone(),
            renderer.descriptor_set_allocator.clone(),
            800,
            600,
        )?;
        println!("Deferred renderer ready!");

        // Frame synchronization
        let previous_frame_end: Option<Box<dyn GpuFuture>> =
            Some(vulkano::sync::now(renderer.device.clone()).boxed());

        Ok(Self {
            renderer,
            gui,
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
            selection: Selection::new(),
        })
    }

    /// Print control instructions
    pub fn print_controls(&self) {
        game_setup::print_controls();
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

    /// Handle window events
    pub fn handle_window_event(&mut self, event: &WindowEvent, control_flow: &mut ControlFlow) {
        self.gui.handle_event(event);

        match event {
            WindowEvent::CloseRequested => {
                println!("Closing...");
                *control_flow = ControlFlow::Exit;
            }
            WindowEvent::Resized(new_size) => {
                println!("Window resized to {}x{}", new_size.width, new_size.height);
                self.renderer.recreate_swapchain = true;
                self.gui
                    .set_screen_size(new_size.width as f32, new_size.height as f32);
            }
            WindowEvent::KeyboardInput {
                input: keyboard_input,
                ..
            } => {
                // Handle F12 immediately (before begin_frame clears keys_just_pressed)
                if keyboard_input.state == winit::event::ElementState::Pressed {
                    if keyboard_input.virtual_keycode == Some(VirtualKeyCode::F12) {
                        self.show_profiler = !self.show_profiler;
                        println!("Profiler: {}", if self.show_profiler { "ON" } else { "OFF" });
                    }

                    // Handle Ctrl+S immediately for scene save
                    if keyboard_input.virtual_keycode == Some(VirtualKeyCode::S)
                        && self.input_manager.is_key_pressed(VirtualKeyCode::LControl)
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
                self.input_manager
                    .handle_keyboard(keyboard_input.virtual_keycode, keyboard_input.state);
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

        // Prepare render data
        let mesh_data =
            render_loop::prepare_mesh_data(&self.world, &self.asset_manager, &self.renderer);
        let light_data = render_loop::prepare_light_data(&self.world, &self.renderer);

        // Clean up previous frame
        if let Some(mut prev_future) = self.previous_frame_end.take() {
            prev_future.cleanup_finished();
        }

        // Handle swapchain recreation
        if self.renderer.recreate_swapchain {
            match render_loop::handle_swapchain_recreation(
                &mut self.renderer,
                &mut self.deferred_renderer,
                self.current_debug_view,
            ) {
                Ok(false) => {
                    // Window minimized
                    self.previous_frame_end = Some(render_loop::create_now_future(&self.renderer));
                    return Ok(());
                }
                Ok(true) => {}
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

        // Render with deferred pipeline
        let deferred_cb = match self
            .deferred_renderer
            .render(&mesh_data, &light_data, target_image.clone())
        {
            Ok(cb) => cb,
            Err(e) => {
                eprintln!("Render error: {}", e);
                self.previous_frame_end = Some(render_loop::create_now_future(&self.renderer));
                return Ok(());
            }
        };

        // Render GUI
        let entity_count = self.world.len() as usize;
        let game_loop = &self.game_loop;
        let camera_distance = self.camera_distance;
        let renderer = &self.renderer;
        let show_profiler = &mut self.show_profiler;
        let hierarchy_panel = &mut self.hierarchy_panel;
        let world = &mut self.world;
        let selection = &mut self.selection;

        let gui_result = match self.gui.render(window, target_image, |ctx| {
            gui_panel::create_stats_window(ctx, entity_count, game_loop, camera_distance, renderer);
            gui_panel::render_profiler_window(ctx, show_profiler, game_loop);

            // Hierarchy panel
            hierarchy_panel.show(ctx, world, selection);
        }) {
            Ok(result) => result,
            Err(e) => {
                eprintln!("GUI render error: {}", e);
                self.previous_frame_end = Some(render_loop::create_now_future(&self.renderer));
                return Ok(());
            }
        };

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
            Err(e) => {
                eprintln!("Present/flush error: {:?}", e);
                self.previous_frame_end = Some(render_loop::create_now_future(&self.renderer));
            }
        }

        Ok(())
    }

    /// Handle input during frame rendering
    fn handle_frame_input(&mut self, gui_result: &rust_engine::engine::gui::GuiRenderResult) {
        // ESC always works
        if self.input_manager.is_key_just_pressed(VirtualKeyCode::Escape) {
            std::process::exit(0);
        }

        // F12 and Ctrl+S are handled in handle_window_event() to avoid timing issues
        // (begin_frame clears keys_just_pressed before render is called)

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
            if self.input_manager.is_key_pressed(VirtualKeyCode::R) {
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

            if self.input_manager.is_key_pressed(VirtualKeyCode::C) {
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
