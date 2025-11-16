pub mod engine;

use engine::Renderer;
use std::sync::Arc;
use winit::event::{Event, VirtualKeyCode, WindowEvent, MouseScrollDelta, ElementState};
use winit::event_loop::{ControlFlow, EventLoop};
use crate::engine::{Transform2D, InputManager, Camera2D, SpriteBatch, Scene, SpriteComponent};
use crate::engine::{SpriteSheet, Animation, AnimationController};
use crate::engine::{AnimationStateMachine, AnimationTransition, TransitionCondition};



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

    let mut input_manager = InputManager::new();

    // Load your idle animation sprite sheet (128×32 = 4 frames of 32×32 each)
    let (texture_view, sampler) = engine::load_texture(
        renderer.device.clone(),
        renderer.queue.clone(),
        &renderer.command_buffer_allocator,
        renderer.memory_allocator.clone(),
        "assets/idle_animation.png",  // Your downloaded 128×32 idle animation
    )?;
    println!("✅ Idle animation texture loaded successfully!");

    // Create sprite batch
    let mut batch = SpriteBatch::new();

    // Register texture (do this once)
    let texture_id = batch.register_texture(renderer.descriptor_set.clone());

    // Create scene
    let mut scene = Scene::new();

    // Define sprite sheet layout for your 128×32 texture with 32×32 frames
    // 128 pixels wide ÷ 32 pixels per frame = 4 frames horizontally
    // 32 pixels tall ÷ 32 pixels per frame = 1 row vertically
    let sprite_sheet = SpriteSheet::new(128.0, 32.0, 32.0, 32.0);

    // Create animation controller
    let mut anim_controller = AnimationController::new();

    // Add idle animation: frames 0-3 (all 4 frames) at 8 FPS, looping
    anim_controller.add_animation(Animation::new("idle", 0, 3, 8.0, true));

    // Start playing idle animation
    anim_controller.play("idle");

    println!("🎬 Idle animation created: 4 frames at 8 FPS");

    // Add animated character entity
    let player = scene.add_entity(
        Transform2D::new([0.0, 0.0], 0.0, [32.0, 32.0]),  // Center, 0.3 world units
        Some(SpriteComponent { texture_id, layer: 10 }),
        Some(anim_controller),
        Some(sprite_sheet.clone()),
    );

    // No background for now - just the animated character

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

                    // Still handle ESC for quit
                    if let Some(VirtualKeyCode::Escape) = keyboard_input.virtual_keycode {
                        if keyboard_input.state == ElementState::Pressed {
                            *control_flow = ControlFlow::Exit;
                        }
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
                    input_manager.handle_mouse_wheel(scroll);
                }
                _ => {}
            },
            Event::RedrawRequested(_) => {
                // Update camera with current input state
                update_camera(&mut renderer.camera, &input_manager, 0.016); // ~60fps

                // Clear input state BEFORE rendering (so scroll doesn't accumulate)
                input_manager.new_frame();

                // Submit scene to batch with UV calculation
                for entity in scene.iter_entities() {
                    if let Some(sprite) = &entity.sprite {
                        let uv_rect = if let (Some(anim), Some(sheet)) = (&entity.animation, &entity.sprite_sheet) {
                            // Animated sprite - get current frame UVs
                            let frame = anim.get_current_frame();
                            let uvs = sheet.get_frame_uvs(frame);
                            [uvs[0].x, uvs[0].y, uvs[3].x, uvs[3].y]  // Convert to [u_min, v_min, u_max, v_max]
                        } else {
                            // Static sprite - use full texture
                            [0.0, 0.0, 0.0, 0.0]
                        };

                        batch.add_sprite_animated(sprite.texture_id, entity.transform, uv_rect);
                    }
                }

                // Render
                if let Err(e) = renderer.render_sprite_batch(&batch) {
                    eprintln!("❌ Render error: {:?}", e);
                }

                // Clear batch after rendering to prevent sprite accumulation
                batch.clear();
            }
            Event::MainEventsCleared => {
                // Update animations (advance to next frame)
                scene.update_animations(0.016);  // 60 FPS = ~0.016 seconds per frame

                if let Some(player_entity) = scene.get_entity_mut(player) {
                    // Move 2 pixels per frame (120 pixels/sec at 60 FPS)
                    player_entity.transform.position[0] += 2.0;
                }

                // Request redraw
                window.request_redraw();
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