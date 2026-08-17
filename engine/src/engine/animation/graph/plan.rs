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

use node_graph_types::{GraphDoc, GraphRealm, PinType, PropValue};

use super::machine::ParamValue;

// ---------------------------------------------------------------------------
// Node library slugs (stable identity, per the Task 40 identity rules)
// ---------------------------------------------------------------------------

/// The ENTRY node — a real node on the canvas, exactly one per machine. Its
/// single outgoing edge names the starting state.
pub const ANIM_ENTRY_TYPE_ID: &str = "anim_entry";
/// A State: plays the `.anim` clip its [`CLIP_PROP`] names. (Blend trees and
/// nested machines widen this in later slices.)
pub const ANIM_STATE_TYPE_ID: &str = "anim_state";
/// A Transition between two states, carrying blend duration and priority as
/// node data.
pub const ANIM_TRANSITION_TYPE_ID: &str = "anim_transition";

/// State pins: machine-topology flow in/out. (Not Pose wires — those belong
/// to blend trees inside a state.)
pub const STATE_IN_PIN: &str = "in";
pub const STATE_OUT_PIN: &str = "out";
/// Transition pins: `from` receives the source state's `out`; `to` feeds the
/// target state's `in`.
pub const TRANSITION_FROM_PIN: &str = "from";
pub const TRANSITION_TO_PIN: &str = "to";

/// State properties. `clip` is the content-relative `.anim` path (required);
/// `clip_name` picks a clip inside the container (default: the first);
/// `speed` is a playback-rate multiplier (default 1.0 — 0.0 holds the first
/// frame as a pose).
pub const CLIP_PROP: &str = "clip";
pub const CLIP_NAME_PROP: &str = "clip_name";
pub const SPEED_PROP: &str = "speed";

/// Transition properties. `duration` is the crossfade length in seconds
/// (default 0.0 = instant); `priority` orders evaluation when several rules
/// pass — **lower value wins**, ties broken by node id, so resolution is
/// deterministic.
pub const DURATION_PROP: &str = "duration";
pub const PRIORITY_PROP: &str = "priority";

/// Tracer-slice placeholder condition: the slug of a Bool parameter; the
/// transition fires while that parameter is true. Absent or empty means
/// always-true. The full rule-graph machinery (embedded boolean networks on
/// the transition) replaces this — it is document data only a hand-authored
/// tracer doc carries, never a supported long-term surface.
pub const WHEN_BOOL_PROP: &str = "when_bool";

// ---------------------------------------------------------------------------
// Plan (compiled form)
// ---------------------------------------------------------------------------

/// Parameter types gameplay may declare in this slice. Trigger
/// (consume-on-transition) arrives with the rule-graph slice — the machine
/// owns that statefulness, and it does not exist until rules do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimParamType {
    Float,
    Bool,
}

/// One declared parameter — the typed contract between gameplay and the
/// graph.
#[derive(Debug, Clone, PartialEq)]
pub struct ParamDecl {
    pub slug: String,
    pub ty: AnimParamType,
    pub default: ParamValue,
}

/// A compiled State: what to play and how fast.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanState {
    /// The document node this came from (error anchoring, editor viz later).
    pub node_id: u64,
    /// Author-facing name (node title, falling back to `State <id>`).
    pub name: String,
    /// Content-relative `.anim` path.
    pub clip: String,
    /// Clip inside the container; `None` = the first.
    pub clip_name: Option<String>,
    /// Playback-rate multiplier.
    pub speed: f32,
}

/// The tracer's condition forms. Replaced wholesale by rule graphs.
#[derive(Debug, Clone, PartialEq)]
pub enum TransitionCondition {
    /// An unwired condition reads as always-true (the "hollow socket dot").
    Always,
    /// Fires while the named Bool parameter is true.
    BoolParam(String),
}

/// A compiled Transition between two state indices.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanTransition {
    pub node_id: u64,
    /// Index into [`AnimGraphPlan::states`].
    pub from: usize,
    pub to: usize,
    /// Crossfade duration in seconds (0.0 = instant switch).
    pub duration: f32,
    /// Lower value wins; ties break by node id.
    pub priority: i32,
    pub condition: TransitionCondition,
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
}

impl AnimGraphPlan {
    /// Deduplicated content-relative `.anim` paths this plan samples.
    pub fn clip_refs(&self) -> Vec<&str> {
        let mut refs: Vec<&str> = self.states.iter().map(|s| s.clip.as_str()).collect();
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

    // States, in document order (index = plan identity).
    let mut states: Vec<PlanState> = Vec::new();
    for n in doc.nodes.iter().filter(|n| n.type_id == ANIM_STATE_TYPE_ID) {
        let name = n
            .title
            .clone()
            .unwrap_or_else(|| format!("State {}", n.id));
        let clip = str_prop(&n.properties, CLIP_PROP)
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| format!("state '{name}' names no clip (property `{CLIP_PROP}`)"))?;
        states.push(PlanState {
            node_id: n.id,
            name,
            clip: crate::engine::scripting::normalize_graph_path(clip),
            clip_name: str_prop(&n.properties, CLIP_NAME_PROP)
                .filter(|s| !s.is_empty())
                .map(str::to_string),
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

    // Parameters, from the document's variables.
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
            ref other => {
                return Err(format!(
                    "parameter '{}': {other:?} is not an animation parameter type \
                     (Float and Bool in this slice; Trigger arrives with rule graphs)",
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

    // Transitions: resolve both endpoints through their pins.
    let mut transitions: Vec<PlanTransition> = Vec::new();
    for n in doc
        .nodes
        .iter()
        .filter(|n| n.type_id == ANIM_TRANSITION_TYPE_ID)
    {
        let from = doc
            .edges
            .iter()
            .find(|e| e.to_node == n.id && e.to_pin == TRANSITION_FROM_PIN)
            .and_then(|e| state_index(e.from_node))
            .ok_or_else(|| format!("transition {} has no source state", n.id))?;
        let to = doc
            .edges
            .iter()
            .find(|e| e.from_node == n.id && e.from_pin == TRANSITION_TO_PIN)
            .and_then(|e| state_index(e.to_node))
            .ok_or_else(|| format!("transition {} has no target state", n.id))?;
        let condition = match str_prop(&n.properties, WHEN_BOOL_PROP).filter(|s| !s.is_empty()) {
            None => TransitionCondition::Always,
            Some(slug) => {
                let declared = parameters
                    .iter()
                    .find(|p| p.slug == slug)
                    .ok_or_else(|| {
                        format!("transition {}: parameter '{slug}' is not declared", n.id)
                    })?;
                if declared.ty != AnimParamType::Bool {
                    return Err(format!(
                        "transition {}: parameter '{slug}' is not a Bool",
                        n.id
                    ));
                }
                TransitionCondition::BoolParam(slug.to_string())
            }
        };
        transitions.push(PlanTransition {
            node_id: n.id,
            from,
            to,
            duration: float_prop(&n.properties, DURATION_PROP)
                .unwrap_or(0.0)
                .max(0.0),
            priority: match n.properties.get(PRIORITY_PROP) {
                Some(PropValue::Int(i)) => *i,
                _ => 0,
            },
            condition,
        });
    }
    // Evaluation order is the sort order: lower priority value first, node id
    // as the deterministic tiebreak.
    transitions.sort_by_key(|t| (t.priority, t.node_id));

    Ok(AnimGraphPlan {
        states,
        transitions,
        entry,
        parameters,
    })
}
