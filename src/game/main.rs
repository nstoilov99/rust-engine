use rust_engine::Renderer;
use std::sync::Arc;
use std::sync::mpsc;
use winit::event::{Event, VirtualKeyCode, WindowEvent, MouseScrollDelta, ElementState};
use winit::event_loop::{ControlFlow, EventLoop};
use rust_engine::{InputManager, Camera2D};
use rust_engine::{AnimationStateMachine, AnimationTransition, TransitionCondition};
use glam::{Mat4, Vec3};
use rust_engine::DirectionalLight;
use rust_engine::assets::{AssetManager, HotReloadWatcher, AsyncAssetLoader, ReloadEvent};
use hecs::World;
use rust_engine::engine::ecs::components::{Transform, MeshRenderer, Camera, Name};
use rust_engine::engine::ecs::components::DirectionalLight as EcsDirectionalLight;
use rust_engine::engine::scene::load_scene;
use nalgebra_glm as glm;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🎮 Rust Game Engine - Starting up...\n");

    let event_loop = EventLoop::new();
    let window = Arc::new(
        winit::window::WindowBuilder::new()
            .with_title("Rust Game Engine")
            .with_inner_size(winit::dpi::LogicalSize::new(800, 600))
            .build(&event_loop)?,
    );

    let mut renderer = Renderer::new(window.clone())?;

    // ========== Asset Manager Setup ==========
    println!("📦 Setting up Asset Manager...");
    let asset_manager = Arc::new(AssetManager::new(
        renderer.device.clone(),
        renderer.queue.clone(),
        renderer.memory_allocator.clone(),
        renderer.command_buffer_allocator.clone(),
    ));

    // Setup hot-reload channel
    let (reload_tx, reload_rx) = mpsc::channel::<ReloadEvent>();

    // Setup hot-reload watcher
    let mut hot_reload = HotReloadWatcher::new(asset_manager.clone(), reload_tx);
    hot_reload.watch_directory("assets/")?;
    hot_reload.track_asset("assets/models/Duck.glb");
    println!("✅ Hot-reload enabled for assets/ directory (auto-reload active!)");

    // Setup async loader
    let _async_loader = AsyncAssetLoader::new(asset_manager.clone());
    println!("✅ Async asset loader ready");

    // Load GLTF model using asset manager (automatically uploads to GPU)
    println!("🦆 Loading Duck model...");
    let (mut mesh_indices, duck_model) = asset_manager.load_model_gpu("assets/models/Duck.glb")?;
    let model = duck_model.get();

    let mut input_manager = InputManager::new();

    println!("🌍 Setting up ECS World...");
    let mut world = World::new();

        // Try to load scene from file, or create default scene
    if std::path::Path::new("assets/scenes/main.scene.ron").exists() {
        println!("📂 Loading scene from file...");
        load_scene(&mut world, "assets/scenes/main.scene.ron")?;
    } else {
        println!("⚠️  No scene file found, creating default scene...");

        // Spawn Camera entity
        world.spawn((
            Transform::new(glm::vec3(0.0, 5.0, 10.0)),
            Camera::default(),
            Name::new("Main Camera"),
        ));

        // Spawn Duck entity (using mesh_indices from AssetManager)
        // Apply 180° rotation around X-axis to flip upside-down models
        let flip_rotation = glm::quat_angle_axis(std::f32::consts::PI, &glm::vec3(1.0, 0.0, 0.0));
        println!("DEBUG: flip_rotation = ({}, {}, {}, {})", flip_rotation.i, flip_rotation.j, flip_rotation.k, flip_rotation.w);
        world.spawn((
            Transform::new(glm::vec3(0.0, 0.0, 0.0))
                .with_rotation(flip_rotation)
                .with_scale(glm::vec3(0.01, 0.01, 0.01)),
            MeshRenderer {
                mesh_index: mesh_indices[0],  // First mesh from Duck model
                material_index: 0,
            },
            Name::new("Duck"),
        ));

        // Spawn Directional Light
        world.spawn((
            EcsDirectionalLight {
                direction: glm::vec3(0.0, -1.0, -1.0),
                color: glm::vec3(1.0, 1.0, 1.0),
                intensity: 1.0,
            },
            Name::new("Sun"),
        ));

        println!("✅ Default scene created with {} entities", world.len());
    }

    // Extract Duck's embedded texture or use white fallback
    use vulkano::image::{Image, ImageCreateInfo, ImageType, ImageUsage};
    use vulkano::image::view::ImageView;
    use vulkano::buffer::{Buffer, BufferCreateInfo, BufferUsage};
    use vulkano::memory::allocator::{AllocationCreateInfo, MemoryTypeFilter};
    use vulkano::command_buffer::{AutoCommandBufferBuilder, CommandBufferUsage, CopyBufferToImageInfo, PrimaryCommandBufferAbstract};
    use vulkano::sync::GpuFuture;
    use vulkano::format::Format;
    use vulkano::image::sampler::{Sampler, SamplerCreateInfo, Filter, SamplerAddressMode};

    // Get duck model from asset manager to extract texture
    let duck_model_handle = asset_manager.models.load("assets/models/Duck.glb")?;
    let duck_model = duck_model_handle.get();

    let (texture_pixels, texture_width, texture_height) = if !duck_model.textures.is_empty() {
        let duck_texture = &duck_model.textures[0];
        println!("🖼️  Using Duck texture: {}x{}", duck_texture.width(), duck_texture.height());
        (duck_texture.clone().into_raw(), duck_texture.width(), duck_texture.height())
    } else {
        println!("⚠️  No textures in model, using white texture");
        (vec![255u8, 255, 255, 255], 1, 1)
    };

    let image = Image::new(
        renderer.memory_allocator.clone(),
        ImageCreateInfo {
            image_type: ImageType::Dim2d,
            format: Format::R8G8B8A8_SRGB,
            extent: [texture_width, texture_height, 1],
            usage: ImageUsage::TRANSFER_DST | ImageUsage::SAMPLED,
            ..Default::default()
        },
        AllocationCreateInfo::default(),
    )?;

    let buffer = Buffer::from_iter(
        renderer.memory_allocator.clone(),
        BufferCreateInfo {
            usage: BufferUsage::TRANSFER_SRC,
            ..Default::default()
        },
        AllocationCreateInfo {
            memory_type_filter: MemoryTypeFilter::HOST_SEQUENTIAL_WRITE,
            ..Default::default()
        },
        texture_pixels,
    )?;

    let mut builder = AutoCommandBufferBuilder::primary(
        renderer.command_buffer_allocator.as_ref(),
        renderer.queue.queue_family_index(),
        CommandBufferUsage::OneTimeSubmit,
    )?;

    builder.copy_buffer_to_image(CopyBufferToImageInfo::buffer_image(
        buffer,
        image.clone(),
    ))?;

    let command_buffer = builder.build()?;
    command_buffer.execute(renderer.queue.clone())?
        .then_signal_fence_and_flush()?
        .wait(None)?;

    let texture_view = ImageView::new_default(image)?;
    let sampler = Sampler::new(
        renderer.device.clone(),
        SamplerCreateInfo {
            mag_filter: Filter::Linear,
            min_filter: Filter::Linear,
            address_mode: [SamplerAddressMode::Repeat; 3],
            ..Default::default()
        },
    )?;

    // Create descriptor set for texture
    use rust_engine::rendering::rendering_2d::pipeline_2d::create_texture_descriptor_set;
    let descriptor_set = create_texture_descriptor_set(
        renderer.descriptor_set_allocator.clone(),
        renderer.pipeline_3d.clone(),
        texture_view,
        sampler,
    )?;

    // Animation state
    let rotation = 0.0f32;
    let mut camera_distance = 5.0f32;

    // Create game loop for delta time
    let mut game_loop = rust_engine::GameLoop::new();
    
    // Camera movement speed
    let camera_speed = 0.1;

    println!("✅ GLTF model loaded and ready to render!");
    println!("Controls:");
    println!("  WASD: Move camera (forward/left/back/right)");
    println!("  Space/Shift: Move up/down");
    println!("  Arrow keys: Look around");
    println!("  1-3: Light controls");
    println!("  R: Reload assets (hot-reload demo)");
    println!("  C: Show cache stats");
    println!("  S: Save scene");
    println!("  ESC: Quit\n");
    println!("💡 TIP: Edit Duck.glb in Blender and save - it will reload automatically!\n");

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Poll;

        match event {
            Event::WindowEvent { event, .. } => match event {
                WindowEvent::CloseRequested => {
                    println!("👋 Closing...");
                    *control_flow = ControlFlow::Exit;
                }
                WindowEvent::Resized(new_size) => {
                    println!("Window resized to {}x{}", new_size.width, new_size.height);
                    renderer.recreate_swapchain = true;
                }
                WindowEvent::KeyboardInput { input: keyboard_input, .. } => {
                    input_manager.handle_keyboard(keyboard_input.virtual_keycode, keyboard_input.state);

                    // Handle ESC for quit
                    if let Some(VirtualKeyCode::Escape) = keyboard_input.virtual_keycode {
                        if keyboard_input.state == ElementState::Pressed {
                            *control_flow = ControlFlow::Exit;
                        }
                    }

                    if input_manager.is_key_pressed(VirtualKeyCode::LControl) {
                           if input_manager.is_key_just_pressed(VirtualKeyCode::S) {
                            use rust_engine::engine::scene::save_scene;
                            match save_scene(&world, "assets/scenes/main.scene.ron", "Main Scene") {
                                Ok(_) => println!("💾 Scene saved!"),
                                Err(e) => eprintln!("❌ Save failed: {}", e),
                            }
                        }
                    }

                    // Free camera movement (WASD)
                    let forward = (renderer.camera_3d.target - renderer.camera_3d.position).normalize();
                    let right = forward.cross(renderer.camera_3d.up).normalize();

                    if input_manager.is_key_pressed(VirtualKeyCode::W) {
                        renderer.camera_3d.position += forward * camera_speed;
                        renderer.camera_3d.target += forward * camera_speed;
                    }
                    if input_manager.is_key_pressed(VirtualKeyCode::S) {
                        renderer.camera_3d.position -= forward * camera_speed;
                        renderer.camera_3d.target -= forward * camera_speed;
                    }
                    if input_manager.is_key_pressed(VirtualKeyCode::A) {
                        renderer.camera_3d.position -= right * camera_speed;
                        renderer.camera_3d.target -= right * camera_speed;
                    }
                    if input_manager.is_key_pressed(VirtualKeyCode::D) {
                        renderer.camera_3d.position += right * camera_speed;
                        renderer.camera_3d.target += right * camera_speed;
                    }
                    if input_manager.is_key_pressed(VirtualKeyCode::Space) {
                        renderer.camera_3d.position += renderer.camera_3d.up * camera_speed;
                        renderer.camera_3d.target += renderer.camera_3d.up * camera_speed;
                    }
                    if input_manager.is_key_pressed(VirtualKeyCode::LShift) {
                        renderer.camera_3d.position -= renderer.camera_3d.up * camera_speed;
                        renderer.camera_3d.target -= renderer.camera_3d.up * camera_speed;
                    }

                    // Camera look around (Arrow keys)
                    let look_speed = 0.05f32;
                    if input_manager.is_key_pressed(VirtualKeyCode::Left) {
                        // Rotate target left around position
                        let direction = renderer.camera_3d.target - renderer.camera_3d.position;
                        let angle = look_speed;
                        let cos = angle.cos();
                        let sin = angle.sin();
                        let new_x = direction.x * cos + direction.z * sin;
                        let new_z = -direction.x * sin + direction.z * cos;
                        renderer.camera_3d.target = renderer.camera_3d.position + Vec3::new(new_x, direction.y, new_z);
                    }
                    if input_manager.is_key_pressed(VirtualKeyCode::Right) {
                        // Rotate target right around position
                        let direction = renderer.camera_3d.target - renderer.camera_3d.position;
                        let angle = -look_speed;
                        let cos = angle.cos();
                        let sin = angle.sin();
                        let new_x = direction.x * cos + direction.z * sin;
                        let new_z = -direction.x * sin + direction.z * cos;
                        renderer.camera_3d.target = renderer.camera_3d.position + Vec3::new(new_x, direction.y, new_z);
                    }
                    if input_manager.is_key_pressed(VirtualKeyCode::Up) {
                        // Look up
                        let direction = renderer.camera_3d.target - renderer.camera_3d.position;
                        let new_y = (direction.y + look_speed).clamp(-1.5, 1.5);
                        renderer.camera_3d.target = renderer.camera_3d.position + Vec3::new(direction.x, new_y, direction.z);
                    }
                    if input_manager.is_key_pressed(VirtualKeyCode::Down) {
                        // Look down
                        let direction = renderer.camera_3d.target - renderer.camera_3d.position;
                        let new_y = (direction.y - look_speed).clamp(-1.5, 1.5);
                        renderer.camera_3d.target = renderer.camera_3d.position + Vec3::new(direction.x, new_y, direction.z);
                    }

                        // Light controls
                    if input_manager.is_key_pressed(VirtualKeyCode::Key1) {
                        // Toggle directional light
                        if renderer.directional_light.is_some() {
                            renderer.directional_light = None;
                            println!("Directional light OFF");
                        } else {
                            renderer.directional_light = Some(DirectionalLight::sun());
                            println!("Directional light ON");
                        }
                    }
                    if input_manager.is_key_pressed(VirtualKeyCode::Key2) {
                        // Increase ambient
                        renderer.ambient_light.intensity = (renderer.ambient_light.intensity + 0.1).min(1.0);
                        println!("Ambient: {:.1}", renderer.ambient_light.intensity);
                    }
                    if input_manager.is_key_pressed(VirtualKeyCode::Key3) {
                        // Decrease ambient
                        renderer.ambient_light.intensity = (renderer.ambient_light.intensity - 0.1).max(0.0);
                        println!("Ambient: {:.1}", renderer.ambient_light.intensity);
                    }

                    // Asset management controls
                    if input_manager.is_key_pressed(VirtualKeyCode::R) {
                        println!("\n🔄 Manual reload requested...");
                        match asset_manager.reload_model_gpu("assets/models/Duck.glb") {
                            Ok((new_indices, _new_model)) => {
                                mesh_indices = new_indices;
                                // TODO: Re-upload texture
                                println!("✅ Duck model reloaded and re-uploaded to GPU");
                            }
                            Err(e) => eprintln!("❌ Reload failed: {}", e),
                        }
                    }
                    if input_manager.is_key_pressed(VirtualKeyCode::C) {
                        let stats = asset_manager.cache_stats();
                        println!("\n📊 Asset Cache Stats: {}", stats);
                    }
                }
                WindowEvent::MouseInput { button, state, .. } => {
                    input_manager.handle_mouse_button(button, state);
                }
                WindowEvent::CursorMoved { position, .. } => {
                    input_manager.handle_mouse_move(position.x as f32, position.y as f32);
                }
                WindowEvent::MouseWheel { delta, .. } => {
                    let scroll = match delta {
                        MouseScrollDelta::LineDelta(_x, y) => y,
                        MouseScrollDelta::PixelDelta(pos) => pos.y as f32 * 0.01,
                    };
                    camera_distance = (camera_distance - scroll).clamp(2.0, 200.0);
                    renderer.camera_3d.orbit(0.0, 0.0, camera_distance);
                }
                _ => {}
            },
            Event::MainEventsCleared => {
                // Check for hot-reload events (non-blocking)
                while let Ok(event) = reload_rx.try_recv() {
                    match event {
                        ReloadEvent::ModelChanged { path, mesh_indices: new_indices, model: _new_model } => {
                            // Update mesh indices in ECS entities
                            for (_entity, mesh_renderer) in world.query_mut::<&mut MeshRenderer>() {
                                if !new_indices.is_empty() {
                                    mesh_renderer.mesh_index = new_indices[0];
                                }
                            }
                            println!("✨ Auto-reload complete: {}", path);
                        }
                        ReloadEvent::TextureChanged { path } => {
                            println!("✨ Texture auto-reloaded: {}", path);
                        }
                        ReloadEvent::ReloadFailed { path, error } => {
                            eprintln!("❌ Auto-reload failed for {}: {}", path, error);
                        }
                    }
                }

                // Update delta time
                let _delta_time = game_loop.tick();

                // Animate rotation (1 radian per second)
                let _rotation_speed = 1.0; // radians/second
                //rotation += rotation_speed * delta_time;

                window.request_redraw();
            }
            Event::RedrawRequested(_) => {
                // Render all entities with MeshRenderer using ECS
                let meshes = asset_manager.meshes.read();
                if let Err(e) = renderer.render_ecs_meshes(
                    &world,
                    &*meshes,
                    descriptor_set.clone(),
                ) {
                    eprintln!("❌ Render error: {:?}", e);
                }
            }
            _ => {}
        }
    });
}

/// Update camera position based on input
fn update_camera(camera: &mut Camera2D, input: &InputManager, delta_time: f32) {
    let speed = 300.0 * delta_time; // 2 world units per second (slower camera movement)

    // WASD movement (winit 0.28 uses VirtualKeyCode)
    let mut movement = glam::Vec2::ZERO;

    if input.is_key_pressed(VirtualKeyCode::W) || input.is_key_pressed(VirtualKeyCode::Up) {
        movement.y -= speed;
    }
    if input.is_key_pressed(VirtualKeyCode::S) || input.is_key_pressed(VirtualKeyCode::Down) {
        movement.y += speed;
    }
    if input.is_key_pressed(VirtualKeyCode::A) || input.is_key_pressed(VirtualKeyCode::Left) {
        movement.x -= speed;
    }
    if input.is_key_pressed(VirtualKeyCode::D) || input.is_key_pressed(VirtualKeyCode::Right) {
        movement.x += speed;
    }

    camera.translate(movement);

    // Mouse wheel zoom
    let scroll = input.scroll_delta();
    if scroll != 0.0 {
        camera.adjust_zoom(scroll);
    }
}

fn create_character_animations() -> AnimationStateMachine {
    let mut fsm = AnimationStateMachine::new("idle");

    // Define states
    fsm.add_state("idle", "idle_anim");
    fsm.add_state("walking", "walk_anim");
    fsm.add_state("running", "run_anim");
    fsm.add_state("jumping", "jump_anim");

    // Add transitions
    fsm.add_transition(AnimationTransition {
        from_state: "idle".to_string(),
        to_state: "walking".to_string(),
        condition: TransitionCondition::OnParameter("speed".to_string(), 0.1),
    });

    fsm.add_transition(AnimationTransition {
        from_state: "walking".to_string(),
        to_state: "running".to_string(),
        condition: TransitionCondition::OnParameter("speed".to_string(), 5.0),
    });

    fsm.add_transition(AnimationTransition {
        from_state: "jumping".to_string(),
        to_state: "idle".to_string(),
        condition: TransitionCondition::OnComplete,
    });

    fsm
}