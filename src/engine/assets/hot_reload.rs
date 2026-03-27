use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use parking_lot::RwLock;
use std::collections::HashSet;
use std::path::Path;
use std::sync::mpsc;
use std::sync::Arc;

use super::asset_manager::AssetManager;

use super::model_loader::Model;

/// Hot-reload event messages
#[derive(Debug, Clone)]
pub enum ReloadEvent {
    ModelChanged {
        path: String,
        mesh_indices: Vec<usize>,
        model: std::sync::Arc<Model>,
    },
    TextureChanged {
        path: String,
    },
    ReloadFailed {
        path: String,
        error: String,
    },
}

/// Watches asset files and triggers hot-reload
pub struct HotReloadWatcher {
    assets: Arc<AssetManager>,
    watched_paths: Arc<RwLock<HashSet<String>>>,
    _watcher: Option<RecommendedWatcher>,
    reload_sender: mpsc::Sender<ReloadEvent>,
}

impl HotReloadWatcher {
    pub fn new(assets: Arc<AssetManager>, reload_sender: mpsc::Sender<ReloadEvent>) -> Self {
        Self {
            assets,
            watched_paths: Arc::new(RwLock::new(HashSet::new())),
            _watcher: None,
            reload_sender,
        }
    }

    /// Start watching a directory for changes
    pub fn watch_directory(&mut self, path: &str) -> Result<(), Box<dyn std::error::Error>> {
        let assets = self.assets.clone();
        let watched_paths = self.watched_paths.clone();
        let reload_sender = self.reload_sender.clone();

        let watcher = notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
            if let Ok(event) = res {
                match event.kind {
                    EventKind::Modify(_)
                    | EventKind::Create(_)
                    | EventKind::Remove(_)
                    | EventKind::Any => {
                        for path in event.paths {
                            if let Some(path_str) = path.to_str() {
                                // Normalize path separators to forward slashes
                                let normalized_path = path_str.replace('\\', "/");

                                // Check if we're tracking this asset (check both absolute and relative paths)
                                let watched = watched_paths.read();
                                let is_tracked = watched.iter().any(|tracked| {
                                    // Check if the normalized path ends with the tracked path
                                    normalized_path.ends_with(tracked)
                                });

                                if is_tracked {
                                    // Find the matching tracked path
                                    let tracked_path = watched
                                        .iter()
                                        .find(|tracked| normalized_path.ends_with(*tracked))
                                        .unwrap()
                                        .clone();

                                    // Determine asset type and reload (use tracked_path for consistency)
                                    if tracked_path.ends_with(".png")
                                        || tracked_path.ends_with(".jpg")
                                        || tracked_path.ends_with(".jpeg")
                                    {
                                        match assets.textures.reload(&tracked_path) {
                                            Ok(_) => {
                                                let _ = reload_sender.send(
                                                    ReloadEvent::TextureChanged {
                                                        path: tracked_path.clone(),
                                                    },
                                                );
                                            }
                                            Err(e) => {
                                                eprintln!("Failed to reload texture: {}", e);
                                                let _ =
                                                    reload_sender.send(ReloadEvent::ReloadFailed {
                                                        path: tracked_path.clone(),
                                                        error: e.to_string(),
                                                    });
                                            }
                                        }
                                    } else if {
                                        let ext = Path::new(&tracked_path)
                                            .extension()
                                            .and_then(|e| e.to_str())
                                            .unwrap_or("")
                                            .to_lowercase();
                                        matches!(ext.as_str(), "gltf" | "glb" | "fbx" | "obj")
                                    } {
                                        match assets.reload_model_gpu(&tracked_path) {
                                            Ok((new_indices, model_handle)) => {
                                                let _ =
                                                    reload_sender.send(ReloadEvent::ModelChanged {
                                                        path: tracked_path.clone(),
                                                        mesh_indices: new_indices,
                                                        model: model_handle.get_arc(),
                                                    });
                                            }
                                            Err(e) => {
                                                eprintln!("Failed to reload model: {}", e);
                                                let _ =
                                                    reload_sender.send(ReloadEvent::ReloadFailed {
                                                        path: tracked_path.clone(),
                                                        error: e.to_string(),
                                                    });
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        })?;

        // Store watcher to keep it alive
        self._watcher = Some(watcher);

        // Start watching the directory
        if let Some(watcher) = &mut self._watcher {
            watcher.watch(Path::new(path), RecursiveMode::Recursive)?;
        }

        Ok(())
    }

    /// Track an asset path for hot-reload
    pub fn track_asset(&self, path: &str) {
        let normalized_path = path.replace('\\', "/");
        let mut watched = self.watched_paths.write();
        watched.insert(normalized_path.clone());
    }

    /// Untrack an asset path
    pub fn untrack_asset(&self, path: &str) {
        let normalized_path = path.replace('\\', "/");
        let mut watched = self.watched_paths.write();
        watched.remove(&normalized_path);
    }

    /// Get list of tracked assets
    pub fn tracked_assets(&self) -> Vec<String> {
        let watched = self.watched_paths.read();
        watched.iter().cloned().collect()
    }

    /// Clear all tracked assets
    pub fn clear_tracked(&self) {
        let mut watched = self.watched_paths.write();
        watched.clear();
    }
}
