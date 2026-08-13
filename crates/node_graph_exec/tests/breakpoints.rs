//! Breakpoints, PAUSED and the debugger's three commands (GS-4).
//!
//! The claims under test are the ones the design makes, in the order it makes
//! them:
//!
//! 1. a pause parks **before** the node fires — its effect must not appear;
//! 2. while one activation is paused the **whole instance** holds: siblings do
//!    not advance, due latents do not wake, queued events do not drain, and
//!    instance time does not move;
//! 3. Resume continues from the exact parked cursor and re-arms afterwards;
//! 4. Step is exactly one firing for the instance, then a re-park;
//! 5. an instance serialized mid-pause resumes correctly after a round trip;
//! 6. Stop ends the session with no error framing;
//! 7. a held tick spends no budget.

#![allow(clippy::field_reassign_with_default)]

use std::collections::BTreeMap;

use node_graph_exec::nodes as n;
use node_graph_exec::{
    compile, tick_debug, BreakSet, DebugCommand, DebugCtl, Effect, EntityRef, GraphInstance,
    NoTrace, NoWorld, NodeImpls, Plan, ThreadState, TickInput, TickReport, DEFAULT_BUDGET,
};
use node_graph_types::{
    register_std_events, register_std_nodes, Edge, GraphDoc, NodeInst, NodeRegistry, PropValue,
    EVENT_BEGIN_PLAY_TYPE_ID, EVENT_TICK_TYPE_ID, EXEC_IN_PIN, EXEC_OUT_PIN,
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
    Edge { from_node: from, from_pin: fp.to_string(), to_node: to, to_pin: tp.to_string() }
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

/// A runner in miniature, with the debug channel wired up.
struct Runner {
    plan: Plan,
    impls: NodeImpls,
    instance: GraphInstance,
    breaks: BreakSet,
    time: f64,
    dt: f32,
}

impl Runner {
    fn new(doc: &GraphDoc, dt: f32) -> Self {
        let plan = compile(doc, "test.graph", &registry(), &BTreeMap::new())
            .unwrap_or_else(|e| panic!("compile: {e}"));
        let impls = n::std_impls();
        assert_eq!(impls.check_plan(&plan), vec![], "plan/impl cross-check");
        let instance = GraphInstance::new(&plan, EntityRef::SelfEntity, 1);
        Self { plan, impls, instance, breaks: BreakSet::new(), time: 0.0, dt }
    }

    /// The plan index of a document node — what a breakpoint is, once the
    /// runner has resolved it. Impure only: a pure node never fires.
    fn plan_ix(&self, doc_id: u64) -> usize {
        self.plan
            .nodes
            .iter()
            .position(|n| n.doc_node == doc_id && !n.inlined && !n.pure)
            .unwrap_or_else(|| panic!("no impure plan node for document node {doc_id}"))
    }

    fn arm(&mut self, doc_id: u64) {
        let ix = self.plan_ix(doc_id);
        self.breaks.insert(ix);
    }

    fn run(&mut self, command: DebugCommand) -> (TickReport, Vec<String>) {
        let mut fx = Vec::new();
        self.time += self.dt as f64;
        let report = tick_debug(
            &self.plan,
            &mut self.instance,
            &self.impls,
            TickInput { dt: self.dt, time: self.time },
            &NoWorld,
            &mut fx,
            DEFAULT_BUDGET,
            &mut NoTrace,
            DebugCtl { breaks: Some(&self.breaks), command },
        );
        (report, logs(&fx))
    }

    fn tick(&mut self) -> Vec<String> {
        self.run(DebugCommand::Run).1
    }

    fn resume(&mut self) -> Vec<String> {
        self.run(DebugCommand::Resume).1
    }

    fn step(&mut self) -> Vec<String> {
        self.run(DebugCommand::Step).1
    }

    fn paused_on(&self) -> Option<usize> {
        self.instance.paused().map(|(n, _)| n)
    }

    fn suspended(&self) -> usize {
        self.instance
            .threads
            .iter()
            .filter(|t| matches!(t.state, ThreadState::Suspended(_)))
            .count()
    }
}

/// BeginPlay → a → b → c, so "the breakpoint is on b" has an unambiguous
/// before and after.
fn chain_doc() -> GraphDoc {
    let mut doc = GraphDoc::default();
    doc.nodes = vec![
        node(0, EVENT_BEGIN_PLAY_TYPE_ID),
        print(1, "a"),
        print(2, "b"),
        print(3, "c"),
    ];
    doc.edges = vec![
        edge(0, EXEC_OUT_PIN, 1, EXEC_IN_PIN),
        edge(1, EXEC_OUT_PIN, 2, EXEC_IN_PIN),
        edge(2, EXEC_OUT_PIN, 3, EXEC_IN_PIN),
    ];
    doc
}

// ---------------------------------------------------------------------------
// 1. Pause happens before the effect
// ---------------------------------------------------------------------------

/// The whole promise of "pause **before** firing": at the moment the banner
/// appears, the node's effect has not happened. A debugger that showed you the
/// consequence of the statement it claims not to have run yet would be lying
/// about the one thing it exists to report.
#[test]
fn a_breakpoint_parks_before_the_node_fires() {
    let mut r = Runner::new(&chain_doc(), 0.1);
    r.arm(2);

    let (report, out) = r.run(DebugCommand::Run);
    assert_eq!(out, vec!["a"], "everything up to the breakpoint ran");
    assert!(!out.contains(&"b".to_string()), "the parked node's effect did not");
    assert_eq!(r.paused_on(), Some(r.plan_ix(2)), "parked on b");
    assert_eq!(report.paused.len(), 1, "the tick reports the hit");
    assert_eq!(report.paused[0].0, r.plan_ix(2));

    // Held: ticking again changes nothing at all, and reports no *new* hit.
    let (held, out) = r.run(DebugCommand::Run);
    assert!(out.is_empty(), "a held instance emits nothing");
    assert!(held.paused.is_empty(), "the hit is the transition, not the state");
    assert_eq!(r.paused_on(), Some(r.plan_ix(2)));
}

// ---------------------------------------------------------------------------
// 2. The hold is instance-wide
// ---------------------------------------------------------------------------

/// One graph is one timeline of effects: a second activation, a latent that
/// has come due and a queued event are **all** frozen by one paused
/// activation. Letting the siblings run would produce an ordering no unpaused
/// run could ever produce.
#[test]
fn a_paused_activation_holds_the_whole_instance() {
    let mut doc = GraphDoc::default();
    doc.nodes = vec![
        // The chain that will park.
        node(0, EVENT_BEGIN_PLAY_TYPE_ID),
        print(1, "a"),
        print(2, "b"),
        // A latent that is due while the instance is parked.
        node(4, EVENT_BEGIN_PLAY_TYPE_ID),
        delay(5, 0.15),
        print(6, "latent"),
        // A Tick chain: a fresh activation every tick.
        node(7, EVENT_TICK_TYPE_ID),
        print(8, "tick"),
    ];
    doc.edges = vec![
        edge(0, EXEC_OUT_PIN, 1, EXEC_IN_PIN),
        edge(1, EXEC_OUT_PIN, 2, EXEC_IN_PIN),
        edge(4, EXEC_OUT_PIN, 5, EXEC_IN_PIN),
        edge(5, EXEC_OUT_PIN, 6, EXEC_IN_PIN),
        edge(7, EXEC_OUT_PIN, 8, EXEC_IN_PIN),
    ];

    let mut r = Runner::new(&doc, 0.1);
    r.arm(2);

    // t=0.1: BeginPlay runs both chains and Tick runs once. One parks on b,
    // one suspends until t=0.25.
    let out = r.tick();
    assert!(out.contains(&"a".to_string()));
    assert!(out.contains(&"tick".to_string()));
    assert_eq!(r.suspended(), 1, "the Delay is parked");
    assert_eq!(r.paused_on(), Some(r.plan_ix(2)));
    let frozen_time = r.instance.time;
    let threads = r.instance.threads.len();

    // Three held ticks: the wait would be long due, Tick would have started
    // three more activations, and neither happened.
    for _ in 0..3 {
        assert!(r.tick().is_empty(), "nothing runs while the instance holds");
    }
    assert_eq!(r.instance.threads.len(), threads, "no activation started");
    assert_eq!(r.suspended(), 1, "the latent did not wake");
    assert_eq!(r.instance.time, frozen_time, "instance time is frozen too");
    assert_eq!(
        r.instance.queue.len(),
        0,
        "and the queue was never drained into a working set it could not use"
    );

    // Resume: b fires, and the rest of the timeline picks up from there —
    // the latent is now due against a clock that only moved when it ran.
    let out = r.resume();
    assert_eq!(out.first().map(String::as_str), Some("b"), "the parked node fires first");
    assert!(r.paused_on().is_none());
    let mut latent_seen = false;
    for _ in 0..4 {
        if r.tick().contains(&"latent".to_string()) {
            latent_seen = true;
            break;
        }
    }
    assert!(latent_seen, "the wait resumes on the timeline it was measured against");
}

// ---------------------------------------------------------------------------
// 3. Resume continues from the exact cursor
// ---------------------------------------------------------------------------

/// Resume fires the parked node and runs on. The breakpoint is not consumed:
/// the *next* time control reaches that node it parks again, which is what
/// makes a breakpoint in a loop useful.
#[test]
fn resume_continues_from_the_parked_cursor_and_re_arms() {
    let mut doc = chain_doc();
    // A second, slower route to the same node, so control reaches b twice in
    // one session without both arrivals landing in the same tick.
    doc.nodes.push(node(9, EVENT_BEGIN_PLAY_TYPE_ID));
    doc.nodes.push(delay(10, 0.15));
    doc.edges.push(edge(9, EXEC_OUT_PIN, 10, EXEC_IN_PIN));
    doc.edges.push(edge(10, EXEC_OUT_PIN, 2, EXEC_IN_PIN));

    let mut r = Runner::new(&doc, 0.1);
    r.arm(2);

    assert_eq!(r.tick(), vec!["a"], "parks on b before firing it");
    assert_eq!(r.resume(), vec!["b", "c"], "exactly the continuation it parked with");
    assert!(r.paused_on().is_none());

    // The waiting activation comes due and arrives at b, which is still armed
    // — a breakpoint is not consumed by being hit, which is what makes one
    // inside a loop useful.
    let mut re_armed = false;
    for _ in 0..4 {
        r.tick();
        if r.paused_on() == Some(r.plan_ix(2)) {
            re_armed = true;
            break;
        }
    }
    assert!(re_armed, "the mark caught the second arrival too");
    assert_eq!(r.resume(), vec!["b", "c"]);
}

// ---------------------------------------------------------------------------
// 4. Step is exactly one firing
// ---------------------------------------------------------------------------

/// One press, one node. Not one per activation and not "until the next
/// breakpoint": the seam is the interpreter's own firing loop.
#[test]
fn step_fires_exactly_one_node_then_re_parks() {
    let mut r = Runner::new(&chain_doc(), 0.1);
    r.arm(2);

    assert_eq!(r.tick(), vec!["a"]);
    assert_eq!(r.step(), vec!["b"], "one firing");
    assert_eq!(r.paused_on(), Some(r.plan_ix(3)), "re-parked on the next node, unfired");
    assert_eq!(r.step(), vec!["c"], "and the next press fires that one");
    assert!(r.paused_on().is_none(), "the activation finished — nothing to park on");
    assert!(r.tick().is_empty());
}

/// Two parked activations step **together**, and the instance still advances
/// exactly one firing: the step budget belongs to the timeline, not to a
/// thread. The activation that did not get the firing keeps its pass, so the
/// next press moves it rather than re-parking it on the spot.
#[test]
fn step_spends_one_firing_for_the_whole_instance() {
    let mut doc = GraphDoc::default();
    doc.nodes = vec![
        node(0, EVENT_BEGIN_PLAY_TYPE_ID),
        print(1, "one"),
        node(2, EVENT_TICK_TYPE_ID),
        print(3, "two"),
    ];
    doc.edges = vec![
        edge(0, EXEC_OUT_PIN, 1, EXEC_IN_PIN),
        edge(2, EXEC_OUT_PIN, 3, EXEC_IN_PIN),
    ];
    let mut r = Runner::new(&doc, 0.1);
    r.arm(1);
    r.arm(3);

    assert!(r.tick().is_empty(), "both activations park before their print");
    assert_eq!(
        r.instance
            .threads
            .iter()
            .filter(|t| matches!(t.state, ThreadState::Paused { .. }))
            .count(),
        2
    );
    let first = r.step();
    assert_eq!(first.len(), 1, "one firing for the instance: {first:?}");
    let second = r.step();
    assert_eq!(second.len(), 1, "and the sibling moves on the next press");
    assert_ne!(first, second, "the two activations, not the same one twice");
}

// ---------------------------------------------------------------------------
// 5. Serialization mid-pause
// ---------------------------------------------------------------------------

/// Paused is plain data like every other thread state, so an instance parked
/// on a breakpoint survives a round trip and resumes at the same cursor. This
/// is the P4 requirement applied to the debugger.
#[test]
fn an_instance_serializes_mid_pause_and_resumes() {
    let mut r = Runner::new(&chain_doc(), 0.1);
    r.arm(2);
    assert_eq!(r.tick(), vec!["a"]);

    let text = ron::ser::to_string(&r.instance).expect("serialize mid-pause");
    assert!(text.contains("Paused"), "the state is in the file:\n{text}");
    let back: GraphInstance = ron::from_str(&text).expect("deserialize mid-pause");
    assert_eq!(back, r.instance);

    r.instance = back;
    assert_eq!(r.paused_on(), Some(r.plan_ix(2)));
    assert_eq!(r.resume(), vec!["b", "c"], "resumes exactly where it parked");
}

// ---------------------------------------------------------------------------
// 6. Stop
// ---------------------------------------------------------------------------

/// Stop ends the session, and says nothing about errors while doing it: no
/// `halted`, no reason, no ⊗ badge. Nothing went wrong — the debugging did.
#[test]
fn stop_ends_the_session_without_error_framing() {
    let mut doc = chain_doc();
    // A long wait, so Stop has a suspended activation to end as well as the
    // parked one.
    doc.nodes.push(node(9, EVENT_BEGIN_PLAY_TYPE_ID));
    doc.nodes.push(delay(10, 5.0));
    doc.nodes.push(print(11, "never"));
    doc.edges.push(edge(9, EXEC_OUT_PIN, 10, EXEC_IN_PIN));
    doc.edges.push(edge(10, EXEC_OUT_PIN, 11, EXEC_IN_PIN));

    let mut r = Runner::new(&doc, 0.1);
    r.arm(2);
    assert_eq!(r.tick(), vec!["a"]);
    assert_eq!(r.suspended(), 1, "and something is waiting when Stop lands");

    let (report, out) = r.run(DebugCommand::Stop);
    assert!(out.is_empty(), "stopping emits nothing");
    assert!(report.halted.is_none(), "and is not a kill");
    assert!(r.instance.halted.is_none(), "no error is invented for it");
    assert!(r.instance.stopped, "the session is over");
    assert!(r.paused_on().is_none(), "nothing is parked any more");
    assert!(
        r.instance
            .threads
            .iter()
            .all(|t| matches!(t.state, ThreadState::Finished)),
        "every activation ended, including the one that was merely waiting"
    );

    // Dead quiet afterwards: no wait comes due, no entry starts.
    for _ in 0..3 {
        assert!(r.tick().is_empty());
        assert!(r.resume().is_empty(), "not even a resume revives it");
    }
}

// ---------------------------------------------------------------------------
// 7. Budget
// ---------------------------------------------------------------------------

/// A held tick spends nothing: no firing, no pure evaluation, no step. A
/// paused graph must be able to sit there for ten minutes without the budget
/// noticing.
#[test]
fn a_held_instance_spends_no_budget() {
    let mut r = Runner::new(&chain_doc(), 0.1);
    r.arm(2);
    let (first, _) = r.run(DebugCommand::Run);
    assert!(first.steps > 0, "the tick that reached the breakpoint did work");

    for _ in 0..5 {
        let (held, _) = r.run(DebugCommand::Run);
        assert_eq!(held.steps, 0, "a held tick spends no budget");
        assert_eq!(held.activations, 0);
        assert_eq!(held.resumed, 0);
    }

    // And the park itself is free: the node it parked before was never
    // charged, so resuming pays for it exactly once.
    let (resumed, out) = r.run(DebugCommand::Resume);
    assert_eq!(out, vec!["b", "c"]);
    assert_eq!(resumed.steps, 2, "two firings, two charges");
}

/// An unarmed control is the interpreter as it was: no breakpoints, no
/// commands, and nothing in the report. This is the standalone path.
#[test]
fn an_unarmed_tick_behaves_exactly_like_the_shipped_one() {
    let doc = chain_doc();
    let mut r = Runner::new(&doc, 0.1);
    r.breaks.clear();
    let mut fx = Vec::new();
    let report = tick_debug(
        &r.plan,
        &mut r.instance,
        &r.impls,
        TickInput { dt: 0.1, time: 0.1 },
        &NoWorld,
        &mut fx,
        DEFAULT_BUDGET,
        &mut NoTrace,
        DebugCtl::default(),
    );
    assert_eq!(logs(&fx), vec!["a", "b", "c"]);
    assert!(report.paused.is_empty());
    assert!(!r.instance.is_paused());
}
