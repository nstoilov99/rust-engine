//! Standalone game application (no editor UI)
//!
//! Runs the game with direct swapchain rendering, no UI, no editor panels.

use super::{game_setup, render_loop};
use rust_engine::assets::AssetManager;
use rust_engine::engine::animation::{AnimationPlayer, AnimationUpdateSystem, SkeletonInstance};
use rust_engine::engine::collision::CollisionWorld;
use rust_engine::engine::ecs::access::SystemDescriptor;
use rust_engine::engine::ecs::components::{Camera, Transform, TransformDirty};
use rust_engine::engine::ecs::game_world::GameWorld;
use rust_engine::engine::ecs::hierarchy::{
    Children, HierarchyChanged, Parent, TransformCache, TransformPropagationSystem,
};
use rust_engine::engine::ecs::resources::{EditorState, PlayMode, Time};
use rust_engine::engine::ecs::schedule::{Schedule, Stage};
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
use rust_engine::engine::world::{StreamingCtx, WorldStreamer};
use rust_engine::{GameLoop, InputManager, Renderer};
use std::sync::Arc;
use vulkano::descriptor_set::DescriptorSet;
use winit::event::{ElementState, MouseScrollDelta, WindowEvent};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorGrabMode, Window};

#[allow(dead_code)]
pub struct StandaloneApp {
    pub window: Arc<Window>,
    pub renderer: Renderer,
    pub asset_manager: Arc<AssetManager>,
    pub game_world: GameWorld,
    pub skinning: SkinningBackend,
    pub game_loop: GameLoop,
    pub current_debug_view: DebugView,
    pub _camera_distance: f32,
    pub _mesh_indices: Vec<usize>,
    default_material_set: Arc<DescriptorSet>,
    mesh_data_buffer: Vec<MeshRenderData>,
    shadow_caster_buffer: Vec<MeshRenderData>,
    plankton_emitter_buffer:
        Vec<rust_engine::engine::rendering::frame_packet::PlanktonEmitterFrameData>,
    schedule: Schedule,
    /// The built plugin set — owns the `on_world_loaded` callbacks that run
    /// at every content moment.
    plugin_set: rust_engine::engine::plugins::PluginSet,
    /// Node types registered by plugins. Unused by the runtime today (no
    /// graph evaluator ships yet), but plugin registrations must land
    /// somewhere that outlives startup rather than being dropped.
    #[allow(dead_code)]
    node_registry: std::sync::Arc<rust_engine::engine::node_graph::NodeRegistry>,
    frame_number: u64,
    render_thread: Option<RenderThread>,
    /// Net session (M5); `Some` when launched with `--connect`.
    net: Option<crate::net::NetSession>,
    /// In-game HUD overlay (M7 D7): main-thread layout half; the paint list
    /// crosses to the render thread in the frame packet.
    #[cfg(feature = "hud")]
    hud: rust_engine::engine::gui::crusty::CrustyGui,
    /// Last net status shown in the window title (hud-less builds only —
    /// with the HUD it lives in the connection chip).
    #[cfg(not(feature = "hud"))]
    net_title_status: String,
    materials: crate::asset_resolve::MaterialStore,
    /// M4 runtime streaming (net play on the greybox world); inert for
    /// manifest-less scenes.
    world_streamer: WorldStreamer,
    /// Streamed collision chunks (visual/debug parity; prediction keeps its
    /// own full `ChunkStore`).
    collision: CollisionWorld,
    /// Geometry pipeline layout, kept for deferred material resolution
    /// (M9.6: net sessions load the world after the handshake).
    geom_layout: Arc<vulkano::pipeline::PipelineLayout>,
    /// False while a net session waits for the server-announced scene.
    world_ready: bool,
    plane_mesh_index: usize,
    cube_mesh_index: usize,
    /// Task 41.5 P0: `--stress-anim N` — characters spawned at world load.
    stress_anim: usize,
    /// Task 41.5 P0: `--bench-secs S` — per-frame metric collector.
    bench: Option<crate::bench::BenchRun>,
    /// Task 41.6 D4: true while Escape has released the cursor.
    cursor_released: bool,
    /// Task 41.6 D10: the offline / fallback scene, `--scene <content-relative
    /// path>` or [`OFFLINE_SCENE`].
    offline_scene: String,
}

/// Default offline / fallback scene.
const OFFLINE_SCENE: &str = "scenes/main.scene";

impl StandaloneApp {
    pub fn new(
        window: Arc<Window>,
        mut plugin_set: rust_engine::engine::plugins::PluginSet,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        println!("Rust Game Engine - Starting up (standalone)...");

        let args: Vec<String> = std::env::args().collect();
        let bench_flags = crate::bench::parse_flags(&args);
        let offline_scene = crate::bench::arg_value(&args, "--scene")
            .unwrap_or_else(|| OFFLINE_SCENE.to_string());
        println!("standalone: offline scene '{offline_scene}'");

        let window_config = rust_engine::engine::utils::WindowConfig::load_or_default();
        // Bench runs measure frame time; an fps cap from the configured
        // present mode would flatten the numbers, so force uncapped.
        let present_preference = if bench_flags.bench_secs.is_some() {
            rust_engine::engine::core::SwapchainPresentModePreference::Immediate
        } else {
            window_config.vsync.as_present_preference()
        };
        println!(
            "VSync = {:?} (present mode = {:?})",
            window_config.vsync, present_preference
        );
        let mut renderer = Renderer::new_with_present_mode(window.clone(), present_preference)?;
        let (asset_manager, _hot_reload_stub, _reload_rx_stub) = {
            let asset_manager = Arc::new(AssetManager::new(
                renderer.gpu.device.clone(),
                renderer.gpu.queue.clone(),
                renderer.gpu.memory_allocator.clone(),
                renderer.gpu.command_buffer_allocator.clone(),
            ));
            let (tx, rx) = std::sync::mpsc::channel::<()>();
            (asset_manager, tx, rx)
        };

        let (mesh_indices, plane_mesh_index, cube_mesh_index) =
            game_setup::load_assets(&asset_manager)?;

        let mut game_world = GameWorld::new();

        // Force PlayMode::Playing so RunIfPlaying always returns true
        if let Some(state) = game_world.resource_mut::<EditorState>() {
            state.play_mode = PlayMode::Playing;
        }

        // M9.6: the server announces the scene it simulates (`Config.world_scene`),
        // so a net session starts sceneless and loads the world once the
        // handshake delivers it (`load_world` below). Offline runs load the
        // demo scene right after construction.
        let net = crate::net::NetSession::from_args_or_config(&args);

        game_world.resources_mut().insert(PhysicsWorld::new());
        game_world.resources_mut().insert(TransformCache::new());
        game_world.resources_mut().insert(InputManager::new());
        // Enhanced input system
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
        subsystem.add_context("gameplay");
        game_world.resources_mut().insert(subsystem);
        game_world.resources_mut().insert(ActionState::new());
        game_world
            .resources_mut()
            .insert(rust_engine::engine::ecs::events::Events::<InputEvent>::new());
        if let Some(gamepad_state) = GamepadState::try_new() {
            game_world.resources_mut().insert(gamepad_state);
        }

        let size = window.inner_size();
        let width = size.width.max(1);
        let height = size.height.max(1);

        // Create a temporary DeferredRenderer to extract the geometry pipeline for SkinningBackend
        // and the default material descriptor set. The render thread creates its own
        // DeferredRenderer for actual rendering.
        let (geometry_pipeline, default_material_set) = {
            let tmp = DeferredRenderer::new(
                renderer.gpu.device.clone(),
                renderer.gpu.queue.clone(),
                renderer.gpu.memory_allocator.clone(),
                renderer.gpu.command_buffer_allocator.clone(),
                renderer.gpu.descriptor_set_allocator.clone(),
                width,
                height,
            )?;
            (tmp.geometry_pipeline(), tmp.default_material_set().clone())
        };

        let skinning = SkinningBackend::new(renderer.gpu.memory_allocator.clone())?;

        // Scene-referenced meshes/materials resolve inside `load_world` (once
        // per world load, not per frame like the editor).
        let geom_layout = {
            use vulkano::pipeline::Pipeline;
            geometry_pipeline.layout().clone()
        };
        let materials = crate::asset_resolve::MaterialStore::default();

        // Set camera from first Camera entity, or use default
        {
            let tc = game_world
                .resource::<TransformCache>()
                .expect("TransformCache resource missing");
            Self::sync_camera_from_ecs(
                &mut renderer,
                game_world.hecs(),
                tc,
                width as f32,
                height as f32,
            );
        }

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
        // Task 41: `.animgraph` state machines — same registration as the
        // editor build (see `app.rs`).
        {
            use rust_engine::engine::animation::graph::{
                AnimClipCache, AnimGraphPlanCache, AnimGraphRunner, AnimGraphRuntime,
                AnimGraphSystem, BlendSpaceCache, DiskAnimAssets, IkTargets,
            };
            game_world.resources_mut().insert(AnimGraphPlanCache::new());
            game_world.resources_mut().insert(AnimClipCache::new());
            game_world.resources_mut().insert(BlendSpaceCache::new());
            // Task 41.5 P6: foot placement feeds IK targets from ground
            // raycasts — serial, immediately before the graph system.
            schedule.add_system_described(
                rust_engine::engine::animation::FootPlacementSystem::new(),
                Stage::PreUpdate,
                SystemDescriptor::new(rust_engine::engine::ecs::system_names::FOOT_PLACEMENT)
                    .reads_resource::<Time>()
                    .reads_resource::<rust_engine::engine::physics::PhysicsWorld>()
                    .reads_resource::<TransformCache>()
                    .reads::<Transform>()
                    .reads::<rust_engine::engine::physics::RigidBody>()
                    .reads::<rust_engine::engine::ecs::hierarchy::Parent>()
                    .writes::<AnimGraphRuntime>()
                    .writes::<IkTargets>()
                    .after(rust_engine::engine::ecs::system_names::ANIMATION_UPDATE)
                    .before(rust_engine::engine::ecs::system_names::ANIM_GRAPH),
            );
            let descriptor = || {
                SystemDescriptor::new(rust_engine::engine::ecs::system_names::ANIM_GRAPH)
                    .reads_resource::<Time>()
                    .reads_resource::<rust_engine::engine::animation::graph::AnimViewInfo>()
                    .reads_resource::<TransformCache>()
                    .writes_resource::<AnimGraphPlanCache>()
                    .writes_resource::<AnimClipCache>()
                    .writes_resource::<BlendSpaceCache>()
                    .reads::<AnimGraphRunner>()
                    .reads::<Transform>()
                    .writes::<AnimGraphRuntime>()
                    .writes::<SkeletonInstance>()
                    .after(rust_engine::engine::ecs::system_names::ANIMATION_UPDATE)
            };
            let system = AnimGraphSystem::new(Box::new(DiskAnimAssets {
                content_root: rust_engine::engine::assets::content_root::content_root(),
            }));
            // Task 41.5 P0: with --bench-secs the system is wrapped to record
            // its wall time; without the flag it registers plain (zero cost).
            if bench_flags.bench_secs.is_some() {
                schedule.add_system_described(
                    crate::bench::TimedAnimGraph(system),
                    Stage::PreUpdate,
                    descriptor(),
                );
            } else {
                schedule.add_system_described(system, Stage::PreUpdate, descriptor());
            }
        }
        // `PhysicsStepSystem` is registered by `RapierPhysicsPlugin` below.
        // It carries `RunIfPlaying` there; `StandaloneApp` forces
        // `PlayMode::Playing` at construction precisely so that criteria is
        // always true in a shipped game.
        //
        // Plugins register *before* transform propagation, matching the
        // editor (39.8 D4): gameplay systems that move entities must run
        // before their transforms are propagated. Exports resolve plugin
        // activation at build time, so there is no manifest filter here.
        let mut node_registry = rust_engine::engine::node_graph::NodeRegistry::new();
        plugin_set.build_all(
            rust_engine::engine::plugins::PluginTargets {
                schedule: &mut schedule,
                resources: game_world.resources_mut(),
                node_registry: &mut node_registry,
            },
            None,
        );

        // The registry becomes shared *after* the plugins have filled it:
        // building needs `&mut`, and the graph runner needs a read-only view
        // from inside a system. An `Arc` gives both without a second copy that
        // could drift — nothing mutates it afterwards, because plugin
        // activation is restart-only (39.8).
        let node_registry = std::sync::Arc::new(node_registry);
        game_world
            .resources_mut()
            .insert(std::sync::Arc::clone(&node_registry));

        if let Some(failure) = plugin_set.failures().first() {
            // A shipped game missing a plugin it needs should not limp.
            return Err(format!(
                "plugin '{}' failed during {}: {}",
                failure.id,
                failure.phase.label(),
                failure.error
            )
            .into());
        }

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

        let validation_errors = schedule.validate();
        if !validation_errors.is_empty() {
            // No logger is installed in game_client — log::error! is
            // invisible. Put the errors where the user can see them.
            for err in &validation_errors {
                eprintln!("Schedule validation error: {err}");
            }
            panic!(
                "Schedule validation failed with {} error(s):\n{}",
                validation_errors.len(),
                validation_errors
                    .iter()
                    .map(|e| format!("  - {e}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            );
        }
        schedule.print_access_report();

        #[cfg(feature = "hud")]
        let hud = rust_engine::engine::gui::crusty::CrustyGui::new(
            renderer.gpu.device.clone(),
            [width as f32, height as f32],
        );

        let render_thread = RenderThread::spawn(RenderThreadConfig {
            gpu_context: renderer.gpu.clone(),
            render_mode: rust_engine::engine::rendering::frame_packet::RenderMode::Standalone,
            initial_dimensions: [width, height],
            palette_sync: skinning.sync().clone(),
            swapchain_transfer: Some(
                rust_engine::engine::rendering::render_thread::SwapchainTransfer {
                    surface: renderer.swapchain_state.surface.clone(),
                    swapchain: renderer.swapchain_state.swapchain.clone(),
                    images: renderer.swapchain_state.images.clone(),
                },
            ),
            #[cfg(feature = "editor")]
            viewport_dimensions: None,
            #[cfg(feature = "hud")]
            crusty_text: Some(hud.text_handle()),
        });

        match render_thread.wait_for_ready(std::time::Duration::from_secs(10)) {
            Ok(rust_engine::engine::rendering::frame_packet::RenderEvent::RenderThreadReady {
                ..
            }) => {
                log::info!("standalone: render thread ready");
            }
            Ok(rust_engine::engine::rendering::frame_packet::RenderEvent::RenderError {
                message,
            }) => {
                return Err(format!("render thread init failed: {}", message).into());
            }
            Ok(_) => {
                log::warn!("standalone: unexpected event while waiting for render thread ready");
            }
            Err(e) => {
                return Err(format!("render thread did not become ready: {}", e).into());
            }
        }

        let mut app = Self {
            renderer,
            window,
            asset_manager,
            game_world,
            skinning,
            game_loop: GameLoop::new(),
            current_debug_view: DebugView::None,
            _camera_distance: 5.0,
            _mesh_indices: mesh_indices,
            default_material_set,
            mesh_data_buffer: Vec::with_capacity(64),
            shadow_caster_buffer: Vec::with_capacity(64),
            plankton_emitter_buffer: Vec::with_capacity(32),
            schedule,
            plugin_set,
            node_registry,
            frame_number: 0,
            render_thread: Some(render_thread),
            net,
            #[cfg(feature = "hud")]
            hud,
            #[cfg(not(feature = "hud"))]
            net_title_status: String::new(),
            materials,
            world_streamer: WorldStreamer::default(),
            collision: CollisionWorld::new(),
            geom_layout,
            world_ready: false,
            plane_mesh_index,
            cube_mesh_index,
            stress_anim: bench_flags.stress_anim,
            bench: bench_flags
                .bench_secs
                .map(|s| crate::bench::BenchRun::new(s, bench_flags.stress_anim)),
            cursor_released: true,
            offline_scene,
        };
        // D4: mouse look from the first frame; bench runs are unattended.
        if bench_flags.bench_secs.is_none() {
            app.set_cursor_captured(true);
        }
        if app.net.is_none() {
            let scene = app.offline_scene.clone();
            app.load_world(&scene);
        } else {
            println!("standalone: waiting for server world scene");
        }
        Ok(app)
    }

    /// Load a world into the (empty) ECS: scene + streaming manifest +
    /// physics registration + transform propagation + mesh/material
    /// resolution. Net sessions call this deferred, once the server has
    /// announced its scene (M9.6); offline runs call it at startup.
    fn load_world(&mut self, scene_relative: &str) {
        let scene_loaded = match game_setup::load_or_create_scene(
            self.game_world.hecs_mut(),
            self._mesh_indices[0],
            scene_relative,
        ) {
            Ok((loaded, _roots)) => loaded,
            Err(e) => {
                println!("standalone: failed to load '{scene_relative}': {e}");
                false
            }
        };

        let report = self.world_streamer.load_for_scene(scene_relative);
        if let Some(reason) = &report.disabled {
            println!("standalone: world streaming inert: {reason}");
        }
        for w in &report.warnings {
            println!("standalone: world manifest warning: {w}");
        }

        if !scene_loaded {
            game_setup::spawn_physics_test_objects(
                self.game_world.hecs_mut(),
                self.plane_mesh_index,
                self.cube_mesh_index,
            );
        }

        // Task 41.5 P0: stress characters spawn before mesh/material
        // resolution below so their paths resolve with the scene's.
        if self.stress_anim > 0 {
            crate::bench::spawn_stress_characters(self.game_world.hecs_mut(), self.stress_anim);
        }

        // Content moment (39.8 ruling §5.5): physics registration + plugin
        // `on_world_loaded`. A shipped game treats a failure here as fatal.
        crate::world_population::abort_on_failures(
            &crate::world_population::after_world_populated(
                &mut self.game_world,
                &mut self.plugin_set,
            ),
        );

        let mut transform_cache = self
            .game_world
            .resources_mut()
            .remove::<TransformCache>()
            .unwrap_or_else(TransformCache::new);
        transform_cache.propagate(self.game_world.hecs_mut());
        self.game_world.resources_mut().insert(transform_cache);

        crate::asset_resolve::resolve_mesh_paths(self.game_world.hecs_mut(), &self.asset_manager);
        let gpu = crate::asset_resolve::MaterialGpu {
            allocator: self.renderer.gpu.memory_allocator.clone(),
            ds_allocator: self.renderer.gpu.descriptor_set_allocator.clone(),
            cmd_allocator: self.renderer.gpu.command_buffer_allocator.clone(),
            queue: self.renderer.gpu.queue.clone(),
            device: self.renderer.gpu.device.clone(),
            geom_layout: self.geom_layout.clone(),
        };
        crate::asset_resolve::resolve_material_sets(
            self.game_world.hecs_mut(),
            &self.asset_manager,
            &gpu,
            &mut self.materials,
        );
        self.world_ready = true;
    }

    /// While a net session is world-less: load the server-announced scene as
    /// soon as it arrives; if the scene is not in local content, refuse
    /// (disconnect) and fall back offline; if the connection dies first
    /// (handshake timeout, refusal), fall back offline.
    fn poll_deferred_world(&mut self) {
        enum Decision {
            Wait,
            Load(String),
            MissingScene(String),
            Offline,
        }
        let decision = match &self.net {
            Some(net) => match net.world_scene() {
                Some(scene) if rust_engine::assets::asset_source::exists(scene) => {
                    Decision::Load(scene.to_string())
                }
                Some(scene) => Decision::MissingScene(scene.to_string()),
                None if net.is_disconnected() => Decision::Offline,
                None => Decision::Wait,
            },
            None => Decision::Offline,
        };
        match decision {
            Decision::Wait => {}
            Decision::Load(scene) => {
                println!("standalone: loading server world '{scene}'");
                self.load_world(&scene);
            }
            Decision::MissingScene(scene) => {
                println!(
                    "standalone: server world '{scene}' missing from local content; \
                     disconnecting (client build too old?)"
                );
                if let Some(net) = &mut self.net {
                    net.disconnect();
                }
                let scene = self.offline_scene.clone();
                self.load_world(&scene);
            }
            Decision::Offline => {
                println!("standalone: no server world (connection failed); loading offline scene");
                let scene = self.offline_scene.clone();
                self.load_world(&scene);
            }
        }
    }

    fn sync_camera_from_ecs(
        renderer: &mut Renderer,
        world: &hecs::World,
        cache: &TransformCache,
        width: f32,
        height: f32,
    ) {
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

            renderer.camera_3d.position = pos;
            renderer.camera_3d.target = pos + forward;
            renderer.camera_3d.fov = camera.fov.to_radians();
            renderer.camera_3d.near = camera.near;
            renderer.camera_3d.far = camera.far;
            renderer.camera_3d.set_viewport_size(width, height);
            return;
        }
    }

    pub fn begin_frame(&mut self) {
        puffin::GlobalProfiler::lock().new_frame();
        #[cfg(feature = "tracy")]
        tracy_client::Client::running().map(|c| c.frame_mark());
        if let Some(gp) = self.game_world.resource_mut::<GamepadState>() {
            gp.update();
        }
        self.game_world.begin_frame();
    }

    pub fn end_frame(&mut self) {
        if let Some(im) = self.game_world.resource_mut::<InputManager>() {
            im.clear_transient_state();
        }
    }

    pub fn update(&mut self) {
        if let Some(net) = &mut self.net {
            net.update(&mut self.game_world);
            // With the HUD the status lives in the connection chip; the
            // title keeps only the app name.
            #[cfg(not(feature = "hud"))]
            {
                let status = net.status_line();
                if status != self.net_title_status {
                    self.window.set_title(&format!("Rust Game Engine — {status}"));
                    self.net_title_status = status;
                }
            }
        }
        if !self.world_ready {
            self.poll_deferred_world();
        }
        self.update_world_streaming();
        let delta_time = self.game_loop.tick();

        if let Some(time) = self.game_world.resource_mut::<Time>() {
            time.advance(delta_time);
        }

        // Task 41.5 P4: the previous frame's camera feeds animation
        // significance bucketing (Y-up render space, the same camera the
        // mesh path culls with; absent ⇒ machines evaluate at full rate).
        {
            use rust_engine::engine::animation::graph::AnimViewInfo;
            use rust_engine::engine::math::Frustum;
            let cam = &self.renderer.camera_3d;
            self.game_world.resources_mut().insert(AnimViewInfo {
                camera_pos: cam.position,
                frustum: Frustum::from_view_projection(cam.view_projection_matrix()),
            });
        }

        self.game_world.run_schedule(&mut self.schedule);
    }

    /// Per-frame world streaming around the predicted local player (camera
    /// as fallback until in world). Inert for manifest-less scenes.
    fn update_world_streaming(&mut self) {
        if !self.world_streamer.is_active() {
            return;
        }
        let center = self
            .net
            .as_ref()
            .and_then(|n| n.local_pos())
            .unwrap_or_else(|| {
                rust_engine::engine::utils::coords::convert_position_yup_to_zup(
                    self.renderer.camera_3d.position,
                )
            });
        let allocator = self.renderer.gpu.memory_allocator.clone();
        let mut meshes = self.asset_manager.meshes.write();
        let mut ctx = StreamingCtx {
            world: self.game_world.hecs_mut(),
            meshes: &mut meshes,
            allocator,
            collision: &mut self.collision,
        };
        let output = self.world_streamer.update_streaming(center, &mut ctx);
        if let Some(event) = output.zone_changed {
            println!(
                "standalone: zone changed: {:?} -> {:?}",
                event.previous, event.current
            );
        }
    }

    /// D4: capture (confine, else lock, else give up) hides the cursor and
    /// switches the `look` axis to raw motion; release undoes all three.
    /// Same behaviour as the editor's Play-mode F1 toggle.
    pub fn set_cursor_captured(&mut self, captured: bool) {
        let mode = if captured {
            [CursorGrabMode::Confined, CursorGrabMode::Locked]
                .into_iter()
                .find(|m| self.window.set_cursor_grab(*m).is_ok())
        } else {
            None
        };
        if mode.is_none() {
            let _ = self.window.set_cursor_grab(CursorGrabMode::None);
        }
        self.window.set_cursor_visible(!captured);
        if let Some(im) = self.game_world.resource_mut::<InputManager>() {
            im.set_use_raw_mouse(captured);
        }
        self.cursor_released = !captured;
    }

    pub fn handle_window_event(&mut self, event: &WindowEvent) {
        match event {
            #[cfg_attr(not(feature = "hud"), allow(unused_variables))]
            WindowEvent::Resized(new_size) => {
                self.renderer.swapchain_state.recreate_swapchain = true;
                #[cfg(feature = "hud")]
                self.hud
                    .set_screen_size(new_size.width as f32, new_size.height as f32);
            }
            WindowEvent::KeyboardInput {
                event: key_event, ..
            } => {
                let keycode = match key_event.physical_key {
                    PhysicalKey::Code(code) => Some(code),
                    _ => None,
                };
                // D4: Escape toggles cursor release / recapture.
                if keycode == Some(KeyCode::Escape)
                    && key_event.state == ElementState::Pressed
                    && !key_event.repeat
                {
                    let recapture = self.cursor_released;
                    self.set_cursor_captured(recapture);
                }
                if let Some(im) = self.game_world.resource_mut::<InputManager>() {
                    im.handle_keyboard(keycode, key_event.state);
                }
            }
            WindowEvent::MouseInput { button, state, .. } => {
                if let Some(im) = self.game_world.resource_mut::<InputManager>() {
                    im.handle_mouse_button(*button, *state);
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                if let Some(im) = self.game_world.resource_mut::<InputManager>() {
                    im.handle_mouse_move(position.x as f32, position.y as f32);
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let scroll = match delta {
                    MouseScrollDelta::LineDelta(_x, y) => *y,
                    MouseScrollDelta::PixelDelta(pos) => pos.y as f32 * 0.01,
                };
                if let Some(im) = self.game_world.resource_mut::<InputManager>() {
                    im.handle_mouse_wheel(scroll);
                }
            }
            _ => {}
        }
    }

    pub fn render(&mut self, _window: &Window) -> Result<(), Box<dyn std::error::Error>> {
        // Poll render thread events
        if let Some(ref rt) = self.render_thread {
            for event in rt.poll_events() {
                match &event {
                    rust_engine::engine::rendering::frame_packet::RenderEvent::SwapchainRecreated { dimensions } => {
                        self.renderer.camera_3d.set_viewport_size(
                            dimensions[0] as f32,
                            dimensions[1] as f32,
                        );
                    }
                    rust_engine::engine::rendering::frame_packet::RenderEvent::RenderError { message } => {
                        // eprintln — no logger installed, log:: is invisible.
                        eprintln!("standalone: render thread error: {}", message);
                    }
                    _ => {}
                }
            }
        }

        let size = self.window.inner_size();
        if size.width == 0 || size.height == 0 {
            return Ok(());
        }

        {
            let tc = self
                .game_world
                .resource::<TransformCache>()
                .expect("TransformCache resource missing");
            Self::sync_camera_from_ecs(
                &mut self.renderer,
                self.game_world.hecs(),
                tc,
                size.width as f32,
                size.height as f32,
            );
        }

        let tc = self
            .game_world
            .resource::<TransformCache>()
            .expect("TransformCache resource missing");
        let palette_frame = render_loop::prepare_mesh_data(
            self.game_world.hecs(),
            &self.asset_manager,
            &self.renderer,
            &mut self.mesh_data_buffer,
            &mut self.shadow_caster_buffer,
            tc,
            &mut self.skinning,
            self.frame_number,
            &self.default_material_set,
            &self.materials.cache,
        );
        let light_data = render_loop::prepare_light_data(self.game_world.hecs(), &self.renderer);

        {
            let tc = self
                .game_world
                .resource::<TransformCache>()
                .expect("TransformCache resource missing");
            let dt = self.game_loop.delta();
            render_loop::prepare_plankton_data(
                self.game_world.hecs(),
                &mut self.plankton_emitter_buffer,
                tc,
                dt,
            );
        }

        let view_proj = self.renderer.camera_3d.view_projection_matrix();
        let camera_pos = self.renderer.camera_3d.position;

        let debug_draw_data = rust_engine::engine::debug_draw::DebugDrawData::empty();

        let mut packet = FramePacket::build_standalone(
            std::mem::take(&mut self.mesh_data_buffer),
            std::mem::take(&mut self.shadow_caster_buffer),
            light_data,
            view_proj,
            camera_pos,
            (self.renderer.camera_3d.near, self.renderer.camera_3d.far),
            false,
            debug_draw_data,
            [size.width, size.height],
            self.frame_number,
            std::mem::take(&mut self.plankton_emitter_buffer),
        );
        packet.palette = Some(palette_frame);
        self.frame_number += 1;

        #[cfg(feature = "hud")]
        {
            let state = self.net.as_mut().map(|n| n.hud_state());
            let out = self.hud.layout(|ui| crate::hud::draw(ui, state.as_ref()));
            packet.crusty_paint = Some(out.paint);
        }

        if let Some(ref rt) = self.render_thread {
            if let Err(e) = rt.send(packet) {
                log::error!("standalone: failed to send frame packet: {}", e);
            }
        }

        if let Some(bench) = &mut self.bench {
            bench.end_frame(self.game_loop.delta_ms());
        }

        Ok(())
    }

    /// True once `--bench-secs` wrote its baseline file — the event loop
    /// exits cleanly (code 0).
    pub fn bench_finished(&self) -> bool {
        self.bench.as_ref().is_some_and(|b| b.finished())
    }
}
