//! Per-cell terrain mesh emission (RMSH, Y-up render-local convention).
//!
//! Cell (cx, cy) covers world `[cx·64, (cx+1)·64) × [cy·64, (cy+1)·64)` with
//! 33×33 vertices on the integer global grid — vertex (i, j) is global sample
//! `(cx·32 + i, cy·32 + j)`, so border vertices of adjacent cells are
//! bit-identical. World placement is baked into the vertices (entities use
//! identity transforms; cooked chunks land exactly cell-aligned).

use glam::Vec3;
use rust_engine::engine::assets::model_loader::{ImportedMaterial, LoadedMesh, Model};
use rust_engine::engine::rendering::rendering_3d::pipeline_3d::Vertex3D;
use rust_engine::engine::utils::coords::convert_position_zup_to_yup;

use crate::height::height_at;
use crate::params::{CELL_QUADS, SPACING};

const VERTS: i32 = CELL_QUADS + 1; // 33

pub fn cell_mesh_name(cx: i32, cy: i32) -> String {
    format!("cell_{cx}_{cy}")
}

/// Content-relative mesh path for a cell.
pub fn cell_mesh_path(cx: i32, cy: i32) -> String {
    format!("models/greybox/{}.mesh", cell_mesh_name(cx, cy))
}

/// Terrain normal (Z-up) from central differences on the global grid —
/// neighbor cells sample the same integers, so seam normals match too.
fn normal_zup(gx: i32, gy: i32) -> Vec3 {
    let dx = (height_at(gx + 1, gy) - height_at(gx - 1, gy)) / (2.0 * SPACING);
    let dy = (height_at(gx, gy + 1) - height_at(gx, gy - 1)) / (2.0 * SPACING);
    Vec3::new(-dx, -dy, 1.0).normalize()
}

/// Build the render/collision model for one cell.
pub fn cell_model(cx: i32, cy: i32) -> Model {
    let mut vertices = Vec::with_capacity((VERTS * VERTS) as usize);
    let mut min = Vec3::MAX;
    let mut max = Vec3::MIN;

    for j in 0..VERTS {
        for i in 0..VERTS {
            let gx = cx * CELL_QUADS + i;
            let gy = cy * CELL_QUADS + j;
            let pos_zup = Vec3::new(
                gx as f32 * SPACING,
                gy as f32 * SPACING,
                height_at(gx, gy),
            );
            let p = convert_position_zup_to_yup(pos_zup);
            let n = convert_position_zup_to_yup(normal_zup(gx, gy));
            min = min.min(p);
            max = max.max(p);
            vertices.push(Vertex3D {
                position: p.to_array(),
                normal: n.to_array(),
                uv: [i as f32 / CELL_QUADS as f32, j as f32 / CELL_QUADS as f32],
                tangent: [1.0, 0.0, 0.0, 1.0],
                ..Default::default()
            });
        }
    }

    // Same winding relationship as the engine's up-facing Plane primitive.
    let idx = |i: i32, j: i32| (j * VERTS + i) as u32;
    let mut indices = Vec::with_capacity((CELL_QUADS * CELL_QUADS * 6) as usize);
    for j in 0..CELL_QUADS {
        for i in 0..CELL_QUADS {
            indices.extend_from_slice(&[
                idx(i + 1, j),
                idx(i + 1, j + 1),
                idx(i, j + 1),
                idx(i, j + 1),
                idx(i, j),
                idx(i + 1, j),
            ]);
        }
    }

    let center = (min + max) * 0.5;
    let radius = vertices
        .iter()
        .map(|v| Vec3::from_array(v.position).distance(center))
        .fold(0.0f32, f32::max);

    let mut model = Model::new(cell_mesh_name(cx, cy));
    model.materials.push(ImportedMaterial {
        name: "greybox".into(),
        base_color_factor: [0.5, 0.5, 0.5, 1.0],
        ..Default::default()
    });
    model.meshes.push(LoadedMesh {
        vertices,
        indices,
        material_index: Some(0),
        center,
        radius,
        aabb_min: min,
        aabb_max: max,
        skinning: None,
    });
    model
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::{MAX_CELL, MIN_CELL};

    fn border_column(model: &Model, i: i32) -> Vec<[f32; 3]> {
        (0..VERTS)
            .map(|j| model.meshes[0].vertices[(j * VERTS + i) as usize].position)
            .collect()
    }

    fn border_row(model: &Model, j: i32) -> Vec<[f32; 3]> {
        (0..VERTS)
            .map(|i| model.meshes[0].vertices[(j * VERTS + i) as usize].position)
            .collect()
    }

    #[test]
    fn adjacent_cells_share_bit_identical_borders() {
        // Includes the negative→zero crossings; corners are covered where
        // rows and columns intersect.
        for c in MIN_CELL..MAX_CELL - 1 {
            let a = cell_model(c, 0);
            let b = cell_model(c + 1, 0);
            assert_eq!(border_column(&a, CELL_QUADS), border_column(&b, 0), "x seam at cell {c}");
            let a = cell_model(0, c);
            let b = cell_model(0, c + 1);
            assert_eq!(border_row(&a, CELL_QUADS), border_row(&b, 0), "y seam at cell {c}");
        }
    }

    #[test]
    fn cell_geometry_is_world_baked() {
        let m = cell_model(-4, -4);
        // First vertex is global sample (-128, -128) → world (-256, -256, h).
        let p = m.meshes[0].vertices[0].position;
        let zup = rust_engine::engine::utils::coords::convert_position_yup_to_zup(
            glam::Vec3::from_array(p),
        );
        assert_eq!(zup.x, -256.0);
        assert_eq!(zup.y, -256.0);
    }
}
