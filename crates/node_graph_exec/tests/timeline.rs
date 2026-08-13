//! Timeline semantics (Task 45-A P8, D7).
//!
//! The interesting claim is the **execution model**: a Timeline is a per-tick
//! stateful node, not a suspension user, built out of the loop frame and a
//! zero-length yield. So these tests count *ticks*, not firings — one Update
//! per tick between Play and Finished, exactly one Finished, and never two
//! advances in one tick.

#![allow(clippy::field_reassign_with_default)]

use std::collections::BTreeMap;

use curve_asset::{CurveDoc, Interp, Key, Track};
use node_graph_exec::nodes::{FLOAT_TO_STRING, PRINT};
use node_graph_exec::{
    compile_with_curves, tick, Effect, EntityRef, GraphInstance, NodeImpls, TickInput,
    TransformSnapshot, WorldRead,
};
use node_graph_types::{
    register_std_events, register_std_nodes, Edge, GraphDoc, NodeInst, NodeRegistry, PropValue,
    CURVE_PROP, EVENT_BEGIN_PLAY_TYPE_ID, EXEC_IN_PIN, EXEC_OUT_PIN, TIMELINE_FINISHED_PIN,
    TIMELINE_PLAY_PIN, TIMELINE_REVERSE_PIN, TIMELINE_STOP_PIN, TIMELINE_TYPE_ID,
    TIMELINE_UPDATE_PIN,
};

const CURVE_PATH: &str = "curves/demo.curve";

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// A world that has exactly one curve: `height` ramping 0 -> 10 over 1s.
///
/// It doubles as the compiler's `CurveResolver`, which is the point: the
/// descriptor the compiler validates against and the data the interpreter
/// samples come from the same place, so they cannot disagree.
struct CurveWorld {
    curves: BTreeMap<String, CurveDoc>,
}

impl CurveWorld {
    fn new() -> Self {
        let mut doc = CurveDoc::default();
        doc.tracks = vec![Track {
            slug: "height".into(),
            label: "Height".into(),
            keys: vec![
                Key { t: 0.0, value: 0.0, interp: Interp::Linear, tangent: Default::default() },
                Key { t: 1.0, value: 10.0, interp: Interp::Linear, tangent: Default::default() },
            ],
        }];
        let mut curves = BTreeMap::new();
        curves.insert(CURVE_PATH.to_string(), doc);
        Self { curves }
    }

    fn empty() -> Self {
        Self { curves: BTreeMap::new() }
    }
}

impl WorldRead for CurveWorld {
    fn transform(&self, _e: EntityRef) -> Option<TransformSnapshot> {
        None
    }
    fn exists(&self, _e: EntityRef) -> bool {
        false
    }
    fn curve(&self, path: &str) -> Option<&CurveDoc> {
        self.curves.get(path)
    }
}

impl node_graph_types::CurveResolver for CurveWorld {
    fn resolve_curve(&self, path: &str) -> Option<&CurveDoc> {
        self.curves.get(path)
    }
}

fn registry() -> NodeRegistry {
    let mut reg = NodeRegistry::new();
    register_std_nodes(&mut reg).unwrap();
    register_std_events(&mut reg).unwrap();
    reg
}

fn node(id: u64, type_id: &str) -> NodeInst {
    NodeInst {
        id,
        type_id: type_id.to_string(),
        type_version: 1,
        position: [id as f32 * 200.0, 0.0],
        properties: BTreeMap::new(),
        subgraph: None,
        tint: None,
        title: None,
    }
}

fn edge(from: u64, fp: &str, to: u64, tp: &str) -> Edge {
    Edge {
        from_node: from,
        from_pin: fp.to_string(),
        to_node: to,
        to_pin: tp.to_string(),
    }
}

/// BeginPlay -> Timeline(`enter_pin`); Update -> Print(ToString(height));
/// Finished -> Print("done"). Node 1 is the Timeline.
fn doc(enter_pin: &str, looping: bool, duration: f32) -> GraphDoc {
    let mut d = GraphDoc::default();
    let mut tl = node(1, TIMELINE_TYPE_ID);
    tl.properties
        .insert(CURVE_PROP.into(), PropValue::Asset(CURVE_PATH.into()));
    tl.properties.insert("looping".into(), PropValue::Bool(looping));
    tl.properties.insert("duration".into(), PropValue::Float(duration));
    let mut done = node(3, PRINT);
    done.properties
        .insert("text".into(), PropValue::Str("done".into()));
    d.nodes = vec![
        node(0, EVENT_BEGIN_PLAY_TYPE_ID),
        tl,
        node(2, PRINT),
        done,
        node(4, FLOAT_TO_STRING),
    ];
    d.edges = vec![
        edge(0, EXEC_OUT_PIN, 1, enter_pin),
        edge(1, TIMELINE_UPDATE_PIN, 2, EXEC_IN_PIN),
        edge(1, "height", 4, "value"),
        edge(4, "text", 2, "text"),
        edge(1, TIMELINE_FINISHED_PIN, 3, EXEC_IN_PIN),
    ];
    d
}

fn impls() -> NodeImpls {
    let mut i = NodeImpls::new();
    node_graph_exec::nodes::register_impls(&mut i);
    i
}

/// Run `ticks` ticks of `dt` and return the printed lines — one per Update,
/// plus "done" when Finished fires.
fn run(doc: &GraphDoc, ticks: usize, dt: f32, world: &CurveWorld) -> Vec<String> {
    run_with(doc, ticks, dt, world, |_, _| {}).0
}

/// …with a hook that can queue events (Stop, Reverse) at a given tick.
fn run_with(
    doc: &GraphDoc,
    ticks: usize,
    dt: f32,
    world: &CurveWorld,
    at_tick: impl FnMut(usize, &mut GraphInstance),
) -> (Vec<String>, GraphInstance) {
    run_split(doc, ticks, dt, &CurveWorld::new(), world, at_tick)
}

/// …and with the *compiler's* curves separate from the *runtime's* world, so
/// a test can compile against an asset the runtime then cannot reach.
fn run_split(
    doc: &GraphDoc,
    ticks: usize,
    dt: f32,
    compile_curves: &CurveWorld,
    world: &CurveWorld,
    mut at_tick: impl FnMut(usize, &mut GraphInstance),
) -> (Vec<String>, GraphInstance) {
    let reg = registry();
    let plan = compile_with_curves(doc, "test.graph", &reg, &BTreeMap::new(), compile_curves)
        .unwrap_or_else(|e| panic!("compile: {e}"));
    let impls = impls();
    assert_eq!(impls.check_plan(&plan), vec![], "plan/impl cross-check");
    let mut inst = GraphInstance::new(&plan, EntityRef::SelfEntity, 1);
    let mut effects: Vec<Effect> = Vec::new();
    for i in 0..ticks {
        at_tick(i, &mut inst);
        let t = TickInput { dt, time: i as f64 * dt as f64 };
        tick(&plan, &mut inst, &impls, t, world, &mut effects);
    }
    let logs = effects
        .iter()
        .filter_map(|e| match e {
            Effect::Log { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect();
    (logs, inst)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// The core claim: **one Update per tick**, sampling the curve as time
/// advances, then exactly one Finished. Never two advances in a tick — that
/// is what the yield buys, and a Timeline that advanced twice would run at
/// double speed on the first frame and be impossible to notice later.
#[test]
fn a_timeline_updates_once_per_tick_and_finishes_once() {
    // dt 0.25 over a 1s curve: t = 0, .25, .5, .75, 1.0(end).
    let logs = run(&doc(TIMELINE_PLAY_PIN, false, 0.0), 8, 0.25, &CurveWorld::new());
    assert_eq!(
        logs,
        vec!["0", "2.5", "5", "7.5", "10", "done"],
        "one sample per tick, the last one clamped to the end, then Finished"
    );
    // …and it stays finished: the extra ticks above produced nothing more.
    assert_eq!(logs.iter().filter(|l| *l == "done").count(), 1);
}

/// Looping never finishes, and it carries the overshoot instead of snapping —
/// a Timeline that reset to exactly 0 each cycle would drift by up to a frame
/// every lap.
#[test]
fn looping_wraps_and_never_finishes() {
    let logs = run(&doc(TIMELINE_PLAY_PIN, true, 0.0), 10, 0.4, &CurveWorld::new());
    assert!(!logs.contains(&"done".to_string()), "a loop has no end: {logs:?}");
    // t = 0, .4, .8, then 1.2 wraps to .2 — carrying the overshoot, not
    // snapping to 0 — then .6, then 1.0 wraps to ~0 and the cycle repeats.
    let got: Vec<f32> = logs.iter().map(|s| s.parse().unwrap()).collect();
    let want = [0.0, 4.0, 8.0, 2.0, 6.0, 0.0, 4.0, 8.0, 2.0, 6.0];
    assert_eq!(got.len(), want.len(), "{logs:?}");
    for (g, w) in got.iter().zip(want) {
        assert!((g - w).abs() < 1e-3, "{got:?} != {want:?}");
    }
}

/// Stop ends the run **without** claiming it finished: Finished means
/// "reached the end", and being stopped is not that.
#[test]
fn stop_ends_the_run_without_firing_finished() {
    // A second entry node fires Stop on a custom event queued at tick 3.
    let mut d = doc(TIMELINE_PLAY_PIN, false, 0.0);
    d.nodes.push(node(5, node_graph_types::EVENT_CUSTOM_TYPE_ID));
    d.nodes[5]
        .properties
        .insert(node_graph_types::EVENT_NAME_PROP.into(), PropValue::Str("halt".into()));
    d.edges.push(edge(5, EXEC_OUT_PIN, 1, TIMELINE_STOP_PIN));

    let (logs, _) = run_with(&d, 8, 0.25, &CurveWorld::new(), |i, inst| {
        if i == 3 {
            inst.queue_event(
                node_graph_types::EventPhase::Custom,
                Some("halt".into()),
                BTreeMap::new(),
            );
        }
    });
    assert!(!logs.contains(&"done".to_string()), "Stop is not Finished: {logs:?}");
    // Queued at tick 3, delivered at tick 4 (no reentrancy, D3) — and the
    // playing activation started first, so it takes tick 4's Update before the
    // Stop activation runs. Stopping is therefore visible from tick 5 on.
    assert_eq!(logs, vec!["0", "2.5", "5", "7.5"], "and it stops advancing: {logs:?}");
}

/// Reverse from a standing start plays the curve backwards from its end.
#[test]
fn reverse_from_stopped_plays_backwards_from_the_end() {
    let logs = run(&doc(TIMELINE_REVERSE_PIN, false, 0.0), 8, 0.25, &CurveWorld::new());
    assert_eq!(logs, vec!["10", "7.5", "5", "2.5", "0", "done"], "{logs:?}");
}

/// The Duration input overrides the asset's own length — the curve is still
/// sampled in its own time base, so a 2s override stretches a 1s curve.
#[test]
fn a_duration_override_stretches_the_run() {
    let logs = run(&doc(TIMELINE_PLAY_PIN, false, 2.0), 12, 0.5, &CurveWorld::new());
    // t = 0, .5, 1.0, 1.5, then 2.0 = the end. Past 1s the curve clamps at 10.
    assert_eq!(logs, vec!["0", "5", "10", "10", "10", "done"], "{logs:?}");
}

/// A curve that does not resolve is a **validation** error: the node's output
/// pins are unknowable, so every wire leaving it dangles.
#[test]
fn a_curve_that_does_not_resolve_fails_validation() {
    let reg = registry();
    let err = compile_with_curves(
        &doc(TIMELINE_PLAY_PIN, false, 0.0),
        "test.graph",
        &reg,
        &BTreeMap::new(),
        &CurveWorld::empty(),
    )
    .expect_err("a Timeline with no reachable curve must not compile");
    let text = err.to_string();
    assert!(text.contains(CURVE_PATH), "the error names the asset: {text}");
}

/// …and if the asset goes missing *after* compilation — deleted, or simply
/// never loaded at run time — the activation stops and names it, rather than
/// playing silence for the rest of the session.
#[test]
fn a_missing_curve_stops_the_activation_and_says_so() {
    let (logs, inst) = run_split(
        &doc(TIMELINE_PLAY_PIN, false, 0.0),
        3,
        0.25,
        &CurveWorld::new(),
        &CurveWorld::empty(),
        |_, _| {},
    );
    assert!(logs.is_empty(), "nothing ran: {logs:?}");
    let failed: Vec<String> = inst
        .threads
        .iter()
        .filter_map(|t| match &t.state {
            node_graph_exec::ThreadState::Failed(e) => Some(e.to_string()),
            _ => None,
        })
        .collect();
    assert_eq!(failed.len(), 1, "{failed:?}");
    assert!(failed[0].contains(CURVE_PATH), "the error names the asset: {failed:?}");
    // The *instance* is not halted — one broken Timeline does not take the
    // whole graph down (only the budget does that).
    assert!(inst.halted.is_none());
}

/// A run in progress is plain serializable data, like every other latent
/// state (D1): stop the process mid-timeline, resume, and it continues from
/// where it was rather than restarting.
#[test]
fn a_running_timeline_survives_a_round_trip_mid_play() {
    let reg = registry();
    let d = doc(TIMELINE_PLAY_PIN, false, 0.0);
    let world = CurveWorld::new();
    let plan = compile_with_curves(&d, "test.graph", &reg, &BTreeMap::new(), &world)
        .expect("compile");
    let impls = impls();
    let mut inst = GraphInstance::new(&plan, EntityRef::SelfEntity, 1);
    let mut effects: Vec<Effect> = Vec::new();
    for i in 0..3 {
        let t = TickInput { dt: 0.25, time: i as f64 * 0.25 };
        tick(&plan, &mut inst, &impls, t, &world, &mut effects);
    }

    let text = ron::to_string(&inst).expect("serialize mid-run");
    let mut back: GraphInstance = ron::from_str(&text).expect("deserialize");
    assert_eq!(back, inst);

    for i in 3..8 {
        let t = TickInput { dt: 0.25, time: i as f64 * 0.25 };
        tick(&plan, &mut back, &impls, t, &world, &mut effects);
    }
    let logs: Vec<String> = effects
        .iter()
        .filter_map(|e| match e {
            Effect::Log { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        logs,
        vec!["0", "2.5", "5", "7.5", "10", "done"],
        "resuming continued the run rather than restarting it: {logs:?}"
    );
}
