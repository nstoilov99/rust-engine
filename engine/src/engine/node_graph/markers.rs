//! Marker types for the node graph derive macros (Task 40 P8).
//!
//! `ExecPin` is a zero-sized field type that the `ScriptNode` / `AnimationNode`
//! derives map to [`PinType::Exec`](super::doc::PinType::Exec). It carries no
//! data — execution flow is a wiring concept, not a value.

/// Marks a derive-macro field as an execution-flow pin.
pub struct ExecPin;
