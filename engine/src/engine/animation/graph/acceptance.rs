//! Acceptance evidence for the animation-graph tracer (Task 41, ticket 01).
//!
//! Tests sit at the seams the spec pre-agreed: the document (round-trip
//! through the shared container io), the machine (document + parameter
//! writes + ticks in → active state and blend weights out), pose values on a
//! synthetic skeleton (CPU only, no GPU, no asset files), and the system
//! (arming, invalidation, coexistence with the single-clip player).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use glam::{Mat4, Vec3};
use node_graph_types::{
    parse_graph, serialize_graph, Edge, GraphDoc, GraphRealm, NodeInst, PinType, PropValue, VarDecl,
};

use crate::engine::animation::components::{LocalBoneTransform, SkeletonInstance};
use crate::engine::animation::{AnimationPlayer, AnimationUpdateSystem};
use crate::engine::assets::model_loader::{AnimationChannel, BoneData, RawAnimationClip};
use crate::engine::ecs::components::MeshRenderer;
use crate::engine::ecs::resources::{Resources, Time};
use crate::engine::ecs::schedule::System;

use super::machine::{evaluate_pose, AnimMachine, AnimParams};
use super::plan::{self, compile_anim_graph, TransitionCondition};
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

/// The tracer machine: ENTRY → Idle, one transition Idle → Walk while the
/// Bool parameter `walk` is true, 0.5s crossfade.
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
                (plan::WHEN_BOOL_PROP, PropValue::Str("walk".into())),
            ],
        ),
    ];
    doc.edges = vec![
        edge(1, plan::STATE_OUT_PIN, 2, plan::STATE_IN_PIN),
        edge(2, plan::STATE_OUT_PIN, 4, plan::TRANSITION_FROM_PIN),
        edge(4, plan::TRANSITION_TO_PIN, 3, plan::STATE_IN_PIN),
    ];
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
    }
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
    assert_eq!(
        plan.transitions[0].condition,
        TransitionCondition::BoolParam("walk".into())
    );
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
    assert_eq!(t.condition, TransitionCondition::BoolParam("walk".into()));
    assert_eq!(plan.states[t.from].name, "Idle");
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

    // A condition naming an undeclared parameter.
    let mut doc = two_state_doc();
    doc.variables.clear();
    assert!(compile_anim_graph(&doc).unwrap_err().contains("not declared"));

    // A parameter type outside this slice.
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
    let params = AnimParams::from_decls(&plan.parameters);
    let mut m = AnimMachine::new(&plan);
    m.tick(&plan, &params, 0.1);
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
        m.tick(&plan, &params, 0.1);
    }
    assert_eq!(plan.states[m.current_state()].name, "Idle");

    // Gameplay writes the parameter — never the state.
    assert!(params.set_bool("walk", true));
    m.tick(&plan, &params, 0.1);
    assert_eq!(plan.states[m.current_state()].name, "Walk");
    assert_eq!(m.blend_weight(), 0.0, "the target blends in from zero");

    // Weight climbs linearly over the 0.5s duration: 0.1s per tick.
    let mut weights = Vec::new();
    for _ in 0..4 {
        m.tick(&plan, &params, 0.1);
        weights.push(m.blend_weight());
    }
    for (i, w) in weights.iter().enumerate() {
        let expected = 0.2 * (i + 1) as f32;
        assert!((w - expected).abs() < 1e-4, "weight {w} at step {i}, expected {expected}");
    }

    // One more tick reaches the duration; the fade retires and the machine
    // is fully in Walk.
    m.tick(&plan, &params, 0.1);
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
        &[
            (plan::DURATION_PROP, PropValue::Float(0.5)),
            (plan::WHEN_BOOL_PROP, PropValue::Str("walk".into())),
        ],
    ));
    doc.edges.push(edge(3, plan::STATE_OUT_PIN, 5, plan::TRANSITION_FROM_PIN));
    doc.edges.push(edge(5, plan::TRANSITION_TO_PIN, 2, plan::STATE_IN_PIN));
    let plan = compile_anim_graph(&doc).expect("compiles");
    let mut params = AnimParams::from_decls(&plan.parameters);
    let mut m = AnimMachine::new(&plan);

    params.set_bool("walk", true);
    m.tick(&plan, &params, 0.1); // Idle → Walk fires, fade starts
    let from = m.crossfade().expect("fading").from;
    m.tick(&plan, &params, 0.1);
    m.tick(&plan, &params, 0.1);
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
    doc.node_mut(4).unwrap().properties.remove(plan::WHEN_BOOL_PROP);
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
    let params = AnimParams::from_decls(&plan.parameters);
    let mut m = AnimMachine::new(&plan);
    m.tick(&plan, &params, 0.1);
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
// Pose values on a synthetic skeleton (CPU only)
// ---------------------------------------------------------------------------

#[test]
fn crossfade_blends_pose_values_on_a_synthetic_skeleton() {
    let plan = compile_anim_graph(&two_state_doc()).expect("compiles");
    let idle = constant_clip("Idle", 0.0);
    let walk = constant_clip("Walk", 10.0);
    let clip_for = |state: usize| -> Option<&RawAnimationClip> {
        match plan.states[state].name.as_str() {
            "Idle" => Some(&idle),
            _ => Some(&walk),
        }
    };

    let mut params = AnimParams::from_decls(&plan.parameters);
    let mut m = AnimMachine::new(&plan);
    let mut pose = vec![LocalBoneTransform::default(); 2];
    let mut scratch = Vec::new();

    m.tick(&plan, &params, 0.1);
    evaluate_pose(&m, clip_for, &mut pose, &mut scratch);
    assert_eq!(pose[0].translation.x, 0.0, "entry state pose");

    params.set_bool("walk", true);
    m.tick(&plan, &params, 0.1); // fade starts, elapsed 0.0
    m.tick(&plan, &params, 0.1);
    m.tick(&plan, &params, 0.1);
    m.tick(&plan, &params, 0.05); // elapsed 0.25 of 0.5 → weight 0.5
    evaluate_pose(&m, clip_for, &mut pose, &mut scratch);
    assert!(
        (pose[0].translation.x - 5.0).abs() < 1e-4,
        "halfway through the fade the pose is halfway between the clips, got {}",
        pose[0].translation.x
    );
    // The unanimated bone is untouched by either clip.
    assert_eq!(pose[1].translation, Vec3::ZERO);

    // Fade done → pure Walk pose.
    m.tick(&plan, &params, 0.3);
    evaluate_pose(&m, clip_for, &mut pose, &mut scratch);
    assert!((pose[0].translation.x - 10.0).abs() < 1e-4);
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
        let name = match content_rel {
            "anims/idle.anim" => "Idle",
            "anims/walk.anim" => "Walk",
            _ => return None,
        };
        Some(ClipSet {
            bone_names: vec!["root".into(), "child".into()],
            clips: vec![constant_clip(name, if name == "Idle" { 0.0 } else { 10.0 })],
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
