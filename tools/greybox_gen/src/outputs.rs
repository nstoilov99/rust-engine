//! Complete generated-output set as (content-relative path, bytes) —
//! shared by the write mode and the `--check` drift guard.

use rust_engine::engine::assets::mesh_import::mesh_binary_bytes;

use crate::params::{MAX_CELL, MIN_CELL};
use crate::{scene, terrain};

pub fn generate_all() -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    for cy in MIN_CELL..MAX_CELL {
        for cx in MIN_CELL..MAX_CELL {
            out.push((
                terrain::cell_mesh_path(cx, cy),
                mesh_binary_bytes(&terrain::cell_model(cx, cy)),
            ));
        }
    }
    out.push((
        scene::SCENE_PATH.to_string(),
        scene::scene_bytes(&scene::build_scene()),
    ));
    out.push((
        scene::MANIFEST_PATH.to_string(),
        scene::manifest_bytes(&scene::build_manifest()),
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every cell chunk the world manifest references must exist in the
    /// checked-in cooked set (the cooked set may contain extra border-sliver
    /// chunks — manifest ⊆ cooked is the invariant).
    #[test]
    fn manifest_chunks_are_subset_of_cooked_set() {
        use game_shared::collision::format::CollisionManifest;
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../content/collision/greybox/manifest.ron"
        );
        let text = std::fs::read_to_string(path)
            .expect("cooked greybox collision missing — run collision_cooker scenes/greybox.scene");
        let cooked: CollisionManifest = ron::from_str(&text).unwrap();
        let coords: std::collections::HashSet<[i32; 2]> =
            cooked.chunks.iter().map(|c| c.coord).collect();
        for cell in crate::scene::build_manifest().cells {
            assert!(
                coords.contains(&cell.coord),
                "cell {:?} chunk not in cooked set",
                cell.coord
            );
        }
    }

    /// Spawn points must sit on (or ≤1 m above) the terrain surface.
    #[test]
    fn spawn_points_rest_on_ground() {
        for sp in crate::scene::build_manifest().spawn_points {
            let [x, y, z] = sp.position;
            let h = crate::height::height_at_world(x, y);
            assert!(
                z >= h - 0.01 && z - h <= 1.01,
                "spawn '{}' z={z} vs terrain h={h}",
                sp.name
            );
        }
    }

    #[test]
    fn generation_is_deterministic() {
        let a = generate_all();
        let b = generate_all();
        assert_eq!(a.len(), b.len());
        for ((pa, ba), (pb, bb)) in a.iter().zip(b.iter()) {
            assert_eq!(pa, pb);
            assert_eq!(ba, bb, "output drifted between runs: {pa}");
        }
    }
}
