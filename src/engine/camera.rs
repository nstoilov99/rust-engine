use glam::{Vec2, Vec3, Mat4};

/// Push constants with camera support
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct CameraPushConstants {
    pub view_projection: [[f32; 4]; 4],  // Camera matrix (4x4)
    pub position: [f32; 2],              // Sprite position
    pub rotation: f32,                   // Sprite rotation
    pub scale: [f32; 2],                 // Sprite scale
}

impl CameraPushConstants {
    pub fn new(camera_vp: Mat4, position: glam::Vec2, rotation: f32, scale: glam::Vec2) -> Self {
        Self {
            view_projection: camera_vp.to_cols_array_2d(),
            position: position.to_array(),
            rotation,
            scale: scale.to_array(),
        }
    }
}

/// 2D Camera for viewing the game world
#[derive(Debug, Clone)]
pub struct Camera2D {
    pub position: Vec2,     // Camera position in world space
    pub zoom: f32,          // Zoom level (1.0 = normal, 2.0 = 2x zoom)
    pub viewport_size: Vec2, // Window size in pixels
}

impl Camera2D {
    /// Creates a new camera at the origin
    pub fn new(viewport_width: f32, viewport_height: f32) -> Self {
        Self {
            position: Vec2::ZERO,
            zoom: 1.0,
            viewport_size: Vec2::new(viewport_width, viewport_height),
        }
    }

    /// Updates viewport size (call when window resizes)
    pub fn set_viewport_size(&mut self, width: f32, height: f32) {
        self.viewport_size = Vec2::new(width, height);
        let aspect = width / height;
        println!("📐 Camera viewport updated: {}×{} (aspect: {:.2})", width, height, aspect);
    }

    /// Moves the camera by an offset
    pub fn translate(&mut self, offset: Vec2) {
        self.position += offset;
    }

    /// Sets the camera position
    pub fn set_position(&mut self, position: Vec2) {
        self.position = position;
    }

    /// Adjusts zoom level
    pub fn set_zoom(&mut self, zoom: f32) {
        self.zoom = zoom.clamp(0.1, 10.0); // Prevent negative/zero zoom and extreme zoom
    }

    /// Zooms in/out by a delta (useful for mouse wheel)
    /// Uses multiplicative zoom for smooth scaling
    /// Positive delta = zoom in, Negative delta = zoom out
    pub fn adjust_zoom(&mut self, delta: f32) {
        // Clamp delta to prevent huge zoom jumps
        let clamped_delta = delta.clamp(-3.0, 3.0);

        // Multiplicative zoom: delta > 0 zooms in, delta < 0 zooms out
        // Factor of 1.1 = 10% change per scroll notch
        let zoom_factor = 1.0 + (clamped_delta * 0.1);
        self.zoom = (self.zoom * zoom_factor).clamp(0.1, 10.0);
    }

    /// Calculates the view matrix (transforms world → screen)
    pub fn view_matrix(&self) -> Mat4 {
        // Only apply camera position (translate)
        // Zoom is now handled in projection matrix
        Mat4::from_translation(glam::Vec3::new(-self.position.x, -self.position.y, 0.0))
    }

    /// Calculates the projection matrix (maps world space to NDC)
pub fn projection_matrix(&self) -> Mat4 {
    let half_width = (self.viewport_size.x / 2.0) / self.zoom;
    let half_height = (self.viewport_size.y / 2.0) / self.zoom;

    Mat4::orthographic_rh(
        -half_width,   // e.g., -400 for 800×600 at zoom 1.0
        half_width,    // e.g., +400
        -half_height,  // -300
        half_height,   // +300
        -1.0, 1.0
    )
}

    /// Combined view-projection matrix (use this in shaders)
    pub fn view_projection_matrix(&self) -> Mat4 {
        self.projection_matrix() * self.view_matrix()
    }

    /// Converts screen coordinates to world coordinates
    pub fn screen_to_world(&self, screen_pos: Vec2) -> Vec2 {
        // Convert screen coords (0,0 = top-left) to NDC (-1 to 1)
        let ndc_x = (screen_pos.x / self.viewport_size.x) * 2.0 - 1.0;
        let ndc_y = 1.0 - (screen_pos.y / self.viewport_size.y) * 2.0;

        // Invert the view-projection matrix
        let vp_matrix = self.view_projection_matrix();
        let inv_matrix = vp_matrix.inverse();

        // Transform NDC to world space
        let world_pos = inv_matrix.transform_point3(glam::Vec3::new(ndc_x, ndc_y, 0.0));

        Vec2::new(world_pos.x, world_pos.y)
    }

    /// Get the visible area in pixels at current zoom
    pub fn visible_area(&self) -> (f32, f32) {
        let width = self.viewport_size.x / self.zoom;
        let height = self.viewport_size.y / self.zoom;
        (width, height)
    }

    /// Get world bounds (min, max) in pixel coordinates
    pub fn world_bounds(&self) -> (Vec2, Vec2) {
        let half_width = (self.viewport_size.x / 2.0) / self.zoom;
        let half_height = (self.viewport_size.y / 2.0) / self.zoom;

        let min = self.position - Vec2::new(half_width, half_height);
        let max = self.position + Vec2::new(half_width, half_height);

        (min, max)
    }

    /// Check if a point (in pixel coordinates) is visible
    pub fn is_visible(&self, point: Vec2, margin: f32) -> bool {
        let (min, max) = self.world_bounds();
        point.x >= min.x - margin
            && point.x <= max.x + margin
            && point.y >= min.y - margin
            && point.y <= max.y + margin
    }
}

// Keep your existing Camera2D...

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
            target: Vec3::ZERO,                   // Looking at origin (Y-up space)
            up: Vec3::Y,                          // Y is up in render space
            fov: 45.0_f32.to_radians(),          // 45 degree FOV
            aspect_ratio: viewport_width / viewport_height,
            near: 0.1,                            // Don't clip too close
            far: 100.0,                           // Don't clip too far
        }
    }

    /// Alternative constructor for pure Y-up coordinates (rarely needed)
    /// Only use this if you're NOT using the Z-up coordinate system
    pub fn new_yup(viewport_width: f32, viewport_height: f32) -> Self {
        Self {
            position: Vec3::new(0.0, 2.0, 5.0),  // Y-up: camera back and up
            target: Vec3::ZERO,
            up: Vec3::Y,
            fov: 45.0_f32.to_radians(),
            aspect_ratio: viewport_width / viewport_height,
            near: 0.1,
            far: 100.0,
        }
    }

    /// Updates aspect ratio when window resizes
    pub fn set_viewport_size(&mut self, width: f32, height: f32) {
        self.aspect_ratio = width / height;
    }

    /// Creates view matrix (world → camera space)
    /// Note: self.position and self.target are stored in Y-up render space
    /// (they were converted from Z-up when you called set_position_zup, etc.)
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
        // Calculate current spherical coordinates
        let offset = self.position - self.target;
        let radius = offset.length().max(0.1);

        // Convert to spherical coordinates
        let mut yaw = offset.z.atan2(offset.x);
        let mut pitch = (offset.y / radius).asin();

        // Apply deltas
        yaw += delta_yaw;
        pitch = (pitch + delta_pitch).clamp(-1.5, 1.5); // Prevent gimbal lock

        // Convert back to Cartesian
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