//! The `.animgraph` document shape and its compiler.
//!
//! A `.animgraph` is a [`GraphDoc`] (the Task 40 container — same RON io,
//! same migration chain) whose nodes come from the animation library below
//! and whose `variables` are the graph's typed parameters. This module
//! defines the stable slugs and compiles the document into an
//! [`AnimGraphPlan`] — the immutable, index-based form the machine evaluates.
//!
//! Compilation follows the script compiler's posture: every refusal is a
//! `String` a person can act on, and a broken document never produces a
//! half-working plan.

use node_graph_types::registry::{
    REROUTE_IN, REROUTE_OUT, REROUTE_TYPE_ID, VAR_GET_TYPE_ID, VAR_PROP, VAR_VALUE_PIN,
};
use node_graph_types::std_nodes::{
    ADD_FLOAT, AND, COMPARE_FLOAT, DIV_FLOAT, MUL_FLOAT, NOT, OR, SUB_FLOAT,
};
use node_graph_types::{GraphDoc, GraphRealm, GraphRegion, NodeInst, PinType, PropValue};

use super::machine::{AnimParams, ParamValue};
use crate::engine::animation::blend_space::BlendSpace;

// ---------------------------------------------------------------------------
// Node library slugs (stable identity, per the Task 40 identity rules)
// ---------------------------------------------------------------------------

/// The ENTRY node — a real node on the canvas, exactly one per machine. Its
/// single outgoing edge names the starting state.
pub const ANIM_ENTRY_TYPE_ID: &str = "anim_entry";
/// ENTRY property (`Str`/`Asset`, optional): the content-relative `.mesh`
/// the editor's Preview panel poses this graph on. Editor-only data that
/// rides on the document's one ENTRY node (per-document layouts ruling) so
/// it is an ordinary undoable node-property edit; empty or absent means
/// "auto-pick a mesh whose bones cover the plan's clips". The compiler
/// ignores it.
pub const PREVIEW_MESH_PROP: &str = "preview_mesh";

/// The document's preview mesh — its ENTRY node's [`PREVIEW_MESH_PROP`];
/// empty when unset or when there is no ENTRY (auto-pick).
pub fn preview_mesh_of(doc: &GraphDoc) -> String {
    doc.nodes
        .iter()
        .find(|n| n.type_id == ANIM_ENTRY_TYPE_ID)
        .and_then(|n| str_prop(&n.properties, PREVIEW_MESH_PROP))
        .unwrap_or_default()
        .to_string()
}

/// A State: a leaf that plays the `.anim` clip its [`CLIP_PROP`] names, a
/// blend tree (when the document carries a region keyed by the state's id),
/// or a nested sub-state-machine (when [`GRAPH_PROP`] names another
/// `.animgraph`) evaluated as this state's Pose source. Precedence mirrors
/// the clip rule: a non-empty tree region wins over `graph`, which wins over
/// `clip` — the ignored properties keep their data but do nothing.
pub const ANIM_STATE_TYPE_ID: &str = "anim_state";
/// A Transition between two states, carrying blend duration and priority as
/// node data.
pub const ANIM_TRANSITION_TYPE_ID: &str = "anim_transition";
/// A State Alias: a named source standing for a chosen set of states (or,
/// when [`ALIAS_GLOBAL_PROP`] is true, every state). A transition leaving it
/// is an *ordinary* transition from each aliased state — same priority
/// scale, no fade-interrupt right, the source must be the current state —
/// exactly as if the author had drawn it from each of them. Its name is the
/// node title (default "Alias"); no inputs, one flow [`STATE_OUT_PIN`].
pub const ANIM_STATE_ALIAS_TYPE_ID: &str = "anim_state_alias";
/// Alias property (`Bool`, default true): stand for every state. While set,
/// [`ALIAS_STATES_PROP`] is ignored.
pub const ALIAS_GLOBAL_PROP: &str = "global";
/// Alias property (`Array` of `Int`): document node ids of the aliased
/// states. Written by the editor; the compiler dedupes and ignores
/// non-`Int`/negative entries, and refuses an id that is not a state.
pub const ALIAS_STATES_PROP: &str = "states";
/// **Legacy.** The Any State node of documents saved before state aliases:
/// [`upgrade_any_state`] matches on this id and rewrites the node to a
/// Global alias. It is not in the registry and never compiles as itself.
pub const ANIM_ANY_STATE_TYPE_ID: &str = "anim_any_state";
/// The single Bool sink inside a transition's rule region — exactly one per
/// non-empty rule. Its [`RULE_RESULT_PIN`] input unwired means always-true.
pub const ANIM_RULE_RESULT_TYPE_ID: &str = "anim_rule_result";
/// The RESULT node's one input pin.
pub const RULE_RESULT_PIN: &str = "value";

/// Blend-tree node library. A state's tree lives in an embedded region keyed
/// by the STATE node id — the same container v3 mechanism transitions use
/// for rules, so descend/duplicate/delete semantics come for free.
///
/// A clip leaf: plays the `.anim` its [`CLIP_PROP`] names ([`CLIP_NAME_PROP`]
/// selects inside the container).
pub const ANIM_CLIP_TYPE_ID: &str = "anim_clip";
/// A 1D blend (walk → run): driven by the Float parameter its
/// [`BLEND_PARAM_PROP`] names; children on [`blend_in_pin`] pins, placed on
/// the axis by [`blend_threshold_prop`] properties.
pub const ANIM_BLEND1D_TYPE_ID: &str = "anim_blend1d";
/// A 2D directional blend (8-way movement): driven by the Float parameters
/// [`BLEND_PARAM_X_PROP`]/[`BLEND_PARAM_Y_PROP`]; children on
/// [`blend_in_pin`] pins, each owning a direction ([`blend_x_prop`]/
/// [`blend_y_prop`]). Only the input's direction matters, not its magnitude.
pub const ANIM_BLEND2D_TYPE_ID: &str = "anim_blend2d";
/// The single Pose sink of a state's tree — exactly one per non-empty tree
/// region, and its input must be wired (a state must produce a pose; there
/// is no "always-true" reading here).
pub const ANIM_POSE_RESULT_TYPE_ID: &str = "anim_pose_result";
/// The Pose pin: every tree node's output, and the RESULT sink's input.
pub const POSE_PIN: &str = "pose";
/// Blend node properties: the driving parameter(s), bound **by name** (an
/// editor dropdown), not wired — tree regions stay single-typed, only Pose
/// flows.
pub const BLEND_PARAM_PROP: &str = "param";
pub const BLEND_PARAM_X_PROP: &str = "param_x";
pub const BLEND_PARAM_Y_PROP: &str = "param_y";

/// A blend node's child input pins are indexed: `in_0`, `in_1`, …
pub fn blend_in_pin(i: usize) -> String {
    format!("in_{i}")
}

/// The 1D threshold property paired with `in_i`: `threshold_0`, …
pub fn blend_threshold_prop(i: usize) -> String {
    format!("threshold_{i}")
}

/// The 2D direction properties paired with `in_i`: `x_0`/`y_0`, …
pub fn blend_x_prop(i: usize) -> String {
    format!("x_{i}")
}
pub fn blend_y_prop(i: usize) -> String {
    format!("y_{i}")
}

/// State pins: machine-topology flow in/out. (Not Pose wires — those belong
/// to blend trees inside a state.)
pub const STATE_IN_PIN: &str = "in";
pub const STATE_OUT_PIN: &str = "out";
/// Transition pins: `from` receives the source state's `out`; `to` feeds the
/// target state's `in`.
pub const TRANSITION_FROM_PIN: &str = "from";
pub const TRANSITION_TO_PIN: &str = "to";

/// State properties. `clip` is the content-relative `.anim` path (required
/// on a leaf state — a state with a tree region ignores it); `clip_name`
/// picks a clip inside the container (default: the first); `speed` is a
/// playback-rate multiplier on the state's clock (default 1.0 — 0.0 holds
/// the first frame as a pose). Both clip properties also apply to
/// [`ANIM_CLIP_TYPE_ID`] nodes inside a tree.
pub const CLIP_PROP: &str = "clip";
pub const CLIP_NAME_PROP: &str = "clip_name";
pub const SPEED_PROP: &str = "speed";
/// The content-relative `.animgraph` path a nested state references (spec
/// story 3: factor Locomotion into its own file-backed sub-state-machine).
/// The referenced document compiles into the plan as a child machine —
/// see [`PoseSource::Machine`]; a [`SPEED_PROP`] on the state scales the
/// sub-machine's clock. Editing tools treat it like `clip`: double-click
/// descends into the file instead of playing anything here.
pub const GRAPH_PROP: &str = "graph";
/// The content-relative `.blendspace` path a state plays as its Pose source
/// (Task 41.5). Compiles to [`PlanTree::Space`]; precedence sits between
/// `graph` and `clip`: tree region > `graph` > `space` > `clip`.
pub const SPACE_PROP: &str = "space";

/// Transition properties. `duration` is the crossfade length in seconds
/// (default 0.0 = instant); `priority` orders evaluation when several rules
/// pass — **lower value wins**, ties broken by node id, so resolution is
/// deterministic.
pub const DURATION_PROP: &str = "duration";
pub const PRIORITY_PROP: &str = "priority";

/// A play-once slot: v1's only override channel (CONTEXT.md). A standalone
/// node on the machine canvas — no wires — that plays the clip its
/// [`CLIP_PROP`] names over the base result when the Trigger parameter its
/// [`SLOT_TRIGGER_PROP`] names fires, then returns to the base result when
/// the clip finishes. Firing stays inside the parameter contract: gameplay
/// writes the Trigger, never the slot. [`SPEED_PROP`] applies.
pub const ANIM_PLAY_ONCE_TYPE_ID: &str = "anim_play_once";
/// The declared Trigger parameter that starts the slot (consume-on-start,
/// mirroring consume-on-transition).
pub const SLOT_TRIGGER_PROP: &str = "trigger";
/// Overlay envelope, seconds: the slot's weight ramps 0→1 over `fade_in`
/// and 1→0 over the clip's last `fade_out` seconds. Defaults 0.0 (hard cut).
pub const SLOT_FADE_IN_PROP: &str = "fade_in";
pub const SLOT_FADE_OUT_PROP: &str = "fade_out";

/// An IK chain (Task 41.5 P5, I-D3): a standalone node on the machine canvas
/// — no wires, like a play-once slot — declaring a post-pose IK pass. The
/// node title is the chain's name (default `IK <id>`), which is also the key
/// gameplay writes `IkTargets` under. Compiles to [`PlanIkChain`], **not**
/// into any `PlanTree` (tree regions stay single-typed Pose).
pub const ANIM_IK_CHAIN_TYPE_ID: &str = "anim_ik_chain";
/// IK property (`Str`, required): comma-separated bone names, root→tip.
/// The two-bone solver takes exactly 3 (root, mid, tip); look-at exactly 1.
/// Names resolve to skeleton indices at arm time.
pub const IK_BONES_PROP: &str = "bones";
/// IK property (`Enum`/`Str`): [`IK_SOLVER_TWO_BONE`] (default) or
/// [`IK_SOLVER_LOOK_AT`].
pub const IK_SOLVER_PROP: &str = "solver";
pub const IK_SOLVER_TWO_BONE: &str = "two_bone";
pub const IK_SOLVER_LOOK_AT: &str = "look_at";
/// IK property (`Str`, required): the declared **Float** parameter that
/// fades the chain — 0 skips the solve entirely (I-D5), 1 is fully solved.
/// States fade IK through the existing parameter contract.
pub const IK_WEIGHT_PARAM_PROP: &str = "weight_param";
/// Look-at properties (`Float`): the bone-local aim axis (defaults 0,0,1 —
/// mesh-space +Z) and the clamp in **degrees** (default 90; the compiled
/// plan carries radians).
pub const IK_AXIS_X_PROP: &str = "axis_x";
pub const IK_AXIS_Y_PROP: &str = "axis_y";
pub const IK_AXIS_Z_PROP: &str = "axis_z";
pub const IK_MAX_ANGLE_PROP: &str = "max_angle";
/// Foot-placement properties (Task 41.5 P6, I-D4) — two-bone chains only.
/// `foot` (`Bool`, default false) marks the chain as a foot:
/// `FootPlacementSystem` rays the ground under the chain's tip bone and
/// writes the chain's `IkTargets` entry each frame; anim events named
/// `<chain name>_down` / `<chain name>_up` drive the plant lock.
/// `ankle_offset` (`Float`, default 0.1) lifts the effector along the hit
/// normal (the foot bone sits at ankle height, not on the sole). `pelvis`
/// (`Str`) names the bone the pelvis adjust lowers — every foot chain that
/// sets it must agree; empty = no pelvis adjust.
pub const IK_FOOT_PROP: &str = "foot";
pub const IK_ANKLE_OFFSET_PROP: &str = "ankle_offset";
pub const IK_PELVIS_PROP: &str = "pelvis";

/// Trigger parameters are declared as `PinType::Domain("anim_trigger")` —
/// the pin system's consumer-owned extension point, so the shared container
/// needs no animation-specific variant. Inside a rule a trigger reads as a
/// Bool ("is it currently set"); buffering and consume-on-transition are the
/// machine's statefulness, never the rule's.
pub const TRIGGER_PARAM_DOMAIN: &str = "anim_trigger";

/// The declaration type of a Trigger parameter (what `GraphDoc::variables`
/// carries for one).
pub fn trigger_pin_type() -> PinType {
    PinType::Domain(TRIGGER_PARAM_DOMAIN.to_string())
}

// ---------------------------------------------------------------------------
// Plan (compiled form)
// ---------------------------------------------------------------------------

/// The three parameter types gameplay may declare (per CONTEXT.md). A
/// Trigger is a one-shot: set by gameplay, buffered until a transition whose
/// rule reads it fires, which consumes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimParamType {
    Float,
    Bool,
    Trigger,
}

/// One declared parameter — the typed contract between gameplay and the
/// graph.
#[derive(Debug, Clone, PartialEq)]
pub struct ParamDecl {
    pub slug: String,
    pub ty: AnimParamType,
    pub default: ParamValue,
}

/// A clip reference inside a blend tree (and what a clip-only leaf state
/// compiles to).
#[derive(Debug, Clone, PartialEq)]
pub struct PlanClip {
    /// Content-relative `.anim` path.
    pub clip: String,
    /// Clip inside the container; `None` = the first.
    pub clip_name: Option<String>,
}

/// A state's compiled pose producer — recursive tree evaluation per the
/// spec: clips at the leaves, 1D/2D blends above them. Children are sorted
/// at compile time (by threshold / by direction angle), so at most two of
/// them — the bracketing pair — are ever active at once.
#[derive(Debug, Clone, PartialEq)]
pub enum PlanTree {
    Clip(PlanClip),
    /// 1D blend: `children` sorted by threshold ascending. Outside the range
    /// the nearest endpoint plays pure.
    Blend1D {
        /// The declared Float parameter driving the blend.
        param: String,
        children: Vec<(f32, PlanTree)>,
    },
    /// 2D directional blend: `children` sorted by direction angle (radians
    /// in `[0, 2π)`, precomputed from each child's `x`/`y`). The input
    /// direction blends the two angularly-adjacent children; a zero input
    /// holds the first child.
    Blend2D {
        param_x: String,
        param_y: String,
        children: Vec<(f32, PlanTree)>,
    },
    /// A blend space (`.blendspace`): the compiled space picks up to three
    /// samples for the axis inputs. State-level only — the tree compiler
    /// never produces one inside a region.
    Space(PlanSpace),
}

/// A state's compiled blend-space source: the axis parameters it reads, the
/// compiled space (shared through the host's cache), and one clip + rate
/// scale per sample, in the space's sample order (what its weights index).
#[derive(Debug, Clone, PartialEq)]
pub struct PlanSpace {
    /// One declared Float parameter per live axis.
    pub params: Vec<String>,
    pub space: std::sync::Arc<BlendSpace>,
    pub samples: Vec<(PlanClip, f32)>,
    /// Exponential input smoothing time in seconds; 0 = off.
    pub smoothing: f32,
}

impl PlanSpace {
    /// The raw axis input the parameters give right now, clamped to each
    /// axis's range (`[1]` is 0 for one axis).
    pub fn target(&self, params: &AnimParams) -> [f32; 2] {
        let mut v = [0.0; 2];
        for (i, (slug, axis)) in self.params.iter().zip(self.space.axes()).enumerate() {
            let (lo, hi) = (axis.min.min(axis.max), axis.max.max(axis.min));
            v[i] = params.get_float(slug).unwrap_or(0.0).clamp(lo, hi);
        }
        v
    }
}

impl PlanTree {
    /// Every clip reference in the tree, depth-first.
    pub fn clips(&self) -> Vec<&PlanClip> {
        fn walk<'a>(t: &'a PlanTree, out: &mut Vec<&'a PlanClip>) {
            match t {
                PlanTree::Clip(c) => out.push(c),
                PlanTree::Blend1D { children, .. } | PlanTree::Blend2D { children, .. } => {
                    for (_, c) in children {
                        walk(c, out);
                    }
                }
                PlanTree::Space(s) => out.extend(s.samples.iter().map(|(c, _)| c)),
            }
        }
        let mut out = Vec::new();
        walk(self, &mut out);
        out
    }
}

/// A state's compiled Pose source: a blend tree (a single clip is a
/// one-node tree), or a nested sub-state-machine — the whole referenced
/// `.animgraph`, compiled, evaluated by a child [`super::machine::AnimMachine`]
/// the parent machine owns per nested state. Nesting is state-level only
/// (spec: "states reference either a clip or a nested `.animgraph`"); a
/// machine can never appear *inside* a blend tree, which is what keeps tree
/// evaluation stateless.
#[derive(Debug, Clone, PartialEq)]
pub enum PoseSource {
    Tree(PlanTree),
    Machine {
        /// The normalized content-relative `.animgraph` path (diagnostics,
        /// editor descend).
        graph: String,
        /// The child plan, compiled recursively — cycle-refused, so this is
        /// always finite.
        plan: std::sync::Arc<AnimGraphPlan>,
    },
}

impl PoseSource {
    /// Every clip reference this source samples — nested machines walk their
    /// whole child plan (states and slots), so arm-time checks and
    /// [`AnimGraphPlan::clip_refs`] see across files.
    pub fn clips(&self) -> Vec<&PlanClip> {
        match self {
            PoseSource::Tree(t) => t.clips(),
            PoseSource::Machine { plan, .. } => plan
                .states
                .iter()
                .flat_map(|s| s.source.clips())
                .chain(plan.slots.iter().map(|s| &s.clip))
                .collect(),
        }
    }
}

/// A compiled State: what to play and how fast.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanState {
    /// The document node this came from (error anchoring, editor viz later).
    pub node_id: u64,
    /// Author-facing name (node title, falling back to `State <id>`).
    pub name: String,
    /// The pose producer: a single clip for a leaf state, a blend tree when
    /// the document carries a region keyed by this state's node id, or a
    /// nested sub-state-machine when the state's [`GRAPH_PROP`] names one.
    pub source: PoseSource,
    /// Playback-rate multiplier on the state's clock. For a nested state it
    /// scales the sub-machine's whole clock (its `dt`).
    pub speed: f32,
}

/// Comparison operator inside a rule (`compare_float`'s `op` enum).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
}

/// Float arithmetic inside a rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MathOp {
    Add,
    Sub,
    Mul,
    Div,
}

/// A compiled rule expression: one transition's pure boolean condition
/// network, reduced at compile time to a tree the machine evaluates against
/// the parameter blackboard. Rules hold no state — the one stateful element
/// of rule evaluation (trigger buffering/consumption) is owned by the
/// machine, per the spec.
#[derive(Debug, Clone, PartialEq)]
pub enum RuleExpr {
    ConstFloat(f32),
    ConstBool(bool),
    /// A Float parameter read.
    ParamFloat(String),
    /// A Bool parameter read.
    ParamBool(String),
    /// A Trigger read: "is it currently set". Every trigger a rule reads is
    /// also collected into [`PlanRule::triggers`] for consume-on-fire.
    ParamTrigger(String),
    Compare(CmpOp, Box<RuleExpr>, Box<RuleExpr>),
    Math(MathOp, Box<RuleExpr>, Box<RuleExpr>),
    And(Box<RuleExpr>, Box<RuleExpr>),
    Or(Box<RuleExpr>, Box<RuleExpr>),
    Not(Box<RuleExpr>),
}

/// One transition's compiled rule.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanRule {
    /// Bool-typed by construction (the compiler refuses anything else).
    pub expr: RuleExpr,
    /// The Trigger parameters this rule reads — statically collected, so a
    /// fire consumes them all, whether or not a given read decided the
    /// outcome. Deduplicated.
    pub triggers: Vec<String>,
}

/// A compiled play-once slot.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanSlot {
    pub node_id: u64,
    /// Author-facing name (node title, falling back to `Slot <id>`).
    pub name: String,
    pub clip: PlanClip,
    /// The declared Trigger parameter that starts this slot.
    pub trigger: String,
    /// Playback-rate multiplier on the slot's clock.
    pub speed: f32,
    /// Overlay-weight ramp durations in seconds (0.0 = hard cut).
    pub fade_in: f32,
    pub fade_out: f32,
}

/// How a compiled IK chain solves (Task 41.5 P5, I-D2).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PlanIkSolver {
    /// Two-bone analytic (root, mid, tip). The pole comes from the entity's
    /// `IkTargets` entry at runtime — it is data, not configuration.
    TwoBone,
    /// Aim one bone's local `axis` at the target, clamped to `max_angle`
    /// **radians** away from the animated orientation.
    LookAt { axis: glam::Vec3, max_angle: f32 },
}

/// Foot-placement config on a two-bone chain (Task 41.5 P6, I-D4): the tip
/// bone is the foot; `<chain name>_down` / `<chain name>_up` anim events
/// drive the plant lock.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanFootPlacement {
    /// Effector lift along the ground-hit normal, meters.
    pub ankle_offset: f32,
    /// The bone the pelvis adjust lowers; empty = no pelvis adjust.
    pub pelvis_bone: String,
}

/// A compiled IK chain (I-D3): bone *names* — resolution to indices happens
/// at arm time against the entity's actual skeleton (`runner.rs`), which is
/// also where a missing bone refuses. Lives beside states/slots on
/// [`AnimGraphPlan`], never inside a `PlanTree`.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanIkChain {
    pub node_id: u64,
    /// Chain name (node title) — the `IkTargets` key gameplay writes.
    pub name: String,
    /// Bone names, root→tip down one hierarchy path.
    pub bones: Vec<String>,
    pub solver: PlanIkSolver,
    /// The declared Float parameter fading this chain (0 = off).
    pub weight_param: String,
    /// Foot-placement config (P6); `None` = a plain gameplay-driven chain.
    pub foot: Option<PlanFootPlacement>,
}

/// Where a transition starts. Always a concrete state: a transition drawn
/// from a State Alias compiles into one entry per aliased state (sharing the
/// transition's `node_id`), so the machine never sees an alias.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionFrom {
    /// Index into [`AnimGraphPlan::states`].
    State(usize),
}

/// A compiled Transition.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanTransition {
    pub node_id: u64,
    pub from: TransitionFrom,
    /// Index into [`AnimGraphPlan::states`].
    pub to: usize,
    /// Crossfade duration in seconds (0.0 = instant switch).
    pub duration: f32,
    /// Lower value wins; ties break by node id.
    pub priority: i32,
    /// `None` — the transition's Bool input is unwired (no region, an empty
    /// one, or a RESULT with nothing wired in): always-true, the "hollow
    /// socket dot".
    pub rule: Option<PlanRule>,
}

/// The compiled, immutable form of one `.animgraph` — what the cache holds
/// and the machine walks. Indices, not ids: resolution happened at compile
/// time, so evaluation never touches the document.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AnimGraphPlan {
    pub states: Vec<PlanState>,
    /// Sorted by `(priority, node_id)` — evaluation order is the sort order.
    pub transitions: Vec<PlanTransition>,
    /// Index of the ENTRY-wired state.
    pub entry: usize,
    /// The parameter blackboard's declarations: this document's, plus every
    /// nested graph's (merged by name — type conflicts refuse at compile, the
    /// host's default wins a tie). One shared blackboard drives the whole
    /// machine tree; rules still compile strictly against their *own*
    /// document's declarations.
    pub parameters: Vec<ParamDecl>,
    /// Play-once slots, sorted by node id — when several triggers are set at
    /// once, the first slot in this order takes the (single) channel. Nested
    /// graphs' slots merge in here (the overlay channel is machine-wide;
    /// exact duplicates from nesting one graph twice are dropped).
    pub slots: Vec<PlanSlot>,
    /// IK chains, sorted by node id — applied in this order after pose
    /// evaluation, each seeing the previous chain's result. Nested graphs'
    /// chains merge in here (one skeleton serves the whole machine tree;
    /// exact duplicates drop, name collisions refuse — names key
    /// `IkTargets`).
    pub ik_chains: Vec<PlanIkChain>,
}

impl AnimGraphPlan {
    /// Deduplicated content-relative `.anim` paths this plan samples —
    /// nested graphs included.
    pub fn clip_refs(&self) -> Vec<&str> {
        let mut refs: Vec<&str> = self
            .states
            .iter()
            .flat_map(|s| s.source.clips())
            .map(|c| c.clip.as_str())
            .chain(self.slots.iter().map(|s| s.clip.clip.as_str()))
            .collect();
        refs.sort_unstable();
        refs.dedup();
        refs
    }
}

// ---------------------------------------------------------------------------
// Compiler
// ---------------------------------------------------------------------------

fn float_prop(props: &std::collections::BTreeMap<String, PropValue>, key: &str) -> Option<f32> {
    match props.get(key) {
        Some(PropValue::Float(f)) => Some(*f),
        _ => None,
    }
}

fn str_prop<'a>(
    props: &'a std::collections::BTreeMap<String, PropValue>,
    key: &str,
) -> Option<&'a str> {
    match props.get(key) {
        // `Asset` is the natural authoring for a clip path; `Str` is accepted
        // because a hand-written doc will reach for it (same latitude
        // `GraphDoc::curve_refs` gives Timeline curves).
        Some(PropValue::Asset(s)) | Some(PropValue::Str(s)) => Some(s.as_str()),
        _ => None,
    }
}

/// Compile the document's `variables` into the typed parameter list — the
/// contract between gameplay and the graph. Public seam: the editor's rule
/// canvas (ticket 05) compiles a rule region against these without walking
/// the whole machine.
pub fn compile_parameters(doc: &GraphDoc) -> Result<Vec<ParamDecl>, String> {
    let mut parameters: Vec<ParamDecl> = Vec::new();
    for v in &doc.variables {
        if parameters.iter().any(|p| p.slug == v.slug) {
            return Err(format!("parameter '{}' is declared twice", v.slug));
        }
        let (ty, default) = match v.ty {
            PinType::Float => (
                AnimParamType::Float,
                match v.default {
                    Some(PropValue::Float(f)) => ParamValue::Float(f),
                    _ => ParamValue::Float(0.0),
                },
            ),
            PinType::Bool => (
                AnimParamType::Bool,
                match v.default {
                    Some(PropValue::Bool(b)) => ParamValue::Bool(b),
                    _ => ParamValue::Bool(false),
                },
            ),
            PinType::Domain(ref d) if d == TRIGGER_PARAM_DOMAIN => (
                // A Trigger always starts unset — a declaration default is
                // meaningless for a one-shot and is ignored.
                AnimParamType::Trigger,
                ParamValue::Trigger(false),
            ),
            ref other => {
                return Err(format!(
                    "parameter '{}': {other:?} is not an animation parameter type \
                     (Float, Bool and Trigger)",
                    v.slug
                ))
            }
        };
        parameters.push(ParamDecl {
            slug: v.slug.clone(),
            ty,
            default,
        });
    }
    Ok(parameters)
}

/// How the compiler resolves the files a document references: nested
/// `.animgraph` documents and `.blendspace` assets. The runner hands its
/// asset loader (through its caches), the editor reads the content root, and
/// tests hand over maps. A plain `Fn(&str) -> Option<GraphDoc>` closure is a
/// loader that knows graphs only (every blend space reads as "not found").
pub trait AnimGraphLoader {
    /// A nested `.animgraph`, by normalized content-relative path.
    fn graph(&self, content_rel: &str) -> Option<GraphDoc>;
    /// A compiled `.blendspace`: `None` when the file does not exist,
    /// `Some(Err)` when it exists but fails to parse or compile.
    fn blend_space(
        &self,
        content_rel: &str,
    ) -> Option<Result<std::sync::Arc<BlendSpace>, String>> {
        let _ = content_rel;
        None
    }
}

impl<F: Fn(&str) -> Option<GraphDoc>> AnimGraphLoader for F {
    fn graph(&self, content_rel: &str) -> Option<GraphDoc> {
        self(content_rel)
    }
}

/// Compile a `.animgraph` document into a plan, with no way to resolve
/// nested `.animgraph` or `.blendspace` references — a document that has any
/// refuses with "could not be loaded" / "not found". The seam for callers
/// that know their document is self-contained (and for the editor's rule
/// projection); everything else goes through [`compile_anim_graph_with`].
pub fn compile_anim_graph(doc: &GraphDoc) -> Result<AnimGraphPlan, String> {
    compile_anim_graph_with(doc, "", &|_: &str| None)
}

/// Compile a `.animgraph` document into a plan, resolving nested
/// sub-state-machine and blend-space references through `load` (see
/// [`AnimGraphLoader`]). `path` is this document's own content-relative
/// path; it seeds the cycle guard so `a.animgraph` nesting itself — directly
/// or through any chain — refuses instead of recursing forever.
///
/// Refusals are author errors, phrased against the node that caused them;
/// a nested graph's refusal is wrapped with the referencing state and file
/// ("state 'Locomotion': in 'graphs/loco.animgraph': …"), so the anchored
/// error lands on the state whose reference is broken.
pub fn compile_anim_graph_with(
    doc: &GraphDoc,
    path: &str,
    load: &dyn AnimGraphLoader,
) -> Result<AnimGraphPlan, String> {
    let mut stack = Vec::new();
    let root = crate::engine::scripting::normalize_graph_path(path);
    if !root.is_empty() {
        stack.push(root);
    }
    compile_doc(doc, &mut stack, load)
}

/// Rewrite every legacy Any State node ([`ANIM_ANY_STATE_TYPE_ID`]) in place
/// into a Global State Alias titled "Any State" — same id, same edges, so
/// nothing else in the document moves. Returns how many nodes changed (0 =
/// the document was already current; running it again is a no-op).
///
/// Runs at both loading seams — compile (on a private copy, root and nested
/// documents alike) and the editor's open — so old files keep working
/// unsaved and migrate when saved.
pub fn upgrade_any_state(doc: &mut GraphDoc) -> usize {
    let mut count = 0;
    for n in doc
        .nodes
        .iter_mut()
        .filter(|n| n.type_id == ANIM_ANY_STATE_TYPE_ID)
    {
        n.type_id = ANIM_STATE_ALIAS_TYPE_ID.to_string();
        n.type_version = 1;
        n.properties
            .insert(ALIAS_GLOBAL_PROP.to_string(), PropValue::Bool(true));
        if n.title.as_deref().is_none_or(|t| t.trim().is_empty()) {
            n.title = Some("Any State".to_string());
        }
        count += 1;
    }
    count
}

/// One document of the nesting tree. `stack` holds the normalized paths
/// currently being compiled, root-first — a nested reference back into it is
/// a cycle, refused with the chain spelled out.
fn compile_doc(
    doc: &GraphDoc,
    stack: &mut Vec<String>,
    load: &dyn AnimGraphLoader,
) -> Result<AnimGraphPlan, String> {
    // A pre-alias document compiles as its upgraded self; the caller's copy
    // stays untouched (the editor migrates its own on open).
    let mut current = std::borrow::Cow::Borrowed(doc);
    if doc.nodes.iter().any(|n| n.type_id == ANIM_ANY_STATE_TYPE_ID) {
        upgrade_any_state(current.to_mut());
    }
    let doc = &*current;

    // Animation graphs are Client-realm by definition (spec realm note): the
    // server never evaluates animation (ADR 0002), and saying so in the
    // document is the authority statement the realm field exists for.
    if doc.realm != GraphRealm::Client {
        return Err(format!(
            "an animation graph must declare `realm: Client` (found {:?}) — \
             animation is client-derived (ADR 0002)",
            doc.realm
        ));
    }

    // Parameters first, from the document's variables — states need them to
    // validate the parameters their blend trees read.
    let parameters = compile_parameters(doc)?;

    // States, in document order (index = plan identity). A state with a
    // non-empty region compiles it as a blend tree; a `graph` property makes
    // it a nested sub-state-machine; a `space` property plays a blend space;
    // a leaf state plays the clip its `clip` property names.
    let mut states: Vec<PlanState> = Vec::new();
    // Nested declarations to merge into the blackboard, with the state that
    // brought them in (refusal anchoring).
    let mut nested_params: Vec<(String, ParamDecl)> = Vec::new();
    for n in doc.nodes.iter().filter(|n| n.type_id == ANIM_STATE_TYPE_ID) {
        let name = n
            .title
            .clone()
            .unwrap_or_else(|| format!("State {}", n.id));
        let nested = str_prop(&n.properties, GRAPH_PROP).filter(|s| !s.trim().is_empty());
        let space = str_prop(&n.properties, SPACE_PROP).filter(|s| !s.trim().is_empty());
        let source = match doc.regions.get(&n.id).filter(|r| !r.nodes.is_empty()) {
            Some(region) => PoseSource::Tree(compile_tree(region, &name, &parameters)?),
            None if nested.is_some() => {
                let child_rel = crate::engine::scripting::normalize_graph_path(
                    nested.unwrap_or_default(),
                );
                if stack.contains(&child_rel) {
                    let chain: Vec<&str> = stack
                        .iter()
                        .map(String::as_str)
                        .chain([child_rel.as_str()])
                        .collect();
                    return Err(format!(
                        "state '{name}': nesting cycle: {}",
                        chain.join(" \u{2192} ")
                    ));
                }
                let child_doc = load.graph(&child_rel).ok_or_else(|| {
                    format!("state '{name}': nested graph '{child_rel}' could not be loaded")
                })?;
                stack.push(child_rel.clone());
                let child = compile_doc(&child_doc, stack, load)
                    .map_err(|e| format!("state '{name}': in '{child_rel}': {e}"))?;
                stack.pop();
                for d in &child.parameters {
                    nested_params.push((name.clone(), d.clone()));
                }
                PoseSource::Machine {
                    graph: child_rel,
                    plan: std::sync::Arc::new(child),
                }
            }
            None if space.is_some() => PoseSource::Tree(PlanTree::Space(compile_space(
                &name,
                space.unwrap_or_default(),
                &parameters,
                load,
            )?)),
            None => {
                let clip = str_prop(&n.properties, CLIP_PROP)
                    .filter(|s| !s.trim().is_empty())
                    .ok_or_else(|| {
                        format!(
                            "state '{name}' names no clip (property `{CLIP_PROP}`), nested \
                             graph (property `{GRAPH_PROP}`) or blend space (property \
                             `{SPACE_PROP}`), and has no blend tree"
                        )
                    })?;
                PoseSource::Tree(PlanTree::Clip(PlanClip {
                    clip: crate::engine::scripting::normalize_graph_path(clip),
                    clip_name: str_prop(&n.properties, CLIP_NAME_PROP)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string),
                }))
            }
        };
        states.push(PlanState {
            node_id: n.id,
            name,
            source,
            speed: float_prop(&n.properties, SPEED_PROP).unwrap_or(1.0),
        });
    }
    if states.is_empty() {
        return Err("an animation graph needs at least one state".to_string());
    }
    let state_index =
        |id: u64| -> Option<usize> { states.iter().position(|s| s.node_id == id) };

    // ENTRY: exactly one node, exactly one outgoing edge, into a state.
    let mut entries = doc.nodes.iter().filter(|n| n.type_id == ANIM_ENTRY_TYPE_ID);
    let entry_node = entries
        .next()
        .ok_or_else(|| "an animation graph needs an ENTRY node".to_string())?;
    if entries.next().is_some() {
        return Err("an animation graph has exactly one ENTRY node".to_string());
    }
    let mut entry_edges = doc.edges.iter().filter(|e| e.from_node == entry_node.id);
    let entry_edge = entry_edges
        .next()
        .ok_or_else(|| "the ENTRY node is not wired to a state".to_string())?;
    if entry_edges.next().is_some() {
        return Err("the ENTRY node has exactly one outgoing wire".to_string());
    }
    let entry = state_index(entry_edge.to_node)
        .ok_or_else(|| "the ENTRY node must be wired to a state".to_string())?;

    // Aliases: each resolves to the state indices it stands for, sorted —
    // a Global alias to every state; a listed one to its normalised list
    // (dupes dropped, non-Int/negative entries ignored, a non-state id
    // refused). Refusals name the alias so they anchor on its node.
    let mut aliases: Vec<(u64, Vec<usize>)> = Vec::new();
    for n in doc
        .nodes
        .iter()
        .filter(|n| n.type_id == ANIM_STATE_ALIAS_TYPE_ID)
    {
        let name = n
            .title
            .clone()
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| "Alias".to_string());
        let global = !matches!(
            n.properties.get(ALIAS_GLOBAL_PROP),
            Some(PropValue::Bool(false))
        );
        let mut indices: Vec<usize> = if global {
            (0..states.len()).collect()
        } else {
            let listed = match n.properties.get(ALIAS_STATES_PROP) {
                Some(PropValue::Array(items)) => items.as_slice(),
                _ => &[],
            };
            let mut indices = Vec::new();
            for id in listed.iter().filter_map(|v| match v {
                PropValue::Int(id) if *id >= 0 => Some(*id as u64),
                _ => None,
            }) {
                indices.push(state_index(id).ok_or_else(|| {
                    format!("alias '{name}' references a missing state (node {id})")
                })?);
            }
            indices
        };
        indices.sort_unstable();
        indices.dedup();
        if indices.is_empty() && !global {
            return Err(format!("alias '{name}' has no states"));
        }
        aliases.push((n.id, indices));
    }

    // Transitions: resolve both endpoints through their pins. A source is a
    // state or an alias; a target is always a state. A transition leaving an
    // alias expands into one ordinary transition per aliased state (sharing
    // its node id), never into its own target — a Global alias in a
    // one-state graph therefore compiles to nothing, which is not an error.
    let mut transitions: Vec<PlanTransition> = Vec::new();
    for n in doc
        .nodes
        .iter()
        .filter(|n| n.type_id == ANIM_TRANSITION_TYPE_ID)
    {
        let from_edge = doc
            .edges
            .iter()
            .find(|e| e.to_node == n.id && e.to_pin == TRANSITION_FROM_PIN);
        let from_edge = from_edge
            .ok_or_else(|| format!("transition {} has no source state", n.id))?;
        let alias = aliases
            .iter()
            .find(|(id, _)| *id == from_edge.from_node)
            .map(|(_, indices)| indices);
        let single = match alias {
            Some(_) => None,
            None => Some(
                state_index(from_edge.from_node)
                    .ok_or_else(|| format!("transition {} has no source state", n.id))?,
            ),
        };
        let to = doc
            .edges
            .iter()
            .find(|e| e.from_node == n.id && e.from_pin == TRANSITION_TO_PIN)
            .and_then(|e| state_index(e.to_node))
            .ok_or_else(|| format!("transition {} has no target state", n.id))?;
        let sources: Vec<usize> = match alias {
            Some(indices) => indices.iter().copied().filter(|&s| s != to).collect(),
            None => single.into_iter().collect(),
        };
        let rule = compile_rule(doc, n.id, &parameters)?;
        let duration = float_prop(&n.properties, DURATION_PROP)
            .unwrap_or(0.0)
            .max(0.0);
        let priority = match n.properties.get(PRIORITY_PROP) {
            Some(PropValue::Int(i)) => *i,
            _ => 0,
        };
        for s in sources {
            transitions.push(PlanTransition {
                node_id: n.id,
                from: TransitionFrom::State(s),
                to,
                rule: rule.clone(),
                duration,
                priority,
            });
        }
    }
    // Evaluation order is the sort order: lower priority value first, node id
    // as the deterministic tiebreak.
    transitions.sort_by_key(|t| (t.priority, t.node_id));

    // Play-once slots: a clip, a starting Trigger, an overlay envelope.
    let mut slots: Vec<PlanSlot> = Vec::new();
    for n in doc
        .nodes
        .iter()
        .filter(|n| n.type_id == ANIM_PLAY_ONCE_TYPE_ID)
    {
        let name = n.title.clone().unwrap_or_else(|| format!("Slot {}", n.id));
        let clip = str_prop(&n.properties, CLIP_PROP)
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| {
                format!("play-once slot '{name}' names no clip (property `{CLIP_PROP}`)")
            })?;
        let trigger = match n.properties.get(SLOT_TRIGGER_PROP) {
            Some(PropValue::Str(s)) if !s.is_empty() => s.clone(),
            _ => {
                return Err(format!(
                    "play-once slot '{name}' names no trigger (property `{SLOT_TRIGGER_PROP}`)"
                ))
            }
        };
        match parameters.iter().find(|p| p.slug == trigger) {
            None => {
                return Err(format!(
                    "play-once slot '{name}': parameter '{trigger}' is not declared"
                ))
            }
            Some(p) if p.ty != AnimParamType::Trigger => {
                return Err(format!(
                    "play-once slot '{name}': parameter '{trigger}' is not a Trigger"
                ))
            }
            Some(_) => {}
        }
        slots.push(PlanSlot {
            node_id: n.id,
            name,
            clip: PlanClip {
                clip: crate::engine::scripting::normalize_graph_path(clip),
                clip_name: str_prop(&n.properties, CLIP_NAME_PROP)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string),
            },
            trigger,
            speed: float_prop(&n.properties, SPEED_PROP).unwrap_or(1.0),
            fade_in: float_prop(&n.properties, SLOT_FADE_IN_PROP)
                .unwrap_or(0.0)
                .max(0.0),
            fade_out: float_prop(&n.properties, SLOT_FADE_OUT_PROP)
                .unwrap_or(0.0)
                .max(0.0),
        });
    }
    slots.sort_by_key(|s| s.node_id);

    // Nested graphs' slots join the root's single override channel, after
    // this document's own (deterministic: host slots by node id, then nested
    // in state order). Nesting one graph twice would clone its slots — exact
    // duplicates are dropped, the channel needs only one.
    let nested_slots: Vec<PlanSlot> = states
        .iter()
        .filter_map(|s| match &s.source {
            PoseSource::Machine { plan, .. } => Some(plan.slots.clone()),
            _ => None,
        })
        .flatten()
        .collect();
    for s in nested_slots {
        if !slots.contains(&s) {
            slots.push(s);
        }
    }

    // IK chains (Task 41.5 P5): standalone nodes, like slots. Bone existence
    // is an arm-time check (the compiler never sees a skeleton); everything
    // knowable from the document refuses here, anchored on the chain.
    let mut ik_chains: Vec<PlanIkChain> = Vec::new();
    for n in doc
        .nodes
        .iter()
        .filter(|n| n.type_id == ANIM_IK_CHAIN_TYPE_ID)
    {
        let name = n
            .title
            .clone()
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| format!("IK {}", n.id));
        let bones: Vec<String> = str_prop(&n.properties, IK_BONES_PROP)
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        if bones.is_empty() {
            return Err(format!(
                "IK chain '{name}' names no bones (property `{IK_BONES_PROP}`: \
                 comma-separated bone names, root\u{2192}tip)"
            ));
        }
        let solver_slug = match n.properties.get(IK_SOLVER_PROP) {
            Some(PropValue::Enum(s)) | Some(PropValue::Str(s)) if !s.is_empty() => s.as_str(),
            _ => IK_SOLVER_TWO_BONE,
        };
        let solver = match solver_slug {
            IK_SOLVER_TWO_BONE => {
                if bones.len() != 3 {
                    return Err(format!(
                        "IK chain '{name}': the two-bone solver takes exactly 3 bones \
                         (root, mid, tip), got {}",
                        bones.len()
                    ));
                }
                PlanIkSolver::TwoBone
            }
            IK_SOLVER_LOOK_AT => {
                if bones.len() != 1 {
                    return Err(format!(
                        "IK chain '{name}': the look-at solver takes exactly 1 bone, got {}",
                        bones.len()
                    ));
                }
                let axis = glam::Vec3::new(
                    float_prop(&n.properties, IK_AXIS_X_PROP).unwrap_or(0.0),
                    float_prop(&n.properties, IK_AXIS_Y_PROP).unwrap_or(0.0),
                    float_prop(&n.properties, IK_AXIS_Z_PROP).unwrap_or(1.0),
                );
                if axis.length_squared() < 1e-8 {
                    return Err(format!("IK chain '{name}': the aim axis is zero"));
                }
                PlanIkSolver::LookAt {
                    axis: axis.normalize(),
                    max_angle: float_prop(&n.properties, IK_MAX_ANGLE_PROP)
                        .unwrap_or(90.0)
                        .max(0.0)
                        .to_radians(),
                }
            }
            other => {
                return Err(format!(
                    "IK chain '{name}': unknown solver '{other}' (the solvers are \
                     '{IK_SOLVER_TWO_BONE}' and '{IK_SOLVER_LOOK_AT}')"
                ))
            }
        };
        let weight_param = match n.properties.get(IK_WEIGHT_PARAM_PROP) {
            Some(PropValue::Str(s)) if !s.is_empty() => s.clone(),
            _ => {
                return Err(format!(
                    "IK chain '{name}' names no weight parameter \
                     (property `{IK_WEIGHT_PARAM_PROP}`)"
                ))
            }
        };
        match parameters.iter().find(|p| p.slug == weight_param) {
            Some(p) if p.ty == AnimParamType::Float => {}
            Some(_) => {
                return Err(format!(
                    "IK chain '{name}': parameter '{weight_param}' is not a Float"
                ))
            }
            None => {
                return Err(format!(
                    "IK chain '{name}': parameter '{weight_param}' is not declared"
                ))
            }
        }
        // Foot placement (P6): opt-in per chain, two-bone only — the tip
        // bone is the foot, so a look-at chain has nothing to plant.
        let foot = match n.properties.get(IK_FOOT_PROP) {
            Some(PropValue::Bool(true)) => {
                if !matches!(solver, PlanIkSolver::TwoBone) {
                    return Err(format!(
                        "IK chain '{name}': foot placement needs the \
                         '{IK_SOLVER_TWO_BONE}' solver"
                    ));
                }
                Some(PlanFootPlacement {
                    ankle_offset: float_prop(&n.properties, IK_ANKLE_OFFSET_PROP)
                        .unwrap_or(0.1),
                    pelvis_bone: str_prop(&n.properties, IK_PELVIS_PROP)
                        .unwrap_or_default()
                        .trim()
                        .to_string(),
                })
            }
            _ => None,
        };
        ik_chains.push(PlanIkChain {
            node_id: n.id,
            name,
            bones,
            solver,
            weight_param,
            foot,
        });
    }
    // Nested graphs' chains act on the same skeleton, so they join the
    // host's list (exact duplicates from nesting one graph twice drop).
    let nested_chains: Vec<PlanIkChain> = states
        .iter()
        .filter_map(|s| match &s.source {
            PoseSource::Machine { plan, .. } => Some(plan.ik_chains.clone()),
            _ => None,
        })
        .flatten()
        .collect();
    for c in nested_chains {
        if !ik_chains.contains(&c) {
            ik_chains.push(c);
        }
    }
    // Sort after the merge so the "applied in node-id order" contract holds
    // across host + nested chains (stable: host wins ties on cross-document
    // id collisions).
    ik_chains.sort_by_key(|c| c.node_id);
    // Chain names key the `IkTargets` component, so they must be unique.
    for (i, c) in ik_chains.iter().enumerate() {
        if ik_chains[..i].iter().any(|o| o.name == c.name) {
            return Err(format!(
                "two IK chains are named '{}' — chain names key the IkTargets \
                 component, so they must be unique",
                c.name
            ));
        }
    }
    // One pelvis drives the character: every foot chain that names a pelvis
    // bone must name the same one (nested chains included).
    let mut pelvis: Option<(&str, &str)> = None;
    for c in &ik_chains {
        let Some(f) = &c.foot else { continue };
        if f.pelvis_bone.is_empty() {
            continue;
        }
        match pelvis {
            None => pelvis = Some((&c.name, &f.pelvis_bone)),
            Some((_, b)) if b == f.pelvis_bone => {}
            Some((other, b)) => {
                return Err(format!(
                    "IK chains '{other}' and '{}' name different pelvis bones \
                     ('{b}' vs '{}') — one pelvis drives the character",
                    c.name, f.pelvis_bone
                ))
            }
        }
    }

    // Nested declarations join the blackboard: one shared surface drives the
    // whole machine tree, so gameplay writes the union. Same name and type
    // collapse to one entry (this document's declaration and default win); a
    // type conflict refuses, or the nested rules would read a value of the
    // wrong shape at runtime.
    let mut parameters = parameters;
    for (state, d) in nested_params {
        match parameters.iter().find(|p| p.slug == d.slug) {
            None => parameters.push(d),
            Some(p) if p.ty == d.ty => {}
            Some(p) => {
                return Err(format!(
                    "state '{state}': parameter '{}' is a {:?} in the nested graph but a \
                     {:?} here — one blackboard drives the whole machine, so the types \
                     must agree",
                    d.slug, d.ty, p.ty
                ))
            }
        }
    }

    Ok(AnimGraphPlan {
        states,
        transitions,
        entry,
        parameters,
        slots,
        ik_chains,
    })
}

/// Compile a state's `space` reference: the file must resolve and compile,
/// every live axis must read a declared Float, every sample must name a clip.
/// Clip *existence* is an arm-time check (the runner's clip cache), exactly
/// as it is for `clip` states and tree leaves — the compiler never touches
/// `.anim` files.
fn compile_space(
    state: &str,
    rel: &str,
    parameters: &[ParamDecl],
    load: &dyn AnimGraphLoader,
) -> Result<PlanSpace, String> {
    let rel = crate::engine::scripting::normalize_graph_path(rel);
    let space = match load.blend_space(&rel) {
        None => return Err(format!("state '{state}': blend space '{rel}' not found")),
        Some(Err(e)) => return Err(format!("state '{state}': blend space '{rel}': {e}")),
        Some(Ok(space)) => space,
    };
    let mut params = Vec::with_capacity(space.axes().len());
    for axis in space.axes() {
        let slug = axis.param_name();
        match parameters.iter().find(|p| p.slug == slug) {
            Some(p) if p.ty == AnimParamType::Float => params.push(slug.to_string()),
            _ => {
                return Err(format!(
                    "state '{state}': blend space '{rel}' axis '{}' parameter '{slug}' is not \
                     a declared Float parameter",
                    axis.name
                ))
            }
        }
    }
    let mut samples = Vec::with_capacity(space.samples().len());
    for (i, s) in space.samples().iter().enumerate() {
        if s.clip.trim().is_empty() {
            return Err(format!(
                "state '{state}': blend space '{rel}' sample {i} names no clip"
            ));
        }
        samples.push((
            PlanClip {
                clip: crate::engine::scripting::normalize_graph_path(&s.clip),
                clip_name: s.clip_name.clone().filter(|n| !n.is_empty()),
            },
            s.rate_scale,
        ));
    }
    Ok(PlanSpace {
        params,
        samples,
        smoothing: space.input_smoothing().max(0.0),
        space,
    })
}

// ---------------------------------------------------------------------------
// Rule compiler (the embedded boolean network on a transition)
// ---------------------------------------------------------------------------

/// The node types a rule may contain besides the RESULT sink: parameter
/// reads, comparisons, float math, boolean logic, and reroutes. This
/// whitelist **is** the purity rule — effects, exec flow, event emitters,
/// latent nodes and anything realm-restricted (Server included) are refused
/// by not being on it, which catches node types that do not exist yet too.
const RULE_NODE_TYPES: [&str; 10] = [
    VAR_GET_TYPE_ID,
    COMPARE_FLOAT,
    ADD_FLOAT,
    SUB_FLOAT,
    MUL_FLOAT,
    DIV_FLOAT,
    AND,
    OR,
    NOT,
    REROUTE_TYPE_ID,
];

/// The std nodes' conventional pin slugs (they are literals in the std
/// descriptors, not exported constants).
const A_PIN: &str = "a";
const B_PIN: &str = "b";
const OP_PROP: &str = "op";
const RESULT_PIN: &str = "result";

/// What a rule wire carries. Rules know exactly two value kinds; a Trigger
/// read is a Bool at the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuleTy {
    Float,
    Bool,
}

impl RuleTy {
    fn name(self) -> &'static str {
        match self {
            RuleTy::Float => "a Float",
            RuleTy::Bool => "a Bool",
        }
    }
}

/// Compile a transition's embedded rule region into a [`PlanRule`].
///
/// `Ok(None)` is always-true: no region, an empty one, or a RESULT whose
/// input is unwired — the "hollow socket dot" reading the spec promises.
fn compile_rule(
    doc: &GraphDoc,
    tid: u64,
    parameters: &[ParamDecl],
) -> Result<Option<PlanRule>, String> {
    let Some(region) = doc.regions.get(&tid) else {
        return Ok(None);
    };
    compile_rule_region(region, tid, parameters)
}

/// Compile one rule region against an already-compiled parameter list.
/// Public seam: the editor's rule canvas (ticket 05) checks the region it is
/// editing without recompiling the whole machine, and gets refusals phrased
/// exactly as [`compile_anim_graph`] would phrase them ("transition {tid}:
/// rule node {id} …"), so the two never disagree about what is wrong.
pub fn compile_rule_region(
    region: &GraphRegion,
    tid: u64,
    parameters: &[ParamDecl],
) -> Result<Option<PlanRule>, String> {
    if region.nodes.is_empty() {
        return Ok(None);
    }

    // Purity by whitelist, refused by name so the message is actionable.
    for n in &region.nodes {
        if n.type_id != ANIM_RULE_RESULT_TYPE_ID && !RULE_NODE_TYPES.contains(&n.type_id.as_str())
        {
            return Err(format!(
                "transition {tid}: rule node {} ('{}') is not a rule node — a rule is \
                 pure boolean logic (parameter reads, comparisons, float math, and/or/not)",
                n.id, n.type_id
            ));
        }
    }

    // Exactly one RESULT sink.
    let mut results = region
        .nodes
        .iter()
        .filter(|n| n.type_id == ANIM_RULE_RESULT_TYPE_ID);
    let result = results
        .next()
        .ok_or_else(|| format!("transition {tid}: rule has no RESULT node"))?;
    if results.next().is_some() {
        return Err(format!(
            "transition {tid}: a rule has exactly one RESULT node"
        ));
    }

    // Fan-in is meaningless in a value network.
    for e in &region.edges {
        let wires = region
            .edges
            .iter()
            .filter(|o| o.to_node == e.to_node && o.to_pin == e.to_pin)
            .count();
        if wires > 1 {
            return Err(format!(
                "transition {tid}: rule node {} input '{}' has {wires} wires",
                e.to_node, e.to_pin
            ));
        }
    }

    let Some(edge) = region
        .edges
        .iter()
        .find(|e| e.to_node == result.id && e.to_pin == RULE_RESULT_PIN)
    else {
        return Ok(None); // unwired Bool input = always-true
    };
    let mut cx = RuleCx {
        region,
        parameters,
        tid,
        triggers: Vec::new(),
        stack: Vec::new(),
    };
    let expr = cx.output(edge.from_node, &edge.from_pin, RuleTy::Bool)?;
    Ok(Some(PlanRule {
        expr,
        triggers: cx.triggers,
    }))
}

/// Backward walk from the RESULT input: each node compiles from the pin the
/// wire names, type-checked against what the consumer expects.
struct RuleCx<'a> {
    region: &'a GraphRegion,
    parameters: &'a [ParamDecl],
    tid: u64,
    triggers: Vec<String>,
    /// Nodes currently being compiled — a wire back into one is a cycle.
    stack: Vec<u64>,
}

impl RuleCx<'_> {
    /// The expression feeding one input pin: the wire if there is one, else
    /// the pin's stored constant (the framework's unconnected-input idiom),
    /// else the type's zero.
    fn input(&mut self, node: &NodeInst, pin: &str, ty: RuleTy) -> Result<RuleExpr, String> {
        if let Some(e) = self
            .region
            .edges
            .iter()
            .find(|e| e.to_node == node.id && e.to_pin == pin)
        {
            return self.output(e.from_node, &e.from_pin, ty);
        }
        Ok(match ty {
            RuleTy::Float => RuleExpr::ConstFloat(match node.properties.get(pin) {
                Some(PropValue::Float(f)) => *f,
                _ => 0.0,
            }),
            RuleTy::Bool => RuleExpr::ConstBool(matches!(
                node.properties.get(pin),
                Some(PropValue::Bool(true))
            )),
        })
    }

    /// The expression a node produces on `from_pin`, expected to be `want`.
    fn output(&mut self, node_id: u64, from_pin: &str, want: RuleTy) -> Result<RuleExpr, String> {
        let tid = self.tid;
        let node = self.region.node(node_id).ok_or_else(|| {
            format!("transition {tid}: rule wire names node {node_id}, which does not exist")
        })?;
        if self.stack.contains(&node_id) {
            return Err(format!(
                "transition {tid}: the rule contains a cycle through node {node_id}"
            ));
        }
        self.stack.push(node_id);
        let (out_pin, got, expr) = match node.type_id.as_str() {
            VAR_GET_TYPE_ID => {
                let slug = match node.properties.get(VAR_PROP) {
                    Some(PropValue::Str(s)) if !s.is_empty() => s.clone(),
                    _ => {
                        return Err(format!(
                            "transition {tid}: rule node {node_id} names no parameter"
                        ))
                    }
                };
                let decl = self
                    .parameters
                    .iter()
                    .find(|p| p.slug == slug)
                    .ok_or_else(|| {
                        format!("transition {tid}: parameter '{slug}' is not declared")
                    })?;
                let (ty, expr) = match decl.ty {
                    AnimParamType::Float => (RuleTy::Float, RuleExpr::ParamFloat(slug)),
                    AnimParamType::Bool => (RuleTy::Bool, RuleExpr::ParamBool(slug)),
                    AnimParamType::Trigger => {
                        if !self.triggers.contains(&slug) {
                            self.triggers.push(slug.clone());
                        }
                        (RuleTy::Bool, RuleExpr::ParamTrigger(slug))
                    }
                };
                (VAR_VALUE_PIN, ty, expr)
            }
            COMPARE_FLOAT => {
                let op = match node.properties.get(OP_PROP) {
                    None => CmpOp::Equal, // the descriptor's default
                    Some(PropValue::Enum(s)) => match s.as_str() {
                        "equal" => CmpOp::Equal,
                        "not_equal" => CmpOp::NotEqual,
                        "less" => CmpOp::Less,
                        "less_equal" => CmpOp::LessEqual,
                        "greater" => CmpOp::Greater,
                        "greater_equal" => CmpOp::GreaterEqual,
                        other => {
                            return Err(format!(
                                "transition {tid}: rule node {node_id}: unknown comparison \
                                 operator '{other}'"
                            ))
                        }
                    },
                    Some(_) => {
                        return Err(format!(
                            "transition {tid}: rule node {node_id}: the comparison operator \
                             must be an enum value"
                        ))
                    }
                };
                let a = self.input(node, A_PIN, RuleTy::Float)?;
                let b = self.input(node, B_PIN, RuleTy::Float)?;
                (
                    RESULT_PIN,
                    RuleTy::Bool,
                    RuleExpr::Compare(op, Box::new(a), Box::new(b)),
                )
            }
            ADD_FLOAT | SUB_FLOAT | MUL_FLOAT | DIV_FLOAT => {
                let op = match node.type_id.as_str() {
                    ADD_FLOAT => MathOp::Add,
                    SUB_FLOAT => MathOp::Sub,
                    MUL_FLOAT => MathOp::Mul,
                    _ => MathOp::Div,
                };
                let a = self.input(node, A_PIN, RuleTy::Float)?;
                let b = self.input(node, B_PIN, RuleTy::Float)?;
                (
                    RESULT_PIN,
                    RuleTy::Float,
                    RuleExpr::Math(op, Box::new(a), Box::new(b)),
                )
            }
            AND | OR => {
                let a = Box::new(self.input(node, A_PIN, RuleTy::Bool)?);
                let b = Box::new(self.input(node, B_PIN, RuleTy::Bool)?);
                let expr = if node.type_id == AND {
                    RuleExpr::And(a, b)
                } else {
                    RuleExpr::Or(a, b)
                };
                (RESULT_PIN, RuleTy::Bool, expr)
            }
            NOT => {
                let a = self.input(node, A_PIN, RuleTy::Bool)?;
                (RESULT_PIN, RuleTy::Bool, RuleExpr::Not(Box::new(a)))
            }
            // A reroute is a typed pass-through: whatever the consumer
            // expects flows through unchanged.
            REROUTE_TYPE_ID => {
                let inner = self.input(node, REROUTE_IN, want)?;
                (REROUTE_OUT, want, inner)
            }
            other => {
                // RESULT as a source lands here too: it is a sink.
                return Err(format!(
                    "transition {tid}: rule node {node_id} ('{other}') has no outputs"
                ));
            }
        };
        self.stack.pop();
        if from_pin != out_pin {
            return Err(format!(
                "transition {tid}: rule node {node_id} ('{}') has no output '{from_pin}'",
                node.type_id
            ));
        }
        if got != want {
            return Err(format!(
                "transition {tid}: rule node {node_id} ('{}') produces {} where {} is expected",
                node.type_id,
                got.name(),
                want.name()
            ));
        }
        Ok(expr)
    }
}

// ---------------------------------------------------------------------------
// Blend-tree compiler (the embedded pose network inside a state)
// ---------------------------------------------------------------------------

/// The node types a blend tree may contain besides the RESULT sink. The same
/// whitelist posture as rules: anything else — rule nodes, effects, exec
/// flow, node types that do not exist yet — is refused by not being on it.
const TREE_NODE_TYPES: [&str; 4] = [
    ANIM_CLIP_TYPE_ID,
    ANIM_BLEND1D_TYPE_ID,
    ANIM_BLEND2D_TYPE_ID,
    REROUTE_TYPE_ID,
];

/// Compile a state's embedded tree region into a [`PlanTree`].
///
/// Unlike a rule, a tree has no "unwired = default" reading: a state must
/// produce a pose, so an unwired RESULT is a refusal, not a fallback.
fn compile_tree(
    region: &GraphRegion,
    state: &str,
    parameters: &[ParamDecl],
) -> Result<PlanTree, String> {
    for n in &region.nodes {
        if n.type_id != ANIM_POSE_RESULT_TYPE_ID && !TREE_NODE_TYPES.contains(&n.type_id.as_str())
        {
            return Err(format!(
                "state '{state}': tree node {} ('{}') is not a blend-tree node — a state's \
                 tree is clip nodes, 1D/2D blends and reroutes",
                n.id, n.type_id
            ));
        }
    }

    // Exactly one RESULT sink.
    let mut results = region
        .nodes
        .iter()
        .filter(|n| n.type_id == ANIM_POSE_RESULT_TYPE_ID);
    let result = results
        .next()
        .ok_or_else(|| format!("state '{state}': blend tree has no RESULT node"))?;
    if results.next().is_some() {
        return Err(format!(
            "state '{state}': a blend tree has exactly one RESULT node"
        ));
    }

    // Fan-in is meaningless in a value network.
    for e in &region.edges {
        let wires = region
            .edges
            .iter()
            .filter(|o| o.to_node == e.to_node && o.to_pin == e.to_pin)
            .count();
        if wires > 1 {
            return Err(format!(
                "state '{state}': tree node {} input '{}' has {wires} wires",
                e.to_node, e.to_pin
            ));
        }
    }

    let edge = region
        .edges
        .iter()
        .find(|e| e.to_node == result.id && e.to_pin == POSE_PIN)
        .ok_or_else(|| {
            format!("state '{state}': nothing is wired into the blend tree's RESULT")
        })?;
    let mut cx = TreeCx {
        region,
        parameters,
        state,
        stack: Vec::new(),
    };
    cx.output(edge.from_node, &edge.from_pin)
}

/// Backward walk from the RESULT input. Only Pose flows in a tree, so there
/// is no wire typing to check — just structure: pins exist, driving
/// parameters are declared Floats, children carry their placement data.
struct TreeCx<'a> {
    region: &'a GraphRegion,
    parameters: &'a [ParamDecl],
    state: &'a str,
    /// Nodes currently being compiled — a wire back into one is a cycle.
    stack: Vec<u64>,
}

impl TreeCx<'_> {
    /// A blend node's driving parameter: bound by name, must be a declared
    /// Float (a Bool or Trigger has no axis to place children on).
    fn float_param(&self, node: &NodeInst, key: &str) -> Result<String, String> {
        let state = self.state;
        let slug = match node.properties.get(key) {
            Some(PropValue::Str(s)) if !s.is_empty() => s.clone(),
            _ => {
                return Err(format!(
                    "state '{state}': blend node {} names no parameter (property `{key}`)",
                    node.id
                ))
            }
        };
        match self.parameters.iter().find(|p| p.slug == slug) {
            None => Err(format!(
                "state '{state}': parameter '{slug}' is not declared"
            )),
            Some(p) if p.ty != AnimParamType::Float => Err(format!(
                "state '{state}': blend node {}: parameter '{slug}' is not a Float",
                node.id
            )),
            Some(_) => Ok(slug),
        }
    }

    /// A blend node's wired children as `(pin index, subtree)`, sorted by
    /// pin index. Indices need not be contiguous — they are names.
    fn children(&mut self, node_id: u64) -> Result<Vec<(usize, PlanTree)>, String> {
        let state = self.state;
        let mut wires: Vec<(usize, u64, String)> = Vec::new();
        for e in self.region.edges.iter().filter(|e| e.to_node == node_id) {
            let idx = e
                .to_pin
                .strip_prefix("in_")
                .and_then(|s| s.parse::<usize>().ok())
                .ok_or_else(|| {
                    format!(
                        "state '{state}': blend node {node_id} has no input '{}'",
                        e.to_pin
                    )
                })?;
            wires.push((idx, e.from_node, e.from_pin.clone()));
        }
        wires.sort_by_key(|(i, ..)| *i);
        let mut children = Vec::new();
        for (idx, from, from_pin) in wires {
            children.push((idx, self.output(from, &from_pin)?));
        }
        Ok(children)
    }

    /// The subtree a node produces on `from_pin`.
    fn output(&mut self, node_id: u64, from_pin: &str) -> Result<PlanTree, String> {
        let state = self.state;
        let node = self.region.node(node_id).ok_or_else(|| {
            format!("state '{state}': tree wire names node {node_id}, which does not exist")
        })?;
        if self.stack.contains(&node_id) {
            return Err(format!(
                "state '{state}': the blend tree contains a cycle through node {node_id}"
            ));
        }
        self.stack.push(node_id);
        let (out_pin, tree) = match node.type_id.as_str() {
            ANIM_CLIP_TYPE_ID => {
                let clip = str_prop(&node.properties, CLIP_PROP)
                    .filter(|s| !s.trim().is_empty())
                    .ok_or_else(|| {
                        format!(
                            "state '{state}': clip node {node_id} names no clip \
                             (property `{CLIP_PROP}`)"
                        )
                    })?;
                (
                    POSE_PIN,
                    PlanTree::Clip(PlanClip {
                        clip: crate::engine::scripting::normalize_graph_path(clip),
                        clip_name: str_prop(&node.properties, CLIP_NAME_PROP)
                            .filter(|s| !s.is_empty())
                            .map(str::to_string),
                    }),
                )
            }
            ANIM_BLEND1D_TYPE_ID => {
                let param = self.float_param(node, BLEND_PARAM_PROP)?;
                let mut children = Vec::new();
                for (idx, sub) in self.children(node_id)? {
                    let threshold = float_prop(&node.properties, &blend_threshold_prop(idx))
                        .ok_or_else(|| {
                            format!(
                                "state '{state}': blend node {node_id} child `in_{idx}` has \
                                 no threshold (property `threshold_{idx}`)"
                            )
                        })?;
                    children.push((threshold, sub));
                }
                if children.len() < 2 {
                    return Err(format!(
                        "state '{state}': blend node {node_id} needs at least two wired children"
                    ));
                }
                children.sort_by(|a, b| a.0.total_cmp(&b.0));
                if children.windows(2).any(|w| w[0].0 == w[1].0) {
                    return Err(format!(
                        "state '{state}': blend node {node_id} has two children sharing a \
                         threshold"
                    ));
                }
                (POSE_PIN, PlanTree::Blend1D { param, children })
            }
            ANIM_BLEND2D_TYPE_ID => {
                let param_x = self.float_param(node, BLEND_PARAM_X_PROP)?;
                let param_y = self.float_param(node, BLEND_PARAM_Y_PROP)?;
                let mut children = Vec::new();
                for (idx, sub) in self.children(node_id)? {
                    let (Some(x), Some(y)) = (
                        float_prop(&node.properties, &blend_x_prop(idx)),
                        float_prop(&node.properties, &blend_y_prop(idx)),
                    ) else {
                        return Err(format!(
                            "state '{state}': blend node {node_id} child `in_{idx}` has no \
                             direction (properties `x_{idx}`/`y_{idx}`)"
                        ));
                    };
                    if x == 0.0 && y == 0.0 {
                        return Err(format!(
                            "state '{state}': blend node {node_id} child `in_{idx}` has a \
                             zero direction"
                        ));
                    }
                    children.push((y.atan2(x).rem_euclid(std::f32::consts::TAU), sub));
                }
                if children.len() < 2 {
                    return Err(format!(
                        "state '{state}': blend node {node_id} needs at least two wired children"
                    ));
                }
                children.sort_by(|a, b| a.0.total_cmp(&b.0));
                if children.windows(2).any(|w| w[0].0 == w[1].0) {
                    return Err(format!(
                        "state '{state}': blend node {node_id} has two children sharing a \
                         direction"
                    ));
                }
                (
                    POSE_PIN,
                    PlanTree::Blend2D {
                        param_x,
                        param_y,
                        children,
                    },
                )
            }
            // A reroute is a Pose pass-through.
            REROUTE_TYPE_ID => {
                let e = self
                    .region
                    .edges
                    .iter()
                    .find(|e| e.to_node == node_id && e.to_pin == REROUTE_IN)
                    .ok_or_else(|| {
                        format!("state '{state}': reroute {node_id} has nothing wired in")
                    })?;
                let (from, pin) = (e.from_node, e.from_pin.clone());
                (REROUTE_OUT, self.output(from, &pin)?)
            }
            other => {
                // RESULT as a source lands here too: it is a sink.
                return Err(format!(
                    "state '{state}': tree node {node_id} ('{other}') has no outputs"
                ));
            }
        };
        self.stack.pop();
        if from_pin != out_pin {
            return Err(format!(
                "state '{state}': tree node {node_id} ('{}') has no output '{from_pin}'",
                node.type_id
            ));
        }
        Ok(tree)
    }
}
