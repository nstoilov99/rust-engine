use std::path::Path;
use std::sync::Arc;
use std::collections::HashMap;
use parking_lot::RwLock;
use vulkano::device::Device;
use vulkano::memory::allocator::StandardMemoryAllocator;

use super::asset_source;
use super::handle::{Handle, AssetId};
use super::model_loader::{Model, load_model, load_model_from_bytes};

/// Manages 3D model loading and caching
pub struct ModelManager {
    device: Arc<Device>,
    allocator: Arc<StandardMemoryAllocator>,
    cache: RwLock<HashMap<AssetId, Arc<Model>>>,
}

impl ModelManager {
    pub fn new(device: Arc<Device>, allocator: Arc<StandardMemoryAllocator>) -> Self {
        Self {
            device,
            allocator,
            cache: RwLock::new(HashMap::new()),
        }
    }

    /// Load model by content-relative path (e.g. `"models/Duck.glb"`).
    /// Reads via the global asset source (pak or filesystem). Caches result.
    pub fn load(&self, relative: &str) -> Result<Handle<Model>, Box<dyn std::error::Error>> {
        let id = AssetId::from_path(relative);

        {
            let cache = self.cache.read();
            if let Some(model) = cache.get(&id) {
                return Ok(Handle::new(id, model.clone()));
            }
        }

        let mut cache = self.cache.write();
        if let Some(model) = cache.get(&id) {
            return Ok(Handle::new(id, model.clone()));
        }

        println!("Loading model: {}", relative);
        let name = Path::new(relative)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Unnamed");

        let model = if asset_source::is_pak() {
            let data = asset_source::read_bytes(relative)?;
            load_model_from_bytes(&data, name)?
        } else {
            let fs_path = asset_source::resolve(relative);
            load_model(&fs_path.to_string_lossy())?
        };

        let model_arc = Arc::new(model);
        cache.insert(id, model_arc.clone());
        Ok(Handle::new(id, model_arc))
    }

    /// Reload model from filesystem (editor only, for hot-reload).
    /// `fs_path` is an absolute filesystem path.
    pub fn reload(&self, fs_path: &str) -> Result<(), Box<dyn std::error::Error>> {
        let relative = asset_source::to_content_relative(fs_path);
        let id = AssetId::from_path(&relative);

        println!("Reloading model: {}", fs_path);
        let model = load_model(fs_path)?;

        let mut cache = self.cache.write();
        cache.insert(id, Arc::new(model));
        Ok(())
    }

    pub fn clear_cache(&self) {
        self.cache.write().clear();
    }

    pub fn cache_size(&self) -> usize {
        self.cache.read().len()
    }
}