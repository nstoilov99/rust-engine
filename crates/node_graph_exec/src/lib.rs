//! **The portable graph interpreter** (Task 45-A D1/D8).
//!
//! A `.graph` document compiles to a flattened [`Plan`]; a [`GraphInstance`]
//! runs it. Blueprint-style: event entry points start execution threads, an
//! impure node fires, pulls its data inputs backward through pure chains,
//! performs its effect and names the exec output to continue on.
//!
//! # Why this is a separate crate
//!
//! Portability here is structural, not aspirational (D8). This crate depends
//! on `node_graph_types` and `serde` and on **nothing else** — no engine, no
//! ECS, no renderer, no clock. That is what lets the same interpreter later
//! compile into the SpacetimeDB module unchanged, and it is checked the way
//! `game_shared` checks the M6 controller: CI builds this crate standalone
//! and for `wasm32-unknown-unknown`.
//!
//! Everything nondeterministic is injected:
//!
//! - **time** — [`TickInput`], supplied by the runner;
//! - **randomness** — [`Rng`], a seeded per-instance SplitMix64 living in the
//!   instance state;
//! - **the world** — read through [`WorldRead`], written only by emitting
//!   [`Effect`]s into an [`EffectSink`]. The core mutates nothing itself, and
//!   real entity ids never enter it: a spawn hands the graph an
//!   instance-local alias that the runner resolves afterwards.
//!
//! Given the same plan, seed and scripted inputs, two runs produce
//! byte-identical effect streams — the property `determinism_holds_across_runs`
//! pins and the future WASM parity suite grows from.
//!
//! # Shape
//!
//! ```text
//!   GraphDoc ──compile()──> Plan ──tick(instance, impls, world)──> [Effect]
//!             validate                     ▲
//!             flatten                      │ NodeImpls: PureNode / ImpureNode
//!             splice (subgraphs inlined)
//!             prune
//!             index entries
//! ```

pub mod effect;
pub mod exec;
pub mod instance;
pub mod node;
pub mod nodes;
pub mod plan;
pub mod value;

pub use effect::{Effect, EffectSink, LogLevel, NoWorld, TransformSnapshot, WorldRead};
pub use exec::{tick, tick_with_budget, TickReport, DEFAULT_BUDGET};
pub use instance::{
    Activation, ActivationId, Frame, GraphInstance, QueuedEvent, Rng, ThreadState,
};
pub use node::{
    EvalCtx, ExecError, FireCtx, FireResult, ImpureNode, LoopFrameView, NodeImpl, NodeImpls,
    PureNode, Suspension, TickInput,
};
pub use plan::{compile, CompileError, Entry, InputSource, Plan, PlanNode, VarSlot};
pub use value::{EntityRef, Value};

// Re-exported so a consumer needs one dependency, not two, to hold a document
// and run it.
pub use node_graph_types as types;
