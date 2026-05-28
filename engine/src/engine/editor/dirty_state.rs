//! Global editor dirty-state tracking.
//!
//! Asset editors use this service to record unsaved edits independently from
//! scene command history. Keys are normalized so the same asset does not fork
//! into separate entries because of Windows case or path separator differences.

use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

#[derive(Debug, Clone)]
pub struct DirtyAsset {
    pub key: String,
    pub first_marked_at: Instant,
    pub last_marked_at: Instant,
}

#[derive(Debug, Default)]
pub struct DirtyState {
    assets: HashMap<String, DirtyAsset>,
}

impl DirtyState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_asset_dirty<P: AsRef<Path>>(&self, path: P) -> bool {
        self.assets.contains_key(&normalize_asset_key(path))
    }

    pub fn mark_asset<P: AsRef<Path>>(&mut self, path: P) {
        let key = normalize_asset_key(path);
        let now = Instant::now();
        self.assets
            .entry(key.clone())
            .and_modify(|entry| entry.last_marked_at = now)
            .or_insert(DirtyAsset {
                key,
                first_marked_at: now,
                last_marked_at: now,
            });
    }

    pub fn clear_asset<P: AsRef<Path>>(&mut self, path: P) {
        self.assets.remove(&normalize_asset_key(path));
    }

    pub fn clear_all(&mut self) {
        self.assets.clear();
    }

    pub fn dirty_asset_count(&self) -> usize {
        self.assets.len()
    }

    pub fn dirty_assets(&self) -> impl Iterator<Item = &DirtyAsset> {
        self.assets.values()
    }
}

pub fn normalize_asset_key<P: AsRef<Path>>(path: P) -> String {
    path.as_ref()
        .to_string_lossy()
        .replace('\\', "/")
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_case_and_separators() {
        let mut dirty = DirtyState::new();
        dirty.mark_asset("Content\\Materials\\Foo.material.ron");

        assert!(dirty.is_asset_dirty("content/materials/foo.material.ron"));

        dirty.clear_asset("CONTENT/MATERIALS/FOO.MATERIAL.RON");
        assert!(!dirty.is_asset_dirty("content/materials/foo.material.ron"));
    }
}
