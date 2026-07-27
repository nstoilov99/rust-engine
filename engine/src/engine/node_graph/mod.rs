//! Node graph framework (Task 40) — domain-agnostic graph documents,
//! registry, validation, and migration. Consumers (animation, scripting,
//! materials, VFX, audio, AI) build their node libraries and evaluators on
//! top of this; no domain logic lives here.
//!
//! Plan: `docs/roadmap/VULKANO-40-NODE-GRAPH-FRAMEWORK.md`.

pub mod doc;
#[cfg(any(test, feature = "dev_nodes"))]
pub mod dev_nodes;
pub mod io;
pub mod migrate;
pub mod registry;
pub mod validate;

pub use doc::{
    CommentBox, Edge, GraphDoc, GraphRealm, GroupBox, IfacePin, NodeInst, NodeRealm, PinType,
    PropValue, GRAPH_DOC_VERSION,
};
pub use io::{load_graph, parse_graph, save_graph, serialize_graph, GraphIoError};
pub use migrate::{migrate_doc, MigrationCtx, MigrationError, MigrationRecord};
pub use registry::{
    MigrationFn, NodeDescriptor, NodeRegistry, PinDescriptor, RegistryError, SUBGRAPH_TYPE_ID,
};
pub use validate::{validate_doc, GraphError};
