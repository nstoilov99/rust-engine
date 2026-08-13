//! Breakpoints and the paused state (GS-4).
//!
//! A [`BreakSet`] of plan-node indices rides into a tick; when the advance
//! loop is about to **fire** an impure node in the set, the activation parks
//! in [`ThreadState::Paused`](crate::ThreadState::Paused) *before* the node
//! pulls its inputs and *before* it performs its effect. Pausing after the
//! effect would be a debugger that shows you the consequence of the statement
//! it claims not to have run yet.
//!
//! ## One graph is one timeline of effects
//!
//! While **any** activation of an instance is paused the whole instance holds:
//! no other activation advances, no due latent wakes, no queued event drains,
//! and instance time does not move. Freezing one activation and letting its
//! siblings run would produce an effect ordering no unpaused run could ever
//! produce — and the effect stream is the thing this interpreter promises is
//! deterministic (D8). A paused graph is stopped, not slowed.
//!
//! ## The three commands
//!
//! - **Resume** — unpark every paused activation and continue from the exact
//!   parked cursor. The parked node is fired *without* re-checking its own
//!   breakpoint (else resuming would re-park on the spot); every node after it
//!   breaks normally.
//! - **Step** — resume with a budget of exactly one firing across the whole
//!   instance, then re-park wherever each activation now sits. One firing, not
//!   one per activation: the instance is one timeline.
//! - **Stop** — end the session cleanly. Every live activation finishes, the
//!   queue is dropped and the instance ticks no further. Unlike a budget kill
//!   this carries no error: nothing went wrong, the debugging did. The runtime
//!   component is rebuilt on the next play, so it re-arms by itself.
//!
//! Everything here is plain serializable data, like the rest of the instance
//! state — an instance parked on a breakpoint round-trips through RON and
//! resumes at the same cursor.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// Plan-node indices that pause before firing.
///
/// Plan indices, not document ids: the interpreter has never heard of a
/// document, and resolving ids to indices is the runner's job (a document node
/// that compiled away has no index, which is exactly the editor's "invalid
/// breakpoint" state).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BreakSet {
    nodes: BTreeSet<usize>,
}

impl BreakSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, node: usize) -> bool {
        self.nodes.insert(node)
    }

    pub fn remove(&mut self, node: usize) -> bool {
        self.nodes.remove(&node)
    }

    pub fn contains(&self, node: usize) -> bool {
        self.nodes.contains(&node)
    }

    pub fn clear(&mut self) {
        self.nodes.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = usize> + '_ {
        self.nodes.iter().copied()
    }
}

impl FromIterator<usize> for BreakSet {
    fn from_iter<I: IntoIterator<Item = usize>>(iter: I) -> Self {
        Self { nodes: iter.into_iter().collect() }
    }
}

/// What the debugger is asking of the next tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DebugCommand {
    /// Nothing — run normally, and hold if something is parked.
    #[default]
    Run,
    /// Unpark and continue.
    Resume,
    /// Unpark, fire exactly one node, park again.
    Step,
    /// End this instance's session.
    Stop,
}

/// The debug channel of one tick. `Default` is "no breakpoints, no command",
/// which is byte-for-byte the pre-GS-4 interpreter: the only cost a shipped
/// game pays is [`DebugCtl::armed`], one `Option` test on an absent pointer.
#[derive(Debug, Clone, Copy, Default)]
pub struct DebugCtl<'a> {
    /// Where to pause. `None` = nowhere, and no per-node check is made.
    pub breaks: Option<&'a BreakSet>,
    pub command: DebugCommand,
}

impl<'a> DebugCtl<'a> {
    /// Breakpoints for one tick, no command.
    pub fn breaks(set: &'a BreakSet) -> Self {
        Self { breaks: Some(set), command: DebugCommand::Run }
    }

    pub fn with_command(mut self, command: DebugCommand) -> Self {
        self.command = command;
        self
    }

    /// Is anything debug-shaped happening at all? When this is false the tick
    /// never even looks for a parked activation, which is what keeps the
    /// shipped path free of the whole mechanism.
    pub fn armed(&self) -> bool {
        self.breaks.is_some() || self.command != DebugCommand::Run
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_default_ctl_is_unarmed_and_a_break_set_round_trips() {
        assert!(!DebugCtl::default().armed(), "the shipped path is unarmed");
        let set: BreakSet = [3usize, 1, 3].into_iter().collect();
        assert_eq!(set.len(), 2, "a set, not a list");
        assert!(set.contains(1) && !set.contains(2));
        assert!(DebugCtl::breaks(&set).armed());
        assert!(DebugCtl::default().with_command(DebugCommand::Stop).armed());

        let text = ron::ser::to_string(&set).unwrap();
        assert_eq!(ron::from_str::<BreakSet>(&text).unwrap(), set);
    }
}
