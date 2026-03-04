//! Math type conversions between glam and nalgebra-glm
//!
//! The engine uses both math libraries:
//! - glam: Main rendering code (Camera3D, Mat4 transforms)
//! - nalgebra-glm: ECS Transform components (for quaternion support)
//!
//! This module provides zero-cost conversions between the two.

use glam as g;
use nalgebra_glm as glm;

// ========== Vec3 Conversions ==========

/// Convert glam::Vec3 to nalgebra_glm::Vec3
#[inline]
pub fn vec3_to_glm(v: g::Vec3) -> glm::Vec3 {
    glm::vec3(v.x, v.y, v.z)
}

/// Convert nalgebra_glm::Vec3 to glam::Vec3
#[inline]
pub fn vec3_from_glm(v: &glm::Vec3) -> g::Vec3 {
    g::Vec3::new(v.x, v.y, v.z)
}

// ========== Mat4 Conversions ==========

/// Convert glam::Mat4 to nalgebra_glm::Mat4
#[inline]
pub fn mat4_to_glm(m: g::Mat4) -> glm::Mat4 {
    let cols = m.to_cols_array_2d();
    glm::mat4(
        cols[0][0], cols[0][1], cols[0][2], cols[0][3], cols[1][0], cols[1][1], cols[1][2],
        cols[1][3], cols[2][0], cols[2][1], cols[2][2], cols[2][3], cols[3][0], cols[3][1],
        cols[3][2], cols[3][3],
    )
}

/// Convert nalgebra_glm::Mat4 to glam::Mat4
#[inline]
pub fn mat4_from_glm(m: &glm::Mat4) -> g::Mat4 {
    g::Mat4::from_cols_array_2d(&[
        [m.m11, m.m12, m.m13, m.m14],
        [m.m21, m.m22, m.m23, m.m24],
        [m.m31, m.m32, m.m33, m.m34],
        [m.m41, m.m42, m.m43, m.m44],
    ])
}

// ========== Quaternion Conversions ==========

/// Convert glam::Quat to nalgebra_glm::Quat
/// Note: glm::quat takes parameters in order (x, y, z, w), NOT (w, x, y, z)!
#[inline]
pub fn quat_to_glm(q: g::Quat) -> glm::Quat {
    glm::quat(q.x, q.y, q.z, q.w)
}

/// Convert nalgebra_glm::Quat to glam::Quat
#[inline]
pub fn quat_from_glm(q: &glm::Quat) -> g::Quat {
    g::Quat::from_xyzw(q.coords.x, q.coords.y, q.coords.z, q.coords.w)
}
