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
}
