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

use super::machine::ParamValue;

// ---------------------------------------------------------------------------
// Node library slugs (stable identity, per the Task 40 identity rules)
// ---------------------------------------------------------------------------

/// The ENTRY node — a real node on the canvas, exactly one per machine. Its
/// single outgoing edge names the starting state.
pub const ANIM_ENTRY_TYPE_ID: &str = "anim_entry";
/// A State: either a leaf that plays the `.anim` clip its [`CLIP_PROP`]
/// names, or — when the document carries a region keyed by the state's id —
/// a blend tree evaluated recursively into one Pose. (Nested machines widen
/// this in a later slice.)
pub const ANIM_STATE_TYPE_ID: &str = "anim_state";
/// A Transition between two states, carrying blend duration and priority as
/// node data.
pub const ANIM_TRANSITION_TYPE_ID: &str = "anim_transition";
/// The Any State node: a source whose outgoing transitions apply from
/// whatever state is active — and the only transitions allowed to interrupt
/// a running crossfade (interruption rule v1). Several Any State nodes are
/// legal and equivalent; they exist for wire tidiness, not semantics.
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
            }
        }
        let mut out = Vec::new();
        walk(self, &mut out);
        out
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
    /// the document carries a region keyed by this state's node id.
    pub tree: PlanTree,
    /// Playback-rate multiplier on the state's clock.
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

/// Where a transition starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionFrom {
    /// Index into [`AnimGraphPlan::states`].
    State(usize),
    /// An Any State transition: applies from whatever state is active.
    AnyState,
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
    pub parameters: Vec<ParamDecl>,
    /// Play-once slots, sorted by node id — when several triggers are set at
    /// once, the first slot in this order takes the (single) channel.
    pub slots: Vec<PlanSlot>,
}

impl AnimGraphPlan {
    /// Deduplicated content-relative `.anim` paths this plan samples.
    pub fn clip_refs(&self) -> Vec<&str> {
        let mut refs: Vec<&str> = self
            .states
            .iter()
            .flat_map(|s| s.tree.clips())
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

/// Compile a `.animgraph` document into a plan.
///
/// Refusals are author errors, phrased against the node that caused them.
pub fn compile_anim_graph(doc: &GraphDoc) -> Result<AnimGraphPlan, String> {
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
    // non-empty region compiles it as a blend tree; a leaf state plays the
    // clip its `clip` property names.
    let mut states: Vec<PlanState> = Vec::new();
    for n in doc.nodes.iter().filter(|n| n.type_id == ANIM_STATE_TYPE_ID) {
        let name = n
            .title
            .clone()
            .unwrap_or_else(|| format!("State {}", n.id));
        let tree = match doc.regions.get(&n.id).filter(|r| !r.nodes.is_empty()) {
            Some(region) => compile_tree(region, &name, &parameters)?,
            None => {
                let clip = str_prop(&n.properties, CLIP_PROP)
                    .filter(|s| !s.trim().is_empty())
                    .ok_or_else(|| {
                        format!(
                            "state '{name}' names no clip (property `{CLIP_PROP}`) and has \
                             no blend tree"
                        )
                    })?;
                PlanTree::Clip(PlanClip {
                    clip: crate::engine::scripting::normalize_graph_path(clip),
                    clip_name: str_prop(&n.properties, CLIP_NAME_PROP)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string),
                })
            }
        };
        states.push(PlanState {
            node_id: n.id,
            name,
            tree,
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

    // Transitions: resolve both endpoints through their pins. A source may
    // be a state or an Any State node; a target is always a state.
    let any_state_ids: Vec<u64> = doc
        .nodes
        .iter()
        .filter(|n| n.type_id == ANIM_ANY_STATE_TYPE_ID)
        .map(|n| n.id)
        .collect();
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
        let from = match from_edge {
            Some(e) if any_state_ids.contains(&e.from_node) => TransitionFrom::AnyState,
            Some(e) => TransitionFrom::State(
                state_index(e.from_node)
                    .ok_or_else(|| format!("transition {} has no source state", n.id))?,
            ),
            None => return Err(format!("transition {} has no source state", n.id)),
        };
        let to = doc
            .edges
            .iter()
            .find(|e| e.from_node == n.id && e.from_pin == TRANSITION_TO_PIN)
            .and_then(|e| state_index(e.to_node))
            .ok_or_else(|| format!("transition {} has no target state", n.id))?;
        transitions.push(PlanTransition {
            node_id: n.id,
            from,
            to,
            rule: compile_rule(doc, n.id, &parameters)?,
            duration: float_prop(&n.properties, DURATION_PROP)
                .unwrap_or(0.0)
                .max(0.0),
            priority: match n.properties.get(PRIORITY_PROP) {
                Some(PropValue::Int(i)) => *i,
                _ => 0,
            },
        });
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

    Ok(AnimGraphPlan {
        states,
        transitions,
        entry,
        parameters,
        slots,
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
