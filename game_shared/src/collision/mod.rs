//! Shared static-collision query layer.
//!
//! Cooked collision chunks (`.ccol`) are loaded and queried identically by
//! the client and the server WASM module. All cooked data and all queries
//! are Z-up world space; parry3d is the query backend (pinned to the exact
//! version the workspace lockfile resolves for rapier3d — see
//! `docs/roadmap/VULKANO-M2-COLLISION-PIPELINE.md`).
//!
//! This crate does no I/O: callers hand chunk bytes to the store.
//!
pub mod format;
pub mod store;

pub use store::{ChunkLoadError, ChunkStore, ContactHit, LoadedChunk, RayHit, ShapeHit};

/// Stable per-triangle identity: hash of source-entity GUID + source triangle
/// index. Powers seam dedup (border triangles are duplicated across chunks)
/// and golden-battery references.
pub type TriangleId = u64;

/// Per-triangle flags: material id in the low 16 bits; walkable/blocking bits
/// reserved for M6; the rest must be zero.
pub type TriangleFlags = u32;

pub const FLAGS_MATERIAL_MASK: TriangleFlags = 0xFFFF;

#[cfg(test)]
mod parry_pin {
    /// Validates the pinned parry3d API surface the plan relies on:
    /// `TriMesh::new` returns `Result` and builds its BVH internally.
    #[test]
    fn trimesh_builds() {
        use parry3d::math::Point;
        use parry3d::shape::TriMesh;

        let vertices = vec![
            Point::new(0.0_f32, 0.0, 0.0),
            Point::new(1.0, 0.0, 0.0),
            Point::new(0.0, 1.0, 0.0),
        ];
        let mesh = TriMesh::new(vertices, vec![[0u32, 1, 2]]).expect("valid trimesh");
        assert_eq!(mesh.num_triangles(), 1);
    }
}
