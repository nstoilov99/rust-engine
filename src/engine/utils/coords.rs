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

/// Converts Z-up rotation to Y-up rotation using stable component remapping.
///
/// The coordinate change involves a reflection (det = -1), which requires
/// negating the rotation angle. Combined with axis remapping:
/// - Z-up X (forward) → Y-up -Z (backward)
/// - Z-up Y (right)   → Y-up X (right)
/// - Z-up Z (up)      → Y-up Y (up)
///
/// This is computed via direct quaternion component manipulation,
/// avoiding unstable axis-angle decomposition which has numerical issues
/// for small rotation angles (division by near-zero sin(θ/2)).
///
/// For quaternion q = (x, y, z, w) with axis (ax, ay, az) and angle θ:
///   x = ax * sin(θ/2), y = ay * sin(θ/2), z = az * sin(θ/2), w = cos(θ/2)
///
/// With angle negation (sin(-θ/2) = -sin(θ/2), cos(-θ/2) = cos(θ/2)):
///   x' (Y-up X from Z-up Y) = -rot.y
///   y' (Y-up Y from Z-up Z) = -rot.z
///   z' (Y-up Z from -Z-up X) = rot.x (double negation)
///   w' = rot.w (unchanged)
pub fn convert_rotation_zup_to_yup(rot: Quat) -> Quat {
    // Component remapping is length-preserving, no normalization needed.
    // Normalizing here would introduce floating-point errors that accumulate
    // across frames during gizmo interaction, causing flickering.
    Quat::from_xyzw(-rot.y, -rot.z, rot.x, rot.w)
}

/// Converts Y-up rotation to Z-up rotation using stable component remapping.
///
/// Inverse of convert_rotation_zup_to_yup:
/// - Y-up X (right)   → Z-up Y (right)
/// - Y-up Y (up)      → Z-up Z (up)
/// - Y-up -Z (backward) → Z-up X (forward)
///
/// Uses direct quaternion component manipulation for numerical stability.
///
/// Inverse remapping with angle negation:
///   x' (Z-up X from -Y-up Z) = rot.z (double negation)
///   y' (Z-up Y from Y-up X) = -rot.x
///   z' (Z-up Z from Y-up Y) = -rot.y
///   w' = rot.w (unchanged)
pub fn convert_rotation_yup_to_zup(rot: Quat) -> Quat {
    // Component remapping is length-preserving, no normalization needed.
    // Normalizing here would introduce floating-point errors that accumulate
    // across frames during gizmo interaction, causing flickering.
    Quat::from_xyzw(rot.z, -rot.x, -rot.y, rot.w)
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

    #[test]
    fn test_rotation_roundtrip() {
        // Test that rotation round-trip conversion preserves the original rotation
        let rotations = [
            Quat::IDENTITY,
            Quat::from_rotation_x(0.5),
            Quat::from_rotation_y(0.7),
            Quat::from_rotation_z(1.2),
            Quat::from_euler(glam::EulerRot::XYZ, 0.3, 0.4, 0.5),
        ];

        for rot in rotations {
            let yup = convert_rotation_zup_to_yup(rot);
            let back = convert_rotation_yup_to_zup(yup);

            // Quaternions can be negated and still represent same rotation
            let diff = (back - rot).length().min((back + rot).length());
            assert!(diff < 0.001, "Round-trip failed for {:?}, got {:?}", rot, back);
        }
    }

    #[test]
    fn test_rotation_axis_mapping() {
        // The coordinate change Z-up → Y-up has det = -1 (reflection), which flips handedness.
        // Therefore, rotation angles are negated to maintain the same rotation effect.
        //
        // Axis mapping:
        // - Z-up Z (up) → Y-up Y (up), angle negated
        // - Z-up X (forward) → Y-up -Z (backward), angle negated
        // - Z-up Y (right) → Y-up X (right), angle negated

        // Z-up Z rotation → Y-up -Y rotation (angle negated)
        let rot_around_zup_z = Quat::from_rotation_z(std::f32::consts::FRAC_PI_4);
        let converted = convert_rotation_zup_to_yup(rot_around_zup_z);
        let expected = Quat::from_rotation_y(-std::f32::consts::FRAC_PI_4);
        let diff = (converted - expected).length().min((converted + expected).length());
        assert!(diff < 0.001, "Z-up Z rotation should map to Y-up -Y rotation");

        // Z-up X rotation → Y-up Z rotation (axis flipped, angle negated → positive Z)
        let rot_around_zup_x = Quat::from_rotation_x(std::f32::consts::FRAC_PI_4);
        let converted_x = convert_rotation_zup_to_yup(rot_around_zup_x);
        let expected_x = Quat::from_rotation_z(std::f32::consts::FRAC_PI_4);
        let diff_x = (converted_x - expected_x).length().min((converted_x + expected_x).length());
        assert!(diff_x < 0.001, "Z-up X rotation should map to Y-up Z rotation");

        // Z-up Y rotation → Y-up -X rotation (angle negated)
        let rot_around_zup_y = Quat::from_rotation_y(std::f32::consts::FRAC_PI_4);
        let converted_y = convert_rotation_zup_to_yup(rot_around_zup_y);
        let expected_y = Quat::from_rotation_x(-std::f32::consts::FRAC_PI_4);
        let diff_y = (converted_y - expected_y).length().min((converted_y + expected_y).length());
        assert!(diff_y < 0.001, "Z-up Y rotation should map to Y-up -X rotation");
    }

    #[test]
    fn test_rotation_arbitrary_axis() {
        // Test rotation around an arbitrary axis (not aligned with any coordinate axis)
        // This is the critical test that would fail with simple component remapping
        let arbitrary_axis = Vec3::new(1.0, 1.0, 1.0).normalize();
        let angle = std::f32::consts::FRAC_PI_3; // 60 degrees
        let rot_zup = Quat::from_axis_angle(arbitrary_axis, angle);

        // Convert to Y-up
        let rot_yup = convert_rotation_zup_to_yup(rot_zup);

        // The axis should be transformed the same way as a position vector
        let expected_axis_yup = convert_position_zup_to_yup(arbitrary_axis).normalize();
        let (actual_axis, actual_angle) = rot_yup.to_axis_angle();

        // Axis should match (accounting for sign flip of axis with angle negation)
        let axis_diff = (actual_axis - expected_axis_yup).length()
            .min((actual_axis + expected_axis_yup).length());
        assert!(axis_diff < 0.01, "Axis mismatch: expected {:?}, got {:?}", expected_axis_yup, actual_axis);

        // Angle should be preserved (possibly negated with axis flip)
        assert!((actual_angle.abs() - angle).abs() < 0.01,
            "Angle mismatch: expected {}, got {}", angle, actual_angle.abs());
    }

    #[test]
    fn test_rotation_preserves_transformed_vectors() {
        // The most important test: a rotation should transform vectors consistently
        // after coordinate conversion
        let test_rotations = [
            Quat::from_rotation_x(0.3),
            Quat::from_rotation_y(0.5),
            Quat::from_rotation_z(0.7),
            Quat::from_euler(glam::EulerRot::XYZ, 0.2, 0.4, 0.6),
            Quat::from_axis_angle(Vec3::new(1.0, 2.0, 3.0).normalize(), 0.8),
        ];

        let test_vectors = [
            Vec3::X,
            Vec3::Y,
            Vec3::Z,
            Vec3::new(1.0, 2.0, 3.0),
        ];

        for rot_zup in test_rotations {
            for vec_zup in test_vectors {
                // Method 1: Rotate in Z-up, then convert result to Y-up
                let rotated_zup = rot_zup * vec_zup;
                let result1 = convert_position_zup_to_yup(rotated_zup);

                // Method 2: Convert both to Y-up, then rotate
                let vec_yup = convert_position_zup_to_yup(vec_zup);
                let rot_yup = convert_rotation_zup_to_yup(rot_zup);
                let result2 = rot_yup * vec_yup;

                // Both methods should produce the same result
                let diff = (result1 - result2).length();
                assert!(diff < 0.001,
                    "Vector transformation inconsistent for rot {:?}, vec {:?}: {:?} vs {:?}",
                    rot_zup, vec_zup, result1, result2);
            }
        }
    }
}