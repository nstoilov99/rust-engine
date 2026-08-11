//! Per-node semantics for the standard library (Task 45-A P3, D5).
//!
//! Every registered node type is exercised here, and
//! `every_std_node_is_covered` fails if one is added without a test — the
//! cheapest way to keep "~40 nodes" from quietly becoming "~40 nodes, six of
//! which have never run".
//!
//! Pure nodes are tested through a one-statement harness rather than a real
//! graph: wire the node's output into a `Print` and read the effect stream.
//! That deliberately goes through the *whole* pipeline — validation, splice,
//! pull — so a descriptor that disagrees with its implementation fails here
//! rather than in P5.

// Tests build documents the way an author does: start from the default and
// fill in what matters. A single giant struct literal would technically
// satisfy clippy and be markedly harder to read.
#![allow(clippy::field_reassign_with_default)]

use std::collections::BTreeMap;

use node_graph_exec::nodes as n;
use node_graph_exec::{
    compile, tick, Effect, EntityRef, GraphInstance, NoWorld, TickInput, TransformSnapshot,
    WorldRead,
};
use node_graph_types::{
    register_std_events, register_std_nodes, Edge, GraphDoc, NodeInst, NodeRegistry, PinType,
    PropValue, VarDecl, EVENT_BEGIN_PLAY_TYPE_ID, EVENT_CUSTOM_TYPE_ID, EVENT_NAME_PROP,
    EVENT_TICK_TYPE_ID, EXEC_IN_PIN, EXEC_OUT_PIN, VAR_GET_TYPE_ID, VAR_PROP, VAR_SET_TYPE_ID,
    VAR_VALUE_PIN,
};

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// Every standard node, and the test that pins its semantics. This is a
/// hand-maintained manifest rather than a runtime tally on purpose: `cargo
/// test` runs a binary's tests in parallel, so anything accumulated at run
/// time is a race, and a coverage check that passes because it ran early is
/// worse than none. Adding a node without adding it here fails
/// `every_std_node_is_covered`.
const TESTED: &[(&str, &str)] = &[
    // control
    (n::BRANCH, "branch_and_for_loop_basics"),
    (n::SEQUENCE, "sequence_runs_each_output_in_order_skipping_gaps"),
    (n::FOR_LOOP, "branch_and_for_loop_basics"),
    (n::WHILE_LOOP, "while_loop_rechecks_its_condition_each_pass"),
    (n::DELAY, "tests/latent.rs — the whole file"),
    (
        node_graph_types::TIMELINE_TYPE_ID,
        "tests/timeline.rs — the whole file",
    ),
    (n::FOR_EACH_INT, "for_each_walks_an_array_with_element_and_index"),
    (n::FOR_EACH_FLOAT, "for_each_walks_an_array_with_element_and_index"),
    (n::FOR_EACH_ENTITY, "for_each_walks_an_array_with_element_and_index"),
    (n::GATE, "gate_opens_closes_and_toggles_by_entrance"),
    (n::DO_ONCE, "do_once_passes_once_then_needs_a_reset"),
    (n::FLIP_FLOP, "flip_flop_alternates_and_reports_its_side"),
    (n::SELECT_FLOAT, "select_picks_a_value_per_type"),
    (n::SELECT_INT, "select_picks_a_value_per_type"),
    (n::SELECT_BOOL, "select_picks_a_value_per_type"),
    (n::SELECT_STRING, "select_picks_a_value_per_type"),
    // logic / math
    (n::AND, "boolean_logic"),
    (n::OR, "boolean_logic"),
    (n::NOT, "boolean_logic"),
    (n::COMPARE_INT, "comparisons_cover_every_declared_operator"),
    (n::COMPARE_FLOAT, "comparisons_cover_every_declared_operator"),
    (n::ADD_INT, "integer_arithmetic"),
    (n::SUB_INT, "integer_arithmetic"),
    (n::MUL_INT, "integer_arithmetic"),
    (n::DIV_INT, "integer_arithmetic"),
    (n::ADD_FLOAT, "float_arithmetic_lerp_and_clamp"),
    (n::SUB_FLOAT, "float_arithmetic_lerp_and_clamp"),
    (n::MUL_FLOAT, "float_arithmetic_lerp_and_clamp"),
    (n::DIV_FLOAT, "float_arithmetic_lerp_and_clamp"),
    (n::LERP_FLOAT, "float_arithmetic_lerp_and_clamp"),
    (n::CLAMP_FLOAT, "float_arithmetic_lerp_and_clamp"),
    (n::CLAMP_INT, "float_arithmetic_lerp_and_clamp"),
    (n::RANDOM_FLOAT, "random_nodes_are_seeded_and_volatile"),
    (n::RANDOM_INT, "random_nodes_are_seeded_and_volatile"),
    // data
    (n::MAKE_VEC3, "vec3_make_and_break_round_trip"),
    (n::BREAK_VEC3, "vec3_make_and_break_round_trip"),
    (n::INT_TO_FLOAT, "numeric_and_text_conversions"),
    (n::FLOAT_TO_INT, "numeric_and_text_conversions"),
    (n::INT_TO_STRING, "numeric_and_text_conversions"),
    (n::FLOAT_TO_STRING, "numeric_and_text_conversions"),
    (n::BOOL_TO_STRING, "numeric_and_text_conversions"),
    // effects
    (n::PRINT, "sequence_runs_each_output_in_order_skipping_gaps"),
    (n::EMIT_EVENT, "emit_event_queues_on_this_instance_for_the_next_tick"),
    (n::SPAWN_PREFAB, "spawn_hands_back_an_alias_the_runner_still_has_to_bind"),
    (n::DESTROY_ENTITY, "spawn_hands_back_an_alias_the_runner_still_has_to_bind"),
    (n::GET_SELF, "transform_get_set_and_the_self_entity_default"),
    (n::GET_POSITION, "transform_get_set_and_the_self_entity_default"),
    (n::SET_POSITION, "transform_get_set_and_the_self_entity_default"),
    (n::GET_TRANSFORM, "transform_get_set_and_the_self_entity_default"),
    (n::SET_TRANSFORM, "transform_get_set_and_the_self_entity_default"),
];

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

fn edge(from: u64, fp: &str, to: u64, tp: &str) -> Edge {
    Edge {
        from_node: from,
        from_pin: fp.to_string(),
        to_node: to,
        to_pin: tp.to_string(),
    }
}

fn run_world(doc: &GraphDoc, ticks: usize, seed: u64, world: &dyn WorldRead) -> Vec<Effect> {
    let reg = registry();
    let subs: BTreeMap<String, GraphDoc> = BTreeMap::new();
    let plan = compile(doc, "test.graph", &reg, &subs)
        .unwrap_or_else(|e| panic!("compile: {e}"));
    let impls = n::std_impls();
    assert_eq!(impls.check_plan(&plan), vec![], "plan/impl cross-check");
    let mut inst = GraphInstance::new(&plan, EntityRef::SelfEntity, seed);
    let mut effects = Vec::new();
    for i in 0..ticks {
        tick(
            &plan,
            &mut inst,
            &impls,
            TickInput { dt: 0.1, time: i as f64 * 0.1 },
            world,
            &mut effects,
        );
    }
    effects
}

fn run(doc: &GraphDoc, ticks: usize) -> Vec<Effect> {
    run_world(doc, ticks, 1, &NoWorld)
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

/// The one-statement pure-node harness: BeginPlay → Print, with the node
/// under test feeding Print's `text` through the matching `*_to_string`.
///
/// `wire` names the node's output pin; `converter` is the node id that turns
/// it into text. Returns what got printed.
fn eval_pure(
    node_id: &str,
    props: &[(&str, PropValue)],
    out_pin: &str,
    converter: &str,
) -> String {
    let mut doc = GraphDoc::default();
    doc.nodes = vec![
        node(0, EVENT_BEGIN_PLAY_TYPE_ID),
        node(1, n::PRINT),
        with(2, node_id, props),
        node(3, converter),
    ];
    doc.edges = vec![
        edge(0, EXEC_OUT_PIN, 1, EXEC_IN_PIN),
        edge(2, out_pin, 3, "value"),
        edge(3, "text", 1, "text"),
    ];
    let out = logs(&run(&doc, 1));
    assert_eq!(out.len(), 1, "{node_id}: expected one line, got {out:?}");
    out.into_iter().next().unwrap()
}

fn eval_int(node_id: &str, props: &[(&str, PropValue)]) -> String {
    eval_pure(node_id, props, "result", n::INT_TO_STRING)
}

fn eval_float(node_id: &str, props: &[(&str, PropValue)]) -> String {
    eval_pure(node_id, props, "result", n::FLOAT_TO_STRING)
}

fn eval_bool(node_id: &str, props: &[(&str, PropValue)]) -> String {
    eval_pure(node_id, props, "result", n::BOOL_TO_STRING)
}

/// A statement driven by `event_begin_play` through the listed exec chain.
fn chain(nodes: Vec<NodeInst>, edges: Vec<Edge>) -> GraphDoc {
    GraphDoc { nodes, edges, ..GraphDoc::default() }
}

// ---------------------------------------------------------------------------
// Control flow
// ---------------------------------------------------------------------------

/// Sequence runs each connected output to completion, in order, and skips the
/// gaps — which is what lets one node cover D5's "Sequence(2–4)".
#[test]
fn sequence_runs_each_output_in_order_skipping_gaps() {
    let mut doc = chain(
        vec![
            node(0, EVENT_BEGIN_PLAY_TYPE_ID),
            node(1, n::SEQUENCE),
            with(2, n::PRINT, &[("text", PropValue::Str("first".into()))]),
            with(3, n::PRINT, &[("text", PropValue::Str("second".into()))]),
            with(4, n::PRINT, &[("text", PropValue::Str("fourth".into()))]),
        ],
        vec![
            edge(0, EXEC_OUT_PIN, 1, EXEC_IN_PIN),
            edge(1, "then_0", 2, EXEC_IN_PIN),
            edge(1, "then_1", 3, EXEC_IN_PIN),
            // then_2 deliberately unwired.
            edge(1, "then_3", 4, EXEC_IN_PIN),
        ],
    );
    assert_eq!(logs(&run(&doc, 1)), vec!["first", "second", "fourth"]);

    // Each branch runs *to completion* before the next starts.
    doc.nodes.push(with(5, n::PRINT, &[("text", PropValue::Str("first-tail".into()))]));
    doc.edges.push(edge(2, EXEC_OUT_PIN, 5, EXEC_IN_PIN));
    assert_eq!(
        logs(&run(&doc, 1)),
        vec!["first", "first-tail", "second", "fourth"]
    );
}

/// While loops re-check their condition every pass — which works only because
/// the condition is a data input, re-pulled on each firing.
#[test]
fn while_loop_rechecks_its_condition_each_pass() {
    let mut doc = GraphDoc::default();
    doc.variables = vec![VarDecl {
        slug: "n".into(),
        label: "N".into(),
        ty: PinType::Int,
        default: Some(PropValue::Int(0)),
    }];
    let var = |id: u64, ty: &str| with(id, ty, &[(VAR_PROP, PropValue::Str("n".into()))]);
    doc.nodes = vec![
        node(0, EVENT_BEGIN_PLAY_TYPE_ID),
        node(1, n::WHILE_LOOP),
        var(2, VAR_GET_TYPE_ID),
        with(3, n::COMPARE_INT, &[("b", PropValue::Int(3)), ("op", PropValue::Enum("less".into()))]),
        with(4, n::ADD_INT, &[("b", PropValue::Int(1))]),
        var(5, VAR_SET_TYPE_ID),
        node(6, n::INT_TO_STRING),
        with(7, n::PRINT, &[]),
        with(8, n::PRINT, &[("text", PropValue::Str("done".into()))]),
    ];
    doc.edges = vec![
        edge(0, EXEC_OUT_PIN, 1, EXEC_IN_PIN),
        edge(2, VAR_VALUE_PIN, 3, "a"),
        edge(3, "result", 1, "condition"),
        edge(1, "body", 5, EXEC_IN_PIN),
        edge(2, VAR_VALUE_PIN, 4, "a"),
        edge(4, "result", 5, VAR_VALUE_PIN),
        edge(5, EXEC_OUT_PIN, 7, EXEC_IN_PIN),
        edge(2, VAR_VALUE_PIN, 6, "value"),
        edge(6, "text", 7, "text"),
        edge(1, "completed", 8, EXEC_IN_PIN),
    ];
    assert_eq!(
        logs(&run(&doc, 1)),
        vec!["1", "2", "3", "done"],
        "the loop stops the pass after the variable reaches 3"
    );
}

/// ForEach walks an array, publishing element and index, then completes. The
/// array arrives from a variable, which is how arrays flow in v1 (D2: no
/// literal editor).
#[test]
fn for_each_walks_an_array_with_element_and_index() {
    let mut doc = GraphDoc::default();
    doc.variables = vec![VarDecl {
        slug: "xs".into(),
        label: "Xs".into(),
        ty: PinType::Array(Box::new(PinType::Int)),
        default: Some(PropValue::Array(vec![
            PropValue::Int(10),
            PropValue::Int(20),
            PropValue::Int(30),
        ])),
    }];
    doc.nodes = vec![
        node(0, EVENT_BEGIN_PLAY_TYPE_ID),
        node(1, n::FOR_EACH_INT),
        with(2, VAR_GET_TYPE_ID, &[(VAR_PROP, PropValue::Str("xs".into()))]),
        with(3, n::ADD_INT, &[]),
        node(4, n::INT_TO_STRING),
        with(5, n::PRINT, &[]),
        with(6, n::PRINT, &[("text", PropValue::Str("end".into()))]),
    ];
    doc.edges = vec![
        edge(0, EXEC_OUT_PIN, 1, EXEC_IN_PIN),
        edge(2, VAR_VALUE_PIN, 1, "array"),
        edge(1, "body", 5, EXEC_IN_PIN),
        // element + index, so both outputs are proven.
        edge(1, "element", 3, "a"),
        edge(1, "index", 3, "b"),
        edge(3, "result", 4, "value"),
        edge(4, "text", 5, "text"),
        edge(1, "completed", 6, EXEC_IN_PIN),
    ];
    assert_eq!(logs(&run(&doc, 1)), vec!["10", "21", "32", "end"]);

    // An empty array runs the body zero times and still completes.
    doc.variables[0].default = Some(PropValue::Array(vec![]));
    assert_eq!(logs(&run(&doc, 1)), vec!["end"]);

    // The other two element types compile and run — one implementation, three
    // descriptors.
    for (id, ty, default) in [
        (n::FOR_EACH_FLOAT, PinType::Float, PropValue::Float(1.5)),
        (n::FOR_EACH_ENTITY, PinType::Entity, PropValue::Raw("x".into())),
    ] {
        let mut d = GraphDoc::default();
        d.variables = vec![VarDecl {
            slug: "xs".into(),
            label: "Xs".into(),
            ty: PinType::Array(Box::new(ty)),
            default: Some(PropValue::Array(vec![default])),
        }];
        d.nodes = vec![
            node(0, EVENT_BEGIN_PLAY_TYPE_ID),
            node(1, id),
            with(2, VAR_GET_TYPE_ID, &[(VAR_PROP, PropValue::Str("xs".into()))]),
            with(3, n::PRINT, &[("text", PropValue::Str("hit".into()))]),
        ];
        d.edges = vec![
            edge(0, EXEC_OUT_PIN, 1, EXEC_IN_PIN),
            edge(2, VAR_VALUE_PIN, 1, "array"),
            edge(1, "body", 3, EXEC_IN_PIN),
        ];
        assert_eq!(logs(&run(&d, 1)), vec!["hit"], "{id}");
    }
}

/// Gate is the multi-entrance node: which exec input arrived decides what it
/// does, and its open/closed state survives across ticks.
#[test]
fn gate_opens_closes_and_toggles_by_entrance() {
    // Tick → Gate.enter → Print. A custom event opens it.
    let mut doc = GraphDoc::default();
    doc.nodes = vec![
        node(0, EVENT_TICK_TYPE_ID),
        with(1, n::GATE, &[("start_closed", PropValue::Bool(true))]),
        with(2, n::PRINT, &[("text", PropValue::Str("through".into()))]),
        with(3, EVENT_CUSTOM_TYPE_ID, &[(EVENT_NAME_PROP, PropValue::Str("open".into()))]),
        with(4, EVENT_CUSTOM_TYPE_ID, &[(EVENT_NAME_PROP, PropValue::Str("shut".into()))]),
    ];
    doc.edges = vec![
        edge(0, EXEC_OUT_PIN, 1, "enter"),
        edge(1, "exit", 2, EXEC_IN_PIN),
        edge(3, EXEC_OUT_PIN, 1, "open"),
        edge(4, EXEC_OUT_PIN, 1, "close"),
    ];

    let reg = registry();
    let plan = compile(&doc, "test.graph", &reg, &BTreeMap::new()).unwrap();
    let impls = n::std_impls();
    let mut inst = GraphInstance::new(&plan, EntityRef::SelfEntity, 1);
    let mut fx = Vec::new();
    let t = TickInput { dt: 0.1, time: 0.0 };

    tick(&plan, &mut inst, &impls, t, &NoWorld, &mut fx);
    assert_eq!(logs(&fx), Vec::<String>::new(), "starts closed");

    fx.clear();
    inst.queue_event(node_graph_types::EventPhase::Custom, Some("open".into()), BTreeMap::new());
    tick(&plan, &mut inst, &impls, t, &NoWorld, &mut fx);
    assert_eq!(logs(&fx), vec!["through"], "opened, and Enter passes");

    fx.clear();
    tick(&plan, &mut inst, &impls, t, &NoWorld, &mut fx);
    assert_eq!(logs(&fx), vec!["through"], "the open state persists across ticks");

    fx.clear();
    inst.queue_event(node_graph_types::EventPhase::Custom, Some("shut".into()), BTreeMap::new());
    tick(&plan, &mut inst, &impls, t, &NoWorld, &mut fx);
    assert_eq!(logs(&fx), Vec::<String>::new(), "closed again");
}

/// DoOnce passes the first entry and nothing after, until Reset.
#[test]
fn do_once_passes_once_then_needs_a_reset() {
    let mut doc = GraphDoc::default();
    doc.nodes = vec![
        node(0, EVENT_TICK_TYPE_ID),
        node(1, n::DO_ONCE),
        with(2, n::PRINT, &[("text", PropValue::Str("once".into()))]),
        with(3, EVENT_CUSTOM_TYPE_ID, &[(EVENT_NAME_PROP, PropValue::Str("again".into()))]),
    ];
    doc.edges = vec![
        edge(0, EXEC_OUT_PIN, 1, "enter"),
        edge(1, "completed", 2, EXEC_IN_PIN),
        edge(3, EXEC_OUT_PIN, 1, "reset"),
    ];

    let reg = registry();
    let plan = compile(&doc, "test.graph", &reg, &BTreeMap::new()).unwrap();
    let impls = n::std_impls();
    let mut inst = GraphInstance::new(&plan, EntityRef::SelfEntity, 1);
    let mut fx = Vec::new();
    let t = TickInput { dt: 0.1, time: 0.0 };

    tick(&plan, &mut inst, &impls, t, &NoWorld, &mut fx);
    tick(&plan, &mut inst, &impls, t, &NoWorld, &mut fx);
    tick(&plan, &mut inst, &impls, t, &NoWorld, &mut fx);
    assert_eq!(logs(&fx), vec!["once"], "three ticks, one pass");

    fx.clear();
    inst.queue_event(node_graph_types::EventPhase::Custom, Some("again".into()), BTreeMap::new());
    tick(&plan, &mut inst, &impls, t, &NoWorld, &mut fx);
    assert_eq!(logs(&fx), vec!["once"], "reset re-arms it — in the same tick");
}

/// FlipFlop alternates, and says which side it took.
#[test]
fn flip_flop_alternates_and_reports_its_side() {
    let mut doc = GraphDoc::default();
    doc.nodes = vec![
        node(0, EVENT_TICK_TYPE_ID),
        node(1, n::FLIP_FLOP),
        with(2, n::PRINT, &[]),
        with(3, n::PRINT, &[("text", PropValue::Str("B".into()))]),
        node(4, n::BOOL_TO_STRING),
    ];
    doc.edges = vec![
        edge(0, EXEC_OUT_PIN, 1, EXEC_IN_PIN),
        edge(1, "a", 2, EXEC_IN_PIN),
        edge(1, "b", 3, EXEC_IN_PIN),
        edge(1, "is_a", 4, "value"),
        edge(4, "text", 2, "text"),
    ];
    assert_eq!(
        logs(&run(&doc, 4)),
        vec!["true", "B", "true", "B"],
        "A first, then B, alternating — and `is_a` agrees"
    );
}

/// Select picks a value without any exec flow — it is pure.
#[test]
fn select_picks_a_value_per_type() {
    assert_eq!(
        eval_int(n::SELECT_INT, &[
            ("condition", PropValue::Bool(true)),
            ("if_true", PropValue::Int(7)),
            ("if_false", PropValue::Int(9)),
        ]),
        "7"
    );
    assert_eq!(
        eval_int(n::SELECT_INT, &[
            ("condition", PropValue::Bool(false)),
            ("if_true", PropValue::Int(7)),
            ("if_false", PropValue::Int(9)),
        ]),
        "9"
    );
    assert_eq!(
        eval_float(n::SELECT_FLOAT, &[
            ("condition", PropValue::Bool(true)),
            ("if_true", PropValue::Float(1.5)),
            ("if_false", PropValue::Float(2.5)),
        ]),
        "1.5"
    );
    assert_eq!(
        eval_bool(n::SELECT_BOOL, &[
            ("condition", PropValue::Bool(false)),
            ("if_true", PropValue::Bool(true)),
            ("if_false", PropValue::Bool(false)),
        ]),
        "false"
    );
    // The String variant needs no converter — it already prints.
    let doc = chain(
        vec![
            node(0, EVENT_BEGIN_PLAY_TYPE_ID),
            with(1, n::PRINT, &[]),
            with(2, n::SELECT_STRING, &[
                ("condition", PropValue::Bool(true)),
                ("if_true", PropValue::Str("yes".into())),
                ("if_false", PropValue::Str("no".into())),
            ]),
        ],
        vec![edge(0, EXEC_OUT_PIN, 1, EXEC_IN_PIN), edge(2, "result", 1, "text")],
    );
    assert_eq!(logs(&run(&doc, 1)), vec!["yes"]);
}

// ---------------------------------------------------------------------------
// Logic and math
// ---------------------------------------------------------------------------

#[test]
fn boolean_logic() {
    for (a, b, and, or) in [
        (false, false, "false", "false"),
        (false, true, "false", "true"),
        (true, false, "false", "true"),
        (true, true, "true", "true"),
    ] {
        let props = [("a", PropValue::Bool(a)), ("b", PropValue::Bool(b))];
        assert_eq!(eval_bool(n::AND, &props), and, "{a} and {b}");
        assert_eq!(eval_bool(n::OR, &props), or, "{a} or {b}");
    }
    assert_eq!(eval_bool(n::NOT, &[("a", PropValue::Bool(true))]), "false");
    assert_eq!(eval_bool(n::NOT, &[("a", PropValue::Bool(false))]), "true");
}

/// One comparison node per type, with the operator as a declared-variant
/// enum — the granularity decision, exercised across all six operators.
#[test]
fn comparisons_cover_every_declared_operator() {
    let cases = [
        ("equal", false),
        ("not_equal", true),
        ("less", true),
        ("less_equal", true),
        ("greater", false),
        ("greater_equal", false),
    ];
    for (op, want) in cases {
        assert_eq!(
            eval_bool(n::COMPARE_INT, &[
                ("a", PropValue::Int(2)),
                ("b", PropValue::Int(5)),
                ("op", PropValue::Enum(op.into())),
            ]),
            want.to_string(),
            "int {op}"
        );
        assert_eq!(
            eval_bool(n::COMPARE_FLOAT, &[
                ("a", PropValue::Float(2.0)),
                ("b", PropValue::Float(5.0)),
                ("op", PropValue::Enum(op.into())),
            ]),
            want.to_string(),
            "float {op}"
        );
    }
    // Equal values, to pin the boundary operators.
    for (op, want) in [("equal", true), ("less_equal", true), ("greater_equal", true), ("less", false)] {
        assert_eq!(
            eval_bool(n::COMPARE_INT, &[
                ("a", PropValue::Int(4)),
                ("b", PropValue::Int(4)),
                ("op", PropValue::Enum(op.into())),
            ]),
            want.to_string(),
            "int {op} on equal values"
        );
    }
    // A stale operator slug falls back to the descriptor's default rather
    // than failing the graph — an out-of-list enum is data to fix, not a
    // broken document (the Task 40 ruling).
    assert_eq!(
        eval_bool(n::COMPARE_INT, &[
            ("a", PropValue::Int(4)),
            ("b", PropValue::Int(4)),
            ("op", PropValue::Enum("approximately".into())),
        ]),
        "true"
    );
}

#[test]
fn integer_arithmetic() {
    let p = |a: i32, b: i32| [("a", PropValue::Int(a)), ("b", PropValue::Int(b))];
    assert_eq!(eval_int(n::ADD_INT, &p(2, 3)), "5");
    assert_eq!(eval_int(n::SUB_INT, &p(2, 3)), "-1");
    assert_eq!(eval_int(n::MUL_INT, &p(4, 3)), "12");
    assert_eq!(eval_int(n::DIV_INT, &p(7, 2)), "3");
    // A graph is user input: neither of these may panic.
    assert_eq!(eval_int(n::DIV_INT, &p(7, 0)), "0", "divide by zero yields 0");
    assert_eq!(eval_int(n::ADD_INT, &p(i32::MAX, 1)), i32::MIN.to_string(), "wrapping");
}

#[test]
fn float_arithmetic_lerp_and_clamp() {
    let p = |a: f32, b: f32| [("a", PropValue::Float(a)), ("b", PropValue::Float(b))];
    assert_eq!(eval_float(n::ADD_FLOAT, &p(2.5, 3.0)), "5.5");
    assert_eq!(eval_float(n::SUB_FLOAT, &p(2.5, 3.0)), "-0.5");
    assert_eq!(eval_float(n::MUL_FLOAT, &p(2.5, 4.0)), "10");
    assert_eq!(eval_float(n::DIV_FLOAT, &p(5.0, 2.0)), "2.5");
    assert_eq!(
        eval_float(n::DIV_FLOAT, &p(5.0, 0.0)),
        "0",
        "0 rather than an infinity: a NaN loose in a graph poisons silently"
    );

    let lerp = |t: f32| {
        eval_float(n::LERP_FLOAT, &[
            ("a", PropValue::Float(10.0)),
            ("b", PropValue::Float(20.0)),
            ("alpha", PropValue::Float(t)),
        ])
    };
    assert_eq!(lerp(0.0), "10");
    assert_eq!(lerp(0.5), "15");
    assert_eq!(lerp(1.0), "20");
    assert_eq!(lerp(2.0), "20", "alpha is clamped");
    assert_eq!(lerp(-1.0), "10");

    let clamp_f = |v: f32, lo: f32, hi: f32| {
        eval_float(n::CLAMP_FLOAT, &[
            ("value", PropValue::Float(v)),
            ("min", PropValue::Float(lo)),
            ("max", PropValue::Float(hi)),
        ])
    };
    assert_eq!(clamp_f(5.0, 0.0, 1.0), "1");
    assert_eq!(clamp_f(-5.0, 0.0, 1.0), "0");
    assert_eq!(clamp_f(0.5, 0.0, 1.0), "0.5");
    assert_eq!(clamp_f(0.5, 10.0, 0.0), "10", "inverted bounds do not panic");

    let clamp_i = |v: i32, lo: i32, hi: i32| {
        eval_int(n::CLAMP_INT, &[
            ("value", PropValue::Int(v)),
            ("min", PropValue::Int(lo)),
            ("max", PropValue::Int(hi)),
        ])
    };
    assert_eq!(clamp_i(50, 0, 10), "10");
    assert_eq!(clamp_i(-50, 0, 10), "0");
    assert_eq!(clamp_i(5, 0, 10), "5");
}

/// Random nodes draw from the *instance's* seeded generator: same seed, same
/// sequence; different seeds, different sequences. And because they are
/// volatile, two pulls in one statement genuinely draw twice.
#[test]
fn random_nodes_are_seeded_and_volatile() {
    let mut doc = GraphDoc::default();
    doc.nodes = vec![
        node(0, EVENT_TICK_TYPE_ID),
        with(1, n::PRINT, &[]),
        with(2, n::RANDOM_INT, &[("min", PropValue::Int(1)), ("max", PropValue::Int(1000))]),
        node(3, n::INT_TO_STRING),
    ];
    doc.edges = vec![
        edge(0, EXEC_OUT_PIN, 1, EXEC_IN_PIN),
        edge(2, "value", 3, "value"),
        edge(3, "text", 1, "text"),
    ];

    let a = logs(&run_world(&doc, 4, 42, &NoWorld));
    let b = logs(&run_world(&doc, 4, 42, &NoWorld));
    let c = logs(&run_world(&doc, 4, 99, &NoWorld));
    assert_eq!(a, b, "same seed, same sequence");
    assert_ne!(a, c, "a different seed is a different sequence");
    assert!(
        a.windows(2).any(|w| w[0] != w[1]),
        "successive ticks draw new values, not one frozen one: {a:?}"
    );
    for v in &a {
        let v: i32 = v.parse().unwrap();
        assert!((1..=1000).contains(&v), "{v} out of range");
    }

    // Volatility: two independent pulls of the *same* Random node inside one
    // statement must differ. A memoized (deterministic) node would return the
    // same number twice.
    let mut doc = GraphDoc::default();
    doc.nodes = vec![
        node(0, EVENT_BEGIN_PLAY_TYPE_ID),
        with(1, n::PRINT, &[]),
        with(2, n::RANDOM_INT, &[("min", PropValue::Int(1)), ("max", PropValue::Int(1_000_000))]),
        with(3, n::SUB_INT, &[]),
        node(4, n::INT_TO_STRING),
    ];
    doc.edges = vec![
        edge(0, EXEC_OUT_PIN, 1, EXEC_IN_PIN),
        edge(2, "value", 3, "a"),
        edge(2, "value", 3, "b"),
        edge(3, "result", 4, "value"),
        edge(4, "text", 1, "text"),
    ];
    assert_ne!(
        logs(&run(&doc, 1)),
        vec!["0"],
        "a volatile node re-draws per pull; memoizing it would give a - a = 0"
    );

    // RandomFloat stays inside its range.
    let mut doc = GraphDoc::default();
    doc.nodes = vec![
        node(0, EVENT_TICK_TYPE_ID),
        with(1, n::PRINT, &[]),
        with(2, n::RANDOM_FLOAT, &[("min", PropValue::Float(-2.0)), ("max", PropValue::Float(2.0))]),
        node(3, n::FLOAT_TO_STRING),
    ];
    doc.edges = vec![
        edge(0, EXEC_OUT_PIN, 1, EXEC_IN_PIN),
        edge(2, "value", 3, "value"),
        edge(3, "text", 1, "text"),
    ];
    for v in logs(&run_world(&doc, 8, 7, &NoWorld)) {
        let v: f32 = v.parse().unwrap();
        assert!((-2.0..=2.0).contains(&v), "{v} out of range");
    }
}

// ---------------------------------------------------------------------------
// Data
// ---------------------------------------------------------------------------

#[test]
fn vec3_make_and_break_round_trip() {
    // Make → Break → print Y. Z-up: X forward, Y right, Z up.
    let doc = chain(
        vec![
            node(0, EVENT_BEGIN_PLAY_TYPE_ID),
            with(1, n::PRINT, &[]),
            with(2, n::MAKE_VEC3, &[
                ("x", PropValue::Float(1.0)),
                ("y", PropValue::Float(2.0)),
                ("z", PropValue::Float(3.0)),
            ]),
            node(3, n::BREAK_VEC3),
            node(4, n::FLOAT_TO_STRING),
        ],
        vec![
            edge(0, EXEC_OUT_PIN, 1, EXEC_IN_PIN),
            edge(2, "result", 3, "value"),
            edge(3, "y", 4, "value"),
            edge(4, "text", 1, "text"),
        ],
    );
    assert_eq!(logs(&run(&doc, 1)), vec!["2"]);
}

#[test]
fn numeric_and_text_conversions() {
    assert_eq!(eval_float(n::INT_TO_FLOAT, &[("value", PropValue::Int(-3))]), "-3");
    assert_eq!(eval_int(n::FLOAT_TO_INT, &[("value", PropValue::Float(2.9))]), "2");
    assert_eq!(
        eval_int(n::FLOAT_TO_INT, &[("value", PropValue::Float(-2.9))]),
        "-2",
        "truncation is toward zero"
    );
    assert_eq!(eval_int(n::FLOAT_TO_INT, &[("value", PropValue::Float(f32::NAN))]), "0");

    // The three ToString variants, each through its own pin type.
    let text = |id: &str, v: PropValue| {
        let doc = chain(
            vec![
                node(0, EVENT_BEGIN_PLAY_TYPE_ID),
                with(1, n::PRINT, &[]),
                with(2, id, &[("value", v)]),
            ],
            vec![edge(0, EXEC_OUT_PIN, 1, EXEC_IN_PIN), edge(2, "text", 1, "text")],
        );
        logs(&run(&doc, 1)).remove(0)
    };
    assert_eq!(text(n::INT_TO_STRING, PropValue::Int(-12)), "-12");
    assert_eq!(text(n::FLOAT_TO_STRING, PropValue::Float(1.25)), "1.25");
    assert_eq!(text(n::BOOL_TO_STRING, PropValue::Bool(true)), "true");
}

// ---------------------------------------------------------------------------
// Effects
// ---------------------------------------------------------------------------

/// EmitEvent queues on this instance for the *next* tick, and records itself
/// in the stream so the console and the P7 trace see it.
#[test]
fn emit_event_queues_on_this_instance_for_the_next_tick() {
    let doc = chain(
        vec![
            node(0, EVENT_BEGIN_PLAY_TYPE_ID),
            with(1, n::EMIT_EVENT, &[("name", PropValue::Str("ping".into()))]),
            with(2, EVENT_CUSTOM_TYPE_ID, &[(EVENT_NAME_PROP, PropValue::Str("ping".into()))]),
            with(3, n::PRINT, &[("text", PropValue::Str("heard".into()))]),
        ],
        vec![
            edge(0, EXEC_OUT_PIN, 1, EXEC_IN_PIN),
            edge(2, EXEC_OUT_PIN, 3, EXEC_IN_PIN),
        ],
    );

    let reg = registry();
    let plan = compile(&doc, "test.graph", &reg, &BTreeMap::new()).unwrap();
    let impls = n::std_impls();
    let mut inst = GraphInstance::new(&plan, EntityRef::SelfEntity, 1);
    let mut fx = Vec::new();
    let t = TickInput { dt: 0.1, time: 0.0 };

    tick(&plan, &mut inst, &impls, t, &NoWorld, &mut fx);
    assert_eq!(
        fx,
        vec![Effect::EmitEvent { name: "ping".into(), payload: vec![] }],
        "emitted, and NOT delivered in the same tick"
    );

    fx.clear();
    tick(&plan, &mut inst, &impls, t, &NoWorld, &mut fx);
    assert_eq!(logs(&fx), vec!["heard"], "delivered on the next tick");
}

/// The spawn alias protocol: the handle exists immediately, the effect names
/// it, and the instance lists it as pending until the runner binds it.
#[test]
fn spawn_hands_back_an_alias_the_runner_still_has_to_bind() {
    let doc = chain(
        vec![
            node(0, EVENT_BEGIN_PLAY_TYPE_ID),
            with(1, n::SPAWN_PREFAB, &[
                ("path", PropValue::Str("prefabs/crate.prefab".into())),
                ("position", PropValue::Vec3([1.0, 2.0, 3.0])),
            ]),
            // …and the handle is usable in the very next statement.
            node(2, n::DESTROY_ENTITY),
        ],
        vec![
            edge(0, EXEC_OUT_PIN, 1, EXEC_IN_PIN),
            edge(1, EXEC_OUT_PIN, 2, EXEC_IN_PIN),
            edge(1, "spawned", 2, "entity"),
        ],
    );

    let reg = registry();
    let plan = compile(&doc, "test.graph", &reg, &BTreeMap::new()).unwrap();
    let impls = n::std_impls();
    let mut inst = GraphInstance::new(&plan, EntityRef::SelfEntity, 1);
    let mut fx = Vec::new();
    tick(&plan, &mut inst, &impls, TickInput::default(), &NoWorld, &mut fx);

    assert_eq!(
        fx,
        vec![
            Effect::SpawnPrefab {
                path: "prefabs/crate.prefab".into(),
                alias: 0,
                transform: TransformSnapshot { position: [1.0, 2.0, 3.0], ..Default::default() },
            },
            Effect::DestroyEntity { entity: EntityRef::Spawned(0) },
        ],
        "the alias flows from the spawn into the next node, with no real id in sight"
    );
    assert_eq!(
        inst.take_pending_aliases(),
        vec![0],
        "the runner is told which aliases it still owes an entity"
    );
    assert!(inst.take_pending_aliases().is_empty(), "draining is the handshake");
}

/// Transform reads and writes, including the `entity` pin's "unwired means
/// self" rule and the fine-grained effect split.
#[test]
fn transform_get_set_and_the_self_entity_default() {
    struct At(TransformSnapshot);
    impl WorldRead for At {
        fn transform(&self, e: EntityRef) -> Option<TransformSnapshot> {
            (e == EntityRef::SelfEntity).then_some(self.0)
        }
        fn exists(&self, e: EntityRef) -> bool {
            e == EntityRef::SelfEntity
        }
    }
    let world = At(TransformSnapshot {
        position: [1.0, 2.0, 3.0],
        rotation: [0.0, 0.0, 0.7, 0.7],
        scale: [2.0, 2.0, 2.0],
    });

    // get_transform → set_transform, entity pins left unwired (= self).
    let doc = chain(
        vec![
            node(0, EVENT_BEGIN_PLAY_TYPE_ID),
            node(1, n::GET_TRANSFORM),
            node(2, n::SET_TRANSFORM),
        ],
        vec![
            edge(0, EXEC_OUT_PIN, 2, EXEC_IN_PIN),
            edge(1, "position", 2, "position"),
            edge(1, "rotation", 2, "rotation"),
            edge(1, "scale", 2, "scale"),
        ],
    );
    assert_eq!(
        run_world(&doc, 1, 1, &world),
        vec![
            Effect::SetPosition { entity: EntityRef::SelfEntity, position: [1.0, 2.0, 3.0] },
            Effect::SetRotation { entity: EntityRef::SelfEntity, rotation: [0.0, 0.0, 0.7, 0.7] },
            Effect::SetScale { entity: EntityRef::SelfEntity, scale: [2.0, 2.0, 2.0] },
        ],
        "three fine-grained effects, read straight back out"
    );

    // get_position / set_position, and get_self proving the entity pin is a
    // real pin rather than an implicit.
    let doc = chain(
        vec![
            node(0, EVENT_BEGIN_PLAY_TYPE_ID),
            node(1, n::GET_POSITION),
            node(2, n::SET_POSITION),
            node(3, n::GET_SELF),
        ],
        vec![
            edge(0, EXEC_OUT_PIN, 2, EXEC_IN_PIN),
            edge(1, "position", 2, "position"),
            edge(3, "self", 2, "entity"),
            edge(3, "self", 1, "entity"),
        ],
    );
    assert_eq!(
        run_world(&doc, 1, 1, &world),
        vec![Effect::SetPosition {
            entity: EntityRef::SelfEntity,
            position: [1.0, 2.0, 3.0],
        }]
    );

    // With no world at all, a read is the type's zero rather than a failure —
    // a graph that runs before its entity exists must not die.
    assert_eq!(
        run(&doc, 1),
        vec![Effect::SetPosition { entity: EntityRef::SelfEntity, position: [0.0; 3] }]
    );
}

// ---------------------------------------------------------------------------
// Coverage
// ---------------------------------------------------------------------------

/// Branch and ForLoop, so this file stands alone rather than leaning on the
/// walking-skeleton suite for two of the most-used nodes.
#[test]
fn branch_and_for_loop_basics() {
    let mut doc = chain(
        vec![
            node(0, EVENT_BEGIN_PLAY_TYPE_ID),
            with(1, n::BRANCH, &[("condition", PropValue::Bool(true))]),
            with(2, n::FOR_LOOP, &[
                ("first", PropValue::Int(2)),
                ("last", PropValue::Int(4)),
            ]),
            with(3, n::PRINT, &[]),
            node(4, n::INT_TO_STRING),
            with(5, n::PRINT, &[("text", PropValue::Str("skipped".into()))]),
        ],
        vec![
            edge(0, EXEC_OUT_PIN, 1, EXEC_IN_PIN),
            edge(1, "true", 2, EXEC_IN_PIN),
            edge(1, "false", 5, EXEC_IN_PIN),
            edge(2, "body", 3, EXEC_IN_PIN),
            edge(2, "index", 4, "value"),
            edge(4, "text", 3, "text"),
        ],
    );
    assert_eq!(logs(&run(&doc, 1)), vec!["2", "3", "4"]);

    doc.nodes[1]
        .properties
        .insert("condition".into(), PropValue::Bool(false));
    assert_eq!(logs(&run(&doc, 1)), vec!["skipped"], "the loop is not entered");
}

/// Every registered node type has an implementation, and every one of them is
/// named in [`TESTED`]. Adding a node without a semantic test fails here.
#[test]
fn every_std_node_is_covered() {
    use std::collections::BTreeSet;

    let reg = registry();
    let impls = n::std_impls();
    let declared: Vec<String> = node_graph_types::std_node_descriptors()
        .into_iter()
        .map(|d| d.id)
        .collect();

    // Descriptor -> implementation.
    let missing_impl: Vec<&String> = declared.iter().filter(|id| !impls.contains(id)).collect();
    assert_eq!(missing_impl, Vec::<&String>::new(), "descriptors without an implementation");
    for id in &declared {
        assert!(reg.get(id).is_some(), "{id} did not register");
    }

    // The reserved doc-dependent types and every event entry are implemented
    // too — they have no descriptor in `std_nodes`, so nothing above sees them.
    for id in [VAR_GET_TYPE_ID, VAR_SET_TYPE_ID] {
        assert!(impls.contains(id), "{id}");
    }
    for d in node_graph_types::std_event_descriptors() {
        assert!(impls.contains(&d.id), "{}", d.id);
    }

    // Descriptor <-> test manifest, both directions.
    let declared: BTreeSet<&str> = declared.iter().map(|s| s.as_str()).collect();
    let tested: BTreeSet<&str> = TESTED.iter().map(|(id, _)| *id).collect();
    assert_eq!(
        declared.difference(&tested).collect::<Vec<_>>(),
        Vec::<&&str>::new(),
        "std nodes with no semantic test"
    );
    assert_eq!(
        tested.difference(&declared).collect::<Vec<_>>(),
        Vec::<&&str>::new(),
        "the manifest names a node that no longer exists"
    );
    assert_eq!(TESTED.len(), tested.len(), "the manifest lists a node twice");
}
