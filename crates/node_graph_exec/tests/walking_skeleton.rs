//! End-to-end proof of the P2 contract: documents authored the way the
//! editor authors them, compiled, run, and checked against their effect
//! streams.
//!
//! These are integration tests on purpose — they exercise the crate through
//! its public API only, which is the same surface the engine spike and the
//! future SpacetimeDB module see.

use std::collections::BTreeMap;

use node_graph_exec::nodes::{ADD_INT, BRANCH, FOR_LOOP, INT_TO_STRING, PRINT, SET_POSITION};
use node_graph_exec::{
    compile, nodes, tick, tick_with_budget, CompileError, Effect, EntityRef, ExecError,
    GraphInstance, LogLevel, NoWorld, NodeImpls, TickInput, Value, WorldRead,
};
use node_graph_types::{
    register_std_events, Edge, GraphDoc, IfacePin, NodeInst, NodeRegistry, PinType, PropValue,
    VarDecl, EVENT_BEGIN_PLAY_TYPE_ID, EVENT_CUSTOM_TYPE_ID, EVENT_NAME_PROP, EVENT_TICK_TYPE_ID,
    EXEC_IN_PIN, EXEC_OUT_PIN, GRAPH_INPUT_TYPE_ID, GRAPH_OUTPUT_TYPE_ID, REROUTE_IN, REROUTE_OUT,
    REROUTE_TYPE_ID, SUBGRAPH_TYPE_ID, VAR_GET_TYPE_ID, VAR_PROP, VAR_SET_TYPE_ID, VAR_VALUE_PIN,
};

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn registry() -> NodeRegistry {
    let mut reg = NodeRegistry::new();
    nodes::register_descriptors(&mut reg).unwrap();
    register_std_events(&mut reg).unwrap();
    reg
}

fn impls() -> NodeImpls {
    let mut i = NodeImpls::new();
    nodes::register_impls(&mut i);
    i
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

fn with(id: u64, type_id: &str, props: &[(&str, PropValue)]) -> NodeInst {
    let mut n = node(id, type_id);
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

/// Run a document for `ticks` ticks and return the effect stream.
fn run(doc: &GraphDoc, ticks: usize) -> Vec<Effect> {
    run_with(doc, &BTreeMap::new(), ticks, 1234, &NoWorld).0
}

fn run_with(
    doc: &GraphDoc,
    subs: &BTreeMap<String, GraphDoc>,
    ticks: usize,
    seed: u64,
    world: &dyn WorldRead,
) -> (Vec<Effect>, GraphInstance) {
    let reg = registry();
    let plan = compile(doc, "test.graph", &reg, subs).expect("compile");
    let impls = impls();
    assert_eq!(impls.check_plan(&plan), vec![], "plan/impl cross-check");
    let mut inst = GraphInstance::new(&plan, EntityRef::SelfEntity, seed);
    let mut effects: Vec<Effect> = Vec::new();
    for i in 0..ticks {
        let t = TickInput { dt: 0.1, time: i as f64 * 0.1 };
        tick(&plan, &mut inst, &impls, t, world, &mut effects);
    }
    (effects, inst)
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

fn print(id: u64, text: &str) -> NodeInst {
    with(id, PRINT, &[("text", PropValue::Str(text.to_string()))])
}

// ---------------------------------------------------------------------------
// Compilation
// ---------------------------------------------------------------------------

/// A document that does not validate never becomes a plan. Compilation is not
/// the place to discover a mistyped wire — but a *warning* (an unbound
/// interface pin) is not an error and must not block a run.
#[test]
fn compile_refuses_errors_and_tolerates_warnings() {
    let reg = registry();
    let subs: BTreeMap<String, GraphDoc> = BTreeMap::new();

    let mut broken = GraphDoc::default();
    broken.nodes = vec![node(0, "no_such_type")];
    match compile(&broken, "test.graph", &reg, &subs) {
        Err(CompileError::Invalid { errors, .. }) => assert_eq!(errors.len(), 1, "{errors:?}"),
        other => panic!("expected refusal, got {other:?}"),
    }

    // An interface pin nothing binds is a warning: the graph still runs.
    let mut warned = GraphDoc::default();
    warned.inputs = vec![IfacePin {
        slug: "amount".into(),
        label: "Amount".into(),
        ty: PinType::Float,
    }];
    warned.nodes = vec![node(0, EVENT_BEGIN_PLAY_TYPE_ID), print(1, "ran")];
    warned.edges = vec![edge(0, EXEC_OUT_PIN, 1, EXEC_IN_PIN)];
    assert!(compile(&warned, "test.graph", &reg, &subs).is_ok());
}

/// Reroutes are transparent at run time: they carry a value and an exec
/// continuation, and leave no node in the plan.
#[test]
fn reroutes_are_spliced_away() {
    let mut doc = GraphDoc::default();
    doc.nodes = vec![
        node(0, EVENT_BEGIN_PLAY_TYPE_ID),
        node(1, REROUTE_TYPE_ID),
        with(2, ADD_INT, &[("a", PropValue::Int(2)), ("b", PropValue::Int(3))]),
        node(3, REROUTE_TYPE_ID),
        print(4, "unused"),
        node(5, INT_TO_STRING),
    ];
    doc.edges = vec![
        edge(0, EXEC_OUT_PIN, 1, REROUTE_IN),
        edge(1, REROUTE_OUT, 4, EXEC_IN_PIN),
        edge(2, "sum", 3, REROUTE_IN),
        edge(3, REROUTE_OUT, 5, "value"),
        edge(5, "text", 4, "text"),
    ];

    let reg = registry();
    let subs: BTreeMap<String, GraphDoc> = BTreeMap::new();
    let plan = compile(&doc, "test.graph", &reg, &subs).expect("compile");
    assert!(
        plan.nodes.iter().all(|n| n.type_id != REROUTE_TYPE_ID),
        "no reroute survives compilation: {:?}",
        plan.nodes.iter().map(|n| &n.type_id).collect::<Vec<_>>()
    );
    // Both the exec hop and the data hop landed where they should.
    assert_eq!(logs(&run(&doc, 1)), vec!["5"]);
}

/// Subgraph splicing (the P1 interface binding, consumed): host edges pass
/// *through* `graph_input`/`graph_output` into the inlined body, and the
/// subgraph leaves no node of its own behind.
#[test]
fn subgraphs_inline_through_their_interface() {
    // The subgraph: exec + an Int in, the Int + 10 out.
    let mut sub = GraphDoc::default();
    sub.inputs = vec![
        IfacePin { slug: "run".into(), label: "Run".into(), ty: PinType::Exec },
        IfacePin { slug: "n".into(), label: "N".into(), ty: PinType::Int },
    ];
    sub.outputs = vec![
        IfacePin { slug: "done".into(), label: "Done".into(), ty: PinType::Exec },
        IfacePin { slug: "out".into(), label: "Out".into(), ty: PinType::Int },
    ];
    sub.nodes = vec![
        node(0, GRAPH_INPUT_TYPE_ID),
        node(1, GRAPH_OUTPUT_TYPE_ID),
        with(2, ADD_INT, &[("b", PropValue::Int(10))]),
        print(3, "inside"),
    ];
    sub.edges = vec![
        edge(0, "run", 3, EXEC_IN_PIN),
        edge(3, EXEC_OUT_PIN, 1, "done"),
        edge(0, "n", 2, "a"),
        edge(2, "sum", 1, "out"),
    ];

    // The host: BeginPlay -> subgraph(5) -> Print(the result).
    let mut host = GraphDoc::default();
    let mut sub_node = node(1, SUBGRAPH_TYPE_ID);
    sub_node.subgraph = Some("lib/add10.subgraph".into());
    sub_node.properties.insert("n".into(), PropValue::Int(5));
    host.nodes = vec![
        node(0, EVENT_BEGIN_PLAY_TYPE_ID),
        sub_node,
        print(2, "after"),
        node(3, INT_TO_STRING),
    ];
    host.edges = vec![
        edge(0, EXEC_OUT_PIN, 1, "run"),
        edge(1, "done", 2, EXEC_IN_PIN),
        edge(1, "out", 3, "value"),
        edge(3, "text", 2, "text"),
    ];

    let mut subs = BTreeMap::new();
    subs.insert("lib/add10.subgraph".to_string(), sub);

    let reg = registry();
    let plan = compile(&host, "test.graph", &reg, &subs).expect("compile");
    assert!(
        plan.nodes.iter().all(|n| {
            !matches!(
                n.type_id.as_str(),
                SUBGRAPH_TYPE_ID | GRAPH_INPUT_TYPE_ID | GRAPH_OUTPUT_TYPE_ID
            )
        }),
        "the boundary is gone at run time: {:?}",
        plan.nodes.iter().map(|n| &n.type_id).collect::<Vec<_>>()
    );

    let (effects, _) = run_with(&host, &subs, 1, 1, &NoWorld);
    assert_eq!(
        logs(&effects),
        vec!["inside", "15"],
        "exec entered the subgraph, the data crossed both ways, and exec came back out"
    );
}

/// Unreachable nodes are pruned: a disconnected experiment costs nothing.
#[test]
fn unreachable_nodes_are_pruned() {
    let mut doc = GraphDoc::default();
    doc.nodes = vec![
        node(0, EVENT_BEGIN_PLAY_TYPE_ID),
        print(1, "reached"),
        print(2, "orphan"),
        with(3, ADD_INT, &[]),
    ];
    doc.edges = vec![edge(0, EXEC_OUT_PIN, 1, EXEC_IN_PIN)];
    let reg = registry();
    let subs: BTreeMap<String, GraphDoc> = BTreeMap::new();
    let plan = compile(&doc, "test.graph", &reg, &subs).unwrap();
    assert_eq!(plan.nodes.len(), 2, "{:?}", plan.nodes);
    assert_eq!(logs(&run(&doc, 1)), vec!["reached"]);
}

// ---------------------------------------------------------------------------
// Execution
// ---------------------------------------------------------------------------

/// Branch picks a continuation; the untaken side never runs.
#[test]
fn branch_picks_one_side() {
    let mut doc = GraphDoc::default();
    doc.nodes = vec![
        node(0, EVENT_BEGIN_PLAY_TYPE_ID),
        with(1, BRANCH, &[("condition", PropValue::Bool(true))]),
        print(2, "yes"),
        print(3, "no"),
    ];
    doc.edges = vec![
        edge(0, EXEC_OUT_PIN, 1, EXEC_IN_PIN),
        edge(1, "true", 2, EXEC_IN_PIN),
        edge(1, "false", 3, EXEC_IN_PIN),
    ];
    assert_eq!(logs(&run(&doc, 1)), vec!["yes"]);

    doc.nodes[1]
        .properties
        .insert("condition".into(), PropValue::Bool(false));
    assert_eq!(logs(&run(&doc, 1)), vec!["no"]);
}

/// The frame protocol, end to end: the loop runs its body once per index,
/// the index is visible to the body's pure chain, and `completed` fires once
/// afterwards — with a stateless implementation throughout.
#[test]
fn for_loop_runs_its_body_then_completes() {
    let mut doc = GraphDoc::default();
    doc.nodes = vec![
        node(0, EVENT_BEGIN_PLAY_TYPE_ID),
        with(
            1,
            FOR_LOOP,
            &[("first", PropValue::Int(1)), ("last", PropValue::Int(3))],
        ),
        print(2, ""),
        print(3, "done"),
        with(4, ADD_INT, &[("b", PropValue::Int(100))]),
        node(5, INT_TO_STRING),
    ];
    doc.edges = vec![
        edge(0, EXEC_OUT_PIN, 1, EXEC_IN_PIN),
        edge(1, "body", 2, EXEC_IN_PIN),
        edge(1, "completed", 3, EXEC_IN_PIN),
        // index -> Add(+100) -> the printed text, so the body observes the
        // loop variable through a pure chain re-evaluated per iteration.
        edge(1, "index", 4, "a"),
        edge(4, "sum", 5, "value"),
        edge(5, "text", 2, "text"),
    ];
    assert_eq!(logs(&run(&doc, 1)), vec!["101", "102", "103", "done"]);

    // An empty range runs the body zero times and still completes.
    doc.nodes[1].properties.insert("last".into(), PropValue::Int(0));
    assert_eq!(logs(&run(&doc, 1)), vec!["done"]);

    // A loop with nothing wired to its body terminates rather than spinning.
    doc.nodes[1].properties.insert("last".into(), PropValue::Int(3));
    doc.edges.retain(|e| !(e.from_node == 1 && e.from_pin == "body"));
    assert_eq!(logs(&run(&doc, 1)), vec!["done"]);
}

/// Nested loops: the frame *stack* is what makes the inner loop resume the
/// outer one, and the implementations know nothing about each other.
#[test]
fn nested_loops_use_the_frame_stack() {
    let mut doc = GraphDoc::default();
    doc.nodes = vec![
        node(0, EVENT_BEGIN_PLAY_TYPE_ID),
        with(1, FOR_LOOP, &[("first", PropValue::Int(1)), ("last", PropValue::Int(2))]),
        with(2, FOR_LOOP, &[("first", PropValue::Int(1)), ("last", PropValue::Int(2))]),
        print(3, ""),
        print(4, "end"),
        node(5, INT_TO_STRING),
    ];
    doc.edges = vec![
        edge(0, EXEC_OUT_PIN, 1, EXEC_IN_PIN),
        edge(1, "body", 2, EXEC_IN_PIN),
        edge(2, "body", 3, EXEC_IN_PIN),
        edge(2, "index", 5, "value"),
        edge(5, "text", 3, "text"),
        edge(1, "completed", 4, EXEC_IN_PIN),
    ];
    assert_eq!(
        logs(&run(&doc, 1)),
        vec!["1", "2", "1", "2", "end"],
        "the inner loop runs in full per outer iteration"
    );
}

/// Variables: a `var_set` writes, a `var_get` reads, the value survives
/// across ticks, and the pure read is memoized per statement but re-read
/// after a write.
#[test]
fn variables_read_and_write_across_ticks() {
    let mut doc = GraphDoc::default();
    doc.variables = vec![VarDecl {
        slug: "count".into(),
        label: "Count".into(),
        ty: PinType::Int,
        default: Some(PropValue::Int(0)),
    }];
    let var_prop = |n: u64, ty: &str| {
        with(n, ty, &[(VAR_PROP, PropValue::Str("count".into()))])
    };
    doc.nodes = vec![
        node(0, EVENT_TICK_TYPE_ID),
        var_prop(1, VAR_GET_TYPE_ID),
        with(2, ADD_INT, &[("b", PropValue::Int(1))]),
        var_prop(3, VAR_SET_TYPE_ID),
        print(4, ""),
        var_prop(5, VAR_GET_TYPE_ID),
        node(6, INT_TO_STRING),
    ];
    doc.edges = vec![
        edge(0, EXEC_OUT_PIN, 3, EXEC_IN_PIN),
        edge(1, VAR_VALUE_PIN, 2, "a"),
        edge(2, "sum", 3, VAR_VALUE_PIN),
        edge(3, EXEC_OUT_PIN, 4, EXEC_IN_PIN),
        edge(5, VAR_VALUE_PIN, 6, "value"),
        edge(6, "text", 4, "text"),
    ];

    let (effects, inst) = run_with(&doc, &BTreeMap::new(), 3, 7, &NoWorld);
    assert_eq!(
        logs(&effects),
        vec!["1", "2", "3"],
        "the write of one tick is visible to the read of the next, and the \
         read after the write in the same tick sees the new value"
    );
    assert_eq!(inst.variables.get("count"), Some(&Value::Int(3)));
}

/// The world seam: a pure read through `WorldRead`, an effect out through
/// `EffectSink`. Nothing else touches the world.
#[test]
fn world_read_and_effect_out() {
    struct At([f32; 3]);
    impl WorldRead for At {
        fn position(&self, _e: EntityRef) -> Option<[f32; 3]> {
            Some(self.0)
        }
        fn exists(&self, _e: EntityRef) -> bool {
            true
        }
    }

    let mut doc = GraphDoc::default();
    doc.nodes = vec![
        node(0, EVENT_BEGIN_PLAY_TYPE_ID),
        node(1, nodes::GET_POSITION),
        node(2, SET_POSITION),
    ];
    doc.edges = vec![
        edge(0, EXEC_OUT_PIN, 2, EXEC_IN_PIN),
        edge(1, "position", 2, "position"),
    ];
    let (effects, _) = run_with(&doc, &BTreeMap::new(), 1, 1, &At([1.0, 2.0, 3.0]));
    assert_eq!(
        effects,
        vec![Effect::SetPosition {
            entity: EntityRef::SelfEntity,
            position: [1.0, 2.0, 3.0],
        }]
    );
}

// ---------------------------------------------------------------------------
// Events, budget, determinism
// ---------------------------------------------------------------------------

/// BeginPlay fires exactly once per instance lifetime; Tick fires every tick;
/// and the drain order within a tick is the documented one.
#[test]
fn event_phases_drain_in_order() {
    let mut doc = GraphDoc::default();
    doc.nodes = vec![
        node(0, EVENT_BEGIN_PLAY_TYPE_ID),
        print(1, "begin"),
        node(2, EVENT_TICK_TYPE_ID),
        print(3, "tick"),
        with(4, EVENT_CUSTOM_TYPE_ID, &[(EVENT_NAME_PROP, PropValue::Str("ping".into()))]),
        print(5, "custom"),
    ];
    doc.edges = vec![
        edge(0, EXEC_OUT_PIN, 1, EXEC_IN_PIN),
        edge(2, EXEC_OUT_PIN, 3, EXEC_IN_PIN),
        edge(4, EXEC_OUT_PIN, 5, EXEC_IN_PIN),
    ];

    let reg = registry();
    let subs: BTreeMap<String, GraphDoc> = BTreeMap::new();
    let plan = compile(&doc, "test.graph", &reg, &subs).unwrap();
    let impls = impls();
    let mut inst = GraphInstance::new(&plan, EntityRef::SelfEntity, 1);
    let mut effects: Vec<Effect> = Vec::new();

    let t = TickInput { dt: 0.1, time: 0.0 };
    tick(&plan, &mut inst, &impls, t, &NoWorld, &mut effects);
    assert_eq!(logs(&effects), vec!["begin", "tick"], "BeginPlay precedes Tick");

    // A custom event queued now is delivered on the *next* tick — before
    // Tick, after everything earlier in the order.
    effects.clear();
    inst.queue_event(
        node_graph_types::EventPhase::Custom,
        Some("ping".into()),
        BTreeMap::new(),
    );
    tick(&plan, &mut inst, &impls, t, &NoWorld, &mut effects);
    assert_eq!(
        logs(&effects),
        vec!["custom", "tick"],
        "custom events drain before Tick, and BeginPlay does not fire twice"
    );

    // A custom event nobody declared is delivered to nothing, not to
    // everything.
    effects.clear();
    inst.queue_event(
        node_graph_types::EventPhase::Custom,
        Some("nope".into()),
        BTreeMap::new(),
    );
    tick(&plan, &mut inst, &impls, t, &NoWorld, &mut effects);
    assert_eq!(logs(&effects), vec!["tick"]);
}

/// Multiple entry nodes for the same event all fire, in document order (D3).
#[test]
fn multiple_entries_for_one_event_all_fire_in_doc_order() {
    let mut doc = GraphDoc::default();
    doc.nodes = vec![
        node(0, EVENT_BEGIN_PLAY_TYPE_ID),
        print(1, "first"),
        node(2, EVENT_BEGIN_PLAY_TYPE_ID),
        print(3, "second"),
    ];
    doc.edges = vec![
        edge(0, EXEC_OUT_PIN, 1, EXEC_IN_PIN),
        edge(2, EXEC_OUT_PIN, 3, EXEC_IN_PIN),
    ];
    assert_eq!(logs(&run(&doc, 1)), vec!["first", "second"]);
}

/// The budget kills a runaway and the report names the node, and the whole
/// **instance** halts rather than being trusted afterwards.
///
/// The runaway here is a loop, not a bare exec cycle — and that is a finding
/// worth recording rather than a limitation of the test. Task 40's
/// `InputMultiplyConnected` rule applies to *every* input pin including exec
/// ones, so an exec pin takes exactly one incoming wire. Any cycle reachable
/// from an entry point would need a second wire into its join node, which
/// means a bare exec cycle **cannot be authored at all** in v1. The budget's
/// real job is runaway iteration.
#[test]
fn budget_kills_a_runaway_and_names_the_node() {
    let mut doc = GraphDoc::default();
    doc.nodes = vec![
        node(0, EVENT_BEGIN_PLAY_TYPE_ID),
        with(
            1,
            FOR_LOOP,
            &[("first", PropValue::Int(0)), ("last", PropValue::Int(i32::MAX))],
        ),
        print(2, "spin"),
    ];
    doc.edges = vec![
        edge(0, EXEC_OUT_PIN, 1, EXEC_IN_PIN),
        edge(1, "body", 2, EXEC_IN_PIN),
    ];

    // An exec cycle really is unauthorable: a second wire into an exec input
    // is the same error as a second wire into a data input.
    let mut cyclic = doc.clone();
    cyclic.edges.push(edge(2, EXEC_OUT_PIN, 2, EXEC_IN_PIN));
    match compile(&cyclic, "test.graph", &registry(), &BTreeMap::new()) {
        Err(CompileError::Invalid { errors, .. }) => assert!(
            errors
                .iter()
                .any(|e| matches!(e, node_graph_types::GraphError::InputMultiplyConnected { .. })),
            "{errors:?}"
        ),
        other => panic!("expected refusal, got {other:?}"),
    }

    let reg = registry();
    let subs: BTreeMap<String, GraphDoc> = BTreeMap::new();
    let plan = compile(&doc, "test.graph", &reg, &subs).unwrap();
    let impls = impls();
    let mut inst = GraphInstance::new(&plan, EntityRef::SelfEntity, 1);
    let mut effects: Vec<Effect> = Vec::new();
    let report = tick_with_budget(
        &plan,
        &mut inst,
        &impls,
        TickInput { dt: 0.1, time: 0.0 },
        &NoWorld,
        &mut effects,
        500,
    );

    match report.halted {
        Some(ExecError::BudgetExceeded { node, budget }) => {
            assert_eq!(budget, 500);
            assert!(
                node.contains("print") || node.contains("for_loop"),
                "the report names the node it died on: {node}"
            );
        }
        other => panic!("expected a budget kill, got {other:?}"),
    }
    assert!(effects.len() <= 500, "it stopped, it did not hang");
    assert!(inst.halted.is_some(), "the instance halts");

    // A halted instance ticks no further.
    effects.clear();
    tick(&plan, &mut inst, &impls, TickInput::default(), &NoWorld, &mut effects);
    assert_eq!(effects, vec![]);
}

/// A runaway *loop* (rather than an exec cycle) hits the same wall.
#[test]
fn budget_kills_a_runaway_loop() {
    let mut doc = GraphDoc::default();
    doc.nodes = vec![
        node(0, EVENT_BEGIN_PLAY_TYPE_ID),
        with(
            1,
            FOR_LOOP,
            &[("first", PropValue::Int(0)), ("last", PropValue::Int(i32::MAX))],
        ),
        print(2, "x"),
    ];
    doc.edges = vec![
        edge(0, EXEC_OUT_PIN, 1, EXEC_IN_PIN),
        edge(1, "body", 2, EXEC_IN_PIN),
    ];
    let reg = registry();
    let subs: BTreeMap<String, GraphDoc> = BTreeMap::new();
    let plan = compile(&doc, "test.graph", &reg, &subs).unwrap();
    let mut inst = GraphInstance::new(&plan, EntityRef::SelfEntity, 1);
    let mut effects: Vec<Effect> = Vec::new();
    let report = tick_with_budget(
        &plan,
        &mut inst,
        &impls(),
        TickInput::default(),
        &NoWorld,
        &mut effects,
        1_000,
    );
    assert!(matches!(report.halted, Some(ExecError::BudgetExceeded { .. })));
}

/// **The determinism test** (D8): the same plan, the same seed and the same
/// scripted inputs produce identical effect streams, run after run. This is
/// the seed of the future WASM parity suite — if a node ever reads a clock or
/// iterates a `HashMap`, this fails loudly.
#[test]
fn determinism_holds_across_runs() {
    let mut doc = GraphDoc::default();
    doc.variables = vec![VarDecl {
        slug: "total".into(),
        label: "Total".into(),
        ty: PinType::Int,
        default: Some(PropValue::Int(0)),
    }];
    doc.nodes = vec![
        node(0, EVENT_TICK_TYPE_ID),
        with(1, FOR_LOOP, &[("first", PropValue::Int(1)), ("last", PropValue::Int(4))]),
        with(2, VAR_GET_TYPE_ID, &[(VAR_PROP, PropValue::Str("total".into()))]),
        with(3, ADD_INT, &[]),
        with(4, VAR_SET_TYPE_ID, &[(VAR_PROP, PropValue::Str("total".into()))]),
        print(5, ""),
        with(6, BRANCH, &[("condition", PropValue::Bool(true))]),
        node(7, SET_POSITION),
        node(8, INT_TO_STRING),
    ];
    doc.edges = vec![
        edge(0, EXEC_OUT_PIN, 1, EXEC_IN_PIN),
        edge(1, "body", 4, EXEC_IN_PIN),
        edge(1, "index", 3, "b"),
        edge(2, VAR_VALUE_PIN, 3, "a"),
        edge(3, "sum", 4, VAR_VALUE_PIN),
        edge(4, EXEC_OUT_PIN, 6, EXEC_IN_PIN),
        edge(6, "true", 5, EXEC_IN_PIN),
        edge(2, VAR_VALUE_PIN, 8, "value"),
        edge(8, "text", 5, "text"),
        edge(1, "completed", 7, EXEC_IN_PIN),
    ];

    let scripted: Vec<TickInput> = (0..5)
        .map(|i| TickInput { dt: 0.016, time: i as f64 * 0.016 })
        .collect();

    let once = |seed: u64| -> Vec<Effect> {
        let reg = registry();
        let subs: BTreeMap<String, GraphDoc> = BTreeMap::new();
        let plan = compile(&doc, "test.graph", &reg, &subs).unwrap();
        let impls = impls();
        let mut inst = GraphInstance::new(&plan, EntityRef::SelfEntity, seed);
        let mut effects: Vec<Effect> = Vec::new();
        for t in &scripted {
            tick(&plan, &mut inst, &impls, *t, &NoWorld, &mut effects);
        }
        effects
    };

    let a = once(0xDEAD_BEEF);
    let b = once(0xDEAD_BEEF);
    assert_eq!(a, b, "same seed, same inputs, same effects");
    assert!(!a.is_empty(), "the fixture has to actually do something");

    // The instance state matches too, not just the visible output.
    let state = |seed: u64| {
        let reg = registry();
        let subs: BTreeMap<String, GraphDoc> = BTreeMap::new();
        let plan = compile(&doc, "test.graph", &reg, &subs).unwrap();
        let mut inst = GraphInstance::new(&plan, EntityRef::SelfEntity, seed);
        let mut sink: Vec<Effect> = Vec::new();
        for t in &scripted {
            tick(&plan, &mut inst, &impls(), *t, &NoWorld, &mut sink);
        }
        inst
    };
    assert_eq!(state(1), state(1));
}

/// The instance is plain data: it round-trips through RON mid-run, which is
/// what P4's "serialize an instance mid-wait" requirement rests on.
#[test]
fn instance_state_is_plain_serializable_data() {
    let mut doc = GraphDoc::default();
    doc.variables = vec![VarDecl {
        slug: "n".into(),
        label: "N".into(),
        ty: PinType::Int,
        default: Some(PropValue::Int(2)),
    }];
    doc.nodes = vec![
        node(0, EVENT_TICK_TYPE_ID),
        with(1, FOR_LOOP, &[("first", PropValue::Int(1)), ("last", PropValue::Int(2))]),
        print(2, "x"),
    ];
    doc.edges = vec![
        edge(0, EXEC_OUT_PIN, 1, EXEC_IN_PIN),
        edge(1, "body", 2, EXEC_IN_PIN),
    ];
    let (_, inst) = run_with(&doc, &BTreeMap::new(), 2, 5, &NoWorld);
    let text = ron::ser::to_string(&inst).expect("serialize");
    let back: GraphInstance = ron::from_str(&text).expect("deserialize");
    assert_eq!(back, inst);
}

/// A missing implementation is caught before the run, not when execution
/// happens to reach that node.
#[test]
fn plan_impl_cross_check_catches_a_missing_node() {
    let mut doc = GraphDoc::default();
    doc.nodes = vec![node(0, EVENT_BEGIN_PLAY_TYPE_ID), print(1, "x")];
    doc.edges = vec![edge(0, EXEC_OUT_PIN, 1, EXEC_IN_PIN)];
    let reg = registry();
    let subs: BTreeMap<String, GraphDoc> = BTreeMap::new();
    let plan = compile(&doc, "test.graph", &reg, &subs).unwrap();

    // An empty implementation set: every node in the plan is reported, up
    // front, rather than when execution happens to reach it — which, inside a
    // rarely-taken Branch, could be days later.
    let problems = NodeImpls::new().check_plan(&plan);
    assert_eq!(problems.len(), plan.nodes.len(), "{problems:?}");
    assert!(
        problems
            .iter()
            .any(|e| matches!(e, ExecError::MissingImpl { type_id, .. }
                if type_id == EVENT_BEGIN_PLAY_TYPE_ID)),
        "{problems:?}"
    );
    // The full set is clean.
    assert_eq!(impls().check_plan(&plan), vec![]);
}

/// `Log` carries a level, so the runner can route warnings differently from
/// prints without parsing text.
#[test]
fn log_effects_carry_a_level() {
    let mut doc = GraphDoc::default();
    doc.nodes = vec![node(0, EVENT_BEGIN_PLAY_TYPE_ID), print(1, "hi")];
    doc.edges = vec![edge(0, EXEC_OUT_PIN, 1, EXEC_IN_PIN)];
    assert_eq!(
        run(&doc, 1),
        vec![Effect::Log { level: LogLevel::Info, text: "hi".into() }]
    );
}
