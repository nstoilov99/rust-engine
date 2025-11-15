pub mod engine;

use engine::Renderer;
use std::sync::Arc;
use winit::event::{Event, VirtualKeyCode, WindowEvent, MouseScrollDelta, ElementState};
use winit::event_loop::{ControlFlow, EventLoop};
use crate::engine::{Transform2D, InputManager, Camera2D};

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

        // Create transforms for 3 sprites
    let sprites = vec![
        // Sprite 1: Center, no rotation
        (Transform2D::at_position(0.0, 0.0), renderer.descriptor_set.clone()),

        // Sprite 2: Right, rotated 45 degrees
        (Transform2D::new([0.5, 0.0], std::f32::consts::PI / 4.0, [0.5, 0.5]), renderer.descriptor_set.clone()),

        // Sprite 3: Left, scaled 2x
        (Transform2D::new([-0.5, 0.0], 0.0, [2.0, 2.0]), renderer.descriptor_set.clone()),
    ];

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

                if let Err(e) = renderer.render_sprites(&sprites) {
                    eprintln!("❌ Render error: {:?}", e);
                }

                input_manager.new_frame(); // Clear input state after processing
            }
            Event::MainEventsCleared => {
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