pub mod engine;

use engine::Renderer;
use std::sync::Arc;
use winit::event::{Event, VirtualKeyCode, WindowEvent, MouseScrollDelta, ElementState};
use winit::event_loop::{ControlFlow, EventLoop};
use crate::engine::{Transform2D, InputManager, Camera2D, SpriteBatch, Scene, SpriteComponent};


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

    let (texture_view, sampler) = engine::load_texture(
        renderer.device.clone(),
        renderer.queue.clone(),
        &renderer.command_buffer_allocator,
        renderer.memory_allocator.clone(),
        "assets/sprite.png",  // Put a test image here
    )?;
    println!("Texture loaded successfully!");

    // Create sprite batch
    let mut batch = SpriteBatch::new();

    // Register texture (do this once)
    let texture_id = batch.register_texture(renderer.descriptor_set.clone());

    // Create scene
    let mut scene = Scene::new();


    // Add entities with proper world coordinates
    // Camera shows ±1.0 vertically, and ±(aspect) horizontally
    // Use small scales so sprites are visible

    let player = scene.add_entity(
        Transform2D::new([0.0, 0.0], 0.0, [0.3, 0.3]),  // Center, 0.3 world units
        Some(SpriteComponent { texture_id, layer: 10 })
    );

    let enemy1 = scene.add_entity(
        Transform2D::new([0.6, 0.4], 0.0, [0.25, 0.25]),  // Top-right, smaller
        Some(SpriteComponent { texture_id, layer: 5 })
    );

    let background = scene.add_entity(
        Transform2D::new([-0.6, -0.4], 0.0, [0.2, 0.2]),  // Bottom-left, smallest
        Some(SpriteComponent { texture_id, layer: 0 })  // Layer 0 = drawn first (back)
    );

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
                update_camera(&mut renderer.camera, &input_manager, 0.016); // ~60fps

                // Prepare batch for rendering
                batch.clear();
                scene.submit_to_batch(&mut batch);

                // Render the batch
                if let Err(e) = renderer.render_sprite_batch(&batch) {
                    eprintln!("❌ Render error: {:?}", e);
                }

                // Clear input state after processing
                input_manager.new_frame();
            }
            Event::MainEventsCleared => {
                // Update entities (game logic goes here)
                if let Some(player_entity) = scene.get_entity_mut(player) {
                    // Example: Move player slowly (0.01 world units per frame at 60fps = ~0.6 units/sec)
                    player_entity.transform.position[0] += 0.01;
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
    let speed = 2.0 * delta_time; // 2 world units per second (slower camera movement)

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