//! The execution recorder (45-A P7) — **editor builds only**.
//!
//! ## Gating
//!
//! This module is `#[cfg(feature = "editor")]`, and the only thing that
//! writes to it is one `cfg`-selected call in the runner
//! ([`GraphScriptRunnerSystem::run`](super::runner::GraphScriptRunnerSystem)).
//! A standalone game compiles `tick(...)`, which instantiates the interpreter
//! against [`NoTrace`](node_graph_exec::NoTrace) — empty inlined hooks, no
//! branch, no buffer, no type. There is nothing to strip because nothing is
//! built: the recorder does not exist in a shipped binary, and neither does
//! the [`GraphRuntime::trace`](super::runner::GraphRuntime) field that holds
//! it.
//!
//! ## Why the trace lives on the instance
//!
//! A trace describes *one running instance*. Hanging it off `GraphRuntime` —
//! which is already the never-serialized runtime half — means it is created,
//! restarted and destroyed exactly when the instance is, with no store to
//! keep in step and no stale entry to evict. Stop play, the world snapshot
//! restores, the runtime component ceases to exist, and so does its history.
//!
//! ## Bounds
//!
//! Two, both fixed: a ring of the most recent exec firings, and a cap on how
//! many distinct data wires are remembered. A graph that fires 100k times in a
//! tick (the budget ceiling) must not grow a 100k-entry buffer behind a
//! visualization that can only show the last half second of it.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use node_graph_exec::{Plan, TraceSink, Value};

use crate::engine::editor::graph_exec_viz::GraphExecViz;

/// How many exec firings are kept. At 60Hz this is several seconds of a
/// normal graph and a fraction of a tick of a pathological one — which is the
/// right trade: the visualization only ever shows the newest [`PULSE_FADE`]
/// seconds.
pub const EXEC_RING: usize = 512;

/// How many distinct data wires keep a last value. Far above any hand-authored
/// graph's wire count; the cap exists so a generated or looping graph cannot
/// grow the map without bound.
pub const VALUE_CAP: usize = 1024;

/// Seconds a pulse takes to fade from full to nothing. Short enough that a
/// 60Hz Tick chain reads as *flowing* rather than permanently lit, long enough
/// that a once-per-second Delay chain is visibly caught.
pub const PULSE_FADE: f64 = 0.45;

/// Longest value spelling shown in a tooltip before it is elided.
const VALUE_CHARS: usize = 96;

/// One traversal of one exec edge.
#[derive(Debug, Clone, PartialEq)]
struct ExecHit {
    from: usize,
    pin: String,
    to: usize,
    /// The runner tick this happened on — monotonic, and what a future
    /// step-debugger orders by.
    tick: u64,
    /// Engine time at that tick, which is what the fade is measured against.
    time: f64,
}

/// Window the firing rate is averaged over, seconds. One second is what
/// "firings per second" means; anything shorter makes the steady-hot decision
/// jitter frame to frame.
const RATE_WINDOW: f64 = 1.0;

/// Per-instance execution history.
#[derive(Debug, Default)]
pub struct GraphTrace {
    tick: u64,
    time: f64,
    fired: VecDeque<ExecHit>,
    /// Every exec edge travelled **since the session started**, as plan
    /// indices. Bounded by the plan's edge count, not by the firing count —
    /// which is exactly why it is a separate structure from the ring above:
    /// "did this ever fire" and "what fired just now" are different questions
    /// with different bounds (GS-3).
    taken: BTreeSet<(usize, String, usize)>,
    /// Producing plan node -> output pin -> last value seen. Keyed by the
    /// producer because that is what a data wire carries; the consumers of a
    /// fanned-out output all show the same thing, correctly.
    values: BTreeMap<usize, BTreeMap<String, (Value, f64)>>,
    /// Number of distinct `(node, pin)` pairs in `values`, kept alongside so
    /// the cap check is not a nested count every pull.
    value_keys: usize,
}

impl GraphTrace {
    /// Open a new tick. The runner calls this immediately before ticking, so
    /// every hit recorded until the next call carries this stamp.
    pub fn begin_tick(&mut self, time: f64) {
        self.tick = self.tick.wrapping_add(1);
        self.time = time;
    }

    /// The tick counter, for tests and for anything that wants to know
    /// whether the instance moved.
    pub fn tick(&self) -> u64 {
        self.tick
    }

    pub fn fired_len(&self) -> usize {
        self.fired.len()
    }

    pub fn taken_len(&self) -> usize {
        self.taken.len()
    }

    /// Engine time of the newest recorded firing — "when did this instance
    /// last do anything", which is what the picker's recency column reports.
    pub fn last_activity(&self) -> Option<f64> {
        self.fired.back().map(|h| h.time)
    }

    /// Forget everything — the toolbar's "Clear trace", and what a fresh play
    /// session starts from. The taken-path tint, the pulses and the last
    /// values all go: they are one session's worth of statement.
    pub fn clear(&mut self) {
        self.fired.clear();
        self.taken.clear();
        self.values.clear();
        self.value_keys = 0;
    }

    /// Resolve the recorded history into what the canvas draws: document node
    /// ids, decayed intensities and formatted values.
    ///
    /// `now` is engine time; anything older than [`PULSE_FADE`] contributes
    /// nothing and is skipped rather than drawn at zero.
    ///
    /// **Subgraphs**: a plan node inlined out of a subgraph maps to its host
    /// subgraph node (`Plan::doc_node`), so the host lights up as a whole. Its
    /// *internal* wires are not document edges of the graph being shown and
    /// therefore never pulse — diving into the subgraph's own tab shows
    /// nothing in v1 (45.5 owns per-instance subgraph inspection).
    pub fn viz(&self, plan: &Plan, now: f64, instance: &str) -> GraphExecViz {
        let mut out = GraphExecViz::new(instance);
        // The taken path first: session-lifetime, in document space. An edge
        // whose ends inline out of a subgraph is not an edge of this document
        // and is dropped rather than lighting the host's wires.
        for (from, pin, to) in &self.taken {
            let (Some((a, false)), Some((b, _))) = (plan.doc_node(*from), plan.doc_node(*to))
            else {
                continue;
            };
            out.mark_taken(a, pin, b);
        }
        // Firing rate over the last second, per producing pin — the input to
        // the steady-hot decision, computed here because the ring is already
        // being walked and the panel has no clock of the runtime's.
        let mut counts: BTreeMap<(u64, String), u32> = BTreeMap::new();
        for hit in &self.fired {
            if now - hit.time > RATE_WINDOW {
                continue;
            }
            if let Some((node, false)) = plan.doc_node(hit.from) {
                *counts.entry((node, hit.pin.clone())).or_default() += 1;
            }
        }
        for ((node, pin), n) in counts {
            out.set_rate(node, &pin, n as f32 / RATE_WINDOW as f32);
        }
        for hit in &self.fired {
            let age = now - hit.time;
            if !(0.0..PULSE_FADE).contains(&age) {
                // Negative age means the clock moved backwards under us (a
                // scene reload resets it): treat it as "not now" rather than
                // as a permanent full-brightness pulse.
                continue;
            }
            let Some((node, inlined)) = plan.doc_node(hit.from) else {
                continue;
            };
            let intensity = 1.0 - (age / PULSE_FADE) as f32;
            if inlined {
                // The pin name belongs to a node inside the subgraph and means
                // nothing on the host. Record it under a reserved key so the
                // host still reads as active without claiming an output pin it
                // does not have.
                out.add_pulse(node, HOST_ACTIVITY_PIN, intensity);
            } else {
                out.add_pulse(node, &hit.pin, intensity);
            }
            // The *arriving* end runs too. Recorded under the reserved key
            // rather than a pin, because entering a node is not one of its
            // outputs — without this the last node of a chain (a Set that
            // continues to nothing) never lights, which reads as "it did not
            // run" when it plainly did.
            if let Some((to, _)) = plan.doc_node(hit.to) {
                out.add_pulse(to, HOST_ACTIVITY_PIN, intensity);
            }
        }
        for (plan_node, pins) in &self.values {
            let Some((node, inlined)) = plan.doc_node(*plan_node) else {
                continue;
            };
            if inlined {
                continue; // see above: an inner pin is not a host pin
            }
            for (pin, (v, at)) in pins {
                out.set_value(node, pin, elide(&v.to_string()));
                // How long ago it crossed — the tooltip says "0.3 s ago", and
                // a value with no age reads as if it were current.
                out.set_value_age(node, pin, (now - at).max(0.0) as f32);
            }
        }
        out
    }
}

/// Reserved pin key for "something inside this subgraph host fired". Not a
/// real pin, and deliberately unspellable as one.
pub const HOST_ACTIVITY_PIN: &str = "\u{0}subgraph";


/// Fold one instance's **live** state — parked activations and a runtime kill
/// — into a viz built from its trace (GS-3).
///
/// Separate from [`GraphTrace::viz`] because it reads the *instance*, not the
/// history: a wait is not something that happened, it is something that is
/// happening, and the trace records only the former. The clock is instance
/// time, which is what a `Suspension` is measured in — engine time would be a
/// different clock and a wrong bar.
pub fn add_live_state(
    out: &mut GraphExecViz,
    instance: &node_graph_exec::GraphInstance,
    plan: &Plan,
) {
    use node_graph_exec::ThreadState;
    let now = instance.time;
    for t in &instance.threads {
        let ThreadState::Suspended(s) = &t.state else {
            continue;
        };
        // The node the activation is parked *on* is the latent node itself —
        // recorded as the resume edge's source at suspend time. Without one (a
        // suspension that unwinds) the wait belongs to wherever the cursor
        // sits, and with neither there is nothing to draw on.
        let Some(plan_node) = t.resume_edge.as_ref().map(|(n, _)| *n).or(t.cursor) else {
            continue;
        };
        let Some((node, _)) = plan.doc_node(plan_node) else {
            continue;
        };
        out.add_wait(node, s.fraction(now), s.remaining(now));
    }
    if let Some(err) = &instance.halted {
        out.killed = Some(crate::engine::editor::graph_exec_viz::KillInfo {
            node: kill_node(plan, err),
            reason: kill_reason(err),
        });
    }
}

/// Which document node an `ExecError` is about. The error carries the plan
/// node's *name*, which is what the plan indexes by — so this is a lookup, not
/// a guess, and an unmatched name answers `None` rather than blaming a node at
/// random.
fn kill_node(plan: &Plan, err: &node_graph_exec::ExecError) -> Option<u64> {
    use node_graph_exec::ExecError as E;
    let name = match err {
        E::BudgetExceeded { node, .. }
        | E::TypeMismatch { node, .. }
        | E::MissingImpl { node, .. }
        | E::UnknownVariable { node, .. }
        | E::Stopped { node, .. } => node,
    };
    plan.nodes
        .iter()
        .position(|n| n.name == *name)
        .and_then(|i| plan.doc_node(i))
        .map(|(node, _)| node)
}

/// The kill's one mono line. Short, and phrased as the consequence rather than
/// as an enum variant — it is drawn under the node, not logged.
fn kill_reason(err: &node_graph_exec::ExecError) -> String {
    use node_graph_exec::ExecError as E;
    match err {
        E::BudgetExceeded { budget, .. } => {
            format!("iteration budget exceeded \u{2014} {budget} firings in one frame")
        }
        E::TypeMismatch { pin, expected, .. } => format!("pin '{pin}' expected {expected}"),
        E::MissingImpl { type_id, .. } => format!("no implementation for '{type_id}'"),
        E::UnknownVariable { slug, .. } => format!("unknown variable '{slug}'"),
        E::Stopped { reason, .. } => reason.clone(),
    }
}
/// Which running instance a graph tab shows — **the binding rule**, in one
/// place (45-A P7).
///
/// The rule is the simplest correct one: *the entity selected in the hierarchy
/// that runs this graph*. Walk the selection in order, take the first entity
/// whose `GraphRunner` names `graph_path` and that has a live `GraphRuntime`,
/// and resolve its trace against the plan it is actually running.
///
/// Everything else answers `None`, and `None` draws nothing:
/// - not playing (no `GraphRuntime` exists outside play),
/// - nothing selected, or a selection that runs a different graph,
/// - an instance that refused to run (a realm violation or a compile error —
///   the console already said so; a dead canvas must not imply it is fine),
/// - two selected entities running the same graph: the first wins, and the
///   chip names it, so the canvas never averages two instances.
pub fn viz_for_selection(
    world: &hecs::World,
    selected: impl IntoIterator<Item = hecs::Entity>,
    graph_path: &str,
    now: f64,
) -> Option<GraphExecViz> {
    let want = super::normalize_graph_path(graph_path);
    for entity in selected {
        let runs_this = world
            .get::<&super::GraphRunner>(entity)
            .map(|r| super::normalize_graph_path(&r.graph) == want)
            .unwrap_or(false);
        if !runs_this {
            continue;
        }
        let Ok(rt) = world.get::<&super::runner::GraphRuntime>(entity) else {
            continue;
        };
        if rt.disabled.is_some() {
            continue;
        }
        let label = world
            .get::<&crate::engine::ecs::components::Name>(entity)
            .map(|n| n.0.clone())
            .unwrap_or_default();
        let mut viz = rt.trace.viz(&rt.plan, now, &label);
        viz.instance_id = entity.to_bits().get();
        add_live_state(&mut viz, &rt.instance, &rt.plan);
        return Some(viz);
    }
    None
}

/// Every instance of `graph_path` that is running right now, for the LIVE
/// chip's picker (GS-3).
///
/// Ordered nearest-camera first, which is the order a person means by "that
/// one over there". Killed instances stay in the list: a post-mortem is
/// exactly when you want to select one.
pub fn instances_for(
    world: &hecs::World,
    graph_path: &str,
    camera: Option<[f32; 3]>,
    now: f64,
) -> Vec<crate::engine::editor::graph_exec_viz::ExecInstance> {
    use crate::engine::editor::graph_exec_viz::ExecInstance;
    let want = super::normalize_graph_path(graph_path);
    let mut out: Vec<ExecInstance> = world
        .query::<(&super::GraphRunner, &super::runner::GraphRuntime)>()
        .iter()
        .filter(|(_, (r, _))| super::normalize_graph_path(&r.graph) == want)
        .map(|(entity, (_, rt))| {
            let distance = match (
                camera,
                world.get::<&crate::engine::ecs::components::Transform>(entity).ok(),
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
                last_active: rt.trace.last_activity().map(|t| (now - t).max(0.0) as f32),
                killed: rt.instance.halted.is_some() || rt.disabled.is_some(),
            }
        })
        .collect();
    out.sort_by(|a, b| a.distance.total_cmp(&b.distance).then(a.id.cmp(&b.id)));
    out
}

/// The viz for one **explicitly picked** instance (the chip's dropdown).
///
/// `None` when that entity is gone or no longer runs this graph — the picker
/// falls back to the selection rule, which is what a stale binding should
/// degrade to rather than an empty canvas that implies nothing is running.
pub fn viz_for_entity(
    world: &hecs::World,
    bits: u64,
    graph_path: &str,
    now: f64,
) -> Option<GraphExecViz> {
    let entity = hecs::Entity::from_bits(bits)?;
    let want = super::normalize_graph_path(graph_path);
    let runs_this = world
        .get::<&super::GraphRunner>(entity)
        .map(|r| super::normalize_graph_path(&r.graph) == want)
        .unwrap_or(false);
    if !runs_this {
        return None;
    }
    let rt = world.get::<&super::runner::GraphRuntime>(entity).ok()?;
    let label = world
        .get::<&crate::engine::ecs::components::Name>(entity)
        .map(|n| n.0.clone())
        .unwrap_or_default();
    let mut viz = rt.trace.viz(&rt.plan, now, &label);
    viz.instance_id = bits;
    add_live_state(&mut viz, &rt.instance, &rt.plan);
    Some(viz)
}

/// Keep a tooltip a tooltip. An array of 400 floats is a real value and a
/// useless string.
fn elide(s: &str) -> String {
    if s.chars().count() <= VALUE_CHARS {
        return s.to_string();
    }
    let head: String = s.chars().take(VALUE_CHARS).collect();
    format!("{head}…")
}

impl TraceSink for GraphTrace {
    fn exec_edge(&mut self, from: usize, pin: &str, to: usize) {
        // Session set first: it must survive the ring dropping the hit.
        if !self
            .taken
            .range((from, String::new(), 0)..)
            .take_while(|(f, _, _)| *f == from)
            .any(|(_, p, t)| p == pin && *t == to)
        {
            self.taken.insert((from, pin.to_string(), to));
        }
        if self.fired.len() == EXEC_RING {
            self.fired.pop_front();
        }
        self.fired.push_back(ExecHit {
            from,
            pin: pin.to_string(),
            to,
            tick: self.tick,
            time: self.time,
        });
    }

    fn data_value(&mut self, from: usize, pin: &str, value: &Value) {
        let now = self.time;
        match self.values.entry(from).or_default().get_mut(pin) {
            // Updating a wire already being watched is always allowed — the
            // cap bounds how many wires are remembered, not how often.
            Some(slot) => *slot = (value.clone(), now),
            None => {
                if self.value_keys >= VALUE_CAP {
                    return;
                }
                self.value_keys += 1;
                self.values
                    .entry(from)
                    .or_default()
                    .insert(pin.to_string(), (value.clone(), now));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ring keeps the newest [`EXEC_RING`] firings and drops the oldest —
    /// a runaway tick must not grow a buffer.
    #[test]
    fn the_exec_ring_is_bounded_and_keeps_the_newest() {
        let mut t = GraphTrace::default();
        t.begin_tick(0.0);
        for i in 0..(EXEC_RING + 50) {
            t.exec_edge(i, "exec_out", i + 1);
        }
        assert_eq!(t.fired_len(), EXEC_RING, "the ring never exceeds its cap");
        assert_eq!(
            t.fired.front().map(|h| h.from),
            Some(50),
            "the oldest 50 were dropped"
        );
        assert_eq!(
            t.fired.back().map(|h| h.from),
            Some(EXEC_RING + 49),
            "the newest firing is still there"
        );
    }

    /// Values are last-write-wins per wire, and the *number of wires* is
    /// capped — updates to an already-known wire always land.
    #[test]
    fn values_are_last_write_wins_and_the_wire_count_is_capped() {
        let mut t = GraphTrace::default();
        t.data_value(3, "result", &Value::Int(1));
        t.data_value(3, "result", &Value::Int(2));
        assert_eq!(t.values[&3]["result"].0, Value::Int(2));

        for i in 0..(VALUE_CAP * 2) {
            t.data_value(1000 + i, "out", &Value::Int(i as i32));
        }
        assert_eq!(t.value_keys, VALUE_CAP, "the wire count stops at the cap");
        // …and the wire that was already there still updates past the cap.
        t.data_value(3, "result", &Value::Int(9));
        assert_eq!(t.values[&3]["result"].0, Value::Int(9));
    }

    /// Ticks stamp what follows them.
    #[test]
    fn hits_carry_the_tick_they_happened_on() {
        let mut t = GraphTrace::default();
        t.begin_tick(0.5);
        t.exec_edge(0, "exec_out", 1);
        t.begin_tick(1.0);
        t.exec_edge(1, "exec_out", 2);
        assert_eq!(t.tick(), 2);
        let hits: Vec<(u64, f64)> = t.fired.iter().map(|h| (h.tick, h.time)).collect();
        assert_eq!(hits, vec![(1, 0.5), (2, 1.0)]);
    }

    fn plan_node(name: &str, doc_node: u64, inlined: bool) -> node_graph_exec::PlanNode {
        node_graph_exec::PlanNode {
            type_id: name.to_string(),
            pure: false,
            volatile: false,
            name: name.to_string(),
            doc_node,
            inlined,
            inputs: Default::default(),
            exec: Default::default(),
            variable: None,
            curve: None,
        }
    }

    /// The resolve step: plan indices become document ids, age becomes
    /// intensity, stale hits are dropped, and everything inlined out of a
    /// subgraph lands on its host instead of claiming a pin the host has not
    /// got.
    #[test]
    fn viz_maps_to_doc_space_and_fades() {
        let mut plan = Plan::default();
        // 0: authored in the document. 1: inlined out of the subgraph node 9.
        plan.nodes = vec![plan_node("print", 4, false), plan_node("add_int", 9, true)];

        let mut t = GraphTrace::default();
        t.begin_tick(10.0);
        t.exec_edge(0, "exec_out", 1);
        t.data_value(0, "text", &Value::Str("hello".into()));
        t.data_value(1, "result", &Value::Int(3));
        // A hit from a tick long gone.
        t.begin_tick(1.0);
        t.exec_edge(1, "exec_out", 0);

        let v = t.viz(&plan, 10.0, "Duck");
        assert_eq!(v.instance, "Duck");
        assert_eq!(v.pulse(4, "exec_out"), 1.0, "just fired = full brightness");
        assert_eq!(
            v.pulse(9, "exec_out"),
            0.0,
            "the stale hit is gone, and an inner pin is never a host pin"
        );
        assert_eq!(v.value(4, "text"), Some("hello"));
        assert_eq!(
            v.value(9, "result"),
            None,
            "an inlined node's data wire is not an edge of this document"
        );

        // Halfway through the fade window, halfway bright.
        let half = t.viz(&plan, 10.0 + PULSE_FADE / 2.0, "Duck");
        assert!((half.pulse(4, "exec_out") - 0.5).abs() < 1e-3, "{half:?}");
        // Past it, no pulse at all — but the last value survives, because a
        // value is the answer to "what went through here", not "what is going
        // through here right now".
        let cold = t.viz(&plan, 10.0 + PULSE_FADE, "Duck");
        assert_eq!(cold.pulse(4, "exec_out"), 0.0);
        assert_eq!(cold.value(4, "text"), Some("hello"));
    }

    /// A subgraph host lights up as a node even though none of its internal
    /// wires are document edges.
    #[test]
    fn subgraph_hosts_report_activity_without_a_pin() {
        let mut plan = Plan::default();
        plan.nodes = vec![plan_node("add_int", 9, true)];
        let mut t = GraphTrace::default();
        t.begin_tick(2.0);
        t.exec_edge(0, "exec_out", 0);

        let v = t.viz(&plan, 2.0, "Duck");
        assert!(v.node_active(9) > 0.0, "the host node reads as running");
        assert_eq!(v.pulse(9, "exec_out"), 0.0, "…but claims no pin of its own");
    }

    /// A chain's *last* node ran too. Without crediting the arriving end, a
    /// `Set` that continues to nothing never lights, which reads as "it did
    /// not run".
    #[test]
    fn the_arriving_end_of_an_edge_lights_as_well() {
        let mut plan = Plan::default();
        plan.nodes = vec![plan_node("event_tick", 6, false), plan_node("var_set", 14, false)];
        let mut t = GraphTrace::default();
        t.begin_tick(1.0);
        t.exec_edge(0, "exec_out", 1);

        let v = t.viz(&plan, 1.0, "Duck");
        assert_eq!(v.pulse(6, "exec_out"), 1.0, "the wire out of the source pulses");
        assert!(v.node_active(14) > 0.0, "and the node it reached is running");
        assert_eq!(
            v.pulse(14, "exec_out"),
            0.0,
            "arriving is not one of the target's outputs"
        );
    }

    /// A plan that changed under a live trace (hot reload recompiles) reads as
    /// unknown rather than panicking or lighting the wrong node.
    #[test]
    fn hits_past_the_end_of_the_plan_are_dropped() {
        let mut plan = Plan::default();
        plan.nodes = vec![plan_node("print", 4, false)];
        let mut t = GraphTrace::default();
        t.begin_tick(0.0);
        t.exec_edge(7, "exec_out", 9);
        t.data_value(7, "result", &Value::Int(1));
        assert!(t.viz(&plan, 0.0, "Duck").is_empty());
    }

    /// **GS-3: the taken path is a session, not a window.** The pulse ring is
    /// bounded at 512 firings because it is about recency; the fired set is
    /// about *ever*, so an edge stays taken long after its hit fell out of the
    /// ring — and "Clear trace" is what ends the session, not time.
    #[test]
    fn the_taken_set_outlives_the_ring_and_clears_on_demand() {
        let mut plan = Plan::default();
        plan.nodes = vec![plan_node("event_tick", 6, false), plan_node("print", 4, false)];
        let mut t = GraphTrace::default();
        t.begin_tick(1.0);
        t.exec_edge(0, "exec_out", 1);
        // Flood the ring past its cap with other edges.
        for _ in 0..(EXEC_RING + 10) {
            t.exec_edge(1, "exec_out", 0);
        }
        assert_eq!(t.fired_len(), EXEC_RING);
        assert_eq!(t.taken_len(), 2, "two distinct edges, however often they fired");

        let v = t.viz(&plan, 1.0, "Duck");
        assert!(v.is_taken(6, "exec_out", 4), "the first edge is still taken");
        assert!(v.has_session());
        // An edge nobody travelled is not taken — which is what dims.
        assert!(!v.is_taken(4, "other", 6));

        t.clear();
        assert_eq!(t.taken_len(), 0);
        assert_eq!(t.fired_len(), 0);
        assert!(!t.viz(&plan, 1.0, "Duck").has_session(), "clear ends the session");
    }

    /// The firing rate is what decides when a wire stops strobing. Averaged
    /// over the last second, so it is a rate rather than a frame count.
    #[test]
    fn the_firing_rate_drives_the_steady_hot_cap() {
        let mut plan = Plan::default();
        plan.nodes = vec![plan_node("event_tick", 6, false), plan_node("print", 4, false)];
        let mut t = GraphTrace::default();
        // 30 firings inside the window, plus one long past it.
        t.begin_tick(0.0);
        t.exec_edge(0, "exec_out", 1);
        for i in 0..30 {
            t.begin_tick(10.0 + i as f64 * 0.01);
            t.exec_edge(0, "exec_out", 1);
        }
        let v = t.viz(&plan, 10.3, "Duck");
        assert_eq!(v.rate(6, "exec_out"), 30.0, "the stale firing does not count");
        assert!(v.steady_hot(6, "exec_out"), "30 Hz is past the cap");

        let mut slow = GraphTrace::default();
        for i in 0..5 {
            slow.begin_tick(1.0 + i as f64 * 0.1);
            slow.exec_edge(0, "exec_out", 1);
        }
        let v = slow.viz(&plan, 1.5, "Duck");
        assert_eq!(v.rate(6, "exec_out"), 5.0);
        assert!(!v.steady_hot(6, "exec_out"), "5 Hz still animates");
    }

    /// Waiting is read off the *instance*, not the history: one entry per
    /// node, the bar tracking the nearest-due activation and the count naming
    /// the rest.
    #[test]
    fn parked_activations_become_one_wait_per_node() {
        use node_graph_exec::{Activation, ActivationId, GraphInstance, Suspension, ThreadState};
        let mut plan = Plan::default();
        plan.nodes = vec![plan_node("delay", 12, false), plan_node("timeline", 13, false)];

        let park = |id: u64, node: usize, since: f64, until: f64| Activation {
            id: ActivationId(id),
            entry: 0,
            cursor: Some(node),
            entered: None,
            frames: Vec::new(),
            locals: Default::default(),
            payload: Default::default(),
            resume_edge: Some((node, "exec_out".to_string())),
            state: ThreadState::Suspended(Suspension {
                until,
                resume: Some("exec_out".to_string()),
                since,
            }),
        };
        let mut instance =
            GraphInstance::new(&plan, node_graph_exec::EntityRef::SelfEntity, 1);
        instance.time = 4.0;
        instance.threads = vec![
            // Two on the Delay: 40% through a 5s wait, and one nearly due.
            park(1, 0, 2.0, 7.0),
            park(2, 0, 3.5, 4.5),
            park(3, 1, 0.0, 8.0),
        ];

        let mut viz = GraphExecViz::new("Duck");
        add_live_state(&mut viz, &instance, &plan);
        assert_eq!(viz.waiting_len(), 2, "one entry per node, never per activation");
        let delay = viz.wait(12).unwrap();
        assert_eq!(delay.count, 2);
        assert!((delay.remaining - 0.5).abs() < 1e-3, "the bar tracks the nearest due");
        assert!((delay.fraction - 0.5).abs() < 1e-3, "…and its own elapsed fraction");
        let timeline = viz.wait(13).unwrap();
        assert_eq!(timeline.count, 1);
        assert!((timeline.fraction - 0.5).abs() < 1e-3);
        assert!(viz.wait(99).is_none());
        assert!(viz.has_session(), "a wait is a live session even before anything fires");
    }

    /// A runtime kill anchors to the node that raised it, with the reason
    /// phrased for a reader rather than as an enum variant.
    #[test]
    fn a_kill_anchors_to_its_node_and_says_why() {
        use node_graph_exec::{ExecError, GraphInstance};
        let mut plan = Plan::default();
        plan.nodes = vec![plan_node("event_tick", 6, false), plan_node("for_loop", 21, false)];
        let mut instance =
            GraphInstance::new(&plan, node_graph_exec::EntityRef::SelfEntity, 1);
        instance.halted = Some(ExecError::BudgetExceeded {
            node: "for_loop".into(),
            budget: 12_000,
        });

        let mut viz = GraphExecViz::new("Duck");
        add_live_state(&mut viz, &instance, &plan);
        let kill = viz.killed.as_ref().expect("the canvas knows it died");
        assert_eq!(kill.node, Some(21), "anchored to the offending document node");
        assert!(kill.reason.contains("12000") || kill.reason.contains("12,000"));
        assert!(kill.reason.contains("budget"));

        // An error naming a node the plan does not have anchors nowhere rather
        // than blaming one at random — the reason still reaches the console.
        instance.halted = Some(ExecError::Stopped {
            node: "ghost".into(),
            reason: "stopped by the author".into(),
        });
        let mut viz = GraphExecViz::new("Duck");
        add_live_state(&mut viz, &instance, &plan);
        assert_eq!(viz.killed.as_ref().unwrap().node, None);
        assert_eq!(viz.killed.unwrap().reason, "stopped by the author");
    }

    #[test]
    fn long_values_are_elided() {
        let long = "x".repeat(VALUE_CHARS + 20);
        let out = elide(&long);
        assert_eq!(out.chars().count(), VALUE_CHARS + 1);
        assert!(out.ends_with('…'));
        assert_eq!(elide("short"), "short");
    }
}
