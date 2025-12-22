use glam::{Vec3, Mat4};

/// 3D perspective camera
///
/// IMPORTANT: position, target, and up are stored in Y-up render space.
/// Use set_position_zup() and set_target_zup() to work in Z-up gameplay space.
pub struct Camera3D {
    pub position: Vec3,       // Camera position (Y-up render space)
    pub target: Vec3,         // What the camera looks at (Y-up render space)
    pub up: Vec3,             // Up direction (Vec3::Y for Y-up render space)
    pub fov: f32,             // Field of view in radians
    pub aspect_ratio: f32,    // Width / height
    pub near: f32,            // Near clip plane
    pub far: f32,             // Far clip plane
}

impl Camera3D {
    /// Creates a new 3D perspective camera
    /// Uses Z-up coordinates: X=forward, Y=right, Z=up
    pub fn new(viewport_width: f32, viewport_height: f32) -> Self {
        use crate::engine::coords::convert_position_zup_to_yup;

        // Position in Z-up space: back 10 units, elevated 5 units
        let position_zup = Vec3::new(-10.0, 0.0, 5.0);

        Self {
            position: convert_position_zup_to_yup(position_zup),
            target: Vec3::ZERO,
            up: Vec3::Y,
            fov: 45.0_f32.to_radians(),
            aspect_ratio: viewport_width / viewport_height,
            near: 0.1,
            far: 1000.0,
        }
    }

    /// Alternative constructor for pure Y-up coordinates (rarely needed)
    pub fn new_yup(viewport_width: f32, viewport_height: f32) -> Self {
        Self {
            position: Vec3::new(0.0, 2.0, 5.0),
            target: Vec3::ZERO,
            up: Vec3::Y,
            fov: 45.0_f32.to_radians(),
            aspect_ratio: viewport_width / viewport_height,
            near: 0.1,
            far: 1000.0,
        }
    }

    /// Updates aspect ratio when window resizes
    pub fn set_viewport_size(&mut self, width: f32, height: f32) {
        self.aspect_ratio = width / height;
    }

    /// Creates view matrix (world → camera space)
    pub fn view_matrix(&self) -> Mat4 {
        Mat4::look_at_rh(self.position, self.target, self.up)
    }

    /// Creates perspective projection matrix (camera → clip space)
    pub fn projection_matrix(&self) -> Mat4 {
        Mat4::perspective_rh(self.fov, self.aspect_ratio, self.near, self.far)
    }

    /// Combined view-projection matrix (optimization)
    pub fn view_projection_matrix(&self) -> Mat4 {
        self.projection_matrix() * self.view_matrix()
    }

    /// Orbits camera around target (for editor-style controls)
    pub fn orbit(&mut self, delta_yaw: f32, delta_pitch: f32, distance: f32) {
        let offset = self.position - self.target;
        let radius = offset.length().max(0.1);
        let mut yaw = offset.z.atan2(offset.x);
        let mut pitch = (offset.y / radius).asin();
        yaw += delta_yaw;
        pitch = (pitch + delta_pitch).clamp(-1.5, 1.5);
        let new_offset = Vec3::new(
            yaw.cos() * pitch.cos() * distance,
            pitch.sin() * distance,
            yaw.sin() * pitch.cos() * distance,
        );
        self.position = self.target + new_offset;
    }

    /// Moves camera forward/backward along view direction
    pub fn dolly(&mut self, delta: f32) {
        let direction = (self.target - self.position).normalize();
        self.position += direction * delta;
        self.target += direction * delta;
    }

    /// Pans camera (moves target and position together)
    pub fn pan(&mut self, delta_x: f32, delta_y: f32) {
        let forward = (self.target - self.position).normalize();
        let right = forward.cross(self.up).normalize();
        let up = right.cross(forward);
        let offset = right * delta_x + up * delta_y;
        self.position += offset;
        self.target += offset;
    }
}
