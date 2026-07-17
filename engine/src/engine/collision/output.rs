//! Disk output and headless model resolution for cooked collision.
//!
//! Cooked chunks live under `content/collision/<scene_stem>/` as
//! `<x>_<y>.ccol` files plus a `manifest.ron`. The directory is owned by the
//! cooker and fully replaced on each cook.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use game_shared::collision::format::{fnv1a_64, CollisionManifest, FORMAT_VERSION};

use super::cook::{CookedScene, COOKER_VERSION_HASH};
use crate::engine::assets::asset_source;
use crate::engine::assets::model_loader::{self, Model};

/// Directory name for a scene's cooked collision, derived from the scene's
/// content-relative path stem (e.g. `"scenes/main.scene"` → `"main"`).
pub fn scene_stem(scene_relative: &str) -> String {
    Path::new(scene_relative)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| scene_relative.to_string())
}

/// Cooked-collision output directory for a scene under the active content
/// root. `None` if no content root has been initialized.
pub fn collision_dir_for_scene(scene_relative: &str) -> Option<PathBuf> {
    asset_source::content_root_path()
        .map(|root| root.join("collision").join(scene_stem(scene_relative)))
}

/// Headless model resolution for cooking: content-relative `mesh_path` →
/// CPU-side model (no GPU upload, no cache). Load failures surface as cook
/// warnings, so errors are folded into `None` here.
pub fn load_model_from_content(mesh_path: &str) -> Option<Arc<Model>> {
    let abs = asset_source::resolve(mesh_path);
    model_loader::load_model(&abs.to_string_lossy())
        .ok()
        .map(Arc::new)
}

/// Content-relative world-manifest path for a scene (M4 streaming):
/// `world/<stem>.world.ron`.
pub fn world_manifest_rel(scene_relative: &str) -> String {
    format!("world/{}.world.ron", scene_stem(scene_relative))
}

/// Extra cook sources from the scene's world manifest, if one exists:
/// (cell root GUID, mesh path), cooked at identity transform since cell
/// meshes bake world placement. Read/parse failures degrade to an empty list
/// (a manifest-less scene simply has no streamed cells).
pub fn manifest_cell_sources(scene_relative: &str) -> Vec<(uuid::Uuid, String)> {
    let Ok(text) = asset_source::read_string(&world_manifest_rel(scene_relative)) else {
        return Vec::new();
    };
    let Ok(m) = ron::from_str::<game_shared::world_manifest::WorldManifest>(&text) else {
        return Vec::new();
    };
    m.cells
        .iter()
        .filter_map(|c| {
            uuid::Uuid::parse_str(&c.root_entity_guid)
                .ok()
                .map(|guid| (guid, c.mesh.clone()))
        })
        .collect()
}

/// Staleness hash for the manifest's `scene_hash`: `fnv1a_64` of the scene
/// file bytes, folded with the world-manifest bytes (if any) and the bytes of
/// every mesh FILE that feeds the cook (`StaticCollision` entities plus
/// world-manifest cells). Scene-byte hashing alone is not enough — the
/// common generator iteration (tweak height function → meshes change, scene
/// byte-identical) must not silently skip re-cooking.
pub fn scene_content_hash(scene_relative: &str) -> Option<u64> {
    let fold_in = |hash: u64, path: &str, content_hash: u64| {
        let mut fold = Vec::with_capacity(8 + path.len() + 8);
        fold.extend_from_slice(&hash.to_le_bytes());
        fold.extend_from_slice(path.as_bytes());
        fold.extend_from_slice(&content_hash.to_le_bytes());
        fnv1a_64(&fold)
    };

    let scene_bytes = asset_source::read_bytes(scene_relative).ok()?;
    let mut hash = fnv1a_64(&scene_bytes);
    let mut paths = collision_mesh_paths(&scene_bytes);

    let manifest_rel = world_manifest_rel(scene_relative);
    if let Ok(bytes) = asset_source::read_bytes(&manifest_rel) {
        hash = fold_in(hash, &manifest_rel, fnv1a_64(&bytes));
        paths.extend(
            manifest_cell_sources(scene_relative)
                .into_iter()
                .map(|(_, mesh)| mesh),
        );
        paths.sort();
        paths.dedup();
    }

    for path in paths {
        let mesh_hash = asset_source::read_bytes(&path)
            .ok()
            .map(|b| fnv1a_64(&b))
            .unwrap_or(0); // missing file still folds the path → hash flips when it appears
        hash = fold_in(hash, &path, mesh_hash);
    }
    Some(hash)
}

/// Content-relative mesh files whose bytes affect cooked collision: the
/// non-primitive `mesh_path` of every entity carrying `StaticCollision`.
/// Sorted + deduped for a stable fold order. Parse failures degrade to an
/// empty list (the scene-byte hash still guards those).
fn collision_mesh_paths(scene_bytes: &[u8]) -> Vec<String> {
    use crate::engine::scene::scene_format::{ComponentData, SceneFile};
    let Ok(text) = std::str::from_utf8(scene_bytes) else {
        return Vec::new();
    };
    let Ok(scene) = ron::from_str::<SceneFile>(text) else {
        return Vec::new();
    };
    let mut paths: Vec<String> = scene
        .entities
        .iter()
        .filter(|e| {
            e.components
                .iter()
                .any(|c| matches!(c, ComponentData::StaticCollision))
        })
        .filter_map(|e| {
            e.components.iter().find_map(|c| match c {
                ComponentData::MeshRenderer { mesh_path, .. }
                    if !mesh_path.is_empty() && !mesh_path.starts_with("__primitive__/") =>
                {
                    Some(mesh_path.clone())
                }
                _ => None,
            })
        })
        .collect();
    paths.sort();
    paths.dedup();
    paths
}

/// Cheap staleness check: `true` iff a manifest exists for the scene and was
/// produced by the current cooker/format from byte-identical scene contents.
/// Conservative — any read/parse failure means "not current".
pub fn cook_is_current(scene_relative: &str) -> bool {
    let Some(hash) = scene_content_hash(scene_relative) else {
        return false;
    };
    let manifest_rel = format!("collision/{}/manifest.ron", scene_stem(scene_relative));
    let Ok(text) = asset_source::read_string(&manifest_rel) else {
        return false;
    };
    let Ok(m) = ron::from_str::<CollisionManifest>(&text) else {
        return false;
    };
    m.format_version == FORMAT_VERSION
        && m.cooker_hash == COOKER_VERSION_HASH
        && m.scene_hash == hash
}

/// Full headless cook for one scene: load into a fresh world, cook, write.
/// Returns (chunk count, cook warnings). Used by build/export paths that must
/// guarantee current collision before packing.
pub fn cook_scene_headless(scene_relative: &str) -> Result<(usize, Vec<String>), String> {
    let mut world = crate::engine::ecs::World::new();
    crate::engine::scene::scene_serializer::load_scene(&mut world, scene_relative)
        .map_err(|e| format!("load '{scene_relative}': {e}"))?;
    let stem = scene_stem(scene_relative);
    let cells = manifest_cell_sources(scene_relative);
    let mut loader = load_model_from_content;
    let mut cooked = super::cook::cook_scene(&world, &stem, &cells, &mut loader);
    cooked.manifest.scene_hash = scene_content_hash(scene_relative).unwrap_or(0);
    let dir = collision_dir_for_scene(scene_relative)
        .ok_or_else(|| "no content root initialized".to_string())?;
    write_cooked_scene(&dir, &cooked).map_err(|e| format!("write '{}': {e}", dir.display()))?;
    Ok((cooked.chunks.len(), cooked.warnings))
}

/// Write chunks + manifest to `dir`, replacing any previous cook output
/// (the directory is cooker-owned build output).
pub fn write_cooked_scene(dir: &Path, cooked: &CookedScene) -> io::Result<()> {
    if dir.exists() {
        fs::remove_dir_all(dir)?;
    }
    fs::create_dir_all(dir)?;
    for (coord, bytes) in &cooked.chunks {
        fs::write(dir.join(format!("{}_{}.ccol", coord.x, coord.y)), bytes)?;
    }
    let manifest = ron::ser::to_string_pretty(&cooked.manifest, ron::ser::PrettyConfig::default())
        .map_err(io::Error::other)?;
    fs::write(dir.join("manifest.ron"), manifest)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{collision_mesh_paths, scene_stem};

    #[test]
    fn scene_stem_strips_dir_and_extension() {
        assert_eq!(scene_stem("scenes/main.scene"), "main");
        assert_eq!(scene_stem("demo.scene"), "demo");
    }

    #[test]
    fn collision_mesh_paths_filters_and_dedups() {
        let scene = r#"(
            version: "1.0",
            name: "t",
            entities: [
                (name: "a", components: [
                    (type: "MeshRenderer", mesh_path: "models/b.mesh"),
                    (type: "StaticCollision"),
                ]),
                (name: "a2", components: [
                    (type: "MeshRenderer", mesh_path: "models/b.mesh"),
                    (type: "StaticCollision"),
                ]),
                (name: "prim", components: [
                    (type: "MeshRenderer", mesh_path: "__primitive__/Cube"),
                    (type: "StaticCollision"),
                ]),
                (name: "no_collision", components: [
                    (type: "MeshRenderer", mesh_path: "models/c.mesh"),
                ]),
            ],
        )"#;
        assert_eq!(collision_mesh_paths(scene.as_bytes()), vec!["models/b.mesh"]);
    }
}
