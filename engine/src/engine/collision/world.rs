//! `CollisionWorld` — engine resource owning the loaded static-collision
//! chunks for the current scene.
//!
//! Populated at scene load from `content/collision/<scene_stem>/` (filesystem
//! or pak via `asset_source`), cleared on scene switch. Play mode reads the
//! same store — static geometry has no play/edit divergence.
//!
//! Failure policy (per the M2 plan): manifest missing or version-mismatched
//! disables collision for the scene with a visible warning; an individual
//! corrupt or stale chunk is skipped with a logged error so editor iteration
//! stays unblocked — the cooker is the fix, not the loader.

use game_shared::collision::format::{fnv1a_64, CollisionManifest, FORMAT_VERSION};
use game_shared::collision::ChunkStore;
use game_shared::world_grid::{self, CHUNK_SIZE};
use glam::Vec3;

use super::output::scene_stem;
use crate::engine::assets::asset_source;
use crate::engine::debug_draw::DebugDrawBuffer;

const CHUNK_WIREFRAME_COLOR: [f32; 4] = [0.2, 0.9, 0.4, 1.0];
const GRID_COLOR: [f32; 4] = [0.9, 0.6, 0.1, 1.0];

#[derive(Default)]
pub struct CollisionWorld {
    store: ChunkStore,
    /// Scene stem the store was loaded for (`None` = empty/disabled).
    scene: Option<String>,
    /// Editor toggle: wireframe of loaded chunk geometry.
    pub debug_draw_chunks: bool,
    /// Editor toggle: chunk-grid overlay.
    pub debug_draw_grid: bool,
    /// Bumped whenever the chunk contents change, so debug-draw caches can
    /// tell when their line data went stale.
    generation: u64,
}

/// Outcome of `load_for_scene`, for surfacing in the editor console.
pub struct CollisionLoadReport {
    pub loaded: usize,
    /// Chunks skipped (corrupt/stale/missing); details in `warnings`.
    pub skipped: usize,
    /// `Some` = collision is disabled for this scene, with the reason.
    pub disabled: Option<String>,
    pub warnings: Vec<String>,
    /// Wall-clock load time — the M4 streaming baseline number.
    pub elapsed_ms: f32,
}

impl CollisionWorld {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn store(&self) -> &ChunkStore {
        &self.store
    }

    pub fn scene(&self) -> Option<&str> {
        self.scene.as_deref()
    }

    pub fn is_loaded(&self) -> bool {
        self.scene.is_some() && !self.store.is_empty()
    }

    pub fn clear(&mut self) {
        self.store.clear();
        self.scene = None;
        self.generation += 1;
    }

    /// Chunk-content version; changes on every `clear`/`load_for_scene`.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Streaming (M4): insert one cooked chunk's bytes. Bumps generation so
    /// the debug-wireframe GPU cache invalidates.
    pub fn insert_chunk_bytes(
        &mut self,
        bytes: &[u8],
    ) -> Result<glam::IVec2, game_shared::collision::store::ChunkLoadError> {
        let coord = self.store.insert_chunk(bytes)?;
        self.generation += 1;
        Ok(coord)
    }

    /// Streaming (M4): remove a chunk. Bumps generation when present.
    pub fn remove_chunk(&mut self, coord: glam::IVec2) -> bool {
        let removed = self.store.remove_chunk(coord);
        if removed {
            self.generation += 1;
        }
        removed
    }

    /// Streaming (M4): mark the store as belonging to a scene without bulk
    /// loading (chunks arrive via `insert_chunk_bytes`).
    pub fn begin_streaming(&mut self, scene_relative: &str) {
        self.clear();
        self.scene = Some(scene_stem(scene_relative));
    }

    /// Load all cooked chunks for a scene (`scene_relative` is the
    /// content-relative `.scene` path). Replaces any previous contents.
    pub fn load_for_scene(&mut self, scene_relative: &str) -> CollisionLoadReport {
        let start = std::time::Instant::now();
        self.clear();
        let stem = scene_stem(scene_relative);
        let manifest_rel = format!("collision/{stem}/manifest.ron");

        let disabled = |reason: String| CollisionLoadReport {
            loaded: 0,
            skipped: 0,
            disabled: Some(reason),
            warnings: Vec::new(),
            elapsed_ms: 0.0,
        };

        if !asset_source::exists(&manifest_rel) {
            return disabled(format!(
                "no cooked collision for '{stem}' (missing {manifest_rel}) — collision disabled"
            ));
        }
        let manifest: CollisionManifest = match asset_source::read_string(&manifest_rel)
            .map_err(|e| e.to_string())
            .and_then(|s| ron::from_str(&s).map_err(|e| e.to_string()))
        {
            Ok(m) => m,
            Err(e) => {
                return disabled(format!(
                    "unreadable {manifest_rel}: {e} — collision disabled"
                ))
            }
        };
        if manifest.format_version != FORMAT_VERSION {
            return disabled(format!(
                "cooked collision format v{} != supported v{FORMAT_VERSION} — re-cook '{stem}'; collision disabled",
                manifest.format_version
            ));
        }
        if manifest.chunk_size != CHUNK_SIZE {
            return disabled(format!(
                "cooked chunk size {} != runtime {CHUNK_SIZE} — re-cook '{stem}'; collision disabled",
                manifest.chunk_size
            ));
        }

        let mut warnings = Vec::new();
        let mut loaded = 0usize;
        for entry in &manifest.chunks {
            let [x, y] = entry.coord;
            let rel = format!("collision/{stem}/{x}_{y}.ccol");
            let bytes = match asset_source::read_bytes(&rel) {
                Ok(b) => b,
                Err(e) => {
                    warnings.push(format!("{rel}: read failed ({e}) — chunk skipped"));
                    continue;
                }
            };
            if fnv1a_64(&bytes) != entry.content_hash {
                warnings.push(format!(
                    "{rel}: content hash mismatch (stale cook?) — chunk skipped"
                ));
                continue;
            }
            match self.store.insert_chunk(&bytes) {
                Ok(_) => loaded += 1,
                Err(e) => warnings.push(format!("{rel}: {e} — chunk skipped")),
            }
        }

        self.scene = Some(stem);
        CollisionLoadReport {
            loaded,
            skipped: manifest.chunks.len() - loaded,
            disabled: None,
            warnings,
            elapsed_ms: start.elapsed().as_secs_f32() * 1000.0,
        }
    }

    /// Submit debug wireframes according to the enabled toggles.
    pub fn submit_debug_draws(&self, buffer: &mut DebugDrawBuffer) {
        if self.debug_draw_chunks {
            for (coord, chunk) in self.store.chunks() {
                for i in 0..chunk.num_triangles() as u32 {
                    let tri = chunk
                        .triangle_local(i)
                        .map(|v| world_grid::local_to_world(coord, v));
                    for (a, b) in [(0, 1), (1, 2), (2, 0)] {
                        buffer.line(tri[a].into(), tri[b].into(), CHUNK_WIREFRAME_COLOR);
                    }
                }
            }
        }
        if self.debug_draw_grid {
            for (coord, chunk) in self.store.chunks() {
                let (min, max) = chunk.local_aabb();
                let origin = world_grid::chunk_origin(coord);
                // Full chunk footprint on XY, geometry extent on Z.
                let lo = Vec3::new(origin.x, origin.y, min.z - 0.1);
                let hi = Vec3::new(origin.x + CHUNK_SIZE, origin.y + CHUNK_SIZE, max.z + 0.1);
                let center = (lo + hi) * 0.5;
                let half = (hi - lo) * 0.5;
                buffer.box_wireframe(center.into(), half.into(), GRID_COLOR);
            }
        }
    }
}
