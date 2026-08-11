//! Node graph framework (Task 40) — domain-agnostic graph documents,
//! registry, validation, and migration. Consumers (animation, scripting,
//! materials, VFX, audio, AI) build their node libraries and evaluators on
//! top of this; no domain logic lives here.
//!
//! **The types live in `crates/node_graph_types` since Task 45-A P2** and are
//! re-exported here whole. The move is structural, not cosmetic: the portable
//! interpreter (`node_graph_exec`) compiles *documents*, and D8 requires it to
//! build standalone and for wasm32 — which it cannot do if the document model
//! is inside a crate that pulls in Vulkan.
//!
//! This module is therefore a shim plus the two pieces that genuinely belong
//! to the engine build: the derive-macro marker types and the `inventory`
//! auto-registration backend. Every path that worked before still works,
//! including the `::rust_engine::engine::node_graph::…` paths the derive
//! macros emit.
//!
//! Plans: `docs/roadmap/VULKANO-40-NODE-GRAPH-FRAMEWORK.md`,
//! `docs/roadmap/VULKANO-45A-GRAPH-EXECUTION-CORE.md`.

pub mod auto_register;
pub mod markers;

#[cfg(test)]
mod macros_test;

/// The 45-A P2 engine-side integration spike: proves the interpreter's
/// effect/world contract against a real `hecs` world. Test-only; P5 replaces
/// it with the real runner.
#[cfg(test)]
mod exec_spike;

// Modules, re-exported so `node_graph::doc::…` / `node_graph::registry::…`
// paths keep resolving.
pub use node_graph_types::{
    descriptors, doc, io, migrate, registry, resolver, std_events, validate,
};

// The dev node set is always *compiled* in the types crate (it is static
// descriptor data, and the framework's own tests need it); what the engine's
// `dev_nodes` feature gates is whether the editor and game can reach it —
// registration happens through `DevNodesPlugin`.
#[cfg(any(test, feature = "dev_nodes"))]
pub use node_graph_types::dev_nodes;

pub use node_graph_types::*;

pub use auto_register::{register_inventory_nodes, NodeFactory};
pub use markers::ExecPin;
// Derive macros (Task 40 P8). Re-exported so consumers can
// `use rust_engine::node_graph::{ScriptNode, AnimationNode};`.
pub use node_graph_macros::{AnimationNode, ScriptNode};
