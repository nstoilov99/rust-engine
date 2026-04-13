//! RON serialization for ActionMap.
//!
//! Loads and saves input bindings from/to `config/input_bindings.ron`.

use super::action_map::ActionMap;
use std::path::{Path, PathBuf};

/// Default path for the input bindings file.
pub fn default_bindings_path() -> PathBuf {
    PathBuf::from("config/input_bindings.ron")
}

/// Load an ActionMap from a RON file. Returns None if the file doesn't exist
/// or cannot be parsed.
pub fn load_action_map(path: &Path) -> Option<ActionMap> {
    let content = std::fs::read_to_string(path).ok()?;
    match ron::from_str::<ActionMap>(&content) {
        Ok(map) => {
            log::info!("Loaded input bindings from {}", path.display());
            Some(map)
        }
        Err(e) => {
            log::warn!("Failed to parse input bindings at {}: {e}", path.display());
            None
        }
    }
}

/// Save an ActionMap to a RON file. Creates parent directories if needed.
pub fn save_action_map(map: &ActionMap, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let ron_str = ron::ser::to_string_pretty(map, Default::default())?;
    std::fs::write(path, ron_str)?;
    log::info!("Saved input bindings to {}", path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::input::default_bindings::default_action_map;

    #[test]
    fn roundtrip_action_map() {
        let map = default_action_map();
        let ron_str = ron::ser::to_string_pretty(&map, Default::default()).unwrap();
        let loaded: ActionMap = ron::from_str(&ron_str).unwrap();
        assert_eq!(map.contexts.len(), loaded.contexts.len());
        for key in map.contexts.keys() {
            assert!(loaded.contexts.contains_key(key), "missing context: {key}");
        }
    }
}
