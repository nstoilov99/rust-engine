//! Animation graph (Task 41) — a state machine evaluated per frame into a
//! Pose for a skeleton.
//!
//! The `.animgraph` asset is a [`node_graph_types::GraphDoc`] — the shared
//! graph document container from Task 40 — with an animation node library:
//! ENTRY, State and Transition are all nodes, parameters are the document's
//! `variables`. No exec wires exist in this domain; the flowing value is a
//! Pose (per CONTEXT.md). A state's pose comes from a single clip or from a
//! blend tree (clip nodes, 1D/2D blends with minimal sync groups) embedded
//! as a region keyed by the state's node id — the same container transitions
//! use for their rule graphs.
//!
//! Evaluation is engine-side by decision (ADR 0001): it samples `.anim` clip
//! assets through the existing keyframe sampling functions and writes the
//! skinning palette through `SkeletonInstance`, and it never runs on the
//! server — every client derives animation locally from replicated state
//! (ADR 0002). Gameplay's only surface is the typed parameter blackboard on
//! the runner component; gameplay never touches states.
//!
//! Layout mirrors the script runtime's seams (Task 45-A):
//! - [`plan`] — document shape (type ids, property keys) and the compiler
//!   from `GraphDoc` to [`AnimGraphPlan`].
//! - [`machine`] — the state machine core: parameter blackboard, active
//!   state, crossfades. Pure CPU, no assets — the evaluator-seam tests live
//!   against this.
//! - [`runner`] — the ECS binding: `AnimGraphRunner` component, plan/clip
//!   caches, and the system that ticks machines into skeletons.

pub mod library;
pub mod machine;
pub mod plan;
pub mod runner;

#[cfg(test)]
mod acceptance;

pub use library::{
    anim_node_registry, anim_node_tag, anim_rule_registry, new_animgraph_doc, ANIM_CATEGORY,
    ANIM_FLOW_DOMAIN, ANIM_POSE_DOMAIN,
};
pub use machine::{
    collect_anim_events, evaluate_pose, AnimEventFire, AnimMachine, AnimParams, Crossfade,
    ParamValue, PlayOnceSlot, PoseScratch,
};
pub use plan::{
    compile_anim_graph, compile_anim_graph_with, trigger_pin_type, AnimGraphPlan, AnimParamType,
    CmpOp, MathOp, ParamDecl, PlanClip, PlanRule, PlanSlot, PlanState, PlanTransition, PlanTree,
    PoseSource, RuleExpr, TransitionFrom, TRIGGER_PARAM_DOMAIN,
};
pub use runner::{
    AnimAssetLoader, AnimClipCache, AnimGraphPlanCache, AnimGraphRunner, AnimGraphRuntime,
    AnimGraphSystem, ClipSet, DiskAnimAssets,
};
