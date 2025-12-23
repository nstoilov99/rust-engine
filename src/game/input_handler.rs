//! Input handling for camera controls, debug views, and hotkeys
//!
//! Uses table-driven approach to reduce code duplication.

use glam::Vec3;
use rust_engine::engine::rendering::rendering_3d::deferred_renderer::DebugView;
use rust_engine::engine::rendering::rendering_3d::DeferredRenderer;
use rust_engine::InputManager;
use rust_engine::Renderer;
use winit::event::VirtualKeyCode;

/// Handle camera movement with WASD + Space/Shift
///
/// Camera uses Y-up coordinate system (Y=up, X=right, Z=forward).
/// Uses table-driven approach to avoid 6 repetitive if-blocks.
pub fn handle_camera_movement(renderer: &mut Renderer, input: &InputManager, speed: f32) {
    let forward = (renderer.camera_3d.target - renderer.camera_3d.position).normalize();
    // Y-up: standard cross product forward × up = right
    let right = forward.cross(Vec3::Y).normalize();
    let up = Vec3::Y;  // Y is up in render space

    // Table-driven: (key, direction multiplier)
    let movements: &[(VirtualKeyCode, Vec3)] = &[
        (VirtualKeyCode::W, forward),
        (VirtualKeyCode::S, -forward),
        (VirtualKeyCode::A, -right),
        (VirtualKeyCode::D, right),
        (VirtualKeyCode::Space, up),      // +Y (up in Y-up)
        (VirtualKeyCode::LShift, -up),    // -Y (down in Y-up)
    ];

    for (key, direction) in movements {
        if input.is_key_pressed(*key) {
            renderer.camera_3d.position += *direction * speed;
            renderer.camera_3d.target += *direction * speed;
        }
    }
}

/// Handle camera rotation with arrow keys
///
/// Y-up: yaw rotates around Y axis, pitch adjusts Y component.
/// Consolidated from 4 separate if-blocks.
pub fn handle_camera_rotation(renderer: &mut Renderer, input: &InputManager, look_speed: f32) {
    let mut yaw_delta = 0.0f32;
    let mut pitch_delta = 0.0f32;

    if input.is_key_pressed(VirtualKeyCode::Left) {
        yaw_delta += look_speed;
    }
    if input.is_key_pressed(VirtualKeyCode::Right) {
        yaw_delta -= look_speed;
    }
    if input.is_key_pressed(VirtualKeyCode::Up) {
        pitch_delta += look_speed;
    }
    if input.is_key_pressed(VirtualKeyCode::Down) {
        pitch_delta -= look_speed;
    }

    if yaw_delta != 0.0 || pitch_delta != 0.0 {
        let direction = renderer.camera_3d.target - renderer.camera_3d.position;

        // Y-up: Apply yaw (horizontal rotation around Y axis)
        let cos = yaw_delta.cos();
        let sin = yaw_delta.sin();
        let new_x = direction.x * cos + direction.z * sin;
        let new_z = -direction.x * sin + direction.z * cos;

        // Y-up: Apply pitch (vertical rotation adjusts Y, clamped to avoid gimbal lock)
        let new_y = (direction.y + pitch_delta).clamp(-1.5, 1.5);

        renderer.camera_3d.target =
            renderer.camera_3d.position + Vec3::new(new_x, new_y, new_z);
    }
}

/// Handle debug view toggles (keys 0-5)
///
/// Uses lookup table instead of 6 repetitive if-blocks.
pub fn handle_debug_views(
    input: &InputManager,
    deferred_renderer: &mut DeferredRenderer,
    current_view: &mut DebugView,
) {
    let view_mappings: &[(VirtualKeyCode, DebugView, &str)] = &[
        (VirtualKeyCode::Key0, DebugView::None, "Normal rendering"),
        (VirtualKeyCode::Key1, DebugView::Position, "Position buffer"),
        (VirtualKeyCode::Key2, DebugView::Normal, "Normal buffer"),
        (VirtualKeyCode::Key3, DebugView::Albedo, "Albedo buffer"),
        (VirtualKeyCode::Key4, DebugView::Material, "Material buffer"),
        (VirtualKeyCode::Key5, DebugView::Depth, "Depth buffer"),
    ];

    for (key, view, name) in view_mappings {
        if input.is_key_just_pressed(*key) {
            *current_view = *view;
            deferred_renderer.set_debug_view(*view);
            println!("Debug: {}", name);
            break;
        }
    }
}

/// Handle mouse wheel zoom
pub fn handle_zoom(
    renderer: &mut Renderer,
    input: &InputManager,
    camera_distance: &mut f32,
) {
    let scroll = input.scroll_delta();
    if scroll != 0.0 {
        *camera_distance = (*camera_distance - scroll).clamp(2.0, 200.0);
        renderer.camera_3d.orbit(0.0, 0.0, *camera_distance);
    }
}
