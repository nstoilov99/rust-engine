use glam::{Vec2, Mat4};

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
        self.zoom = zoom.clamp(0.1, 10.0);
    }

    /// Zooms in/out by a delta
    pub fn adjust_zoom(&mut self, delta: f32) {
        let clamped_delta = delta.clamp(-3.0, 3.0);
        let zoom_factor = 1.0 + (clamped_delta * 0.1);
        self.zoom = (self.zoom * zoom_factor).clamp(0.1, 10.0);
    }

    /// Calculates the view matrix
    pub fn view_matrix(&self) -> Mat4 {
        Mat4::from_translation(glam::Vec3::new(-self.position.x, -self.position.y, 0.0))
    }

    /// Calculates the projection matrix
    pub fn projection_matrix(&self) -> Mat4 {
        let half_width = (self.viewport_size.x / 2.0) / self.zoom;
        let half_height = (self.viewport_size.y / 2.0) / self.zoom;
        Mat4::orthographic_rh(-half_width, half_width, -half_height, half_height, -1.0, 1.0)
    }

    /// Combined view-projection matrix
    pub fn view_projection_matrix(&self) -> Mat4 {
        self.projection_matrix() * self.view_matrix()
    }

    /// Converts screen coordinates to world coordinates
    pub fn screen_to_world(&self, screen_pos: Vec2) -> Vec2 {
        let ndc_x = (screen_pos.x / self.viewport_size.x) * 2.0 - 1.0;
        let ndc_y = 1.0 - (screen_pos.y / self.viewport_size.y) * 2.0;
        let vp_matrix = self.view_projection_matrix();
        let inv_matrix = vp_matrix.inverse();
        let world_pos = inv_matrix.transform_point3(glam::Vec3::new(ndc_x, ndc_y, 0.0));
        Vec2::new(world_pos.x, world_pos.y)
    }

    /// Get the visible area in pixels at current zoom
    pub fn visible_area(&self) -> (f32, f32) {
        let width = self.viewport_size.x / self.zoom;
        let height = self.viewport_size.y / self.zoom;
        (width, height)
    }

    /// Get world bounds (min, max)
    pub fn world_bounds(&self) -> (Vec2, Vec2) {
        let half_width = (self.viewport_size.x / 2.0) / self.zoom;
        let half_height = (self.viewport_size.y / 2.0) / self.zoom;
        let min = self.position - Vec2::new(half_width, half_height);
        let max = self.position + Vec2::new(half_width, half_height);
        (min, max)
    }

    /// Check if a point is visible
    pub fn is_visible(&self, point: Vec2, margin: f32) -> bool {
        let (min, max) = self.world_bounds();
        point.x >= min.x - margin
            && point.x <= max.x + margin
            && point.y >= min.y - margin
            && point.y <= max.y + margin
    }
}
