//! Cell lifecycle execution: streamer-owned IO worker (mesh parse + chunk
//! read off-thread), generation-token stale-result rejection, budgeted
//! main-thread effects (GPU upload, chunk insert, spawn/despawn).
//!
//! Cells and collision chunks stream independently: the desired *chunk* set
//! comes from the cooked collision manifest (picks up border slivers the
//! world manifest doesn't list); the desired *cell* set from the world
//! manifest. Same rings and hysteresis for both.
//!
//! Failure policy mirrors M2: a failed cell/chunk logs a warning and stays
//! non-resident; it retries only after leaving and re-entering the load
//! ring. Never panics, never blocks the frame.

use crate::engine::assets::model_loader::{load_model_from_bytes, Model};
use crate::engine::assets::asset_source;
use crate::engine::collision::world::CollisionWorld;
use crate::engine::ecs::components::{EntityGuid, MeshRenderer, Name, Transform};
use crate::engine::rendering::rendering_3d::mesh_manager::MeshManager;
use game_shared::collision::format::fnv1a_64;
use game_shared::world_grid;
use glam::{IVec2, Vec3};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::Instant;
use vulkano::memory::allocator::StandardMemoryAllocator;

use super::streamer::{diff_ops, WorldStreamer, ZoneChanged};

/// Marker on streamed cell root entities. Excluded from scene serialization
/// and play-mode snapshots; despawned wholesale on cell unload.
pub struct StreamedCell {
    pub coord: IVec2,
}

// ---------------------------------------------------------------- IO worker

enum IoRequest {
    /// Read + parse a cell's mesh file.
    CellMesh { id: u64, coord: IVec2, mesh_rel: String },
    /// Read a cooked collision chunk's bytes.
    Chunk { id: u64, coord: IVec2, chunk_rel: String },
}

enum IoPayload {
    CellMesh(Result<Model, String>),
    Chunk(Result<Vec<u8>, String>),
}

struct IoResult {
    id: u64,
    coord: IVec2,
    payload: IoPayload,
}

struct IoWorker {
    tx: crossbeam_channel::Sender<IoRequest>,
    rx: crossbeam_channel::Receiver<IoResult>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl IoWorker {
    fn spawn() -> Self {
        let (req_tx, req_rx) = crossbeam_channel::unbounded::<IoRequest>();
        let (res_tx, res_rx) = crossbeam_channel::unbounded::<IoResult>();
        let handle = std::thread::Builder::new()
            .name("world-streamer-io".into())
            .spawn(move || {
                while let Ok(req) = req_rx.recv() {
                    let result = match req {
                        IoRequest::CellMesh { id, coord, mesh_rel } => {
                            let model = asset_source::read_bytes(&mesh_rel)
                                .map_err(|e| e.to_string())
                                .and_then(|bytes| {
                                    load_model_from_bytes(&bytes, &mesh_rel)
                                        .map_err(|e| e.to_string())
                                });
                            IoResult {
                                id,
                                coord,
                                payload: IoPayload::CellMesh(model),
                            }
                        }
                        IoRequest::Chunk { id, coord, chunk_rel } => {
                            let bytes =
                                asset_source::read_bytes(&chunk_rel).map_err(|e| e.to_string());
                            IoResult {
                                id,
                                coord,
                                payload: IoPayload::Chunk(bytes),
                            }
                        }
                    };
                    if res_tx.send(result).is_err() {
                        break; // streamer dropped
                    }
                }
            })
            .expect("spawn world-streamer-io thread");
        Self {
            tx: req_tx,
            rx: res_rx,
            handle: Some(handle),
        }
    }
}

impl Drop for IoWorker {
    fn drop(&mut self) {
        // Closing the request channel ends the worker loop.
        let (dead_tx, _) = crossbeam_channel::bounded(0);
        self.tx = dead_tx;
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

// ---------------------------------------------------------------- runtime

/// Mutable main-thread resources the streamer applies effects to.
pub struct StreamingCtx<'a> {
    pub world: &'a mut hecs::World,
    pub meshes: &'a mut MeshManager,
    pub allocator: Arc<StandardMemoryAllocator>,
    pub collision: &'a mut CollisionWorld,
}

/// Per-frame streaming outcome for the caller (zone event to forward,
/// overlay stats read separately).
#[derive(Default)]
pub struct StreamingFrameOutput {
    pub zone_changed: Option<ZoneChanged>,
}

const WARNING_CAP: usize = 50;
const FRAME_HISTORY: usize = 300;

#[derive(Default)]
pub struct Runtime {
    worker: Option<IoWorker>,
    next_request_id: u64,

    // Cells
    resident_cells: HashMap<IVec2, hecs::Entity>,
    guid_map: HashMap<uuid::Uuid, hecs::Entity>,
    cells_in_flight: HashMap<IVec2, u64>,
    failed_cells: HashSet<IVec2>,

    // Collision chunks
    resident_chunks: HashSet<IVec2>,
    chunks_in_flight: HashMap<IVec2, u64>,
    failed_chunks: HashSet<IVec2>,

    /// Verified results waiting for budget.
    ready: VecDeque<IoResult>,

    // Diagnostics (debug overlay)
    warnings: VecDeque<String>,
    frame_ms: VecDeque<f32>,
}

impl Runtime {
    pub(super) fn is_empty(&self) -> bool {
        self.resident_cells.is_empty() && self.resident_chunks.is_empty()
    }

    fn warn(&mut self, msg: String) {
        println!("⚠ streamer: {msg}");
        if self.warnings.len() >= WARNING_CAP {
            self.warnings.pop_front();
        }
        self.warnings.push_back(msg);
    }

    fn record_frame(&mut self, ms: f32) {
        if self.frame_ms.len() >= FRAME_HISTORY {
            self.frame_ms.pop_front();
        }
        self.frame_ms.push_back(ms);
    }

    fn worker(&mut self) -> &IoWorker {
        self.worker.get_or_insert_with(IoWorker::spawn)
    }
}

impl WorldStreamer {
    // ------------------------------------------------------------ overlay

    pub fn resident_cell_count(&self) -> usize {
        self.runtime.resident_cells.len()
    }

    pub fn resident_chunk_count(&self) -> usize {
        self.runtime.resident_chunks.len()
    }

    pub fn in_flight_count(&self) -> usize {
        self.runtime.cells_in_flight.len() + self.runtime.chunks_in_flight.len()
    }

    pub fn ready_queue_depth(&self) -> usize {
        self.runtime.ready.len()
    }

    /// Worst main-thread streaming time over the last 300 frames, in ms.
    pub fn worst_frame_ms(&self) -> f32 {
        self.runtime.frame_ms.iter().copied().fold(0.0, f32::max)
    }

    pub fn recent_warnings(&self) -> impl Iterator<Item = &str> {
        self.runtime.warnings.iter().map(|s| s.as_str())
    }

    /// Streamed-cell entity for a manifest root GUID (M4 scope: streamed
    /// cells only; the engine-wide registry is M5).
    pub fn entity_for_guid(&self, guid: uuid::Uuid) -> Option<hecs::Entity> {
        self.runtime.guid_map.get(&guid).copied()
    }

    // ------------------------------------------------------------ update

    /// Per-frame streaming around a Z-up world-space center. Dispatches IO,
    /// applies budgeted main-thread effects, returns the zone event (if any)
    /// for the caller to forward to the ECS event queue.
    pub fn update_streaming(
        &mut self,
        center: Vec3,
        ctx: &mut StreamingCtx<'_>,
    ) -> StreamingFrameOutput {
        crate::profile_function!();
        if self.world.is_none() {
            return StreamingFrameOutput::default();
        }
        let start = Instant::now();
        let center_cell = world_grid::chunk_coord(center);

        let zone_changed = self.update_zone(center_cell);

        let (cell_ops, chunk_ops) = {
            let world = self.world.as_ref().expect("checked above");
            let rt = &self.runtime;
            let resident_cells: HashSet<IVec2> = rt.resident_cells.keys().copied().collect();
            let resident_chunks = rt.resident_chunks.clone();
            (
                diff_ops(center_cell, &resident_cells, &world.cell_coords(), &self.config),
                diff_ops(center_cell, &resident_chunks, &world.chunk_coords(), &self.config),
            )
        };

        self.cancel_undesired(&cell_ops.loads, &chunk_ops.loads);
        self.retry_outside_ring(&cell_ops.loads, &chunk_ops.loads);
        self.dispatch(&cell_ops.loads, &chunk_ops.loads);
        self.collect_results();
        self.apply_effects(start, &cell_ops.unloads, &chunk_ops.unloads, ctx);

        self.runtime
            .record_frame(start.elapsed().as_secs_f32() * 1000.0);
        StreamingFrameOutput { zone_changed }
    }

    /// Drops in-flight requests whose coord is no longer in the wanted load
    /// list (their results get rejected by the token check on arrival).
    fn cancel_undesired(&mut self, cell_loads: &[IVec2], chunk_loads: &[IVec2]) {
        let wanted_cells: HashSet<IVec2> = cell_loads.iter().copied().collect();
        let wanted_chunks: HashSet<IVec2> = chunk_loads.iter().copied().collect();
        self.runtime
            .cells_in_flight
            .retain(|coord, _| wanted_cells.contains(coord));
        self.runtime
            .chunks_in_flight
            .retain(|coord, _| wanted_chunks.contains(coord));
    }

    /// A failed coord retries only after leaving the load ring: clear the
    /// failed mark once the coord is no longer wanted.
    fn retry_outside_ring(&mut self, cell_loads: &[IVec2], chunk_loads: &[IVec2]) {
        let wanted_cells: HashSet<IVec2> = cell_loads.iter().copied().collect();
        let wanted_chunks: HashSet<IVec2> = chunk_loads.iter().copied().collect();
        self.runtime.failed_cells.retain(|c| wanted_cells.contains(c));
        self.runtime
            .failed_chunks
            .retain(|c| wanted_chunks.contains(c));
    }

    /// Sends up to `max_in_flight` requests, nearest still-desired first.
    /// Cells before chunks (visuals first; chunk reads are tiny).
    fn dispatch(&mut self, cell_loads: &[IVec2], chunk_loads: &[IVec2]) {
        let world = self.world.as_ref().expect("active");
        let max = self.config.max_in_flight;

        let mut requests: Vec<IoRequest> = Vec::new();
        {
            let rt = &mut self.runtime;
            let mut in_flight = rt.cells_in_flight.len() + rt.chunks_in_flight.len();

            for &coord in cell_loads {
                if in_flight >= max {
                    break;
                }
                if rt.cells_in_flight.contains_key(&coord) || rt.failed_cells.contains(&coord) {
                    continue;
                }
                let Some(cell) = world.cell(coord) else { continue };
                let id = rt.next_request_id;
                rt.next_request_id += 1;
                rt.cells_in_flight.insert(coord, id);
                requests.push(IoRequest::CellMesh {
                    id,
                    coord,
                    mesh_rel: cell.mesh.clone(),
                });
                in_flight += 1;
            }

            for &coord in chunk_loads {
                if in_flight >= max {
                    break;
                }
                if rt.chunks_in_flight.contains_key(&coord) || rt.failed_chunks.contains(&coord) {
                    continue;
                }
                let id = rt.next_request_id;
                rt.next_request_id += 1;
                rt.chunks_in_flight.insert(coord, id);
                requests.push(IoRequest::Chunk {
                    id,
                    coord,
                    chunk_rel: world.chunk_path(coord),
                });
                in_flight += 1;
            }
        }

        for req in requests {
            let _ = self.runtime.worker().tx.send(req);
        }
    }

    /// Drains worker results, dropping any whose token no longer matches
    /// (cancelled or superseded requests).
    fn collect_results(&mut self) {
        let rt = &mut self.runtime;
        let Some(worker) = rt.worker.as_ref() else { return };
        let drained: Vec<IoResult> = worker.rx.try_iter().collect();
        for result in drained {
            let current = match result.payload {
                IoPayload::CellMesh(_) => rt.cells_in_flight.get(&result.coord),
                IoPayload::Chunk(_) => rt.chunks_in_flight.get(&result.coord),
            };
            if current == Some(&result.id) {
                rt.ready.push_back(result);
            } // else: stale — cell was cancelled while the read ran
        }
    }

    /// Budgeted main-thread stage: ready loads first (nearest were dispatched
    /// first, so arrival order approximates priority), then unloads. At least
    /// one op per frame guarantees progress under budget pressure.
    fn apply_effects(
        &mut self,
        start: Instant,
        cell_unloads: &[IVec2],
        chunk_unloads: &[IVec2],
        ctx: &mut StreamingCtx<'_>,
    ) {
        let budget = std::time::Duration::from_secs_f32(self.config.budget_ms / 1000.0);
        let mut unload_queue: VecDeque<(bool, IVec2)> = cell_unloads
            .iter()
            .map(|&c| (true, c))
            .chain(chunk_unloads.iter().map(|&c| (false, c)))
            .collect();

        let mut ops_done = 0usize;
        loop {
            if ops_done > 0 && start.elapsed() >= budget {
                break;
            }
            if let Some(result) = self.runtime.ready.pop_front() {
                self.apply_load(result, ctx);
            } else if let Some((is_cell, coord)) = unload_queue.pop_front() {
                if is_cell {
                    self.unload_cell(coord, ctx);
                } else {
                    self.unload_chunk(coord, ctx);
                }
            } else {
                break;
            }
            ops_done += 1;
        }
    }

    fn apply_load(&mut self, result: IoResult, ctx: &mut StreamingCtx<'_>) {
        let coord = result.coord;
        match result.payload {
            IoPayload::CellMesh(model) => {
                self.runtime.cells_in_flight.remove(&coord);
                match model {
                    Ok(model) => self.spawn_cell(coord, &model, ctx),
                    Err(e) => {
                        self.runtime.failed_cells.insert(coord);
                        self.runtime.warn(format!("cell {coord} mesh load failed: {e}"));
                    }
                }
            }
            IoPayload::Chunk(bytes) => {
                self.runtime.chunks_in_flight.remove(&coord);
                match bytes {
                    Ok(bytes) => self.insert_chunk(coord, &bytes, ctx),
                    Err(e) => {
                        self.runtime.failed_chunks.insert(coord);
                        self.runtime.warn(format!("chunk {coord} read failed: {e}"));
                    }
                }
            }
        }
    }

    fn spawn_cell(&mut self, coord: IVec2, model: &Model, ctx: &mut StreamingCtx<'_>) {
        let world = self.world.as_ref().expect("active");
        let Some(cell) = world.cell(coord) else { return };
        let mesh_path = cell.mesh.clone();
        let guid = EntityGuid::from_string(&cell.root_entity_guid);
        let guid_missing = guid.is_none();

        if let Err(e) =
            ctx.meshes
                .acquire_streamed(&mesh_path, model, ctx.allocator.clone())
        {
            self.runtime.failed_cells.insert(coord);
            self.runtime
                .warn(format!("cell {coord} GPU upload failed: {e}"));
            return;
        }

        // Identity transform: greybox cell meshes bake world placement into
        // their vertices; the manifest coord is metadata, not an offset.
        let guid = guid.unwrap_or_default();
        let guid_uuid = guid.0;
        let entity = ctx.world.spawn((
            Name::new(format!("cell_{}_{}", coord.x, coord.y)),
            guid,
            Transform::default(),
            MeshRenderer {
                mesh_path,
                ..Default::default()
            },
            StreamedCell { coord },
        ));

        if guid_missing {
            self.runtime.warn(format!(
                "cell {coord}: invalid root_entity_guid in manifest — generated a fresh GUID"
            ));
        }
        self.runtime.resident_cells.insert(coord, entity);
        self.runtime.guid_map.insert(guid_uuid, entity);
    }

    fn insert_chunk(&mut self, coord: IVec2, bytes: &[u8], ctx: &mut StreamingCtx<'_>) {
        let world = self.world.as_ref().expect("active");
        if let Some(&expected) = world.cooked_chunks.get(&coord) {
            if fnv1a_64(bytes) != expected {
                self.runtime.failed_chunks.insert(coord);
                self.runtime
                    .warn(format!("chunk {coord}: content hash mismatch (stale cook?)"));
                return;
            }
        }
        match ctx.collision.insert_chunk_bytes(bytes) {
            Ok(_) => {
                self.runtime.resident_chunks.insert(coord);
            }
            Err(e) => {
                self.runtime.failed_chunks.insert(coord);
                self.runtime.warn(format!("chunk {coord}: {e}"));
            }
        }
    }

    fn unload_cell(&mut self, coord: IVec2, ctx: &mut StreamingCtx<'_>) {
        let Some(entity) = self.runtime.resident_cells.remove(&coord) else {
            return;
        };
        if let Ok(guid) = ctx.world.get::<&EntityGuid>(entity).map(|g| g.0) {
            self.runtime.guid_map.remove(&guid);
        }
        let mesh_path = ctx
            .world
            .get::<&MeshRenderer>(entity)
            .map(|mr| mr.mesh_path.clone())
            .ok();
        let _ = ctx.world.despawn(entity);
        if let Some(path) = mesh_path {
            // GPU safety: frames in flight hold their own Arc<Subbuffer>
            // clones inside FramePacket; vulkano frees after the last user.
            ctx.meshes.release_streamed(&path);
        }
    }

    fn unload_chunk(&mut self, coord: IVec2, ctx: &mut StreamingCtx<'_>) {
        if self.runtime.resident_chunks.remove(&coord) {
            ctx.collision.remove_chunk(coord);
        }
    }

    // ------------------------------------------------------------ flush

    /// Synchronously tears down all streamed content: despawns cells,
    /// releases meshes, removes chunks, cancels in-flight IO. Must run
    /// before scene swaps, tab parking, and asset hot-reload rebuilds.
    pub fn flush(&mut self, ctx: &mut StreamingCtx<'_>) {
        let cells: Vec<IVec2> = self.runtime.resident_cells.keys().copied().collect();
        for coord in cells {
            self.unload_cell(coord, ctx);
        }
        let chunks: Vec<IVec2> = self.runtime.resident_chunks.iter().copied().collect();
        for coord in chunks {
            self.unload_chunk(coord, ctx);
        }
        // Cancel pending IO: tokens won't match on arrival.
        self.runtime.cells_in_flight.clear();
        self.runtime.chunks_in_flight.clear();
        self.runtime.ready.clear();
        self.runtime.failed_cells.clear();
        self.runtime.failed_chunks.clear();
    }
}
