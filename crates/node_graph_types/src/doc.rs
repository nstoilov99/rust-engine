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
///
/// - **v1** — Task 40 shape.
/// - **v2** — Task 45-A P1: `GraphDoc::variables` and `NodeInst::title`. Both
///   default, so a v1 document still *parses*; the migration exists so a
///   loaded v1 doc is stamped v2 and every consumer sees exactly one shape.
/// - **v3** — Task 41: `GraphDoc::regions` — embedded per-node graph regions
///   ("virtual subgraphs", animation transition rules). Defaulted and skipped
///   when empty, so a document without regions serializes as it did before.
pub const GRAPH_DOC_VERSION: u32 = 3;

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
    /// 32-bit signed. `i32` rather than `i64` (Task 45-A resolved Q2):
    /// SpacetimeDB-friendlier, and no graph needs 64-bit loop counters.
    Int,
    Vec2,
    Vec3,
    Vec4,
    Color,
    Bool,
    Enum,
    String,
    /// Homogeneous list of another pin type. Nesting is legal
    /// (`Array(Array(Float))`); `Array(Exec)` is not meaningful and no
    /// descriptor should declare one.
    Array(Box<PinType>),
    Texture,
    Mesh,
    Entity,
    Exec,
    Domain(String),
}

impl PinType {
    /// The domain slug this type ultimately names, seeing through `Array`.
    /// `Array(Domain("shader"))` is still a shader pin as far as
    /// registration is concerned.
    pub fn domain_slug(&self) -> Option<&str> {
        match self {
            PinType::Domain(d) => Some(d.as_str()),
            PinType::Array(inner) => inner.domain_slug(),
            _ => None,
        }
    }

    /// Stable lowercase spelling, used where a pin type has to survive as a
    /// *string* — today the `event_custom` payload declaration
    /// (`std_events::EVENT_PAYLOAD_PREFIX`). Round-trips through
    /// [`PinType::from_type_slug`].
    pub fn type_slug(&self) -> String {
        match self {
            PinType::Float => "float".to_string(),
            PinType::Int => "int".to_string(),
            PinType::Vec2 => "vec2".to_string(),
            PinType::Vec3 => "vec3".to_string(),
            PinType::Vec4 => "vec4".to_string(),
            PinType::Color => "color".to_string(),
            PinType::Bool => "bool".to_string(),
            PinType::Enum => "enum".to_string(),
            PinType::String => "string".to_string(),
            PinType::Array(inner) => format!("array<{}>", inner.type_slug()),
            PinType::Texture => "texture".to_string(),
            PinType::Mesh => "mesh".to_string(),
            PinType::Entity => "entity".to_string(),
            PinType::Exec => "exec".to_string(),
            PinType::Domain(d) => format!("domain:{d}"),
        }
    }

    /// Inverse of [`PinType::type_slug`]. `None` for anything unrecognized —
    /// stale data to fix, never a guessed type.
    pub fn from_type_slug(s: &str) -> Option<PinType> {
        Some(match s {
            "float" => PinType::Float,
            "int" => PinType::Int,
            "vec2" => PinType::Vec2,
            "vec3" => PinType::Vec3,
            "vec4" => PinType::Vec4,
            "color" => PinType::Color,
            "bool" => PinType::Bool,
            "enum" => PinType::Enum,
            "string" => PinType::String,
            "texture" => PinType::Texture,
            "mesh" => PinType::Mesh,
            "entity" => PinType::Entity,
            "exec" => PinType::Exec,
            other => match other.strip_prefix("array<").and_then(|x| x.strip_suffix('>')) {
                Some(inner) => PinType::Array(Box::new(PinType::from_type_slug(inner)?)),
                None => PinType::Domain(other.strip_prefix("domain:")?.to_string()),
            },
        })
    }
}

/// A constant value stored on an unconnected input pin. Mirrors [`PinType`];
/// `Raw` carries any unrecognized value through a load/save cycle untouched
/// (forward compatibility — unknown data is never dropped). `Entity` pins
/// are connection-only and have no constant form.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PropValue {
    Float(f32),
    Int(i32),
    Vec2([f32; 2]),
    Vec3([f32; 3]),
    Vec4([f32; 4]),
    Color([f32; 4]),
    Bool(bool),
    /// Enum variant slug.
    Enum(String),
    /// Plain text. Spelled `Str` so it does not shadow `std::string::String`
    /// at every match site.
    Str(String),
    /// Homogeneous list. Element types are not enforced by the schema —
    /// validation compares *pin* types, and a hand-edited mixed array is
    /// stale data the type-compat rule reports at the wire.
    Array(Vec<PropValue>),
    /// Content-relative asset path.
    Asset(String),
    /// Unrecognized value preserved verbatim (RON text).
    Raw(String),
}

impl PropValue {
    /// The stored constant a pin type starts at. `None` for the types with no
    /// constant form — `Entity` is connection-only, and a texture/mesh/domain
    /// pin means "whatever is wired here".
    ///
    /// Distinct from the interpreter's `Value::zero_of`, which answers the
    /// same question for *runtime* values and therefore has an `Entity`
    /// answer (`self`). This one is about what goes in the asset.
    pub fn zero_of(ty: &PinType) -> Option<PropValue> {
        Some(match ty {
            PinType::Float => PropValue::Float(0.0),
            PinType::Int => PropValue::Int(0),
            PinType::Bool => PropValue::Bool(false),
            PinType::String => PropValue::Str(String::new()),
            PinType::Enum => PropValue::Enum(String::new()),
            PinType::Vec2 => PropValue::Vec2([0.0; 2]),
            PinType::Vec3 => PropValue::Vec3([0.0; 3]),
            PinType::Vec4 => PropValue::Vec4([0.0, 0.0, 0.0, 1.0]),
            PinType::Color => PropValue::Color([0.0, 0.0, 0.0, 1.0]),
            PinType::Array(_) => PropValue::Array(Vec::new()),
            PinType::Entity | PinType::Texture | PinType::Mesh | PinType::Exec => return None,
            PinType::Domain(_) => return None,
        })
    }

    /// Does this stored value already hold `ty`'s shape? Used when a variable
    /// is retyped: an exact match survives, anything else is reset rather
    /// than silently coerced.
    pub fn matches_type(&self, ty: &PinType) -> bool {
        matches!(
            (self, ty),
            (PropValue::Float(_), PinType::Float)
                | (PropValue::Int(_), PinType::Int)
                | (PropValue::Bool(_), PinType::Bool)
                | (PropValue::Str(_), PinType::String)
                | (PropValue::Enum(_), PinType::Enum)
                | (PropValue::Vec2(_), PinType::Vec2)
                | (PropValue::Vec3(_), PinType::Vec3)
                | (PropValue::Vec4(_), PinType::Vec4)
                | (PropValue::Color(_), PinType::Color)
                | (PropValue::Array(_), PinType::Array(_))
                | (PropValue::Asset(_), PinType::Texture)
                | (PropValue::Asset(_), PinType::Mesh)
        )
    }
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
    ///
    /// Doc-dependent reserved types (45-A) also store their *configuration*
    /// here under keys that are deliberately not pin slugs — `var` on
    /// `var_get`/`var_set`, `event_name` and `payload.<slug>` on
    /// `event_custom`, `action` on `event_input_action`. The keys are
    /// documented next to each reserved id in `registry.rs`; nothing else may
    /// invent one, because an undocumented key is indistinguishable from a
    /// stale pin constant.
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
    /// Author-supplied node title, overriding the descriptor's display name.
    /// Schema only in 45-A P1 (the rename UI is Task 45.5); it lands with the
    /// container migration because adding a field to `NodeInst` later would
    /// cost a second container bump for nothing.
    #[serde(default)]
    pub title: Option<String>,
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

/// One per-graph variable (45-A D3). `var_get`/`var_set` node instances name
/// a `slug` and take their pin type from here — which is why variables live
/// in the document and not in a node's properties.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VarDecl {
    /// Stable identity, referenced by `var_get`/`var_set` instances.
    pub slug: String,
    /// Display label, free to change.
    pub label: String,
    pub ty: PinType,
    /// Initial value. `Option` because not every type has a constant form
    /// (`Entity` is connection-only) — `None` means "the type's zero", which
    /// the interpreter defines, not the schema.
    #[serde(default)]
    pub default: Option<PropValue>,
    /// **Display metadata only** (GS-2): which panel group the row lists
    /// under. It never reaches the runtime, never changes declaration order,
    /// and is skipped on write when unset — so a document nobody grouped
    /// serializes exactly as it did before groups existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
}

/// Free-floating comment box (canvas world-space rect: min x/y, size w/h).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommentBox {
    pub rect: [f32; 4],
    pub text: String,
    /// Ramp index (never a hex). Paints the NOTE bar's fill and a 1px left
    /// edge; the body stays opaque `elevated` and is never washed — that is
    /// what tells a comment apart from a group at any zoom.
    #[serde(default)]
    pub tint: Option<u8>,
    /// Scales the NOTE bar and the body text together; the bar height grows
    /// with it. Clamped to [`COMMENT_FONT_SCALE_MIN`, `..._MAX`] on load.
    #[serde(default = "one")]
    pub font_scale: f32,
    /// Anchored notes move with the node they explain and are deleted with
    /// it. Free-floating is the default — anchoring is what stops notes
    /// drifting stale.
    #[serde(default)]
    pub anchor: Option<u64>,
    /// Folded to its NOTE bar.
    #[serde(default)]
    pub collapsed: bool,
}

/// The `font_scale` range. A note may be a heading or fine print, but not a
/// billboard: past 3x it stops being an annotation and starts being chrome.
pub const COMMENT_FONT_SCALE_MIN: f32 = 0.75;
pub const COMMENT_FONT_SCALE_MAX: f32 = 3.0;

fn one() -> f32 {
    1.0
}

impl CommentBox {
    /// `font_scale` as it may actually be used — a hand-edited asset does not
    /// get to make a note 40x.
    pub fn clamped_font_scale(&self) -> f32 {
        if self.font_scale.is_finite() {
            self.font_scale
                .clamp(COMMENT_FONT_SCALE_MIN, COMMENT_FONT_SCALE_MAX)
        } else {
            1.0
        }
    }
}

impl Default for CommentBox {
    fn default() -> Self {
        Self {
            rect: [0.0, 0.0, 220.0, 130.0],
            text: String::new(),
            tint: None,
            font_scale: 1.0,
            anchor: None,
            collapsed: false,
        }
    }
}

/// Visual group frame around a region of nodes.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct GroupBox {
    pub rect: [f32; 4],
    pub title: String,
    /// Ramp index. Paints a 6% body wash plus a 45% border — a group is a
    /// translucent region *containing* things, where a comment is an opaque
    /// card *next to* them.
    #[serde(default)]
    pub tint: Option<u8>,
    /// Folded to its title bar.
    #[serde(default)]
    pub collapsed: bool,
}

/// An embedded graph region — a "virtual subgraph" living *inside* a parent
/// document (container v3, Task 41): no file on disk, serialized inline,
/// versioned/migrated with the parent, one undo history because it is one
/// document. The animation domain stores a transition's boolean rule graph
/// this way, keyed by the transition node's id in [`GraphDoc::regions`].
///
/// Node ids are **region-local**: two regions may both contain a node 0, and
/// region ids never collide with the parent's. That is what makes a region a
/// self-contained unit — duplicating the owner node is cloning the map entry
/// under the new owner id, no id remapping.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct GraphRegion {
    #[serde(default)]
    pub nodes: Vec<NodeInst>,
    /// Edges between region nodes (region-local ids on both ends).
    #[serde(default)]
    pub edges: Vec<Edge>,
}

impl GraphRegion {
    pub fn node(&self, id: u64) -> Option<&NodeInst> {
        self.nodes.iter().find(|n| n.id == id)
    }

    /// Next free region-local node id.
    pub fn next_node_id(&self) -> u64 {
        self.nodes.iter().map(|n| n.id).max().map_or(0, |m| m + 1)
    }
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
    /// Per-graph variables (container v2).
    #[serde(default)]
    pub variables: Vec<VarDecl>,
    /// Embedded graph regions (container v3), keyed by **owner node id** —
    /// the node whose double-click descends into the region (Task 41: the
    /// transition whose rule this is). A region must die with its owner:
    /// delete nodes through [`GraphDoc::remove_node`] so it does.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub regions: BTreeMap<u64, GraphRegion>,
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
            variables: Vec::new(),
            regions: BTreeMap::new(),
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

    /// The declaration a `var_get`/`var_set` instance names.
    pub fn variable(&self, slug: &str) -> Option<&VarDecl> {
        self.variables.iter().find(|v| v.slug == slug)
    }

    /// Next free doc-local node id.
    pub fn next_node_id(&self) -> u64 {
        self.nodes.iter().map(|n| n.id).max().map_or(0, |m| m + 1)
    }

    /// Remove a node together with everything that belongs to it: incident
    /// edges, its embedded region, and comments anchored to it. This is *the*
    /// way to delete a node — piecemeal removal is how a region outlives its
    /// owner. `true` if the node existed.
    pub fn remove_node(&mut self, id: u64) -> bool {
        let before = self.nodes.len();
        self.nodes.retain(|n| n.id != id);
        if self.nodes.len() == before {
            return false;
        }
        self.edges.retain(|e| e.from_node != id && e.to_node != id);
        self.regions.remove(&id);
        self.comments.retain(|c| c.anchor != Some(id));
        true
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

    /// Every `.curve` this document references, deduplicated and sorted
    /// (45-A P8). The counterpart of [`GraphDoc::subgraph_refs`]: a host has
    /// to materialize these before it can compile, because a Timeline's output
    /// pins are its curve's tracks.
    pub fn curve_refs(&self) -> Vec<&str> {
        let mut refs: Vec<&str> = self
            .nodes
            .iter()
            .filter(|n| n.type_id == crate::registry::TIMELINE_TYPE_ID)
            .filter_map(|n| match n.properties.get(crate::registry::CURVE_PROP) {
                Some(PropValue::Asset(s)) | Some(PropValue::Str(s)) if !s.is_empty() => {
                    Some(s.as_str())
                }
                _ => None,
            })
            .collect();
        refs.sort_unstable();
        refs.dedup();
        refs
    }
}
