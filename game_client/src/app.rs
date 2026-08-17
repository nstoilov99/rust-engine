//! Main application state and orchestration
//!
//! Split into CoreApp (engine core) and EditorApp (editor UI).
//! The App struct composes both, with EditorApp only present in editor builds.

use super::{game_setup, input_handler, render_loop};
use rust_engine::assets::asset_source;
use rust_engine::assets::AssetType;
use rust_engine::assets::{AssetManager, HotReloadWatcher, ReloadEvent};
use rust_engine::engine::animation::AnimationUpdateSystem;
use rust_engine::engine::audio::{AudioEngine, AudioReloadQueue, AudioSystem};
use rust_engine::engine::benchmark::{
    load_or_create_benchmark_scene, BenchmarkConfig, BENCHMARK_SCENE_RELATIVE,
};
use rust_engine::engine::collision::CollisionWorld;
use rust_engine::engine::ecs::access::SystemDescriptor;
use rust_engine::engine::ecs::components::{Camera, Transform};
use rust_engine::engine::ecs::events::PlayModeChanged;
use rust_engine::engine::ecs::game_world::GameWorld;
use rust_engine::engine::ecs::hierarchy::{
    despawn_recursive, HierarchyChanged, TransformCache, TransformPropagationSystem,
};
use rust_engine::engine::ecs::resources::Time;
use rust_engine::engine::ecs::resources::{EditorState, PlayMode};
use rust_engine::engine::ecs::schedule::{Schedule, Stage};
use rust_engine::engine::editor::commands::Command as _;
use rust_engine::engine::editor::play_mode::{self, PlayModeSnapshot};
use rust_engine::engine::editor::graph_prefs::WireStyle;
use rust_engine::engine::editor::graph_state_store::GraphStateStore;
use rust_engine::engine::editor::theme::Density;
use rust_engine::engine::editor::{
    AssetBrowserEvent, AssetBrowserPanel, BuildDialog, CommandHistory,
    ConsoleCommandSystem, ConsoleLog, DeleteSubtreeCommand, DormantScene, EditorAction,
    EditorCamera, EditorServices, EditorTab, GizmoHandler, GpuThumbnailContext, HierarchyPanel,
    ImportDialogAction, ImportDialogState, ImportPreview, InputActionEditor, InputContextEditor,
    InputSettingsPanel, InspectorPanel, LogFilter, LogMessage, MenuAction, PasteCommand,
    ProfilerPanel, RenameTarget, SaveAsDialog, SceneId, SceneRegistry, SecondaryWindowKind,
    Selection, ViewportSettings, ViewportTexture, WindowConfig,
};
use rust_engine::engine::input::action_state::ActionState;
use rust_engine::engine::input::enhanced_defaults::default_action_set;
use rust_engine::engine::input::enhanced_serialization;
use rust_engine::engine::input::event::InputEvent;
use rust_engine::engine::input::gamepad::GamepadState;
use rust_engine::engine::input::serialization;
use rust_engine::engine::input::subsystem::{EnhancedInputSystem, InputSubsystem};
use rust_engine::engine::physics::PhysicsWorld;
use rust_engine::engine::rendering::frame_packet::FramePacket;
use rust_engine::engine::rendering::render_thread::{RenderThread, RenderThreadConfig};
use rust_engine::engine::rendering::rendering_3d::deferred_renderer::DebugView;
use rust_engine::engine::rendering::rendering_3d::{
    DeferredRenderer, MeshRenderData, SkinningBackend,
};
use rust_engine::engine::rendering::ResourceCounters;
use rust_engine::engine::scene::{load_scene, save_scene, scene_serializer, EntityData};
use rust_engine::engine::world::{StreamingCtx, WorldStreamer};
use rust_engine::{GameLoop, InputManager, Renderer};
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use winit::event::{MouseScrollDelta, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorGrabMode, Window};

const MIN_VIEWPORT_SIZE_FOR_CAMERA: u32 = 50;
const MAIN_SCENE_RELATIVE: &str = "scenes/main.scene";

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
    /// The built plugin set — kept beside `game_world` because it owns the
    /// `on_world_loaded` callbacks that run at every content moment, and the
    /// per-plugin records the Plugin Manager reads.
    pub plugin_set: rust_engine::engine::plugins::PluginSet,
    pub deferred_renderer: DeferredRenderer,
    pub skinning: SkinningBackend,
    pub game_loop: GameLoop,
    pub current_debug_view: DebugView,
    pub camera_distance: f32,
    pub mesh_indices: Vec<usize>,
    pub plane_mesh_index: usize,
    pub cube_mesh_index: usize,
    mesh_data_buffer: Vec<MeshRenderData>,
    shadow_caster_buffer: Vec<MeshRenderData>,
    plankton_emitter_buffer:
        Vec<rust_engine::engine::rendering::frame_packet::PlanktonEmitterFrameData>,
    frame_number: u64,
    pub render_thread: Option<RenderThread>,
    /// Material descriptor-set cache and `.matinst` bookkeeping.
    pub materials: crate::asset_resolve::MaterialStore,
    #[cfg(debug_assertions)]
    pub debug_draw_buffer: rust_engine::engine::debug_draw::DebugDrawBuffer,
    /// Cached collision-wireframe GPU lines, keyed by (collision generation,
    /// chunks toggle, grid toggle). Rebuilt only when the key changes — the
    /// full-world wireframe is millions of vertices and must not be
    /// regenerated per frame.
    #[cfg(debug_assertions)]
    pub collision_debug_cache: Option<CollisionDebugCache>,
    /// M4 world streamer: cell/chunk streaming for scenes with a world
    /// manifest. Lives on CoreApp (not as a world resource) so streaming can
    /// borrow the CollisionWorld resource mutably alongside the hecs world.
    pub world_streamer: WorldStreamer,
}

#[cfg(debug_assertions)]
pub struct CollisionDebugCache {
    key: (u64, bool, bool),
    lines: Option<(
        vulkano::buffer::Subbuffer<[rust_engine::engine::debug_draw::DebugLineVertex]>,
        u32,
    )>,
}

/// Viewport rendering, camera, gizmo, and interaction state.
pub struct ViewportState {
    /// Placeholder viewport texture, kept alongside the field-initialization
    /// path. The render thread renders into its own `ViewportTexture` and
    /// hands the crusty renderer a texture id via `RenderThreadReady`.
    #[allow(dead_code)]
    pub texture: ViewportTexture,
    pub size: (u32, u32),
    pub pending_sync: bool,
    pub camera: EditorCamera,
    pub gizmo_handler: GizmoHandler,
    pub grid_visible: bool,
    pub hovered: bool,
    pub rect: rust_engine::engine::editor::dock_crusty::Rect,
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
///
/// The fields `hierarchy_panel`, `selection`, `command_history`, `current_scene_*`,
/// and `active_dirty` describe the *active* scene. Inactive scene state lives in
/// [`registry.dormant`](SceneRegistry).
pub struct SceneEditorState {
    pub hierarchy_panel: HierarchyPanel,
    pub inspector_panel: InspectorPanel,
    pub selection: Selection,
    pub command_history: CommandHistory,
    pub asset_browser: AssetBrowserPanel,
    pub current_scene_relative: String,
    pub current_scene_name: String,
    /// Whether the active scene has unsaved changes.
    pub active_dirty: bool,
    /// Multi-scene registry (active id + dormant tabs).
    pub registry: SceneRegistry,
    /// Model import dialog state (shown when model files are dropped).
    pub import_dialog: Option<ImportDialogState>,
    /// Anim event marker list for a `.anim` asset (Task 41; opened by
    /// double-clicking the asset).
    pub anim_events_dialog: Option<rust_engine::engine::editor::anim_events_dialog::AnimEventsDialog>,
    /// Open mesh editors keyed by content-relative mesh path.
    pub mesh_editors:
        std::collections::HashMap<String, rust_engine::engine::editor::mesh_editor::MeshEditorData>,
    /// Open node graph editors keyed by content-relative graph path.
    pub graph_editors: std::collections::HashMap<
        String,
        rust_engine::engine::editor::graph_editor::GraphEditorState,
    >,
    /// Open `.curve` editors keyed by content-relative curve path (45-A P8b).
    pub curve_editors: std::collections::HashMap<
        String,
        rust_engine::engine::editor::curve_editor::CurveEditorState,
    >,
    /// Node type registry (Task 40) — feeds graph load/validate; shared by
    /// every open graph editor.
    pub node_registry: std::sync::Arc<rust_engine::engine::node_graph::NodeRegistry>,
    /// Shared graph editor clipboard (copy/paste across open graphs).
    pub graph_clipboard: Option<rust_engine::engine::editor::graph_editor::GraphFragment>,
    /// Open input action editors (one per .inputaction file).
    pub input_action_editor: InputActionEditor,
    /// Open mapping context editors (one per .mappingcontext file).
    pub input_context_editor: InputContextEditor,
    /// Active "Save As" dialog state (shown when saving an untitled scene).
    pub save_as_dialog: Option<SaveAsDialog>,
    /// Entity clipboard: serialized subtrees from Copy/Cut.
    pub clipboard: Vec<EntityData>,
}

/// The two per-frame document resolvers the graph canvas reads: `.graph`
/// documents for subgraph interfaces, `.curve` documents for Timeline track
/// pins (45-A D3/P8b). Returned together because they are rebuilt together and
/// every panel that takes one takes the other.
#[cfg(feature = "editor")]
pub struct GraphResolverDocs {
    pub docs: std::collections::BTreeMap<String, rust_engine::engine::node_graph::GraphDoc>,
    pub curves: std::collections::BTreeMap<
        String,
        rust_engine::engine::node_graph::curve_asset::CurveDoc,
    >,
}

/// General editor UI state: dock, profiler, icons, overlays.
pub struct EditorUIState {
    /// Crusty dock layout — drives the editor panel layout.
    pub crusty_dock: rust_engine::engine::editor::dock_crusty::CrustyDockLayout,
    pub show_stat_fps: bool,
    pub show_profiler: bool,
    pub icons_loaded: bool,
    pub profiler_panel: ProfilerPanel,
    pub input_settings_panel: InputSettingsPanel,
    /// Editor Preferences + Project Settings windows (M10 P7).
    pub settings: rust_engine::engine::editor::settings_crusty::SettingsState,
}

/// Play-mode snapshots and build dialog.
pub struct PlayModeState {
    pub snapshot: Option<PlayModeSnapshot>,
    pub pre_play_camera: Option<PrePlayCameraState>,
    pub build_dialog: BuildDialog,
    /// When true, cursor is temporarily released during play mode (F1 toggle).
    pub cursor_released: bool,
    /// Set once the server's `world_scene` was compared against the open
    /// scene this play session (M9.6: PIE keeps the open scene, warns on
    /// mismatch instead of swapping it).
    pub net_scene_warned: bool,
}

/// Editor-specific state, decomposed into semantic sub-structures.
pub struct EditorApp {
    pub services: EditorServices,
    pub viewport: ViewportState,
    pub console: ConsoleState,
    pub scene: SceneEditorState,
    pub ui: EditorUIState,
    pub play: PlayModeState,
    pub mesh_preview_renderer:
        Option<rust_engine::engine::editor::mesh_editor::MeshPreviewRenderer>,
}

/// Main application combining CoreApp and EditorApp.
pub struct App {
    pub core: CoreApp,
    pub editor: EditorApp,
    runtime_flags: EditorRuntimeFlags,
    /// VSync mode last written to `window_config.ron`.
    vsync_saved: rust_engine::engine::utils::window_config::VSyncMode,
    /// Present mode active since startup — a differing saved mode needs restart.
    vsync_active: rust_engine::engine::utils::window_config::VSyncMode,
    /// Net session (M5); `Some` when launched with `--connect`.
    net: Option<crate::net::NetSession>,
    /// M9.6 P4: pending listen-server launcher (spacetime start + publish
    /// off-thread); yields once, then the editor connects as a client.
    listen_server_rx: Option<std::sync::mpsc::Receiver<Result<(), String>>>,
    /// M9.6 P5: extra client child processes (Number of Players > 1),
    /// killed on play-exit.
    play_clients: Vec<std::process::Child>,
    /// Main-thread half of the crusty-gui integration — the editor's sole UI.
    #[cfg(feature = "editor")]
    pub crusty_gui: rust_engine::engine::gui::crusty::CrustyGui,
    /// Hierarchy icon textures uploaded by the render thread at startup
    /// (SVG stem → crusty TextureId).
    #[cfg(feature = "editor")]
    pub crusty_icons:
        std::collections::HashMap<String, rust_engine::engine::gui::crusty::TextureId>,
    /// Crusty texture id for the viewport render target (registered by the
    /// render thread at startup; survives resizes).
    #[cfg(feature = "editor")]
    pub crusty_viewport_texture: Option<rust_engine::engine::gui::crusty::TextureId>,
    /// Menu action picked in the crusty menu bar. The crusty layout runs
    /// *after* this frame's `menu_action` match, so the action is stored here
    /// and applied at the start of the next frame's match.
    #[cfg(feature = "editor")]
    crusty_menu_action: MenuAction,
    /// A tab torn off the crusty dock, following the cursor as an in-window
    /// ghost until re-docked or dropped (spawning a float window).
    #[cfg(feature = "editor")]
    crusty_dock_drag: Option<String>,
    /// Subject of the open tab context menu (right-clicked dock tab).
    #[cfg(feature = "editor")]
    crusty_tab_ctx: Option<String>,
    /// Tabs dropped outside any dock target — main.rs turns these into OS
    /// windows next `about_to_wait` (window creation needs ActiveEventLoop).
    #[cfg(feature = "editor")]
    pending_crusty_floats: Vec<rust_engine::engine::editor::crusty_window::CrustyWindowRequest>,
    /// A dock layout can name per-file editor tabs whose documents nothing has
    /// opened — on startup it always does, since the layout is restored from
    /// disk while the state maps start empty. Set here and whenever tabs are
    /// adopted; cleared once a scan finds nothing missing.
    pending_hydration: bool,
    /// Tab ids whose document could not be loaded, so the scan does not retry
    /// them every frame. Cleared per-key when the user opens the asset again.
    hydration_failed: std::collections::HashSet<String>,
    /// Live torn-off panel windows, keyed by winit window id.
    #[cfg(feature = "editor")]
    crusty_floats: std::collections::HashMap<
        winit::window::WindowId,
        rust_engine::engine::editor::crusty_window::CrustyFloatWindow,
    >,
    /// Mesh-preview render targets registered with the render thread's
    /// crusty renderer, keyed by mesh editor key. `id` is None while the
    /// registration round-trips via `RenderEvent::CrustyNativeRegistered`.
    #[cfg(feature = "editor")]
    crusty_mesh_textures: std::collections::HashMap<String, CrustyMeshTexture>,
    /// Preview CBs for mesh tabs docked in the main crusty dock — sent to
    /// the render thread in the frame packet (executed before the GUI pass).
    #[cfg(feature = "editor")]
    pub crusty_docked_preview_cbs: Vec<(
        String,
        std::sync::Arc<vulkano::command_buffer::PrimaryAutoCommandBuffer>,
    )>,
    /// Preview CBs for mesh tabs hosted in crusty float windows — chained
    /// into each float's own submission in `crusty_float_frames`.
    #[cfg(feature = "editor")]
    pub crusty_float_preview_cbs: Vec<(
        String,
        std::sync::Arc<vulkano::command_buffer::PrimaryAutoCommandBuffer>,
    )>,
}

/// A mesh-preview texture registered (or pending) with the render thread's
/// crusty renderer. The `view` pointer detects render-target recreation on
/// resize so the id can be re-pointed via `crusty_native_updates`.
#[cfg(feature = "editor")]
struct CrustyMeshTexture {
    id: Option<rust_engine::engine::gui::crusty::TextureId>,
    view: std::sync::Arc<vulkano::image::view::ImageView>,
}

impl App {
    pub fn new(
        window: Arc<Window>,
        runtime_flags: EditorRuntimeFlags,
        mut plugin_set: rust_engine::engine::plugins::PluginSet,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        println!("Rust Game Engine - Starting up...");

        let window_config = rust_engine::engine::utils::WindowConfig::load_or_default();
        let project_config =
            rust_engine::engine::editor::project_config::ProjectConfig::load_or_default();
        let startup_scene = project_config.startup_scene().to_string();
        let present_preference = window_config.vsync.as_present_preference();
        println!(
            "VSync = {:?} (present mode = {:?})",
            window_config.vsync, present_preference
        );
        let renderer = Renderer::new_with_present_mode(window.clone(), present_preference)?;

        #[cfg(feature = "editor")]
        let crusty_gui = {
            let size = window.inner_size();
            rust_engine::engine::gui::crusty::CrustyGui::new(
                renderer.gpu.device.clone(),
                [size.width as f32, size.height as f32],
            )
        };

        let (asset_manager, hot_reload, reload_rx) = game_setup::setup_asset_system(&renderer)?;

        let (mesh_indices, plane_mesh_index, cube_mesh_index) =
            game_setup::load_assets(&asset_manager)?;

        // Registries-then-content (39.8 D4): the world starts empty, every
        // registry is built and every plugin has registered before any scene
        // content exists. Scene load happens further down, after
        // `schedule.validate()`.
        let mut game_world = GameWorld::new();

        // Audio engine — no-audio fallback if initialization fails
        if let Some(audio_engine) = AudioEngine::new() {
            game_world.resources_mut().insert(audio_engine);
        }
        game_world.resources_mut().insert(AudioReloadQueue::new());
        game_world.resources_mut().insert(asset_manager.clone());

        let mut physics_world = PhysicsWorld::new();
        physics_world.set_gravity(nalgebra_glm::vec3(0.0, 0.0, project_config.gravity_z));
        physics_world.set_timestep(1.0 / project_config.fixed_timestep_hz.max(1.0));
        // Bodies are registered by the world-population helper after the
        // scene loads, not here.
        game_world.resources_mut().insert(physics_world);
        // Collision fills in below via `init_streaming_for_scene` (streamed
        // or monolithic depending on the scene's world manifest).
        game_world.resources_mut().insert(CollisionWorld::new());
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

        // Node type registry (Task 40) — starts empty and is filled entirely by
        // plugins in `build_all` below (39.8 D5: `dev_nodes` is a plugin now).
        // Moves into `SceneEditorState` further down.
        let mut node_registry = rust_engine::engine::node_graph::NodeRegistry::new();

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
        // Task 41: `.animgraph` state machines. Ordered after the single-clip
        // player because both write skeletons (the graph wins on entities
        // that carry both — the system skip lives engine-side).
        {
            use rust_engine::engine::animation::graph::{
                AnimClipCache, AnimGraphPlanCache, AnimGraphRunner, AnimGraphRuntime,
                AnimGraphSystem, DiskAnimAssets,
            };
            game_world.resources_mut().insert(AnimGraphPlanCache::new());
            game_world.resources_mut().insert(AnimClipCache::new());
            schedule.add_system_described(
                AnimGraphSystem::new(Box::new(DiskAnimAssets {
                    content_root: rust_engine::engine::assets::content_root::content_root(),
                })),
                Stage::PreUpdate,
                SystemDescriptor::new(rust_engine::engine::ecs::system_names::ANIM_GRAPH)
                    .reads_resource::<Time>()
                    .writes_resource::<AnimGraphPlanCache>()
                    .writes_resource::<AnimClipCache>()
                    .reads::<AnimGraphRunner>()
                    .writes::<AnimGraphRuntime>()
                    .writes::<SkeletonInstance>()
                    .after(rust_engine::engine::ecs::system_names::ANIMATION_UPDATE),
            );
        }
        // `PhysicsStepSystem` is registered by `RapierPhysicsPlugin` inside
        // `build_all` below (39.8 D7) — it lands in the same stage, in the
        // same relative position, with the same descriptor and criteria.
        //
        // Registration is staged per plugin and committed only on success, so
        // a failing plugin registers nothing. The editor surfaces the failure
        // and boots anyway (the Plugin Manager shows it); only a shipped game
        // treats it as fatal.
        plugin_set.build_all(
            rust_engine::engine::plugins::PluginTargets {
                schedule: &mut schedule,
                resources: game_world.resources_mut(),
                node_registry: &mut node_registry,
            },
            Some(&project_config.plugins),
        );

        // Shared with the graph runner after the plugins have filled it; see
        // the note in `standalone.rs`. Restart-only plugin activation is what
        // makes an immutable shared registry correct.
        let node_registry = std::sync::Arc::new(node_registry);
        game_world
            .resources_mut()
            .insert(std::sync::Arc::clone(&node_registry));
        // The console does not exist yet, so plugin diagnostics are collected
        // here and seeded into it below. P6 turns this data into the Plugin
        // Manager's Failed / Enabled-with-warnings rows; it has to flow now.
        let mut plugin_diagnostics: Vec<LogMessage> = {
            let mut out = Vec::new();
            for failure in plugin_set.failures() {
                let line = format!(
                    "plugin '{}' failed during {}: {}",
                    failure.id,
                    failure.phase.label(),
                    failure.error
                );
                eprintln!("{line}");
                out.push(LogMessage::error(line));
            }
            for record in plugin_set.records() {
                for warning in &record.warnings {
                    let line = format!("plugin '{}': {warning}", record.manifest.id);
                    eprintln!("{line}");
                    out.push(LogMessage::warning(line));
                }
            }
            for id in plugin_set.orphans() {
                out.push(LogMessage::warning(format!(
                    "project.ron lists plugin '{id}', which is not in this build"
                )));
            }
            out
        };

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

        // === Content ===
        // Everything above is registration; the world is empty until here, so
        // plugin-registered systems, resources and node types are all in place
        // before any scene entity exists (39.8 D4).
        let (scene_loaded, root_entities) =
            game_setup::load_or_create_scene(game_world.hecs_mut(), mesh_indices[0], &startup_scene)?;

        if !scene_loaded {
            game_setup::spawn_physics_test_objects(
                game_world.hecs_mut(),
                plane_mesh_index,
                cube_mesh_index,
            );
        }

        // The first content moment. Its callback failures arrive *after* the
        // diagnostics block above ran, so they are appended here — otherwise
        // the console would never mention them and the only trace would be
        // stderr (no logger is installed in `game_client`).
        {
            let failures =
                crate::world_population::after_world_populated(&mut game_world, &mut plugin_set);
            crate::world_population::report_failures(&failures);
            for failure in &failures {
                plugin_diagnostics.push(LogMessage::error(crate::world_population::describe(
                    failure,
                )));
            }
        }

        let mut hierarchy_panel = HierarchyPanel::new();
        if !root_entities.is_empty() {
            hierarchy_panel.set_root_order(root_entities);
        }

        let render_thread = RenderThread::spawn(RenderThreadConfig {
            gpu_context: renderer.gpu.clone(),
            render_mode: rust_engine::engine::rendering::frame_packet::RenderMode::Editor,
            initial_dimensions: [800, 600],
            swapchain_transfer: Some(
                rust_engine::engine::rendering::render_thread::SwapchainTransfer {
                    surface: renderer.swapchain_state.surface.clone(),
                    swapchain: renderer.swapchain_state.swapchain.clone(),
                    images: renderer.swapchain_state.images.clone(),
                },
            ),
            viewport_dimensions: Some([800, 600]),
            #[cfg(feature = "editor")]
            crusty_text: Some(crusty_gui.text_handle()),
        });

        #[cfg(feature = "editor")]
        let mut crusty_icons = std::collections::HashMap::new();
        #[cfg(feature = "editor")]
        let mut crusty_viewport_texture = None;
        match render_thread.wait_for_ready(std::time::Duration::from_secs(10)) {
            Ok(rust_engine::engine::rendering::frame_packet::RenderEvent::RenderThreadReady {
                #[cfg(feature = "editor")]
                    crusty_icons: icons,
                #[cfg(feature = "editor")]
                    crusty_viewport_texture: vp_tex,
                ..
            }) => {
                log::info!("editor: render thread ready");
                #[cfg(feature = "editor")]
                {
                    crusty_icons = icons;
                    crusty_viewport_texture = vp_tex;
                }
            }
            Ok(rust_engine::engine::rendering::frame_packet::RenderEvent::RenderError {
                message,
            }) => {
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
            plugin_set,
            deferred_renderer,
            skinning,
            game_loop: GameLoop::new(),
            current_debug_view: DebugView::None,
            camera_distance: 5.0,
            mesh_indices,
            plane_mesh_index,
            cube_mesh_index,
            mesh_data_buffer: Vec::with_capacity(64),
            shadow_caster_buffer: Vec::with_capacity(64),
            plankton_emitter_buffer: Vec::with_capacity(32),
            frame_number: 0,
            render_thread: Some(render_thread),
            materials: crate::asset_resolve::MaterialStore::default(),
            #[cfg(debug_assertions)]
            debug_draw_buffer: rust_engine::engine::debug_draw::DebugDrawBuffer::new(),
            #[cfg(debug_assertions)]
            collision_debug_cache: None,
            world_streamer: {
                let mut s = WorldStreamer::default();
                s.config.r_load = project_config.stream_r_load;
                s.config.r_unload = project_config.stream_r_unload;
                s.config.budget_ms = project_config.stream_budget_ms;
                s.config.max_in_flight = project_config.stream_max_in_flight;
                // Editor default: keep the whole world resident. The Debug
                // menu toggles ring streaming for testing.
                s.full_world = true;
                s
            },
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

        let crusty_dock =
            rust_engine::engine::editor::dock_crusty::CrustyDockLayout::load_or_default();
        let (mut editor_prefs, prefs_existed) =
            rust_engine::engine::editor::editor_prefs::EditorPrefs::load();
        if !prefs_existed {
            // First run: adopt the play settings previously persisted in the
            // dock layout (their pre-M10 home).
            editor_prefs.play = crusty_dock.play_settings.clone();
        }
        let settings_state = rust_engine::engine::editor::settings_crusty::SettingsState::new(
            editor_prefs,
            project_config.clone(),
            window_config.vsync,
        );
        let scene_display_name = if startup_scene == MAIN_SCENE_RELATIVE {
            "Main Scene".to_string()
        } else {
            std::path::Path::new(&startup_scene)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| startup_scene.clone())
        };

        let mut services = EditorServices::new();
        services.load_keymap();
        let editor = EditorApp {
            services,
            viewport: ViewportState {
                texture: viewport_texture,
                size: (800, 600),
                pending_sync: false,
                camera: EditorCamera::new(800.0, 600.0),
                gizmo_handler: GizmoHandler::new(),
                grid_visible: true,
                hovered: false,
                // ZERO stands in for "not yet laid out"; the viewport panel
                // overwrites this on its first frame.
                rect: rust_engine::engine::editor::dock_crusty::Rect::ZERO,
                cursor_locked: false,
                drag_start_cursor_pos: None,
                settings: ViewportSettings::default(),
            },
            console: ConsoleState {
                messages: {
                    let mut log = ConsoleLog::new();
                    log.push(LogMessage::info("Engine initialized successfully"));
                    log.push(LogMessage::info("Scene loaded"));
                    for message in plugin_diagnostics {
                        log.push(message);
                    }
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
                current_scene_relative: startup_scene.clone(),
                current_scene_name: scene_display_name,
                active_dirty: false,
                registry: SceneRegistry::new(SceneId(0)),
                import_dialog: None,
                anim_events_dialog: None,
                mesh_editors: std::collections::HashMap::new(),
                graph_editors: std::collections::HashMap::new(),
                curve_editors: std::collections::HashMap::new(),
                node_registry,
                graph_clipboard: None,
                input_action_editor: InputActionEditor::new(),
                input_context_editor: InputContextEditor::new(),
                save_as_dialog: None,
                clipboard: Vec::new(),
            },
            ui: EditorUIState {
                crusty_dock,
                show_stat_fps: false,
                show_profiler: false,
                icons_loaded: false,
                profiler_panel,
                input_settings_panel: InputSettingsPanel::new(),
                settings: settings_state,
            },
            play: PlayModeState {
                snapshot: None,
                pre_play_camera: None,
                build_dialog: {
                    let mut bd = BuildDialog::new();
                    bd.settings.target = project_config.build_target;
                    bd.settings.output_dir = project_config.build_output_dir.clone();
                    bd.settings.server_uri = project_config.net_host.clone();
                    bd.settings.module = project_config.net_module.clone();
                    bd
                },
                cursor_released: false,
                net_scene_warned: false,
            },
            mesh_preview_renderer:
                match rust_engine::engine::editor::mesh_editor::MeshPreviewRenderer::new(
                    core.renderer.gpu.device.clone(),
                    core.renderer.gpu.queue.clone(),
                    core.renderer.gpu.memory_allocator.clone(),
                    core.renderer.gpu.command_buffer_allocator.clone(),
                    core.renderer.gpu.descriptor_set_allocator.clone(),
                ) {
                    Ok(r) => Some(r),
                    Err(e) => {
                        eprintln!("Failed to create MeshPreviewRenderer: {}", e);
                        None
                    }
                },
        };

        let mut app = Self {
            core,
            editor,
            runtime_flags,
            vsync_saved: window_config.vsync,
            vsync_active: window_config.vsync,
            // M9.6 P3: in the editor a net session exists only between
            // play-enter and play-exit; `--connect` pre-fills the play
            // settings below instead of connecting at startup.
            net: None,
            listen_server_rx: None,
            play_clients: Vec::new(),
            #[cfg(feature = "editor")]
            crusty_gui,
            #[cfg(feature = "editor")]
            crusty_icons,
            #[cfg(feature = "editor")]
            crusty_viewport_texture,
            #[cfg(feature = "editor")]
            crusty_menu_action: MenuAction::None,
            #[cfg(feature = "editor")]
            crusty_dock_drag: None,
            crusty_tab_ctx: None,
            #[cfg(feature = "editor")]
            pending_crusty_floats: Vec::new(),
            // The layout was just restored from disk; its per-file tabs have
            // no documents behind them yet.
            pending_hydration: true,
            hydration_failed: std::collections::HashSet::new(),
            #[cfg(feature = "editor")]
            crusty_floats: std::collections::HashMap::new(),
            #[cfg(feature = "editor")]
            crusty_mesh_textures: std::collections::HashMap::new(),
            #[cfg(feature = "editor")]
            crusty_docked_preview_cbs: Vec::new(),
            #[cfg(feature = "editor")]
            crusty_float_preview_cbs: Vec::new(),
        };
        if scene_loaded {
            app.init_streaming_for_scene(&startup_scene);
        }
        if let Some(addr) =
            crate::net::NetSession::parse_connect_args(&std::env::args().collect::<Vec<_>>())
        {
            let ps = &mut app.editor.ui.settings.prefs.play;
            ps.mode = rust_engine::engine::editor::play_settings::NetPlayMode::Client;
            ps.host = addr.host;
            ps.module = addr.module;
        }
        app.apply_editor_prefs(true);
        Ok(app)
    }

    pub fn print_controls(&self) {
        game_setup::print_controls();
    }

    pub fn save_layout_on_exit(&mut self) {
        // Fold torn-off float windows back into the tree so their panels
        // reappear docked on next launch (float positions aren't persisted).
        #[cfg(feature = "editor")]
        for fw in self.crusty_floats.values() {
            let mut tabs = Vec::new();
            fw.tree.collect_tabs(&mut tabs);
            for tab in tabs {
                rust_engine::engine::editor::dock_crusty::redock_tab(
                    &mut self.editor.ui.crusty_dock.tree,
                    tab,
                );
            }
        }
        #[cfg(feature = "editor")]
        if let Err(e) = self.editor.ui.crusty_dock.save_to_default() {
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

    /// Diff `settings.prefs` against the last-applied snapshot and push
    /// changes into live editor state. Per-field guards keep session-only
    /// edits (toolbar snap toggles, console chips, G-key grid) from being
    /// stomped by unrelated pref fields.
    fn apply_editor_prefs(&mut self, force: bool) {
        let new = self.editor.ui.settings.prefs.clone();
        let old = self.editor.ui.settings.prefs_applied.clone();
        if !force && new == old {
            return;
        }

        if force
            || new.theme_preset != old.theme_preset
            || new.popover_translucent != old.popover_translucent
            || new.ui_scale != old.ui_scale
        {
            // `ui_scale` is the one master knob; density presets are named
            // values of it, so prefs are the single source of truth.
            let mut theme = new.theme_preset.theme().with_ui_scale(new.ui_scale);
            theme.density = Density::from_ui_scale(theme.metrics.ui_scale)
                .unwrap_or(theme.density);
            if !new.popover_translucent {
                theme.metrics.popover_alpha = 1.0;
            }
            self.editor.services.theme = std::sync::Arc::new(theme);
            self.crusty_gui.apply_theme(&self.editor.services.theme);
            for fw in self.crusty_floats.values_mut() {
                fw.gui.apply_theme(&self.editor.services.theme);
            }
        }

        let vp = &mut self.editor.viewport;
        if force || new.camera_speed != old.camera_speed {
            vp.settings.camera_speed = new.camera_speed;
        }
        if force || new.camera_speed_scalar != old.camera_speed_scalar {
            vp.settings.camera_speed_scalar = new.camera_speed_scalar;
        }
        if force || new.mouse_sensitivity != old.mouse_sensitivity {
            vp.settings.mouse_sensitivity = new.mouse_sensitivity;
            vp.camera.mouse_sensitivity = new.mouse_sensitivity;
        }
        if force || new.invert_y != old.invert_y {
            vp.camera.invert_y = new.invert_y;
        }
        if force || new.fov_deg != old.fov_deg {
            vp.camera.fov = new.fov_deg.to_radians();
        }
        if force || new.grid_visible != old.grid_visible {
            vp.grid_visible = new.grid_visible;
            vp.settings.grid_visible = new.grid_visible;
        }
        if force || new.gizmo_size != old.gizmo_size {
            vp.gizmo_handler.gizmo_size = new.gizmo_size;
        }
        if force || new.grid_snap_enabled != old.grid_snap_enabled {
            vp.settings.grid_snap_enabled = new.grid_snap_enabled;
        }
        if force || new.rotation_snap_enabled != old.rotation_snap_enabled {
            vp.settings.rotation_snap_enabled = new.rotation_snap_enabled;
        }
        if force || new.scale_snap_enabled != old.scale_snap_enabled {
            vp.settings.scale_snap_enabled = new.scale_snap_enabled;
        }
        if force || new.snap_translate != old.snap_translate {
            vp.settings.snap_translate = new.snap_translate;
        }
        if force || new.snap_rotate != old.snap_rotate {
            vp.settings.snap_rotate = new.snap_rotate;
        }
        if force || new.snap_scale != old.snap_scale {
            vp.settings.snap_scale = new.snap_scale;
        }

        if force || new.undo_limit != old.undo_limit {
            self.editor
                .scene
                .command_history
                .set_max_history(new.undo_limit);
        }
        if force || new.thumbnail_size != old.thumbnail_size {
            self.editor.scene.asset_browser.grid_item_size = new.thumbnail_size;
        }
        if force || new.console_max_lines != old.console_max_lines {
            self.editor
                .console
                .messages
                .set_max_messages(new.console_max_lines);
        }
        if force || new.console_show_info != old.console_show_info {
            self.editor.console.log_filter.show_info = new.console_show_info;
        }
        if force || new.console_show_warning != old.console_show_warning {
            self.editor.console.log_filter.show_warning = new.console_show_warning;
        }
        if force || new.console_show_error != old.console_show_error {
            self.editor.console.log_filter.show_error = new.console_show_error;
        }

        let s = &mut self.editor.ui.settings;
        if !force && new != old {
            s.mark_prefs_dirty();
        }
        s.prefs_applied = new;
    }

    /// Write every open graph's view + bookmarks to the user-local sidecar,
    /// on the same debounce as the prefs autosave. Doing it here rather than
    /// on tab close also covers an editor exit that never closes tabs.
    fn flush_graph_ui_state(&mut self) {
        if !self.editor.ui.settings.prefs_saving() {
            return;
        }
        let root = std::path::Path::new("content");
        let mut store = GraphStateStore::load(root);
        for (rel, st) in self.editor.scene.graph_editors.iter() {
            store.store(
                rel,
                st.view,
                &st.bookmarks,
                stored_watches(st),
                stored_breakpoints(st),
            );
        }
        if let Err(e) = store.save() {
            eprintln!("Warning: failed to save graph UI state: {e}");
        }
    }

    /// Persist a VSync change to `window_config.ron`. The swapchain present
    /// mode is fixed at startup, so flag a restart when it now differs.
    fn persist_vsync_change(&mut self) {
        let s = &mut self.editor.ui.settings;
        if s.vsync == self.vsync_saved {
            return;
        }
        let mut wc = WindowConfig::load_or_default();
        wc.vsync = s.vsync;
        if let Err(e) = wc.save_to_default() {
            eprintln!("Failed to save window config: {e}");
        }
        self.vsync_saved = s.vsync;
        s.restart_pending = s.vsync != self.vsync_active;
    }

    /// Open an input action file as a dock tab (default behavior).
    pub fn open_input_action_as_tab(&mut self, file_path: std::path::PathBuf) {
        let key = self.editor.scene.input_action_editor.open(file_path);
        #[cfg(feature = "editor")]
        self.hydration_failed.remove(&format!("ia:{key}"));
        let tab = EditorTab::InputActionEditor(key);
        self.editor.ui.crusty_dock.open_tab(tab);
    }

    /// Open a mapping context file as a dock tab (default behavior).
    pub fn open_input_context_as_tab(&mut self, file_path: std::path::PathBuf) {
        self.editor
            .scene
            .input_context_editor
            .refresh_action_names(std::path::Path::new("content"));
        let key = self.editor.scene.input_context_editor.open(file_path);
        #[cfg(feature = "editor")]
        self.hydration_failed.remove(&format!("mc:{key}"));
        let tab = EditorTab::InputContextEditor(key);
        self.editor.ui.crusty_dock.open_tab(tab);
    }

    /// Load a mesh document into `mesh_editors`. Pure file I/O — the preview
    /// render target is created later from `preview_dirty`, so this needs no
    /// GPU resources and can run on the first frame.
    ///
    /// Shared by the asset-browser double-click and the restored-tab
    /// hydration pass, so the two can never drift.
    #[cfg(feature = "editor")]
    pub fn open_mesh_document(&mut self, relative: &str) -> Result<(), String> {
        use rust_engine::engine::assets::mesh_import::MeshImportMeta;
        if self.editor.scene.mesh_editors.contains_key(relative) {
            return Ok(());
        }
        let full_path = std::path::Path::new("content").join(relative);
        if !full_path.exists() {
            return Err("file missing".to_string());
        }
        // The `.mesh.ron` sidecar is optional: a mesh with no import metadata
        // still opens, it just has nothing to say about where it came from.
        let sidecar_path = full_path.with_extension("mesh.ron");
        let blank = || MeshImportMeta {
            source: String::new(),
            settings: Default::default(),
            source_hash: 0,
            material_slots: vec![],
        };
        let meta = if sidecar_path.exists() {
            match std::fs::read_to_string(&sidecar_path) {
                Ok(text) => ron::from_str(&text).unwrap_or_else(|e| {
                    log::warn!("Failed to parse {}: {}", sidecar_path.display(), e);
                    blank()
                }),
                Err(e) => {
                    log::warn!("Failed to read {}: {}", sidecar_path.display(), e);
                    blank()
                }
            }
        } else {
            blank()
        };
        self.editor.scene.mesh_editors.insert(
            relative.to_string(),
            rust_engine::engine::editor::mesh_editor::MeshEditorData {
                mesh_path: relative.to_string(),
                meta,
                dirty: false,
                preview: None,
                open: true,
                preview_dirty: true,
            },
        );
        Ok(())
    }

    /// Open a mesh file as a dock tab (default behavior).
    pub fn open_mesh_as_tab(&mut self, mesh_key: String) {
        let tab = EditorTab::MeshEditor(mesh_key);
        self.editor.ui.crusty_dock.open_tab(tab);
    }

    /// Open a node graph file as a dock tab (default behavior).
    pub fn open_graph_as_tab(&mut self, graph_key: String) {
        let tab = EditorTab::GraphEditor(graph_key);
        self.editor.ui.crusty_dock.open_tab(tab);
    }

    /// Open a `.curve` file as a dock tab (45-A P8b).
    pub fn open_curve_as_tab(&mut self, curve_key: String) {
        let tab = EditorTab::CurveEditor(curve_key);
        self.editor.ui.crusty_dock.open_tab(tab);
    }

    pub fn begin_frame(&mut self) {
        puffin::GlobalProfiler::lock().new_frame();
        #[cfg(feature = "tracy")]
        tracy_client::Client::running().map(|c| c.frame_mark());
        if let Some(gp) = self.core.game_world.resource_mut::<GamepadState>() {
            gp.update();
        }
        self.core.game_world.begin_frame();
    }

    pub fn end_frame(&mut self) {
        if let Some(im) = self.core.game_world.resource_mut::<InputManager>() {
            im.clear_transient_state();
        }
    }

    fn push_action_unavailable(&mut self, action: &str, reason: &str) {
        self.editor
            .console
            .messages
            .push(LogMessage::warning(format!("{action}: {reason}")));
        self.editor
            .services
            .toasts
            .warning(format!("{action}: {reason}"));
    }

    fn toggle_tab(&mut self, tab: EditorTab) {
        if self.editor.ui.crusty_dock.is_tab_open(&tab) {
            self.editor.ui.crusty_dock.remove_tab(&tab);
        } else {
            self.editor.ui.crusty_dock.open_tab(tab);
        }
    }

    fn set_secondary_editor_open(&mut self, kind: SecondaryWindowKind, key: &str, open: bool) {
        match kind {
            SecondaryWindowKind::Mesh => {
                if let Some(data) = self.editor.scene.mesh_editors.get_mut(key) {
                    data.open = open;
                }
            }
            SecondaryWindowKind::InputAction => {
                if let Some(data) = self
                    .editor
                    .scene
                    .input_action_editor
                    .open_actions
                    .get_mut(key)
                {
                    data.open = open;
                }
            }
            SecondaryWindowKind::InputContext => {
                if let Some(data) = self
                    .editor
                    .scene
                    .input_context_editor
                    .open_contexts
                    .get_mut(key)
                {
                    data.open = open;
                }
            }
            _ => {}
        }
    }

    fn save_secondary_editor(
        &mut self,
        kind: SecondaryWindowKind,
        key: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        match kind {
            SecondaryWindowKind::Mesh => {
                if let Some(data) = self.editor.scene.mesh_editors.get_mut(key) {
                    rust_engine::engine::editor::mesh_editor::MeshEditorPanel::save_sidecar(data)?;
                    data.dirty = false;
                }
            }
            SecondaryWindowKind::Graph => {
                if let Some(state) = self.editor.scene.graph_editors.get_mut(key) {
                    let abs = std::path::Path::new("content").join(&state.path);
                    state.save(&abs)?;
                }
            }
            SecondaryWindowKind::Curve => {
                #[cfg(feature = "editor")]
                self.save_curve_editor(key);
            }
            SecondaryWindowKind::InputAction => {
                if let Some(data) = self
                    .editor
                    .scene
                    .input_action_editor
                    .open_actions
                    .get_mut(key)
                {
                    InputActionEditor::save_state(data)?;
                }
            }
            SecondaryWindowKind::InputContext => {
                if let Some(data) = self
                    .editor
                    .scene
                    .input_context_editor
                    .open_contexts
                    .get_mut(key)
                {
                    InputContextEditor::save_state(data)?;
                }
            }
            _ if self.editor.services.dirty.is_asset_dirty(key) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Unsupported,
                    format!("no save handler is registered for '{key}'"),
                )
                .into());
            }
            _ => {}
        }
        self.editor.services.dirty.clear_asset(key);
        Ok(())
    }

    fn handle_editor_action(&mut self, action: EditorAction) {
        // Route document edit actions to the focused graph tab, if any
        // (Task 40 P5 edit-target focus routing). Docked graphs only — a graph
        // in a float window handles its own keyboard.
        #[cfg(feature = "editor")]
        if matches!(
            action,
            EditorAction::Undo
                | EditorAction::Redo
                | EditorAction::Cut
                | EditorAction::Copy
                | EditorAction::Paste
                | EditorAction::Duplicate
                | EditorAction::Delete
        ) {
            if let Some(key) = self.active_graph_key() {
                self.graph_edit(&key, action);
                return;
            }
            // The curve editor claims the same focus routing (45-A P8b) — it
            // just answers fewer verbs: there is no curve clipboard, so
            // Cut/Copy/Paste/Duplicate fall through to the scene rather than
            // being silently swallowed by the focused tab.
            if let Some(key) = self.active_curve_key() {
                if matches!(
                    action,
                    EditorAction::Undo | EditorAction::Redo | EditorAction::Delete
                ) {
                    self.curve_edit(&key, action);
                    return;
                }
            }
        }
        match action {
            EditorAction::NewScene => {
                let _ = self.create_new_scene();
            }
            EditorAction::OpenScene => {
                self.editor.ui.crusty_dock.open_tab(EditorTab::AssetBrowser);
                self.push_action_unavailable(
                    "Open Scene",
                    "use the Asset Browser to open a .scene file",
                );
            }
            EditorAction::SaveScene => self.save_active_scene(),
            EditorAction::SaveSceneAs(path) => {
                if let Some(path) = path {
                    let filename = path
                        .file_stem()
                        .and_then(|stem| stem.to_str())
                        .unwrap_or("Untitled");
                    self.commit_save_as(filename);
                } else if self.editor.scene.save_as_dialog.is_none() {
                    let initial = self.editor.scene.current_scene_name.clone();
                    self.editor.scene.save_as_dialog = Some(SaveAsDialog::new(&initial));
                }
            }
            EditorAction::Quit => {
                self.save_layout_on_exit();
                println!("Closing...");
                std::process::exit(0);
            }
            EditorAction::Undo => {
                if self.play_mode() == PlayMode::Edit {
                    if let Some(desc) = self
                        .editor
                        .scene
                        .command_history
                        .undo(self.core.game_world.hecs_mut())
                    {
                        self.editor
                            .console
                            .messages
                            .push(LogMessage::info(format!("Undo: {desc}")));
                        self.post_history_change();
                    }
                }
            }
            EditorAction::Redo => {
                if self.play_mode() == PlayMode::Edit {
                    if let Some(desc) = self
                        .editor
                        .scene
                        .command_history
                        .redo(self.core.game_world.hecs_mut())
                    {
                        self.editor
                            .console
                            .messages
                            .push(LogMessage::info(format!("Redo: {desc}")));
                        self.post_history_change();
                    }
                }
            }
            EditorAction::Cut => {
                if self.play_mode() == PlayMode::Edit {
                    if self.copy_selection_to_clipboard() > 0 {
                        self.delete_selection_undoable("Cut");
                    }
                }
            }
            EditorAction::Copy => {
                if self.play_mode() == PlayMode::Edit {
                    let n = self.copy_selection_to_clipboard();
                    if n > 0 {
                        self.editor.console.messages.push(LogMessage::info(format!(
                            "Copied {n} {}",
                            if n == 1 { "entity" } else { "entities" }
                        )));
                    }
                }
            }
            EditorAction::Paste => {
                if self.play_mode() == PlayMode::Edit {
                    self.paste_clipboard();
                }
            }
            EditorAction::Duplicate => {
                if self.play_mode() == PlayMode::Edit {
                    let selected: Vec<_> =
                        self.editor.scene.selection.all().copied().collect();
                    for entity in selected {
                        if self.core.game_world.hecs().contains(entity) {
                            self.editor
                                .scene
                                .hierarchy_panel
                                .duplicate_entity(self.core.game_world.hecs_mut(), entity);
                        }
                    }
                    self.editor
                        .scene
                        .hierarchy_panel
                        .sync_root_order(self.core.game_world.hecs());
                    self.editor.scene.command_history.mark_dirty();
                }
            }
            EditorAction::Delete => {
                if self.play_mode() == PlayMode::Edit {
                    self.delete_selection_undoable("Delete");
                } else {
                    // Play mode: transient delete — snapshot restore discards it.
                    let selected: Vec<_> = self.editor.scene.selection.all().copied().collect();
                    for entity in selected {
                        if self.core.game_world.hecs().contains(entity) {
                            self.editor.scene.selection.remove(entity);
                            despawn_recursive(self.core.game_world.hecs_mut(), entity);
                        }
                    }
                    self.editor
                        .scene
                        .hierarchy_panel
                        .sync_root_order(self.core.game_world.hecs());
                    self.editor.scene.command_history.mark_dirty();
                }
            }
            EditorAction::ToggleHierarchy => self.toggle_tab(EditorTab::Hierarchy),
            EditorAction::ToggleInspector => self.toggle_tab(EditorTab::Inspector),
            EditorAction::ToggleAssetBrowser => self.toggle_tab(EditorTab::AssetBrowser),
            EditorAction::ToggleConsole => self.toggle_tab(EditorTab::Console),
            EditorAction::ToggleProfiler => self.toggle_tab(EditorTab::Profiler),
            EditorAction::ResetLayoutToDefault => {
                self.editor.ui.crusty_dock.reset();
                let _ = self.editor.ui.crusty_dock.save_to_default();
                self.editor
                    .console
                    .messages
                    .push(LogMessage::info("Layout reset to default".to_string()));
            }
            EditorAction::SwitchDensity(density) => {
                // Density writes the same pref field the Appearance slider
                // does, so the two can never disagree.
                self.editor.ui.settings.prefs.ui_scale = density.ui_scale();
                self.apply_editor_prefs(false);
            }
            EditorAction::ToggleDevShowcase => {
                self.push_action_unavailable(
                    "Widget Showcase",
                    "the showcase window is not available in this build",
                );
            }
            EditorAction::SelectAll => {
                let entities: Vec<_> = self
                    .core
                    .game_world
                    .hecs()
                    .iter()
                    .map(|entity_ref| entity_ref.entity())
                    .collect();
                for entity in entities {
                    self.editor.scene.selection.add(entity);
                }
            }
            EditorAction::DeselectAll => self.editor.scene.selection.clear(),
            EditorAction::FocusSelection => {
                let Some(entity) = self.editor.scene.selection.primary() else {
                    return;
                };
                if let Ok(transform) = self.core.game_world.hecs().get::<&Transform>(entity) {
                    let center = glam::Vec3::new(
                        transform.position.x,
                        transform.position.y,
                        transform.position.z,
                    );
                    self.editor.viewport.camera.focus_on(center, 2.0);
                }
            }
            EditorAction::FindEntityByName => {
                self.editor.ui.crusty_dock.open_tab(EditorTab::Hierarchy);
                self.push_action_unavailable("Find Entity", "use the Hierarchy search field");
            }
            EditorAction::TogglePlayMode => match self.play_mode() {
                PlayMode::Edit => self.enter_play_mode(),
                PlayMode::Playing | PlayMode::Paused => self.stop_play_mode(),
            },
            EditorAction::StepFrame => {
                self.push_action_unavailable(
                    "Step Frame",
                    "single-frame stepping is not implemented yet",
                );
            }
            EditorAction::RestartFromEditState => {
                if self.play_mode() != PlayMode::Edit {
                    self.stop_play_mode();
                }
                self.enter_play_mode();
            }
            EditorAction::SwitchDebugView(view) => {
                self.core.current_debug_view = view;
                self.core.deferred_renderer.set_debug_view(view);
            }
            EditorAction::ToggleWireframe => {
                self.push_action_unavailable(
                    "Wireframe",
                    "global wireframe mode is not implemented yet",
                );
            }
            EditorAction::ToggleGrid => {
                self.editor.viewport.grid_visible = !self.editor.viewport.grid_visible;
                self.editor.viewport.settings.grid_visible = self.editor.viewport.grid_visible;
            }
            EditorAction::ToggleGizmos => {
                self.push_action_unavailable(
                    "Gizmos",
                    "global gizmo visibility is not implemented yet",
                );
            }
            EditorAction::ReimportSelected => {
                self.editor.scene.asset_browser.request_rescan();
                self.editor.console.messages.push(LogMessage::info(
                    "Asset browser rescan requested".to_string(),
                ));
            }
            EditorAction::ShowInExplorer => {
                if let Some(asset) = self.editor.scene.asset_browser.selected_assets().first() {
                    let path = self
                        .editor
                        .scene
                        .asset_browser
                        .registry
                        .root_path()
                        .join(&asset.path);
                    let _ = std::process::Command::new("explorer")
                        .arg("/select,")
                        .arg(path)
                        .spawn();
                } else {
                    self.push_action_unavailable("Show in Explorer", "select an asset first");
                }
            }
            EditorAction::RevealInAssetBrowser => {
                self.editor.ui.crusty_dock.open_tab(EditorTab::AssetBrowser);
            }
            EditorAction::ReloadAllShaders => self.rebuild_all_shaders(),
            EditorAction::OpenSettings => {
                self.editor
                    .ui
                    .crusty_dock
                    .open_tab(EditorTab::InputSettings);
            }
            EditorAction::SaveAndCloseEditor { kind, key } => {
                match self.save_secondary_editor(kind, &key) {
                    Ok(()) => {
                        self.set_secondary_editor_open(kind, &key, false);
                        #[cfg(feature = "editor")]
                        if kind == SecondaryWindowKind::Graph {
                            self.close_graph_tab(&key);
                        }
                        #[cfg(feature = "editor")]
                        if kind == SecondaryWindowKind::Curve {
                            self.close_curve_tab(&key);
                        }
                    }
                    Err(error) => self.editor.console.messages.push(LogMessage::error(format!(
                        "Failed to save '{}': {}",
                        key, error
                    ))),
                }
            }
            EditorAction::DiscardAndCloseEditor { kind, key } => {
                match kind {
                    SecondaryWindowKind::Mesh => {
                        if let Some(data) = self.editor.scene.mesh_editors.get_mut(&key) {
                            data.dirty = false;
                        }
                    }
                    SecondaryWindowKind::Graph => {
                        if let Some(state) = self.editor.scene.graph_editors.get_mut(&key) {
                            state.dirty = false;
                        }
                    }
                    SecondaryWindowKind::Curve => {
                        if let Some(state) = self.editor.scene.curve_editors.get_mut(&key) {
                            state.dirty = false;
                        }
                    }
                    SecondaryWindowKind::InputAction => {
                        if let Some(data) = self
                            .editor
                            .scene
                            .input_action_editor
                            .open_actions
                            .get_mut(&key)
                        {
                            data.dirty = false;
                        }
                    }
                    SecondaryWindowKind::InputContext => {
                        if let Some(data) = self
                            .editor
                            .scene
                            .input_context_editor
                            .open_contexts
                            .get_mut(&key)
                        {
                            data.dirty = false;
                        }
                    }
                    _ => {}
                }
                self.editor.services.dirty.clear_asset(&key);
                self.set_secondary_editor_open(kind, &key, false);
                #[cfg(feature = "editor")]
                if kind == SecondaryWindowKind::Graph {
                    self.close_graph_tab(&key);
                }
                #[cfg(feature = "editor")]
                if kind == SecondaryWindowKind::Curve {
                    self.close_curve_tab(&key);
                }
            }
            EditorAction::GraphSaveGraph => {
                #[cfg(feature = "editor")]
                match self.active_graph_key() {
                    Some(key) => self.save_graph_editor(&key),
                    None => self.editor.console.messages.push(LogMessage::info(
                        "Graph: Save Graph — focus a graph tab first".to_string(),
                    )),
                }
            }
        }
    }

    /// One console warning per play session when the server simulates a
    /// different scene than the one open in the editor (M9.6: PIE keeps the
    /// open scene; prediction collision may diverge, server stays
    /// authoritative).
    fn warn_on_net_scene_mismatch(&mut self) {
        if self.editor.play.net_scene_warned {
            return;
        }
        let Some(server_scene) = self.net.as_ref().and_then(|n| n.world_scene()) else {
            return;
        };
        if server_scene != self.editor.scene.current_scene_relative {
            self.editor.console.messages.push(LogMessage::warning(format!(
                "Server world is '{server_scene}' but the open scene is '{}' — \
                 playing on the open scene; movement collision may diverge",
                self.editor.scene.current_scene_relative
            )));
        }
        self.editor.play.net_scene_warned = true;
    }

    /// M9.6 P5: Number of Players — N−1 extra clients as child processes,
    /// each on its own identity slot (`--net-id editor_p<i>`, the M9.5
    /// same-machine lesson). Prefers the packaged exe; falls back to
    /// `cargo run --release`. Killed on play-exit.
    fn spawn_extra_clients(&mut self, host: &str, module: &str) {
        let count = self.editor.ui.settings.prefs.play.player_count;
        for i in 1..count {
            let exe = std::path::Path::new("build/export/game.exe");
            let mut cmd = if exe.exists() {
                let mut c = std::process::Command::new(exe);
                c.current_dir("build/export");
                c
            } else {
                let mut c = std::process::Command::new("cargo");
                c.args(["run", "--release", "-p", "game_client", "--"]);
                c
            };
            cmd.args(["--connect", host, module, "--net-id", &format!("editor_p{i}")]);
            match cmd.spawn() {
                Ok(child) => {
                    self.editor.console.messages.push(LogMessage::info(format!(
                        "Players: launched extra client {} of {count} (editor_p{i})",
                        i + 1
                    )));
                    self.play_clients.push(child);
                }
                Err(e) => self
                    .editor
                    .console
                    .messages
                    .push(LogMessage::error(format!(
                        "Players: failed to launch extra client {}: {e}",
                        i + 1
                    ))),
            }
        }
    }

    /// M9.6 P4: once the off-thread launcher reports the local server up
    /// and published, join it as a client (play continues meanwhile).
    fn poll_listen_server_launcher(&mut self) {
        let Some(rx) = self.listen_server_rx.take() else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok(())) => {
                let ps = &self.editor.ui.settings.prefs.play;
                let (host, module) = (ps.host.clone(), ps.module.clone());
                self.editor.console.messages.push(LogMessage::info(format!(
                    "Listen Server ready: connecting to {host} / {module}"
                )));
                self.spawn_extra_clients(&host, &module);
                self.net = Some(crate::net::NetSession::connect_to(host, module));
            }
            Ok(Err(e)) => {
                self.editor
                    .console
                    .messages
                    .push(LogMessage::error(format!("Listen Server failed: {e}")));
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                self.listen_server_rx = Some(rx);
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {}
        }
    }

    pub fn update(&mut self) {
        rust_engine::profile_function!();

        self.process_hot_reload();
        // M9.6 P3: net runs while playing OR paused (freezing the pump
        // under pause would time the connection out), never in edit mode.
        let play_mode = self
            .core
            .game_world
            .resource::<EditorState>()
            .map(|s| s.play_mode)
            .unwrap_or(PlayMode::Edit);
        if play_mode != PlayMode::Edit {
            self.poll_listen_server_launcher();
            if let Some(net) = &mut self.net {
                net.update(&mut self.core.game_world);
            }
            self.warn_on_net_scene_mismatch();
        }
        self.update_world_streaming();
        self.resolve_mesh_paths();
        self.resolve_material_sets();

        let delta_time = self.core.game_loop.tick();

        if let Some(time) = self.core.game_world.resource_mut::<Time>() {
            time.advance(delta_time);
        }

        self.core.game_world.run_schedule(&mut self.core.schedule);

        // Graph `Print` output, and the runner's own refusals, reach the
        // Console here. There is no logger in this binary, so the runner parks
        // its lines in a bounded sink and this is the one place that empties
        // it — immediately after the schedule, so a print and the frame that
        // produced it land together.
        #[cfg(feature = "graph-scripting")]
        {
            use rust_engine::engine::scripting::log_sink;
            let console = &mut self.editor.console.messages;
            if let Some(sink) = self
                .core
                .game_world
                .resources_mut()
                .get_mut::<log_sink::GraphLogSink>()
            {
                log_sink::drain_into_console(sink, console);
            }
        }

        // Update debug draw persistent line lifetimes
        #[cfg(debug_assertions)]
        self.core.debug_draw_buffer.update(delta_time);
    }

    /// Park the active scene and open `relative` in a fresh tab.
    ///
    /// Extracted from the asset-browser double-click arm in 39.8 P2 so the
    /// content moment it contains has a name and can be exercised directly.
    fn open_scene_in_new_tab(&mut self, relative: &str, display_name: &str) {
        let new_id = self.editor.scene.registry.allocate_id();
        let parked = self.park_active_scene();
        self.editor.scene.registry.park(parked);

        self.core.game_world = self.fresh_scene_world();
        self.editor.scene.registry.active_id = new_id;
        self.editor.scene.current_scene_relative = relative.to_string();
        self.editor.scene.current_scene_name = String::new();
        self.editor.scene.active_dirty = false;
        self.editor.scene.selection.clear();
        self.editor.scene.command_history.clear();
        self.editor.scene.hierarchy_panel.set_root_order(Vec::new());
        self.editor.ui.crusty_dock.open_viewport_tab(new_id);

        match load_scene(self.core.game_world.hecs_mut(), relative) {
            Ok((scene_name, root_entities)) => {
                self.editor.scene.hierarchy_panel.set_root_order(root_entities);
                self.editor.scene.current_scene_name = scene_name.clone();
                // Content moment. Before P2 this path registered no physics
                // bodies at all, so a scene opened in a new tab had an empty
                // Rapier world until play mode happened to rebuild it.
                self.run_world_population_hooks();
                {
                    self.core
                        .game_world
                        .resources_mut()
                        .remove::<TransformCache>();
                    let mut tc = TransformCache::new();
                    tc.propagate(self.core.game_world.hecs_mut());
                    self.core.game_world.resources_mut().insert(tc);
                }
                // Resolve mesh_path → mesh_index for loaded entities
                self.resolve_mesh_paths();
                self.init_streaming_for_scene(relative);

                self.editor
                    .console
                    .messages
                    .push(LogMessage::info(format!("Loaded scene: {}", scene_name)));
                println!("Scene loaded: {}", display_name);
            }
            Err(e) => {
                self.editor
                    .console
                    .messages
                    .push(LogMessage::error(format!("Failed to load scene: {}", e)));
                eprintln!("Failed to load scene: {}", e);
            }
        }
    }

    /// Post-population hooks for every editor content moment (39.8 ruling
    /// §5.5): re-register physics bodies, then run each plugin's
    /// `on_world_loaded`. Editor policy on failure is surface-and-continue.
    fn run_world_population_hooks(&mut self) {
        let failures = crate::world_population::after_world_populated(
            &mut self.core.game_world,
            &mut self.core.plugin_set,
        );
        crate::world_population::report_failures(&failures);
        for failure in &failures {
            self.editor
                .console
                .messages
                .push(LogMessage::error(crate::world_population::describe(failure)));
        }
    }

    /// Resolve `mesh_path` to `mesh_index` for all MeshRenderer components.
    fn resolve_mesh_paths(&mut self) {
        crate::asset_resolve::resolve_mesh_paths(
            self.core.game_world.hecs_mut(),
            &self.core.asset_manager,
        );
    }

    fn material_gpu(&self) -> crate::asset_resolve::MaterialGpu {
        use vulkano::pipeline::Pipeline;
        crate::asset_resolve::MaterialGpu {
            allocator: self.core.renderer.gpu.memory_allocator.clone(),
            ds_allocator: self.core.renderer.gpu.descriptor_set_allocator.clone(),
            cmd_allocator: self.core.renderer.gpu.command_buffer_allocator.clone(),
            queue: self.core.renderer.gpu.queue.clone(),
            device: self.core.renderer.gpu.device.clone(),
            geom_layout: self
                .core
                .deferred_renderer
                .geometry_pipeline()
                .layout()
                .clone(),
        }
    }

    /// Load `.material` / `.matinst` files referenced by MeshRenderers and cache their
    /// GPU descriptor sets so `prepare_mesh_data` can bind them.
    fn resolve_material_sets(&mut self) {
        let gpu = self.material_gpu();
        crate::asset_resolve::resolve_material_sets(
            self.core.game_world.hecs_mut(),
            &self.core.asset_manager,
            &gpu,
            &mut self.core.materials,
        );
    }

    fn process_hot_reload(&mut self) {
        while let Ok(event) = self.core.reload_rx.try_recv() {
            match event {
                ReloadEvent::ModelChanged {
                    path,
                    mesh_indices: new_indices,
                    model: _,
                } => {
                    if !new_indices.is_empty() {
                        for (_entity, mesh_renderer) in
                            self.core
                                .game_world
                                .hecs_mut()
                                .query_mut::<&mut rust_engine::engine::ecs::components::MeshRenderer>()
                        {
                            if mesh_renderer.mesh_path == path {
                                mesh_renderer.mesh_index = new_indices[0];
                            }
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
                    // Evict the instance; resolve_material_sets rebuilds it
                    // next frame from the changed file. Watcher paths are
                    // absolute with forward slashes, cache keys are
                    // content-relative and may use backslashes.
                    let matches_event = |key: &str| path.ends_with(&key.replace('\\', "/"));
                    let stale: Vec<String> = self
                        .core
                        .materials
                        .matinst_ids
                        .keys()
                        .filter(|k| matches_event(k))
                        .cloned()
                        .collect();
                    for key in &stale {
                        if let Some(id) = self.core.materials.matinst_ids.remove(key) {
                            self.core.materials.manager.remove_instance(id);
                        }
                        self.core.materials.cache.remove(key);
                    }
                    println!("Material instance changed: {}", path);
                    self.editor.console.messages.push(LogMessage::info(format!(
                        "Material instance reloaded: {}",
                        path,
                    )));
                }
                ReloadEvent::GraphChanged { path } => {
                    #[cfg(feature = "editor")]
                    self.on_graph_changed(&path);
                    #[cfg(not(feature = "editor"))]
                    let _ = path;
                }
                ReloadEvent::CurveChanged { path } => {
                    #[cfg(feature = "editor")]
                    self.on_curve_changed(&path);
                    #[cfg(not(feature = "editor"))]
                    let _ = path;
                }
                ReloadEvent::ShaderChanged { path } => {
                    use rust_engine::engine::rendering::shader_compiler::ShaderCompiler;

                    println!("Shader changed: {}", path);
                    let compiler = match ShaderCompiler::new() {
                        Ok(c) => c,
                        Err(e) => {
                            self.editor.console.messages.push(LogMessage::error(format!(
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
                                self.editor.console.messages.push(LogMessage::info(format!(
                                    "Hot-reloaded pipeline {:?}",
                                    result.id
                                )));
                            }
                            Err(e) => {
                                self.editor.console.messages.push(LogMessage::error(format!(
                                    "Pipeline {:?} hot-reload failed: {}",
                                    result.id, e
                                )));
                            }
                        }
                    }
                }
            }
        }
    }

    pub fn handle_window_event(&mut self, event: &WindowEvent, _event_loop: &ActiveEventLoop) {
        #[cfg(feature = "editor")]
        self.crusty_gui.handle_event(event);

        match event {
            WindowEvent::Resized(new_size) => {
                self.core.renderer.swapchain_state.recreate_swapchain = true;
                #[cfg(feature = "editor")]
                self.crusty_gui
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
                        && self
                            .core
                            .game_world
                            .resource::<InputManager>()
                            .is_some_and(|im| im.is_winit_key_pressed(KeyCode::ControlLeft))
                    {
                        // Ctrl+S saves the focused graph tab when one has focus,
                        // otherwise the active scene.
                        #[cfg(feature = "editor")]
                        let saved_graph = if let Some(key) = self.active_graph_key() {
                            self.save_graph_editor(&key);
                            true
                        } else if let Some(key) = self.active_curve_key() {
                            self.save_curve_editor(&key);
                            true
                        } else {
                            false
                        };
                        #[cfg(not(feature = "editor"))]
                        let saved_graph = false;
                        if !saved_graph {
                            self.save_active_scene();
                        }
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
    /// CB can be chained with its window's acquire → UI → present chain.
    /// This keeps the preview render and the UI sample in the **same**
    /// Vulkan submission, eliminating cross-submission layout/memory issues.
    pub fn build_mesh_preview_cbs(
        &mut self,
    ) -> Vec<(
        String,
        std::sync::Arc<vulkano::command_buffer::PrimaryAutoCommandBuffer>,
    )> {
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
                            Ok(state) => {
                                // Initialize the image layout (General) before any
                                // GUI pass can sample it — a fresh image is in
                                // Undefined layout, which is a validation panic.
                                if let Err(e) = state.texture.clear(
                                    self.core.renderer.gpu.queue.clone(),
                                    self.core.renderer.gpu.command_buffer_allocator.clone(),
                                ) {
                                    eprintln!("Mesh preview clear failed: {}", e);
                                }
                                data.preview = Some(state);
                            }
                            Err(e) => {
                                eprintln!("Failed to create mesh preview: {}", e);
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
                                // Same layout-init as at creation: the resized
                                // image must never be sampled while Undefined.
                                if let Err(e) = preview.texture.clear(
                                    self.core.renderer.gpu.queue.clone(),
                                    self.core.renderer.gpu.command_buffer_allocator.clone(),
                                ) {
                                    eprintln!("Mesh preview clear failed: {}", e);
                                }
                            }
                        }
                    }

                    // Always render the preview when we have mesh data and a
                    // valid size.  The CB must be in the submission chain every
                    // frame (matching the main viewport pattern) so vulkano's
                    // AutoCommandBufferBuilder in the UI CB correctly tracks
                    // the image layout transition and inserts a proper barrier
                    // with memory-visibility flags.  Rendering only when dirty
                    // leaves frames without a preview CB, and the UI builder
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
                                match renderer.render(&preview.framebuffer, pw, ph, &gpu_meshes, vp)
                                {
                                    Ok(cb) => {
                                        result.push((key.clone(), cb));
                                        data.preview_dirty = false;
                                    }
                                    Err(e) => {
                                        eprintln!("Mesh preview render error: {}", e);
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
                    rust_engine::engine::rendering::frame_packet::RenderEvent::RenderError { message } => {
                        // Surface on the terminal and in the editor console —
                        // no logger is installed, so log:: alone is invisible.
                        eprintln!("editor: render thread error: {}", message);
                        self.editor.console.messages.push(LogMessage::error(format!(
                            "Render thread error: {}",
                            message
                        )));
                    }
                    rust_engine::engine::rendering::frame_packet::RenderEvent::CrustyTexturesRegistered(regs) => {
                        self.editor
                            .scene
                            .asset_browser
                            .thumbnails
                            .apply_crusty_registered(regs);
                    }
                    rust_engine::engine::rendering::frame_packet::RenderEvent::CrustyNativeRegistered(regs) => {
                        for (key, tid) in regs {
                            if let Some(k) = key.strip_prefix("mesh_preview:") {
                                if let Some(entry) = self.crusty_mesh_textures.get_mut(k) {
                                    entry.id = Some(tid);
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        if !self.editor.ui.icons_loaded {
            self.editor.services.load_icons_crusty();
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
            self.core.deferred_renderer.default_material_set(),
            &self.core.materials.cache,
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

        // Plugin debug overlays (39.8 D7): the collider wireframes come from
        // `RapierPhysicsPlugin`, so with it disabled nothing is drawn and this
        // path never mentions physics.
        #[cfg(debug_assertions)]
        self.core
            .plugin_set
            .run_debug_draw(self.core.game_world.hecs(), &mut self.core.debug_draw_buffer);

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

        // Cooked collision debug: chunk wireframe / grid overlay, plus a
        // temporary M2 verification — fly-camera raycast marker against the
        // ChunkStore (visually checks cooked transforms against the render).
        #[cfg(debug_assertions)]
        if let Some(collision) = self.core.game_world.resource::<CollisionWorld>() {
            // Collision wireframes are static and huge (millions of lines for
            // a full world) — cache the uploaded GPU buffer and rebuild only
            // when the chunk contents or toggles change.
            let key = (
                collision.generation(),
                collision.debug_draw_chunks,
                collision.debug_draw_grid,
            );
            if self.core.collision_debug_cache.as_ref().map(|c| c.key) != Some(key) {
                let mut lines = rust_engine::engine::debug_draw::DebugDrawBuffer::new();
                collision.submit_debug_draws(&mut lines);
                let (depth_lines, _) = lines.drain();
                self.core.collision_debug_cache = Some(CollisionDebugCache {
                    key,
                    lines: render_loop::upload_debug_lines(&depth_lines, &self.core.renderer),
                });
            }
            if collision.debug_draw_chunks {
                use rust_engine::engine::utils::coords::convert_position_yup_to_zup;
                let origin = convert_position_yup_to_zup(camera_pos);
                let dir = convert_position_yup_to_zup(
                    (self.editor.viewport.camera.target - camera_pos).normalize_or_zero(),
                );
                if dir.length_squared() > 0.0 {
                    if let Some(hit) = collision.store().raycast(origin, dir, 1000.0) {
                        let p = hit.position;
                        let red = [1.0, 0.15, 0.15, 1.0];
                        for axis in [glam::Vec3::X, glam::Vec3::Y, glam::Vec3::Z] {
                            let a = p - axis * 0.25;
                            let b = p + axis * 0.25;
                            self.core
                                .debug_draw_buffer
                                .line_overlay(a.into(), b.into(), red);
                        }
                        self.core.debug_draw_buffer.line_overlay(
                            p.into(),
                            (p + hit.normal).into(),
                            [0.2, 0.4, 1.0, 1.0],
                        );
                    }
                }
            }
        }

        #[cfg(debug_assertions)]
        let debug_draw_data = {
            let mut data = render_loop::prepare_debug_draw_data(
                &mut self.core.debug_draw_buffer,
                &self.core.renderer,
            );
            if let Some((buf, count)) = self
                .core
                .collision_debug_cache
                .as_ref()
                .and_then(|c| c.lines.clone())
            {
                data.static_depth_buffer = Some(buf);
                data.static_depth_vertex_count = count;
            }
            data
        };
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
            // Must match `view_proj`, which is built from the viewport camera
            // (renderer.camera_3d never receives its near/far).
            (
                self.editor.viewport.camera.near,
                self.editor.viewport.camera.far,
            ),
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

        // Derive dirty flag from the active command history each frame so it stays in sync.
        // Done BEFORE we take the mutable borrow of command_history below.
        self.editor.scene.active_dirty = self.editor.scene.command_history.is_dirty();
        let active_dirty = self.editor.scene.active_dirty;
        let active_scene_id = self.editor.scene.registry.active_id;
        let current_scene_name = self.editor.scene.current_scene_name.clone();

        // Compute layout rects for the crusty pass in physical pixels
        // (crusty runs at pixels_per_point = 1.0, so the values below are the
        // exact screen coordinates the panels draw to).
        use rust_engine::engine::editor::dock_crusty::{Pos2 as CPos2, Rect as CRect};
        let size = self.core.window.inner_size();
        let screen_w = size.width as f32;
        let screen_h = size.height as f32;
        let crusty_screen_rect =
            CRect::from_min_max(CPos2::new(0.0, 0.0), CPos2::new(screen_w, screen_h));
        let crusty_menu_bar_rect =
            CRect::from_min_max(CPos2::new(0.0, 0.0), CPos2::new(screen_w, 30.0));
        let crusty_status_bar_rect = CRect::from_min_max(
            CPos2::new(0.0, screen_h - 28.0),
            CPos2::new(screen_w, screen_h),
        );

        let is_hovering_files = self.crusty_gui.is_hovering_external_files();

        let mut menu_action = MenuAction::None;
        let toolbar_action = MenuAction::None;
        let close_scene_request: Option<SceneId> = None;

        // Apply edited input bindings back to the InputSubsystem
        if let Some(new_set) = self.editor.ui.input_settings_panel.take_pending_apply() {
            if let Some(subsystem) = self.core.game_world.resource_mut::<InputSubsystem>() {
                subsystem.set_action_set(new_set);
            }
        }

        // Process Save As dialog commit
        let save_as_commit = self
            .editor
            .scene
            .save_as_dialog
            .as_ref()
            .map(|d| (d.commit, d.filename.clone()));
        if let Some((true, filename)) = save_as_commit {
            self.editor.scene.save_as_dialog = None;
            self.commit_save_as(&filename);
        }

        // Process viewport tab close requests (deferred from on_close).
        if let Some(scene_id) = close_scene_request {
            self.close_scene_tab(scene_id);
        }

        // Detect tab switch: if the dock's currently-focused viewport tab is
        // not the active scene id, perform a swap.
        let focused_scene = self.editor.ui.crusty_dock.focused_viewport_id();
        if let Some(focused_scene_id) = focused_scene {
            if focused_scene_id != self.editor.scene.registry.active_id {
                self.switch_to_scene(focused_scene_id);
            }
        }

        if menu_action == MenuAction::None && toolbar_action != MenuAction::None {
            menu_action = toolbar_action;
        }
        // Crusty menu actions are recorded after this match runs (the crusty
        // layout is at the end of the frame), so apply last frame's here.
        if menu_action == MenuAction::None {
            menu_action = std::mem::replace(&mut self.crusty_menu_action, MenuAction::None);
        }

        match menu_action {
            MenuAction::None => {}
            MenuAction::NewScene => {
                let _ = self.create_new_scene();
            }
            MenuAction::SaveScene => self.save_active_scene(),
            MenuAction::CookCollision => self.cook_scene_collision(),
            MenuAction::Exit => {
                self.save_layout_on_exit();
                println!("Closing...");
                std::process::exit(0);
            }
            MenuAction::Undo => self.handle_editor_action(EditorAction::Undo),
            MenuAction::Redo => self.handle_editor_action(EditorAction::Redo),
            MenuAction::Cut => self.handle_editor_action(EditorAction::Cut),
            MenuAction::Copy => self.handle_editor_action(EditorAction::Copy),
            MenuAction::Paste => self.handle_editor_action(EditorAction::Paste),
            MenuAction::Duplicate => self.handle_editor_action(EditorAction::Duplicate),
            MenuAction::Delete => self.handle_editor_action(EditorAction::Delete),
            MenuAction::OpenEditorPreferences => {
                self.editor.ui.settings.prefs_open = true;
            }
            MenuAction::OpenProjectSettings => {
                self.editor.ui.settings.project_open = true;
            }
            MenuAction::OpenKeyboardShortcuts => {
                self.editor.ui.settings.prefs_open = true;
                self.editor.ui.settings.select_prefs_category(
                    rust_engine::engine::editor::settings_crusty::shortcuts_category(),
                );
            }
            MenuAction::OpenPlugins => {
                self.editor.ui.settings.project_open = true;
                self.editor.ui.settings.project_cat =
                    rust_engine::engine::editor::settings_crusty::plugins_category();
            }
            MenuAction::SaveLayout => {
                match self.editor.ui.crusty_dock.save_to_default() {
                    Ok(()) => println!(
                        "Layout saved to {}",
                        rust_engine::engine::editor::dock_crusty::CrustyDockLayout::default_layout_path().display()
                    ),
                    Err(e) => eprintln!("Failed to save layout: {}", e),
                }
            }
            MenuAction::ResetLayout => {
                self.editor.ui.crusty_dock.reset();
                let _ = self.editor.ui.crusty_dock.save_to_default();
                println!("Layout reset to default");
            }
            MenuAction::LoadBenchmarkScene => self.load_benchmark_scene(),
            MenuAction::RunBenchmark => self.run_benchmark(),
            MenuAction::Play => self.enter_play_mode(),
            MenuAction::Pause => self.pause_play_mode(),
            MenuAction::Resume => self.resume_play_mode(),
            MenuAction::Stop => self.stop_play_mode(),
            MenuAction::RebuildShaders => self.rebuild_all_shaders(),
            MenuAction::ToggleCollisionChunkDraw => {
                if let Some(collision) = self.core.game_world.resource_mut::<CollisionWorld>() {
                    collision.debug_draw_chunks = !collision.debug_draw_chunks;
                }
            }
            MenuAction::ToggleCollisionGridDraw => {
                if let Some(collision) = self.core.game_world.resource_mut::<CollisionWorld>() {
                    collision.debug_draw_grid = !collision.debug_draw_grid;
                }
            }
            MenuAction::ToggleStreamAroundCamera => {
                // full→rings: far cells unload next update; rings→full: the
                // rest of the world loads back in. No flush needed.
                let streamer = &mut self.core.world_streamer;
                streamer.full_world = !streamer.full_world;
                let mode = if streamer.full_world {
                    "full world"
                } else {
                    "around camera"
                };
                self.editor
                    .console
                    .messages
                    .push(LogMessage::info(format!("World streaming: {mode}")));
            }
            #[cfg(feature = "editor-debug")]
            MenuAction::ToggleIconInspector => {
                // Icon Inspector was a secondary-window tool tied to the old
                // UI runtime; removed along with that runtime.
                self.push_action_unavailable(
                    "Icon Inspector",
                    "the inspector window is not available in this build",
                );
            }
            #[cfg(feature = "editor-debug")]
            MenuAction::ToggleShowcase => {
                self.push_action_unavailable(
                    "Widget Showcase",
                    "the showcase window is not available in this build",
                );
            }
        }

        // Process OS file drops (files dragged from Windows Explorer / file manager)
        let dropped_files = self.crusty_gui.take_dropped_files();
        if !dropped_files.is_empty() {
            self.import_dropped_files(dropped_files);
        }

        // Process asset browser events
        let asset_events: Vec<_> = self.editor.scene.asset_browser.events.drain().collect();
        for event in asset_events {
            match event {
                AssetBrowserEvent::AssetOpened { id } => {
                    // Extract metadata fields before mutating self
                    let meta_info = self
                        .editor
                        .scene
                        .asset_browser
                        .registry
                        .get(id)
                        .map(|m| (m.asset_type, m.path.clone(), m.display_name.clone()));
                    if let Some((asset_type, meta_path, display_name)) = meta_info {
                        if asset_type == AssetType::Scene {
                            if self.play_mode() != PlayMode::Edit {
                                self.editor.console.messages.push(LogMessage::warning(
                                    "Stop play mode before loading a scene".to_string(),
                                ));
                                continue;
                            }

                            if meta_path.as_path() == std::path::Path::new(BENCHMARK_SCENE_RELATIVE)
                                && !self.runtime_flags.benchmark_tools_enabled
                            {
                                self.editor.console.messages.push(LogMessage::warning(
                                    "Benchmark scene access is locked behind --editor-benchmark-tools"
                                        .to_string(),
                                ));
                                continue;
                            }

                            // Registry paths are OS-native; scene-relative
                            // strings are forward-slash everywhere else
                            // (manifest scene field, MAIN_SCENE_RELATIVE,
                            // tab dedup).
                            let relative =
                                asset_source::to_content_relative(&meta_path.to_string_lossy());

                            // If this scene is already the active tab, nothing to do.
                            if relative == self.editor.scene.current_scene_relative {
                                continue;
                            }
                            // If it's open in another tab, just focus that tab.
                            if let Some(existing_id) =
                                self.editor.scene.registry.find_dormant_by_path(&relative)
                            {
                                self.switch_to_scene(existing_id);
                                continue;
                            }

                            self.open_scene_in_new_tab(&relative, &display_name);
                        } else if asset_type == AssetType::Audio {
                            // Play audio preview on dedicated preview track
                            let relative = meta_path.to_string_lossy().to_string();
                            let load_result = self.core.asset_manager.audio.load(&relative);
                            match load_result {
                                Ok(handle) => {
                                    let data = handle.get().clone();
                                    if let Some(engine) = self
                                        .core
                                        .game_world
                                        .resource_mut::<rust_engine::engine::audio::AudioEngine>(
                                    ) {
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
                            self.hydration_failed.remove(&format!("mesh:{relative}"));
                            if !self.editor.scene.mesh_editors.contains_key(&relative) {
                                if let Err(e) = self.open_mesh_document(&relative) {
                                    self.editor.console.messages.push(LogMessage::error(
                                        format!("Failed to open mesh '{relative}': {e}"),
                                    ));
                                    continue;
                                }
                                self.open_mesh_as_tab(relative);
                            } else {
                                // Already open: surface the docked tab. A tab
                                // living in a float OS window stays there.
                                #[cfg(feature = "editor")]
                                let in_float =
                                    self.crusty_float_hosts_tab(&format!("mesh:{relative}"));
                                #[cfg(not(feature = "editor"))]
                                let in_float = false;
                                if !in_float {
                                    self.open_mesh_as_tab(relative);
                                }
                            }
                        } else if asset_type == AssetType::Graph {
                            // Open node graph editor tab (Task 40 P4/P6).
                            let relative =
                                asset_source::to_content_relative(&meta_path.to_string_lossy());
                            #[cfg(feature = "editor")]
                            self.open_graph_document(relative);
                            #[cfg(not(feature = "editor"))]
                            let _ = relative;
                        } else if asset_type == AssetType::Curve {
                            // Open the `.curve` editor tab (45-A P8b).
                            let relative =
                                asset_source::to_content_relative(&meta_path.to_string_lossy());
                            #[cfg(feature = "editor")]
                            self.open_curve_document(relative);
                            #[cfg(not(feature = "editor"))]
                            let _ = relative;
                        } else if asset_type == AssetType::Animation {
                            // Anim event markers, as a minimal list (Task 41).
                            use rust_engine::engine::editor::anim_events_dialog::AnimEventsDialog;
                            let abs = self
                                .editor
                                .scene
                                .asset_browser
                                .registry
                                .root_path()
                                .join(&meta_path);
                            let relative =
                                asset_source::to_content_relative(&meta_path.to_string_lossy());
                            match AnimEventsDialog::load(abs, relative) {
                                Ok(dlg) => self.editor.scene.anim_events_dialog = Some(dlg),
                                Err(e) => self.editor.console.messages.push(LogMessage::error(
                                    format!("Cannot open anim events: {e}"),
                                )),
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
                AssetBrowserEvent::CreateAsset {
                    asset_type,
                    parent_path,
                } => {
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
                            let mut new_name = format!("{}.inputaction", base_name);
                            let mut counter = 1;
                            while full_parent.join(&new_name).exists() {
                                new_name = format!("{}_{}.inputaction", base_name, counter);
                                counter += 1;
                            }
                            let action_name = new_name.trim_end_matches(".inputaction").to_string();
                            let action = rust_engine::engine::input::enhanced_action::InputActionDefinition::new(
                                &action_name,
                                rust_engine::engine::input::value::InputValueType::Digital,
                            );
                            let file_path = full_parent.join(&new_name);
                            match enhanced_serialization::save_input_action(&action, &file_path) {
                                Ok(()) => {
                                    self.editor.console.messages.push(LogMessage::info(format!(
                                        "Created input action: {}",
                                        action_name
                                    )));
                                    self.editor.scene.asset_browser.request_rescan();
                                }
                                Err(e) => {
                                    self.editor.console.messages.push(LogMessage::error(format!(
                                        "Failed to create input action: {}",
                                        e
                                    )));
                                }
                            }
                        }
                        AssetType::InputMappingContext => {
                            let base_name = "NewMappingContext";
                            let mut new_name = format!("{}.mappingcontext", base_name);
                            let mut counter = 1;
                            while full_parent.join(&new_name).exists() {
                                new_name = format!("{}_{}.mappingcontext", base_name, counter);
                                counter += 1;
                            }
                            let ctx_name = new_name.trim_end_matches(".mappingcontext").to_string();
                            let mapping_ctx =
                                rust_engine::engine::input::enhanced_action::MappingContext::new(
                                    &ctx_name, 0,
                                );
                            let file_path = full_parent.join(&new_name);
                            match enhanced_serialization::save_mapping_context(
                                &mapping_ctx,
                                &file_path,
                            ) {
                                Ok(()) => {
                                    self.editor.console.messages.push(LogMessage::info(format!(
                                        "Created mapping context: {}",
                                        ctx_name
                                    )));
                                    self.editor.scene.asset_browser.request_rescan();
                                }
                                Err(e) => {
                                    self.editor.console.messages.push(LogMessage::error(format!(
                                        "Failed to create mapping context: {}",
                                        e
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

        // crusty-gui layout pass — the editor's sole UI. Runs before
        // `handle_frame_input` so its wants_* flags can gate game input
        // (camera, hotkeys) for the same frame.
        let mut crusty_close_tab: Option<String> = None;
        let mut crusty_ctx_close: Vec<String> = Vec::new();
        let mut crusty_docked = false;
        let mut crusty_float_drag: Option<(winit::window::WindowId, bool)> = None;
        let mut crusty_dialog_actions = Vec::new();
        let mut crusty_import_action = ImportDialogAction::None;
        let mut anim_events_action =
            rust_engine::engine::editor::anim_events_dialog::AnimEventsAction::None;
        // Subgraph node double-clicked in a docked graph this frame (P6);
        // declared out here so it outlives the layout block and can be applied
        // once the panel borrows are released.
        let mut graph_open_request: Option<String> = None;
        // Set by a graph toolbar's "Clear trace"; applied after the UI, where
        // the world is reachable (GS-3).
        let mut graph_clear_trace: Option<String> = None;
        // Which instance each graph tab is debugging, what it armed, and what
        // it asked the debugger for (GS-4). Applied after the UI for the same
        // reason "Clear trace" is: it writes to the running instances.
        let mut graph_debug: Vec<GraphDebugPlan> = Vec::new();
        // Toolbar quick-switch: the panel reports a style pick, the host
        // writes it into the same pref the Preferences window edits, so it
        // shows the overridden dot there and rides the debounced autosave.
        let mut graph_style_request: Option<WireStyle> = None;
        // Curve tabs that asked to be saved this frame (toolbar button or the
        // float window's own Ctrl+S). Saving reaches the plan/curve caches, so
        // it has to happen outside the panel borrows.
        let mut curve_save_requests: Vec<String> = Vec::new();
        let crusty_result = {
            use rust_engine::engine::editor::asset_browser_crusty::{
                asset_browser_panel, AssetBrowserPanelCtx,
            };
            use rust_engine::engine::editor::console_crusty::{console_panel, ConsolePanelCtx};
            use rust_engine::engine::editor::dock_crusty;
            use rust_engine::engine::editor::hierarchy_crusty::{
                hierarchy_panel, HierarchyPanelCtx,
            };
            use rust_engine::engine::editor::input_editors_crusty::{
                input_action_panel, input_context_panel, input_settings_panel,
            };
            use rust_engine::engine::editor::inspector_crusty::{
                inspector_panel, InspectorPanelCtx,
            };
            use rust_engine::engine::editor::menu_bar_crusty::{menu_bar_panel, MenuBarCtx};
            use rust_engine::engine::editor::mesh_editor_crusty::{
                mesh_editor_panel, MeshEditorPanelCtx,
            };
            use rust_engine::engine::editor::graph_editor_crusty::{
                graph_editor_panel, GraphEditorPanelCtx,
            };
            use rust_engine::engine::editor::curve_editor_crusty::{
                curve_editor_panel, CurveEditorPanelCtx,
            };
            use rust_engine::engine::editor::profiler_crusty::profiler_panel;
            use rust_engine::engine::editor::status_bar_crusty::{status_bar_panel, StatusBarCtx};
            use rust_engine::engine::plugins::PluginPanelCtx;
            use rust_engine::engine::editor::toasts_crusty::toasts_panel;
            use rust_engine::engine::editor::viewport_crusty::{viewport_panel, ViewportPanelCtx};
            // A tab dragged out of a float window: the OS window follows the
            // cursor; feed its position into the main dock as an external
            // drag so the compass/drop-zones light up for re-docking.
            let mut float_ext: Option<dock_crusty::ExternalDrag> = None;
            if let Ok(main_origin) = self.core.window.inner_position() {
                for (id, fw) in &self.crusty_floats {
                    if let (Some(drag), Some(screen)) = (&fw.drag_out, fw.drag_screen_pos()) {
                        float_ext = Some(dock_crusty::ExternalDrag {
                            tab_id: drag.tab.clone(),
                            pointer: dock_crusty::Pos2::new(
                                screen.x - main_origin.x as f32,
                                screen.y - main_origin.y as f32,
                            ),
                            grab: drag.grab,
                            released: fw.released,
                            force: false,
                            ghost: false,
                        });
                        crusty_float_drag = Some((*id, fw.released));
                        break;
                    }
                }
            }
            for fw in self.crusty_floats.values_mut() {
                fw.released = false;
            }
            // P6: refresh subgraph cross-asset validation + build the resolver
            // for the canvas panels; list `.subgraph` assets for the create
            // menu. Both must precede the `&mut self.editor` panel bindings.
            // Restored tabs get their documents before anything reads them —
            // and before `revalidate_graph_refs`, so a hydrated subgraph host
            // shows its cross-asset errors on this frame rather than the next.
            self.hydrate_restored_tabs();
            let GraphResolverDocs { docs: graph_resolver_docs, curves: graph_curve_docs } =
                self.revalidate_graph_refs();
            let subgraph_assets: Vec<String> = {
                let filter = rust_engine::engine::editor::AssetFilter {
                    asset_types: Some(vec![AssetType::Graph]),
                    include_subfolders: true,
                    ..Default::default()
                };
                self.editor
                    .scene
                    .asset_browser
                    .registry
                    .query(&filter)
                    .into_iter()
                    .filter(|m| m.path.extension().and_then(|e| e.to_str()) == Some("subgraph"))
                    .map(|m| asset_source::to_content_relative(&m.path.to_string_lossy()))
                    .collect()
            };
            // P9: graph canvas zoom limits from EditorPrefs.
            let graph_prefs = self.editor.ui.settings.prefs.graph;
            // Graph selection outline: preset-invariant except for Graphite's
            // achromatic carve-out, so it must come from the live theme.
            let graph_sel_outline = self.editor.services.theme.palette.selection.outline;
            let console = &mut self.editor.console;
            let fps = self.core.game_loop.fps();
            let delta_ms = self.core.game_loop.delta_ms();
            let streaming_overlay = if self.core.world_streamer.is_active() {
                let st = &self.core.world_streamer;
                Some(
                    rust_engine::engine::editor::viewport_crusty::StreamingOverlay {
                        cells: st.resident_cell_count(),
                        chunks: st.resident_chunk_count(),
                        in_flight: st.in_flight_count(),
                        ready: st.ready_queue_depth(),
                        worst_ms: st.worst_frame_ms(),
                    },
                )
            } else {
                None
            };
            let net_status = self.net.as_ref().map(|n| n.status_line());
            let world_object_info =
                rust_engine::engine::editor::world_object::WorldObjectInfo::from_streamer(
                    &self.core.world_streamer,
                );
            // Split borrow: plugin panels get both halves (P1's
            // `world_and_resources_mut`), every built-in panel keeps taking
            // just the hecs world.
            let (world, world_resources) = self.core.game_world.world_and_resources_mut();
            let show_stat_fps = &mut self.editor.ui.show_stat_fps;
            let vp = &mut self.editor.viewport;
            let vp_command_history = &mut self.editor.scene.command_history;
            let crusty_viewport_texture = self.crusty_viewport_texture;
            let hierarchy = &mut self.editor.scene.hierarchy_panel;
            let inspector = &mut self.editor.scene.inspector_panel;
            inspector.drive_eyedropper();
            let asset_browser = &mut self.editor.scene.asset_browser;
            let profiler = &mut self.editor.ui.profiler_panel;
            let input_settings = &mut self.editor.ui.input_settings_panel;
            let action_set = action_set_snapshot.as_ref();
            let ia_states = &mut self.editor.scene.input_action_editor.open_actions;
            let input_context_editor = &mut self.editor.scene.input_context_editor;
            let mc_states = &mut input_context_editor.open_contexts;
            let mc_actions = input_context_editor.available_actions.as_slice();
            let sel = &mut self.editor.scene.selection;
            let mesh_editors = &mut self.editor.scene.mesh_editors;
            let graph_editors = &mut self.editor.scene.graph_editors;
            let curve_editors = &mut self.editor.scene.curve_editors;
            let graph_registry = &self.editor.scene.node_registry;
            // A frame-local snapshot: the preferences page edits the live
            // keymap mutably in the same block, and the graph panel only
            // reads. A rebind therefore takes effect next frame, which is
            // also the behaviour you want — the chord that committed the
            // rebind must not immediately fire its new action.
            let graph_keymap_snapshot = self.editor.services.keymap.clone();
            // Editor camera position, snapshotted for the GS-3 instance picker
            // (its distance column). Render space is Y-up; the world the
            // entities live in is Z-up, so it converts on the way out.
            let graph_camera = {
                let p = rust_engine::engine::utils::coords::convert_position_yup_to_zup(
                    vp.camera.position,
                );
                [p.x, p.y, p.z]
            };
            let graph_keymap = &graph_keymap_snapshot;
            let graph_clipboard = &mut self.editor.scene.graph_clipboard;
            // 45-A P7: bind each open graph tab to a running instance of it.
            // The rule itself lives in the engine
            // (`scripting::trace::viz_for_selection` — the selected entity
            // that runs this graph); resolved once per frame here rather than
            // once per tab draw, because the selection is the same for all of
            // them. Empty in edit mode, with nothing selected, or in a build
            // without the interpreter — and empty draws nothing.
            // Where the person asking "which Duck is that" is standing.
            let graph_camera_pos = graph_camera;
            // GS-4 rides along: each tab's armed marks resolve against the
            // instance it is bound to (so the canvas can flag the ones that
            // resolve to nothing), and the debugger command it raised last
            // frame travels out to be delivered after the UI, where the world
            // is reachable mutably.
            let graph_debug_tabs: Vec<GraphDebugTab> = graph_editors
                .iter_mut()
                .map(|(k, st)| GraphDebugTab {
                    path: k.clone(),
                    bind: st.exec_bind,
                    armed: st.armed_breakpoints(),
                    request: st.debug_request.take(),
                })
                .collect();
            let graph_exec = {
                let picked: Vec<hecs::Entity> = sel.all().copied().collect();
                graph_exec_bindings(&*world, &*world_resources, &picked, &graph_debug_tabs)
            };
            graph_debug.extend(graph_debug_plan(&graph_debug_tabs, &graph_exec));
            // Every instance of every open graph, for the LIVE chip's picker
            // (GS-3). Present even when nothing is bound — "N RUNNING" is a
            // state, not an absence.
            let graph_instances = graph_instance_lists(
                &*world,
                &*world_resources,
                graph_camera_pos,
                graph_editors.keys(),
            );
            let graph_focused_tab = self.editor.ui.crusty_dock.state.focused_tab.clone();
            // The curve editor's own Edit-menu override — same rule, fewer
            // verbs (no curve clipboard, so paste/duplicate stay the scene's).
            let curve_edit_override = graph_focused_tab
                .as_deref()
                .filter(|ft| self.editor.ui.crusty_dock.tree.contains_tab(ft))
                .and_then(|ft| ft.strip_prefix("curve:"))
                .and_then(|k| curve_editors.get(k))
                .map(|st| {
                    use rust_engine::engine::editor::menu_bar_crusty::EditMenuOverride;
                    EditMenuOverride {
                        undo_label: st
                            .stack
                            .undo_description()
                            .map(|d| format!("Undo {d}"))
                            .unwrap_or_else(|| "Undo".to_string()),
                        can_undo: st.stack.can_undo(),
                        redo_label: st
                            .stack
                            .redo_description()
                            .map(|d| format!("Redo {d}"))
                            .unwrap_or_else(|| "Redo".to_string()),
                        can_redo: st.stack.can_redo(),
                        has_selection: st.has_selection(),
                        has_deletable: st.has_selection(),
                        has_clipboard: false,
                    }
                });
            // Edit-menu override when a docked graph tab has focus (P5 routing).
            let graph_edit_override = graph_focused_tab
                .as_deref()
                .filter(|ft| self.editor.ui.crusty_dock.tree.contains_tab(ft))
                .and_then(|ft| ft.strip_prefix("graph:"))
                .and_then(|k| graph_editors.get(k))
                .map(|st| {
                    use rust_engine::engine::editor::menu_bar_crusty::EditMenuOverride;
                    EditMenuOverride {
                        undo_label: st
                            .stack
                            .undo_description()
                            .map(|d| format!("Undo {d}"))
                            .unwrap_or_else(|| "Undo".to_string()),
                        can_undo: st.stack.can_undo(),
                        redo_label: st
                            .stack
                            .redo_description()
                            .map(|d| format!("Redo {d}"))
                            .unwrap_or_else(|| "Redo".to_string()),
                        can_redo: st.stack.can_redo(),
                        has_selection: !st.selection.is_empty(),
                        has_deletable: !st.selection.is_empty()
                            || st.sel_comment.is_some()
                            || st.sel_group.is_some(),
                        has_clipboard: graph_clipboard.is_some(),
                    }
                });
            let mesh_textures = &self.crusty_mesh_textures;
            let icons = &self.crusty_icons;
            let icon_registry = self.editor.services.icons.clone();
            // Per-file editor dirty dots: build the set of dirty tab ids so
            // mesh/graph tabs show the warning dot (scene tabs still route
            // through the scene registry inside `tab_titles`).
            let editor_dirty: std::collections::HashSet<String> = mesh_editors
                .iter()
                .filter(|(_, d)| d.dirty)
                .map(|(k, _)| format!("mesh:{k}"))
                .chain(
                    graph_editors
                        .iter()
                        .filter(|(_, s)| s.dirty)
                        .map(|(k, _)| format!("graph:{k}")),
                )
                .chain(
                    curve_editors
                        .iter()
                        .filter(|(_, s)| s.dirty)
                        .map(|(k, _)| format!("curve:{k}")),
                )
                .collect();
            // Both driven by the *active* runtime plugin set (39.8 §5.8/D7),
            // never by a pending manifest edit.
            let physics_inactive = !self
                .core
                .plugin_set
                .is_active(rust_engine::engine::plugins::PHYSICS_RAPIER_ID);
            let scripting_inactive = !self
                .core
                .plugin_set
                .is_active(rust_engine::engine::plugins::GRAPH_SCRIPTING_ID);
            let gameplay_disabled = !self
                .core
                .plugin_set
                .is_active(crate::plugin::GAME_CLIENT_ID);
            // Owned snapshot: the manager needs to read the plugin set while
            // the settings window holds its plugin *pages* mutably.
            let plugin_model =
                rust_engine::engine::editor::plugin_manager::PluginManagerModel::build(
                    &self.core.plugin_set,
                    &self.editor.ui.settings.project,
                );
            let plugin_panel_entries = self.core.plugin_set.panel_menu_entries();
            let plugin_panel_titles: std::collections::HashMap<String, String> =
                plugin_panel_entries.iter().cloned().collect();
            let (titles, dirty_tabs) = dock_crusty::tab_titles(
                &self.editor.ui.crusty_dock.tree,
                dock_crusty::TabTitlesCtx {
                    active_id: active_scene_id,
                    active_name: &current_scene_name,
                    active_dirty,
                    dormant: &self.editor.scene.registry.dormant,
                    extra: self.crusty_dock_drag.as_deref(),
                    editor_dirty: &editor_dirty,
                    plugin_titles: &plugin_panel_titles,
                },
            );
            let theme = self.editor.services.theme.clone();
            let has_selection = !sel.is_empty();
            let has_clipboard = !self.editor.scene.clipboard.is_empty();
            // D9: resolve the export's feature set from the live plugin
            // manifests + the edited project manifest, every frame the dialog
            // could be opened, so a toggle is reflected before the build runs.
            {
                let manifests = self.core.plugin_set.manifests();
                let entries = &self.editor.ui.settings.project.plugins;
                let features = rust_engine::engine::plugins::export_features(&manifests, entries);
                let listed = rust_engine::engine::plugins::exported_plugins(&manifests, entries);
                // …and the dirty flag alongside it: `features` above comes from
                // the *edited* config, so an unsaved toggle must block the
                // build rather than silently ship.
                let dirty = self.editor.ui.settings.project_dirty();
                let bd = &mut self.editor.play.build_dialog;
                bd.settings.features = features;
                bd.settings.exported_plugins = listed;
                bd.settings.project_dirty = dirty;
            }
            let plugin_set = &mut self.core.plugin_set;
            let crusty_dock = &mut self.editor.ui.crusty_dock;
            let dock_drag = &mut self.crusty_dock_drag;
            let tab_ctx_target = &mut self.crusty_tab_ctx;
            let pending_floats = &mut self.pending_crusty_floats;
            let build_dialog = &mut self.editor.play.build_dialog;
            let benchmark_tools = self.runtime_flags.benchmark_tools_enabled;
            let status_error_count = console.messages.counts().2;
            let project_dirty = self.editor.ui.settings.project_dirty();
            let unsaved_count = dirty_tabs.len()
                + self.editor.services.dirty.dirty_asset_count()
                + usize::from(project_dirty);
            let settings = &mut self.editor.ui.settings;
            let settings_keymap = &mut self.editor.services.keymap;
            let toasts = &mut self.editor.services.toasts;
            let dialog_stack = &mut self.editor.services.dialogs;
            let command_palette = &mut self.editor.services.command_palette;
            let command_registry = &self.editor.services.command_registry;
            let import_dialog = &mut self.editor.scene.import_dialog;
            let anim_events_dialog = &mut self.editor.scene.anim_events_dialog;
            let save_as_dialog = &mut self.editor.scene.save_as_dialog;
            let mut save_as_cancel = false;
            let mut crusty_menu_action = MenuAction::None;
            let crusty_result = self.crusty_gui.layout(|ui| {
                // Global palette hotkey — Ctrl+Shift+P opens the command palette.
                {
                    use rust_engine::engine::gui::crusty::{Key as CKey, Modifiers as CMods};
                    let input = &ui.ctx().input;
                    if input.modifiers.contains(CMods::CTRL | CMods::SHIFT)
                        && input.key_pressed(CKey::Char('P'))
                    {
                        command_palette.open();
                    }
                }
                crusty_menu_action = menu_bar_panel(
                    ui,
                    crusty_menu_bar_rect,
                    MenuBarCtx {
                        dock_state: crusty_dock,
                        command_history: &*vp_command_history,
                        edit_override: graph_edit_override
                            .clone()
                            .or_else(|| curve_edit_override.clone()),
                        play_mode: current_play_mode,
                        build_dialog,
                        console_messages: &mut console.messages,
                        show_benchmark_tools: benchmark_tools,
                        icons,
                        theme: &theme,
                        has_selection,
                        has_clipboard,
                        play_settings: &mut settings.prefs.play,
                        scene_name: &current_scene_name,
                        plugin_panels: &plugin_panel_entries,
                        gameplay_disabled,
                    },
                );
                if status_bar_panel(
                    ui,
                    crusty_status_bar_rect,
                    StatusBarCtx {
                        dock_state: crusty_dock,
                        theme: &theme,
                        error_count: status_error_count,
                        unsaved_count,
                        icons,
                    },
                ) {
                    command_palette.open();
                }

                // Dock: everything between the menu and status strips.
                let dock_rect = CRect::from_min_max(
                    CPos2::new(crusty_screen_rect.min.x, crusty_menu_bar_rect.max.y),
                    CPos2::new(crusty_screen_rect.max.x, crusty_status_bar_rect.min.y),
                );
                let ext = float_ext.take().or_else(|| {
                    dock_drag
                        .as_ref()
                        .map(|tab| dock_crusty::ghost_drag(ui, tab))
                });
                let drag_released = ext.as_ref().is_some_and(|e| e.ghost && e.released);
                let drop_pos = ext.as_ref().filter(|e| e.ghost).map(|e| e.pointer);
                let dock_hidden_tabs = dock_crusty::hidden_tabs(&crusty_dock.tree);
                let resp =
                    dock_crusty::DockArea::new(&mut crusty_dock.tree, &mut crusty_dock.state)
                        .titles(&titles)
                        .show_close_buttons(true)
                        .min_tab_width(120.0)
                        .tab_bar_height(30.0)
                        .dirty_tabs(&dirty_tabs, theme.palette.status.warning)
                        .external_drag(ext)
                        .show_in(ui, dock_rect, |ui, tab| {
                            let rect = ui.clip_rect();
                            match dock_crusty::parse_tab(tab) {
                                Some(EditorTab::Console) => console_panel(
                                    ui,
                                    rect,
                                    ConsolePanelCtx {
                                        messages: &mut console.messages,
                                        filter: &mut console.log_filter,
                                        command_system: &mut console.command_system,
                                        input: &mut console.input,
                                        world: &mut *world,
                                        show_stat_fps: &mut *show_stat_fps,
                                    },
                                ),
                                Some(EditorTab::Hierarchy) => hierarchy_panel(
                                    ui,
                                    rect,
                                    HierarchyPanelCtx {
                                        panel: hierarchy,
                                        world: &mut *world,
                                        selection: sel,
                                        play_mode: current_play_mode,
                                        icons,
                                        registry: &icon_registry,
                                        world_object: world_object_info.as_ref(),
                                    },
                                ),
                                Some(EditorTab::Inspector) => inspector_panel(
                                    ui,
                                    rect,
                                    InspectorPanelCtx {
                                        panel: inspector,
                                        world: &mut *world,
                                        selection: &*sel,
                                        play_mode: current_play_mode,
                                        asset_browser: &mut *asset_browser,
                                        icons,
                                        world_object: world_object_info.as_ref(),
                                        physics_inactive,
                                        scripting_inactive,
                                    },
                                ),
                                Some(EditorTab::AssetBrowser) => asset_browser_panel(
                                    ui,
                                    rect,
                                    AssetBrowserPanelCtx {
                                        panel: &mut *asset_browser,
                                        icons,
                                    },
                                ),
                                Some(EditorTab::Profiler) => profiler_panel(ui, rect, profiler),
                                Some(EditorTab::Viewport(id)) if id == active_scene_id => {
                                    viewport_panel(
                                        ui,
                                        rect,
                                        ViewportPanelCtx {
                                            texture: crusty_viewport_texture,
                                            viewport_size: &mut vp.size,
                                            viewport_rect: &mut vp.rect,
                                            viewport_hovered: &mut vp.hovered,
                                            settings: &mut vp.settings,
                                            gizmo: &mut vp.gizmo_handler,
                                            grid_visible: &mut vp.grid_visible,
                                            camera: &vp.camera,
                                            selected: sel.primary(),
                                            world: &mut *world,
                                            command_history: vp_command_history,
                                            play_mode: current_play_mode,
                                            icons,
                                            strip_hidden: dock_hidden_tabs.contains(tab),
                                            show_stat_fps: *show_stat_fps,
                                            fps,
                                            delta_ms,
                                            streaming: streaming_overlay,
                                            net_status: net_status.clone(),
                                        },
                                    )
                                }
                                Some(EditorTab::Viewport(_)) => {
                                    dock_crusty::placeholder_panel(ui, "Activating scene...")
                                }
                                Some(EditorTab::InputSettings) => {
                                    input_settings_panel(ui, rect, input_settings, action_set)
                                }
                                Some(EditorTab::InputActionEditor(key)) => {
                                    match ia_states.get_mut(&key) {
                                        Some(state) => input_action_panel(ui, rect, &key, state),
                                        None => dock_crusty::missing_document_panel(
                                            ui,
                                            "Input action", &key, None,
                                        ),
                                    }
                                }
                                Some(EditorTab::InputContextEditor(key)) => {
                                    match mc_states.get_mut(&key) {
                                        Some(state) => {
                                            input_context_panel(ui, rect, &key, state, mc_actions)
                                        }
                                        None => dock_crusty::missing_document_panel(
                                            ui,
                                            "Mapping context", &key, None,
                                        ),
                                    }
                                }
                                Some(EditorTab::MeshEditor(key)) => {
                                    match mesh_editors.get_mut(&key) {
                                        Some(data) => mesh_editor_panel(
                                            ui,
                                            rect,
                                            MeshEditorPanelCtx {
                                                data,
                                                texture: mesh_textures.get(&key).and_then(|e| e.id),
                                                asset_browser: &mut *asset_browser,
                                                icons,
                                                float_thumbs: None,
                                            },
                                        ),
                                        None => {
                                            dock_crusty::missing_document_panel(ui, "Mesh", &key, None)
                                        }
                                    }
                                }
                                Some(EditorTab::GraphEditor(key)) => {
                                    match graph_editors.get_mut(&key) {
                                        Some(state) => graph_editor_panel(
                                            ui,
                                            GraphEditorPanelCtx {
                                                state,
                                                registry: graph_registry,
                                                keymap: graph_keymap,
                                                clipboard: graph_clipboard,
                                                resolver: &graph_resolver_docs,
                                                curves: &graph_curve_docs,
                                                subgraph_assets: &subgraph_assets,
                                                open_subgraph: &mut graph_open_request,
                                                selection_outline: graph_sel_outline,
                                                wire_prefs: graph_prefs.wires,
                                                wire_style_request: &mut graph_style_request,
                                                zoom_min: graph_prefs.zoom_min,
                                                zoom_max: graph_prefs.zoom_max,
                                                focused: graph_focused_tab.as_deref()
                                                    == Some(tab),
                                                // Docked: keyboard editing runs
                                                // through the main menu/winit path.
                                                handle_shortcuts: false,
                                                exec: graph_exec
                                                    .iter()
                                                    .find(|(k, _)| *k == key)
                                                    .map(|(_, v)| v),
                                                exec_instances: graph_instances
                                                    .get(&key)
                                                    .map(Vec::as_slice)
                                                    .unwrap_or(&[]),
                                                exec_clear: &mut graph_clear_trace,
                                            },
                                        ),
                                        None => dock_crusty::missing_document_panel(
                                            ui,
                                            "Graph", &key, None,
                                        ),
                                    }
                                }
                                Some(EditorTab::CurveEditor(key)) => {
                                    match curve_editors.get_mut(&key) {
                                        Some(state) => {
                                            if curve_editor_panel(
                                                ui,
                                                rect,
                                                CurveEditorPanelCtx {
                                                    state,
                                                    selection_outline: graph_sel_outline,
                                                    focused: graph_focused_tab.as_deref()
                                                        == Some(tab),
                                                    // Docked: the menu/winit
                                                    // path owns the keyboard.
                                                    handle_shortcuts: false,
                                                },
                                            )
                                            .save_requested
                                            {
                                                curve_save_requests.push(key.clone());
                                            }
                                        }
                                        None => dock_crusty::missing_document_panel(
                                            ui, "Curve", &key, None,
                                        ),
                                    }
                                }
                                Some(EditorTab::Plugin(id)) => match plugin_set.panel_mut(&id) {
                                    Some(entry) => entry.panel.draw(
                                        ui,
                                        rect,
                                        &mut PluginPanelCtx {
                                            world: &mut *world,
                                            resources: &mut *world_resources,
                                            play_mode: current_play_mode,
                                        },
                                    ),
                                    // Plugin disabled, failed, or not in this
                                    // build: keep the tab, say why.
                                    None => dock_crusty::missing_document_panel(
                                        ui,
                                        "Plugin panel",
                                        &id,
                                        Some("no enabled plugin registers this panel"),
                                    ),
                                },
                                _ => dock_crusty::placeholder_panel(
                                    ui,
                                    "This panel is not yet ported to crusty-gui.",
                                ),
                            }
                        });

                // "+" new-scene button after the viewport tab strip (same
                // deferred path as the crusty menu's New Scene).
                if let Some(slot) = resp
                    .tab_bar_slots
                    .iter()
                    .find(|s| s.anchor.starts_with("viewport:"))
                {
                    if dock_crusty::new_tab_button(ui, slot) {
                        crusty_menu_action = MenuAction::NewScene;
                    }
                }

                // Eye button at the right end of every tab bar: hides that
                // leaf's strip (the corner triangle restores it).
                for slot in &resp.tab_bar_slots {
                    if dock_crusty::hide_tabs_button(ui, slot, icons) {
                        crusty_dock.tree.set_tabs_hidden(&slot.anchor, true);
                    }
                }

                // Right-click tab menu: Hide Tabs + the close family.
                let tab_menu = dock_crusty::tab_context_menu(
                    ui,
                    resp.tab_right_clicked.clone(),
                    tab_ctx_target,
                    &crusty_dock.tree,
                );
                if let Some(tab) = &tab_menu.hide_tabs {
                    crusty_dock.tree.set_tabs_hidden(tab, true);
                }
                crusty_ctx_close = tab_menu.close;

                // Tear-off: carry the tab as a cursor ghost until it re-docks;
                // dropped on no dock target → spawn an OS float window there.
                if let Some(det) = resp.detached {
                    *dock_drag = Some(det.tab);
                } else if resp.docked {
                    *dock_drag = None;
                } else if drag_released {
                    if let Some(tab) = dock_drag.take() {
                        if tab.starts_with("viewport:") {
                            // Viewports render only in the main window.
                            crusty_dock.tree.add_tab(tab);
                        } else {
                            pending_floats.push(
                                rust_engine::engine::editor::crusty_window::CrustyWindowRequest {
                                    tab,
                                    main_local: drop_pos
                                        .unwrap_or(dock_crusty::Pos2::new(120.0, 120.0)),
                                },
                            );
                        }
                    }
                }
                crusty_docked = resp.docked;
                crusty_close_tab = resp.close_requested;

                // Last so they draw above the panels (menus/tooltips live in
                // the overlay list and stay on top regardless).
                toasts_panel(ui, crusty_screen_rect, toasts, &theme);

                use rust_engine::engine::editor::{command_palette_crusty, dialogs_crusty};
                crusty_dialog_actions = dialogs_crusty::dialog_stack_panel(ui, dialog_stack);
                if let Some(action) = command_palette_crusty::command_palette_panel(
                    ui,
                    command_palette,
                    command_registry,
                ) {
                    crusty_dialog_actions.push(action);
                }
                if let Some(state) = import_dialog.as_mut() {
                    crusty_import_action = dialogs_crusty::import_dialog_panel(ui, state);
                }
                if let Some(dlg) = anim_events_dialog.as_mut() {
                    use rust_engine::engine::editor::anim_events_dialog;
                    anim_events_action = anim_events_dialog::anim_events_dialog_panel(ui, dlg);
                }
                if let Some(dlg) = save_as_dialog.as_mut() {
                    save_as_cancel = dialogs_crusty::save_as_dialog_panel(ui, dlg);
                }
                {
                    use rust_engine::engine::editor::settings_crusty;
                    settings_crusty::editor_prefs_window(ui, settings, settings_keymap);
                    settings_crusty::project_settings_window(
                        ui,
                        settings,
                        plugin_set.settings_pages_mut(),
                        &plugin_model,
                    );
                }
                if is_hovering_files {
                    dialogs_crusty::file_drop_overlay(ui, crusty_screen_rect);
                }
            });
            if save_as_cancel {
                *save_as_dialog = None;
            }
            self.crusty_menu_action = crusty_menu_action;
            crusty_result
        };

        // Subgraph node double-clicked in a docked graph → open its doc (P6).
        if let Some(style) = graph_style_request {
            self.editor.ui.settings.prefs.graph.wires.style = style;
        }
        if let Some(relative) = graph_open_request {
            self.open_graph_document(relative);
        }
        // "Clear trace" (GS-3): the taken-path tint and the pulse history are
        // one session's statement, and the recorder that owns them lives on
        // the instance — so the panel names the graph and the host does it.
        if let Some(key) = graph_clear_trace.take() {
            self.clear_graph_traces(&key);
        }
        // GS-4: the bound instance gets this tab's breakpoints; every other
        // instance of the same graph gets an empty set, every frame.
        if !graph_debug.is_empty() {
            self.apply_graph_debug(&graph_debug);
        }
        for key in std::mem::take(&mut curve_save_requests) {
            self.save_curve_editor(&key);
        }

        // Commit (or veto) a crusty dock tab-close request.
        if let Some(tab) = crusty_close_tab.take() {
            self.handle_crusty_tab_close(&tab);
        }
        for tab in std::mem::take(&mut crusty_ctx_close) {
            self.handle_crusty_tab_close(&tab);
        }

        // Resolve a cross-window drag from a float window: docked into the
        // main tree → drop the tab from the float (an emptied window closes
        // next frame); released anywhere else → the window stays put.
        if let Some((id, released)) = crusty_float_drag {
            if let Some(fw) = self.crusty_floats.get_mut(&id) {
                if crusty_docked {
                    if let Some(drag) = fw.drag_out.take() {
                        fw.tree.close_tab(&drag.tab);
                    }
                } else if released {
                    fw.drag_out = None;
                }
            }
        }

        self.handle_import_dialog_action(crusty_import_action);
        self.handle_anim_events_action(anim_events_action);
        for action in crusty_dialog_actions {
            self.handle_editor_action(action);
        }

        self.handle_frame_input(&crusty_result);

        // Live-apply + debounced autosave of editor preferences (M10 P7).
        self.apply_editor_prefs(false);
        self.editor.ui.settings.flush_prefs();
        // Rebinding autosaves on the same debounce as every other preference.
        let keymap = self.editor.services.keymap.clone();
        self.editor.ui.settings.flush_keymap(&keymap);
        self.flush_graph_ui_state();
        self.persist_vsync_change();

        // Attach crusty layout data to frame packet and send to render thread
        let mut packet = packet;
        {
            packet.crusty_paint = Some(crusty_result.paint);
            packet.crusty_texture_uploads =
                self.editor.scene.asset_browser.thumbnails.poll_crusty();

            // Mesh-preview render targets: register new ones with the render
            // thread's crusty renderer, re-point ids after a resize recreated
            // the target, and drop entries for closed editors.
            let mesh_editors = &self.editor.scene.mesh_editors;
            self.crusty_mesh_textures.retain(|k, entry| {
                if mesh_editors.contains_key(k) {
                    return true;
                }
                if let Some(id) = entry.id {
                    packet.crusty_native_removals.push(id);
                }
                false
            });
            for (key, data) in mesh_editors.iter() {
                let Some(ref preview) = data.preview else {
                    continue;
                };
                if preview.mesh_indices.is_empty() {
                    continue;
                }
                // Only (re)bind a view in a frame whose packet also carries
                // this key's preview render CB: a freshly created/resized
                // target is in Undefined layout, and the render thread
                // sampling it without the CB chained in the same submission
                // is a validation panic (e.g. while the CB is routed to a
                // float window during a tab tear-off).
                let has_cb = self.crusty_docked_preview_cbs.iter().any(|(k, _)| k == key);
                if !has_cb {
                    continue;
                }
                let view = preview.texture.image_view();
                match self.crusty_mesh_textures.entry(key.clone()) {
                    std::collections::hash_map::Entry::Vacant(e) => {
                        packet
                            .crusty_native_registrations
                            .push((format!("mesh_preview:{key}"), view.clone()));
                        e.insert(CrustyMeshTexture { id: None, view });
                    }
                    std::collections::hash_map::Entry::Occupied(mut e) => {
                        let entry = e.get_mut();
                        if !std::sync::Arc::ptr_eq(&entry.view, &view) {
                            if let Some(id) = entry.id {
                                packet.crusty_native_updates.push((id, view.clone()));
                                entry.view = view;
                            }
                        }
                    }
                }
            }
            packet.crusty_preview_cbs = std::mem::take(&mut self.crusty_docked_preview_cbs)
                .into_iter()
                .map(|(_, cb)| cb)
                .collect();
        }

        // CRITICAL: re-sync the viewport dimensions and view_projection with the size
        // the UI layout just produced (it ran *after* the initial packet build above
        // and updated `viewport.size` to the current available rect). Without this,
        // the 3D scene is rendered at the previous frame's size while the UI paints
        // the texture into the new-size rect, producing visible scaling artifacts
        // on every resize step (banding/blocky stripes on meshes and shadows during
        // fast drag).
        let (vp_w_now, vp_h_now) = self.editor.viewport.size;
        if vp_w_now > 0 && vp_h_now > 0 {
            self.editor
                .viewport
                .camera
                .set_viewport_size(vp_w_now as f32, vp_h_now as f32);
            self.core
                .renderer
                .camera_3d
                .set_viewport_size(vp_w_now as f32, vp_h_now as f32);
            packet.viewport_dimensions = Some([vp_w_now, vp_h_now]);
            packet.view_proj = self.editor.viewport.camera.view_projection_matrix();
        }

        if let Some(ref rt) = self.core.render_thread {
            if let Err(e) = rt.send(packet) {
                log::error!("editor: failed to send frame packet: {}", e);
            }
        }

        Ok(())
    }

    /// Build a fresh `GameWorld` populated with the standard scene-local resources.
    /// Shared globals (asset_manager Arc, input action set) are sourced from the active world.
    fn fresh_scene_world(&self) -> GameWorld {
        let mut world = GameWorld::new();
        world
            .resources_mut()
            .insert(self.core.asset_manager.clone());
        world.resources_mut().insert(PhysicsWorld::new());
        world.resources_mut().insert(CollisionWorld::new());
        world.resources_mut().insert(TransformCache::new());
        world.resources_mut().insert(AudioReloadQueue::new());
        if let Some(audio_engine) = AudioEngine::new() {
            world.resources_mut().insert(audio_engine);
        }
        world.resources_mut().insert(InputManager::new());
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
        world.resources_mut().insert(subsystem);
        world.resources_mut().insert(ActionState::new());
        world
            .resources_mut()
            .insert(rust_engine::engine::ecs::events::Events::<InputEvent>::new());
        if let Some(gamepad_state) = GamepadState::try_new() {
            world.resources_mut().insert(gamepad_state);
        }
        world
    }

    /// Move the currently-active scene's state into a [`DormantScene`] record
    /// (preserving its id) and return it.
    fn park_active_scene(&mut self) -> DormantScene {
        // Streamed content is runtime state, not scene content: tear it down
        // before the world is swapped out so nothing leaks into the parked
        // world (mesh refcounts, StreamedCell entities).
        self.flush_streaming();
        self.core.world_streamer.clear();
        let id = self.editor.scene.registry.active_id;
        let mut parked_world = GameWorld::new(); // placeholder; will be swapped out
        std::mem::swap(&mut parked_world, &mut self.core.game_world);
        let mut parked_selection = Selection::new();
        std::mem::swap(&mut parked_selection, &mut self.editor.scene.selection);
        let mut parked_history = CommandHistory::new(100);
        std::mem::swap(&mut parked_history, &mut self.editor.scene.command_history);
        let parked_hierarchy = self.editor.scene.hierarchy_panel.root_order().to_vec();
        DormantScene {
            id,
            relative_path: std::mem::take(&mut self.editor.scene.current_scene_relative),
            display_name: std::mem::take(&mut self.editor.scene.current_scene_name),
            dirty: std::mem::replace(&mut self.editor.scene.active_dirty, false),
            world: parked_world,
            selection: parked_selection,
            command_history: parked_history,
            hierarchy_root_order: parked_hierarchy,
        }
    }

    /// Restore a [`DormantScene`] into the live active slots, replacing the current
    /// active scene contents (which should have been parked first).
    fn restore_dormant_scene(&mut self, mut dormant: DormantScene) {
        self.editor.scene.registry.active_id = dormant.id;
        self.editor.scene.current_scene_relative = std::mem::take(&mut dormant.relative_path);
        self.editor.scene.current_scene_name = std::mem::take(&mut dormant.display_name);
        self.editor.scene.active_dirty = dormant.dirty;
        std::mem::swap(&mut dormant.world, &mut self.core.game_world);
        std::mem::swap(&mut dormant.selection, &mut self.editor.scene.selection);
        std::mem::swap(
            &mut dormant.command_history,
            &mut self.editor.scene.command_history,
        );
        self.editor
            .scene
            .hierarchy_panel
            .set_root_order(dormant.hierarchy_root_order);

        // Re-arm streaming (streamed content was flushed at park time).
        // Manifest-less scenes keep their parked CollisionWorld as-is.
        let relative = self.editor.scene.current_scene_relative.clone();
        if !relative.is_empty() {
            let report = self.core.world_streamer.load_for_scene(&relative);
            if report.disabled.is_none() {
                if let Some(collision) = self.core.game_world.resource_mut::<CollisionWorld>() {
                    collision.begin_streaming(&relative);
                }
            }
        }
    }

    /// Create a new empty scene as a fresh tab and make it active.
    /// Returns the new scene id (and pushes a viewport tab into the dock).
    fn create_new_scene(&mut self) -> Option<SceneId> {
        if self.play_mode() != PlayMode::Edit {
            self.editor.console.messages.push(LogMessage::warning(
                "Stop play mode before creating a new scene".to_string(),
            ));
            return None;
        }

        let new_id = self.editor.scene.registry.allocate_id();

        // Park the current active state into the registry.
        let parked = self.park_active_scene();
        self.editor.scene.registry.park(parked);

        // Place a fresh world and reset editor state for the new scene.
        self.core.game_world = self.fresh_scene_world();
        self.editor.scene.registry.active_id = new_id;
        self.editor.scene.current_scene_relative = String::new();
        self.editor.scene.current_scene_name = "Untitled Scene".to_string();
        self.editor.scene.active_dirty = false;
        self.editor.scene.selection.clear();
        self.editor.scene.command_history.clear();
        self.editor.scene.hierarchy_panel.set_root_order(Vec::new());

        // Push a new viewport tab into the existing viewport leaf and focus it.
        self.editor.ui.crusty_dock.open_viewport_tab(new_id);

        self.editor
            .console
            .messages
            .push(LogMessage::info("New scene created".to_string()));
        Some(new_id)
    }

    /// Switch the active scene to `target_id`. No-op if it is already active.
    fn switch_to_scene(&mut self, target_id: SceneId) {
        if self.editor.scene.registry.active_id == target_id {
            return;
        }
        let Some(target) = self.editor.scene.registry.take_dormant(target_id) else {
            log::warn!(
                "switch_to_scene: target {} not found in dormant",
                target_id.0
            );
            return;
        };
        let parked = self.park_active_scene();
        self.editor.scene.registry.park(parked);
        self.restore_dormant_scene(target);
    }

    /// Close (and drop) a scene tab. If it's the active tab and other tabs exist,
    /// switches to one of the others first. Refuses to close the last scene tab.
    ///
    /// `on_close` already removed the dock tab by the time we get here, so the
    /// remaining-viewport-tabs count of 0 means we just closed the only one.
    fn close_scene_tab(&mut self, id: SceneId) {
        let remaining_viewport_tabs: usize = {
            let mut tabs = Vec::new();
            self.editor.ui.crusty_dock.tree.collect_tabs(&mut tabs);
            tabs.iter().filter(|t| t.starts_with("viewport:")).count()
        };

        if remaining_viewport_tabs == 0 {
            self.editor.console.messages.push(LogMessage::warning(
                "Cannot close the last scene tab".to_string(),
            ));
            // Re-add the tab since the dock has already removed it.
            self.editor.ui.crusty_dock.open_viewport_tab(id);
            return;
        }

        if id == self.editor.scene.registry.active_id {
            // Pick any other dormant id as the new active.
            let next_active = self.editor.scene.registry.dormant.first().map(|d| d.id);
            if let Some(next_id) = next_active {
                self.switch_to_scene(next_id);
            }
        }

        // Now `id` should be in dormant; drop it.
        self.editor.scene.registry.drop_dormant(id);
    }

    /// Handle a crusty dock ×-close click. The tree is not mutated yet —
    /// this is the veto point: the last scene tab is refused, anything else
    /// is committed via `close_tab`.
    #[cfg(feature = "editor")]
    fn handle_crusty_tab_close(&mut self, tab: &str) {
        use rust_engine::engine::editor::dock_crusty;
        if let Some(EditorTab::Viewport(id)) = dock_crusty::parse_tab(tab) {
            let mut all = Vec::new();
            self.editor.ui.crusty_dock.tree.collect_tabs(&mut all);
            if all.iter().filter(|t| t.starts_with("viewport:")).count() <= 1 {
                self.editor.console.messages.push(LogMessage::warning(
                    "Cannot close the last scene tab".to_string(),
                ));
                return;
            }
            self.editor.ui.crusty_dock.tree.close_tab(tab);
            if id == self.editor.scene.registry.active_id {
                if let Some(next_id) = self.editor.scene.registry.dormant.first().map(|d| d.id) {
                    self.switch_to_scene(next_id);
                }
            }
            self.editor.scene.registry.drop_dormant(id);
        } else if let Some(key) = tab.strip_prefix("graph:") {
            // Dirty graph → veto the close and confirm save/discard first.
            if self
                .editor
                .scene
                .graph_editors
                .get(key)
                .is_some_and(|s| s.dirty)
            {
                let key = key.to_string();
                let msg = format!("Save changes to '{key}' before closing?");
                self.editor.services.dialogs.save_discard_cancel(
                    format!("graph_close:{key}"),
                    "Unsaved Graph",
                    msg,
                    EditorAction::SaveAndCloseEditor {
                        kind: SecondaryWindowKind::Graph,
                        key: key.clone(),
                    },
                    EditorAction::DiscardAndCloseEditor {
                        kind: SecondaryWindowKind::Graph,
                        key,
                    },
                );
                return;
            }
            self.close_graph_tab(key);
        } else if let Some(key) = tab.strip_prefix("curve:") {
            // Dirty curve → same veto-and-confirm as a dirty graph.
            if self
                .editor
                .scene
                .curve_editors
                .get(key)
                .is_some_and(|s| s.dirty)
            {
                let key = key.to_string();
                let msg = format!("Save changes to '{key}' before closing?");
                self.editor.services.dialogs.save_discard_cancel(
                    format!("curve_close:{key}"),
                    "Unsaved Curve",
                    msg,
                    EditorAction::SaveAndCloseEditor {
                        kind: SecondaryWindowKind::Curve,
                        key: key.clone(),
                    },
                    EditorAction::DiscardAndCloseEditor {
                        kind: SecondaryWindowKind::Curve,
                        key,
                    },
                );
                return;
            }
            self.close_curve_tab(key);
        } else {
            self.editor.ui.crusty_dock.tree.close_tab(tab);
            if let Some(key) = tab.strip_prefix("mesh:") {
                // Closing the tab closes the editor (`open = false` is culled
                // by the per-frame retain in run_frame, allowing reopen).
                if let Some(data) = self.editor.scene.mesh_editors.get_mut(key) {
                    data.open = false;
                }
            }
        }
    }

    /// Content-relative key of the graph tab that currently has focus in the
    /// *main* dock, if any. Returns `None` when the focused tab isn't a graph,
    /// or the graph is torn off into a float window (that window owns its
    /// keyboard editing). Drives edit-action focus routing (Task 40 P5).
    #[cfg(feature = "editor")]
    fn active_graph_key(&self) -> Option<String> {
        let ft = self.editor.ui.crusty_dock.state.focused_tab.clone()?;
        if !self.editor.ui.crusty_dock.tree.contains_tab(&ft) {
            return None;
        }
        let key = ft.strip_prefix("graph:")?.to_string();
        self.editor
            .scene
            .graph_editors
            .contains_key(&key)
            .then_some(key)
    }

    /// Apply a document edit action to the graph editor `key`.
    #[cfg(feature = "editor")]
    fn graph_edit(&mut self, key: &str, action: EditorAction) {
        let scene = &mut self.editor.scene;
        let Some(st) = scene.graph_editors.get_mut(key) else {
            return;
        };
        let reg = &scene.node_registry;
        let clip = &mut scene.graph_clipboard;
        // Never act across a half-finished gesture: undo/redo/save would
        // otherwise skip an untracked mutation, or mark a save cursor over
        // content the file does not have. `undo`/`redo` abandon a live
        // gesture themselves; everything else here wants it committed.
        if !matches!(action, EditorAction::Undo | EditorAction::Redo)
            && st.gesture_in_flight()
        {
            st.flush_prop_edit(reg);
            st.cancel_interactions();
        }
        match action {
            EditorAction::Undo => st.undo(reg),
            EditorAction::Redo => st.redo(reg),
            EditorAction::Delete => st.delete_selection(reg),
            EditorAction::Copy => st.copy_selection(clip),
            EditorAction::Cut => {
                st.copy_selection(clip);
                st.delete_selection(reg);
            }
            // Menu-driven paste has no cursor to land on, so it falls back
            // to the fragment's own offset.
            EditorAction::Paste => st.paste_clipboard(clip, None, reg),
            EditorAction::Duplicate => st.duplicate_selection(reg),
            _ => {}
        }
    }

    /// Save the graph editor `key` to disk, reporting failures to the console.
    #[cfg(feature = "editor")]
    fn save_graph_editor(&mut self, key: &str) {
        // A save is also a natural point to remember where the author is
        // looking, and costs nothing next to the file write.
        self.persist_graph_ui_state(key);
        if let Some(st) = self.editor.scene.graph_editors.get_mut(key) {
            // A save cursor must describe committed content, so any live
            // gesture is settled before the write.
            if st.gesture_in_flight() {
                let reg = &self.editor.scene.node_registry;
                st.flush_prop_edit(reg);
                st.cancel_interactions();
            }
            let abs = std::path::Path::new("content").join(&st.path);
            if let Err(e) = st.save(&abs) {
                self.editor
                    .console
                    .messages
                    .push(LogMessage::error(format!("Failed to save graph '{key}': {e}")));
                return;
            }
        } else {
            return;
        }
        // The write is what makes the cached plan stale, so invalidation
        // belongs *here* and not in the watcher handler — whose echo guard
        // returns before it precisely because this save is not an external
        // edit. Missing this meant play mode kept executing the plan the file
        // no longer contains, across restarts, for the rest of the session.
        //
        // Same shape as `save_curve_state` (P8b): write, then invalidate.
        #[cfg(feature = "graph-scripting")]
        if let Some(cache) = self
            .core
            .game_world
            .resources_mut()
            .get_mut::<rust_engine::engine::scripting::GraphPlanCache>()
        {
            cache.invalidate(key);
        }
    }

    /// Close a graph tab everywhere (main dock + any float window) and drop
    /// its document.
    #[cfg(feature = "editor")]
    fn close_graph_tab(&mut self, key: &str) {
        let tab = format!("graph:{key}");
        self.editor.ui.crusty_dock.tree.close_tab(&tab);
        for fw in self.crusty_floats.values_mut() {
            fw.tree.close_tab(&tab);
        }
        // Persist the view and bookmarks *before* the state goes: closing a
        // tab after only panning would otherwise lose both, since the prefs
        // autosave that normally carries them never fired.
        self.persist_graph_ui_state(key);
        self.editor.scene.graph_editors.remove(key);
    }

    /// Write one graph's view + bookmarks to the user-local sidecar.
    #[cfg(feature = "editor")]
    fn persist_graph_ui_state(&mut self, key: &str) {
        let Some(st) = self.editor.scene.graph_editors.get(key) else {
            return;
        };
        let root = std::path::Path::new("content");
        let mut store = GraphStateStore::load(root);
        store.store(
            key,
            st.view,
            &st.bookmarks,
            stored_watches(st),
            stored_breakpoints(st),
        );
        if let Err(e) = store.save() {
            eprintln!("Warning: failed to save graph UI state: {e}");
        }
    }

    /// Open the documents behind per-file editor tabs that a restored layout
    /// (or an adopted float window) put on screen without them.
    ///
    /// Every open path here is CPU-only — graph and input editors are plain
    /// file loads, and a mesh editor's GPU preview is built later from its
    /// `preview_dirty` flag — so this needs no render-thread gate and can run
    /// on the first frame. Runs only while `pending_hydration` is set, and
    /// clears the flag as soon as a scan finds nothing missing, so the steady
    /// state costs one boolean test per frame.
    #[cfg(feature = "editor")]
    fn hydrate_restored_tabs(&mut self) {
        use rust_engine::engine::editor::dock_crusty::{collect_tabs, parse_tab};

        if !self.pending_hydration {
            return;
        }
        let mut ids: Vec<String> = Vec::new();
        collect_tabs(&self.editor.ui.crusty_dock.tree, &mut ids);
        for fw in self.crusty_floats.values() {
            collect_tabs(&fw.tree, &mut ids);
        }

        let mut hydrated = 0usize;
        let mut still_missing = false;
        for id in ids {
            if self.hydration_failed.contains(&id) {
                continue;
            }
            let Some(tab) = parse_tab(&id) else { continue };
            let outcome = match &tab {
                EditorTab::GraphEditor(key) => {
                    if self.editor.scene.graph_editors.contains_key(key) {
                        continue;
                    }
                    Some(("Graph", key.clone(), self.hydrate_graph(key)))
                }
                EditorTab::CurveEditor(key) => {
                    if self.editor.scene.curve_editors.contains_key(key) {
                        continue;
                    }
                    Some(("Curve", key.clone(), self.hydrate_curve(key)))
                }
                EditorTab::MeshEditor(key) => {
                    if self.editor.scene.mesh_editors.contains_key(key) {
                        continue;
                    }
                    let r = self.open_mesh_document(key);
                    Some(("Mesh", key.clone(), r))
                }
                EditorTab::InputActionEditor(key) => {
                    if self
                        .editor
                        .scene
                        .input_action_editor
                        .open_actions
                        .contains_key(key)
                    {
                        continue;
                    }
                    Some(("Input action", key.clone(), self.hydrate_input_action(key)))
                }
                EditorTab::InputContextEditor(key) => {
                    if self
                        .editor
                        .scene
                        .input_context_editor
                        .open_contexts
                        .contains_key(key)
                    {
                        continue;
                    }
                    Some(("Mapping context", key.clone(), self.hydrate_input_context(key)))
                }
                _ => None,
            };
            let Some((what, key, result)) = outcome else {
                continue;
            };
            match result {
                Ok(()) => {
                    hydrated += 1;
                }
                Err(e) => {
                    // One message, then never again: a dead tab must not
                    // reopen the same error on every frame.
                    self.hydration_failed.insert(id);
                    self.editor.console.messages.push(LogMessage::error(format!(
                        "{what} not loaded — {key} ({e})"
                    )));
                    still_missing = true;
                }
            }
        }
        if hydrated > 0 {
            // `println!`, not `log::` — no logger is installed in this binary,
            // and a silent hydration is exactly the failure this pass exists
            // to make visible.
            println!("editor: hydrated {hydrated} restored editor tab(s)");
        }
        // Nothing left to do until something adds tabs again. Failures are
        // recorded, so they do not count as outstanding work.
        let _ = still_missing;
        self.pending_hydration = false;
    }

    /// Load a graph document for a restored tab, without touching the dock
    /// (the tab is already there). Keeps the sidecar view/bookmark restore.
    #[cfg(feature = "editor")]
    fn hydrate_graph(&mut self, key: &str) -> Result<(), String> {
        let abs = std::path::Path::new("content").join(key);
        if !abs.exists() {
            return Err("file missing".to_string());
        }
        let mut state = rust_engine::engine::editor::graph_editor::GraphEditorState::open(
            &abs,
            key,
            &self.editor.scene.node_registry,
        )?;
        restore_graph_ui_state(&mut state, key);
        self.editor.scene.graph_editors.insert(key.to_string(), state);
        Ok(())
    }

    /// Load a `.curve` for a restored tab, without touching the dock. A file
    /// that has gone missing fails loudly here — the Task 40 lesson: a
    /// restored tab must degrade visibly, not into a blank document that
    /// looks like data loss.
    #[cfg(feature = "editor")]
    fn hydrate_curve(&mut self, key: &str) -> Result<(), String> {
        let abs = std::path::Path::new("content").join(key);
        if !abs.exists() {
            return Err("file missing".to_string());
        }
        let state =
            rust_engine::engine::editor::curve_editor::CurveEditorState::open(&abs, key)?;
        self.editor.scene.curve_editors.insert(key.to_string(), state);
        Ok(())
    }

    /// Input-action and mapping-context keys are absolute-ish path strings,
    /// not content-relative — `InputActionEditor::open` keys on the path it
    /// was handed. Their loaders fall back to an empty definition rather than
    /// failing, so a missing file is checked here instead: hydrating a tab
    /// into a blank document would look like data loss.
    #[cfg(feature = "editor")]
    fn hydrate_input_action(&mut self, key: &str) -> Result<(), String> {
        let path = std::path::PathBuf::from(key);
        if !path.exists() {
            return Err("file missing".to_string());
        }
        self.editor.scene.input_action_editor.open(path);
        Ok(())
    }

    #[cfg(feature = "editor")]
    fn hydrate_input_context(&mut self, key: &str) -> Result<(), String> {
        let path = std::path::PathBuf::from(key);
        if !path.exists() {
            return Err("file missing".to_string());
        }
        self.editor
            .scene
            .input_context_editor
            .refresh_action_names(std::path::Path::new("content"));
        self.editor.scene.input_context_editor.open(path);
        Ok(())
    }

    /// Open a graph document by content-relative key: focus it if already
    /// open (unless it lives in a float window), else load it from the content
    /// root and open a tab. Shared by the asset browser and subgraph
    /// double-click navigation (P6).
    #[cfg(feature = "editor")]
    fn open_graph_document(&mut self, relative: String) {
        // An explicit open is the user saying "try again" — drop any failure
        // recorded by the hydration pass.
        self.hydration_failed.remove(&format!("graph:{relative}"));
        if !self.editor.scene.graph_editors.contains_key(&relative) {
            let abs = std::path::Path::new("content").join(&relative);
            match rust_engine::engine::editor::graph_editor::GraphEditorState::open(
                &abs,
                &relative,
                &self.editor.scene.node_registry,
            ) {
                Ok(mut state) => {
                    restore_graph_ui_state(&mut state, &relative);
                    self.editor.scene.graph_editors.insert(relative.clone(), state);
                    self.open_graph_as_tab(relative);
                }
                Err(e) => {
                    self.editor
                        .console
                        .messages
                        .push(LogMessage::error(format!("Failed to open graph '{relative}': {e}")));
                }
            }
        } else {
            let in_float = self.crusty_float_hosts_tab(&format!("graph:{relative}"));
            if !in_float {
                self.open_graph_as_tab(relative);
            }
        }
    }

    /// Open a `.curve` by content-relative key: focus it if already open
    /// (unless it lives in a float window), else load it and open a tab.
    #[cfg(feature = "editor")]
    fn open_curve_document(&mut self, relative: String) {
        self.hydration_failed.remove(&format!("curve:{relative}"));
        if !self.editor.scene.curve_editors.contains_key(&relative) {
            match self.hydrate_curve(&relative) {
                Ok(()) => self.open_curve_as_tab(relative),
                Err(e) => self.editor.console.messages.push(LogMessage::error(format!(
                    "Failed to open curve '{relative}': {e}"
                ))),
            }
        } else if !self.crusty_float_hosts_tab(&format!("curve:{relative}")) {
            self.open_curve_as_tab(relative);
        }
    }

    /// Content-relative key of the focused *main-dock* curve tab, if any. The
    /// `active_graph_key` rule, verbatim: a tab torn off into a float window
    /// owns its own keyboard, so it does not answer here.
    #[cfg(feature = "editor")]
    fn active_curve_key(&self) -> Option<String> {
        let ft = self.editor.ui.crusty_dock.state.focused_tab.clone()?;
        if !self.editor.ui.crusty_dock.tree.contains_tab(&ft) {
            return None;
        }
        let key = ft.strip_prefix("curve:")?.to_string();
        self.editor
            .scene
            .curve_editors
            .contains_key(&key)
            .then_some(key)
    }

    /// Apply a document edit action to the curve editor `key`.
    #[cfg(feature = "editor")]
    fn curve_edit(&mut self, key: &str, action: EditorAction) {
        let Some(st) = self.editor.scene.curve_editors.get_mut(key) else {
            return;
        };
        match action {
            EditorAction::Undo => st.undo(),
            EditorAction::Redo => st.redo(),
            EditorAction::Delete => {
                // A half-finished drag would otherwise leave the stack
                // describing a key that is no longer where it was recorded.
                st.cancel_gestures();
                st.delete_selection();
            }
            _ => {}
        }
    }

    /// Save the curve editor `key` (docked path). See [`save_curve_state`].
    #[cfg(feature = "editor")]
    fn save_curve_editor(&mut self, key: &str) {
        save_curve_state(
            &mut self.editor.scene.curve_editors,
            &mut self.editor.console,
            &mut self.core.game_world,
            key,
        );
    }

    /// Close a curve tab everywhere (main dock + any float window) and drop
    /// its document.
    #[cfg(feature = "editor")]
    fn close_curve_tab(&mut self, key: &str) {
        let tab = format!("curve:{key}");
        self.editor.ui.crusty_dock.tree.close_tab(&tab);
        for fw in self.crusty_floats.values_mut() {
            fw.tree.close_tab(&tab);
        }
        self.editor.scene.curve_editors.remove(key);
    }

    /// Rebuild the subgraph resolver from open docs + disk and refresh every
    /// open graph's cross-asset (`ref_errors`) validation. Returns the
    /// resolver doc map so callers can hand it to the canvas panels. Runs each
    /// frame (cheap for a handful of open graphs) and after a subgraph reload.
    #[cfg(feature = "editor")]
    fn revalidate_graph_refs(&mut self) -> GraphResolverDocs {
        use rust_engine::engine::editor::graph_editor::{build_curve_docs, build_resolver_docs};
        use rust_engine::engine::node_graph::{validate_curves, validate_refs, DocDescriptors};
        let root = std::path::Path::new("content");
        let scene = &mut self.editor.scene;
        let docs = build_resolver_docs(
            scene.graph_editors.iter().map(|(k, s)| (k.as_str(), &s.doc)),
            root,
        );
        // 45-A P8b: the `.curve` half of the same question. Open curve tabs
        // win over disk, so a track added in the curve editor grows the
        // Timeline's pin immediately — the rule subgraphs already follow.
        let curves = build_curve_docs(
            scene.curve_editors.iter().map(|(k, s)| (k.as_str(), &s.doc)),
            &docs,
            root,
        );
        for st in scene.graph_editors.values_mut() {
            st.ref_errors = validate_refs(&st.doc, &st.path, &scene.node_registry, &docs);
            // A dangling `.curve` is cross-asset too, so it belongs to this
            // pass and not to the doc-local `errors` recomputed on every
            // keystroke. Same rule the compiler applies, shared code.
            st.ref_errors.extend(validate_curves(
                &DocDescriptors::new(&st.doc, &scene.node_registry).with_curves(&curves),
            ));
        }
        GraphResolverDocs { docs, curves }
    }

    /// Hot-reload handler for a `.graph`/`.subgraph` write (P6): normalize the
    /// watcher path, suppress our own save echo, reload a clean open doc (warn
    /// if dirty), then refresh every host that references the changed subgraph.
    #[cfg(feature = "editor")]
    fn on_graph_changed(&mut self, abs_path: &str) {
        use rust_engine::engine::editor::graph_editor::GraphEditorState;
        use rust_engine::engine::node_graph::referencing_hosts;
        let key = asset_source::to_content_relative(abs_path);

        // A `.animgraph` write (Task 41). No editor opens these yet — a save
        // *is* an external edit — so the whole response is dropping the
        // compiled plan: the generation bump restarts live machines against
        // the new document on their next tick, and a stale plan never runs.
        if key.ends_with(".animgraph") {
            if let Some(cache) = self
                .core
                .game_world
                .resources_mut()
                .get_mut::<rust_engine::engine::animation::graph::AnimGraphPlanCache>()
            {
                cache.invalidate(&key);
            }
            self.editor
                .console
                .messages
                .push(LogMessage::info(format!("Animation graph reloaded: {key}")));
            return;
        }

        // Suppress the watcher echo of our own just-completed save: consume
        // exactly one event per save (clear the stamp), so a genuine external
        // write that arrives later — even within the same second — still
        // reloads instead of being swallowed by a time window.
        let own_echo = self
            .editor
            .scene
            .graph_editors
            .get(&key)
            .and_then(|s| s.last_saved_at)
            .is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(1));
        if own_echo {
            if let Some(st) = self.editor.scene.graph_editors.get_mut(&key) {
                st.last_saved_at = None;
            }
            return;
        }

        // Drop the compiled plan (Task 45-A P5). Deliberately *after* the
        // echo check: our own save produced the document the cache already
        // holds, so re-compiling it would be work for nothing.
        //
        // Invalidation also bumps a generation counter, so any *running*
        // instance restarts on its next tick and re-fires BeginPlay. Editing a
        // graph while playing is a stated non-goal (D9); restarting is the
        // simplest behavior that is never subtly wrong.
        #[cfg(feature = "graph-scripting")]
        if let Some(cache) = self
            .core
            .game_world
            .resources_mut()
            .get_mut::<rust_engine::engine::scripting::GraphPlanCache>()
        {
            cache.invalidate(&key);
        }

        // Reload the changed doc if it's open and clean; warn if dirty.
        if let Some(st) = self.editor.scene.graph_editors.get_mut(&key) {
            if st.dirty {
                self.editor.console.messages.push(LogMessage::warning(format!(
                    "Graph '{key}' changed on disk; keeping your unsaved edits"
                )));
            } else {
                let abs = std::path::Path::new("content").join(&key);
                match GraphEditorState::open(&abs, &key, &self.editor.scene.node_registry) {
                    Ok(mut fresh) => {
                        // Preserve the view and any selection whose ids survive.
                        fresh.view = st.view;
                        fresh.selection = st
                            .selection
                            .iter()
                            .copied()
                            .filter(|id| fresh.doc.node(*id).is_some())
                            .collect();
                        *st = fresh;
                        self.editor
                            .console
                            .messages
                            .push(LogMessage::info(format!("Graph reloaded: {key}")));
                    }
                    Err(e) => self.editor.console.messages.push(LogMessage::error(format!(
                        "Failed to reload graph '{key}': {e}"
                    ))),
                }
            }
        }

        // Refresh hosts referencing the changed subgraph (derived pins + errors).
        let hosts = referencing_hosts(
            self.editor
                .scene
                .graph_editors
                .iter()
                .map(|(k, s)| (k.as_str(), &s.doc)),
            &key,
        );
        let _ = self.revalidate_graph_refs();
        if !hosts.is_empty() {
            self.editor.console.messages.push(LogMessage::info(format!(
                "Subgraph '{key}' changed; refreshed {} host graph(s)",
                hosts.len()
            )));
        }
    }

    /// Hot-reload handler for a `.curve` write (45-A P8b): the `.graph`
    /// handler's shape, one asset kind over.
    ///
    /// Editing a curve changes what a Timeline's pins *are*, so the plan cache
    /// goes wholesale — the same reasoning `save_curve_state` gives. The
    /// editor's own resolver needs nothing: it is rebuilt from disk plus open
    /// tabs every frame.
    #[cfg(feature = "editor")]
    fn on_curve_changed(&mut self, abs_path: &str) {
        use rust_engine::engine::editor::curve_editor::CurveEditorState;
        let key = asset_source::to_content_relative(abs_path);

        // Suppress the watcher echo of our own save: one event per save,
        // consumed by clearing the stamp, so a genuine external write that
        // lands later still reloads.
        let own_echo = self
            .editor
            .scene
            .curve_editors
            .get(&key)
            .and_then(|s| s.last_saved_at)
            .is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(1));
        if own_echo {
            if let Some(st) = self.editor.scene.curve_editors.get_mut(&key) {
                st.last_saved_at = None;
            }
            return;
        }

        #[cfg(feature = "graph-scripting")]
        {
            let res = self.core.game_world.resources_mut();
            if let Some(curves) = res.get_mut::<rust_engine::engine::scripting::CurveCache>() {
                curves.invalidate(&key);
            }
            if let Some(plans) = res.get_mut::<rust_engine::engine::scripting::GraphPlanCache>()
            {
                plans.invalidate_all();
            }
        }

        // Reload the changed doc if it is open and clean; warn if dirty. The
        // view is preserved — a reload must not throw away where you were
        // looking.
        if let Some(st) = self.editor.scene.curve_editors.get_mut(&key) {
            if st.dirty {
                self.editor.console.messages.push(LogMessage::warning(format!(
                    "Curve '{key}' changed on disk; keeping your unsaved edits"
                )));
                return;
            }
            let abs = std::path::Path::new("content").join(&key);
            match CurveEditorState::open(&abs, &key) {
                Ok(mut fresh) => {
                    fresh.view = st.view;
                    fresh.frame_pending = false;
                    fresh.selected_track = st.selected_track.min(
                        fresh.doc.tracks.len().saturating_sub(1),
                    );
                    *st = fresh;
                    self.editor
                        .console
                        .messages
                        .push(LogMessage::info(format!("Curve reloaded: {key}")));
                }
                Err(e) => self.editor.console.messages.push(LogMessage::error(format!(
                    "Failed to reload curve '{key}': {e}"
                ))),
            }
        }
    }

    /// Route a winit event to a torn-off float window. Returns true if the
    /// window id belongs to a float (event consumed).
    #[cfg(feature = "editor")]
    pub fn crusty_float_event(
        &mut self,
        id: winit::window::WindowId,
        event: &winit::event::WindowEvent,
    ) -> bool {
        let Some(fw) = self.crusty_floats.get_mut(&id) else {
            return false;
        };
        fw.handle_event(event);
        true
    }

    /// Whether any crusty float window hosts the given dock tab id.
    #[cfg(feature = "editor")]
    pub fn crusty_float_hosts_tab(&self, tab: &str) -> bool {
        self.crusty_floats
            .values()
            .any(|fw| fw.tree.contains_tab(tab))
    }

    /// Create OS windows for tabs dropped outside the dock. Window creation
    /// needs the event loop, so main.rs calls this from `about_to_wait`.
    #[cfg(feature = "editor")]
    pub fn crusty_spawn_floats(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        use rust_engine::engine::editor::crusty_window::{float_window_attrs, CrustyFloatWindow};
        if self.pending_crusty_floats.is_empty() {
            return;
        }
        let main_origin = self.core.window.inner_position().unwrap_or_default();
        let device = self.core.renderer.gpu.device.clone();
        let queue = self.core.renderer.gpu.queue.clone();
        let memory_allocator = self.core.renderer.gpu.memory_allocator.clone();
        let command_buffer_allocator = self.core.renderer.gpu.command_buffer_allocator.clone();
        for req in std::mem::take(&mut self.pending_crusty_floats) {
            let (mut title, w, h) = float_window_attrs(&req.tab);
            // A plugin panel's real title lives in the plugin set, not in the
            // tab enum — without this the OS window would say the panel id.
            if let Some(id) = req.tab.strip_prefix("plugin:") {
                if let Some(entry) = self.core.plugin_set.panel_mut(id) {
                    title = entry.title.clone();
                }
            }
            // Tab strip roughly under the cursor, where the ghost card was.
            let pos = winit::dpi::PhysicalPosition::new(
                main_origin.x + req.main_local.x as i32 - 60,
                main_origin.y + req.main_local.y as i32 - 14,
            );
            // Borderless: the dock tab bar doubles as the title bar
            // (Unreal-style); move/resize/caption buttons are hand-rolled
            // in CrustyFloatWindow.
            let attrs = winit::window::Window::default_attributes()
                .with_title(title)
                .with_decorations(false)
                .with_inner_size(winit::dpi::PhysicalSize::new(w, h))
                .with_position(pos);
            let win = match event_loop.create_window(attrs) {
                Ok(w) => Arc::new(w),
                Err(e) => {
                    eprintln!("Failed to create float window: {e}");
                    rust_engine::engine::editor::dock_crusty::redock_tab(
                        &mut self.editor.ui.crusty_dock.tree,
                        req.tab,
                    );
                    // A tab re-entering the dock re-arms the scan; its
                    // document normally survives the round trip, but this
                    // costs one boolean and closes the gap either way.
                    self.pending_hydration = true;
                    continue;
                }
            };
            match CrustyFloatWindow::new(
                win.clone(),
                device.clone(),
                queue.clone(),
                memory_allocator.clone(),
                command_buffer_allocator.clone(),
                req.tab.clone(),
            ) {
                Ok(mut fw) => {
                    fw.gui.apply_theme(&self.editor.services.theme);
                    self.crusty_floats.insert(win.id(), fw);
                }
                Err(e) => {
                    eprintln!("Failed to init float window: {e}");
                    rust_engine::engine::editor::dock_crusty::redock_tab(
                        &mut self.editor.ui.crusty_dock.tree,
                        req.tab,
                    );
                    self.pending_hydration = true;
                }
            }
        }
    }

    /// Layout + render every float window (main thread, each with its own
    /// renderer). Emptied or user-closed windows re-dock their tabs into the
    /// main tree so a panel is never lost.
    #[cfg(feature = "editor")]
    pub fn crusty_float_frames(&mut self) {
        if self.crusty_floats.is_empty() {
            return;
        }
        use rust_engine::engine::editor::asset_browser_crusty::{
            asset_browser_panel, AssetBrowserPanelCtx,
        };
        use rust_engine::engine::editor::console_crusty::{console_panel, ConsolePanelCtx};
        use rust_engine::engine::editor::dock_crusty;
        use rust_engine::engine::editor::hierarchy_crusty::{hierarchy_panel, HierarchyPanelCtx};
        use rust_engine::engine::editor::input_editors_crusty::{
            input_action_panel, input_context_panel, input_settings_panel,
        };
        use rust_engine::engine::editor::inspector_crusty::{inspector_panel, InspectorPanelCtx};
        use rust_engine::engine::editor::mesh_editor_crusty::{
            mesh_editor_panel, MeshEditorPanelCtx,
        };
        use rust_engine::engine::editor::graph_editor_crusty::{
            graph_editor_panel, GraphEditorPanelCtx,
        };
        use rust_engine::engine::editor::profiler_crusty::profiler_panel;

        let device = self.core.renderer.gpu.device.clone();
        let queue = self.core.renderer.gpu.queue.clone();
        let memory_allocator = self.core.renderer.gpu.memory_allocator.clone();
        let command_buffer_allocator = self.core.renderer.gpu.command_buffer_allocator.clone();
        let play_mode = self.play_mode();
        let active_scene_id = self.editor.scene.registry.active_id;
        let scene_name = self.editor.scene.current_scene_name.clone();

        let Self {
            crusty_floats,
            crusty_float_preview_cbs,
            editor,
            core,
            ..
        } = self;
        let mut float_cbs = std::mem::take(crusty_float_preview_cbs);
        let action_set_snapshot = core
            .game_world
            .resource::<InputSubsystem>()
            .map(|s| s.action_set.clone());
        let world_object_info =
            rust_engine::engine::editor::world_object::WorldObjectInfo::from_streamer(
                &core.world_streamer,
            );
        let (world, world_resources) = core.game_world.world_and_resources_mut();
        let plugin_set = &mut core.plugin_set;
        let float_scripting_inactive =
            !plugin_set.is_active(rust_engine::engine::plugins::GRAPH_SCRIPTING_ID);
        let float_physics_inactive =
            !plugin_set.is_active(rust_engine::engine::plugins::PHYSICS_RAPIER_ID);
        let plugin_panel_titles: std::collections::HashMap<String, String> =
            plugin_set.panel_menu_entries().into_iter().collect();
        let console = &mut editor.console;
        let show_stat_fps = &mut editor.ui.show_stat_fps;
        let hierarchy = &mut editor.scene.hierarchy_panel;
        let inspector = &mut editor.scene.inspector_panel;
        let asset_browser = &mut editor.scene.asset_browser;
        let profiler = &mut editor.ui.profiler_panel;
        let sel = &mut editor.scene.selection;
        let icon_registry = editor.services.icons.clone();
        let dormant = &editor.scene.registry.dormant;
        let input_settings = &mut editor.ui.input_settings_panel;
        let action_set = action_set_snapshot.as_ref();
        let ia_states = &mut editor.scene.input_action_editor.open_actions;
        let input_context_editor = &mut editor.scene.input_context_editor;
        let mc_states = &mut input_context_editor.open_contexts;
        let mc_actions = input_context_editor.available_actions.as_slice();
        let mesh_editors = &mut editor.scene.mesh_editors;
        let graph_editors = &mut editor.scene.graph_editors;
        let graph_registry = &editor.scene.node_registry;
        let graph_keymap = &editor.services.keymap;
        let graph_clipboard = &mut editor.scene.graph_clipboard;
        let curve_editors = &mut editor.scene.curve_editors;
        // P6: subgraph resolver + `.subgraph` asset list for float graph panels.
        let graph_resolver_docs = rust_engine::engine::editor::graph_editor::build_resolver_docs(
            graph_editors.iter().map(|(k, s)| (k.as_str(), &s.doc)),
            std::path::Path::new("content"),
        );
        // 45-A P8b: and the `.curve` resolver, on the same rule.
        let graph_curve_docs = rust_engine::engine::editor::graph_editor::build_curve_docs(
            curve_editors.iter().map(|(k, s)| (k.as_str(), &s.doc)),
            &graph_resolver_docs,
            std::path::Path::new("content"),
        );
        let subgraph_assets: Vec<String> = {
            let filter = rust_engine::engine::editor::AssetFilter {
                asset_types: Some(vec![AssetType::Graph]),
                include_subfolders: true,
                ..Default::default()
            };
            asset_browser
                .registry
                .query(&filter)
                .into_iter()
                .filter(|m| m.path.extension().and_then(|e| e.to_str()) == Some("subgraph"))
                .map(|m| asset_source::to_content_relative(&m.path.to_string_lossy()))
                .collect()
        };
        let graph_prefs = editor.ui.settings.prefs.graph;
        let graph_sel_outline = editor.services.theme.palette.selection.outline;
        let graph_camera = {
            let p = rust_engine::engine::utils::coords::convert_position_yup_to_zup(
                editor.viewport.camera.position,
            );
            [p.x, p.y, p.z]
        };
        // 45-A P7 execution binding, same rule as the docked path.
        let graph_camera_pos = graph_camera;
        let graph_debug_tabs: Vec<GraphDebugTab> = graph_editors
            .iter_mut()
            .map(|(k, st)| GraphDebugTab {
                path: k.clone(),
                bind: st.exec_bind,
                armed: st.armed_breakpoints(),
                request: st.debug_request.take(),
            })
            .collect();
        let graph_exec = {
            let picked: Vec<hecs::Entity> = sel.all().copied().collect();
            graph_exec_bindings(&*world, &*world_resources, &picked, &graph_debug_tabs)
        };
        let graph_debug = graph_debug_plan(&graph_debug_tabs, &graph_exec);
        let graph_instances = graph_instance_lists(
            &*world,
            &*world_resources,
            graph_camera_pos,
            graph_editors.keys(),
        );
        let mut graph_clear_trace: Option<String> = None;
        let mut graph_open_requests: Vec<String> = Vec::new();
        let mut graph_style_request: Option<WireStyle> = None;
        // Curve tabs in float windows that asked to save (their own Ctrl+S or
        // the toolbar). Applied after the window loop, like the graph opens.
        let mut float_curve_saves: Vec<String> = Vec::new();

        for fw in crusty_floats.values_mut() {
            let mut tabs = Vec::new();
            fw.tree.collect_tabs(&mut tabs);

            // Mesh previews hosted here: drop stale textures, register this
            // window's registry-local ids, claim this window's preview CBs.
            fw.prune_mesh_textures(|k| tabs.iter().any(|t| t == &format!("mesh:{k}")));
            let mut cbs = Vec::new();
            let mut cb_keys = Vec::new();
            float_cbs.retain(|(k, cb)| {
                if tabs.iter().any(|t| t == &format!("mesh:{k}")) {
                    cbs.push(cb.clone());
                    cb_keys.push(k.clone());
                    false
                } else {
                    true
                }
            });
            let mut mesh_tex = std::collections::HashMap::new();
            for tab in &tabs {
                let Some(key) = tab.strip_prefix("mesh:") else {
                    continue;
                };
                if let Some(preview) = mesh_editors.get(key).and_then(|d| d.preview.as_ref()) {
                    if !preview.mesh_indices.is_empty() {
                        let has_cb = cb_keys.iter().any(|k| k == key);
                        if let Some(id) =
                            fw.ensure_mesh_texture(key, preview.texture.image_view(), has_cb)
                        {
                            mesh_tex.insert(key.to_string(), id);
                        }
                    }
                }
            }

            // Material thumbnails: upload the shared cache's retained pixels
            // into this window's own registry (ids are registry-local).
            if tabs.iter().any(|t| t.starts_with("mesh:")) {
                for (aid, (rgba, w, h)) in asset_browser.thumbnails.crusty_rgba_iter() {
                    fw.register_thumb(
                        queue.clone(),
                        memory_allocator.clone(),
                        command_buffer_allocator.clone(),
                        aid,
                        rgba,
                        *w,
                        *h,
                    );
                }
            }

            // Float windows don't render dirty dots (DockArea here has no
            // `.dirty_tabs`), so an empty set suffices.
            let float_editor_dirty = std::collections::HashSet::new();
            let (titles, _) = dock_crusty::tab_titles(
                &fw.tree,
                dock_crusty::TabTitlesCtx {
                    active_id: active_scene_id,
                    active_name: &scene_name,
                    active_dirty: false,
                    dormant,
                    extra: None,
                    editor_dirty: &float_editor_dirty,
                    plugin_titles: &plugin_panel_titles,
                },
            );
            // Subgraph double-click in this float → queued for the host to open.
            let mut float_open_request: Option<String> = None;
            let res = fw.frame(
                device.clone(),
                queue.clone(),
                &titles,
                cbs,
                |ui, tab, icons, thumbs| {
                    let rect = ui.clip_rect();
                    match dock_crusty::parse_tab(tab) {
                        Some(EditorTab::Console) => console_panel(
                            ui,
                            rect,
                            ConsolePanelCtx {
                                messages: &mut console.messages,
                                filter: &mut console.log_filter,
                                command_system: &mut console.command_system,
                                input: &mut console.input,
                                world: &mut *world,
                                show_stat_fps: &mut *show_stat_fps,
                            },
                        ),
                        Some(EditorTab::Hierarchy) => hierarchy_panel(
                            ui,
                            rect,
                            HierarchyPanelCtx {
                                panel: hierarchy,
                                world: &mut *world,
                                selection: sel,
                                play_mode,
                                icons,
                                registry: &icon_registry,
                                world_object: world_object_info.as_ref(),
                            },
                        ),
                        Some(EditorTab::Inspector) => inspector_panel(
                            ui,
                            rect,
                            InspectorPanelCtx {
                                panel: inspector,
                                world: &mut *world,
                                selection: &*sel,
                                play_mode,
                                asset_browser: &mut *asset_browser,
                                icons,
                                world_object: world_object_info.as_ref(),
                                physics_inactive: float_physics_inactive,
                                scripting_inactive: float_scripting_inactive,
                            },
                        ),
                        Some(EditorTab::AssetBrowser) => asset_browser_panel(
                            ui,
                            rect,
                            AssetBrowserPanelCtx {
                                panel: &mut *asset_browser,
                                icons,
                            },
                        ),
                        Some(EditorTab::Profiler) => profiler_panel(ui, rect, profiler),
                        Some(EditorTab::InputSettings) => {
                            input_settings_panel(ui, rect, input_settings, action_set)
                        }
                        Some(EditorTab::InputActionEditor(key)) => match ia_states.get_mut(&key) {
                            Some(state) => input_action_panel(ui, rect, &key, state),
                            None => dock_crusty::missing_document_panel(ui, "Input action", &key, None),
                        },
                        Some(EditorTab::InputContextEditor(key)) => match mc_states.get_mut(&key) {
                            Some(state) => input_context_panel(ui, rect, &key, state, mc_actions),
                            None => {
                                dock_crusty::missing_document_panel(ui, "Mapping context", &key, None)
                            }
                        },
                        Some(EditorTab::MeshEditor(key)) => match mesh_editors.get_mut(&key) {
                            Some(data) => mesh_editor_panel(
                                ui,
                                rect,
                                MeshEditorPanelCtx {
                                    data,
                                    texture: mesh_tex.get(&key).copied(),
                                    asset_browser: &mut *asset_browser,
                                    icons,
                                    float_thumbs: Some(thumbs),
                                },
                            ),
                            None => dock_crusty::missing_document_panel(ui, "Mesh", &key, None),
                        },
                        Some(EditorTab::GraphEditor(key)) => match graph_editors.get_mut(&key) {
                            Some(state) => graph_editor_panel(
                                ui,
                                GraphEditorPanelCtx {
                                    state,
                                    registry: graph_registry,
                                    keymap: graph_keymap,
                                    clipboard: graph_clipboard,
                                    resolver: &graph_resolver_docs,
                                    curves: &graph_curve_docs,
                                    subgraph_assets: &subgraph_assets,
                                    open_subgraph: &mut float_open_request,
                                    selection_outline: graph_sel_outline,
                                    wire_prefs: graph_prefs.wires,
                                    wire_style_request: &mut graph_style_request,
                                    zoom_min: graph_prefs.zoom_min,
                                    zoom_max: graph_prefs.zoom_max,
                                    // A float window is a dedicated surface;
                                    // keys only arrive when it's OS-focused, so
                                    // the panel owns keyboard editing here.
                                    focused: true,
                                    handle_shortcuts: true,
                                    exec: graph_exec
                                        .iter()
                                        .find(|(k, _)| *k == key)
                                        .map(|(_, v)| v),
                                    exec_instances: graph_instances
                                        .get(&key)
                                        .map(Vec::as_slice)
                                        .unwrap_or(&[]),
                                    exec_clear: &mut graph_clear_trace,
                                },
                            ),
                            None => dock_crusty::missing_document_panel(ui, "Graph", &key, None),
                        },
                        Some(EditorTab::CurveEditor(key)) => match curve_editors.get_mut(&key)
                        {
                            Some(state) => {
                                if rust_engine::engine::editor::curve_editor_crusty::
                                    curve_editor_panel(
                                        ui,
                                        rect,
                                        rust_engine::engine::editor::curve_editor_crusty::
                                            CurveEditorPanelCtx {
                                                state,
                                                selection_outline: graph_sel_outline,
                                                // A float window is a dedicated
                                                // surface: keys only arrive when
                                                // it is OS-focused, so the panel
                                                // owns its keyboard here.
                                                focused: true,
                                                handle_shortcuts: true,
                                            },
                                    )
                                    .save_requested
                                {
                                    float_curve_saves.push(key.clone());
                                }
                            }
                            None => dock_crusty::missing_document_panel(ui, "Curve", &key, None),
                        },
                        Some(EditorTab::Plugin(id)) => match plugin_set.panel_mut(&id) {
                            Some(entry) => entry.panel.draw(
                                ui,
                                rect,
                                &mut rust_engine::engine::plugins::PluginPanelCtx {
                                    world: &mut *world,
                                    resources: &mut *world_resources,
                                    play_mode,
                                },
                            ),
                            None => dock_crusty::missing_document_panel(
                                ui,
                                "Plugin panel",
                                &id,
                                Some("no enabled plugin registers this panel"),
                            ),
                        },
                        _ => dock_crusty::placeholder_panel(
                            ui,
                            "This panel is not yet ported to crusty-gui.",
                        ),
                    }
                },
            );
            if let Err(e) = res {
                eprintln!("crusty float window frame failed: {e}");
            }
            if let Some(path) = float_open_request {
                graph_open_requests.push(path);
            }
        }

        // Open any subgraphs double-clicked in float windows, as main-dock
        // tabs. Uses the existing `graph_editors`/`graph_registry` bindings so
        // it doesn't re-borrow `editor.scene` fields already held.
        if let Some(style) = graph_style_request {
            editor.ui.settings.prefs.graph.wires.style = style;
        }
        for relative in graph_open_requests {
            if graph_editors.contains_key(&relative) {
                editor.ui.crusty_dock.open_tab(EditorTab::GraphEditor(relative));
            } else {
                let abs = std::path::Path::new("content").join(&relative);
                match rust_engine::engine::editor::graph_editor::GraphEditorState::open(
                    &abs,
                    &relative,
                    graph_registry,
                ) {
                    Ok(mut state) => {
                        restore_graph_ui_state(&mut state, &relative);
                        graph_editors.insert(relative.clone(), state);
                        editor.ui.crusty_dock.open_tab(EditorTab::GraphEditor(relative));
                    }
                    Err(e) => editor.console.messages.push(LogMessage::error(format!(
                        "Failed to open graph '{relative}': {e}"
                    ))),
                }
            }
        }

        for key in std::mem::take(&mut float_curve_saves) {
            save_curve_state(curve_editors, &mut editor.console, &mut core.game_world, &key);
        }

        // GS-4, float-window path: same delivery as the docked one — the bound
        // instance gets this tab's breakpoints, everyone else gets an empty
        // set — done here because the world is mutably reachable again.
        for tab in &graph_debug {
            rust_engine::engine::scripting::trace::arm_debug(
                core.game_world.hecs_mut(),
                &tab.path,
                tab.instance,
                &tab.armed,
                tab.request,
            );
        }

        crusty_floats.retain(|_, fw| {
            let mut tabs = Vec::new();
            fw.tree.collect_tabs(&mut tabs);
            if fw.close_requested {
                // Dirty graph → veto: confirm via the main dialog stack and
                // keep the window open (the dialog's Save/Discard drives the
                // actual close through `close_graph_tab`, emptying this window).
                let dirty_graph = tabs.iter().find_map(|t| {
                    let key = t.strip_prefix("graph:")?;
                    graph_editors.get(key).filter(|s| s.dirty).map(|_| key.to_string())
                });
                let dirty_curve = tabs.iter().find_map(|t| {
                    let key = t.strip_prefix("curve:")?;
                    curve_editors.get(key).filter(|s| s.dirty).map(|_| key.to_string())
                });
                if let Some(key) = dirty_curve {
                    let msg = format!("Save changes to '{key}' before closing?");
                    editor.services.dialogs.save_discard_cancel(
                        format!("curve_close:{key}"),
                        "Unsaved Curve",
                        msg,
                        EditorAction::SaveAndCloseEditor {
                            kind: SecondaryWindowKind::Curve,
                            key: key.clone(),
                        },
                        EditorAction::DiscardAndCloseEditor {
                            kind: SecondaryWindowKind::Curve,
                            key,
                        },
                    );
                    fw.close_requested = false;
                    return true;
                }
                if let Some(key) = dirty_graph {
                    let msg = format!("Save changes to '{key}' before closing?");
                    editor.services.dialogs.save_discard_cancel(
                        format!("graph_close:{key}"),
                        "Unsaved Graph",
                        msg,
                        EditorAction::SaveAndCloseEditor {
                            kind: SecondaryWindowKind::Graph,
                            key: key.clone(),
                        },
                        EditorAction::DiscardAndCloseEditor {
                            kind: SecondaryWindowKind::Graph,
                            key,
                        },
                    );
                    fw.close_requested = false;
                    return true;
                }
                for tab in tabs {
                    if let Some(key) = tab.strip_prefix("mesh:") {
                        // Asset editors close with their window (`open = false`
                        // is culled by the per-frame retain in run_frame).
                        if let Some(data) = mesh_editors.get_mut(key) {
                            data.open = false;
                        }
                    } else if let Some(key) = tab.strip_prefix("graph:") {
                        // Clean graph: drop the doc.
                        graph_editors.remove(key);
                    } else if let Some(key) = tab.strip_prefix("curve:") {
                        curve_editors.remove(key);
                    } else {
                        dock_crusty::redock_tab(&mut editor.ui.crusty_dock.tree, tab);
                    }
                }
                return false;
            }
            !tabs.is_empty()
        });
    }

    fn save_active_scene(&mut self) {
        if self.play_mode() != PlayMode::Edit {
            log::warn!("Cannot save scene during play mode");
            return;
        }

        // Untitled scene: open the Save As dialog instead of saving silently.
        if self.editor.scene.current_scene_relative.is_empty() {
            if self.editor.scene.save_as_dialog.is_none() {
                let initial = self.editor.scene.current_scene_name.clone();
                self.editor.scene.save_as_dialog = Some(SaveAsDialog::new(&initial));
            }
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
                self.editor.scene.active_dirty = false;
                self.editor.scene.command_history.mark_saved();
                self.editor
                    .console
                    .messages
                    .push(LogMessage::info(format!("Saved scene: {}", scene_relative)));
                if !rust_engine::engine::collision::output::cook_is_current(&scene_relative) {
                    self.cook_scene_collision();
                }
            }
            Err(error) => {
                eprintln!("Save failed: {}", error);
                self.editor.console.messages.push(LogMessage::error(format!(
                    "Failed to save scene '{}': {}",
                    scene_relative, error
                )));
            }
        }

        // Project settings ride along with Ctrl+S.
        if self.editor.ui.settings.project_dirty() {
            match self.editor.ui.settings.save_project() {
                Ok(()) => {
                    self.apply_project_settings();
                    self.editor.console.messages.push(LogMessage::info(
                        "Saved project settings: project.ron".to_string(),
                    ));
                }
                Err(e) => self.editor.console.messages.push(LogMessage::error(format!(
                    "Failed to save project settings: {e}"
                ))),
            }
        }
    }

    /// Handle a Relaunch Now click from the Plugin Manager (39.8 §5.6).
    ///
    /// Order matters: save first (through the atomic-write paths), then spawn
    /// the replacement, then ask the loop to exit. The child parks on our
    /// process handle until we are actually gone, so it never reads a config
    /// we are still holding.
    ///
    /// Returns `true` when the caller should exit the event loop.
    #[must_use]
    pub fn take_relaunch_request(&mut self) -> bool {
        if !std::mem::take(&mut self.editor.ui.settings.plugins.relaunch_requested) {
            return false;
        }

        // A relaunch is a process exit, so anything that would lose work or be
        // cut off refuses it outright. Project settings are saved below (they
        // are the thing being relaunched *for*); scenes are not — silently
        // auto-saving someone's edits is worse than refusing.
        //
        // The manifest edit stays pending, so the manager's banner is still
        // there once the user has dealt with the reason.
        let mut refusals: Vec<&str> = Vec::new();
        if self.play_mode() != PlayMode::Edit {
            // Exiting mid-play would drop the edit-world snapshot taken on
            // enter, i.e. every edit made before pressing Play.
            refusals.push("Stop play mode before relaunching");
        }
        let scene = &self.editor.scene;
        if scene.active_dirty
            || scene.command_history.is_dirty()
            || scene.registry.dormant.iter().any(|d| d.dirty)
        {
            refusals.push("Save scenes before relaunching");
        }
        if matches!(
            self.editor.play.build_dialog.state,
            rust_engine::engine::editor::build_dialog::BuildState::Building
                | rust_engine::engine::editor::build_dialog::BuildState::CopyingContent
        ) {
            refusals.push("Wait for the build to finish before relaunching");
        }
        if !refusals.is_empty() {
            for reason in refusals {
                self.editor
                    .console
                    .messages
                    .push(LogMessage::error(format!("Relaunch cancelled — {reason}")));
            }
            return false;
        }

        // Unsaved-changes check runs first, exactly like the normal close
        // path — a restart must not be a way to lose work.
        if self.editor.ui.settings.project_dirty() {
            if let Err(e) = self.editor.ui.settings.save_project() {
                self.editor.console.messages.push(LogMessage::error(format!(
                    "Relaunch cancelled — could not save project settings: {e}"
                )));
                return false;
            }
            self.apply_project_settings();
        }
        self.save_layout_on_exit();

        match rust_engine::engine::editor::relaunch::spawn_replacement() {
            Ok(pid) => {
                println!("relaunching editor: child pid {pid}");
                true
            }
            Err(e) => {
                self.editor
                    .console
                    .messages
                    .push(LogMessage::error(format!("Relaunch failed: {e}")));
                false
            }
        }
    }

    /// Push saved project settings into live engine state (physics constants
    /// and streaming budgets; scene/build fields apply on next launch).
    fn apply_project_settings(&mut self) {
        let pc = self.editor.ui.settings.project.clone();
        if let Some(physics) = self.core.game_world.resource_mut::<PhysicsWorld>() {
            physics.set_gravity(nalgebra_glm::vec3(0.0, 0.0, pc.gravity_z));
            physics.set_timestep(1.0 / pc.fixed_timestep_hz.max(1.0));
        }
        let cfg = &mut self.core.world_streamer.config;
        cfg.r_load = pc.stream_r_load;
        cfg.r_unload = pc.stream_r_unload;
        cfg.budget_ms = pc.stream_budget_ms;
        cfg.max_in_flight = pc.stream_max_in_flight;
    }

    /// Runs `f` with a `StreamingCtx` assembled from engine parts. The
    /// `CollisionWorld` resource is temporarily removed (PhysicsWorld
    /// pattern) so it can be borrowed alongside the hecs world.
    fn with_streaming_ctx<R>(
        &mut self,
        f: impl FnOnce(&mut WorldStreamer, &mut StreamingCtx<'_>) -> R,
    ) -> R {
        let mut collision = self
            .core
            .game_world
            .resources_mut()
            .remove::<CollisionWorld>()
            .unwrap_or_default();
        let allocator = self.core.renderer.gpu.memory_allocator.clone();
        let result = {
            let mut meshes = self.core.asset_manager.meshes.write();
            let mut ctx = StreamingCtx {
                world: self.core.game_world.hecs_mut(),
                meshes: &mut meshes,
                allocator,
                collision: &mut collision,
            };
            f(&mut self.core.world_streamer, &mut ctx)
        };
        self.core.game_world.resources_mut().insert(collision);
        result
    }

    /// Per-frame world streaming around the editor camera (Z-up center).
    /// Inert for scenes without a world manifest.
    fn update_world_streaming(&mut self) {
        if !self.core.world_streamer.is_active() {
            return;
        }
        use rust_engine::engine::utils::coords::convert_position_yup_to_zup;
        let center = convert_position_yup_to_zup(self.editor.viewport.camera.position);
        let output =
            self.with_streaming_ctx(|streamer, ctx| streamer.update_streaming(center, ctx));
        if let Some(event) = output.zone_changed {
            self.editor.console.messages.push(LogMessage::info(format!(
                "Zone changed: {:?} -> {:?}",
                event.previous, event.current
            )));
            self.core.game_world.send_event(event);
        }
    }

    /// Tears down all streamed content. Must run before scene swaps, tab
    /// parking, and play-mode snapshot restore (those clear/replace the hecs
    /// world, which would leak mesh refcounts and dangle entity ids).
    fn flush_streaming(&mut self) {
        if !self.core.world_streamer.is_active() {
            return;
        }
        self.with_streaming_ctx(|streamer, ctx| streamer.flush(ctx));
    }

    /// Sets up world streaming for a newly active scene, falling back to the
    /// monolithic collision load for manifest-less scenes.
    fn init_streaming_for_scene(&mut self, scene_relative: &str) {
        let report = self.core.world_streamer.load_for_scene(scene_relative);
        for warning in &report.warnings {
            self.editor
                .console
                .messages
                .push(LogMessage::warning(format!("Streaming: {warning}")));
        }
        if let Some(reason) = &report.disabled {
            self.editor.console.messages.push(LogMessage::info(format!(
                "Streaming inactive ({reason}) — monolithic collision load"
            )));
            self.reload_collision_world(scene_relative);
            return;
        }
        if let Some(collision) = self.core.game_world.resource_mut::<CollisionWorld>() {
            collision.begin_streaming(scene_relative);
        }
        let world = self.core.world_streamer.world().expect("just loaded");
        self.editor.console.messages.push(LogMessage::info(format!(
            "Streaming '{}': {} cell(s), {} collision chunk(s) available ({})",
            world.stem,
            world.manifest.cells.len(),
            world.cooked_chunks.len(),
            if self.core.world_streamer.full_world {
                "full world"
            } else {
                "around camera"
            }
        )));
    }

    /// Reload the `CollisionWorld` from cooked chunks for `scene_relative`,
    /// reporting the outcome to the editor console.
    fn reload_collision_world(&mut self, scene_relative: &str) {
        let Some(collision) = self.core.game_world.resource_mut::<CollisionWorld>() else {
            return;
        };
        let report = collision.load_for_scene(scene_relative);
        if let Some(reason) = &report.disabled {
            self.editor
                .console
                .messages
                .push(LogMessage::warning(format!("Collision: {reason}")));
        } else {
            self.editor.console.messages.push(LogMessage::info(format!(
                "Collision: {} chunk(s) loaded, {} skipped in {:.1} ms",
                report.loaded, report.skipped, report.elapsed_ms
            )));
        }
        for warning in &report.warnings {
            self.editor
                .console
                .messages
                .push(LogMessage::warning(format!("Collision: {warning}")));
        }
    }

    /// Cook the active scene's static collision to `content/collision/<scene>/`.
    fn cook_scene_collision(&mut self) {
        use rust_engine::engine::collision::{cook, output};

        if self.play_mode() != PlayMode::Edit {
            self.editor.console.messages.push(LogMessage::warning(
                "Cook Collision: exit play mode first".to_string(),
            ));
            return;
        }
        if self.editor.scene.current_scene_relative.is_empty() {
            self.editor.console.messages.push(LogMessage::warning(
                "Cook Collision: save the scene first".to_string(),
            ));
            return;
        }

        let scene_relative = self.editor.scene.current_scene_relative.clone();
        let stem = output::scene_stem(&scene_relative);
        // Streamed cells are not scene entities — cook them from the world
        // manifest (resident StreamedCell entities carry no StaticCollision).
        let cells = output::manifest_cell_sources(&scene_relative);
        let mut loader = output::load_model_from_content;
        let mut cooked = cook::cook_scene(self.core.game_world.hecs(), &stem, &cells, &mut loader);
        cooked.manifest.scene_hash = output::scene_content_hash(&scene_relative).unwrap_or(0);
        for warning in &cooked.warnings {
            self.editor
                .console
                .messages
                .push(LogMessage::warning(format!("Cook Collision: {warning}")));
        }

        let Some(dir) = output::collision_dir_for_scene(&scene_relative) else {
            self.editor.console.messages.push(LogMessage::error(
                "Cook Collision: no content root initialized".to_string(),
            ));
            return;
        };
        match output::write_cooked_scene(&dir, &cooked) {
            Ok(()) => {
                self.editor.console.messages.push(LogMessage::info(format!(
                    "Cooked collision: {} chunk(s) -> {}",
                    cooked.chunks.len(),
                    dir.display()
                )));
                if self.core.world_streamer.is_active() {
                    // Chunk hashes changed: restart streaming on the new cook.
                    self.flush_streaming();
                    self.init_streaming_for_scene(&scene_relative);
                } else {
                    self.reload_collision_world(&scene_relative);
                }
            }
            Err(error) => self.editor.console.messages.push(LogMessage::error(format!(
                "Cook Collision failed writing to '{}': {error}",
                dir.display()
            ))),
        }
    }

    /// Commit the Save As dialog: persist the active scene to the chosen filename
    /// and update the active scene's path/name accordingly.
    fn commit_save_as(&mut self, filename: &str) {
        let trimmed = filename.trim();
        if trimmed.is_empty() {
            self.editor.console.messages.push(LogMessage::warning(
                "Save As: filename cannot be empty".to_string(),
            ));
            return;
        }
        let relative = format!("scenes/{}.scene", trimmed);
        let scene_path = asset_source::resolve(&relative);
        // Use the trimmed filename as the display name if the scene is untitled.
        let display_name = if self.editor.scene.current_scene_name.is_empty()
            || self.editor.scene.current_scene_name == "Untitled Scene"
        {
            trimmed.to_string()
        } else {
            self.editor.scene.current_scene_name.clone()
        };

        match save_scene(
            self.core.game_world.hecs(),
            &scene_path.to_string_lossy(),
            &display_name,
            self.editor.scene.hierarchy_panel.root_order(),
        ) {
            Ok(_) => {
                println!("Scene saved to {}", scene_path.display());
                self.editor.scene.current_scene_relative = relative.clone();
                self.editor.scene.current_scene_name = display_name;
                self.editor.scene.active_dirty = false;
                self.editor.scene.command_history.mark_saved();
                self.editor
                    .console
                    .messages
                    .push(LogMessage::info(format!("Saved scene: {}", relative)));
                if !rust_engine::engine::collision::output::cook_is_current(&relative) {
                    self.cook_scene_collision();
                }
            }
            Err(error) => {
                eprintln!("Save failed: {}", error);
                self.editor.console.messages.push(LogMessage::error(format!(
                    "Failed to save scene '{}': {}",
                    relative, error
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
            "png",
            "jpg",
            "jpeg",
            "tga",
            "bmp",
            "dds",
            // Native mesh (already processed)
            "mesh", // Audio
            "wav",
            "ogg",
            "mp3",
            "flac", // Shaders
            "glsl",
            "vert",
            "frag",
            "comp",
            "spv", // Engine RON assets
            "scene",
            "material",
            "matinst",
            "prefab",
            "inputaction",
            "mappingcontext",
            // Legacy `.ron`-suffixed assets
            "ron",
        ];

        let files = other_files;

        let mut imported_count = 0;
        let mut skipped_count = 0;

        for source_path in &files {
            // Validate that the file exists and is a file (not directory)
            if !source_path.is_file() {
                self.editor
                    .console
                    .messages
                    .push(LogMessage::warning(format!(
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
                self.editor
                    .console
                    .messages
                    .push(LogMessage::warning(format!(
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
                    let new_name = format!(
                        "{} ({}){}",
                        stem,
                        counter,
                        if extension.is_empty() {
                            String::new()
                        } else {
                            format!(".{}", extension)
                        }
                    );
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
                                let mtl_dest =
                                    target_dir.join(mtl_path.file_name().unwrap_or_default());
                                if let Err(e) = std::fs::copy(&mtl_path, &mtl_dest) {
                                    self.editor.console.messages.push(LogMessage::warning(
                                        format!("Could not copy companion .mtl file: {}", e),
                                    ));
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

    /// Apply the import dialog's per-frame result: import, cancel, or lazily
    /// populate the preview stats while it is open.
    fn handle_import_dialog_action(&mut self, action: ImportDialogAction) {
        match action {
            ImportDialogAction::Import => {
                if let Some(dialog) = self.editor.scene.import_dialog.take() {
                    self.execute_model_import(dialog);
                }
            }
            ImportDialogAction::Cancel => {
                self.editor.scene.import_dialog = None;
            }
            ImportDialogAction::None => {
                if let Some(ref mut dialog) = self.editor.scene.import_dialog {
                    if !dialog.preview_attempted {
                        dialog.preview_attempted = true;
                        if let Some(source) = dialog.current_file().cloned() {
                            // Attempt a quick parse to get stats
                            match rust_engine::assets::load_model(&source.to_string_lossy()) {
                                Ok(model) => {
                                    let total_verts: u32 =
                                        model.meshes.iter().map(|m| m.vertices.len() as u32).sum();
                                    let total_idx: u32 =
                                        model.meshes.iter().map(|m| m.indices.len() as u32).sum();
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
    }

    /// Apply the anim events dialog's result: Save rewrites the `.anim` and
    /// invalidates the clip + plan caches, so running graph machines re-arm
    /// against the edited markers on their next tick.
    fn handle_anim_events_action(
        &mut self,
        action: rust_engine::engine::editor::anim_events_dialog::AnimEventsAction,
    ) {
        use rust_engine::engine::animation::graph::{AnimClipCache, AnimGraphPlanCache};
        use rust_engine::engine::editor::anim_events_dialog::AnimEventsAction;
        match action {
            AnimEventsAction::None => {}
            AnimEventsAction::Cancel => {
                self.editor.scene.anim_events_dialog = None;
            }
            AnimEventsAction::Save => {
                let Some(mut dlg) = self.editor.scene.anim_events_dialog.take() else {
                    return;
                };
                match dlg.save() {
                    Ok(()) => {
                        let resources = self.core.game_world.resources_mut();
                        if let Some(clips) = resources.get_mut::<AnimClipCache>() {
                            clips.invalidate(&dlg.relative);
                        }
                        if let Some(plans) = resources.get_mut::<AnimGraphPlanCache>() {
                            plans.invalidate(&dlg.relative);
                        }
                        self.editor.console.messages.push(LogMessage::info(format!(
                            "Anim events saved: {}",
                            dlg.relative
                        )));
                    }
                    Err(e) => {
                        self.editor
                            .console
                            .messages
                            .push(LogMessage::error(format!("Anim events save failed: {e}")));
                    }
                }
            }
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
            let source_dest = target_dir.join(source_path.file_name().unwrap_or_default());
            if !source_dest.exists() {
                if let Err(e) = std::fs::copy(source_path, &source_dest) {
                    self.editor
                        .console
                        .messages
                        .push(LogMessage::warning(format!(
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
                        msg.push_str(&format!(", {} material(s)", result.material_count));
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

        // The benchmark scene replaces the active world's contents.
        self.flush_streaming();
        self.core.world_streamer.clear();

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
        // Content moment. `load_or_create_benchmark_scene` already registered
        // bodies; the helper clears first, so this re-registers rather than
        // doubling them.
        self.run_world_population_hooks();
        self.editor.scene.hierarchy_panel.set_root_order(roots);
        self.editor.scene.current_scene_relative = BENCHMARK_SCENE_RELATIVE.to_string();
        self.editor.scene.current_scene_name = "Benchmark Scene".to_string();
        {
            self.core
                .game_world
                .resources_mut()
                .remove::<TransformCache>();
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
                self.editor.console.messages.push(LogMessage::error(format!(
                    "Shader compiler init failed: {e}"
                )));
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

        // M9.6 P4: a listen server is by definition local — refuse before
        // any play state changes (use Play As Client for remote servers).
        {
            let ps = &self.editor.ui.settings.prefs.play;
            if ps.mode == rust_engine::engine::editor::play_settings::NetPlayMode::ListenServer
                && !crate::listen_server::is_local_host(&ps.host)
            {
                self.editor.console.messages.push(LogMessage::error(format!(
                    "Listen Server requires a local host (got '{}') — use Play As Client \
                     for remote servers",
                    ps.host
                )));
                return;
            }
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

        // Resync Rapier with transforms edited in edit mode. Not a world
        // population moment (nothing was loaded), so it does not run the
        // plugin callbacks — but it *is* body registration, so it is gated on
        // the physics plugin actually running. Disabled means no handle is
        // created anywhere, play mode included (D7).
        if self
            .core
            .plugin_set
            .is_active(rust_engine::engine::plugins::PHYSICS_RAPIER_ID)
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

        // M9.6 P3/P4: the net session lives from play-enter to play-exit;
        // re-entering play reconnects fresh (M9.5 rejoin path).
        let ps = &self.editor.ui.settings.prefs.play;
        match ps.mode {
            rust_engine::engine::editor::play_settings::NetPlayMode::Client => {
                let (host, module) = (ps.host.clone(), ps.module.clone());
                self.editor.console.messages.push(LogMessage::info(format!(
                    "Play As Client: connecting to {host} / {module}"
                )));
                self.spawn_extra_clients(&host, &module);
                self.net = Some(crate::net::NetSession::connect_to(host, module));
                self.editor.play.net_scene_warned = false;
            }
            rust_engine::engine::editor::play_settings::NetPlayMode::ListenServer => {
                self.editor.console.messages.push(LogMessage::info(format!(
                    "Listen Server: ensuring local SpacetimeDB + publishing '{}'…",
                    ps.module
                )));
                self.listen_server_rx = Some(crate::listen_server::spawn_launcher(
                    ps.host.clone(),
                    ps.module.clone(),
                ));
                self.editor.play.net_scene_warned = false;
            }
            rust_engine::engine::editor::play_settings::NetPlayMode::Standalone => {}
        }

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

        // M9.6 P3: clean disconnect, then drop the session before the
        // snapshot restore clears the world (net proxies die with it;
        // server-side cleanup is the proven M5 disconnect path). P4: an
        // unfinished listen-server launch is abandoned — the spawned
        // SpacetimeDB deliberately outlives play (reuse-don't-stop).
        self.listen_server_rx = None;
        if let Some(mut net) = self.net.take() {
            net.disconnect();
        }
        // M9.6 P5: kill is the harshest disconnect — M9.5 proved server
        // cleanup handles it within seconds.
        let extra = self.play_clients.len();
        for mut child in self.play_clients.drain(..) {
            let _ = child.kill();
        }
        if extra > 0 {
            self.editor
                .console
                .messages
                .push(LogMessage::info(format!("Players: stopped {extra} extra client(s)")));
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

        // Snapshot restore clears the whole hecs world — streamed cells must
        // be torn down first (else dangling entity ids + mesh refcount leaks).
        // Streaming refills automatically next frame; the manifest stays loaded.
        self.flush_streaming();

        if let Some(snapshot) = self.editor.play.snapshot.as_ref() {
            match play_mode::restore_snapshot(
                snapshot,
                &mut self.core.game_world,
                &mut self.editor.scene.hierarchy_panel,
                &mut self.editor.scene.selection,
                &mut self.editor.scene.command_history,
            ) {
                Ok(()) => {
                    self.editor.play.snapshot = None;
                    // Content moment: the restored world's physics handles are
                    // stale and plugins need their world-loaded hooks.
                    self.run_world_population_hooks();
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

    /// Selected entities minus any whose ancestor is also selected —
    /// subtree operations must not process nested selections twice.
    fn topmost_selected_roots(&self) -> Vec<hecs::Entity> {
        use rust_engine::engine::ecs::hierarchy::Parent;
        let world = self.core.game_world.hecs();
        let selected: std::collections::HashSet<_> =
            self.editor.scene.selection.all().copied().collect();
        selected
            .iter()
            .copied()
            .filter(|&e| world.contains(e))
            .filter(|&e| {
                let mut cur = e;
                while let Ok(parent) = world.get::<&Parent>(cur) {
                    let parent = parent.0;
                    if selected.contains(&parent) {
                        return false;
                    }
                    cur = parent;
                }
                true
            })
            .collect()
    }

    /// Serialize the top-most selected subtrees into the entity clipboard.
    /// Returns the number of copied subtree roots.
    fn copy_selection_to_clipboard(&mut self) -> usize {
        let roots = self.topmost_selected_roots();
        if roots.is_empty() {
            return 0;
        }
        let world = self.core.game_world.hecs();
        self.editor.scene.clipboard = roots
            .iter()
            .flat_map(|&r| scene_serializer::serialize_subtree(world, r))
            .collect();
        roots.len()
    }

    /// Delete the selected subtrees as one undoable action ("Delete"/"Cut").
    fn delete_selection_undoable(&mut self, verb: &str) {
        let roots = self.topmost_selected_roots();
        if roots.is_empty() {
            return;
        }
        // Every selected entity is a root or lives inside a deleted subtree.
        self.editor.scene.selection.clear();
        let mut cmd = DeleteSubtreeCommand::new(self.core.game_world.hecs(), &roots, verb);
        cmd.execute(self.core.game_world.hecs_mut());
        let desc = cmd.description().to_string();
        self.editor
            .scene
            .command_history
            .push_executed(Box::new(cmd));
        self.post_history_change();
        self.editor.console.messages.push(LogMessage::info(desc));
    }

    /// Paste the entity clipboard as siblings of the primary selection
    /// (scene roots when nothing is selected), undoable, and select the result.
    fn paste_clipboard(&mut self) {
        use rust_engine::engine::ecs::hierarchy::Parent;
        if self.editor.scene.clipboard.is_empty() {
            return;
        }
        let target_parent = self.editor.scene.selection.primary().and_then(|e| {
            self.core
                .game_world
                .hecs()
                .get::<&Parent>(e)
                .ok()
                .map(|p| p.0)
        });
        let mut cmd = PasteCommand::new(
            self.core.game_world.hecs(),
            self.editor.scene.clipboard.clone(),
            target_parent,
        );
        cmd.execute(self.core.game_world.hecs_mut());
        let pasted = cmd.roots(self.core.game_world.hecs());
        let desc = cmd.description().to_string();
        self.editor
            .scene
            .command_history
            .push_executed(Box::new(cmd));
        self.editor.scene.selection.clear();
        for entity in pasted {
            self.editor.scene.selection.add(entity);
        }
        self.post_history_change();
        self.editor.console.messages.push(LogMessage::info(desc));
    }

    /// After any command that spawns/despawns entities (or its undo/redo):
    /// re-sync hierarchy roots and drop dead entities from the selection.
    fn post_history_change(&mut self) {
        self.editor
            .scene
            .hierarchy_panel
            .sync_root_order(self.core.game_world.hecs());
        let dead: Vec<_> = self
            .editor
            .scene
            .selection
            .all()
            .copied()
            .filter(|&e| !self.core.game_world.hecs().contains(e))
            .collect();
        for entity in dead {
            self.editor.scene.selection.remove(entity);
        }
    }

    fn handle_frame_input(
        &mut self,
        gui_result: &rust_engine::engine::gui::crusty::CrustyLayoutResult,
    ) {
        // Temporarily remove InputManager to avoid borrow conflicts with game_world
        let Some(mut input_manager) = self
            .core
            .game_world
            .resources_mut()
            .remove::<InputManager>()
        else {
            return;
        };

        // Editor shortcuts — suppressed while a crusty text field owns the
        // keyboard (Ctrl+C in a text edit must not copy entities).
        if self.play_mode() == PlayMode::Edit && !gui_result.wants_keyboard {
            if input_manager.is_winit_key_pressed(KeyCode::ControlLeft) {
                let shortcuts = [
                    (KeyCode::KeyZ, EditorAction::Undo),
                    (KeyCode::KeyY, EditorAction::Redo),
                    (KeyCode::KeyX, EditorAction::Cut),
                    (KeyCode::KeyC, EditorAction::Copy),
                    (KeyCode::KeyV, EditorAction::Paste),
                    (KeyCode::KeyD, EditorAction::Duplicate),
                ];
                for (key, action) in shortcuts {
                    if input_manager.is_winit_key_just_pressed(key) {
                        self.handle_editor_action(action);
                    }
                }
            }
            if input_manager.is_winit_key_just_pressed(KeyCode::Delete) {
                self.handle_editor_action(EditorAction::Delete);
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
                && !gui_result.owns_pointer
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
            self.editor.viewport.drag_start_cursor_pos = Some(input_manager.mouse_position());
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

/// Bind open graph tabs to running instances (45-A P7).
///
/// One call per frame for all tabs: the hierarchy selection is the same for
/// every one of them, and the rule ("the selected entity that runs this
/// graph") lives in `scripting::trace::viz_for_selection`. Returns only the
/// tabs that are bound, so an unbound tab is an absent entry rather than an
/// empty one — the panel's `exec: None` path draws nothing at all.
///
/// A build without the interpreter has nothing to bind and says so here, once,
/// instead of `cfg`-ing every call site.
fn graph_exec_bindings(
    world: &hecs::World,
    resources: &rust_engine::engine::ecs::resources::Resources,
    selected: &[hecs::Entity],
    binds: &[GraphDebugTab],
) -> Vec<(String, rust_engine::engine::editor::graph_exec_viz::GraphExecViz)> {
    #[cfg(not(feature = "graph-scripting"))]
    {
        let _ = (world, resources, selected, binds);
        Vec::new()
    }
    #[cfg(feature = "graph-scripting")]
    {
        // The same clock the runner stamps its trace with, so "how long ago"
        // means the same thing on both sides of the seam.
        let now = resources
            .get::<rust_engine::engine::ecs::resources::Time>()
            .map(|t| t.total)
            .unwrap_or(0.0);
        binds
            .iter()
            .filter_map(|tab| {
                let k = tab.path.as_str();
                // An explicit pick wins; a stale one (the entity is gone, or
                // no longer runs this graph) falls back to the selection rule
                // rather than blanking the canvas.
                let viz = tab
                    .bind
                    .and_then(|bits| {
                        rust_engine::engine::scripting::trace::viz_for_entity(
                            world, bits, k, now, &tab.armed,
                        )
                    })
                    .or_else(|| {
                        rust_engine::engine::scripting::trace::viz_for_selection(
                            world,
                            selected.iter().copied(),
                            k,
                            now,
                            &tab.armed,
                        )
                    })?;
                Some((k.to_string(), viz))
            })
            .collect()
    }
}

/// One graph tab's debug state, as the host reads it before drawing (GS-4).
#[cfg(feature = "editor")]
struct GraphDebugTab {
    path: String,
    /// The instance explicitly picked in the LIVE chip, if any.
    bind: Option<u64>,
    /// Armed document nodes — disabled marks never leave the editor.
    armed: Vec<u64>,
    /// What the banner or a shortcut asked for last frame.
    request: Option<rust_engine::engine::editor::graph_exec_viz::DebugRequest>,
}

/// What the host will write into the running instances after the UI (GS-4):
/// the graph, the one instance that gets its breakpoints, and any command.
#[cfg(feature = "editor")]
struct GraphDebugPlan {
    path: String,
    /// `None` = nothing bound, which arms nothing anywhere.
    instance: Option<u64>,
    armed: Vec<u64>,
    request: Option<rust_engine::engine::editor::graph_exec_viz::DebugRequest>,
}

/// Pair each tab with the instance its canvas is actually showing.
///
/// The binding rule already ran (an explicit pick, else the selected entity);
/// reusing its answer is what guarantees the breakpoints go to the instance
/// whose execution the person is looking at, rather than to a second one the
/// arming code picked by its own rule.
#[cfg(feature = "editor")]
fn graph_debug_plan(
    tabs: &[GraphDebugTab],
    exec: &[(String, rust_engine::engine::editor::graph_exec_viz::GraphExecViz)],
) -> Vec<GraphDebugPlan> {
    tabs.iter()
        .map(|t| GraphDebugPlan {
            path: t.path.clone(),
            instance: exec
                .iter()
                .find(|(k, _)| *k == t.path)
                .map(|(_, v)| v.instance_id),
            armed: t.armed.clone(),
            request: t.request,
        })
        .collect()
}

/// Every instance running each open graph, for the LIVE chip's picker (GS-3).
///
/// Distances are measured from the editor camera, which is where the person
/// asking "which Duck is that" is standing.
#[cfg(feature = "editor")]
fn graph_instance_lists<'a>(
    world: &hecs::World,
    resources: &rust_engine::engine::ecs::resources::Resources,
    camera: [f32; 3],
    keys: impl IntoIterator<Item = &'a String>,
) -> std::collections::HashMap<
    String,
    Vec<rust_engine::engine::editor::graph_exec_viz::ExecInstance>,
> {
    #[cfg(not(feature = "graph-scripting"))]
    {
        let _ = (world, resources, camera, keys);
        Default::default()
    }
    #[cfg(feature = "graph-scripting")]
    {
        let now = resources
            .get::<rust_engine::engine::ecs::resources::Time>()
            .map(|t| t.total)
            .unwrap_or(0.0);
        keys.into_iter()
            .map(|k| {
                (
                    k.clone(),
                    rust_engine::engine::scripting::trace::instances_for(
                        world,
                        k,
                        Some(camera),
                        now,
                    ),
                )
            })
            .filter(|(_, v)| !v.is_empty())
            .collect()
    }
}

/// Write one open `.curve` to disk and make every consumer re-read it.
///
/// A free function because two callers need it under different borrows: the
/// docked path holds `&mut self`, the float-window path holds `self`
/// destructured into pieces.
///
/// Three caches, one write. The editor's own resolver map is rebuilt from the
/// open document every frame, so Timeline pins in open graphs already track
/// the edit; the runtime's `CurveCache` and the compiled `GraphPlanCache` are
/// keyed by path and have to be told. The plan cache is cleared wholesale
/// rather than per-graph: a track added or removed changes the pin list of
/// every Timeline that names this curve, and the cache does not track that
/// reference tree (the same reasoning `invalidate` gives for subgraphs).
#[cfg(feature = "editor")]
fn save_curve_state(
    curve_editors: &mut std::collections::HashMap<
        String,
        rust_engine::engine::editor::curve_editor::CurveEditorState,
    >,
    console: &mut ConsoleState,
    world: &mut GameWorld,
    key: &str,
) {
    let Some(st) = curve_editors.get_mut(key) else {
        return;
    };
    // `save` settles its own in-flight gesture, so there is nothing to do
    // here but name the file.
    let abs = std::path::Path::new("content").join(&st.path);
    if let Err(e) = st.save(&abs) {
        console
            .messages
            .push(LogMessage::error(format!("Failed to save curve '{key}': {e}")));
        return;
    }
    #[cfg(feature = "graph-scripting")]
    {
        let res = world.resources_mut();
        if let Some(curves) = res.get_mut::<rust_engine::engine::scripting::CurveCache>() {
            curves.invalidate(key);
        }
        if let Some(plans) = res.get_mut::<rust_engine::engine::scripting::GraphPlanCache>() {
            plans.invalidate_all();
        }
    }
    #[cfg(not(feature = "graph-scripting"))]
    let _ = world;
}

/// Forget every recorded session for the instances running `graph_path`.
///
/// Every instance, not just the bound one: the button is next to a chip that
/// names one entity, but the trace it clears is what the *canvas* shows, and
/// switching instances immediately afterwards should not resurrect the tint
/// the user just dismissed.
#[cfg(all(feature = "editor", feature = "graph-scripting"))]
impl App {
    /// Deliver each graph tab's breakpoints and debugger command to the
    /// instance it is bound to (GS-4).
    ///
    /// Runs after the UI for the same reason "Clear trace" does: it writes to
    /// live instances, and the panel borrows are gone by here. One frame of
    /// latency, on both the arm and the command — a person pressing Resume
    /// cannot perceive it, and the alternative is threading `&mut World`
    /// through the layout pass.
    fn apply_graph_debug(&mut self, plan: &[GraphDebugPlan]) {
        use rust_engine::engine::scripting::trace::arm_debug;
        for tab in plan {
            arm_debug(
                self.core.game_world.hecs_mut(),
                &tab.path,
                tab.instance,
                &tab.armed,
                tab.request,
            );
        }
    }

    fn clear_graph_traces(&mut self, graph_path: &str) {
        use rust_engine::engine::scripting::{normalize_graph_path, runner::GraphRuntime};
        let want = normalize_graph_path(graph_path);
        for (_, rt) in self
            .core
            .game_world
            .hecs_mut()
            .query_mut::<&mut GraphRuntime>()
        {
            if normalize_graph_path(&rt.graph) == want {
                rt.trace.clear();
            }
        }
    }
}

#[cfg(all(feature = "editor", not(feature = "graph-scripting")))]
impl App {
    fn clear_graph_traces(&mut self, _graph_path: &str) {}
    fn apply_graph_debug(&mut self, _plan: &[GraphDebugPlan]) {}
}

/// A tab's watches in the shape the sidecar stores (GS-3).
#[cfg(feature = "editor")]
fn stored_watches(
    state: &rust_engine::engine::editor::graph_editor::GraphEditorState,
) -> Vec<rust_engine::engine::editor::graph_state_store::StoredWatch> {
    use rust_engine::engine::editor::graph_state_store::StoredWatch;
    state
        .watches
        .iter()
        .map(|w| StoredWatch {
            node: w.node,
            pin: w.pin.clone(),
            output: w.output,
            last: w.last.clone(),
        })
        .collect()
}

/// A tab's breakpoints in the shape the sidecar stores (GS-4).
#[cfg(feature = "editor")]
fn stored_breakpoints(
    state: &rust_engine::engine::editor::graph_editor::GraphEditorState,
) -> Vec<rust_engine::engine::editor::graph_state_store::StoredBreakpoint> {
    use rust_engine::engine::editor::graph_state_store::StoredBreakpoint;
    state
        .breakpoints
        .iter()
        .map(|(node, enabled)| StoredBreakpoint { node: *node, enabled: *enabled })
        .collect()
}

/// Restore a graph's remembered pan/zoom and bookmarks from the user-local
/// sidecar. With no entry the tab frames its own content on the first draw —
/// landing on an empty corner of a graph you have never opened is a worse
/// default than seeing all of it.
fn restore_graph_ui_state(
    state: &mut rust_engine::engine::editor::graph_editor::GraphEditorState,
    relative: &str,
) {
    use rust_engine::engine::editor::graph_editor::Watch;
    let store = GraphStateStore::load(std::path::Path::new("content"));
    state.bookmarks = store.bookmarks_for(relative);
    // Watches come back with their last value and *no* freshness: a restored
    // chip is residue from a previous session, and pretending it just updated
    // would be the one lie the staleness tag exists to prevent.
    state.watches = store
        .watches_for(relative)
        .into_iter()
        .map(|w| Watch { last: w.last, ..Watch::new(w.node, &w.pin, w.output) })
        .collect();
    // Breakpoints come back exactly as they were left, armed or disabled
    // (GS-4). A mark whose node has since been deleted is dropped rather than
    // kept as an id nothing draws.
    state.breakpoints = store
        .breakpoints_for(relative)
        .into_iter()
        .filter(|b| state.doc.node(b.node).is_some())
        .map(|b| (b.node, b.enabled))
        .collect();
    match store.view_for(relative) {
        Some(v) => state.view = v,
        None => state.frame_all_on_open = true,
    }
}
