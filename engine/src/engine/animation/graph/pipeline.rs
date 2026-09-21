//! The animation pipeline root (Task 41.7): the constrained pose graph that
//! assembles a character's final pose.
//!
//! An `.animgraph`'s top level holds two node families in one flat list.
//! The **machine** family (entry / states / aliases / transitions) is the
//! state-machine canvas; the **pipeline** family is the root canvas: pose
//! sources (the inline State Machine, nested machines, root clips) →
//! per-bone layers and play-once overlays (local space) → IK chains (model
//! space) → Output Pose. This module owns the pipeline half — its type ids
//! and properties, the compiled form ([`PlanPipeline`] and friends on
//! [`AnimGraphPlan`]), the compiler that walks back from Output Pose, and
//! the structural upgrade ([`upgrade_pipeline_root`]) that gives a
//! machine-only document the implicit pipeline it always had.
//!
//! Slot and IK-chain *collection* (node → [`PlanSlot`] / [`PlanIkChain`])
//! lives here too, because those nodes are pipeline family; their compiled
//! lists keep the pre-41.7 contents and order (`plan.slots` by node id,
//! `plan.ik_chains` globally by node id) — the pipeline's *wire* orders are
//! [`PlanPipeline::slot_order`] / [`PlanPipeline::ik_order`].

use std::collections::BTreeSet;
use std::sync::Arc;

use node_graph_types::{Edge, GraphDoc, NodeInst, PropValue};

use super::plan::{
    compile_doc, float_prop, speed_prop, str_prop, AnchoredWarning, AnimGraphLoader, AnimGraphPlan,
    AnimParamType, ParamDecl, PlanClip, PlanFootPlacement, PlanIkChain, PlanIkSolver, PlanSlot,
    PlanState, PoseSource, ANIM_ANY_STATE_TYPE_ID, ANIM_CLIP_TYPE_ID, ANIM_ENTRY_TYPE_ID,
    ANIM_IK_CHAIN_TYPE_ID, ANIM_PLAY_ONCE_TYPE_ID, ANIM_STATE_ALIAS_TYPE_ID, ANIM_STATE_TYPE_ID,
    ANIM_TRANSITION_TYPE_ID, CLIP_NAME_PROP, CLIP_PROP, GRAPH_PROP, IK_ANKLE_OFFSET_PROP,
    IK_AXIS_X_PROP, IK_AXIS_Y_PROP, IK_AXIS_Z_PROP, IK_BONES_PROP, IK_FOOT_PROP,
    IK_MAX_ANGLE_PROP, IK_PELVIS_PROP, IK_SOLVER_LOOK_AT, IK_SOLVER_PROP, IK_SOLVER_TWO_BONE,
    IK_WEIGHT_PARAM_PROP, POSE_PIN, SLOT_FADE_IN_PROP, SLOT_FADE_OUT_PROP, SLOT_TRIGGER_PROP,
};

// ---------------------------------------------------------------------------
// Node library slugs (pipeline family)
// ---------------------------------------------------------------------------

/// State Machine: a pose source. Prop [`GRAPH_PROP`] empty = *this*
/// document's inline machine (exactly one such node per document); a path =
/// a nested `.animgraph` whose machine runs as its own instance. Out:
/// [`POSE_PIN`].
pub const ANIM_PIPE_MACHINE_TYPE_ID: &str = "anim_pipe_machine";
/// Layered Blend Per Bone: blends `Layer` over `Base` on the bones under the
/// mask roots [`MASK_BONES_PROP`] names, weighted by the Float parameter
/// [`LAYER_WEIGHT_PARAM_PROP`] names. In: [`LAYER_BASE_PIN`],
/// [`LAYER_LAYER_PIN`]; out: [`POSE_PIN`].
pub const ANIM_PIPE_LAYER_TYPE_ID: &str = "anim_pipe_layer";
/// Output Pose: the pipeline's single sink. In: [`PIPE_IN_PIN`].
pub const ANIM_PIPE_OUTPUT_TYPE_ID: &str = "anim_pipe_output";

/// The single pose input of Play Once, IK Chain and Output Pose nodes.
pub const PIPE_IN_PIN: &str = "in";
/// A Layer's two inputs. Evaluation order is Base then Layer (published — it
/// is also the slot arbitration order across sibling branches).
pub const LAYER_BASE_PIN: &str = "base";
pub const LAYER_LAYER_PIN: &str = "layer";

/// Mask roots (`Str`, comma-separated bone names) on a Layer (required) and a
/// Play Once (optional; empty = whole body). Same key as [`IK_BONES_PROP`].
pub const MASK_BONES_PROP: &str = "bones";
/// Layer property (`Str`, required): the declared **Float** parameter that
/// weights the layer (clamped to `[0, 1]` at runtime).
pub const LAYER_WEIGHT_PARAM_PROP: &str = "weight_param";
/// Layer property (`Bool`, default true): the mask covers the listed roots
/// themselves, not only their descendants.
pub const LAYER_INCLUDE_ROOT_PROP: &str = "include_root";

/// Annotation `family` tags: which canvas a comment/group belongs to. An
/// untagged annotation is [`FAMILY_MACHINE`].
pub const FAMILY_MACHINE: &str = "machine";
pub const FAMILY_PIPELINE: &str = "pipeline";

/// The machine family (the legacy Any State included — it upgrades in place).
pub const MACHINE_NODE_TYPES: [&str; 5] = [
    ANIM_ENTRY_TYPE_ID,
    ANIM_STATE_TYPE_ID,
    ANIM_STATE_ALIAS_TYPE_ID,
    ANIM_TRANSITION_TYPE_ID,
    ANIM_ANY_STATE_TYPE_ID,
];
/// The pipeline family.
pub const PIPELINE_NODE_TYPES: [&str; 6] = [
    ANIM_PIPE_MACHINE_TYPE_ID,
    ANIM_CLIP_TYPE_ID,
    ANIM_PIPE_LAYER_TYPE_ID,
    ANIM_PLAY_ONCE_TYPE_ID,
    ANIM_IK_CHAIN_TYPE_ID,
    ANIM_PIPE_OUTPUT_TYPE_ID,
];
/// The pipeline-*only* types: their presence means the document already
/// carries a pipeline root. Slots and IK chains are not in this set — they
/// were legal on the pre-41.7 machine canvas, so they cannot mark a document
/// as upgraded.
const PIPELINE_ROOT_TYPES: [&str; 4] = [
    ANIM_PIPE_MACHINE_TYPE_ID,
    ANIM_CLIP_TYPE_ID,
    ANIM_PIPE_LAYER_TYPE_ID,
    ANIM_PIPE_OUTPUT_TYPE_ID,
];

/// Canvas spacing of the row [`upgrade_pipeline_root`] lays out.
pub const PIPELINE_ROW_STEP: f32 = 260.0;

pub fn is_machine_node(type_id: &str) -> bool {
    MACHINE_NODE_TYPES.contains(&type_id)
}

pub fn is_pipeline_node(type_id: &str) -> bool {
    PIPELINE_NODE_TYPES.contains(&type_id)
}

// ---------------------------------------------------------------------------
// Plan (compiled form)
// ---------------------------------------------------------------------------

/// A State Machine node's compiled source.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanMachineRef {
    pub node_id: u64,
    pub source: MachineSource,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MachineSource {
    /// This document's machine (`plan.states` / `transitions` / `entry`).
    Inline,
    /// A nested `.animgraph`'s machine, run as its own instance. Only its
    /// *machine* is consumed; its pipeline is never compiled.
    Nested {
        graph: String,
        plan: Arc<AnimGraphPlan>,
    },
}

/// A root Clip node: a looping pose source with its own clock and event
/// track (the runtime's `RootClipClock`).
#[derive(Debug, Clone, PartialEq)]
pub struct PlanRootClip {
    pub clip: PlanClip,
    /// Playback-rate multiplier (default 1.0).
    pub speed: f32,
    pub node_id: u64,
}

/// A symbolic bone mask: resolved per skeleton at arm time (1 on each root
/// — unless `include_root` is false — and every descendant, 0 elsewhere).
#[derive(Debug, Clone, PartialEq)]
pub struct PlanMask {
    pub roots: Vec<String>,
    pub include_root: bool,
}

/// The local-space stage, as a tree rooted at Output Pose's input (IK chains
/// are not in it — they follow in [`PlanPipeline::ik_order`]).
#[derive(Debug, Clone, PartialEq)]
pub enum PlanPose {
    /// Index into [`AnimGraphPlan::machines`].
    Machine(usize),
    /// Index into [`AnimGraphPlan::root_clips`].
    Clip(usize),
    /// Layered blend per bone: `base`, then `layer` blended over it under
    /// `mask × weight_param`.
    Layer {
        base: Box<PlanPose>,
        layer: Box<PlanPose>,
        mask: PlanMask,
        weight_param: String,
        node_id: u64,
    },
    /// A play-once overlay: `input`, then — if the single channel's playing
    /// slot is *this* one — the slot's clip over it (whole body when `mask`
    /// is `None`). One slot per node. `node_id` is the Play Once node, or,
    /// for a slot lifted out of a nested document, the node that nests it
    /// (a state or a State Machine) — lifted overlays never carry a mask.
    Overlay {
        input: Box<PlanPose>,
        /// Index into [`AnimGraphPlan::slots`].
        slot: usize,
        mask: Option<PlanMask>,
        node_id: u64,
    },
}

/// The compiled pipeline: the local-space tree plus the wire orders of the
/// slot channel and the IK stage (indices into `plan.slots` /
/// `plan.ik_chains`; lifted nested entries appended after the host's).
#[derive(Debug, Clone, PartialEq)]
pub struct PlanPipeline {
    pub root: PlanPose,
    /// "First triggered wins" order of the single overlay channel.
    pub slot_order: Vec<usize>,
    /// IK application order, each chain seeing the previous one's result.
    pub ik_order: Vec<usize>,
    /// `true` when the compiler produced the orders: an empty order then
    /// means "nothing reachable" and must stay empty. `false` on hand-built
    /// plans (tests, the blend-space preview) and nested plans, whose empty
    /// orders fall back to index order (R13).
    pub compiled: bool,
}

impl PlanPipeline {
    /// Slot arbitration order: the compiled wire order, or index order for
    /// an uncompiled plan.
    pub fn slot_index(&self, k: usize) -> usize {
        if self.compiled { self.slot_order[k] } else { k }
    }

    pub fn slot_count(&self, uncompiled_len: usize) -> usize {
        if self.compiled { self.slot_order.len() } else { uncompiled_len }
    }

    /// IK application order, same rule.
    pub fn ik_index(&self, k: usize) -> usize {
        if self.compiled { self.ik_order[k] } else { k }
    }

    pub fn ik_count(&self, uncompiled_len: usize) -> usize {
        if self.compiled { self.ik_order.len() } else { uncompiled_len }
    }
}

impl Default for PlanPipeline {
    /// The inline machine straight to Output, nothing in between — what an
    /// empty plan (and a nested plan, whose pipeline is never compiled)
    /// carries.
    fn default() -> Self {
        Self {
            root: PlanPose::Machine(0),
            slot_order: Vec::new(),
            ik_order: Vec::new(),
            compiled: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Structural upgrade (D5)
// ---------------------------------------------------------------------------

/// The D5 trigger: machine nodes present and no pipeline-only node at all.
/// A document that has any pipeline-only node but no Output Pose is a v4
/// document whose author deleted it — a compile refusal, never a silent
/// re-upgrade.
pub fn needs_pipeline_root(doc: &GraphDoc) -> bool {
    doc.nodes.iter().any(|n| is_machine_node(&n.type_id))
        && !doc
            .nodes
            .iter()
            .any(|n| PIPELINE_ROOT_TYPES.contains(&n.type_id.as_str()))
}

/// Give a machine-only document its implicit pipeline root: an inline State
/// Machine, the existing play-once slots (node-id order) then IK chains
/// (node-id order) chained after it, and an Output Pose — the exact order
/// the pre-41.7 runtime applied. The pipeline nodes are laid out on a row
/// (their old positions were machine-canvas coordinates); machine nodes,
/// regions, variables and ids are untouched. Comments/groups whose rect
/// encloses only moved nodes are tagged [`FAMILY_PIPELINE`] and move with
/// them; the rest stay machine-scoped. `true` if the document changed.
///
/// Pure: the caller decides what to do with the result (the compiler runs it
/// on a private copy; the editor marks the document dirty).
pub fn upgrade_pipeline_root(doc: &mut GraphDoc) -> bool {
    if !needs_pipeline_root(doc) {
        return false;
    }
    let ids_of = |type_id: &str| {
        let mut ids: Vec<u64> = doc
            .nodes
            .iter()
            .filter(|n| n.type_id == type_id)
            .map(|n| n.id)
            .collect();
        ids.sort_unstable();
        ids
    };
    let mut moved = ids_of(ANIM_PLAY_ONCE_TYPE_ID);
    moved.extend(ids_of(ANIM_IK_CHAIN_TYPE_ID));
    let sm = doc.next_node_id();
    let out = sm + 1;
    let row = |i: usize| [i as f32 * PIPELINE_ROW_STEP, 0.0];

    // Annotations are judged against the *old* positions.
    let nodes = &doc.nodes;
    let retag = |rect: &mut [f32; 4], family: &mut Option<String>| {
        let inside = |p: [f32; 2]| {
            p[0] >= rect[0]
                && p[1] >= rect[1]
                && p[0] <= rect[0] + rect[2]
                && p[1] <= rect[1] + rect[3]
        };
        let enclosed: Vec<&NodeInst> = nodes.iter().filter(|n| inside(n.position)).collect();
        if enclosed.is_empty() || !enclosed.iter().all(|n| moved.contains(&n.id)) {
            return;
        }
        *family = Some(FAMILY_PIPELINE.to_string());
        let first = enclosed[0];
        let slot = moved.iter().position(|id| *id == first.id).unwrap_or(0);
        let new = row(slot + 1);
        rect[0] += new[0] - first.position[0];
        rect[1] += new[1] - first.position[1];
    };
    for c in doc.comments.iter_mut() {
        retag(&mut c.rect, &mut c.family);
    }
    for g in doc.groups.iter_mut() {
        retag(&mut g.rect, &mut g.family);
    }

    for (i, id) in moved.iter().enumerate() {
        if let Some(n) = doc.node_mut(*id) {
            n.position = row(i + 1);
        }
    }
    let node = |id: u64, type_id: &str, position: [f32; 2]| NodeInst {
        id,
        type_id: type_id.to_string(),
        type_version: 1,
        position,
        properties: Default::default(),
        subgraph: None,
        tint: None,
        title: None,
    };
    doc.nodes.push(node(sm, ANIM_PIPE_MACHINE_TYPE_ID, row(0)));
    doc.nodes
        .push(node(out, ANIM_PIPE_OUTPUT_TYPE_ID, row(moved.len() + 1)));
    let mut prev = sm;
    for id in moved.iter().copied().chain([out]) {
        doc.edges.push(Edge {
            from_node: prev,
            from_pin: POSE_PIN.to_string(),
            to_node: id,
            to_pin: PIPE_IN_PIN.to_string(),
        });
        prev = id;
    }
    true
}

// ---------------------------------------------------------------------------
// Slot / IK chain collection (host nodes → plan entries)
// ---------------------------------------------------------------------------

/// The document's own play-once slots, sorted by node id (the pre-41.7
/// channel order and the plan's index space).
pub(super) fn compile_slots(doc: &GraphDoc, parameters: &[ParamDecl]) -> Result<Vec<PlanSlot>, String> {
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
        let speed = speed_prop(&n.properties, &format!("play-once slot '{name}'"))?;
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
            speed,
            fade_in: float_prop(&n.properties, SLOT_FADE_IN_PROP)
                .unwrap_or(0.0)
                .max(0.0),
            fade_out: float_prop(&n.properties, SLOT_FADE_OUT_PROP)
                .unwrap_or(0.0)
                .max(0.0),
        });
    }
    slots.sort_by_key(|s| s.node_id);
    Ok(slots)
}

/// The document's own IK chains, in document order (sorted globally by node
/// id once nested chains have joined — [`finish_ik_chains`]). Bone existence
/// is an arm-time check (the compiler never sees a skeleton); everything
/// knowable from the document refuses here, anchored on the chain.
pub(super) fn compile_ik_chains(
    doc: &GraphDoc,
    parameters: &[ParamDecl],
) -> Result<Vec<PlanIkChain>, String> {
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
        let bones = bone_list(n, IK_BONES_PROP);
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
        let weight_param = float_param(n, IK_WEIGHT_PARAM_PROP, parameters, &|| {
            format!("IK chain '{name}'")
        })?;
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
    Ok(ik_chains)
}

/// A comma-separated bone list property, trimmed, empties dropped.
fn bone_list(n: &NodeInst, key: &str) -> Vec<String> {
    str_prop(&n.properties, key)
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// A `Str` property naming a declared Float parameter, refused against the
/// node's display name ("IK chain 'foot_l'", "layer 'Aim'").
fn float_param(
    n: &NodeInst,
    key: &str,
    parameters: &[ParamDecl],
    who: &dyn Fn() -> String,
) -> Result<String, String> {
    let slug = match n.properties.get(key) {
        Some(PropValue::Str(s)) if !s.is_empty() => s.clone(),
        _ => {
            return Err(format!(
                "{} names no weight parameter (property `{key}`)",
                who()
            ))
        }
    };
    match parameters.iter().find(|p| p.slug == slug) {
        Some(p) if p.ty == AnimParamType::Float => Ok(slug),
        Some(_) => Err(format!("{}: parameter '{slug}' is not a Float", who())),
        None => Err(format!("{}: parameter '{slug}' is not declared", who())),
    }
}

/// Sort the merged chain list by node id (stable: the host wins ties on
/// cross-document id collisions) and check what one skeleton requires:
/// unique names (they key `IkTargets`) and one pelvis bone.
pub(super) fn finish_ik_chains(ik_chains: &mut [PlanIkChain]) -> Result<(), String> {
    ik_chains.sort_by_key(|c| c.node_id);
    for (i, c) in ik_chains.iter().enumerate() {
        if ik_chains[..i].iter().any(|o| o.name == c.name) {
            return Err(format!(
                "two IK chains are named '{}' — chain names key the IkTargets \
                 component, so they must be unique",
                c.name
            ));
        }
    }
    let mut pelvis: Option<(&str, &str)> = None;
    for c in ik_chains.iter() {
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
    Ok(())
}

// ---------------------------------------------------------------------------
// Assembly: lifting + the pipeline compiler
// ---------------------------------------------------------------------------

/// Everything a document contributes beyond its machine: the final slot and
/// chain lists (host + lifted), the pipeline, and what the pipeline's nested
/// machines add to the blackboard.
pub(super) struct Assembly {
    pub slots: Vec<PlanSlot>,
    pub ik_chains: Vec<PlanIkChain>,
    pub machines: Vec<PlanMachineRef>,
    pub inline_machine: usize,
    pub root_clips: Vec<PlanRootClip>,
    pub pipeline: PlanPipeline,
    pub nested_params: Vec<(String, ParamDecl)>,
    pub warnings: Vec<AnchoredWarning>,
}

/// One nested document's contribution to the host, with provenance.
struct Lifted {
    /// The host node that nests it (a state or a State Machine node).
    node_id: u64,
    /// "state 'Loco'" / "state machine 'Upper'".
    who: String,
    graph: String,
    plan: Arc<AnimGraphPlan>,
}

/// Assemble a document's slots, chains and pipeline. `root` = this is the
/// document being compiled (its pipeline is walked and validated); a nested
/// document only lifts its state-nested slots/chains and gets the identity
/// orders — its pipeline nodes are ignored (D3.7).
pub(super) fn assemble(
    doc: &GraphDoc,
    stack: &mut Vec<String>,
    load: &dyn AnimGraphLoader,
    parameters: &[ParamDecl],
    states: &[PlanState],
    root: bool,
) -> Result<Assembly, String> {
    let mut slots = compile_slots(doc, parameters)?;
    let mut ik_chains = compile_ik_chains(doc, parameters)?;
    let host_slot_n = slots.len();

    // Lifting order (D3.7): nested in state order, then the pipeline's
    // nested machines in walk order — host entries always come first.
    let mut lifted: Vec<Lifted> = states
        .iter()
        .filter_map(|s| match &s.source {
            PoseSource::Machine { graph, plan } => Some(Lifted {
                node_id: s.node_id,
                who: format!("state '{}'", s.name),
                graph: graph.clone(),
                plan: Arc::clone(plan),
            }),
            _ => None,
        })
        .collect();

    let walk = if root {
        let cx = PipeCx {
            doc,
            parameters,
            stack,
            load,
            host_slots: &slots,
            machines: Vec::new(),
            nested_params: Vec::new(),
            root_clips: Vec::new(),
            slot_walk: Vec::new(),
            reachable: BTreeSet::new(),
            visiting: Vec::new(),
        };
        Some(cx.compile(&mut lifted)?)
    } else {
        None
    };

    // Lift: exact duplicates (one graph nested twice) drop; a nested chain
    // or slot the host already has is the host's.
    let mut lifted_slots: Vec<(u64, PlanSlot)> = Vec::new();
    let mut lifted_chains: Vec<PlanIkChain> = Vec::new();
    let mut warnings = Vec::new();
    for l in &lifted {
        for s in &l.plan.slots {
            if !slots.contains(s) {
                slots.push(s.clone());
                lifted_slots.push((l.node_id, s.clone()));
            }
        }
        for c in &l.plan.ik_chains {
            if !ik_chains.contains(c) {
                ik_chains.push(c.clone());
                lifted_chains.push(c.clone());
            }
        }
        if root && !(l.plan.slots.is_empty() && l.plan.ik_chains.is_empty()) {
            let names = |what: &str, names: Vec<&str>| match names.len() {
                0 => None,
                n => Some(format!(
                    "{what}{} {}",
                    if n == 1 { "" } else { "s" },
                    names.join(", ")
                )),
            };
            let parts: Vec<String> = [
                names(
                    "play-once slot",
                    l.plan.slots.iter().map(|s| s.name.as_str()).collect(),
                ),
                names(
                    "IK chain",
                    l.plan.ik_chains.iter().map(|c| c.name.as_str()).collect(),
                ),
            ]
            .into_iter()
            .flatten()
            .collect();
            warnings.push(AnchoredWarning {
                node_id: l.node_id,
                message: format!(
                    "{} nests '{}': its {} apply after this document's own",
                    l.who,
                    l.graph,
                    parts.join(" and ")
                ),
            });
        }
    }
    finish_ik_chains(&mut ik_chains)?;

    let Some(walk) = walk else {
        return Ok(Assembly {
            pipeline: PlanPipeline {
                root: PlanPose::Machine(0),
                slot_order: (0..slots.len()).collect(),
                ik_order: (0..ik_chains.len()).collect(),
                compiled: true,
            },
            slots,
            ik_chains,
            machines: Vec::new(),
            inline_machine: 0,
            root_clips: Vec::new(),
            nested_params: Vec::new(),
            warnings: Vec::new(),
        });
    };
    let Walk {
        mut root,
        ik_walk,
        inline_machine,
        machines,
        nested_params,
        root_clips,
        slot_walk,
        reachable,
    } = walk;
    let chain_index = |c: &PlanIkChain| -> usize {
        ik_chains
            .iter()
            .position(|o| o == c)
            .expect("every chain is in the merged list")
    };

    // Unreachable pipeline nodes are ignored, and said so.
    for n in doc.nodes.iter().filter(|n| is_pipeline_node(&n.type_id)) {
        if !reachable.contains(&n.id) {
            let kind = kind_of(&n.type_id).expect("pipeline node");
            warnings.push(AnchoredWarning {
                node_id: n.id,
                message: format!(
                    "{} '{}' is not connected to Output Pose and is ignored",
                    kind.word(),
                    display_name(n, kind)
                ),
            });
        }
    }
    warnings.sort_by_key(|w| w.node_id);

    // Lifted slots join the channel at the host's last Play Once (walk
    // order), else at the end of the local-space stage — whole body, with
    // the nesting node as provenance.
    let lifted_overlays: Vec<(usize, u64)> = lifted_slots
        .iter()
        .map(|(node_id, s)| {
            let i = host_slot_n
                + slots[host_slot_n..]
                    .iter()
                    .position(|o| o == s)
                    .expect("lifted slot is in the merged list");
            (i, *node_id)
        })
        .collect();
    if !lifted_overlays.is_empty() {
        let target = if has_overlay(&root) {
            last_overlay_mut(&mut root).expect("has an overlay")
        } else {
            &mut root
        };
        let mut inner = std::mem::replace(target, PlanPose::Machine(0));
        for (slot, node_id) in &lifted_overlays {
            inner = PlanPose::Overlay {
                input: Box::new(inner),
                slot: *slot,
                mask: None,
                node_id: *node_id,
            };
        }
        *target = inner;
    }
    let slot_order: Vec<usize> = slot_walk
        .into_iter()
        .chain(lifted_overlays.iter().map(|(i, _)| *i))
        .collect();
    let host_chain = |id: u64| {
        ik_chains
            .iter()
            .find(|c| c.node_id == id && lifted_chains.iter().all(|l| l != *c))
            .expect("every wired IK chain compiled")
    };
    let mut ik_order: Vec<usize> = ik_walk.iter().map(|id| chain_index(host_chain(*id))).collect();
    for c in &lifted_chains {
        let i = chain_index(c);
        if !ik_order.contains(&i) {
            ik_order.push(i);
        }
    }

    Ok(Assembly {
        pipeline: PlanPipeline {
            root,
            slot_order,
            ik_order,
            compiled: true,
        },
        slots,
        ik_chains,
        machines,
        inline_machine,
        root_clips,
        nested_params,
        warnings,
    })
}

fn has_overlay(p: &PlanPose) -> bool {
    match p {
        PlanPose::Overlay { .. } => true,
        PlanPose::Layer { base, layer, .. } => has_overlay(layer) || has_overlay(base),
        _ => false,
    }
}

/// The last overlay in evaluation (post-) order — the one lifted slots
/// wrap. Base is evaluated before Layer, so the Layer branch is searched
/// first.
fn last_overlay_mut(p: &mut PlanPose) -> Option<&mut PlanPose> {
    match p {
        PlanPose::Overlay { .. } => Some(p),
        PlanPose::Layer { base, layer, .. } => {
            if has_overlay(layer) {
                last_overlay_mut(layer)
            } else {
                last_overlay_mut(base)
            }
        }
        _ => None,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Machine,
    Clip,
    Layer,
    PlayOnce,
    Ik,
    Output,
}

fn kind_of(type_id: &str) -> Option<Kind> {
    Some(match type_id {
        ANIM_PIPE_MACHINE_TYPE_ID => Kind::Machine,
        ANIM_CLIP_TYPE_ID => Kind::Clip,
        ANIM_PIPE_LAYER_TYPE_ID => Kind::Layer,
        ANIM_PLAY_ONCE_TYPE_ID => Kind::PlayOnce,
        ANIM_IK_CHAIN_TYPE_ID => Kind::Ik,
        ANIM_PIPE_OUTPUT_TYPE_ID => Kind::Output,
        _ => return None,
    })
}

impl Kind {
    /// The refusal prefix word ("layer 'Aim': …").
    fn word(self) -> &'static str {
        match self {
            Kind::Machine => "state machine",
            Kind::Clip => "clip",
            Kind::Layer => "layer",
            Kind::PlayOnce => "play-once slot",
            Kind::Ik => "IK chain",
            Kind::Output => "Output Pose",
        }
    }

    fn inputs(self) -> &'static [&'static str] {
        match self {
            Kind::Machine | Kind::Clip => &[],
            Kind::Layer => &[LAYER_BASE_PIN, LAYER_LAYER_PIN],
            Kind::PlayOnce | Kind::Ik | Kind::Output => &[PIPE_IN_PIN],
        }
    }
}

/// The name the compiler phrases refusals with: the node title, else the
/// typed fallback — the same one the slot / IK compilers use, so one node
/// is named one way everywhere.
fn display_name(n: &NodeInst, kind: Kind) -> String {
    if let Some(t) = n.title.as_deref().filter(|t| !t.trim().is_empty()) {
        return t.to_string();
    }
    match kind {
        Kind::Machine => "State Machine".to_string(),
        Kind::Output => "Output Pose".to_string(),
        Kind::Clip => format!("Clip {}", n.id),
        Kind::Layer => format!("Layer {}", n.id),
        Kind::PlayOnce => format!("Slot {}", n.id),
        Kind::Ik => format!("IK {}", n.id),
    }
}

/// What the walk from Output Pose produced.
struct Walk {
    root: PlanPose,
    /// Wired IK chain node ids in application order.
    ik_walk: Vec<u64>,
    inline_machine: usize,
    machines: Vec<PlanMachineRef>,
    nested_params: Vec<(String, ParamDecl)>,
    root_clips: Vec<PlanRootClip>,
    /// Host slot indices in walk order.
    slot_walk: Vec<usize>,
    reachable: BTreeSet<u64>,
}

struct PipeCx<'a> {
    doc: &'a GraphDoc,
    parameters: &'a [ParamDecl],
    stack: &'a mut Vec<String>,
    load: &'a dyn AnimGraphLoader,
    host_slots: &'a [PlanSlot],
    machines: Vec<PlanMachineRef>,
    nested_params: Vec<(String, ParamDecl)>,
    root_clips: Vec<PlanRootClip>,
    /// Host slot indices in walk order.
    slot_walk: Vec<usize>,
    reachable: BTreeSet<u64>,
    /// Nodes on the current path — a wire back into one is a loop.
    visiting: Vec<u64>,
}

impl<'a> PipeCx<'a> {
    /// D3.1–D3.5: hygiene over every wire, then the walk back from Output.
    fn compile(mut self, lifted: &mut Vec<Lifted>) -> Result<Walk, String> {
        self.check_edges()?;

        let doc = self.doc;
        let mut outputs = doc
            .nodes
            .iter()
            .filter(|n| n.type_id == ANIM_PIPE_OUTPUT_TYPE_ID);
        let output = outputs
            .next()
            .ok_or_else(|| "an animation graph needs an Output Pose node".to_string())?;
        let extra = outputs.count();
        if extra > 0 {
            return Err(format!(
                "an animation graph has exactly one Output Pose node (found {})",
                extra + 1
            ));
        }
        let inline: Vec<&NodeInst> = doc
            .nodes
            .iter()
            .filter(|n| n.type_id == ANIM_PIPE_MACHINE_TYPE_ID && nested_graph(n).is_none())
            .collect();
        match inline.len() {
            0 => {
                return Err("an animation graph needs an inline State Machine node (a State \
                            Machine whose `graph` is empty)"
                    .to_string())
            }
            1 => {}
            _ => return Err("an animation graph has at most one inline State Machine node".into()),
        }
        let inline_id = inline[0].id;

        // The model-space stage: IK chains, nearest Output first.
        self.reachable.insert(output.id);
        self.visiting.push(output.id);
        let mut cur = self
            .input(output, PIPE_IN_PIN)
            .ok_or_else(|| "the Output Pose node has nothing wired in".to_string())?;
        let mut ik_walk = Vec::new();
        loop {
            let (n, kind) = self.node(cur)?;
            if self.visiting.contains(&cur) {
                return Err(format!(
                    "the pipeline loops through '{}'",
                    display_name(n, kind)
                ));
            }
            if kind != Kind::Ik {
                break;
            }
            self.reachable.insert(cur);
            self.visiting.push(cur);
            ik_walk.push(cur);
            let name = display_name(n, kind);
            cur = self.wired(n, kind, &name, PIPE_IN_PIN)?;
        }
        ik_walk.reverse();

        // The local-space stage.
        let root = self.build(cur, "Output Pose")?;
        if !self.reachable.contains(&inline_id) {
            return Err("the inline State Machine node is not wired to Output Pose".to_string());
        }
        let inline_machine = self
            .machines
            .iter()
            .position(|m| m.node_id == inline_id)
            .expect("reachable inline machine compiled");
        for m in &self.machines {
            if let MachineSource::Nested { graph, plan } = &m.source {
                let n = doc.node(m.node_id).expect("machine node");
                lifted.push(Lifted {
                    node_id: m.node_id,
                    who: format!("state machine '{}'", display_name(n, Kind::Machine)),
                    graph: graph.clone(),
                    plan: Arc::clone(plan),
                });
            }
        }
        Ok(Walk {
            root,
            ik_walk,
            inline_machine,
            machines: self.machines,
            nested_params: self.nested_params,
            root_clips: self.root_clips,
            slot_walk: self.slot_walk,
            reachable: self.reachable,
        })
    }

    /// D1 / D3.4 over every wire: no cross-family wires, pins exist, one
    /// wire per input, one consumer per output.
    fn check_edges(&self) -> Result<(), String> {
        let doc = self.doc;
        let label = |n: &NodeInst| match kind_of(&n.type_id) {
            Some(k) => display_name(n, k),
            None => n
                .title
                .clone()
                .filter(|t| !t.trim().is_empty())
                .unwrap_or_else(|| format!("node {}", n.id)),
        };
        let mut pipe_edges: Vec<&Edge> = Vec::new();
        for e in &doc.edges {
            let (Some(a), Some(b)) = (doc.node(e.from_node), doc.node(e.to_node)) else {
                continue;
            };
            match (kind_of(&a.type_id), kind_of(&b.type_id)) {
                (Some(ka), Some(kb)) => {
                    if e.from_pin != POSE_PIN || ka == Kind::Output {
                        return Err(format!(
                            "{} '{}' has no output '{}'",
                            ka.word(),
                            display_name(a, ka),
                            e.from_pin
                        ));
                    }
                    if !kb.inputs().contains(&e.to_pin.as_str()) {
                        return Err(format!(
                            "{} '{}' has no input '{}'",
                            kb.word(),
                            display_name(b, kb),
                            e.to_pin
                        ));
                    }
                    pipe_edges.push(e);
                }
                (Some(_), None) if is_machine_node(&b.type_id) => {
                    return Err(format!(
                        "the wire from '{}' into '{}' crosses from the pipeline into the \
                         state machine",
                        label(a),
                        label(b)
                    ))
                }
                (None, Some(_)) if is_machine_node(&a.type_id) => {
                    return Err(format!(
                        "the wire from '{}' into '{}' crosses from the state machine into \
                         the pipeline",
                        label(a),
                        label(b)
                    ))
                }
                // Machine wires (or foreign types — validation's business).
                _ => {}
            }
        }
        for e in &pipe_edges {
            let fan_in = pipe_edges
                .iter()
                .filter(|o| o.to_node == e.to_node && o.to_pin == e.to_pin)
                .count();
            if fan_in > 1 {
                let b = doc.node(e.to_node).expect("checked");
                let kb = kind_of(&b.type_id).expect("pipeline node");
                return Err(format!(
                    "{} '{}': input '{}' has {fan_in} wires",
                    kb.word(),
                    display_name(b, kb),
                    e.to_pin
                ));
            }
            let fan_out = pipe_edges
                .iter()
                .filter(|o| o.from_node == e.from_node)
                .count();
            if fan_out > 1 {
                let a = doc.node(e.from_node).expect("checked");
                let ka = kind_of(&a.type_id).expect("pipeline node");
                return Err(format!(
                    "{} '{}' feeds {fan_out} nodes — a pose can be wired to one input only \
                     (fan-out is not supported)",
                    ka.word(),
                    display_name(a, ka)
                ));
            }
        }
        Ok(())
    }

    fn node(&self, id: u64) -> Result<(&'a NodeInst, Kind), String> {
        let n = self
            .doc
            .node(id)
            .ok_or_else(|| format!("a pipeline wire names node {id}, which does not exist"))?;
        let kind = kind_of(&n.type_id)
            .ok_or_else(|| format!("node {id} ('{}') is not a pipeline node", n.type_id))?;
        Ok((n, kind))
    }

    /// The node wired into `pin`, if any (edge hygiene already ran).
    fn input(&self, n: &NodeInst, pin: &str) -> Option<u64> {
        self.doc
            .edges
            .iter()
            .find(|e| e.to_node == n.id && e.to_pin == pin)
            .map(|e| e.from_node)
    }

    fn wired(&self, n: &NodeInst, kind: Kind, name: &str, pin: &str) -> Result<u64, String> {
        self.input(n, pin).ok_or_else(|| {
            format!("{} '{name}': input '{pin}' is not wired", kind.word())
        })
    }

    /// The local-space subtree feeding `downstream` (the node whose input
    /// this is — named when an IK chain turns up where only local-space
    /// nodes may be).
    fn build(&mut self, id: u64, downstream: &str) -> Result<PlanPose, String> {
        let (n, kind) = self.node(id)?;
        let name = display_name(n, kind);
        if self.visiting.contains(&id) {
            return Err(format!("the pipeline loops through '{name}'"));
        }
        self.reachable.insert(id);
        match kind {
            Kind::Ik => Err(format!(
                "'{downstream}' must come before the first IK Chain \u{2014} IK works in \
                 model space"
            )),
            Kind::Output => Err("the Output Pose node has no output".to_string()),
            Kind::Machine => {
                let i = self.machine(n, &name)?;
                Ok(PlanPose::Machine(i))
            }
            Kind::Clip => {
                let clip = str_prop(&n.properties, CLIP_PROP)
                    .filter(|s| !s.trim().is_empty())
                    .ok_or_else(|| {
                        format!("clip '{name}' names no clip (property `{CLIP_PROP}`)")
                    })?;
                self.root_clips.push(PlanRootClip {
                    clip: PlanClip {
                        clip: crate::engine::scripting::normalize_graph_path(clip),
                        clip_name: str_prop(&n.properties, CLIP_NAME_PROP)
                            .filter(|s| !s.is_empty())
                            .map(str::to_string),
                    },
                    speed: speed_prop(&n.properties, &format!("clip '{name}'"))?,
                    node_id: id,
                });
                Ok(PlanPose::Clip(self.root_clips.len() - 1))
            }
            Kind::Layer => {
                let roots = bone_list(n, MASK_BONES_PROP);
                if roots.is_empty() {
                    return Err(format!(
                        "layer '{name}' names no bones (property `{MASK_BONES_PROP}`: \
                         comma-separated mask roots)"
                    ));
                }
                let weight_param =
                    float_param(n, LAYER_WEIGHT_PARAM_PROP, self.parameters, &|| {
                        format!("layer '{name}'")
                    })?;
                let include_root = !matches!(
                    n.properties.get(LAYER_INCLUDE_ROOT_PROP),
                    Some(PropValue::Bool(false))
                );
                let base_id = self.wired(n, kind, &name, LAYER_BASE_PIN)?;
                let layer_id = self.wired(n, kind, &name, LAYER_LAYER_PIN)?;
                self.visiting.push(id);
                let base = Box::new(self.build(base_id, &name)?);
                let layer = Box::new(self.build(layer_id, &name)?);
                self.visiting.pop();
                Ok(PlanPose::Layer {
                    base,
                    layer,
                    mask: PlanMask {
                        roots,
                        include_root,
                    },
                    weight_param,
                    node_id: id,
                })
            }
            Kind::PlayOnce => {
                let slot = self
                    .host_slots
                    .iter()
                    .position(|s| s.node_id == id)
                    .expect("every play-once node compiled into a slot");
                let roots = bone_list(n, MASK_BONES_PROP);
                let mask = (!roots.is_empty()).then(|| PlanMask {
                    roots,
                    include_root: true,
                });
                let input_id = self.wired(n, kind, &name, PIPE_IN_PIN)?;
                self.visiting.push(id);
                let input = Box::new(self.build(input_id, &name)?);
                self.visiting.pop();
                self.slot_walk.push(slot);
                Ok(PlanPose::Overlay {
                    input,
                    slot,
                    mask,
                    node_id: id,
                })
            }
        }
    }

    /// A State Machine node: the inline machine, or a nested file compiled
    /// through the same cycle guard states use.
    fn machine(&mut self, n: &NodeInst, name: &str) -> Result<usize, String> {
        let source = match nested_graph(n) {
            None => MachineSource::Inline,
            Some(rel) => {
                let rel = crate::engine::scripting::normalize_graph_path(rel);
                if self.stack.contains(&rel) {
                    let chain: Vec<&str> = self
                        .stack
                        .iter()
                        .map(String::as_str)
                        .chain([rel.as_str()])
                        .collect();
                    return Err(format!(
                        "state machine '{name}': nesting cycle: {}",
                        chain.join(" \u{2192} ")
                    ));
                }
                let child_doc = self.load.graph(&rel).ok_or_else(|| {
                    format!("state machine '{name}': nested graph '{rel}' could not be loaded")
                })?;
                self.stack.push(rel.clone());
                let child = compile_doc(&child_doc, self.stack, self.load, false)
                    .map_err(|e| format!("state machine '{name}': in '{rel}': {e}"))?;
                self.stack.pop();
                for d in &child.plan.parameters {
                    self.nested_params.push((name.to_string(), d.clone()));
                }
                MachineSource::Nested {
                    graph: rel,
                    plan: Arc::new(child.plan),
                }
            }
        };
        self.machines.push(PlanMachineRef {
            node_id: n.id,
            source,
        });
        Ok(self.machines.len() - 1)
    }
}

/// A State Machine node's nested graph path (`None` = inline).
fn nested_graph(n: &NodeInst) -> Option<&str> {
    str_prop(&n.properties, GRAPH_PROP).filter(|s| !s.trim().is_empty())
}
