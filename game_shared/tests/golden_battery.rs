//! Golden battery + seam fuzz against the checked-in cooked test chunk set
//! in `tests/data/collision/` (see the M2 plan, "Parity & correctness").
//!
//! The `.ccol` bytes are canonical — M6 reruns this battery in the server
//! WASM against the same files. After changing `test_chunk_set` or the
//! format, regenerate them:
//!   cargo test -p game_shared --test golden_battery regenerate -- --ignored

use game_shared::collision::battery::{self, Battery};
use game_shared::collision::format::{write_chunk, ChunkData, TriMeshSection};
use game_shared::collision::ChunkStore;
use glam::{IVec2, Quat, Vec3};
use parry3d::shape::Ball;
use std::path::PathBuf;

fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/collision")
}

fn chunk_path(coord: IVec2) -> PathBuf {
    data_dir().join(format!("{}_{}.ccol", coord.x, coord.y))
}

/// Hand-authored test geometry (all coordinates dyadic → exact in f32):
///
/// Chunk (0,0), local == world:
/// - ground quad z=0 over [0,64]²                        ids 1, 2
/// - 45° slope rising +x: x 8→24, z 0→16, y∈[8,24]       ids 10, 11
/// - step top z=2, x∈[40,48], y∈[8,16]                   ids 20, 21
/// - step west face x=40, y∈[8,16], z∈[0,2], normal -x   ids 22, 23
/// - seam platform z=4, world x∈[60,68], y∈[20,28]       ids 30, 31
///
/// Chunk (1,0): ground quad ids 3, 4; the seam platform duplicated with the
/// same ids (border-duplication convention — local coords relative to x=64).
fn test_chunk_set() -> Vec<ChunkData> {
    let quad = |v: &mut Vec<[f32; 3]>, i: &mut Vec<[u32; 3]>, corners: [[f32; 3]; 4]| {
        let base = v.len() as u32;
        v.extend_from_slice(&corners);
        i.push([base, base + 1, base + 2]);
        i.push([base, base + 2, base + 3]);
    };

    // Chunk (0,0)
    let mut v0 = Vec::new();
    let mut i0 = Vec::new();
    quad(
        &mut v0,
        &mut i0,
        [[0., 0., 0.], [64., 0., 0.], [64., 64., 0.], [0., 64., 0.]],
    );
    quad(
        &mut v0,
        &mut i0,
        [[8., 8., 0.], [24., 8., 16.], [24., 24., 16.], [8., 24., 0.]],
    );
    quad(
        &mut v0,
        &mut i0,
        [[40., 8., 2.], [48., 8., 2.], [48., 16., 2.], [40., 16., 2.]],
    );
    quad(
        &mut v0,
        &mut i0,
        [[40., 8., 0.], [40., 8., 2.], [40., 16., 2.], [40., 16., 0.]],
    );
    quad(
        &mut v0,
        &mut i0,
        [
            [60., 20., 4.],
            [68., 20., 4.],
            [68., 28., 4.],
            [60., 28., 4.],
        ],
    );
    let chunk0 = ChunkData {
        coord: IVec2::new(0, 0),
        local_aabb: ([0., 0., 0.], [68., 64., 16.]),
        trimesh: TriMeshSection {
            vertices: v0,
            indices: i0,
            triangle_ids: vec![1, 2, 10, 11, 20, 21, 22, 23, 30, 31],
            triangle_flags: vec![0; 10],
        },
    };

    // Chunk (1,0), local = world - (64, 0)
    let mut v1 = Vec::new();
    let mut i1 = Vec::new();
    quad(
        &mut v1,
        &mut i1,
        [[0., 0., 0.], [64., 0., 0.], [64., 64., 0.], [0., 64., 0.]],
    );
    quad(
        &mut v1,
        &mut i1,
        [[-4., 20., 4.], [4., 20., 4.], [4., 28., 4.], [-4., 28., 4.]],
    );
    let chunk1 = ChunkData {
        coord: IVec2::new(1, 0),
        local_aabb: ([-4., 0., 0.], [64., 64., 4.]),
        trimesh: TriMeshSection {
            vertices: v1,
            indices: i1,
            triangle_ids: vec![3, 4, 30, 31],
            triangle_flags: vec![0; 4],
        },
    };

    vec![chunk0, chunk1]
}

fn load_store() -> ChunkStore {
    let mut store = ChunkStore::default();
    for chunk in test_chunk_set() {
        let bytes = std::fs::read(chunk_path(chunk.coord)).expect("checked-in test chunk");
        store.insert_chunk(&bytes).expect("valid test chunk");
    }
    store
}

#[test]
#[ignore = "regenerates tests/data/collision/*.ccol — run after geometry/format changes"]
fn regenerate() {
    std::fs::create_dir_all(data_dir()).unwrap();
    for chunk in test_chunk_set() {
        std::fs::write(chunk_path(chunk.coord), write_chunk(&chunk, 0)).unwrap();
    }
}

/// The checked-in bytes are canonical for M6 parity; this catches silent
/// drift between them and the generator/format (rerun `regenerate` when
/// the change is intentional).
#[test]
fn checked_in_chunks_match_generator() {
    for chunk in test_chunk_set() {
        let generated = write_chunk(&chunk, 0);
        let on_disk = std::fs::read(chunk_path(chunk.coord)).expect("checked-in test chunk");
        assert_eq!(
            generated, on_disk,
            "chunk {:?} differs from checked-in bytes — rerun the `regenerate` test",
            chunk.coord
        );
    }
}

#[test]
fn golden_battery() {
    let store = load_store();
    let text = std::fs::read_to_string(data_dir().join("battery.ron")).expect("battery.ron");
    let battery: Battery = ron::from_str(&text).expect("battery.ron parses");
    assert!(battery.cases.len() >= 10, "battery lost cases");
    let failures = battery::run(&store, &battery);
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// Deterministic pseudo-random f32 in [0, 1) — no rand dep.
struct Lcg(u64);
impl Lcg {
    fn next_f32(&mut self) -> f32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }
}

/// Randomized sweeps over/across the chunk border at x=64: every cast must
/// produce exactly one consistent hit on the duplicated seam platform —
/// no gap, no double-hit artifacts (single-`Option` API + id dedup), stable
/// attribution to the duplicated triangles.
#[test]
fn seam_fuzz_no_gap_single_consistent_hit() {
    let store = load_store();
    let ball = Ball::new(0.3);
    let mut rng = Lcg(0x5EED_CAFE);

    let check = |hit: game_shared::collision::ShapeHit, delta: Vec3, what: &str| {
        assert!(
            (hit.toi - 0.7125).abs() * delta.length() <= 1e-3,
            "{what}: toi {} off analytic 0.7125",
            hit.toi
        );
        assert!(
            (hit.position.z - 4.0).abs() <= 1e-3,
            "{what}: contact z {} != platform top",
            hit.position.z
        );
        let angle = hit.normal.normalize().dot(Vec3::Z).clamp(-1.0, 1.0).acos();
        assert!(
            angle.to_degrees() <= 0.1,
            "{what}: normal {} not up",
            hit.normal
        );
        assert!(
            hit.triangle_id == 30 || hit.triangle_id == 31,
            "{what}: hit id {} is not a platform triangle",
            hit.triangle_id
        );
    };

    // Vertical drops over the platform interior (spanning the seam).
    for _ in 0..200 {
        let x = 61.0 + rng.next_f32() * 6.0;
        let y = 21.0 + rng.next_f32() * 6.0;
        let delta = Vec3::new(0.0, 0.0, -8.0);
        let hit = store
            .cast_shape(&ball, Vec3::new(x, y, 10.0), Quat::IDENTITY, delta)
            .unwrap_or_else(|| panic!("gap at seam: drop at ({x}, {y})"));
        check(hit, delta, "drop");
    }

    // Diagonal sweeps starting west of the seam and landing east of it.
    for _ in 0..200 {
        let xs = 60.0 + rng.next_f32() * 2.0;
        let dx = 2.0 + rng.next_f32() * 3.0;
        let y = 21.0 + rng.next_f32() * 6.0;
        let delta = Vec3::new(dx, 0.0, -8.0);
        let hit = store
            .cast_shape(&ball, Vec3::new(xs, y, 10.0), Quat::IDENTITY, delta)
            .unwrap_or_else(|| panic!("gap at seam: sweep from ({xs}, {y}) delta {delta}"));
        check(hit, delta, "sweep");
    }
}
