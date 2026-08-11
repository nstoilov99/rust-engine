//! 45-A P7: the trace seam and the plan's reverse mapping.
//!
//! Two properties are load-bearing for the editor's execution visualization
//! and are pinned here rather than in the engine, because they belong to the
//! portable core: **every plan node names the document node an editor should
//! light up**, and **the interpreter reports the edges it takes and the values
//! it pulls, without changing what it does**.

#![allow(clippy::field_reassign_with_default)]

use std::collections::BTreeMap;

use node_graph_exec::nodes::{ADD_INT, FOR_LOOP, INT_TO_STRING, PRINT};
use node_graph_exec::{
    compile, nodes, tick, tick_traced, Effect, EntityRef, GraphInstance, NoWorld, NodeImpls, Plan,
    TickInput, TraceSink, Value, DEFAULT_BUDGET,
};
use node_graph_types::{
    register_std_events, Edge, GraphDoc, IfacePin, NodeInst, NodeRegistry, PinType, PropValue,
    EVENT_BEGIN_PLAY_TYPE_ID, EXEC_IN_PIN, EXEC_OUT_PIN, GRAPH_INPUT_TYPE_ID, GRAPH_OUTPUT_TYPE_ID,
    SUBGRAPH_TYPE_ID,
};

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn registry() -> NodeRegistry {
    let mut reg = NodeRegistry::new();
    node_graph_types::register_std_nodes(&mut reg).unwrap();
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

fn print(id: u64, text: &str) -> NodeInst {
    with(id, PRINT, &[("text", PropValue::Str(text.to_string()))])
}

/// BeginPlay -> ForLoop(1..2) -> Print(IntToString(index)).
fn loop_doc() -> GraphDoc {
    let mut doc = GraphDoc::default();
    doc.nodes = vec![
        node(0, EVENT_BEGIN_PLAY_TYPE_ID),
        with(
            1,
            FOR_LOOP,
            &[("first", PropValue::Int(1)), ("last", PropValue::Int(2))],
        ),
        node(2, INT_TO_STRING),
        print(3, "x"),
    ];
    doc.edges = vec![
        edge(0, EXEC_OUT_PIN, 1, EXEC_IN_PIN),
        edge(1, "body", 3, EXEC_IN_PIN),
        edge(1, "index", 2, "value"),
        edge(2, "text", 3, "text"),
    ];
    doc
}

/// A host document with one subgraph node (id 1) whose body is three real
/// nodes, so the reverse mapping has something to collapse.
fn host_and_sub() -> (GraphDoc, BTreeMap<String, GraphDoc>) {
    let mut sub = GraphDoc::default();
    sub.inputs = vec![
        IfacePin { slug: "run".into(), label: "Run".into(), ty: PinType::Exec },
        IfacePin { slug: "n".into(), label: "N".into(), ty: PinType::Int },
    ];
    sub.outputs = vec![IfacePin {
        slug: "done".into(),
        label: "Done".into(),
        ty: PinType::Exec,
    }];
    sub.nodes = vec![
        node(0, GRAPH_INPUT_TYPE_ID),
        node(1, GRAPH_OUTPUT_TYPE_ID),
        with(2, ADD_INT, &[("b", PropValue::Int(10))]),
        node(3, INT_TO_STRING),
        print(4, "inside"),
    ];
    sub.edges = vec![
        edge(0, "run", 4, EXEC_IN_PIN),
        edge(4, EXEC_OUT_PIN, 1, "done"),
        edge(0, "n", 2, "a"),
        edge(2, "result", 3, "value"),
        edge(3, "text", 4, "text"),
    ];

    let mut host = GraphDoc::default();
    let mut sub_node = node(1, SUBGRAPH_TYPE_ID);
    sub_node.subgraph = Some("lib/add10.subgraph".into());
    sub_node.properties.insert("n".into(), PropValue::Int(5));
    host.nodes = vec![
        node(0, EVENT_BEGIN_PLAY_TYPE_ID),
        sub_node,
        print(2, "after"),
    ];
    host.edges = vec![
        edge(0, EXEC_OUT_PIN, 1, "run"),
        edge(1, "done", 2, EXEC_IN_PIN),
    ];

    let mut subs = BTreeMap::new();
    subs.insert("lib/add10.subgraph".to_string(), sub);
    (host, subs)
}

/// A recorder that keeps everything, so a test can assert on order as well as
/// content. The engine's real one is a ring buffer; ring semantics are its
/// own business and are tested there.
#[derive(Default)]
struct Recorder {
    exec: Vec<(usize, String, usize)>,
    data: Vec<(usize, String, Value)>,
}

impl TraceSink for Recorder {
    fn exec_edge(&mut self, from: usize, pin: &str, to: usize) {
        self.exec.push((from, pin.to_string(), to));
    }

    fn data_value(&mut self, from: usize, pin: &str, value: &Value) {
        self.data.push((from, pin.to_string(), value.clone()));
    }
}

fn run_traced(doc: &GraphDoc, subs: &BTreeMap<String, GraphDoc>, ticks: usize) -> (Plan, Recorder) {
    let reg = registry();
    let plan = compile(doc, "test.graph", &reg, subs).expect("compile");
    let impls = impls();
    let mut inst = GraphInstance::new(&plan, EntityRef::SelfEntity, 7);
    let mut effects: Vec<Effect> = Vec::new();
    let mut rec = Recorder::default();
    for i in 0..ticks {
        let t = TickInput { dt: 0.1, time: i as f64 * 0.1 };
        tick_traced(
            &plan,
            &mut inst,
            &impls,
            t,
            &NoWorld,
            &mut effects,
            DEFAULT_BUDGET,
            &mut rec,
        );
    }
    (plan, rec)
}

/// Plan index -> the type id it came from, for readable assertions.
fn type_of(plan: &Plan, ix: usize) -> &str {
    &plan.nodes[ix].type_id
}

// ---------------------------------------------------------------------------
// Reverse mapping
// ---------------------------------------------------------------------------

/// A node authored in the document maps to itself, and says it was not
/// inlined.
#[test]
fn plan_nodes_map_back_to_their_document_ids() {
    let doc = loop_doc();
    let reg = registry();
    let plan = compile(&doc, "test.graph", &reg, &BTreeMap::new()).expect("compile");

    let mut seen: Vec<(u64, &str)> = plan
        .nodes
        .iter()
        .map(|n| (n.doc_node, n.type_id.as_str()))
        .collect();
    seen.sort();
    assert_eq!(
        seen,
        vec![
            (0, EVENT_BEGIN_PLAY_TYPE_ID),
            (1, FOR_LOOP),
            (2, INT_TO_STRING),
            (3, PRINT),
        ],
        "every plan node names the doc node it came from"
    );
    assert!(
        plan.nodes.iter().all(|n| !n.inlined),
        "nothing in a flat document is inlined"
    );

    // The accessor agrees with the field, and an index past the end reads as
    // unknown rather than panicking (a trace can outlive its plan).
    assert_eq!(plan.doc_node(0), Some((plan.nodes[0].doc_node, false)));
    assert_eq!(plan.doc_node(plan.nodes.len()), None);
}

/// Everything inlined out of a subgraph reports the **host** subgraph node —
/// the one node the document the editor is showing actually has.
#[test]
fn inlined_subgraph_nodes_map_to_their_host() {
    let (host, subs) = host_and_sub();
    let reg = registry();
    let plan = compile(&host, "test.graph", &reg, &subs).expect("compile");

    for (i, n) in plan.nodes.iter().enumerate() {
        let from_sub = matches!(type_of(&plan, i), ADD_INT | INT_TO_STRING)
            || (type_of(&plan, i) == PRINT && n.name.contains("add10"));
        if from_sub {
            assert!(n.inlined, "{} should report as inlined", n.name);
            assert_eq!(n.doc_node, 1, "{} should map to the host subgraph node", n.name);
        } else {
            assert!(!n.inlined, "{} is authored in the host", n.name);
        }
    }

    // The host's own nodes are untouched by inlining.
    let entry = plan.entries.first().expect("BeginPlay entry");
    assert_eq!(plan.doc_node(entry.node), Some((0, false)));
}

// ---------------------------------------------------------------------------
// Trace recording
// ---------------------------------------------------------------------------

/// The exec edges the interpreter takes are reported as (plan node, out pin,
/// plan node) at the moment it commits to them — including the loop body edge
/// once per iteration.
#[test]
fn fired_exec_edges_are_reported_in_order() {
    let (plan, rec) = run_traced(&loop_doc(), &BTreeMap::new(), 1);

    let readable: Vec<(String, &str)> = rec
        .exec
        .iter()
        .map(|(from, pin, to)| {
            (
                format!("{}.{pin}", type_of(&plan, *from)),
                type_of(&plan, *to),
            )
        })
        .collect();
    assert_eq!(
        readable,
        vec![
            (format!("{EVENT_BEGIN_PLAY_TYPE_ID}.{EXEC_OUT_PIN}"), FOR_LOOP),
            ("for_loop.body".to_string(), PRINT),
            ("for_loop.body".to_string(), PRINT),
        ],
        "entry -> loop, then the body edge once per iteration"
    );
    // `completed` is wired to nothing, so nothing is reported for it: a node
    // that fired but chose no continuation crossed no wire.
}

/// Data wires report the value that came back, keyed by the **producing** pin,
/// at pull time.
#[test]
fn pulled_data_wires_report_their_last_value() {
    let (plan, rec) = run_traced(&loop_doc(), &BTreeMap::new(), 1);

    let readable: Vec<(String, String)> = rec
        .data
        .iter()
        .map(|(from, pin, v)| (format!("{}.{pin}", type_of(&plan, *from)), v.to_string()))
        .collect();
    assert_eq!(
        readable,
        vec![
            ("for_loop.index".to_string(), "1".to_string()),
            ("int_to_string.text".to_string(), "1".to_string()),
            ("for_loop.index".to_string(), "2".to_string()),
            ("int_to_string.text".to_string(), "2".to_string()),
        ],
        "both hops of the pure chain are captured, per firing"
    );

    // Constants are not wires: Print's `text` is wired, but the ForLoop's
    // `first`/`last` are stored constants and report nothing.
    assert!(
        !rec.data.iter().any(|(_, pin, _)| pin == "first" || pin == "last"),
        "an unwired input has no wire to hover"
    );
}

/// Tracing observes; it does not participate. The effect stream is
/// byte-identical with and without a sink — the determinism contract (D8)
/// covers the editor build too.
#[test]
fn tracing_does_not_change_what_the_graph_does() {
    let doc = loop_doc();
    let reg = registry();
    let plan = compile(&doc, "test.graph", &reg, &BTreeMap::new()).expect("compile");
    let impls = impls();

    let mut plain: Vec<Effect> = Vec::new();
    let mut a = GraphInstance::new(&plan, EntityRef::SelfEntity, 7);
    let mut traced: Vec<Effect> = Vec::new();
    let mut b = GraphInstance::new(&plan, EntityRef::SelfEntity, 7);
    let mut rec = Recorder::default();
    for i in 0..3 {
        let t = TickInput { dt: 0.1, time: i as f64 * 0.1 };
        tick(&plan, &mut a, &impls, t, &NoWorld, &mut plain);
        tick_traced(
            &plan,
            &mut b,
            &impls,
            t,
            &NoWorld,
            &mut traced,
            DEFAULT_BUDGET,
            &mut rec,
        );
    }
    assert_eq!(plain, traced);
    assert_eq!(a, b, "instance state matches too, not just the effects");
    assert!(!rec.exec.is_empty(), "the sink did see something");
}
