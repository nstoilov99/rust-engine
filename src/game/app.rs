//! Main application state and orchestration
//!
//! Split into CoreApp (engine core) and EditorApp (editor UI).
//! The App struct composes both, with EditorApp only present in editor builds.

use super::{game_setup, input_handler, render_loop};
use egui_dock::DockArea;
use rust_engine::assets::asset_source;
use rust_engine::assets::AssetType;
use rust_engine::assets::{AssetManager, HotReloadWatcher, ReloadEvent};
use rust_engine::engine::benchmark::{
    load_or_create_benchmark_scene, BenchmarkConfig, BENCHMARK_SCENE_RELATIVE,
};
use rust_engine::engine::ecs::components::{Camera, Transform};
use rust_engine::engine::ecs::events::PlayModeChanged;
use rust_engine::engine::ecs::game_world::GameWorld;
use rust_engine::engine::ecs::hierarchy::TransformCache;
use rust_engine::engine::ecs::resources::Time;
use rust_engine::engine::ecs::resources::{EditorState, PlayMode};
use rust_engine::engine::ecs::schedule::Schedule;
use rust_engine::engine::editor::play_mode::{self, PlayModeSnapshot};
use rust_engine::engine::editor::{
    create_editor_dock_style, render_menu_bar, AssetBrowserEvent, AssetBrowserPanel, BuildDialog,
    CommandHistory, ConsoleCommandSystem, ConsoleLog, EditorCamera, EditorContext, EditorDockState,
    EditorTabViewer, GizmoHandler, HierarchyPanel, IconManager, InspectorPanel, LogFilter,
    LogMessage, MenuAction, ProfilerPanel, RenameTarget, Selection, ViewportSettings,
    ViewportTexture, WindowConfig,
};
use rust_engine::engine::gui::Gui;
use rust_engine::engine::physics::PhysicsWorld;
use rust_engine::engine::rendering::rendering_3d::deferred_renderer::DebugView;
use rust_engine::engine::rendering::rendering_3d::{DeferredRenderer, MeshRenderData};
use rust_engine::engine::rendering::{RenderTarget, ResourceCounters};
use rust_engine::engine::scene::{load_scene, save_scene};
use rust_engine::{GameLoop, InputManager, Renderer};
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use vulkano::descriptor_set::DescriptorSet;
use vulkano::sync::GpuFuture;
use winit::event::{MouseScrollDelta, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorGrabMode, Window};

const MIN_VIEWPORT_SIZE_FOR_CAMERA: u32 = 50;
const MAIN_SCENE_RELATIVE: &str = "scenes/main.scene.ron";

#[derive(Debug, Clone, Copy, Default)]
pub struct EditorRuntimeFlags {
    pub benchmark_tools_enabled: bool,
}

impl EditorRuntimeFlags {
    pub fn from_args(args: &[String]) -> Self {
        Self {
            benchmark_tools_enabled: args.iter().any(|arg| arg == "--editor-benchmark-tools"),
        }
    }
}

/// Saved editor state for restoring after play mode ends.
pub(crate) struct PrePlayCameraState {
    position: glam::Vec3,
    target: glam::Vec3,
    fov: f32,
    near: f32,
    far: f32,
    debug_view: DebugView,
}

/// Core engine state: renderer, ECS, physics, assets, input.
/// Contains zero references to editor or gui types.
#[allow(dead_code)]
pub struct CoreApp {
    pub window: Arc<Window>,
    pub renderer: Renderer,
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
    pub plane_mesh_index: usize,
    pub cube_mesh_index: usize,
    pub descriptor_set: Arc<DescriptorSet>,
    pub previous_frame_end: Option<Box<dyn GpuFuture>>,
    mesh_data_buffer: Vec<MeshRenderData>,
    pub transform_cache: TransformCache,
    #[cfg(debug_assertions)]
    pub debug_draw_buffer: rust_engine::engine::debug_draw::DebugDrawBuffer,
}

/// Viewport rendering, camera, gizmo, and interaction state.
pub struct ViewportState {
    pub texture: ViewportTexture,
    pub texture_id: Option<egui::TextureId>,
    pub size: (u32, u32),
    pub pending_sync: bool,
    pub camera: EditorCamera,
    pub gizmo_handler: GizmoHandler,
    pub grid_visible: bool,
    pub hovered: bool,
    pub rect: egui::Rect,
    pub cursor_locked: bool,
    pub drag_start_cursor_pos: Option<(f32, f32)>,
    pub settings: ViewportSettings,
}

/// Console log, filter, command system.
pub struct ConsoleState {
    pub messages: ConsoleLog,
    pub log_filter: LogFilter,
    pub command_system: ConsoleCommandSystem,
    pub input: String,
}

/// Scene editing panels and undo history.
pub struct SceneEditorState {
    pub hierarchy_panel: HierarchyPanel,
    pub inspector_panel: InspectorPanel,
    pub selection: Selection,
    pub command_history: CommandHistory,
    pub asset_browser: AssetBrowserPanel,
    pub current_scene_relative: String,
    pub current_scene_name: String,
}

/// General editor UI state: dock, profiler, icons, overlays.
pub struct EditorUIState {
    pub gui: Gui,
    pub dock_state: EditorDockState,
    pub show_stat_fps: bool,
    pub show_profiler: bool,
    pub icon_manager: IconManager,
    pub icons_loaded: bool,
    pub profiler_panel: ProfilerPanel,
}

/// Play-mode snapshots and build dialog.
pub struct PlayModeState {
    pub snapshot: Option<PlayModeSnapshot>,
    pub pre_play_camera: Option<PrePlayCameraState>,
    pub build_dialog: BuildDialog,
}

/// Editor-specific state, decomposed into semantic sub-structures.
pub struct EditorApp {
    pub viewport: ViewportState,
    pub console: ConsoleState,
    pub scene: SceneEditorState,
    pub ui: EditorUIState,
    pub play: PlayModeState,
}

/// Main application combining CoreApp and EditorApp.
pub struct App {
    pub core: CoreApp,
    pub editor: EditorApp,
    runtime_flags: EditorRuntimeFlags,
}

impl App {
    pub fn new(
        window: Arc<Window>,
        runtime_flags: EditorRuntimeFlags,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        println!("Rust Game Engine - Starting up...");

        let renderer = Renderer::new(window.clone())?;

        let swapchain_format = renderer.images[0].format();
        let gui = Gui::new(
            renderer.device.clone(),
            renderer.queue.clone(),
            swapchain_format,
            &window,
        )?;

        let (asset_manager, hot_reload, reload_rx) = game_setup::setup_asset_system(&renderer)?;

        let (mesh_indices, plane_mesh_index, cube_mesh_index) =
            game_setup::load_assets(&asset_manager)?;

        let mut game_world = GameWorld::new();

        let (scene_loaded, root_entities) =
            game_setup::load_or_create_scene(game_world.hecs_mut(), mesh_indices[0])?;

        if !scene_loaded {
            game_setup::spawn_physics_test_objects(
                game_world.hecs_mut(),
                plane_mesh_index,
                cube_mesh_index,
            );
        }

        let mut hierarchy_panel = HierarchyPanel::new();
        if !root_entities.is_empty() {
            hierarchy_panel.set_root_order(root_entities);
        }

        let mut physics_world = PhysicsWorld::new();
        game_setup::register_physics_entities(&mut physics_world, game_world.hecs_mut());

        let descriptor_set = game_setup::upload_model_texture(&renderer, &asset_manager)?;

        let deferred_renderer = DeferredRenderer::new(
            renderer.device.clone(),
            renderer.queue.clone(),
            renderer.memory_allocator.clone(),
            renderer.command_buffer_allocator.clone(),
            renderer.descriptor_set_allocator.clone(),
            800,
            600,
        )?;

        let viewport_texture = ViewportTexture::new(
            renderer.device.clone(),
            renderer.memory_allocator.clone(),
            800,
            600,
        )?;

        let previous_frame_end: Option<Box<dyn GpuFuture>> =
            Some(vulkano::sync::now(renderer.device.clone()).boxed());

        let mut profiler_panel = ProfilerPanel::new();
        profiler_panel.register_sink();

        let core = CoreApp {
            renderer,
            window: window.clone(),
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
            plane_mesh_index,
            cube_mesh_index,
            descriptor_set,
            previous_frame_end,
            mesh_data_buffer: Vec::with_capacity(64),
            transform_cache: TransformCache::new(),
            #[cfg(debug_assertions)]
            debug_draw_buffer: rust_engine::engine::debug_draw::DebugDrawBuffer::new(),
        };

        let mut asset_browser = AssetBrowserPanel::new(std::path::PathBuf::from("content"));
        if !runtime_flags.benchmark_tools_enabled {
            asset_browser.set_hidden_paths([std::path::PathBuf::from(BENCHMARK_SCENE_RELATIVE)]);
        }

        let editor = EditorApp {
            viewport: ViewportState {
                texture: viewport_texture,
                texture_id: None,
                size: (800, 600),
                pending_sync: false,
                camera: EditorCamera::new(800.0, 600.0),
                gizmo_handler: GizmoHandler::new(),
                grid_visible: true,
                hovered: false,
                rect: egui::Rect::NOTHING,
                cursor_locked: false,
                drag_start_cursor_pos: None,
                settings: ViewportSettings::default(),
            },
            console: ConsoleState {
                messages: {
                    let mut log = ConsoleLog::new();
                    log.push(LogMessage::info("Engine initialized successfully"));
                    log.push(LogMessage::info("Scene loaded"));
                    log
                },
                log_filter: LogFilter::default(),
                command_system: ConsoleCommandSystem::new(),
                input: String::new(),
            },
            scene: SceneEditorState {
                hierarchy_panel,
                inspector_panel: InspectorPanel::new(),
                selection: Selection::new(),
                command_history: CommandHistory::new(100),
                asset_browser,
                current_scene_relative: MAIN_SCENE_RELATIVE.to_string(),
                current_scene_name: "Main Scene".to_string(),
            },
            ui: EditorUIState {
                gui,
                dock_state: EditorDockState::load_or_default(),
                show_stat_fps: false,
                show_profiler: false,
                icon_manager: IconManager::new(20, egui::Color32::WHITE),
                icons_loaded: false,
                profiler_panel,
            },
            play: PlayModeState {
                snapshot: None,
                pre_play_camera: None,
                build_dialog: BuildDialog::new(),
            },
        };

        Ok(Self {
            core,
            editor,
            runtime_flags,
        })
    }

    pub fn print_controls(&self) {
        game_setup::print_controls();
    }

    pub fn save_layout_on_exit(&self) {
        if let Err(e) = self.editor.ui.dock_state.save_to_default() {
            eprintln!("Warning: Failed to save layout on exit: {}", e);
        }

        let size = self.core.window.inner_size();
        let position = self.core.window.outer_position().unwrap_or_default();
        let is_fullscreen = self.core.window.fullscreen().is_some();
        let is_maximized = self.core.window.is_maximized();

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

    pub fn begin_frame(&mut self) {
        puffin::GlobalProfiler::lock().new_frame();
        #[cfg(feature = "tracy")]
        tracy_client::Client::running().map(|c| c.frame_mark());
        self.core.input_manager.new_frame();
        self.core.game_world.begin_frame();
    }

    pub fn update(&mut self) {
        rust_engine::profile_function!();

        self.process_hot_reload();

        let delta_time = self.core.game_loop.tick();

        if let Some(time) = self.core.game_world.resource_mut::<Time>() {
            time.advance(delta_time);
        }

        self.core.game_world.run_schedule(&mut self.core.schedule);

        if self.play_mode() == PlayMode::Playing {
            rust_engine::profile_scope!("physics_step");
            self.core
                .physics_world
                .step(delta_time, self.core.game_world.hecs_mut());
        }

        // Update debug draw persistent line lifetimes
        #[cfg(debug_assertions)]
        self.core.debug_draw_buffer.update(delta_time);
    }

    fn process_hot_reload(&mut self) {
        while let Ok(event) = self.core.reload_rx.try_recv() {
            match event {
                ReloadEvent::ModelChanged {
                    path,
                    mesh_indices: new_indices,
                    model: _,
                } => {
                    for (_entity, mesh_renderer) in
                        self.core
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

    pub fn handle_window_event(&mut self, event: &WindowEvent, _event_loop: &ActiveEventLoop) {
        self.editor.ui.gui.handle_event(event);

        match event {
            WindowEvent::Resized(new_size) => {
                self.core.renderer.recreate_swapchain = true;
                self.editor
                    .ui
                    .gui
                    .set_screen_size(new_size.width as f32, new_size.height as f32);
            }
            WindowEvent::KeyboardInput {
                event: key_event, ..
            } => {
                let keycode = match key_event.physical_key {
                    PhysicalKey::Code(code) => Some(code),
                    _ => None,
                };

                if key_event.state.is_pressed() {
                    if keycode == Some(KeyCode::F12) {
                        self.editor.ui.show_profiler = !self.editor.ui.show_profiler;
                        println!(
                            "Profiler: {}",
                            if self.editor.ui.show_profiler {
                                "ON"
                            } else {
                                "OFF"
                            }
                        );
                    }

                    if keycode == Some(KeyCode::F5) {
                        match self.play_mode() {
                            PlayMode::Edit => self.enter_play_mode(),
                            PlayMode::Playing | PlayMode::Paused => self.stop_play_mode(),
                        }
                    }

                    if keycode == Some(KeyCode::F6) {
                        match self.play_mode() {
                            PlayMode::Playing => self.pause_play_mode(),
                            PlayMode::Paused => self.resume_play_mode(),
                            PlayMode::Edit => {}
                        }
                    }

                    if keycode == Some(KeyCode::KeyS)
                        && self.core.input_manager.is_key_pressed(KeyCode::ControlLeft)
                    {
                        self.save_active_scene();
                    }
                }
                self.core
                    .input_manager
                    .handle_keyboard(keycode, key_event.state);
            }
            WindowEvent::MouseInput { button, state, .. } => {
                self.core.input_manager.handle_mouse_button(*button, *state);
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.core
                    .input_manager
                    .handle_mouse_move(position.x as f32, position.y as f32);
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let scroll = match delta {
                    MouseScrollDelta::LineDelta(_x, y) => *y,
                    MouseScrollDelta::PixelDelta(pos) => pos.y as f32 * 0.01,
                };
                self.core.input_manager.handle_mouse_wheel(scroll);
            }
            WindowEvent::Focused(false) => {
                self.editor.viewport.camera.reset_active_drag();
                if self.editor.viewport.cursor_locked {
                    let _ = self.core.window.set_cursor_grab(CursorGrabMode::None);
                    self.core.window.set_cursor_visible(true);
                    self.editor.viewport.cursor_locked = false;
                    self.core.input_manager.set_use_raw_mouse(false);
                    self.editor.viewport.drag_start_cursor_pos = None;
                }
            }
            _ => {}
        }
    }

    pub fn render(&mut self, window: &Window) -> Result<(), Box<dyn std::error::Error>> {
        rust_engine::profile_function!();

        // Forward structural hierarchy changes (spawn/despawn/reparent) to the
        // transform cache so it performs a full rebuild instead of incremental.
        if self.core.game_world.take_hierarchy_changed() {
            self.core.transform_cache.request_full_propagation();
        }

        // Single-pass authoritative transform propagation for the entire hierarchy.
        self.core
            .transform_cache
            .propagate(self.core.game_world.hecs_mut());

        if self.editor.viewport.texture_id.is_none() {
            let texture_id = self
                .editor
                .ui
                .gui
                .register_native_texture(self.editor.viewport.texture.image_view());
            self.editor.viewport.texture_id = Some(texture_id);
        }

        if !self.editor.ui.icons_loaded {
            let engine_path = std::path::Path::new("engine");
            self.editor
                .ui
                .icon_manager
                .load_toolbar_icons(self.editor.ui.gui.context(), engine_path);
            self.editor
                .ui
                .icon_manager
                .load_asset_browser_icons(self.editor.ui.gui.context(), engine_path);
            self.editor.ui.icons_loaded = true;
        }

        if self.play_mode() != PlayMode::Edit {
            self.sync_camera_from_ecs();
        }

        self.core.renderer.camera_3d.position = self.editor.viewport.camera.position;
        self.core.renderer.camera_3d.target = self.editor.viewport.camera.target;
        self.core.renderer.camera_3d.up = self.editor.viewport.camera.up;
        self.core.renderer.camera_3d.fov = self.editor.viewport.camera.fov;
        self.core.renderer.camera_3d.aspect_ratio = self.editor.viewport.camera.aspect_ratio;

        render_loop::prepare_mesh_data(
            self.core.game_world.hecs(),
            &self.core.asset_manager,
            &self.core.renderer,
            &mut self.core.mesh_data_buffer,
            &self.core.transform_cache,
        );
        let light_data =
            render_loop::prepare_light_data(self.core.game_world.hecs(), &self.core.renderer);

        if let Some(mut prev_future) = self.core.previous_frame_end.take() {
            prev_future.cleanup_finished();
        }

        if self.editor.viewport.pending_sync {
            let (vp_width, vp_height) = self.editor.viewport.size;
            if vp_width > 0 && vp_height > 0 {
                if let Ok(true) = self.editor.viewport.texture.resize(vp_width, vp_height) {
                    if let Some(texture_id) = self.editor.viewport.texture_id {
                        self.editor.ui.gui.update_native_texture(
                            texture_id,
                            self.editor.viewport.texture.image_view(),
                        );
                    }
                    self.editor
                        .viewport
                        .camera
                        .set_viewport_size(vp_width as f32, vp_height as f32);
                    self.core
                        .renderer
                        .camera_3d
                        .set_viewport_size(vp_width as f32, vp_height as f32);
                }
                if let Err(e) = self.core.deferred_renderer.resize(vp_width, vp_height) {
                    eprintln!(
                        "Failed to sync deferred renderer after swapchain recreation: {}",
                        e
                    );
                }
            }
            self.editor.viewport.pending_sync = false;
        }

        if self.core.renderer.recreate_swapchain {
            match render_loop::handle_swapchain_recreation(
                &mut self.core.renderer,
                &mut self.core.deferred_renderer,
            ) {
                Ok(false) => {
                    self.core.previous_frame_end =
                        Some(render_loop::create_now_future(&self.core.renderer));
                    return Ok(());
                }
                Ok(true) => {
                    self.editor.viewport.pending_sync = true;
                    self.editor.ui.gui.clear_framebuffer_cache();
                }
                Err(e) => {
                    self.core.previous_frame_end =
                        Some(render_loop::create_now_future(&self.core.renderer));
                    return Err(e);
                }
            }
        }

        let (image_index, target_image, acquire_future) =
            match render_loop::acquire_swapchain_image(&mut self.core.renderer) {
                Ok(result) => result,
                Err(_) => {
                    self.core.previous_frame_end =
                        Some(render_loop::create_now_future(&self.core.renderer));
                    return Ok(());
                }
            };

        let (vp_width, vp_height) = self.editor.viewport.size;
        if (vp_width != self.editor.viewport.texture.width()
            || vp_height != self.editor.viewport.texture.height())
            && vp_width > 0
            && vp_height > 0
        {
            if let Ok(resized) = self.editor.viewport.texture.resize(vp_width, vp_height) {
                if resized {
                    if let Some(texture_id) = self.editor.viewport.texture_id {
                        self.editor.ui.gui.update_native_texture(
                            texture_id,
                            self.editor.viewport.texture.image_view(),
                        );
                    }
                    self.editor
                        .viewport
                        .camera
                        .set_viewport_size(vp_width as f32, vp_height as f32);
                    self.core
                        .renderer
                        .camera_3d
                        .set_viewport_size(vp_width as f32, vp_height as f32);
                    if let Err(e) = self.core.deferred_renderer.resize(vp_width, vp_height) {
                        eprintln!("Failed to resize deferred renderer: {}", e);
                    }
                }
            }
        }

        let view_proj = self.editor.viewport.camera.view_projection_matrix();
        let camera_pos = self.editor.viewport.camera.position;

        let render_target = RenderTarget::Texture {
            image: self.editor.viewport.texture.image(),
        };

        let is_editing = self.play_mode() == PlayMode::Edit;

        // Submit collider debug wireframes for entities with debug_draw_visible
        #[cfg(debug_assertions)]
        rust_engine::engine::physics::submit_collider_debug_draws(
            self.core.game_world.hecs(),
            &mut self.core.debug_draw_buffer,
        );

        #[cfg(debug_assertions)]
        let debug_draw_data = render_loop::prepare_debug_draw_data(
            &mut self.core.debug_draw_buffer,
            &self.core.renderer,
        );
        #[cfg(not(debug_assertions))]
        let debug_draw_data = rust_engine::engine::debug_draw::DebugDrawData::empty();

        let deferred_cb = match self.core.deferred_renderer.render(
            &self.core.mesh_data_buffer,
            &light_data,
            render_target,
            self.editor.viewport.grid_visible && is_editing,
            view_proj,
            camera_pos,
            &debug_draw_data,
        ) {
            Ok(cb) => cb,
            Err(e) => {
                eprintln!("Render error: {}", e);
                self.core.previous_frame_end =
                    Some(render_loop::create_now_future(&self.core.renderer));
                return Ok(());
            }
        };

        self.editor.ui.profiler_panel.set_runtime_counters(
            self.core.deferred_renderer.render_counters().clone(),
            ResourceCounters::collect(
                self.core.game_world.hecs(),
                &self.core.asset_manager,
                &self.core.physics_world,
            ),
        );

        let current_play_mode = self.play_mode();

        let show_profiler = &mut self.editor.ui.show_profiler;
        let hierarchy_panel = &mut self.editor.scene.hierarchy_panel;
        let inspector_panel = &mut self.editor.scene.inspector_panel;
        let profiler_panel = &mut self.editor.ui.profiler_panel;
        let world = self.core.game_world.hecs_mut();
        let selection = &mut self.editor.scene.selection;
        let command_history = &mut self.editor.scene.command_history;
        let dock_state = &mut self.editor.ui.dock_state;
        let console_messages = &mut self.editor.console.messages;
        let log_filter = &mut self.editor.console.log_filter;
        let console_command_system = &mut self.editor.console.command_system;
        let console_input = &mut self.editor.console.input;
        let show_stat_fps = &mut self.editor.ui.show_stat_fps;
        let fps = self.core.game_loop.fps();
        let delta_ms = self.core.game_loop.delta_ms();
        let viewport_texture_id = self.editor.viewport.texture_id;
        let viewport_size = &mut self.editor.viewport.size;
        let editor_camera = &mut self.editor.viewport.camera;
        let gizmo_handler = &mut self.editor.viewport.gizmo_handler;
        let grid_visible = &mut self.editor.viewport.grid_visible;
        let viewport_hovered = &mut self.editor.viewport.hovered;
        let prev_viewport_rect = self.editor.viewport.rect;
        let mut new_viewport_rect = self.editor.viewport.rect;
        let viewport_settings = &mut self.editor.viewport.settings;

        let icon_manager = if self.editor.ui.icon_manager.has_any_icons() {
            Some(&self.editor.ui.icon_manager)
        } else {
            None
        };

        let asset_browser = &mut self.editor.scene.asset_browser;
        let build_dialog = &mut self.editor.play.build_dialog;

        let mut menu_action = MenuAction::None;
        let mut toolbar_action = MenuAction::None;

        let gui_result =
            match self
                .editor
                .ui
                .gui
                .render(window, target_image, Some(prev_viewport_rect), |ctx| {
                    menu_action = render_menu_bar(
                        ctx,
                        dock_state,
                        command_history,
                        current_play_mode,
                        build_dialog,
                        console_messages,
                        self.runtime_flags.benchmark_tools_enabled,
                    );

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
                        play_mode: current_play_mode,
                        toolbar_action: &mut toolbar_action,
                    };

                    let mut tab_viewer = EditorTabViewer { editor: editor_ctx };

                    DockArea::new(&mut dock_state.dock_state)
                        .style(create_editor_dock_style(ctx))
                        .show(ctx, &mut tab_viewer);
                }) {
                Ok(result) => result,
                Err(e) => {
                    eprintln!("GUI render error: {}", e);
                    self.core.previous_frame_end =
                        Some(render_loop::create_now_future(&self.core.renderer));
                    return Ok(());
                }
            };

        self.editor.viewport.rect = new_viewport_rect;

        if menu_action == MenuAction::None && toolbar_action != MenuAction::None {
            menu_action = toolbar_action;
        }

        match menu_action {
            MenuAction::None => {}
            MenuAction::SaveScene => self.save_active_scene(),
            MenuAction::Exit => {
                self.save_layout_on_exit();
                println!("Closing...");
                std::process::exit(0);
            }
            MenuAction::Undo => {
                if self.play_mode() == PlayMode::Edit {
                    if let Some(desc) = self
                        .editor
                        .scene
                        .command_history
                        .undo(self.core.game_world.hecs_mut())
                    {
                        println!("Undo: {}", desc);
                    }
                }
            }
            MenuAction::Redo => {
                if self.play_mode() == PlayMode::Edit {
                    if let Some(desc) = self
                        .editor
                        .scene
                        .command_history
                        .redo(self.core.game_world.hecs_mut())
                    {
                        println!("Redo: {}", desc);
                    }
                }
            }
            MenuAction::SaveLayout => match self.editor.ui.dock_state.save_to_default() {
                Ok(()) => println!(
                    "Layout saved to {}",
                    EditorDockState::default_layout_path().display()
                ),
                Err(e) => eprintln!("Failed to save layout: {}", e),
            },
            MenuAction::ResetLayout => {
                self.editor.ui.dock_state = EditorDockState::new();
                let _ = self.editor.ui.dock_state.save_to_default();
                println!("Layout reset to default");
            }
            MenuAction::LoadBenchmarkScene => self.load_benchmark_scene(),
            MenuAction::RunBenchmark => self.run_benchmark(),
            MenuAction::Play => self.enter_play_mode(),
            MenuAction::Pause => self.pause_play_mode(),
            MenuAction::Resume => self.resume_play_mode(),
            MenuAction::Stop => self.stop_play_mode(),
        }

        // Process asset browser events
        let asset_events: Vec<_> = self.editor.scene.asset_browser.events.drain().collect();
        for event in asset_events {
            match event {
                AssetBrowserEvent::AssetOpened { id } => {
                    if let Some(metadata) = self.editor.scene.asset_browser.registry.get(id) {
                        if metadata.asset_type == AssetType::Scene {
                            if self.play_mode() != PlayMode::Edit {
                                self.editor.console.messages.push(LogMessage::warning(
                                    "Stop play mode before loading a scene".to_string(),
                                ));
                                continue;
                            }

                            if metadata.path.as_path()
                                == std::path::Path::new(BENCHMARK_SCENE_RELATIVE)
                                && !self.runtime_flags.benchmark_tools_enabled
                            {
                                self.editor.console.messages.push(LogMessage::warning(
                                    "Benchmark scene access is locked behind --editor-benchmark-tools"
                                        .to_string(),
                                ));
                                continue;
                            }

                            let relative = metadata.path.to_string_lossy();

                            self.core.game_world.reset_transients(false);
                            self.editor.scene.selection.clear();
                            self.core.physics_world = PhysicsWorld::new();

                            match load_scene(self.core.game_world.hecs_mut(), &relative) {
                                Ok((scene_name, root_entities)) => {
                                    self.editor
                                        .scene
                                        .hierarchy_panel
                                        .set_root_order(root_entities);
                                    self.editor.scene.current_scene_relative = relative.to_string();
                                    self.editor.scene.current_scene_name = scene_name.clone();
                                    self.core.transform_cache = TransformCache::new();
                                    self.core
                                        .transform_cache
                                        .propagate(self.core.game_world.hecs_mut());
                                    self.editor.console.messages.push(LogMessage::info(format!(
                                        "Loaded scene: {}",
                                        scene_name
                                    )));
                                    println!("Scene loaded: {}", metadata.display_name);
                                }
                                Err(e) => {
                                    self.editor.console.messages.push(LogMessage::error(format!(
                                        "Failed to load scene: {}",
                                        e
                                    )));
                                    eprintln!("Failed to load scene: {}", e);
                                }
                            }
                        }
                    }
                }
                AssetBrowserEvent::AssetDroppedInViewport { id, position, .. } => {
                    println!("Asset {} dropped at {:?}", id.0, position);
                }
                AssetBrowserEvent::RevealInExplorer { path } => {
                    let full_path = self
                        .editor
                        .scene
                        .asset_browser
                        .registry
                        .root_path()
                        .join(&path);
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
                    let full_path = self
                        .editor
                        .scene
                        .asset_browser
                        .registry
                        .root_path()
                        .join(&path);
                    match std::fs::remove_file(&full_path) {
                        Ok(()) => {
                            self.editor
                                .console
                                .messages
                                .push(LogMessage::info(format!("Deleted: {}", path.display())));
                            if self.editor.scene.asset_browser.selection.is_selected(id) {
                                self.editor.scene.asset_browser.selection.remove(id);
                            }
                            self.editor.scene.asset_browser.request_rescan();
                        }
                        Err(e) => {
                            self.editor.console.messages.push(LogMessage::error(format!(
                                "Failed to delete {}: {}",
                                path.display(),
                                e
                            )));
                            eprintln!("Failed to delete file: {}", e);
                        }
                    }
                }
                AssetBrowserEvent::AssetRenamed {
                    id,
                    old_name,
                    new_name,
                } => {
                    if let Some(metadata) = self.editor.scene.asset_browser.registry.get(id) {
                        let old_path = self
                            .editor
                            .scene
                            .asset_browser
                            .registry
                            .root_path()
                            .join(&metadata.path);
                        let extension = old_path
                            .extension()
                            .map(|e| format!(".{}", e.to_string_lossy()))
                            .unwrap_or_default();
                        let new_filename = format!("{}{}", new_name, extension);
                        let new_path = old_path
                            .parent()
                            .map(|p| p.join(&new_filename))
                            .unwrap_or_else(|| std::path::PathBuf::from(&new_filename));

                        if new_path.exists() && new_path != old_path {
                            self.editor.console.messages.push(LogMessage::error(format!(
                                "Cannot rename: '{}' already exists",
                                new_filename
                            )));
                        } else if new_name.is_empty() || new_name.trim().is_empty() {
                            self.editor.console.messages.push(LogMessage::error(
                                "Cannot rename: name cannot be empty".to_string(),
                            ));
                        } else if new_name.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|'])
                        {
                            self.editor.console.messages.push(LogMessage::error(
                                "Cannot rename: name contains invalid characters".to_string(),
                            ));
                        } else {
                            match std::fs::rename(&old_path, &new_path) {
                                Ok(()) => {
                                    self.editor.console.messages.push(LogMessage::info(format!(
                                        "Renamed '{}' to '{}'",
                                        old_name, new_name
                                    )));
                                    self.editor.scene.asset_browser.request_rescan();
                                }
                                Err(e) => {
                                    self.editor.console.messages.push(LogMessage::error(format!(
                                        "Failed to rename '{}': {}",
                                        old_name, e
                                    )));
                                    eprintln!("Failed to rename file: {}", e);
                                }
                            }
                        }
                    }
                }
                AssetBrowserEvent::CreateFolder { parent_path } => {
                    let full_parent = self
                        .editor
                        .scene
                        .asset_browser
                        .registry
                        .root_path()
                        .join(&parent_path);
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
                            self.editor
                                .console
                                .messages
                                .push(LogMessage::info(format!("Created folder: {}", new_name)));
                            if !parent_path.as_os_str().is_empty() {
                                self.editor
                                    .scene
                                    .asset_browser
                                    .folder_expanded
                                    .insert(parent_path.clone());
                            }
                            self.editor.scene.asset_browser.request_rescan();
                            let relative_new_folder_path = parent_path.join(&new_name);
                            self.editor.scene.asset_browser.renaming = Some(RenameTarget::Folder {
                                path: relative_new_folder_path,
                                current_name: new_name.clone(),
                            });
                        }
                        Err(e) => {
                            self.editor
                                .console
                                .messages
                                .push(LogMessage::error(format!("Failed to create folder: {}", e)));
                            eprintln!("Failed to create folder: {}", e);
                        }
                    }
                }
                AssetBrowserEvent::FolderDeleted { path } => {
                    let full_path = self
                        .editor
                        .scene
                        .asset_browser
                        .registry
                        .root_path()
                        .join(&path);
                    let result = std::fs::remove_dir(&full_path)
                        .or_else(|_| std::fs::remove_dir_all(&full_path));
                    match result {
                        Ok(()) => {
                            self.editor.console.messages.push(LogMessage::info(format!(
                                "Deleted folder: {}",
                                path.display()
                            )));
                            if self.editor.scene.asset_browser.current_folder == path
                                || self
                                    .editor
                                    .scene
                                    .asset_browser
                                    .current_folder
                                    .starts_with(&path)
                            {
                                if let Some(parent) = path.parent() {
                                    self.editor.scene.asset_browser.current_folder =
                                        parent.to_path_buf();
                                } else {
                                    self.editor.scene.asset_browser.current_folder =
                                        std::path::PathBuf::new();
                                }
                            }
                            self.editor.scene.asset_browser.request_rescan();
                        }
                        Err(e) => {
                            self.editor
                                .console
                                .messages
                                .push(LogMessage::error(format!("Failed to delete folder: {}", e)));
                            eprintln!("Failed to delete folder: {}", e);
                        }
                    }
                }
                AssetBrowserEvent::RevealFolderInExplorer { path } => {
                    let full_path = self
                        .editor
                        .scene
                        .asset_browser
                        .registry
                        .root_path()
                        .join(&path);
                    #[cfg(target_os = "windows")]
                    {
                        let _ = std::process::Command::new("explorer")
                            .arg(&full_path)
                            .spawn();
                    }
                    #[cfg(target_os = "macos")]
                    {
                        let _ = std::process::Command::new("open").arg(&full_path).spawn();
                    }
                    #[cfg(target_os = "linux")]
                    {
                        let _ = std::process::Command::new("xdg-open")
                            .arg(&full_path)
                            .spawn();
                    }
                }
                AssetBrowserEvent::AssetMoved {
                    id: _,
                    old_path,
                    new_path,
                } => {
                    let full_old_path = self
                        .editor
                        .scene
                        .asset_browser
                        .registry
                        .root_path()
                        .join(&old_path);
                    let full_new_path = self
                        .editor
                        .scene
                        .asset_browser
                        .registry
                        .root_path()
                        .join(&new_path);

                    if let Some(parent) = full_new_path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }

                    if full_new_path.exists() {
                        self.editor.console.messages.push(LogMessage::error(format!(
                            "Cannot move: '{}' already exists in target folder",
                            new_path
                                .file_name()
                                .map(|n| n.to_string_lossy().to_string())
                                .unwrap_or_else(|| new_path.display().to_string())
                        )));
                    } else {
                        match std::fs::rename(&full_old_path, &full_new_path) {
                            Ok(()) => {
                                self.editor.console.messages.push(LogMessage::info(format!(
                                    "Moved '{}' to '{}'",
                                    old_path
                                        .file_name()
                                        .map(|n| n.to_string_lossy().to_string())
                                        .unwrap_or_else(|| old_path.display().to_string()),
                                    new_path
                                        .parent()
                                        .map(|p| p.display().to_string())
                                        .unwrap_or_else(|| "root".to_string())
                                )));
                                self.editor.scene.asset_browser.request_rescan();
                            }
                            Err(e) => {
                                self.editor
                                    .console
                                    .messages
                                    .push(LogMessage::error(format!("Failed to move file: {}", e)));
                                eprintln!("Failed to move file: {}", e);
                            }
                        }
                    }
                }
                AssetBrowserEvent::FolderMoved { old_path, new_path } => {
                    let full_old_path = self
                        .editor
                        .scene
                        .asset_browser
                        .registry
                        .root_path()
                        .join(&old_path);
                    let mut full_new_path = self
                        .editor
                        .scene
                        .asset_browser
                        .registry
                        .root_path()
                        .join(&new_path);
                    let mut final_new_path = new_path.clone();
                    let mut was_renamed = false;

                    if full_new_path.exists() {
                        let base_name = new_path
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default();
                        let parent = new_path.parent().unwrap_or(std::path::Path::new(""));

                        let mut counter = 1;
                        loop {
                            let new_name = format!("{} ({})", base_name, counter);
                            let candidate = parent.join(&new_name);
                            let full_candidate = self
                                .editor
                                .scene
                                .asset_browser
                                .registry
                                .root_path()
                                .join(&candidate);
                            if !full_candidate.exists() {
                                final_new_path = candidate;
                                full_new_path = full_candidate;
                                was_renamed = true;
                                break;
                            }
                            counter += 1;
                            if counter > 100 {
                                self.editor.console.messages.push(LogMessage::error(format!(
                                    "Cannot move: too many folders named '{}' in target location",
                                    base_name
                                )));
                                continue;
                            }
                        }
                    }

                    match std::fs::rename(&full_old_path, &full_new_path) {
                        Ok(()) => {
                            let original_name = old_path
                                .file_name()
                                .map(|n| n.to_string_lossy().to_string())
                                .unwrap_or_else(|| old_path.display().to_string());
                            let target_dir = final_new_path
                                .parent()
                                .map(|p| p.display().to_string())
                                .unwrap_or_else(|| "root".to_string());

                            if was_renamed {
                                let new_name = final_new_path
                                    .file_name()
                                    .map(|n| n.to_string_lossy().to_string())
                                    .unwrap_or_default();
                                self.editor.console.messages.push(LogMessage::info(format!(
                                    "Moved folder '{}' to '{}' (renamed to '{}')",
                                    original_name, target_dir, new_name
                                )));
                            } else {
                                self.editor.console.messages.push(LogMessage::info(format!(
                                    "Moved folder '{}' to '{}'",
                                    original_name, target_dir
                                )));
                            }

                            if self
                                .editor
                                .scene
                                .asset_browser
                                .current_folder
                                .starts_with(&old_path)
                            {
                                if let Ok(relative) = self
                                    .editor
                                    .scene
                                    .asset_browser
                                    .current_folder
                                    .strip_prefix(&old_path)
                                {
                                    self.editor.scene.asset_browser.current_folder =
                                        final_new_path.join(relative);
                                } else {
                                    self.editor.scene.asset_browser.current_folder =
                                        final_new_path.clone();
                                }
                            }
                            self.editor.scene.asset_browser.request_rescan();
                        }
                        Err(e) => {
                            self.editor
                                .console
                                .messages
                                .push(LogMessage::error(format!("Failed to move folder: {}", e)));
                            eprintln!("Failed to move folder: {}", e);
                        }
                    }
                }
                AssetBrowserEvent::FolderRenamed { old_path, new_path } => {
                    let full_old_path = self
                        .editor
                        .scene
                        .asset_browser
                        .registry
                        .root_path()
                        .join(&old_path);
                    let full_new_path = self
                        .editor
                        .scene
                        .asset_browser
                        .registry
                        .root_path()
                        .join(&new_path);

                    let new_name = new_path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();

                    if new_name.is_empty() || new_name.trim().is_empty() {
                        self.editor.console.messages.push(LogMessage::error(
                            "Cannot rename: folder name cannot be empty".to_string(),
                        ));
                    } else if new_name.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|']) {
                        self.editor.console.messages.push(LogMessage::error(
                            "Cannot rename: folder name contains invalid characters".to_string(),
                        ));
                    } else if full_new_path.exists() && full_new_path != full_old_path {
                        self.editor.console.messages.push(LogMessage::error(format!(
                            "Cannot rename: folder '{}' already exists",
                            new_name
                        )));
                    } else {
                        match std::fs::rename(&full_old_path, &full_new_path) {
                            Ok(()) => {
                                let old_name = old_path
                                    .file_name()
                                    .map(|n| n.to_string_lossy().to_string())
                                    .unwrap_or_else(|| old_path.display().to_string());
                                self.editor.console.messages.push(LogMessage::info(format!(
                                    "Renamed folder '{}' to '{}'",
                                    old_name, new_name
                                )));
                                if self.editor.scene.asset_browser.current_folder == old_path
                                    || self
                                        .editor
                                        .scene
                                        .asset_browser
                                        .current_folder
                                        .starts_with(&old_path)
                                {
                                    if let Ok(relative) = self
                                        .editor
                                        .scene
                                        .asset_browser
                                        .current_folder
                                        .strip_prefix(&old_path)
                                    {
                                        self.editor.scene.asset_browser.current_folder =
                                            new_path.join(relative);
                                    } else {
                                        self.editor.scene.asset_browser.current_folder =
                                            new_path.clone();
                                    }
                                }
                                self.editor.scene.asset_browser.request_rescan();
                            }
                            Err(e) => {
                                self.editor.console.messages.push(LogMessage::error(format!(
                                    "Failed to rename folder: {}",
                                    e
                                )));
                                eprintln!("Failed to rename folder: {}", e);
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        self.handle_frame_input(&gui_result);

        {
            rust_engine::profile_scope!("swapchain_present");
            let future = acquire_future
                .then_execute(self.core.renderer.queue.clone(), deferred_cb)
                .unwrap()
                .then_execute(self.core.renderer.queue.clone(), gui_result.command_buffer)
                .unwrap()
                .then_swapchain_present(
                    self.core.renderer.queue.clone(),
                    vulkano::swapchain::SwapchainPresentInfo::swapchain_image_index(
                        self.core.renderer.swapchain.clone(),
                        image_index,
                    ),
                )
                .then_signal_fence_and_flush();

            match future {
                Ok(future) => {
                    self.core.previous_frame_end = Some(future.boxed());
                }
                Err(_) => {
                    self.core.previous_frame_end =
                        Some(render_loop::create_now_future(&self.core.renderer));
                }
            }
        }

        Ok(())
    }

    fn save_active_scene(&mut self) {
        if self.play_mode() != PlayMode::Edit {
            log::warn!("Cannot save scene during play mode");
            return;
        }

        let scene_relative = self.editor.scene.current_scene_relative.clone();
        let scene_name = self.editor.scene.current_scene_name.clone();
        let scene_path = asset_source::resolve(&scene_relative);

        match save_scene(
            self.core.game_world.hecs(),
            &scene_path.to_string_lossy(),
            &scene_name,
            self.editor.scene.hierarchy_panel.root_order(),
        ) {
            Ok(_) => {
                println!("Scene saved to {}", scene_path.display());
                self.editor
                    .console
                    .messages
                    .push(LogMessage::info(format!("Saved scene: {}", scene_relative)));
            }
            Err(error) => {
                eprintln!("Save failed: {}", error);
                self.editor.console.messages.push(LogMessage::error(format!(
                    "Failed to save scene '{}': {}",
                    scene_relative, error
                )));
            }
        }
    }

    fn load_benchmark_scene(&mut self) {
        if !self.runtime_flags.benchmark_tools_enabled {
            self.editor.console.messages.push(LogMessage::warning(
                "Benchmark tools are disabled. Launch the editor with --editor-benchmark-tools"
                    .to_string(),
            ));
            return;
        }

        if self.play_mode() != PlayMode::Edit {
            self.editor.console.messages.push(LogMessage::warning(
                "Stop play mode before loading the benchmark scene".to_string(),
            ));
            return;
        }

        self.core.game_world.reset_transients(false);
        self.editor.scene.selection.clear();
        let roots = match load_or_create_benchmark_scene(
            self.core.game_world.hecs_mut(),
            &mut self.core.physics_world,
            &BenchmarkConfig::default(),
            self.core.cube_mesh_index,
        ) {
            Ok(roots) => roots,
            Err(error) => {
                self.editor.console.messages.push(LogMessage::error(format!(
                    "Failed to load benchmark scene: {}",
                    error
                )));
                return;
            }
        };
        self.editor.scene.hierarchy_panel.set_root_order(roots);
        self.editor.scene.current_scene_relative = BENCHMARK_SCENE_RELATIVE.to_string();
        self.editor.scene.current_scene_name = "Benchmark Scene".to_string();
        self.core.transform_cache = TransformCache::new();
        self.core
            .transform_cache
            .propagate(self.core.game_world.hecs_mut());

        self.editor
            .console
            .messages
            .push(LogMessage::info("Loaded benchmark scene".to_string()));
    }

    fn run_benchmark(&mut self) {
        if !self.runtime_flags.benchmark_tools_enabled {
            self.editor.console.messages.push(LogMessage::warning(
                "Benchmark tools are disabled. Launch the editor with --editor-benchmark-tools"
                    .to_string(),
            ));
            return;
        }

        match std::env::current_exe() {
            Ok(exe_path) => {
                let result = std::process::Command::new(&exe_path)
                    .args(["--benchmark", "--uncapped"])
                    .spawn();
                match result {
                    Ok(_) => self.editor.console.messages.push(LogMessage::info(format!(
                        "Launched uncapped benchmark runner: {}",
                        exe_path.display()
                    ))),
                    Err(error) => self.editor.console.messages.push(LogMessage::error(format!(
                        "Failed to launch benchmark runner: {error}"
                    ))),
                }
            }
            Err(error) => self.editor.console.messages.push(LogMessage::error(format!(
                "Failed to resolve current executable: {error}"
            ))),
        }
    }

    // === Play Mode Management ===

    pub fn enter_play_mode(&mut self) {
        let current_mode = self
            .core
            .game_world
            .resource::<EditorState>()
            .map(|s| s.play_mode)
            .unwrap_or(PlayMode::Edit);
        if current_mode != PlayMode::Edit {
            log::warn!(
                "enter_play_mode called but not in Edit mode (current: {:?})",
                current_mode
            );
            return;
        }

        self.core.game_world.reset_transients(true);

        match play_mode::create_snapshot(
            self.core.game_world.hecs(),
            &mut self.editor.scene.hierarchy_panel,
            &self.editor.scene.selection,
        ) {
            Ok(snapshot) => {
                self.editor.play.snapshot = Some(snapshot);
            }
            Err(e) => {
                log::error!("Failed to create play mode snapshot: {}", e);
                return;
            }
        }

        if let Some(state) = self.core.game_world.resource_mut::<EditorState>() {
            state.play_mode = PlayMode::Playing;
        }

        play_mode::rebuild_physics(
            &mut self.core.physics_world,
            self.core.game_world.hecs_mut(),
        );

        if let Some(time) = self.core.game_world.resource_mut::<Time>() {
            time.paused = false;
        }

        self.editor.play.pre_play_camera = Some(PrePlayCameraState {
            position: self.editor.viewport.camera.position,
            target: self.editor.viewport.camera.target,
            fov: self.editor.viewport.camera.fov,
            near: self.editor.viewport.camera.near,
            far: self.editor.viewport.camera.far,
            debug_view: self.core.current_debug_view,
        });

        self.core.current_debug_view = DebugView::None;
        self.core.deferred_renderer.set_debug_view(DebugView::None);

        self.sync_camera_from_ecs();

        self.core.game_world.send_event(PlayModeChanged {
            previous: PlayMode::Edit,
            current: PlayMode::Playing,
        });

        log::info!("Entered play mode");
    }

    pub fn stop_play_mode(&mut self) {
        let previous_mode = self
            .core
            .game_world
            .resource::<EditorState>()
            .map(|s| s.play_mode)
            .unwrap_or(PlayMode::Edit);
        if previous_mode == PlayMode::Edit {
            log::warn!("stop_play_mode called but already in Edit mode");
            return;
        }

        if let Some(state) = self.core.game_world.resource_mut::<EditorState>() {
            state.play_mode = PlayMode::Edit;
        }
        if let Some(time) = self.core.game_world.resource_mut::<Time>() {
            time.paused = false;
        }

        self.core.game_world.reset_transients(false);

        if let Some(snapshot) = self.editor.play.snapshot.as_ref() {
            match play_mode::restore_snapshot(
                snapshot,
                &mut self.core.game_world,
                &mut self.editor.scene.hierarchy_panel,
                &mut self.editor.scene.selection,
                &mut self.core.physics_world,
                &mut self.editor.scene.command_history,
            ) {
                Ok(()) => {
                    self.editor.play.snapshot = None;
                }
                Err(e) => {
                    log::error!(
                        "Failed to restore play mode snapshot (snapshot preserved): {}",
                        e
                    );
                }
            }
        } else {
            log::warn!("stop_play_mode called but no snapshot exists");
        }

        if let Some(saved) = self.editor.play.pre_play_camera.take() {
            self.editor.viewport.camera.position = saved.position;
            self.editor.viewport.camera.target = saved.target;
            self.editor.viewport.camera.fov = saved.fov;
            self.editor.viewport.camera.near = saved.near;
            self.editor.viewport.camera.far = saved.far;
            self.core.current_debug_view = saved.debug_view;
            self.core.deferred_renderer.set_debug_view(saved.debug_view);
        }

        self.core.game_world.send_event(PlayModeChanged {
            previous: previous_mode,
            current: PlayMode::Edit,
        });

        log::info!("Stopped play mode, scene restored");
    }

    pub fn pause_play_mode(&mut self) {
        let current_mode = self
            .core
            .game_world
            .resource::<EditorState>()
            .map(|s| s.play_mode)
            .unwrap_or(PlayMode::Edit);
        if current_mode != PlayMode::Playing {
            log::warn!(
                "pause_play_mode called but not Playing (current: {:?})",
                current_mode
            );
            return;
        }

        if let Some(state) = self.core.game_world.resource_mut::<EditorState>() {
            state.play_mode = PlayMode::Paused;
        }
        if let Some(time) = self.core.game_world.resource_mut::<Time>() {
            time.paused = true;
        }

        self.core.game_world.send_event(PlayModeChanged {
            previous: PlayMode::Playing,
            current: PlayMode::Paused,
        });

        log::info!("Play mode paused");
    }

    pub fn resume_play_mode(&mut self) {
        let current_mode = self
            .core
            .game_world
            .resource::<EditorState>()
            .map(|s| s.play_mode)
            .unwrap_or(PlayMode::Edit);
        if current_mode != PlayMode::Paused {
            log::warn!(
                "resume_play_mode called but not Paused (current: {:?})",
                current_mode
            );
            return;
        }

        if let Some(state) = self.core.game_world.resource_mut::<EditorState>() {
            state.play_mode = PlayMode::Playing;
        }
        if let Some(time) = self.core.game_world.resource_mut::<Time>() {
            time.paused = false;
        }

        self.core.game_world.send_event(PlayModeChanged {
            previous: PlayMode::Paused,
            current: PlayMode::Playing,
        });

        log::info!("Play mode resumed");
    }

    pub fn play_mode(&self) -> PlayMode {
        self.core
            .game_world
            .resource::<EditorState>()
            .map(|s| s.play_mode)
            .unwrap_or(PlayMode::Edit)
    }

    /// Syncs the editor camera from the first active ECS Camera entity,
    /// matching the standalone build's behavior exactly.
    fn sync_camera_from_ecs(&mut self) {
        let (vp_w, vp_h) = self.editor.viewport.size;
        let world = self.core.game_world.hecs();
        let cache = &self.core.transform_cache;

        for (entity, (_transform, camera)) in world.query::<(&Transform, &Camera)>().iter() {
            if !camera.active {
                continue;
            }
            let render_mat = cache.get_render(entity);

            let pos = glam::Vec3::new(render_mat[(0, 3)], render_mat[(1, 3)], render_mat[(2, 3)]);
            let forward = glam::Vec3::new(
                -render_mat[(0, 2)],
                -render_mat[(1, 2)],
                -render_mat[(2, 2)],
            );

            self.editor.viewport.camera.position = pos;
            self.editor.viewport.camera.target = pos + forward;
            self.editor.viewport.camera.fov = camera.fov.to_radians();
            self.editor.viewport.camera.near = camera.near;
            self.editor.viewport.camera.far = camera.far;
            self.editor
                .viewport
                .camera
                .set_viewport_size(vp_w as f32, vp_h as f32);
            return;
        }
    }

    fn handle_frame_input(&mut self, gui_result: &rust_engine::engine::gui::GuiRenderResult) {
        if self.play_mode() == PlayMode::Edit
            && self.core.input_manager.is_key_pressed(KeyCode::ControlLeft)
        {
            if self.core.input_manager.is_key_just_pressed(KeyCode::KeyZ) {
                if let Some(desc) = self
                    .editor
                    .scene
                    .command_history
                    .undo(self.core.game_world.hecs_mut())
                {
                    println!("Undo: {}", desc);
                }
            }
            if self.core.input_manager.is_key_just_pressed(KeyCode::KeyY) {
                if let Some(desc) = self
                    .editor
                    .scene
                    .command_history
                    .redo(self.core.game_world.hecs_mut())
                {
                    println!("Redo: {}", desc);
                }
            }
        }

        let gizmo_active = self.editor.viewport.gizmo_handler.is_dragging();
        let delta_time = self.core.game_loop.delta();

        let is_playing = self.play_mode() != PlayMode::Edit;

        self.editor.viewport.camera.mouse_sensitivity =
            self.editor.viewport.settings.mouse_sensitivity;

        let (vp_w, vp_h) = self.editor.viewport.size;
        let viewport_usable =
            vp_w >= MIN_VIEWPORT_SIZE_FOR_CAMERA && vp_h >= MIN_VIEWPORT_SIZE_FOR_CAMERA;

        if is_playing || (!viewport_usable && self.editor.viewport.camera.is_active_drag()) {
            self.editor.viewport.camera.reset_active_drag();
        }

        if !is_playing {
            let viewport_available = (self.editor.viewport.hovered
                || self.editor.viewport.camera.is_active_drag())
                && !gui_result.is_using_pointer
                && viewport_usable;

            self.editor.viewport.camera.update(
                &self.core.input_manager,
                delta_time,
                viewport_available,
                gizmo_active,
                self.editor.viewport.settings.camera_speed,
            );

            if (self.editor.viewport.camera.fly_speed_multiplier - 1.0).abs() > 0.001 {
                let new_speed = (self.editor.viewport.settings.camera_speed
                    * self.editor.viewport.camera.fly_speed_multiplier)
                    .clamp(0.03, 8.0);
                self.editor.viewport.settings.camera_speed = new_speed;
                self.editor.viewport.camera.fly_speed_multiplier = 1.0;
            }
        }

        let camera_dragging = !is_playing && self.editor.viewport.camera.is_active_drag();

        if camera_dragging && !self.editor.viewport.cursor_locked {
            self.editor.viewport.drag_start_cursor_pos =
                Some(self.core.input_manager.mouse_position());
            if self
                .core
                .window
                .set_cursor_grab(CursorGrabMode::Confined)
                .is_err()
            {
                let _ = self.core.window.set_cursor_grab(CursorGrabMode::None);
            }
            self.core.window.set_cursor_visible(false);
            self.editor.viewport.cursor_locked = true;
            self.core.input_manager.set_use_raw_mouse(true);
        } else if !camera_dragging && self.editor.viewport.cursor_locked {
            let _ = self.core.window.set_cursor_grab(CursorGrabMode::None);
            if let Some((x, y)) = self.editor.viewport.drag_start_cursor_pos.take() {
                let pos = winit::dpi::PhysicalPosition::new(x as f64, y as f64);
                let _ = self.core.window.set_cursor_position(pos);
            }
            self.core.window.set_cursor_visible(true);
            self.editor.viewport.cursor_locked = false;
            self.core.input_manager.set_use_raw_mouse(false);
        }

        if !gui_result.wants_keyboard && self.play_mode() == PlayMode::Edit {
            input_handler::handle_debug_views(
                &self.core.input_manager,
                &mut self.core.deferred_renderer,
                &mut self.core.current_debug_view,
            );
        }

        if !gui_result.wants_pointer && !self.editor.viewport.hovered {
            input_handler::handle_zoom(
                &mut self.core.renderer,
                &self.core.input_manager,
                &mut self.core.camera_distance,
            );
        }
    }
}
