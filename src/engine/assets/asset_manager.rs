use std::sync::Arc;
use vulkano::device::{Device, Queue};
use vulkano::command_buffer::allocator::StandardCommandBufferAllocator;
use vulkano::memory::allocator::StandardMemoryAllocator;

use super::texture_manager::TextureManager;
use super::model_manager::ModelManager;
use crate::MeshManager;

/// Master asset manager - provides access to all asset types
pub struct AssetManager {
    pub textures: TextureManager,
    pub models: ModelManager,
    pub meshes: Arc<parking_lot::RwLock<MeshManager>>,
    allocator: Arc<StandardMemoryAllocator>,
}

impl AssetManager {
    pub fn new(
        device: Arc<Device>,
        queue: Arc<Queue>,
        allocator: Arc<StandardMemoryAllocator>,
        command_buffer_allocator: Arc<StandardCommandBufferAllocator>,
    ) -> Self {
        Self {
            textures: TextureManager::new(
                device.clone(),
                queue.clone(),
                allocator.clone(),
                command_buffer_allocator.clone(),
            ),
            models: ModelManager::new(device.clone(), allocator.clone()),
            meshes: Arc::new(parking_lot::RwLock::new(MeshManager::new())),
            allocator,
        }
    }

    /// Load a model and upload it to GPU, returns (mesh indices, model handle)
    pub fn load_model_gpu(&self, path: &str) -> Result<(Vec<usize>, super::handle::Handle<super::model_loader::Model>), Box<dyn std::error::Error>> {
        // Load model from cache
        let model_handle = self.models.load(path)?;
        let model = model_handle.get();

        // Upload to GPU mesh manager
        let mut meshes = self.meshes.write();
        let indices = meshes.upload_model(&model, self.allocator.clone())?;

        Ok((indices, model_handle))
    }

    /// Reload model and re-upload to GPU, returns (mesh indices, model handle)
    pub fn reload_model_gpu(&self, path: &str) -> Result<(Vec<usize>, super::handle::Handle<super::model_loader::Model>), Box<dyn std::error::Error>> {
        // Reload from disk
        self.models.reload(path)?;

        // Clear old GPU meshes and re-upload
        {
            let mut meshes = self.meshes.write();
            *meshes = MeshManager::new(); // Clear old meshes
        }

        // Load and upload new version
        self.load_model_gpu(path)
    }

    /// Clear all caches
    pub fn clear_all_caches(&self) {
        self.textures.clear_cache();
        self.models.clear_cache();
    }

    /// Get total cache statistics
    pub fn cache_stats(&self) -> CacheStats {
        CacheStats {
            textures: self.textures.cache_size(),
            models: self.models.cache_size(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CacheStats {
    pub textures: usize,
    pub models: usize,
}

impl std::fmt::Display for CacheStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Textures: {}, Models: {}", self.textures, self.models)
    }
}