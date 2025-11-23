use std::sync::Arc;
use std::collections::HashMap;
use parking_lot::RwLock;
use vulkano::device::Device;
use vulkano::memory::allocator::StandardMemoryAllocator;

use super::handle::{Handle, AssetId};
use super::model_loader::{Model, load_model};

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

    /// Load model from GLTF file (caches result)
    pub fn load(&self, path: &str) -> Result<Handle<Model>, Box<dyn std::error::Error>> {
        let id = AssetId::from_path(path);

        // Check cache
        {
            let cache = self.cache.read();
            if let Some(model) = cache.get(&id) {
                return Ok(Handle::new(id, model.clone()));
            }
        }

        // Load from disk
        let mut cache = self.cache.write();

        // Double-check
        if let Some(model) = cache.get(&id) {
            return Ok(Handle::new(id, model.clone()));
        }

        println!("Loading model: {}", path);
        let model = load_model(path)?;
        let model_arc = Arc::new(model);

        cache.insert(id, model_arc.clone());

        Ok(Handle::new(id, model_arc))
    }

    /// Reload model from disk
    pub fn reload(&self, path: &str) -> Result<(), Box<dyn std::error::Error>> {
        let id = AssetId::from_path(path);

        println!("Reloading model: {}", path);
        let model = load_model(path)?;

        let mut cache = self.cache.write();
        cache.insert(id, Arc::new(model));

        Ok(())
    }

    /// Clear all cached models
    pub fn clear_cache(&self) {
        let mut cache = self.cache.write();
        cache.clear();
    }

    /// Get number of cached models
    pub fn cache_size(&self) -> usize {
        self.cache.read().len()
    }
}