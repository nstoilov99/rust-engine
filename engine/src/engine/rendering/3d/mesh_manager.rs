use crate::engine::rendering::rendering_3d::pipeline_3d::Vertex3D;
use glam::Vec3;
use std::collections::HashMap;
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

/// Reference state of a path's meshes.
///
/// Pinned paths live for the manager's lifetime (resolver-loaded assets,
/// primitives). Streamed paths are refcounted by the world streamer and their
/// GPU slots are recycled when the count hits zero. "Pin wins": registering a
/// streamed path through the pinned API promotes it permanently.
enum RefState {
    Pinned,
    Streamed(u32),
}

struct PathEntry {
    indices: Vec<usize>,
    state: RefState,
}

/// Manages GPU meshes in reusable slots.
///
/// Generic over the mesh type so the slot/refcount bookkeeping is testable
/// without a Vulkan device; the engine always uses `MeshManager<GpuMesh>`.
pub struct MeshManager<M = GpuMesh> {
    meshes: Vec<Option<M>>,
    free_slots: Vec<usize>,
    /// Map from content-relative asset path to GPU mesh slots
    path_index: HashMap<String, PathEntry>,
}

impl<M> MeshManager<M> {
    pub fn new() -> Self {
        Self {
            meshes: Vec::new(),
            free_slots: Vec::new(),
            path_index: HashMap::new(),
        }
    }

    fn alloc_slot(&mut self, mesh: M) -> usize {
        match self.free_slots.pop() {
            Some(slot) => {
                self.meshes[slot] = Some(mesh);
                slot
            }
            None => {
                self.meshes.push(Some(mesh));
                self.meshes.len() - 1
            }
        }
    }

    /// Bumps the streamed refcount if `path` is resident, returning its slots.
    /// Pinned paths return their slots without refcounting (pin wins).
    pub fn touch_streamed(&mut self, path: &str) -> Option<Vec<usize>> {
        let entry = self.path_index.get_mut(path)?;
        if let RefState::Streamed(n) = &mut entry.state {
            *n += 1;
        }
        Some(entry.indices.clone())
    }

    /// Inserts already-built meshes for a streamed path (refcount 1).
    /// Caller must ensure the path is not already resident.
    pub fn insert_streamed(&mut self, path: &str, meshes: Vec<M>) -> Vec<usize> {
        debug_assert!(
            !self.path_index.contains_key(path),
            "insert_streamed on resident path: {path}"
        );
        let indices: Vec<usize> = meshes.into_iter().map(|m| self.alloc_slot(m)).collect();
        self.path_index.insert(
            path.to_string(),
            PathEntry {
                indices: indices.clone(),
                state: RefState::Streamed(1),
            },
        );
        indices
    }

    /// Releases a streamed reference. On the last release the path's GPU
    /// slots are freed for reuse. No-op on pinned paths (pin wins).
    pub fn release_streamed(&mut self, path: &str) {
        let Some(entry) = self.path_index.get_mut(path) else {
            debug_assert!(false, "release_streamed on unknown path: {path}");
            return;
        };
        match &mut entry.state {
            RefState::Pinned => {}
            RefState::Streamed(n) => {
                *n -= 1;
                if *n == 0 {
                    let entry = self.path_index.remove(path).expect("entry exists");
                    for slot in entry.indices {
                        self.meshes[slot] = None;
                        self.free_slots.push(slot);
                    }
                }
            }
        }
    }

    /// Inserts a single pre-built mesh (pinned, unnamed). Returns its slot.
    pub fn insert_mesh(&mut self, mesh: M) -> usize {
        self.alloc_slot(mesh)
    }

    /// Gets a mesh by slot index
    pub fn get(&self, index: usize) -> Option<&M> {
        self.meshes.get(index).and_then(|slot| slot.as_ref())
    }

    /// Gets the first GPU mesh slot for a content-relative path.
    pub fn first_index_for_path(&self, path: &str) -> Option<usize> {
        self.path_index
            .get(path)
            .and_then(|e| e.indices.first().copied())
    }

    /// Gets all GPU mesh slots for a content-relative path.
    pub fn indices_for_path(&self, path: &str) -> Option<&[usize]> {
        self.path_index.get(path).map(|e| e.indices.as_slice())
    }

    /// Manually register a content-relative path → mesh slot mapping (pinned).
    /// Promotes an already-streamed path (pin wins).
    pub fn register_path(&mut self, path: &str, indices: Vec<usize>) {
        self.path_index.insert(
            path.to_string(),
            PathEntry {
                indices,
                state: RefState::Pinned,
            },
        );
    }

    /// Removes a path's meshes regardless of ref state, freeing its slots.
    /// Hot-reload uses this to drop a stale upload for one path without
    /// disturbing other resident (possibly streamed) meshes.
    pub fn evict_path(&mut self, path: &str) {
        if let Some(entry) = self.path_index.remove(path) {
            for slot in entry.indices {
                self.meshes[slot] = None;
                self.free_slots.push(slot);
            }
        }
    }

    /// Number of live (occupied) mesh slots.
    pub fn mesh_count(&self) -> usize {
        self.meshes.len() - self.free_slots.len()
    }
}

impl MeshManager<GpuMesh> {
    fn upload_meshes(
        &mut self,
        model: &crate::engine::assets::model_loader::Model,
        memory_allocator: Arc<StandardMemoryAllocator>,
    ) -> Result<Vec<usize>, Box<dyn std::error::Error>> {
        let mut mesh_indices = Vec::with_capacity(model.meshes.len());
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
            mesh_indices.push(self.alloc_slot(gpu_mesh));
        }
        Ok(mesh_indices)
    }

    /// Uploads a model to GPU without a path mapping (pinned, unnamed).
    pub fn upload_model(
        &mut self,
        model: &crate::engine::assets::model_loader::Model,
        memory_allocator: Arc<StandardMemoryAllocator>,
    ) -> Result<Vec<usize>, Box<dyn std::error::Error>> {
        self.upload_meshes(model, memory_allocator)
    }

    /// Uploads a model to GPU with an optional content-relative path (pinned).
    pub fn upload_model_with_path(
        &mut self,
        model: &crate::engine::assets::model_loader::Model,
        memory_allocator: Arc<StandardMemoryAllocator>,
        asset_path: Option<&str>,
    ) -> Result<Vec<usize>, Box<dyn std::error::Error>> {
        // Pin wins: a path already resident gets promoted, no re-upload.
        if let Some(path) = asset_path {
            if let Some(entry) = self.path_index.get_mut(path) {
                entry.state = RefState::Pinned;
                return Ok(entry.indices.clone());
            }
        }

        let mesh_indices = self.upload_meshes(model, memory_allocator)?;

        if let Some(path) = asset_path {
            self.path_index.insert(
                path.to_string(),
                PathEntry {
                    indices: mesh_indices.clone(),
                    state: RefState::Pinned,
                },
            );
        }

        Ok(mesh_indices)
    }

    /// Acquires a streamed reference to `path`, uploading if not resident.
    /// Each acquire must be balanced by a `release_streamed`.
    pub fn acquire_streamed(
        &mut self,
        path: &str,
        model: &crate::engine::assets::model_loader::Model,
        memory_allocator: Arc<StandardMemoryAllocator>,
    ) -> Result<Vec<usize>, Box<dyn std::error::Error>> {
        if let Some(indices) = self.touch_streamed(path) {
            return Ok(indices);
        }
        let mesh_indices = self.upload_meshes(model, memory_allocator)?;
        self.path_index.insert(
            path.to_string(),
            PathEntry {
                indices: mesh_indices.clone(),
                state: RefState::Streamed(1),
            },
        );
        Ok(mesh_indices)
    }
}

impl<M> Default for MeshManager<M> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streamed_refcount_frees_slots_on_last_release() {
        let mut mm: MeshManager<u32> = MeshManager::new();
        let indices = mm.insert_streamed("cell_a", vec![10, 11]);
        assert_eq!(indices, vec![0, 1]);
        assert_eq!(mm.mesh_count(), 2);

        assert_eq!(mm.touch_streamed("cell_a"), Some(vec![0, 1]));
        mm.release_streamed("cell_a");
        assert_eq!(mm.mesh_count(), 2); // still one ref held

        mm.release_streamed("cell_a");
        assert_eq!(mm.mesh_count(), 0);
        assert!(mm.get(0).is_none());
        assert!(mm.indices_for_path("cell_a").is_none());
    }

    #[test]
    fn slots_are_reused_and_path_grouping_survives() {
        let mut mm: MeshManager<u32> = MeshManager::new();
        mm.insert_streamed("cell_a", vec![1, 2]);
        mm.insert_streamed("cell_b", vec![3]);
        mm.release_streamed("cell_a"); // frees slots 0, 1

        let indices = mm.insert_streamed("cell_c", vec![4, 5]);
        assert_eq!(indices.len(), 2);
        for &i in &indices {
            assert!(i < 2, "expected freed slot reuse, got {i}");
        }
        // Grouping intact per path after reuse
        assert_eq!(mm.indices_for_path("cell_b"), Some(&[2][..]));
        assert_eq!(mm.indices_for_path("cell_c").unwrap(), indices.as_slice());
        assert_eq!(*mm.get(indices[0]).unwrap(), 4);
        assert_eq!(*mm.get(indices[1]).unwrap(), 5);
    }

    #[test]
    fn pin_wins_over_streaming() {
        let mut mm: MeshManager<u32> = MeshManager::new();
        let indices = mm.insert_streamed("shared", vec![7]);
        // Resolver pins the same path (promotion)
        mm.register_path("shared", indices.clone());

        // Releases no longer free pinned meshes
        mm.release_streamed("shared");
        mm.release_streamed("shared");
        assert_eq!(mm.mesh_count(), 1);
        assert_eq!(mm.indices_for_path("shared"), Some(indices.as_slice()));

        // Touch on pinned returns slots without refcounting
        assert_eq!(mm.touch_streamed("shared"), Some(indices));
    }

    #[test]
    fn pinned_paths_never_freed() {
        let mut mm: MeshManager<u32> = MeshManager::new();
        let slot = mm.insert_mesh(9);
        mm.register_path("__primitive__/Cube", vec![slot]);
        mm.release_streamed("__primitive__/Cube"); // no-op
        assert_eq!(mm.mesh_count(), 1);
        assert_eq!(*mm.get(slot).unwrap(), 9);
    }
}
