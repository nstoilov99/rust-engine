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
use rust_engine::engine::ecs::access::SystemDescriptor;
use rust_engine::engine::ecs::components::{Camera, Transform};
use rust_engine::engine::ecs::events::PlayModeChanged;
use rust_engine::engine::ecs::game_world::GameWorld;
use rust_engine::engine::ecs::hierarchy::{HierarchyChanged, TransformCache, TransformPropagationSystem};
use rust_engine::engine::ecs::resources::Time;
use rust_engine::engine::ecs::resources::{EditorState, PlayMode};
use rust_engine::engine::animation::AnimationUpdateSystem;
use rust_engine::engine::audio::{AudioEngine, AudioReloadQueue, AudioSystem};
use rust_engine::engine::ecs::schedule::{RunIfPlaying, Schedule, Stage};
use rust_engine::engine::editor::play_mode::{self, PlayModeSnapshot};
use rust_engine::engine::editor::{
    create_editor_dock_style, render_menu_bar, AssetBrowserEvent, AssetBrowserPanel, BuildDialog,
    CommandHistory, ConsoleCommandSystem, ConsoleLog, EditorCamera, EditorContext, EditorDockState,
    EditorTab, EditorTabViewer, GizmoHandler, GpuThumbnailContext, HierarchyPanel, IconManager,
    ImportDialogAction, ImportDialogState, ImportPreview, InputActionEditor, InputContextEditor,
    InputSettingsPanel, InspectorPanel, LogFilter, LogMessage, MenuAction,
    PendingWindowRequest, ProfilerPanel, RenameTarget, SecondaryWindowKind, Selection,
    ViewportSettings, ViewportTexture, WindowConfig,
};
use rust_engine::engine::gui::Gui;
use rust_engine::engine::physics::PhysicsWorld;
use rust_engine::engine::rendering::frame_packet::FramePacket;
use rust_engine::engine::rendering::render_thread::{RenderThread, RenderThreadConfig};
use rust_engine::engine::rendering::rendering_3d::deferred_renderer::DebugView;
use rust_engine::engine::rendering::rendering_3d::{DeferredRenderer, MeshRenderData, SkinningBackend};
use rust_engine::engine::rendering::ResourceCounters;
use rust_engine::engine::scene::{load_scene, save_scene};
use rust_engine::{GameLoop, InputManager, Renderer};
use rust_engine::engine::input::action_state::ActionState;
use rust_engine::engine::input::gamepad::GamepadState;
use rust_engine::engine::input::serialization;
use rust_engine::engine::input::enhanced_defaults::default_action_set;
use rust_engine::engine::input::enhanced_serialization;
use rust_engine::engine::input::subsystem::{EnhancedInputSystem, InputSubsystem};
use rust_engine::engine::input::event::InputEvent;
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use vulkano::descriptor_set::DescriptorSet;
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
    pub deferred_renderer: DeferredRenderer,
    pub skinning: SkinningBackend,
    pub game_loop: GameLoop,
    pub current_debug_view: DebugView,
    pub camera_distance: f32,
    pub mesh_indices: Vec<usize>,
    pub plane_mesh_index: usize,
    pub cube_mesh_index: usize,
    pub descriptor_set: Arc<DescriptorSet>,
    mesh_data_buffer: Vec<MeshRenderData>,
    shadow_caster_buffer: Vec<MeshRenderData>,
    plankton_emitter_buffer: Vec<rust_engine::engine::rendering::frame_packet::PlanktonEmitterFrameData>,
    frame_number: u64,
    pub render_thread: Option<RenderThread>,
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
    /// Model import dialog state (shown when model files are dropped).
    pub import_dialog: Option<ImportDialogState>,
    /// Open mesh editors keyed by content-relative mesh path.
    pub mesh_editors: std::collections::HashMap<String, rust_engine::engine::editor::mesh_editor::MeshEditorData>,
    /// Open input action editors (one per .inputaction.ron file).
    pub input_action_editor: InputActionEditor,
    /// Open mapping context editors (one per .mappingcontext.ron file).
    pub input_context_editor: InputContextEditor,
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
    pub input_settings_panel: InputSettingsPanel,
}

/// Play-mode snapshots and build dialog.
pub struct PlayModeState {
    pub snapshot: Option<PlayModeSnapshot>,
    pub pre_play_camera: Option<PrePlayCameraState>,
    pub build_dialog: BuildDialog,
    /// When true, cursor is temporarily released during play mode (F1 toggle).
    pub cursor_released: bool,
}

/// Editor-specific state, decomposed into semantic sub-structures.
pub struct EditorApp {
    pub viewport: ViewportState,
    pub console: ConsoleState,
    pub scene: SceneEditorState,
    pub ui: EditorUIState,
    pub play: PlayModeState,
    pub mesh_preview_renderer: Option<rust_engine::engine::editor::mesh_editor::MeshPreviewRenderer>,
}

/// Main application combining CoreApp and EditorApp.
pub struct App {
    pub core: CoreApp,
    pub editor: EditorApp,
    runtime_flags: EditorRuntimeFlags,
    pub pending_window_requests: Vec<PendingWindowRequest>,
}

impl App {
    pub fn new(
        window: Arc<Window>,
        runtime_flags: EditorRuntimeFlags,
        plugin: &dyn rust_engine::engine::plugin::GamePlugin,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        println!("Rust Game Engine - Starting up...");

        let window_config = rust_engine::engine::utils::WindowConfig::load_or_default();
        let present_preference = window_config.vsync.as_present_preference();
        println!(
            "VSync = {:?} (present mode = {:?})",
            window_config.vsync, present_preference
        );
        let renderer = Renderer::new_with_present_mode(window.clone(), present_preference)?;

        let swapchain_format = renderer.swapchain_state.images[0].format();
        let gui = Gui::new(
            renderer.gpu.device.clone(),
            renderer.gpu.queue.clone(),
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

        // Audio engine — no-audio fallback if initialization fails
        if let Some(audio_engine) = AudioEngine::new() {
            game_world.resources_mut().insert(audio_engine);
        }
        game_world
            .resources_mut()
            .insert(AudioReloadQueue::new());
        game_world
            .resources_mut()
            .insert(asset_manager.clone());

        let mut physics_world = PhysicsWorld::new();
        game_setup::register_physics_entities(&mut physics_world, game_world.hecs_mut());
        game_world.resources_mut().insert(physics_world);
        game_world.resources_mut().insert(TransformCache::new());
        game_world.resources_mut().insert(InputManager::new());
        // Enhanced input system: try loading enhanced config, fall back to legacy migration, then defaults
        let action_set = enhanced_serialization::load_action_set(
            &enhanced_serialization::default_action_set_path(),
        )
        .or_else(|| {
            serialization::load_action_map(&serialization::default_bindings_path())
                .map(|legacy| enhanced_serialization::migrate_legacy_action_map(&legacy))
        })
        .unwrap_or_else(default_action_set);
        let mut subsystem = InputSubsystem::new(action_set);
        subsystem.add_context("global");
        game_world.resources_mut().insert(subsystem);
        game_world.resources_mut().insert(ActionState::new());
        game_world
            .resources_mut()
            .insert(rust_engine::engine::ecs::events::Events::<InputEvent>::new());
        if let Some(gamepad_state) = GamepadState::try_new() {
            game_world.resources_mut().insert(gamepad_state);
        }

        let descriptor_set = game_setup::upload_model_texture(&renderer, &asset_manager)?;

        let deferred_renderer = DeferredRenderer::new(
            renderer.gpu.device.clone(),
            renderer.gpu.queue.clone(),
            renderer.gpu.memory_allocator.clone(),
            renderer.gpu.command_buffer_allocator.clone(),
            renderer.gpu.descriptor_set_allocator.clone(),
            800,
            600,
        )?;

        let skinning = SkinningBackend::new(
            renderer.gpu.memory_allocator.clone(),
            renderer.gpu.descriptor_set_allocator.clone(),
            &deferred_renderer.geometry_pipeline(),
        )?;

        let viewport_texture = ViewportTexture::new(
            renderer.gpu.device.clone(),
            renderer.gpu.memory_allocator.clone(),
            800,
            600,
        )?;

        let mut profiler_panel = ProfilerPanel::new();
        profiler_panel.register_sink();

        use rust_engine::engine::animation::{AnimationPlayer, SkeletonInstance};
        use rust_engine::engine::audio::components::{AudioEmitter, AudioListener};
        use rust_engine::engine::ecs::components::TransformDirty;
        use rust_engine::engine::ecs::hierarchy::{Children, Parent};
        use rust_engine::engine::physics::{
            Collider as PhysCollider, PhysicsStepSystem,
            RigidBody as PhysRigidBody, Velocity as PhysVelocity,
        };

        let mut schedule = Schedule::new();
        schedule.add_system_described(
            EnhancedInputSystem,
            EnhancedInputSystem::stage(),
            EnhancedInputSystem::descriptor(),
        );
        schedule.add_system_described(
            AnimationUpdateSystem,
            Stage::PreUpdate,
            SystemDescriptor::new("AnimationUpdateSystem")
                .reads_resource::<Time>()
                .writes::<AnimationPlayer>()
                .writes::<SkeletonInstance>(),
        );
        schedule.add_system_described_with_criteria(
            PhysicsStepSystem,
            Stage::PreUpdate,
            SystemDescriptor::new("PhysicsStepSystem")
                .reads_resource::<Time>()
                .writes_resource::<PhysicsWorld>()
                .writes::<Transform>()
                .writes::<TransformDirty>()
                .reads::<PhysRigidBody>()
                .reads::<PhysCollider>()
                .reads::<PhysVelocity>()
                .after("AnimationUpdateSystem"),
            RunIfPlaying,
        );
        plugin.build(&mut schedule, game_world.resources_mut());
        schedule.add_system_described(
            TransformPropagationSystem,
            Stage::PostUpdate,
            SystemDescriptor::new("TransformPropagationSystem")
                .writes_resource::<TransformCache>()
                .writes_resource::<HierarchyChanged>()
                .reads::<Transform>()
                .reads::<Parent>()
                .reads::<Children>()
                .writes::<TransformDirty>(),
        );
        schedule.add_system_described(
            AudioSystem::new(),
            Stage::PostUpdate,
            SystemDescriptor::new("AudioSystem")
                .reads_resource::<Time>()
                .reads_resource::<EditorState>()
                .writes_resource::<AudioEngine>()
                .writes_resource::<AudioReloadQueue>()
                .reads::<Transform>()
                .reads::<Camera>()
                .reads::<AudioEmitter>()
                .reads::<AudioListener>()
                .after("TransformPropagationSystem"),
        );

        let validation_errors = schedule.validate();
        if !validation_errors.is_empty() {
            for err in &validation_errors {
                log::error!("Schedule validation error: {err}");
            }
            panic!(
                "Schedule validation failed with {} error(s) — see log above",
                validation_errors.len()
            );
        }
        schedule.print_access_report();

        let render_thread = RenderThread::spawn(RenderThreadConfig {
            gpu_context: renderer.gpu.clone(),
            render_mode: rust_engine::engine::rendering::frame_packet::RenderMode::Editor,
            initial_dimensions: [800, 600],
            swapchain_transfer: Some(rust_engine::engine::rendering::render_thread::SwapchainTransfer {
                surface: renderer.swapchain_state.surface.clone(),
                swapchain: renderer.swapchain_state.swapchain.clone(),
                images: renderer.swapchain_state.images.clone(),
            }),
            viewport_dimensions: Some([800, 600]),
        });

        match render_thread.wait_for_ready(std::time::Duration::from_secs(10)) {
            Ok(rust_engine::engine::rendering::frame_packet::RenderEvent::RenderThreadReady { .. }) => {
                log::info!("editor: render thread ready");
            }
            Ok(rust_engine::engine::rendering::frame_packet::RenderEvent::RenderError { message }) => {
                return Err(format!("render thread init failed: {}", message).into());
            }
            Ok(_) => {
                log::warn!("editor: unexpected event while waiting for render thread ready");
            }
            Err(e) => {
                return Err(format!("render thread did not become ready: {}", e).into());
            }
        }

        let core = CoreApp {
            renderer,
            window: window.clone(),
            asset_manager,
            hot_reload,
            reload_rx,
            game_world,
            schedule,
            deferred_renderer,
            skinning,
            game_loop: GameLoop::new(),
            current_debug_view: DebugView::None,
            camera_distance: 5.0,
            mesh_indices,
            plane_mesh_index,
            cube_mesh_index,
            descriptor_set,
            mesh_data_buffer: Vec::with_capacity(64),
            shadow_caster_buffer: Vec::with_capacity(64),
            plankton_emitter_buffer: Vec::with_capacity(32),
            frame_number: 0,
            render_thread: Some(render_thread),
            #[cfg(debug_assertions)]
            debug_draw_buffer: rust_engine::engine::debug_draw::DebugDrawBuffer::new(),
        };

        let gpu_ctx = GpuThumbnailContext {
            device: core.renderer.gpu.device.clone(),
            queue: core.renderer.gpu.queue.clone(),
            memory_allocator: core.renderer.gpu.memory_allocator.clone(),
            command_buffer_allocator: core.renderer.gpu.command_buffer_allocator.clone(),
            descriptor_set_allocator: core.renderer.gpu.descriptor_set_allocator.clone(),
        };
        let mut asset_browser =
            AssetBrowserPanel::new(std::path::PathBuf::from("content"), Some(gpu_ctx));
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
                import_dialog: None,
                mesh_editors: std::collections::HashMap::new(),
                input_action_editor: InputActionEditor::new(),
                input_context_editor: InputContextEditor::new(),
            },
            ui: EditorUIState {
                gui,
                dock_state: EditorDockState::load_or_default(),
                show_stat_fps: false,
                show_profiler: false,
                icon_manager: IconManager::new(20, egui::Color32::WHITE),
                icons_loaded: false,
                profiler_panel,
                input_settings_panel: InputSettingsPanel::new(),
            },
            play: PlayModeState {
                snapshot: None,
                pre_play_camera: None,
                build_dialog: BuildDialog::new(),
                cursor_released: false,
            },
            mesh_preview_renderer: match rust_engine::engine::editor::mesh_editor::MeshPreviewRenderer::new(
                core.renderer.gpu.device.clone(),
                core.renderer.gpu.queue.clone(),
                core.renderer.gpu.memory_allocator.clone(),
                core.renderer.gpu.command_buffer_allocator.clone(),
                core.renderer.gpu.descriptor_set_allocator.clone(),
            ) {
                Ok(r) => Some(r),
                Err(e) => {
                    log::error!("Failed to create MeshPreviewRenderer: {}", e);
                    None
                }
            },
        };

        Ok(Self {
            core,
            editor,
            runtime_flags,
            pending_window_requests: Vec::new(),
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

        // Preserve non-window fields (like vsync) by loading existing config first.
        let mut window_config = WindowConfig::load_or_default();
        window_config.width = size.width;
        window_config.height = size.height;
        window_config.x = position.x;
        window_config.y = position.y;
        window_config.maximized = is_maximized;
        window_config.fullscreen = is_fullscreen;

        if let Err(e) = window_config.save_to_default() {
            eprintln!("Warning: Failed to save window config on exit: {}", e);
        }
    }

    /// Drain all pending window requests (called by GameApp in about_to_wait).
    pub fn drain_pending_window_requests(&mut self) -> Vec<PendingWindowRequest> {
        std::mem::take(&mut self.pending_window_requests)
    }

    /// Undock a tab from the dock area to a secondary OS window.
    pub fn undock_tab(&mut self, tab: EditorTab) {
        if let Some((kind, editor_key)) = tab.to_window_kind() {
            // Remove from dock
            self.editor.ui.dock_state.remove_tab(&tab);

            // Ensure editor state exists for per-file tabs
            match &tab {
                EditorTab::MeshEditor(key) => {
                    if let Some(data) = self.editor.scene.mesh_editors.get_mut(key) {
                        data.open = true;
                    }
                }
                EditorTab::InputActionEditor(key) => {
                    if let Some(data) = self.editor.scene.input_action_editor.open_actions.get_mut(key) {
                        data.open = true;
                    }
                }
                EditorTab::InputContextEditor(key) => {
                    if let Some(data) = self.editor.scene.input_context_editor.open_contexts.get_mut(key) {
                        data.open = true;
                    }
                }
                _ => {}
            }

            let title = kind.window_title(&editor_key);
            let (width, height) = kind.default_size();
            self.pending_window_requests.push(PendingWindowRequest {
                editor_key,
                kind,
                title,
                width,
                height,
            });
        }
    }

    /// Dock a secondary OS window back into the dock area as a tab.
    pub fn dock_tab(&mut self, editor_key: &str, kind: SecondaryWindowKind) {
        let tab = kind.to_editor_tab(editor_key);
        self.editor.ui.dock_state.open_tab(tab);

        // Mark the editor state for closure (secondary window will be removed by cleanup)
        match kind {
            SecondaryWindowKind::Mesh => {
                if let Some(data) = self.editor.scene.mesh_editors.get_mut(editor_key) {
                    data.open = false;
                }
            }
            SecondaryWindowKind::InputAction => {
                if let Some(data) = self.editor.scene.input_action_editor.open_actions.get_mut(editor_key) {
                    data.open = false;
                }
            }
            SecondaryWindowKind::InputContext => {
                if let Some(data) = self.editor.scene.input_context_editor.open_contexts.get_mut(editor_key) {
                    data.open = false;
                }
            }
            // Built-in panels: just add tab, window will be cleaned up
            _ => {}
        }
    }

    /// Open an input action file as a dock tab (default behavior).
    pub fn open_input_action_as_tab(&mut self, file_path: std::path::PathBuf) {
        let key = self.editor.scene.input_action_editor.open(file_path);
        let tab = EditorTab::InputActionEditor(key);
        self.editor.ui.dock_state.open_tab(tab);
    }

    /// Open a mapping context file as a dock tab (default behavior).
    pub fn open_input_context_as_tab(&mut self, file_path: std::path::PathBuf) {
        self.editor.scene.input_context_editor.refresh_action_names(std::path::Path::new("content"));
        let key = self.editor.scene.input_context_editor.open(file_path);
        let tab = EditorTab::InputContextEditor(key);
        self.editor.ui.dock_state.open_tab(tab);
    }

    /// Open a mesh file as a dock tab (default behavior).
    pub fn open_mesh_as_tab(&mut self, mesh_key: String) {
        let tab = EditorTab::MeshEditor(mesh_key);
        self.editor.ui.dock_state.open_tab(tab);
    }

    pub fn begin_frame(&mut self) {
        puffin::GlobalProfiler::lock().new_frame();
        #[cfg(feature = "tracy")]
        tracy_client::Client::running().map(|c| c.frame_mark());
        if let Some(im) = self.core.game_world.resource_mut::<InputManager>() {
            im.new_frame();
        }
        if let Some(gp) = self.core.game_world.resource_mut::<GamepadState>() {
            gp.update();
        }
        self.core.game_world.begin_frame();
    }

    pub fn update(&mut self) {
        rust_engine::profile_function!();

        self.process_hot_reload();
        self.resolve_mesh_paths();

        let delta_time = self.core.game_loop.tick();

        if let Some(time) = self.core.game_world.resource_mut::<Time>() {
            time.advance(delta_time);
        }

        self.core.game_world.run_schedule(&mut self.core.schedule);

        // Update debug draw persistent line lifetimes
        #[cfg(debug_assertions)]
        self.core.debug_draw_buffer.update(delta_time);
    }

    /// Resolve `mesh_path` to `mesh_index` for all MeshRenderer components.
    ///
    /// Loads meshes via the AssetManager if they aren't already uploaded.
    fn resolve_mesh_paths(&mut self) {
        use rust_engine::engine::ecs::components::MeshRenderer;

        // Collect paths that need resolving
        let mut paths_to_load: Vec<String> = Vec::new();
        for (_entity, mr) in self
            .core
            .game_world
            .hecs_mut()
            .query_mut::<&MeshRenderer>()
        {
            if !mr.mesh_path.is_empty() {
                let meshes = self.core.asset_manager.meshes.read();
                if meshes.first_index_for_path(&mr.mesh_path).is_none() {
                    paths_to_load.push(mr.mesh_path.clone());
                }
            }
        }

        // Load unique paths
        paths_to_load.sort();
        paths_to_load.dedup();
        for path in &paths_to_load {
            if let Err(e) = self.core.asset_manager.load_model_gpu(path) {
                log::warn!("Failed to load mesh '{}': {}", path, e);
            }
        }

        // Resolve indices
        let meshes = self.core.asset_manager.meshes.read();
        for (_entity, mr) in self
            .core
            .game_world
            .hecs_mut()
            .query_mut::<&mut MeshRenderer>()
        {
            if !mr.mesh_path.is_empty() {
                if let Some(idx) = meshes.first_index_for_path(&mr.mesh_path) {
                    mr.mesh_index = idx;
                }
            }
        }
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
                ReloadEvent::AudioChanged { path } => {
                    // Push into AudioReloadQueue — AudioSystem drains it each frame
                    if let Some(queue) = self.core.game_world.resource_mut::<AudioReloadQueue>() {
                        queue.0.push(path.clone());
                    }
                    println!("Audio auto-reload queued: {}", path);
                }
                ReloadEvent::ReloadFailed { path, error } => {
                    eprintln!("Auto-reload failed for {}: {}", path, error);
                }
                ReloadEvent::MaterialInstanceChanged { path } => {
                    println!("Material instance changed: {}", path);
                    self.editor
                        .console
                        .messages
                        .push(LogMessage::info(format!(
                            "Material instance file changed: {}",
                            path,
                        )));
                    // Full material instance hot-reload requires MaterialManager
                    // integration at the scene level — log for now.
                }
                ReloadEvent::ShaderChanged { path } => {
                    use rust_engine::engine::rendering::shader_compiler::ShaderCompiler;

                    println!("Shader changed: {}", path);
                    let compiler = match ShaderCompiler::new() {
                        Ok(c) => c,
                        Err(e) => {
                            self.editor
                                .console
                                .messages
                                .push(LogMessage::error(format!(
                                    "Shader compiler init failed: {e}"
                                )));
                            continue;
                        }
                    };

                    let device = &self.core.renderer.gpu.device;
                    let shader_path = std::path::Path::new(&path);
                    let results = self
                        .core
                        .deferred_renderer
                        .pipeline_registry()
                        .rebuild_for_shader(shader_path, &compiler, device);

                    for result in &results {
                        match &result.outcome {
                            Ok(()) => {
                                self.editor.console.messages.push(LogMessage::info(
                                    format!("Hot-reloaded pipeline {:?}", result.id),
                                ));
                            }
                            Err(e) => {
                                self.editor.console.messages.push(LogMessage::error(
                                    format!(
                                        "Pipeline {:?} hot-reload failed: {}",
                                        result.id, e
                                    ),
                                ));
                            }
                        }
                    }
                }
            }
        }
    }

    pub fn handle_window_event(&mut self, event: &WindowEvent, _event_loop: &ActiveEventLoop) {
        self.editor.ui.gui.handle_event(event);

        match event {
            WindowEvent::Resized(new_size) => {
                self.core.renderer.swapchain_state.recreate_swapchain = true;
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

                    // F1: toggle cursor capture during play mode
                    if keycode == Some(KeyCode::F1) && self.play_mode() == PlayMode::Playing {
                        if self.editor.play.cursor_released {
                            // Re-capture cursor
                            if self
                                .core
                                .window
                                .set_cursor_grab(CursorGrabMode::Confined)
                                .is_err()
                            {
                                let _ = self.core.window.set_cursor_grab(CursorGrabMode::None);
                            }
                            self.core.window.set_cursor_visible(false);
                            if let Some(im) = self.core.game_world.resource_mut::<InputManager>() {
                                im.set_use_raw_mouse(true);
                            }
                            self.editor.play.cursor_released = false;
                            log::info!("Cursor captured (F1)");
                        } else {
                            // Release cursor
                            let _ = self.core.window.set_cursor_grab(CursorGrabMode::None);
                            self.core.window.set_cursor_visible(true);
                            if let Some(im) = self.core.game_world.resource_mut::<InputManager>() {
                                im.set_use_raw_mouse(false);
                            }
                            self.editor.play.cursor_released = true;
                            log::info!("Cursor released (F1)");
                        }
                    }

                    if keycode == Some(KeyCode::KeyS)
                        && self.core.game_world.resource::<InputManager>().is_some_and(|im| im.is_winit_key_pressed(KeyCode::ControlLeft))
                    {
                        self.save_active_scene();
                    }
                }
                if let Some(im) = self.core.game_world.resource_mut::<InputManager>() {
                    im.handle_keyboard(keycode, key_event.state);
                }
            }
            WindowEvent::MouseInput { button, state, .. } => {
                if let Some(im) = self.core.game_world.resource_mut::<InputManager>() {
                    im.handle_mouse_button(*button, *state);
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                if let Some(im) = self.core.game_world.resource_mut::<InputManager>() {
                    im.handle_mouse_move(position.x as f32, position.y as f32);
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let scroll = match delta {
                    MouseScrollDelta::LineDelta(_x, y) => *y,
                    MouseScrollDelta::PixelDelta(pos) => pos.y as f32 * 0.01,
                };
                if let Some(im) = self.core.game_world.resource_mut::<InputManager>() {
                    im.handle_mouse_wheel(scroll);
                }
            }
            WindowEvent::Focused(false) => {
                self.editor.viewport.camera.reset_active_drag();
                if self.editor.viewport.cursor_locked {
                    let _ = self.core.window.set_cursor_grab(CursorGrabMode::None);
                    self.core.window.set_cursor_visible(true);
                    self.editor.viewport.cursor_locked = false;
                    if let Some(im) = self.core.game_world.resource_mut::<InputManager>() {
                        im.set_use_raw_mouse(false);
                    }
                    self.editor.viewport.drag_start_cursor_pos = None;
                }
                // Release play-mode cursor on unfocus
                if self.play_mode() == PlayMode::Playing && !self.editor.play.cursor_released {
                    let _ = self.core.window.set_cursor_grab(CursorGrabMode::None);
                    self.core.window.set_cursor_visible(true);
                    if let Some(im) = self.core.game_world.resource_mut::<InputManager>() {
                        im.set_use_raw_mouse(false);
                    }
                    self.editor.play.cursor_released = true;
                }
            }
            _ => {}
        }
    }

    /// Build mesh-preview command buffers for all active mesh editors.
    ///
    /// Must be called **before** the secondary-window render loop so each
    /// CB can be chained with its window's acquire → egui → present chain.
    /// This keeps the preview render and the egui sample in the **same**
    /// Vulkan submission, eliminating cross-submission layout/memory issues.
    pub fn build_mesh_preview_cbs(
        &mut self,
    ) -> Vec<(String, std::sync::Arc<vulkano::command_buffer::PrimaryAutoCommandBuffer>)> {
        // Pre-load meshes that haven't been imported yet.
        {
            let paths_to_load: Vec<String> = {
                let meshes = self.core.asset_manager.meshes.read();
                self.editor
                    .scene
                    .mesh_editors
                    .values()
                    .filter(|data| data.preview.is_none())
                    .filter(|data| meshes.indices_for_path(&data.mesh_path).is_none())
                    .map(|data| data.mesh_path.clone())
                    .collect()
            };
            for path in paths_to_load {
                match self.core.asset_manager.load_model_gpu(&path) {
                    Ok((indices, _)) => {
                        log::info!(
                            "Pre-loaded mesh '{}' for preview ({} submeshes)",
                            path,
                            indices.len()
                        );
                    }
                    Err(e) => {
                        log::warn!("Failed to load mesh '{}' for preview: {}", path, e);
                    }
                }
            }
        }

        let mut result = Vec::new();
        {
            let meshes = self.core.asset_manager.meshes.read();
            for (key, data) in self.editor.scene.mesh_editors.iter_mut() {
                // Lazy-init preview state
                if data.preview.is_none() {
                    if let Some(ref renderer) = self.editor.mesh_preview_renderer {
                        match rust_engine::engine::editor::mesh_editor::MeshPreviewState::new(
                            renderer,
                            &meshes,
                            &data.mesh_path,
                        ) {
                            Ok(state) => data.preview = Some(state),
                            Err(e) => {
                                log::error!("Failed to create mesh preview: {}", e);
                            }
                        }
                    }
                }

                if let Some(ref mut preview) = data.preview {
                    let (pw, ph) = preview.size;
                    // Resize if needed
                    if pw > 0
                        && ph > 0
                        && (pw != preview.texture.width() || ph != preview.texture.height())
                    {
                        if let Some(ref renderer) = self.editor.mesh_preview_renderer {
                            if let Ok(true) = preview.resize(renderer, pw, ph) {
                                data.preview_dirty = true;
                            }
                        }
                    }

                    // Always render the preview when we have mesh data and a
                    // valid size.  The CB must be in the submission chain every
                    // frame (matching the main viewport pattern) so vulkano's
                    // AutoCommandBufferBuilder in the egui CB correctly tracks
                    // the image layout transition and inserts a proper barrier
                    // with memory-visibility flags.  Rendering only when dirty
                    // leaves frames without a preview CB, and the egui builder
                    // then inserts an Undefined→ShaderReadOnlyOptimal barrier
                    // that can discard content (white square).
                    if !preview.mesh_indices.is_empty() && pw > 0 && ph > 0 {
                        if let Some(ref renderer) = self.editor.mesh_preview_renderer {
                            let gpu_meshes: Vec<_> = preview
                                .mesh_indices
                                .iter()
                                .filter_map(|&idx| meshes.get(idx))
                                .map(|gm| {
                                    (
                                        gm.vertex_buffer.clone(),
                                        gm.index_buffer.clone(),
                                        gm.index_count,
                                    )
                                })
                                .collect();
                            if !gpu_meshes.is_empty() {
                                let aspect = pw as f32 / ph.max(1) as f32;
                                let vp = preview.compute_view_projection(aspect);
                                match renderer.render(
                                    &preview.framebuffer,
                                    pw,
                                    ph,
                                    &gpu_meshes,
                                    vp,
                                ) {
                                    Ok(cb) => {
                                        result.push((key.clone(), cb));
                                        data.preview_dirty = false;
                                    }
                                    Err(e) => {
                                        log::error!("Mesh preview render error: {}", e);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        result
    }

    pub fn render(&mut self, _window: &Window) -> Result<(), Box<dyn std::error::Error>> {
        rust_engine::profile_function!();

        // Poll render thread events
        if let Some(ref rt) = self.core.render_thread {
            for event in rt.poll_events() {
                match event {
                    rust_engine::engine::rendering::frame_packet::RenderEvent::SwapchainRecreated { dimensions } => {
                        log::info!("editor: swapchain recreated to {}x{}", dimensions[0], dimensions[1]);
                        self.editor.viewport.pending_sync = true;
                    }
                    rust_engine::engine::rendering::frame_packet::RenderEvent::ViewportTextureChanged { texture_id, image_view } => {
                        log::info!("editor: viewport texture changed");
                        self.editor.ui.gui.update_native_texture(texture_id, image_view);
                    }
                    rust_engine::engine::rendering::frame_packet::RenderEvent::RenderError { message } => {
                        log::error!("editor: render thread error: {}", message);
                    }
                    _ => {}
                }
            }
        }

        if self.editor.viewport.texture_id.is_none() {
            let texture_id = self
                .editor
                .ui
                .gui
                .register_native_texture(self.editor.viewport.texture.image_view());
            self.editor.viewport.texture_id = Some(texture_id);
        }

        // Register/update mesh preview textures for docked mesh editors
        for (key, data) in self.editor.scene.mesh_editors.iter_mut() {
            if self.editor.ui.dock_state.is_tab_open(&EditorTab::MeshEditor(key.clone())) {
                if let Some(ref preview) = data.preview {
                    if !preview.mesh_indices.is_empty() && !data.preview_dirty {
                        let iv = preview.texture.image_view();
                        if preview.texture_id.is_none() {
                            let tid = self.editor.ui.gui.register_native_texture(iv);
                            data.preview.as_mut().unwrap().texture_id = Some(tid);
                        }
                    }
                }
            }
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

        let transform_cache = self
            .core
            .game_world
            .resource::<TransformCache>()
            .expect("TransformCache resource missing");
        render_loop::prepare_mesh_data(
            self.core.game_world.hecs(),
            &self.core.asset_manager,
            &self.core.renderer,
            &mut self.core.mesh_data_buffer,
            &mut self.core.shadow_caster_buffer,
            transform_cache,
            &self.core.skinning,
        );
        let light_data =
            render_loop::prepare_light_data(self.core.game_world.hecs(), &self.core.renderer);

        {
            let tc = self
                .core
                .game_world
                .resource::<TransformCache>()
                .expect("TransformCache resource missing");
            let dt = self.core.game_loop.delta();
            render_loop::prepare_plankton_data(
                self.core.game_world.hecs(),
                &mut self.core.plankton_emitter_buffer,
                tc,
                dt,
            );
        }

        if self.editor.viewport.pending_sync {
            let (vp_width, vp_height) = self.editor.viewport.size;
            if vp_width > 0 && vp_height > 0 {
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
            self.editor.viewport.pending_sync = false;
        }

        // Update camera for current viewport size (render thread handles the actual texture resize)
        let (vp_width, vp_height) = self.editor.viewport.size;
        if vp_width > 0 && vp_height > 0 {
            self.editor
                .viewport
                .camera
                .set_viewport_size(vp_width as f32, vp_height as f32);
            self.core
                .renderer
                .camera_3d
                .set_viewport_size(vp_width as f32, vp_height as f32);
        }

        let view_proj = self.editor.viewport.camera.view_projection_matrix();
        let camera_pos = self.editor.viewport.camera.position;

        let is_editing = self.play_mode() == PlayMode::Edit;

        // Submit collider debug wireframes for entities with debug_draw_visible
        #[cfg(debug_assertions)]
        rust_engine::engine::physics::submit_collider_debug_draws(
            self.core.game_world.hecs(),
            &mut self.core.debug_draw_buffer,
        );

        // Submit bone debug wireframes for skeletons with debug_draw_visible
        #[cfg(debug_assertions)]
        {
            let tc = self
                .core
                .game_world
                .resource::<TransformCache>()
                .expect("TransformCache resource missing");
            rust_engine::engine::animation::debug_draw::submit_skeleton_debug_draws(
                self.core.game_world.hecs(),
                &mut self.core.debug_draw_buffer,
                tc,
            );
        }

        // Submit audio emitter debug wireframes (spatial emitters only)
        #[cfg(debug_assertions)]
        rust_engine::engine::audio::debug_draw::submit_audio_debug_draws(
            self.core.game_world.hecs(),
            &mut self.core.debug_draw_buffer,
            !is_editing,
        );

        // Submit plankton particle emitter debug gizmos
        #[cfg(debug_assertions)]
        rust_engine::engine::ecs::plankton_debug_draw::submit_plankton_debug_draws(
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

        let window_size = self.core.window.inner_size();
        let (vp_w, vp_h) = self.editor.viewport.size;
        let packet = FramePacket::build_editor(
            std::mem::take(&mut self.core.mesh_data_buffer),
            std::mem::take(&mut self.core.shadow_caster_buffer),
            light_data,
            view_proj,
            camera_pos,
            self.editor.viewport.grid_visible && is_editing,
            debug_draw_data,
            [window_size.width, window_size.height],
            Some([vp_w, vp_h]),
            self.core.frame_number,
            std::mem::take(&mut self.core.plankton_emitter_buffer),
        );
        self.core.frame_number += 1;

        let physics_ref = self
            .core
            .game_world
            .resource::<PhysicsWorld>()
            .expect("PhysicsWorld resource missing");
        self.editor.ui.profiler_panel.set_runtime_counters(
            Default::default(),
            ResourceCounters::collect(
                self.core.game_world.hecs(),
                &self.core.asset_manager,
                physics_ref,
            ),
        );

        let current_play_mode = self.play_mode();

        // Snapshot InputActionSet for the input settings panel (before mutable world borrow)
        let action_set_snapshot = self
            .core
            .game_world
            .resource::<InputSubsystem>()
            .map(|s| s.action_set.clone());

        let show_profiler = &mut self.editor.ui.show_profiler;
        let hierarchy_panel = &mut self.editor.scene.hierarchy_panel;
        let inspector_panel = &mut self.editor.scene.inspector_panel;
        let profiler_panel = &mut self.editor.ui.profiler_panel;
        let input_settings_panel = &mut self.editor.ui.input_settings_panel;
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
        let mesh_editors = &mut self.editor.scene.mesh_editors;
        let ia_open_actions = &mut self.editor.scene.input_action_editor.open_actions;
        let ic_open_contexts = &mut self.editor.scene.input_context_editor.open_contexts;
        let ic_available_actions = self.editor.scene.input_context_editor.available_actions.as_slice();
        let build_dialog = &mut self.editor.play.build_dialog;
        let import_dialog = &mut self.editor.scene.import_dialog;
        let is_hovering_files = self.editor.ui.gui.is_hovering_external_files();

        let mut menu_action = MenuAction::None;
        let mut toolbar_action = MenuAction::None;
        let mut import_action = ImportDialogAction::None;
        let mut undock_request: Option<EditorTab> = None;

        let gui_result =
            self
                .editor
                .ui
                .gui
                .layout(Some(prev_viewport_rect), |ctx| {
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
                        input_settings_panel,
                        action_set_snapshot: action_set_snapshot.as_ref(),
                        play_mode: current_play_mode,
                        toolbar_action: &mut toolbar_action,
                        mesh_editors,
                        ia_open_actions,
                        ic_open_contexts,
                        ic_available_actions,
                        undock_request: &mut undock_request,
                        dock_area_rect: ctx.available_rect(),
                    };

                    let mut tab_viewer = EditorTabViewer { editor: editor_ctx };

                    DockArea::new(&mut dock_state.dock_state)
                        .style(create_editor_dock_style(ctx))
                        .show(ctx, &mut tab_viewer);

                    // Show file drop overlay when hovering external files
                    if is_hovering_files {
                        #[allow(deprecated)]
                        let screen = ctx.screen_rect();
                        let painter = ctx.layer_painter(egui::LayerId::new(
                            egui::Order::Foreground,
                            egui::Id::new("file_drop_overlay"),
                        ));
                        painter.rect_filled(
                            screen,
                            0.0,
                            egui::Color32::from_rgba_unmultiplied(30, 80, 180, 100),
                        );
                        painter.rect_stroke(
                            screen.shrink(4.0),
                            8.0,
                            egui::Stroke::new(3.0, egui::Color32::from_rgb(100, 160, 255)),
                            egui::StrokeKind::Outside,
                        );
                        painter.text(
                            screen.center(),
                            egui::Align2::CENTER_CENTER,
                            "Drop files to import into current folder",
                            egui::FontId::proportional(24.0),
                            egui::Color32::WHITE,
                        );
                    }

                    // Render import dialog if active
                    if let Some(ref mut dialog_state) = import_dialog {
                        import_action = rust_engine::engine::editor::import_dialog::render_import_dialog(ctx, dialog_state);
                    }
                });

        self.editor.viewport.rect = new_viewport_rect;

        // Handle import dialog result
        match import_action {
            ImportDialogAction::Import => {
                if let Some(dialog) = self.editor.scene.import_dialog.take() {
                    self.execute_model_import(dialog);
                }
            }
            ImportDialogAction::Cancel => {
                self.editor.scene.import_dialog = None;
            }
            ImportDialogAction::None => {
                // Try to populate preview if we haven't yet
                if let Some(ref mut dialog) = self.editor.scene.import_dialog {
                    if !dialog.preview_attempted {
                        dialog.preview_attempted = true;
                        if let Some(source) = dialog.current_file().cloned() {
                            // Attempt a quick parse to get stats
                            match rust_engine::assets::load_model(
                                &source.to_string_lossy(),
                            ) {
                                Ok(model) => {
                                    let total_verts: u32 = model
                                        .meshes
                                        .iter()
                                        .map(|m| m.vertices.len() as u32)
                                        .sum();
                                    let total_idx: u32 = model
                                        .meshes
                                        .iter()
                                        .map(|m| m.indices.len() as u32)
                                        .sum();
                                    dialog.preview = Some(ImportPreview {
                                        mesh_count: model.meshes.len(),
                                        total_vertices: total_verts,
                                        total_indices: total_idx,
                                        material_count: model.materials.len(),
                                        bone_count: model.bones.len(),
                                        animation_count: model.animations.len(),
                                    });
                                }
                                Err(e) => {
                                    eprintln!("Preview parse failed: {}", e);
                                }
                            }
                        }
                    }
                }
            }
        }

        // Apply edited input bindings back to the InputSubsystem
        if let Some(new_set) = self.editor.ui.input_settings_panel.take_pending_apply() {
            if let Some(subsystem) = self.core.game_world.resource_mut::<InputSubsystem>() {
                subsystem.set_action_set(new_set);
            }
        }

        // Process undock request from tab context menu
        if let Some(tab) = undock_request {
            self.undock_tab(tab);
        }

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
            MenuAction::RebuildShaders => self.rebuild_all_shaders(),
        }

        // Process OS file drops (files dragged from Windows Explorer / file manager)
        let dropped_files = self.editor.ui.gui.take_dropped_files();
        if !dropped_files.is_empty() {
            self.import_dropped_files(dropped_files);
        }

        // Process asset browser events
        let asset_events: Vec<_> = self.editor.scene.asset_browser.events.drain().collect();
        for event in asset_events {
            match event {
                AssetBrowserEvent::AssetOpened { id } => {
                    // Extract metadata fields before mutating self
                    let meta_info = self.editor.scene.asset_browser.registry.get(id).map(|m| {
                        (m.asset_type, m.path.clone(), m.display_name.clone())
                    });
                    if let Some((asset_type, meta_path, display_name)) = meta_info {
                        if asset_type == AssetType::Scene {
                            if self.play_mode() != PlayMode::Edit {
                                self.editor.console.messages.push(LogMessage::warning(
                                    "Stop play mode before loading a scene".to_string(),
                                ));
                                continue;
                            }

                            if meta_path.as_path()
                                == std::path::Path::new(BENCHMARK_SCENE_RELATIVE)
                                && !self.runtime_flags.benchmark_tools_enabled
                            {
                                self.editor.console.messages.push(LogMessage::warning(
                                    "Benchmark scene access is locked behind --editor-benchmark-tools"
                                        .to_string(),
                                ));
                                continue;
                            }

                            let relative = meta_path.to_string_lossy();

                            self.core.game_world.reset_transients(false);
                            self.editor.scene.selection.clear();
                            self.core.game_world.resources_mut().insert(PhysicsWorld::new());

                            match load_scene(self.core.game_world.hecs_mut(), &relative) {
                                Ok((scene_name, root_entities)) => {
                                    self.editor
                                        .scene
                                        .hierarchy_panel
                                        .set_root_order(root_entities);
                                    self.editor.scene.current_scene_relative = relative.to_string();
                                    self.editor.scene.current_scene_name = scene_name.clone();
                                    {
                                        self.core.game_world.resources_mut().remove::<TransformCache>();
                                        let mut tc = TransformCache::new();
                                        tc.propagate(self.core.game_world.hecs_mut());
                                        self.core.game_world.resources_mut().insert(tc);
                                    }
                                    // Resolve mesh_path → mesh_index for loaded entities
                                    self.resolve_mesh_paths();

                                    self.editor.console.messages.push(LogMessage::info(format!(
                                        "Loaded scene: {}",
                                        scene_name
                                    )));
                                    println!("Scene loaded: {}", display_name);
                                }
                                Err(e) => {
                                    self.editor.console.messages.push(LogMessage::error(format!(
                                        "Failed to load scene: {}",
                                        e
                                    )));
                                    eprintln!("Failed to load scene: {}", e);
                                }
                            }
                        } else if asset_type == AssetType::Audio {
                            // Play audio preview on dedicated preview track
                            let relative = meta_path.to_string_lossy().to_string();
                            let load_result = self.core.asset_manager.audio.load(&relative);
                            match load_result {
                                Ok(handle) => {
                                    let data = handle.get().clone();
                                    if let Some(engine) = self.core.game_world.resource_mut::<rust_engine::engine::audio::AudioEngine>() {
                                        if let Err(e) = engine.play_preview(data) {
                                            log::warn!("Audio preview failed: {e}");
                                        }
                                    }
                                }
                                Err(e) => {
                                    log::warn!("Failed to load audio for preview: {e}");
                                }
                            }
                        } else if asset_type == AssetType::Mesh {
                            // Open mesh editor tab
                            let relative = meta_path.to_string_lossy().to_string();
                            if !self.editor.scene.mesh_editors.contains_key(&relative) {
                                // Load sidecar metadata
                                let full_path = std::path::Path::new("content").join(&relative);
                                let sidecar_path = full_path.with_extension("mesh.ron");
                                // Handle double extension: "Foo.mesh" → read "Foo.mesh.ron"
                                let meta = if sidecar_path.exists() {
                                    match std::fs::read_to_string(&sidecar_path) {
                                        Ok(text) => ron::from_str(&text).unwrap_or_else(|e| {
                                            log::warn!("Failed to parse {}: {}", sidecar_path.display(), e);
                                            rust_engine::engine::assets::mesh_import::MeshImportMeta {
                                                source: String::new(),
                                                settings: Default::default(),
                                                source_hash: 0,
                                                material_slots: vec![],
                                            }
                                        }),
                                        Err(e) => {
                                            log::warn!("Failed to read {}: {}", sidecar_path.display(), e);
                                            rust_engine::engine::assets::mesh_import::MeshImportMeta {
                                                source: String::new(),
                                                settings: Default::default(),
                                                source_hash: 0,
                                                material_slots: vec![],
                                            }
                                        }
                                    }
                                } else {
                                    rust_engine::engine::assets::mesh_import::MeshImportMeta {
                                        source: String::new(),
                                        settings: Default::default(),
                                        source_hash: 0,
                                        material_slots: vec![],
                                    }
                                };
                                self.editor.scene.mesh_editors.insert(
                                    relative.clone(),
                                    rust_engine::engine::editor::mesh_editor::MeshEditorData {
                                        mesh_path: relative.clone(),
                                        meta,
                                        dirty: false,
                                        preview: None,
                                        open: true,
                                        preview_dirty: true,
                                    },
                                );
                                self.open_mesh_as_tab(relative);
                            }
                        } else if asset_type == AssetType::InputAction {
                            let full_path = std::path::Path::new("content").join(&meta_path);
                            self.open_input_action_as_tab(full_path);
                        } else if asset_type == AssetType::InputMappingContext {
                            let full_path = std::path::Path::new("content").join(&meta_path);
                            self.open_input_context_as_tab(full_path);
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
                AssetBrowserEvent::CreateAsset { asset_type, parent_path } => {
                    let full_parent = self
                        .editor
                        .scene
                        .asset_browser
                        .registry
                        .root_path()
                        .join(&parent_path);

                    match asset_type {
                        AssetType::InputAction => {
                            let base_name = "NewInputAction";
                            let mut new_name = format!("{}.inputaction.ron", base_name);
                            let mut counter = 1;
                            while full_parent.join(&new_name).exists() {
                                new_name = format!("{}_{}.inputaction.ron", base_name, counter);
                                counter += 1;
                            }
                            let action_name = new_name.trim_end_matches(".inputaction.ron").to_string();
                            let action = rust_engine::engine::input::enhanced_action::InputActionDefinition::new(
                                &action_name,
                                rust_engine::engine::input::value::InputValueType::Digital,
                            );
                            let file_path = full_parent.join(&new_name);
                            match enhanced_serialization::save_input_action(&action, &file_path) {
                                Ok(()) => {
                                    self.editor.console.messages.push(LogMessage::info(format!(
                                        "Created input action: {}", action_name
                                    )));
                                    self.editor.scene.asset_browser.request_rescan();
                                }
                                Err(e) => {
                                    self.editor.console.messages.push(LogMessage::error(format!(
                                        "Failed to create input action: {}", e
                                    )));
                                }
                            }
                        }
                        AssetType::InputMappingContext => {
                            let base_name = "NewMappingContext";
                            let mut new_name = format!("{}.mappingcontext.ron", base_name);
                            let mut counter = 1;
                            while full_parent.join(&new_name).exists() {
                                new_name = format!("{}_{}.mappingcontext.ron", base_name, counter);
                                counter += 1;
                            }
                            let ctx_name = new_name.trim_end_matches(".mappingcontext.ron").to_string();
                            let mapping_ctx = rust_engine::engine::input::enhanced_action::MappingContext::new(
                                &ctx_name, 0,
                            );
                            let file_path = full_parent.join(&new_name);
                            match enhanced_serialization::save_mapping_context(&mapping_ctx, &file_path) {
                                Ok(()) => {
                                    self.editor.console.messages.push(LogMessage::info(format!(
                                        "Created mapping context: {}", ctx_name
                                    )));
                                    self.editor.scene.asset_browser.request_rescan();
                                }
                                Err(e) => {
                                    self.editor.console.messages.push(LogMessage::error(format!(
                                        "Failed to create mapping context: {}", e
                                    )));
                                }
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }

        self.handle_frame_input(&gui_result);

        // Attach egui layout data to frame packet and send to render thread
        let mut packet = packet;
        packet.egui_primitives = Some(gui_result.clipped_primitives);
        packet.egui_texture_deltas = Some(gui_result.textures_delta);
        packet.texture_binds = gui_result.texture_binds;
        packet.viewport_texture_id = self.editor.viewport.texture_id;

        if let Some(ref rt) = self.core.render_thread {
            if let Err(e) = rt.send(packet) {
                log::error!("editor: failed to send frame packet: {}", e);
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

    /// Import files dropped from the OS file manager into the current asset browser folder.
    fn import_dropped_files(&mut self, files: Vec<std::path::PathBuf>) {
        // Split files into model files (go through import dialog) and other files (direct copy)
        let model_extensions = ["gltf", "glb", "obj", "fbx"];
        let mut model_files = Vec::new();
        let mut other_files = Vec::new();

        for file in files {
            let ext = file
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if model_extensions.contains(&ext.as_str()) && file.is_file() {
                model_files.push(file);
            } else {
                other_files.push(file);
            }
        }

        // Model files → open import dialog with settings
        if !model_files.is_empty() {
            let target_folder = self.editor.scene.asset_browser.current_folder.clone();
            self.editor.scene.import_dialog =
                Some(ImportDialogState::new(model_files, target_folder));
        }

        // Non-model files → import directly (existing behavior)
        if other_files.is_empty() {
            return;
        }

        let assets_root = self
            .editor
            .scene
            .asset_browser
            .registry
            .root_path()
            .to_path_buf();

        let relative_folder = self.editor.scene.asset_browser.current_folder.clone();
        let target_dir = assets_root.join(&relative_folder);

        if let Err(e) = std::fs::create_dir_all(&target_dir) {
            self.editor.console.messages.push(LogMessage::error(format!(
                "Cannot create target directory: {}",
                e
            )));
            return;
        }

        let supported_extensions: &[&str] = &[
            // Textures
            "png", "jpg", "jpeg", "tga", "bmp", "dds",
            // Native mesh (already processed)
            "mesh",
            // Audio
            "wav", "ogg", "mp3", "flac",
            // Shaders
            "glsl", "vert", "frag", "comp", "spv",
            // Scene/Material/Prefab definitions
            "ron",
        ];

        let files = other_files;

        let mut imported_count = 0;
        let mut skipped_count = 0;

        for source_path in &files {
            // Validate that the file exists and is a file (not directory)
            if !source_path.is_file() {
                self.editor.console.messages.push(LogMessage::warning(format!(
                    "Skipped '{}': not a file",
                    source_path.display()
                )));
                skipped_count += 1;
                continue;
            }

            // Check file extension against supported types
            let ext = source_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();

            if !supported_extensions.contains(&ext.as_str()) {
                self.editor.console.messages.push(LogMessage::warning(format!(
                    "Skipped '{}': unsupported file type (.{})",
                    source_path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default(),
                    ext
                )));
                skipped_count += 1;
                continue;
            }

            let file_name = match source_path.file_name() {
                Some(name) => name.to_owned(),
                None => {
                    skipped_count += 1;
                    continue;
                }
            };

            let mut dest_path = target_dir.join(&file_name);

            // Handle name conflicts by appending a number
            if dest_path.exists() {
                let stem = source_path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("file");
                let extension = source_path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("");
                let mut counter = 1;
                loop {
                    let new_name = format!("{} ({}){}", stem, counter,
                        if extension.is_empty() { String::new() } else { format!(".{}", extension) });
                    dest_path = target_dir.join(&new_name);
                    if !dest_path.exists() {
                        break;
                    }
                    counter += 1;
                    if counter > 100 {
                        self.editor.console.messages.push(LogMessage::error(format!(
                            "Cannot import '{}': too many duplicates",
                            file_name.to_string_lossy()
                        )));
                        break;
                    }
                }
                if counter > 100 {
                    skipped_count += 1;
                    continue;
                }
            }

            // Copy the file
            match std::fs::copy(source_path, &dest_path) {
                Ok(bytes) => {
                    let display_name = dest_path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    let size_kb = bytes as f64 / 1024.0;
                    self.editor.console.messages.push(LogMessage::info(format!(
                        "Imported '{}' ({:.1} KB)",
                        display_name, size_kb
                    )));
                    imported_count += 1;

                    // Also copy companion files for certain formats:
                    // OBJ → .mtl (material library)
                    if ext == "obj" {
                        if let Some(mtl_path) = source_path.parent().map(|p| {
                            let stem = source_path.file_stem().unwrap_or_default();
                            p.join(format!("{}.mtl", stem.to_string_lossy()))
                        }) {
                            if mtl_path.is_file() {
                                let mtl_dest = target_dir
                                    .join(mtl_path.file_name().unwrap_or_default());
                                if let Err(e) = std::fs::copy(&mtl_path, &mtl_dest) {
                                    self.editor.console.messages.push(LogMessage::warning(format!(
                                        "Could not copy companion .mtl file: {}",
                                        e
                                    )));
                                } else {
                                    self.editor.console.messages.push(LogMessage::info(format!(
                                        "Imported companion '{}'",
                                        mtl_path.file_name().unwrap_or_default().to_string_lossy()
                                    )));
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    self.editor.console.messages.push(LogMessage::error(format!(
                        "Failed to import '{}': {}",
                        file_name.to_string_lossy(),
                        e
                    )));
                    skipped_count += 1;
                }
            }
        }

        // Trigger rescan so the asset browser picks up new files
        if imported_count > 0 {
            self.editor.scene.asset_browser.request_rescan();
            println!(
                "Imported {} file(s) into '{}'",
                imported_count,
                if relative_folder.as_os_str().is_empty() {
                    "assets/".to_string()
                } else {
                    relative_folder.display().to_string()
                }
            );
        }

        if skipped_count > 0 {
            println!("Skipped {} file(s) during import", skipped_count);
        }
    }

    /// Execute model import: convert source files to .mesh using the dialog's settings.
    fn execute_model_import(&mut self, dialog: ImportDialogState) {
        let assets_root = self
            .editor
            .scene
            .asset_browser
            .registry
            .root_path()
            .to_path_buf();
        let target_dir = assets_root.join(&dialog.target_folder);

        if let Err(e) = std::fs::create_dir_all(&target_dir) {
            self.editor.console.messages.push(LogMessage::error(format!(
                "Cannot create target directory: {}",
                e
            )));
            return;
        }

        let mut imported_count = 0;

        for source_path in &dialog.source_files {
            let stem = source_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("model");

            // Determine output .mesh path with duplicate handling
            let mut mesh_path = target_dir.join(format!("{}.mesh", stem));
            if mesh_path.exists() {
                let mut counter = 1;
                loop {
                    mesh_path = target_dir.join(format!("{} ({}).mesh", stem, counter));
                    if !mesh_path.exists() || counter > 100 {
                        break;
                    }
                    counter += 1;
                }
            }

            // Also copy the source file alongside the .mesh for re-import
            let source_dest = target_dir.join(
                source_path
                    .file_name()
                    .unwrap_or_default(),
            );
            if !source_dest.exists() {
                if let Err(e) = std::fs::copy(source_path, &source_dest) {
                    self.editor.console.messages.push(LogMessage::warning(format!(
                        "Could not copy source file: {}",
                        e
                    )));
                }
            }

            // Run the import pipeline
            match rust_engine::assets::mesh_import::import_model_to_mesh(
                source_path,
                &mesh_path,
                &dialog.settings,
            ) {
                Ok(result) => {
                    let mesh_size = std::fs::metadata(&mesh_path)
                        .map(|m| m.len() as f64 / 1024.0)
                        .unwrap_or(0.0);
                    let display_name = mesh_path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    let source_name = source_path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy();

                    let mut msg = format!(
                        "Imported '{}' -> '{}' ({:.1} KB)",
                        source_name, display_name, mesh_size
                    );

                    if result.bone_count > 0 {
                        msg.push_str(&format!(", {} bones", result.bone_count));
                    }

                    if result.material_count > 0 {
                        msg.push_str(&format!(
                            ", {} material(s)",
                            result.material_count
                        ));
                    }

                    if result.anim_written {
                        let anim_path = mesh_path.with_extension("anim");
                        let anim_size = std::fs::metadata(&anim_path)
                            .map(|m| m.len() as f64 / 1024.0)
                            .unwrap_or(0.0);
                        msg.push_str(&format!(
                            " + {} animation(s) ({:.1} KB)",
                            result.anim_clip_count, anim_size
                        ));
                    }

                    self.editor.console.messages.push(LogMessage::info(msg));
                    imported_count += 1;
                }
                Err(e) => {
                    self.editor.console.messages.push(LogMessage::error(format!(
                        "Failed to import '{}': {}",
                        source_path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy(),
                        e
                    )));
                }
            }
        }

        if imported_count > 0 {
            self.editor.scene.asset_browser.request_rescan();
            println!(
                "Imported {} model(s) as .mesh into '{}'",
                imported_count,
                if dialog.target_folder.as_os_str().is_empty() {
                    "content/".to_string()
                } else {
                    dialog.target_folder.display().to_string()
                }
            );
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
        let mut pw = self
            .core
            .game_world
            .resources_mut()
            .remove::<PhysicsWorld>()
            .unwrap_or_default();
        let roots = match load_or_create_benchmark_scene(
            self.core.game_world.hecs_mut(),
            &mut pw,
            &BenchmarkConfig::default(),
            self.core.cube_mesh_index,
        ) {
            Ok(roots) => roots,
            Err(error) => {
                self.core.game_world.resources_mut().insert(pw);
                self.editor.console.messages.push(LogMessage::error(format!(
                    "Failed to load benchmark scene: {}",
                    error
                )));
                return;
            }
        };
        self.core.game_world.resources_mut().insert(pw);
        self.editor.scene.hierarchy_panel.set_root_order(roots);
        self.editor.scene.current_scene_relative = BENCHMARK_SCENE_RELATIVE.to_string();
        self.editor.scene.current_scene_name = "Benchmark Scene".to_string();
        {
            self.core.game_world.resources_mut().remove::<TransformCache>();
            let mut tc = TransformCache::new();
            tc.propagate(self.core.game_world.hecs_mut());
            self.core.game_world.resources_mut().insert(tc);
        }

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

    fn rebuild_all_shaders(&mut self) {
        use rust_engine::engine::rendering::shader_compiler::ShaderCompiler;

        let compiler = match ShaderCompiler::new() {
            Ok(c) => c,
            Err(e) => {
                self.editor
                    .console
                    .messages
                    .push(LogMessage::error(format!("Shader compiler init failed: {e}")));
                return;
            }
        };

        let device = &self.core.renderer.gpu.device;
        let results = self
            .core
            .deferred_renderer
            .pipeline_registry()
            .rebuild_all(&compiler, device);

        for result in &results {
            match &result.outcome {
                Ok(()) => {
                    self.editor.console.messages.push(LogMessage::info(format!(
                        "Rebuilt pipeline {:?}",
                        result.id
                    )));
                }
                Err(e) => {
                    self.editor.console.messages.push(LogMessage::error(format!(
                        "Pipeline {:?} rebuild failed: {}",
                        result.id, e
                    )));
                }
            }
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

        {
            let mut pw = self
                .core
                .game_world
                .resources_mut()
                .remove::<PhysicsWorld>()
                .unwrap_or_default();
            play_mode::rebuild_physics(&mut pw, self.core.game_world.hecs_mut());
            self.core.game_world.resources_mut().insert(pw);
        }

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

        // Mapping context activation is handled by PlayerInputSystem
        // (reads from the PlayerInput component's mapping_context field)

        // Capture cursor for mouse look during gameplay
        self.editor.play.cursor_released = false;
        if self
            .core
            .window
            .set_cursor_grab(CursorGrabMode::Confined)
            .is_err()
        {
            let _ = self.core.window.set_cursor_grab(CursorGrabMode::None);
        }
        self.core.window.set_cursor_visible(false);
        if let Some(im) = self.core.game_world.resource_mut::<InputManager>() {
            im.set_use_raw_mouse(true);
        }

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

        // Remove all gameplay mapping contexts (pushed by PlayerInputSystem)
        if let Some(subsystem) = self.core.game_world.resource_mut::<InputSubsystem>() {
            let to_remove: Vec<String> = subsystem
                .active_contexts()
                .iter()
                .filter(|c| c.as_str() != "global")
                .cloned()
                .collect();
            for ctx in to_remove {
                subsystem.remove_context(&ctx);
            }
        }

        // Release cursor capture
        let _ = self.core.window.set_cursor_grab(CursorGrabMode::None);
        self.core.window.set_cursor_visible(true);
        if let Some(im) = self.core.game_world.resource_mut::<InputManager>() {
            im.set_use_raw_mouse(false);
        }

        self.core.game_world.reset_transients(false);

        if let Some(snapshot) = self.editor.play.snapshot.as_ref() {
            let mut pw = self
                .core
                .game_world
                .resources_mut()
                .remove::<PhysicsWorld>()
                .unwrap_or_default();
            match play_mode::restore_snapshot(
                snapshot,
                &mut self.core.game_world,
                &mut self.editor.scene.hierarchy_panel,
                &mut self.editor.scene.selection,
                &mut pw,
                &mut self.editor.scene.command_history,
            ) {
                Ok(()) => {
                    self.editor.play.snapshot = None;
                    if let Some(tc) = self.core.game_world.resource_mut::<TransformCache>() {
                        tc.request_full_propagation();
                    }
                }
                Err(e) => {
                    log::error!(
                        "Failed to restore play mode snapshot (snapshot preserved): {}",
                        e
                    );
                }
            }
            self.core.game_world.resources_mut().insert(pw);
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
        let cache = self
            .core
            .game_world
            .resource::<TransformCache>()
            .expect("TransformCache resource missing");

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

    fn handle_frame_input(&mut self, gui_result: &rust_engine::engine::gui::GuiLayoutResult) {
        // Temporarily remove InputManager to avoid borrow conflicts with game_world
        let Some(mut input_manager) = self.core.game_world.resources_mut().remove::<InputManager>() else {
            return;
        };

        if self.play_mode() == PlayMode::Edit
            && input_manager.is_winit_key_pressed(KeyCode::ControlLeft)
        {
            if input_manager.is_winit_key_just_pressed(KeyCode::KeyZ) {
                if let Some(desc) = self
                    .editor
                    .scene
                    .command_history
                    .undo(self.core.game_world.hecs_mut())
                {
                    println!("Undo: {}", desc);
                }
            }
            if input_manager.is_winit_key_just_pressed(KeyCode::KeyY) {
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
                &input_manager,
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
                Some(input_manager.mouse_position());
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
            input_manager.set_use_raw_mouse(true);
        } else if !camera_dragging && self.editor.viewport.cursor_locked {
            let _ = self.core.window.set_cursor_grab(CursorGrabMode::None);
            if let Some((x, y)) = self.editor.viewport.drag_start_cursor_pos.take() {
                let pos = winit::dpi::PhysicalPosition::new(x as f64, y as f64);
                let _ = self.core.window.set_cursor_position(pos);
            }
            self.core.window.set_cursor_visible(true);
            self.editor.viewport.cursor_locked = false;
            input_manager.set_use_raw_mouse(false);
        }

        if !gui_result.wants_keyboard && self.play_mode() == PlayMode::Edit {
            input_handler::handle_debug_views(
                &input_manager,
                &mut self.core.deferred_renderer,
                &mut self.core.current_debug_view,
            );
        }

        if !gui_result.wants_pointer && !self.editor.viewport.hovered {
            input_handler::handle_zoom(
                &mut self.core.renderer,
                &input_manager,
                &mut self.core.camera_distance,
            );
        }

        // Re-insert InputManager into resources
        self.core.game_world.resources_mut().insert(input_manager);
    }
}
