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
/// A State: plays the `.anim` clip its [`CLIP_PROP`] names. (Blend trees and
/// nested machines widen this in later slices.)
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

    Ok(AnimGraphPlan {
        states,
        transitions,
        entry,
        parameters,
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
