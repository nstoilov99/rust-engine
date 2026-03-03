use std::sync::Arc;
use glam::Vec3;
use vulkano::device::{Device, Queue};
use vulkano::command_buffer::allocator::StandardCommandBufferAllocator;
use vulkano::memory::allocator::StandardMemoryAllocator;

use super::texture_manager::TextureManager;
use super::model_manager::ModelManager;
use crate::MeshManager;
use crate::engine::math::Aabb;
use crate::engine::rendering::rendering_3d::mesh_manager::GpuMesh;
use crate::engine::rendering::rendering_3d::pipeline_3d::Vertex3D;

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
        crate::profile_function!();

        // Load model from cache
        let model_handle = self.models.load(path)?;
        let model = model_handle.get();

        // Upload to GPU mesh manager
        let mut meshes = self.meshes.write();
        let indices = {
            crate::profile_scope!("gpu_mesh_upload");
            meshes.upload_model(&model, self.allocator.clone())?
        };

        Ok((indices, model_handle))
    }

    /// Reload model from filesystem and re-upload to GPU.
    /// `fs_path` is the absolute filesystem path (from hot-reload watcher).
    pub fn reload_model_gpu(&self, fs_path: &str) -> Result<(Vec<usize>, super::handle::Handle<super::model_loader::Model>), Box<dyn std::error::Error>> {
        self.models.reload(fs_path)?;

        {
            let mut meshes = self.meshes.write();
            *meshes = MeshManager::new();
        }

        let relative = super::asset_source::to_content_relative(fs_path);
        self.load_model_gpu(&relative)
    }

    /// Upload procedural geometry to GPU, returns mesh index
    pub fn upload_procedural_mesh(
        &self,
        vertices: &[Vertex3D],
        indices: &[u32],
    ) -> Result<usize, Box<dyn std::error::Error>> {
        crate::profile_scope!("upload_procedural_mesh");

        // Compute bounding sphere for frustum culling
        let (center, radius) = compute_bounding_sphere(vertices);

        // Compute AABB for frustum culling
        let aabb = Aabb::from_points(
            vertices.iter().map(|v| Vec3::new(v.position[0], v.position[1], v.position[2])),
        );

        let mut meshes = self.meshes.write();
        let gpu_mesh = GpuMesh::new(self.allocator.clone(), vertices, indices, center, radius, aabb.min, aabb.max)?;
        let index = meshes.meshes.len();
        meshes.meshes.push(gpu_mesh);
        Ok(index)
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

/// Compute bounding sphere for a set of vertices
fn compute_bounding_sphere(vertices: &[Vertex3D]) -> (Vec3, f32) {
    if vertices.is_empty() {
        return (Vec3::ZERO, 0.0);
    }

    // Compute center as average of all positions
    let sum: Vec3 = vertices
        .iter()
        .map(|v| Vec3::new(v.position[0], v.position[1], v.position[2]))
        .sum();
    let center = sum / vertices.len() as f32;

    // Compute radius as max distance from center
    let radius = vertices
        .iter()
        .map(|v| {
            let pos = Vec3::new(v.position[0], v.position[1], v.position[2]);
            (pos - center).length()
        })
        .fold(0.0f32, f32::max);

    (center, radius)
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