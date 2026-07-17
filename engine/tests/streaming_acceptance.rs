//! M4 acceptance (package 5): streamed-vs-full collision parity, streamed
//! cell GUID stability across load/unload/reload, and the D4 flythrough
//! hitch harness (streaming main-thread budget).
//!
//! The GPU-backed tests create a headless Vulkan device; on hosts without a
//! Vulkan driver they skip with a message instead of failing.

use std::collections::HashMap;
use std::sync::{Arc, Once};
use std::time::Duration;

use game_shared::world_grid::{self, CHUNK_SIZE};
use glam::{IVec2, Vec3};
use rust_engine::engine::assets::asset_source;
use rust_engine::engine::collision::CollisionWorld;
use rust_engine::engine::ecs::components::EntityGuid;
use rust_engine::engine::rendering::rendering_3d::mesh_manager::MeshManager;
use rust_engine::engine::world::{load_world, StreamedCell, StreamingCtx, WorldStreamer};
use vulkano::memory::allocator::StandardMemoryAllocator;

const SCENE: &str = "scenes/greybox.scene";

fn init_content() {
    static INIT: Once = Once::new();
    INIT.call_once(|| asset_source::init_filesystem("../content".into()));
}

fn headless_allocator() -> Option<Arc<StandardMemoryAllocator>> {
    use vulkano::device::{Device, DeviceCreateInfo, QueueCreateInfo, QueueFlags};
    use vulkano::instance::{Instance, InstanceCreateInfo};
    use vulkano::VulkanLibrary;

    let library = VulkanLibrary::new().ok()?;
    let instance = Instance::new(library, InstanceCreateInfo::default()).ok()?;
    let physical = instance.enumerate_physical_devices().ok()?.next()?;
    let queue_family_index = physical
        .queue_family_properties()
        .iter()
        .position(|q| q.queue_flags.intersects(QueueFlags::GRAPHICS))?
        as u32;
    let (device, _queues) = Device::new(
        physical,
        DeviceCreateInfo {
            queue_create_infos: vec![QueueCreateInfo {
                queue_family_index,
                ..Default::default()
            }],
            ..Default::default()
        },
    )
    .ok()?;
    Some(Arc::new(StandardMemoryAllocator::new_default(device)))
}

fn chebyshev(a: IVec2, b: IVec2) -> i32 {
    (a.x - b.x).abs().max((a.y - b.y).abs())
}

// ------------------------------------------------------------------ harness

struct Harness {
    streamer: WorldStreamer,
    world: hecs::World,
    meshes: MeshManager,
    collision: CollisionWorld,
    allocator: Arc<StandardMemoryAllocator>,
}

impl Harness {
    fn new(allocator: Arc<StandardMemoryAllocator>) -> Self {
        let mut streamer = WorldStreamer::default();
        let report = streamer.load_for_scene(SCENE);
        assert!(report.disabled.is_none(), "{:?}", report.disabled);
        let mut collision = CollisionWorld::new();
        collision.begin_streaming(SCENE);
        Self {
            streamer,
            world: hecs::World::new(),
            meshes: MeshManager::default(),
            collision,
            allocator,
        }
    }

    /// One streaming update; returns the main-thread time in ms.
    fn tick(&mut self, center: Vec3) -> f32 {
        let start = std::time::Instant::now();
        let mut ctx = StreamingCtx {
            world: &mut self.world,
            meshes: &mut self.meshes,
            allocator: self.allocator.clone(),
            collision: &mut self.collision,
        };
        self.streamer.update_streaming(center, &mut ctx);
        start.elapsed().as_secs_f32() * 1000.0
    }

    /// Ticks until the resident cell set matches `expected_cells` and no IO
    /// is pending.
    fn converge(&mut self, center: Vec3, expected_cells: usize) {
        for _ in 0..4000 {
            self.tick(center);
            if self.streamer.resident_cell_count() == expected_cells
                && self.streamer.in_flight_count() == 0
                && self.streamer.ready_queue_depth() == 0
            {
                return;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        panic!(
            "streaming never converged to {expected_cells} cells at {center} (resident {})",
            self.streamer.resident_cell_count()
        );
    }

    /// Ticks until every cell in the load ring is resident and no IO is
    /// pending. Unlike `converge`, tolerates extra trailing residents kept
    /// by hysteresis.
    fn converge_ring(&mut self, center: Vec3) {
        let center_cell = world_grid::chunk_coord(center);
        let r = self.streamer.config.r_load;
        let required: Vec<IVec2> = self
            .streamer
            .world()
            .expect("world loaded")
            .cell_coords()
            .iter()
            .copied()
            .filter(|&c| chebyshev(c, center_cell) <= r)
            .collect();
        for _ in 0..4000 {
            self.tick(center);
            let resident = self.cell_guids();
            if required.iter().all(|c| resident.contains_key(c))
                && self.streamer.in_flight_count() == 0
                && self.streamer.ready_queue_depth() == 0
            {
                return;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        panic!(
            "load ring at {center} never fully resident ({} cells)",
            self.streamer.resident_cell_count()
        );
    }

    /// Manifest cells within the load ring of `center_cell`.
    fn expected_ring(&self, center_cell: IVec2) -> usize {
        let r = self.streamer.config.r_load;
        self.streamer
            .world()
            .expect("world loaded")
            .cell_coords()
            .iter()
            .filter(|&&c| chebyshev(c, center_cell) <= r)
            .count()
    }

    fn cell_guids(&self) -> HashMap<IVec2, uuid::Uuid> {
        self.world
            .query::<(&StreamedCell, &EntityGuid)>()
            .iter()
            .map(|(_, (cell, guid))| (cell.coord, guid.0))
            .collect()
    }

    fn flush(&mut self) {
        let mut ctx = StreamingCtx {
            world: &mut self.world,
            meshes: &mut self.meshes,
            allocator: self.allocator.clone(),
            collision: &mut self.collision,
        };
        self.streamer.flush(&mut ctx);
    }
}

// ----------------------------------------------------- collision parity

fn compare_ray(
    full: &CollisionWorld,
    streamed: &CollisionWorld,
    origin: Vec3,
    dir: Vec3,
    max_toi: f32,
) -> usize {
    let a = full.store().raycast(origin, dir, max_toi);
    let b = streamed.store().raycast(origin, dir, max_toi);
    match (a, b) {
        (None, None) => 0,
        (Some(a), Some(b)) => {
            assert_eq!(a.triangle_id, b.triangle_id, "ray {origin} dir {dir}");
            assert_eq!(a.toi, b.toi, "ray {origin} dir {dir}");
            assert_eq!(a.position, b.position, "ray {origin} dir {dir}");
            assert_eq!(a.normal, b.normal, "ray {origin} dir {dir}");
            assert_eq!(a.flags, b.flags, "ray {origin} dir {dir}");
            1
        }
        (a, b) => panic!(
            "parity mismatch at {origin} dir {dir}: full={:?} streamed={:?}",
            a.map(|h| h.toi),
            b.map(|h| h.toi)
        ),
    }
}

/// A chunk-streamed CollisionWorld (insert/remove/reinsert) must answer
/// raycasts identically to the monolithic full load.
#[test]
fn streamed_collision_matches_full_load() {
    init_content();

    let mut full = CollisionWorld::new();
    let report = full.load_for_scene(SCENE);
    assert_eq!(report.disabled, None);
    assert_eq!(report.skipped, 0, "warnings: {:?}", report.warnings);

    let (world, report) = load_world(SCENE);
    assert!(report.disabled.is_none(), "{:?}", report.disabled);
    let world = world.expect("loaded world");

    let mut streamed = CollisionWorld::new();
    streamed.begin_streaming(SCENE);
    let mut coords: Vec<IVec2> = world.chunk_coords().into_iter().collect();
    coords.sort_by_key(|c| (c.y, c.x));
    assert_eq!(coords.len(), full.store().len());

    let insert = |cw: &mut CollisionWorld, coord: IVec2| {
        let bytes = asset_source::read_bytes(&world.chunk_path(coord)).expect("chunk bytes");
        cw.insert_chunk_bytes(&bytes).expect("chunk insert");
    };
    for &c in &coords {
        insert(&mut streamed, c);
    }
    // Unload + reload one column: parity must survive remove/reinsert churn.
    for &c in coords.iter().filter(|c| c.x == 0) {
        assert!(streamed.remove_chunk(c));
    }
    for &c in coords.iter().rev().filter(|c| c.x == 0) {
        insert(&mut streamed, c);
    }

    // Battery: 3×3 downward rays per chunk, plus long horizontal and
    // diagonal rays across the whole world.
    let mut hits = 0usize;
    let mut casts = 0usize;
    for &coord in &coords {
        let origin = world_grid::chunk_origin(coord);
        for sy in 0..3 {
            for sx in 0..3 {
                let p = Vec3::new(
                    origin.x + (sx as f32 + 0.5) * CHUNK_SIZE / 3.0,
                    origin.y + (sy as f32 + 0.5) * CHUNK_SIZE / 3.0,
                    80.0,
                );
                hits += compare_ray(&full, &streamed, p, Vec3::NEG_Z, 200.0);
                casts += 1;
            }
        }
    }
    for i in -8..=8 {
        let t = i as f32 * 30.0;
        hits += compare_ray(&full, &streamed, Vec3::new(-300.0, t, 1.5), Vec3::X, 600.0);
        hits += compare_ray(&full, &streamed, Vec3::new(t, -300.0, 3.0), Vec3::Y, 600.0);
        let diag = Vec3::new(1.0, 0.2, -0.15).normalize();
        hits += compare_ray(&full, &streamed, Vec3::new(-300.0, t, 60.0), diag, 700.0);
        casts += 3;
    }
    assert!(hits * 2 > casts, "battery too sparse: {hits}/{casts} hits");
}

// ----------------------------------------------------- GUID stability

/// Cells keep their manifest GUID across load → unload → reload; the
/// streamed-cell GUID map tracks the live entity.
#[test]
fn streamed_cell_guids_stable_across_reload() {
    init_content();
    let Some(allocator) = headless_allocator() else {
        eprintln!("skipping: no Vulkan device");
        return;
    };
    let mut h = Harness::new(allocator);

    let home = Vec3::new(32.0, 32.0, 10.0); // cell (0, 0), world center
    let expected = h.expected_ring(IVec2::ZERO);
    assert_eq!(expected, 25, "5×5 load ring fully inside the 8×8 world");
    h.converge(home, expected);

    let before = h.cell_guids();
    assert_eq!(before.len(), expected);
    {
        let world = h.streamer.world().expect("world loaded");
        for (&coord, &guid) in &before {
            let manifest = EntityGuid::from_string(&world.cell(coord).unwrap().root_entity_guid)
                .expect("valid manifest guid");
            assert_eq!(guid, manifest.0, "cell {coord} guid != manifest");
        }
    }
    for (&coord, &guid) in &before {
        let entity = h.streamer.entity_for_guid(guid).expect("guid resolves");
        assert_eq!(h.world.get::<&StreamedCell>(entity).unwrap().coord, coord);
    }

    // Far jump: everything leaves the unload ring. Chunk unloads trail the
    // cell unloads under the frame budget, so keep ticking until drained.
    let far = Vec3::new(2000.0, 2000.0, 10.0);
    h.converge(far, 0);
    for _ in 0..200 {
        if h.streamer.resident_chunk_count() == 0 {
            break;
        }
        h.tick(far);
    }
    assert_eq!(h.streamer.resident_chunk_count(), 0);
    assert!(before.values().all(|&g| h.streamer.entity_for_guid(g).is_none()));
    assert_eq!(h.cell_guids().len(), 0);

    // Return: same GUIDs on fresh entities.
    h.converge(home, expected);
    assert_eq!(before, h.cell_guids());
    for &guid in before.values() {
        assert!(h.streamer.entity_for_guid(guid).is_some());
    }
    h.flush();
}

// ----------------------------------------------------- flythrough harness

fn p99(samples: &[f32]) -> f32 {
    let mut v = samples.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[((v.len() as f32 * 0.99).ceil() as usize).clamp(1, v.len()) - 1]
}

/// D4 acceptance: straight-line flythrough across the greybox world at 3×
/// run speed. Streaming main-thread work must never exceed 2 ms per frame,
/// and its p99 must sit within 1 ms of the standing-still baseline.
/// (Headless: measures the streaming update itself, not renderer frame time.)
#[test]
fn flythrough_streaming_stays_within_budget() {
    init_content();
    let Some(allocator) = headless_allocator() else {
        eprintln!("skipping: no Vulkan device");
        return;
    };
    let mut h = Harness::new(allocator);

    const RUN_SPEED: f32 = 8.0; // m/s humanoid run reference
    let speed = 3.0 * RUN_SPEED;
    let dt = 1.0 / 60.0;
    // Cell row y=0, from one cell outside the west edge to one outside the
    // east edge: 9 boundary crossings on X.
    let start = Vec3::new(-288.0, 32.0, 10.0);
    let end_x = 288.0;

    // Standing-still baseline.
    h.converge(start, h.expected_ring(world_grid::chunk_coord(start)));
    let mut baseline = Vec::new();
    for _ in 0..300 {
        baseline.push(h.tick(start));
        std::thread::sleep(Duration::from_millis(1));
    }

    // Flythrough.
    let mut pos = start;
    let mut frames = Vec::new();
    let mut crossings = 0usize;
    let mut prev_cell = world_grid::chunk_coord(pos);
    while pos.x < end_x {
        pos.x += speed * dt;
        let cell = world_grid::chunk_coord(pos);
        if cell != prev_cell {
            crossings += 1;
            prev_cell = cell;
        }
        frames.push(h.tick(pos));
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(crossings >= 8, "only {crossings} cell boundary crossings");

    let worst = frames.iter().copied().fold(0.0f32, f32::max);
    let (fly, base) = (p99(&frames), p99(&baseline));
    println!(
        "flythrough: {} frames, {crossings} crossings, worst {worst:.3} ms, \
         p99 {fly:.3} ms (baseline p99 {base:.3} ms)",
        frames.len()
    );
    assert!(worst < 2.0, "streaming main-thread work hit {worst:.2} ms");
    assert!(
        fly <= base + 1.0,
        "flythrough p99 {fly:.2} ms exceeds baseline {base:.2} ms + 1 ms"
    );

    // Streaming kept up: the ring at the end position is fully resident
    // (hysteresis may keep trailing cells too).
    h.converge_ring(pos);
    h.flush();
}
