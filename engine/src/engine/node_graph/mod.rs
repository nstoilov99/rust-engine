//! Node graph framework (Task 40) — domain-agnostic graph documents,
//! registry, validation, and migration. Consumers (animation, scripting,
//! materials, VFX, audio, AI) build their node libraries and evaluators on
//! top of this; no domain logic lives here.
//!
//! Plan: `docs/roadmap/VULKANO-40-NODE-GRAPH-FRAMEWORK.md`.

pub mod auto_register;
pub mod doc;
#[cfg(any(test, feature = "dev_nodes"))]
pub mod dev_nodes;
pub mod io;
pub mod markers;
pub mod migrate;
pub mod registry;
pub mod resolver;
pub mod validate;

#[cfg(test)]
mod macros_test;

pub use doc::{
    CommentBox, Edge, GraphDoc, GraphRealm, GroupBox, IfacePin, NodeInst, NodeRealm, PinType,
    PropValue, COMMENT_FONT_SCALE_MAX, COMMENT_FONT_SCALE_MIN, GRAPH_DOC_VERSION,
};
pub use io::{load_graph, parse_graph, save_graph, serialize_graph, GraphIoError};
pub use migrate::{migrate_doc, MigrationCtx, MigrationError, MigrationRecord};
pub use registry::{
    MigrationFn, NodeDescriptor, NodeRegistry, PinDescriptor, RegistryError, SUBGRAPH_TYPE_ID,
};
pub use resolver::{referencing_hosts, validate_refs, GraphResolver};
pub use validate::{validate_doc, GraphError};

pub use auto_register::{register_inventory_nodes, NodeFactory};
pub use markers::ExecPin;
// Derive macros (Task 40 P8). Re-exported so consumers can
// `use rust_engine::node_graph::{ScriptNode, AnimationNode};`.
pub use node_graph_macros::{AnimationNode, ScriptNode};
