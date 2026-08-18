//! Animation-graph preview data for the editor (Task 41 ticket 06).
//!
//! The bridge between a running [`AnimGraphRuntime`] and the graph panel's
//! preview surfaces (footer parameter strip, live state/transition
//! highlight). The same posture as [`super::graph_exec_viz`]: plain data in
//! **document space** (node ids, current values), rebuilt every frame by the
//! host, so the panel never touches the ECS.
//!
//! Writes travel the other way as [`AnimParamEdit`]s: the strip records what
//! the author did, the host applies them to the bound entity's blackboard
//! after the UI — parameters in, never states (ADR 0002's posture holds for
//! the editor too).

use crate::engine::animation::graph::machine::{AnimParams, ParamValue};
use crate::engine::animation::graph::plan::{AnimGraphPlan, PlanTree, PoseSource, RuleExpr};
use crate::engine::animation::graph::runner::{AnimGraphRunner, AnimGraphRuntime};
use crate::engine::scripting::normalize_graph_path;

use super::graph_exec_viz::ExecInstance;

/// One declared parameter, as the strip draws it: the runtime's current
/// value (so a value written by anything shows live) plus a derived slider
/// range for Floats. Labels resolve panel-side from the open document.
#[derive(Debug, Clone, PartialEq)]
pub struct AnimPreviewParam {
    pub slug: String,
    pub value: ParamValue,
    /// Slider range for a Float — derived from what the graph actually reads
    /// (blend thresholds, rule compare constants); `(0, 1)` when it reads
    /// nothing numeric.
    pub range: (f32, f32),
}

/// What the panel knows about the bound preview instance this frame.
#[derive(Debug, Clone, PartialEq)]
pub struct AnimPreview {
    /// The bound entity's display name (empty = unnamed, never unbound —
    /// an unbound tab has no `AnimPreview` at all).
    pub instance: String,
    /// `Entity::to_bits`, so the picker can mark the current row and the
    /// host knows where the strip's edits land.
    pub instance_id: u64,
    /// Set when the runtime refused to run (compile error, missing clip).
    pub disabled: Option<String>,
    /// Document node id of the active state.
    pub active_state: Option<u64>,
    /// An in-flight crossfade: `(outgoing state's node id, target weight)`.
    pub fade: Option<(u64, f32)>,
    /// The last transition that fired: `(node id, seconds since)`.
    pub fired: Option<(u64, f32)>,
    /// Declared parameters with current values, in declaration order.
    pub params: Vec<AnimPreviewParam>,
}

/// One strip edit, applied by the host to the bound runtime's blackboard.
/// The same three verbs gameplay has — the strip is just another writer.
#[derive(Debug, Clone, PartialEq)]
pub enum AnimParamEdit {
    SetFloat(String, f32),
    SetBool(String, bool),
    FireTrigger(String),
}

impl AnimParamEdit {
    /// Apply to a blackboard. `false` = refused (undeclared or mistyped),
    /// which for the strip only happens against a stale plan — harmless.
    pub fn apply(&self, params: &mut AnimParams) -> bool {
        match self {
            AnimParamEdit::SetFloat(slug, v) => params.set_float(slug, *v),
            AnimParamEdit::SetBool(slug, v) => params.set_bool(slug, *v),
            AnimParamEdit::FireTrigger(slug) => params.fire_trigger(slug),
        }
    }
}

/// A Float parameter's useful slider range: the values the graph actually
/// compares or blends it against. Collected from 1D blend thresholds keyed
/// by this parameter and rule-graph comparisons against constants; the top
/// gets 25% headroom so the author can push past the last threshold and see
/// the clamp. Nothing collected (2D params — their thresholds are angles —
/// or a parameter only read as-is) falls back to `(0, 1)`.
pub fn float_param_range(plan: &AnimGraphPlan, slug: &str) -> (f32, f32) {
    let mut vals: Vec<f32> = Vec::new();
    collect_plan(plan, slug, &mut vals);
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    for v in vals {
        lo = lo.min(v);
        hi = hi.max(v);
    }
    let pad = (hi - lo) * 0.25;
    (if lo < 0.0 { lo - pad } else { lo }, hi + pad)
}

/// One plan's reads of `slug`, nested sub-machines included — a merged
/// (nested) parameter's slider range comes from wherever the read lives.
fn collect_plan(plan: &AnimGraphPlan, slug: &str, out: &mut Vec<f32>) {
    for st in &plan.states {
        match &st.source {
            PoseSource::Tree(tree) => collect_tree(tree, slug, out),
            PoseSource::Machine { plan: child, .. } => collect_plan(child, slug, out),
        }
    }
    for t in &plan.transitions {
        if let Some(rule) = &t.rule {
            collect_expr(&rule.expr, slug, out);
        }
    }
}

fn collect_tree(tree: &PlanTree, slug: &str, out: &mut Vec<f32>) {
    match tree {
        PlanTree::Clip(_) => {}
        PlanTree::Blend1D { param, children } => {
            for (t, c) in children {
                if param == slug {
                    out.push(*t);
                }
                collect_tree(c, slug, out);
            }
        }
        // 2D thresholds are precomputed angles, not parameter values.
        PlanTree::Blend2D { children, .. } => {
            for (_, c) in children {
                collect_tree(c, slug, out);
            }
        }
    }
}

fn collect_expr(e: &RuleExpr, slug: &str, out: &mut Vec<f32>) {
    match e {
        RuleExpr::Compare(_, a, b) => {
            match (a.as_ref(), b.as_ref()) {
                (RuleExpr::ParamFloat(p), RuleExpr::ConstFloat(c))
                | (RuleExpr::ConstFloat(c), RuleExpr::ParamFloat(p))
                    if p == slug =>
                {
                    out.push(*c)
                }
                _ => {}
            }
            collect_expr(a, slug, out);
            collect_expr(b, slug, out);
        }
        RuleExpr::And(a, b) | RuleExpr::Or(a, b) | RuleExpr::Math(_, a, b) => {
            collect_expr(a, slug, out);
            collect_expr(b, slug, out);
        }
        RuleExpr::Not(a) => collect_expr(a, slug, out),
        _ => {}
    }
}

/// Build the frame's preview from a runtime. Pure over the runtime — the
/// world queries below are the only ECS-facing pieces.
pub fn preview_from_runtime(rt: &AnimGraphRuntime, name: String, id: u64) -> AnimPreview {
    let plan = &rt.plan;
    let params = plan
        .parameters
        .iter()
        .map(|d| AnimPreviewParam {
            slug: d.slug.clone(),
            value: match d.ty {
                crate::engine::animation::graph::plan::AnimParamType::Float => {
                    ParamValue::Float(rt.params.get_float(&d.slug).unwrap_or(0.0))
                }
                crate::engine::animation::graph::plan::AnimParamType::Bool => {
                    ParamValue::Bool(rt.params.get_bool(&d.slug).unwrap_or(false))
                }
                crate::engine::animation::graph::plan::AnimParamType::Trigger => {
                    ParamValue::Trigger(rt.params.trigger_set(&d.slug).unwrap_or(false))
                }
            },
            range: float_param_range(plan, &d.slug),
        })
        .collect();
    AnimPreview {
        instance: name,
        instance_id: id,
        disabled: rt.disabled.clone(),
        active_state: plan.states.get(rt.machine.current_state()).map(|s| s.node_id),
        fade: rt.machine.crossfade().and_then(|f| {
            plan.states.get(f.from).map(|s| (s.node_id, f.weight()))
        }),
        fired: rt
            .machine
            .last_fired()
            .and_then(|(i, age)| plan.transitions.get(i).map(|t| (t.node_id, age))),
        params,
    }
}

/// Which entity a graph tab previews: the explicit pick while it is still a
/// candidate, else the first selected entity that is one — the LIVE chip's
/// binding rule, reused verbatim so the two chips feel like one mechanism.
pub fn resolve_anim_bind(
    explicit: Option<u64>,
    selected: &[u64],
    candidates: &[ExecInstance],
) -> Option<u64> {
    let is_candidate = |id: u64| candidates.iter().any(|c| c.id == id);
    explicit
        .filter(|b| is_candidate(*b))
        .or_else(|| selected.iter().copied().find(|s| is_candidate(*s)))
}

/// Every entity running `graph_path` right now, nearest-camera first, for
/// the preview chip's picker. `excluded` names entities whose parameters are
/// owned by another writer (the net-play `anim_bridge` rigs) — a preview
/// strip driving those would fight gameplay every frame, so they are not
/// preview targets at all.
pub fn anim_instances_for(
    world: &hecs::World,
    graph_path: &str,
    camera: Option<[f32; 3]>,
    excluded: &[u64],
) -> Vec<ExecInstance> {
    let want = normalize_graph_path(graph_path);
    let mut out: Vec<ExecInstance> = world
        .query::<(&AnimGraphRunner, &AnimGraphRuntime)>()
        .iter()
        .filter(|(e, (r, _))| {
            normalize_graph_path(&r.graph) == want && !excluded.contains(&e.to_bits().get())
        })
        .map(|(entity, (_, rt))| {
            let distance = match (
                camera,
                world
                    .get::<&crate::engine::ecs::components::Transform>(entity)
                    .ok(),
            ) {
                (Some(c), Some(t)) => {
                    let p = t.position;
                    ((p.x - c[0]).powi(2) + (p.y - c[1]).powi(2) + (p.z - c[2]).powi(2)).sqrt()
                }
                _ => 0.0,
            };
            ExecInstance {
                id: entity.to_bits().get(),
                name: world
                    .get::<&crate::engine::ecs::components::Name>(entity)
                    .map(|n| n.0.clone())
                    .unwrap_or_default(),
                distance,
                // Machines tick every frame; recency is not a signal here.
                last_active: None,
                killed: rt.disabled.is_some(),
            }
        })
        .collect();
    out.sort_by(|a, b| a.distance.total_cmp(&b.distance).then(a.id.cmp(&b.id)));
    out
}

/// The preview for one bound entity; `None` when it is gone or not armed.
pub fn anim_preview_for(world: &hecs::World, bits: u64) -> Option<AnimPreview> {
    let entity = hecs::Entity::from_bits(bits)?;
    let rt = world.get::<&AnimGraphRuntime>(entity).ok()?;
    let name = world
        .get::<&crate::engine::ecs::components::Name>(entity)
        .map(|n| n.0.clone())
        .unwrap_or_default();
    Some(preview_from_runtime(&rt, name, bits))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::animation::graph::machine::AnimMachine;
    use crate::engine::animation::graph::plan::{
        AnimParamType, CmpOp, ParamDecl, PlanClip, PlanRule, PlanState, PlanTransition,
        PoseSource, TransitionFrom,
    };
    use crate::engine::animation::graph::{AnimGraphPlan, PlayOnceSlot};
    use std::sync::Arc;

    fn clip(name: &str) -> PlanTree {
        PlanTree::Clip(PlanClip {
            clip: name.to_string(),
            clip_name: None,
        })
    }

    /// Two states, one rule-gated transition on a Bool, a Float read by a 1D
    /// blend and a rule compare — the shapes the strip has to answer for.
    fn plan() -> AnimGraphPlan {
        AnimGraphPlan {
            states: vec![
                PlanState {
                    node_id: 10,
                    name: "Idle".into(),
                    source: PoseSource::Tree(clip("a.anim")),
                    speed: 1.0,
                },
                PlanState {
                    node_id: 11,
                    name: "Move".into(),
                    source: PoseSource::Tree(PlanTree::Blend1D {
                        param: "speed".into(),
                        children: vec![(0.0, clip("a.anim")), (6.0, clip("a.anim"))],
                    }),
                    speed: 1.0,
                },
            ],
            transitions: vec![PlanTransition {
                node_id: 20,
                from: TransitionFrom::State(0),
                to: 1,
                duration: 0.2,
                priority: 0,
                rule: Some(PlanRule {
                    expr: RuleExpr::And(
                        Box::new(RuleExpr::ParamBool("go".into())),
                        Box::new(RuleExpr::Compare(
                            CmpOp::Greater,
                            Box::new(RuleExpr::ParamFloat("speed".into())),
                            Box::new(RuleExpr::ConstFloat(3.0)),
                        )),
                    ),
                    triggers: Vec::new(),
                }),
            }],
            entry: 0,
            parameters: vec![
                ParamDecl {
                    slug: "speed".into(),
                    ty: AnimParamType::Float,
                    default: ParamValue::Float(0.0),
                },
                ParamDecl {
                    slug: "go".into(),
                    ty: AnimParamType::Bool,
                    default: ParamValue::Bool(false),
                },
                ParamDecl {
                    slug: "jump".into(),
                    ty: AnimParamType::Trigger,
                    default: ParamValue::Trigger(false),
                },
            ],
            slots: Vec::new(),
        }
    }

    fn runtime(plan: AnimGraphPlan) -> AnimGraphRuntime {
        let plan = Arc::new(plan);
        AnimGraphRuntime {
            graph: "graphs/t.animgraph".into(),
            machine: AnimMachine::new(&plan),
            slot: PlayOnceSlot::new(),
            params: AnimParams::from_decls(&plan.parameters),
            events: Vec::new(),
            plan,
            generation: 0,
            disabled: None,
        }
    }

    /// The strip's read surface: declared params in order with live values,
    /// the active state's document id, and the firing transition while its
    /// fade runs.
    #[test]
    fn preview_reads_the_machine_in_document_space() {
        let mut rt = runtime(plan());
        let p = preview_from_runtime(&rt, "Human".into(), 7);
        assert_eq!(p.active_state, Some(10), "entry state, by document id");
        assert_eq!(p.fade, None);
        assert_eq!(p.fired, None);
        let slugs: Vec<&str> = p.params.iter().map(|q| q.slug.as_str()).collect();
        assert_eq!(slugs, ["speed", "go", "jump"]);

        // Drive the transition the way the strip does — parameter writes.
        assert!(AnimParamEdit::SetBool("go".into(), true).apply(&mut rt.params));
        assert!(AnimParamEdit::SetFloat("speed".into(), 4.0).apply(&mut rt.params));
        let plan = rt.plan.clone();
        rt.machine.tick(&plan, &mut rt.params, 0.05);
        let p = preview_from_runtime(&rt, "Human".into(), 7);
        assert_eq!(p.active_state, Some(11), "the target is current mid-fade");
        let (from, w) = p.fade.expect("crossfade in flight");
        assert_eq!(from, 10);
        assert_eq!(w, 0.0, "the fade starts on the firing tick");
        let (node, age) = p.fired.expect("the fired transition is lit");
        assert_eq!(node, 20);
        assert_eq!(age, 0.0);

        // The flash decays by age, not by edge: next tick it is older, and
        // the fade has advanced.
        rt.machine.tick(&plan, &mut rt.params, 0.05);
        let p = preview_from_runtime(&rt, "Human".into(), 7);
        let (_, w) = p.fade.expect("still fading");
        assert!(w > 0.0 && w < 1.0);
        assert_eq!(p.fired.map(|(n, _)| n), Some(20));
        assert!(p.fired.unwrap().1 > 0.0);
    }

    /// FIRE lights the trigger and it stays lit until a transition consumes
    /// it — the strip shows buffering by reading, never by remembering.
    #[test]
    fn trigger_edits_stay_lit_until_consumed() {
        let mut rt = runtime(plan());
        assert!(AnimParamEdit::FireTrigger("jump".into()).apply(&mut rt.params));
        let lit = |rt: &AnimGraphRuntime| {
            preview_from_runtime(rt, String::new(), 0)
                .params
                .iter()
                .any(|p| p.slug == "jump" && p.value == ParamValue::Trigger(true))
        };
        assert!(lit(&rt));
        // Ticks without a consuming transition leave it buffered.
        let plan = rt.plan.clone();
        rt.machine.tick(&plan, &mut rt.params, 0.1);
        assert!(lit(&rt), "buffered across frames, not edge-triggered");
        // A mistyped or undeclared edit is refused, not silently created.
        assert!(!AnimParamEdit::SetFloat("jump".into(), 1.0).apply(&mut rt.params));
        assert!(!AnimParamEdit::SetBool("nope".into(), true).apply(&mut rt.params));
    }

    /// Float ranges come from what the graph reads: blend thresholds and
    /// rule constants, top-padded; unread params fall back to `(0, 1)`.
    #[test]
    fn float_ranges_derive_from_graph_reads() {
        let plan = plan();
        let (lo, hi) = float_param_range(&plan, "speed");
        assert_eq!(lo, 0.0);
        assert_eq!(hi, 7.5, "max(6.0 threshold, 3.0 compare) + 25% headroom");
        assert_eq!(float_param_range(&plan, "unread"), (0.0, 1.25));
    }

    /// The binding rule: explicit-while-valid, else selection, else nothing —
    /// the LIVE chip's ladder.
    #[test]
    fn binding_prefers_explicit_then_selection() {
        let inst = |id: u64| ExecInstance {
            id,
            name: String::new(),
            distance: 0.0,
            last_active: None,
            killed: false,
        };
        let c = [inst(1), inst(2)];
        assert_eq!(resolve_anim_bind(Some(2), &[1], &c), Some(2));
        assert_eq!(resolve_anim_bind(Some(9), &[1], &c), Some(1), "stale pick falls back");
        assert_eq!(resolve_anim_bind(None, &[7, 2], &c), Some(2));
        assert_eq!(resolve_anim_bind(None, &[7], &c), None);
    }
}
