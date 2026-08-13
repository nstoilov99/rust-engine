//! Execution visualization data, in **document space** (45-A P7).
//!
//! The bridge between the runtime's trace (plan indices, instance time) and
//! the graph canvas (document node ids, a normalized intensity). Deliberately
//! a plain struct with no dependency on the interpreter: the panel must draw
//! the same way whether the graph-scripting plugin is compiled in or not, and
//! a build without it simply never has one of these.
//!
//! Everything is keyed by the **producing pin** — `(source node, output pin)`
//! — because that is what a wire *is* from the trace's point of view: one
//! output feeding three inputs is one truth, not three. Wires whose source is
//! a reroute resolve by walking back to the real producer at draw time; the
//! transparent chain is a document fact, not a runtime one.
//!
//! Scope (audit addendum #11): 45-A ships the pulse and the value hover.
//! Flow bubbles, pinned watches and breakpoints are Task 45.5's, and they will
//! want richer per-activation data than this carries.

use std::collections::{BTreeMap, BTreeSet};

/// A node with at least one activation parked on it (GS-3).
///
/// One entry per *node*, never per activation: the design's rule is one
/// indicator per node, because two bars would imply two nodes. The bar tracks
/// the nearest-due wait and `count` names the rest.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct WaitInfo {
    /// How far through the nearest-due wait we are, `[0, 1]`.
    pub fraction: f32,
    /// Seconds left on it.
    pub remaining: f32,
    /// How many activations are parked on this node.
    pub count: u32,
}

/// The runtime error that stopped an instance, anchored to the node that
/// raised it (GS-3). The console already said it once; this is the canvas's
/// copy, so "what died and where" is answerable without reading a log.
#[derive(Debug, Clone, PartialEq)]
pub struct KillInfo {
    /// Document node the error names, when it could be resolved.
    pub node: Option<u64>,
    /// One mono line, already phrased for a reader.
    pub reason: String,
}

/// What the PAUSED banner asked the runtime for (GS-4).
///
/// A mirror of the interpreter's own command enum rather than a re-export, for
/// the reason this whole module exists: the panel draws the same way in a
/// build without the graph-scripting plugin, so it cannot name a type from the
/// interpreter crate. The host translates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebugRequest {
    Resume,
    Step,
    Stop,
}

/// The instance is parked on a breakpoint (GS-4).
///
/// One per instance, never per activation: the banner has one Resume button
/// because the whole instance holds, so naming a second parked activation
/// would imply a second thing to resume.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PauseInfo {
    /// Document node the activation parked *before* firing.
    pub node: u64,
    /// Which activation, for the report — not shown, but it is what makes two
    /// consecutive pauses on one node distinguishable.
    pub activation: u64,
    /// How many times this session has parked on that node ("hit 3×").
    pub hits: u32,
}

/// One entity running this graph, for the LIVE chip's instance picker.
///
/// Plain data with an `id` rather than an `Entity`: the panel must not depend
/// on the ECS, and the id is only ever compared and handed back.
#[derive(Debug, Clone, PartialEq)]
pub struct ExecInstance {
    /// `Entity::to_bits`, the handle the picker hands back when it is picked.
    pub id: u64,
    pub name: String,
    /// Metres from the editor camera, for telling two "Duck"s apart.
    pub distance: f32,
    /// Seconds since this instance last executed anything; `None` when it has
    /// never run.
    pub last_active: Option<f32>,
    /// Stopped by a runtime error — still listed, because a post-mortem is
    /// exactly when you want to look at it.
    pub killed: bool,
}

/// What the canvas knows about a live instance this frame.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GraphExecViz {
    /// The bound entity's display name, for the toolbar's live chip. Empty
    /// means "bound but unnamed", never "unbound" — an unbound tab has no
    /// `GraphExecViz` at all.
    pub instance: String,
    /// The bound entity's handle, so the picker can mark the current row.
    pub instance_id: u64,
    /// Exec edges that have fired **at some point this session**, in document
    /// space: `(source node, output pin, target node)`.
    ///
    /// Deliberately a set of per-edge booleans rather than a ring: the pulse
    /// ring is bounded at 512 firings because it is about *recency*, and the
    /// taken path is about *ever*. The set is bounded by the document's edge
    /// count, which is the honest bound for the question it answers.
    taken: BTreeSet<(u64, String, u64)>,
    /// `(source node, output pin)` -> firings per second, averaged over the
    /// last second of the ring. Only used to decide when a wire stops
    /// strobing and holds steady-hot instead.
    rates: BTreeMap<(u64, String), f32>,
    /// Nodes with activations parked on them.
    waiting: BTreeMap<u64, WaitInfo>,
    /// Set when this instance was stopped by a runtime error.
    pub killed: Option<KillInfo>,
    /// Set while this instance sits on a breakpoint (GS-4).
    pub paused: Option<PauseInfo>,
    /// Armed document nodes that resolve to nothing the interpreter can pause
    /// before — pruned as unreachable, compiled away, or simply not an impure
    /// node. The mark draws as the invalid state (warning + `!`) rather than
    /// pretending it will fire.
    pub invalid_breaks: BTreeSet<u64>,
    /// `(source node, output pin)` -> seconds since that value crossed.
    ages: BTreeMap<u64, BTreeMap<String, f32>>,
    /// `(source node, output pin)` -> pulse intensity in `[0, 1]`, already
    /// decayed against the moment the frame was built. Absent = not fired
    /// recently enough to draw, which is the same thing as zero and costs
    /// less to store.
    pulses: BTreeMap<u64, BTreeMap<String, f32>>,
    /// `(source node, output pin)` -> the last value that crossed, formatted
    /// with the same spelling `Print` puts in the console.
    values: BTreeMap<u64, BTreeMap<String, String>>,
}

/// Firings per second above which a wire stops strobing and holds steady-hot.
/// Past this the bubbles are a flicker rather than a flow, and the honest
/// statement is "this is running constantly" (DESIGN-graphscripting ▸ S3).
pub const STEADY_HOT_HZ: f32 = 20.0;

impl GraphExecViz {
    pub fn new(instance: impl Into<String>) -> Self {
        Self {
            instance: instance.into(),
            ..Self::default()
        }
    }

    /// Record a pulse. **Max-merges**: several plan nodes can map to one
    /// document node (everything inlined out of one subgraph does), and the
    /// brightest recent firing is the one that should show.
    pub fn add_pulse(&mut self, node: u64, pin: &str, intensity: f32) {
        if intensity <= 0.0 {
            return;
        }
        let slot = self
            .pulses
            .entry(node)
            .or_default()
            .entry(pin.to_string())
            .or_insert(0.0);
        *slot = slot.max(intensity.min(1.0));
    }

    /// Record a last value. Later writes win, and the trace hands them over
    /// oldest-first, so the newest firing is what remains.
    pub fn set_value(&mut self, node: u64, pin: &str, text: String) {
        self.values
            .entry(node)
            .or_default()
            .insert(pin.to_string(), text);
    }

    /// Pulse intensity for one output pin; `0.0` when it has not fired.
    pub fn pulse(&self, node: u64, pin: &str) -> f32 {
        self.pulses
            .get(&node)
            .and_then(|m| m.get(pin))
            .copied()
            .unwrap_or(0.0)
    }

    /// Record how old a last value is.
    pub fn set_value_age(&mut self, node: u64, pin: &str, secs: f32) {
        self.ages
            .entry(node)
            .or_default()
            .insert(pin.to_string(), secs.max(0.0));
    }

    /// How long ago the value on one output pin crossed it.
    pub fn value_age(&self, node: u64, pin: &str) -> Option<f32> {
        self.ages.get(&node).and_then(|m| m.get(pin)).copied()
    }

    /// The last value that crossed one output pin.
    pub fn value(&self, node: u64, pin: &str) -> Option<&str> {
        self.values.get(&node).and_then(|m| m.get(pin)).map(|s| s.as_str())
    }

    /// Has this node fired anything at all recently? Drives the node-level
    /// affordance for subgraph hosts, whose *internal* wires never pulse.
    pub fn node_active(&self, node: u64) -> f32 {
        self.pulses
            .get(&node)
            .and_then(|m| m.values().cloned().fold(None, |a: Option<f32>, b| Some(a.map_or(b, |x| x.max(b)))))
            .unwrap_or(0.0)
    }

    pub fn is_empty(&self) -> bool {
        self.pulses.is_empty()
            && self.values.is_empty()
            && self.taken.is_empty()
            && self.waiting.is_empty()
            && self.killed.is_none()
            && self.paused.is_none()
    }

    /// Is this the node the instance is parked on? The top of the badge
    /// gutter's precedence ladder (GS-4).
    pub fn paused_on(&self, node: u64) -> bool {
        self.paused.is_some_and(|p| p.node == node)
    }

    /// Is this armed mark unresolvable — the mockup's warning + `!` state?
    pub fn break_invalid(&self, node: u64) -> bool {
        self.invalid_breaks.contains(&node)
    }

    // --- GS-3 -----------------------------------------------------------

    /// Record that an exec edge has been travelled this session.
    pub fn mark_taken(&mut self, from: u64, pin: &str, to: u64) {
        self.taken.insert((from, pin.to_string(), to));
    }

    /// Has this exec edge fired at any point this session? The **unfired**
    /// answer is what dims: orientation comes from quieting the rest, not
    /// from tinting the hot path a new colour.
    pub fn is_taken(&self, from: u64, pin: &str, to: u64) -> bool {
        // `BTreeSet` cannot be probed by a borrowed tuple without allocating
        // the key, and this runs per wire per frame — a range scan over the
        // node's own entries is cheaper and needs no allocation.
        self.taken
            .range((from, String::new(), 0)..)
            .take_while(|(f, _, _)| *f == from)
            .any(|(_, p, t)| p == pin && *t == to)
    }

    /// Any session history at all — the switch between "editor" and "editor
    /// with a live layer over it".
    pub fn has_session(&self) -> bool {
        !self.taken.is_empty()
            || !self.waiting.is_empty()
            || self.killed.is_some()
            || self.paused.is_some()
    }

    pub fn set_rate(&mut self, node: u64, pin: &str, hz: f32) {
        self.rates.insert((node, pin.to_string()), hz);
    }

    /// Firings per second on one output pin.
    pub fn rate(&self, node: u64, pin: &str) -> f32 {
        self.rates
            .get(&(node, pin.to_string()))
            .copied()
            .unwrap_or(0.0)
    }

    /// Is this wire firing too fast to animate honestly?
    pub fn steady_hot(&self, node: u64, pin: &str) -> bool {
        self.rate(node, pin) >= STEADY_HOT_HZ
    }

    /// Record a parked activation on `node`. **Nearest-due wins** the bar and
    /// every one of them adds to the count.
    pub fn add_wait(&mut self, node: u64, fraction: f32, remaining: f32) {
        let slot = self.waiting.entry(node).or_default();
        if slot.count == 0 || remaining < slot.remaining {
            slot.fraction = fraction.clamp(0.0, 1.0);
            slot.remaining = remaining.max(0.0);
        }
        slot.count += 1;
    }

    pub fn wait(&self, node: u64) -> Option<&WaitInfo> {
        self.waiting.get(&node)
    }

    pub fn waiting_len(&self) -> usize {
        self.waiting.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pulses_max_merge_and_values_overwrite() {
        let mut v = GraphExecViz::new("Duck");
        v.add_pulse(7, "exec_out", 0.3);
        v.add_pulse(7, "exec_out", 0.9);
        v.add_pulse(7, "exec_out", 0.5);
        assert_eq!(v.pulse(7, "exec_out"), 0.9, "the brightest firing wins");
        assert_eq!(v.pulse(7, "body"), 0.0);
        assert_eq!(v.pulse(8, "exec_out"), 0.0);

        // Zero is not stored: absent and zero are the same statement.
        v.add_pulse(9, "exec_out", 0.0);
        assert_eq!(v.node_active(9), 0.0);

        v.set_value(3, "result", "1".into());
        v.set_value(3, "result", "2".into());
        assert_eq!(v.value(3, "result"), Some("2"), "the newest value wins");
        assert_eq!(v.value(3, "other"), None);
        assert!(!v.is_empty());
        assert!(GraphExecViz::default().is_empty());
    }

    /// Intensity is clamped on the way in, so no draw site has to.
    #[test]
    fn intensity_is_clamped() {
        let mut v = GraphExecViz::new("Duck");
        v.add_pulse(1, "exec_out", 4.0);
        assert_eq!(v.pulse(1, "exec_out"), 1.0);
    }
}
