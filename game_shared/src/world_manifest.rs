//! World manifest (M3): cell/zone orchestration metadata for a partitioned
//! world, separate from the scene so M4's streaming loader can read it
//! without loading (or parsing) scene entities.
//!
//! Cell bounds are derived from `coord × cell_size` — never stored. Mesh and
//! chunk paths are explicit dependencies (content-relative), not derived from
//! naming conventions.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldManifest {
    pub version: u32,
    /// Content-relative scene path (e.g. `scenes/greybox.scene`).
    pub scene: String,
    /// Cell edge length in meters (matches `world_grid::CHUNK_SIZE`).
    pub cell_size: f32,
    /// Provisional character-controller limits the traversal gym is built
    /// around. M6 reads these instead of re-inventing them.
    pub controller_ref: ControllerRef,
    pub cells: Vec<CellEntry>,
    pub zones: Vec<ZoneEntry>,
    pub spawn_points: Vec<SpawnPoint>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ControllerRef {
    pub slope_limit_deg: f32,
    pub step_height: f32,
    pub jump_gap: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CellEntry {
    /// Cell coordinate on the world grid (floor semantics, may be negative).
    pub coord: [i32; 2],
    pub zone_id: u32,
    /// GUID of the cell's root entity in the scene.
    pub root_entity_guid: String,
    /// Content-relative terrain mesh path.
    pub mesh: String,
    /// Content-relative cooked collision chunk path.
    pub collision_chunk: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZoneEntry {
    pub id: u32,
    pub cells: Vec<[i32; 2]>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpawnPoint {
    pub name: String,
    /// World Z-up position.
    pub position: [f32; 3],
}
