use super::{game_setup, render_loop};
use rust_engine::assets::AssetManager;
use rust_engine::engine::benchmark::profile::create_profile_sink;
use rust_engine::engine::benchmark::{
    load_or_create_benchmark_scene, print_summary, write_report, BenchmarkConfig,
    BenchmarkMetadata, BenchmarkResults, RenderCounterSnapshot, RenderCounterTotals,
};
use rust_engine::engine::core::SwapchainPresentModePreference;
use rust_engine::engine::ecs::components::{Camera, Transform};
use rust_engine::engine::ecs::game_world::GameWorld;
use rust_engine::engine::ecs::hierarchy::TransformCache;
use rust_engine::engine::ecs::resources::Time;
use rust_engine::engine::physics::PhysicsWorld;
use rust_engine::engine::rendering::rendering_3d::{DeferredRenderer, MeshRenderData};
use rust_engine::engine::rendering::{RenderTarget, ResourceCounters};
use rust_engine::{GameLoop, Renderer};
use std::process::Command;
use std::sync::Arc;
use vulkano::swapchain::PresentMode;
use vulkano::sync::GpuFuture;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{DeviceEvent, DeviceId, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowAttributes, WindowId};

pub fn parse_benchmark_config(args: &[String]) -> Option<BenchmarkConfig> {
    if !args.iter().any(|arg| arg == "--benchmark") {
        return None;
    }

    let mut config = BenchmarkConfig::default();
    if let Some(value) = option_value(args, "--warmup").and_then(|value| value.parse().ok()) {
        config.warmup_frames = value;
    }
    if let Some(value) = option_value(args, "--samples").and_then(|value| value.parse().ok()) {
        config.sample_frames = value;
    }
    if let Some(value) = option_value(args, "--seed").and_then(|value| value.parse().ok()) {
        config.seed = value;
    }
    if let Some(value) = option_value(args, "--entities").and_then(|value| value.parse().ok()) {
        config.entity_count = value;
    }
    if let Some(value) = option_value(args, "--width").and_then(|value| value.parse().ok()) {
        config.resolution[0] = value;
    }
    if let Some(value) = option_value(args, "--height").and_then(|value| value.parse().ok()) {
        config.resolution[1] = value;
    }
    config.uncapped = args.iter().any(|arg| arg == "--uncapped");

    Some(config)
}

pub struct BenchmarkApp {
    window: Option<Arc<Window>>,
    runner: Option<BenchmarkRunner>,
    config: BenchmarkConfig,
}

impl BenchmarkApp {
    pub fn new(config: BenchmarkConfig) -> Self {
        Self {
            window: None,
            runner: None,
            config,
        }
    }
}

impl ApplicationHandler for BenchmarkApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let window = match event_loop.create_window(
            WindowAttributes::default()
                .with_title("Rust Engine Benchmark")
                .with_resizable(false)
                .with_inner_size(LogicalSize::new(
                    self.config.resolution[0],
                    self.config.resolution[1],
                )),
        ) {
            Ok(window) => Arc::new(window),
            Err(error) => {
                eprintln!("Failed to create benchmark window: {error}");
                event_loop.exit();
                return;
            }
        };

        match BenchmarkRunner::new(window.clone(), self.config.clone()) {
            Ok(runner) => {
                self.window = Some(window);
                self.runner = Some(runner);
            }
            Err(error) => {
                eprintln!("Failed to initialize benchmark runner: {error}");
                event_loop.exit();
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(runner) = &mut self.runner else {
            return;
        };

        match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
            }
            WindowEvent::Resized(_) => {
                runner.renderer.swapchain_state.recreate_swapchain = true;
            }
            WindowEvent::RedrawRequested => {
                if let Err(error) = runner.render() {
                    eprintln!("Benchmark render failed: {error}");
                    event_loop.exit();
                    return;
                }

                if runner.is_finished() {
                    event_loop.exit();
                }
            }
            _ => {}
        }
    }

    fn device_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _device_id: DeviceId,
        _event: DeviceEvent,
    ) {
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let Some(runner) = &mut self.runner else {
            return;
        };
        let Some(window) = &self.window else { return };

        if runner.is_finished() {
            event_loop.exit();
            return;
        }

        runner.begin_frame();
        runner.update();
        window.request_redraw();
    }
}

struct BenchmarkRunner {
    window: Arc<Window>,
    renderer: Renderer,
    asset_manager: Arc<AssetManager>,
    game_world: GameWorld,
    deferred_renderer: DeferredRenderer,
    skinning: rust_engine::engine::rendering::rendering_3d::SkinningBackend,
    game_loop: GameLoop,
    previous_frame_end: Option<Box<dyn GpuFuture>>,
    mesh_data_buffer: Vec<MeshRenderData>,
    config: BenchmarkConfig,
    rendered_frames: u32,
    frame_times_ms: Vec<f64>,
    render_counter_totals: RenderCounterTotals,
    resource_counters: ResourceCounters,
    profile_collector:
        Arc<parking_lot::Mutex<rust_engine::engine::benchmark::profile::BenchmarkProfileCollector>>,
    profile_sink_id: Option<puffin::FrameSinkId>,
    finished: bool,
}

impl BenchmarkRunner {
    fn new(
        window: Arc<Window>,
        config: BenchmarkConfig,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        println!(
            "Benchmark mode: {} warmup, {} samples, {} entities, seed {}, uncapped {}",
            config.warmup_frames,
            config.sample_frames,
            config.entity_count,
            config.seed,
            config.uncapped
        );

        let mut renderer = Renderer::new_with_present_mode(
            window.clone(),
            benchmark_present_mode_preference(&config),
        )?;
        let actual_present_mode = renderer.swapchain_state.swapchain.create_info().present_mode;
        if config.uncapped && actual_present_mode != PresentMode::Immediate {
            println!(
                "Benchmark uncapped mode requested, but {:?} was selected instead",
                actual_present_mode
            );
        } else {
            println!("Benchmark present mode: {:?}", actual_present_mode);
        }
        let asset_manager = Arc::new(AssetManager::new(
            renderer.gpu.device.clone(),
            renderer.gpu.queue.clone(),
            renderer.gpu.memory_allocator.clone(),
            renderer.gpu.command_buffer_allocator.clone(),
        ));
        let (_mesh_indices, _plane_mesh_index, cube_mesh_index) =
            game_setup::load_assets(&asset_manager)?;

        let mut game_world = GameWorld::new();
        let mut physics_world = PhysicsWorld::new();
        load_or_create_benchmark_scene(
            game_world.hecs_mut(),
            &mut physics_world,
            &config,
            cube_mesh_index,
        )?;

        let resource_counters =
            ResourceCounters::collect(game_world.hecs(), &asset_manager, &physics_world);

        game_world.resources_mut().insert(physics_world);

        let deferred_renderer = DeferredRenderer::new(
            renderer.gpu.device.clone(),
            renderer.gpu.queue.clone(),
            renderer.gpu.memory_allocator.clone(),
            renderer.gpu.command_buffer_allocator.clone(),
            renderer.gpu.descriptor_set_allocator.clone(),
            config.resolution[0],
            config.resolution[1],
        )?;

        let skinning = rust_engine::engine::rendering::rendering_3d::SkinningBackend::new(
            renderer.gpu.memory_allocator.clone(),
            renderer.gpu.descriptor_set_allocator.clone(),
            &deferred_renderer.geometry_pipeline(),
        )?;

        let mut transform_cache = TransformCache::new();
        transform_cache.propagate(game_world.hecs_mut());
        Self::sync_camera_from_ecs(
            &mut renderer,
            game_world.hecs(),
            &transform_cache,
            config.resolution[0] as f32,
            config.resolution[1] as f32,
        );
        game_world.resources_mut().insert(transform_cache);

        let previous_frame_end = Some(vulkano::sync::now(renderer.gpu.device.clone()).boxed());
        let (profile_collector, profile_sink_id) = create_profile_sink();

        Ok(Self {
            window,
            renderer,
            asset_manager,
            game_world,
            deferred_renderer,
            skinning,
            game_loop: GameLoop::new(),
            previous_frame_end,
            mesh_data_buffer: Vec::with_capacity(1024),
            config,
            rendered_frames: 0,
            frame_times_ms: Vec::new(),
            render_counter_totals: RenderCounterTotals::default(),
            resource_counters,
            profile_collector,
            profile_sink_id: Some(profile_sink_id),
            finished: false,
        })
    }

    fn begin_frame(&mut self) {
        puffin::GlobalProfiler::lock().new_frame();
        #[cfg(feature = "tracy")]
        tracy_client::Client::running().map(|client| client.frame_mark());
        self.game_world.begin_frame();
    }

    fn update(&mut self) {
        rust_engine::profile_scope!("frame_update");
        let delta_time = self.game_loop.tick();
        if let Some(time) = self.game_world.resource_mut::<Time>() {
            time.advance(delta_time);
        }
        {
            rust_engine::profile_scope!("physics_step");
            let mut pw = self
                .game_world
                .resources_mut()
                .remove::<PhysicsWorld>()
                .unwrap_or_default();
            pw.step(delta_time, self.game_world.hecs_mut());
            self.game_world.resources_mut().insert(pw);
        }
    }

    fn render(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        rust_engine::profile_scope!("frame_render");
        {
            let mut tc = self
                .game_world
                .resources_mut()
                .remove::<TransformCache>()
                .unwrap_or_default();
            tc.propagate(self.game_world.hecs_mut());
            self.game_world.resources_mut().insert(tc);
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
            tc,
            &self.skinning,
        );
        let light_data = render_loop::prepare_light_data(self.game_world.hecs(), &self.renderer);

        if let Some(mut prev_future) = self.previous_frame_end.take() {
            prev_future.cleanup_finished();
        }

        if self.renderer.swapchain_state.recreate_swapchain {
            match render_loop::handle_swapchain_recreation(
                &mut self.renderer,
                &mut self.deferred_renderer,
            )? {
                false => {
                    self.previous_frame_end = Some(render_loop::create_now_future(&self.renderer));
                    return Ok(());
                }
                true => {
                    let new_size = self.window.inner_size();
                    self.deferred_renderer
                        .resize(new_size.width, new_size.height)?;
                    self.renderer
                        .camera_3d
                        .set_viewport_size(new_size.width as f32, new_size.height as f32);
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

        let debug_draw_data = rust_engine::engine::debug_draw::DebugDrawData::empty();
        let deferred_cb = self.deferred_renderer.render(
            &self.mesh_data_buffer,
            &light_data,
            RenderTarget::Swapchain {
                image: target_image.clone(),
            },
            false,
            self.renderer.camera_3d.view_projection_matrix(),
            self.renderer.camera_3d.position,
            &debug_draw_data,
            &rust_engine::engine::rendering::rendering_3d::PostProcessingSettings::default(),
        )?;

        let future = {
            rust_engine::profile_scope!("swapchain_present");
            acquire_future
                .then_execute(self.renderer.gpu.queue.clone(), deferred_cb)
                .map_err(|error| format!("Execute error: {error:?}"))?
                .then_swapchain_present(
                    self.renderer.gpu.queue.clone(),
                    vulkano::swapchain::SwapchainPresentInfo::swapchain_image_index(
                        self.renderer.swapchain_state.swapchain.clone(),
                        image_index,
                    ),
                )
                .then_signal_fence_and_flush()
        };

        match future {
            Ok(future) => {
                self.previous_frame_end = Some(future.boxed());
            }
            Err(error) => {
                self.previous_frame_end = Some(render_loop::create_now_future(&self.renderer));
                return Err(format!("Present error: {error:?}").into());
            }
        }

        if self.rendered_frames == self.config.warmup_frames {
            self.profile_collector.lock().start_sampling();
        }
        if self.rendered_frames >= self.config.warmup_frames {
            self.frame_times_ms.push(self.game_loop.delta_ms() as f64);
            self.render_counter_totals
                .accumulate(self.deferred_renderer.render_counters());
        }
        self.rendered_frames += 1;

        if self.frame_times_ms.len() >= self.config.sample_frames as usize {
            puffin::GlobalProfiler::lock().new_frame();
            self.finish()?;
        }

        Ok(())
    }

    fn finish(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        if self.finished {
            return Ok(());
        }

        let metadata = BenchmarkMetadata {
            git_hash: option_env!("GIT_HASH").unwrap_or("unknown").to_string(),
            build_profile: option_env!("BUILD_PROFILE")
                .unwrap_or("unknown")
                .to_string(),
            features: enabled_features(),
            cpu_name: detect_cpu_name(),
            gpu_name: self
                .renderer
                .gpu
                .device
                .physical_device()
                .properties()
                .device_name
                .clone(),
            present_mode: format!("{:?}", self.renderer.swapchain_state.swapchain.create_info().present_mode),
            resolution: self.config.resolution,
            seed: self.config.seed,
        };
        let sample_frames = self.frame_times_ms.len().min(u32::MAX as usize) as u32;
        let results = BenchmarkResults::compute(
            metadata,
            self.frame_times_ms.clone(),
            RenderCounterSnapshot::from_average(&self.render_counter_totals, sample_frames),
            self.resource_counters.clone(),
            self.profile_collector.lock().category_averages(),
        );
        let path = write_report(&results)?;
        print_summary(&path, &results);

        if let Some(sink_id) = self.profile_sink_id.take() {
            puffin::GlobalProfiler::lock().remove_sink(sink_id);
        }

        self.finished = true;
        Ok(())
    }

    fn is_finished(&self) -> bool {
        self.finished
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
            let position =
                glam::Vec3::new(render_mat[(0, 3)], render_mat[(1, 3)], render_mat[(2, 3)]);
            let forward = glam::Vec3::new(
                -render_mat[(0, 2)],
                -render_mat[(1, 2)],
                -render_mat[(2, 2)],
            );

            renderer.camera_3d.position = position;
            renderer.camera_3d.target = position + forward;
            renderer.camera_3d.fov = camera.fov.to_radians();
            renderer.camera_3d.near = camera.near;
            renderer.camera_3d.far = camera.far;
            renderer.camera_3d.set_viewport_size(width, height);
            return;
        }
    }
}

impl Drop for BenchmarkRunner {
    fn drop(&mut self) {
        if let Some(sink_id) = self.profile_sink_id.take() {
            puffin::GlobalProfiler::lock().remove_sink(sink_id);
        }
    }
}

fn option_value(args: &[String], name: &str) -> Option<String> {
    let prefix = format!("{name}=");
    for (index, arg) in args.iter().enumerate() {
        if arg == name {
            return args.get(index + 1).cloned();
        }
        if let Some(value) = arg.strip_prefix(&prefix) {
            return Some(value.to_string());
        }
    }
    None
}

fn benchmark_present_mode_preference(config: &BenchmarkConfig) -> SwapchainPresentModePreference {
    if config.uncapped {
        SwapchainPresentModePreference::Immediate
    } else {
        SwapchainPresentModePreference::Default
    }
}

fn enabled_features() -> Vec<String> {
    let mut features = Vec::new();
    if cfg!(feature = "editor") {
        features.push("editor".to_string());
    }
    if cfg!(feature = "tracy") {
        features.push("tracy".to_string());
    }
    features
}

fn detect_cpu_name() -> String {
    #[cfg(target_os = "windows")]
    {
        if let Ok(output) = Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                "(Get-CimInstance Win32_Processor | Select-Object -First 1 -ExpandProperty Name)",
            ])
            .output()
        {
            if output.status.success() {
                if let Ok(name) = String::from_utf8(output.stdout) {
                    let name = name.trim();
                    if !name.is_empty() {
                        return name.to_string();
                    }
                }
            }
        }
    }

    "unknown".to_string()
}
