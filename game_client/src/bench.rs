//! P0 stress scene + baseline capture (Task 41.5).
//!
//! `--stress-anim N` spawns N animated characters after world load;
//! `--bench-secs S` samples per-frame metrics for S seconds (starting after
//! the first frame), writes `.scratch/anim-scale/baseline-N.txt`, and exits.
//!
//! The render-loop hooks below are shared with the editor build but are inert
//! (one relaxed atomic load) unless a `BenchRun` armed them — zero overhead
//! when the flags are absent.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::Relaxed};

// ---------------------------------------------------------------------------
// Render-loop hooks (both builds; armed only by BenchRun::new)
// ---------------------------------------------------------------------------

static RENDER_HOOKS: AtomicBool = AtomicBool::new(false);
static PALETTE_UPLOADS: AtomicU32 = AtomicU32::new(0);
static PALETTE_NANOS: AtomicU64 = AtomicU64::new(0);
static SKINNED_DRAWS: AtomicU32 = AtomicU32::new(0);

/// Whether `prepare_mesh_data` should time palette uploads and count skinned
/// draws this frame.
pub fn render_hooks_enabled() -> bool {
    RENDER_HOOKS.load(Relaxed)
}

/// One skeleton's palette write into the SSBO ring took `nanos`.
pub fn palette_upload(nanos: u64) {
    PALETTE_UPLOADS.fetch_add(1, Relaxed);
    PALETTE_NANOS.fetch_add(nanos, Relaxed);
}

/// Skinned submesh draws submitted this frame (camera + shadow lists).
pub fn add_skinned_draws(n: u32) {
    SKINNED_DRAWS.fetch_add(n, Relaxed);
}

// ---------------------------------------------------------------------------
// Standalone-only: flag parsing, stress spawn, timed system, collector
// ---------------------------------------------------------------------------

#[cfg(not(feature = "editor"))]
static ANIM_NANOS: AtomicU64 = AtomicU64::new(0);
#[cfg(not(feature = "editor"))]
static EVAL_SKIPS: AtomicU32 = AtomicU32::new(0);

#[cfg(not(feature = "editor"))]
pub struct BenchFlags {
    pub stress_anim: usize,
    pub bench_secs: Option<f32>,
}

/// Parse `--stress-anim N` / `--bench-secs S` (both `--flag value` and
/// `--flag=value` forms). Absent or unparsable values disable the feature.
#[cfg(not(feature = "editor"))]
pub fn parse_flags(args: &[String]) -> BenchFlags {
    let value = |name: &str| -> Option<String> {
        let prefix = format!("{name}=");
        args.iter().enumerate().find_map(|(i, a)| {
            if a == name {
                args.get(i + 1).cloned()
            } else {
                a.strip_prefix(&prefix).map(str::to_string)
            }
        })
    };
    BenchFlags {
        stress_anim: value("--stress-anim")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0),
        bench_secs: value("--bench-secs").and_then(|v| v.parse().ok()),
    }
}

/// Spawn `n` characters on `graphs/character.animgraph` in a grid on the
/// ground plane (Z-up: XY). The component recipe is cloned from the scene's
/// existing animated character (Transform + MeshRenderer + AnimGraphRunner —
/// `AnimGraphSystem` arms lazily and inserts `SkeletonInstance` from the
/// mesh's bones; all instances share the compiled plan via
/// `AnimGraphPlanCache`). Falls back to the net character-rig recipe from
/// `replication.rs` if the scene has none.
#[cfg(not(feature = "editor"))]
pub fn spawn_stress_characters(world: &mut hecs::World, n: usize) {
    use rust_engine::engine::animation::graph::AnimGraphRunner;
    use rust_engine::engine::ecs::components::{MeshRenderer, Name, Transform};

    const STRESS_GRAPH: &str = "graphs/character.animgraph";

    let template = world
        .query::<(&MeshRenderer, &AnimGraphRunner)>()
        .iter()
        .next()
        .map(|(_, (m, _))| m.clone());
    if template.is_none() {
        println!("stress: no animated character in scene; using net rig recipe");
    }
    let mesh = template.unwrap_or_else(|| MeshRenderer {
        mesh_path: "Defeated.mesh".to_string(),
        material_paths: vec![
            "Defeated_Beta_HighLimbsGeoSG3.material".to_string(),
            "Defeated_Beta_Joints_MAT1.material".to_string(),
        ],
        ..Default::default()
    });

    let side = (n as f32).sqrt().ceil().max(1.0) as usize;
    let spacing = 1.5f32;
    let half = (side.saturating_sub(1)) as f32 * 0.5;
    for i in 0..n {
        let col = (i % side) as f32;
        let row = (i / side) as f32;
        // Centered on the origin, nudged +X so row 0 doesn't overlap the
        // scene's own character at (0,0,0).
        let pos = nalgebra_glm::vec3((col - half) * spacing + 2.0, (row - half) * spacing, 0.0);
        world.spawn((
            Transform::new(pos),
            mesh.clone(),
            Name::new(format!("StressAnim {i}")),
            AnimGraphRunner::new(STRESS_GRAPH),
        ));
    }
    println!("stress: spawned {n} characters on {STRESS_GRAPH} ({side}x{side} grid)");
}

/// Wraps `AnimGraphSystem` and records its wall time (and, since P4, the
/// pose evaluations update-rate throttling skipped). Registered only when
/// `--bench-secs` is present; the plain system is registered otherwise.
#[cfg(not(feature = "editor"))]
pub struct TimedAnimGraph(pub rust_engine::engine::animation::graph::AnimGraphSystem);

#[cfg(not(feature = "editor"))]
impl rust_engine::engine::ecs::schedule::System for TimedAnimGraph {
    fn run(
        &mut self,
        world: &mut hecs::World,
        resources: &mut rust_engine::engine::ecs::resources::Resources,
    ) {
        let t0 = std::time::Instant::now();
        self.0.run(world, resources);
        ANIM_NANOS.fetch_add(t0.elapsed().as_nanos() as u64, Relaxed);
        EVAL_SKIPS.fetch_add(self.0.evals_skipped_last_run(), Relaxed);
    }

    fn name(&self) -> &str {
        self.0.name()
    }
}

/// Per-frame metric collector for `--bench-secs`. Call [`end_frame`] once per
/// frame; when the window elapses it writes the baseline file and flips
/// [`finished`], which the event loop turns into a clean exit(0).
#[cfg(not(feature = "editor"))]
pub struct BenchRun {
    secs: f32,
    n: usize,
    started: Option<std::time::Instant>,
    frame_ms: Vec<f32>,
    anim_ms: Vec<f32>,
    palette_counts: Vec<f32>,
    palette_ms: Vec<f32>,
    skinned_draws: Vec<f32>,
    evals_skipped: Vec<f32>,
    finished: bool,
}

#[cfg(not(feature = "editor"))]
impl BenchRun {
    pub fn new(secs: f32, n: usize) -> Self {
        RENDER_HOOKS.store(true, Relaxed);
        println!("bench: capturing {secs}s after first frame (N={n})");
        Self {
            secs,
            n,
            started: None,
            frame_ms: Vec::with_capacity(4096),
            anim_ms: Vec::with_capacity(4096),
            palette_counts: Vec::with_capacity(4096),
            palette_ms: Vec::with_capacity(4096),
            skinned_draws: Vec::with_capacity(4096),
            evals_skipped: Vec::with_capacity(4096),
            finished: false,
        }
    }

    pub fn finished(&self) -> bool {
        self.finished
    }

    pub fn end_frame(&mut self, frame_ms: f32) {
        if self.finished {
            return;
        }
        let anim_ns = ANIM_NANOS.swap(0, Relaxed);
        let uploads = PALETTE_UPLOADS.swap(0, Relaxed);
        let palette_ns = PALETTE_NANOS.swap(0, Relaxed);
        let draws = SKINNED_DRAWS.swap(0, Relaxed);
        let skips = EVAL_SKIPS.swap(0, Relaxed);

        // First frame carries plan compile + first uploads; the bench window
        // starts after it.
        let Some(started) = self.started else {
            self.started = Some(std::time::Instant::now());
            return;
        };

        self.frame_ms.push(frame_ms);
        self.anim_ms.push(anim_ns as f32 / 1.0e6);
        self.palette_counts.push(uploads as f32);
        self.palette_ms.push(palette_ns as f32 / 1.0e6);
        self.skinned_draws.push(draws as f32);
        self.evals_skipped.push(skips as f32);

        if started.elapsed().as_secs_f32() >= self.secs {
            self.write_report(started.elapsed().as_secs_f32());
            self.finished = true;
        }
    }

    fn write_report(&self, measured_secs: f32) {
        let profile = if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        };
        let mut out = String::new();
        out.push_str(&format!(
            "P0 anim-scale baseline — N={} stress characters\n",
            self.n
        ));
        out.push_str(&format!(
            "build: {profile} | present: forced Immediate (uncapped) | window: {:.1}s requested, \
             {measured_secs:.2}s measured, {} frames (first frame excluded)\n\n",
            self.secs,
            self.frame_ms.len()
        ));
        out.push_str("metric                        avg        p95\n");
        let mut row = |name: &str, v: &[f32]| {
            let (avg, p95) = avg_p95(v);
            out.push_str(&format!("{name:<24} {avg:>10.3} {p95:>10.3}\n"));
        };
        row("frame ms", &self.frame_ms);
        row("anim system ms", &self.anim_ms);
        row("palette uploads", &self.palette_counts);
        row("palette upload ms", &self.palette_ms);
        row("skinned draws", &self.skinned_draws);
        row("pose evals skipped", &self.evals_skipped);
        out.push_str("\nskinned draws = per-submesh draws submitted (camera list + shadow list).\n");
        out.push_str(
            "pose evals skipped = pose evaluations held by update-rate throttling per frame \
             (P4; machine/slot/event ticks still ran).\n",
        );
        out.push_str(
            "palette uploads = skeleton palettes written into the SSBO ring per frame \
             (P1; pre-P1 baselines measured per-entity UBO + descriptor-set allocations).\n",
        );

        let dir = std::path::Path::new(".scratch/anim-scale");
        let path = dir.join(format!("baseline-{}.txt", self.n));
        let write = std::fs::create_dir_all(dir).and_then(|_| std::fs::write(&path, &out));
        match write {
            Ok(()) => println!("bench: wrote {}\n{out}", path.display()),
            Err(e) => eprintln!("bench: failed to write {}: {e}\n{out}", path.display()),
        }
    }
}

#[cfg(not(feature = "editor"))]
fn avg_p95(v: &[f32]) -> (f32, f32) {
    if v.is_empty() {
        return (0.0, 0.0);
    }
    let avg = v.iter().sum::<f32>() / v.len() as f32;
    let mut sorted = v.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let idx = ((sorted.len() as f32 * 0.95).ceil() as usize)
        .saturating_sub(1)
        .min(sorted.len() - 1);
    (avg, sorted[idx])
}
