//! Acceptance evidence for the animation graph (Task 41: ticket 01 tracer,
//! ticket 02 rule graphs / triggers / Any State).
//!
//! Tests sit at the seams the spec pre-agreed: the document (round-trip
//! through the shared container io, embedded rule regions as one unit with
//! their transition), the machine (document + parameter writes + ticks in →
//! active state and blend weights out — rules, trigger consumption and Any
//! State interruption included), pose values on a synthetic skeleton (CPU
//! only, no GPU, no asset files), and the system (arming, invalidation,
//! coexistence with the single-clip player).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use glam::{Mat4, Vec3};
use node_graph_types::std_nodes::{AND, COMPARE_FLOAT, MUL_FLOAT, NOT};
use node_graph_types::{
    parse_graph, serialize_graph, Edge, GraphDoc, GraphRealm, GraphRegion, NodeInst, PinType,
    PropValue, VarDecl, VAR_GET_TYPE_ID, VAR_PROP, VAR_VALUE_PIN,
};

use crate::engine::animation::components::{LocalBoneTransform, SkeletonInstance};
use crate::engine::animation::{AnimationPlayer, AnimationUpdateSystem};
use crate::engine::assets::model_loader::{
    AnimEventMarker, AnimationChannel, BoneData, RawAnimationClip,
};
use crate::engine::ecs::components::MeshRenderer;
use crate::engine::ecs::resources::{Resources, Time};
use crate::engine::ecs::schedule::System;

use super::machine::{
    collect_anim_events, evaluate_pose, AnimEventFire, AnimMachine, AnimParams, PlayOnceSlot,
    PoseScratch,
};
use super::plan::AnimGraphPlan;
use super::plan::{self, compile_anim_graph, RuleExpr, TransitionFrom};
use super::runner::{
    AnimAssetLoader, AnimClipCache, AnimGraphPlanCache, AnimGraphRunner, AnimGraphRuntime,
    AnimGraphSystem, ClipSet,
};

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn node(id: u64, type_id: &str, title: Option<&str>) -> NodeInst {
    NodeInst {
        id,
        type_id: type_id.to_string(),
        type_version: 1,
        position: [id as f32 * 200.0, 0.0],
        properties: BTreeMap::new(),
        subgraph: None,
        tint: None,
        title: title.map(str::to_string),
    }
}

fn with(id: u64, type_id: &str, title: Option<&str>, props: &[(&str, PropValue)]) -> NodeInst {
    let mut n = node(id, type_id, title);
    for (k, v) in props {
        n.properties.insert(k.to_string(), v.clone());
    }
    n
}

fn edge(from: u64, fp: &str, to: u64, tp: &str) -> Edge {
    Edge {
        from_node: from,
        from_pin: fp.to_string(),
        to_node: to,
        to_pin: tp.to_string(),
    }
}

/// The smallest real rule: one parameter read wired into RESULT, as an
/// embedded region. Region node ids are region-local (0 and 1 here, in every
/// region, colliding with nothing).
fn param_rule(slug: &str) -> GraphRegion {
    GraphRegion {
        nodes: vec![
            with(
                0,
                VAR_GET_TYPE_ID,
                None,
                &[(VAR_PROP, PropValue::Str(slug.into()))],
            ),
            node(1, plan::ANIM_RULE_RESULT_TYPE_ID, None),
        ],
        edges: vec![edge(0, VAR_VALUE_PIN, 1, plan::RULE_RESULT_PIN)],
    }
}

/// A Trigger parameter declaration.
fn trigger_decl(slug: &str) -> VarDecl {
    VarDecl {
        slug: slug.into(),
        label: slug.into(),
        ty: plan::trigger_pin_type(),
        default: None,
        group: None,
    }
}

/// The tracer machine: ENTRY → Idle, one transition Idle → Walk whose rule
/// reads the Bool parameter `walk`, 0.5s crossfade.
fn two_state_doc() -> GraphDoc {
    let mut doc = GraphDoc {
        realm: GraphRealm::Client,
        ..GraphDoc::default()
    };
    doc.variables = vec![VarDecl {
        slug: "walk".into(),
        label: "Walk".into(),
        ty: PinType::Bool,
        default: Some(PropValue::Bool(false)),
        group: None,
    }];
    doc.nodes = vec![
        node(1, plan::ANIM_ENTRY_TYPE_ID, None),
        with(
            2,
            plan::ANIM_STATE_TYPE_ID,
            Some("Idle"),
            &[(plan::CLIP_PROP, PropValue::Asset("anims/idle.anim".into()))],
        ),
        with(
            3,
            plan::ANIM_STATE_TYPE_ID,
            Some("Walk"),
            &[(plan::CLIP_PROP, PropValue::Asset("anims/walk.anim".into()))],
        ),
        with(
            4,
            plan::ANIM_TRANSITION_TYPE_ID,
            None,
            &[
                (plan::DURATION_PROP, PropValue::Float(0.5)),
                (plan::PRIORITY_PROP, PropValue::Int(0)),
            ],
        ),
    ];
    doc.edges = vec![
        edge(1, plan::STATE_OUT_PIN, 2, plan::STATE_IN_PIN),
        edge(2, plan::STATE_OUT_PIN, 4, plan::TRANSITION_FROM_PIN),
        edge(4, plan::TRANSITION_TO_PIN, 3, plan::STATE_IN_PIN),
    ];
    doc.regions.insert(4, param_rule("walk"));
    doc
}

/// A one-bone clip holding a constant translation — enough to see whose pose
/// (and how much of it) reached the skeleton.
fn constant_clip(name: &str, x: f32) -> RawAnimationClip {
    RawAnimationClip {
        name: name.to_string(),
        duration_seconds: 1.0,
        channels: vec![AnimationChannel {
            bone_index: 0,
            position_keys: vec![(0.0, Vec3::new(x, 0.0, 0.0))],
            rotation_keys: vec![],
            scale_keys: vec![],
        }],
        events: vec![],
    }
}

/// A Float parameter declaration (default 0.0).
fn float_decl(slug: &str) -> VarDecl {
    VarDecl {
        slug: slug.into(),
        label: slug.into(),
        ty: PinType::Float,
        default: Some(PropValue::Float(0.0)),
        group: None,
    }
}

/// A clip node inside a state's tree region.
fn clip_node(id: u64, path: &str) -> NodeInst {
    with(
        id,
        plan::ANIM_CLIP_TYPE_ID,
        None,
        &[(plan::CLIP_PROP, PropValue::Asset(path.into()))],
    )
}

/// Walk/Run under a 1D blend on `speed` (thresholds 0 and 6), wired into the
/// tree's RESULT — the canonical walk→run tree.
fn walk_run_tree() -> GraphRegion {
    GraphRegion {
        nodes: vec![
            clip_node(0, "anims/walk.anim"),
            clip_node(1, "anims/run.anim"),
            with(
                2,
                plan::ANIM_BLEND1D_TYPE_ID,
                None,
                &[
                    (plan::BLEND_PARAM_PROP, PropValue::Str("speed".into())),
                    ("threshold_0", PropValue::Float(0.0)),
                    ("threshold_1", PropValue::Float(6.0)),
                ],
            ),
            node(3, plan::ANIM_POSE_RESULT_TYPE_ID, None),
        ],
        edges: vec![
            edge(0, plan::POSE_PIN, 2, "in_0"),
            edge(1, plan::POSE_PIN, 2, "in_1"),
            edge(2, plan::POSE_PIN, 3, plan::POSE_PIN),
        ],
    }
}

/// ENTRY → a single "Move" state whose pose is [`walk_run_tree`]. The state
/// names no clip — the tree is its pose producer.
fn blend1d_doc() -> GraphDoc {
    let mut doc = GraphDoc {
        realm: GraphRealm::Client,
        ..GraphDoc::default()
    };
    doc.variables = vec![float_decl("speed")];
    doc.nodes = vec![
        node(1, plan::ANIM_ENTRY_TYPE_ID, None),
        with(2, plan::ANIM_STATE_TYPE_ID, Some("Move"), &[]),
    ];
    doc.edges = vec![edge(1, plan::STATE_OUT_PIN, 2, plan::STATE_IN_PIN)];
    doc.regions.insert(2, walk_run_tree());
    doc
}

/// ENTRY → "Strafe" with an 8-way-style 2D directional blend on
/// `dir_x`/`dir_y`: E/N/W/S clips at the cardinal directions.
fn blend2d_doc() -> GraphDoc {
    let mut doc = GraphDoc {
        realm: GraphRealm::Client,
        ..GraphDoc::default()
    };
    doc.variables = vec![float_decl("dir_x"), float_decl("dir_y")];
    doc.nodes = vec![
        node(1, plan::ANIM_ENTRY_TYPE_ID, None),
        with(2, plan::ANIM_STATE_TYPE_ID, Some("Strafe"), &[]),
    ];
    doc.edges = vec![edge(1, plan::STATE_OUT_PIN, 2, plan::STATE_IN_PIN)];
    doc.regions.insert(
        2,
        GraphRegion {
            nodes: vec![
                clip_node(0, "anims/east.anim"),
                clip_node(1, "anims/north.anim"),
                clip_node(2, "anims/west.anim"),
                clip_node(3, "anims/south.anim"),
                with(
                    4,
                    plan::ANIM_BLEND2D_TYPE_ID,
                    None,
                    &[
                        (plan::BLEND_PARAM_X_PROP, PropValue::Str("dir_x".into())),
                        (plan::BLEND_PARAM_Y_PROP, PropValue::Str("dir_y".into())),
                        ("x_0", PropValue::Float(1.0)),
                        ("y_0", PropValue::Float(0.0)),
                        ("x_1", PropValue::Float(0.0)),
                        ("y_1", PropValue::Float(1.0)),
                        ("x_2", PropValue::Float(-1.0)),
                        ("y_2", PropValue::Float(0.0)),
                        ("x_3", PropValue::Float(0.0)),
                        ("y_3", PropValue::Float(-1.0)),
                    ],
                ),
                node(5, plan::ANIM_POSE_RESULT_TYPE_ID, None),
            ],
            edges: vec![
                edge(0, plan::POSE_PIN, 4, "in_0"),
                edge(1, plan::POSE_PIN, 4, "in_1"),
                edge(2, plan::POSE_PIN, 4, "in_2"),
                edge(3, plan::POSE_PIN, 4, "in_3"),
                edge(4, plan::POSE_PIN, 5, plan::POSE_PIN),
            ],
        },
    );
    doc
}

/// A cyclic clip whose bone-0 x value *is* its own normalized phase: keys
/// run linearly 0→1 over the duration, so a sampled value reads back where
/// in its cycle the clip was sampled.
fn phase_clip(name: &str, duration: f32) -> RawAnimationClip {
    RawAnimationClip {
        name: name.to_string(),
        duration_seconds: duration,
        channels: vec![AnimationChannel {
            bone_index: 0,
            position_keys: vec![(0.0, Vec3::ZERO), (duration, Vec3::new(1.0, 0.0, 0.0))],
            rotation_keys: vec![],
            scale_keys: vec![],
        }],
        events: vec![],
    }
}

/// A clip resolver over a `(path, clip)` table — the test-side stand-in for
/// the runner's clip cache.
fn resolver<'a>(
    clips: &'a [(&'a str, RawAnimationClip)],
) -> impl Fn(&plan::PlanClip) -> Option<&'a RawAnimationClip> {
    move |c| clips.iter().find(|(p, _)| *p == c.clip).map(|(_, cl)| cl)
}

/// Mutable properties of node `nid` inside the region owned by `owner`.
fn region_node_props(
    doc: &mut GraphDoc,
    owner: u64,
    nid: u64,
) -> &mut BTreeMap<String, PropValue> {
    &mut doc
        .regions
        .get_mut(&owner)
        .unwrap()
        .nodes
        .iter_mut()
        .find(|n| n.id == nid)
        .unwrap()
        .properties
}

fn synthetic_bones() -> Vec<BoneData> {
    vec![
        BoneData {
            name: "root".into(),
            parent_index: None,
            inverse_bind_matrix: Mat4::IDENTITY,
        },
        BoneData {
            name: "child".into(),
            parent_index: Some(0),
            inverse_bind_matrix: Mat4::IDENTITY,
        },
    ]
}

// ---------------------------------------------------------------------------
// Document round-trip
// ---------------------------------------------------------------------------

#[test]
fn animgraph_doc_round_trips() {
    let doc = two_state_doc();
    let text = serialize_graph(&doc).expect("serializes");
    let back = parse_graph(&text).expect("parses");
    assert_eq!(back, doc, "states, transition data and parameters survive");

    // …and the reloaded document compiles to the identical plan, which is
    // the round-trip a running machine actually cares about.
    assert_eq!(
        compile_anim_graph(&back).expect("compiles"),
        compile_anim_graph(&doc).expect("compiles")
    );
}

/// The committed demo asset, read from disk — proving the file agrees with
/// the engine, not that a fixture agrees with itself (the `demo_curve`
/// posture from the scripting acceptance tests).
#[test]
fn the_committed_demo_document_loads_and_compiles() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("content/graphs/defeated.animgraph");
    let doc = node_graph_types::load_graph(&path).expect("the demo animgraph loads");
    let plan = compile_anim_graph(&doc).expect("the demo animgraph compiles");
    assert_eq!(plan.states[plan.entry].name, "Idle");
    assert_eq!(plan.states[plan.entry].speed, 0.0, "Idle holds a pose");
    assert_eq!(plan.transitions.len(), 1);
    assert_eq!(plan.transitions[0].duration, 0.4);
    let rule = plan.transitions[0].rule.as_ref().expect("the walk rule compiled");
    assert_eq!(rule.expr, RuleExpr::ParamBool("walk".into()));
    assert!(rule.triggers.is_empty());
}

// ---------------------------------------------------------------------------
// Compiler
// ---------------------------------------------------------------------------

#[test]
fn compiles_states_transition_and_parameters() {
    let plan = compile_anim_graph(&two_state_doc()).expect("compiles");
    assert_eq!(plan.states.len(), 2);
    assert_eq!(plan.states[plan.entry].name, "Idle", "ENTRY wires the start state");
    let t = &plan.transitions[0];
    assert_eq!((t.duration, t.priority), (0.5, 0));
    let rule = t.rule.as_ref().expect("the wired rule compiled");
    assert_eq!(rule.expr, RuleExpr::ParamBool("walk".into()));
    assert_eq!(t.from, TransitionFrom::State(plan.entry), "Idle is the source");
    assert_eq!(plan.states[t.to].name, "Walk");
    assert_eq!(plan.parameters.len(), 1);
    assert_eq!(plan.clip_refs(), vec!["anims/idle.anim", "anims/walk.anim"]);
}

#[test]
fn compile_refusals_are_author_errors() {
    // Wrong realm: animation is client-derived by definition.
    let mut doc = two_state_doc();
    doc.realm = GraphRealm::Shared;
    assert!(compile_anim_graph(&doc).unwrap_err().contains("realm"));

    // No ENTRY node.
    let mut doc = two_state_doc();
    doc.nodes.retain(|n| n.type_id != plan::ANIM_ENTRY_TYPE_ID);
    assert!(compile_anim_graph(&doc).unwrap_err().contains("ENTRY"));

    // A state that names no clip.
    let mut doc = two_state_doc();
    doc.node_mut(2).unwrap().properties.remove(plan::CLIP_PROP);
    assert!(compile_anim_graph(&doc).unwrap_err().contains("no clip"));

    // A rule reading an undeclared parameter.
    let mut doc = two_state_doc();
    doc.variables.clear();
    assert!(compile_anim_graph(&doc).unwrap_err().contains("not declared"));

    // A parameter type outside the animation blackboard.
    let mut doc = two_state_doc();
    doc.variables[0].ty = PinType::String;
    assert!(compile_anim_graph(&doc)
        .unwrap_err()
        .contains("not an animation parameter type"));

    // A transition wired on one side only.
    let mut doc = two_state_doc();
    doc.edges.retain(|e| e.to_pin != plan::TRANSITION_FROM_PIN);
    assert!(compile_anim_graph(&doc).unwrap_err().contains("no source state"));
}

// ---------------------------------------------------------------------------
// Machine seam: parameters + ticks in → active state and weights out
// ---------------------------------------------------------------------------

#[test]
fn entry_state_is_active_on_the_first_tick() {
    let plan = compile_anim_graph(&two_state_doc()).expect("compiles");
    let mut params = AnimParams::from_decls(&plan.parameters);
    let mut m = AnimMachine::new(&plan);
    m.tick(&plan, &mut params, 0.1);
    assert_eq!(plan.states[m.current_state()].name, "Idle");
    assert_eq!(m.blend_weight(), 1.0, "no crossfade at rest");
    assert!(m.crossfade().is_none());
}

#[test]
fn parameter_flip_starts_a_crossfade_that_follows_the_stated_duration() {
    let plan = compile_anim_graph(&two_state_doc()).expect("compiles");
    let mut params = AnimParams::from_decls(&plan.parameters);
    let mut m = AnimMachine::new(&plan);

    // Sits in Idle while the parameter is false.
    for _ in 0..5 {
        m.tick(&plan, &mut params, 0.1);
    }
    assert_eq!(plan.states[m.current_state()].name, "Idle");

    // Gameplay writes the parameter — never the state.
    assert!(params.set_bool("walk", true));
    m.tick(&plan, &mut params, 0.1);
    assert_eq!(plan.states[m.current_state()].name, "Walk");
    assert_eq!(m.blend_weight(), 0.0, "the target blends in from zero");

    // Weight climbs linearly over the 0.5s duration: 0.1s per tick.
    let mut weights = Vec::new();
    for _ in 0..4 {
        m.tick(&plan, &mut params, 0.1);
        weights.push(m.blend_weight());
    }
    for (i, w) in weights.iter().enumerate() {
        let expected = 0.2 * (i + 1) as f32;
        assert!((w - expected).abs() < 1e-4, "weight {w} at step {i}, expected {expected}");
    }

    // One more tick reaches the duration; the fade retires and the machine
    // is fully in Walk.
    m.tick(&plan, &mut params, 0.1);
    assert!(m.crossfade().is_none());
    assert_eq!(m.blend_weight(), 1.0);
    assert_eq!(plan.states[m.current_state()].name, "Walk");
}

#[test]
fn ordinary_transitions_wait_out_a_running_crossfade() {
    // Add a Walk → Idle transition on the same (still-true) parameter: while
    // the first crossfade runs it must not fire (interruption rule v1).
    let mut doc = two_state_doc();
    doc.nodes.push(with(
        5,
        plan::ANIM_TRANSITION_TYPE_ID,
        None,
        &[(plan::DURATION_PROP, PropValue::Float(0.5))],
    ));
    doc.edges.push(edge(3, plan::STATE_OUT_PIN, 5, plan::TRANSITION_FROM_PIN));
    doc.edges.push(edge(5, plan::TRANSITION_TO_PIN, 2, plan::STATE_IN_PIN));
    doc.regions.insert(5, param_rule("walk"));
    let plan = compile_anim_graph(&doc).expect("compiles");
    let mut params = AnimParams::from_decls(&plan.parameters);
    let mut m = AnimMachine::new(&plan);

    params.set_bool("walk", true);
    m.tick(&plan, &mut params, 0.1); // Idle → Walk fires, fade starts
    let from = m.crossfade().expect("fading").from;
    m.tick(&plan, &mut params, 0.1);
    m.tick(&plan, &mut params, 0.1);
    assert_eq!(plan.states[m.current_state()].name, "Walk", "still Walk mid-fade");
    assert_eq!(m.crossfade().expect("still fading").from, from, "no re-transition");
}

#[test]
fn priority_resolves_deterministically_and_zero_duration_switches_instantly() {
    // Two always-true transitions out of Idle; the lower priority value wins.
    let mut doc = two_state_doc();
    doc.nodes.push(with(
        6,
        plan::ANIM_STATE_TYPE_ID,
        Some("Third"),
        &[(plan::CLIP_PROP, PropValue::Asset("anims/idle.anim".into()))],
    ));
    doc.regions.remove(&4); // no rule = always-true
    doc.node_mut(4)
        .unwrap()
        .properties
        .insert(plan::PRIORITY_PROP.into(), PropValue::Int(2));
    doc.nodes.push(with(
        7,
        plan::ANIM_TRANSITION_TYPE_ID,
        None,
        &[(plan::PRIORITY_PROP, PropValue::Int(1))],
    ));
    doc.edges.push(edge(2, plan::STATE_OUT_PIN, 7, plan::TRANSITION_FROM_PIN));
    doc.edges.push(edge(7, plan::TRANSITION_TO_PIN, 6, plan::STATE_IN_PIN));

    let plan = compile_anim_graph(&doc).expect("compiles");
    let mut params = AnimParams::from_decls(&plan.parameters);
    let mut m = AnimMachine::new(&plan);
    m.tick(&plan, &mut params, 0.1);
    assert_eq!(
        plan.states[m.current_state()].name, "Third",
        "priority 1 beats priority 2"
    );
    assert!(
        m.crossfade().is_none(),
        "an omitted duration is 0.0 — an instant switch, no fade"
    );
}

#[test]
fn parameter_writes_are_typed_and_declared_only() {
    let plan = compile_anim_graph(&two_state_doc()).expect("compiles");
    let mut params = AnimParams::from_decls(&plan.parameters);
    assert!(params.set_bool("walk", true));
    assert!(!params.set_float("walk", 1.0), "a Bool refuses a Float write");
    assert!(!params.set_bool("run", true), "undeclared slugs are refused");
    assert_eq!(params.get_bool("walk"), Some(true));
}

// ---------------------------------------------------------------------------
// Rule graphs (document unit, purity, typing)
// ---------------------------------------------------------------------------

#[test]
fn duplicating_a_transition_carries_its_rule_and_deleting_never_orphans_it() {
    let mut doc = two_state_doc();

    // Duplicate the transition the way an editor would: clone the node under
    // a fresh id and clone the region entry under that id. Region node ids
    // are region-local, so no remapping exists to get wrong.
    let dup = doc.next_node_id();
    let mut copy = doc.node(4).unwrap().clone();
    copy.id = dup;
    doc.nodes.push(copy);
    let rule = doc.regions.get(&4).cloned().expect("the original rule");
    doc.regions.insert(dup, rule);
    doc.edges.push(edge(2, plan::STATE_OUT_PIN, dup, plan::TRANSITION_FROM_PIN));
    doc.edges.push(edge(dup, plan::TRANSITION_TO_PIN, 3, plan::STATE_IN_PIN));

    // The copy compiles to the same rule as the original…
    let compiled = compile_anim_graph(&doc).expect("compiles");
    assert_eq!(compiled.transitions.len(), 2);
    assert_eq!(compiled.transitions[0].rule, compiled.transitions[1].rule);

    // …the document round-trips with both rules inline…
    let back = parse_graph(&serialize_graph(&doc).unwrap()).unwrap();
    assert_eq!(back, doc, "embedded rules serialize with the parent");

    // …and deleting a transition takes exactly its own rule with it.
    assert!(doc.remove_node(dup));
    assert!(doc.regions.get(&dup).is_none(), "no orphaned rule");
    assert!(doc.regions.get(&4).is_some(), "the original is untouched");
    assert_eq!(compile_anim_graph(&doc).unwrap().transitions.len(), 1);
}

#[test]
fn rule_purity_is_enforced() {
    // Effect (print), exec flow (branch), event emitter (emit_event) and
    // latent (delay) nodes are refused by name — as is any node type outside
    // the pure whitelist, which is what rejects Server-realm nodes (and node
    // types that do not exist yet) outright.
    for bad in ["print", "branch", "emit_event", "delay", "srv_apply_damage"] {
        let mut doc = two_state_doc();
        doc.regions.get_mut(&4).unwrap().nodes.push(node(9, bad, None));
        let err = compile_anim_graph(&doc).unwrap_err();
        assert!(err.contains(bad) && err.contains("not a rule node"), "{err}");
    }

    // Exactly one RESULT sink.
    let mut doc = two_state_doc();
    doc.regions
        .get_mut(&4)
        .unwrap()
        .nodes
        .push(node(9, plan::ANIM_RULE_RESULT_TYPE_ID, None));
    assert!(compile_anim_graph(&doc)
        .unwrap_err()
        .contains("exactly one RESULT"));

    // A non-empty rule with no RESULT at all.
    let mut doc = two_state_doc();
    let region = doc.regions.get_mut(&4).unwrap();
    region.nodes.retain(|n| n.type_id != plan::ANIM_RULE_RESULT_TYPE_ID);
    region.edges.clear();
    assert!(compile_anim_graph(&doc).unwrap_err().contains("no RESULT"));

    // A cycle refuses rather than hanging the compiler.
    let mut doc = two_state_doc();
    *doc.regions.get_mut(&4).unwrap() = GraphRegion {
        nodes: vec![
            node(0, NOT, None),
            node(1, plan::ANIM_RULE_RESULT_TYPE_ID, None),
        ],
        edges: vec![
            edge(0, "result", 0, "a"),
            edge(0, "result", 1, plan::RULE_RESULT_PIN),
        ],
    };
    assert!(compile_anim_graph(&doc).unwrap_err().contains("cycle"));

    // Fan-in: two wires into one input.
    let mut doc = two_state_doc();
    let region = doc.regions.get_mut(&4).unwrap();
    region.nodes.push(node(2, NOT, None));
    region.edges.push(edge(2, "result", 1, plan::RULE_RESULT_PIN));
    assert!(compile_anim_graph(&doc)
        .unwrap_err()
        .contains("has 2 wires"));
}

#[test]
fn rule_type_mismatches_are_refused() {
    // A Bool parameter fed into a float comparison input.
    let mut doc = two_state_doc();
    let region = doc.regions.get_mut(&4).unwrap();
    region.nodes.push(node(2, COMPARE_FLOAT, None));
    region.edges = vec![
        edge(0, VAR_VALUE_PIN, 2, "a"),
        edge(2, "result", 1, plan::RULE_RESULT_PIN),
    ];
    let err = compile_anim_graph(&doc).unwrap_err();
    assert!(err.contains("produces a Bool where a Float is expected"), "{err}");

    // A Float parameter wired straight into the Bool RESULT.
    let mut doc = two_state_doc();
    doc.variables.push(VarDecl {
        slug: "speed".into(),
        label: "Speed".into(),
        ty: PinType::Float,
        default: None,
        group: None,
    });
    doc.regions.get_mut(&4).unwrap().nodes[0]
        .properties
        .insert(VAR_PROP.into(), PropValue::Str("speed".into()));
    let err = compile_anim_graph(&doc).unwrap_err();
    assert!(err.contains("produces a Float where a Bool is expected"), "{err}");
}

#[test]
fn an_unwired_rule_input_is_always_true() {
    // RESULT present, nothing wired into it: the hollow socket dot.
    let mut doc = two_state_doc();
    doc.regions.get_mut(&4).unwrap().edges.clear();
    let plan = compile_anim_graph(&doc).expect("compiles");
    assert_eq!(plan.transitions[0].rule, None);
    let mut params = AnimParams::from_decls(&plan.parameters);
    let mut m = AnimMachine::new(&plan);
    m.tick(&plan, &mut params, 0.1);
    assert_eq!(
        plan.states[m.current_state()].name, "Walk",
        "fires with no parameter written at all"
    );

    // No region at all reads the same way.
    let mut doc = two_state_doc();
    doc.regions.remove(&4);
    assert_eq!(compile_anim_graph(&doc).unwrap().transitions[0].rule, None);
}

#[test]
fn a_compound_rule_fires_exactly_when_its_expression_passes() {
    // Speed × 2 > 6 ∧ walk — parameter reads through math, a comparison and
    // boolean logic, with pin constants on the unwired inputs.
    let mut doc = two_state_doc();
    doc.variables.push(VarDecl {
        slug: "speed".into(),
        label: "Speed".into(),
        ty: PinType::Float,
        default: Some(PropValue::Float(0.0)),
        group: None,
    });
    doc.regions.insert(
        4,
        GraphRegion {
            nodes: vec![
                with(0, VAR_GET_TYPE_ID, None, &[(VAR_PROP, PropValue::Str("speed".into()))]),
                with(1, MUL_FLOAT, None, &[("b", PropValue::Float(2.0))]),
                with(
                    2,
                    COMPARE_FLOAT,
                    None,
                    &[
                        ("op", PropValue::Enum("greater".into())),
                        ("b", PropValue::Float(6.0)),
                    ],
                ),
                with(3, VAR_GET_TYPE_ID, None, &[(VAR_PROP, PropValue::Str("walk".into()))]),
                // Region-local id 4 collides with the transition's own doc id
                // on purpose: the namespaces are separate.
                node(4, AND, None),
                node(5, plan::ANIM_RULE_RESULT_TYPE_ID, None),
            ],
            edges: vec![
                edge(0, VAR_VALUE_PIN, 1, "a"),
                edge(1, "result", 2, "a"),
                edge(2, "result", 4, "a"),
                edge(3, VAR_VALUE_PIN, 4, "b"),
                edge(4, "result", 5, plan::RULE_RESULT_PIN),
            ],
        },
    );

    let plan = compile_anim_graph(&doc).expect("compiles");
    let mut params = AnimParams::from_decls(&plan.parameters);
    let mut m = AnimMachine::new(&plan);

    params.set_bool("walk", true);
    params.set_float("speed", 3.0); // 3×2 = 6, not > 6
    m.tick(&plan, &mut params, 0.1);
    assert_eq!(plan.states[m.current_state()].name, "Idle");

    params.set_float("speed", 3.5); // 7 > 6, but walk must hold too
    params.set_bool("walk", false);
    m.tick(&plan, &mut params, 0.1);
    assert_eq!(plan.states[m.current_state()].name, "Idle");

    params.set_bool("walk", true);
    m.tick(&plan, &mut params, 0.1);
    assert_eq!(plan.states[m.current_state()].name, "Walk");
}

// ---------------------------------------------------------------------------
// Triggers (buffering, consume-on-transition)
// ---------------------------------------------------------------------------

#[test]
fn trigger_writes_are_typed_and_declared_only() {
    let mut doc = two_state_doc();
    doc.variables.push(trigger_decl("go"));
    let plan = compile_anim_graph(&doc).expect("compiles");
    let mut params = AnimParams::from_decls(&plan.parameters);

    assert!(!params.fire_trigger("walk"), "a Bool refuses a fire");
    assert!(!params.set_bool("go", true), "a Trigger refuses a Bool write");
    assert!(!params.fire_trigger("jump"), "undeclared slugs are refused");
    assert_eq!(params.trigger_set("go"), Some(false), "unset until fired");
    assert!(params.fire_trigger("go"));
    assert!(params.fire_trigger("go"), "re-firing while set is one buffered shot");
    assert_eq!(params.trigger_set("go"), Some(true));
    assert_eq!(params.get_bool("go"), None, "a trigger is not a Bool");
}

#[test]
fn a_trigger_buffers_across_frames_and_is_consumed_exactly_once() {
    // Idle → Walk on `walk` (0.5s fade), Walk → Idle on the Trigger `go`.
    let mut doc = two_state_doc();
    doc.variables.push(trigger_decl("go"));
    doc.nodes.push(with(
        5,
        plan::ANIM_TRANSITION_TYPE_ID,
        None,
        &[(plan::DURATION_PROP, PropValue::Float(0.0))],
    ));
    doc.edges.push(edge(3, plan::STATE_OUT_PIN, 5, plan::TRANSITION_FROM_PIN));
    doc.edges.push(edge(5, plan::TRANSITION_TO_PIN, 2, plan::STATE_IN_PIN));
    doc.regions.insert(5, param_rule("go"));

    let plan = compile_anim_graph(&doc).expect("compiles");
    let t5 = plan.transitions.iter().find(|t| t.node_id == 5).unwrap();
    assert_eq!(
        t5.rule.as_ref().unwrap().triggers,
        vec!["go".to_string()],
        "the rule's trigger reads are collected at compile time"
    );

    let mut params = AnimParams::from_decls(&plan.parameters);
    let mut m = AnimMachine::new(&plan);

    // Start the Idle → Walk fade, then fire the trigger mid-fade: ordinary
    // transitions wait out a crossfade, and the shot must not be lost.
    params.set_bool("walk", true);
    m.tick(&plan, &mut params, 0.1);
    assert_eq!(plan.states[m.current_state()].name, "Walk");
    params.set_bool("walk", false); // fades run to completion regardless
    assert!(params.fire_trigger("go"));
    m.tick(&plan, &mut params, 0.1);
    m.tick(&plan, &mut params, 0.1);
    assert_eq!(
        plan.states[m.current_state()].name, "Walk",
        "blocked by the running fade"
    );
    assert_eq!(params.trigger_set("go"), Some(true), "buffered, not lost");

    // The tick that retires the fade also lets the transition fire — and the
    // fire consumes the trigger.
    m.tick(&plan, &mut params, 0.3);
    assert_eq!(plan.states[m.current_state()].name, "Idle");
    assert_eq!(params.trigger_set("go"), Some(false), "consumed by the fire");

    // Consumed means spent: nothing re-fires on later frames.
    m.tick(&plan, &mut params, 0.1);
    m.tick(&plan, &mut params, 0.1);
    assert_eq!(plan.states[m.current_state()].name, "Idle");
}

#[test]
fn only_the_firing_transition_consumes_a_trigger() {
    // Idle → Walk always-true at priority 0; Idle → Third reads the Trigger
    // `go` at priority 5. Both pass — Walk fires. Losing a priority contest
    // is not firing: the trigger must stay buffered.
    let mut doc = two_state_doc();
    doc.variables.push(trigger_decl("go"));
    doc.regions.remove(&4); // Idle → Walk becomes always-true
    doc.nodes.push(with(
        6,
        plan::ANIM_STATE_TYPE_ID,
        Some("Third"),
        &[(plan::CLIP_PROP, PropValue::Asset("anims/idle.anim".into()))],
    ));
    doc.nodes.push(with(
        7,
        plan::ANIM_TRANSITION_TYPE_ID,
        None,
        &[(plan::PRIORITY_PROP, PropValue::Int(5))],
    ));
    doc.edges.push(edge(2, plan::STATE_OUT_PIN, 7, plan::TRANSITION_FROM_PIN));
    doc.edges.push(edge(7, plan::TRANSITION_TO_PIN, 6, plan::STATE_IN_PIN));
    doc.regions.insert(7, param_rule("go"));

    let plan = compile_anim_graph(&doc).expect("compiles");
    let mut params = AnimParams::from_decls(&plan.parameters);
    let mut m = AnimMachine::new(&plan);
    params.fire_trigger("go");
    m.tick(&plan, &mut params, 0.1);
    assert_eq!(plan.states[m.current_state()].name, "Walk", "priority 0 won");
    assert_eq!(
        params.trigger_set("go"),
        Some(true),
        "a read that did not fire consumes nothing"
    );
}

// ---------------------------------------------------------------------------
// Any State
// ---------------------------------------------------------------------------

/// `two_state_doc` plus a Dead state and Any State → Dead on the Trigger
/// `died`, priority −1, 0.2s fade.
fn any_state_doc() -> GraphDoc {
    let mut doc = two_state_doc();
    doc.variables.push(trigger_decl("died"));
    doc.nodes.push(with(
        6,
        plan::ANIM_STATE_TYPE_ID,
        Some("Dead"),
        &[(plan::CLIP_PROP, PropValue::Asset("anims/idle.anim".into()))],
    ));
    doc.nodes.push(node(7, plan::ANIM_ANY_STATE_TYPE_ID, None));
    doc.nodes.push(with(
        8,
        plan::ANIM_TRANSITION_TYPE_ID,
        None,
        &[
            (plan::DURATION_PROP, PropValue::Float(0.2)),
            (plan::PRIORITY_PROP, PropValue::Int(-1)),
        ],
    ));
    doc.edges.push(edge(7, plan::STATE_OUT_PIN, 8, plan::TRANSITION_FROM_PIN));
    doc.edges.push(edge(8, plan::TRANSITION_TO_PIN, 6, plan::STATE_IN_PIN));
    doc.regions.insert(8, param_rule("died"));
    doc
}

#[test]
fn any_state_fires_from_whatever_state_is_active() {
    let plan = compile_anim_graph(&any_state_doc()).expect("compiles");
    let t8 = plan.transitions.iter().find(|t| t.node_id == 8).unwrap();
    assert_eq!(t8.from, TransitionFrom::AnyState);

    // From Idle — no Idle → Dead edge exists.
    let mut params = AnimParams::from_decls(&plan.parameters);
    let mut m = AnimMachine::new(&plan);
    params.fire_trigger("died");
    m.tick(&plan, &mut params, 0.1);
    assert_eq!(plan.states[m.current_state()].name, "Dead");
    assert_eq!(params.trigger_set("died"), Some(false), "consumed by the fire");

    // From Walk — nor a Walk → Dead one.
    let mut params = AnimParams::from_decls(&plan.parameters);
    let mut m = AnimMachine::new(&plan);
    params.set_bool("walk", true);
    for _ in 0..7 {
        m.tick(&plan, &mut params, 0.1); // into Walk and out the fade
    }
    assert_eq!(plan.states[m.current_state()].name, "Walk");
    assert!(m.crossfade().is_none());
    params.fire_trigger("died");
    m.tick(&plan, &mut params, 0.1);
    assert_eq!(plan.states[m.current_state()].name, "Dead");

    // Priority is one scale: with both rules passing from Idle, the Any
    // State transition's −1 beats the ordinary transition's 0.
    let mut params = AnimParams::from_decls(&plan.parameters);
    let mut m = AnimMachine::new(&plan);
    params.set_bool("walk", true);
    params.fire_trigger("died");
    m.tick(&plan, &mut params, 0.1);
    assert_eq!(plan.states[m.current_state()].name, "Dead");
}

#[test]
fn only_any_state_interrupts_a_running_crossfade() {
    let plan = compile_anim_graph(&any_state_doc()).expect("compiles");
    let mut params = AnimParams::from_decls(&plan.parameters);
    let mut m = AnimMachine::new(&plan);

    // Start Idle → Walk (0.5s) and kill mid-fade.
    params.set_bool("walk", true);
    m.tick(&plan, &mut params, 0.1);
    m.tick(&plan, &mut params, 0.1);
    assert!(m.crossfade().is_some());
    params.fire_trigger("died");
    m.tick(&plan, &mut params, 0.1);
    assert_eq!(
        plan.states[m.current_state()].name, "Dead",
        "an Any State transition interrupts the fade"
    );
    let fade = m.crossfade().expect("a fresh fade into Dead");
    assert_eq!(
        plan.states[fade.from].name, "Walk",
        "the new outgoing side is the interrupted fade's target"
    );
    assert_eq!(fade.duration, 0.2);
    assert_eq!(fade.elapsed, 0.0);
}

#[test]
fn a_held_any_state_rule_does_not_restart_its_target() {
    // Strip the rule: an always-true Any State → Dead. Without the
    // self-target skip this would restart Dead every single frame.
    let mut doc = any_state_doc();
    doc.regions.remove(&8);
    let plan = compile_anim_graph(&doc).expect("compiles");
    let mut params = AnimParams::from_decls(&plan.parameters);
    let mut m = AnimMachine::new(&plan);

    m.tick(&plan, &mut params, 0.1);
    assert_eq!(plan.states[m.current_state()].name, "Dead");
    m.tick(&plan, &mut params, 0.1);
    m.tick(&plan, &mut params, 0.1); // fade (0.2s) retires
    assert!(m.crossfade().is_none());
    let t = m.time();
    m.tick(&plan, &mut params, 0.1);
    assert_eq!(plan.states[m.current_state()].name, "Dead");
    assert!(m.time() > t, "Dead keeps playing — never re-entered");
    assert!(m.crossfade().is_none(), "and never re-fades");
}

// ---------------------------------------------------------------------------
// Pose values on a synthetic skeleton (CPU only)
// ---------------------------------------------------------------------------

#[test]
fn crossfade_blends_pose_values_on_a_synthetic_skeleton() {
    let plan = compile_anim_graph(&two_state_doc()).expect("compiles");
    let clips = [
        ("anims/idle.anim", constant_clip("Idle", 0.0)),
        ("anims/walk.anim", constant_clip("Walk", 10.0)),
    ];
    let clip_for = resolver(&clips);

    let mut params = AnimParams::from_decls(&plan.parameters);
    let mut m = AnimMachine::new(&plan);
    let mut pose = vec![LocalBoneTransform::default(); 2];
    let mut scratch = PoseScratch::new();

    m.tick(&plan, &mut params, 0.1);
    evaluate_pose(&m, &plan, &params, &clip_for, &mut pose, &mut scratch);
    assert_eq!(pose[0].translation.x, 0.0, "entry state pose");

    params.set_bool("walk", true);
    m.tick(&plan, &mut params, 0.1); // fade starts, elapsed 0.0
    m.tick(&plan, &mut params, 0.1);
    m.tick(&plan, &mut params, 0.1);
    m.tick(&plan, &mut params, 0.05); // elapsed 0.25 of 0.5 → weight 0.5
    evaluate_pose(&m, &plan, &params, &clip_for, &mut pose, &mut scratch);
    assert!(
        (pose[0].translation.x - 5.0).abs() < 1e-4,
        "halfway through the fade the pose is halfway between the clips, got {}",
        pose[0].translation.x
    );
    // The unanimated bone is untouched by either clip.
    assert_eq!(pose[1].translation, Vec3::ZERO);

    // Fade done → pure Walk pose.
    m.tick(&plan, &mut params, 0.3);
    evaluate_pose(&m, &plan, &params, &clip_for, &mut pose, &mut scratch);
    assert!((pose[0].translation.x - 10.0).abs() < 1e-4);
}

// ---------------------------------------------------------------------------
// Blend trees (1D/2D blends, sync groups, crossfading against trees)
// ---------------------------------------------------------------------------

#[test]
fn a_state_region_compiles_to_a_blend_tree() {
    let compiled = compile_anim_graph(&blend1d_doc()).expect("compiles");
    let plan::PlanTree::Blend1D { param, children } = &compiled.states[0].tree else {
        panic!("expected a 1D blend, got {:?}", compiled.states[0].tree);
    };
    assert_eq!(param, "speed");
    assert_eq!(
        children.iter().map(|(t, _)| *t).collect::<Vec<_>>(),
        vec![0.0, 6.0],
        "children sorted by threshold"
    );
    assert!(children
        .iter()
        .all(|(_, c)| matches!(c, plan::PlanTree::Clip(_))));
    assert_eq!(
        compiled.clip_refs(),
        vec!["anims/run.anim", "anims/walk.anim"],
        "clip refs walk the tree"
    );
}

#[test]
fn blend1d_endpoints_play_pure_clips_and_midpoints_blend_proportionally() {
    let plan = compile_anim_graph(&blend1d_doc()).expect("compiles");
    let clips = [
        ("anims/walk.anim", constant_clip("Walk", 2.0)),
        ("anims/run.anim", constant_clip("Run", 10.0)),
    ];
    let clip_for = resolver(&clips);
    let mut params = AnimParams::from_decls(&plan.parameters);
    let mut m = AnimMachine::new(&plan);
    m.tick(&plan, &mut params, 0.1);
    assert_eq!(plan.states[m.current_state()].name, "Move");

    let mut pose = vec![LocalBoneTransform::default(); 2];
    let mut scratch = PoseScratch::new();
    for (speed, expected) in [
        (0.0, 2.0),   // low endpoint: pure Walk
        (-5.0, 2.0),  // below the range clamps to the endpoint
        (6.0, 10.0),  // high endpoint: pure Run
        (50.0, 10.0), // above the range clamps
        (3.0, 6.0),   // midpoint: 50/50
        (1.5, 4.0),   // quarter: 0.75·Walk + 0.25·Run
    ] {
        params.set_float("speed", speed);
        evaluate_pose(&m, &plan, &params, &clip_for, &mut pose, &mut scratch);
        assert!(
            (pose[0].translation.x - expected).abs() < 1e-4,
            "speed {speed}: pose {} expected {expected}",
            pose[0].translation.x
        );
    }
    // The unanimated bone is untouched by any child.
    assert_eq!(pose[1].translation, Vec3::ZERO);
}

#[test]
fn blend2d_blends_the_directionally_adjacent_children() {
    let plan = compile_anim_graph(&blend2d_doc()).expect("compiles");
    let clips = [
        ("anims/east.anim", constant_clip("East", 10.0)),
        ("anims/north.anim", constant_clip("North", 20.0)),
        ("anims/west.anim", constant_clip("West", 30.0)),
        ("anims/south.anim", constant_clip("South", 40.0)),
    ];
    let clip_for = resolver(&clips);
    let mut params = AnimParams::from_decls(&plan.parameters);
    let mut m = AnimMachine::new(&plan);
    m.tick(&plan, &mut params, 0.1);

    let mut pose = vec![LocalBoneTransform::default(); 2];
    let mut scratch = PoseScratch::new();
    for (x, y, expected, why) in [
        (1.0, 0.0, 10.0, "cardinal east is pure"),
        (0.0, 1.0, 20.0, "cardinal north is pure"),
        (-1.0, 0.0, 30.0, "cardinal west is pure"),
        (0.0, -1.0, 40.0, "cardinal south is pure"),
        (0.7, 0.7, 15.0, "north-east splits east/north 50/50"),
        (-0.7, 0.7, 25.0, "north-west splits north/west 50/50"),
        (0.7, -0.7, 25.0, "south-east splits south/east across the wrap"),
        (0.1, 0.1, 15.0, "magnitude does not matter, only direction"),
        (0.0, 0.0, 10.0, "a zero input has no direction — holds the first child"),
    ] {
        params.set_float("dir_x", x);
        params.set_float("dir_y", y);
        evaluate_pose(&m, &plan, &params, &clip_for, &mut pose, &mut scratch);
        assert!(
            (pose[0].translation.x - expected).abs() < 1e-3,
            "({x}, {y}): pose {} expected {expected} — {why}",
            pose[0].translation.x
        );
    }
}

#[test]
fn sync_group_keeps_cyclic_clips_phase_aligned_as_weights_shift() {
    // Walk (1.0s) and Run (0.4s) each report their own normalized phase as a
    // pose value. Phase-matched, both always read the same number, so the
    // blend reads that number too — whatever the weights are. The reference
    // is the first sorted child (Walk): expected phase = t mod 1.0.
    let plan = compile_anim_graph(&blend1d_doc()).expect("compiles");
    let clips = [
        ("anims/walk.anim", phase_clip("Walk", 1.0)),
        ("anims/run.anim", phase_clip("Run", 0.4)),
    ];
    let clip_for = resolver(&clips);
    let mut params = AnimParams::from_decls(&plan.parameters);
    let mut m = AnimMachine::new(&plan);

    let mut pose = vec![LocalBoneTransform::default(); 1];
    let mut scratch = PoseScratch::new();
    for step in 0..30 {
        // Sweep pure-Walk → pure-Run while the clock runs: the dominant clip
        // changes mid-sweep, and the phase must never jump.
        params.set_float("speed", 6.0 * step as f32 / 29.0);
        m.tick(&plan, &mut params, 0.05);
        evaluate_pose(&m, &plan, &params, &clip_for, &mut pose, &mut scratch);
        let expected = m.time().rem_euclid(1.0);
        assert!(
            (pose[0].translation.x - expected).abs() < 1e-3,
            "step {step}: pose {} expected phase {expected}",
            pose[0].translation.x
        );
    }
    // Sanity that the assertion has teeth: at the sweep's end (pure Run) an
    // unsynced Run on the raw clock would read a different value.
    let unsynced = m.time().rem_euclid(0.4) / 0.4;
    assert!((unsynced - m.time().rem_euclid(1.0)).abs() > 0.1);
}

#[test]
fn a_blend_tree_state_crossfades_against_a_plain_clip_state() {
    // two_state_doc's Walk state becomes a blend-tree state: Idle (plain
    // clip) crossfades into a walk/run tree over 0.5s.
    let mut doc = two_state_doc();
    doc.variables.push(float_decl("speed"));
    doc.node_mut(3).unwrap().properties.remove(plan::CLIP_PROP);
    doc.regions.insert(3, walk_run_tree());
    let plan = compile_anim_graph(&doc).expect("compiles");

    let clips = [
        ("anims/idle.anim", constant_clip("Idle", 0.0)),
        ("anims/walk.anim", constant_clip("Walk", 2.0)),
        ("anims/run.anim", constant_clip("Run", 10.0)),
    ];
    let clip_for = resolver(&clips);
    let mut params = AnimParams::from_decls(&plan.parameters);
    let mut m = AnimMachine::new(&plan);
    let mut pose = vec![LocalBoneTransform::default(); 2];
    let mut scratch = PoseScratch::new();

    params.set_float("speed", 3.0); // the tree blends to 6.0
    params.set_bool("walk", true);
    m.tick(&plan, &mut params, 0.1); // fade starts, elapsed 0.0
    m.tick(&plan, &mut params, 0.1);
    m.tick(&plan, &mut params, 0.1);
    m.tick(&plan, &mut params, 0.05); // elapsed 0.25 of 0.5 → weight 0.5
    evaluate_pose(&m, &plan, &params, &clip_for, &mut pose, &mut scratch);
    assert!(
        (pose[0].translation.x - 3.0).abs() < 1e-4,
        "halfway: 0.5·Idle(0) + 0.5·tree(6) = 3, got {}",
        pose[0].translation.x
    );
    assert_eq!(pose[1].translation, Vec3::ZERO, "unanimated bone untouched");

    // Fade done → the tree alone.
    m.tick(&plan, &mut params, 0.3);
    evaluate_pose(&m, &plan, &params, &clip_for, &mut pose, &mut scratch);
    assert!((pose[0].translation.x - 6.0).abs() < 1e-4);
}

#[test]
fn nested_blends_evaluate_recursively() {
    // An inner 1D blend (on `lean`) as a child of the outer 1D blend (on
    // `speed`): Walk vs (A↔B).
    let mut doc = blend1d_doc();
    doc.variables.push(float_decl("lean"));
    doc.regions.insert(
        2,
        GraphRegion {
            nodes: vec![
                clip_node(0, "anims/walk.anim"),
                clip_node(1, "anims/a.anim"),
                clip_node(2, "anims/b.anim"),
                with(
                    3,
                    plan::ANIM_BLEND1D_TYPE_ID,
                    None,
                    &[
                        (plan::BLEND_PARAM_PROP, PropValue::Str("lean".into())),
                        ("threshold_0", PropValue::Float(0.0)),
                        ("threshold_1", PropValue::Float(1.0)),
                    ],
                ),
                with(
                    4,
                    plan::ANIM_BLEND1D_TYPE_ID,
                    None,
                    &[
                        (plan::BLEND_PARAM_PROP, PropValue::Str("speed".into())),
                        ("threshold_0", PropValue::Float(0.0)),
                        ("threshold_1", PropValue::Float(6.0)),
                    ],
                ),
                node(5, plan::ANIM_POSE_RESULT_TYPE_ID, None),
            ],
            edges: vec![
                edge(1, plan::POSE_PIN, 3, "in_0"),
                edge(2, plan::POSE_PIN, 3, "in_1"),
                edge(0, plan::POSE_PIN, 4, "in_0"),
                edge(3, plan::POSE_PIN, 4, "in_1"),
                edge(4, plan::POSE_PIN, 5, plan::POSE_PIN),
            ],
        },
    );
    let plan = compile_anim_graph(&doc).expect("compiles");
    let clips = [
        ("anims/walk.anim", constant_clip("Walk", 2.0)),
        ("anims/a.anim", constant_clip("A", 10.0)),
        ("anims/b.anim", constant_clip("B", 20.0)),
    ];
    let clip_for = resolver(&clips);
    let mut params = AnimParams::from_decls(&plan.parameters);
    let mut m = AnimMachine::new(&plan);
    m.tick(&plan, &mut params, 0.1);

    let mut pose = vec![LocalBoneTransform::default(); 1];
    let mut scratch = PoseScratch::new();
    for (speed, lean, expected) in [
        (0.0, 0.5, 2.0),  // outer low endpoint: pure Walk
        (6.0, 0.5, 15.0), // outer high endpoint: the inner blend, 50/50
        (6.0, 1.0, 20.0), // inner endpoint through the outer endpoint
        (3.0, 0.5, 8.5),  // 0.5·Walk(2) + 0.5·inner(15)
    ] {
        params.set_float("speed", speed);
        params.set_float("lean", lean);
        evaluate_pose(&m, &plan, &params, &clip_for, &mut pose, &mut scratch);
        assert!(
            (pose[0].translation.x - expected).abs() < 1e-4,
            "speed {speed} lean {lean}: pose {} expected {expected}",
            pose[0].translation.x
        );
    }
}

#[test]
fn blend_tree_compile_refusals_are_author_errors() {
    // A rule node — or anything else off the whitelist — inside a tree.
    let mut doc = blend1d_doc();
    doc.regions
        .get_mut(&2)
        .unwrap()
        .nodes
        .push(node(9, VAR_GET_TYPE_ID, None));
    let err = compile_anim_graph(&doc).unwrap_err();
    assert!(err.contains("not a blend-tree node"), "{err}");

    // Exactly one RESULT sink.
    let mut doc = blend1d_doc();
    doc.regions
        .get_mut(&2)
        .unwrap()
        .nodes
        .push(node(9, plan::ANIM_POSE_RESULT_TYPE_ID, None));
    assert!(compile_anim_graph(&doc)
        .unwrap_err()
        .contains("exactly one RESULT"));

    // A non-empty tree with no RESULT at all.
    let mut doc = blend1d_doc();
    let region = doc.regions.get_mut(&2).unwrap();
    region.nodes.retain(|n| n.type_id != plan::ANIM_POSE_RESULT_TYPE_ID);
    region.edges.clear();
    assert!(compile_anim_graph(&doc).unwrap_err().contains("no RESULT"));

    // A RESULT with nothing wired in: a state must produce a pose — no
    // "always-true" reading exists here.
    let mut doc = blend1d_doc();
    doc.regions
        .get_mut(&2)
        .unwrap()
        .edges
        .retain(|e| e.to_node != 3);
    assert!(compile_anim_graph(&doc)
        .unwrap_err()
        .contains("nothing is wired into"));

    // Fan-in: two wires into the RESULT.
    let mut doc = blend1d_doc();
    doc.regions
        .get_mut(&2)
        .unwrap()
        .edges
        .push(edge(0, plan::POSE_PIN, 3, plan::POSE_PIN));
    assert!(compile_anim_graph(&doc).unwrap_err().contains("has 2 wires"));

    // Fewer than two wired children.
    let mut doc = blend1d_doc();
    doc.regions
        .get_mut(&2)
        .unwrap()
        .edges
        .retain(|e| e.to_pin != "in_1");
    assert!(compile_anim_graph(&doc)
        .unwrap_err()
        .contains("at least two"));

    // A wired child with no threshold.
    let mut doc = blend1d_doc();
    region_node_props(&mut doc, 2, 2).remove("threshold_1");
    let err = compile_anim_graph(&doc).unwrap_err();
    assert!(err.contains("threshold_1"), "{err}");

    // Two children sharing a threshold.
    let mut doc = blend1d_doc();
    region_node_props(&mut doc, 2, 2)
        .insert("threshold_1".into(), PropValue::Float(0.0));
    assert!(compile_anim_graph(&doc)
        .unwrap_err()
        .contains("sharing a threshold"));

    // The driving parameter must exist…
    let mut doc = blend1d_doc();
    region_node_props(&mut doc, 2, 2)
        .insert(plan::BLEND_PARAM_PROP.into(), PropValue::Str("missing".into()));
    assert!(compile_anim_graph(&doc)
        .unwrap_err()
        .contains("not declared"));

    // …and be a Float.
    let mut doc = blend1d_doc();
    doc.variables[0].ty = PinType::Bool;
    assert!(compile_anim_graph(&doc)
        .unwrap_err()
        .contains("not a Float"));

    // A cycle refuses rather than hanging the compiler.
    let mut doc = blend1d_doc();
    let region = doc.regions.get_mut(&2).unwrap();
    region.edges.retain(|e| e.to_pin != "in_0");
    region.edges.push(edge(2, plan::POSE_PIN, 2, "in_0"));
    assert!(compile_anim_graph(&doc).unwrap_err().contains("cycle"));

    // A clip node that names no clip.
    let mut doc = blend1d_doc();
    region_node_props(&mut doc, 2, 0).remove(plan::CLIP_PROP);
    assert!(compile_anim_graph(&doc).unwrap_err().contains("names no clip"));

    // 2D: a child with a zero direction…
    let mut doc = blend2d_doc();
    region_node_props(&mut doc, 2, 4)
        .insert("x_1".into(), PropValue::Float(0.0));
    region_node_props(&mut doc, 2, 4)
        .insert("y_1".into(), PropValue::Float(0.0));
    assert!(compile_anim_graph(&doc)
        .unwrap_err()
        .contains("zero direction"));

    // …and two children sharing a direction (collinear counts).
    let mut doc = blend2d_doc();
    region_node_props(&mut doc, 2, 4)
        .insert("x_1".into(), PropValue::Float(2.0));
    region_node_props(&mut doc, 2, 4)
        .insert("y_1".into(), PropValue::Float(0.0));
    assert!(compile_anim_graph(&doc)
        .unwrap_err()
        .contains("sharing a direction"));
}

#[test]
fn a_blend_tree_round_trips_and_dies_with_its_state() {
    let doc = blend1d_doc();
    let back = parse_graph(&serialize_graph(&doc).unwrap()).unwrap();
    assert_eq!(back, doc, "the tree region serializes with the parent");
    assert_eq!(
        compile_anim_graph(&back).expect("compiles"),
        compile_anim_graph(&doc).expect("compiles")
    );

    let mut doc = doc;
    assert!(doc.remove_node(2));
    assert!(doc.regions.get(&2).is_none(), "the tree died with its state");
}

// ---------------------------------------------------------------------------
// System level: arming, invalidation, coexistence
// ---------------------------------------------------------------------------

/// In-memory assets behind a mutex, so a test can edit the "file" and prove
/// invalidation — the same trick the script runtime's acceptance tests use.
#[derive(Default, Clone)]
struct MapAssets {
    graphs: Arc<Mutex<BTreeMap<String, GraphDoc>>>,
}

impl AnimAssetLoader for MapAssets {
    fn load_graph(&self, content_rel: &str) -> Option<GraphDoc> {
        self.graphs.lock().ok()?.get(content_rel).cloned()
    }

    fn load_clips(&self, content_rel: &str) -> Option<ClipSet> {
        let (name, x) = match content_rel {
            "anims/idle.anim" => ("Idle", 0.0),
            "anims/walk.anim" => ("Walk", 10.0),
            "anims/run.anim" => ("Run", 20.0),
            _ => return None,
        };
        let mut clip = constant_clip(name, x);
        if name == "Idle" {
            // One marker for the system-level event test.
            clip.events.push(AnimEventMarker {
                time_seconds: 0.05,
                name: "step".into(),
            });
        }
        Some(ClipSet {
            bone_names: vec!["root".into(), "child".into()],
            clips: vec![clip],
        })
    }

    fn load_skeleton(&self, mesh_content_rel: &str) -> Option<Vec<BoneData>> {
        (mesh_content_rel == "hero.mesh").then(synthetic_bones)
    }
}

struct Harness {
    world: hecs::World,
    resources: Resources,
    system: AnimGraphSystem,
}

impl Harness {
    fn new(assets: MapAssets) -> Self {
        let mut resources = Resources::new();
        let mut time = Time::new();
        time.delta = 0.1;
        resources.insert(time);
        resources.insert(AnimGraphPlanCache::new());
        resources.insert(AnimClipCache::new());
        Self {
            world: hecs::World::new(),
            resources,
            system: AnimGraphSystem::new(Box::new(assets)),
        }
    }

    fn tick(&mut self) {
        self.system.run(&mut self.world, &mut self.resources);
    }
}

const GRAPH: &str = "graphs/hero.animgraph";

#[test]
fn the_system_arms_ticks_and_restarts_on_invalidation() {
    let assets = MapAssets::default();
    assets
        .graphs
        .lock()
        .unwrap()
        .insert(GRAPH.into(), two_state_doc());
    let mut h = Harness::new(assets.clone());

    let e = h.world.spawn((
        AnimGraphRunner::new(GRAPH),
        SkeletonInstance::from_bones(synthetic_bones()),
    ));

    // First tick arms the machine in the entry state and poses the skeleton.
    h.tick();
    {
        let rt = h.world.get::<&AnimGraphRuntime>(e).expect("armed");
        assert!(rt.disabled.is_none(), "{:?}", rt.disabled);
        assert_eq!(rt.plan.states[rt.machine.current_state()].name, "Idle");
    }

    // Gameplay writes the parameter; the machine crossfades into Walk and
    // the blended pose lands in the skeleton.
    h.world
        .get::<&mut AnimGraphRuntime>(e)
        .unwrap()
        .params
        .set_bool("walk", true);
    h.tick(); // transition fires (weight 0)
    h.tick();
    h.tick(); // elapsed 0.2s of 0.5s → walk weight 0.4
    {
        let rt = h.world.get::<&AnimGraphRuntime>(e).unwrap();
        assert_eq!(rt.plan.states[rt.machine.current_state()].name, "Walk");
        let sk = h.world.get::<&SkeletonInstance>(e).unwrap();
        let x = sk.local_transforms[0].translation.x;
        assert!((x - 4.0).abs() < 1e-3, "blended pose reached the skeleton: {x}");
    }

    // Edit the document (retitle a state) and save: the cache invalidates,
    // and the stale plan never runs again — the machine re-arms against the
    // new plan, back at ENTRY.
    {
        let mut graphs = assets.graphs.lock().unwrap();
        let doc = graphs.get_mut(GRAPH).unwrap();
        doc.node_mut(2).unwrap().title = Some("IdleV2".into());
    }
    h.resources
        .get_mut::<AnimGraphPlanCache>()
        .unwrap()
        .invalidate(GRAPH);
    h.tick(); // drops the stale runtime
    h.tick(); // re-arms against the fresh compile
    {
        let rt = h.world.get::<&AnimGraphRuntime>(e).expect("re-armed");
        assert_eq!(
            rt.generation,
            h.resources.get::<AnimGraphPlanCache>().unwrap().generation(),
            "the runtime rides the new generation"
        );
        assert_eq!(
            rt.plan.states[rt.machine.current_state()].name, "IdleV2",
            "the edited document is what runs"
        );
    }
}

#[test]
fn the_system_runs_a_blend_tree_state_end_to_end() {
    let assets = MapAssets::default();
    assets
        .graphs
        .lock()
        .unwrap()
        .insert(GRAPH.into(), blend1d_doc());
    let mut h = Harness::new(assets);

    let e = h.world.spawn((
        AnimGraphRunner::new(GRAPH),
        SkeletonInstance::from_bones(synthetic_bones()),
    ));
    h.tick(); // arms (both tree clips prefetched) and evaluates
    {
        let rt = h.world.get::<&AnimGraphRuntime>(e).expect("armed");
        assert!(rt.disabled.is_none(), "{:?}", rt.disabled);
    }
    h.world
        .get::<&mut AnimGraphRuntime>(e)
        .unwrap()
        .params
        .set_float("speed", 3.0);
    h.tick();
    let sk = h.world.get::<&SkeletonInstance>(e).unwrap();
    let x = sk.local_transforms[0].translation.x;
    assert!(
        (x - 15.0).abs() < 1e-3,
        "the 50/50 walk/run blend reached the skeleton: {x}"
    );
}

#[test]
fn arming_attaches_a_skeleton_from_the_entitys_mesh() {
    let assets = MapAssets::default();
    assets
        .graphs
        .lock()
        .unwrap()
        .insert(GRAPH.into(), two_state_doc());
    let mut h = Harness::new(assets);

    let e = h.world.spawn((
        AnimGraphRunner::new(GRAPH),
        MeshRenderer {
            mesh_path: "hero.mesh".into(),
            ..Default::default()
        },
    ));
    h.tick();
    assert!(
        h.world.get::<&SkeletonInstance>(e).is_ok(),
        "a skinned mesh without an instance gets one at arm time"
    );
    assert!(h.world.get::<&AnimGraphRuntime>(e).unwrap().disabled.is_none());
}

#[test]
fn entities_without_a_graph_keep_the_single_clip_player() {
    let assets = MapAssets::default();
    assets
        .graphs
        .lock()
        .unwrap()
        .insert(GRAPH.into(), two_state_doc());
    let mut h = Harness::new(assets);

    // A graph-driven entity that *also* carries a player (worst case), and a
    // plain player entity.
    let mut stray_player = AnimationPlayer::new(constant_clip("Stray", 1.0));
    stray_player.play();
    let graphed = h.world.spawn((
        AnimGraphRunner::new(GRAPH),
        SkeletonInstance::from_bones(synthetic_bones()),
        stray_player,
    ));
    let mut plain_player = AnimationPlayer::new(constant_clip("Plain", 7.0));
    plain_player.play();
    let plain = h
        .world
        .spawn((plain_player, SkeletonInstance::from_bones(synthetic_bones())));

    h.tick(); // arm the graph entity
    let mut legacy = AnimationUpdateSystem;
    legacy.run(&mut h.world, &mut h.resources);

    // The plain entity animated through the existing player…
    let sk = h.world.get::<&SkeletonInstance>(plain).unwrap();
    assert!((sk.local_transforms[0].translation.x - 7.0).abs() < 1e-4);
    let time = h.world.get::<&AnimationPlayer>(plain).unwrap().time;
    assert!(time > 0.0, "the single-clip player still advances");

    // …while the graph-driven entity's stray player was left alone.
    let stray_time = h.world.get::<&AnimationPlayer>(graphed).unwrap().time;
    assert_eq!(stray_time, 0.0, "the graph owns this skeleton; the player is inert");
}

#[test]
fn a_missing_clip_refuses_to_arm_with_a_reason() {
    let assets = MapAssets::default();
    let mut doc = two_state_doc();
    doc.node_mut(3)
        .unwrap()
        .properties
        .insert(plan::CLIP_PROP.into(), PropValue::Asset("anims/missing.anim".into()));
    assets.graphs.lock().unwrap().insert(GRAPH.into(), doc);
    let mut h = Harness::new(assets);

    let e = h.world.spawn((
        AnimGraphRunner::new(GRAPH),
        SkeletonInstance::from_bones(synthetic_bones()),
    ));
    h.tick();
    let rt = h.world.get::<&AnimGraphRuntime>(e).unwrap();
    let why = rt.disabled.as_deref().expect("refused");
    assert!(why.contains("Walk") && why.contains("missing.anim"), "{why}");
}

// ---------------------------------------------------------------------------
// Play-once slot and anim events (ticket 07)
// ---------------------------------------------------------------------------

/// [`constant_clip`] with a chosen duration and anim event markers.
fn marked_clip(name: &str, x: f32, duration: f32, marks: &[(f32, &str)]) -> RawAnimationClip {
    let mut clip = constant_clip(name, x);
    clip.duration_seconds = duration;
    clip.events = marks
        .iter()
        .map(|(t, n)| AnimEventMarker {
            time_seconds: *t,
            name: n.to_string(),
        })
        .collect();
    clip
}

/// `two_state_doc` plus the Trigger `attack` and a play-once slot node
/// playing `anims/attack.anim` on it.
fn slot_doc() -> GraphDoc {
    let mut doc = two_state_doc();
    doc.variables.push(trigger_decl("attack"));
    doc.nodes.push(with(
        9,
        plan::ANIM_PLAY_ONCE_TYPE_ID,
        Some("Attack"),
        &[
            (plan::CLIP_PROP, PropValue::Asset("anims/attack.anim".into())),
            (plan::SLOT_TRIGGER_PROP, PropValue::Str("attack".into())),
        ],
    ));
    doc
}

/// One frame at the evaluator seam, in the runner's exact order: machine tick
/// → slot tick → event collection → pose evaluation → play-once overlay.
#[allow(clippy::too_many_arguments)]
fn tick_frame<'a, F>(
    plan: &AnimGraphPlan,
    m: &mut AnimMachine,
    slot: &mut PlayOnceSlot,
    params: &mut AnimParams,
    clip_for: &F,
    dt: f32,
    pose: &mut [LocalBoneTransform],
    scratch: &mut PoseScratch,
    events: &mut Vec<AnimEventFire>,
) where
    F: Fn(&plan::PlanClip) -> Option<&'a RawAnimationClip>,
{
    m.tick(plan, params, dt);
    slot.tick(plan, params, dt, clip_for);
    collect_anim_events(m, slot, plan, params, clip_for, events);
    evaluate_pose(m, plan, params, clip_for, pose, scratch);
    slot.apply(plan, clip_for, pose, scratch);
}

fn count(events: &[AnimEventFire], name: &str) -> usize {
    events.iter().filter(|e| e.name == name).count()
}

#[test]
fn a_play_once_slot_compiles_and_round_trips() {
    let doc = slot_doc();
    let compiled = compile_anim_graph(&doc).expect("compiles");
    assert_eq!(compiled.slots.len(), 1);
    let s = &compiled.slots[0];
    assert_eq!(s.name, "Attack");
    assert_eq!(s.clip.clip, "anims/attack.anim");
    assert_eq!(s.trigger, "attack");
    assert_eq!((s.speed, s.fade_in, s.fade_out), (1.0, 0.0, 0.0));
    assert_eq!(
        compiled.clip_refs(),
        vec!["anims/attack.anim", "anims/idle.anim", "anims/walk.anim"],
        "slot clips prefetch with the rest"
    );
    // The slot node round-trips through the shared container io.
    let back = parse_graph(&serialize_graph(&doc).unwrap()).unwrap();
    assert_eq!(compile_anim_graph(&back).expect("compiles"), compiled);
}

#[test]
fn play_once_compile_refusals_are_author_errors() {
    // No clip.
    let mut doc = slot_doc();
    doc.node_mut(9).unwrap().properties.remove(plan::CLIP_PROP);
    let err = compile_anim_graph(&doc).unwrap_err();
    assert!(err.contains("Attack") && err.contains("names no clip"), "{err}");

    // No trigger.
    let mut doc = slot_doc();
    doc.node_mut(9).unwrap().properties.remove(plan::SLOT_TRIGGER_PROP);
    assert!(compile_anim_graph(&doc).unwrap_err().contains("names no trigger"));

    // An undeclared trigger.
    let mut doc = slot_doc();
    doc.variables.retain(|v| v.slug != "attack");
    assert!(compile_anim_graph(&doc).unwrap_err().contains("not declared"));

    // A declared parameter that is not a Trigger.
    let mut doc = slot_doc();
    doc.node_mut(9)
        .unwrap()
        .properties
        .insert(plan::SLOT_TRIGGER_PROP.into(), PropValue::Str("walk".into()));
    assert!(compile_anim_graph(&doc).unwrap_err().contains("not a Trigger"));
}

#[test]
fn play_once_overlays_the_base_and_returns_when_the_clip_finishes() {
    let plan = compile_anim_graph(&slot_doc()).expect("compiles");
    let clips = [
        ("anims/idle.anim", constant_clip("Idle", 0.0)),
        ("anims/walk.anim", constant_clip("Walk", 5.0)),
        ("anims/attack.anim", marked_clip("Attack", 10.0, 0.3, &[])),
    ];
    let clip_for = resolver(&clips);
    let mut params = AnimParams::from_decls(&plan.parameters);
    let mut m = AnimMachine::new(&plan);
    let mut slot = PlayOnceSlot::new();
    let mut pose = vec![LocalBoneTransform::default(); 1];
    let mut scratch = PoseScratch::new();
    let mut events = Vec::new();
    macro_rules! frame {
        () => {
            tick_frame(
                &plan, &mut m, &mut slot, &mut params, &clip_for, 0.1, &mut pose, &mut scratch,
                &mut events,
            )
        };
    }

    // Base result before any request: Idle.
    frame!();
    assert_eq!(pose[0].translation.x, 0.0);
    assert!(slot.playing().is_none());

    // Gameplay fires the Trigger — parameters in, never the slot directly.
    assert!(params.fire_trigger("attack"));
    frame!();
    assert_eq!(slot.playing(), Some(0));
    assert_eq!(
        params.trigger_set("attack"),
        Some(false),
        "consume-on-start, like consume-on-transition"
    );
    assert_eq!(slot.weight(&plan), 1.0, "no fades: full override");
    assert_eq!(pose[0].translation.x, 10.0, "the overlay replaces the base");

    // Mid-clip: still the overlay.
    frame!();
    frame!();
    assert_eq!(pose[0].translation.x, 10.0);

    // The clock passes the clip end (0.3s): the base result returns, and the
    // machine underneath never noticed — same state, clock kept running.
    frame!();
    assert_eq!(slot.weight(&plan), 0.0, "finished: the overlay contributes nothing");
    assert_eq!(pose[0].translation.x, 0.0, "back to the base result");
    assert_eq!(plan.states[m.current_state()].name, "Idle");

    // The playback retires shortly after (bookkeeping, not visible).
    frame!();
    frame!();
    assert!(slot.playing().is_none(), "the channel is free again");

    // And a fresh fire plays it again from the top.
    params.fire_trigger("attack");
    frame!();
    assert_eq!(slot.playing(), Some(0));
    assert_eq!(pose[0].translation.x, 10.0);
}

#[test]
fn slot_fades_ramp_the_overlay_weight() {
    let mut doc = slot_doc();
    doc.node_mut(9).unwrap().properties.extend([
        (plan::SLOT_FADE_IN_PROP.to_string(), PropValue::Float(0.2)),
        (plan::SLOT_FADE_OUT_PROP.to_string(), PropValue::Float(0.2)),
    ]);
    let plan = compile_anim_graph(&doc).expect("compiles");
    let clips = [
        ("anims/idle.anim", constant_clip("Idle", 0.0)),
        ("anims/walk.anim", constant_clip("Walk", 5.0)),
        ("anims/attack.anim", marked_clip("Attack", 10.0, 1.0, &[])),
    ];
    let clip_for = resolver(&clips);
    let mut params = AnimParams::from_decls(&plan.parameters);
    let mut m = AnimMachine::new(&plan);
    let mut slot = PlayOnceSlot::new();
    let mut pose = vec![LocalBoneTransform::default(); 1];
    let mut scratch = PoseScratch::new();
    let mut events = Vec::new();
    macro_rules! frame {
        () => {
            tick_frame(
                &plan, &mut m, &mut slot, &mut params, &clip_for, 0.1, &mut pose, &mut scratch,
                &mut events,
            )
        };
    }

    params.fire_trigger("attack");
    // Start tick: t = 0.0 → weight 0 (the overlay blends in from zero, the
    // same posture a crossfade target has).
    frame!();
    assert_eq!(slot.weight(&plan), 0.0);
    assert_eq!(pose[0].translation.x, 0.0);
    // t = 0.1 → halfway up the 0.2s fade-in.
    frame!();
    assert!((slot.weight(&plan) - 0.5).abs() < 1e-4);
    assert!((pose[0].translation.x - 5.0).abs() < 1e-4);
    // t = 0.5 → plateau.
    for _ in 0..4 {
        frame!();
    }
    assert_eq!(slot.weight(&plan), 1.0);
    // t = 0.9 → halfway down the 0.2s fade-out ((1.0 − 0.9) / 0.2).
    for _ in 0..4 {
        frame!();
    }
    assert!((slot.weight(&plan) - 0.5).abs() < 1e-4);
    assert!((pose[0].translation.x - 5.0).abs() < 1e-4);
}

#[test]
fn the_channel_is_single_a_later_start_replaces_and_buffered_triggers_wait() {
    // A second slot on its own trigger. Node 10 sorts after node 9, so with
    // both triggers set the same tick, slot 9 takes the channel first.
    let mut doc = slot_doc();
    doc.variables.push(trigger_decl("hurt"));
    doc.nodes.push(with(
        10,
        plan::ANIM_PLAY_ONCE_TYPE_ID,
        Some("Hurt"),
        &[
            (plan::CLIP_PROP, PropValue::Asset("anims/hurt.anim".into())),
            (plan::SLOT_TRIGGER_PROP, PropValue::Str("hurt".into())),
        ],
    ));
    let plan = compile_anim_graph(&doc).expect("compiles");
    let clips = [
        ("anims/idle.anim", constant_clip("Idle", 0.0)),
        ("anims/walk.anim", constant_clip("Walk", 5.0)),
        ("anims/attack.anim", marked_clip("Attack", 10.0, 1.0, &[])),
        ("anims/hurt.anim", marked_clip("Hurt", 20.0, 1.0, &[])),
    ];
    let clip_for = resolver(&clips);
    let mut params = AnimParams::from_decls(&plan.parameters);
    let mut m = AnimMachine::new(&plan);
    let mut slot = PlayOnceSlot::new();
    let mut pose = vec![LocalBoneTransform::default(); 1];
    let mut scratch = PoseScratch::new();
    let mut events = Vec::new();
    macro_rules! frame {
        () => {
            tick_frame(
                &plan, &mut m, &mut slot, &mut params, &clip_for, 0.1, &mut pose, &mut scratch,
                &mut events,
            )
        };
    }

    params.fire_trigger("attack");
    params.fire_trigger("hurt");
    frame!();
    assert_eq!(slot.playing(), Some(0), "plan order takes the channel");
    assert_eq!(params.trigger_set("attack"), Some(false), "consumed by the start");
    assert_eq!(
        params.trigger_set("hurt"),
        Some(true),
        "one start per tick — the loser stays buffered, never lost"
    );

    // Next tick the buffered trigger takes the channel, replacing: one
    // override channel is the v1 contract.
    frame!();
    assert_eq!(slot.playing(), Some(1));
    assert_eq!(params.trigger_set("hurt"), Some(false));
    assert_eq!(pose[0].translation.x, 20.0, "the replacement plays from its top");
}

// ---------------------------------------------------------------------------
// Anim events: crossings, cycles, blend-weight suppression
// ---------------------------------------------------------------------------

#[test]
fn anim_events_fire_once_per_crossing_and_refire_each_cycle() {
    // Idle loops a 1.0s clip with markers at 0.0 and 0.5. Ticking 0.2s at a
    // time for 1.8s crosses each marker exactly twice (at 0.0/1.0 and
    // 0.5/1.5) — and each individual tick fires a marker at most once.
    let plan = compile_anim_graph(&two_state_doc()).expect("compiles");
    let clips = [
        (
            "anims/idle.anim",
            marked_clip("Idle", 0.0, 1.0, &[(0.0, "loop"), (0.5, "mid")]),
        ),
        ("anims/walk.anim", constant_clip("Walk", 5.0)),
    ];
    let clip_for = resolver(&clips);
    let mut params = AnimParams::from_decls(&plan.parameters);
    let mut m = AnimMachine::new(&plan);
    let mut slot = PlayOnceSlot::new();
    let mut pose = vec![LocalBoneTransform::default(); 1];
    let mut scratch = PoseScratch::new();
    let mut events = Vec::new();

    let (mut loops, mut mids) = (0, 0);
    for _ in 0..9 {
        tick_frame(
            &plan, &mut m, &mut slot, &mut params, &clip_for, 0.2, &mut pose, &mut scratch,
            &mut events,
        );
        assert!(count(&events, "loop") <= 1 && count(&events, "mid") <= 1);
        loops += count(&events, "loop");
        mids += count(&events, "mid");
        assert!(events.iter().all(|e| e.weight == 1.0), "at rest, full weight");
    }
    assert_eq!((loops, mids), (2, 2), "one fire per crossing, refired per cycle");

    // A held clock (speed 0 would do the same) crosses nothing.
    tick_frame(
        &plan, &mut m, &mut slot, &mut params, &clip_for, 0.0, &mut pose, &mut scratch,
        &mut events,
    );
    assert!(events.is_empty(), "a zero-length span fires nothing");
}

#[test]
fn no_events_fire_from_a_fully_blended_out_clip() {
    // Walk and Run both carry a marker; the 1D blend's weight decides who may
    // fire. Same 1.0s duration keeps the sync phase equal to either clock.
    let plan = compile_anim_graph(&blend1d_doc()).expect("compiles");
    let clips = [
        ("anims/walk.anim", marked_clip("Walk", 2.0, 1.0, &[(0.5, "wstep")])),
        ("anims/run.anim", marked_clip("Run", 10.0, 1.0, &[(0.5, "rstep")])),
    ];
    let clip_for = resolver(&clips);
    let mut scratch = PoseScratch::new();
    let mut pose = vec![LocalBoneTransform::default(); 1];
    let mut events = Vec::new();

    for (speed, wstep, rstep, w_expected) in [
        (0.0, 1, 0, 1.0),  // pure Walk: Run is fully blended out — silent
        (6.0, 0, 1, 1.0),  // pure Run: Walk is silent
        (3.0, 1, 1, 0.5),  // 50/50: both fire, at their blend weight
    ] {
        let mut params = AnimParams::from_decls(&plan.parameters);
        let mut m = AnimMachine::new(&plan);
        let mut slot = PlayOnceSlot::new();
        params.set_float("speed", speed);
        let (mut ws, mut rs) = (0, 0);
        for _ in 0..5 {
            tick_frame(
                &plan, &mut m, &mut slot, &mut params, &clip_for, 0.2, &mut pose, &mut scratch,
                &mut events,
            );
            ws += count(&events, "wstep");
            rs += count(&events, "rstep");
            for e in &events {
                assert!(
                    (e.weight - w_expected).abs() < 1e-4,
                    "speed {speed}: weight {}",
                    e.weight
                );
            }
        }
        assert_eq!((ws, rs), (wstep, rstep), "speed {speed}");
    }
}

#[test]
fn blend_child_events_follow_the_sync_group_phase() {
    // Run (0.4s) under the walk/run blend at pure Run: the sync group drives
    // Run at Walk's (1.0s) phase, so Run's marker at 0.2 — phase 0.5 — fires
    // when the *state clock* crosses 0.5, not when it crosses 0.2.
    let plan = compile_anim_graph(&blend1d_doc()).expect("compiles");
    let clips = [
        ("anims/walk.anim", marked_clip("Walk", 2.0, 1.0, &[])),
        ("anims/run.anim", marked_clip("Run", 10.0, 0.4, &[(0.2, "rstep")])),
    ];
    let clip_for = resolver(&clips);
    let mut params = AnimParams::from_decls(&plan.parameters);
    let mut m = AnimMachine::new(&plan);
    let mut slot = PlayOnceSlot::new();
    let mut pose = vec![LocalBoneTransform::default(); 1];
    let mut scratch = PoseScratch::new();
    let mut events = Vec::new();

    params.set_float("speed", 6.0);
    let mut fired_at = Vec::new();
    for _ in 0..5 {
        tick_frame(
            &plan, &mut m, &mut slot, &mut params, &clip_for, 0.2, &mut pose, &mut scratch,
            &mut events,
        );
        if count(&events, "rstep") > 0 {
            fired_at.push(m.time());
        }
    }
    // Crossings of phase 0.5 within 1.0s of clock: the tick reaching 0.6
    // (span [0.4, 0.6)) — not the raw-clock tick reaching 0.2.
    assert_eq!(fired_at, vec![0.6], "phase space, not raw clip time");
}

#[test]
fn a_crossfade_keeps_the_outgoing_state_audible_and_an_instant_switch_does_not() {
    // Idle marker at 0.45, Walk marker at 0.25 (both 1.0s clips).
    let make_clips = || {
        [
            ("anims/idle.anim", marked_clip("Idle", 0.0, 1.0, &[(0.45, "istep")])),
            ("anims/walk.anim", marked_clip("Walk", 5.0, 1.0, &[(0.25, "wstep")])),
        ]
    };

    // With the 0.5s crossfade: the outgoing state's clock keeps firing at its
    // fading weight, and the target fires once its own clock reaches markers.
    let plan = compile_anim_graph(&two_state_doc()).expect("compiles");
    let clips = make_clips();
    let clip_for = resolver(&clips);
    let mut params = AnimParams::from_decls(&plan.parameters);
    let mut m = AnimMachine::new(&plan);
    let mut slot = PlayOnceSlot::new();
    let mut pose = vec![LocalBoneTransform::default(); 1];
    let mut scratch = PoseScratch::new();
    let mut events = Vec::new();
    macro_rules! frame {
        () => {
            tick_frame(
                &plan, &mut m, &mut slot, &mut params, &clip_for, 0.2, &mut pose, &mut scratch,
                &mut events,
            )
        };
    }

    frame!(); // Idle [0.0, 0.2)
    params.set_bool("walk", true);
    frame!(); // the transition fires; Idle advanced [0.2, 0.4)
    assert_eq!(plan.states[m.current_state()].name, "Walk");
    assert!(events.is_empty());
    frame!(); // Walk [0, 0.2), Idle [0.4, 0.6)
    assert_eq!(count(&events, "istep"), 1, "the outgoing state still fires mid-fade");
    let istep = events.iter().find(|e| e.name == "istep").unwrap();
    assert!((istep.weight - 0.6).abs() < 1e-4, "at its fading weight, got {}", istep.weight);
    frame!(); // Walk [0.2, 0.4), Idle [0.6, 0.8)
    assert_eq!(count(&events, "wstep"), 1, "the target fires on its own clock");
    let wstep = events.iter().find(|e| e.name == "wstep").unwrap();
    assert!((wstep.weight - 0.8).abs() < 1e-4, "at the fade-in weight, got {}", wstep.weight);

    // With an instant transition: the outgoing state's final sliver was never
    // rendered, so it fires nothing.
    let mut doc = two_state_doc();
    doc.node_mut(4)
        .unwrap()
        .properties
        .insert(plan::DURATION_PROP.into(), PropValue::Float(0.0));
    let plan = compile_anim_graph(&doc).expect("compiles");
    let clips = make_clips();
    let clip_for = resolver(&clips);
    let mut params = AnimParams::from_decls(&plan.parameters);
    let mut m = AnimMachine::new(&plan);
    let mut slot = PlayOnceSlot::new();
    macro_rules! frame {
        () => {
            tick_frame(
                &plan, &mut m, &mut slot, &mut params, &clip_for, 0.4, &mut pose, &mut scratch,
                &mut events,
            )
        };
    }

    frame!(); // Idle [0.0, 0.4)
    assert!(events.is_empty());
    params.set_bool("walk", true);
    // This tick advances Idle across its 0.45 marker *and* switches — the
    // sliver is invisible, so nothing fires.
    frame!();
    assert_eq!(plan.states[m.current_state()].name, "Walk");
    assert!(events.is_empty(), "no events from a state an instant switch discarded");
    // Walk's own clock starts at zero and fires its marker as it reaches it.
    frame!();
    assert_eq!(count(&events, "wstep"), 1);
}

#[test]
fn a_full_weight_overlay_silences_the_base_and_fires_its_own_markers_once() {
    // Idle loops 0.2s with a step marker each cycle; the attack overlay
    // (0.5s, no fades) carries a hit frame at 0.45.
    let mut doc = slot_doc();
    doc.node_mut(2)
        .unwrap()
        .properties
        .insert(plan::CLIP_PROP.into(), PropValue::Asset("anims/tap.anim".into()));
    let plan = compile_anim_graph(&doc).expect("compiles");
    let clips = [
        ("anims/tap.anim", marked_clip("Tap", 0.0, 0.2, &[(0.05, "istep")])),
        ("anims/walk.anim", constant_clip("Walk", 5.0)),
        ("anims/attack.anim", marked_clip("Attack", 10.0, 0.5, &[(0.45, "hit")])),
    ];
    let clip_for = resolver(&clips);
    let mut params = AnimParams::from_decls(&plan.parameters);
    let mut m = AnimMachine::new(&plan);
    let mut slot = PlayOnceSlot::new();
    let mut pose = vec![LocalBoneTransform::default(); 1];
    let mut scratch = PoseScratch::new();
    let mut events = Vec::new();
    macro_rules! frame {
        () => {
            tick_frame(
                &plan, &mut m, &mut slot, &mut params, &clip_for, 0.2, &mut pose, &mut scratch,
                &mut events,
            )
        };
    }

    // Base steps while nothing overlays.
    frame!();
    assert_eq!(count(&events, "istep"), 1);

    // Full-weight overlay: the base is invisible — and silent.
    params.fire_trigger("attack");
    let mut hits = 0;
    let mut base_while_overlaid = 0;
    for _ in 0..3 {
        frame!();
        hits += count(&events, "hit");
        base_while_overlaid += count(&events, "istep");
    }
    assert_eq!(base_while_overlaid, 0, "no footsteps from an invisible walk");
    assert_eq!(hits, 0, "the hit frame is later in the clip");

    // The tick that crosses the hit frame also passes the clip end: the hit
    // fires (at the envelope of its own moment), and the base is back.
    frame!();
    assert_eq!(count(&events, "hit"), 1, "the overlay's marker fires exactly once");
    assert_eq!(count(&events, "istep"), 1, "the base is audible again");
    assert_eq!(pose[0].translation.x, 0.0, "and visible again");

    // No refires from a finished one-shot.
    for _ in 0..3 {
        frame!();
        assert_eq!(count(&events, "hit"), 0);
    }
}

#[test]
fn the_system_surfaces_events_on_the_runtime() {
    // The idle clip carries a marker at 0.05; the harness ticks 0.1s frames.
    let assets = MapAssets::default();
    assets
        .graphs
        .lock()
        .unwrap()
        .insert(GRAPH.into(), two_state_doc());
    let mut h = Harness::new(assets);

    let e = h.world.spawn((
        AnimGraphRunner::new(GRAPH),
        SkeletonInstance::from_bones(synthetic_bones()),
    ));
    h.tick(); // arms and runs the first frame: span [0.0, 0.1) crosses 0.05
    {
        let rt = h.world.get::<&AnimGraphRuntime>(e).expect("armed");
        assert_eq!(
            rt.events,
            vec![AnimEventFire {
                name: "step".into(),
                weight: 1.0
            }],
            "the fire is readable by gameplay systems after this one"
        );
    }
    h.tick(); // span [0.1, 0.2): no crossing
    {
        let rt = h.world.get::<&AnimGraphRuntime>(e).unwrap();
        assert!(rt.events.is_empty(), "one frame's worth, never accumulated");
    }
}

