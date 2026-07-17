//! Synthetic "world object" shown in the hierarchy when streaming is
//! active (UE Landscape-style): one row representing all streamed cells,
//! with read-only streaming info in the inspector.

use crate::engine::world::WorldStreamer;

/// Read-only snapshot of the streamed world, rebuilt each frame.
pub struct WorldObjectInfo {
    /// Display name derived from the scene stem (e.g. "Greybox World").
    pub name: String,
    pub cell_size: f32,
    pub cells_total: usize,
    pub cells_resident: usize,
    pub chunks_total: usize,
    pub chunks_resident: usize,
    pub zone_count: usize,
    pub current_zone: Option<u32>,
    /// true = whole world resident; false = streaming around the camera.
    pub full_world: bool,
    pub in_flight: usize,
    pub ready: usize,
    pub worst_ms: f32,
}

impl WorldObjectInfo {
    pub fn from_streamer(streamer: &WorldStreamer) -> Option<Self> {
        let world = streamer.world()?;
        let mut chars = world.stem.chars();
        let name = match chars.next() {
            Some(f) => format!("{}{} World", f.to_uppercase(), chars.as_str()),
            None => "World".to_string(),
        };
        Some(Self {
            name,
            cell_size: world.manifest.cell_size,
            cells_total: world.manifest.cells.len(),
            cells_resident: streamer.resident_cell_count(),
            chunks_total: world.cooked_chunks.len(),
            chunks_resident: streamer.resident_chunk_count(),
            zone_count: world.manifest.zones.len(),
            current_zone: streamer.current_zone(),
            full_world: streamer.full_world,
            in_flight: streamer.in_flight_count(),
            ready: streamer.ready_queue_depth(),
            worst_ms: streamer.worst_frame_ms(),
        })
    }
}
