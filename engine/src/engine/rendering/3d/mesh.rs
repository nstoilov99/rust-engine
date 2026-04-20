use crate::engine::rendering::rendering_3d::pipeline_3d::Vertex3D;

/// Generates a unit cube (1×1×1) centered at origin
pub fn create_cube() -> (Vec<Vertex3D>, Vec<u32>) {
    // 8 unique vertices (cube corners)
    // But we need 24 (4 per face) for proper normals
    let vertices = vec![
        // Front face (Z+) - tangent points right (X+)
        Vertex3D {
            position: [-0.5, -0.5, 0.5],
            normal: [0.0, 0.0, 1.0],
            uv: [0.0, 1.0],
            tangent: [1.0, 0.0, 0.0, 1.0],
            ..Default::default()
        },
        Vertex3D {
            position: [0.5, -0.5, 0.5],
            normal: [0.0, 0.0, 1.0],
            uv: [1.0, 1.0],
            tangent: [1.0, 0.0, 0.0, 1.0],
            ..Default::default()
        },
        Vertex3D {
            position: [0.5, 0.5, 0.5],
            normal: [0.0, 0.0, 1.0],
            uv: [1.0, 0.0],
            tangent: [1.0, 0.0, 0.0, 1.0],
            ..Default::default()
        },
        Vertex3D {
            position: [-0.5, 0.5, 0.5],
            normal: [0.0, 0.0, 1.0],
            uv: [0.0, 0.0],
            tangent: [1.0, 0.0, 0.0, 1.0],
            ..Default::default()
        },
        // Back face (Z-) - tangent points left (X-)
        Vertex3D {
            position: [0.5, -0.5, -0.5],
            normal: [0.0, 0.0, -1.0],
            uv: [0.0, 1.0],
            tangent: [-1.0, 0.0, 0.0, 1.0],
            ..Default::default()
        },
        Vertex3D {
            position: [-0.5, -0.5, -0.5],
            normal: [0.0, 0.0, -1.0],
            uv: [1.0, 1.0],
            tangent: [-1.0, 0.0, 0.0, 1.0],
            ..Default::default()
        },
        Vertex3D {
            position: [-0.5, 0.5, -0.5],
            normal: [0.0, 0.0, -1.0],
            uv: [1.0, 0.0],
            tangent: [-1.0, 0.0, 0.0, 1.0],
            ..Default::default()
        },
        Vertex3D {
            position: [0.5, 0.5, -0.5],
            normal: [0.0, 0.0, -1.0],
            uv: [0.0, 0.0],
            tangent: [-1.0, 0.0, 0.0, 1.0],
            ..Default::default()
        },
        // Right face (X+) - tangent points back (Z-)
        Vertex3D {
            position: [0.5, -0.5, 0.5],
            normal: [1.0, 0.0, 0.0],
            uv: [0.0, 1.0],
            tangent: [0.0, 0.0, -1.0, 1.0],
            ..Default::default()
        },
        Vertex3D {
            position: [0.5, -0.5, -0.5],
            normal: [1.0, 0.0, 0.0],
            uv: [1.0, 1.0],
            tangent: [0.0, 0.0, -1.0, 1.0],
            ..Default::default()
        },
        Vertex3D {
            position: [0.5, 0.5, -0.5],
            normal: [1.0, 0.0, 0.0],
            uv: [1.0, 0.0],
            tangent: [0.0, 0.0, -1.0, 1.0],
            ..Default::default()
        },
        Vertex3D {
            position: [0.5, 0.5, 0.5],
            normal: [1.0, 0.0, 0.0],
            uv: [0.0, 0.0],
            tangent: [0.0, 0.0, -1.0, 1.0],
            ..Default::default()
        },
        // Left face (X-) - tangent points forward (Z+)
        Vertex3D {
            position: [-0.5, -0.5, -0.5],
            normal: [-1.0, 0.0, 0.0],
            uv: [0.0, 1.0],
            tangent: [0.0, 0.0, 1.0, 1.0],
            ..Default::default()
        },
        Vertex3D {
            position: [-0.5, -0.5, 0.5],
            normal: [-1.0, 0.0, 0.0],
            uv: [1.0, 1.0],
            tangent: [0.0, 0.0, 1.0, 1.0],
            ..Default::default()
        },
        Vertex3D {
            position: [-0.5, 0.5, 0.5],
            normal: [-1.0, 0.0, 0.0],
            uv: [1.0, 0.0],
            tangent: [0.0, 0.0, 1.0, 1.0],
            ..Default::default()
        },
        Vertex3D {
            position: [-0.5, 0.5, -0.5],
            normal: [-1.0, 0.0, 0.0],
            uv: [0.0, 0.0],
            tangent: [0.0, 0.0, 1.0, 1.0],
            ..Default::default()
        },
        // Top face (Y+) - tangent points right (X+)
        Vertex3D {
            position: [-0.5, 0.5, 0.5],
            normal: [0.0, 1.0, 0.0],
            uv: [0.0, 1.0],
            tangent: [1.0, 0.0, 0.0, 1.0],
            ..Default::default()
        },
        Vertex3D {
            position: [0.5, 0.5, 0.5],
            normal: [0.0, 1.0, 0.0],
            uv: [1.0, 1.0],
            tangent: [1.0, 0.0, 0.0, 1.0],
            ..Default::default()
        },
        Vertex3D {
            position: [0.5, 0.5, -0.5],
            normal: [0.0, 1.0, 0.0],
            uv: [1.0, 0.0],
            tangent: [1.0, 0.0, 0.0, 1.0],
            ..Default::default()
        },
        Vertex3D {
            position: [-0.5, 0.5, -0.5],
            normal: [0.0, 1.0, 0.0],
            uv: [0.0, 0.0],
            tangent: [1.0, 0.0, 0.0, 1.0],
            ..Default::default()
        },
        // Bottom face (Y-) - tangent points right (X+)
        Vertex3D {
            position: [-0.5, -0.5, -0.5],
            normal: [0.0, -1.0, 0.0],
            uv: [0.0, 1.0],
            tangent: [1.0, 0.0, 0.0, 1.0],
            ..Default::default()
        },
        Vertex3D {
            position: [0.5, -0.5, -0.5],
            normal: [0.0, -1.0, 0.0],
            uv: [1.0, 1.0],
            tangent: [1.0, 0.0, 0.0, 1.0],
            ..Default::default()
        },
        Vertex3D {
            position: [0.5, -0.5, 0.5],
            normal: [0.0, -1.0, 0.0],
            uv: [1.0, 0.0],
            tangent: [1.0, 0.0, 0.0, 1.0],
            ..Default::default()
        },
        Vertex3D {
            position: [-0.5, -0.5, 0.5],
            normal: [0.0, -1.0, 0.0],
            uv: [0.0, 0.0],
            tangent: [1.0, 0.0, 0.0, 1.0],
            ..Default::default()
        },
    ];

    // 6 faces × 2 triangles × 3 indices = 36 indices
    let indices = vec![
        // Front
        0, 1, 2, 2, 3, 0, // Back
        4, 5, 6, 6, 7, 4, // Right
        8, 9, 10, 10, 11, 8, // Left
        12, 13, 14, 14, 15, 12, // Top
        16, 17, 18, 18, 19, 16, // Bottom
        20, 21, 22, 22, 23, 20,
    ];

    (vertices, indices)
}

/// Generates a ground plane (XZ plane)
pub fn create_plane(size: f32) -> (Vec<Vertex3D>, Vec<u32>) {
    let half = size / 2.0;

    // Plane facing up (Y+) with tangent pointing right (X+)
    let vertices = vec![
        Vertex3D {
            position: [-half, 0.0, -half],
            normal: [0.0, 1.0, 0.0],
            uv: [0.0, 0.0],
            tangent: [1.0, 0.0, 0.0, 1.0],
            ..Default::default()
        },
        Vertex3D {
            position: [half, 0.0, -half],
            normal: [0.0, 1.0, 0.0],
            uv: [1.0, 0.0],
            tangent: [1.0, 0.0, 0.0, 1.0],
            ..Default::default()
        },
        Vertex3D {
            position: [half, 0.0, half],
            normal: [0.0, 1.0, 0.0],
            uv: [1.0, 1.0],
            tangent: [1.0, 0.0, 0.0, 1.0],
            ..Default::default()
        },
        Vertex3D {
            position: [-half, 0.0, half],
            normal: [0.0, 1.0, 0.0],
            uv: [0.0, 1.0],
            tangent: [1.0, 0.0, 0.0, 1.0],
            ..Default::default()
        },
    ];

    let indices = vec![0, 1, 2, 2, 3, 0];

    (vertices, indices)
}

/// Well-known content-relative paths for built-in primitive meshes.
pub const PRIMITIVE_CUBE: &str = "__primitive__/Cube";
pub const PRIMITIVE_SPHERE: &str = "__primitive__/Sphere";
pub const PRIMITIVE_PLANE: &str = "__primitive__/Plane";

/// All primitive mesh paths, for UI dropdowns.
pub const PRIMITIVE_PATHS: &[&str] = &[PRIMITIVE_CUBE, PRIMITIVE_SPHERE, PRIMITIVE_PLANE];

/// Generates a UV sphere with the given number of segments and rings.
pub fn create_sphere(segments: u32, rings: u32) -> (Vec<Vertex3D>, Vec<u32>) {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();

    for ring in 0..=rings {
        let theta = std::f32::consts::PI * ring as f32 / rings as f32;
        let sin_theta = theta.sin();
        let cos_theta = theta.cos();

        for seg in 0..=segments {
            let phi = 2.0 * std::f32::consts::PI * seg as f32 / segments as f32;
            let sin_phi = phi.sin();
            let cos_phi = phi.cos();

            let x = cos_phi * sin_theta;
            let y = cos_theta;
            let z = sin_phi * sin_theta;

            let u = seg as f32 / segments as f32;
            let v = ring as f32 / rings as f32;

            // Tangent in the direction of increasing phi
            let tx = -sin_phi;
            let tz = cos_phi;

            vertices.push(Vertex3D {
                position: [x * 0.5, y * 0.5, z * 0.5],
                normal: [x, y, z],
                uv: [u, v],
                tangent: [tx, 0.0, tz, 1.0],
                ..Default::default()
            });
        }
    }

    for ring in 0..rings {
        for seg in 0..segments {
            let curr = ring * (segments + 1) + seg;
            let next = curr + segments + 1;

            if ring != 0 {
                indices.push(curr);
                indices.push(next);
                indices.push(curr + 1);
            }
            if ring != rings - 1 {
                indices.push(curr + 1);
                indices.push(next);
                indices.push(next + 1);
            }
        }
    }

    (vertices, indices)
}
