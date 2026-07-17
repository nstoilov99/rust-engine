//! WorldStreamer core: manifest loading/validation and the pure desired-set
//! logic (Chebyshev rings with hysteresis, ordered load/unload ops).
//!
//! Scenes without a world manifest (`world/<stem>.world.ron`) keep the
//! monolithic load path — the streamer stays inert. Failure policy mirrors
//! M2 collision loading: warn + degrade, never crash.

use crate::engine::assets::asset_source;
use crate::engine::collision::output::scene_stem;
use crate::engine::ecs::events::Event;
use game_shared::collision::format::CollisionManifest;
use game_shared::world_manifest::WorldManifest;
use glam::IVec2;
use std::collections::{HashMap, HashSet};

/// Supported world manifest version.
pub const WORLD_MANIFEST_VERSION: u32 = 1;

/// Emitted when the streaming center crosses into a different zone.
/// `None` = outside all manifest cells. M4 only logs it; M5 subscription
/// scoping consumes it.
pub struct ZoneChanged {
    pub previous: Option<u32>,
    pub current: Option<u32>,
}

impl Event for ZoneChanged {}

/// Streaming radii and budgets. `r_load < r_unload` gives hysteresis:
/// cells load at Chebyshev distance ≤ `r_load` and unload only beyond
/// `r_unload`, so a center oscillating on a boundary never thrashes.
#[derive(Debug, Clone, Copy)]
pub struct StreamingConfig {
    pub r_load: i32,
    pub r_unload: i32,
    /// Main-thread streaming budget per frame, in milliseconds.
    pub budget_ms: f32,
    /// Max IO requests in flight on the streamer worker.
    pub max_in_flight: usize,
}

impl Default for StreamingConfig {
    fn default() -> Self {
        Self {
            r_load: 2,
            r_unload: 3,
            budget_ms: 1.0,
            max_in_flight: 2,
        }
    }
}

/// Validated world manifest plus derived lookup maps.
pub struct LoadedWorld {
    /// Scene stem (`"greybox"`), used to derive cooked chunk paths.
    pub stem: String,
    pub manifest: WorldManifest,
    /// coord → index into `manifest.cells`
    pub cell_by_coord: HashMap<IVec2, usize>,
    /// coord → `.ccol` content hash for all cooked collision chunks
    /// (superset of cell coords — includes border slivers not listed in
    /// the world manifest).
    pub cooked_chunks: HashMap<IVec2, u64>,
}

impl LoadedWorld {
    pub fn zone_of_cell(&self, coord: IVec2) -> Option<u32> {
        self.cell_by_coord
            .get(&coord)
            .map(|&i| self.manifest.cells[i].zone_id)
    }

    pub fn cell(&self, coord: IVec2) -> Option<&game_shared::world_manifest::CellEntry> {
        self.cell_by_coord
            .get(&coord)
            .map(|&i| &self.manifest.cells[i])
    }

    pub fn cell_coords(&self) -> HashSet<IVec2> {
        self.cell_by_coord.keys().copied().collect()
    }

    pub fn chunk_coords(&self) -> HashSet<IVec2> {
        self.cooked_chunks.keys().copied().collect()
    }

    /// Cooked chunk path for a coord (`collision/<stem>/<x>_<y>.ccol`).
    pub fn chunk_path(&self, coord: IVec2) -> String {
        format!("collision/{}/{}_{}.ccol", self.stem, coord.x, coord.y)
    }
}

pub struct WorldLoadReport {
    pub warnings: Vec<String>,
    /// Present when the manifest was missing or invalid (streamer inert).
    pub disabled: Option<String>,
}

/// Loads and validates the world manifest for a scene. Returns `None` in the
/// report-disabled case; the caller logs and keeps the monolithic path.
pub fn load_world(scene_relative: &str) -> (Option<LoadedWorld>, WorldLoadReport) {
    let stem = scene_stem(scene_relative);
    let manifest_rel = format!("world/{stem}.world.ron");

    let disabled = |reason: String| {
        (
            None,
            WorldLoadReport {
                warnings: Vec::new(),
                disabled: Some(reason),
            },
        )
    };

    if !asset_source::exists(&manifest_rel) {
        return disabled(format!("no world manifest ({manifest_rel})"));
    }

    let manifest: WorldManifest = match asset_source::read_string(&manifest_rel)
        .map_err(|e| e.to_string())
        .and_then(|s| ron::from_str(&s).map_err(|e| e.to_string()))
    {
        Ok(m) => m,
        Err(e) => return disabled(format!("unreadable {manifest_rel}: {e}")),
    };

    if manifest.version != WORLD_MANIFEST_VERSION {
        return disabled(format!(
            "world manifest v{} != supported v{WORLD_MANIFEST_VERSION}",
            manifest.version
        ));
    }
    // Callers may pass OS-native separators; the manifest is authored with
    // forward slashes.
    if manifest.scene != scene_relative.replace('\\', "/") {
        return disabled(format!(
            "world manifest is for '{}', not '{scene_relative}'",
            manifest.scene
        ));
    }

    let mut warnings = Vec::new();

    // Cooked collision manifest: source of the streamable chunk set
    // (includes border slivers the world manifest doesn't list).
    let collision_rel = format!("collision/{stem}/manifest.ron");
    let cooked_chunks: HashMap<IVec2, u64> = match asset_source::read_string(&collision_rel)
        .map_err(|e| e.to_string())
        .and_then(|s| ron::from_str::<CollisionManifest>(&s).map_err(|e| e.to_string()))
    {
        Ok(m) => m
            .chunks
            .iter()
            .map(|c| (IVec2::new(c.coord[0], c.coord[1]), c.content_hash))
            .collect(),
        Err(e) => {
            warnings.push(format!(
                "{collision_rel}: {e} — streaming without collision chunks"
            ));
            HashMap::new()
        }
    };

    let mut cell_by_coord = HashMap::new();
    for (i, cell) in manifest.cells.iter().enumerate() {
        let coord = IVec2::new(cell.coord[0], cell.coord[1]);
        if cell_by_coord.insert(coord, i).is_some() {
            warnings.push(format!("duplicate cell {coord} in {manifest_rel}"));
        }
        if !cooked_chunks.is_empty() && !cooked_chunks.contains_key(&coord) {
            warnings.push(format!(
                "cell {coord}: collision chunk '{}' not in cooked manifest — cell collision degraded",
                cell.collision_chunk
            ));
        }
    }

    (
        Some(LoadedWorld {
            stem,
            manifest,
            cell_by_coord,
            cooked_chunks,
        }),
        WorldLoadReport {
            warnings,
            disabled: None,
        },
    )
}

/// Ordered streaming operations for one update: loads nearest-first,
/// unloads farthest-first.
#[derive(Debug, Default, PartialEq)]
pub struct StreamOps {
    pub loads: Vec<IVec2>,
    pub unloads: Vec<IVec2>,
}

fn chebyshev(a: IVec2, b: IVec2) -> i32 {
    (a.x - b.x).abs().max((a.y - b.y).abs())
}

/// Pure desired-set diff with hysteresis. `available` bounds what can load
/// (manifest cells or cooked chunk coords); `resident` is what currently is.
pub fn diff_ops(
    center: IVec2,
    resident: &HashSet<IVec2>,
    available: &HashSet<IVec2>,
    config: &StreamingConfig,
) -> StreamOps {
    let mut loads: Vec<IVec2> = Vec::new();
    for dy in -config.r_load..=config.r_load {
        for dx in -config.r_load..=config.r_load {
            let coord = center + IVec2::new(dx, dy);
            if available.contains(&coord) && !resident.contains(&coord) {
                loads.push(coord);
            }
        }
    }
    // Deterministic: nearest-first, then row-major
    loads.sort_by_key(|c| (chebyshev(*c, center), c.y, c.x));

    let mut unloads: Vec<IVec2> = resident
        .iter()
        .copied()
        .filter(|c| chebyshev(*c, center) > config.r_unload)
        .collect();
    unloads.sort_by_key(|c| (-chebyshev(*c, center), c.y, c.x));

    StreamOps { loads, unloads }
}

/// Full-world diff (editor mode): everything `available` loads
/// (nearest-first from `center`), anything resident that left `available`
/// unloads. No rings, no hysteresis.
pub fn full_ops(center: IVec2, resident: &HashSet<IVec2>, available: &HashSet<IVec2>) -> StreamOps {
    let mut loads: Vec<IVec2> = available.difference(resident).copied().collect();
    loads.sort_by_key(|c| (chebyshev(*c, center), c.y, c.x));
    let mut unloads: Vec<IVec2> = resident.difference(available).copied().collect();
    unloads.sort_by_key(|c| (-chebyshev(*c, center), c.y, c.x));
    StreamOps { loads, unloads }
}

/// World streamer resource. This package carries the manifest + zone
/// tracking + pure desired-set core; lifecycle execution (IO worker, budget,
/// spawn/despawn) lands in package 3.
#[derive(Default)]
pub struct WorldStreamer {
    pub config: StreamingConfig,
    /// Editor mode: keep the entire world resident instead of rings around
    /// the streaming center.
    pub full_world: bool,
    pub(super) world: Option<LoadedWorld>,
    current_zone: Option<u32>,
    /// Lifecycle execution state (IO worker, residency, in-flight tokens).
    pub(super) runtime: super::lifecycle::Runtime,
}

impl WorldStreamer {
    /// Loads the world manifest for a scene, replacing any previous world.
    /// Caller must `flush` first if a previous world was streaming.
    /// Returns the report for the caller to log.
    pub fn load_for_scene(&mut self, scene_relative: &str) -> WorldLoadReport {
        debug_assert!(
            self.runtime.is_empty(),
            "load_for_scene without flush: streamed content leaks"
        );
        let (world, report) = load_world(scene_relative);
        self.world = world;
        self.current_zone = None;
        report
    }

    /// Drops manifest and zone state (scene unload / tab park).
    /// Caller must `flush` first if streaming was active.
    pub fn clear(&mut self) {
        debug_assert!(
            self.runtime.is_empty(),
            "clear without flush: streamed content leaks"
        );
        self.world = None;
        self.current_zone = None;
    }

    pub fn world(&self) -> Option<&LoadedWorld> {
        self.world.as_ref()
    }

    /// True when a valid world manifest is loaded (streaming possible).
    pub fn is_active(&self) -> bool {
        self.world.is_some()
    }

    pub fn current_zone(&self) -> Option<u32> {
        self.current_zone
    }

    /// Updates zone tracking for the center cell; returns an event on
    /// transition (caller sends it to the ECS event queue).
    pub fn update_zone(&mut self, center_cell: IVec2) -> Option<ZoneChanged> {
        let world = self.world.as_ref()?;
        let zone = world.zone_of_cell(center_cell);
        if zone != self.current_zone {
            let event = ZoneChanged {
                previous: self.current_zone,
                current: zone,
            };
            self.current_zone = zone;
            Some(event)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(min: i32, max: i32) -> HashSet<IVec2> {
        let mut s = HashSet::new();
        for y in min..=max {
            for x in min..=max {
                s.insert(IVec2::new(x, y));
            }
        }
        s
    }

    fn config() -> StreamingConfig {
        StreamingConfig::default() // r_load = 2, r_unload = 3
    }

    #[test]
    fn loads_ring_nearest_first_clamped_to_available() {
        let available = grid(-4, 3);
        let ops = diff_ops(IVec2::new(-4, -4), &HashSet::new(), &available, &config());

        // Corner center: only the in-world quadrant of the 5×5 ring exists
        assert_eq!(ops.loads.len(), 9);
        assert_eq!(ops.loads[0], IVec2::new(-4, -4)); // center first
        assert!(ops.unloads.is_empty());

        // Nearest-first ordering
        let dists: Vec<i32> = ops
            .loads
            .iter()
            .map(|c| (c.x + 4).abs().max((c.y + 4).abs()))
            .collect();
        let mut sorted = dists.clone();
        sorted.sort();
        assert_eq!(dists, sorted);
    }

    #[test]
    fn hysteresis_keeps_lingering_ring_resident() {
        let available = grid(-4, 3);
        let center = IVec2::new(0, 0);
        let resident: HashSet<IVec2> = diff_ops(center, &HashSet::new(), &available, &config())
            .loads
            .into_iter()
            .collect();

        // Step one cell right: old -2 column is now at distance 3 = r_unload
        // → must NOT unload (hysteresis), but new +3 column loads.
        let ops = diff_ops(IVec2::new(1, 0), &resident, &available, &config());
        assert!(ops.unloads.is_empty());
        assert!(ops.loads.iter().all(|c| c.x == 3));
    }

    #[test]
    fn oscillation_on_boundary_never_thrashes() {
        let available = grid(-4, 3);
        let cfg = config();
        let mut resident: HashSet<IVec2> = HashSet::new();

        for i in 0..10 {
            let center = IVec2::new(i % 2, 0); // hop between two cells
            let ops = diff_ops(center, &resident, &available, &cfg);
            assert!(ops.unloads.is_empty(), "oscillation caused unloads");
            resident.extend(ops.loads);
        }
    }

    #[test]
    fn far_jump_unloads_farthest_first() {
        let available = grid(-4, 3);
        let cfg = config();
        let resident: HashSet<IVec2> =
            diff_ops(IVec2::new(-4, -4), &HashSet::new(), &available, &cfg)
                .loads
                .into_iter()
                .collect();

        let center = IVec2::new(3, 3);
        let ops = diff_ops(center, &resident, &available, &cfg);
        // Whole old corner is beyond r_unload from (3,3)
        assert_eq!(ops.unloads.len(), resident.len());

        let dists: Vec<i32> = ops
            .unloads
            .iter()
            .map(|c| (c.x - 3).abs().max((c.y - 3).abs()))
            .collect();
        let mut sorted = dists.clone();
        sorted.sort_by(|a, b| b.cmp(a));
        assert_eq!(dists, sorted, "unloads must be farthest-first");
    }

    #[test]
    fn resident_cells_outside_available_still_unload() {
        // Cells can leave `available` (e.g. hot reload shrank the manifest)
        let mut resident = HashSet::new();
        resident.insert(IVec2::new(9, 9));
        let ops = diff_ops(IVec2::new(0, 0), &resident, &grid(-4, 3), &config());
        assert_eq!(ops.unloads, vec![IVec2::new(9, 9)]);
    }

    #[test]
    fn zone_transition_emits_event_once() {
        let mut streamer = WorldStreamer::default();
        streamer.world = Some(LoadedWorld {
            stem: "test".into(),
            manifest: WorldManifest {
                version: 1,
                scene: "scenes/test.scene".into(),
                cell_size: 64.0,
                controller_ref: game_shared::world_manifest::ControllerRef {
                    slope_limit_deg: 45.0,
                    step_height: 0.4,
                    jump_gap: 3.0,
                },
                cells: vec![
                    game_shared::world_manifest::CellEntry {
                        coord: [0, 0],
                        zone_id: 0,
                        root_entity_guid: String::new(),
                        mesh: String::new(),
                        collision_chunk: String::new(),
                    },
                    game_shared::world_manifest::CellEntry {
                        coord: [1, 0],
                        zone_id: 1,
                        root_entity_guid: String::new(),
                        mesh: String::new(),
                        collision_chunk: String::new(),
                    },
                ],
                zones: Vec::new(),
                spawn_points: Vec::new(),
            },
            cell_by_coord: [(IVec2::new(0, 0), 0), (IVec2::new(1, 0), 1)]
                .into_iter()
                .collect(),
            cooked_chunks: HashMap::new(),
        });

        // Entering zone 0 from nowhere
        let e = streamer.update_zone(IVec2::new(0, 0)).unwrap();
        assert_eq!((e.previous, e.current), (None, Some(0)));
        // Same zone: no event
        assert!(streamer.update_zone(IVec2::new(0, 0)).is_none());
        // Crossing into zone 1
        let e = streamer.update_zone(IVec2::new(1, 0)).unwrap();
        assert_eq!((e.previous, e.current), (Some(0), Some(1)));
        // Leaving the world
        let e = streamer.update_zone(IVec2::new(9, 9)).unwrap();
        assert_eq!((e.previous, e.current), (Some(1), None));
    }
}
