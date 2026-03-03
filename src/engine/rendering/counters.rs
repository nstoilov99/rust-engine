use crate::engine::assets::AssetManager;
use crate::engine::physics::PhysicsWorld;
use hecs::World;
use std::sync::Arc;

/// Per-frame rendering statistics collected during the geometry pass.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct RenderCounters {
    /// Number of draw calls submitted this frame.
    pub draw_calls: u32,
    /// Total triangles submitted this frame.
    pub triangles: u32,
    /// Number of material transitions during geometry submission.
    pub material_changes: u32,
    /// Number of entities that passed visibility checks and were rendered.
    pub visible_entities: u32,
}

impl RenderCounters {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Scene-level resource counts (snapshot, not per-frame).
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ResourceCounters {
    pub entity_count: u32,
    pub mesh_count: u32,
    pub texture_count: u32,
    pub rigid_body_count: u32,
}

impl ResourceCounters {
    pub fn collect(
        world: &World,
        asset_manager: &Arc<AssetManager>,
        physics_world: &PhysicsWorld,
    ) -> Self {
        let mesh_count = asset_manager.meshes.read().mesh_count();
        Self {
            entity_count: world.len(),
            mesh_count: mesh_count.min(u32::MAX as usize) as u32,
            texture_count: asset_manager.textures.cache_size().min(u32::MAX as usize) as u32,
            rigid_body_count: physics_world.rigid_body_count(),
        }
    }
}
