//! Controller battery on synthetic meshes, plus recorded greybox traces.
//! Synthetic chunks live at coord (0, 0) so chunk-local == world coordinates.

use super::trace::{MotionTrace, TraceStart};
use super::*;
use crate::collision::format::{write_chunk, ChunkData, TriMeshSection};
use glam::IVec2;
use std::path::PathBuf;

type Tri = ([[f32; 3]; 3], u32);

fn store_from_tris(tris: &[Tri]) -> ChunkStore {
    let mut trimesh = TriMeshSection::default();
    let (mut min, mut max) = ([f32::MAX; 3], [f32::MIN; 3]);
    for (i, (tri, flags)) in tris.iter().enumerate() {
        let base = trimesh.vertices.len() as u32;
        for v in tri {
            for a in 0..3 {
                min[a] = min[a].min(v[a]);
                max[a] = max[a].max(v[a]);
            }
        }
        trimesh.vertices.extend_from_slice(tri);
        trimesh.indices.push([base, base + 1, base + 2]);
        trimesh.triangle_ids.push(i as u64 + 1);
        trimesh.triangle_flags.push(*flags);
    }
    let chunk = ChunkData {
        coord: IVec2::ZERO,
        local_aabb: (min, max),
        trimesh,
    };
    let mut store = ChunkStore::new();
    store.insert_chunk(&write_chunk(&chunk, 0)).unwrap();
    store
}

/// Two CCW-from-outside triangles for the quad `c0..c3`.
fn quad(c: [[f32; 3]; 4], flags: u32) -> [Tri; 2] {
    [([c[0], c[1], c[2]], flags), ([c[0], c[2], c[3]], flags)]
}

/// Horizontal floor at height `z` (normal +Z).
fn floor(x0: f32, y0: f32, x1: f32, y1: f32, z: f32, flags: u32) -> [Tri; 2] {
    quad(
        [[x0, y0, z], [x1, y0, z], [x1, y1, z], [x0, y1, z]],
        flags,
    )
}

/// Vertical wall in the YZ plane at `x`, facing −X.
fn wall_neg_x(x: f32, y0: f32, y1: f32, z0: f32, z1: f32, flags: u32) -> [Tri; 2] {
    quad(
        [[x, y0, z0], [x, y0, z1], [x, y1, z1], [x, y1, z0]],
        flags,
    )
}

/// Ramp rising along +X from `(x0, z0)` to `(x1, z1)`.
fn ramp_x(x0: f32, x1: f32, y0: f32, y1: f32, z0: f32, z1: f32, flags: u32) -> [Tri; 2] {
    quad(
        [[x0, y0, z0], [x1, y0, z1], [x1, y1, z1], [x0, y1, z0]],
        flags,
    )
}

fn flat_store() -> ChunkStore {
    store_from_tris(&floor(0.0, 0.0, 60.0, 60.0, 0.0, 0))
}

fn walk_x(sprint: bool) -> MoveIntent {
    MoveIntent {
        move_dir: [1.0, 0.0],
        yaw: 0.0,
        sprint,
        jump: false,
    }
}

fn run(cfg: &MotionConfig, state: MotionState, intent: &MoveIntent, n: usize, store: &ChunkStore) -> MotionState {
    (0..n).fold(state, |s, _| step(cfg, &s, intent, store))
}

fn feet(cfg: &MotionConfig, state: &MotionState) -> f32 {
    state.pos.z - cfg.capsule_half_seg - cfg.capsule_radius
}

const START: Vec3 = Vec3::new(30.0, 30.0, 0.0);

#[test]
fn falls_and_lands() {
    let store = flat_store();
    let cfg = MotionConfig::default();
    let mut state = MotionState::standing_at(&cfg, START + Vec3::Z * 3.0);
    state.grounded = false;
    state = run(&cfg, state, &MoveIntent::IDLE, 40, &store);
    assert!(state.grounded);
    assert!((feet(&cfg, &state)).abs() < 0.05, "feet at {}", feet(&cfg, &state));
    assert_eq!(state.vel.z, 0.0);
}

#[test]
fn walks_at_configured_speed() {
    let store = flat_store();
    let cfg = MotionConfig::default();
    let start = MotionState::standing_at(&cfg, START);
    let state = run(&cfg, start, &walk_x(false), 20, &store); // 1 s
    assert!(state.grounded);
    let dist = state.pos.x - start.pos.x;
    assert!((dist - cfg.walk_speed).abs() < 0.05, "walked {dist}");
}

#[test]
fn sprint_applies_multiplier() {
    let store = flat_store();
    let cfg = MotionConfig::default();
    let start = MotionState::standing_at(&cfg, START);
    let state = run(&cfg, start, &walk_x(true), 20, &store);
    let dist = state.pos.x - start.pos.x;
    let expected = cfg.walk_speed * cfg.sprint_mult;
    assert!((dist - expected).abs() < 0.05, "sprinted {dist}");
}

#[test]
fn diagonal_input_is_unit_clamped() {
    let store = flat_store();
    let cfg = MotionConfig::default();
    let start = MotionState::standing_at(&cfg, START);
    let intent = MoveIntent {
        move_dir: [1.0, 1.0],
        ..MoveIntent::IDLE
    };
    let state = run(&cfg, start, &intent, 20, &store);
    let dist = (state.pos - start.pos).truncate().length();
    assert!((dist - cfg.walk_speed).abs() < 0.05, "moved {dist}");
}

#[test]
fn jump_arc_and_reland() {
    let store = flat_store();
    let cfg = MotionConfig::default();
    let mut state = MotionState::standing_at(&cfg, START);
    let jump = MoveIntent {
        jump: true,
        ..MoveIntent::IDLE
    };
    state = step(&cfg, &state, &jump, &store);
    assert!(!state.grounded);
    let mut apex = 0.0f32;
    let mut relanded = 0;
    for i in 0..40 {
        state = step(&cfg, &state, &MoveIntent::IDLE, &store);
        apex = apex.max(feet(&cfg, &state));
        if state.grounded {
            relanded = i;
            break;
        }
    }
    // Discrete apex for jump_speed 8, gravity 20 @ 20 Hz ≈ 1.4 m.
    assert!(apex > 1.2 && apex < 1.7, "apex {apex}");
    assert!(state.grounded, "did not reland");
    assert!(relanded > 5, "relanded too fast (step {relanded})");
}

#[test]
fn jump_requires_ground() {
    let store = flat_store();
    let cfg = MotionConfig::default();
    let mut state = MotionState::standing_at(&cfg, START + Vec3::Z * 3.0);
    state.grounded = false;
    let jump = MoveIntent {
        jump: true,
        ..MoveIntent::IDLE
    };
    let next = step(&cfg, &state, &jump, &store);
    assert!(next.vel.z < 0.0, "airborne jump must not fire");
}

#[test]
fn slides_along_wall() {
    let mut tris = floor(0.0, 0.0, 60.0, 60.0, 0.0, 0).to_vec();
    tris.extend(wall_neg_x(33.0, 0.0, 60.0, 0.0, 3.0, 0));
    let store = store_from_tris(&tris);
    let cfg = MotionConfig::default();
    let start = MotionState::standing_at(&cfg, START);
    let intent = MoveIntent {
        move_dir: [1.0, 1.0],
        ..MoveIntent::IDLE
    };
    let state = run(&cfg, start, &intent, 40, &store);
    assert!(state.pos.x < 33.0 - cfg.capsule_radius + 0.05, "clipped into wall: x {}", state.pos.x);
    assert!(state.pos.y > start.pos.y + 3.0, "did not slide: y {}", state.pos.y);
    assert!(state.grounded);
}

#[test]
fn climbs_walkable_slope() {
    // 30° ramp (normal.z ≈ 0.87 > cos 50°).
    let mut tris = floor(0.0, 0.0, 33.0, 60.0, 0.0, 0).to_vec();
    let rise = 7.0 * 30f32.to_radians().tan();
    tris.extend(ramp_x(33.0, 40.0, 0.0, 60.0, 0.0, rise, 0));
    let store = store_from_tris(&tris);
    let cfg = MotionConfig::default();
    let start = MotionState::standing_at(&cfg, START);
    let state = run(&cfg, start, &walk_x(false), 40, &store);
    assert!(state.grounded);
    assert!(state.pos.z > start.pos.z + 1.0, "did not climb: z {}", state.pos.z);
}

#[test]
fn steep_slope_is_not_climbable() {
    // 60° ramp (normal.z = 0.5 < cos 50° ≈ 0.643).
    let mut tris = floor(0.0, 0.0, 33.0, 60.0, 0.0, 0).to_vec();
    let rise = 4.0 * 60f32.to_radians().tan();
    tris.extend(ramp_x(33.0, 37.0, 0.0, 60.0, 0.0, rise, 0));
    let store = store_from_tris(&tris);
    let cfg = MotionConfig::default();
    let start = MotionState::standing_at(&cfg, START);
    let state = run(&cfg, start, &walk_x(false), 40, &store);
    assert!(feet(&cfg, &state) < 1.0, "climbed steep slope to {}", feet(&cfg, &state));
}

#[test]
fn walkable_flag_overrides_slope() {
    let mut tris = floor(0.0, 0.0, 33.0, 60.0, 0.0, 0).to_vec();
    let rise = 4.0 * 60f32.to_radians().tan();
    tris.extend(ramp_x(33.0, 37.0, 0.0, 60.0, 0.0, rise, FLAG_WALKABLE));
    let store = store_from_tris(&tris);
    let cfg = MotionConfig::default();
    let start = MotionState::standing_at(&cfg, START);
    let state = run(&cfg, start, &walk_x(false), 40, &store);
    assert!(state.pos.z > start.pos.z + 0.5, "flagged slope not climbed: z {}", state.pos.z);
}

#[test]
fn blocking_flag_prevents_grounding() {
    let store = store_from_tris(&floor(0.0, 0.0, 60.0, 60.0, 0.0, FLAG_BLOCKING));
    let cfg = MotionConfig::default();
    let mut state = MotionState::standing_at(&cfg, START + Vec3::Z * 2.0);
    state.grounded = false;
    state = run(&cfg, state, &MoveIntent::IDLE, 40, &store);
    assert!(!state.grounded, "grounded on FLAG_BLOCKING surface");
    assert!(feet(&cfg, &state) < 0.1, "should rest near the surface");
}

#[test]
fn steps_up_small_ledge() {
    // 0.3 m ledge (< step_height 0.35).
    let mut tris = floor(0.0, 0.0, 33.0, 60.0, 0.0, 0).to_vec();
    tris.extend(floor(33.0, 0.0, 60.0, 60.0, 0.3, 0));
    tris.extend(wall_neg_x(33.0, 0.0, 60.0, 0.0, 0.3, 0));
    let store = store_from_tris(&tris);
    let cfg = MotionConfig::default();
    let start = MotionState::standing_at(&cfg, START);
    let state = run(&cfg, start, &walk_x(false), 30, &store);
    assert!(state.pos.x > 34.0, "did not pass ledge: x {}", state.pos.x);
    assert!(state.grounded);
    assert!((feet(&cfg, &state) - 0.3).abs() < 0.05, "feet at {}", feet(&cfg, &state));
}

#[test]
fn tall_ledge_blocks() {
    let mut tris = floor(0.0, 0.0, 33.0, 60.0, 0.0, 0).to_vec();
    tris.extend(floor(33.0, 0.0, 60.0, 60.0, 0.6, 0));
    tris.extend(wall_neg_x(33.0, 0.0, 60.0, 0.0, 0.6, 0));
    let store = store_from_tris(&tris);
    let cfg = MotionConfig::default();
    let start = MotionState::standing_at(&cfg, START);
    let state = run(&cfg, start, &walk_x(false), 30, &store);
    assert!(state.pos.x < 33.0, "climbed 0.6 m ledge: x {}", state.pos.x);
}

#[test]
fn snaps_down_small_drop() {
    // 0.2 m drop (< snap_dist 0.3): stays grounded across the edge.
    let mut tris = floor(0.0, 0.0, 33.0, 60.0, 0.0, 0).to_vec();
    tris.extend(floor(33.0, 0.0, 60.0, 60.0, -0.2, 0));
    let store = store_from_tris(&tris);
    let cfg = MotionConfig::default();
    let mut state = MotionState::standing_at(&cfg, START);
    for _ in 0..30 {
        state = step(&cfg, &state, &walk_x(false), &store);
        assert!(state.grounded, "lost ground at x {}", state.pos.x);
    }
    assert!(state.pos.x > 34.0);
    assert!((feet(&cfg, &state) + 0.2).abs() < 0.05, "feet at {}", feet(&cfg, &state));
}

#[test]
fn depenetrates_bounded() {
    let store = flat_store();
    let cfg = MotionConfig::default();
    let mut state = MotionState::standing_at(&cfg, START);
    state.pos.z -= 0.12; // buried 0.12 m
    let first = step(&cfg, &state, &MoveIntent::IDLE, &store);
    assert!(first.pos.z - state.pos.z <= 2.0 * cfg.skin + 1e-4, "push not bounded");
    state = run(&cfg, first, &MoveIntent::IDLE, 10, &store);
    assert!(feet(&cfg, &state) > -0.02, "still buried: feet {}", feet(&cfg, &state));
    assert!(state.grounded);
}

#[test]
fn yaw_wraps_and_rejects_non_finite() {
    assert!((wrap_yaw(3.0 * std::f32::consts::PI, 0.0) - std::f32::consts::PI).abs() < 1e-5);
    assert_eq!(wrap_yaw(f32::NAN, 0.7), 0.7);
    assert_eq!(wrap_yaw(f32::INFINITY, -0.3), -0.3);
    let store = flat_store();
    let cfg = MotionConfig::default();
    let start = MotionState::standing_at(&cfg, START);
    let intent = MoveIntent {
        yaw: -4.0 * std::f32::consts::PI + 1.0,
        ..MoveIntent::IDLE
    };
    let state = step(&cfg, &start, &intent, &store);
    assert!((state.yaw - 1.0).abs() < 1e-5);
}

#[test]
fn step_is_deterministic() {
    let store = flat_store();
    let cfg = MotionConfig::default();
    let start = MotionState::standing_at(&cfg, START);
    let intent = MoveIntent {
        move_dir: [0.6, -0.8],
        yaw: 1.0,
        sprint: true,
        jump: false,
    };
    let a = run(&cfg, start, &intent, 25, &store);
    let b = run(&cfg, start, &intent, 25, &store);
    assert_eq!(a, b);
}

// ---------------------------------------------------------------- traces

#[test]
fn trace_record_replay_roundtrip() {
    let store = flat_store();
    let mut trace = MotionTrace {
        name: "synthetic_walk".into(),
        config: MotionConfig::default(),
        start: TraceStart {
            pos: (START + Vec3::Z * 0.92).to_array(),
            vel: [0.0; 3],
            yaw: 0.0,
            grounded: true,
        },
        intents: (0..30)
            .map(|i| MoveIntent {
                move_dir: [1.0, 0.0],
                yaw: 0.0,
                sprint: false,
                jump: i == 10,
            })
            .collect(),
        expected: Vec::new(),
    };
    trace.record(&store);
    let text = ron::ser::to_string_pretty(&trace, Default::default()).unwrap();
    let parsed: MotionTrace = ron::from_str(&text).unwrap();
    let report = parsed.replay(&store).unwrap();
    assert_eq!(report.steps, 30);

    let mut broken = parsed;
    broken.expected[15][2] += 0.5;
    assert!(broken.replay(&store).is_err());
}

// ---------------------------------------------------------------- greybox

fn greybox_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../content/collision/greybox")
}

fn greybox_store() -> ChunkStore {
    let mut paths: Vec<_> = std::fs::read_dir(greybox_dir())
        .expect("content/collision/greybox present")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "ccol"))
        .collect();
    paths.sort();
    let mut store = ChunkStore::new();
    for p in &paths {
        store.insert_chunk(&std::fs::read(p).unwrap()).unwrap();
    }
    assert!(!store.is_empty());
    store
}

fn traces_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/motion/traces")
}

/// Golden parity trace (D5): recorded once by `record_greybox_trace`,
/// committed, and replayed here natively (and in WASM via `run_parity_trace`).
#[test]
fn greybox_golden_trace_replays() {
    let path = traces_dir().join("greybox_walk.ron");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("missing golden trace {path:?} ({e}); run `cargo test -p game_shared record_greybox_trace -- --ignored`"));
    let trace: MotionTrace = ron::from_str(&text).unwrap();
    let store = greybox_store();
    let report = trace.replay(&store).unwrap();
    assert_eq!(report.steps, trace.intents.len() as u32);
}

/// Run once to (re)record the golden trace:
/// `cargo test -p game_shared record_greybox_trace -- --ignored`
#[test]
#[ignore = "writes src/motion/traces/greybox_walk.ron"]
fn record_greybox_trace() {
    let store = greybox_store();
    let cfg = MotionConfig::default();
    let ground = store
        .raycast(Vec3::new(0.0, 0.0, 50.0), Vec3::NEG_Z, 100.0)
        .expect("ground under origin");
    let feet = Vec3::new(0.0, 0.0, 50.0 - ground.toi);
    let start = MotionState::standing_at(&cfg, feet);
    let mut trace = MotionTrace {
        name: "greybox_walk".into(),
        config: cfg,
        start: TraceStart {
            pos: start.pos.to_array(),
            vel: start.vel.to_array(),
            yaw: start.yaw,
            grounded: start.grounded,
        },
        // Walk +X, jump mid-run, turn to +Y sprinting, then settle idle —
        // exercises ground, air, landing and snap over real greybox terrain.
        intents: (0..80)
            .map(|i| match i {
                0..=29 => MoveIntent {
                    move_dir: [1.0, 0.0],
                    yaw: 0.0,
                    sprint: false,
                    jump: i == 15,
                },
                30..=59 => MoveIntent {
                    move_dir: [0.0, 1.0],
                    yaw: std::f32::consts::FRAC_PI_2,
                    sprint: true,
                    jump: false,
                },
                _ => MoveIntent::IDLE,
            })
            .collect(),
        expected: Vec::new(),
    };
    trace.record(&store);
    std::fs::create_dir_all(traces_dir()).unwrap();
    let text = ron::ser::to_string_pretty(&trace, Default::default()).unwrap();
    std::fs::write(traces_dir().join("greybox_walk.ron"), text).unwrap();
}

// ------------------------------------------------------- parity battery (D5)

/// Rest state standing on whatever the greybox has at `(x, y)` (raycast down).
fn standing_on(store: &ChunkStore, cfg: &MotionConfig, x: f32, y: f32) -> MotionState {
    let hit = store
        .raycast(Vec3::new(x, y, 60.0), Vec3::NEG_Z, 120.0)
        .unwrap_or_else(|| panic!("no ground under ({x}, {y})"));
    MotionState {
        yaw: 0.0,
        ..MotionState::standing_at(cfg, Vec3::new(x, y, 60.0 - hit.toi))
    }
}

/// Build a trace by choosing each intent from the live simulated state
/// (`None` ends the trace), then record expected positions.
fn build_trace(
    name: &str,
    store: &ChunkStore,
    start: MotionState,
    mut next: impl FnMut(usize, &MotionState) -> Option<MoveIntent>,
) -> MotionTrace {
    let cfg = MotionConfig::default();
    let mut state = start;
    let mut intents = Vec::new();
    while let Some(intent) = next(intents.len(), &state) {
        state = step(&cfg, &state, &intent, store);
        intents.push(intent);
    }
    let mut trace = MotionTrace {
        name: name.into(),
        config: cfg,
        start: TraceStart {
            pos: start.pos.to_array(),
            vel: start.vel.to_array(),
            yaw: start.yaw,
            grounded: start.grounded,
        },
        intents,
        expected: Vec::new(),
    };
    trace.record(store);
    trace
}

fn walk(dir: [f32; 2], sprint: bool, jump: bool) -> MoveIntent {
    MoveIntent {
        move_dir: dir,
        yaw: dir[1].atan2(dir[0]),
        sprint,
        jump,
    }
}

fn for_steps(n: usize, intent: MoveIntent) -> impl FnMut(usize, &MotionState) -> Option<MoveIntent> {
    move |i, _| (i < n).then_some(intent)
}

/// Every registered parity trace replays natively within tolerance against
/// the checked-in greybox chunks — the native half of D5 (the WASM half runs
/// the same replays via `run_parity_trace`).
#[test]
fn parity_battery_replays() {
    let store = greybox_store();
    for (id, _) in trace::EMBEDDED_TRACES {
        let t = trace::load_embedded(id)
            .unwrap_or_else(|e| panic!("{e}; run `cargo test -p game_shared record_parity_traces -- --ignored`"));
        let report = t
            .replay(&store)
            .unwrap_or_else(|e| panic!("trace {id:?} failed: {e}"));
        assert_eq!(report.steps, t.intents.len() as u32, "{id}");
    }
}

/// Randomized-intent fuzz (native only): the controller must stay finite,
/// inside the world envelope, and within speed bounds for arbitrary intent
/// streams — and be exactly deterministic across runs.
#[test]
fn randomized_intent_fuzz() {
    let store = greybox_store();
    let cfg = MotionConfig::default();
    let max_speed = cfg.walk_speed * cfg.sprint_mult + 1e-3;
    for seed in [1u64, 0xDEAD_BEEF, 0x00C0_FFEE] {
        let run = |mut rng: u64| {
            let mut next = move || {
                rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                (rng >> 33) as u32
            };
            // Start mid-gym so the stream wanders through real stations.
            let mut state = standing_on(&store, &cfg, 120.0, 60.0);
            let mut intent = MoveIntent::IDLE;
            for i in 0..1500 {
                if i % 7 == 0 {
                    let dirs: [[f32; 2]; 8] = [
                        [1.0, 0.0], [-1.0, 0.0], [0.0, 1.0], [0.0, -1.0],
                        [1.0, 1.0], [1.0, -1.0], [-1.0, 1.0], [-1.0, -1.0],
                    ];
                    let r = next();
                    intent = walk(dirs[(r % 8) as usize], r % 3 == 0, r % 11 == 0);
                } else {
                    intent.jump = false;
                }
                state = step(&cfg, &state, &intent, &store);
                assert!(state.pos.is_finite() && state.vel.is_finite(), "seed {seed} step {i}");
                assert!(state.pos.z > -50.0 && state.pos.z < 100.0, "seed {seed} step {i}: z {}", state.pos.z);
                let h = Vec3::new(state.vel.x, state.vel.y, 0.0).length();
                assert!(h <= max_speed, "seed {seed} step {i}: speed {h}");
            }
            (state.pos, state.vel, state.yaw)
        };
        assert_eq!(run(seed), run(seed), "seed {seed} not deterministic");
    }
}

/// Records the full parity battery onto `src/motion/traces/`. Each script is
/// anchored to gym stations (`tools/greybox_gen/src/gym.rs`, plateau ground
/// z = 9) and sanity-checked so a silently-degenerate recording can't be
/// committed. Rerun after any controller or greybox content change:
/// `cargo test -p game_shared record_parity_traces -- --ignored`
#[test]
#[ignore = "writes src/motion/traces/*.ron"]
fn record_parity_traces() {
    let store = greybox_store();
    let cfg = MotionConfig::default();
    let rest_z = |ground: f32| ground + cfg.capsule_half_seg + cfg.capsule_radius + cfg.skin;
    let save = |trace: &MotionTrace| {
        let text = ron::ser::to_string_pretty(trace, Default::default()).unwrap();
        std::fs::write(traces_dir().join(format!("{}.ron", trace.name)), text).unwrap();
    };

    // Slope up/down: 30° ramp (foot x=88, y=96); up, pause, back down.
    let t = build_trace("slope_up_down", &store, standing_on(&store, &cfg, 85.0, 96.0), |i, _| {
        match i {
            0..=24 => Some(walk([1.0, 0.0], false, false)),
            25..=29 => Some(MoveIntent::IDLE),
            30..=59 => Some(walk([-1.0, 0.0], false, false)),
            _ => None,
        }
    });
    let peak = t.expected.iter().map(|p| p[2]).fold(f32::MIN, f32::max);
    assert!(peak > rest_z(9.0) + 1.0, "never climbed the 30° ramp (peak {peak})");
    save(&t);

    // Over-limit slope: 60° ramp (foot x=168) refuses to be climbed.
    let t = build_trace("slope_blocked", &store, standing_on(&store, &cfg, 165.0, 96.0), |i, _| {
        (i < 40).then_some(walk([1.0, 0.0], false, false))
    });
    let end = t.expected.last().unwrap();
    assert!(end[2] < rest_z(9.0) + 0.4, "climbed a 60° slope (z {})", end[2]);
    save(&t);

    // Step-up: 0.3 m riser array (x0=108, y=72): up 5, across, down 5.
    let t = build_trace("step_up", &store, standing_on(&store, &cfg, 106.0, 72.0), {
        let mut f = for_steps(90, walk([1.0, 0.0], false, false));
        move |i, s| f(i, s)
    });
    let peak = t.expected.iter().map(|p| p[2]).fold(f32::MIN, f32::max);
    assert!(peak > rest_z(9.0) + 1.3, "never reached the step-array top (peak {peak})");
    assert!(t.expected.last().unwrap()[0] > 119.0, "did not cross the step array");
    save(&t);

    // Over-limit riser: 0.6 m (x0=168) blocks at the first face (x=168).
    // (The 0.45 array is climbable: the capsule's rounded bottom rolls over
    // risers up to roughly `step_height + radius` — a property both sides
    // share, so parity holds; the trace pins the genuinely-blocking case.)
    let t = build_trace("step_blocked", &store, standing_on(&store, &cfg, 166.0, 72.0), {
        let mut f = for_steps(30, walk([1.0, 0.0], false, false));
        move |i, s| f(i, s)
    });
    let end = t.expected.last().unwrap();
    assert!(end[0] < 167.8 && end[2] < rest_z(9.0) + 0.1, "stepped a 0.6 riser ({end:?})");
    save(&t);

    // Jump arc over the 1 m and 2 m gap-gauntlet gaps (platform tops z=10,
    // east edges x=92 and 97); jump when the live state nears an edge.
    let t = build_trace("jump_gap", &store, standing_on(&store, &cfg, 89.5, 48.0), |i, s| {
        if i >= 120 || (s.pos.x > 100.5 && s.grounded) {
            return None;
        }
        let near_edge = [92.0f32, 97.0]
            .iter()
            .any(|e| s.pos.x < *e && e - s.pos.x < 0.45);
        Some(walk([1.0, 0.0], false, s.grounded && near_edge))
    });
    let end = t.expected.last().unwrap();
    assert!(end[0] > 99.0 && (end[2] - rest_z(10.0)).abs() < 0.1, "missed a gap landing ({end:?})");
    save(&t);

    // Fall + land: walk off the tower top floor (z=18) onto the drop pad
    // (top z=9.25) — an ~8.75 m fall.
    let t = build_trace("fall_land", &store, standing_on(&store, &cfg, 96.0, 16.0), |i, _| {
        match i {
            0..=37 => Some(walk([0.0, 1.0], false, false)),
            38..=94 => Some(MoveIntent::IDLE),
            _ => None,
        }
    });
    let end = t.expected.last().unwrap();
    assert!((end[2] - rest_z(9.25)).abs() < 0.1, "did not land on the drop pad ({end:?})");
    save(&t);

    // Wall slide: run diagonally into the 16 m gym wall (y≈12.25 north
    // face), sliding along +X.
    let t = build_trace("wall_slide", &store, standing_on(&store, &cfg, 166.0, 14.5), {
        let mut f = for_steps(60, walk([1.0, -1.0], false, false));
        move |i, s| f(i, s)
    });
    let end = t.expected.last().unwrap();
    assert!(end[0] > 169.0, "did not slide along the wall ({end:?})");
    assert!(end[1] > 12.4 && end[1] < 13.5, "clipped through the wall ({end:?})");
    save(&t);

    // Chunk seam (named failure mode): the walled corridor across the cell
    // border x=128, over the 0.35 m threshold step straddling it.
    let t = build_trace("chunk_seam", &store, standing_on(&store, &cfg, 122.0, 32.0), {
        let mut f = for_steps(80, walk([1.0, 0.0], false, false));
        move |i, s| f(i, s)
    });
    let peak = t.expected.iter().map(|p| p[2]).fold(f32::MIN, f32::max);
    assert!(t.expected.last().unwrap()[0] > 134.0, "did not cross the seam corridor");
    assert!(peak > rest_z(9.0) + 0.25, "never mounted the threshold step (peak {peak})");
    save(&t);

    // 60 s long-run on the spawn plateau (ground z=8): 24 paired
    // out-and-back blocks of 50 steps, alternating axis, sprint on odd
    // pairs, one jump per block — bounds terminal drift natively so the
    // WASM replay bounds native/WASM divergence over a long horizon.
    let t = build_trace("long_run", &store, standing_on(&store, &cfg, 0.0, 0.0), |i, _| {
        if i >= 1200 {
            return None;
        }
        let block = i / 50;
        let pair = block / 2;
        let sign = if block % 2 == 0 { 1.0 } else { -1.0 };
        let dir = if pair % 2 == 0 { [sign, 0.0] } else { [0.0, sign] };
        Some(walk(dir, pair % 2 == 1, i % 50 == 25))
    });
    let end = t.expected.last().unwrap();
    assert!(end[0].abs() < 40.0 && end[1].abs() < 40.0, "long_run left the plateau ({end:?})");
    assert!((end[2] - rest_z(8.0)).abs() < 0.2, "long_run ended airborne ({end:?})");
    save(&t);
}
