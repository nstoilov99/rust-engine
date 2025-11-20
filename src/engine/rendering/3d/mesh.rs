use crate::engine::rendering::rendering_3d::pipeline_3d::Vertex3D;

/// Generates a unit cube (1×1×1) centered at origin
pub fn create_cube() -> (Vec<Vertex3D>, Vec<u32>) {
    // 8 unique vertices (cube corners)
    // But we need 24 (4 per face) for proper normals
    let vertices = vec![
        // Front face (Z+) - tangent points right (X+)
        Vertex3D { position: [-0.5, -0.5,  0.5], normal: [0.0, 0.0, 1.0], uv: [0.0, 1.0], tangent: [1.0, 0.0, 0.0, 1.0] },
        Vertex3D { position: [ 0.5, -0.5,  0.5], normal: [0.0, 0.0, 1.0], uv: [1.0, 1.0], tangent: [1.0, 0.0, 0.0, 1.0] },
        Vertex3D { position: [ 0.5,  0.5,  0.5], normal: [0.0, 0.0, 1.0], uv: [1.0, 0.0], tangent: [1.0, 0.0, 0.0, 1.0] },
        Vertex3D { position: [-0.5,  0.5,  0.5], normal: [0.0, 0.0, 1.0], uv: [0.0, 0.0], tangent: [1.0, 0.0, 0.0, 1.0] },

        // Back face (Z-) - tangent points left (X-)
        Vertex3D { position: [ 0.5, -0.5, -0.5], normal: [0.0, 0.0, -1.0], uv: [0.0, 1.0], tangent: [-1.0, 0.0, 0.0, 1.0] },
        Vertex3D { position: [-0.5, -0.5, -0.5], normal: [0.0, 0.0, -1.0], uv: [1.0, 1.0], tangent: [-1.0, 0.0, 0.0, 1.0] },
        Vertex3D { position: [-0.5,  0.5, -0.5], normal: [0.0, 0.0, -1.0], uv: [1.0, 0.0], tangent: [-1.0, 0.0, 0.0, 1.0] },
        Vertex3D { position: [ 0.5,  0.5, -0.5], normal: [0.0, 0.0, -1.0], uv: [0.0, 0.0], tangent: [-1.0, 0.0, 0.0, 1.0] },

        // Right face (X+) - tangent points back (Z-)
        Vertex3D { position: [ 0.5, -0.5,  0.5], normal: [1.0, 0.0, 0.0], uv: [0.0, 1.0], tangent: [0.0, 0.0, -1.0, 1.0] },
        Vertex3D { position: [ 0.5, -0.5, -0.5], normal: [1.0, 0.0, 0.0], uv: [1.0, 1.0], tangent: [0.0, 0.0, -1.0, 1.0] },
        Vertex3D { position: [ 0.5,  0.5, -0.5], normal: [1.0, 0.0, 0.0], uv: [1.0, 0.0], tangent: [0.0, 0.0, -1.0, 1.0] },
        Vertex3D { position: [ 0.5,  0.5,  0.5], normal: [1.0, 0.0, 0.0], uv: [0.0, 0.0], tangent: [0.0, 0.0, -1.0, 1.0] },

        // Left face (X-) - tangent points forward (Z+)
        Vertex3D { position: [-0.5, -0.5, -0.5], normal: [-1.0, 0.0, 0.0], uv: [0.0, 1.0], tangent: [0.0, 0.0, 1.0, 1.0] },
        Vertex3D { position: [-0.5, -0.5,  0.5], normal: [-1.0, 0.0, 0.0], uv: [1.0, 1.0], tangent: [0.0, 0.0, 1.0, 1.0] },
        Vertex3D { position: [-0.5,  0.5,  0.5], normal: [-1.0, 0.0, 0.0], uv: [1.0, 0.0], tangent: [0.0, 0.0, 1.0, 1.0] },
        Vertex3D { position: [-0.5,  0.5, -0.5], normal: [-1.0, 0.0, 0.0], uv: [0.0, 0.0], tangent: [0.0, 0.0, 1.0, 1.0] },

        // Top face (Y+) - tangent points right (X+)
        Vertex3D { position: [-0.5,  0.5,  0.5], normal: [0.0, 1.0, 0.0], uv: [0.0, 1.0], tangent: [1.0, 0.0, 0.0, 1.0] },
        Vertex3D { position: [ 0.5,  0.5,  0.5], normal: [0.0, 1.0, 0.0], uv: [1.0, 1.0], tangent: [1.0, 0.0, 0.0, 1.0] },
        Vertex3D { position: [ 0.5,  0.5, -0.5], normal: [0.0, 1.0, 0.0], uv: [1.0, 0.0], tangent: [1.0, 0.0, 0.0, 1.0] },
        Vertex3D { position: [-0.5,  0.5, -0.5], normal: [0.0, 1.0, 0.0], uv: [0.0, 0.0], tangent: [1.0, 0.0, 0.0, 1.0] },

        // Bottom face (Y-) - tangent points right (X+)
        Vertex3D { position: [-0.5, -0.5, -0.5], normal: [0.0, -1.0, 0.0], uv: [0.0, 1.0], tangent: [1.0, 0.0, 0.0, 1.0] },
        Vertex3D { position: [ 0.5, -0.5, -0.5], normal: [0.0, -1.0, 0.0], uv: [1.0, 1.0], tangent: [1.0, 0.0, 0.0, 1.0] },
        Vertex3D { position: [ 0.5, -0.5,  0.5], normal: [0.0, -1.0, 0.0], uv: [1.0, 0.0], tangent: [1.0, 0.0, 0.0, 1.0] },
        Vertex3D { position: [-0.5, -0.5,  0.5], normal: [0.0, -1.0, 0.0], uv: [0.0, 0.0], tangent: [1.0, 0.0, 0.0, 1.0] },
    ];

    // 6 faces × 2 triangles × 3 indices = 36 indices
    let indices = vec![
        // Front
        0, 1, 2,  2, 3, 0,
        // Back
        4, 5, 6,  6, 7, 4,
        // Right
        8, 9, 10,  10, 11, 8,
        // Left
        12, 13, 14,  14, 15, 12,
        // Top
        16, 17, 18,  18, 19, 16,
        // Bottom
        20, 21, 22,  22, 23, 20,
    ];

    (vertices, indices)
}

/// Generates a ground plane (XZ plane)
pub fn create_plane(size: f32) -> (Vec<Vertex3D>, Vec<u32>) {
    let half = size / 2.0;

    // Plane facing up (Y+) with tangent pointing right (X+)
    let vertices = vec![
        Vertex3D { position: [-half, 0.0, -half], normal: [0.0, 1.0, 0.0], uv: [0.0, 0.0], tangent: [1.0, 0.0, 0.0, 1.0] },
        Vertex3D { position: [ half, 0.0, -half], normal: [0.0, 1.0, 0.0], uv: [1.0, 0.0], tangent: [1.0, 0.0, 0.0, 1.0] },
        Vertex3D { position: [ half, 0.0,  half], normal: [0.0, 1.0, 0.0], uv: [1.0, 1.0], tangent: [1.0, 0.0, 0.0, 1.0] },
        Vertex3D { position: [-half, 0.0,  half], normal: [0.0, 1.0, 0.0], uv: [0.0, 1.0], tangent: [1.0, 0.0, 0.0, 1.0] },
    ];

    let indices = vec![0, 1, 2,  2, 3, 0];

    (vertices, indices)
}