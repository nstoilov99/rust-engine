//! The ECS binding: component, caches, and the system that ticks machines
//! into skeletons.
//!
//! Mirrors the script runtime's shape deliberately (Task 45-A `runner.rs`):
//! a serialized config component ([`AnimGraphRunner`]), a never-serialized
//! runtime component ([`AnimGraphRuntime`]) created lazily on the first tick,
//! a compiled-plan cache resource whose generation bump restarts live
//! instances, and a loader trait so tests hand over in-memory documents.
//! Same seams, same lifecycle rules — one pattern to learn, not two.

use std::collections::BTreeMap;
use std::sync::Arc;

use node_graph_types::GraphDoc;
use serde::{Deserialize, Serialize};

use crate::engine::animation::components::SkeletonInstance;
use crate::engine::assets::model_loader::{BoneData, RawAnimationClip};
use crate::engine::ecs::components::{MeshRenderer, Transform};
use crate::engine::ecs::hierarchy::TransformCache;
use crate::engine::ecs::resources::{Resources, Time};
use crate::engine::ecs::schedule::System;
use crate::engine::math::Frustum;
use crate::engine::scripting::normalize_graph_path;

use crate::engine::animation::blend_space::{parse_blend_space, BlendSpace};

use super::machine::{
    collect_anim_events, evaluate_pose, AnimEventFire, AnimMachine, AnimParams, PlayOnceSlot,
    PoseScratch,
};
use crate::engine::animation::ik;

use super::plan::{
    compile_anim_graph_with, upgrade_any_state, AnimGraphLoader, AnimGraphPlan, PlanClip,
    PlanIkSolver, PlanTree, PoseSource,
};

// ---------------------------------------------------------------------------
// Components
// ---------------------------------------------------------------------------

/// Attaches a `.animgraph` asset to an entity. Serialized config only — the
/// running machine lives in [`AnimGraphRuntime`], created by the system on
/// its first tick and never written to a scene.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnimGraphRunner {
    /// Content-relative path with forward slashes, e.g.
    /// `graphs/defeated.animgraph`.
    pub graph: String,
    /// Off means "attached but not running" — the reference survives, the
    /// machine does not.
    pub enabled: bool,
}

impl Default for AnimGraphRunner {
    fn default() -> Self {
        Self {
            graph: String::new(),
            enabled: true,
        }
    }
}

impl AnimGraphRunner {
    pub fn new(graph: &str) -> Self {
        Self {
            graph: normalize_graph_path(graph),
            enabled: true,
        }
    }

    /// An empty path is a component not yet pointed anywhere — a normal
    /// intermediate state, not an error.
    pub fn is_runnable(&self) -> bool {
        self.enabled && !self.graph.trim().is_empty()
    }
}

// ---------------------------------------------------------------------------
// IK targets (Task 41.5 P5, I-D3)
// ---------------------------------------------------------------------------

/// One chain's IK goals, in **world Z-up** game space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IkTarget {
    /// Where the effector should land (two-bone: the tip; look-at: the point
    /// to aim at).
    pub effector: glam::Vec3,
    /// Bend-plane disambiguator for the two-bone solver; ignored by look-at.
    pub pole: glam::Vec3,
}

/// Per-chain IK goals, written by gameplay (P6's foot placement, a look-at
/// controller, tests) and read by the runner's serial target-resolution
/// pass. Keyed by chain name — the IK Chain node's title. An entry for a
/// name the graph does not declare is ignored; a declared chain with no
/// entry simply does not solve this frame.
///
/// Positions are **world Z-up**; the runner converts them into the mesh's
/// Y-up model space each frame through the entity's render matrix (I-D1,
/// previous-frame `TransformCache` latency accepted).
#[derive(Debug, Clone, Default)]
pub struct IkTargets {
    pub targets: BTreeMap<String, IkTarget>,
}

impl IkTargets {
    /// Upsert one chain's goals (world Z-up).
    pub fn set(&mut self, chain: &str, effector: glam::Vec3, pole: glam::Vec3) {
        self.targets
            .insert(chain.to_string(), IkTarget { effector, pole });
    }
}

/// A frame's resolved targets for one chain, in the mesh's **Y-up model
/// space** — what the solvers consume directly.
#[derive(Debug, Clone, Copy)]
pub struct ResolvedIkTarget {
    pub target: glam::Vec3,
    pub pole: glam::Vec3,
}

/// A ground contact as foot placement wrote it: the target (world Z-up) and
/// the raw contact height the pelvis drop measures against.
#[derive(Debug, Clone, Copy)]
pub struct HeldContact {
    pub target: IkTarget,
    pub contact_z: f32,
}

/// One foot chain's placement config + lock state (Task 41.5 P6, I-D4),
/// armed from [`super::plan::PlanFootPlacement`]. Lock edges come from anim
/// event name conventions: `<chain>_down` latches the current contact until
/// `<chain>_up` releases it (`FootPlacementSystem` reads last tick's fires).
#[derive(Debug, Clone)]
pub struct FootState {
    /// Effector lift along the ground-hit normal (the foot bone sits at
    /// ankle height, not on the sole).
    pub ankle_offset: f32,
    pub locked: bool,
    /// The latched contact while locked.
    pub held: Option<HeldContact>,
}

/// The cosmetic pelvis drop (P6, I-D4). `offset` (world Z, ≤ 0) is smoothed
/// by `FootPlacementSystem` toward the lowest foot contact below the
/// entity's ground plane; `model_offset` is the same drop converted into
/// the mesh's Y-up model space (through the entity render matrix), which is
/// what the IK stage applies to the pelvis bone before the leg chains
/// solve. Cosmetic only — the entity/collider never move from animation
/// (non-goal: root motion).
#[derive(Debug, Clone, Copy)]
pub struct PelvisState {
    /// Index into `SkeletonInstance::bones`, resolved at arm time.
    pub bone: usize,
    /// Smoothed world-Z drop, ≤ 0.
    pub offset: f32,
    /// `offset` as a model-space vector — `apply_ik`'s input.
    pub model_offset: glam::Vec3,
}

/// A plan IK chain armed against one entity's skeleton: bone names resolved
/// to indices (arm time refuses on a missing bone), plus the per-frame
/// resolved targets the serial pre-pass writes.
pub struct ArmedIkChain {
    pub name: String,
    /// Indices into `SkeletonInstance::bones`, root→tip.
    pub bones: Vec<usize>,
    pub solver: PlanIkSolver,
    pub weight_param: String,
    /// This frame's model-space targets — written by the serial resolution
    /// pass (it needs `TransformCache` from `Resources`, which the parallel
    /// section must not touch), read inside `tick_entity`. `None` = no
    /// `IkTargets` entry for this chain, so it does not solve.
    pub resolved: Option<ResolvedIkTarget>,
    /// Foot-placement config + lock state (P6); `None` = a plain chain.
    pub foot: Option<FootState>,
    /// The *animated* (pre-IK) model-space tip position recorded by the last
    /// evaluation (`apply_ik`, two-bone chains only). Foot placement rays
    /// down from this pose and measures the pelvis drop against it — it is
    /// IK-free, so the solve never feeds back into its own inputs.
    pub animated_tip: Option<glam::Vec3>,
}

// ---------------------------------------------------------------------------
// Update-rate throttling (Task 41.5 P4, S-D4)
// ---------------------------------------------------------------------------

/// The camera's view of the world for significance bucketing, in **Y-up
/// render space** (the same camera `prepare_mesh_data` culls with). Written
/// by the host every frame before the schedule runs — it carries the
/// *previous* frame's camera, the same one-frame latency the render
/// transform path accepts. Absent ⇒ no throttling: every machine evaluates
/// its pose every frame (tests, tools, hosts that never insert it).
///
/// Shadow-frustum note (v1): the directional light's VP is computed at
/// packet-build time (`prepare_light_data`), *after* this system ran, and
/// the shadow pass draws every caster unculled — there is no shadow frustum
/// to test against here. So the significance inputs are camera distance +
/// camera-frustum visibility, and an off-camera entity never freezes: it
/// clamps to the slowest bucket so its shadow keeps moving.
pub struct AnimViewInfo {
    pub camera_pos: glam::Vec3,
    pub frustum: Frustum,
}

/// Pose-eval interval (frames) per significance bucket, nearest first.
/// Tuning constants for S-D4 live here, in one place.
const BUCKET_INTERVALS: [u32; 4] = [1, 2, 4, 8];
/// Camera-distance upper bound (render-space units ≈ meters) of buckets
/// 0..N-1; beyond the last bound is the slowest bucket.
const BUCKET_MAX_DISTANCE: [f32; 3] = [15.0, 35.0, 70.0];
/// A bucket boundary must be overshot by this factor before an entity moves
/// buckets — hysteresis, so oscillating on a boundary never flips.
const BUCKET_HYSTERESIS: f32 = 1.15;
/// Radius of the visibility test sphere: pads the frustum so characters
/// half-off the screen edge still count as visible.
const VIS_RADIUS: f32 = 2.0;
/// Eval interval for entities outside the camera frustum (see
/// [`AnimViewInfo`]'s shadow note — off-screen means slow, never frozen).
const OFFSCREEN_INTERVAL: u32 = 8;

/// The bucket for `distance`, given the entity currently sits in `current`.
/// Movement in either direction requires crossing the boundary by
/// [`BUCKET_HYSTERESIS`]: outward needs `> bound × H`, inward needs
/// `< bound ÷ H` — inside the band, the entity stays where it is.
fn significance_bucket(distance: f32, current: u8) -> u8 {
    let mut b = (current as usize).min(BUCKET_INTERVALS.len() - 1);
    while b < BUCKET_MAX_DISTANCE.len() && distance > BUCKET_MAX_DISTANCE[b] * BUCKET_HYSTERESIS {
        b += 1;
    }
    while b > 0 && distance < BUCKET_MAX_DISTANCE[b - 1] / BUCKET_HYSTERESIS {
        b -= 1;
    }
    b as u8
}

/// Per-entity throttle state, living on [`AnimGraphRuntime`]. The serial
/// significance pre-pass writes it each frame (it needs the camera from
/// `Resources`, which the parallel section must not touch); `tick_entity`
/// only reads `eval_this_frame` — its own tick-local forces (event fired,
/// crossfade, play-once) override a skip locally.
#[derive(Debug, Clone)]
pub struct ThrottleState {
    /// Significance bucket, an index into [`BUCKET_INTERVALS`] (0 = nearest).
    pub bucket: u8,
    /// Pose evaluation is due this frame (bucket interval + entity stagger,
    /// or a serial-side force).
    pub eval_this_frame: bool,
    /// Entity was inside the camera frustum last frame — the
    /// first-visible-frame force fires on the `false → true` edge.
    pub was_visible: bool,
    /// Never evaluated under this runtime yet: the first frame after arming
    /// always evaluates (consumed by the pre-pass).
    pub pending_first_eval: bool,
    /// P5/P6 IK hook: an external system sets this to force one full
    /// evaluation; the pre-pass consumes it. `FootPlacementSystem` sets it
    /// on foot lock/unlock edges (and when leaving the top bucket clears
    /// its targets) — always from serial code, never the parallel section.
    pub force_eval_external: bool,
}

impl Default for ThrottleState {
    fn default() -> Self {
        Self {
            bucket: 0,
            eval_this_frame: true,
            was_visible: false,
            pending_first_eval: true,
            force_eval_external: false,
        }
    }
}

/// The runtime half of a running graph: compiled plan, machine state and the
/// parameter blackboard gameplay writes to. **Never serialized** — its
/// absence is what tells the system to arm a fresh machine.
pub struct AnimGraphRuntime {
    /// The asset this was compiled from (stale-detection against the runner).
    pub graph: String,
    pub plan: Arc<AnimGraphPlan>,
    pub machine: AnimMachine,
    /// The play-once override channel (started through Trigger parameters).
    pub slot: PlayOnceSlot,
    /// Gameplay's write surface (ADR 0002): parameters in, never states.
    pub params: AnimParams,
    /// The anim events this frame's playback crossed — refilled every tick
    /// (one frame's worth, never accumulated). Gameplay's read surface:
    /// systems after this one see the fires that match the pose on screen.
    pub events: Vec<AnimEventFire>,
    /// The cache generation this was compiled at; a mismatch means the asset
    /// changed and the instance must restart.
    pub generation: u64,
    /// Set when this instance refused to run (compile error, missing clip,
    /// no skeleton). Reported once at arm time, then remembered.
    pub disabled: Option<String>,
    /// Update-rate throttle state (Task 41.5 P4).
    pub throttle: ThrottleState,
    /// Armed IK chains (Task 41.5 P5): the plan's chains with bone names
    /// resolved to this skeleton's indices. Empty when the plan has none.
    pub ik: Vec<ArmedIkChain>,
    /// Pelvis adjust (P6): armed when a foot chain names a pelvis bone.
    /// `FootPlacementSystem` writes the per-frame offsets; `apply_ik`
    /// consumes the model-space vector before the leg chains solve.
    pub pelvis: Option<PelvisState>,
    /// Scratch for the IK descendant re-walk (sized to the bone count on
    /// first use, reused every frame — no steady-state allocation).
    pub ik_touched: Vec<bool>,
}

// ---------------------------------------------------------------------------
// Caches and loading
// ---------------------------------------------------------------------------

/// The clips of one `.anim` container: `names` is the bone-name table the
/// channels' `bone_index` refers to. Import writes `.mesh` and `.anim` from
/// one model, so the indices line up with the sibling mesh's bones — the
/// same assumption the asset-browser thumbnails make.
#[derive(Clone)]
pub struct ClipSet {
    pub bone_names: Vec<String>,
    pub clips: Vec<RawAnimationClip>,
}

impl ClipSet {
    /// The clip a state names: by name if it says one, else the first.
    pub fn select(&self, name: Option<&str>) -> Option<&RawAnimationClip> {
        match name {
            Some(n) => self.clips.iter().find(|c| c.name == n),
            None => self.clips.first(),
        }
    }
}

/// How the system gets assets. An indirection because the engine loads from
/// disk while tests hand over maps — the same seam `GraphLoader` cuts for
/// script graphs.
pub trait AnimAssetLoader {
    fn load_graph(&self, content_rel: &str) -> Option<GraphDoc>;
    fn load_clips(&self, content_rel: &str) -> Option<ClipSet>;
    /// The bone hierarchy of a `.mesh`, for arming an entity that has a
    /// skinned mesh but no `SkeletonInstance` yet.
    fn load_skeleton(&self, mesh_content_rel: &str) -> Option<Vec<BoneData>>;
    /// The RON text of a `.blendspace`; `None` when the file does not exist.
    /// Parsing and compiling happen in [`compile_blend_space`], so a broken
    /// file refuses with its reason rather than reading as missing.
    fn load_blend_space(&self, content_rel: &str) -> Option<String> {
        let _ = content_rel;
        None
    }
}

/// Parse + compile a `.blendspace` through `loader` — the one path both the
/// runner's cache and the editor's disk loader take. `None` = no such file.
pub fn compile_blend_space(
    loader: &dyn AnimAssetLoader,
    content_rel: &str,
) -> Option<Result<Arc<BlendSpace>, String>> {
    let text = loader.load_blend_space(content_rel)?;
    Some(parse_blend_space(&text).and_then(|doc| BlendSpace::compile(&doc)).map(Arc::new))
}

/// Loads from the content root on disk.
pub struct DiskAnimAssets {
    pub content_root: std::path::PathBuf,
}

impl AnimAssetLoader for DiskAnimAssets {
    fn load_graph(&self, content_rel: &str) -> Option<GraphDoc> {
        node_graph_types::load_graph(&self.content_root.join(content_rel)).ok()
    }

    fn load_blend_space(&self, content_rel: &str) -> Option<String> {
        std::fs::read_to_string(self.content_root.join(content_rel)).ok()
    }

    fn load_clips(&self, content_rel: &str) -> Option<ClipSet> {
        let (bone_names, clips) =
            crate::engine::assets::mesh_import::load_anim_binary(&self.content_root.join(content_rel))
                .ok()?;
        Some(ClipSet { bone_names, clips })
    }

    fn load_skeleton(&self, mesh_content_rel: &str) -> Option<Vec<BoneData>> {
        // Loads the whole model to get at its bones. Heavier than a dedicated
        // bone reader, but arming is an author action, not a per-frame cost.
        crate::engine::assets::mesh_import::load_mesh_binary(
            &self.content_root.join(mesh_content_rel),
        )
        .ok()
        .map(|m| m.bones)
    }
}

/// The disk loader doubles as the compiler's resolver (the editor's anchored
/// refusals compile against the files on disk, uncached — an author action).
impl AnimGraphLoader for DiskAnimAssets {
    fn graph(&self, content_rel: &str) -> Option<GraphDoc> {
        let mut doc = self.load_graph(content_rel)?;
        upgrade_any_state(&mut doc);
        Some(doc)
    }

    fn blend_space(&self, content_rel: &str) -> Option<Result<Arc<BlendSpace>, String>> {
        compile_blend_space(self, content_rel)
    }
}

/// Compiled `.blendspace` assets, keyed by content-relative path — so the
/// triangulation is built once however many plans (or entities) reference
/// the file. Failures are cached like plans: a broken space refuses the same
/// way every compile until its file changes. A `Resource`.
#[derive(Default)]
pub struct BlendSpaceCache {
    spaces: BTreeMap<String, Result<Arc<BlendSpace>, String>>,
}

impl BlendSpaceCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.spaces.len()
    }

    pub fn is_empty(&self) -> bool {
        self.spaces.is_empty()
    }

    pub fn peek(&self, content_rel: &str) -> Option<Result<Arc<BlendSpace>, String>> {
        self.spaces.get(&normalize_graph_path(content_rel)).cloned()
    }

    /// The compiled space, loading through `loader` on a miss. `None` = the
    /// file does not exist (not cached: it may appear).
    pub fn get_or_load(
        &mut self,
        content_rel: &str,
        loader: &dyn AnimAssetLoader,
    ) -> Option<Result<Arc<BlendSpace>, String>> {
        let key = normalize_graph_path(content_rel);
        if let Some(hit) = self.spaces.get(&key) {
            return Some(hit.clone());
        }
        let compiled = compile_blend_space(loader, &key)?;
        self.spaces.insert(key, compiled.clone());
        Some(compiled)
    }

    /// Forget one file (its next use reloads from the loader).
    pub fn invalidate(&mut self, content_rel: &str) {
        self.spaces.remove(&normalize_graph_path(content_rel));
    }

    pub fn invalidate_all(&mut self) {
        self.spaces.clear();
    }
}

/// A `.blendspace` changed (editor save or external write): drop its compiled
/// form and every animation plan — a state compiles the space *into* its
/// plan, and the plan cache does not track that reference (the same
/// wholesale rule `.animgraph` nesting follows). The host calls this from
/// both the save path and the file watcher.
pub fn invalidate_blend_space(resources: &mut Resources, content_rel: &str) {
    if let Some(spaces) = resources.get_mut::<BlendSpaceCache>() {
        spaces.invalidate(content_rel);
    }
    if let Some(plans) = resources.get_mut::<AnimGraphPlanCache>() {
        plans.invalidate_all();
    }
}

/// The compiler's resolver at arm time: graphs straight from the assets,
/// blend spaces through the cache (when the host registered one).
struct ArmLoader<'a> {
    assets: &'a dyn AnimAssetLoader,
    spaces: std::cell::RefCell<Option<&'a mut BlendSpaceCache>>,
}

impl AnimGraphLoader for ArmLoader<'_> {
    fn graph(&self, content_rel: &str) -> Option<GraphDoc> {
        let mut doc = self.assets.load_graph(content_rel)?;
        upgrade_any_state(&mut doc);
        Some(doc)
    }

    fn blend_space(&self, content_rel: &str) -> Option<Result<Arc<BlendSpace>, String>> {
        match self.spaces.borrow_mut().as_deref_mut() {
            Some(cache) => cache.get_or_load(content_rel, self.assets),
            None => compile_blend_space(self.assets, content_rel),
        }
    }
}

/// Compiled plans, keyed by content-relative asset path. A `Resource`; two
/// entities running one graph share the compilation.
///
/// Invalidation is wholesale with a generation bump, exactly like
/// [`crate::engine::scripting::GraphPlanCache`]: nested `.animgraph` states
/// (ticket 09) compile the referenced documents *into* the host's plan, and
/// the cache does not track that reference tree — dropping everything on any
/// `.animgraph` write is what keeps a host plan from outliving an edit to a
/// graph it nests. Compilation is cheap and invalidation is an author action.
#[derive(Default)]
pub struct AnimGraphPlanCache {
    plans: BTreeMap<String, Result<Arc<AnimGraphPlan>, String>>,
    generation: u64,
}

impl AnimGraphPlanCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn len(&self) -> usize {
        self.plans.len()
    }

    pub fn is_empty(&self) -> bool {
        self.plans.is_empty()
    }

    /// Drop every compilation because `content_rel` changed; live machines
    /// restart on their next tick via the generation bump.
    pub fn invalidate(&mut self, content_rel: &str) {
        let _ = content_rel;
        self.invalidate_all();
    }

    pub fn invalidate_all(&mut self) {
        self.plans.clear();
        self.generation = self.generation.wrapping_add(1);
    }

    pub fn peek(&self, content_rel: &str) -> Option<Result<Arc<AnimGraphPlan>, String>> {
        self.plans.get(&normalize_graph_path(content_rel)).cloned()
    }

    /// Record a compilation. Failures are cached too — a broken graph must
    /// not recompile every frame just to fail the same way.
    pub fn store(&mut self, content_rel: &str, result: Result<Arc<AnimGraphPlan>, String>) {
        self.plans.insert(normalize_graph_path(content_rel), result);
    }
}

/// Loaded `.anim` containers, keyed by content-relative path — loaded at arm
/// time, sampled every frame. One copy per file however many machines play
/// it.
#[derive(Default)]
pub struct AnimClipCache {
    sets: BTreeMap<String, Arc<ClipSet>>,
}

impl AnimClipCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, content_rel: &str) -> Option<&ClipSet> {
        self.sets.get(&normalize_graph_path(content_rel)).map(|s| s.as_ref())
    }

    pub fn insert(&mut self, content_rel: &str, set: ClipSet) {
        self.sets.insert(normalize_graph_path(content_rel), Arc::new(set));
    }

    pub fn invalidate(&mut self, content_rel: &str) {
        self.sets.remove(&normalize_graph_path(content_rel));
    }

    /// Load what `paths` names and is not already held. Failures are silent
    /// here — arming reports a missing clip against the state that named it.
    pub fn prefetch(&mut self, paths: &[&str], loader: &dyn AnimAssetLoader) {
        for p in paths {
            let key = normalize_graph_path(p);
            if self.sets.contains_key(&key) {
                continue;
            }
            if let Some(set) = loader.load_clips(&key) {
                self.sets.insert(key, Arc::new(set));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The system
// ---------------------------------------------------------------------------

/// Ticks every enabled [`AnimGraphRunner`]'s machine and writes the resulting
/// Pose into the entity's `SkeletonInstance` (palette included).
///
/// Runs unconditionally, like `AnimationUpdateSystem`: an armed machine plays
/// its entry state in the editor viewport too, which is how an author judges
/// a graph without entering play mode.
///
/// Like the script runner, this system performs structural work the
/// descriptor cannot declare (inserting `SkeletonInstance` /
/// `AnimGraphRuntime` at arm time); the sequential executor makes that safe
/// today, and the same Task 58 prerequisite recorded for
/// `GraphScriptRunnerSystem` applies here.
pub struct AnimGraphSystem {
    loader: Box<dyn AnimAssetLoader + Send + Sync>,
    /// True only while the parallel evaluation section (step 3) runs. `arm`
    /// debug-asserts against it so structural mutation can never migrate into
    /// the parallel region unnoticed (plan §7 risk 2).
    evaluating: std::sync::atomic::AtomicBool,
    /// Lifecycle scratch reused across frames: cleared each run, never
    /// reallocated at steady state (both stay empty once the scene settles).
    dead: Vec<hecs::Entity>,
    needs_runtime: Vec<(hecs::Entity, String)>,
    /// Frames this system has run — the clock the per-bucket stagger phases
    /// against (see the significance pre-pass in `run`).
    frame: u64,
    /// Pose evaluations skipped by throttling in the last run (bench read
    /// surface; written from the parallel section).
    skipped: std::sync::atomic::AtomicU32,
}

/// Entities per batch handed to a rayon worker in step 3. Small enough that
/// a few hundred characters split across workers (300 → ~5 batches), large
/// enough to amortize `par_bridge`'s per-item handoff.
const EVAL_BATCH: u32 = 64;

thread_local! {
    /// Per-thread blend scratch for the parallel section. Rayon's workers
    /// (and the calling thread) live across frames, so each thread's scratch
    /// warms up once and steady-state evaluation allocates nothing.
    static EVAL_SCRATCH: std::cell::RefCell<PoseScratch> =
        std::cell::RefCell::new(PoseScratch::new());
}

impl AnimGraphSystem {
    pub fn new(loader: Box<dyn AnimAssetLoader + Send + Sync>) -> Self {
        Self {
            loader,
            evaluating: std::sync::atomic::AtomicBool::new(false),
            dead: Vec::new(),
            needs_runtime: Vec::new(),
            frame: 0,
            skipped: std::sync::atomic::AtomicU32::new(0),
        }
    }

    /// Pose evaluations the last `run` skipped under update-rate throttling
    /// (0 whenever no [`AnimViewInfo`] resource is present). Bench surface.
    pub fn evals_skipped_last_run(&self) -> u32 {
        self.skipped.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Compile (or reuse) the plan and build a fresh runtime sitting in the
    /// entry state. Every refusal lands in `disabled` rather than panicking.
    fn arm(&self, graph: &str, generation: u64, resources: &mut Resources) -> AnimGraphRuntime {
        debug_assert!(
            !self.evaluating.load(std::sync::atomic::Ordering::Relaxed),
            "arm() must never run inside the parallel evaluation section"
        );
        let refused = |why: String| AnimGraphRuntime {
            graph: graph.to_string(),
            plan: Arc::new(AnimGraphPlan::default()),
            machine: AnimMachine::new(&AnimGraphPlan::default()),
            slot: PlayOnceSlot::new(),
            params: AnimParams::default(),
            events: Vec::new(),
            generation,
            disabled: Some(why),
            throttle: ThrottleState::default(),
            ik: Vec::new(),
            pelvis: None,
            ik_touched: Vec::new(),
        };

        // Peek, compile, store — short borrows, one at a time, the same dance
        // the script runner does against `Resources`.
        let cached = resources
            .get::<AnimGraphPlanCache>()
            .and_then(|c| c.peek(graph));
        let plan = match cached {
            Some(hit) => hit,
            None => {
                // Nested `.animgraph` and `.blendspace` references resolve
                // through the same loader (spaces via their cache); `graph`
                // seeds the compiler's cycle guard.
                let load = ArmLoader {
                    assets: &*self.loader,
                    spaces: std::cell::RefCell::new(resources.get_mut::<BlendSpaceCache>()),
                };
                let compiled = self
                    .loader
                    .load_graph(graph)
                    .ok_or_else(|| format!("'{graph}' could not be loaded"))
                    .and_then(|doc| compile_anim_graph_with(&doc, graph, &load))
                    .map(Arc::new);
                drop(load);
                if let Some(cache) = resources.get_mut::<AnimGraphPlanCache>() {
                    cache.store(graph, compiled.clone());
                }
                compiled
            }
        };
        let plan = match plan {
            Err(e) => return refused(format!("{graph}: {e}")),
            Ok(plan) => plan,
        };

        // Clips load with the plan, not per frame — and a state whose clip is
        // missing refuses here, against the state's name, instead of playing
        // a frozen pose with no explanation.
        if let Some(clips) = resources.get_mut::<AnimClipCache>() {
            clips.prefetch(&plan.clip_refs(), &*self.loader);
        }
        if let Some(clips) = resources.get::<AnimClipCache>() {
            for st in &plan.states {
                // A blend space names its samples by index, so its refusal
                // says which sample to fix.
                if let PoseSource::Tree(PlanTree::Space(sp)) = &st.source {
                    if let Some((i, (c, _))) = sp
                        .samples
                        .iter()
                        .enumerate()
                        .find(|(_, (c, _))| clip_of(clips, c).is_none())
                    {
                        return refused(format!(
                            "{graph}: state '{}': blend space sample {i} clip '{}' could not \
                             be loaded",
                            st.name, c.clip
                        ));
                    }
                }
                for c in st.source.clips() {
                    if clip_of(clips, c).is_none() {
                        return refused(format!(
                            "{graph}: state '{}': clip '{}' could not be loaded",
                            st.name, c.clip
                        ));
                    }
                }
            }
            for slot in &plan.slots {
                if clip_of(clips, &slot.clip).is_none() {
                    return refused(format!(
                        "{graph}: play-once slot '{}': clip '{}' could not be loaded",
                        slot.name, slot.clip.clip
                    ));
                }
            }
        }

        AnimGraphRuntime {
            graph: graph.to_string(),
            machine: AnimMachine::new(&plan),
            slot: PlayOnceSlot::new(),
            params: AnimParams::from_decls(&plan.parameters),
            events: Vec::new(),
            plan,
            generation,
            disabled: None,
            throttle: ThrottleState::default(),
            ik: Vec::new(),
            pelvis: None,
            ik_touched: Vec::new(),
        }
    }
}

/// Resolve a plan's IK chains against an entity's skeleton — the arm-time
/// step of I-D3. Refuses, anchored on the chain, when a bone is missing or
/// the chain does not run root→tip down one hierarchy path (the descendant
/// re-walk relies on that; twist bones *between* the named ones are fine —
/// they re-walk).
fn arm_ik_chains(
    plan: &AnimGraphPlan,
    skeleton: &SkeletonInstance,
) -> Result<(Vec<ArmedIkChain>, Option<PelvisState>), String> {
    let index_of = |name: &str| skeleton.bones.iter().position(|b| b.name == name);
    let mut out = Vec::with_capacity(plan.ik_chains.len());
    let mut pelvis: Option<PelvisState> = None;
    for chain in &plan.ik_chains {
        let mut bones = Vec::with_capacity(chain.bones.len());
        for b in &chain.bones {
            bones.push(index_of(b).ok_or_else(|| {
                format!(
                    "IK chain '{}': bone '{b}' is not in the skeleton",
                    chain.name
                )
            })?);
        }
        for w in bones.windows(2) {
            let (anc, desc) = (w[0], w[1]);
            let mut p = skeleton.bones[desc].parent_index;
            while let Some(i) = p {
                if i == anc {
                    break;
                }
                p = skeleton.bones[i].parent_index;
            }
            if p.is_none() {
                return Err(format!(
                    "IK chain '{}': bone '{}' is not a descendant of '{}' — chain bones \
                     go root\u{2192}tip down one hierarchy path",
                    chain.name, skeleton.bones[desc].name, skeleton.bones[anc].name
                ));
            }
        }
        // Foot placement (P6): the pelvis bone resolves here too — the
        // compiler already guaranteed every foot chain names the same one,
        // so the first non-empty name is *the* name.
        if let Some(f) = &chain.foot {
            if !f.pelvis_bone.is_empty() && pelvis.is_none() {
                let bone = index_of(&f.pelvis_bone).ok_or_else(|| {
                    format!(
                        "IK chain '{}': pelvis bone '{}' is not in the skeleton",
                        chain.name, f.pelvis_bone
                    )
                })?;
                pelvis = Some(PelvisState {
                    bone,
                    offset: 0.0,
                    model_offset: glam::Vec3::ZERO,
                });
            }
        }
        out.push(ArmedIkChain {
            name: chain.name.clone(),
            bones,
            solver: chain.solver,
            weight_param: chain.weight_param.clone(),
            resolved: None,
            foot: chain.foot.as_ref().map(|f| FootState {
                ankle_offset: f.ankle_offset,
                locked: false,
                held: None,
            }),
            animated_tip: None,
        });
    }
    Ok((out, pelvis))
}

/// The clip a plan reference names, out of the cache.
fn clip_of<'a>(cache: &'a AnimClipCache, c: &PlanClip) -> Option<&'a RawAnimationClip> {
    cache.get(&c.clip)?.select(c.clip_name.as_deref())
}

/// One entity's step-3 work: machine + slot tick, event collection into the
/// entity's own `events` Vec, then — throttling permitting — pose
/// evaluation, play-once overlay, palette. Runs on rayon workers — touches
/// only this entity's components plus the immutable clip cache.
///
/// The S-D4 split: machine tick, slot tick and event collection run **every
/// frame** (rules, crossfade clocks, trigger consumption and event-crossing
/// detection never miss a frame — the ruling that keeps event semantics
/// exact under throttling). Only the pose work below the gate is
/// rate-limited; a skipped frame holds the last evaluated pose (no
/// interpolation in v1). Returns whether the pose was evaluated.
fn tick_entity(
    rt: &mut AnimGraphRuntime,
    skeleton: &mut SkeletonInstance,
    clips: &AnimClipCache,
    dt: f32,
    scratch: &mut PoseScratch,
) -> bool {
    let plan = rt.plan.clone();
    let clip_for = |c: &PlanClip| clip_of(clips, c);
    // Checked before the tick too, so the frame a crossfade *completes* on
    // still evaluates (the fade is dropped inside `tick`).
    let fading_before = rt.machine.crossfade().is_some();
    rt.machine.tick(&plan, &mut rt.params, dt);
    rt.slot.tick(&plan, &mut rt.params, dt, &clip_for);
    let mut events = std::mem::take(&mut rt.events);
    collect_anim_events(&rt.machine, &rt.slot, &plan, &rt.params, clip_for, &mut events);
    rt.events = events;
    // Tick-local forced-eval sources (S-D4): an active or just-completed
    // crossfade, a transition fired this tick, an active play-once, or an
    // event fired this tick. All visible on `rt` right here — they override
    // a pre-pass "skip" without touching shared state. (The serial-side
    // forces — first visible frame, first frame after arming, the external
    // IK hook — already landed in `eval_this_frame`.)
    let force = fading_before
        || rt.machine.transition_activity()
        || rt.slot.playing().is_some()
        || !rt.events.is_empty();
    if !rt.throttle.eval_this_frame && !force {
        return false; // held pose: skeleton keeps its last palette + revision
    }
    evaluate_pose(
        &rt.machine,
        &plan,
        &rt.params,
        clip_for,
        &mut skeleton.local_transforms,
        scratch,
    );
    rt.slot.apply(&plan, &clip_for, &mut skeleton.local_transforms, scratch);
    // FK phase 1, the IK stage (Task 41.5 P5) over the retained model space,
    // then phase 2 — one palette refresh however many chains ran. Sitting
    // inside the eval gate means IK follows the same rate as the pose it
    // corrects (I-D5; P6's lock-edge `force_eval_external` hook covers the
    // frames that must not be skipped).
    skeleton.compute_model_space();
    apply_ik(rt, skeleton);
    skeleton.refresh_palette_from_model_space();
    true
}

/// The IK stage (I-D1/I-D5): for each armed chain with a resolved target
/// and a positive weight, solve in the mesh's Y-up model space, blend
/// solved vs animated by the weight parameter, write the chain bones'
/// corrected matrices and re-walk their descendants (P2 caveat: FK phase 2
/// never auto-updates them). Chains apply in plan order, each seeing the
/// previous one's result. Runs on rayon workers — everything it touches is
/// this entity's own state.
///
/// Weight-blend ruling: per edited bone on the model-space decomposition —
/// slerp rotation, lerp translation, animated scale kept ([`ik::blend_model`]).
/// Weight 0 (or a missing target) skips the chain entirely — no solve, no
/// matrix writes.
fn apply_ik(rt: &mut AnimGraphRuntime, skeleton: &mut SkeletonInstance) {
    if rt.ik.is_empty() {
        return;
    }
    let mut touched = std::mem::take(&mut rt.ik_touched);
    let SkeletonInstance {
        bones,
        local_transforms,
        model_space,
        ..
    } = skeleton;
    // P6 — record the animated (pre-IK, pre-pelvis) two-bone tips first:
    // foot placement rays down from this pose next frame and measures the
    // pelvis drop against it, so it must never contain this frame's
    // corrections (no feedback loop).
    for chain in &mut rt.ik {
        if matches!(chain.solver, PlanIkSolver::TwoBone) {
            chain.animated_tip = chain
                .bones
                .get(2)
                .filter(|&&i| i < model_space.len())
                .map(|&i| model_space[i].w_axis.truncate());
        }
    }
    // P6 — pelvis adjust: a cosmetic model-space drop on the pelvis bone,
    // faded by the strongest foot chain's weight, applied *before* the leg
    // chains solve so both feet can still reach their contacts. The entity
    // and collider never move (non-goal: root motion).
    if let Some(p) = &rt.pelvis {
        let weight = rt
            .ik
            .iter()
            .filter(|c| c.foot.is_some())
            .filter_map(|c| rt.params.get_float(&c.weight_param))
            .fold(0.0f32, f32::max)
            .min(1.0);
        let offset = p.model_offset * weight;
        if p.bone < model_space.len() && offset.length_squared() > 1e-10 {
            model_space[p.bone].w_axis += offset.extend(0.0);
            ik::rewalk_descendants(
                model_space,
                local_transforms,
                |i| bones[i].parent_index,
                &[p.bone],
                &mut touched,
            );
        }
    }
    for chain in &rt.ik {
        let Some(t) = chain.resolved else { continue };
        let weight = rt
            .params
            .get_float(&chain.weight_param)
            .unwrap_or(0.0)
            .min(1.0);
        if weight <= 0.0 {
            continue;
        }
        // A skeleton swapped under a live runtime could shrink; never index
        // out of bounds — the chain simply stops until re-arm.
        if chain.bones.iter().any(|&i| i >= model_space.len()) {
            continue;
        }
        match chain.solver {
            PlanIkSolver::TwoBone => {
                let (r, m, tip) = (chain.bones[0], chain.bones[1], chain.bones[2]);
                let (root2, mid2) = ik::solve_two_bone(
                    model_space[r],
                    model_space[m],
                    model_space[tip],
                    t.target,
                    t.pole,
                );
                model_space[r] = ik::blend_model(&model_space[r], &root2, weight);
                model_space[m] = ik::blend_model(&model_space[m], &mid2, weight);
                ik::rewalk_descendants(
                    model_space,
                    local_transforms,
                    |i| bones[i].parent_index,
                    &[r, m],
                    &mut touched,
                );
            }
            PlanIkSolver::LookAt { axis, max_angle } => {
                let b = chain.bones[0];
                let solved = ik::solve_look_at(model_space[b], t.target, axis, max_angle);
                model_space[b] = ik::blend_model(&model_space[b], &solved, weight);
                ik::rewalk_descendants(
                    model_space,
                    local_transforms,
                    |i| bones[i].parent_index,
                    &[b],
                    &mut touched,
                );
            }
        }
    }
    rt.ik_touched = touched;
}

impl System for AnimGraphSystem {
    fn run(&mut self, world: &mut hecs::World, resources: &mut Resources) {
        crate::profile_scope!("anim_graph");

        let dt = resources
            .get::<Time>()
            .map(|t| t.scaled_delta())
            .unwrap_or(0.0);
        let generation = resources
            .get::<AnimGraphPlanCache>()
            .map(|c| c.generation())
            .unwrap_or(0);

        // 1. Drop runtimes whose plan went stale (invalidation) or whose
        //    runner was disabled / re-pointed. Re-arming next tick restarts
        //    the machine at ENTRY — a stale plan never ticks again.
        self.dead.clear();
        self.dead.extend(
            world
                .query::<&AnimGraphRuntime>()
                .iter()
                .filter(|(e, rt)| {
                    rt.generation != generation
                        || world
                            .get::<&AnimGraphRunner>(*e)
                            .map(|r| !r.is_runnable() || r.graph != rt.graph)
                            .unwrap_or(true)
                })
                .map(|(e, _)| e),
        );
        for &e in &self.dead {
            let _ = world.remove_one::<AnimGraphRuntime>(e);
        }

        // 2. Arm anything runnable that has no runtime yet. (The Vec is taken
        //    out of `self` so `arm(&self)` can borrow alongside the drain.)
        let mut needs_runtime = std::mem::take(&mut self.needs_runtime);
        needs_runtime.extend(
            world
                .query::<&AnimGraphRunner>()
                .iter()
                .filter(|(e, r)| r.is_runnable() && world.get::<&AnimGraphRuntime>(*e).is_err())
                .map(|(e, r)| (e, r.graph.clone())),
        );
        for (entity, graph) in needs_runtime.drain(..) {
            let mut runtime = self.arm(&graph, generation, resources);

            // A machine needs a skeleton to pose. An entity that has a
            // skinned mesh but no instance yet gets one from the mesh's
            // bones; anything else refuses with a reason.
            if runtime.disabled.is_none() && world.get::<&SkeletonInstance>(entity).is_err() {
                let mesh_path = world
                    .get::<&MeshRenderer>(entity)
                    .map(|m| m.mesh_path.clone())
                    .unwrap_or_default();
                let bones = (!mesh_path.is_empty())
                    .then(|| self.loader.load_skeleton(&mesh_path))
                    .flatten()
                    .unwrap_or_default();
                if bones.is_empty() {
                    runtime.disabled =
                        Some(format!("{graph}: entity has no skeleton to animate"));
                } else {
                    let _ = world.insert_one(entity, SkeletonInstance::from_bones(bones));
                }
            }

            // Arm-time IK resolution (Task 41.5 P5): bone names → this
            // skeleton's indices. A missing bone refuses the whole runtime,
            // like a missing clip — anchored on the chain that named it.
            if runtime.disabled.is_none() && !runtime.plan.ik_chains.is_empty() {
                if let Ok(skel) = world.get::<&SkeletonInstance>(entity) {
                    match arm_ik_chains(&runtime.plan, &skel) {
                        Ok((chains, pelvis)) => {
                            runtime.ik = chains;
                            runtime.pelvis = pelvis;
                        }
                        Err(why) => runtime.disabled = Some(format!("{graph}: {why}")),
                    }
                }
            }

            // Arm-time refusals print once — arming only happens when there
            // is no runtime, so this cannot repeat per frame.
            if let Some(why) = &runtime.disabled {
                println!("[animgraph] {entity:?} will not animate — {why}");
            }
            let _ = world.insert_one(entity, runtime);
        }
        self.needs_runtime = needs_runtime;

        // 2.5. Significance pre-pass (S-D4): decide, per entity, whether pose
        //    evaluation is due this frame. Serial by design — it reads the
        //    camera (and transforms) from `Resources`, which the parallel
        //    section must not touch; step 3 only reads the flag this leaves
        //    behind. Without an `AnimViewInfo` resource everything evaluates
        //    every frame. Positions come from the `TransformCache` when the
        //    host maintains one (hierarchy-correct, previous frame — the
        //    accepted render-path latency), else from the entity's own
        //    `Transform` (tests).
        self.frame = self.frame.wrapping_add(1);
        let frame = self.frame;
        let view = resources.get::<AnimViewInfo>();
        let cache = resources.get::<TransformCache>();
        for (e, (rt, transform)) in world
            .query_mut::<(&mut AnimGraphRuntime, Option<&Transform>)>()
        {
            if rt.disabled.is_some() {
                continue;
            }
            let forced = std::mem::take(&mut rt.throttle.force_eval_external)
                || std::mem::take(&mut rt.throttle.pending_first_eval);
            let Some(view) = view else {
                rt.throttle.eval_this_frame = true;
                continue;
            };
            let pos = match cache {
                Some(c) => {
                    let m = c.get_render(e);
                    glam::Vec3::new(m[(0, 3)], m[(1, 3)], m[(2, 3)])
                }
                None => transform
                    .map(|t| {
                        crate::engine::utils::coords::convert_position_zup_to_yup(
                            glam::Vec3::new(t.position.x, t.position.y, t.position.z),
                        )
                    })
                    .unwrap_or(glam::Vec3::ZERO),
            };
            let visible = view.frustum.contains_sphere(pos, VIS_RADIUS);
            let first_visible = visible && !rt.throttle.was_visible;
            rt.throttle.was_visible = visible;
            let distance = pos.distance(view.camera_pos);
            rt.throttle.bucket = significance_bucket(distance, rt.throttle.bucket);
            let interval = if visible {
                BUCKET_INTERVALS[rt.throttle.bucket as usize]
            } else {
                OFFSCREEN_INTERVAL
            };
            // Entity-id stagger: members of one bucket phase out across the
            // interval instead of all evaluating on the same frame.
            let due = interval <= 1
                || frame.wrapping_add(e.to_bits().get()) % u64::from(interval) == 0;
            rt.throttle.eval_this_frame = due || forced || first_visible;
        }

        // 2.6. IK target resolution (Task 41.5 P5, I-D1): gameplay's world
        //    Z-up effector/pole become this frame's mesh-space targets —
        //    `target_model = entity_render⁻¹ * zup_to_yup(target_world)`.
        //    Serial by design, like the significance pass: `TransformCache`
        //    lives in `Resources`, which the parallel section must not
        //    touch; `tick_entity` reads only the `resolved` slots written
        //    here. `entity_render` is the previous frame's render matrix
        //    (the accepted render-path latency), with the entity's own
        //    `Transform` as the cache-less fallback (tests).
        for (e, (rt, targets, transform)) in world.query_mut::<(
            &mut AnimGraphRuntime,
            Option<&IkTargets>,
            Option<&Transform>,
        )>() {
            if rt.disabled.is_some() || rt.ik.is_empty() {
                continue;
            }
            let entity_render = match cache {
                Some(c) => glam::Mat4::from_cols_slice(c.get_render(e).as_slice()),
                None => transform
                    .map(|t| glam::Mat4::from_cols_slice(t.model_matrix().as_slice()))
                    .unwrap_or(glam::Mat4::IDENTITY),
            };
            let inv = entity_render.inverse();
            for chain in &mut rt.ik {
                chain.resolved = targets
                    .and_then(|t| t.targets.get(&chain.name))
                    .map(|t| ResolvedIkTarget {
                        target: inv.transform_point3(
                            crate::engine::utils::coords::convert_position_zup_to_yup(
                                t.effector,
                            ),
                        ),
                        pole: inv.transform_point3(
                            crate::engine::utils::coords::convert_position_zup_to_yup(t.pole),
                        ),
                    });
            }
        }

        // 3. Tick machines and write poses. Per-frame order per the spec:
        //    parameters were written by gameplay before this system ran;
        //    machine update (rules, crossfades) precedes pose evaluation.
        //
        //    This step runs in parallel (S-D3): everything it touches is
        //    per-entity state (hecs components are Send + Sync) except the
        //    clip cache, borrowed immutably — every lazy load happened in the
        //    serial arm phase above (`prefetch`). Events land on each
        //    entity's own `events` Vec, so their order is deterministic
        //    however rayon schedules the batches.
        let clips = match resources.get::<AnimClipCache>() {
            Some(c) => c,
            None => return,
        };
        use rayon::iter::{ParallelBridge, ParallelIterator};
        use std::sync::atomic::Ordering;
        let skipped = &self.skipped;
        skipped.store(0, Ordering::Relaxed);
        self.evaluating.store(true, Ordering::Relaxed);
        world
            .query_mut::<(&mut AnimGraphRuntime, &mut SkeletonInstance)>()
            .into_iter_batched(EVAL_BATCH)
            .par_bridge()
            .for_each(|batch| {
                EVAL_SCRATCH.with(|cell| {
                    let scratch = &mut *cell.borrow_mut();
                    let mut held = 0u32;
                    for (_entity, (rt, skeleton)) in batch {
                        if rt.disabled.is_some() {
                            continue;
                        }
                        if !tick_entity(rt, skeleton, clips, dt, scratch) {
                            held += 1;
                        }
                    }
                    if held > 0 {
                        skipped.fetch_add(held, Ordering::Relaxed);
                    }
                });
            });
        self.evaluating.store(false, Ordering::Relaxed);
    }

    fn name(&self) -> &str {
        crate::engine::ecs::system_names::ANIM_GRAPH
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_normalize_and_empty_runners_are_not_runnable() {
        assert_eq!(
            AnimGraphRunner::new("graphs\\defeated.animgraph").graph,
            "graphs/defeated.animgraph"
        );
        let mut r = AnimGraphRunner::new("graphs/defeated.animgraph");
        assert!(r.is_runnable());
        r.enabled = false;
        assert!(!r.is_runnable(), "disabled keeps the reference, not the machine");
        assert!(!AnimGraphRunner::default().is_runnable());
    }

    /// A `.blendspace` write drops the compiled space and every plan (the
    /// generation bump restarts live machines) — the host's save path and
    /// watcher both go through `invalidate_blend_space`.
    #[test]
    fn a_blend_space_write_invalidates_its_space_and_every_plan() {
        struct OneSpace;
        impl AnimAssetLoader for OneSpace {
            fn load_graph(&self, _: &str) -> Option<GraphDoc> {
                None
            }
            fn load_clips(&self, _: &str) -> Option<ClipSet> {
                None
            }
            fn load_skeleton(&self, _: &str) -> Option<Vec<BoneData>> {
                None
            }
            fn load_blend_space(&self, rel: &str) -> Option<String> {
                (rel == "blendspaces/loco.blendspace").then(|| {
                    "(samples: [(x: 0.0, clip: \"a.anim\")])".to_string()
                })
            }
        }
        let mut resources = Resources::new();
        resources.insert(BlendSpaceCache::new());
        resources.insert(AnimGraphPlanCache::new());
        let spaces = resources.get_mut::<BlendSpaceCache>().unwrap();
        assert!(spaces.get_or_load("blendspaces\\loco.blendspace", &OneSpace).unwrap().is_ok());
        assert!(spaces.get_or_load("blendspaces/none.blendspace", &OneSpace).is_none());
        assert_eq!(spaces.len(), 1);
        let plans = resources.get_mut::<AnimGraphPlanCache>().unwrap();
        plans.store("graphs/a.animgraph", Ok(Arc::new(AnimGraphPlan::default())));
        let g = plans.generation();

        invalidate_blend_space(&mut resources, "blendspaces/loco.blendspace");
        assert!(resources.get::<BlendSpaceCache>().unwrap().is_empty());
        let plans = resources.get::<AnimGraphPlanCache>().unwrap();
        assert!(plans.is_empty(), "plans go wholesale: they compiled the space in");
        assert_ne!(plans.generation(), g, "live machines restart on the bump");
    }

    #[test]
    fn plan_cache_invalidation_bumps_the_generation() {
        let mut cache = AnimGraphPlanCache::new();
        cache.store("graphs/a.animgraph", Ok(Arc::new(AnimGraphPlan::default())));
        assert_eq!(cache.len(), 1);
        let g = cache.generation();
        cache.invalidate("graphs/a.animgraph");
        assert!(cache.is_empty(), "invalidation is wholesale");
        assert_ne!(cache.generation(), g, "live machines restart on the bump");
    }
}
