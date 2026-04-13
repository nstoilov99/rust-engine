//! Axis-Aligned Bounding Box (AABB)
//!
//! Local-space AABBs are computed once at asset load time and stored in `GpuMesh`.
//! At runtime, `transformed()` produces a world-space AABB for frustum culling.

use glam::{Mat4, Vec3};

/// Axis-aligned bounding box defined by its minimum and maximum corners.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Aabb {
    /// Construct an AABB from its min and max corners.
    #[inline]
    pub fn new(min: Vec3, max: Vec3) -> Self {
        Self { min, max }
    }

    /// Compute the AABB enclosing all given points.
    ///
    /// Returns a zero-volume AABB at the origin for an empty slice.
    pub fn from_points(points: impl IntoIterator<Item = Vec3>) -> Self {
        let mut iter = points.into_iter();
        let Some(first) = iter.next() else {
            return Self {
                min: Vec3::ZERO,
                max: Vec3::ZERO,
            };
        };
        let mut min = first;
        let mut max = first;
        for p in iter {
            min = min.min(p);
            max = max.max(p);
        }
        Self { min, max }
    }

    /// Centre of the AABB.
    #[inline]
    pub fn center(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    /// Half-extents (half the size along each axis).
    #[inline]
    pub fn half_extents(&self) -> Vec3 {
        (self.max - self.min) * 0.5
    }

    /// Transform the AABB by an affine 4×4 matrix using the fast
    /// Ericson method (Real-Time Collision Detection §4.2.6).
    ///
    /// Correctly handles non-uniform scaling and rotation.
    pub fn transformed(&self, m: &Mat4) -> Self {
        let center = self.center();
        let half = self.half_extents();

        // Extract translation from the matrix.
        let new_center = m.transform_point3(center);

        // For each axis of the new AABB, accumulate the contribution of
        // each column of the 3×3 part multiplied by the half-extent.
        let abs_col0 = Vec3::new(m.x_axis.x.abs(), m.x_axis.y.abs(), m.x_axis.z.abs());
        let abs_col1 = Vec3::new(m.y_axis.x.abs(), m.y_axis.y.abs(), m.y_axis.z.abs());
        let abs_col2 = Vec3::new(m.z_axis.x.abs(), m.z_axis.y.abs(), m.z_axis.z.abs());

        let new_half = abs_col0 * half.x + abs_col1 * half.y + abs_col2 * half.z;

        Self {
            min: new_center - new_half,
            max: new_center + new_half,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::FRAC_PI_2;

    #[test]
    fn from_points_basic() {
        let aabb = Aabb::from_points([
            Vec3::new(-1.0, -2.0, -3.0),
            Vec3::new(4.0, 5.0, 6.0),
            Vec3::new(0.0, 0.0, 0.0),
        ]);
        assert_eq!(aabb.min, Vec3::new(-1.0, -2.0, -3.0));
        assert_eq!(aabb.max, Vec3::new(4.0, 5.0, 6.0));
    }

    #[test]
    fn from_points_single() {
        let p = Vec3::new(3.0, 4.0, 5.0);
        let aabb = Aabb::from_points([p]);
        assert_eq!(aabb.min, p);
        assert_eq!(aabb.max, p);
    }

    #[test]
    fn from_points_empty() {
        let aabb = Aabb::from_points(std::iter::empty::<Vec3>());
        assert_eq!(aabb.min, Vec3::ZERO);
        assert_eq!(aabb.max, Vec3::ZERO);
    }

    #[test]
    fn center_and_half_extents() {
        let aabb = Aabb::new(Vec3::new(-1.0, -2.0, -3.0), Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(aabb.center(), Vec3::ZERO);
        assert_eq!(aabb.half_extents(), Vec3::new(1.0, 2.0, 3.0));
    }

    #[test]
    fn transformed_identity() {
        let aabb = Aabb::new(Vec3::new(-1.0, -1.0, -1.0), Vec3::new(1.0, 1.0, 1.0));
        let result = aabb.transformed(&Mat4::IDENTITY);
        assert!((result.min - aabb.min).length() < 1e-5);
        assert!((result.max - aabb.max).length() < 1e-5);
    }

    #[test]
    fn transformed_translation() {
        let aabb = Aabb::new(Vec3::new(0.0, 0.0, 0.0), Vec3::new(2.0, 2.0, 2.0));
        let m = Mat4::from_translation(Vec3::new(10.0, 20.0, 30.0));
        let result = aabb.transformed(&m);
        assert!((result.min - Vec3::new(10.0, 20.0, 30.0)).length() < 1e-5);
        assert!((result.max - Vec3::new(12.0, 22.0, 32.0)).length() < 1e-5);
    }

    #[test]
    fn transformed_rotation_90_y() {
        // Rotate 90 degrees around Y: X→Z, Z→-X
        let aabb = Aabb::new(Vec3::new(0.0, 0.0, 0.0), Vec3::new(2.0, 1.0, 0.0));
        let m = Mat4::from_rotation_y(FRAC_PI_2);
        let result = aabb.transformed(&m);
        // After 90° Y rotation, the X extent maps to Z and vice versa.
        // The unit box [0,2]×[0,1]×[0,0] center is (1, 0.5, 0), half=(1, 0.5, 0).
        // Rotated center ≈ (0, 0.5, -1), new half ≈ (0, 0.5, 1).
        assert!((result.center() - Vec3::new(0.0, 0.5, -1.0)).length() < 1e-4);
        assert!((result.half_extents() - Vec3::new(0.0, 0.5, 1.0)).length() < 1e-4);
    }

    #[test]
    fn transformed_non_uniform_scale() {
        let aabb = Aabb::new(Vec3::new(-1.0, -1.0, -1.0), Vec3::new(1.0, 1.0, 1.0));
        let m = Mat4::from_scale(Vec3::new(2.0, 3.0, 4.0));
        let result = aabb.transformed(&m);
        assert!((result.min - Vec3::new(-2.0, -3.0, -4.0)).length() < 1e-5);
        assert!((result.max - Vec3::new(2.0, 3.0, 4.0)).length() < 1e-5);
    }
}
