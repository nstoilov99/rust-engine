//! Render adapter for ECS → Vulkan coordinate conversion
//!
//! Converts Z-up ECS transforms to Y-up render space for Vulkan.
//!
//! # Coordinate Systems
//!
//! | System      | Up  | Forward | Right |
//! |-------------|-----|---------|-------|
//! | Game (Z-up) | +Z  | +X      | +Y    |
//! | Vulkan(Y-up)| +Y  | -Z      | +X    |
//!
//! # Conversion Strategy
//!
//! All transforms are composed in Z-up space (the game's native coordinate system).
//! At render time, the final world matrix is converted to Y-up using a basis change:
//!
//! ```text
//! render_matrix = C * world_matrix_zup * C^(-1)
//! ```
//!
//! where C is the coordinate conversion matrix. This ensures correct behavior for:
//! - Non-uniform scaling combined with rotation
//! - Hierarchical parent-child transforms
//! - Negative scale (mirroring)

use crate::engine::ecs::components::Transform;
use crate::engine::utils::coords::convert_position_zup_to_yup;
use glam::Vec3;
use nalgebra_glm as glm;

/// The basis change matrix from Z-up to Y-up coordinate system.
///
/// This matrix transforms vectors/points from Z-up to Y-up:
/// - Z-up Y (right) → Y-up X (right)
/// - Z-up Z (up) → Y-up Y (up)
/// - Z-up X (forward) → Y-up -Z (forward into screen)
///
/// Matrix layout (column-major):
/// ```text
/// | 0  0 -1  0 |
/// | 1  0  0  0 |
/// | 0  1  0  0 |
/// | 0  0  0  1 |
/// ```
fn get_basis_change_matrix() -> glm::Mat4 {
    // Column-major: each column is specified
    glm::mat4(
        0.0, 1.0, 0.0, 0.0, // column 0: Y-up X basis = Z-up Y
        0.0, 0.0, 1.0, 0.0, // column 1: Y-up Y basis = Z-up Z
        -1.0, 0.0, 0.0, 0.0, // column 2: Y-up Z basis = -Z-up X
        0.0, 0.0, 0.0, 1.0, // column 3: translation (identity)
    )
}

/// Convert a Z-up world matrix to Y-up render matrix using basis change.
///
/// This is the mathematically correct way to transform an entire composed matrix
/// between coordinate systems. It handles rotation, scale, and translation together,
/// preserving the matrix determinant sign (handedness).
///
/// Use this function after composing hierarchical transforms in Z-up space.
///
/// # Example
/// ```ignore
/// let world_zup = hierarchy::get_world_transform(world, entity);
/// let render_matrix = world_matrix_to_render(&world_zup);
/// ```
pub fn world_matrix_to_render(world_matrix_zup: &glm::Mat4) -> glm::Mat4 {
    let c = get_basis_change_matrix();
    // C is orthogonal, so C^(-1) = C^T
    let c_inv = glm::transpose(&c);
    c * world_matrix_zup * c_inv
}

/// Convert local ECS Transform (Z-up) to render model matrix (Y-up)
///
/// This function builds the local transform matrix in Z-up space, then
/// converts it to Y-up for rendering.
///
/// **Note**: For hierarchical entities with parents, prefer using:
/// 1. `hierarchy::get_world_transform()` to compose the full world matrix in Z-up
/// 2. `world_matrix_to_render()` to convert the result to Y-up
///
/// This function is kept for backward compatibility with simple (non-hierarchical) scenes.
///
/// # Example
/// ```ignore
/// let model_matrix = transform_to_model_matrix(&entity_transform);
/// // Use model_matrix with view_projection for rendering
/// ```
pub fn transform_to_model_matrix(transform: &Transform) -> glm::Mat4 {
    // Build local matrix in Z-up space (same as hierarchy.rs)
    let translation = glm::translation(&transform.position);
    let rotation = glm::quat_to_mat4(&transform.rotation);
    let scale = glm::scaling(&transform.scale);
    let local_matrix_zup = translation * rotation * scale;

    // Convert to Y-up for rendering
    world_matrix_to_render(&local_matrix_zup)
}

/// Convert a Z-up direction vector to Y-up for lighting calculations
///
/// Use this for light directions and other directional vectors in rendering.
pub fn direction_to_render(dir: &glm::Vec3) -> glm::Vec3 {
    let dir_zup = Vec3::new(dir.x, dir.y, dir.z);
    let dir_yup = convert_position_zup_to_yup(dir_zup);
    glm::vec3(dir_yup.x, dir_yup.y, dir_yup.z)
}

/// Convert a Z-up position to Y-up for shader uniforms
///
/// Use this for camera position and other positions passed to shaders.
pub fn position_to_render(pos: &glm::Vec3) -> glm::Vec3 {
    let pos_zup = Vec3::new(pos.x, pos.y, pos.z);
    let pos_yup = convert_position_zup_to_yup(pos_zup);
    glm::vec3(pos_yup.x, pos_yup.y, pos_yup.z)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_position_conversion() {
        // Forward 10 units in Z-up (X=forward)
        let pos = glm::vec3(10.0, 0.0, 0.0);
        let render_pos = position_to_render(&pos);

        // Should be -Z in Y-up (forward into screen)
        assert!((render_pos.x - 0.0).abs() < 0.001);
        assert!((render_pos.y - 0.0).abs() < 0.001);
        assert!((render_pos.z - (-10.0)).abs() < 0.001);
    }

    #[test]
    fn test_up_vector_conversion() {
        // Up in Z-up is +Z
        let up = glm::vec3(0.0, 0.0, 5.0);
        let render_up = position_to_render(&up);

        // Should be +Y in Y-up
        assert!((render_up.x - 0.0).abs() < 0.001);
        assert!((render_up.y - 5.0).abs() < 0.001);
        assert!((render_up.z - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_world_matrix_identity() {
        // Identity matrix should convert to identity (both are valid transforms)
        let identity = glm::identity();
        let converted = world_matrix_to_render(&identity);

        // The conversion matrix applied to identity gives the basis change
        // but determinant should still be 1 (valid orthogonal transform)
        let det = glm::determinant(&converted);
        assert!(
            (det - 1.0).abs() < 0.001,
            "Determinant should be 1.0, got {}",
            det
        );
    }

    #[test]
    fn test_world_matrix_preserves_determinant() {
        // A scaled matrix should preserve its determinant after conversion
        let scale_matrix = glm::scaling(&glm::vec3(2.0, 3.0, 4.0));
        let converted = world_matrix_to_render(&scale_matrix);

        let original_det = glm::determinant(&scale_matrix);
        let converted_det = glm::determinant(&converted);

        assert!(
            (original_det - converted_det).abs() < 0.001,
            "Determinant changed: {} -> {}",
            original_det,
            converted_det
        );
    }

    #[test]
    fn test_world_matrix_scale_values_preserved() {
        // Create a pure scale matrix in Z-up
        let scale_zup = glm::scaling(&glm::vec3(2.0, 3.0, 4.0));
        let converted = world_matrix_to_render(&scale_zup);

        // Extract scale magnitudes from columns
        let scale_x = glm::length(&glm::vec3(
            converted[(0, 0)],
            converted[(1, 0)],
            converted[(2, 0)],
        ));
        let scale_y = glm::length(&glm::vec3(
            converted[(0, 1)],
            converted[(1, 1)],
            converted[(2, 1)],
        ));
        let scale_z = glm::length(&glm::vec3(
            converted[(0, 2)],
            converted[(1, 2)],
            converted[(2, 2)],
        ));

        // All three scale values should appear (remapped to different axes)
        let mut scales = [scale_x, scale_y, scale_z];
        scales.sort_by(|a, b| a.partial_cmp(b).unwrap());

        assert!(
            (scales[0] - 2.0).abs() < 0.001,
            "Smallest scale should be 2.0"
        );
        assert!(
            (scales[1] - 3.0).abs() < 0.001,
            "Middle scale should be 3.0"
        );
        assert!(
            (scales[2] - 4.0).abs() < 0.001,
            "Largest scale should be 4.0"
        );
    }

    #[test]
    fn test_transform_to_model_matrix_with_scale() {
        // Create a transform with non-uniform scale
        let transform = Transform {
            position: glm::vec3(0.0, 0.0, 0.0),
            rotation: glm::quat_identity(),
            scale: glm::vec3(2.0, 3.0, 4.0),
        };

        let model = transform_to_model_matrix(&transform);

        // Extract scale magnitudes
        let scale_x = glm::length(&glm::vec3(model[(0, 0)], model[(1, 0)], model[(2, 0)]));
        let scale_y = glm::length(&glm::vec3(model[(0, 1)], model[(1, 1)], model[(2, 1)]));
        let scale_z = glm::length(&glm::vec3(model[(0, 2)], model[(1, 2)], model[(2, 2)]));

        // Scale values should be preserved (remapped)
        let mut scales = [scale_x, scale_y, scale_z];
        scales.sort_by(|a, b| a.partial_cmp(b).unwrap());

        assert!((scales[0] - 2.0).abs() < 0.001);
        assert!((scales[1] - 3.0).abs() < 0.001);
        assert!((scales[2] - 4.0).abs() < 0.001);
    }

    #[test]
    fn test_transform_with_rotation_and_scale() {
        // Create a transform with rotation and non-uniform scale
        let transform = Transform {
            position: glm::vec3(1.0, 2.0, 3.0),
            rotation: glm::quat_angle_axis(
                std::f32::consts::FRAC_PI_4,
                &glm::vec3(0.0, 0.0, 1.0), // Rotate around Z-up
            ),
            scale: glm::vec3(2.0, 1.0, 1.0), // Non-uniform scale
        };

        let model = transform_to_model_matrix(&transform);

        // Matrix should be valid
        let det = glm::determinant(&model);
        assert!(det.abs() > 0.001, "Matrix should have non-zero determinant");

        // Determinant should equal product of scales (2 * 1 * 1 = 2)
        assert!(
            (det.abs() - 2.0).abs() < 0.001,
            "Determinant should be 2.0, got {}",
            det
        );
    }

    #[test]
    fn test_basis_change_is_orthogonal() {
        // The basis change matrix should be orthogonal
        let c = get_basis_change_matrix();
        let c_t = glm::transpose(&c);
        let product = c * c_t;

        // C * C^T should be identity for orthogonal matrix
        for i in 0..4 {
            for j in 0..4 {
                let expected = if i == j { 1.0 } else { 0.0 };
                assert!(
                    (product[(i, j)] - expected).abs() < 0.001,
                    "C * C^T [{},{}] should be {}, got {}",
                    i,
                    j,
                    expected,
                    product[(i, j)]
                );
            }
        }
    }

    #[test]
    fn position_zup_to_yup_roundtrip_via_matrix() {
        // Convert Z-up position through matrix pipeline and verify
        let pos_zup = glm::vec3(3.0, 7.0, 11.0);
        let t = Transform::new(pos_zup);
        let render_mat = transform_to_model_matrix(&t);

        // Extract position from render matrix (column 3)
        let render_pos = glm::vec3(render_mat[(0, 3)], render_mat[(1, 3)], render_mat[(2, 3)]);

        // The simple position conversion should match
        let expected = position_to_render(&pos_zup);
        assert!((render_pos.x - expected.x).abs() < 0.001);
        assert!((render_pos.y - expected.y).abs() < 0.001);
        assert!((render_pos.z - expected.z).abs() < 0.001);
    }

    #[test]
    fn rotation_roundtrip_zup_yup_zup() {
        // Rotate 45 degrees around Z-up's Z axis
        let rot_zup = glm::quat_angle_axis(std::f32::consts::FRAC_PI_4, &glm::vec3(0.0, 0.0, 1.0));
        let t_original = Transform::new(glm::vec3(0.0, 0.0, 0.0)).with_rotation(rot_zup);
        let mat_zup = t_original.local_matrix_zup();

        // Convert to render (Y-up) and back
        let mat_yup = world_matrix_to_render(&mat_zup);
        let c = get_basis_change_matrix();
        let c_inv = glm::transpose(&c);
        let mat_back = c_inv * mat_yup * c;

        // Should recover the original matrix
        for i in 0..4 {
            for j in 0..4 {
                assert!(
                    (mat_back[(i, j)] - mat_zup[(i, j)]).abs() < 0.001,
                    "roundtrip failed at [{},{}]: {} vs {}",
                    i,
                    j,
                    mat_back[(i, j)],
                    mat_zup[(i, j)]
                );
            }
        }
    }

    #[test]
    fn non_uniform_scale_roundtrip() {
        let t = Transform {
            position: glm::vec3(1.0, 2.0, 3.0),
            rotation: glm::quat_identity(),
            scale: glm::vec3(2.0, 3.0, 4.0),
        };
        let mat_zup = t.local_matrix_zup();
        let mat_yup = world_matrix_to_render(&mat_zup);

        // Roundtrip back
        let c = get_basis_change_matrix();
        let c_inv = glm::transpose(&c);
        let mat_back = c_inv * mat_yup * c;

        for i in 0..4 {
            for j in 0..4 {
                assert!(
                    (mat_back[(i, j)] - mat_zup[(i, j)]).abs() < 0.001,
                    "non-uniform scale roundtrip failed at [{},{}]",
                    i,
                    j
                );
            }
        }
    }
}
