//! Standalone game application (no editor UI)
//!
//! Runs the game with direct swapchain rendering, no egui, no editor panels.

use super::{game_setup, input_handler, render_loop};
use rust_engine::assets::AssetManager;
use rust_engine::engine::ecs::game_world::GameWorld;
use rust_engine::engine::ecs::resources::{PlayMode, EditorState, Time};
use rust_engine::engine::physics::PhysicsWorld;
use rust_engine::engine::rendering::rendering_3d::deferred_renderer::DebugView;
use rust_engine::engine::rendering::rendering_3d::{DeferredRenderer, MeshRenderData};
use rust_engine::engine::rendering::RenderTarget;
use rust_engine::engine::ecs::components::{Camera, Transform};
use rust_engine::engine::ecs::hierarchy::get_world_transform;
use rust_engine::engine::adapters::render_adapter::world_matrix_to_render;
use rust_engine::{GameLoop, InputManager, Renderer};
use std::sync::Arc;
use vulkano::descriptor_set::DescriptorSet;
use vulkano::sync::GpuFuture;
use winit::event::{MouseScrollDelta, WindowEvent};
use winit::keyboard::PhysicalKey;
use winit::window::Window;

pub struct StandaloneApp {
    pub window: Arc<Window>,
    pub renderer: Renderer,
    pub asset_manager: Arc<AssetManager>,
    pub game_world: GameWorld,
    pub physics_world: PhysicsWorld,
    pub input_manager: InputManager,
    pub deferred_renderer: DeferredRenderer,
    pub game_loop: GameLoop,
    pub current_debug_view: DebugView,
    pub _camera_distance: f32,
    pub _mesh_indices: Vec<usize>,
    pub _descriptor_set: Arc<DescriptorSet>,
    pub previous_frame_end: Option<Box<dyn GpuFuture>>,
    mesh_data_buffer: Vec<MeshRenderData>,
}

impl StandaloneApp {
    pub fn new(window: Arc<Window>) -> Result<Self, Box<dyn std::error::Error>> {
        println!("Rust Game Engine - Starting up (standalone)...");

        let mut renderer = Renderer::new(window.clone())?;
        let (asset_manager, _hot_reload_stub, _reload_rx_stub) = {
            let asset_manager = Arc::new(AssetManager::new(
                renderer.device.clone(),
                renderer.queue.clone(),
                renderer.memory_allocator.clone(),
                renderer.command_buffer_allocator.clone(),
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
            game_setup::spawn_physics_test_objects(game_world.hecs_mut(), plane_mesh_index, cube_mesh_index);
        }

        let mut physics_world = PhysicsWorld::new();
        game_setup::register_physics_entities(&mut physics_world, game_world.hecs_mut());

        let descriptor_set = game_setup::upload_model_texture(&renderer, &asset_manager)?;

        let size = window.inner_size();
        let width = size.width.max(1);
        let height = size.height.max(1);

        let deferred_renderer = DeferredRenderer::new(
            renderer.device.clone(),
            renderer.queue.clone(),
            renderer.memory_allocator.clone(),
            renderer.command_buffer_allocator.clone(),
            renderer.descriptor_set_allocator.clone(),
            width,
            height,
        )?;

        // Set camera from first Camera entity, or use default
        Self::sync_camera_from_ecs(&mut renderer, game_world.hecs(), width as f32, height as f32);

        let previous_frame_end: Option<Box<dyn GpuFuture>> =
            Some(vulkano::sync::now(renderer.device.clone()).boxed());

        Ok(Self {
            renderer,
            window,
            asset_manager,
            game_world,
            physics_world,
            input_manager: InputManager::new(),
            deferred_renderer,
            game_loop: GameLoop::new(),
            current_debug_view: DebugView::None,
            _camera_distance: 5.0,
            _mesh_indices: mesh_indices,
            _descriptor_set: descriptor_set,
            previous_frame_end,
            mesh_data_buffer: Vec::with_capacity(64),
        })
    }

    fn sync_camera_from_ecs(renderer: &mut Renderer, world: &hecs::World, width: f32, height: f32) {
        for (_entity, (_transform, camera)) in world.query::<(&Transform, &Camera)>().iter() {
            if !camera.active {
                continue;
            }
            let world_mat = get_world_transform(world, _entity);
            let render_mat = world_matrix_to_render(&world_mat);

            let pos = glam::Vec3::new(render_mat[(0, 3)], render_mat[(1, 3)], render_mat[(2, 3)]);
            let forward = glam::Vec3::new(-render_mat[(0, 2)], -render_mat[(1, 2)], -render_mat[(2, 2)]);

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
        self.input_manager.new_frame();
        self.game_world.begin_frame();
    }

    pub fn update(&mut self) {
        let delta_time = self.game_loop.tick();

        if let Some(time) = self.game_world.resource_mut::<Time>() {
            time.advance(delta_time);
        }

        self.physics_world.step(delta_time, self.game_world.hecs_mut());
    }

    pub fn handle_window_event(&mut self, event: &WindowEvent) {
        match event {
            WindowEvent::Resized(_new_size) => {
                self.renderer.recreate_swapchain = true;
            }
            WindowEvent::KeyboardInput { event: key_event, .. } => {
                let keycode = match key_event.physical_key {
                    PhysicalKey::Code(code) => Some(code),
                    _ => None,
                };
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

    pub fn render(&mut self, _window: &Window) -> Result<(), Box<dyn std::error::Error>> {
        // Sync camera from ECS entities
        let size = self.window.inner_size();
        Self::sync_camera_from_ecs(
            &mut self.renderer,
            self.game_world.hecs(),
            size.width as f32,
            size.height as f32,
        );

        render_loop::prepare_mesh_data(
            self.game_world.hecs(),
            &self.asset_manager,
            &self.renderer,
            &mut self.mesh_data_buffer,
        );
        let light_data = render_loop::prepare_light_data(self.game_world.hecs(), &self.renderer);

        if let Some(mut prev_future) = self.previous_frame_end.take() {
            prev_future.cleanup_finished();
        }

        if self.renderer.recreate_swapchain {
            match render_loop::handle_swapchain_recreation(
                &mut self.renderer,
                &mut self.deferred_renderer,
            ) {
                Ok(false) => {
                    self.previous_frame_end = Some(render_loop::create_now_future(&self.renderer));
                    return Ok(());
                }
                Ok(true) => {
                    let new_size = self.window.inner_size();
                    if new_size.width > 0 && new_size.height > 0 {
                        if let Err(e) = self.deferred_renderer.resize(new_size.width, new_size.height) {
                            log::error!("Failed to resize deferred renderer: {}", e);
                        }
                        self.renderer.camera_3d.set_viewport_size(
                            new_size.width as f32,
                            new_size.height as f32,
                        );
                    }
                }
                Err(e) => {
                    self.previous_frame_end = Some(render_loop::create_now_future(&self.renderer));
                    return Err(e);
                }
            }
        }

        let (image_index, target_image, acquire_future) =
            match render_loop::acquire_swapchain_image(&mut self.renderer) {
                Ok(result) => result,
                Err(_) => {
                    self.previous_frame_end = Some(render_loop::create_now_future(&self.renderer));
                    return Ok(());
                }
            };

        let view_proj = self.renderer.camera_3d.view_projection_matrix();
        let camera_pos = self.renderer.camera_3d.position;

        let render_target = RenderTarget::Swapchain { image: target_image.clone() };

        let deferred_cb = match self.deferred_renderer.render(
            &self.mesh_data_buffer,
            &light_data,
            render_target,
            false,
            view_proj,
            camera_pos,
        ) {
            Ok(cb) => cb,
            Err(e) => {
                log::error!("Render error: {}", e);
                self.previous_frame_end = Some(render_loop::create_now_future(&self.renderer));
                return Ok(());
            }
        };

        input_handler::handle_debug_views(
            &self.input_manager,
            &mut self.deferred_renderer,
            &mut self.current_debug_view,
        );

        let future = acquire_future
            .then_execute(self.renderer.queue.clone(), deferred_cb)
            .map_err(|e| format!("Execute error: {:?}", e))?
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
                self.previous_frame_end = Some(render_loop::create_now_future(&self.renderer));
            }
        }

        Ok(())
    }
}
