//! Editor camera with Unreal-style viewport controls
//!
//! Supports multiple control modes:
//! - Fly: RMB + WASD/QE for movement, mouse for look
//! - Orbit: Alt + LMB around target/selection
//! - Pan: MMB or LMB+RMB
//! - Dolly: Scroll wheel

use glam::{Mat4, Quat, Vec3};
use winit::event::MouseButton;
use winit::keyboard::KeyCode;

use crate::engine::input::InputManager;

/// Camera control mode based on input combination
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CameraControlMode {
    #[default]
    None,
    /// RMB held: WASD/QE movement + mouse look
    Fly,
    /// Alt+LMB: Orbit around target
    Orbit,
    /// MMB or LMB+RMB: Pan camera
    Pan,
    /// LMB drag: Move forward/back + rotate left/right
    LookDrag,
}

/// Editor camera with Unreal-style controls
///
/// Works in Y-up render space (Vulkan native).
/// Position and target are stored in Y-up coordinates.
pub struct EditorCamera {
    /// Camera position (Y-up render space)
    pub position: Vec3,
    /// What the camera looks at / orbits around (Y-up render space)
    pub target: Vec3,
    /// Up direction (always Vec3::Y in Y-up space)
    pub up: Vec3,
    /// Field of view in radians
    pub fov: f32,
    /// Viewport aspect ratio (width / height)
    pub aspect_ratio: f32,
    /// Near clip plane
    pub near: f32,
    /// Far clip plane
    pub far: f32,

    // Control settings
    /// Fly speed multiplier (adjusted via scroll while RMB held)
    pub fly_speed_multiplier: f32,
    /// Mouse sensitivity for rotation
    pub mouse_sensitivity: f32,
    /// Distance from camera to orbit target
    orbit_distance: f32,

    // Input state tracking
    current_mode: CameraControlMode,
    /// Track if we consumed input this frame
    consumed_input: bool,
}

impl EditorCamera {
    /// Creates a new editor camera
    pub fn new(viewport_width: f32, viewport_height: f32) -> Self {
        Self {
            // Y-up: elevated 5 units in +Y, back 10 units in +Z
            position: Vec3::new(0.0, 5.0, 10.0),
            target: Vec3::ZERO,
            up: Vec3::Y,
            fov: 45.0_f32.to_radians(),
            aspect_ratio: viewport_width / viewport_height,
            near: 0.1,
            far: 1000.0,
            fly_speed_multiplier: 1.0,
            mouse_sensitivity: 0.003,
            orbit_distance: 10.0,
            current_mode: CameraControlMode::None,
            consumed_input: false,
        }
    }

    /// Updates aspect ratio when viewport resizes
    pub fn set_viewport_size(&mut self, width: f32, height: f32) {
        if width > 0.0 && height > 0.0 {
            self.aspect_ratio = width / height;
        }
    }

    /// Creates view matrix (world → camera space)
    pub fn view_matrix(&self) -> Mat4 {
        Mat4::look_at_rh(self.position, self.target, self.up)
    }

    /// Creates perspective projection matrix (camera → clip space)
    ///
    /// Includes Vulkan Y-flip for correct NDC orientation.
    /// Use this for Vulkan rendering.
    pub fn projection_matrix(&self) -> Mat4 {
        let mut proj = Mat4::perspective_rh(self.fov, self.aspect_ratio, self.near, self.far);
        // Vulkan Y-flip: NDC Y is inverted compared to OpenGL
        proj.y_axis.y *= -1.0;
        proj
    }

    /// Creates projection matrix for egui gizmo (no Y-flip)
    ///
    /// The gizmo library uses OpenGL/egui conventions where Y increases upward.
    /// Use this for gizmo rendering, NOT for Vulkan rendering.
    pub fn projection_matrix_for_gizmo(&self) -> Mat4 {
        // Standard perspective projection without Vulkan Y-flip
        Mat4::perspective_rh(self.fov, self.aspect_ratio, self.near, self.far)
    }

    /// Combined view-projection matrix
    pub fn view_projection_matrix(&self) -> Mat4 {
        self.projection_matrix() * self.view_matrix()
    }

    /// Returns the current control mode
    pub fn current_mode(&self) -> CameraControlMode {
        self.current_mode
    }

    /// Returns true if camera consumed input this frame
    pub fn consumed_input(&self) -> bool {
        self.consumed_input
    }

    /// Process input and update camera. Returns true if camera consumed input.
    ///
    /// # Arguments
    /// * `input` - Input manager with current input state
    /// * `delta_time` - Time since last frame in seconds
    /// * `viewport_hovered` - Whether mouse is over the viewport
    /// * `gizmo_active` - Whether gizmo is currently being manipulated
    /// * `camera_speed` - Camera speed from viewport settings (overrides internal fly_speed)
    pub fn update(
        &mut self,
        input: &InputManager,
        delta_time: f32,
        viewport_hovered: bool,
        gizmo_active: bool,
        camera_speed: f32,
    ) -> bool {
        self.consumed_input = false;

        // Don't process if gizmo is being manipulated
        if gizmo_active {
            self.current_mode = CameraControlMode::None;
            return false;
        }

        if !viewport_hovered {
            self.current_mode = CameraControlMode::None;
            return false;
        }

        let rmb = input.is_mouse_pressed(MouseButton::Right);
        let lmb = input.is_mouse_pressed(MouseButton::Left);
        let mmb = input.is_mouse_pressed(MouseButton::Middle);
        let alt = input.is_key_pressed(KeyCode::AltLeft) || input.is_key_pressed(KeyCode::AltRight);

        // Determine control mode based on input combination
        let new_mode = if rmb && lmb {
            CameraControlMode::Pan
        } else if rmb {
            CameraControlMode::Fly
        } else if alt && lmb {
            CameraControlMode::Orbit
        } else if mmb {
            CameraControlMode::Pan
        } else if lmb {
            CameraControlMode::LookDrag
        } else {
            CameraControlMode::None
        };

        self.current_mode = new_mode;

        match self.current_mode {
            CameraControlMode::Fly => {
                self.process_fly_mode(input, delta_time, camera_speed);
                self.consumed_input = true;
            }
            CameraControlMode::Orbit => {
                self.process_orbit_mode(input);
                self.consumed_input = true;
            }
            CameraControlMode::Pan => {
                self.process_pan_mode(input, camera_speed);
                self.consumed_input = true;
            }
            CameraControlMode::LookDrag => {
                self.process_look_drag_mode(input, delta_time, camera_speed);
                self.consumed_input = true;
            }
            CameraControlMode::None => {
                // Handle scroll dolly even when no button held
                if viewport_hovered && input.scroll_delta().abs() > 0.0 {
                    self.process_dolly(input);
                    self.consumed_input = true;
                }
            }
        }

        self.consumed_input
    }

    /// Fly mode: RMB + WASD/QE for movement, mouse for look
    fn process_fly_mode(&mut self, input: &InputManager, delta_time: f32, camera_speed: f32) {
        let forward = (self.target - self.position).normalize();
        let right = forward.cross(self.up).normalize();

        // Use camera_speed from viewport settings, scaled by multiplier (scroll adjustment)
        let speed = camera_speed * self.fly_speed_multiplier * delta_time * 10.0;

        // WASD + QE movement
        let mut movement = Vec3::ZERO;
        if input.is_key_pressed(KeyCode::KeyW) {
            movement += forward;
        }
        if input.is_key_pressed(KeyCode::KeyS) {
            movement -= forward;
        }
        if input.is_key_pressed(KeyCode::KeyA) {
            movement -= right;
        }
        if input.is_key_pressed(KeyCode::KeyD) {
            movement += right;
        }
        if input.is_key_pressed(KeyCode::KeyQ) {
            movement -= self.up;
        }
        if input.is_key_pressed(KeyCode::KeyE) {
            movement += self.up;
        }

        if movement.length_squared() > 0.0 {
            movement = movement.normalize() * speed;
            self.position += movement;
            self.target += movement;
        }

        // Mouse look
        let (dx, dy) = input.mouse_delta();
        if dx != 0.0 || dy != 0.0 {
            self.rotate_view(-dx * self.mouse_sensitivity, -dy * self.mouse_sensitivity);
        }

        // Scroll to adjust fly speed
        let scroll = input.scroll_delta();
        if scroll != 0.0 {
            self.fly_speed_multiplier =
                (self.fly_speed_multiplier * (1.0 + scroll * 0.1)).clamp(0.1, 10.0);
        }
    }

    /// Orbit mode: Alt+LMB orbits around target
    fn process_orbit_mode(&mut self, input: &InputManager) {
        let (dx, dy) = input.mouse_delta();
        if dx == 0.0 && dy == 0.0 {
            return;
        }

        // Orbit around target
        let offset = self.position - self.target;
        let distance = offset.length().max(0.1);

        // Spherical coordinates
        let mut yaw = offset.z.atan2(offset.x);
        let mut pitch = (offset.y / distance).clamp(-0.999, 0.999).asin();

        yaw -= dx * self.mouse_sensitivity;
        pitch = (pitch - dy * self.mouse_sensitivity).clamp(-1.5, 1.5);

        // Reconstruct position
        self.position = self.target
            + Vec3::new(
                yaw.cos() * pitch.cos() * distance,
                pitch.sin() * distance,
                yaw.sin() * pitch.cos() * distance,
            );

        self.orbit_distance = distance;
    }

    /// Pan mode: MMB or LMB+RMB moves camera laterally
    fn process_pan_mode(&mut self, input: &InputManager, camera_speed: f32) {
        let (dx, dy) = input.mouse_delta();
        if dx == 0.0 && dy == 0.0 {
            return;
        }

        let forward = (self.target - self.position).normalize();
        let right = forward.cross(self.up).normalize();
        let up = right.cross(forward);

        // Scale with distance and camera speed setting
        let pan_speed = 0.01 * self.orbit_distance * camera_speed;
        let offset = right * -dx * pan_speed + up * dy * pan_speed;

        self.position += offset;
        self.target += offset;
    }

    /// LookDrag mode: LMB drag moves forward/back and rotates
    fn process_look_drag_mode(&mut self, input: &InputManager, delta_time: f32, camera_speed: f32) {
        let (dx, dy) = input.mouse_delta();
        if dx == 0.0 && dy == 0.0 {
            return;
        }

        // Y movement: forward/backward
        let forward = (self.target - self.position).normalize();
        let move_speed = camera_speed * self.fly_speed_multiplier * delta_time * 20.0;
        let movement = forward * -dy * move_speed * 0.1;
        self.position += movement;
        self.target += movement;

        // X movement: rotate (yaw only)
        self.rotate_view(-dx * self.mouse_sensitivity * 0.5, 0.0);
    }

    /// Dolly: scroll wheel moves camera forward/backward
    fn process_dolly(&mut self, input: &InputManager) {
        let scroll = input.scroll_delta();
        if scroll == 0.0 {
            return;
        }

        let direction = (self.target - self.position).normalize();
        let dolly_amount = scroll * self.orbit_distance * 0.1;

        self.position += direction * dolly_amount;
        self.orbit_distance = (self.target - self.position).length();
    }

    /// Focus camera on a target position
    ///
    /// # Arguments
    /// * `center` - World position to focus on (Y-up render space)
    /// * `bounds_radius` - Approximate radius of the object to frame
    pub fn focus_on(&mut self, center: Vec3, bounds_radius: f32) {
        let direction = (self.position - self.target).normalize();
        let distance = (bounds_radius * 2.5).max(2.0); // Fit in view with margin

        self.target = center;
        self.position = center + direction * distance;
        self.orbit_distance = distance;
    }

    /// Rotate the camera view direction
    fn rotate_view(&mut self, yaw: f32, pitch: f32) {
        let direction = self.target - self.position;
        let distance = direction.length();

        // Apply yaw (around world Y-up axis)
        let yaw_rotation = Quat::from_rotation_y(yaw);
        let new_dir = yaw_rotation * direction;

        // Apply pitch (around local right axis)
        let right = new_dir.normalize().cross(self.up).normalize();
        let pitch_rotation = Quat::from_axis_angle(right, pitch);
        let final_dir = pitch_rotation * new_dir;

        // Prevent flipping (don't let camera go past vertical)
        let dot = final_dir.normalize().dot(self.up).abs();
        if dot < 0.99 {
            self.target = self.position + final_dir.normalize() * distance;
        }
    }

    /// Get the forward direction (normalized)
    pub fn forward(&self) -> Vec3 {
        (self.target - self.position).normalize()
    }

    /// Get the right direction (normalized)
    pub fn right(&self) -> Vec3 {
        self.forward().cross(self.up).normalize()
    }
}
