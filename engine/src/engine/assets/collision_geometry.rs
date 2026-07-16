//! Canonical collision-geometry accessor: Z-up-local mesh positions.
//!
//! `.mesh` (RMSH) vertices are stored in Y-up render space — the importer
//! applies the Z-up→Y-up axis conversion and import scale once, at import
//! time. The renderer then draws them with
//! `render_adapter::world_matrix_to_render(M_zup) = C · M_zup · C⁻¹`, so a
//! rendered vertex's Z-up world position is `M_zup · (C⁻¹ · v_yup)`.
//!
//! The canonical Z-up LOCAL position is therefore `C⁻¹ · v_yup`
//! (`coords::convert_position_yup_to_zup`) — composing it with the same
//! `local_matrix_zup()` hierarchy the renderer trusts reproduces exactly
//! what is drawn. The collision cooker and any future consumer of collision
//! geometry MUST go through this accessor; reading raw RMSH positions and
//! applying Z-up transforms directly double-converts.

use glam::Vec3;

use super::model_loader::{LoadedMesh, Model};
use crate::engine::utils::coords::convert_position_yup_to_zup;

pub struct CollisionMeshGeometry {
    /// Positions in Z-up local (model) space.
    pub positions: Vec<Vec3>,
    /// Triangle list indices into `positions`.
    pub indices: Vec<u32>,
}

/// Z-up-local collision geometry for one mesh. Returns `None` for skinned
/// meshes — they are not valid static collision sources.
pub fn mesh_collision_geometry_zup(mesh: &LoadedMesh) -> Option<CollisionMeshGeometry> {
    if mesh.skinning.is_some() {
        return None;
    }
    Some(CollisionMeshGeometry {
        positions: mesh
            .vertices
            .iter()
            .map(|v| convert_position_yup_to_zup(Vec3::from_array(v.position)))
            .collect(),
        indices: mesh.indices.clone(),
    })
}

/// Z-up-local collision geometry for every static mesh in a model, plus the
/// number of skinned meshes that were skipped (callers should warn if > 0).
pub fn model_collision_geometry_zup(model: &Model) -> (Vec<CollisionMeshGeometry>, usize) {
    let mut out = Vec::with_capacity(model.meshes.len());
    let mut skipped = 0;
    for mesh in &model.meshes {
        match mesh_collision_geometry_zup(mesh) {
            Some(g) => out.push(g),
            None => skipped += 1,
        }
    }
    (out, skipped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::rendering::rendering_3d::pipeline_3d::Vertex3D;

    fn mesh_with(positions: &[[f32; 3]], skinned: bool) -> LoadedMesh {
        LoadedMesh {
            vertices: positions
                .iter()
                .map(|&position| Vertex3D {
                    position,
                    ..Default::default()
                })
                .collect(),
            indices: vec![0, 1, 2],
            material_index: None,
            center: Vec3::ZERO,
            radius: 1.0,
            aabb_min: Vec3::ZERO,
            aabb_max: Vec3::ONE,
            skinning: skinned.then(Vec::new),
        }
    }

    #[test]
    fn inverts_render_basis_change() {
        // Y-up (right=X, up=Y, forward=-Z) → Z-up (forward=X, right=Y, up=Z).
        let mesh = mesh_with(&[[1.0, 2.0, 3.0]], false);
        let g = mesh_collision_geometry_zup(&mesh).unwrap();
        assert_eq!(g.positions[0], Vec3::new(-3.0, 1.0, 2.0));
    }

    #[test]
    fn rejects_skinned() {
        let mesh = mesh_with(&[[0.0; 3]], true);
        assert!(mesh_collision_geometry_zup(&mesh).is_none());
    }
}
