//! Disk output and headless model resolution for cooked collision.
//!
//! Cooked chunks live under `content/collision/<scene_stem>/` as
//! `<x>_<y>.ccol` files plus a `manifest.ron`. The directory is owned by the
//! cooker and fully replaced on each cook.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::cook::CookedScene;
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
    use super::scene_stem;

    #[test]
    fn scene_stem_strips_dir_and_extension() {
        assert_eq!(scene_stem("scenes/main.scene"), "main");
        assert_eq!(scene_stem("demo.scene"), "demo");
    }
}
