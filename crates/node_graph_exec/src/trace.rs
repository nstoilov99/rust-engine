//! Execution tracing (45-A P7) — the *seam*, not the recorder.
//!
//! The interpreter tells a sink two things while it runs: which exec edge it
//! just traversed, and what value came back over a data wire it pulled. That
//! is everything the editor's execution visualization needs, and deliberately
//! all this crate knows about it — the ring buffer, the decay curve and the
//! doc-space mapping live editor-side.
//!
//! # Why a generic parameter and not `Option<&mut dyn TraceSink>`
//!
//! A shipped game must carry **no recording path at all**, not a null check
//! per fired edge. [`tick`](crate::tick) instantiates the interpreter with
//! [`NoTrace`], whose methods are empty `#[inline]` bodies: after
//! monomorphization the call sites are gone, and there is no trait object, no
//! branch and no `Option` to test. The editor build instantiates a second copy
//! against a real sink. The cost of that is one extra monomorphization in the
//! editor; the benefit is that "zero shipped-game overhead" is a property of
//! the generated code rather than a claim about a branch predictor.

use crate::value::Value;

/// What the interpreter reports while it runs.
///
/// Both hooks are called with **plan** indices. Mapping those back to document
/// nodes is [`Plan::doc_node`](crate::plan::Plan::doc_node)'s job, and it is
/// deliberately not done here: the interpreter should not pay for a lookup a
/// consumer may not want.
pub trait TraceSink {
    /// Control moved from `from`'s exec output `pin` to node `to`.
    ///
    /// Called at the moment the interpreter commits to the edge, so a node
    /// that fired but chose no continuation reports nothing.
    fn exec_edge(&mut self, from: usize, pin: &str, to: usize);

    /// A data wire was pulled: node `from`'s output `pin` produced `value`.
    ///
    /// Called at pull time, which is the only moment the value exists — the
    /// pull already computed it, so this hook costs a borrow, not a
    /// re-evaluation. Constants are *not* reported: an unwired input has no
    /// wire to hover.
    fn data_value(&mut self, from: usize, pin: &str, value: &Value);
}

/// The sink a shipped game runs with: every hook compiles away.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoTrace;

impl TraceSink for NoTrace {
    #[inline(always)]
    fn exec_edge(&mut self, _from: usize, _pin: &str, _to: usize) {}

    #[inline(always)]
    fn data_value(&mut self, _from: usize, _pin: &str, _value: &Value) {}
}
