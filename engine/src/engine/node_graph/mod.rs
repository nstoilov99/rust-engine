//! Node graph framework (Task 40) — domain-agnostic graph documents,
//! registry, validation, and migration. Consumers (animation, scripting,
//! materials, VFX, audio, AI) build their node libraries and evaluators on
//! top of this; no domain logic lives here.
//!
//! Plan: `docs/roadmap/VULKANO-40-NODE-GRAPH-FRAMEWORK.md`.

pub mod auto_register;
pub mod descriptors;
pub mod doc;
#[cfg(any(test, feature = "dev_nodes"))]
pub mod dev_nodes;
pub mod io;
pub mod markers;
pub mod migrate;
pub mod registry;
pub mod resolver;
pub mod std_events;
pub mod validate;

#[cfg(test)]
mod macros_test;

pub use descriptors::{DocDescriptors, NodeKind};
pub use doc::{
    CommentBox, Edge, GraphDoc, GraphRealm, GroupBox, IfacePin, NodeInst, NodeRealm, PinType,
    PropValue, VarDecl, COMMENT_FONT_SCALE_MAX, COMMENT_FONT_SCALE_MIN, GRAPH_DOC_VERSION,
};
pub use io::{load_graph, parse_graph, save_graph, serialize_graph, GraphIoError};
pub use migrate::{migrate_doc, MigrationCtx, MigrationError, MigrationRecord};
pub use registry::{
    MergeReport, MigrationFn, NodeDescriptor, NodeRegistry, PinDescriptor, PreviewKind,
    RegistryError, StagedRegistry, EXEC_IN_PIN, EXEC_OUT_PIN, GRAPH_INPUT_TYPE_ID,
    GRAPH_OUTPUT_TYPE_ID, RESERVED_TYPE_IDS,
    REROUTE_IN,
    REROUTE_OUT, REROUTE_TYPE_ID, SUBGRAPH_TYPE_ID, VAR_GET_TYPE_ID, VAR_PROP, VAR_SET_TYPE_ID,
    VAR_VALUE_PIN,
};
pub use resolver::{referencing_hosts, validate_refs, GraphResolver};
pub use std_events::{
    register_std_events, std_event_descriptors, EventPhase, EVENT_BEGIN_PLAY_TYPE_ID,
    EVENT_CUSTOM_TYPE_ID, EVENT_DRAIN_ORDER, EVENT_INPUT_ACTION_TYPE_ID, EVENT_TICK_TYPE_ID,
};
pub use validate::{
    endpoint_type, reroute_type, validate_doc, validate_doc_with, ErrorAnchor, ErrorSeverity,
    GraphError,
};

pub use auto_register::{register_inventory_nodes, NodeFactory};
pub use markers::ExecPin;
// Derive macros (Task 40 P8). Re-exported so consumers can
// `use rust_engine::node_graph::{ScriptNode, AnimationNode};`.
pub use node_graph_macros::{AnimationNode, ScriptNode};
