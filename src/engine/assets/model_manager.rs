use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use vulkano::device::Device;
use vulkano::memory::allocator::StandardMemoryAllocator;

use super::asset_source;
use super::handle::{AssetId, Handle};
use super::model_loader::{load_model, load_model_from_bytes, Model};

/// Manages 3D model loading and caching
pub struct ModelManager {
    _device: Arc<Device>,
    _allocator: Arc<StandardMemoryAllocator>,
    cache: RwLock<HashMap<AssetId, Arc<Model>>>,
}

impl ModelManager {
    pub fn new(device: Arc<Device>, allocator: Arc<StandardMemoryAllocator>) -> Self {
        Self {
            _device: device,
            _allocator: allocator,
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

        // Prefer native .mesh over source formats for fast loading
        let mesh_relative = mesh_path_for(relative);
        let effective_path = if !asset_source::is_pak()
            && asset_source::resolve(&mesh_relative).exists()
        {
            println!("Loading native mesh: {}", mesh_relative);
            mesh_relative
        } else {
            println!("Loading model: {}", relative);
            relative.to_string()
        };

        let model = if asset_source::is_pak() {
            let data = asset_source::read_bytes(&effective_path)?;
            load_model_from_bytes(&data, &effective_path)?
        } else {
            let fs_path = asset_source::resolve(&effective_path);
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

/// Convert a source model path to its native `.mesh` sibling path.
/// e.g. `"models/Duck.glb"` → `"models/Duck.mesh"`
fn mesh_path_for(relative: &str) -> String {
    let p = std::path::Path::new(relative);
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("unknown");
    if let Some(parent) = p.parent().filter(|p| !p.as_os_str().is_empty()) {
        format!("{}/{}.mesh", parent.display(), stem)
    } else {
        format!("{}.mesh", stem)
    }
}
