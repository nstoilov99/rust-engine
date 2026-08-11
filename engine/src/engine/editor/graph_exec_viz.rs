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

use std::collections::BTreeMap;

/// What the canvas knows about a live instance this frame.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GraphExecViz {
    /// The bound entity's display name, for the toolbar's live chip. Empty
    /// means "bound but unnamed", never "unbound" — an unbound tab has no
    /// `GraphExecViz` at all.
    pub instance: String,
    /// `(source node, output pin)` -> pulse intensity in `[0, 1]`, already
    /// decayed against the moment the frame was built. Absent = not fired
    /// recently enough to draw, which is the same thing as zero and costs
    /// less to store.
    pulses: BTreeMap<u64, BTreeMap<String, f32>>,
    /// `(source node, output pin)` -> the last value that crossed, formatted
    /// with the same spelling `Print` puts in the console.
    values: BTreeMap<u64, BTreeMap<String, String>>,
}

impl GraphExecViz {
    pub fn new(instance: impl Into<String>) -> Self {
        Self {
            instance: instance.into(),
            pulses: BTreeMap::new(),
            values: BTreeMap::new(),
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
        self.pulses.is_empty() && self.values.is_empty()
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
