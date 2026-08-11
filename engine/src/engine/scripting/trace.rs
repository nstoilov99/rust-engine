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

use std::collections::{BTreeMap, VecDeque};

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

/// Per-instance execution history.
#[derive(Debug, Default)]
pub struct GraphTrace {
    tick: u64,
    time: f64,
    fired: VecDeque<ExecHit>,
    /// Producing plan node -> output pin -> last value seen. Keyed by the
    /// producer because that is what a data wire carries; the consumers of a
    /// fanned-out output all show the same thing, correctly.
    values: BTreeMap<usize, BTreeMap<String, Value>>,
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
            for (pin, v) in pins {
                out.set_value(node, pin, elide(&v.to_string()));
            }
        }
        out
    }
}

/// Reserved pin key for "something inside this subgraph host fired". Not a
/// real pin, and deliberately unspellable as one.
pub const HOST_ACTIVITY_PIN: &str = "\u{0}subgraph";

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
        return Some(rt.trace.viz(&rt.plan, now, &label));
    }
    None
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
        match self.values.entry(from).or_default().get_mut(pin) {
            // Updating a wire already being watched is always allowed — the
            // cap bounds how many wires are remembered, not how often.
            Some(slot) => *slot = value.clone(),
            None => {
                if self.value_keys >= VALUE_CAP {
                    return;
                }
                self.value_keys += 1;
                self.values
                    .entry(from)
                    .or_default()
                    .insert(pin.to_string(), value.clone());
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
        assert_eq!(t.values[&3]["result"], Value::Int(2));

        for i in 0..(VALUE_CAP * 2) {
            t.data_value(1000 + i, "out", &Value::Int(i as i32));
        }
        assert_eq!(t.value_keys, VALUE_CAP, "the wire count stops at the cap");
        // …and the wire that was already there still updates past the cap.
        t.data_value(3, "result", &Value::Int(9));
        assert_eq!(t.values[&3]["result"], Value::Int(9));
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

    #[test]
    fn long_values_are_elided() {
        let long = "x".repeat(VALUE_CHARS + 20);
        let out = elide(&long);
        assert_eq!(out.chars().count(), VALUE_CHARS + 1);
        assert!(out.ends_with('…'));
        assert_eq!(elide("short"), "short");
    }
}
