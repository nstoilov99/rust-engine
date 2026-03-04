/// 2D transform for sprites (position, rotation, scale)
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct Transform2D {
    pub position: [f32; 2], // X, Y position in screen space
    pub rotation: f32,      // Rotation in radians
    pub scale: [f32; 2],    // X, Y scale (1.0 = normal size)
}

impl Default for Transform2D {
    fn default() -> Self {
        Self {
            position: [0.0, 0.0],
            rotation: 0.0,
            scale: [1.0, 1.0],
        }
    }
}

impl Transform2D {
    pub fn new(position: [f32; 2], rotation: f32, scale: [f32; 2]) -> Self {
        Self {
            position,
            rotation,
            scale,
        }
    }

    /// Helper: Create transform at position with default rotation/scale
    pub fn at_position(x: f32, y: f32) -> Self {
        Self {
            position: [x, y],
            ..Default::default()
        }
    }
}
