//! Standalone game application (no editor UI)
//!
//! Runs the game with direct swapchain rendering, no egui, no editor panels.

use super::{game_setup, render_loop};
use rust_engine::assets::AssetManager;
use rust_engine::engine::ecs::access::SystemDescriptor;
use rust_engine::engine::ecs::components::{Camera, Transform, TransformDirty};
use rust_engine::engine::ecs::game_world::GameWorld;
use rust_engine::engine::ecs::hierarchy::{
    Children, HierarchyChanged, Parent, TransformCache, TransformPropagationSystem,
};
use rust_engine::engine::animation::{AnimationPlayer, AnimationUpdateSystem, SkeletonInstance};
use rust_engine::engine::ecs::resources::{EditorState, PlayMode, Time};
use rust_engine::engine::ecs::schedule::{Schedule, Stage};
use rust_engine::engine::physics::{
    Collider as PhysCollider, PhysicsStepSystem, PhysicsWorld,
    RigidBody as PhysRigidBody, Velocity as PhysVelocity,
};
use rust_engine::engine::rendering::frame_packet::FramePacket;
use rust_engine::engine::rendering::render_thread::{RenderThread, RenderThreadConfig};
use rust_engine::engine::rendering::rendering_3d::deferred_renderer::DebugView;
use rust_engine::engine::rendering::rendering_3d::{DeferredRenderer, MeshRenderData, SkinningBackend};
use rust_engine::{GameLoop, InputManager, Renderer};
use rust_engine::engine::input::action_state::ActionState;
use rust_engine::engine::input::gamepad::GamepadState;
use rust_engine::engine::input::serialization;
use rust_engine::engine::input::enhanced_defaults::default_action_set;
use rust_engine::engine::input::enhanced_serialization;
use rust_engine::engine::input::subsystem::{EnhancedInputSystem, InputSubsystem};
use rust_engine::engine::input::event::InputEvent;
use std::sync::Arc;
use vulkano::descriptor_set::DescriptorSet;
use winit::event::{MouseScrollDelta, WindowEvent};
use winit::keyboard::PhysicalKey;
use winit::window::Window;

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
    pub _descriptor_set: Arc<DescriptorSet>,
    mesh_data_buffer: Vec<MeshRenderData>,
    shadow_caster_buffer: Vec<MeshRenderData>,
    plankton_emitter_buffer: Vec<rust_engine::engine::rendering::frame_packet::PlanktonEmitterFrameData>,
    schedule: Schedule,
    frame_number: u64,
    render_thread: Option<RenderThread>,
}

impl StandaloneApp {
    pub fn new(window: Arc<Window>, plugin: &dyn rust_engine::engine::plugin::GamePlugin) -> Result<Self, Box<dyn std::error::Error>> {
        println!("Rust Game Engine - Starting up (standalone)...");

        let window_config = rust_engine::engine::utils::WindowConfig::load_or_default();
        let present_preference = window_config.vsync.as_present_preference();
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

        let (scene_loaded, _root_entities) =
            game_setup::load_or_create_scene(game_world.hecs_mut(), mesh_indices[0])?;

        if !scene_loaded {
            game_setup::spawn_physics_test_objects(
                game_world.hecs_mut(),
                plane_mesh_index,
                cube_mesh_index,
            );
        }

        let mut physics_world = PhysicsWorld::new();
        game_setup::register_physics_entities(&mut physics_world, game_world.hecs_mut());
        game_world.resources_mut().insert(physics_world);

        let mut transform_cache = TransformCache::new();
        transform_cache.propagate(game_world.hecs_mut());
        game_world.resources_mut().insert(transform_cache);
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

        let descriptor_set = game_setup::upload_model_texture(&renderer, &asset_manager)?;

        let size = window.inner_size();
        let width = size.width.max(1);
        let height = size.height.max(1);

        // Create a temporary DeferredRenderer to extract the geometry pipeline for SkinningBackend.
        // The render thread creates its own DeferredRenderer for actual rendering.
        let geometry_pipeline = {
            let tmp = DeferredRenderer::new(
                renderer.gpu.device.clone(),
                renderer.gpu.queue.clone(),
                renderer.gpu.memory_allocator.clone(),
                renderer.gpu.command_buffer_allocator.clone(),
                renderer.gpu.descriptor_set_allocator.clone(),
                width,
                height,
            )?;
            tmp.geometry_pipeline()
        };

        let skinning = SkinningBackend::new(
            renderer.gpu.memory_allocator.clone(),
            renderer.gpu.descriptor_set_allocator.clone(),
            &geometry_pipeline,
        )?;

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
        schedule.add_system_described(
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
        );
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

        plugin.build(&mut schedule, game_world.resources_mut());
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
            render_mode: rust_engine::engine::rendering::frame_packet::RenderMode::Standalone,
            initial_dimensions: [width, height],
            swapchain_transfer: Some(rust_engine::engine::rendering::render_thread::SwapchainTransfer {
                surface: renderer.swapchain_state.surface.clone(),
                swapchain: renderer.swapchain_state.swapchain.clone(),
                images: renderer.swapchain_state.images.clone(),
            }),
            #[cfg(feature = "editor")]
            viewport_dimensions: None,
        });

        match render_thread.wait_for_ready(std::time::Duration::from_secs(10)) {
            Ok(rust_engine::engine::rendering::frame_packet::RenderEvent::RenderThreadReady { .. }) => {
                log::info!("standalone: render thread ready");
            }
            Ok(rust_engine::engine::rendering::frame_packet::RenderEvent::RenderError { message }) => {
                return Err(format!("render thread init failed: {}", message).into());
            }
            Ok(_) => {
                log::warn!("standalone: unexpected event while waiting for render thread ready");
            }
            Err(e) => {
                return Err(format!("render thread did not become ready: {}", e).into());
            }
        }

        Ok(Self {
            renderer,
            window,
            asset_manager,
            game_world,
            skinning,
            game_loop: GameLoop::new(),
            current_debug_view: DebugView::None,
            _camera_distance: 5.0,
            _mesh_indices: mesh_indices,
            _descriptor_set: descriptor_set,
            mesh_data_buffer: Vec::with_capacity(64),
            shadow_caster_buffer: Vec::with_capacity(64),
            plankton_emitter_buffer: Vec::with_capacity(32),
            schedule,
            frame_number: 0,
            render_thread: Some(render_thread),
        })
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
        if let Some(im) = self.game_world.resource_mut::<InputManager>() {
            im.new_frame();
        }
        if let Some(gp) = self.game_world.resource_mut::<GamepadState>() {
            gp.update();
        }
        self.game_world.begin_frame();
    }

    pub fn update(&mut self) {
        let delta_time = self.game_loop.tick();

        if let Some(time) = self.game_world.resource_mut::<Time>() {
            time.advance(delta_time);
        }

        self.game_world.run_schedule(&mut self.schedule);
    }

    pub fn handle_window_event(&mut self, event: &WindowEvent) {
        match event {
            WindowEvent::Resized(_new_size) => {
                self.renderer.swapchain_state.recreate_swapchain = true;
            }
            WindowEvent::KeyboardInput {
                event: key_event, ..
            } => {
                let keycode = match key_event.physical_key {
                    PhysicalKey::Code(code) => Some(code),
                    _ => None,
                };
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
                        log::error!("standalone: render thread error: {}", message);
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
        render_loop::prepare_mesh_data(
            self.game_world.hecs(),
            &self.asset_manager,
            &self.renderer,
            &mut self.mesh_data_buffer,
            &mut self.shadow_caster_buffer,
            tc,
            &self.skinning,
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

        let packet = FramePacket::build_standalone(
            std::mem::take(&mut self.mesh_data_buffer),
            std::mem::take(&mut self.shadow_caster_buffer),
            light_data,
            view_proj,
            camera_pos,
            false,
            debug_draw_data,
            [size.width, size.height],
            self.frame_number,
            std::mem::take(&mut self.plankton_emitter_buffer),
        );
        self.frame_number += 1;

        if let Some(ref rt) = self.render_thread {
            if let Err(e) = rt.send(packet) {
                log::error!("standalone: failed to send frame packet: {}", e);
            }
        }

        Ok(())
    }
}
