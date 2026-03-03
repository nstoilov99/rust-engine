//! Frustum culling utilities
//!
//! Provides view frustum extraction from view-projection matrices
//! and sphere-frustum intersection tests for culling.

use glam::{Mat4, Vec3, Vec4};

/// A plane in 3D space defined by normal and distance from origin
#[derive(Clone, Copy, Debug)]
pub struct Plane {
    /// Unit normal vector pointing "inside" the frustum
    pub normal: Vec3,
    /// Signed distance from origin along the normal
    pub distance: f32,
}

impl Plane {
    /// Signed distance from a point to this plane
    /// Positive = in front (inside), Negative = behind (outside)
    #[inline]
    pub fn distance_to_point(&self, point: Vec3) -> f32 {
        self.normal.dot(point) + self.distance
    }
}

/// View frustum represented as 6 planes
#[derive(Clone, Debug)]
pub struct Frustum {
    /// Frustum planes: Left, Right, Bottom, Top, Near, Far
    planes: [Plane; 6],
}

impl Frustum {
    /// Extract frustum planes from a view-projection matrix
    ///
    /// Uses the Gribb/Hartmann method for extracting planes from
    /// a combined view-projection matrix.
    pub fn from_view_projection(vp: Mat4) -> Self {
        // Get matrix rows for plane extraction
        let row0 = vp.row(0);
        let row1 = vp.row(1);
        let row2 = vp.row(2);
        let row3 = vp.row(3);

        // Extract and normalize each plane
        let extract_plane = |coeffs: Vec4| {
            let normal = Vec3::new(coeffs.x, coeffs.y, coeffs.z);
            let len = normal.length();
            if len > 0.0 {
                Plane {
                    normal: normal / len,
                    distance: coeffs.w / len,
                }
            } else {
                Plane {
                    normal: Vec3::ZERO,
                    distance: 0.0,
                }
            }
        };

        Self {
            planes: [
                extract_plane(row3 + row0), // Left
                extract_plane(row3 - row0), // Right
                extract_plane(row3 + row1), // Bottom
                extract_plane(row3 - row1), // Top
                extract_plane(row3 + row2), // Near
                extract_plane(row3 - row2), // Far
            ],
        }
    }

    /// Test if a sphere is visible (inside or intersecting the frustum)
    ///
    /// Returns true if the sphere is at least partially inside the frustum.
    #[inline]
    pub fn contains_sphere(&self, center: Vec3, radius: f32) -> bool {
        for plane in &self.planes {
            // If center is more than radius behind any plane, sphere is outside
            if plane.distance_to_point(center) < -radius {
                return false;
            }
        }
        true
    }

    /// Test if an AABB is visible (inside or intersecting the frustum).
    ///
    /// Uses the p-vertex / n-vertex method: for each frustum plane,
    /// compute the vertex of the AABB that is furthest along the plane
    /// normal (the p-vertex).  If that vertex is behind the plane, the
    /// entire AABB is outside.
    #[inline]
    pub fn contains_aabb(&self, aabb_min: Vec3, aabb_max: Vec3) -> bool {
        for plane in &self.planes {
            // p-vertex: choose the component from min or max that is most
            // aligned with the plane normal.
            let p = Vec3::new(
                if plane.normal.x >= 0.0 { aabb_max.x } else { aabb_min.x },
                if plane.normal.y >= 0.0 { aabb_max.y } else { aabb_min.y },
                if plane.normal.z >= 0.0 { aabb_max.z } else { aabb_min.z },
            );
            if plane.distance_to_point(p) < 0.0 {
                return false;
            }
        }
        true
    }

    /// Test if a point is inside the frustum
    #[inline]
    pub fn contains_point(&self, point: Vec3) -> bool {
        self.contains_sphere(point, 0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Mat4;

    #[test]
    fn test_identity_frustum() {
        // Identity matrix should create a valid frustum
        let frustum = Frustum::from_view_projection(Mat4::IDENTITY);
        // Origin should be inside
        assert!(frustum.contains_point(Vec3::ZERO));
    }

    #[test]
    fn test_sphere_culling() {
        // Create a simple perspective projection looking down -Z
        let proj = Mat4::perspective_rh(std::f32::consts::FRAC_PI_2, 1.0, 0.1, 100.0);
        let view = Mat4::look_at_rh(Vec3::ZERO, Vec3::NEG_Z, Vec3::Y);
        let vp = proj * view;
        let frustum = Frustum::from_view_projection(vp);

        // Sphere in front of camera should be visible
        assert!(frustum.contains_sphere(Vec3::new(0.0, 0.0, -10.0), 1.0));

        // Sphere behind camera should be culled
        assert!(!frustum.contains_sphere(Vec3::new(0.0, 0.0, 10.0), 1.0));

        // Sphere far to the left should be culled
        assert!(!frustum.contains_sphere(Vec3::new(-100.0, 0.0, -10.0), 1.0));
    }

    /// Helper: create a standard test frustum looking down -Z with 90° FOV.
    fn test_frustum() -> Frustum {
        let proj = Mat4::perspective_rh(std::f32::consts::FRAC_PI_2, 1.0, 0.1, 100.0);
        let view = Mat4::look_at_rh(Vec3::ZERO, Vec3::NEG_Z, Vec3::Y);
        Frustum::from_view_projection(proj * view)
    }

    #[test]
    fn aabb_inside_frustum() {
        let frustum = test_frustum();
        // Small box centered in front of camera
        let min = Vec3::new(-1.0, -1.0, -11.0);
        let max = Vec3::new(1.0, 1.0, -9.0);
        assert!(frustum.contains_aabb(min, max));
    }

    #[test]
    fn aabb_behind_camera() {
        let frustum = test_frustum();
        // Box entirely behind the camera (positive Z)
        let min = Vec3::new(-1.0, -1.0, 5.0);
        let max = Vec3::new(1.0, 1.0, 7.0);
        assert!(!frustum.contains_aabb(min, max));
    }

    #[test]
    fn aabb_beyond_far_plane() {
        let frustum = test_frustum();
        // Box beyond the far plane (far = 100)
        let min = Vec3::new(-1.0, -1.0, -200.0);
        let max = Vec3::new(1.0, 1.0, -150.0);
        assert!(!frustum.contains_aabb(min, max));
    }

    #[test]
    fn aabb_outside_left() {
        let frustum = test_frustum();
        // Box far to the left
        let min = Vec3::new(-200.0, -1.0, -11.0);
        let max = Vec3::new(-199.0, 1.0, -9.0);
        assert!(!frustum.contains_aabb(min, max));
    }

    #[test]
    fn aabb_outside_right() {
        let frustum = test_frustum();
        // Box far to the right
        let min = Vec3::new(199.0, -1.0, -11.0);
        let max = Vec3::new(200.0, 1.0, -9.0);
        assert!(!frustum.contains_aabb(min, max));
    }

    #[test]
    fn aabb_outside_top() {
        let frustum = test_frustum();
        // Box far above
        let min = Vec3::new(-1.0, 199.0, -11.0);
        let max = Vec3::new(1.0, 200.0, -9.0);
        assert!(!frustum.contains_aabb(min, max));
    }

    #[test]
    fn aabb_outside_bottom() {
        let frustum = test_frustum();
        // Box far below
        let min = Vec3::new(-1.0, -200.0, -11.0);
        let max = Vec3::new(1.0, -199.0, -9.0);
        assert!(!frustum.contains_aabb(min, max));
    }

    #[test]
    fn aabb_partially_intersecting() {
        let frustum = test_frustum();
        // Large box that straddles the near plane
        let min = Vec3::new(-1.0, -1.0, -1.0);
        let max = Vec3::new(1.0, 1.0, 1.0);
        // The box straddles z=0 where the camera is; near plane is at -0.1.
        // The portion from z=-1 to z=-0.1 is inside the frustum.
        assert!(frustum.contains_aabb(min, max));
    }

    #[test]
    fn aabb_large_enclosing_frustum() {
        let frustum = test_frustum();
        // Huge box enclosing the entire frustum
        let min = Vec3::new(-500.0, -500.0, -500.0);
        let max = Vec3::new(500.0, 500.0, 500.0);
        assert!(frustum.contains_aabb(min, max));
    }
}
