use crate::engine::rendering::rendering_3d::pipeline_3d::Vertex3D;
use glam::Vec3;
use std::sync::Arc;
use vulkano::buffer::{Buffer, BufferCreateInfo, BufferUsage, Subbuffer};
use vulkano::memory::allocator::{AllocationCreateInfo, MemoryTypeFilter, StandardMemoryAllocator};

/// Stores vertex and index buffers for a mesh on GPU
pub struct GpuMesh {
    pub vertex_buffer: Subbuffer<[Vertex3D]>,
    pub index_buffer: Subbuffer<[u32]>,
    pub index_count: u32,
    /// Bounding sphere center in local/model space (for frustum culling)
    pub center: Vec3,
    /// Bounding sphere radius (for frustum culling)
    pub radius: f32,
    /// Local-space AABB min (computed once at load time).
    pub aabb_min: Vec3,
    /// Local-space AABB max (computed once at load time).
    pub aabb_max: Vec3,
}

impl GpuMesh {
    /// Creates GPU buffers from vertex and index data
    pub fn new(
        memory_allocator: Arc<StandardMemoryAllocator>,
        vertices: &[Vertex3D],
        indices: &[u32],
        center: Vec3,
        radius: f32,
        aabb_min: Vec3,
        aabb_max: Vec3,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        // Create vertex buffer
        let vertex_buffer = Buffer::from_iter(
            memory_allocator.clone(),
            BufferCreateInfo {
                usage: BufferUsage::VERTEX_BUFFER,
                ..Default::default()
            },
            AllocationCreateInfo {
                memory_type_filter: MemoryTypeFilter::PREFER_DEVICE
                    | MemoryTypeFilter::HOST_SEQUENTIAL_WRITE,
                ..Default::default()
            },
            vertices.iter().copied(),
        )?;

        // Create index buffer
        let index_buffer = Buffer::from_iter(
            memory_allocator,
            BufferCreateInfo {
                usage: BufferUsage::INDEX_BUFFER,
                ..Default::default()
            },
            AllocationCreateInfo {
                memory_type_filter: MemoryTypeFilter::PREFER_DEVICE
                    | MemoryTypeFilter::HOST_SEQUENTIAL_WRITE,
                ..Default::default()
            },
            indices.iter().copied(),
        )?;

        Ok(Self {
            vertex_buffer,
            index_buffer,
            index_count: indices.len() as u32,
            center,
            radius,
            aabb_min,
            aabb_max,
        })
    }
}

/// Manages multiple GPU meshes
pub struct MeshManager {
    pub meshes: Vec<GpuMesh>,
}

impl MeshManager {
    pub fn new() -> Self {
        Self { meshes: Vec::new() }
    }

    /// Uploads a model to GPU
    pub fn upload_model(
        &mut self,
        model: &crate::engine::assets::model_loader::Model,
        memory_allocator: Arc<StandardMemoryAllocator>,
    ) -> Result<Vec<usize>, Box<dyn std::error::Error>> {
        let mut mesh_indices = Vec::new();

        for loaded_mesh in &model.meshes {
            let gpu_mesh = GpuMesh::new(
                memory_allocator.clone(),
                &loaded_mesh.vertices,
                &loaded_mesh.indices,
                loaded_mesh.center,
                loaded_mesh.radius,
                loaded_mesh.aabb_min,
                loaded_mesh.aabb_max,
            )?;

            let index = self.meshes.len();
            self.meshes.push(gpu_mesh);
            mesh_indices.push(index);
        }

        println!("✅ Uploaded {} meshes to GPU", mesh_indices.len());

        Ok(mesh_indices)
    }

    /// Gets a mesh by index
    pub fn get(&self, index: usize) -> Option<&GpuMesh> {
        self.meshes.get(index)
    }

    pub fn mesh_count(&self) -> usize {
        self.meshes.len()
    }
}

impl Default for MeshManager {
    fn default() -> Self {
        Self::new()
    }
}
