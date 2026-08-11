//! Node graph framework types (Task 40, extracted from the engine in Task
//! 45-A P2) — domain-agnostic graph documents, registry, descriptor
//! resolution, validation and migration.
//!
//! **Why this is its own crate.** Task 45-A D8 makes portability structural
//! rather than aspirational: `node_graph_exec` must build standalone and for
//! `wasm32-unknown-unknown`, the same way `game_shared` proves the M6
//! controller. The interpreter compiles *documents* (subgraph splicing needs
//! `DocDescriptors`, entry-point indexing needs the event descriptors, and
//! compilation refuses a document that fails validation), so the document
//! layer has to be reachable from outside the engine.
//!
//! **Dependency discipline:** `serde` and `ron`, nothing else. No engine, no
//! ECS, no rendering, no wall-clock. The engine re-exports this crate whole
//! as `rust_engine::engine::node_graph`, so every existing path — and the
//! `::rust_engine::engine::node_graph::…` paths the derive macros emit —
//! keeps resolving.
//!
//! What stayed in the engine: the derive-macro marker types, the `inventory`
//! auto-registration backend, and the 39.8 plugins that *register* node sets.

pub mod descriptors;
pub mod dev_nodes;
pub mod doc;
pub mod io;
pub mod migrate;
pub mod registry;
pub mod resolver;
pub mod std_events;
pub mod std_nodes;
pub mod validate;

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
    GRAPH_OUTPUT_TYPE_ID, RESERVED_TYPE_IDS, REROUTE_IN, REROUTE_OUT, REROUTE_TYPE_ID,
    SUBGRAPH_TYPE_ID, VAR_GET_TYPE_ID, VAR_PROP, VAR_SET_TYPE_ID, VAR_VALUE_PIN, CURVE_PROP,
    TIMELINE_FINISHED_PIN, TIMELINE_PLAY_PIN, TIMELINE_REVERSE_PIN, TIMELINE_STOP_PIN,
    TIMELINE_TYPE_ID, TIMELINE_UPDATE_PIN,
};
pub use resolver::{referencing_hosts, validate_refs, CurveResolver, GraphResolver};

// Re-exported so a consumer holding graph documents does not need a second
// dependency to read the curves they reference.
pub use curve_asset;
pub use std_events::{
    register_std_events, std_event_descriptors, EventPhase, EVENT_ACTION_PROP,
    EVENT_BEGIN_PLAY_TYPE_ID, EVENT_CUSTOM_TYPE_ID, EVENT_DRAIN_ORDER,
    EVENT_INPUT_ACTION_TYPE_ID, EVENT_NAME_PROP, EVENT_PAYLOAD_PREFIX, EVENT_TICK_TYPE_ID,
};
pub use std_nodes::{register_std_nodes, std_node_descriptors, COMPARE_OPS, SEQUENCE_PINS};
pub use validate::{
    endpoint_type, reroute_type, validate_curves, validate_doc, validate_doc_with, ErrorAnchor,
    ErrorSeverity, GraphError,
};
