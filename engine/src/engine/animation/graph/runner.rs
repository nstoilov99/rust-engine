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
use super::plan::{
    compile_anim_graph_with, upgrade_any_state, AnimGraphLoader, AnimGraphPlan, PlanClip, PlanTree,
    PoseSource,
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
    /// P5/P6 IK hook: an external system (foot lock/unlock edge) sets this
    /// to force one full evaluation; the pre-pass consumes it. Nothing in
    /// the engine sets it yet.
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
        }
    }
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
    skeleton.compute_palette();
    true
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
