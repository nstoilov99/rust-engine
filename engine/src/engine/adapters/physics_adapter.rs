//! Physics adapter for ECS ↔ Rapier coordinate conversion
//!
//! Converts between Z-up ECS space and Y-up Rapier physics space.
//!
//! # Coordinate Systems
//!
//! | System      | Up  | Forward | Right |
//! |-------------|-----|---------|-------|
//! | Game (Z-up) | +Z  | +X      | +Y    |
//! | Rapier(Y-up)| +Y  | -Z      | +X    |
//!
//! Conversion: `(x, y, z)_zup` → `(y, z, -x)_yup`

use crate::engine::utils::coords::{
    convert_position_yup_to_zup, convert_position_zup_to_yup, convert_rotation_yup_to_zup,
    convert_rotation_zup_to_yup,
};
use glam::Vec3;
use nalgebra_glm as glm;
use rapier3d::na::{Quaternion, UnitQuaternion, Vector3};

/// Convert ECS position (Z-up) to Rapier position (Y-up)
///
/// Use when registering entities or syncing kinematic bodies to physics.
pub fn position_to_physics(pos: &glm::Vec3) -> Vector3<f32> {
    let pos_zup = Vec3::new(pos.x, pos.y, pos.z);
    let pos_yup = convert_position_zup_to_yup(pos_zup);
    Vector3::new(pos_yup.x, pos_yup.y, pos_yup.z)
}

/// Convert Rapier position (Y-up) back to ECS position (Z-up)
///
/// Use when syncing dynamic body positions back to ECS transforms.
pub fn position_from_physics(pos: &Vector3<f32>) -> glm::Vec3 {
    let pos_yup = Vec3::new(pos.x, pos.y, pos.z);
    let pos_zup = convert_position_yup_to_zup(pos_yup);
    glm::vec3(pos_zup.x, pos_zup.y, pos_zup.z)
}

/// Convert velocity from Rapier (Y-up) to ECS (Z-up)
///
/// Use when reading velocity from physics bodies.
pub fn velocity_from_physics(vel: &Vector3<f32>) -> glm::Vec3 {
    position_from_physics(vel) // Same conversion for vectors
}

/// Convert velocity from ECS (Z-up) to Rapier (Y-up)
///
/// Use when applying velocity to physics bodies.
pub fn velocity_to_physics(vel: &glm::Vec3) -> Vector3<f32> {
    position_to_physics(vel) // Same conversion for vectors
}

/// Convert ECS rotation (Z-up) to Rapier rotation (Y-up).
pub fn rotation_to_physics(rot: &glm::Quat) -> UnitQuaternion<f32> {
    let rot_yup = convert_rotation_zup_to_yup(glam::Quat::from_xyzw(
        rot.coords.x,
        rot.coords.y,
        rot.coords.z,
        rot.coords.w,
    ));

    UnitQuaternion::from_quaternion(Quaternion::new(rot_yup.w, rot_yup.x, rot_yup.y, rot_yup.z))
}

/// Convert Rapier rotation (Y-up) back to ECS rotation (Z-up).
pub fn rotation_from_physics(rot: &UnitQuaternion<f32>) -> glm::Quat {
    let rot_yup = glam::Quat::from_xyzw(rot.coords.x, rot.coords.y, rot.coords.z, rot.w);
    let rot_zup = convert_rotation_yup_to_zup(rot_yup);
    glm::quat(rot_zup.x, rot_zup.y, rot_zup.z, rot_zup.w)
}

/// Convert collider cuboid half-extents from Z-up to Rapier Y-up
///
/// In Z-up: half_extents = (forward_size, right_size, up_size) = (X, Y, Z)
/// In Y-up: cuboid expects (right_size, up_size, forward_size) = (X', Y', Z')
///
/// Conversion: Z-up (x, y, z) → Y-up (y, z, x)
pub fn cuboid_half_extents_to_physics(half_extents: &glm::Vec3) -> (f32, f32, f32) {
    // Z-up semantics:
    //   x = half-extent along forward (X axis)
    //   y = half-extent along right (Y axis)
    //   z = half-extent along up (Z axis)
    //
    // Y-up semantics (Rapier):
    //   hx = half-extent along right (X axis)
    //   hy = half-extent along up (Y axis)
    //   hz = half-extent along forward (Z axis)
    //
    // Mapping: Z-up.y → Y-up.hx (right)
    //          Z-up.z → Y-up.hy (up)
    //          Z-up.x → Y-up.hz (forward)
    (half_extents.y, half_extents.z, half_extents.x)
}

/// Convert gravity vector from Z-up to Y-up for Rapier
///
/// In Z-up, gravity is (0, 0, -9.81) - down is -Z
/// In Y-up, gravity is (0, -9.81, 0) - down is -Y
pub fn gravity_to_physics(gravity: &glm::Vec3) -> Vector3<f32> {
    position_to_physics(gravity)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_position_roundtrip() {
        // Position in Z-up: 10 forward, 5 right, 3 up
        let pos_zup = glm::vec3(10.0, 5.0, 3.0);

        // Convert to physics and back
        let physics_pos = position_to_physics(&pos_zup);
        let back_to_zup = position_from_physics(&physics_pos);

        assert!((back_to_zup.x - pos_zup.x).abs() < 0.001);
        assert!((back_to_zup.y - pos_zup.y).abs() < 0.001);
        assert!((back_to_zup.z - pos_zup.z).abs() < 0.001);
    }

    #[test]
    fn test_gravity_conversion() {
        // Z-up gravity: down is -Z
        let gravity_zup = glm::vec3(0.0, 0.0, -9.81);
        let gravity_yup = gravity_to_physics(&gravity_zup);

        // Y-up gravity: down is -Y
        assert!((gravity_yup.x - 0.0).abs() < 0.001);
        assert!((gravity_yup.y - (-9.81)).abs() < 0.001);
        assert!((gravity_yup.z - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_cuboid_extents_conversion() {
        // Z-up cuboid: 2 units forward, 3 units right, 4 units up
        let extents_zup = glm::vec3(2.0, 3.0, 4.0);
        let (hx, hy, hz) = cuboid_half_extents_to_physics(&extents_zup);

        // Y-up: hx=right=3, hy=up=4, hz=forward=2
        assert!((hx - 3.0).abs() < 0.001);
        assert!((hy - 4.0).abs() < 0.001);
        assert!((hz - 2.0).abs() < 0.001);
    }

    #[test]
    fn test_rotation_roundtrip() {
        let rotation_zup = glm::quat(0.25, -0.5, 0.1, 0.82);
        let rotation_zup = glm::quat_normalize(&rotation_zup);

        let physics_rotation = rotation_to_physics(&rotation_zup);
        let back_to_zup = rotation_from_physics(&physics_rotation);

        assert!((back_to_zup.coords.x - rotation_zup.coords.x).abs() < 0.001);
        assert!((back_to_zup.coords.y - rotation_zup.coords.y).abs() < 0.001);
        assert!((back_to_zup.coords.z - rotation_zup.coords.z).abs() < 0.001);
        assert!((back_to_zup.coords.w - rotation_zup.coords.w).abs() < 0.001);
    }

    #[test]
    fn velocity_roundtrip() {
        let vel_zup = glm::vec3(5.0, -3.0, 8.0);
        let vel_yup = velocity_to_physics(&vel_zup);
        let back = velocity_from_physics(&vel_yup);

        assert!((back.x - vel_zup.x).abs() < 0.001);
        assert!((back.y - vel_zup.y).abs() < 0.001);
        assert!((back.z - vel_zup.z).abs() < 0.001);
    }

    #[test]
    fn rotation_90deg_roundtrip() {
        // 90 degrees around Z-up's Z axis (up axis)
        let rot = glm::quat_angle_axis(std::f32::consts::FRAC_PI_2, &glm::vec3(0.0, 0.0, 1.0));
        let rot = glm::quat_normalize(&rot);

        let physics_rot = rotation_to_physics(&rot);
        let back = rotation_from_physics(&physics_rot);

        assert!((back.coords.x - rot.coords.x).abs() < 0.001);
        assert!((back.coords.y - rot.coords.y).abs() < 0.001);
        assert!((back.coords.z - rot.coords.z).abs() < 0.001);
        assert!((back.coords.w - rot.coords.w).abs() < 0.001);
    }

    #[test]
    fn gravity_roundtrip() {
        let gravity_zup = glm::vec3(0.0, 0.0, -9.81);
        let gravity_yup = gravity_to_physics(&gravity_zup);
        let back = position_from_physics(&gravity_yup);

        assert!((back.x - gravity_zup.x).abs() < 0.001);
        assert!((back.y - gravity_zup.y).abs() < 0.001);
        assert!((back.z - gravity_zup.z).abs() < 0.001);
    }

    #[test]
    fn position_zero_roundtrip() {
        let pos = glm::vec3(0.0, 0.0, 0.0);
        let physics = position_to_physics(&pos);
        let back = position_from_physics(&physics);

        assert!((back.x).abs() < 0.001);
        assert!((back.y).abs() < 0.001);
        assert!((back.z).abs() < 0.001);
    }
}
