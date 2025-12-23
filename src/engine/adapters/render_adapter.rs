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
//! Conversion: `(x, y, z)_zup` → `(y, z, -x)_yup`

use crate::engine::ecs::components::Transform;
use crate::engine::utils::coords::convert_position_zup_to_yup;
use glam::Vec3;
use nalgebra_glm as glm;

/// Convert ECS Transform (Z-up) to render model matrix (Y-up)
///
/// This is the single point where ECS transforms are converted for rendering.
/// The rotation is applied directly without conversion to avoid sideways view issues.
///
/// # Example
/// ```ignore
/// let model_matrix = transform_to_model_matrix(&entity_transform);
/// // Use model_matrix with view_projection for rendering
/// ```
pub fn transform_to_model_matrix(transform: &Transform) -> glm::Mat4 {
    // Convert Z-up ECS position to Y-up render position
    let pos_zup = Vec3::new(transform.position.x, transform.position.y, transform.position.z);
    let pos_yup = convert_position_zup_to_yup(pos_zup);
    let render_pos = glm::vec3(pos_yup.x, pos_yup.y, pos_yup.z);

    // NOTE: Rotation is applied directly without conversion.
    // Converting rotation causes sideways view issues.
    let translation = glm::translation(&render_pos);
    let rotation = glm::quat_to_mat4(&transform.rotation);
    let scale = glm::scaling(&transform.scale);

    translation * rotation * scale
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
}
