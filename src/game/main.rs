use rust_engine::Renderer;
use std::sync::Arc;
use winit::event::{Event, VirtualKeyCode, WindowEvent, MouseScrollDelta, ElementState};
use winit::event_loop::{ControlFlow, EventLoop};
use rust_engine::{InputManager, Camera2D};
use rust_engine::{AnimationStateMachine, AnimationTransition, TransitionCondition};
use rust_engine::DirectionalLight;
use glam::{Mat4, Vec3};

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

    // Load GLTF model
    println!("🦆 Loading Duck model...");
    let model = rust_engine::load_model("assets/models/Duck.glb")?;

    // Create mesh manager and upload model to GPU
    let mut mesh_manager = rust_engine::MeshManager::new();
    let mesh_indices = mesh_manager.upload_model(&model, renderer.memory_allocator.clone())?;

    let mut input_manager = InputManager::new();

    // Load your idle animation sprite sheet (128×32 = 4 frames of 32×32 each)
    let (texture_view, sampler) = rust_engine::load_texture(
        renderer.device.clone(),
        renderer.queue.clone(),
        &renderer.command_buffer_allocator,
        renderer.memory_allocator.clone(),
        "assets/sprite.png",  // Your downloaded 128×32 idle animation
    )?;
    // Create descriptor set for texture
    use rust_engine::rendering::rendering_2d::pipeline_2d::create_texture_descriptor_set;
    let descriptor_set = create_texture_descriptor_set(
        renderer.descriptor_set_allocator.clone(),
        renderer.pipeline_3d.clone(),  // Pass the pipeline, not the layout
        texture_view,
        sampler,
    )?;

    // Create cube mesh
    let (cube_vertices, cube_indices) = rust_engine::create_cube();

    // Animation state
    let mut rotation = 0.0f32;
    let mut camera_distance = 5.0f32;

    // Create game loop for delta time
    let mut game_loop = rust_engine::GameLoop::new();
    
    println!("✅ GLTF model loaded and ready to render!");
    println!("Controls:");
    println!("  Arrow keys: Orbit camera");
    println!("  Mouse wheel: Zoom in/out");
    println!("  ESC: Quit\n");

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

                    // Camera orbit
                    if input_manager.is_key_pressed(VirtualKeyCode::Left) {
                        renderer.camera_3d.orbit(0.1, 0.0, camera_distance);
                    }
                    if input_manager.is_key_pressed(VirtualKeyCode::Right) {
                        renderer.camera_3d.orbit(-0.1, 0.0, camera_distance);
                    }
                    if input_manager.is_key_pressed(VirtualKeyCode::Up) {
                        renderer.camera_3d.orbit(0.0, 0.1, camera_distance);
                    }
                    if input_manager.is_key_pressed(VirtualKeyCode::Down) {
                        renderer.camera_3d.orbit(0.0, -0.1, camera_distance);
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
                // Update delta time
                let delta_time = game_loop.tick();

                // Animate rotation (1 radian per second)
                let rotation_speed = 1.0; // radians/second
                //rotation += rotation_speed * delta_time;


                window.request_redraw();
            }
            Event::RedrawRequested(_) => {
                // Create model matrix using Z-up coordinates
                // In Z-up: rotate around Z axis (up)
                // Rotate 180 degrees around X axis to flip upside-down models
                let model = Mat4::from_rotation_x(std::f32::consts::PI)
                    * Mat4::from_scale(Vec3::splat(1.0))
                    * Mat4::from_rotation_y(rotation);

                // Render all meshes from the model WITH LIGHTING
                for &mesh_index in &mesh_indices {
                    if let Some(mesh) = mesh_manager.get(mesh_index) {
                        if let Err(e) = renderer.render_mesh_lit(
                            &cube_vertices,
                            &cube_indices,     // Mesh indices
                            model,               // Model transform
                            descriptor_set.clone(), // Texture
                            0.0,                 // Metallic (0.0 = non-metal, like plastic/wood)
                            0.5,                 // Roughness (0.5 = semi-glossy)
                        ) {
                            eprintln!("❌ Render error: {:?}", e);
                        }
                    }
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