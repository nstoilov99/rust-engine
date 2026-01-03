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
///
/// Z-up: (X=forward, Y=right, Z=up)
/// Y-up: (X=right, Y=up, Z=-forward)
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

/// Converts Z-up scale to Y-up scale
///
/// Scale axes must be remapped to match the coordinate conversion:
/// - Z-up X (forward) scale → Y-up Z (forward) scale
/// - Z-up Y (right) scale → Y-up X (right) scale
/// - Z-up Z (up) scale → Y-up Y (up) scale
///
/// Note: Unlike position, scale is always positive along axes (no sign flip for Z).
pub fn convert_scale_zup_to_yup(scale: Vec3) -> Vec3 {
    Vec3::new(
        scale.y,  // Z-up Y (right) → Y-up X (right)
        scale.z,  // Z-up Z (up)    → Y-up Y (up)
        scale.x,  // Z-up X (forward) → Y-up Z (forward) - no sign flip for scale
    )
}

/// Converts Y-up scale to Z-up scale
pub fn convert_scale_yup_to_zup(scale: Vec3) -> Vec3 {
    Vec3::new(
        scale.z,  // Y-up Z (forward) → Z-up X (forward)
        scale.x,  // Y-up X (right)   → Z-up Y (right)
        scale.y,  // Y-up Y (up)      → Z-up Z (up)
    )
}

/// Converts Z-up rotation to Y-up rotation
pub fn convert_rotation_zup_to_yup(rot: Quat) -> Quat {
    // Rotation conversion: 90° rotation around X axis
    // This rotates the coordinate frame from Z-up to Y-up
    let conversion = Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2);
    conversion * rot
}

/// Converts Y-up rotation to Z-up rotation
pub fn convert_rotation_yup_to_zup(rot: Quat) -> Quat {
    // Inverse rotation conversion: -90° rotation around X axis
    let conversion = Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
    conversion * rot
}

/// Converts Z-up transform to Y-up transform matrix
///
/// All components (position, rotation, scale) are converted to Y-up space
/// before composing the final matrix.
pub fn convert_transform_zup_to_yup(position: Vec3, rotation: Quat, scale: Vec3) -> Mat4 {
    let render_pos = convert_position_zup_to_yup(position);
    let render_rot = convert_rotation_zup_to_yup(rotation);
    let render_scale = convert_scale_zup_to_yup(scale);

    Mat4::from_scale_rotation_translation(render_scale, render_rot, render_pos)
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
    fn test_scale_conversion() {
        // Non-uniform scale in Z-up: 2x forward, 3x right, 4x up
        let zup_scale = Vec3::new(2.0, 3.0, 4.0);

        // Convert to Y-up
        let yup_scale = convert_scale_zup_to_yup(zup_scale);

        // Should be: 3x right (X), 4x up (Y), 2x forward (Z)
        assert_eq!(yup_scale, Vec3::new(3.0, 4.0, 2.0));

        // Roundtrip test
        let back_to_zup = convert_scale_yup_to_zup(yup_scale);
        assert!((back_to_zup - zup_scale).length() < 0.001);
    }

    #[test]
    fn test_scale_conversion_uniform() {
        // Uniform scale should remain unchanged in magnitude
        let uniform = Vec3::new(5.0, 5.0, 5.0);
        let converted = convert_scale_zup_to_yup(uniform);
        assert_eq!(converted, Vec3::new(5.0, 5.0, 5.0));
    }

    #[test]
    fn test_scale_conversion_identity() {
        // Identity scale should remain identity
        let identity = Vec3::ONE;
        let converted = convert_scale_zup_to_yup(identity);
        assert_eq!(converted, Vec3::ONE);
    }

    #[test]
    fn test_transform_with_non_uniform_scale() {
        // Test that convert_transform_zup_to_yup properly converts scale
        let position = Vec3::ZERO;
        let rotation = Quat::IDENTITY;
        let scale = Vec3::new(2.0, 3.0, 4.0);

        let matrix = convert_transform_zup_to_yup(position, rotation, scale);

        // Extract scale from the matrix columns
        let scale_x = Vec3::new(matrix.x_axis.x, matrix.x_axis.y, matrix.x_axis.z).length();
        let scale_y = Vec3::new(matrix.y_axis.x, matrix.y_axis.y, matrix.y_axis.z).length();
        let scale_z = Vec3::new(matrix.z_axis.x, matrix.z_axis.y, matrix.z_axis.z).length();

        // The scale values should be remapped: (2,3,4) -> (3,4,2)
        assert!((scale_x - 3.0).abs() < 0.001, "X scale should be 3.0, got {}", scale_x);
        assert!((scale_y - 4.0).abs() < 0.001, "Y scale should be 4.0, got {}", scale_y);
        assert!((scale_z - 2.0).abs() < 0.001, "Z scale should be 2.0, got {}", scale_z);
    }

    #[test]
    fn test_non_uniform_scale_with_rotation() {
        // Rotate 90 degrees around Z-up's Z axis (up axis)
        let rotation = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        let scale = Vec3::new(2.0, 1.0, 1.0); // Stretched along forward (X in Z-up)
        let position = Vec3::ZERO;

        let matrix = convert_transform_zup_to_yup(position, rotation, scale);

        // Matrix should be valid (non-zero determinant)
        let det = matrix.determinant();
        assert!(det.abs() > 0.001, "Matrix determinant should be non-zero");

        // Scale magnitude should be preserved (product of scales)
        let expected_det = 2.0 * 1.0 * 1.0; // Product of scales
        // Note: Rotation doesn't change determinant, coordinate conversion preserves it
        assert!((det.abs() - expected_det).abs() < 0.001);
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

    #[test]
    fn test_gameplay_transform_with_scale() {
        let transform = GameplayTransform::new(
            Vec3::new(1.0, 2.0, 3.0),
            Quat::IDENTITY,
            Vec3::new(2.0, 3.0, 4.0),
        );

        let matrix = transform.to_render_matrix();

        // Position should be converted
        let pos = matrix.w_axis;
        assert!((pos.x - 2.0).abs() < 0.001); // Y -> X
        assert!((pos.y - 3.0).abs() < 0.001); // Z -> Y
        assert!((pos.z - (-1.0)).abs() < 0.001); // -X -> Z

        // Scale should be converted too
        let scale_x = Vec3::new(matrix.x_axis.x, matrix.x_axis.y, matrix.x_axis.z).length();
        let scale_y = Vec3::new(matrix.y_axis.x, matrix.y_axis.y, matrix.y_axis.z).length();
        let scale_z = Vec3::new(matrix.z_axis.x, matrix.z_axis.y, matrix.z_axis.z).length();

        assert!((scale_x - 3.0).abs() < 0.001); // Y scale -> X
        assert!((scale_y - 4.0).abs() < 0.001); // Z scale -> Y
        assert!((scale_z - 2.0).abs() < 0.001); // X scale -> Z
    }
}