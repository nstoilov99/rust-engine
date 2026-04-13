//! Audio asset cache — follows the TextureManager pattern.

use kira::sound::static_sound::StaticSoundData;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Arc;

use crate::engine::assets::handle::{AssetId, Handle};

/// Manages loading and caching of audio assets (StaticSoundData).
///
/// Thread-safe via `RwLock`. Audio data is kept fully in memory
/// (no GPU upload needed — unlike textures).
pub struct AudioAssetManager {
    cache: RwLock<HashMap<AssetId, Arc<StaticSoundData>>>,
}

impl Default for AudioAssetManager {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioAssetManager {
    pub fn new() -> Self {
        Self {
            cache: RwLock::new(HashMap::new()),
        }
    }

    /// Load audio from a content-relative path. Caches the result.
    pub fn load(&self, path: &str) -> Result<Handle<StaticSoundData>, Box<dyn std::error::Error>> {
        let id = AssetId::from_path(path);

        // Check cache first (read lock)
        {
            let cache = self.cache.read();
            if let Some(data) = cache.get(&id) {
                return Ok(Handle::new(id, data.clone()));
            }
        }

        // Not in cache — load (write lock with double-check)
        let mut cache = self.cache.write();
        if let Some(data) = cache.get(&id) {
            return Ok(Handle::new(id, data.clone()));
        }

        let data = self.load_from_disk(path)?;
        cache.insert(id, data.clone());
        Ok(Handle::new(id, data))
    }

    /// Reload audio from disk, updating the cache entry.
    pub fn reload(&self, path: &str) -> Result<(), Box<dyn std::error::Error>> {
        let id = AssetId::from_path(path);
        let data = self.load_from_disk(path)?;
        let mut cache = self.cache.write();
        cache.insert(id, data);
        Ok(())
    }

    /// Remove a single entry from cache.
    pub fn evict(&self, path: &str) {
        let id = AssetId::from_path(path);
        let mut cache = self.cache.write();
        cache.remove(&id);
    }

    /// Clear all cached audio.
    pub fn clear_cache(&self) {
        let mut cache = self.cache.write();
        cache.clear();
    }

    /// Number of cached entries.
    pub fn cache_size(&self) -> usize {
        self.cache.read().len()
    }

    /// Internal: load audio bytes via `asset_source` and decode with Kira/symphonia.
    fn load_from_disk(
        &self,
        relative: &str,
    ) -> Result<Arc<StaticSoundData>, Box<dyn std::error::Error>> {
        use crate::engine::assets::asset_source;

        let bytes = if asset_source::is_pak() {
            asset_source::read_bytes(relative)?
        } else {
            let fs_path = asset_source::resolve(relative);
            std::fs::read(&fs_path)?
        };

        let cursor = Cursor::new(bytes);
        let sound_data = StaticSoundData::from_cursor(cursor)?;
        Ok(Arc::new(sound_data))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_cache_is_empty() {
        let mgr = AudioAssetManager::new();
        assert_eq!(mgr.cache_size(), 0);
    }

    #[test]
    fn clear_cache_empties() {
        let mgr = AudioAssetManager::new();
        // Can't load real files in unit tests without asset_source init,
        // but we can verify the clear path.
        mgr.clear_cache();
        assert_eq!(mgr.cache_size(), 0);
    }
}
