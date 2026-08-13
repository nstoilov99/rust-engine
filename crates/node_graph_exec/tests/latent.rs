//! Latent machinery (Task 45-A P4, D1's suspend half).
//!
//! The plan names loop-and-latent interaction as *the* risk of this package,
//! so the five guarantees it calls out each get a test here rather than a
//! paragraph:
//!
//! 1. suspension across ticks resumes exactly once, and not early;
//! 2. a Delay inside a ForLoop body keeps the loop frame;
//! 3. concurrent activations of one Delay wait independently;
//! 4. an instance serializes **mid-wait** and resumes after a round trip;
//! 5. a suspended activation coexists with new events, and the drain order
//!    holds — due latents before input actions, custom events and Tick.
//!
//! Plus the two rulings: a resume costs budget like any other firing, and
//! `Delay(0)` yields to the next tick rather than resuming reentrantly.

// Tests build documents the way an author does: start from the default and
// fill in what matters. A single giant struct literal would technically
// satisfy clippy and be markedly harder to read.
#![allow(clippy::field_reassign_with_default)]

use std::collections::BTreeMap;

use node_graph_exec::nodes as n;
use node_graph_exec::{
    compile, tick, tick_with_budget, Effect, EntityRef, ExecError, GraphInstance, NoWorld,
    NodeImpls, Plan, ThreadState, TickInput, TickReport,
};
use node_graph_types::{
    register_std_events, register_std_nodes, Edge, EventPhase, GraphDoc, NodeInst, NodeRegistry,
    PinType, PropValue, VarDecl, EVENT_BEGIN_PLAY_TYPE_ID, EVENT_CUSTOM_TYPE_ID, EVENT_NAME_PROP,
    EVENT_TICK_TYPE_ID, EXEC_IN_PIN, EXEC_OUT_PIN, VAR_GET_TYPE_ID, VAR_PROP, VAR_SET_TYPE_ID,
    VAR_VALUE_PIN,
};

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

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
        position: [id as f32 * 180.0, 0.0],
        properties: BTreeMap::new(),
        subgraph: None,
        tint: None,
        title: None,
    }
}

fn with(id: u64, type_id: &str, props: &[(&str, PropValue)]) -> NodeInst {
    let mut n = node(id, type_id);
    for (k, v) in props {
        n.properties.insert(k.to_string(), v.clone());
    }
    n
}

fn print(id: u64, text: &str) -> NodeInst {
    with(id, n::PRINT, &[("text", PropValue::Str(text.to_string()))])
}

fn delay(id: u64, seconds: f32) -> NodeInst {
    with(id, n::DELAY, &[("duration", PropValue::Float(seconds))])
}

fn edge(from: u64, fp: &str, to: u64, tp: &str) -> Edge {
    Edge {
        from_node: from,
        from_pin: fp.to_string(),
        to_node: to,
        to_pin: tp.to_string(),
    }
}

fn build(doc: &GraphDoc) -> Plan {
    compile(doc, "test.graph", &registry(), &BTreeMap::new())
        .unwrap_or_else(|e| panic!("compile: {e}"))
}

fn logs(effects: &[Effect]) -> Vec<String> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::Log { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

/// A runner in miniature: it owns the clock, and the clock is the only thing
/// the interpreter cannot get for itself.
struct Runner {
    plan: Plan,
    impls: NodeImpls,
    instance: GraphInstance,
    time: f64,
    dt: f32,
}

impl Runner {
    fn new(doc: &GraphDoc, dt: f32) -> Self {
        let plan = build(doc);
        let impls = n::std_impls();
        assert_eq!(impls.check_plan(&plan), vec![], "plan/impl cross-check");
        let instance = GraphInstance::new(&plan, EntityRef::SelfEntity, 1);
        Self { plan, impls, instance, time: 0.0, dt }
    }

    /// One tick; returns what it printed.
    fn tick(&mut self) -> Vec<String> {
        let mut fx = Vec::new();
        self.time += self.dt as f64;
        tick(
            &self.plan,
            &mut self.instance,
            &self.impls,
            TickInput { dt: self.dt, time: self.time },
            &NoWorld,
            &mut fx,
        );
        logs(&fx)
    }

    fn tick_report(&mut self) -> TickReport {
        let mut fx = Vec::new();
        self.time += self.dt as f64;
        tick(
            &self.plan,
            &mut self.instance,
            &self.impls,
            TickInput { dt: self.dt, time: self.time },
            &NoWorld,
            &mut fx,
        )
    }

    /// Ticks, collecting each tick's output separately so a test can say
    /// *when* something happened, not just that it did.
    fn ticks(&mut self, n: usize) -> Vec<Vec<String>> {
        (0..n).map(|_| self.tick()).collect()
    }

    /// Tick until a tick produces output, or `max` ticks pass. For the
    /// tests where *what* happens is the assertion and the exact tick index
    /// is arithmetic.
    fn tick_until_output(&mut self, max: usize) -> Vec<String> {
        for _ in 0..max {
            let out = self.tick();
            if !out.is_empty() {
                return out;
            }
        }
        Vec::new()
    }

    fn suspended(&self) -> usize {
        self.instance
            .threads
            .iter()
            .filter(|t| matches!(t.state, ThreadState::Suspended(_)))
            .count()
    }
}

// ---------------------------------------------------------------------------
// 1. Suspension across ticks
// ---------------------------------------------------------------------------

/// A Delay(0.5) under a dt of 0.2 waits three ticks and resumes exactly once
/// — never early. The arithmetic is spelled out at the assertions.
#[test]
fn delay_suspends_across_ticks_and_resumes_once() {
    let mut doc = GraphDoc::default();
    doc.nodes = vec![
        node(0, EVENT_BEGIN_PLAY_TYPE_ID),
        print(1, "before"),
        delay(2, 0.5),
        print(3, "after"),
    ];
    doc.edges = vec![
        edge(0, EXEC_OUT_PIN, 1, EXEC_IN_PIN),
        edge(1, EXEC_OUT_PIN, 2, EXEC_IN_PIN),
        edge(2, EXEC_OUT_PIN, 3, EXEC_IN_PIN),
    ];

    // Instance time advances before the tick body, so the tick that fires
    // the Delay is already at t=0.2 and the wait is due at 0.7. It therefore
    // resumes on the first tick whose time reaches 0.7 — t=0.8, three ticks
    // of waiting, 0.6s elapsed for a 0.5s request. Never early, and the
    // overshoot is under one frame.
    let mut r = Runner::new(&doc, 0.2);
    assert_eq!(r.tick(), vec!["before"], "t=0.2: runs up to the Delay and parks");
    assert_eq!(r.suspended(), 1, "the activation is parked, not finished");
    assert_eq!(r.tick(), Vec::<String>::new(), "t=0.4: still waiting");
    assert_eq!(r.tick(), Vec::<String>::new(), "t=0.6: 0.4s elapsed — still early");
    assert_eq!(r.suspended(), 1);
    assert_eq!(r.tick(), vec!["after"], "t=0.8: due, and it resumes");
    assert_eq!(r.suspended(), 0);

    // Exactly once: nothing more, ever.
    assert_eq!(r.ticks(5).concat(), Vec::<String>::new(), "no second resume");
    assert!(
        r.instance
            .threads
            .iter()
            .all(|t| matches!(t.state, ThreadState::Finished)),
        "the activation finished rather than lingering"
    );
}

/// `Delay(0)` is a **yield**: it resumes on the next tick, never inside the
/// tick that suspended it. Resuming in-tick would be reentrancy, which D3
/// forbids for events and which would be no less surprising here.
#[test]
fn zero_delay_yields_to_the_next_tick() {
    let mut doc = GraphDoc::default();
    doc.nodes = vec![
        node(0, EVENT_BEGIN_PLAY_TYPE_ID),
        print(1, "before"),
        delay(2, 0.0),
        print(3, "after"),
    ];
    doc.edges = vec![
        edge(0, EXEC_OUT_PIN, 1, EXEC_IN_PIN),
        edge(1, EXEC_OUT_PIN, 2, EXEC_IN_PIN),
        edge(2, EXEC_OUT_PIN, 3, EXEC_IN_PIN),
    ];

    let mut r = Runner::new(&doc, 0.1);
    assert_eq!(r.tick(), vec!["before"], "not the same tick");
    assert_eq!(r.tick(), vec!["after"], "the very next one");

    // A negative duration is the same thing, and a NaN one is too — a wait
    // that never comes due is a hang wearing a costume.
    for bad in [-5.0f32, f32::NAN, f32::INFINITY] {
        let mut d = doc.clone();
        d.nodes[2]
            .properties
            .insert("duration".into(), PropValue::Float(bad));
        let mut r = Runner::new(&d, 0.1);
        assert_eq!(r.tick(), vec!["before"], "{bad}");
        assert_eq!(r.tick(), vec!["after"], "{bad} must not hang");
    }
}

/// A Delay whose output is wired to nothing still waits, then finishes — the
/// resume path unwinds like any other exhausted statement.
#[test]
fn a_delay_with_no_continuation_still_waits_then_finishes() {
    let mut doc = GraphDoc::default();
    doc.nodes = vec![node(0, EVENT_BEGIN_PLAY_TYPE_ID), print(1, "before"), delay(2, 0.3)];
    doc.edges = vec![
        edge(0, EXEC_OUT_PIN, 1, EXEC_IN_PIN),
        edge(1, EXEC_OUT_PIN, 2, EXEC_IN_PIN),
    ];
    let mut r = Runner::new(&doc, 0.2);
    assert_eq!(r.tick(), vec!["before"]);
    assert_eq!(r.suspended(), 1);
    r.tick(); // t=0.4, due at 0.5 — still waiting
    assert_eq!(r.suspended(), 1);
    r.tick(); // t=0.6
    assert_eq!(r.suspended(), 0);
    assert!(r
        .instance
        .threads
        .iter()
        .all(|t| matches!(t.state, ThreadState::Finished)));
}

// ---------------------------------------------------------------------------
// 2. Delay inside a loop body — the named risk
// ---------------------------------------------------------------------------

/// **The loop frame survives suspension.** Iteration N delays, resumes,
/// finishes its body, and iteration N+1 starts — across many ticks, with the
/// index still correct on the far side of every wait.
#[test]
fn delay_inside_a_for_loop_keeps_the_frame() {
    let mut doc = GraphDoc::default();
    doc.nodes = vec![
        node(0, EVENT_BEGIN_PLAY_TYPE_ID),
        with(1, n::FOR_LOOP, &[("first", PropValue::Int(1)), ("last", PropValue::Int(3))]),
        print(2, ""),        // "enter N", before the wait
        delay(3, 0.25),
        print(4, ""),        // "leave N", after the wait
        print(5, "done"),
        node(6, n::INT_TO_STRING),
        node(7, n::INT_TO_STRING),
    ];
    doc.edges = vec![
        edge(0, EXEC_OUT_PIN, 1, EXEC_IN_PIN),
        edge(1, "body", 2, EXEC_IN_PIN),
        edge(2, EXEC_OUT_PIN, 3, EXEC_IN_PIN),
        edge(3, EXEC_OUT_PIN, 4, EXEC_IN_PIN),
        edge(1, "completed", 5, EXEC_IN_PIN),
        // The loop index feeds both prints, so a lost frame shows up as a
        // wrong number rather than merely a missing line.
        edge(1, "index", 6, "value"),
        edge(6, "text", 2, "text"),
        edge(1, "index", 7, "value"),
        edge(7, "text", 4, "text"),
    ];

    let mut r = Runner::new(&doc, 0.1);
    let per_tick = r.ticks(12);
    let flat: Vec<String> = per_tick.iter().flatten().cloned().collect();
    assert_eq!(
        flat,
        vec!["1", "1", "2", "2", "3", "3", "done"],
        "each iteration enters, waits, leaves with its own index, then the \
         next begins: {per_tick:?}"
    );

    // Three waits of 0.25s at dt 0.1 => three resumes, and the loop ran its
    // body exactly three times.
    // Each resume finishes one iteration *and* starts the next in the same
    // tick, so the twelve ticks carry work on four of them.
    assert_eq!(
        per_tick.iter().filter(|t| !t.is_empty()).count(),
        4,
        "{per_tick:?}"
    );
    assert_eq!(r.suspended(), 0);

    // Nested: a Delay in the *inner* loop of two must not disturb the outer
    // frame either.
    let mut doc = GraphDoc::default();
    doc.nodes = vec![
        node(0, EVENT_BEGIN_PLAY_TYPE_ID),
        with(1, n::FOR_LOOP, &[("first", PropValue::Int(1)), ("last", PropValue::Int(2))]),
        with(2, n::FOR_LOOP, &[("first", PropValue::Int(1)), ("last", PropValue::Int(2))]),
        delay(3, 0.15),
        print(4, ""),
        print(5, "end"),
        node(6, n::INT_TO_STRING),
        node(7, n::ADD_INT),
        node(8, n::MUL_INT),
    ];
    doc.edges = vec![
        edge(0, EXEC_OUT_PIN, 1, EXEC_IN_PIN),
        edge(1, "body", 2, EXEC_IN_PIN),
        edge(2, "body", 3, EXEC_IN_PIN),
        edge(3, EXEC_OUT_PIN, 4, EXEC_IN_PIN),
        edge(1, "completed", 5, EXEC_IN_PIN),
        // Print outer*10 + inner, so a corrupted frame is unmistakable.
        edge(1, "index", 8, "a"),
        edge(8, "result", 7, "a"),
        edge(2, "index", 7, "b"),
        edge(7, "result", 6, "value"),
        edge(6, "text", 4, "text"),
    ];
    doc.nodes[8]
        .properties
        .insert("b".into(), PropValue::Int(10));

    let mut r = Runner::new(&doc, 0.1);
    let flat: Vec<String> = r.ticks(16).into_iter().flatten().collect();
    assert_eq!(
        flat,
        vec!["11", "12", "21", "22", "end"],
        "both frames survive every suspension, in order"
    );
}

// ---------------------------------------------------------------------------
// 3. Concurrent activations
// ---------------------------------------------------------------------------

/// Two activations of the same graph hit the same Delay node in the same
/// tick. They park with **distinct activation ids** and resume independently
/// — the suspension lives on the thread, not on the node.
#[test]
fn concurrent_activations_of_one_delay_wait_independently() {
    let mut doc = GraphDoc::default();
    doc.nodes = vec![
        with(0, EVENT_CUSTOM_TYPE_ID, &[(EVENT_NAME_PROP, PropValue::Str("a".into()))]),
        with(1, EVENT_CUSTOM_TYPE_ID, &[(EVENT_NAME_PROP, PropValue::Str("b".into()))]),
        delay(2, 0.25),
        print(3, "A"),
        delay(4, 0.45),
        print(5, "B"),
    ];
    doc.edges = vec![
        edge(0, EXEC_OUT_PIN, 2, EXEC_IN_PIN),
        edge(2, EXEC_OUT_PIN, 3, EXEC_IN_PIN),
        edge(1, EXEC_OUT_PIN, 4, EXEC_IN_PIN),
        edge(4, EXEC_OUT_PIN, 5, EXEC_IN_PIN),
    ];

    let mut r = Runner::new(&doc, 0.1);
    r.instance
        .queue_event(EventPhase::Custom, Some("a".into()), BTreeMap::new());
    r.instance
        .queue_event(EventPhase::Custom, Some("b".into()), BTreeMap::new());

    assert_eq!(r.tick(), Vec::<String>::new(), "both park");
    assert_eq!(r.suspended(), 2);
    let ids: Vec<u64> = r.instance.threads.iter().map(|t| t.id.0).collect();
    assert_eq!(ids, vec![0, 1], "distinct activation ids");

    // Both fired their Delay at t=0.1, so A is due at 0.35 and B at 0.55:
    // A resumes at t=0.4, B at t=0.6.
    let per_tick = r.ticks(6);
    assert_eq!(
        per_tick,
        vec![
            Vec::<String>::new(),  // 0.2
            Vec::<String>::new(),  // 0.3
            vec!["A".to_string()], // 0.4
            Vec::<String>::new(),  // 0.5
            vec!["B".to_string()], // 0.6
            Vec::<String>::new(),  // 0.7
        ],
        "each resumes on its own schedule: {per_tick:?}"
    );

    // Now the harder shape: two activations through the *same* Delay node,
    // with per-activation locals that must not bleed.
    let mut doc = GraphDoc::default();
    doc.nodes = vec![
        with(0, EVENT_CUSTOM_TYPE_ID, &[(EVENT_NAME_PROP, PropValue::Str("go".into()))]),
        delay(1, 0.25),
        print(2, ""),
        node(3, n::INT_TO_STRING),
    ];
    doc.edges = vec![
        edge(0, EXEC_OUT_PIN, 1, EXEC_IN_PIN),
        edge(1, EXEC_OUT_PIN, 2, EXEC_IN_PIN),
        // The event's payload rides the activation across the suspension.
        edge(0, "payload", 3, "value"),
        edge(3, "text", 2, "text"),
    ];
    doc.nodes[0].properties.insert(
        format!("{}payload", node_graph_types::EVENT_PAYLOAD_PREFIX),
        PropValue::Enum("int".into()),
    );

    let mut r = Runner::new(&doc, 0.1);
    for v in [7i32, 9] {
        let mut payload = BTreeMap::new();
        payload.insert("payload".to_string(), node_graph_exec::Value::Int(v));
        r.instance
            .queue_event(EventPhase::Custom, Some("go".into()), payload);
    }
    r.tick();
    assert_eq!(r.suspended(), 2, "both activations of one Delay node");
    let flat: Vec<String> = r.ticks(5).into_iter().flatten().collect();
    assert_eq!(
        flat,
        vec!["7", "9"],
        "each activation carried its own payload across its own wait"
    );
}

/// Per-node state (Gate, DoOnce, FlipFlop) is **per instance**, not per
/// activation — Blueprint's semantics, and the only reading under which
/// "DoOnce" means once. Concurrent activations therefore *share* it on
/// purpose, while their frames and locals stay private.
#[test]
fn node_state_is_shared_but_frames_are_not() {
    let mut doc = GraphDoc::default();
    doc.nodes = vec![
        with(0, EVENT_CUSTOM_TYPE_ID, &[(EVENT_NAME_PROP, PropValue::Str("go".into()))]),
        delay(1, 0.15),
        node(2, n::DO_ONCE),
        print(3, "passed"),
    ];
    doc.edges = vec![
        edge(0, EXEC_OUT_PIN, 1, EXEC_IN_PIN),
        edge(1, EXEC_OUT_PIN, 2, "enter"),
        edge(2, "completed", 3, EXEC_IN_PIN),
    ];

    let mut r = Runner::new(&doc, 0.1);
    for _ in 0..3 {
        r.instance
            .queue_event(EventPhase::Custom, Some("go".into()), BTreeMap::new());
    }
    r.tick();
    assert_eq!(r.suspended(), 3, "three independent waits");
    let flat: Vec<String> = r.ticks(5).into_iter().flatten().collect();
    assert_eq!(
        flat,
        vec!["passed"],
        "three activations, one DoOnce: the state is the node's, not the caller's"
    );
    // All three threads finished — sharing the state did not strand any of
    // them mid-flight.
    assert!(r
        .instance
        .threads
        .iter()
        .all(|t| matches!(t.state, ThreadState::Finished)));
}

/// One activation is one continuation, so **two simultaneous latents in one
/// thread are impossible by construction**: a second Delay cannot be reached
/// until the first has resumed and its statement has moved on.
#[test]
fn an_activation_holds_at_most_one_suspension() {
    let mut doc = GraphDoc::default();
    doc.nodes = vec![
        node(0, EVENT_BEGIN_PLAY_TYPE_ID),
        delay(1, 0.15),
        print(2, "one"),
        delay(3, 0.15),
        print(4, "two"),
    ];
    doc.edges = vec![
        edge(0, EXEC_OUT_PIN, 1, EXEC_IN_PIN),
        edge(1, EXEC_OUT_PIN, 2, EXEC_IN_PIN),
        edge(2, EXEC_OUT_PIN, 3, EXEC_IN_PIN),
        edge(3, EXEC_OUT_PIN, 4, EXEC_IN_PIN),
    ];

    let mut r = Runner::new(&doc, 0.1);
    for _ in 0..6 {
        r.tick();
        assert!(
            r.suspended() <= 1,
            "one thread can only be parked at one place at a time"
        );
        assert_eq!(r.instance.threads.len(), 1, "and it is still one thread");
    }
    assert!(r
        .instance
        .threads
        .iter()
        .all(|t| matches!(t.state, ThreadState::Finished)));
}

// ---------------------------------------------------------------------------
// 4. Serialization mid-wait
// ---------------------------------------------------------------------------

/// **The save-game story.** An instance holding a suspended activation — its
/// due time, its parked continuation, its loop frame, its locals — round-trips
/// through RON and carries on exactly where it was.
#[test]
fn an_instance_serializes_mid_wait_and_resumes() {
    let mut doc = GraphDoc::default();
    doc.variables = vec![VarDecl {
        slug: "total".into(),
        label: "Total".into(),
        ty: PinType::Int,
        default: Some(PropValue::Int(0)),
        group: None,
    }];
    let var = |id: u64, ty: &str| with(id, ty, &[(VAR_PROP, PropValue::Str("total".into()))]);
    doc.nodes = vec![
        node(0, EVENT_BEGIN_PLAY_TYPE_ID),
        with(1, n::FOR_LOOP, &[("first", PropValue::Int(1)), ("last", PropValue::Int(3))]),
        delay(2, 0.25),
        var(3, VAR_GET_TYPE_ID),
        node(4, n::ADD_INT),
        var(5, VAR_SET_TYPE_ID),
        print(6, ""),
        node(7, n::INT_TO_STRING),
        print(8, "done"),
    ];
    doc.edges = vec![
        edge(0, EXEC_OUT_PIN, 1, EXEC_IN_PIN),
        edge(1, "body", 2, EXEC_IN_PIN),
        edge(2, EXEC_OUT_PIN, 5, EXEC_IN_PIN),
        edge(3, VAR_VALUE_PIN, 4, "a"),
        edge(1, "index", 4, "b"),
        edge(4, "result", 5, VAR_VALUE_PIN),
        edge(5, EXEC_OUT_PIN, 6, EXEC_IN_PIN),
        edge(3, VAR_VALUE_PIN, 7, "value"),
        edge(7, "text", 6, "text"),
        edge(1, "completed", 8, EXEC_IN_PIN),
    ];

    // Run until it is parked mid-loop, having done some but not all of the
    // work — the interesting moment to save at.
    let mut r = Runner::new(&doc, 0.1);
    let mut before: Vec<String> = Vec::new();
    for _ in 0..4 {
        before.extend(r.tick());
    }
    assert_eq!(r.suspended(), 1, "parked mid-loop");
    let parked = r
        .instance
        .threads
        .iter()
        .find(|t| matches!(t.state, ThreadState::Suspended(_)))
        .unwrap();
    assert_eq!(parked.frames.len(), 1, "the loop frame is part of what is saved");
    assert!(parked.cursor.is_some(), "so is the resolved continuation");

    // Save, load, carry on.
    let text = ron::ser::to_string(&r.instance).expect("serialize mid-wait");
    let restored: GraphInstance = ron::from_str(&text).expect("deserialize mid-wait");
    assert_eq!(restored, r.instance, "the whole thread state is plain data");

    let mut resumed = Runner {
        plan: build(&doc),
        impls: n::std_impls(),
        instance: restored,
        time: r.time,
        dt: r.dt,
    };
    let mut after_restore: Vec<String> = Vec::new();
    for _ in 0..8 {
        after_restore.extend(resumed.tick());
    }

    // …and an unbroken run produces exactly the same thing.
    let mut straight = Runner::new(&doc, 0.1);
    let mut whole: Vec<String> = Vec::new();
    for _ in 0..12 {
        whole.extend(straight.tick());
    }
    let mut round_tripped = before.clone();
    round_tripped.extend(after_restore);
    assert_eq!(
        round_tripped, whole,
        "saving mid-wait changes nothing about what the graph does"
    );
    assert_eq!(whole, vec!["1", "3", "6", "done"], "and the work is right");
    assert_eq!(
        resumed.instance.variables,
        straight.instance.variables,
        "including the variables it accumulated across the waits"
    );
}

// ---------------------------------------------------------------------------
// 5. Drain order and BeginPlay-exactly-once, with a latent in flight
// ---------------------------------------------------------------------------

/// A suspended activation coexists with new events, and the drain order holds:
/// **due latents before custom events before Tick** — and BeginPlay still
/// fires exactly once per instance lifetime, not once per anything else.
#[test]
fn a_suspended_activation_coexists_with_new_events_in_drain_order() {
    let mut doc = GraphDoc::default();
    doc.nodes = vec![
        node(0, EVENT_BEGIN_PLAY_TYPE_ID),
        delay(1, 0.25),
        print(2, "latent"),
        with(3, EVENT_CUSTOM_TYPE_ID, &[(EVENT_NAME_PROP, PropValue::Str("ping".into()))]),
        print(4, "custom"),
        node(5, EVENT_TICK_TYPE_ID),
        print(6, "tick"),
        print(7, "begin"),
    ];
    doc.edges = vec![
        edge(0, EXEC_OUT_PIN, 7, EXEC_IN_PIN),
        edge(7, EXEC_OUT_PIN, 1, EXEC_IN_PIN),
        edge(1, EXEC_OUT_PIN, 2, EXEC_IN_PIN),
        edge(3, EXEC_OUT_PIN, 4, EXEC_IN_PIN),
        edge(5, EXEC_OUT_PIN, 6, EXEC_IN_PIN),
    ];

    let mut r = Runner::new(&doc, 0.1);
    assert_eq!(r.tick(), vec!["begin", "tick"], "BeginPlay, then Tick");
    assert_eq!(r.suspended(), 1);
    assert_eq!(r.tick(), vec!["tick"], "BeginPlay does not fire again");
    assert_eq!(r.tick(), vec!["tick"], "still waiting, still no second BeginPlay");

    // Queue a custom event for the tick on which the latent also comes due
    // (fired at t=0.1, due at 0.35, so t=0.4): the latent must drain first.
    r.instance
        .queue_event(EventPhase::Custom, Some("ping".into()), BTreeMap::new());
    assert_eq!(
        r.tick(),
        vec!["latent", "custom", "tick"],
        "due latents drain before custom events, which drain before Tick"
    );
    assert_eq!(r.suspended(), 0);

    // …and the ordering is the P1 constant, not a restatement.
    assert_eq!(
        node_graph_types::EVENT_DRAIN_ORDER,
        [
            EventPhase::BeginPlay,
            EventPhase::Latent,
            EventPhase::InputAction,
            EventPhase::Custom,
            EventPhase::Tick,
        ]
    );
}

/// Two latents coming due on the same tick wake in due-time order, then by
/// activation id — the tie-break that makes a replay reproducible.
#[test]
fn simultaneous_latents_wake_in_the_documented_order() {
    let mut doc = GraphDoc::default();
    doc.nodes = vec![
        with(0, EVENT_CUSTOM_TYPE_ID, &[(EVENT_NAME_PROP, PropValue::Str("go".into()))]),
        delay(1, 0.15),
        print(2, ""),
        node(3, n::INT_TO_STRING),
    ];
    doc.edges = vec![
        edge(0, EXEC_OUT_PIN, 1, EXEC_IN_PIN),
        edge(1, EXEC_OUT_PIN, 2, EXEC_IN_PIN),
        edge(0, "payload", 3, "value"),
        edge(3, "text", 2, "text"),
    ];
    doc.nodes[0].properties.insert(
        format!("{}payload", node_graph_types::EVENT_PAYLOAD_PREFIX),
        PropValue::Enum("int".into()),
    );

    let mut r = Runner::new(&doc, 0.1);
    // Three activations, identical due times: the activation id decides.
    for v in [1i32, 2, 3] {
        let mut payload = BTreeMap::new();
        payload.insert("payload".to_string(), node_graph_exec::Value::Int(v));
        r.instance
            .queue_event(EventPhase::Custom, Some("go".into()), payload);
    }
    r.tick(); // t=0.1: all three park, due at 0.25
    assert_eq!(r.suspended(), 3);
    assert_eq!(r.tick_report().resumed, 0, "t=0.2: not yet");
    assert_eq!(r.tick_report().resumed, 3, "t=0.3: all three came due together");

    let mut r2 = Runner::new(&doc, 0.1);
    for v in [1i32, 2, 3] {
        let mut payload = BTreeMap::new();
        payload.insert("payload".to_string(), node_graph_exec::Value::Int(v));
        r2.instance
            .queue_event(EventPhase::Custom, Some("go".into()), payload);
    }
    r2.tick();
    assert_eq!(
        r2.tick_until_output(4),
        vec!["1", "2", "3"],
        "in activation-id order, which is the order they were queued"
    );
}

// ---------------------------------------------------------------------------
// Budget
// ---------------------------------------------------------------------------

/// A resumed activation spends budget like any other work: the tick that
/// wakes a runaway is the tick that kills it.
#[test]
fn a_resume_counts_against_the_tick_budget() {
    let mut doc = GraphDoc::default();
    doc.nodes = vec![
        node(0, EVENT_BEGIN_PLAY_TYPE_ID),
        delay(1, 0.15),
        with(2, n::FOR_LOOP, &[("first", PropValue::Int(0)), ("last", PropValue::Int(i32::MAX))]),
        print(3, "x"),
    ];
    doc.edges = vec![
        edge(0, EXEC_OUT_PIN, 1, EXEC_IN_PIN),
        edge(1, EXEC_OUT_PIN, 2, EXEC_IN_PIN),
        edge(2, "body", 3, EXEC_IN_PIN),
    ];

    let plan = build(&doc);
    let impls = n::std_impls();
    let mut inst = GraphInstance::new(&plan, EntityRef::SelfEntity, 1);
    let mut fx = Vec::new();

    // Tick 1: reach the Delay and park. Cheap, and nothing is killed.
    let r1 = tick_with_budget(
        &plan,
        &mut inst,
        &impls,
        TickInput { dt: 0.1, time: 0.1 },
        &NoWorld,
        &mut fx,
        400,
    );
    assert_eq!(r1.halted, None);
    assert!(r1.steps < 400, "parking is cheap: {}", r1.steps);

    // Tick 2: still waiting — a tick that resumes nothing costs nothing.
    let r2 = tick_with_budget(
        &plan,
        &mut inst,
        &impls,
        TickInput { dt: 0.1, time: 0.2 },
        &NoWorld,
        &mut fx,
        400,
    );
    assert_eq!(r2.resumed, 0, "fired at t=0.1, due at 0.25");
    assert_eq!(r2.halted, None);

    // Tick 3: due.
    let r2 = tick_with_budget(
        &plan,
        &mut inst,
        &impls,
        TickInput { dt: 0.1, time: 0.3 },
        &NoWorld,
        &mut fx,
        400,
    );
    assert_eq!(r2.resumed, 1);

    // The resume ran straight into the runaway loop and hit the wall.
    assert!(
        matches!(r2.halted, Some(ExecError::BudgetExceeded { .. })),
        "a resumed activation is charged like any other firing: {r2:?}"
    );
    assert!(inst.halted.is_some());
}
