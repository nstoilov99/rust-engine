//! Node graph document model (Task 40). Plain data structs, no behavior.
//!
//! Identity rules: node *types* and *pins* are identified by stable string
//! slugs (display names are free to change); node *instances* by a doc-local
//! integer id; cross-asset references (subgraphs) by content-relative path
//! strings, per engine convention. Node positions live here — in the asset —
//! not in GUI memory.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Container schema version written to every saved doc. Bump on container-
/// level changes and add a rewrite step in `io::parse_graph`'s envelope pass.
pub const GRAPH_DOC_VERSION: u32 = 1;

/// The realm a graph targets. Validated against each node type's
/// [`NodeRealm`] so authority violations are caught at edit time, before any
/// networking consumer exists.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GraphRealm {
    Editor,
    Client,
    Server,
    #[default]
    Shared,
}

impl GraphRealm {
    /// Lowercase mono label for the graph toolbar's realm chip. Unlike a
    /// *node's* realm — where `Shared` prints nothing — the graph's realm
    /// chip is always shown: it is the document's authority statement.
    pub fn label(self) -> &'static str {
        match self {
            GraphRealm::Editor => "editor",
            GraphRealm::Client => "client",
            GraphRealm::Server => "server",
            GraphRealm::Shared => "shared",
        }
    }
}

/// Where a node type may execute (descriptor metadata).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeRealm {
    Editor,
    Client,
    Shared,
    ServerSafe,
}

impl NodeRealm {
    /// v1-strict admission: `Shared` nodes are valid in any graph; every
    /// other node realm only in its exact target realm. Consumers can loosen
    /// this later; strict-by-default prevents silent authority leaks.
    pub fn admits(self, graph: GraphRealm) -> bool {
        match self {
            NodeRealm::Shared => true,
            NodeRealm::Editor => graph == GraphRealm::Editor,
            NodeRealm::Client => graph == GraphRealm::Client,
            NodeRealm::ServerSafe => graph == GraphRealm::Server,
        }
    }
}

/// Pin type system: base types plus consumer-registered domain types
/// (`Domain("shader")` etc. — registered with the `NodeRegistry` so
/// validation and pin coloring work without touching this enum).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PinType {
    Float,
    Vec2,
    Vec3,
    Vec4,
    Color,
    Bool,
    Enum,
    Texture,
    Mesh,
    Entity,
    Exec,
    Domain(String),
}

/// A constant value stored on an unconnected input pin. Mirrors [`PinType`];
/// `Raw` carries any unrecognized value through a load/save cycle untouched
/// (forward compatibility — unknown data is never dropped). `Entity` pins
/// are connection-only and have no constant form.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PropValue {
    Float(f32),
    Vec2([f32; 2]),
    Vec3([f32; 3]),
    Vec4([f32; 4]),
    Color([f32; 4]),
    Bool(bool),
    /// Enum variant slug.
    Enum(String),
    /// Content-relative asset path.
    Asset(String),
    /// Unrecognized value preserved verbatim (RON text).
    Raw(String),
}

/// One node instance in a document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeInst {
    /// Doc-local id, unique within this document.
    pub id: u64,
    /// Stable node-type slug (e.g. `"set_world_position"`).
    pub type_id: String,
    /// Descriptor version this instance was saved against — the migration
    /// hook (`migrate` runs when this is older than the registered version).
    pub type_version: u32,
    /// Canvas world-space position.
    pub position: [f32; 2],
    /// Unconnected input constants, keyed by pin slug. `BTreeMap` keeps
    /// serialization byte-stable.
    #[serde(default)]
    pub properties: BTreeMap<String, PropValue>,
    /// Content-relative path of the referenced subgraph asset, present only
    /// on subgraph nodes.
    #[serde(default)]
    pub subgraph: Option<String>,
    /// Per-node color override: an index into the theme's 12-hue ramp, never
    /// a hex. Replaces the category 2px top edge (deep tone); the derived tag
    /// keeps the category's color. `None` = take the category's slot.
    #[serde(default)]
    pub tint: Option<u8>,
}

/// A connection between two pins. Pin *slugs*, not indices — pins can be
/// reordered or added without breaking saved edges.
///
/// `Ord` makes the edge itself usable as a selection key: an index into
/// `doc.edges` shifts under every insert/remove, the tuple does not.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Edge {
    pub from_node: u64,
    pub from_pin: String,
    pub to_node: u64,
    pub to_pin: String,
}

/// A declared input/output of a subgraph — these become the pins of the
/// subgraph node when the asset is used inside a host graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IfacePin {
    pub slug: String,
    pub label: String,
    pub ty: PinType,
}

/// Free-floating comment box (canvas world-space rect: min x/y, size w/h).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommentBox {
    pub rect: [f32; 4],
    pub text: String,
}

/// Visual group frame around a region of nodes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GroupBox {
    pub rect: [f32; 4],
    pub title: String,
}

/// A graph document — the serialized form of `.graph` and `.subgraph`
/// assets. A subgraph is simply a doc with non-empty `inputs`/`outputs`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphDoc {
    pub version: u32,
    #[serde(default)]
    pub realm: GraphRealm,
    #[serde(default)]
    pub nodes: Vec<NodeInst>,
    #[serde(default)]
    pub edges: Vec<Edge>,
    #[serde(default)]
    pub comments: Vec<CommentBox>,
    #[serde(default)]
    pub groups: Vec<GroupBox>,
    #[serde(default)]
    pub inputs: Vec<IfacePin>,
    #[serde(default)]
    pub outputs: Vec<IfacePin>,
}

impl Default for GraphDoc {
    fn default() -> Self {
        Self {
            version: GRAPH_DOC_VERSION,
            realm: GraphRealm::default(),
            nodes: Vec::new(),
            edges: Vec::new(),
            comments: Vec::new(),
            groups: Vec::new(),
            inputs: Vec::new(),
            outputs: Vec::new(),
        }
    }
}

impl GraphDoc {
    pub fn node(&self, id: u64) -> Option<&NodeInst> {
        self.nodes.iter().find(|n| n.id == id)
    }

    pub fn node_mut(&mut self, id: u64) -> Option<&mut NodeInst> {
        self.nodes.iter_mut().find(|n| n.id == id)
    }

    /// Next free doc-local node id.
    pub fn next_node_id(&self) -> u64 {
        self.nodes.iter().map(|n| n.id).max().map_or(0, |m| m + 1)
    }

    /// Content-relative paths of all referenced subgraphs (deduplicated).
    pub fn subgraph_refs(&self) -> Vec<&str> {
        let mut refs: Vec<&str> = self
            .nodes
            .iter()
            .filter_map(|n| n.subgraph.as_deref())
            .collect();
        refs.sort_unstable();
        refs.dedup();
        refs
    }
}
