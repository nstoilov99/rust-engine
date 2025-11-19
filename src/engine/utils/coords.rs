use glam::{Mat4, Vec3, Quat};

/// Coordinate system convention
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoordinateSystem {
    /// Y-up system: X=right, Y=up, Z=forward (Vulkan/OpenGL/GLTF)
    YUp,
    /// Z-up system: X=forward, Y=right, Z=up (CAD/Blender/some engines)
    ZUp,
}

/// Transform in gameplay space (Z-up coordinate system)
#[derive(Debug, Clone, Copy)]
pub struct GameplayTransform {
    pub position: Vec3,     // X=forward, Y=right, Z=up
    pub rotation: Quat,     // Rotation in Z-up space
    pub scale: Vec3,
}

impl GameplayTransform {
    pub fn new(position: Vec3, rotation: Quat, scale: Vec3) -> Self {
        Self { position, rotation, scale }
    }

    pub fn identity() -> Self {
        Self {
            position: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        }
    }

    /// Convert to rendering space (Y-up)
    pub fn to_render_matrix(&self) -> Mat4 {
        convert_transform_zup_to_yup(self.position, self.rotation, self.scale)
    }

    /// Create from position (no rotation, uniform scale)
    pub fn from_position(x: f32, y: f32, z: f32) -> Self {
        Self {
            position: Vec3::new(x, y, z),
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        }
    }

    /// Move forward (along X axis in Z-up)
    pub fn move_forward(&mut self, distance: f32) {
        self.position.x += distance;
    }

    /// Move right (along Y axis in Z-up)
    pub fn move_right(&mut self, distance: f32) {
        self.position.y += distance;
    }

    /// Move up (along Z axis in Z-up)
    pub fn move_up(&mut self, distance: f32) {
        self.position.z += distance;
    }
}

/// Converts Z-up position to Y-up position
pub fn convert_position_zup_to_yup(pos: Vec3) -> Vec3 {
    Vec3::new(
        pos.y,   // Z-up Y (right) → Y-up X (right)
        pos.z,   // Z-up Z (up)    → Y-up Y (up)
        -pos.x,  // Z-up X (forward) → Y-up -Z (forward into screen)
    )
}

/// Converts Y-up position to Z-up position
pub fn convert_position_yup_to_zup(pos: Vec3) -> Vec3 {
    Vec3::new(
        -pos.z,  // Y-up Z (forward) → Z-up X (forward)
        pos.x,   // Y-up X (right)   → Z-up Y (right)
        pos.y,   // Y-up Y (up)      → Z-up Z (up)
    )
}

/// Converts Z-up rotation to Y-up rotation
pub fn convert_rotation_zup_to_yup(rot: Quat) -> Quat {
    // Rotation conversion: 90° rotation around X axis
    // This rotates the coordinate frame from Z-up to Y-up
    let conversion = Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2);
    conversion * rot
}

/// Converts Z-up transform to Y-up transform matrix
pub fn convert_transform_zup_to_yup(position: Vec3, rotation: Quat, scale: Vec3) -> Mat4 {
    let render_pos = convert_position_zup_to_yup(position);
    let render_rot = convert_rotation_zup_to_yup(rotation);

    Mat4::from_scale_rotation_translation(scale, render_rot, render_pos)
}

/// Helper constants for Z-up coordinate system
pub mod zup {
    use glam::Vec3;

    pub const FORWARD: Vec3 = Vec3::X;   // X is forward in Z-up
    pub const RIGHT: Vec3 = Vec3::Y;     // Y is right in Z-up
    pub const UP: Vec3 = Vec3::Z;        // Z is up in Z-up
    pub const BACK: Vec3 = Vec3::NEG_X;
    pub const LEFT: Vec3 = Vec3::NEG_Y;
    pub const DOWN: Vec3 = Vec3::NEG_Z;
}

/// Helper constants for Y-up coordinate system (rendering space)
pub mod yup {
    use glam::Vec3;

    pub const RIGHT: Vec3 = Vec3::X;     // X is right in Y-up
    pub const UP: Vec3 = Vec3::Y;        // Y is up in Y-up
    pub const FORWARD: Vec3 = Vec3::Z;   // Z is forward in Y-up
    pub const LEFT: Vec3 = Vec3::NEG_X;
    pub const DOWN: Vec3 = Vec3::NEG_Y;
    pub const BACK: Vec3 = Vec3::NEG_Z;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_position_conversion() {
        // Forward 10 units, right 5 units, up 3 units (Z-up)
        let zup_pos = Vec3::new(10.0, 5.0, 3.0);

        // Convert to Y-up
        let yup_pos = convert_position_zup_to_yup(zup_pos);

        // Should be: right 5, up 3, back -10 (Y-up)
        assert_eq!(yup_pos, Vec3::new(5.0, 3.0, -10.0));

        // Convert back
        let back_to_zup = convert_position_yup_to_zup(yup_pos);
        assert!((back_to_zup - zup_pos).length() < 0.001);
    }

    #[test]
    fn test_gameplay_transform() {
        let mut transform = GameplayTransform::from_position(10.0, 0.0, 2.0);

        // Move forward in Z-up space (along X)
        transform.move_forward(5.0);
        assert_eq!(transform.position.x, 15.0);

        // Move up in Z-up space (along Z)
        transform.move_up(3.0);
        assert_eq!(transform.position.z, 5.0);
    }
}