//! Graph editor per-document state (Task 40, P4 shell + P5 editing core).
//!
//! Holds one open `.graph`/`.subgraph` document, its doc-local validation
//! errors, canvas view, selection, and a doc-local undo/redo stack — mirroring
//! how `mesh_editor` owns per-document state. All document mutations go through
//! [`GraphEditStack`] so undo/redo and saved-cursor dirty tracking stay
//! coherent (plan D7). The drawing/interaction layer is `graph_editor_crusty`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Instant;

use crusty_gui::math::Vec2;
use serde::{Deserialize, Serialize};
use crusty_gui::widgets::CanvasView;

use crate::engine::node_graph::{
    endpoint_type, load_graph, migrate_doc, save_graph, validate_doc, CommentBox, Edge,
    GraphDoc, GraphRegion, GraphResolver, IfacePin, PinType, VarDecl,
    DocDescriptors, GraphError, GroupBox, NodeInst, NodeRegistry, PropValue, GRAPH_INPUT_TYPE_ID,
    GRAPH_OUTPUT_TYPE_ID, REROUTE_IN, REROUTE_OUT,
    REROUTE_TYPE_ID, SUBGRAPH_TYPE_ID, EVENT_CUSTOM_TYPE_ID, EVENT_NAME_PROP,
    EVENT_PAYLOAD_PREFIX,
};

/// The type a freshly added payload field takes (GS-1): the commonest one, and
/// the row's own dropdown changes it in a click.
pub const DEFAULT_PAYLOAD_TYPE: &str = "float";

/// How far outside the collapsed selection's bounding box the auto-inserted
/// `graph_input` / `graph_output` nodes land, in canvas world units. Far
/// enough that they read as the boundary rather than as part of the content.
const GRAPH_IFACE_NODE_GAP: f32 = 260.0;

// ---------------------------------------------------------------------------
// Edit stack (plan D7): reversible ops + saved-cursor dirty.
// ---------------------------------------------------------------------------

/// A reversible document edit. Each variant stores enough to both apply and
/// revert without consulting the registry.
#[derive(Debug, Clone, PartialEq)]
pub enum GraphEdit {
    /// A node was added (apply = insert, revert = remove by id).
    AddNode(NodeInst),
    /// Nodes and their incident edges were removed together. Each carries its
    /// original index so undo restores the exact vec order (byte-stable
    /// saves). `comments` is the anchored-note collateral: a note anchored to
    /// a deleted node dies with it, and comes back with it. `regions` is the
    /// embedded-region collateral (Task 41): a transition's rule graph lives
    /// keyed under the node's id and must die and return with it — keyed, not
    /// indexed, because `GraphDoc::regions` is a map.
    RemoveNodes {
        nodes: Vec<(usize, NodeInst)>,
        edges: Vec<(usize, Edge)>,
        comments: Vec<(usize, CommentBox)>,
        regions: Vec<(u64, GraphRegion)>,
    },
    /// An edge was created.
    Connect(Edge),
    /// Edges were removed together (wire selection + Delete). Each carries
    /// its original index so undo restores the exact vec order — a graph must
    /// serialize byte-identically after an undo.
    Disconnect { edges: Vec<(usize, Edge)> },
    /// Nodes moved by a fixed world-space delta (drag-coalesced).
    MoveNodes { ids: Vec<u64>, delta: [f32; 2] },
    /// A fragment (nodes + internal edges) was pasted/duplicated. `regions`
    /// carries the pasted nodes' embedded regions, keyed by the *new* node
    /// ids — a duplicated transition arrives with its rule, never orphaned.
    Paste {
        nodes: Vec<NodeInst>,
        edges: Vec<Edge>,
        regions: Vec<(u64, GraphRegion)>,
    },
    /// An inline input constant changed (P2 canvas widgets). `None` on either
    /// side means "no stored property", so setting a first value and clearing
    /// one back to the descriptor default are both round-trippable.
    SetProperty {
        node: u64,
        key: String,
        old: Option<PropValue>,
        new: Option<PropValue>,
    },
    // --- Variables (45-A P6). Declarations live in a Vec, so removal is
    //     index-based like the annotations below: valid because the undo
    //     stack applies and reverts strictly LIFO, and it restores the exact
    //     order a byte-stable save depends on. ---
    /// A variable declaration was appended.
    AddVariable(VarDecl),
    /// A declaration was removed from `index`. The `var_get`/`var_set` nodes
    /// that named it are deliberately left alone: they become
    /// `UnknownVariable` validation errors, which is the honest degradation —
    /// silently deleting an author's nodes would be worse than showing them
    /// what broke.
    RemoveVariable { index: usize, decl: VarDecl },
    /// The display label changed. **Only the label** — the slug is forever
    /// (Task 40 identity rules), which is exactly what lets every existing
    /// `var_get`/`var_set` keep pointing at the same declaration across a
    /// rename.
    RenameVariable { slug: String, old: String, new: String },
    /// The declared type changed, carrying the default with it because a
    /// retype resets a default it can no longer hold.
    RetypeVariable {
        slug: String,
        old_ty: PinType,
        new_ty: PinType,
        old_default: Option<PropValue>,
        new_default: Option<PropValue>,
    },
    /// The declared initial value changed. Coalesced across a drag exactly
    /// like `SetProperty`.
    SetVariableDefault {
        slug: String,
        old: Option<PropValue>,
        new: Option<PropValue>,
    },
    /// The panel group a declaration lists under (GS-2). **Display metadata**
    /// — it moves no declaration and reaches no runtime, which is why it is
    /// its own edit rather than a flavour of reorder.
    SetVariableGroup {
        slug: String,
        old: Option<String>,
        new: Option<String>,
    },
    /// A declaration moved within the list. Index-based like the annotation
    /// ops, and valid for the same reason (the stack applies/reverts strictly
    /// LIFO); `apply` removes at `from` and inserts at `to`, `revert` does the
    /// exact inverse, so a gesture round-trips the vec order a byte-stable
    /// save depends on.
    ReorderVariable { from: usize, to: usize },
    // --- Annotations (P7). Comments/groups have no ids; index-based ops are
    //     valid because the undo stack applies/reverts strictly LIFO. ---
    /// A comment box was appended.
    AddComment(CommentBox),
    /// A comment box was removed from `index`.
    RemoveComment { index: usize, comment: CommentBox },
    /// A comment box's rect origin moved by a delta (drag-coalesced).
    MoveComment { index: usize, delta: [f32; 2] },
    /// A comment box's text changed.
    SetCommentText { index: usize, old: String, new: String },
    /// A group frame was appended.
    AddGroup(GroupBox),
    /// A group frame was removed from `index` (nodes are untouched).
    RemoveGroup { index: usize, group: GroupBox },
    /// A group moved by a delta, carrying the captured member nodes with it.
    MoveGroup { index: usize, node_ids: Vec<u64>, delta: [f32; 2] },
    /// A group's title changed.
    SetGroupTitle { index: usize, old: String, new: String },
    /// An annotation's tint slot changed (a ramp index, never a hex).
    SetAnnotationTint {
        target: Annotation,
        old: Option<u8>,
        new: Option<u8>,
    },
    /// An annotation folded to (or unfolded from) its bar.
    SetAnnotationCollapsed { target: Annotation, new: bool },
    /// An annotation was resized — drag-coalesced like a move, storing both
    /// rects so a gesture reverts exactly.
    ResizeAnnotation {
        target: Annotation,
        old: [f32; 4],
        new: [f32; 4],
    },
    /// A comment's anchor node changed. An anchored note follows the node it
    /// explains and dies with it.
    SetCommentAnchor {
        index: usize,
        old: Option<u64>,
        new: Option<u64>,
    },
    /// Several edits that must undo as one gesture — inserting a reroute
    /// (one edge out, a node and two edges in) is the motivating case.
    /// Applied in order, reverted in reverse.
    Composite { label: String, edits: Vec<GraphEdit> },
    /// An edit inside the embedded region keyed under `owner` (Task 41
    /// ticket 05: the rule canvas). The inner edit speaks region-local node
    /// ids and applies against the region's nodes/edges as if they were a
    /// document — which is what makes rule editing **one history with the
    /// machine's**: the peek records here, and Ctrl+Z at either level pops
    /// the same stack.
    ///
    /// Apply creates the region entry on demand; both directions prune an
    /// entry left empty, because absent and empty both mean always-true and
    /// only one of them serializes — an undo must round-trip the bytes.
    InRegion { owner: u64, edit: Box<GraphEdit> },
}

/// Which annotation an edit targets. Comments and groups have no ids, so
/// index-based addressing is used — valid because the undo stack applies and
/// reverts strictly LIFO.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Annotation {
    Comment(usize),
    Group(usize),
}

impl Annotation {
    pub fn is_group(self) -> bool {
        matches!(self, Annotation::Group(_))
    }
    pub fn index(self) -> usize {
        match self {
            Annotation::Comment(i) | Annotation::Group(i) => i,
        }
    }
}

impl GraphEdit {
    /// Redo direction — the edit as originally performed. Public so the
    /// panel can apply a composite it just built before recording it.
    pub fn apply(&self, doc: &mut GraphDoc) {
        match self {
            GraphEdit::AddNode(n) => doc.nodes.push(n.clone()),
            GraphEdit::RemoveNodes { nodes, edges, comments, regions } => {
                let ids: BTreeSet<u64> = nodes.iter().map(|(_, n)| n.id).collect();
                doc.nodes.retain(|n| !ids.contains(&n.id));
                doc.edges.retain(|e| !edges.iter().any(|(_, re)| re == e));
                let doomed: BTreeSet<usize> = comments.iter().map(|(i, _)| *i).collect();
                let mut i = 0;
                doc.comments.retain(|_| {
                    let keep = !doomed.contains(&i);
                    i += 1;
                    keep
                });
                for (id, _) in regions {
                    doc.regions.remove(id);
                }
            }
            GraphEdit::Connect(e) => doc.edges.push(e.clone()),
            GraphEdit::Disconnect { edges } => {
                // By recorded index, not by equality: two edges can share a
                // from/to tuple (an output fans out, and an identical pair is
                // representable), and removing by value would take both while
                // undo restores one.
                let mut doomed: Vec<usize> = edges
                    .iter()
                    .filter(|(i, e)| doc.edges.get(*i) == Some(e))
                    .map(|(i, _)| *i)
                    .collect();
                doomed.sort_unstable();
                for i in doomed.into_iter().rev() {
                    doc.edges.remove(i);
                }
            }
            GraphEdit::MoveNodes { ids, delta } => move_nodes(doc, ids, *delta),
            GraphEdit::Paste { nodes, edges, regions } => {
                doc.nodes.extend(nodes.iter().cloned());
                doc.edges.extend(edges.iter().cloned());
                for (id, r) in regions {
                    doc.regions.insert(*id, r.clone());
                }
            }
            GraphEdit::SetProperty { node, key, new, .. } => set_prop(doc, *node, key, new),
            GraphEdit::AddVariable(v) => doc.variables.push(v.clone()),
            GraphEdit::RemoveVariable { index, .. } => {
                if *index < doc.variables.len() {
                    doc.variables.remove(*index);
                }
            }
            GraphEdit::RenameVariable { slug, new, .. } => set_var_label(doc, slug, new),
            GraphEdit::RetypeVariable { slug, new_ty, new_default, .. } => {
                set_var_type(doc, slug, new_ty, new_default)
            }
            GraphEdit::SetVariableDefault { slug, new, .. } => set_var_default(doc, slug, new),
            GraphEdit::SetVariableGroup { slug, new, .. } => set_var_group(doc, slug, new),
            GraphEdit::ReorderVariable { from, to } => move_variable(doc, *from, *to),
            GraphEdit::AddComment(c) => doc.comments.push(c.clone()),
            GraphEdit::RemoveComment { index, .. } => {
                doc.comments.remove(*index);
            }
            GraphEdit::MoveComment { index, delta } => shift_rect(&mut doc.comments[*index].rect, *delta),
            GraphEdit::SetCommentText { index, new, .. } => {
                doc.comments[*index].text = new.clone()
            }
            GraphEdit::AddGroup(g) => doc.groups.push(g.clone()),
            GraphEdit::RemoveGroup { index, .. } => {
                doc.groups.remove(*index);
            }
            GraphEdit::MoveGroup { index, node_ids, delta } => {
                shift_rect(&mut doc.groups[*index].rect, *delta);
                move_nodes(doc, node_ids, *delta);
            }
            GraphEdit::SetGroupTitle { index, new, .. } => {
                doc.groups[*index].title = new.clone()
            }
            GraphEdit::SetAnnotationTint { target, new, .. } => set_tint(doc, *target, *new),
            GraphEdit::SetAnnotationCollapsed { target, new } => {
                set_collapsed(doc, *target, *new)
            }
            GraphEdit::ResizeAnnotation { target, new, .. } => set_rect(doc, *target, *new),
            GraphEdit::SetCommentAnchor { index, new, .. } => {
                if let Some(c) = doc.comments.get_mut(*index) {
                    c.anchor = *new;
                }
            }
            GraphEdit::Composite { edits, .. } => {
                for e in edits {
                    e.apply(doc);
                }
            }
            GraphEdit::InRegion { owner, edit } => {
                in_region(doc, *owner, |scratch| edit.apply(scratch));
            }
        }
    }

    /// Undo direction — the inverse of [`apply`](Self::apply).
    fn revert(&self, doc: &mut GraphDoc) {
        match self {
            GraphEdit::AddNode(n) => doc.nodes.retain(|x| x.id != n.id),
            GraphEdit::RemoveNodes { nodes, edges, comments, regions } => {
                // Reinsert at original indices, ascending so each index still
                // refers to the correct slot once earlier ones are back.
                reinsert_indexed(&mut doc.nodes, nodes);
                reinsert_indexed(&mut doc.edges, edges);
                reinsert_indexed(&mut doc.comments, comments);
                for (id, r) in regions {
                    doc.regions.insert(*id, r.clone());
                }
            }
            GraphEdit::Connect(e) => doc.edges.retain(|x| x != e),
            GraphEdit::Disconnect { edges } => reinsert_indexed(&mut doc.edges, edges),
            GraphEdit::MoveNodes { ids, delta } => {
                move_nodes(doc, ids, [-delta[0], -delta[1]])
            }
            GraphEdit::Paste { nodes, edges, regions } => {
                let ids: BTreeSet<u64> = nodes.iter().map(|n| n.id).collect();
                doc.nodes.retain(|n| !ids.contains(&n.id));
                doc.edges.retain(|e| !edges.contains(e));
                for (id, _) in regions {
                    doc.regions.remove(id);
                }
            }
            GraphEdit::SetProperty { node, key, old, .. } => set_prop(doc, *node, key, old),
            GraphEdit::AddVariable(v) => {
                // By slug rather than by popping: a composite may have added
                // more than one, and slugs are unique by construction.
                doc.variables.retain(|d| d.slug != v.slug);
            }
            GraphEdit::RemoveVariable { index, decl } => {
                let at = (*index).min(doc.variables.len());
                doc.variables.insert(at, decl.clone());
            }
            GraphEdit::RenameVariable { slug, old, .. } => set_var_label(doc, slug, old),
            GraphEdit::RetypeVariable { slug, old_ty, old_default, .. } => {
                set_var_type(doc, slug, old_ty, old_default)
            }
            GraphEdit::SetVariableDefault { slug, old, .. } => set_var_default(doc, slug, old),
            GraphEdit::SetVariableGroup { slug, old, .. } => set_var_group(doc, slug, old),
            GraphEdit::ReorderVariable { from, to } => move_variable(doc, *to, *from),
            GraphEdit::AddComment(_) => {
                doc.comments.pop();
            }
            GraphEdit::RemoveComment { index, comment } => {
                doc.comments.insert(*index, comment.clone())
            }
            GraphEdit::MoveComment { index, delta } => {
                shift_rect(&mut doc.comments[*index].rect, [-delta[0], -delta[1]])
            }
            GraphEdit::SetCommentText { index, old, .. } => {
                doc.comments[*index].text = old.clone()
            }
            GraphEdit::AddGroup(_) => {
                doc.groups.pop();
            }
            GraphEdit::RemoveGroup { index, group } => {
                doc.groups.insert(*index, group.clone())
            }
            GraphEdit::MoveGroup { index, node_ids, delta } => {
                shift_rect(&mut doc.groups[*index].rect, [-delta[0], -delta[1]]);
                move_nodes(doc, node_ids, [-delta[0], -delta[1]]);
            }
            GraphEdit::SetGroupTitle { index, old, .. } => {
                doc.groups[*index].title = old.clone()
            }
            GraphEdit::SetAnnotationTint { target, old, .. } => set_tint(doc, *target, *old),
            GraphEdit::SetAnnotationCollapsed { target, new } => {
                set_collapsed(doc, *target, !*new)
            }
            GraphEdit::ResizeAnnotation { target, old, .. } => set_rect(doc, *target, *old),
            GraphEdit::SetCommentAnchor { index, old, .. } => {
                if let Some(c) = doc.comments.get_mut(*index) {
                    c.anchor = *old;
                }
            }
            GraphEdit::Composite { edits, .. } => {
                for e in edits.iter().rev() {
                    e.revert(doc);
                }
            }
            GraphEdit::InRegion { owner, edit } => {
                in_region(doc, *owner, |scratch| edit.revert(scratch));
            }
        }
    }

    /// M10-style verb/object label ("Move 3 Nodes").
    pub fn description(&self) -> String {
        fn plural(n: usize) -> &'static str {
            if n == 1 {
                ""
            } else {
                "s"
            }
        }
        match self {
            GraphEdit::AddNode(_) => "Add Node".to_string(),
            GraphEdit::RemoveNodes { nodes, .. } => {
                format!("Delete {} Node{}", nodes.len(), plural(nodes.len()))
            }
            GraphEdit::Connect(_) => "Connect".to_string(),
            GraphEdit::Disconnect { edges } => {
                format!("Break {} Link{}", edges.len(), plural(edges.len()))
            }
            GraphEdit::MoveNodes { ids, .. } => {
                format!("Move {} Node{}", ids.len(), plural(ids.len()))
            }
            GraphEdit::Paste { nodes, .. } => {
                format!("Paste {} Node{}", nodes.len(), plural(nodes.len()))
            }
            GraphEdit::SetProperty { key, .. } => format!("Set {key}"),
            GraphEdit::AddVariable(_) => "Add Variable".to_string(),
            GraphEdit::RemoveVariable { .. } => "Delete Variable".to_string(),
            GraphEdit::RenameVariable { .. } => "Rename Variable".to_string(),
            GraphEdit::RetypeVariable { .. } => "Retype Variable".to_string(),
            GraphEdit::SetVariableDefault { .. } => "Set Variable Default".to_string(),
            GraphEdit::SetVariableGroup { new, .. } => {
                if new.is_some() { "Group Variable" } else { "Ungroup Variable" }.to_string()
            }
            GraphEdit::ReorderVariable { .. } => "Reorder Variable".to_string(),
            GraphEdit::AddComment(_) => "Add Comment".to_string(),
            GraphEdit::RemoveComment { .. } => "Delete Comment".to_string(),
            GraphEdit::MoveComment { .. } => "Move Comment".to_string(),
            GraphEdit::SetCommentText { .. } => "Edit Comment".to_string(),
            GraphEdit::AddGroup(_) => "Add Group".to_string(),
            GraphEdit::RemoveGroup { .. } => "Delete Group".to_string(),
            GraphEdit::MoveGroup { .. } => "Move Group".to_string(),
            GraphEdit::SetGroupTitle { .. } => "Edit Group".to_string(),
            GraphEdit::SetAnnotationTint { target, new, .. } => format!(
                "{} {}",
                if new.is_some() { "Tint" } else { "Clear Tint on" },
                if target.is_group() { "Group" } else { "Comment" }
            ),
            GraphEdit::SetAnnotationCollapsed { target, new } => format!(
                "{} {}",
                if *new { "Collapse" } else { "Expand" },
                if target.is_group() { "Group" } else { "Comment" }
            ),
            GraphEdit::ResizeAnnotation { target, .. } => format!(
                "Resize {}",
                if target.is_group() { "Group" } else { "Comment" }
            ),
            GraphEdit::SetCommentAnchor { new, .. } => {
                if new.is_some() { "Anchor Comment" } else { "Un-anchor Comment" }.to_string()
            }
            GraphEdit::Composite { label, .. } => label.clone(),
            // "Add Node in Rule" — the verb stays the inner edit's, the
            // suffix says which canvas it happened on.
            GraphEdit::InRegion { edit, .. } => format!("{} in Rule", edit.description()),
        }
    }
}

/// Run `f` over the region keyed under `owner`, viewed as a scratch document
/// — the region's nodes and edges are moved into a bare [`GraphDoc`], the
/// inner edit runs against it with the exact same semantics it has at top
/// level, and the results move back. Creates the entry on demand and prunes
/// it when it ends empty (absent and empty both mean the same thing, and
/// only one spelling may reach a save).
fn in_region(doc: &mut GraphDoc, owner: u64, f: impl FnOnce(&mut GraphDoc)) {
    let region = doc.regions.entry(owner).or_default();
    let mut scratch = GraphDoc {
        nodes: std::mem::take(&mut region.nodes),
        edges: std::mem::take(&mut region.edges),
        ..GraphDoc::default()
    };
    f(&mut scratch);
    region.nodes = scratch.nodes;
    region.edges = scratch.edges;
    if region.nodes.is_empty() && region.edges.is_empty() {
        doc.regions.remove(&owner);
    }
}

fn set_tint(doc: &mut GraphDoc, target: Annotation, v: Option<u8>) {
    match target {
        Annotation::Comment(i) => {
            if let Some(c) = doc.comments.get_mut(i) {
                c.tint = v;
            }
        }
        Annotation::Group(i) => {
            if let Some(g) = doc.groups.get_mut(i) {
                g.tint = v;
            }
        }
    }
}

fn set_collapsed(doc: &mut GraphDoc, target: Annotation, v: bool) {
    match target {
        Annotation::Comment(i) => {
            if let Some(c) = doc.comments.get_mut(i) {
                c.collapsed = v;
            }
        }
        Annotation::Group(i) => {
            if let Some(g) = doc.groups.get_mut(i) {
                g.collapsed = v;
            }
        }
    }
}

fn set_rect(doc: &mut GraphDoc, target: Annotation, r: [f32; 4]) {
    match target {
        Annotation::Comment(i) => {
            if let Some(c) = doc.comments.get_mut(i) {
                c.rect = r;
            }
        }
        Annotation::Group(i) => {
            if let Some(g) = doc.groups.get_mut(i) {
                g.rect = r;
            }
        }
    }
}

/// Write (or clear) one node property. Missing nodes are ignored — an edit
/// may outlive its node across an undo branch.
fn set_prop(doc: &mut GraphDoc, node: u64, key: &str, value: &Option<PropValue>) {
    let Some(n) = doc.node_mut(node) else {
        return;
    };
    match value {
        Some(v) => {
            n.properties.insert(key.to_string(), v.clone());
        }
        None => {
            n.properties.remove(key);
        }
    }
}

fn move_nodes(doc: &mut GraphDoc, ids: &[u64], delta: [f32; 2]) {
    for id in ids {
        if let Some(n) = doc.node_mut(*id) {
            n.position[0] += delta[0];
            n.position[1] += delta[1];
        }
    }
    // An anchored note explains a specific node, so it travels with it. Both
    // directions go through here, so undo carries the note back too.
    for c in doc.comments.iter_mut() {
        if c.anchor.is_some_and(|a| ids.contains(&a)) {
            shift_rect(&mut c.rect, delta);
        }
    }
}

/// Round every position and annotation rect to whole pixels, in place and
/// **without reordering anything**. The disk form additionally sorts nodes by
/// id; that part stays out of memory because undo indexes into these vecs.
fn snap_positions(doc: &mut GraphDoc) {
    fn px(v: f32) -> f32 {
        if v.is_finite() {
            v.round()
        } else {
            0.0
        }
    }
    for n in doc.nodes.iter_mut() {
        n.position = [px(n.position[0]), px(n.position[1])];
    }
    for c in doc.comments.iter_mut() {
        c.rect = c.rect.map(px);
    }
    for g in doc.groups.iter_mut() {
        g.rect = g.rect.map(px);
    }
}

/// A slug not already used by `taken`, suffixed `_2`, `_3`, ... A fan-out
/// from one source pin collapses to one interface pin; two *different* pins
/// that happen to share a slug do not.
fn uniquify(base: &str, taken: &[IfacePin]) -> String {
    if !taken.iter().any(|p| p.slug == base) {
        return base.to_string();
    }
    for n in 2..1000u32 {
        let candidate = format!("{base}_{n}");
        if !taken.iter().any(|p| p.slug == candidate) {
            return candidate;
        }
    }
    base.to_string()
}

fn set_var_label(doc: &mut GraphDoc, slug: &str, label: &str) {
    if let Some(v) = doc.variables.iter_mut().find(|v| v.slug == slug) {
        v.label = label.to_string();
    }
}

fn set_var_type(doc: &mut GraphDoc, slug: &str, ty: &PinType, default: &Option<PropValue>) {
    if let Some(v) = doc.variables.iter_mut().find(|v| v.slug == slug) {
        v.ty = ty.clone();
        v.default = default.clone();
    }
}

fn set_var_default(doc: &mut GraphDoc, slug: &str, value: &Option<PropValue>) {
    if let Some(v) = doc.variables.iter_mut().find(|v| v.slug == slug) {
        v.default = value.clone();
    }
}

fn set_var_group(doc: &mut GraphDoc, slug: &str, group: &Option<String>) {
    if let Some(v) = doc.variables.iter_mut().find(|v| v.slug == slug) {
        v.group = group.clone();
    }
}

/// Move the declaration at `from` to `to`. Out-of-range indices are a no-op
/// rather than a panic: an undo stack that outlived its document is a bug to
/// survive, not to crash on.
fn move_variable(doc: &mut GraphDoc, from: usize, to: usize) {
    if from >= doc.variables.len() || to >= doc.variables.len() || from == to {
        return;
    }
    let decl = doc.variables.remove(from);
    doc.variables.insert(to, decl);
}

/// A variable slug from a display name: lowercase, `_`-joined, alphanumeric.
///
/// Deliberately snake_case rather than the `-` form `slugify` produces for
/// `#node-slug` chips — a variable slug sits beside pin and node-type slugs
/// in the identity rules, and those are snake_case. An empty result becomes
/// `var`, because "" is not a name.
pub fn variable_slug(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut underscore = false;
    for ch in name.chars() {
        if ch.is_alphanumeric() {
            for c in ch.to_lowercase() {
                out.push(c);
            }
            underscore = false;
        } else if !underscore && !out.is_empty() {
            out.push('_');
            underscore = true;
        }
    }
    let out = out.trim_end_matches('_').to_string();
    if out.is_empty() {
        "var".to_string()
    } else {
        out
    }
}

// ---------------------------------------------------------------------------
// Variables panel view-model (GS-2)
//
// The list the strip draws is derived, never stored: groups are metadata on
// the declarations, the filter is session state, and the order is the
// document's. Deriving it here — with no `Ui` in sight — is what lets the
// grouping, the filter partition and the counts be tested directly.
// ---------------------------------------------------------------------------

/// One line of the variables list, in render order.
#[derive(Debug, Clone, PartialEq)]
pub enum VarListRow {
    /// A group header. `name` is `None` for the implicit trailing section that
    /// collects ungrouped declarations — it only appears when some other row
    /// *is* grouped, so an ungrouped document looks exactly as it always did.
    Group { name: Option<String>, count: usize, collapsed: bool },
    /// A declaration, by index into `doc.variables`.
    Var(usize),
}

/// The derived list plus the numbers the header and footer quote.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct VarListView {
    pub rows: Vec<VarListRow>,
    /// Declarations matching the filter (all of them when it is empty).
    pub matches: usize,
    pub total: usize,
    /// `total - matches` — what the "N hidden by filter" line reports.
    pub hidden: usize,
}

/// Does this declaration match `filter`? Substring, case-insensitive, over the
/// label **and** the slug — the two names an author would think to type.
pub fn variable_matches(decl: &VarDecl, filter: &str) -> bool {
    let q = filter.trim().to_lowercase();
    q.is_empty()
        || decl.label.to_lowercase().contains(&q)
        || decl.slug.to_lowercase().contains(&q)
}

/// Build the list: group headers over the *unchanged* declaration order, with
/// non-matching rows dropped and headers that match nothing dropped with them.
///
/// Group order is first-declaration order, so grouping never reshuffles the
/// list and the same file always draws the same way. Collapsed groups keep
/// their header (and its count) and drop their rows.
pub fn variables_view(
    doc: &GraphDoc,
    filter: &str,
    collapsed: &BTreeSet<String>,
) -> VarListView {
    let total = doc.variables.len();
    let matching: Vec<bool> = doc
        .variables
        .iter()
        .map(|v| variable_matches(v, filter))
        .collect();
    let matches = matching.iter().filter(|m| **m).count();

    // Group names in first-declaration order; `None` sorts last, always.
    let mut order: Vec<Option<String>> = Vec::new();
    for v in &doc.variables {
        let key = v.group.clone();
        if !order.contains(&key) {
            order.push(key);
        }
    }
    let grouped = order.iter().any(|g| g.is_some());
    order.sort_by_key(|g| g.is_none());

    let mut rows = Vec::new();
    for group in order {
        let members: Vec<usize> = doc
            .variables
            .iter()
            .enumerate()
            .filter(|(i, v)| v.group == group && matching[*i])
            .map(|(i, _)| i)
            .collect();
        if members.is_empty() {
            continue;
        }
        // A document nobody grouped has no headers at all — the shipped look.
        if grouped {
            let is_collapsed = group
                .as_deref()
                .is_some_and(|g| collapsed.contains(g));
            rows.push(VarListRow::Group {
                name: group.clone(),
                count: members.len(),
                collapsed: is_collapsed,
            });
            if is_collapsed {
                continue;
            }
        }
        rows.extend(members.into_iter().map(VarListRow::Var));
    }
    VarListView { rows, matches, total, hidden: total - matches }
}

/// The `var_get`/`var_set` nodes that name `slug`, in document order — what
/// the usage count counts and what "locate" cycles through.
pub fn variable_node_ids(doc: &GraphDoc, slug: &str) -> Vec<u64> {
    doc.nodes
        .iter()
        .filter(|n| {
            matches!(
                n.type_id.as_str(),
                crate::engine::node_graph::VAR_GET_TYPE_ID
                    | crate::engine::node_graph::VAR_SET_TYPE_ID
            ) && matches!(n.properties.get(crate::engine::node_graph::VAR_PROP),
                Some(PropValue::Str(s)) if s == slug)
        })
        .map(|n| n.id)
        .collect()
}

/// The in-row warning a retyped variable earns: how many *wired* uses now
/// disagree with the declaration, and what they expect.
///
/// Derived from the validation results rather than recomputed, so the row and
/// the canvas can never disagree about whether something is wrong: every
/// `TypeMismatch` whose edge touches this variable's value pin votes, and the
/// type the wire's *other* end expects is the one named.
pub fn variable_mismatch(doc: &GraphDoc, slug: &str, errors: &[GraphError]) -> Option<String> {
    let ids = variable_node_ids(doc, slug);
    if ids.is_empty() {
        return None;
    }
    let mut count = 0usize;
    let mut expected: Option<PinType> = None;
    for e in errors {
        let GraphError::TypeMismatch { edge, from_ty, to_ty } = e else {
            continue;
        };
        // A Get feeds the wire (its type is `from`); a Set is fed by it.
        let (mine, theirs) = if ids.contains(&edge.from_node) {
            (from_ty, to_ty)
        } else if ids.contains(&edge.to_node) {
            (to_ty, from_ty)
        } else {
            continue;
        };
        if mine == theirs {
            continue;
        }
        count += 1;
        expected.get_or_insert_with(|| theirs.clone());
    }
    let expected = expected?;
    Some(format!(
        "{count} wired use{} expect{} {}",
        if count == 1 { "" } else { "s" },
        if count == 1 { "s" } else { "" },
        pin_type_label(&expected)
    ))
}

/// A pin type as the panel spells it (`Float`, `Vec3[]`). Lives here rather
/// than in the drawing layer because the mismatch reason line is text the
/// view-model produces and a test asserts on.
pub fn pin_type_label(ty: &PinType) -> String {
    match ty {
        PinType::Array(inner) => format!("{}[]", pin_type_label(inner)),
        PinType::Float => "Float".to_string(),
        PinType::Int => "Int".to_string(),
        PinType::String => "String".to_string(),
        PinType::Bool => "Bool".to_string(),
        PinType::Vec2 => "Vec2".to_string(),
        PinType::Vec3 => "Vec3".to_string(),
        PinType::Vec4 => "Vec4".to_string(),
        PinType::Color => "Color".to_string(),
        PinType::Entity => "Entity".to_string(),
        PinType::Exec => "Exec".to_string(),
        // The animation Trigger parameter (Task 41) — a first-class type in
        // the variables panel, so it wears its declared name, not its
        // wire-format slug.
        PinType::Domain(d) if d == crate::engine::animation::graph::TRIGGER_PARAM_DOMAIN => {
            "Trigger".to_string()
        }
        other => other.type_slug(),
    }
}

/// What a retype would do to the stored default, in the words the confirmation
/// uses. `None` when there is no default to say anything about.
///
/// The rule is P6b's, unchanged — an exact type match survives, anything else
/// resets — and this only *reports* it, so the dialog can never promise an
/// outcome the edit does not perform.
pub fn retype_default_outcome(decl: &VarDecl, ty: &PinType) -> Option<String> {
    let old = decl.default.as_ref()?;
    let shown = prop_display(old);
    if old.matches_type(ty) {
        return Some(format!("Default {shown} is kept."));
    }
    match PropValue::zero_of(ty) {
        Some(zero) => Some(format!(
            "Default {shown} \u{2192} {} (reset \u{2014} no silent coercion).",
            prop_display(&zero)
        )),
        None => Some(format!(
            "Default {shown} is dropped \u{2014} {} has no literal.",
            pin_type_label(ty)
        )),
    }
}

/// How many wires read one custom-event payload field, and across how many
/// documents (GS-1).
///
/// A "reader" is a **wire out of a `<slug>` payload pin** on an `event_custom`
/// node named `event_name`: every one of them stops carrying a value when the
/// field goes away or changes name. `doc`/`doc_path` are the live editor
/// document (the resolver's copy of it may be a frame stale, so it is skipped
/// there and counted here).
///
/// **The boundary, stated:** the resolver enumerates the *open + resolvable*
/// set — every open graph tab plus every subgraph they reference transitively
/// (see `build_resolver_docs`). A `.graph` on disk that nobody has opened and
/// nothing references is not in it and is not counted. That is the honest
/// limit of what the editor can know without scanning the whole content tree,
/// and it is why the dialog says "N listeners in M graphs" rather than
/// claiming to have found them all.
pub fn payload_reader_count(
    doc: &GraphDoc,
    doc_path: &str,
    event_name: &str,
    slug: &str,
    resolver: &dyn GraphResolver,
) -> (usize, usize) {
    fn in_doc(d: &GraphDoc, event_name: &str, slug: &str) -> usize {
        let listeners: BTreeSet<u64> = d
            .nodes
            .iter()
            .filter(|n| {
                n.type_id == EVENT_CUSTOM_TYPE_ID
                    && matches!(
                        n.properties.get(EVENT_NAME_PROP),
                        Some(PropValue::Str(s)) | Some(PropValue::Enum(s)) if s == event_name
                    )
            })
            .map(|n| n.id)
            .collect();
        if listeners.is_empty() {
            return 0;
        }
        d.edges
            .iter()
            .filter(|e| listeners.contains(&e.from_node) && e.from_pin == slug)
            .count()
    }
    let mut readers = in_doc(doc, event_name, slug);
    let mut graphs = usize::from(readers > 0);
    for (path, other) in resolver.documents() {
        if path == doc_path {
            continue;
        }
        let n = in_doc(other, event_name, slug);
        if n > 0 {
            readers += n;
            graphs += 1;
        }
    }
    (readers, graphs)
}

/// Indices of the comments anchored to any of `ids` — the collateral a node
/// delete has to carry.
pub fn anchored_comments(doc: &GraphDoc, ids: &BTreeSet<u64>) -> Vec<usize> {
    doc.comments
        .iter()
        .enumerate()
        .filter(|(_, c)| c.anchor.is_some_and(|a| ids.contains(&a)))
        .map(|(i, _)| i)
        .collect()
}

/// Would `edge` wire machine flow **directly** from a state-family `out`
/// into a state's `in`? Then the drop means "make a transition here" — the
/// state-machine drag gesture — and this answers the `(source, target)` pair
/// [`GraphEditorState::insert_transition_between`] should join. A plain
/// connect would author an edge the compiler silently ignores, which is the
/// one thing an editor must never let a gesture mean.
pub fn transition_shortcut(doc: &GraphDoc, edge: &Edge) -> Option<(u64, u64)> {
    use crate::engine::animation::graph::plan::{
        ANIM_ANY_STATE_TYPE_ID, ANIM_STATE_TYPE_ID, STATE_IN_PIN, STATE_OUT_PIN,
    };
    let from = doc.node(edge.from_node)?;
    let to = doc.node(edge.to_node)?;
    let from_is_source = (from.type_id == ANIM_STATE_TYPE_ID
        || from.type_id == ANIM_ANY_STATE_TYPE_ID)
        && edge.from_pin == STATE_OUT_PIN;
    let to_is_state = to.type_id == ANIM_STATE_TYPE_ID && edge.to_pin == STATE_IN_PIN;
    (from_is_source && to_is_state).then_some((from.id, to.id))
}

/// The embedded regions keyed by any of `ids` — the other collateral a node
/// delete has to carry (Task 41: a transition's rule dies and returns with
/// its transition).
pub fn region_collateral(
    doc: &GraphDoc,
    ids: impl IntoIterator<Item = u64>,
) -> Vec<(u64, GraphRegion)> {
    ids.into_iter()
        .filter_map(|id| Some((id, doc.regions.get(&id)?.clone())))
        .collect()
}

/// Reinsert `(index, value)` pairs into `v` at their original indices, in
/// ascending index order so restoring one doesn't shift the next.
fn reinsert_indexed<T: Clone>(v: &mut Vec<T>, items: &[(usize, T)]) {
    let mut items: Vec<&(usize, T)> = items.iter().collect();
    items.sort_by_key(|(i, _)| *i);
    for (i, val) in items {
        let at = (*i).min(v.len());
        v.insert(at, val.clone());
    }
}

/// Shift a world-space `[min_x, min_y, w, h]` rect's origin by `delta`.
fn shift_rect(rect: &mut [f32; 4], delta: [f32; 2]) {
    rect[0] += delta[0];
    rect[1] += delta[1];
}

/// Node ids whose center (given as `(id, [cx, cy])`) falls inside a
/// `[min_x, min_y, w, h]` world rect — the group-drag capture rule (P7).
pub fn nodes_captured_by_rect(centers: &[(u64, [f32; 2])], rect: [f32; 4]) -> Vec<u64> {
    let (x0, y0, x1, y1) = (rect[0], rect[1], rect[0] + rect[2], rect[1] + rect[3]);
    centers
        .iter()
        .filter(|(_, c)| c[0] >= x0 && c[0] <= x1 && c[1] >= y0 && c[1] <= y1)
        .map(|(id, _)| *id)
        .collect()
}

/// Doc-local undo/redo with saved-cursor dirty tracking.
///
/// Dirty is the *distance from the save point*, not a sticky flag: the stack
/// records the position (applied-edit count) at the last save, and
/// `is_dirty()` = current position ≠ saved position. Undoing back to the save
/// point clears dirty; a post-undo edit that truncates the redo branch
/// containing the save point invalidates it, so the doc stays dirty until the
/// next save.
#[derive(Default)]
pub struct GraphEditStack {
    undo: Vec<GraphEdit>,
    redo: Vec<GraphEdit>,
    /// Applied-edit count at last save. `None` once the save point is lost to
    /// a truncated redo branch.
    saved: Option<usize>,
}

impl GraphEditStack {
    /// A stack for a freshly loaded (clean) document.
    pub fn new() -> Self {
        Self { undo: Vec::new(), redo: Vec::new(), saved: Some(0) }
    }

    /// Record an edit that has *already* been applied to the doc. Clears the
    /// redo branch; if the save point lived in that branch, it is lost.
    pub fn record(&mut self, edit: GraphEdit) {
        if let Some(s) = self.saved {
            if s > self.undo.len() {
                self.saved = None;
            }
        }
        self.undo.push(edit);
        self.redo.clear();
    }

    /// Pop the last edit and revert it against `doc`. Returns its description.
    pub fn undo(&mut self, doc: &mut GraphDoc) -> Option<String> {
        let edit = self.undo.pop()?;
        edit.revert(doc);
        let desc = edit.description();
        self.redo.push(edit);
        Some(desc)
    }

    /// Re-apply the last undone edit against `doc`. Returns its description.
    pub fn redo(&mut self, doc: &mut GraphDoc) -> Option<String> {
        let edit = self.redo.pop()?;
        edit.apply(doc);
        let desc = edit.description();
        self.undo.push(edit);
        Some(desc)
    }

    /// How many entries deep the undo history is — what a test asserts on
    /// when the claim is "this gesture recorded exactly one entry".
    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }

    /// Drain every recorded edit, oldest first, leaving the stack clean —
    /// the rule scope's per-frame handoff (ticket 05): the projection records
    /// as usual, the parent history takes the entries.
    pub fn take_edits(&mut self) -> Vec<GraphEdit> {
        self.redo.clear();
        self.saved = Some(0);
        std::mem::take(&mut self.undo)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo_description(&self) -> Option<String> {
        self.undo.last().map(GraphEdit::description)
    }

    pub fn redo_description(&self) -> Option<String> {
        self.redo.last().map(GraphEdit::description)
    }

    /// Mark the current position as saved (dirty clears until the next edit).
    pub fn mark_saved(&mut self) {
        self.saved = Some(self.undo.len());
    }

    /// Unsaved changes ⇔ current position differs from the save point.
    pub fn is_dirty(&self) -> bool {
        self.saved != Some(self.undo.len())
    }
}

// ---------------------------------------------------------------------------
// Clipboard fragment.
// ---------------------------------------------------------------------------

/// What a fragment is tagged as on the system clipboard, so arbitrary text
/// that happens to parse as RON is never mistaken for a graph.
pub const FRAGMENT_KIND: &str = "crusty.graph.fragment";
/// Fragment schema version. Bump when the shape changes; older payloads are
/// rejected rather than half-read.
pub const FRAGMENT_VERSION: u32 = 1;
/// Largest clipboard payload worth parsing. A real fragment is kilobytes;
/// this only exists so arbitrary clipboard content cannot cost real time.
pub const MAX_CLIPBOARD_BYTES: usize = 4 * 1024 * 1024;

/// A copied slice of a graph: nodes, the edges internal to them, and the
/// annotations that came along. Ids are remapped on paste, so a fragment can
/// be pasted into any document.
///
/// This is a **RON subset, not an internal pointer set** — that is what lets
/// a paste survive both a tab switch and a whole editor session.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GraphFragment {
    pub nodes: Vec<NodeInst>,
    pub edges: Vec<Edge>,
    pub comments: Vec<CommentBox>,
    pub groups: Vec<GroupBox>,
    /// Embedded regions of the copied nodes, keyed by the *fragment* node id
    /// (remapped with everything else on paste). Additive and defaulted, so a
    /// pre-region clipboard payload still parses — the container-v3 rule.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub regions: BTreeMap<u64, GraphRegion>,
}

/// The clipboard envelope: a kind tag and a version around the payload, so
/// pasting someone else's RON is a no-op rather than a surprise.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct FragmentEnvelope {
    kind: String,
    version: u32,
    fragment: GraphFragment,
}

impl GraphFragment {
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty() && self.comments.is_empty() && self.groups.is_empty()
    }

    /// Serialize for the system clipboard.
    pub fn to_ron(&self) -> Result<String, String> {
        ron::ser::to_string_pretty(
            &FragmentEnvelope {
                kind: FRAGMENT_KIND.to_string(),
                version: FRAGMENT_VERSION,
                fragment: self.clone(),
            },
            Default::default(),
        )
        .map_err(|e| e.to_string())
    }

    /// Parse clipboard text. Defensive by construction: anything that is not
    /// one of our fragments — arbitrary text, other RON, a future version —
    /// returns `None` instead of erroring or panicking.
    pub fn from_ron(text: &str) -> Option<Self> {
        // The system clipboard is arbitrary input. Cap it before parsing:
        // a graph fragment is small, and a paste should not be able to make
        // the editor chew through a hundred megabytes of someone else's
        // text. (Nesting depth is already bounded — `ron` inherits serde's
        // recursion limit of 128, so a deeply-nested payload errors out
        // rather than blowing the stack; size is the part left to us.)
        if text.len() > MAX_CLIPBOARD_BYTES {
            println!(
                "graph: ignoring a {} MB clipboard payload (cap is {} MB)",
                text.len() / 1_000_000,
                MAX_CLIPBOARD_BYTES / 1_000_000
            );
            return None;
        }
        let env: FragmentEnvelope = ron::from_str(text).ok()?;
        (env.kind == FRAGMENT_KIND && env.version == FRAGMENT_VERSION).then_some(env.fragment)
    }

    /// World-space top-left of everything in the fragment — the anchor a
    /// cursor-relative paste is measured from.
    pub fn bbox_min(&self) -> [f32; 2] {
        let mut min = [f32::MAX, f32::MAX];
        let mut fold = |x: f32, y: f32| {
            min[0] = min[0].min(x);
            min[1] = min[1].min(y);
        };
        for n in &self.nodes {
            fold(n.position[0], n.position[1]);
        }
        for c in &self.comments {
            fold(c.rect[0], c.rect[1]);
        }
        for g in &self.groups {
            fold(g.rect[0], g.rect[1]);
        }
        if min[0] == f32::MAX {
            [0.0, 0.0]
        } else {
            min
        }
    }

    /// Produce a copy with fresh ids (starting at `first_id`) and positions
    /// offset by `offset`.
    ///
    /// Edges remap **by pin slug** — the existing edge-identity rule — so a
    /// descriptor that reordered its pins still reconnects. Edges with an
    /// endpoint outside the fragment are dropped, and counted: silent loss is
    /// fine, unreported loss is not.
    fn instantiate(&self, first_id: u64, offset: [f32; 2]) -> Instantiated {
        let mut remap: BTreeMap<u64, u64> = BTreeMap::new();
        let mut nodes = Vec::with_capacity(self.nodes.len());
        for (i, n) in self.nodes.iter().enumerate() {
            let new_id = first_id + i as u64;
            remap.insert(n.id, new_id);
            let mut c = n.clone();
            c.id = new_id;
            c.position[0] += offset[0];
            c.position[1] += offset[1];
            nodes.push(c);
        }
        let before = self.edges.len();
        let edges: Vec<Edge> = self
            .edges
            .iter()
            .filter_map(|e| {
                Some(Edge {
                    from_node: *remap.get(&e.from_node)?,
                    from_pin: e.from_pin.clone(),
                    to_node: *remap.get(&e.to_node)?,
                    to_pin: e.to_pin.clone(),
                })
            })
            .collect();
        let dropped = before - edges.len();

        let comments = self
            .comments
            .iter()
            .map(|c| {
                let mut c = c.clone();
                c.rect[0] += offset[0];
                c.rect[1] += offset[1];
                // An anchor pointing outside the fragment would tie the copy
                // to the original's node; remap it or cut it loose.
                c.anchor = c.anchor.and_then(|a| remap.get(&a).copied());
                c
            })
            .collect();
        let groups = self
            .groups
            .iter()
            .map(|g| {
                let mut g = g.clone();
                g.rect[0] += offset[0];
                g.rect[1] += offset[1];
                g
            })
            .collect();
        // Regions re-key to the new owner ids; their *contents* are
        // region-local and copy untouched — that locality is the whole point
        // of the container-v3 design.
        let regions = self
            .regions
            .iter()
            .filter_map(|(owner, r)| Some((*remap.get(owner)?, r.clone())))
            .collect();
        Instantiated { nodes, edges, comments, groups, regions, dropped }
    }
}

/// One instantiation of a fragment, ready to apply.
struct Instantiated {
    nodes: Vec<NodeInst>,
    edges: Vec<Edge>,
    comments: Vec<CommentBox>,
    groups: Vec<GroupBox>,
    regions: Vec<(u64, GraphRegion)>,
    /// Boundary edges that could not come along.
    dropped: usize,
}

// ---------------------------------------------------------------------------
// Editor state.
// ---------------------------------------------------------------------------

/// State for one open graph document.
pub struct GraphEditorState {
    /// Content-relative path (forward slashes), e.g. `"graphs/foo.graph"`.
    /// This is the tab key; the on-disk file is `content/<path>`.
    pub path: String,
    /// The loaded (and migrated) document.
    pub doc: GraphDoc,
    /// Doc-local validation errors from the last edit/load (never fatal).
    pub errors: Vec<GraphError>,
    /// Cross-asset (subgraph) validation errors, refreshed by the editor's
    /// per-frame `validate_refs` pass (P6). Shown alongside `errors`.
    pub ref_errors: Vec<GraphError>,
    /// Unsaved changes flag, kept in sync with the edit stack's saved cursor.
    pub dirty: bool,
    /// Time of the last save through this editor — used by hot-reload to
    /// suppress the watcher echo of our own write (P6).
    pub last_saved_at: Option<Instant>,
    /// Pan/zoom canvas view (session-only — not persisted in the asset).
    pub view: CanvasView,
    /// Selected node ids.
    pub selection: BTreeSet<u64>,
    /// Last-clicked node of the selection — draws its `selection.outline` at
    /// 100% while the rest of the set draws at 55% (DESIGN-nodegraph
    /// ▸ Selection). `None` when the selection came from a marquee.
    pub primary: Option<u64>,
    /// Selected wires, keyed by the edge itself rather than by index (indices
    /// shift under every insert/remove; the slug tuple is stable).
    pub selected_edges: BTreeSet<Edge>,
    /// Doc-local undo/redo.
    pub stack: GraphEditStack,
    /// In-flight node drag (session-only).
    pub node_drag: Option<NodeDrag>,
    /// In-flight pin→pin connection drag (session-only).
    pub connect_drag: Option<ConnectDrag>,
    /// In-flight marquee box-select origin, world space (session-only).
    pub marquee: Option<[f32; 2]>,
    /// How the in-flight marquee combines with the existing selection,
    /// captured at press time so releasing the modifier mid-drag can't change
    /// the gesture's meaning.
    pub marquee_mode: MarqueeMode,
    /// In-flight inline property edit (canvas widgets). Coalesces a whole
    /// drag/toggle into one undo entry, flushed on pointer release.
    pub prop_edit: Option<PropEdit>,
    /// In-flight coalesced edit of a variable default (45-A P6).
    pub var_edit: Option<VarDefaultEdit>,
    /// World position captured when the node-create menu opened.
    pub create_menu_world: Option<[f32; 2]>,
    /// Search text of the node-create menu.
    pub create_menu_search: String,
    /// Selected comment index (P7). Mutually exclusive with node/group select.
    pub sel_comment: Option<usize>,
    /// Selected group index (P7).
    pub sel_group: Option<usize>,
    /// In-flight comment/group drag (P7).
    pub annotation_drag: Option<AnnotationDrag>,
    /// Active inline text edit for a comment/group (P7).
    pub editing: Option<AnnotationEdit>,
    /// In-flight annotation resize (session-only), coalesced into one edit.
    pub annotation_resize: Option<AnnotationResize>,
    /// Annotation whose context menu is open (tint / collapse / anchor).
    pub annotation_menu: Option<Annotation>,
    /// Cursor into the anchored-error list, advanced by the count chip.
    /// `F8` / `Shift+F8` will drive the same cursor.
    pub error_cursor: usize,
    /// Compiler-row popover (document-level errors) is showing.
    pub error_popover: bool,
    /// Wire whose context menu is open, with the world point clicked.
    pub wire_menu: Option<(Edge, [f32; 2])>,
    /// View bookmarks, session-only for now (the user-local sidecar that
    /// persists them across sessions is a later item).
    pub bookmarks: [Option<CanvasView>; BOOKMARK_SLOTS],
    /// Next slot `Ctrl+B` will write, cycling 1..=5.
    pub bookmark_next: usize,
    /// Nodes a pending purge would remove — the confirm step's payload.
    pub purge_confirm: Option<Vec<u64>>,
    /// Transient canvas messages, newest last.
    pub toasts: Vec<CanvasToast>,
    /// In-flight Ctrl-drag slash cut: the path drawn so far, world space.
    pub cut_path: Option<Vec<[f32; 2]>>,
    /// Node whose context menu is open.
    pub node_menu: Option<u64>,
    /// The add-node palette, when open.
    pub palette: Option<PaletteState>,
    /// Find-in-graph overlay (session-only).
    pub find: Option<FindState>,
    /// The `?` cheat sheet is open. Session-only, like `find` — a reference
    /// card is not document state.
    pub cheat_sheet: bool,
    /// Breakpoints: document node -> **armed**. `false` is the mockup's
    /// disabled state — a mark you keep but do not want to stop at, which is
    /// why a disabled breakpoint is a state and not a deletion.
    ///
    /// Persisted in the per-user sidecar beside the watches (GS-4). It moved
    /// there the moment the marks could actually do something: before the
    /// interpreter could pause, remembering one across a reload would have
    /// been durability we could not honour.
    ///
    /// Ordered, because it goes to disk: a `HashSet`'s iteration order would
    /// rewrite the file on every save with the same content in a new order.
    pub breakpoints: std::collections::BTreeMap<u64, bool>,
    /// Per-tab navigation back-stack for PageUp. Session-only: which graph you
    /// descended *from* is a property of this browsing session, not of the
    /// asset.
    pub nav_back: Vec<String>,
    /// An arrow-key nudge in progress. Holding an arrow key fires the OS
    /// auto-repeat dozens of times a second; each repeat moves the nodes, but
    /// the *whole* hold must land as one undo entry, or a two-second press
    /// costs eighty presses of Ctrl+Z to undo. Opened on the first press,
    /// extended by every repeat, committed when the key comes up.
    pub nudge: Option<Nudge>,
    /// Empty-canvas context menu: (world, screen) of the right-click that
    /// opened it. Both are kept because "Add Node…" needs the world point to
    /// place the node and the screen point to anchor the palette, and the
    /// click that supplied them is long gone by the time the row is chosen.
    pub canvas_menu: Option<([f32; 2], [f32; 2])>,
    /// Frames drawn, for round-robin budgets (preview slots today).
    pub frame: u64,
    /// Set when a graph opened with no remembered view: the first draw frames
    /// all its content instead of landing at the origin at 100%.
    pub frame_all_on_open: bool,
    /// The variables side strip (45-A P6c). Session-only.
    pub vars: VarPanel,
    /// A node the panel just framed, and when — it flashes for a moment so
    /// the eye lands on it after the view moves (GS-2 locate).
    pub flash: Option<(u64, Instant)>,
    /// The instance this tab is bound to, picked explicitly from the LIVE
    /// chip (`Entity::to_bits`). `None` = follow the selection, which is the
    /// baseline rule. Session-only: entity handles do not survive a reload.
    pub exec_bind: Option<u64>,
    /// The chip's dropdown is open.
    pub exec_picker: bool,
    /// Pinned watches (GS-3). Editor annotations, not run state — they live
    /// in the per-user sidecar and survive play/stop.
    pub watches: Vec<Watch>,
    /// Resume / Step / Stop, raised by the banner or a shortcut and taken by
    /// the host, which owns the world the bound instance lives in (GS-4).
    /// Session-only and one-shot: a command is an event.
    pub debug_request: Option<crate::engine::editor::graph_exec_viz::DebugRequest>,
    /// The custom-event payload band's draft/confirm state (GS-1).
    /// Session-only, like `vars`.
    pub payload: PayloadPanel,
    /// Which node library this document authors against (Task 41). Derived
    /// from the file extension at open; never changes over a document's life.
    pub domain: GraphDomain,
    /// Domain-compiler refusals — for an `.animgraph`, `compile_anim_graph`'s
    /// error, anchored to the node it names. Editor-side rather than a new
    /// `GraphError` variant because that set is closed by design ruling; the
    /// panel folds these into the same anchored-error UI (badge, count chip,
    /// F8 cycle).
    pub domain_errors: Vec<DomainError>,
    /// The open rule peek/scope, when a transition's embedded rule is being
    /// edited (Task 41 ticket 05). Session-only — which rule you are inside
    /// is a property of this browsing session, like `nav_back`.
    pub rule_scope: Option<RuleScope>,
    /// The entity this tab previews on, picked explicitly from the strip's
    /// PREVIEW chip (`Entity::to_bits`). `None` = follow the selection —
    /// the LIVE chip's binding ladder, reused (Task 41 ticket 06).
    /// Session-only: entity handles do not survive a reload.
    pub anim_bind: Option<u64>,
    /// The PREVIEW chip's entity picker is open.
    pub anim_picker: bool,
    /// Parameter edits the preview strip recorded this frame, drained by the
    /// host onto the bound runtime's blackboard after the UI. Runtime-only
    /// writes: never document state, never undo entries.
    pub anim_edits: Vec<crate::engine::editor::anim_preview::AnimParamEdit>,
}

/// The graph editor is one product over several node libraries; the domain
/// says which library, palette and validation profile a document uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GraphDomain {
    /// `.graph` / `.subgraph` — the script library.
    #[default]
    Script,
    /// `.animgraph` — the animation state-machine library.
    Animation,
    /// The projection of one transition's embedded rule region (ticket 05) —
    /// the domain a peek's child editor runs as. Never comes from a path:
    /// only [`GraphEditorState::open_rule_scope`] constructs it. `owner` is
    /// the transition's id in the parent document, kept so the projection's
    /// refusals are phrased exactly like the machine compiler's.
    AnimationRule { owner: u64 },
}

impl GraphDomain {
    pub fn of_path(path: &str) -> Self {
        if path.ends_with(".animgraph") {
            GraphDomain::Animation
        } else {
            GraphDomain::Script
        }
    }

    /// The machine canvas specifically — chips, transitions, flow wires.
    pub fn is_animation(self) -> bool {
        self == GraphDomain::Animation
    }

    /// Any animation canvas — machine or embedded rule. What gates the
    /// script-only affordances (subgraph rows, collapse-to-subgraph) and
    /// selects the Float/Bool/Trigger variable types.
    pub fn is_animation_family(self) -> bool {
        matches!(
            self,
            GraphDomain::Animation | GraphDomain::AnimationRule { .. }
        )
    }

    /// This domain's compile refusals for `doc`, anchored. Scripts answer
    /// none: their compile errors surface through the interpreter's own path.
    /// `path` is the document's content-relative key — it seeds the nested
    /// compiler's cycle guard, and nested `.animgraph` references resolve
    /// from the content root on disk (the saved file is the unit of truth
    /// here, exactly as it is for the runtime's plan cache — a dirty child
    /// tab shows in the host once it saves).
    pub fn compile_errors(self, doc: &GraphDoc, path: &str) -> Vec<DomainError> {
        match self {
            GraphDomain::Script => Vec::new(),
            GraphDomain::Animation => {
                let load = |rel: &str| {
                    crate::engine::node_graph::load_graph(
                        &std::path::Path::new("content").join(rel),
                    )
                    .ok()
                };
                match crate::engine::animation::graph::compile_anim_graph_with(doc, path, &load) {
                    Ok(_) => Vec::new(),
                    Err(message) => {
                        let node = anchor_anim_refusal(doc, &message);
                        // A refusal naming a node inside the transition's
                        // rule ("rule node 3") carries the region-local id
                        // too, so F8 can descend into the peek.
                        let region_node = anchor_rule_refusal(&message);
                        vec![DomainError { node, region_node, message }]
                    }
                }
            }
            // The projection compiles as a bare rule: same code path, same
            // message shapes as the machine compiler, but "rule node {id}"
            // here *is* a top-level node of the projection, so it anchors
            // directly.
            GraphDomain::AnimationRule { owner } => {
                use crate::engine::animation::graph::plan::{
                    compile_parameters, compile_rule_region,
                };
                let compile = || -> Result<(), String> {
                    let params = compile_parameters(doc)?;
                    let region = GraphRegion {
                        nodes: doc.nodes.clone(),
                        edges: doc.edges.clone(),
                    };
                    compile_rule_region(&region, owner, &params).map(|_| ())
                };
                match compile() {
                    Ok(()) => Vec::new(),
                    Err(message) => {
                        let node = anchor_rule_refusal(&message);
                        vec![DomainError { node, region_node: None, message }]
                    }
                }
            }
        }
    }
}

/// A request to open another graph document as a tab, raised by descend
/// gestures (double-click / PageDown on a node that references a file) and
/// fulfilled by the host. `back` is the file chain that led there — the host
/// seeds the opened tab's `nav_back` with it (when non-empty), which is what
/// the breadcrumb band renders and PageUp walks. A plain jump (an exec-cycle
/// crumb) carries an empty chain and leaves the target's session nav alone.
#[derive(Debug, Clone, PartialEq)]
pub struct GraphOpenRequest {
    /// Content-relative path of the document to open.
    pub path: String,
    /// Ancestor chain for the opened tab, outermost first.
    pub back: Vec<String>,
}

impl GraphOpenRequest {
    /// A jump with no descent chain.
    pub fn jump(path: String) -> Self {
        Self { path, back: Vec::new() }
    }
}

/// One domain-compiler refusal: a `String` a person can act on (the animation
/// compiler's posture), plus the node it is about when the text names one.
#[derive(Debug, Clone, PartialEq)]
pub struct DomainError {
    /// `None` = document-level: it lists in the compiler-row popover.
    pub node: Option<u64>,
    /// Region-local node id when the refusal names a node *inside* `node`'s
    /// embedded region ("transition 5: rule node 3 …"). F8 descends: it opens
    /// the rule peek on `node` and flashes this node in it (ticket 05).
    pub region_node: Option<u64>,
    pub message: String,
}

/// Resolve which node an animation-compiler refusal is about, from the
/// refusal text itself — the compiler anchors its messages by convention
/// ("state '<name>': …", "transition <id>: …"), and this is the editor-side
/// half of that contract.
pub fn anchor_anim_refusal(doc: &GraphDoc, msg: &str) -> Option<u64> {
    use crate::engine::animation::graph::plan::{
        ANIM_ENTRY_TYPE_ID, ANIM_PLAY_ONCE_TYPE_ID, ANIM_STATE_TYPE_ID, ANIM_TRANSITION_TYPE_ID,
    };
    // The display name the compiler used: node title, or its typed fallback.
    let named = |type_id: &str, fallback: &str, name: &str| -> Option<u64> {
        doc.nodes
            .iter()
            .filter(|n| n.type_id == type_id)
            .find(|n| match &n.title {
                Some(t) => t == name,
                None => format!("{fallback} {}", n.id) == name,
            })
            .map(|n| n.id)
    };
    let quoted = |prefix: &str| -> Option<&str> {
        let rest = msg.strip_prefix(prefix)?.strip_prefix('\'')?;
        rest.split('\'').next()
    };
    if let Some(rest) = msg.strip_prefix("transition ") {
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        let id: u64 = digits.parse().ok()?;
        return doc
            .nodes
            .iter()
            .any(|n| n.id == id && n.type_id == ANIM_TRANSITION_TYPE_ID)
            .then_some(id);
    }
    if let Some(name) = quoted("state ") {
        return named(ANIM_STATE_TYPE_ID, "State", name);
    }
    if let Some(name) = quoted("play-once slot ") {
        return named(ANIM_PLAY_ONCE_TYPE_ID, "Slot", name);
    }
    // The ENTRY-wiring family ("the ENTRY node is not wired to a state", …)
    // anchors on the ENTRY node itself when there is exactly one.
    if msg.contains("ENTRY node") {
        let mut entries = doc.nodes.iter().filter(|n| n.type_id == ANIM_ENTRY_TYPE_ID);
        if let (Some(e), None) = (entries.next(), entries.next()) {
            return Some(e.id);
        }
    }
    None
}

/// The region-local node a refusal names, when it names one — the compiler's
/// rule messages read "transition {tid}: rule node {id} …". Paired with
/// [`anchor_anim_refusal`]'s owner id, this is what lets F8 descend into the
/// peek rather than stopping at the chip (ticket 05).
pub fn anchor_rule_refusal(msg: &str) -> Option<u64> {
    let rest = msg.split("rule node ").nth(1)?;
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

// ---------------------------------------------------------------------------
// Rule scope (Task 41 ticket 05): peek-overlay editing of an embedded rule.
// ---------------------------------------------------------------------------

/// One open rule scope: a transition's embedded rule region, projected into a
/// child editor state the drawing layer runs like any other canvas.
///
/// The child's document is a *projection* — region nodes/edges as top-level
/// content plus a copy of the parent's variables so `var_get` resolves. The
/// child records edits into its own stack exactly as the machinery always
/// does; [`GraphEditorState::drain_rule_scope`] then moves them onto the
/// **parent's** stack (wrapped [`GraphEdit::InRegion`], applied to the real
/// region) every frame, which is what makes a transition and its rule one
/// undo history. The child's stack is therefore always empty between frames;
/// undo/redo while scoped route to the parent and rebuild the projection.
pub struct RuleScope {
    /// The transition node (parent document id) whose rule this is.
    pub owner: u64,
    /// `false` = peek overlay over the dimmed machine (mockup 3b); `true` =
    /// promoted (⤢) to the full canvas with a breadcrumb (3a's reading).
    pub full: bool,
    /// The projection editor. Boxed: a `GraphEditorState` is large, and the
    /// scope is usually absent.
    pub child: Box<GraphEditorState>,
    /// A RESULT sink seeded into the projection of an absent/empty region,
    /// **not yet recorded**: it is recorded lazily in front of the author's
    /// first real edit, so peeking into an always-true transition never
    /// dirties the document. Holds the node as seeded so the deferred
    /// `AddNode` replays it exactly.
    pub seed: Option<NodeInst>,
}

/// The registry a rule projection authors against — built once. The palette
/// listing exactly this registry is the placement gate: machine nodes are not
/// in it, so they cannot land inside a rule, and the rule nodes are not in
/// the machine registry, so they cannot land on the machine canvas.
pub fn rule_scope_registry() -> &'static NodeRegistry {
    static REG: std::sync::OnceLock<NodeRegistry> = std::sync::OnceLock::new();
    REG.get_or_init(crate::engine::animation::graph::anim_rule_registry)
}

/// Where the seeded RESULT lands on an empty rule canvas: to the right, where
/// the mockup draws the sink, with room to build the condition leftward.
const RULE_SEED_POS: [f32; 2] = [260.0, 60.0];

fn rule_seed_node(id: u64) -> NodeInst {
    NodeInst {
        id,
        type_id: crate::engine::animation::graph::plan::ANIM_RULE_RESULT_TYPE_ID.to_string(),
        type_version: 1,
        position: RULE_SEED_POS,
        properties: Default::default(),
        subgraph: None,
        tint: None,
        title: None,
    }
}

/// Does this edit only touch `doc.variables`? Those pass through to the
/// parent unwrapped — declarations live on the document, not in a region —
/// and the projection's copy stays in step because both sides applied the
/// same edit.
fn edit_targets_variables(edit: &GraphEdit) -> bool {
    match edit {
        GraphEdit::AddVariable(_)
        | GraphEdit::RemoveVariable { .. }
        | GraphEdit::RenameVariable { .. }
        | GraphEdit::RetypeVariable { .. }
        | GraphEdit::SetVariableDefault { .. }
        | GraphEdit::SetVariableGroup { .. }
        | GraphEdit::ReorderVariable { .. } => true,
        GraphEdit::Composite { edits, .. } => {
            !edits.is_empty() && edits.iter().all(edit_targets_variables)
        }
        _ => false,
    }
}

/// Can this edit live inside a region — nodes, edges and properties only?
/// A region has no comments, groups or nested regions to hold anything else.
fn edit_is_region_safe(edit: &GraphEdit) -> bool {
    match edit {
        GraphEdit::AddNode(_)
        | GraphEdit::Connect(_)
        | GraphEdit::Disconnect { .. }
        | GraphEdit::MoveNodes { .. }
        | GraphEdit::SetProperty { .. } => true,
        GraphEdit::RemoveNodes { comments, regions, .. } => {
            comments.is_empty() && regions.is_empty()
        }
        GraphEdit::Paste { regions, .. } => regions.is_empty(),
        GraphEdit::Composite { edits, .. } => edits.iter().all(edit_is_region_safe),
        _ => false,
    }
}

/// Find hits inside embedded rule regions: `(owner transition, region-local
/// node)` for every rule node matching `find`, in document order. What lets
/// Ctrl+F reach nodes the canvas is not showing (spec story 22) — activating
/// one opens the peek on its transition.
pub fn region_find_matches(doc: &GraphDoc, find: &FindState) -> Vec<(u64, u64)> {
    use crate::engine::animation::graph::plan::ANIM_TRANSITION_TYPE_ID;
    let mut out = Vec::new();
    for n in doc.nodes.iter().filter(|n| n.type_id == ANIM_TRANSITION_TYPE_ID) {
        let Some(region) = doc.regions.get(&n.id) else { continue };
        for rn in &region.nodes {
            // The display name a user would type: the title if set, else the
            // parameter a `var_get` reads, else nothing but the type id.
            let title = rn.title.clone().unwrap_or_else(|| {
                match rn.properties.get(crate::engine::node_graph::VAR_PROP) {
                    Some(PropValue::Str(s)) => s.clone(),
                    _ => String::new(),
                }
            });
            if find.matches(&title, &rn.type_id) {
                out.push((n.id, rn.id));
            }
        }
    }
    out
}

impl GraphEditorState {
    /// The transition to descend into via the keyboard (`PageDown`): the
    /// primary selected node, when it is a transition on a machine canvas.
    pub fn rule_descend_target(&self) -> Option<u64> {
        use crate::engine::animation::graph::plan::ANIM_TRANSITION_TYPE_ID;
        if !self.domain.is_animation() {
            return None;
        }
        let id = self.primary.or_else(|| self.selection.iter().copied().next())?;
        (self.doc.node(id)?.type_id == ANIM_TRANSITION_TYPE_ID).then_some(id)
    }

    /// Open (or refocus) the rule peek on `owner`. Answers `false` when
    /// `owner` is not a transition of an animation document. `registry` is
    /// the parent's — needed to settle a previously open scope first.
    pub fn open_rule_scope(&mut self, owner: u64, registry: &NodeRegistry) -> bool {
        use crate::engine::animation::graph::plan::ANIM_TRANSITION_TYPE_ID;
        if !self.domain.is_animation() {
            return false;
        }
        if !self
            .doc
            .node(owner)
            .is_some_and(|n| n.type_id == ANIM_TRANSITION_TYPE_ID)
        {
            return false;
        }
        if self.rule_scope.as_ref().is_some_and(|s| s.owner == owner) {
            return true;
        }
        // Switching transitions: settle the old peek's edits first.
        self.close_rule_scope(registry);
        // The machine's transient surfaces close under the peek, and the
        // transition selects so its card unfolds and its states light.
        self.cancel_interactions();
        self.palette = None;
        self.find = None;
        self.node_menu = None;
        self.canvas_menu = None;
        self.wire_menu = None;
        self.annotation_menu = None;
        self.select_only(owner);

        let region = self.doc.regions.get(&owner).cloned().unwrap_or_default();
        let mut doc = GraphDoc {
            realm: crate::engine::node_graph::GraphRealm::Client,
            nodes: region.nodes,
            edges: region.edges,
            variables: self.doc.variables.clone(),
            ..GraphDoc::default()
        };
        let mut seed = None;
        if doc.nodes.is_empty() {
            let n = rule_seed_node(0);
            doc.nodes.push(n.clone());
            seed = Some(n);
        }
        let mut child = GraphEditorState::from_doc(
            format!("{}#rule:{owner}", self.path),
            doc,
            GraphDomain::AnimationRule { owner },
            rule_scope_registry(),
        );
        child.frame_all_on_open = true;
        self.rule_scope = Some(RuleScope { owner, full: false, child: Box::new(child), seed });
        true
    }

    /// Close the scope, settling anything it still holds: in-flight child
    /// gestures revert (Rule 3 — closing is not committing), already-recorded
    /// child edits drain onto the parent history.
    pub fn close_rule_scope(&mut self, registry: &NodeRegistry) {
        if let Some(scope) = &mut self.rule_scope {
            scope.child.cancel_interactions();
        }
        self.drain_rule_scope(registry);
        self.rule_scope = None;
    }

    /// Move every edit the child recorded onto the parent history: variable
    /// edits pass through as themselves, node/edge/property edits wrap as
    /// [`GraphEdit::InRegion`] (recording the pending RESULT seed first, the
    /// moment the region stops being look-only). Called once per frame by the
    /// panel and before any parent-history operation.
    pub fn drain_rule_scope(&mut self, registry: &NodeRegistry) {
        let Some(mut scope) = self.rule_scope.take() else { return };
        let edits = scope.child.stack.take_edits();
        let mut recorded = false;
        for edit in edits {
            if edit_targets_variables(&edit) {
                edit.apply(&mut self.doc);
                self.stack.record(edit);
                recorded = true;
            } else if edit_is_region_safe(&edit) {
                if let Some(seed) = scope.seed.take() {
                    let planted =
                        GraphEdit::InRegion { owner: scope.owner, edit: Box::new(GraphEdit::AddNode(seed)) };
                    planted.apply(&mut self.doc);
                    self.stack.record(planted);
                }
                let wrapped = GraphEdit::InRegion { owner: scope.owner, edit: Box::new(edit) };
                wrapped.apply(&mut self.doc);
                self.stack.record(wrapped);
                recorded = true;
            } else {
                // Annotations cannot live in a region; the UI gates them out,
                // and anything that slips through reverts on the projection
                // so the two documents never disagree.
                edit.revert(&mut scope.child.doc);
            }
        }
        self.rule_scope = Some(scope);
        if recorded {
            self.after_edit(registry);
        }
    }

    /// Rebuild the projection from the parent document — after a parent-side
    /// undo/redo rewrote the region under it. Closes the scope when the owner
    /// transition itself is gone (the undo took the transition, and a peek
    /// into nothing has nothing to show).
    pub fn rebuild_rule_scope(&mut self) {
        use crate::engine::animation::graph::plan::ANIM_TRANSITION_TYPE_ID;
        let Some(mut scope) = self.rule_scope.take() else { return };
        if !self
            .doc
            .node(scope.owner)
            .is_some_and(|n| n.type_id == ANIM_TRANSITION_TYPE_ID)
        {
            return; // owner gone — scope stays closed
        }
        let region = self.doc.regions.get(&scope.owner).cloned().unwrap_or_default();
        scope.child.cancel_interactions();
        scope.child.doc.nodes = region.nodes;
        scope.child.doc.edges = region.edges;
        scope.child.doc.variables = self.doc.variables.clone();
        scope.seed = None;
        if scope.child.doc.nodes.is_empty() {
            let n = rule_seed_node(0);
            scope.child.doc.nodes.push(n.clone());
            scope.seed = Some(n);
        }
        scope.child.stack = GraphEditStack::new();
        scope.child.prune_selection();
        scope.child.after_edit(rule_scope_registry());
        self.rule_scope = Some(scope);
    }

    /// F8 landing on an error inside a rule: open the peek on the owner and
    /// put the eye on the named node in it.
    pub fn open_rule_scope_at(
        &mut self,
        owner: u64,
        region_node: u64,
        registry: &NodeRegistry,
    ) -> bool {
        if !self.open_rule_scope(owner, registry) {
            return false;
        }
        if let Some(scope) = &mut self.rule_scope {
            if scope.child.doc.node(region_node).is_some() {
                scope.child.select_only(region_node);
                scope.child.flash = Some((region_node, Instant::now()));
            }
        }
        true
    }
}

/// The add-node palette's session state.
#[derive(Debug, Clone)]
pub struct PaletteState {
    /// Where a picked node lands, world space.
    pub world: [f32; 2],
    /// Screen anchor for the popover.
    pub screen: [f32; 2],
    pub search: String,
    /// Highlighted row, for Up/Down + Enter.
    pub cursor: usize,
    /// Present when the palette was opened by releasing a pin drag: the type
    /// to filter by, and the pin a pick would wire back to.
    pub from: Option<PaletteDragSource>,
    /// Grab focus on the first frame only.
    pub first_frame: bool,
}

/// The pin a type-filtered palette came off.
#[derive(Debug, Clone)]
pub struct PaletteDragSource {
    pub node: u64,
    pub pin: String,
    /// The source pin is an output, so a picked node needs a matching input.
    pub output: bool,
    pub ty: PinType,
    /// The source pin's label, for the auto-connect name tie-break.
    pub label: String,
}

/// Find-in-graph: a filter over node titles/type ids plus a cycle cursor.
#[derive(Debug, Clone, Default)]
pub struct FindState {
    pub query: String,
    pub cursor: usize,
    pub first_frame: bool,
}

impl FindState {
    /// Does this node match? Substring, case-insensitive, over the display
    /// title and the type id — the two things a user would think to type.
    pub fn matches(&self, title: &str, type_id: &str) -> bool {
        let q = self.query.trim().to_lowercase();
        if q.is_empty() {
            return true;
        }
        title.to_lowercase().contains(&q) || type_id.to_lowercase().contains(&q)
    }

    pub fn active(&self) -> bool {
        !self.query.trim().is_empty()
    }
}

/// The prototype's cap on the cut path — a slash is a gesture, not a drawing,
/// and an unbounded buffer would make the preview's per-segment test quadratic
/// in mouse samples.
pub const CUT_PATH_MAX: usize = 40;

/// A transient canvas message. Every gesture that changes something the user
/// cannot immediately see says so — "Broke 3 links", "Pasted 6 nodes".
#[derive(Debug, Clone)]
pub struct CanvasToast {
    pub text: String,
    pub at: Instant,
}

/// How long a toast stays up before it has fully faded.
pub const TOAST_MS: u128 = 1800;

/// How many view bookmarks a graph keeps.
pub const BOOKMARK_SLOTS: usize = 5;

/// The align & distribute operations offered for 3+ selected nodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlignMode {
    Left,
    Right,
    Top,
    Bottom,
    /// Line the nodes up on a shared vertical centre line (they end up in a
    /// column). Distinct from Left/Right, which use an edge.
    CenterHorizontally,
    /// Shared horizontal centre line (they end up in a row).
    CenterVertically,
    DistributeHorizontally,
    DistributeVertically,
}

impl AlignMode {
    /// Does this mode spread nodes out, rather than line them up?
    ///
    /// The distinction is the whole reason the two have different minimums:
    /// distributing means "equalise the gaps *between*", which needs a middle
    /// node to move, while aligning two nodes is a perfectly ordinary thing to
    /// want. One shared `< 3` guard made two-node align a silent no-op on a
    /// valid gesture.
    pub fn is_distribute(self) -> bool {
        matches!(
            self,
            AlignMode::DistributeHorizontally | AlignMode::DistributeVertically
        )
    }

    /// Smallest selection this mode can act on.
    pub fn min_nodes(self) -> usize {
        if self.is_distribute() {
            3
        } else {
            2
        }
    }

    pub const ALL: [AlignMode; 8] = [
        AlignMode::Left,
        AlignMode::Right,
        AlignMode::Top,
        AlignMode::Bottom,
        AlignMode::CenterHorizontally,
        AlignMode::CenterVertically,
        AlignMode::DistributeHorizontally,
        AlignMode::DistributeVertically,
    ];

    pub fn label(self) -> &'static str {
        match self {
            AlignMode::Left => "Align Left",
            AlignMode::Right => "Align Right",
            AlignMode::Top => "Align Top",
            AlignMode::Bottom => "Align Bottom",
            AlignMode::CenterHorizontally => "Align Centers Horizontally",
            AlignMode::CenterVertically => "Align Centers Vertically",
            AlignMode::DistributeHorizontally => "Distribute Horizontally",
            AlignMode::DistributeVertically => "Distribute Vertically",
        }
    }

    fn horizontal(self) -> bool {
        matches!(self, AlignMode::DistributeHorizontally)
    }
}

/// Which edge or corner of an annotation a resize drag grabbed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizeHandle {
    Left,
    Right,
    Top,
    Bottom,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl ResizeHandle {
    /// Every handle, corners first so a corner wins a hit-test over the two
    /// edges it overlaps.
    pub const ALL: [ResizeHandle; 8] = [
        ResizeHandle::TopLeft,
        ResizeHandle::TopRight,
        ResizeHandle::BottomLeft,
        ResizeHandle::BottomRight,
        ResizeHandle::Left,
        ResizeHandle::Right,
        ResizeHandle::Top,
        ResizeHandle::Bottom,
    ];

    pub fn moves_left(self) -> bool {
        matches!(self, Self::Left | Self::TopLeft | Self::BottomLeft)
    }
    pub fn moves_right(self) -> bool {
        matches!(self, Self::Right | Self::TopRight | Self::BottomRight)
    }
    pub fn moves_top(self) -> bool {
        matches!(self, Self::Top | Self::TopLeft | Self::TopRight)
    }
    pub fn moves_bottom(self) -> bool {
        matches!(self, Self::Bottom | Self::BottomLeft | Self::BottomRight)
    }
}

/// In-flight annotation resize. Absolute-from-origin like the move drags, so
/// the gesture cannot drift, and recorded as one `ResizeAnnotation` on release.
pub struct AnnotationResize {
    pub target: Annotation,
    pub handle: ResizeHandle,
    pub origin_world: [f32; 2],
    /// Rect at grab time.
    pub rect0: [f32; 4],
    /// Smallest height the content allows (comments never auto-shrink below
    /// their wrapped text); 0 for groups.
    pub min_h: f32,
}

/// Smallest an annotation may be dragged to, world units.
pub const ANNOTATION_MIN_W: f32 = 80.0;
pub const ANNOTATION_MIN_H: f32 = 40.0;

/// A held arrow-key nudge, accumulating into one undo entry.
///
/// Deliberately the same shape as [`NodeDrag`]: pre-gesture positions so the
/// movement is absolute rather than incremental (no drift, and Escape can put
/// them back), plus the running total that gets recorded once at the end.
#[derive(Debug, Clone)]
pub struct Nudge {
    /// Node id + position before the first key press.
    pub originals: Vec<(u64, [f32; 2])>,
    /// Total offset applied so far.
    pub delta: [f32; 2],
}

/// In-flight node drag: original positions so live movement is absolute
/// (no drift) and the net delta is recorded once on release.
pub struct NodeDrag {
    pub origin_world: [f32; 2],
    pub originals: Vec<(u64, [f32; 2])>,
    /// An edit already applied to the doc but **not yet recorded**, waiting
    /// to be folded into this gesture's single undo entry. Grabbing a wire's
    /// midpoint inserts a reroute and drags it: that is one gesture, so it is
    /// one entry. Cancelling reverts it instead of recording it.
    pub pending: Option<GraphEdit>,
    /// Comments anchored to the dragged nodes: index + rect origin at drag
    /// start, so the note tracks its node live instead of snapping on release.
    pub anchored: Vec<(usize, [f32; 2])>,
}

/// How a marquee gesture combines with the existing selection. Captured when
/// the drag starts (AUDIT ruling #2: Windows keys — ⇧ adds, ⌥→Alt subtracts).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MarqueeMode {
    #[default]
    Replace,
    Add,
    Subtract,
}

/// An inline property edit in progress: the value as it stood *before* the
/// gesture, so the whole drag commits as one reversible edit.
#[derive(Debug, Clone)]
/// The in-flight coalesced edit of a variable's default.
pub struct VarDefaultEdit {
    pub slug: String,
    pub old: Option<PropValue>,
}

pub struct PropEdit {
    pub node: u64,
    pub key: String,
    pub old: Option<PropValue>,
}

/// In-flight connection drag from a source pin toward the pointer.
pub struct ConnectDrag {
    pub from_node: u64,
    pub from_pin: String,
    /// True if the grabbed pin is an output (wire flows out); false = input.
    pub from_output: bool,
}

/// In-flight drag of a comment or group (P7). Groups carry captured member
/// nodes; comments leave `captured` empty. Absolute-from-origin movement (no
/// drift), recorded as one Move edit on release.
pub struct AnnotationDrag {
    pub is_group: bool,
    pub index: usize,
    pub origin_world: [f32; 2],
    /// Rect origin at drag start.
    pub rect_min0: [f32; 2],
    /// Captured member nodes (group only): id + start position.
    pub captured: Vec<(u64, [f32; 2])>,
}

/// The variables side strip's own session state (45-A P6c).
///
/// Grouped rather than flattened into [`GraphEditorState`] because none of it
/// is document data: it is one panel's open/selected/in-flight-gesture state,
/// and keeping it together makes "does the panel own this?" answerable by
/// looking at one struct. Session-only, like `find` and the cheat sheet.
#[derive(Debug, Clone, Default)]
pub struct VarPanel {
    /// The strip is expanded. Collapsed, it is a narrow rail with a caret.
    pub open: bool,
    /// Slug whose detail block (rename / retype / default / delete) is open.
    /// At most one at a time — an accordion, not a grid of open forms.
    pub selected: Option<String>,
    /// The "new variable" draft row, when the `+` affordance is armed.
    pub new_var: Option<NewVarDraft>,
    /// Rename buffer for `selected`, so typing does not commit per keystroke.
    pub rename_buf: Option<String>,
    /// A drop landed on the canvas and is waiting for the Get/Set choice.
    pub drop: Option<VarDrop>,
    /// A destructive/lossy change waiting on confirmation.
    pub confirm: Option<VarConfirm>,
    /// The list filter (GS-2). Session state: it dims the canvas while it is
    /// set, exactly like find-in-graph, and clears on Esc.
    pub filter: String,
    /// Collapsed group headers, by group name.
    pub collapsed: BTreeSet<String>,
    /// Per-slug locate cursor, so repeat clicks cycle a variable's uses
    /// instead of re-framing the first one forever.
    pub locate: BTreeMap<String, usize>,
    /// In-flight "New group…" name entry from the `+` menu.
    pub new_group: Option<String>,
}

/// The add-variable draft: what the row's fields hold before `Add` is pressed.
#[derive(Debug, Clone, Default)]
pub struct NewVarDraft {
    pub name: String,
    /// Index into the panel's offered scalar types.
    pub ty: usize,
    pub array: bool,
    pub first_frame: bool,
    /// The draft committed or cancelled this frame and should be dropped. Set
    /// by the block that draws it, read by the panel after — the draft lives
    /// inside a scrolled list, so it cannot take itself down mid-layout.
    pub done: bool,
}

/// A variable row dropped on the canvas: where it landed, awaiting Get/Set.
#[derive(Debug, Clone)]
pub struct VarDrop {
    pub slug: String,
    /// Display label, for the two choice rows.
    pub label: String,
    /// World position of the drop — where the node will land.
    pub world: [f32; 2],
    /// Screen anchor for the two-choice popup.
    pub screen: [f32; 2],
}

/// The custom-event payload band's own session state (GS-1).
///
/// Grouped like [`VarPanel`], and for the same reason: none of it is document
/// data — it is one band's in-flight name entry and its pending confirmation.
#[derive(Debug, Clone, Default)]
pub struct PayloadPanel {
    /// In-flight name entry, either for the "+ field" ghost row or for a
    /// rename of an existing row.
    pub draft: Option<PayloadDraft>,
    /// A change with readers, waiting on confirmation.
    pub confirm: Option<PayloadConfirm>,
}

/// An in-flight payload-field name entry.
///
/// The widget only ever *reports* — it sets `submitted` and the panel decides
/// what that means, because the decision (straight through, or a confirmation)
/// needs the cross-document resolver the drawing pass does not carry.
#[derive(Debug, Clone)]
pub struct PayloadDraft {
    pub node: u64,
    /// `None` = the "+ field" ghost row; `Some(slug)` = renaming that field.
    pub slug: Option<String>,
    pub name: String,
    /// Grab the keyboard on the first frame only.
    pub first_frame: bool,
    /// The field has held focus at least once. Focus arrives a frame after it
    /// is requested, so "lost focus" only means anything after that: without
    /// this the draft cancelled itself on the frame after it opened.
    pub seen_focus: bool,
    /// Enter was pressed — the panel picks it up after the draw.
    pub submitted: bool,
}

/// A payload change that would break readers, waiting on confirmation. Both
/// arms carry the counts, because the counts are the reason to ask.
#[derive(Debug, Clone)]
pub enum PayloadConfirm {
    Remove {
        node: u64,
        slug: String,
        readers: usize,
        graphs: usize,
    },
    Rename {
        node: u64,
        slug: String,
        /// The name as typed; the slug is derived on commit.
        name: String,
        readers: usize,
        graphs: usize,
    },
}

impl PayloadConfirm {
    pub fn slug(&self) -> &str {
        match self {
            PayloadConfirm::Remove { slug, .. } | PayloadConfirm::Rename { slug, .. } => slug,
        }
    }
    pub fn counts(&self) -> (usize, usize) {
        match self {
            PayloadConfirm::Remove { readers, graphs, .. }
            | PayloadConfirm::Rename { readers, graphs, .. } => (*readers, *graphs),
        }
    }
}

/// One pinned watch: a pin whose value is shown on the canvas as a chip.
///
/// The identity is `(node, pin, output)`; everything else is what the live
/// layer has learned about it. `last` survives a stop — in edit mode the chip
/// renders dashed with the last run's value, which is the whole reason a watch
/// is an editor annotation rather than run state.
#[derive(Debug, Clone, PartialEq)]
pub struct Watch {
    pub node: u64,
    pub pin: String,
    pub output: bool,
    /// Last value seen, formatted the way the console spells it.
    pub last: Option<String>,
    /// When `last` last *changed*. Session-only, so a restored watch reads as
    /// stale-unknown rather than pretending it updated when the editor opened.
    pub changed_at: Option<Instant>,
}

impl Watch {
    pub fn new(node: u64, pin: &str, output: bool) -> Self {
        Self { node, pin: pin.to_string(), output, last: None, changed_at: None }
    }

    /// Is this the watch on `(node, pin, output)`?
    pub fn is(&self, node: u64, pin: &str, output: bool) -> bool {
        self.node == node && self.pin == pin && self.output == output
    }

    /// Take a fresh value, remembering *when it changed* rather than when it
    /// was read: a value that keeps arriving unchanged is exactly what the
    /// staleness tag is about.
    pub fn observe(&mut self, value: &str) {
        if self.last.as_deref() != Some(value) {
            self.last = Some(value.to_string());
            self.changed_at = Some(Instant::now());
        }
    }

    /// Seconds since the value last changed, once past `after`. `None` while
    /// it is fresh, or when nothing has ever arrived.
    pub fn stale_for(&self, after: f32) -> Option<f32> {
        let secs = self.changed_at?.elapsed().as_secs_f32();
        (secs > after).then_some(secs)
    }
}

/// How long a watched value may sit unchanged before the chip dims and starts
/// reporting its age (DESIGN-graphscripting, Surface 3).
pub const WATCH_STALE_SECS: f32 = 3.0;

/// Longest watched value a chip shows before eliding.
pub const WATCH_CHARS: usize = 24;

/// A watch chip's text: elided to [`WATCH_CHARS`], em-dash when nothing has
/// arrived yet. The full value goes in the tooltip.
pub fn watch_chip_text(value: Option<&str>) -> String {
    match value {
        None => "\u{2014}".to_string(),
        Some(v) if v.chars().count() <= WATCH_CHARS => v.to_string(),
        Some(v) => {
            let head: String = v.chars().take(WATCH_CHARS).collect();
            format!("{head}\u{2026}")
        }
    }
}

/// A pending confirmation. Both arms carry the usage count, because the count
/// is the whole reason to ask.
#[derive(Debug, Clone)]
pub enum VarConfirm {
    /// Retype `slug` to `ty`; `uses` nodes read or write it today.
    Retype { slug: String, ty: PinType, uses: usize },
    /// Delete `slug`; `uses` nodes will become `UnknownVariable` errors.
    Delete { slug: String, uses: usize },
}

/// In-flight inline text edit of a comment/group's text (P7).
pub struct AnnotationEdit {
    pub is_group: bool,
    pub index: usize,
    pub buffer: String,
    pub original: String,
    /// World top-left of the edited annotation (popup anchor).
    pub anchor_world: [f32; 2],
    /// Grab keyboard focus on the first frame only.
    pub first_frame: bool,
}

impl GraphEditorState {
    /// Load a graph asset: `load_graph` → `migrate_doc` → `validate_doc`.
    /// Fails only on I/O, parse, or migration errors; validation errors are
    /// stored in `errors`, never fatal.
    pub fn open(
        abs_path: &std::path::Path,
        content_rel_key: &str,
        registry: &NodeRegistry,
    ) -> Result<Self, String> {
        let mut doc = load_graph(abs_path).map_err(|e| e.to_string())?;
        migrate_doc(&mut doc, registry).map_err(|e| e.to_string())?;
        Ok(Self::from_doc(
            content_rel_key.to_string(),
            doc,
            GraphDomain::of_path(content_rel_key),
            registry,
        ))
    }

    /// The one constructor: a state over an already-loaded (and migrated)
    /// document. [`open`](Self::open) wraps it with disk I/O; the rule scope
    /// (ticket 05) wraps it around a projection; tests hand it a bare doc.
    pub fn from_doc(
        path: String,
        doc: GraphDoc,
        domain: GraphDomain,
        registry: &NodeRegistry,
    ) -> Self {
        let errors = validate_doc(&doc, registry);
        let domain_errors = domain.compile_errors(&doc, &path);
        Self {
            path,
            doc,
            errors,
            domain,
            domain_errors,
            ref_errors: Vec::new(),
            dirty: false,
            last_saved_at: None,
            view: CanvasView::default(),
            selection: BTreeSet::new(),
            primary: None,
            selected_edges: BTreeSet::new(),
            stack: GraphEditStack::new(),
            node_drag: None,
            connect_drag: None,
            marquee: None,
            marquee_mode: MarqueeMode::default(),
            prop_edit: None,
            var_edit: None,
            create_menu_world: None,
            create_menu_search: String::new(),
            sel_comment: None,
            sel_group: None,
            annotation_drag: None,
            editing: None,
            annotation_resize: None,
            annotation_menu: None,
            error_cursor: 0,
            error_popover: false,
            wire_menu: None,
            bookmarks: [None; BOOKMARK_SLOTS],
            bookmark_next: 0,
            purge_confirm: None,
            toasts: Vec::new(),
            cut_path: None,
            node_menu: None,
            palette: None,
            find: None,
            cheat_sheet: false,
            canvas_menu: None,
            breakpoints: Default::default(),
            nav_back: Vec::new(),
            nudge: None,
            frame: 0,
            frame_all_on_open: false,
            // Open by default: 45-A's authoring story starts at the variable
            // list, and a strip nobody can see teaches nobody it exists. One
            // click (or Alt+V) collapses it back to the rail.
            vars: VarPanel { open: true, ..Default::default() },
            flash: None,
            exec_bind: None,
            exec_picker: false,
            watches: Vec::new(),
            debug_request: None,
            payload: PayloadPanel::default(),
            rule_scope: None,
            anim_bind: None,
            anim_picker: false,
            anim_edits: Vec::new(),
        }
    }

    /// Serialize and write the doc back to disk, clearing the dirty flag.
    pub fn save(&mut self, abs_path: &std::path::Path) -> Result<(), String> {
        // Snap positions in memory to what actually went to disk. Saving
        // writes a rounded clone, so leaving sub-pixel values in memory means
        // "clean" would describe content the file does not contain — undo,
        // redo, or a later save would silently disagree with the disk.
        //
        // Only the positions are snapped, never the vec order: the undo stack
        // addresses nodes and edges by index, and `canonical_form`'s sort is
        // a serialization concern that must not reach the live document.
        snap_positions(&mut self.doc);
        save_graph(abs_path, &self.doc).map_err(|e| e.to_string())?;
        self.stack.mark_saved();
        self.dirty = false;
        self.last_saved_at = Some(Instant::now());
        Ok(())
    }

    /// Re-validate the doc and refresh the dirty flag. Call after every edit.
    pub fn after_edit(&mut self, registry: &NodeRegistry) {
        self.errors = validate_doc(&self.doc, registry);
        self.refresh_domain_errors();
        self.dirty = self.stack.is_dirty();
    }

    /// Recompute the domain-compiler refusals alone — what a host tab needs
    /// when a graph it *nests* changed on disk without any edit of its own.
    pub fn refresh_domain_errors(&mut self) {
        self.domain_errors = self.domain.compile_errors(&self.doc, &self.path);
    }

    /// Record an already-applied edit, then re-validate.
    pub fn commit(&mut self, edit: GraphEdit, registry: &NodeRegistry) {
        self.stack.record(edit);
        self.after_edit(registry);
    }

    pub fn undo(&mut self, registry: &NodeRegistry) {
        // Abandon any half-finished gesture first. Its edits are either
        // un-recorded (a midpoint grab's insert) or about to be recorded
        // against state undo is rewriting; either way "undo" should mean
        // "take back what I am doing" before it means "take back what I did".
        let had_gesture = self.gesture_in_flight();
        self.cancel_interactions();
        // One history (ticket 05): with a rule scope open, its recorded edits
        // are already on this stack (drained), so popping here takes back the
        // most recent thing done *anywhere* — then the projection re-derives.
        self.drain_rule_scope(registry);
        if self.stack.undo(&mut self.doc).is_some() {
            self.prune_selection();
            self.after_edit(registry);
        } else if had_gesture {
            self.after_edit(registry);
        }
        self.rebuild_rule_scope();
    }

    pub fn redo(&mut self, registry: &NodeRegistry) {
        self.cancel_interactions();
        self.drain_rule_scope(registry);
        if self.stack.redo(&mut self.doc).is_some() {
            self.prune_selection();
            self.after_edit(registry);
        }
        self.rebuild_rule_scope();
    }

    /// Is a gesture mid-flight — one whose edits are not on the stack yet?
    pub fn gesture_in_flight(&self) -> bool {
        self.node_drag.is_some()
            || self.annotation_drag.is_some()
            || self.annotation_resize.is_some()
            || self.prop_edit.is_some()
            || self.cut_path.is_some()
            || self
                .rule_scope
                .as_ref()
                .is_some_and(|s| s.child.gesture_in_flight())
    }

    /// Drop selection entries whose node no longer exists, and clear
    /// annotation selection/drag/edit state whose indices an undo/redo may
    /// have invalidated (P7).
    fn prune_selection(&mut self) {
        self.selection.retain(|id| self.doc.node(*id).is_some());
        self.primary = self.primary.filter(|id| self.selection.contains(id));
        self.selected_edges.retain(|e| self.doc.edges.contains(e));
        self.sel_comment = self.sel_comment.filter(|&i| i < self.doc.comments.len());
        self.sel_group = self.sel_group.filter(|&i| i < self.doc.groups.len());
        // A payload draft or confirmation names a node an undo may have taken
        // away; both would otherwise sit there addressing nothing.
        let doc = &self.doc;
        self.payload.draft = self.payload.draft.take().filter(|d| doc.node(d.node).is_some());
        self.payload.confirm = self.payload.confirm.take().filter(|c| {
            doc.node(match c {
                PayloadConfirm::Remove { node, .. } | PayloadConfirm::Rename { node, .. } => *node,
            })
            .is_some()
        });
        // An in-flight drag holds pre-undo positions/indices; cancel it so the
        // next frame doesn't overwrite the undone state or commit a bogus move.
        self.cancel_interactions();
    }

    /// Add a comment box at `pos` (default size + placeholder text), select it.
    pub fn add_comment(&mut self, pos: [f32; 2], registry: &NodeRegistry) {
        // A region holds nodes and edges only — an annotation would have
        // nowhere to live (ticket 05).
        if matches!(self.domain, GraphDomain::AnimationRule { .. }) {
            self.toast("A rule keeps no annotations");
            return;
        }
        let comment = CommentBox {
            rect: [pos[0], pos[1], 220.0, 130.0],
            text: "Comment".to_string(),
            ..CommentBox::default()
        };
        self.doc.comments.push(comment.clone());
        self.clear_selection();
        self.sel_comment = Some(self.doc.comments.len() - 1);
        self.commit(GraphEdit::AddComment(comment), registry);
    }

    /// Add a group frame bounding the selected nodes (approximate extent +
    /// padding), select it. No-op when nothing is selected.
    pub fn add_group_around_selection(&mut self, registry: &NodeRegistry) {
        // Same region rule as `add_comment` (ticket 05).
        if matches!(self.domain, GraphDomain::AnimationRule { .. }) {
            self.toast("A rule keeps no annotations");
            return;
        }
        // Rough per-node extent (real geometry lives in the panel); over-cover
        // so the frame encloses the nodes.
        const NODE_EXT: [f32; 2] = [168.0, 100.0];
        const PAD: f32 = 24.0;
        let mut it = self
            .doc
            .nodes
            .iter()
            .filter(|n| self.selection.contains(&n.id))
            .map(|n| n.position);
        let Some(first) = it.next() else {
            return;
        };
        let (mut minx, mut miny, mut maxx, mut maxy) =
            (first[0], first[1], first[0] + NODE_EXT[0], first[1] + NODE_EXT[1]);
        for p in it {
            minx = minx.min(p[0]);
            miny = miny.min(p[1]);
            maxx = maxx.max(p[0] + NODE_EXT[0]);
            maxy = maxy.max(p[1] + NODE_EXT[1]);
        }
        let group = GroupBox {
            rect: [minx - PAD, miny - PAD, (maxx - minx) + PAD * 2.0, (maxy - miny) + PAD * 2.0],
            title: "Group".to_string(),
            ..GroupBox::default()
        };
        self.doc.groups.push(group.clone());
        self.clear_selection();
        self.sel_group = Some(self.doc.groups.len() - 1);
        self.commit(GraphEdit::AddGroup(group), registry);
    }

    /// Clear node + annotation selection (a fresh single selection follows).
    pub fn clear_selection(&mut self) {
        self.selection.clear();
        self.primary = None;
        self.selected_edges.clear();
        self.sel_comment = None;
        self.sel_group = None;
    }

    /// Select exactly `id` and make it the primary (last-clicked) node.
    pub fn select_only(&mut self, id: u64) {
        self.clear_selection();
        self.selection.insert(id);
        self.primary = Some(id);
    }

    /// Toggle `id` in the selection (⇧-click). Adding makes it primary;
    /// removing the primary demotes to "no primary" rather than guessing.
    pub fn toggle_selected(&mut self, id: u64) {
        if self.selection.remove(&id) {
            if self.primary == Some(id) {
                self.primary = None;
            }
        } else {
            self.selection.insert(id);
            self.primary = Some(id);
        }
    }

    /// Begin (or continue) an inline property edit on `node.key`, remembering
    /// the pre-gesture value exactly once so the whole drag is one undo entry.
    pub fn begin_prop_edit(&mut self, node: u64, key: &str, registry: &NodeRegistry) {
        if self
            .prop_edit
            .as_ref()
            .is_some_and(|p| p.node == node && p.key == key)
        {
            return;
        }
        // A different target: the previous gesture is over.
        self.flush_prop_edit(registry);
        let old = self
            .doc
            .node(node)
            .and_then(|n| n.properties.get(key).cloned());
        self.prop_edit = Some(PropEdit { node, key: key.to_string(), old });
    }

    /// Commit the in-flight inline edit as one `SetProperty`. No-op when
    /// nothing is pending or the value ended where it started.
    pub fn flush_prop_edit(&mut self, registry: &NodeRegistry) {
        // A scope's own coalesced edit settles with the parent's, and its
        // settled entries move onto the parent history — so a caller about
        // to save or undo sees one coherent stack (ticket 05).
        if self.rule_scope.is_some() {
            if let Some(scope) = &mut self.rule_scope {
                scope.child.flush_prop_edit(rule_scope_registry());
                scope.child.flush_var_default_edit(rule_scope_registry());
            }
            self.drain_rule_scope(registry);
        }
        let Some(p) = self.prop_edit.take() else {
            return;
        };
        let new = self
            .doc
            .node(p.node)
            .and_then(|n| n.properties.get(&p.key).cloned());
        if new == p.old {
            return;
        }
        self.commit(
            GraphEdit::SetProperty { node: p.node, key: p.key, old: p.old, new },
            registry,
        );
    }

    // -----------------------------------------------------------------
    // Variables (45-A P6). Pure document editing — the panel that drives
    // these is P6c; everything here is testable without a `Ui`.
    // -----------------------------------------------------------------

    /// Declare a variable from a display name. Returns the slug it was given.
    ///
    /// The slug is derived once and then frozen: `variable_slug` normalizes
    /// the name, and a collision takes a `_2`, `_3` … suffix, matching how
    /// interface pins and generated subgraph paths already disambiguate.
    /// Renaming later changes only the label, so this is the single moment
    /// the identity is decided.
    pub fn add_variable(
        &mut self,
        name: &str,
        ty: PinType,
        registry: &NodeRegistry,
    ) -> String {
        let base = variable_slug(name);
        let mut slug = base.clone();
        let mut n = 2u32;
        while self.doc.variables.iter().any(|v| v.slug == slug) {
            slug = format!("{base}_{n}");
            n += 1;
        }
        let label = if name.trim().is_empty() {
            slug.clone()
        } else {
            name.trim().to_string()
        };
        let decl = VarDecl {
            slug: slug.clone(),
            label,
            default: PropValue::zero_of(&ty),
            ty,
            group: None,
        };
        let edit = GraphEdit::AddVariable(decl);
        edit.apply(&mut self.doc);
        self.commit(edit, registry);
        slug
    }

    /// Change a variable's display label. The slug is untouched — that is the
    /// point of having one.
    pub fn rename_variable(&mut self, slug: &str, label: &str, registry: &NodeRegistry) -> bool {
        let Some(old) = self.doc.variable(slug).map(|v| v.label.clone()) else {
            return false;
        };
        if old == label {
            return false;
        }
        let edit = GraphEdit::RenameVariable {
            slug: slug.to_string(),
            old,
            new: label.to_string(),
        };
        edit.apply(&mut self.doc);
        self.commit(edit, registry);
        true
    }

    /// Change a variable's type.
    ///
    /// **The default survives only if it already holds the new type's shape**
    /// (`Int` stays `Int`); otherwise it resets to the new type's zero, and to
    /// `None` for the types with no constant form. Deliberately no coercion:
    /// turning `2.7` into `2` on a Float→Int retype changes the author's value
    /// without saying so, and retyping a number back is one gesture. The undo
    /// entry carries both defaults, so nothing is lost either way.
    ///
    /// Every `var_get`/`var_set` naming this variable re-derives its pin type
    /// through `DocDescriptors`, so wires that no longer type-check surface as
    /// `TypeMismatch` on the next validation — which is why the panel warns
    /// before calling this.
    pub fn retype_variable(&mut self, slug: &str, ty: PinType, registry: &NodeRegistry) -> bool {
        let Some(v) = self.doc.variable(slug) else {
            return false;
        };
        if v.ty == ty {
            return false;
        }
        let old_ty = v.ty.clone();
        let old_default = v.default.clone();
        let new_default = match &old_default {
            Some(d) if d.matches_type(&ty) => old_default.clone(),
            _ => PropValue::zero_of(&ty),
        };
        let edit = GraphEdit::RetypeVariable {
            slug: slug.to_string(),
            old_ty,
            new_ty: ty,
            old_default,
            new_default,
        };
        edit.apply(&mut self.doc);
        self.commit(edit, registry);
        true
    }

    /// Remove a declaration. The nodes that named it stay put and become
    /// `UnknownVariable` errors — see `RemoveVariable`.
    pub fn remove_variable(&mut self, slug: &str, registry: &NodeRegistry) -> bool {
        let Some(index) = self.doc.variables.iter().position(|v| v.slug == slug) else {
            return false;
        };
        let decl = self.doc.variables[index].clone();
        let edit = GraphEdit::RemoveVariable { index, decl };
        edit.apply(&mut self.doc);
        self.commit(edit, registry);
        true
    }

    /// How many `var_get`/`var_set` instances name this variable — what the
    /// delete confirmation counts.
    pub fn variable_usage_count(&self, slug: &str) -> usize {
        self.doc
            .nodes
            .iter()
            .filter(|n| {
                matches!(
                    n.type_id.as_str(),
                    crate::engine::node_graph::VAR_GET_TYPE_ID
                        | crate::engine::node_graph::VAR_SET_TYPE_ID
                ) && matches!(
                    n.properties.get(crate::engine::node_graph::VAR_PROP),
                    Some(PropValue::Str(s)) if s == slug
                )
            })
            .count()
    }

    /// Begin (or continue) a coalesced edit of a variable's default.
    ///
    /// **Variable defaults coalesce exactly like node properties**: the panel
    /// drives them with the same `DragValue`, so a single drag has to be a
    /// single undo entry. The two paths are kept separate rather than merged
    /// because their targets differ — one is keyed by (node, pin), the other
    /// by slug — and switching between them flushes the other, so a drag on a
    /// node pin followed by a drag on a variable never merges into one entry.
    pub fn begin_var_default_edit(&mut self, slug: &str, registry: &NodeRegistry) {
        if self.var_edit.as_ref().is_some_and(|v| v.slug == slug) {
            return;
        }
        self.flush_var_default_edit(registry);
        self.flush_prop_edit(registry);
        let old = self.doc.variable(slug).and_then(|v| v.default.clone());
        self.var_edit = Some(VarDefaultEdit { slug: slug.to_string(), old });
    }

    /// Commit the in-flight default edit as one `SetVariableDefault`. No-op
    /// when nothing is pending or the value ended where it started.
    pub fn flush_var_default_edit(&mut self, registry: &NodeRegistry) {
        let Some(p) = self.var_edit.take() else {
            return;
        };
        let new = self.doc.variable(&p.slug).and_then(|v| v.default.clone());
        if new == p.old {
            return;
        }
        self.commit(
            GraphEdit::SetVariableDefault { slug: p.slug, old: p.old, new },
            registry,
        );
    }

    /// The next `var_get`/`var_set` to frame for this slug, advancing the
    /// per-slug cursor so repeat clicks **cycle** the uses rather than
    /// re-framing the first one forever. Document order, so the cycle is the
    /// same one the count counted.
    pub fn next_locate(&mut self, slug: &str) -> Option<u64> {
        let ids = variable_node_ids(&self.doc, slug);
        if ids.is_empty() {
            return None;
        }
        let cursor = self.vars.locate.entry(slug.to_string()).or_insert(0);
        let id = ids[*cursor % ids.len()];
        *cursor = (*cursor + 1) % ids.len();
        Some(id)
    }

    /// Assign (or clear, with `None`) a declaration's panel group — display
    /// metadata, so nothing moves and nothing recompiles.
    pub fn set_variable_group(
        &mut self,
        slug: &str,
        group: Option<String>,
        registry: &NodeRegistry,
    ) -> bool {
        let Some(old) = self.doc.variable(slug).map(|v| v.group.clone()) else {
            return false;
        };
        if old == group {
            return false;
        }
        let edit = GraphEdit::SetVariableGroup { slug: slug.to_string(), old, new: group };
        edit.apply(&mut self.doc);
        self.commit(edit, registry);
        true
    }

    /// Move a declaration to `to` in the list. **The only thing that changes
    /// declaration order** — every other panel gesture leaves it alone, which
    /// is what makes the order meaningful enough to drag.
    pub fn reorder_variable(&mut self, from: usize, to: usize, registry: &NodeRegistry) -> bool {
        if from >= self.doc.variables.len() || to >= self.doc.variables.len() || from == to {
            return false;
        }
        let edit = GraphEdit::ReorderVariable { from, to };
        edit.apply(&mut self.doc);
        self.commit(edit, registry);
        true
    }

    // --- Array literal entries (GS-2) --------------------------------
    //
    // An array default is one `PropValue::Array`, so every entry gesture is
    // one `SetVariableDefault` — the path the scalar editor already uses, and
    // therefore the same undo, the same validation, the same coalescing rule.
    // Each *gesture* is one entry: adding, removing and reordering commit
    // immediately (they are discrete), while typing into a component field
    // rides the coalescing `set_variable_default` like any drag.

    /// The array default of `slug`, or `None` when it is not an array.
    fn array_default(&self, slug: &str) -> Option<Vec<PropValue>> {
        match self.doc.variable(slug)?.default.as_ref() {
            Some(PropValue::Array(v)) => Some(v.clone()),
            _ => matches!(self.doc.variable(slug)?.ty, PinType::Array(_)).then(Vec::new),
        }
    }

    /// Commit a whole new entry list as one undo entry with its own label.
    fn commit_array(&mut self, slug: &str, items: Vec<PropValue>, registry: &NodeRegistry, label: &str) {
        let old = self.doc.variable(slug).and_then(|v| v.default.clone());
        let new = Some(PropValue::Array(items));
        if old == new {
            return;
        }
        // Any in-flight coalesced edit belongs to the previous gesture.
        self.flush_var_default_edit(registry);
        let edit = GraphEdit::Composite {
            label: label.to_string(),
            edits: vec![GraphEdit::SetVariableDefault {
                slug: slug.to_string(),
                old,
                new,
            }],
        };
        edit.apply(&mut self.doc);
        self.commit(edit, registry);
    }

    /// Append one entry, seeded with the element type's zero.
    pub fn add_array_entry(&mut self, slug: &str, registry: &NodeRegistry) -> bool {
        let Some(mut items) = self.array_default(slug) else {
            return false;
        };
        let elem = match &self.doc.variable(slug).map(|v| v.ty.clone()) {
            Some(PinType::Array(inner)) => (**inner).clone(),
            _ => return false,
        };
        // A type with no constant form (Entity) has no literal entry to add.
        let Some(zero) = PropValue::zero_of(&elem) else {
            return false;
        };
        items.push(zero);
        self.commit_array(slug, items, registry, "Add Array Entry");
        true
    }

    pub fn remove_array_entry(&mut self, slug: &str, index: usize, registry: &NodeRegistry) -> bool {
        let Some(mut items) = self.array_default(slug) else {
            return false;
        };
        if index >= items.len() {
            return false;
        }
        items.remove(index);
        self.commit_array(slug, items, registry, "Remove Array Entry");
        true
    }

    pub fn move_array_entry(
        &mut self,
        slug: &str,
        from: usize,
        to: usize,
        registry: &NodeRegistry,
    ) -> bool {
        let Some(mut items) = self.array_default(slug) else {
            return false;
        };
        if from >= items.len() || to >= items.len() || from == to {
            return false;
        }
        let e = items.remove(from);
        items.insert(to, e);
        self.commit_array(slug, items, registry, "Reorder Array Entry");
        true
    }

    /// Set one entry's value through the **coalescing** default path, so a
    /// drag on a component field is one undo entry like every other drag.
    pub fn set_array_entry(
        &mut self,
        slug: &str,
        index: usize,
        value: PropValue,
        registry: &NodeRegistry,
    ) -> bool {
        let Some(mut items) = self.array_default(slug) else {
            return false;
        };
        if index >= items.len() || items[index] == value {
            return false;
        }
        items[index] = value;
        self.set_variable_default(slug, Some(PropValue::Array(items)), registry);
        true
    }

    // -----------------------------------------------------------------
    // Custom-event payload fields (GS-1). A `payload.<slug>` property on an
    // `event_custom` node *is* one output pin — `DocDescriptors` synthesizes
    // the pin from the property, so every op below is a property edit and the
    // pin follows on the next frame with no separate bookkeeping.
    //
    // All three land as **one** undo entry with the gesture's own name, which
    // is why they wrap their `SetProperty` in a `Composite`: `Set payload.dir`
    // is what the machine did, "Add Payload Field" is what the author did.
    // -----------------------------------------------------------------

    /// Declare a payload field from a typed name. Returns the slug it took, or
    /// `None` when the node is not a custom event.
    ///
    /// The slug rules are the variables' rules — `variable_slug` normalizes,
    /// and a collision takes a `_2`, `_3` … suffix — because a payload slug is
    /// a pin slug, and pin slugs sit in the same identity family. Edges key by
    /// slug, so this is the one moment the field's identity is decided.
    pub fn add_payload_field(
        &mut self,
        node: u64,
        name: &str,
        registry: &NodeRegistry,
    ) -> Option<String> {
        let n = self.doc.node(node)?;
        if n.type_id != EVENT_CUSTOM_TYPE_ID {
            return None;
        }
        let base = variable_slug(name);
        let mut slug = base.clone();
        let mut i = 2u32;
        while n
            .properties
            .contains_key(&format!("{EVENT_PAYLOAD_PREFIX}{slug}"))
        {
            slug = format!("{base}_{i}");
            i += 1;
        }
        let edit = GraphEdit::Composite {
            label: "Add Payload Field".to_string(),
            edits: vec![GraphEdit::SetProperty {
                node,
                key: format!("{EVENT_PAYLOAD_PREFIX}{slug}"),
                old: None,
                // Float by design: the commonest payload, and a type the
                // dropdown in the same row changes in one click.
                new: Some(PropValue::Enum(DEFAULT_PAYLOAD_TYPE.to_string())),
            }],
        };
        edit.apply(&mut self.doc);
        self.commit(edit, registry);
        Some(slug)
    }

    /// Remove a payload field. Edges that named its pin are **left alone** on
    /// purpose: with the property gone the pin vanishes from synthesis, the
    /// edge becomes an `UnknownPin` error, and the panel draws it as a dashed
    /// ghost row — the wire keeps a landing spot and the author sees what
    /// broke. Silently deleting their wires would be the worse degradation.
    pub fn remove_payload_field(
        &mut self,
        node: u64,
        slug: &str,
        registry: &NodeRegistry,
    ) -> bool {
        let key = format!("{EVENT_PAYLOAD_PREFIX}{slug}");
        let Some(old) = self
            .doc
            .node(node)
            .and_then(|n| n.properties.get(&key).cloned())
        else {
            return false;
        };
        let edit = GraphEdit::Composite {
            label: "Remove Payload Field".to_string(),
            edits: vec![GraphEdit::SetProperty { node, key, old: Some(old), new: None }],
        };
        edit.apply(&mut self.doc);
        self.commit(edit, registry);
        true
    }

    /// Rename a payload field, carrying **this document's** incident edges with
    /// it: the property key moves and every edge leaving the old pin is
    /// re-pointed at the new one, as one entry.
    ///
    /// Edges in *other* documents cannot be rewritten from here (they belong to
    /// files this tab does not own), which is exactly what the confirmation
    /// says before this runs. Returns the slug taken, or `None` when nothing
    /// changed.
    pub fn rename_payload_field(
        &mut self,
        node: u64,
        slug: &str,
        name: &str,
        registry: &NodeRegistry,
    ) -> Option<String> {
        let old_key = format!("{EVENT_PAYLOAD_PREFIX}{slug}");
        let old_value = self.doc.node(node)?.properties.get(&old_key).cloned()?;
        let base = variable_slug(name);
        if base == slug {
            return None;
        }
        let mut new_slug = base.clone();
        let mut i = 2u32;
        while self
            .doc
            .node(node)?
            .properties
            .contains_key(&format!("{EVENT_PAYLOAD_PREFIX}{new_slug}"))
        {
            new_slug = format!("{base}_{i}");
            i += 1;
        }
        let new_key = format!("{EVENT_PAYLOAD_PREFIX}{new_slug}");
        // Edges first (recorded with their indices, so an undo restores the
        // exact vec order a byte-stable save depends on), then the two halves
        // of the key move.
        let doomed: Vec<(usize, Edge)> = self
            .doc
            .edges
            .iter()
            .enumerate()
            .filter(|(_, e)| e.from_node == node && e.from_pin == slug)
            .map(|(i, e)| (i, e.clone()))
            .collect();
        let mut edits = vec![
            GraphEdit::SetProperty {
                node,
                key: old_key,
                old: Some(old_value.clone()),
                new: None,
            },
            GraphEdit::SetProperty {
                node,
                key: new_key,
                old: None,
                new: Some(old_value),
            },
        ];
        if !doomed.is_empty() {
            let rewired: Vec<Edge> = doomed
                .iter()
                .map(|(_, e)| Edge { from_pin: new_slug.clone(), ..e.clone() })
                .collect();
            edits.push(GraphEdit::Disconnect { edges: doomed });
            edits.extend(rewired.into_iter().map(GraphEdit::Connect));
        }
        let edit = GraphEdit::Composite {
            label: "Rename Payload Field".to_string(),
            edits,
        };
        edit.apply(&mut self.doc);
        self.commit(edit, registry);
        Some(new_slug)
    }

    /// Set a variable's default through the coalescing path.
    pub fn set_variable_default(
        &mut self,
        slug: &str,
        value: Option<PropValue>,
        registry: &NodeRegistry,
    ) {
        if self.doc.variable(slug).is_none() {
            return;
        }
        self.begin_var_default_edit(slug, registry);
        set_var_default(&mut self.doc, slug, &value);
    }

    /// Add a node of `type_id` at `pos` with descriptor input defaults as
    /// properties, select it, and record the edit.
    pub fn add_node(&mut self, type_id: &str, pos: [f32; 2], registry: &NodeRegistry) {
        let mut properties = std::collections::BTreeMap::new();
        if let Some(desc) = registry.get(type_id) {
            for pin in &desc.inputs {
                if let Some(default) = &pin.default {
                    properties.insert(pin.slug.clone(), default.clone());
                }
            }
        }
        let node = NodeInst {
            id: self.doc.next_node_id(),
            type_id: type_id.to_string(),
            type_version: registry.get(type_id).map(|d| d.version).unwrap_or(1),
            position: pos,
            properties,
            subgraph: None,
        
        tint: None, title: None,};
        let id = node.id;
        self.doc.nodes.push(node.clone());
        self.select_only(id);
        self.commit(GraphEdit::AddNode(node), registry);
    }

    /// Insert a Transition joining `from` (a state or Any State) to `to` (a
    /// state), placed midway between them — the state-machine drag gesture's
    /// other half. One composite, one undo ("Add Transition"). Returns the
    /// new node's id. The transition is deliberately **not** selected: at
    /// rest it renders as its chip, which is the thing the gesture made.
    pub fn insert_transition_between(
        &mut self,
        from: u64,
        to: u64,
        registry: &NodeRegistry,
    ) -> u64 {
        use crate::engine::animation::graph::plan::{
            ANIM_TRANSITION_TYPE_ID, STATE_IN_PIN, STATE_OUT_PIN, TRANSITION_FROM_PIN,
            TRANSITION_TO_PIN,
        };
        let mid = |a: [f32; 2], b: [f32; 2]| [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5];
        let pos = match (self.doc.node(from), self.doc.node(to)) {
            (Some(a), Some(b)) => mid(a.position, b.position),
            _ => [0.0, 0.0],
        };
        let node = NodeInst {
            id: self.doc.next_node_id(),
            type_id: ANIM_TRANSITION_TYPE_ID.to_string(),
            type_version: 1,
            position: pos,
            properties: std::collections::BTreeMap::new(),
            subgraph: None,
            tint: None,
            title: None,
        };
        let id = node.id;
        let edit = GraphEdit::Composite {
            label: "Add Transition".to_string(),
            edits: vec![
                GraphEdit::AddNode(node),
                GraphEdit::Connect(Edge {
                    from_node: from,
                    from_pin: STATE_OUT_PIN.to_string(),
                    to_node: id,
                    to_pin: TRANSITION_FROM_PIN.to_string(),
                }),
                GraphEdit::Connect(Edge {
                    from_node: id,
                    from_pin: TRANSITION_TO_PIN.to_string(),
                    to_node: to,
                    to_pin: STATE_IN_PIN.to_string(),
                }),
            ],
        };
        edit.apply(&mut self.doc);
        self.commit(edit, registry);
        id
    }

    /// Add a subgraph-instance node referencing `subgraph_path` at `pos`
    /// (its pins derive from the referenced asset's interface at draw time).
    pub fn add_subgraph_node(
        &mut self,
        subgraph_path: &str,
        pos: [f32; 2],
        registry: &NodeRegistry,
    ) {
        let node = NodeInst {
            id: self.doc.next_node_id(),
            type_id: SUBGRAPH_TYPE_ID.to_string(),
            type_version: 1,
            position: pos,
            properties: std::collections::BTreeMap::new(),
            subgraph: Some(subgraph_path.to_string()),
        
        tint: None, title: None,};
        let id = node.id;
        self.doc.nodes.push(node.clone());
        self.select_only(id);
        self.commit(GraphEdit::AddNode(node), registry);
    }

    /// Add a `var_get`/`var_set` instance at `pos`, already naming `slug`.
    ///
    /// The reserved [`VAR_PROP`](crate::engine::node_graph::VAR_PROP) property
    /// is preset here rather than by a follow-up `SetProperty` for the same
    /// reason `add_subgraph_node` presets its path: a node that spends one undo
    /// step naming nothing is a node the author can land on by pressing Ctrl+Z
    /// once, and its pins would resolve to nothing while it sat there. One
    /// `AddNode` edit, one undo step.
    pub fn add_variable_node(
        &mut self,
        slug: &str,
        set: bool,
        pos: [f32; 2],
        registry: &NodeRegistry,
    ) -> u64 {
        let mut properties = std::collections::BTreeMap::new();
        properties.insert(
            crate::engine::node_graph::VAR_PROP.to_string(),
            PropValue::Str(slug.to_string()),
        );
        let node = NodeInst {
            id: self.doc.next_node_id(),
            type_id: if set {
                crate::engine::node_graph::VAR_SET_TYPE_ID
            } else {
                crate::engine::node_graph::VAR_GET_TYPE_ID
            }
            .to_string(),
            type_version: 1,
            position: pos,
            properties,
            subgraph: None,
            tint: None,
            title: None,
        };
        let id = node.id;
        self.doc.nodes.push(node.clone());
        self.select_only(id);
        self.commit(GraphEdit::AddNode(node), registry);
        id
    }

    /// Delete the current selection: a selected comment or group frame (frame
    /// only — member nodes stay), else the selected nodes and their edges.
    pub fn delete_selection(&mut self, registry: &NodeRegistry) {
        // With a rule peek open the machine is inert, so Delete means the
        // peek's selection (ticket 05).
        if self.rule_scope.is_some() {
            if let Some(scope) = &mut self.rule_scope {
                scope.child.delete_selection(rule_scope_registry());
            }
            self.drain_rule_scope(registry);
            return;
        }
        // Any in-flight drag / inline edit targets an index this delete may
        // shift or remove — cancel them so nothing commits against it.
        self.cancel_interactions();
        // Wires first: a wire selection is always explicit, so Delete means
        // the wires even when nodes happen to still be selected behind them.
        if self.break_selected_links(registry) {
            return;
        }
        if let Some(i) = self.sel_comment {
            if i < self.doc.comments.len() {
                let comment = self.doc.comments.remove(i);
                self.sel_comment = None;
                self.commit(GraphEdit::RemoveComment { index: i, comment }, registry);
            }
            return;
        }
        if let Some(i) = self.sel_group {
            if i < self.doc.groups.len() {
                let group = self.doc.groups.remove(i);
                self.sel_group = None;
                self.commit(GraphEdit::RemoveGroup { index: i, group }, registry);
            }
            return;
        }
        if self.selection.is_empty() {
            return;
        }
        // A lone reroute heals the wire it sits on instead of severing it.
        if self.selection.len() == 1 {
            let id = *self.selection.iter().next().expect("len 1");
            if self.delete_reroute(id, registry) {
                return;
            }
        }
        let ids = self.selection.clone();
        let nodes: Vec<(usize, NodeInst)> = self
            .doc
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| ids.contains(&n.id))
            .map(|(i, n)| (i, n.clone()))
            .collect();
        if nodes.is_empty() {
            return;
        }
        let edges: Vec<(usize, Edge)> = self
            .doc
            .edges
            .iter()
            .enumerate()
            .filter(|(_, e)| ids.contains(&e.from_node) || ids.contains(&e.to_node))
            .map(|(i, e)| (i, e.clone()))
            .collect();
        let comments: Vec<(usize, CommentBox)> = anchored_comments(&self.doc, &ids)
            .into_iter()
            .map(|i| (i, self.doc.comments[i].clone()))
            .collect();
        let edit = GraphEdit::RemoveNodes {
            nodes,
            edges,
            comments,
            regions: region_collateral(&self.doc, ids.iter().copied()),
        };
        edit.apply(&mut self.doc);
        self.selection.clear();
        self.commit(edit, registry);
    }

    /// Add/remove a wire from the selection (⇧-click extends).
    pub fn toggle_edge_selected(&mut self, edge: &Edge) {
        if !self.selected_edges.remove(edge) {
            self.selected_edges.insert(edge.clone());
        }
    }

    /// Select exactly this wire, dropping any node/annotation selection.
    pub fn select_only_edge(&mut self, edge: &Edge) {
        self.clear_selection();
        self.selected_edges.insert(edge.clone());
    }

    /// Remove every selected wire as **one** undo transaction. Returns how
    /// many were broken (0 = nothing was selected, caller falls through).
    pub fn break_selected_links(&mut self, registry: &NodeRegistry) -> bool {
        if self.selected_edges.is_empty() {
            return false;
        }
        let doomed: Vec<Edge> = std::mem::take(&mut self.selected_edges)
            .into_iter()
            .collect();
        self.break_links(&doomed, "Broke", registry) > 0
    }

    /// Read an annotation's stored rect, if it still exists.
    pub fn annotation_rect(&self, target: Annotation) -> Option<[f32; 4]> {
        match target {
            Annotation::Comment(i) => self.doc.comments.get(i).map(|c| c.rect),
            Annotation::Group(i) => self.doc.groups.get(i).map(|g| g.rect),
        }
    }

    /// Set an annotation's tint slot (a ramp index, never a hex), undoably.
    pub fn set_annotation_tint(
        &mut self,
        target: Annotation,
        tint: Option<u8>,
        registry: &NodeRegistry,
    ) {
        let old = match target {
            Annotation::Comment(i) => self.doc.comments.get(i).map(|c| c.tint),
            Annotation::Group(i) => self.doc.groups.get(i).map(|g| g.tint),
        };
        let Some(old) = old else { return };
        if old == tint {
            return;
        }
        let edit = GraphEdit::SetAnnotationTint { target, old, new: tint };
        edit.apply(&mut self.doc);
        self.commit(edit, registry);
    }

    /// Fold/unfold an annotation to its bar, undoably.
    pub fn toggle_annotation_collapsed(&mut self, target: Annotation, registry: &NodeRegistry) {
        let now = match target {
            Annotation::Comment(i) => self.doc.comments.get(i).map(|c| c.collapsed),
            Annotation::Group(i) => self.doc.groups.get(i).map(|g| g.collapsed),
        };
        let Some(now) = now else { return };
        let edit = GraphEdit::SetAnnotationCollapsed { target, new: !now };
        edit.apply(&mut self.doc);
        self.commit(edit, registry);
    }

    /// Anchor (or un-anchor) a comment to a node, undoably.
    pub fn set_comment_anchor(
        &mut self,
        index: usize,
        node: Option<u64>,
        registry: &NodeRegistry,
    ) {
        let Some(c) = self.doc.comments.get(index) else {
            return;
        };
        let old = c.anchor;
        if old == node {
            return;
        }
        let edit = GraphEdit::SetCommentAnchor { index, old, new: node };
        edit.apply(&mut self.doc);
        self.commit(edit, registry);
    }

    /// Commit an in-flight resize as one edit. No-op if nothing moved.
    pub fn finish_annotation_resize(&mut self, registry: &NodeRegistry) {
        let Some(r) = self.annotation_resize.take() else {
            return;
        };
        let Some(now) = self.annotation_rect(r.target) else {
            return;
        };
        if now == r.rect0 {
            return;
        }
        self.commit(
            GraphEdit::ResizeAnnotation { target: r.target, old: r.rect0, new: now },
            registry,
        );
    }

    /// Insert a reroute in the middle of `edge`, at `pos`. One transaction:
    /// the original edge goes, a reroute node arrives, and two edges replace
    /// it. A reroute has no descriptor — its type is inferred from whatever
    /// feeds it — so nothing needs registering.
    pub fn insert_reroute(&mut self, edge: &Edge, pos: [f32; 2], registry: &NodeRegistry) {
        let Some(edit) = self.reroute_insert_edit(edge, pos) else {
            return;
        };
        edit.apply(&mut self.doc);
        let id = self.doc.nodes.last().map(|n| n.id).unwrap_or_default();
        self.select_only(id);
        self.commit(edit, registry);
    }

    /// Build (but do not apply) the reroute-insert transaction.
    fn reroute_insert_edit(&self, edge: &Edge, pos: [f32; 2]) -> Option<GraphEdit> {
        let index = self.doc.edges.iter().position(|e| e == edge)?;
        let id = self.doc.next_node_id();
        let node = NodeInst {
            id,
            type_id: REROUTE_TYPE_ID.to_string(),
            type_version: 1,
            position: pos,
            properties: Default::default(),
            subgraph: None,
            tint: None, title: None,
        };
        Some(GraphEdit::Composite {
            label: "Add Reroute".to_string(),
            edits: vec![
                GraphEdit::Disconnect { edges: vec![(index, edge.clone())] },
                GraphEdit::AddNode(node),
                GraphEdit::Connect(Edge {
                    from_node: edge.from_node,
                    from_pin: edge.from_pin.clone(),
                    to_node: id,
                    to_pin: REROUTE_IN.to_string(),
                }),
                GraphEdit::Connect(Edge {
                    from_node: id,
                    from_pin: REROUTE_OUT.to_string(),
                    to_node: edge.to_node,
                    to_pin: edge.to_pin.clone(),
                }),
            ],
        })
    }

    /// Splice an existing node into `edge`: the edge is replaced by two, one
    /// into `in_pin` and one out of `out_pin`. One transaction — the reroute
    /// insert machinery generalized, since a reroute is just the degenerate
    /// case of "a node with one compatible input and output".
    ///
    /// Returns false when the edge is gone or the node is one of its
    /// endpoints (splicing a wire into itself is not a thing).
    pub fn splice_node_into(
        &mut self,
        edge: &Edge,
        node: u64,
        in_pin: &str,
        out_pin: &str,
        registry: &NodeRegistry,
    ) -> bool {
        let Some(index) = self.doc.edges.iter().position(|e| e == edge) else {
            return false;
        };
        if edge.from_node == node || edge.to_node == node {
            return false;
        }
        // The spliced wire itself always goes — it is being rerouted through
        // the node. Whether anything *else* on the chosen input goes depends
        // on the pin: a data input takes one wire so a second drop replaces
        // it, while an exec input may fan in (45-A P3) and breaking a wire
        // that converges there would silently delete work.
        let in_is_exec = DocDescriptors::new(&self.doc, registry)
            .pin_type(node, in_pin, false)
            == Some(PinType::Exec);
        let mut doomed: Vec<(usize, Edge)> = vec![(index, edge.clone())];
        if !in_is_exec {
            doomed.extend(
                self.doc
                    .edges
                    .iter()
                    .enumerate()
                    .filter(|(i, e)| {
                        *i != index && e.to_node == node && e.to_pin == in_pin
                    })
                    .map(|(i, e)| (i, e.clone())),
            );
        }
        doomed.sort_by_key(|(i, _)| *i);
        let edit = GraphEdit::Composite {
            label: "Splice Node".to_string(),
            edits: vec![
                GraphEdit::Disconnect { edges: doomed },
                GraphEdit::Connect(Edge {
                    from_node: edge.from_node,
                    from_pin: edge.from_pin.clone(),
                    to_node: node,
                    to_pin: in_pin.to_string(),
                }),
                GraphEdit::Connect(Edge {
                    from_node: node,
                    from_pin: out_pin.to_string(),
                    to_node: edge.to_node,
                    to_pin: edge.to_pin.clone(),
                }),
            ],
        };
        edit.apply(&mut self.doc);
        self.commit(edit, registry);
        self.toast("Spliced into wire");
        true
    }

    /// The input/output pin pair a node would splice into `edge` with, if it
    /// has a compatible one on **both** sides. A node that can only take the
    /// type cannot be spliced — the wire has to come out the other end.
    pub fn splice_pins(
        &self,
        edge: &Edge,
        node: u64,
        registry: &NodeRegistry,
    ) -> Option<(String, String)> {
        if edge.from_node == node || edge.to_node == node {
            return None;
        }
        let ty = endpoint_type(&self.doc, registry, edge.from_node, &edge.from_pin, true)?;
        // Instance pins: splicing a wire through a variable or interface node
        // has to see the pins the canvas draws. (A subgraph node needs the
        // cross-asset resolver, which this gesture does not carry — same
        // answer as before, now for a stated reason.)
        let desc = DocDescriptors::new(&self.doc, registry).descriptor(node)?;
        // An input takes exactly one edge, so prefer a free one; only fall
        // back to an occupied pin, whose existing edge the splice replaces.
        // Picking blind would emit `InputMultiplyConnected` — an illegal
        // graph produced by a gesture the user thought was a convenience.
        let occupied = |slug: &str| {
            self.doc
                .edges
                .iter()
                .any(|e| e.to_node == node && e.to_pin == slug)
        };
        let typed = || desc.inputs.iter().filter(|p| p.ty == ty);
        let input = typed()
            .find(|p| !occupied(&p.slug))
            .or_else(|| typed().next())?;
        let output = desc.outputs.iter().find(|p| p.ty == ty)?;
        Some((input.slug.clone(), output.slug.clone()))
    }

    /// Insert a reroute at the wire's midpoint and hand it straight to a drag,
    /// so grabbing the handle and moving it is one gesture.
    pub fn grab_wire_midpoint(
        &mut self,
        edge: &Edge,
        at: [f32; 2],
        registry: &NodeRegistry,
    ) -> Option<u64> {
        let insert = self.reroute_insert_edit(edge, at)?;
        insert.apply(&mut self.doc);
        let id = self.doc.nodes.last().map(|n| n.id)?;
        self.select_only(id);
        self.after_edit(registry);
        // Applied but not recorded: `finish_node_drag` folds it together with
        // the move into one entry, so grab-and-move is one undo.
        self.node_drag = Some(NodeDrag {
            origin_world: at,
            originals: vec![(id, self.doc.node(id)?.position)],
            anchored: Vec::new(),
            pending: Some(insert),
        });
        Some(id)
    }

    /// Revert a drag's un-recorded edit, if it has one. Called when a gesture
    /// is abandoned rather than finished — an applied-but-unrecorded edit
    /// would otherwise be invisible to undo.
    fn revert_pending_drag(&mut self) {
        if let Some(pending) = self.node_drag.as_mut().and_then(|d| d.pending.take()) {
            pending.revert(&mut self.doc);
        }
    }

    /// Delete a reroute, healing the wire through it. Every downstream branch
    /// is reconnected to the upstream source (a reroute is one-in, many-out),
    /// as one transaction. Falls back to a plain node delete when there is no
    /// upstream to heal from.
    pub fn delete_reroute(&mut self, id: u64, registry: &NodeRegistry) -> bool {
        let Some(n) = self.doc.node(id) else {
            return false;
        };
        if n.type_id != REROUTE_TYPE_ID {
            return false;
        }
        let upstream = self
            .doc
            .edges
            .iter()
            .find(|e| e.to_node == id && e.to_pin == REROUTE_IN)
            .cloned();
        // Heal unless the types are *known* to differ. "Cannot be determined"
        // (an unregistered node type, a subgraph interface) is not the same
        // claim as "they disagree", and refusing to reconnect on a shrug
        // would silently sever a wire the user only meant to simplify.
        let up_ty = upstream.as_ref().and_then(|e| {
            endpoint_type(&self.doc, registry, e.from_node, &e.from_pin, true)
        });

        let mut removed: Vec<(usize, Edge)> = self
            .doc
            .edges
            .iter()
            .enumerate()
            .filter(|(_, e)| e.from_node == id || e.to_node == id)
            .map(|(i, e)| (i, e.clone()))
            .collect();
        removed.sort_by_key(|(i, _)| *i);

        let node_index = self.doc.nodes.iter().position(|x| x.id == id);
        let Some(node_index) = node_index else {
            return false;
        };
        let inst = self.doc.nodes[node_index].clone();

        let mut edits = vec![GraphEdit::RemoveNodes {
            nodes: vec![(node_index, inst)],
            edges: removed.clone(),
            comments: anchored_comments(&self.doc, &[id].into_iter().collect())
                .into_iter()
                .map(|i| (i, self.doc.comments[i].clone()))
                .collect(),
            regions: region_collateral(&self.doc, [id]),
        }];
        if let Some(up) = upstream.as_ref() {
            for (_, e) in removed.iter().filter(|(_, e)| e.from_node == id) {
                let down_ty =
                    endpoint_type(&self.doc, registry, e.to_node, &e.to_pin, false);
                if let (Some(a), Some(b)) = (up_ty.as_ref(), down_ty.as_ref()) {
                    if a != b {
                        continue; // known mismatch: drop this branch instead
                    }
                }
                edits.push(GraphEdit::Connect(Edge {
                    from_node: up.from_node,
                    from_pin: up.from_pin.clone(),
                    to_node: e.to_node,
                    to_pin: e.to_pin.clone(),
                }));
            }
        }
        let edit = GraphEdit::Composite { label: "Delete Reroute".to_string(), edits };
        edit.apply(&mut self.doc);
        self.selection.remove(&id);
        self.commit(edit, registry);
        true
    }

    /// Post a transient canvas message and drop any that have expired.
    pub fn toast(&mut self, text: impl Into<String>) {
        self.toasts
            .retain(|t| t.at.elapsed().as_millis() < TOAST_MS);
        self.toasts.push(CanvasToast { text: text.into(), at: Instant::now() });
    }

    // -- Breaking connections --------------------------------------------

    /// Break every listed edge as one transaction, reporting the count.
    /// Shared by all three break paths so they can never diverge.
    pub fn break_links(&mut self, doomed: &[Edge], verb: &str, registry: &NodeRegistry) -> usize {
        let edges: Vec<(usize, Edge)> = self
            .doc
            .edges
            .iter()
            .enumerate()
            .filter(|(_, e)| doomed.contains(e))
            .map(|(i, e)| (i, e.clone()))
            .collect();
        let n = edges.len();
        if n == 0 {
            return 0;
        }
        let edit = GraphEdit::Disconnect { edges };
        edit.apply(&mut self.doc);
        self.selected_edges.retain(|e| !doomed.contains(e));
        self.commit(edit, registry);
        self.toast(format!("{verb} {n} link{}", if n == 1 { "" } else { "s" }));
        n
    }

    /// Every edge touching one pin.
    pub fn edges_on_pin(&self, node: u64, pin: &str, output: bool) -> Vec<Edge> {
        self.doc
            .edges
            .iter()
            .filter(|e| {
                if output {
                    e.from_node == node && e.from_pin == pin
                } else {
                    e.to_node == node && e.to_pin == pin
                }
            })
            .cloned()
            .collect()
    }

    /// Every edge touching one node, either side.
    pub fn edges_on_node(&self, node: u64) -> Vec<Edge> {
        self.doc
            .edges
            .iter()
            .filter(|e| e.from_node == node || e.to_node == node)
            .cloned()
            .collect()
    }

    /// Alt-click a pin: break its links, or say why nothing happened.
    pub fn break_pin_links(
        &mut self,
        node: u64,
        pin: &str,
        output: bool,
        registry: &NodeRegistry,
    ) {
        let doomed = self.edges_on_pin(node, pin, output);
        if doomed.is_empty() {
            self.toast("Pin has no links");
            return;
        }
        self.break_links(&doomed, "Broke", registry);
    }

    /// Alt-click a node header: break every link the node has.
    pub fn break_node_links(&mut self, node: u64, registry: &NodeRegistry) {
        let doomed = self.edges_on_node(node);
        if doomed.is_empty() {
            self.toast("Node has no links");
            return;
        }
        self.break_links(&doomed, "Broke", registry);
    }

    /// Extend the in-flight cut path, capped at [`CUT_PATH_MAX`] points.
    pub fn push_cut_point(&mut self, p: [f32; 2]) {
        let path = self.cut_path.get_or_insert_with(Vec::new);
        // Skip samples that add nothing — a still pointer must not eat the cap.
        if path
            .last()
            .is_some_and(|q| (q[0] - p[0]).abs() < 0.5 && (q[1] - p[1]).abs() < 0.5)
        {
            return;
        }
        if path.len() >= CUT_PATH_MAX {
            path.remove(0);
        }
        path.push(p);
    }

    // -- Organization ---------------------------------------------------

    /// Align or evenly distribute the selection. `rects` carries each node's
    /// *world* rect (`[x, y, w, h]`) because sizes are auto-fitted at draw
    /// time and the document does not know them.
    ///
    /// Recorded as one `Composite` of per-node `MoveNodes` rather than a new
    /// bulk variant: it reuses the move path verbatim, so anchored notes
    /// travel with their nodes here exactly as they do on a drag.
    pub fn align_nodes(
        &mut self,
        rects: &[(u64, [f32; 4])],
        mode: AlignMode,
        registry: &NodeRegistry,
    ) {
        if rects.len() < mode.min_nodes() {
            return;
        }
        let mut deltas: Vec<(u64, [f32; 2])> = Vec::new();
        match mode {
            AlignMode::Left => {
                let x = rects.iter().map(|(_, r)| r[0]).fold(f32::MAX, f32::min);
                deltas.extend(rects.iter().map(|(id, r)| (*id, [x - r[0], 0.0])));
            }
            AlignMode::Right => {
                let x = rects.iter().map(|(_, r)| r[0] + r[2]).fold(f32::MIN, f32::max);
                deltas.extend(rects.iter().map(|(id, r)| (*id, [x - (r[0] + r[2]), 0.0])));
            }
            AlignMode::Top => {
                let y = rects.iter().map(|(_, r)| r[1]).fold(f32::MAX, f32::min);
                deltas.extend(rects.iter().map(|(id, r)| (*id, [0.0, y - r[1]])));
            }
            // Centre-aligns average the centres rather than picking an
            // extreme, so the group stays where it is instead of sliding to
            // whichever node happened to be furthest out.
            AlignMode::CenterHorizontally => {
                let cx: f32 = rects.iter().map(|(_, r)| r[0] + r[2] * 0.5).sum::<f32>()
                    / rects.len() as f32;
                deltas.extend(
                    rects
                        .iter()
                        .map(|(id, r)| (*id, [cx - (r[0] + r[2] * 0.5), 0.0])),
                );
            }
            AlignMode::CenterVertically => {
                let cy: f32 = rects.iter().map(|(_, r)| r[1] + r[3] * 0.5).sum::<f32>()
                    / rects.len() as f32;
                deltas.extend(
                    rects
                        .iter()
                        .map(|(id, r)| (*id, [0.0, cy - (r[1] + r[3] * 0.5)])),
                );
            }
            AlignMode::Bottom => {
                let y = rects.iter().map(|(_, r)| r[1] + r[3]).fold(f32::MIN, f32::max);
                deltas.extend(rects.iter().map(|(id, r)| (*id, [0.0, y - (r[1] + r[3])])));
            }
            AlignMode::DistributeHorizontally | AlignMode::DistributeVertically => {
                let h = mode.horizontal();
                // Even *gaps*, not even centers: equal whitespace is what
                // reads as distributed when the nodes are different sizes.
                let mut order: Vec<&(u64, [f32; 4])> = rects.iter().collect();
                order.sort_by(|a, b| {
                    let (x, y) = if h { (a.1[0], b.1[0]) } else { (a.1[1], b.1[1]) };
                    x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal)
                });
                let first = order[0].1;
                let last = order[order.len() - 1].1;
                let (start, end) = if h {
                    (first[0], last[0] + last[2])
                } else {
                    (first[1], last[1] + last[3])
                };
                let extent: f32 = order
                    .iter()
                    .map(|(_, r)| if h { r[2] } else { r[3] })
                    .sum();
                let gap = ((end - start) - extent) / (order.len() - 1) as f32;
                let mut cursor = start;
                for (id, r) in &order {
                    let want = cursor;
                    let now = if h { r[0] } else { r[1] };
                    deltas.push((*id, if h { [want - now, 0.0] } else { [0.0, want - now] }));
                    cursor += (if h { r[2] } else { r[3] }) + gap;
                }
            }
        }
        let edits: Vec<GraphEdit> = deltas
            .into_iter()
            .filter(|(_, d)| d[0].abs() > f32::EPSILON || d[1].abs() > f32::EPSILON)
            .map(|(id, delta)| GraphEdit::MoveNodes { ids: vec![id], delta })
            .collect();
        if edits.is_empty() {
            return;
        }
        let edit = GraphEdit::Composite { label: mode.label().to_string(), edits };
        edit.apply(&mut self.doc);
        self.commit(edit, registry);
    }

    /// Collapse the selection into a new `.subgraph` asset and replace it
    /// with a single subgraph node wired to the same neighbours.
    ///
    /// **Path convention** (prompt-free by design — naming a thing is a
    /// separate decision from making it): the asset lands in a `subgraphs/`
    /// folder beside the host graph, named `<host stem>_<n>.subgraph`, with
    /// `n` the first free integer from 1. `content/graphs/ai.graph` therefore
    /// yields `content/graphs/subgraphs/ai_1.subgraph`.
    ///
    /// **Interface**: edges crossing the boundary become the interface —
    /// inbound edges become inputs, outbound become outputs, each slug taken
    /// from its *source* pin and de-duplicated. Multiple boundary edges from
    /// one source pin therefore share one interface pin, which is what a fan-
    /// out means.
    ///
    /// **Undo** reverses the host document only; the created asset stays on
    /// disk. Deleting a file the user may already have opened or edited on an
    /// undo risks data loss, and an orphaned asset costs nothing.
    ///
    /// Returns the content-relative path written, or an error string.
    pub fn collapse_to_subgraph(
        &mut self,
        content_root: &Path,
        registry: &NodeRegistry,
    ) -> Result<String, String> {
        if self.selection.len() < 2 {
            return Err("select at least two nodes to collapse".into());
        }
        let inside = self.selection.clone();
        let nodes: Vec<NodeInst> = self
            .doc
            .nodes
            .iter()
            .filter(|n| inside.contains(&n.id))
            .cloned()
            .collect();

        // Boundary edges, split by direction. Slugs come from the source pin
        // and de-duplicate: a fan-out is one interface pin, not three.
        let mut inputs: Vec<IfacePin> = Vec::new();
        let mut outputs: Vec<IfacePin> = Vec::new();
        let mut inbound: Vec<(Edge, String)> = Vec::new();
        let mut outbound: Vec<(Edge, String)> = Vec::new();
        for e in &self.doc.edges {
            let (f_in, t_in) = (inside.contains(&e.from_node), inside.contains(&e.to_node));
            if f_in == t_in {
                continue;
            }
            let slug = uniquify(&e.from_pin, if f_in { &outputs } else { &inputs });
            let ty = endpoint_type(&self.doc, registry, e.from_node, &e.from_pin, true)
                .unwrap_or(crate::engine::node_graph::PinType::Float);
            let pin = IfacePin { slug: slug.clone(), label: slug.clone(), ty };
            if f_in {
                outputs.push(pin);
                outbound.push((e.clone(), slug));
            } else {
                inputs.push(pin);
                inbound.push((e.clone(), slug));
            }
        }

        // The new asset. Internal edges come along; the boundary ones are
        // replaced by the interface declaration — and by the pair of
        // interface *binding* nodes that carry the values across it (45-A
        // D3). Without them the interface would be a declaration with nothing
        // attached: the subgraph could not be inlined, and every collapse
        // would immediately report unbound interface pins.
        let mut sub_nodes = nodes.clone();
        let mut sub_edges: Vec<Edge> = self
            .doc
            .edges
            .iter()
            .filter(|e| inside.contains(&e.from_node) && inside.contains(&e.to_node))
            .cloned()
            .collect();
        let bbox = nodes.iter().fold(
            [f32::MAX, f32::MAX, f32::MIN, f32::MIN],
            |a, n| {
                [
                    a[0].min(n.position[0]),
                    a[1].min(n.position[1]),
                    a[2].max(n.position[0]),
                    a[3].max(n.position[1]),
                ]
            },
        );
        let mut next_id = sub_nodes.iter().map(|n| n.id).max().map_or(0, |m| m + 1);
        if !inputs.is_empty() {
            let id = next_id;
            next_id += 1;
            sub_nodes.push(NodeInst {
                id,
                type_id: GRAPH_INPUT_TYPE_ID.to_string(),
                type_version: 1,
                position: [bbox[0] - GRAPH_IFACE_NODE_GAP, bbox[1]],
                properties: Default::default(),
                subgraph: None,
                tint: None,
                title: None,
            });
            // Each inbound boundary edge continues inside from the binding
            // node's matching pin to the same destination pin.
            for (e, slug) in &inbound {
                sub_edges.push(Edge {
                    from_node: id,
                    from_pin: slug.clone(),
                    to_node: e.to_node,
                    to_pin: e.to_pin.clone(),
                });
            }
        }
        if !outputs.is_empty() {
            let id = next_id;
            sub_nodes.push(NodeInst {
                id,
                type_id: GRAPH_OUTPUT_TYPE_ID.to_string(),
                type_version: 1,
                position: [bbox[2] + GRAPH_IFACE_NODE_GAP, bbox[1]],
                properties: Default::default(),
                subgraph: None,
                tint: None,
                title: None,
            });
            for (e, slug) in &outbound {
                sub_edges.push(Edge {
                    from_node: e.from_node,
                    from_pin: e.from_pin.clone(),
                    to_node: id,
                    to_pin: slug.clone(),
                });
            }
        }
        let sub = GraphDoc {
            realm: self.doc.realm,
            nodes: sub_nodes,
            edges: sub_edges,
            inputs,
            outputs,
            ..GraphDoc::default()
        };
        let rel = self.next_subgraph_path(content_root);
        let abs = content_root.join(&rel);
        if let Some(dir) = abs.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        save_graph(&abs, &sub).map_err(|e| e.to_string())?;

        // Host side, one transaction: the selection and its edges go, a
        // subgraph node arrives, and the boundary edges reattach to it.
        let anchor = nodes
            .iter()
            .fold([f32::MAX, f32::MAX], |a, n| {
                [a[0].min(n.position[0]), a[1].min(n.position[1])]
            });
        let id = self.doc.next_node_id();
        let removed_nodes: Vec<(usize, NodeInst)> = self
            .doc
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| inside.contains(&n.id))
            .map(|(i, n)| (i, n.clone()))
            .collect();
        let removed_edges: Vec<(usize, Edge)> = self
            .doc
            .edges
            .iter()
            .enumerate()
            .filter(|(_, e)| inside.contains(&e.from_node) || inside.contains(&e.to_node))
            .map(|(i, e)| (i, e.clone()))
            .collect();
        let mut edits = vec![
            GraphEdit::RemoveNodes {
                nodes: removed_nodes,
                edges: removed_edges,
                comments: anchored_comments(&self.doc, &inside)
                    .into_iter()
                    .map(|i| (i, self.doc.comments[i].clone()))
                    .collect(),
                regions: region_collateral(&self.doc, inside.iter().copied()),
            },
            GraphEdit::AddNode(NodeInst {
                id,
                type_id: SUBGRAPH_TYPE_ID.to_string(),
                type_version: 1,
                position: anchor,
                properties: Default::default(),
                subgraph: Some(rel.clone()),
                tint: None, title: None,
            }),
        ];
        for (e, slug) in inbound {
            edits.push(GraphEdit::Connect(Edge {
                from_node: e.from_node,
                from_pin: e.from_pin,
                to_node: id,
                to_pin: slug,
            }));
        }
        for (e, slug) in outbound {
            edits.push(GraphEdit::Connect(Edge {
                from_node: id,
                from_pin: slug,
                to_node: e.to_node,
                to_pin: e.to_pin,
            }));
        }
        let edit = GraphEdit::Composite {
            label: "Collapse to Subgraph".to_string(),
            edits,
        };
        edit.apply(&mut self.doc);
        self.select_only(id);
        self.commit(edit, registry);
        Ok(rel)
    }

    /// First free `subgraphs/<host stem>_<n>.subgraph` beside the host.
    fn next_subgraph_path(&self, content_root: &Path) -> String {
        let dir = Path::new(&self.path)
            .parent()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .filter(|p| !p.is_empty())
            .map(|p| format!("{p}/subgraphs"))
            .unwrap_or_else(|| "subgraphs".to_string());
        let stem = Path::new(&self.path)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "graph".to_string());
        for n in 1..10_000u32 {
            let rel = format!("{dir}/{stem}_{n}.subgraph");
            if !content_root.join(&rel).exists() {
                return rel;
            }
        }
        format!("{dir}/{stem}_overflow.subgraph")
    }

    /// Lay the graph out in layers, left to right. `rects` carries each
    /// node's drawn size (the document does not know them). With 2+ nodes
    /// selected only those move; otherwise the whole graph does.
    ///
    /// One `Composite` of per-node moves, so anchored notes ride along for
    /// free and the whole thing is a single undo. Group frames re-fit
    /// afterwards, since their members have moved out from under them.
    ///
    /// Future: the spec's "pinned nodes never move" waits on a pinning
    /// concept, which does not exist yet — every node is fair game today.
    pub fn auto_layout(
        &mut self,
        rects: &[(u64, [f32; 4])],
        spacing: super::graph_layout::LayoutSpacing,
        registry: &NodeRegistry,
    ) {
        use super::graph_layout::{layout, LayoutEdge, LayoutNode};

        let scope: BTreeSet<u64> = if self.selection.len() >= 2 {
            self.selection.clone()
        } else {
            self.doc.nodes.iter().map(|n| n.id).collect()
        };
        if scope.len() < 2 {
            return;
        }
        let nodes: Vec<LayoutNode> = rects
            .iter()
            .filter(|(id, _)| scope.contains(id))
            .map(|(id, r)| LayoutNode { id: *id, width: r[2], height: r[3] })
            .collect();
        if nodes.len() < 2 {
            return;
        }
        let edges: Vec<LayoutEdge> = self
            .doc
            .edges
            .iter()
            .filter(|e| scope.contains(&e.from_node) && scope.contains(&e.to_node))
            .map(|e| LayoutEdge { from: e.from_node, to: e.to_node })
            .collect();

        // Anchor on the selection's existing top-left, so a layout does not
        // also teleport the work away from where the author was looking.
        let origin = rects
            .iter()
            .filter(|(id, _)| scope.contains(id))
            .fold([f32::MAX, f32::MAX], |a, (_, r)| [a[0].min(r[0]), a[1].min(r[1])]);

        let placed = layout(&nodes, &edges, origin, spacing);
        let edits: Vec<GraphEdit> = placed
            .into_iter()
            .filter_map(|(id, p)| {
                let now = self.doc.node(id)?.position;
                let delta = [p[0] - now[0], p[1] - now[1]];
                (delta[0].abs() > f32::EPSILON || delta[1].abs() > f32::EPSILON)
                    .then_some(GraphEdit::MoveNodes { ids: vec![id], delta })
            })
            .collect();
        if edits.is_empty() {
            return;
        }
        let n = edits.len();
        let edit = GraphEdit::Composite { label: "Auto Layout".to_string(), edits };
        edit.apply(&mut self.doc);
        self.commit(edit, registry);
        self.refit_groups(registry);
        self.toast(format!("Laid out {n} node{}", if n == 1 { "" } else { "s" }));
    }

    /// Re-fit every group frame around whatever now sits inside it. Groups
    /// are fixed containers, so after a layout the frame has to follow.
    fn refit_groups(&mut self, registry: &NodeRegistry) {
        const PAD: f32 = 24.0;
        const NODE_EXT: [f32; 2] = [168.0, 100.0];
        let mut edits = Vec::new();
        for (i, g) in self.doc.groups.iter().enumerate() {
            let members = nodes_captured_by_rect(
                &self
                    .doc
                    .nodes
                    .iter()
                    .map(|n| {
                        (
                            n.id,
                            [
                                n.position[0] + NODE_EXT[0] * 0.5,
                                n.position[1] + NODE_EXT[1] * 0.5,
                            ],
                        )
                    })
                    .collect::<Vec<_>>(),
                g.rect,
            );
            if members.is_empty() {
                continue;
            }
            let (mut x0, mut y0, mut x1, mut y1) =
                (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
            for id in &members {
                if let Some(n) = self.doc.node(*id) {
                    x0 = x0.min(n.position[0]);
                    y0 = y0.min(n.position[1]);
                    x1 = x1.max(n.position[0] + NODE_EXT[0]);
                    y1 = y1.max(n.position[1] + NODE_EXT[1]);
                }
            }
            let want = [x0 - PAD, y0 - PAD, (x1 - x0) + PAD * 2.0, (y1 - y0) + PAD * 2.0];
            if want != g.rect {
                edits.push(GraphEdit::ResizeAnnotation {
                    target: Annotation::Group(i),
                    old: g.rect,
                    new: want,
                });
            }
        }
        if edits.is_empty() {
            return;
        }
        let edit = GraphEdit::Composite { label: "Fit Groups".to_string(), edits };
        edit.apply(&mut self.doc);
        self.commit(edit, registry);
    }

    /// Store the current view in the next bookmark slot, cycling 1..=5.
    /// Returns the 1-based slot written.
    pub fn store_bookmark(&mut self) -> usize {
        let slot = self.bookmark_next;
        self.bookmarks[slot] = Some(self.view);
        self.bookmark_next = (slot + 1) % BOOKMARK_SLOTS;
        slot + 1
    }

    /// Recall a 1-based bookmark slot. `false` when the slot is empty.
    pub fn recall_bookmark(&mut self, slot: usize) -> bool {
        let Some(v) = slot
            .checked_sub(1)
            .and_then(|i| self.bookmarks.get(i))
            .copied()
            .flatten()
        else {
            return false;
        };
        self.view = v;
        true
    }

    /// Nodes with no path to any impure (side-effecting) node — the ones a
    /// purge would remove.
    ///
    /// Reachability runs **backwards** from every impure node over both exec
    /// and data edges: a node earns its place by feeding something that acts.
    /// Subgraph instances count as impure (their contents are opaque here, so
    /// assuming they act is the safe direction); reroutes never seed, they
    /// only pass reachability along. A graph with no impure nodes at all is
    /// pure computation with no output — there is nothing to reach, so the
    /// purge is a no-op rather than a wipe.
    pub fn unused_nodes(&self, registry: &NodeRegistry) -> Vec<u64> {
        let seeds: BTreeSet<u64> = self
            .doc
            .nodes
            .iter()
            .filter(|n| {
                if n.type_id == SUBGRAPH_TYPE_ID {
                    return true;
                }
                if n.type_id == REROUTE_TYPE_ID {
                    return false;
                }
                // A `graph_output` node is the document's *result*: whatever
                // feeds it is reached by definition, even when the interface
                // is pure data and the node therefore reads as pure.
                if n.type_id == GRAPH_OUTPUT_TYPE_ID {
                    return true;
                }
                // Purity is an instance question — a `var_set` writes and a
                // `var_get` does not — so it comes from the resolver. An
                // unresolvable type is not evidence of uselessness.
                DocDescriptors::new(&self.doc, registry)
                    .descriptor(n.id)
                    .map(|d| !d.pure)
                    .unwrap_or(true)
            })
            .map(|n| n.id)
            .collect();
        if seeds.is_empty() {
            return Vec::new();
        }
        let mut reached = seeds.clone();
        let mut frontier: Vec<u64> = seeds.into_iter().collect();
        while let Some(at) = frontier.pop() {
            for e in self.doc.edges.iter().filter(|e| e.to_node == at) {
                if reached.insert(e.from_node) {
                    frontier.push(e.from_node);
                }
            }
        }
        self.doc
            .nodes
            .iter()
            .map(|n| n.id)
            .filter(|id| !reached.contains(id))
            .collect()
    }

    /// Remove `ids` and their incident edges as one transaction.
    pub fn purge_nodes(&mut self, ids: &[u64], registry: &NodeRegistry) {
        let set: BTreeSet<u64> = ids.iter().copied().collect();
        if set.is_empty() {
            return;
        }
        self.cancel_interactions();
        let nodes: Vec<(usize, NodeInst)> = self
            .doc
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| set.contains(&n.id))
            .map(|(i, n)| (i, n.clone()))
            .collect();
        let edges: Vec<(usize, Edge)> = self
            .doc
            .edges
            .iter()
            .enumerate()
            .filter(|(_, e)| set.contains(&e.from_node) || set.contains(&e.to_node))
            .map(|(i, e)| (i, e.clone()))
            .collect();
        let comments: Vec<(usize, CommentBox)> = anchored_comments(&self.doc, &set)
            .into_iter()
            .map(|i| (i, self.doc.comments[i].clone()))
            .collect();
        let n = nodes.len();
        let edit = GraphEdit::Composite {
            label: format!("Purge {n} Node{}", if n == 1 { "" } else { "s" }),
            edits: vec![GraphEdit::RemoveNodes {
                nodes,
                edges,
                comments,
                regions: region_collateral(&self.doc, set.iter().copied()),
            }],
        };
        edit.apply(&mut self.doc);
        self.selection.retain(|id| !set.contains(id));
        self.commit(edit, registry);
    }

    /// Move the selection by `delta` as part of a held-key nudge.
    ///
    /// The first call opens the transaction; later calls extend it. Nothing
    /// reaches the undo stack until [`commit_nudge`](Self::commit_nudge).
    pub fn nudge_selection(&mut self, delta: [f32; 2]) {
        if self.selection.is_empty() {
            return;
        }
        let n = self.nudge.get_or_insert_with(|| Nudge {
            originals: Vec::new(),
            delta: [0.0, 0.0],
        });
        if n.originals.is_empty() {
            n.originals = self
                .doc
                .nodes
                .iter()
                .filter(|node| self.selection.contains(&node.id))
                .map(|node| (node.id, node.position))
                .collect();
        }
        n.delta[0] += delta[0];
        n.delta[1] += delta[1];
        let (dx, dy) = (n.delta[0], n.delta[1]);
        let originals = n.originals.clone();
        for (id, start) in &originals {
            if let Some(node) = self.doc.nodes.iter_mut().find(|n| n.id == *id) {
                node.position = [start[0] + dx, start[1] + dy];
            }
        }
        self.dirty = true;
    }

    /// Close an open nudge, recording the whole hold as one `MoveNodes`.
    pub fn commit_nudge(&mut self, registry: &NodeRegistry) {
        let Some(n) = self.nudge.take() else {
            return;
        };
        if n.delta == [0.0, 0.0] || n.originals.is_empty() {
            return;
        }
        let ids: Vec<u64> = n.originals.iter().map(|(id, _)| *id).collect();
        self.commit(GraphEdit::MoveNodes { ids, delta: n.delta }, registry);
    }

    /// Is a nudge open? (Escape reverts it through `cancel_interactions`.)
    pub fn nudging(&self) -> bool {
        self.nudge.is_some()
    }

    /// `F7` — run validation and say what it found.
    ///
    /// The honest pre-45-A reading of "compile": there is no evaluator yet, so
    /// compiling *is* validating. Refreshes the count chip's source of truth
    /// and reports, because a keypress that silently changes a chip in the
    /// corner has not told the user anything.
    pub fn compile(&mut self, registry: &NodeRegistry) {
        self.errors = validate_doc(&self.doc, registry);
        let n = self.errors.len() + self.ref_errors.len();
        if n == 0 {
            self.toast("Valid");
        } else if n == 1 {
            self.toast("1 error");
        } else {
            self.toast(format!("{n} errors"));
        }
    }

    /// `F9` — add or remove a breakpoint on the last-clicked node.
    ///
    /// Presence, not armed-ness: F9 is how a mark comes into existence and how
    /// it goes away. Arming and disarming an existing one is the gutter click,
    /// which is where the mark you are looking at already is.
    pub fn toggle_breakpoint(&mut self) {
        let Some(id) = self.primary.or_else(|| self.selection.iter().copied().next()) else {
            return;
        };
        if self.breakpoints.remove(&id).is_none() {
            self.breakpoints.insert(id, true);
        }
    }

    /// Gutter click — armed ⇄ disabled, keeping the mark.
    pub fn cycle_breakpoint(&mut self, id: u64) {
        if let Some(armed) = self.breakpoints.get_mut(&id) {
            *armed = !*armed;
        }
    }

    /// Alt+click on the gutter — destroy, per the input contract's Alt verb.
    pub fn remove_breakpoint(&mut self, id: u64) {
        self.breakpoints.remove(&id);
    }

    /// `Ctrl+Shift+F9` — clear every mark, reporting the count. Silent when
    /// there was nothing to clear.
    pub fn clear_breakpoints(&mut self) {
        let n = self.breakpoints.len();
        if n == 0 {
            return;
        }
        self.breakpoints.clear();
        self.toast(if n == 1 {
            "Cleared 1 breakpoint".to_string()
        } else {
            format!("Cleared {n} breakpoints")
        });
    }

    /// Does this node carry a mark at all — armed or disabled?
    pub fn has_breakpoint(&self, id: u64) -> bool {
        self.breakpoints.contains_key(&id)
    }

    /// Is the mark on this node armed? A disabled one draws hollow and arms
    /// nothing.
    pub fn breakpoint_armed(&self, id: u64) -> bool {
        self.breakpoints.get(&id).copied().unwrap_or(false)
    }

    /// The marks that go to the runner — armed only, in document order.
    pub fn armed_breakpoints(&self) -> Vec<u64> {
        self.breakpoints
            .iter()
            .filter(|(_, armed)| **armed)
            .map(|(id, _)| *id)
            .collect()
    }

    /// `F2` — open the inline editor on the selected annotation.
    ///
    /// Annotations only: a node's title comes from its descriptor and
    /// `NodeInst` has no override field, so there is nothing to edit. Recorded
    /// for Task 45-A rather than faked.
    pub fn begin_rename(&mut self) -> bool {
        if let Some(i) = self.sel_comment {
            if let Some(c) = self.doc.comments.get(i) {
                self.editing = Some(AnnotationEdit {
                    is_group: false,
                    index: i,
                    buffer: c.text.clone(),
                    original: c.text.clone(),
                    anchor_world: [c.rect[0], c.rect[1]],
                    first_frame: true,
                });
                return true;
            }
        }
        if let Some(i) = self.sel_group {
            if let Some(g) = self.doc.groups.get(i) {
                self.editing = Some(AnnotationEdit {
                    is_group: true,
                    index: i,
                    buffer: g.title.clone(),
                    original: g.title.clone(),
                    anchor_world: [g.rect[0], g.rect[1]],
                    first_frame: true,
                });
                return true;
            }
        }
        false
    }

    /// `PageDown` — the file the last-clicked node descends into, if any.
    /// The caller opens it as a tab; we only record where we came from.
    pub fn descend_target(&self) -> Option<String> {
        let id = self.primary.or_else(|| self.selection.iter().copied().next())?;
        self.file_descend_target(id)
    }

    /// The file `id` descends into — "double-click always means descend"
    /// (spec): a script node's `.subgraph` asset, or the nested `.animgraph`
    /// an animation state references (ticket 09). A state whose region holds
    /// a blend tree answers nothing, exactly as the compiler ignores its
    /// `graph` property then.
    pub fn file_descend_target(&self, id: u64) -> Option<String> {
        use crate::engine::animation::graph::plan::{ANIM_STATE_TYPE_ID, GRAPH_PROP};
        let n = self.doc.node(id)?;
        if let Some(path) = &n.subgraph {
            return Some(path.clone());
        }
        if !self.domain.is_animation() || n.type_id != ANIM_STATE_TYPE_ID {
            return None;
        }
        if self.doc.regions.get(&id).is_some_and(|r| !r.nodes.is_empty()) {
            return None;
        }
        match n.properties.get(GRAPH_PROP) {
            Some(PropValue::Asset(s)) | Some(PropValue::Str(s)) if !s.trim().is_empty() => {
                Some(crate::engine::scripting::normalize_graph_path(s))
            }
            _ => None,
        }
    }

    /// The request descending from this tab raises: the target file plus the
    /// breadcrumb chain the opened tab should carry (this tab's own chain,
    /// then this tab) — the host seeds the target's `nav_back` with it.
    pub fn open_request(&self, path: String) -> GraphOpenRequest {
        let mut back = self.nav_back.clone();
        back.push(self.path.clone());
        GraphOpenRequest { path, back }
    }

    /// Remember the graph being left, so `PageUp` can come back to it.
    pub fn push_nav(&mut self, from: String) {
        self.nav_back.push(from);
    }

    /// `PageUp` — the graph to return to. `None` (silent no-op) at the root of
    /// this session's descent, which is the honest answer: a subgraph can have
    /// many parents, so "the" parent only exists if you walked in from one.
    pub fn ascend_target(&mut self) -> Option<String> {
        self.nav_back.pop()
    }

    /// Cancel any in-flight drag / inline edit — indices they hold become
    /// invalid on structural edits and on undo/redo.
    ///
    /// Rule 3 of the input model: cancelling **reverts**. Every gesture here
    /// snapshots its pre-drag state precisely so an abandoned drag can put it
    /// back, and none of them has recorded anything on the undo stack yet — so
    /// after this the document reads exactly as it did before the press, and
    /// undo history is untouched.
    pub fn cancel_interactions(&mut self) {
        // The peek's gestures are as abandonable as the machine's. Depth-one
        // recursion: a projection never opens a scope of its own.
        if let Some(scope) = &mut self.rule_scope {
            scope.child.cancel_interactions();
        }
        self.revert_pending_drag();
        if let Some(drag) = self.node_drag.take() {
            for (id, pos) in drag.originals {
                if let Some(n) = self.doc.nodes.iter_mut().find(|n| n.id == id) {
                    n.position = pos;
                }
            }
            for (i, min) in drag.anchored {
                if let Some(c) = self.doc.comments.get_mut(i) {
                    c.rect[0] = min[0];
                    c.rect[1] = min[1];
                }
            }
        }
        if let Some(drag) = self.annotation_drag.take() {
            if drag.is_group {
                if let Some(g) = self.doc.groups.get_mut(drag.index) {
                    g.rect[0] = drag.rect_min0[0];
                    g.rect[1] = drag.rect_min0[1];
                }
            } else if let Some(c) = self.doc.comments.get_mut(drag.index) {
                c.rect[0] = drag.rect_min0[0];
                c.rect[1] = drag.rect_min0[1];
            }
            for (id, pos) in drag.captured {
                if let Some(n) = self.doc.nodes.iter_mut().find(|n| n.id == id) {
                    n.position = pos;
                }
            }
        }
        if let Some(rz) = self.annotation_resize.take() {
            match rz.target {
                Annotation::Group(i) => {
                    if let Some(g) = self.doc.groups.get_mut(i) {
                        g.rect = rz.rect0;
                    }
                }
                Annotation::Comment(i) => {
                    if let Some(c) = self.doc.comments.get_mut(i) {
                        c.rect = rz.rect0;
                    }
                }
            }
        }
        // An inline property drag writes the doc live; put the pre-gesture
        // value back rather than leaving the last dragged one.
        if let Some(p) = self.prop_edit.take() {
            if let Some(n) = self.doc.nodes.iter_mut().find(|n| n.id == p.node) {
                match p.old {
                    Some(v) => {
                        n.properties.insert(p.key, v);
                    }
                    None => {
                        n.properties.remove(&p.key);
                    }
                }
            }
        }
        if let Some(n) = self.nudge.take() {
            for (id, pos) in n.originals {
                if let Some(node) = self.doc.nodes.iter_mut().find(|x| x.id == id) {
                    node.position = pos;
                }
            }
        }
        // These hold no document state — dropping them is the whole revert.
        self.connect_drag = None;
        self.marquee = None;
        self.marquee_mode = MarqueeMode::Replace;
        self.cut_path = None;
        self.editing = None;
    }

    /// Is *any* gesture in flight, including the ones that touch no document
    /// state? [`gesture_in_flight`](Self::gesture_in_flight) deliberately
    /// answers the narrower "is there an unrecorded edit" question for the
    /// undo/save gates; Escape has to reach the others too.
    pub fn interaction_in_flight(&self) -> bool {
        self.gesture_in_flight()
            || self.nudge.is_some()
            || self.connect_drag.is_some()
            || self.marquee.is_some()
            || self.editing.is_some()
    }

    /// Copy the selection to the **system** clipboard as RON (so a paste
    /// survives a restart), keeping the in-memory copy as a fallback for when
    /// the platform clipboard is unavailable.
    pub fn copy_selection(&mut self, clipboard: &mut Option<GraphFragment>) {
        // With a rule peek open, Copy means the peek's selection (ticket 05).
        if self.rule_scope.is_some() {
            if let Some(scope) = &mut self.rule_scope {
                scope.child.copy_selection(clipboard);
            }
            return;
        }
        let Some(frag) = self.selection_fragment() else {
            return;
        };
        match frag.to_ron() {
            Ok(text) => crusty_gui::clipboard::set_text(text),
            Err(e) => println!("graph: clipboard serialize failed: {e}"),
        }
        let n = frag.nodes.len();
        *clipboard = Some(frag);
        self.toast(format!("Copied {n} node{}", if n == 1 { "" } else { "s" }));
    }

    /// The in-memory half of copy, without touching the OS clipboard.
    pub fn copy_selection_local(&mut self, clipboard: &mut Option<GraphFragment>) {
        if let Some(frag) = self.selection_fragment() {
            *clipboard = Some(frag);
        }
    }

    /// Build a fragment from the current selection, or `None` if there is
    /// nothing to copy.
    ///
    /// Annotations come along: the selected comment/group, plus every comment
    /// *anchored* to a copied node — an anchored note explains that node, so
    /// copying the node without it would copy half the thought.
    pub fn selection_fragment(&self) -> Option<GraphFragment> {
        let nodes: Vec<NodeInst> = self
            .doc
            .nodes
            .iter()
            .filter(|n| self.selection.contains(&n.id))
            .cloned()
            .collect();
        let edges: Vec<Edge> = self
            .doc
            .edges
            .iter()
            .filter(|e| {
                self.selection.contains(&e.from_node) && self.selection.contains(&e.to_node)
            })
            .cloned()
            .collect();

        let anchored: BTreeSet<usize> = anchored_comments(&self.doc, &self.selection)
            .into_iter()
            .collect();
        let mut comments: Vec<CommentBox> = Vec::new();
        for (i, c) in self.doc.comments.iter().enumerate() {
            if anchored.contains(&i) || self.sel_comment == Some(i) {
                comments.push(c.clone());
            }
        }
        let groups: Vec<GroupBox> = self
            .sel_group
            .and_then(|i| self.doc.groups.get(i))
            .cloned()
            .into_iter()
            .collect();
        // A copied node's embedded region travels with it — copying a
        // transition without its rule would copy half the thought, the same
        // rule anchored comments follow.
        let regions: BTreeMap<u64, GraphRegion> = self
            .selection
            .iter()
            .filter_map(|id| Some((*id, self.doc.regions.get(id)?.clone())))
            .collect();

        let frag = GraphFragment { nodes, edges, comments, groups, regions };
        (!frag.is_empty()).then_some(frag)
    }

    /// Paste at `at` (world space) preserving the fragment's relative layout.
    ///
    /// The system clipboard wins when it holds one of our fragments — that is
    /// what makes paste work across sessions — and the in-memory copy is the
    /// fallback. Text that is not ours is a no-op with a note, never a panic.
    pub fn paste_clipboard(
        &mut self,
        clipboard: &Option<GraphFragment>,
        at: Option<[f32; 2]>,
        registry: &NodeRegistry,
    ) {
        // With a rule peek open, Paste lands in the peek. `at` came from the
        // machine's view, which is not the peek's — the fragment's own
        // layout is the honest fallback. (Pastes *inside* the peek's canvas
        // go straight to the child and keep their cursor position.)
        if self.rule_scope.is_some() {
            if let Some(mut scope) = self.rule_scope.take() {
                scope.child.paste_clipboard(clipboard, None, rule_scope_registry());
                self.rule_scope = Some(scope);
            }
            self.drain_rule_scope(registry);
            return;
        }
        let from_os = crusty_gui::clipboard::get_text()
            .as_deref()
            .and_then(GraphFragment::from_ron);
        let frag = match (from_os, clipboard) {
            (Some(f), _) => f,
            (None, Some(f)) => f.clone(),
            (None, None) => {
                // Part 6: a valid-but-empty gesture is a silent no-op. Ctrl+V
                // with nothing to paste is not an error, it is nothing.
                return;
            }
        };
        self.paste_fragment(&frag, at, registry);
    }

    /// Duplicate the selection in place. Deliberately does **not** touch the
    /// clipboard: duplicating should not cost you what you copied earlier.
    pub fn duplicate_selection(&mut self, registry: &NodeRegistry) {
        // With a rule peek open, Duplicate means the peek's selection.
        if self.rule_scope.is_some() {
            if let Some(scope) = &mut self.rule_scope {
                scope.child.duplicate_selection(rule_scope_registry());
            }
            self.drain_rule_scope(registry);
            return;
        }
        if let Some(frag) = self.selection_fragment() {
            let min = frag.bbox_min();
            let at = [min[0] + Self::DUPLICATE_OFFSET, min[1] + Self::DUPLICATE_OFFSET];
            self.paste_fragment(&frag, Some(at), registry);
        }
    }

    /// `Ctrl+D` lands the copy just off its original, close enough to read as
    /// a duplicate rather than a new thought.
    const DUPLICATE_OFFSET: f32 = 16.0;
    /// Nudge step used to walk a paste clear of what is already there.
    const NUDGE: f32 = 8.0;
    /// Give up nudging rather than walking off the canvas forever.
    const MAX_NUDGES: usize = 64;

    /// Paste a fragment we already hold, at `at` (world). Separate from
    /// [`paste_clipboard`](Self::paste_clipboard) so nothing but the actual
    /// clipboard path touches the OS clipboard — reaching for it is a
    /// process-global, single-threaded-only operation, and a unit test has no
    /// business depending on what the machine happens to have copied.
    pub fn paste_fragment(
        &mut self,
        frag: &GraphFragment,
        at: Option<[f32; 2]>,
        registry: &NodeRegistry,
    ) {
        if frag.is_empty() {
            return;
        }
        // Placement gating holds through the clipboard back door (ticket 05):
        // on an animation canvas, a fragment carrying node types the active
        // registry does not offer is refused whole, with a note — pasting
        // half a clipboard would be worse. Reserved doc-dependent types
        // (`var_get`, `reroute`, …) have no descriptors anywhere and pass;
        // the compiler's whitelist judges them where they land.
        if self.domain.is_animation_family() {
            let foreign = frag
                .nodes
                .iter()
                .filter(|n| {
                    registry.get(&n.type_id).is_none()
                        && !crate::engine::node_graph::RESERVED_TYPE_IDS
                            .contains(&n.type_id.as_str())
                })
                .count();
            if foreign > 0 {
                self.toast(format!(
                    "{foreign} node{} don't belong on this canvas",
                    if foreign == 1 { "" } else { "s" }
                ));
                return;
            }
        }
        // Anchor the fragment's top-left at the target and keep every
        // internal offset, so a pasted cluster arrives shaped as it was.
        let min = frag.bbox_min();
        let target = at.unwrap_or([min[0] + Self::DUPLICATE_OFFSET, min[1] + Self::DUPLICATE_OFFSET]);
        let mut offset = [target[0] - min[0], target[1] - min[1]];

        // Nudge until the landing spot is clear — the palette's drop rule.
        for _ in 0..Self::MAX_NUDGES {
            let clash = frag.nodes.iter().any(|n| {
                let p = [n.position[0] + offset[0], n.position[1] + offset[1]];
                self.doc
                    .nodes
                    .iter()
                    .any(|e| (e.position[0] - p[0]).abs() < 1.0 && (e.position[1] - p[1]).abs() < 1.0)
            });
            if !clash {
                break;
            }
            offset[0] += Self::NUDGE;
            offset[1] += Self::NUDGE;
        }

        let out = frag.instantiate(self.doc.next_node_id(), offset);
        let (n, dropped) = (out.nodes.len(), out.dropped);
        let mut edits = vec![GraphEdit::Paste {
            nodes: out.nodes.clone(),
            edges: out.edges,
            regions: out.regions,
        }];
        for c in out.comments {
            edits.push(GraphEdit::AddComment(c));
        }
        for g in out.groups {
            edits.push(GraphEdit::AddGroup(g));
        }
        let edit = GraphEdit::Composite {
            label: format!("Paste {n} Node{}", if n == 1 { "" } else { "s" }),
            edits,
        };
        edit.apply(&mut self.doc);
        // Move selection to the pasted nodes; clear any annotation selection so
        // a following Delete hits the pasted nodes, not an off-screen comment.
        self.clear_selection();
        self.selection = out.nodes.iter().map(|x| x.id).collect();
        self.primary = out.nodes.last().map(|x| x.id);
        self.commit(edit, registry);

        if dropped > 0 {
            self.toast(format!(
                "Pasted {n} node{}, {dropped} link{} dropped",
                if n == 1 { "" } else { "s" },
                if dropped == 1 { "" } else { "s" }
            ));
        } else {
            self.toast(format!("Pasted {n} node{}", if n == 1 { "" } else { "s" }));
        }
    }
}

/// Build the transitive-closure document map used as a [`GraphResolver`]
/// (P6): open editor docs win over disk (so unsaved edits validate against
/// what the user sees), and every subgraph they reference — directly or
/// transitively — is loaded from `content_root` and cached in the returned
/// map. `BTreeMap<String, GraphDoc>` already implements `GraphResolver`, so
/// the returned map is the resolver.
pub fn build_resolver_docs<'a>(
    open: impl Iterator<Item = (&'a str, &'a GraphDoc)>,
    content_root: &Path,
) -> BTreeMap<String, GraphDoc> {
    let mut docs: BTreeMap<String, GraphDoc> =
        open.map(|(k, d)| (k.to_string(), d.clone())).collect();
    let mut frontier: Vec<String> = docs
        .values()
        .flat_map(|d| d.subgraph_refs().into_iter().map(str::to_string))
        .collect();
    while let Some(path) = frontier.pop() {
        if docs.contains_key(&path) {
            continue;
        }
        // Missing-on-disk refs are left absent → `MissingSubgraph` at validate.
        if let Ok(d) = load_graph(&content_root.join(&path)) {
            frontier.extend(d.subgraph_refs().into_iter().map(str::to_string));
            docs.insert(path, d);
        }
    }
    docs
}

/// The `.curve` half of the same idea (45-A P8b): every curve the resolver
/// documents reference, with open curve editors winning over disk.
///
/// Open-wins is the rule `build_resolver_docs` already sets for subgraphs, and
/// for the same reason — a Timeline's pins are its curve's track names, so an
/// author who just added a track in the curve tab must see the pin appear on
/// the node they are about to wire it to, not after a save. `BTreeMap<String,
/// CurveDoc>` implements `CurveResolver`, so the returned map *is* the
/// resolver.
pub fn build_curve_docs<'a>(
    open: impl Iterator<Item = (&'a str, &'a curve_asset::CurveDoc)>,
    docs: &BTreeMap<String, GraphDoc>,
    content_root: &Path,
) -> BTreeMap<String, curve_asset::CurveDoc> {
    let mut curves: BTreeMap<String, curve_asset::CurveDoc> =
        open.map(|(k, d)| (k.to_string(), d.clone())).collect();
    for path in docs.values().flat_map(|d| d.curve_refs()) {
        if curves.contains_key(path) {
            continue;
        }
        // Missing on disk is left absent → `MissingCurve` at validate, which
        // says the path the author typed. Curves have no references of their
        // own, so there is no frontier to walk.
        if let Ok(text) = std::fs::read_to_string(content_root.join(path)) {
            if let Ok(d) = curve_asset::parse_curve(&text) {
                curves.insert(path.to_string(), d);
            }
        }
    }
    curves
}

/// Fit a [`CanvasView`] so the world-space box `[bbox_min, bbox_max]` fills the
/// viewport with a small margin, centered. Framing never magnifies past 1.0×
/// (a single node frames at 100%, not the max zoom), then clamps into the pref
/// zoom range. `viewport_size` is in screen pixels; `pan` is the world point at
/// the canvas top-left (`CanvasView` convention). Used by the F/A shortcuts.
pub fn frame_view(
    bbox_min: Vec2,
    bbox_max: Vec2,
    viewport_size: Vec2,
    zoom_min: f32,
    zoom_max: f32,
) -> CanvasView {
    const PAD: f32 = 0.9;
    let w = (bbox_max.x - bbox_min.x).max(1.0);
    let h = (bbox_max.y - bbox_min.y).max(1.0);
    let fit = (viewport_size.x / w).min(viewport_size.y / h) * PAD;
    // Cap at 1.0 so framing never zooms *in* past 100% (single node → 1.0×,
    // not max), then clamp into the configured range.
    let zoom = fit.min(1.0).clamp(zoom_min, zoom_max);
    let cx = (bbox_min.x + bbox_max.x) * 0.5;
    let cy = (bbox_min.y + bbox_max.y) * 0.5;
    let pan = Vec2::new(
        cx - viewport_size.x / (2.0 * zoom),
        cy - viewport_size.y / (2.0 * zoom),
    );
    CanvasView { pan, zoom }
}

/// Read-only display string for a stored input constant.
pub fn prop_display(v: &PropValue) -> String {
    match v {
        PropValue::Float(x) => format!("{x}"),
        PropValue::Int(i) => format!("{i}"),
        PropValue::Vec2(a) => format!("({}, {})", a[0], a[1]),
        PropValue::Vec3(a) => format!("({}, {}, {})", a[0], a[1], a[2]),
        PropValue::Vec4(a) => format!("({}, {}, {}, {})", a[0], a[1], a[2], a[3]),
        PropValue::Color(a) => format!("#{:.2},{:.2},{:.2},{:.2}", a[0], a[1], a[2], a[3]),
        PropValue::Bool(b) => b.to_string(),
        PropValue::Enum(s) => s.clone(),
        PropValue::Str(s) => s.clone(),
        // Arrays have no literal editor in v1 (D9); the chip states the shape
        // rather than pretending to show the contents.
        PropValue::Array(v) => format!(
            "[{}]",
            v.iter().map(prop_display).collect::<Vec<_>>().join(", ")
        ),
        PropValue::Asset(s) => s.clone(),
        PropValue::Raw(s) => s.clone(),
    }
}

/// Test-only helpers that reach into the panel's gesture finishers, so a
/// state-level test can exercise a whole drag without a `Ui`.
#[cfg(test)]
pub mod tests_support {
    use super::*;

    /// The panel's `finish_node_drag`, in the one shape a state test needs.
    pub fn finish_drag(state: &mut GraphEditorState, registry: &NodeRegistry) {
        super::super::graph_editor_crusty::finish_node_drag_for_test(state, registry);
    }

    /// An empty editor state, for tests that only need somewhere to hang a doc.
    pub fn empty_state() -> GraphEditorState {
        super::test_state("t.graph")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::node_graph::doc::GraphDoc;
    use crate::engine::node_graph::{validate_doc, GraphError, NodeDescriptor};

    fn node(id: u64, pos: [f32; 2]) -> NodeInst {
        NodeInst {
            id,
            type_id: "test_add".to_string(),
            type_version: 1,
            position: pos,
            properties: std::collections::BTreeMap::new(),
            subgraph: None,
        
        tint: None, title: None,}
    }

    fn edge(a: u64, b: u64) -> Edge {
        Edge {
            from_node: a,
            from_pin: "sum".to_string(),
            to_node: b,
            to_pin: "a".to_string(),
        }
    }

    fn comment(x: f32) -> CommentBox {
        CommentBox { rect: [x, 0.0, 100.0, 60.0], text: "c".to_string(), ..Default::default() }
    }

    fn group(x: f32) -> GroupBox {
        GroupBox { rect: [x, 0.0, 200.0, 200.0], title: "g".to_string(), ..Default::default() }
    }

    /// apply → revert round-trips every annotation edit variant (P7).
    #[test]
    fn annotation_edits_round_trip() {
        let base = {
            let mut d = GraphDoc::default();
            d.nodes = vec![node(0, [0.0, 0.0]), node(1, [10.0, 10.0])];
            d.comments = vec![comment(0.0), comment(300.0)];
            d.groups = vec![group(0.0)];
            d
        };
        let edits = [
            GraphEdit::SetAnnotationTint {
                target: Annotation::Comment(0),
                old: None,
                new: Some(9),
            },
            GraphEdit::SetAnnotationTint {
                target: Annotation::Group(0),
                old: None,
                new: Some(4),
            },
            GraphEdit::SetAnnotationCollapsed { target: Annotation::Comment(1), new: true },
            GraphEdit::SetAnnotationCollapsed { target: Annotation::Group(0), new: true },
            GraphEdit::ResizeAnnotation {
                target: Annotation::Comment(0),
                old: [0.0, 0.0, 100.0, 60.0],
                new: [0.0, 0.0, 260.0, 180.0],
            },
            GraphEdit::ResizeAnnotation {
                target: Annotation::Group(0),
                old: [0.0, 0.0, 200.0, 200.0],
                new: [-20.0, -20.0, 240.0, 240.0],
            },
            GraphEdit::SetCommentAnchor { index: 0, old: None, new: Some(1) },
            GraphEdit::AddComment(comment(999.0)),
            GraphEdit::RemoveComment { index: 1, comment: comment(300.0) },
            GraphEdit::MoveNodes { ids: vec![0], delta: [9.0, -3.0] },
            GraphEdit::MoveComment { index: 0, delta: [12.0, -7.0] },
            GraphEdit::SetCommentText {
                index: 0,
                old: "c".to_string(),
                new: "hello".to_string(),
            },
            GraphEdit::AddGroup(group(500.0)),
            GraphEdit::RemoveGroup { index: 0, group: group(0.0) },
            GraphEdit::MoveGroup { index: 0, node_ids: vec![0, 1], delta: [5.0, 6.0] },
            GraphEdit::SetGroupTitle {
                index: 0,
                old: "g".to_string(),
                new: "renamed".to_string(),
            },
        ];
        for e in edits {
            let mut doc = base.clone();
            e.apply(&mut doc);
            assert_ne!(doc, base, "{}: apply should change the doc", e.description());
            e.revert(&mut doc);
            assert_eq!(doc, base, "{}: apply→revert must restore", e.description());
        }
    }

    /// An anchored note travels with the node it explains, in both
    /// directions — a move that undoes must carry the note back too.
    #[test]
    fn anchored_comment_follows_and_unfollows_its_node() {
        let mut st = bare_state();
        st.doc.nodes = vec![node(0, [0.0, 0.0]), node(1, [200.0, 0.0])];
        st.doc.comments = vec![comment(10.0), comment(400.0)];
        st.doc.comments[0].anchor = Some(0);
        let before = st.doc.clone();

        // Moving the anchored node carries its note; the free-floating one
        // stays put.
        let edit = GraphEdit::MoveNodes { ids: vec![0], delta: [30.0, -12.0] };
        edit.apply(&mut st.doc);
        assert_eq!(st.doc.comments[0].rect[0], 40.0);
        assert_eq!(st.doc.comments[0].rect[1], -12.0);
        assert_eq!(st.doc.comments[1].rect[0], 400.0, "a free note must not move");
        edit.revert(&mut st.doc);
        assert_eq!(st.doc, before, "the note must come back with its node");

        // Moving the *other* node touches nothing.
        let other = GraphEdit::MoveNodes { ids: vec![1], delta: [50.0, 50.0] };
        other.apply(&mut st.doc);
        assert_eq!(st.doc.comments[0].rect[0], 10.0);
        other.revert(&mut st.doc);
        assert_eq!(st.doc, before);
    }

    /// Deleting a node takes its anchored notes with it, index-carrying, and
    /// undo restores both at their exact positions.
    #[test]
    fn deleting_a_node_takes_its_anchored_notes() {
        let reg = NodeRegistry::new();
        let mut st = bare_state();
        st.doc.nodes = vec![node(0, [0.0, 0.0]), node(1, [200.0, 0.0])];
        st.doc.comments = vec![comment(0.0), comment(300.0), comment(600.0)];
        st.doc.comments[1].anchor = Some(0); // dies with node 0
        st.doc.comments[2].anchor = Some(1); // survives
        let before = st.doc.clone();

        st.selection = [0u64].into_iter().collect();
        st.delete_selection(&reg);
        assert_eq!(st.doc.nodes.len(), 1);
        assert_eq!(
            st.doc.comments.len(),
            2,
            "only the note anchored to the deleted node goes"
        );
        assert_eq!(st.doc.comments[1].anchor, Some(1), "the survivor kept its anchor");

        st.undo(&reg);
        assert_eq!(st.doc, before, "undo restores the note at its original index");
        let a = crate::engine::node_graph::serialize_graph(&before).unwrap();
        let b = crate::engine::node_graph::serialize_graph(&st.doc).unwrap();
        assert_eq!(a, b, "restored doc must serialize byte-identically");
    }

    /// Tint, collapse and anchor go through the undo stack, and a no-op
    /// change records nothing.
    #[test]
    fn annotation_field_edits_are_undoable_and_skip_no_ops() {
        let reg = NodeRegistry::new();
        let mut st = bare_state();
        st.doc.nodes = vec![node(0, [0.0, 0.0])];
        st.doc.comments = vec![comment(0.0)];
        st.doc.groups = vec![group(0.0)];

        st.set_annotation_tint(Annotation::Comment(0), Some(9), &reg);
        assert_eq!(st.doc.comments[0].tint, Some(9));
        assert_eq!(st.stack.undo_description().as_deref(), Some("Tint Comment"));
        // Setting the same value again is not an edit.
        st.set_annotation_tint(Annotation::Comment(0), Some(9), &reg);
        st.undo(&reg);
        assert_eq!(st.doc.comments[0].tint, None);

        st.toggle_annotation_collapsed(Annotation::Group(0), &reg);
        assert!(st.doc.groups[0].collapsed);
        st.undo(&reg);
        assert!(!st.doc.groups[0].collapsed);

        st.set_comment_anchor(0, Some(0), &reg);
        assert_eq!(st.doc.comments[0].anchor, Some(0));
        st.set_comment_anchor(0, Some(0), &reg); // no-op
        st.undo(&reg);
        assert_eq!(st.doc.comments[0].anchor, None);

        // An out-of-range target is ignored rather than panicking.
        st.set_annotation_tint(Annotation::Comment(99), Some(1), &reg);
        st.toggle_annotation_collapsed(Annotation::Group(99), &reg);
    }

    /// A resize gesture coalesces into one entry and reverts exactly.
    #[test]
    fn resize_coalesces_into_one_entry() {
        let reg = NodeRegistry::new();
        let mut st = bare_state();
        st.doc.comments = vec![comment(0.0)];
        let before = st.doc.clone();
        let rect0 = st.doc.comments[0].rect;

        st.annotation_resize = Some(AnnotationResize {
            target: Annotation::Comment(0),
            handle: ResizeHandle::BottomRight,
            origin_world: [0.0, 0.0],
            rect0,
            min_h: 0.0,
        });
        // Live frames write the rect directly; only the release records.
        for w in [120.0f32, 160.0, 210.0] {
            st.doc.comments[0].rect[2] = w;
        }
        st.finish_annotation_resize(&reg);
        assert_eq!(st.doc.comments[0].rect[2], 210.0);
        assert!(st.stack.can_undo());
        st.undo(&reg);
        assert_eq!(st.doc, before, "one gesture = one undo entry");
        assert!(!st.stack.can_undo());

        // A gesture that ends where it started records nothing.
        st.annotation_resize = Some(AnnotationResize {
            target: Annotation::Comment(0),
            handle: ResizeHandle::Left,
            origin_world: [0.0, 0.0],
            rect0,
            min_h: 0.0,
        });
        st.finish_annotation_resize(&reg);
        assert!(!st.stack.can_undo());
    }

    /// `font_scale` is clamped on read, so a hand-edited asset cannot make a
    /// note 40x — and a missing field still reads as 1.0.
    #[test]
    fn comment_font_scale_is_clamped() {
        use crate::engine::node_graph::{COMMENT_FONT_SCALE_MAX, COMMENT_FONT_SCALE_MIN};
        let mut c = comment(0.0);
        assert_eq!(c.clamped_font_scale(), 1.0);
        c.font_scale = 40.0;
        assert_eq!(c.clamped_font_scale(), COMMENT_FONT_SCALE_MAX);
        c.font_scale = 0.01;
        assert_eq!(c.clamped_font_scale(), COMMENT_FONT_SCALE_MIN);
        c.font_scale = f32::NAN;
        assert_eq!(c.clamped_font_scale(), 1.0);

        // A pre-Phase-5 comment (no tint/font_scale/anchor/collapsed) parses
        // and renders pixel-identically to before.
        let old: crate::engine::node_graph::CommentBox =
            ron::from_str(r#"(rect: (1.0, 2.0, 3.0, 4.0), text: "hi")"#).expect("legacy comment");
        assert_eq!(old.font_scale, 1.0);
        assert_eq!(old.tint, None);
        assert_eq!(old.anchor, None);
        assert!(!old.collapsed);
        let old_g: crate::engine::node_graph::GroupBox =
            ron::from_str(r#"(rect: (1.0, 2.0, 3.0, 4.0), title: "g")"#).expect("legacy group");
        assert_eq!(old_g.tint, None);
        assert!(!old_g.collapsed);
    }

    /// Inserting a reroute is one transaction: the edge goes, a node and two
    /// edges arrive, and undo puts the graph back byte-identically.
    #[test]
    fn reroute_insert_and_delete_round_trip() {
        let reg = NodeRegistry::new();
        let mut st = bare_state();
        st.doc.nodes = vec![node(0, [0.0, 0.0]), node(1, [400.0, 0.0])];
        st.doc.edges = vec![edge(0, 1)];
        let before = st.doc.clone();

        st.insert_reroute(&edge(0, 1), [200.0, 10.0], &reg);
        assert_eq!(st.doc.nodes.len(), 3, "a reroute node arrived");
        assert_eq!(st.doc.edges.len(), 2, "the edge became two");
        let rr = st.doc.nodes[2].id;
        assert_eq!(st.doc.nodes[2].type_id, REROUTE_TYPE_ID);
        assert!(st
            .doc
            .edges
            .iter()
            .any(|e| e.to_node == rr && e.to_pin == REROUTE_IN));
        assert!(st
            .doc
            .edges
            .iter()
            .any(|e| e.from_node == rr && e.from_pin == REROUTE_OUT));
        assert_eq!(st.stack.undo_description().as_deref(), Some("Add Reroute"));

        st.undo(&reg);
        assert_eq!(st.doc, before, "one gesture, one undo");
        let a = crate::engine::node_graph::serialize_graph(&before).unwrap();
        let b = crate::engine::node_graph::serialize_graph(&st.doc).unwrap();
        assert_eq!(a, b, "restored doc must serialize byte-identically");

        // Redo, then delete the reroute: the wire heals end to end.
        st.redo(&reg);
        let rr = st.doc.nodes[2].id;
        assert!(st.delete_reroute(rr, &reg));
        assert_eq!(st.doc.nodes.len(), 2);
        assert_eq!(st.doc.edges.len(), 1, "the through-edge was restored");
        assert_eq!(st.doc.edges[0].from_node, 0);
        assert_eq!(st.doc.edges[0].to_node, 1);
        st.undo(&reg);
        assert_eq!(st.doc.nodes.len(), 3, "undo brings the reroute back");
        assert_eq!(st.doc.edges.len(), 2);
    }

    /// A reroute fans out: deleting it reconnects every branch to the source.
    #[test]
    fn deleting_a_reroute_heals_every_branch() {
        let reg = NodeRegistry::new();
        let mut st = bare_state();
        st.doc.nodes = vec![
            node(0, [0.0, 0.0]),
            node(1, [400.0, 0.0]),
            node(2, [400.0, 100.0]),
        ];
        st.doc.nodes.push(NodeInst {
            id: 9,
            type_id: REROUTE_TYPE_ID.to_string(),
            type_version: 1,
            position: [200.0, 0.0],
            properties: Default::default(),
            subgraph: None,
            tint: None, title: None,
        });
        st.doc.edges = vec![
            Edge { from_node: 0, from_pin: "sum".into(), to_node: 9, to_pin: REROUTE_IN.into() },
            Edge { from_node: 9, from_pin: REROUTE_OUT.into(), to_node: 1, to_pin: "a".into() },
            Edge { from_node: 9, from_pin: REROUTE_OUT.into(), to_node: 2, to_pin: "a".into() },
        ];
        st.delete_reroute(9, &reg);
        assert_eq!(st.doc.edges.len(), 2, "both branches survived");
        assert!(st.doc.edges.iter().all(|e| e.from_node == 0 && e.from_pin == "sum"));
        assert!(st.doc.nodes.iter().all(|n| n.type_id != REROUTE_TYPE_ID));

        // Deleting a non-reroute through this path is refused, so the caller
        // falls back to the ordinary node delete.
        assert!(!st.delete_reroute(0, &reg));
    }

    /// A `Composite` reverts its parts in reverse order — otherwise an edit
    /// that depends on an earlier one in the same gesture corrupts the doc.
    #[test]
    fn composite_reverts_in_reverse_order() {
        let base = {
            let mut d = GraphDoc::default();
            d.nodes = vec![node(0, [0.0, 0.0])];
            d
        };
        let mut doc = base.clone();
        let e = GraphEdit::Composite {
            label: "Two Steps".into(),
            edits: vec![
                GraphEdit::AddNode(node(1, [10.0, 0.0])),
                GraphEdit::Connect(edge(0, 1)),
            ],
        };
        e.apply(&mut doc);
        assert_eq!(doc.nodes.len(), 2);
        assert_eq!(doc.edges.len(), 1);
        assert_eq!(e.description(), "Two Steps");
        e.revert(&mut doc);
        assert_eq!(doc, base, "reverse order restores exactly");
    }

    /// Align snaps every node to the extreme edge; distribute leaves equal
    /// *gaps* (not equal centers) and never moves the two outer nodes.
    #[test]
    fn align_and_distribute_are_one_undoable_gesture() {
        let reg = NodeRegistry::new();
        let mut st = bare_state();
        st.doc.nodes = vec![
            node(0, [0.0, 0.0]),
            node(1, [50.0, 30.0]),
            node(2, [200.0, 90.0]),
        ];
        st.selection = [0u64, 1, 2].into_iter().collect();
        let before = st.doc.clone();
        // Deliberately different widths, so "even gaps" and "even centers"
        // would disagree.
        let rects = vec![
            (0u64, [0.0f32, 0.0, 100.0, 40.0]),
            (1, [50.0, 30.0, 60.0, 40.0]),
            (2, [200.0, 90.0, 140.0, 40.0]),
        ];

        st.align_nodes(&rects, AlignMode::Left, &reg);
        assert!(st.doc.nodes.iter().all(|n| n.position[0] == 0.0));
        assert_eq!(st.stack.undo_description().as_deref(), Some("Align Left"));
        st.undo(&reg);
        assert_eq!(st.doc, before, "one gesture, one undo entry");

        st.align_nodes(&rects, AlignMode::Bottom, &reg);
        // Bottom edge = max(y + h) = 130; each node's y becomes 130 - h.
        assert!(st.doc.nodes.iter().all(|n| (n.position[1] - 90.0).abs() < 1e-3));
        st.undo(&reg);

        st.align_nodes(&rects, AlignMode::DistributeHorizontally, &reg);
        let xs: Vec<f32> = st.doc.nodes.iter().map(|n| n.position[0]).collect();
        // Outer nodes are the span and must not move.
        assert!((xs[0] - 0.0).abs() < 1e-3);
        assert!((xs[2] - 200.0).abs() < 1e-3);
        // Gaps: 0..100, then g, then 60 wide, then g, then 200..340.
        let g1 = xs[1] - 100.0;
        let g2 = 200.0 - (xs[1] + 60.0);
        assert!((g1 - g2).abs() < 1e-3, "gaps uneven: {g1} vs {g2}");
        st.undo(&reg);
        assert_eq!(st.doc, before);

        // Fewer than three is not a distribution...
        st.align_nodes(&rects[..2], AlignMode::DistributeHorizontally, &reg);
        assert!(!st.stack.can_undo());
        // ...but two nodes align perfectly well. (This line used to assert the
        // opposite, under a comment about distribution: one shared `< 3` guard
        // covered both modes, so a two-node align was a silent no-op.)
        st.align_nodes(&rects[..2], AlignMode::Left, &reg);
        assert!(st.stack.can_undo());
    }

    /// Bookmarks cycle 1..=5 and recall only what was stored.
    #[test]
    fn bookmarks_cycle_and_recall() {
        let mut st = bare_state();
        assert!(!st.recall_bookmark(1), "an empty slot recalls nothing");
        assert!(!st.recall_bookmark(99), "an out-of-range slot is not a panic");

        for expect in 1..=BOOKMARK_SLOTS {
            st.view = CanvasView { pan: Vec2::new(expect as f32 * 10.0, 0.0), zoom: 1.0 };
            assert_eq!(st.store_bookmark(), expect);
        }
        // The sixth store wraps back onto slot 1.
        st.view = CanvasView { pan: Vec2::new(999.0, 0.0), zoom: 1.0 };
        assert_eq!(st.store_bookmark(), 1);

        st.view = CanvasView::default();
        assert!(st.recall_bookmark(3));
        assert_eq!(st.view.pan.x, 30.0);
        assert!(st.recall_bookmark(1));
        assert_eq!(st.view.pan.x, 999.0, "slot 1 was overwritten by the wrap");
    }

    /// Purge keeps anything feeding a side-effecting node and drops the rest;
    /// a graph with no impure nodes is left alone rather than wiped.
    #[test]
    fn purge_keeps_what_feeds_an_impure_node() {
        let mut reg = NodeRegistry::new();
        reg.register(NodeDescriptor {
            id: "pure_add".into(),
            name: "Add".into(),
            category: "Math".into(),
            version: 1,
            inputs: vec![crate::engine::node_graph::PinDescriptor::new(
                "a",
                "A",
                crate::engine::node_graph::PinType::Float,
            )],
            outputs: vec![crate::engine::node_graph::PinDescriptor::new(
                "sum",
                "Sum",
                crate::engine::node_graph::PinType::Float,
            )],
            pure: true,
            realm: crate::engine::node_graph::NodeRealm::Shared,
            deterministic: true,
            doc: None,
            preview: None,
        })
        .unwrap();
        reg.register(NodeDescriptor {
            id: "sink".into(),
            name: "Sink".into(),
            category: "Gameplay".into(),
            version: 1,
            inputs: vec![
                crate::engine::node_graph::PinDescriptor::new(
                    "exec_in",
                    "",
                    crate::engine::node_graph::PinType::Exec,
                ),
                crate::engine::node_graph::PinDescriptor::new(
                    "a",
                    "A",
                    crate::engine::node_graph::PinType::Float,
                ),
            ],
            outputs: vec![],
            pure: false,
            realm: crate::engine::node_graph::NodeRealm::Shared,
            deterministic: true,
            doc: None,
            preview: None,
        })
        .unwrap();

        let mut st = bare_state();
        let mk = |id: u64, ty: &str| NodeInst {
            id,
            type_id: ty.to_string(),
            type_version: 1,
            position: [0.0, 0.0],
            properties: Default::default(),
            subgraph: None,
            tint: None, title: None,
        };
        // 0 -> 1 -> 2(sink); 3 is an orphan; 4 feeds only the orphan.
        st.doc.nodes = vec![
            mk(0, "pure_add"),
            mk(1, "pure_add"),
            mk(2, "sink"),
            mk(3, "pure_add"),
            mk(4, "pure_add"),
        ];
        st.doc.edges = vec![
            Edge { from_node: 0, from_pin: "sum".into(), to_node: 1, to_pin: "a".into() },
            Edge { from_node: 1, from_pin: "sum".into(), to_node: 2, to_pin: "a".into() },
            Edge { from_node: 4, from_pin: "sum".into(), to_node: 3, to_pin: "a".into() },
        ];
        let unused = st.unused_nodes(&reg);
        assert_eq!(unused, vec![3, 4], "only the orphan branch is unused");

        let before = st.doc.clone();
        st.purge_nodes(&unused, &reg);
        assert_eq!(st.doc.nodes.len(), 3);
        assert_eq!(st.doc.edges.len(), 2);
        assert_eq!(st.stack.undo_description().as_deref(), Some("Purge 2 Nodes"));
        st.undo(&reg);
        assert_eq!(st.doc, before, "purge undoes as one transaction");

        // A graph of pure computation has nothing to reach: purging is a
        // no-op, not a wipe.
        let mut pure_only = bare_state();
        pure_only.doc.nodes = vec![mk(0, "pure_add"), mk(1, "pure_add")];
        assert!(pure_only.unused_nodes(&reg).is_empty());

        // A subgraph's `graph_output` is a seed even though a pure data
        // interface makes it read as pure: it *is* the document's result, so
        // whatever feeds it is reached. Without this, purging a subgraph of
        // pure math would delete the whole body.
        let mut sub = bare_state();
        sub.doc.outputs = vec![IfacePin {
            slug: "result".into(),
            label: "Result".into(),
            ty: PinType::Float,
        }];
        sub.doc.nodes = vec![mk(0, "pure_add"), mk(1, GRAPH_OUTPUT_TYPE_ID), mk(2, "pure_add")];
        sub.doc.edges = vec![Edge {
            from_node: 0,
            from_pin: "sum".into(),
            to_node: 1,
            to_pin: "result".into(),
        }];
        assert_eq!(
            sub.unused_nodes(&reg),
            vec![2],
            "only the node feeding nothing is unused"
        );
    }

    /// Collapse writes the asset, derives the interface from boundary edges,
    /// and replaces the selection with one wired subgraph node.
    #[test]
    fn collapse_to_subgraph_derives_its_interface() {
        let reg = NodeRegistry::new();
        let dir = std::env::temp_dir().join("rust_engine_collapse_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("graphs")).unwrap();

        let mut st = bare_state();
        st.path = "graphs/host.graph".into();
        st.doc.nodes = vec![
            node(0, [0.0, 0.0]),   // outside, feeds in
            node(1, [200.0, 0.0]), // inside
            node(2, [400.0, 0.0]), // inside
            node(3, [600.0, 0.0]), // outside, fed from inside
        ];
        st.doc.edges = vec![
            edge(0, 1), // boundary in
            edge(1, 2), // internal
            edge(2, 3), // boundary out
        ];
        st.selection = [1u64, 2].into_iter().collect();
        let before = st.doc.clone();

        let rel = st.collapse_to_subgraph(&dir, &reg).expect("collapse");
        assert_eq!(rel, "graphs/subgraphs/host_1.subgraph");
        assert!(dir.join(&rel).exists(), "the asset was written");

        // Host: the two nodes became one subgraph node, still wired both ways.
        assert_eq!(st.doc.nodes.len(), 3);
        let sub = st.doc.nodes.iter().find(|n| n.subgraph.is_some()).unwrap();
        assert_eq!(sub.subgraph.as_deref(), Some(rel.as_str()));
        assert_eq!(st.doc.edges.len(), 2, "one edge in, one out");
        assert!(st.doc.edges.iter().any(|e| e.from_node == 0 && e.to_node == sub.id));
        assert!(st.doc.edges.iter().any(|e| e.from_node == sub.id && e.to_node == 3));

        // Asset: internal edge kept, interface derived from the cut edges,
        // and the interface *binding* pair auto-inserted and wired (45-A) so
        // the declaration is attached to something.
        let written =
            crate::engine::node_graph::load_graph(&dir.join(&rel)).expect("reload");
        assert_eq!(written.inputs.len(), 1);
        assert_eq!(written.outputs.len(), 1);
        assert_eq!(written.inputs[0].slug, "sum", "input slug comes from its source pin");
        assert_eq!(written.outputs[0].slug, "sum");

        let gi = written
            .nodes
            .iter()
            .find(|n| n.type_id == GRAPH_INPUT_TYPE_ID)
            .expect("graph_input auto-inserted");
        let go = written
            .nodes
            .iter()
            .find(|n| n.type_id == GRAPH_OUTPUT_TYPE_ID)
            .expect("graph_output auto-inserted");
        assert_eq!(written.nodes.len(), 4, "two collapsed nodes + the pair");
        assert_eq!(
            written.edges.len(),
            3,
            "the internal edge plus one binding edge per boundary edge"
        );
        assert!(
            written
                .edges
                .iter()
                .any(|e| e.from_node == gi.id && e.from_pin == "sum" && e.to_node == 1),
            "the inbound boundary edge continues from graph_input: {:?}",
            written.edges
        );
        assert!(
            written
                .edges
                .iter()
                .any(|e| e.from_node == 2 && e.to_node == go.id && e.to_pin == "sum"),
            "the outbound boundary edge ends at graph_output"
        );
        // …and the result validates clean: no unbound interface pins.
        assert_eq!(
            crate::engine::node_graph::validate_doc(&written, &reg)
                .iter()
                .filter(|e| matches!(e, GraphError::InterfacePinUnbound { .. }))
                .count(),
            0
        );

        // Undo restores the host exactly; the asset deliberately stays.
        st.undo(&reg);
        assert_eq!(st.doc, before, "the host document reverts");
        assert!(
            dir.join(&rel).exists(),
            "the created asset survives undo on purpose - deleting a file the \
             user may have opened risks data loss"
        );

        // A second collapse picks the next free name.
        st.selection = [1u64, 2].into_iter().collect();
        let rel2 = st.collapse_to_subgraph(&dir, &reg).expect("second collapse");
        assert_eq!(rel2, "graphs/subgraphs/host_2.subgraph");

        // Fewer than two nodes is not a subgraph.
        st.clear_selection();
        assert!(st.collapse_to_subgraph(&dir, &reg).is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The clipboard is a RON subset, not a pointer set — that is what makes
    /// a paste survive a restart. Annotations and unknown node types both
    /// round-trip verbatim.
    #[test]
    fn fragment_ron_round_trips_including_annotations_and_unknown_types() {
        let mut unknown = node(7, [10.0, 20.0]);
        unknown.type_id = "plugin.not_installed".into();
        unknown
            .properties
            .insert("weird".into(), PropValue::Raw("(a:1,b:[2,3])".into()));
        unknown.tint = Some(9);

        // Task 45-A: the node title and the new value types ride the
        // clipboard like anything else — a copy that silently dropped them
        // would be data loss across a paste.
        let mut titled = node(9, [30.0, 40.0]);
        titled.title = Some("Renamed".into());
        titled.properties.insert("n".into(), PropValue::Int(-4));
        titled
            .properties
            .insert("s".into(), PropValue::Str("text".into()));
        titled.properties.insert(
            "xs".into(),
            PropValue::Array(vec![PropValue::Int(1), PropValue::Int(2)]),
        );

        let mut anchored = comment(50.0);
        anchored.anchor = Some(7);
        anchored.tint = Some(3);
        anchored.font_scale = 1.75;
        anchored.collapsed = true;

        let frag = GraphFragment {
            nodes: vec![node(0, [0.0, 0.0]), unknown, titled],
            edges: vec![edge(0, 7)],
            comments: vec![anchored],
            groups: vec![group(0.0)],
            regions: BTreeMap::new(),
        };

        let text = frag.to_ron().expect("serialize");
        let back = GraphFragment::from_ron(&text).expect("parse");
        assert_eq!(back, frag, "a fragment must survive a full round trip");
        assert_eq!(back.nodes[2].title.as_deref(), Some("Renamed"));
        assert_eq!(
            back.nodes[2].properties.get("xs"),
            Some(&PropValue::Array(vec![PropValue::Int(1), PropValue::Int(2)]))
        );
        // Forward-compat data is preserved untouched (the Raw philosophy).
        assert_eq!(
            back.nodes[1].properties.get("weird"),
            Some(&PropValue::Raw("(a:1,b:[2,3])".into()))
        );

        // Anything that is not one of our fragments is a no-op, never a panic.
        assert!(GraphFragment::from_ron("").is_none());
        assert!(GraphFragment::from_ron("hello, clipboard").is_none());
        assert!(GraphFragment::from_ron("(kind: \"something.else\", version: 1, fragment: ())").is_none());
        let bumped = text.replace("version: 1", "version: 99");
        assert!(
            GraphFragment::from_ron(&bumped).is_none(),
            "a future version is rejected, not half-read"
        );
    }

    /// Copy takes the selected annotation and every note anchored to a copied
    /// node — copying a node without its explanation copies half the thought.
    #[test]
    fn copy_includes_selected_and_anchored_annotations() {
        let mut st = bare_state();
        st.doc.nodes = vec![node(0, [0.0, 0.0]), node(1, [10.0, 0.0])];
        st.doc.comments = vec![comment(0.0), comment(300.0), comment(600.0)];
        st.doc.comments[0].anchor = Some(0); // anchored to a copied node
        st.doc.comments[1].anchor = Some(1); // anchored to a node NOT copied
        st.doc.groups = vec![group(0.0)];
        st.selection = [0u64].into_iter().collect();
        st.sel_comment = Some(2); // explicitly selected

        let frag = st.selection_fragment().expect("fragment");
        assert_eq!(frag.nodes.len(), 1);
        assert_eq!(frag.comments.len(), 2, "the anchored one and the selected one");
        assert!(frag.groups.is_empty(), "no group was selected");

        // Nothing selected at all is nothing to copy.
        let empty = bare_state();
        assert!(empty.selection_fragment().is_none());
    }

    /// Paste anchors the fragment's top-left at the cursor, preserves the
    /// internal layout, and nudges clear of what is already there.
    #[test]
    fn paste_lands_at_the_cursor_and_nudges_clear() {
        let reg = NodeRegistry::new();
        let mut st = bare_state();
        let frag = GraphFragment {
            nodes: vec![node(0, [100.0, 100.0]), node(1, [140.0, 180.0])],
            edges: vec![edge(0, 1)],
            ..Default::default()
        };

        st.paste_fragment(&frag, Some([500.0, 300.0]), &reg);
        assert_eq!(st.doc.nodes.len(), 2);
        // Top-left lands on the cursor; the relative offset is preserved.
        assert_eq!(st.doc.nodes[0].position, [500.0, 300.0]);
        assert_eq!(st.doc.nodes[1].position, [540.0, 380.0]);
        assert_eq!(st.doc.edges.len(), 1, "the internal edge came along");

        // Pasting again at the same spot walks clear in 8px steps.
        st.paste_fragment(&frag, Some([500.0, 300.0]), &reg);
        assert_eq!(st.doc.nodes[2].position, [508.0, 308.0], "nudged clear");

        // No target = the duplicate offset, so a menu paste still lands.
        let mut st2 = bare_state();
        st2.paste_fragment(&frag, None, &reg);
        assert_eq!(st2.doc.nodes[0].position, [116.0, 116.0]);
    }

    /// Boundary edges drop, and the drop is reported rather than silent.
    #[test]
    fn paste_reports_dropped_boundary_links() {
        let reg = NodeRegistry::new();
        let mut st = bare_state();
        // The fragment claims two edges; only one is internal to it.
        let frag = GraphFragment {
            nodes: vec![node(0, [0.0, 0.0]), node(1, [50.0, 0.0])],
            edges: vec![edge(0, 1), edge(0, 99), edge(98, 1)],
            ..Default::default()
        };
        st.paste_fragment(&frag, Some([0.0, 0.0]), &reg);
        assert_eq!(st.doc.edges.len(), 1, "only the internal edge survives");
        let last = st.toasts.last().expect("a toast reports the drop");
        assert_eq!(last.text, "Pasted 2 nodes, 2 links dropped");
    }

    /// Duplicate offsets by 16 and leaves the clipboard alone.
    #[test]
    fn duplicate_offsets_by_16_and_spares_the_clipboard() {
        let reg = NodeRegistry::new();
        let mut st = bare_state();
        st.doc.nodes = vec![node(0, [200.0, 100.0])];
        st.selection = [0u64].into_iter().collect();
        let clipboard: Option<GraphFragment> = None;

        st.duplicate_selection(&reg);
        assert_eq!(st.doc.nodes.len(), 2);
        assert_eq!(st.doc.nodes[1].position, [216.0, 116.0]);
        assert!(clipboard.is_none(), "duplicate must not touch the clipboard");
    }

    /// An unregistered type pastes and stays: it renders missing-red and its
    /// properties survive, rather than vanishing on the way in.
    #[test]
    fn unknown_types_paste_as_preserved_placeholders() {
        let reg = NodeRegistry::new();
        let mut st = bare_state();
        let mut unknown = node(3, [0.0, 0.0]);
        unknown.type_id = "plugin.gone".into();
        unknown
            .properties
            .insert("k".into(), PropValue::Raw("(x:1)".into()));
        let frag = GraphFragment { nodes: vec![unknown], ..Default::default() };

        st.paste_fragment(&frag, Some([0.0, 0.0]), &reg);
        assert_eq!(st.doc.nodes.len(), 1, "the node was kept, not dropped");
        assert_eq!(st.doc.nodes[0].type_id, "plugin.gone");
        assert_eq!(
            st.doc.nodes[0].properties.get("k"),
            Some(&PropValue::Raw("(x:1)".into())),
            "properties survive verbatim"
        );
        // …and validation anchors the error to it, with no special paste code.
        let errs = crate::engine::node_graph::validate_doc(&st.doc, &reg);
        assert!(errs
            .iter()
            .any(|e| matches!(e, GraphError::UnknownNodeType { .. })));
    }

    /// All three break paths funnel through one transaction + one report.
    #[test]
    fn break_paths_share_one_transaction_and_report() {
        let reg = NodeRegistry::new();
        let mut st = bare_state();
        st.doc.nodes = vec![node(0, [0.0, 0.0]), node(1, [10.0, 0.0]), node(2, [20.0, 0.0])];
        st.doc.edges = vec![edge(0, 1), edge(0, 2), edge(1, 2)];
        let before = st.doc.clone();

        // Pin: only that pin's edges.
        st.break_pin_links(0, "sum", true, &reg);
        assert_eq!(st.doc.edges.len(), 1);
        assert_eq!(st.toasts.last().unwrap().text, "Broke 2 links");
        st.undo(&reg);
        assert_eq!(st.doc, before, "one gesture, one undo");

        // Node: everything touching it.
        st.break_node_links(1, &reg);
        assert_eq!(st.doc.edges.len(), 1);
        assert_eq!(st.toasts.last().unwrap().text, "Broke 2 links");
        st.undo(&reg);
        assert_eq!(st.doc, before);

        // Empty cases say so and record nothing.
        let undo_depth = st.stack.can_undo();
        st.break_pin_links(2, "sum", true, &reg);
        assert_eq!(st.toasts.last().unwrap().text, "Pin has no links");
        assert_eq!(st.stack.can_undo(), undo_depth);
        let mut lonely = bare_state();
        lonely.doc.nodes = vec![node(0, [0.0, 0.0])];
        lonely.break_node_links(0, &reg);
        assert_eq!(lonely.toasts.last().unwrap().text, "Node has no links");
        assert!(!lonely.stack.can_undo());
    }

    /// The cut path is capped and never records a still pointer.
    #[test]
    fn cut_path_is_capped_and_deduped() {
        let mut st = bare_state();
        for i in 0..(CUT_PATH_MAX * 2) {
            st.push_cut_point([i as f32 * 4.0, 0.0]);
        }
        let path = st.cut_path.as_ref().unwrap();
        assert_eq!(path.len(), CUT_PATH_MAX, "the buffer is capped");
        // The cap drops the oldest, so the newest sample is always present.
        assert_eq!(path.last().unwrap()[0], (CUT_PATH_MAX * 2 - 1) as f32 * 4.0);

        // A pointer that has not moved does not consume the cap.
        let before = st.cut_path.as_ref().unwrap().len();
        st.push_cut_point([(CUT_PATH_MAX * 2 - 1) as f32 * 4.0, 0.0]);
        assert_eq!(st.cut_path.as_ref().unwrap().len(), before);
    }

    /// Splicing a node into a wire is one transaction and reverts exactly.
    #[test]
    fn splice_into_wire_round_trips() {
        let mut reg = NodeRegistry::new();
        reg.register(NodeDescriptor {
            id: "test_add".into(),
            name: "Add".into(),
            category: "Math".into(),
            version: 1,
            inputs: vec![crate::engine::node_graph::PinDescriptor::new(
                "a",
                "A",
                crate::engine::node_graph::PinType::Float,
            )],
            outputs: vec![crate::engine::node_graph::PinDescriptor::new(
                "sum",
                "Sum",
                crate::engine::node_graph::PinType::Float,
            )],
            pure: true,
            realm: crate::engine::node_graph::NodeRealm::Shared,
            deterministic: true,
            doc: None,
            preview: None,
        })
        .unwrap();

        let mut st = bare_state();
        st.doc.nodes = vec![node(0, [0.0, 0.0]), node(1, [400.0, 0.0]), node(2, [200.0, 80.0])];
        st.doc.edges = vec![edge(0, 1)];
        let before = st.doc.clone();

        // Node 2 has a Float in and a Float out, so it can sit on the wire.
        let (in_pin, out_pin) = st.splice_pins(&edge(0, 1), 2, &reg).expect("splice pins");
        assert_eq!((in_pin.as_str(), out_pin.as_str()), ("a", "sum"));

        assert!(st.splice_node_into(&edge(0, 1), 2, &in_pin, &out_pin, &reg));
        assert_eq!(st.doc.edges.len(), 2, "the wire became two");
        assert!(st.doc.edges.iter().any(|e| e.from_node == 0 && e.to_node == 2));
        assert!(st.doc.edges.iter().any(|e| e.from_node == 2 && e.to_node == 1));
        assert_eq!(st.stack.undo_description().as_deref(), Some("Splice Node"));
        assert_eq!(st.toasts.last().unwrap().text, "Spliced into wire");

        st.undo(&reg);
        assert_eq!(st.doc, before, "one gesture, one undo");
        let a = crate::engine::node_graph::serialize_graph(&before).unwrap();
        let b = crate::engine::node_graph::serialize_graph(&st.doc).unwrap();
        assert_eq!(a, b, "restored doc must serialize byte-identically");

        // A wire's own endpoints cannot splice into it.
        assert!(st.splice_pins(&edge(0, 1), 0, &reg).is_none());
        assert!(st.splice_pins(&edge(0, 1), 1, &reg).is_none());
        assert!(!st.splice_node_into(&edge(0, 1), 0, "a", "sum", &reg));
        // Neither can a node the registry does not know.
        assert!(st.splice_pins(&edge(0, 1), 99, &reg).is_none());
    }

    /// Grabbing a wire's midpoint inserts a reroute and hands it to a drag,
    /// so the handle is a grab rather than a two-step.
    #[test]
    fn wire_midpoint_grab_inserts_and_drags() {
        let reg = NodeRegistry::new();
        let mut st = bare_state();
        st.doc.nodes = vec![node(0, [0.0, 0.0]), node(1, [400.0, 0.0])];
        st.doc.edges = vec![edge(0, 1)];

        let id = st.grab_wire_midpoint(&edge(0, 1), [200.0, 0.0], &reg).expect("grab");
        assert_eq!(st.doc.nodes.len(), 3);
        assert_eq!(st.doc.edges.len(), 2);
        let drag = st.node_drag.as_ref().expect("a drag is live");
        assert_eq!(drag.originals.len(), 1);
        assert_eq!(drag.originals[0].0, id, "the new reroute is what is dragging");
        assert!(
            drag.pending.is_some(),
            "the insert is applied but held back until the gesture ends"
        );

        // Undoing mid-gesture cancels the drag and takes the un-recorded
        // insert with it — grab-and-move is one gesture, so a half-finished
        // one leaves nothing behind. (The completed-gesture case is
        // `midpoint_grab_and_drag_is_one_undo_entry`.)
        st.undo(&reg);
        assert_eq!(st.doc.edges.len(), 1);
        assert!(st.node_drag.is_none(), "undo cancels the live drag");
    }

    /// Auto-layout is one undo, respects a selection, and leaves a lone node
    /// alone.
    #[test]
    fn auto_layout_is_one_transaction_and_scoped() {
        use super::super::graph_layout::LayoutSpacing;
        let reg = NodeRegistry::new();
        let mut st = bare_state();
        st.doc.nodes = vec![
            node(0, [500.0, 500.0]),
            node(1, [10.0, 10.0]),
            node(2, [900.0, 30.0]),
        ];
        st.doc.edges = vec![edge(0, 1), edge(1, 2)];
        let rects: Vec<(u64, [f32; 4])> = st
            .doc
            .nodes
            .iter()
            .map(|n| (n.id, [n.position[0], n.position[1], 100.0, 40.0]))
            .collect();
        let before = st.doc.clone();

        st.auto_layout(&rects, LayoutSpacing::default(), &reg);
        // Ranked left to right by the edges, not by where they started.
        let x = |id: u64| st.doc.node(id).unwrap().position[0];
        assert!(x(0) < x(1) && x(1) < x(2), "layout follows the edges");
        assert_eq!(st.stack.undo_description().as_deref(), Some("Auto Layout"));
        st.undo(&reg);
        assert_eq!(st.doc, before, "one gesture, one undo");

        // A 2+ selection scopes it: node 2 must not move.
        st.selection = [0u64, 1].into_iter().collect();
        let pinned = st.doc.node(2).unwrap().position;
        st.auto_layout(&rects, LayoutSpacing::default(), &reg);
        assert_eq!(st.doc.node(2).unwrap().position, pinned, "outside the selection");

        // Fewer than two nodes is not a layout.
        let mut lone = bare_state();
        lone.doc.nodes = vec![node(0, [0.0, 0.0])];
        lone.auto_layout(&[(0, [0.0, 0.0, 100.0, 40.0])], LayoutSpacing::default(), &reg);
        assert!(!lone.stack.can_undo());
    }

    /// Review finding 5 (HIGH). `Disconnect` must remove the *recorded
    /// indices*, not everything that compares equal: two edges can carry the
    /// same from/to tuple, and removing by value took both while undo
    /// restored one.
    #[test]
    fn disconnect_removes_by_index_not_by_value() {
        let reg = NodeRegistry::new();
        let mut st = bare_state();
        st.doc.nodes = vec![node(0, [0.0, 0.0]), node(1, [10.0, 0.0])];
        // Two identical tuples — representable, if degenerate.
        st.doc.edges = vec![edge(0, 1), edge(0, 1), edge(1, 0)];
        let before = st.doc.clone();

        let edit = GraphEdit::Disconnect { edges: vec![(0, edge(0, 1))] };
        edit.apply(&mut st.doc);
        assert_eq!(st.doc.edges.len(), 2, "exactly one edge was removed");
        assert_eq!(st.doc.edges[0], edge(0, 1), "the duplicate survived");
        edit.revert(&mut st.doc);
        assert_eq!(st.doc, before, "and undo restores exactly one");

        // A stale index whose tuple no longer matches is skipped rather than
        // removing whatever now sits there.
        let stale = GraphEdit::Disconnect { edges: vec![(2, edge(0, 1))] };
        stale.apply(&mut st.doc);
        assert_eq!(st.doc.edges.len(), 3, "a mismatched index removes nothing");
        let _ = reg;
    }

    /// Review finding 2 (HIGH). Saving writes rounded positions; leaving
    /// sub-pixel values in memory made "clean" describe content the file does
    /// not contain.
    #[test]
    fn save_snaps_positions_so_clean_means_clean() {
        let dir = std::env::temp_dir().join("rust_engine_save_snap_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("g.graph");

        let mut st = bare_state();
        st.doc.nodes = vec![node(0, [10.4, -3.6]), node(1, [0.5, 0.5])];
        st.doc.comments = vec![comment(2.7)];
        st.save(&path).unwrap();

        // In memory now matches what went to disk, exactly.
        assert_eq!(st.doc.nodes[0].position, [10.0, -4.0]);
        assert_eq!(st.doc.nodes[1].position, [1.0, 1.0]);
        assert_eq!(st.doc.comments[0].rect[0], 3.0);
        let on_disk = crate::engine::node_graph::load_graph(&path).unwrap();
        assert_eq!(
            crate::engine::node_graph::serialize_graph(&on_disk).unwrap(),
            crate::engine::node_graph::serialize_graph(&st.doc).unwrap(),
            "a clean document must serialize to what the file holds"
        );
        assert!(!st.dirty);
        // Order is untouched — the undo stack indexes into these vecs.
        assert_eq!(st.doc.nodes.iter().map(|n| n.id).collect::<Vec<_>>(), vec![0, 1]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Review finding 3 (HIGH). Dropping a node on a wire must never produce
    /// input fan-in: a free pin is preferred, and an occupied one has its
    /// existing edge replaced inside the same transaction.
    #[test]
    fn splice_never_creates_input_fan_in() {
        use crate::engine::node_graph::{PinDescriptor, PinType};
        let mut reg = NodeRegistry::new();
        // The wire's source needs a known type too, or the splice cannot
        // resolve what flows through it.
        reg.register(NodeDescriptor {
            id: "test_add".into(),
            name: "Add".into(),
            category: "Math".into(),
            version: 1,
            inputs: vec![PinDescriptor::new("a", "A", PinType::Float)],
            outputs: vec![PinDescriptor::new("sum", "Sum", PinType::Float)],
            pure: true,
            realm: crate::engine::node_graph::NodeRealm::Shared,
            deterministic: true,
            doc: None,
            preview: None,
        })
        .unwrap();
        reg.register(NodeDescriptor {
            id: "two_in".into(),
            name: "Two In".into(),
            category: "Math".into(),
            version: 1,
            inputs: vec![
                PinDescriptor::new("a", "A", PinType::Float),
                PinDescriptor::new("b", "B", PinType::Float),
            ],
            outputs: vec![PinDescriptor::new("sum", "Sum", PinType::Float)],
            pure: true,
            realm: crate::engine::node_graph::NodeRealm::Shared,
            deterministic: true,
            doc: None,
            preview: None,
        })
        .unwrap();

        let mut st = bare_state();
        st.doc.nodes = vec![
            node(0, [0.0, 0.0]),
            node(1, [400.0, 0.0]),
            node(2, [200.0, 80.0]),
            node(3, [0.0, 200.0]),
        ];
        st.doc.nodes[2].type_id = "two_in".into();
        // Pin "a" of the splice target is already fed by node 3.
        st.doc.edges = vec![
            edge(0, 1),
            Edge { from_node: 3, from_pin: "sum".into(), to_node: 2, to_pin: "a".into() },
        ];

        // The free pin is chosen, so nothing has to be replaced.
        let (i, o) = st.splice_pins(&edge(0, 1), 2, &reg).unwrap();
        assert_eq!(i, "b", "an unconnected input is preferred");
        assert!(st.splice_node_into(&edge(0, 1), 2, &i, &o, &reg));
        assert!(
            validate_doc(&st.doc, &reg)
                .iter()
                .all(|e| !matches!(e, GraphError::InputMultiplyConnected { .. })),
            "{:?}",
            validate_doc(&st.doc, &reg)
        );
        st.undo(&reg);

        // With every matching input taken, the splice replaces rather than
        // fanning in — still one transaction.
        st.doc.edges.push(Edge {
            from_node: 3,
            from_pin: "sum".into(),
            to_node: 2,
            to_pin: "b".into(),
        });
        let before = st.doc.clone();
        let (i, o) = st.splice_pins(&edge(0, 1), 2, &reg).unwrap();
        assert!(st.splice_node_into(&edge(0, 1), 2, &i, &o, &reg));
        let errs = validate_doc(&st.doc, &reg);
        assert!(
            errs.iter().all(|e| !matches!(e, GraphError::InputMultiplyConnected { .. })),
            "splice fanned in: {errs:?}"
        );
        st.undo(&reg);
        assert_eq!(st.doc, before, "still one undo entry");

        let _ = std::fs::remove_dir_all(std::env::temp_dir().join("nothing"));
    }

    /// Review finding 6 (MED). Grabbing a wire's midpoint and dragging is one
    /// gesture, so it is one undo entry — and abandoning it leaves nothing
    /// behind.
    #[test]
    fn midpoint_grab_and_drag_is_one_undo_entry() {
        let reg = NodeRegistry::new();
        let mut st = bare_state();
        st.doc.nodes = vec![node(0, [0.0, 0.0]), node(1, [400.0, 0.0])];
        st.doc.edges = vec![edge(0, 1)];
        let before = st.doc.clone();

        let id = st.grab_wire_midpoint(&edge(0, 1), [200.0, 0.0], &reg).unwrap();
        // The insert is applied but not yet recorded.
        assert!(!st.stack.can_undo(), "nothing is recorded mid-gesture");
        assert_eq!(st.doc.nodes.len(), 3);

        // Move it, then release.
        st.doc.node_mut(id).unwrap().position = [220.0, 40.0];
        crate::engine::editor::graph_editor::tests_support::finish_drag(&mut st, &reg);
        assert!(st.stack.can_undo());
        st.undo(&reg);
        assert_eq!(st.doc, before, "insert + move undo together");
        assert!(!st.stack.can_undo(), "and there is no second entry");

        // Abandoning the gesture reverts the applied-but-unrecorded insert.
        st.grab_wire_midpoint(&edge(0, 1), [200.0, 0.0], &reg).unwrap();
        assert_eq!(st.doc.nodes.len(), 3);
        st.cancel_interactions();
        assert_eq!(st.doc, before, "a cancelled grab leaves nothing behind");
        assert!(!st.stack.can_undo());
    }


    /// The ruled split: aligning two nodes is an ordinary gesture, while
    /// distributing needs a middle node to move. One shared `< 3` guard made
    /// two-node align a silent no-op.
    #[test]
    fn align_needs_two_nodes_and_distribute_needs_three() {
        for mode in AlignMode::ALL {
            let expected = if mode.is_distribute() { 3 } else { 2 };
            assert_eq!(mode.min_nodes(), expected, "{:?}", mode);
        }
    }

    #[test]
    fn two_selected_nodes_align_but_do_not_distribute() {
        let reg = NodeRegistry::new();
        let mut st = bare_state();
        st.doc.nodes = vec![node(0, [0.0, 0.0]), node(1, [200.0, 80.0])];
        let rects = vec![(0u64, [0.0, 0.0, 160.0, 60.0]), (1u64, [200.0, 80.0, 160.0, 60.0])];

        st.align_nodes(&rects, AlignMode::Top, &reg);
        assert_eq!(
            st.doc.node(1).unwrap().position[1],
            0.0,
            "two-node align lines them up instead of doing nothing"
        );
        assert!(st.stack.can_undo(), "and it is one undoable edit");

        let before = st.doc.clone();
        st.align_nodes(&rects, AlignMode::DistributeHorizontally, &reg);
        assert_eq!(st.doc, before, "distribute still needs three");
    }

    #[test]
    fn three_selected_nodes_distribute() {
        let reg = NodeRegistry::new();
        let mut st = bare_state();
        st.doc.nodes = vec![
            node(0, [0.0, 0.0]),
            node(1, [10.0, 0.0]),
            node(2, [400.0, 0.0]),
        ];
        let rects = vec![
            (0u64, [0.0, 0.0, 100.0, 60.0]),
            (1u64, [10.0, 0.0, 100.0, 60.0]),
            (2u64, [400.0, 0.0, 100.0, 60.0]),
        ];
        st.align_nodes(&rects, AlignMode::DistributeHorizontally, &reg);
        assert!(st.stack.can_undo(), "three nodes distribute");
        // The middle one moves; the outer two anchor the span.
        assert!(st.doc.node(1).unwrap().position[0] > 10.0);
    }

    #[test]
    fn a_single_node_is_never_enough_for_either() {
        let reg = NodeRegistry::new();
        let mut st = bare_state();
        st.doc.nodes = vec![node(0, [0.0, 0.0])];
        let rects = vec![(0u64, [0.0, 0.0, 100.0, 60.0])];
        let before = st.doc.clone();
        for mode in AlignMode::ALL {
            st.align_nodes(&rects, mode, &reg);
        }
        assert_eq!(st.doc, before);
        assert!(!st.stack.can_undo(), "and nothing reaches the undo stack");
    }


    /// Acceptance 7 — Alt+click a wire removes exactly that wire, reports it,
    /// and costs one undo step. (Bug 2: this gesture did nothing at all.)
    #[test]
    fn acceptance_7_alt_click_a_wire_deletes_one_wire_in_one_undo_step() {
        let reg = NodeRegistry::new();
        let mut st = bare_state();
        st.doc.nodes = vec![node(0, [0.0, 0.0]), node(1, [400.0, 0.0]), node(2, [800.0, 0.0])];
        st.doc.edges = vec![edge(0, 1), edge(1, 2)];
        let before = st.doc.clone();

        let doomed = st.doc.edges[0].clone();
        let n = st.break_links(&[doomed.clone()], "Delete", &reg);

        assert_eq!(n, 1, "it reports the one wire it took");
        assert_eq!(st.doc.edges.len(), 1, "and takes only that one");
        assert!(!st.doc.edges.contains(&doomed));
        assert!(st.stack.can_undo());
        st.undo(&reg);
        assert_eq!(st.doc, before, "one undo puts it back");
        assert!(!st.stack.can_undo(), "and there was only ever one entry");
    }

    /// Acceptance 8 — the same, for a pin carrying several links.
    #[test]
    fn acceptance_8_alt_click_a_pin_with_four_links_is_one_undo_step() {
        let mut reg = NodeRegistry::new();
        reg.register(NodeDescriptor {
            id: "fan".into(),
            name: "Fan".into(),
            category: "Math".into(),
            version: 1,
            inputs: vec![crate::engine::node_graph::PinDescriptor::new(
                "a", "", PinType::Float,
            )],
            outputs: vec![crate::engine::node_graph::PinDescriptor::new(
                "sum", "", PinType::Float,
            )],
            pure: true,
            realm: crate::engine::node_graph::NodeRealm::Shared,
            deterministic: true,
            doc: None,
            preview: None,
        })
        .unwrap();

        let mut st = bare_state();
        st.doc.nodes = (0..5).map(|i| node(i, [i as f32 * 200.0, 0.0])).collect();
        for n in st.doc.nodes.iter_mut() {
            n.type_id = "fan".into();
        }
        // One output fanning out to four inputs.
        st.doc.edges = (1..5).map(|i| edge(0, i)).collect();
        let before = st.doc.clone();
        assert_eq!(st.doc.edges.len(), 4);

        st.break_node_links(0, &reg);
        assert!(st.doc.edges.is_empty(), "all four go");
        st.undo(&reg);
        assert_eq!(st.doc, before, "and all four come back together");
        assert!(!st.stack.can_undo(), "one gesture, one entry");
    }


    /// Acceptance 17 — holding an arrow key moves continuously but costs
    /// exactly one undo step, however long the hold.
    #[test]
    fn acceptance_17_a_held_arrow_key_is_one_undo_step() {
        let reg = NodeRegistry::new();
        let mut st = bare_state();
        st.doc.nodes = vec![node(0, [0.0, 0.0]), node(1, [100.0, 0.0])];
        st.selection = [0, 1].into_iter().collect();
        let before = st.doc.clone();

        // First press, then forty auto-repeats.
        for _ in 0..41 {
            st.nudge_selection([16.0, 0.0]);
        }
        assert!(st.nudging(), "the transaction stays open while the key is held");
        assert!(!st.stack.can_undo(), "and nothing is recorded mid-hold");
        assert_eq!(st.doc.node(0).unwrap().position, [41.0 * 16.0, 0.0]);

        st.commit_nudge(&reg);
        assert!(!st.nudging());
        assert!(st.stack.can_undo());
        st.undo(&reg);
        assert_eq!(st.doc, before, "one Ctrl+Z undoes the whole hold");
        assert!(!st.stack.can_undo(), "there was only ever one entry");
    }

    #[test]
    fn a_nudge_is_absolute_from_the_start_so_it_cannot_drift() {
        let reg = NodeRegistry::new();
        let mut st = bare_state();
        st.doc.nodes = vec![node(0, [10.0, 20.0])];
        st.selection = [0].into_iter().collect();
        st.nudge_selection([1.0, 0.0]);
        st.nudge_selection([1.0, 0.0]);
        st.nudge_selection([0.0, -1.0]);
        assert_eq!(st.doc.node(0).unwrap().position, [12.0, 19.0]);
        st.commit_nudge(&reg);
        st.undo(&reg);
        assert_eq!(st.doc.node(0).unwrap().position, [10.0, 20.0]);
    }

    #[test]
    fn escape_reverts_an_open_nudge_and_records_nothing() {
        let mut st = bare_state();
        st.doc.nodes = vec![node(0, [0.0, 0.0])];
        st.selection = [0].into_iter().collect();
        let before = st.doc.clone();
        for _ in 0..5 {
            st.nudge_selection([16.0, 0.0]);
        }
        assert!(st.interaction_in_flight(), "Escape must reach a nudge");
        st.cancel_interactions();
        assert_eq!(st.doc, before);
        assert!(!st.stack.can_undo());
        assert!(!st.nudging());
    }

    #[test]
    fn nudging_nothing_is_a_silent_no_op() {
        let reg = NodeRegistry::new();
        let mut st = bare_state();
        st.doc.nodes = vec![node(0, [0.0, 0.0])];
        let before = st.doc.clone();
        st.nudge_selection([16.0, 0.0]);
        st.commit_nudge(&reg);
        assert_eq!(st.doc, before);
        assert!(!st.stack.can_undo());
    }

    #[test]
    fn align_centers_average_rather_than_picking_an_extreme() {
        let reg = NodeRegistry::new();
        let mut st = bare_state();
        st.doc.nodes = vec![node(0, [0.0, 0.0]), node(1, [200.0, 0.0])];
        // Centres at 50 and 250; the average is 150.
        let rects = vec![(0u64, [0.0, 0.0, 100.0, 60.0]), (1u64, [200.0, 0.0, 100.0, 60.0])];
        st.align_nodes(&rects, AlignMode::CenterHorizontally, &reg);
        assert_eq!(st.doc.node(0).unwrap().position[0], 100.0);
        assert_eq!(st.doc.node(1).unwrap().position[0], 100.0);
        // Both moved toward each other; the group did not slide to one side.
        assert!(st.stack.can_undo(), "one undoable edit");
    }

    #[test]
    fn align_centers_are_ordinary_two_node_aligns() {
        for m in [AlignMode::CenterHorizontally, AlignMode::CenterVertically] {
            assert!(!m.is_distribute());
            assert_eq!(m.min_nodes(), 2);
            assert!(AlignMode::ALL.contains(&m), "and they appear in the strip");
        }
        assert_eq!(AlignMode::ALL.len(), 8);
    }


    // ── C3 behaviours

    #[test]
    fn f7_reports_what_validation_found() {
        let reg = NodeRegistry::new();
        let mut st = bare_state();
        st.doc.nodes = vec![node(0, [0.0, 0.0])];
        st.compile(&reg);
        // `test_add` is not registered in a bare registry, so this graph has a
        // problem and compile must say so rather than claiming success.
        assert!(!st.errors.is_empty(), "validation ran");
        assert!(
            st.toasts.last().is_some_and(|t| t.text.contains("error")),
            "and reported: {:?}",
            st.toasts.last().map(|t| &t.text)
        );

        st.doc.nodes.clear();
        st.compile(&reg);
        assert!(st.errors.is_empty());
        assert_eq!(st.toasts.last().map(|t| t.text.as_str()), Some("Valid"));
    }

    /// **GS-4: a mark has three states, and only one of them arms.** F9 owns
    /// presence, the gutter click owns armed-ness, Alt+click destroys — and
    /// only armed marks are handed to the runtime.
    #[test]
    fn a_breakpoint_arms_disarms_and_only_the_armed_ones_reach_the_runtime() {
        let mut st = bare_state();
        st.doc.nodes = vec![node(0, [0.0, 0.0]), node(1, [200.0, 0.0])];
        st.primary = Some(1);
        st.toggle_breakpoint();
        assert!(st.breakpoint_armed(1), "a new mark is armed — F9 means stop here");
        assert_eq!(st.armed_breakpoints(), vec![1]);

        st.cycle_breakpoint(1);
        assert!(st.has_breakpoint(1), "disabled keeps the mark");
        assert!(!st.breakpoint_armed(1));
        assert!(st.armed_breakpoints().is_empty(), "and arms nothing");

        st.cycle_breakpoint(1);
        assert!(st.breakpoint_armed(1), "clicking again re-arms it");
        // Cycling a node with no mark is a no-op, not a creation: the gutter
        // slot only draws where a mark already is.
        st.cycle_breakpoint(0);
        assert!(!st.has_breakpoint(0));

        st.remove_breakpoint(1);
        assert!(!st.has_breakpoint(1), "Alt+click destroys, per the input contract");
        assert!(st.armed_breakpoints().is_empty());
    }

    #[test]
    fn f9_toggles_a_breakpoint_on_the_last_clicked_node() {
        let mut st = bare_state();
        st.doc.nodes = vec![node(0, [0.0, 0.0]), node(1, [200.0, 0.0])];
        st.selection = [0, 1].into_iter().collect();
        st.primary = Some(1);

        st.toggle_breakpoint();
        assert!(st.has_breakpoint(1), "the mark lands on the last-clicked node");
        assert!(!st.has_breakpoint(0), "not on the whole selection");
        st.toggle_breakpoint();
        assert!(!st.has_breakpoint(1), "and toggles off");
    }

    #[test]
    fn f9_on_nothing_is_a_silent_no_op() {
        let mut st = bare_state();
        st.doc.nodes = vec![node(0, [0.0, 0.0])];
        st.toggle_breakpoint();
        assert!(st.breakpoints.is_empty());
        assert!(st.toasts.is_empty(), "and says nothing");
    }

    #[test]
    fn clearing_breakpoints_reports_the_count_but_only_when_there_were_some() {
        let mut st = bare_state();
        st.doc.nodes = (0..3).map(|i| node(i, [i as f32 * 100.0, 0.0])).collect();
        for id in 0..3 {
            st.primary = Some(id);
            st.toggle_breakpoint();
        }
        assert_eq!(st.breakpoints.len(), 3);
        st.clear_breakpoints();
        assert!(st.breakpoints.is_empty());
        assert!(st.toasts.last().is_some_and(|t| t.text.contains('3')));

        let before = st.toasts.len();
        st.clear_breakpoints();
        assert_eq!(st.toasts.len(), before, "clearing nothing says nothing");
    }

    #[test]
    fn f2_renames_an_annotation_and_never_a_node() {
        let mut st = bare_state();
        st.doc.nodes = vec![node(0, [0.0, 0.0])];
        st.doc.comments.push(CommentBox {
            rect: [10.0, 10.0, 200.0, 100.0],
            text: "note".into(),
            ..Default::default()
        });

        // A node selected: nothing to rename — `NodeInst` has no title field.
        st.select_only(0);
        assert!(!st.begin_rename());
        assert!(st.editing.is_none());

        st.clear_selection();
        st.sel_comment = Some(0);
        assert!(st.begin_rename());
        let e = st.editing.as_ref().expect("editor opened");
        assert_eq!(e.buffer, "note");
        assert_eq!(e.original, "note", "so Escape can put it back");
        assert!(!e.is_group);
    }

    #[test]
    fn page_navigation_retraces_the_path_actually_walked() {
        let mut st = bare_state();
        // Nothing to go back to at the start: silent no-op, not an error.
        assert_eq!(st.ascend_target(), None);

        st.push_nav("a.graph".into());
        st.push_nav("b.graph".into());
        assert_eq!(st.ascend_target(), Some("b.graph".to_string()));
        assert_eq!(st.ascend_target(), Some("a.graph".to_string()));
        assert_eq!(st.ascend_target(), None, "and empties cleanly");
    }

    #[test]
    fn descending_needs_a_subgraph_node_under_the_cursor() {
        let mut st = bare_state();
        st.doc.nodes = vec![node(0, [0.0, 0.0])];
        st.select_only(0);
        assert_eq!(st.descend_target(), None, "a plain node has nowhere to go");

        st.doc.nodes[0].subgraph = Some("inner.subgraph".into());
        assert_eq!(st.descend_target(), Some("inner.subgraph".to_string()));
    }

    /// T10 — Escape during a node drag puts the nodes back and records nothing.
    #[test]
    fn t10_escaping_a_node_drag_reverts_it_and_leaves_no_undo_entry() {
        let mut st = bare_state();
        st.doc.nodes = vec![node(0, [0.0, 0.0]), node(1, [400.0, 0.0])];
        let before = st.doc.clone();

        // Arm the drag the way the panel does, then move both nodes live.
        st.selection = [0, 1].into_iter().collect();
        st.node_drag = Some(NodeDrag {
            origin_world: [0.0, 0.0],
            originals: vec![(0, [0.0, 0.0]), (1, [400.0, 0.0])],
            pending: None,
            anchored: vec![],
        });
        st.doc.node_mut(0).unwrap().position = [120.0, 60.0];
        st.doc.node_mut(1).unwrap().position = [520.0, 60.0];
        assert_ne!(st.doc, before, "the drag really moved them");

        st.cancel_interactions();

        assert_eq!(st.doc, before, "Escape restores the pre-drag positions");
        assert!(!st.stack.can_undo(), "and puts nothing on the undo stack");
        assert!(st.node_drag.is_none());
        assert!(!st.interaction_in_flight());
    }

    #[test]
    fn escaping_an_annotation_resize_restores_its_rect() {
        let reg = NodeRegistry::new();
        let _ = &reg;
        let mut st = bare_state();
        st.doc.comments.push(CommentBox {
            rect: [10.0, 10.0, 200.0, 100.0],
            ..Default::default()
        });
        let before = st.doc.clone();

        st.annotation_resize = Some(AnnotationResize {
            target: Annotation::Comment(0),
            handle: ResizeHandle::BottomRight,
            origin_world: [210.0, 110.0],
            rect0: [10.0, 10.0, 200.0, 100.0],
            min_h: 40.0,
        });
        st.doc.comments[0].rect = [10.0, 10.0, 320.0, 180.0];

        st.cancel_interactions();
        assert_eq!(st.doc, before, "an abandoned resize leaves the rect alone");
        assert!(!st.stack.can_undo());
    }

    #[test]
    fn escaping_an_inline_property_drag_restores_the_old_value() {
        let mut st = bare_state();
        st.doc.nodes = vec![node(0, [0.0, 0.0])];
        st.doc.nodes[0]
            .properties
            .insert("gain".into(), PropValue::Float(1.0));
        let before = st.doc.clone();

        st.prop_edit = Some(PropEdit {
            node: 0,
            key: "gain".into(),
            old: Some(PropValue::Float(1.0)),
        });
        st.doc.nodes[0]
            .properties
            .insert("gain".into(), PropValue::Float(7.5));

        st.cancel_interactions();
        assert_eq!(st.doc, before, "the pre-gesture value comes back");
        assert!(!st.stack.can_undo());
    }

    #[test]
    fn a_property_that_did_not_exist_before_the_drag_is_removed_again() {
        let mut st = bare_state();
        st.doc.nodes = vec![node(0, [0.0, 0.0])];
        let before = st.doc.clone();

        st.prop_edit = Some(PropEdit { node: 0, key: "gain".into(), old: None });
        st.doc.nodes[0]
            .properties
            .insert("gain".into(), PropValue::Float(7.5));

        st.cancel_interactions();
        assert_eq!(st.doc, before, "no old value means the key should not exist");
    }

    #[test]
    fn interaction_in_flight_catches_the_gestures_that_touch_no_document_state() {
        let mut st = bare_state();
        assert!(!st.interaction_in_flight());
        // A marquee and a pin drag hold no unrecorded edit, so the narrower
        // `gesture_in_flight` ignores them — but Escape must still reach them.
        st.marquee = Some([0.0, 0.0]);
        assert!(!st.gesture_in_flight());
        assert!(st.interaction_in_flight());
        st.cancel_interactions();
        assert!(!st.interaction_in_flight());

        st.connect_drag = Some(ConnectDrag {
            from_node: 0,
            from_pin: "sum".into(),
            from_output: true,
        });
        assert!(st.interaction_in_flight());
        st.cancel_interactions();
        assert!(!st.interaction_in_flight());
    }

    /// Review finding 12 (LOW). An oversized clipboard payload is refused
    /// before it is parsed.
    #[test]
    fn oversized_clipboard_payload_is_refused() {
        let huge = "x".repeat(MAX_CLIPBOARD_BYTES + 1);
        assert!(GraphFragment::from_ron(&huge).is_none());
        // A real fragment is nowhere near the cap and still parses.
        let frag = GraphFragment {
            nodes: vec![node(0, [0.0, 0.0])],
            ..Default::default()
        };
        let text = frag.to_ron().unwrap();
        assert!(text.len() < MAX_CLIPBOARD_BYTES);
        assert_eq!(GraphFragment::from_ron(&text), Some(frag));
    }

    /// Group-drag capture selects exactly the nodes whose centers lie inside
    /// the group rect (P7).
    #[test]
    fn group_drag_captures_nodes_by_center() {
        // rect [0,0,100,100]. Centers inside → captured; on-edge included;
        // outside → excluded.
        let centers = [
            (0u64, [50.0, 50.0]), // inside
            (1, [100.0, 100.0]),  // exactly on the far corner (inclusive)
            (2, [150.0, 20.0]),   // outside (x)
            (3, [-1.0, 50.0]),    // outside (x)
        ];
        assert_eq!(nodes_captured_by_rect(&centers, [0.0, 0.0, 100.0, 100.0]), vec![0, 1]);
    }

    /// apply → undo round-trips the doc for every edit variant.
    #[test]
    fn edits_round_trip() {
        let base = {
            let mut d = GraphDoc::default();
            d.nodes = vec![node(0, [0.0, 0.0]), node(1, [10.0, 10.0])];
            d.edges = vec![edge(0, 1)];
            d
        };
        let edits = [
            GraphEdit::AddNode(node(2, [5.0, 5.0])),
            GraphEdit::RemoveNodes {
                nodes: vec![(1, node(1, [10.0, 10.0]))],
                edges: vec![(0, edge(0, 1))],
                comments: vec![],
                regions: vec![],
            },
            GraphEdit::Connect(Edge {
                from_node: 0,
                from_pin: "sum".to_string(),
                to_node: 1,
                to_pin: "b".to_string(),
            }),
            GraphEdit::Disconnect { edges: vec![(0, edge(0, 1))] },
            GraphEdit::MoveNodes { ids: vec![0, 1], delta: [3.0, -4.0] },
            GraphEdit::Paste {
                nodes: vec![node(7, [1.0, 1.0]), node(8, [2.0, 2.0])],
                edges: vec![edge(7, 8)],
                regions: vec![],
            },
            // Set a first value where none was stored…
            GraphEdit::SetProperty {
                node: 0,
                key: "a".to_string(),
                old: None,
                new: Some(PropValue::Float(3.5)),
            },
        ];
        for e in edits {
            let mut doc = base.clone();
            e.apply(&mut doc);
            assert_ne!(doc, base, "{}: apply should change the doc", e.description());
            e.revert(&mut doc);
            assert_eq!(doc, base, "{}: apply→revert must restore", e.description());
        }
    }

    /// `SetProperty` round-trips in all three shapes — set a first value,
    /// change an existing one, and clear one back to "no stored property" —
    /// and a whole widget gesture coalesces into exactly one undo entry.
    #[test]
    fn set_property_round_trips_and_coalesces() {
        let reg = NodeRegistry::new();

        let base = {
            let mut d = GraphDoc::default();
            let mut n = node(0, [0.0, 0.0]);
            n.properties.insert("a".into(), PropValue::Float(1.0));
            d.nodes = vec![n];
            d
        };
        let edits = [
            // change an existing value
            GraphEdit::SetProperty {
                node: 0,
                key: "a".into(),
                old: Some(PropValue::Float(1.0)),
                new: Some(PropValue::Float(9.0)),
            },
            // clear back to the descriptor default
            GraphEdit::SetProperty {
                node: 0,
                key: "a".into(),
                old: Some(PropValue::Float(1.0)),
                new: None,
            },
            // set a value that was never stored
            GraphEdit::SetProperty {
                node: 0,
                key: "b".into(),
                old: None,
                new: Some(PropValue::Bool(true)),
            },
        ];
        for e in edits {
            let mut doc = base.clone();
            e.apply(&mut doc);
            assert_ne!(doc, base, "{}: apply should change the doc", e.description());
            e.revert(&mut doc);
            assert_eq!(doc, base, "{}: apply→revert must restore", e.description());
        }

        // A drag: many live writes, one undo entry, and undo restores the
        // value as it stood before the gesture.
        let mut st = bare_state();
        st.doc = base.clone();
        for v in [2.0, 3.0, 4.0_f32] {
            st.begin_prop_edit(0, "a", &reg);
            st.doc.node_mut(0).unwrap().properties.insert("a".into(), PropValue::Float(v));
        }
        st.flush_prop_edit(&reg);
        assert!(st.stack.can_undo());
        st.undo(&reg);
        assert_eq!(st.doc, base, "one gesture = one undo entry back to the start");
        assert!(!st.stack.can_undo(), "no second entry was recorded");

        // A gesture that ends where it started records nothing.
        st.begin_prop_edit(0, "a", &reg);
        st.flush_prop_edit(&reg);
        assert!(!st.stack.can_undo(), "a no-op gesture must not dirty the stack");
    }

    /// Breaking a wire selection is one undo transaction, and undo restores
    /// the edges at their original indices so the doc still serializes
    /// byte-identically.
    #[test]
    fn breaking_selected_links_is_one_reversible_transaction() {
        let reg = NodeRegistry::new();
        let mut st = bare_state();
        st.doc.nodes = vec![node(0, [0.0, 0.0]), node(1, [10.0, 0.0]), node(2, [20.0, 0.0])];
        st.doc.edges = vec![edge(0, 1), edge(1, 2), edge(0, 2)];
        let before = st.doc.clone();

        // Nothing selected: falls through so Delete can mean "the nodes".
        assert!(!st.break_selected_links(&reg));

        // Select the first and last wire, leaving the middle one alone.
        st.select_only_edge(&edge(0, 1));
        st.toggle_edge_selected(&edge(0, 2));
        assert_eq!(st.selected_edges.len(), 2);
        assert!(st.break_selected_links(&reg));
        assert_eq!(st.doc.edges, vec![edge(1, 2)]);
        assert!(st.selected_edges.is_empty(), "selection is consumed by the break");
        assert_eq!(st.stack.undo_description().as_deref(), Some("Break 2 Links"));

        st.undo(&reg);
        assert_eq!(st.doc, before, "undo must restore the exact edge order");
        let a = crate::engine::node_graph::serialize_graph(&before).unwrap();
        let b = crate::engine::node_graph::serialize_graph(&st.doc).unwrap();
        assert_eq!(a, b, "restored doc must serialize byte-identically");

        // ⇧-click toggles back off.
        st.toggle_edge_selected(&edge(1, 2));
        st.toggle_edge_selected(&edge(1, 2));
        assert!(st.selected_edges.is_empty());
    }

    /// A wire selection survives unrelated edits but is pruned when its edge
    /// stops existing (undo/redo across a delete).
    #[test]
    fn wire_selection_is_pruned_when_its_edge_goes_away() {
        let reg = NodeRegistry::new();
        let mut st = bare_state();
        st.doc.nodes = vec![node(0, [0.0, 0.0]), node(1, [10.0, 0.0])];
        st.doc.edges = vec![edge(0, 1)];
        st.select_only_edge(&edge(0, 1));
        // Delete the nodes out from under it, then undo.
        st.selected_edges.clear();
        st.selection = [0, 1].into_iter().collect();
        st.delete_selection(&reg);
        st.select_only_edge(&edge(0, 1)); // stale key
        st.undo(&reg);
        st.redo(&reg);
        assert!(
            st.selected_edges.is_empty(),
            "a selection keyed on a removed edge must be pruned"
        );
    }

    /// Regression (review finding 1): deleting a *middle* node and undoing
    /// must restore the exact vec order, not append the node at the end —
    /// otherwise the doc is logically equal but serializes to different bytes.
    #[test]
    fn remove_middle_node_undo_preserves_order() {
        let mut doc = GraphDoc::default();
        doc.nodes = vec![node(0, [0.0, 0.0]), node(1, [1.0, 1.0]), node(2, [2.0, 2.0])];
        doc.edges = vec![edge(0, 1), edge(1, 2)];
        let before = doc.clone();
        // Remove the middle node (index 1) + its incident edges (indices 0, 1).
        let edit = GraphEdit::RemoveNodes {
            nodes: vec![(1, node(1, [1.0, 1.0]))],
            edges: vec![(0, edge(0, 1)), (1, edge(1, 2))],
            comments: vec![],
            regions: vec![],
        };
        edit.apply(&mut doc);
        assert_eq!(doc.nodes.iter().map(|n| n.id).collect::<Vec<_>>(), vec![0, 2]);
        edit.revert(&mut doc);
        // Exact structural equality *including order* (byte-stable save).
        assert_eq!(doc, before);
        assert_eq!(doc.nodes.iter().map(|n| n.id).collect::<Vec<_>>(), vec![0, 1, 2]);
        let a = crate::engine::node_graph::serialize_graph(&before).unwrap();
        let b = crate::engine::node_graph::serialize_graph(&doc).unwrap();
        assert_eq!(a, b, "restored doc must serialize byte-identically");
    }

    fn bare_state() -> GraphEditorState {
        super::test_state("t.graph")
    }


    /// Regression (finding 3): undo/redo cancels a live drag so the next frame
    /// can't overwrite the undone state or commit a bogus delta.
    #[test]
    fn undo_cancels_live_drag() {
        let reg = NodeRegistry::new();
        let mut st = bare_state();
        st.doc.nodes = vec![node(0, [0.0, 0.0])];
        // A committed move...
        st.doc.nodes[0].position = [50.0, 0.0];
        st.stack.record(GraphEdit::MoveNodes { ids: vec![0], delta: [50.0, 0.0] });
        // ...then a *live* drag on top of it, which has recorded nothing.
        st.node_drag = Some(NodeDrag {
            origin_world: [50.0, 0.0],
            originals: vec![(0, [50.0, 0.0])],
            anchored: Vec::new(),
            pending: None,
        });
        st.doc.nodes[0].position = [90.0, 0.0];

        st.undo(&reg);
        assert!(st.node_drag.is_none(), "undo must cancel the live node drag");
        assert_eq!(
            st.doc.nodes[0].position,
            [0.0, 0.0],
            "the live drag reverts to its start, then the recorded move undoes"
        );
    }

    /// Regression (finding 2): deleting the selected annotation while its
    /// inline editor is open must clear `editing` (else a later commit records
    /// against a removed index → OOB on undo/redo).
    #[test]
    fn delete_selection_clears_editing() {
        let reg = NodeRegistry::new();
        let mut st = bare_state();
        st.doc.comments = vec![comment(0.0)];
        st.sel_comment = Some(0);
        st.editing = Some(AnnotationEdit {
            is_group: false,
            index: 0,
            buffer: "x".into(),
            original: "c".into(),
            anchor_world: [0.0, 0.0],
            first_frame: true,
        });
        st.delete_selection(&reg);
        assert!(st.editing.is_none(), "delete must clear the inline editor");
        assert!(st.doc.comments.is_empty());
    }

    /// Regression (finding 5): paste moves selection to the pasted nodes and
    /// clears annotation selection (so a following Delete hits the nodes).
    #[test]
    fn paste_clears_annotation_selection() {
        let reg = NodeRegistry::new();
        let mut st = bare_state();
        st.doc.comments = vec![comment(0.0)];
        st.sel_comment = Some(0);
        let frag = GraphFragment {
            nodes: vec![node(5, [0.0, 0.0])],
            ..Default::default()
        };
        st.paste_fragment(&frag, Some([0.0, 0.0]), &reg);
        assert!(st.sel_comment.is_none(), "paste must clear annotation selection");
        assert!(!st.selection.is_empty(), "pasted nodes become the selection");
    }

    // -- 45-A P8b: the `.curve` resolver the graph canvas reads -------------

    /// A Timeline node naming `path`, in a document of its own.
    fn timeline_doc(path: &str) -> GraphDoc {
        use crate::engine::node_graph::{CURVE_PROP, TIMELINE_TYPE_ID};
        let mut doc = GraphDoc::default();
        let mut n = NodeInst {
            id: 1,
            type_id: TIMELINE_TYPE_ID.into(),
            type_version: 1,
            position: [0.0, 0.0],
            properties: BTreeMap::new(),
            subgraph: None,
            tint: None,
            title: None,
        };
        n.properties
            .insert(CURVE_PROP.into(), PropValue::Asset(path.into()));
        doc.nodes = vec![n];
        doc
    }

    /// The editor resolver loads referenced curves from disk, and an open
    /// curve tab wins over the file — the rule `build_resolver_docs` already
    /// sets for subgraphs, so a track added in the curve editor grows the
    /// Timeline's pin before the save rather than after it.
    #[test]
    fn curve_resolver_loads_from_disk_and_prefers_open_docs() {
        use curve_asset::{CurveDoc, Track};

        let dir = std::env::temp_dir().join(format!("p8b_resolver_{}", std::process::id()));
        std::fs::create_dir_all(dir.join("curves")).expect("tmp dir");
        let mut on_disk = CurveDoc::default();
        on_disk.tracks = vec![Track::new("height", "Height")];
        std::fs::write(
            dir.join("curves/x.curve"),
            curve_asset::serialize_curve(&on_disk).expect("ser"),
        )
        .expect("write");

        let mut docs = BTreeMap::new();
        docs.insert("g.graph".to_string(), timeline_doc("curves/x.curve"));

        let from_disk = build_curve_docs(std::iter::empty(), &docs, &dir);
        assert_eq!(from_disk["curves/x.curve"].slugs(), vec!["height"]);

        // The same path open in a curve tab, with a second track.
        let mut open = on_disk.clone();
        open.tracks.push(Track::new("lean", "Lean"));
        let merged = build_curve_docs(
            std::iter::once(("curves/x.curve", &open)),
            &docs,
            &dir,
        );
        assert_eq!(
            merged["curves/x.curve"].slugs(),
            vec!["height", "lean"],
            "the open document wins over the file"
        );

        // A curve nobody references is not loaded, and a missing one is simply
        // absent (→ `MissingCurve` at validate, never a fabricated track).
        let none = build_curve_docs(std::iter::empty(), &BTreeMap::new(), &dir);
        assert!(none.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// With curves resolved, a Timeline grows one Float output per track;
    /// without them it keeps base pins only and says nothing (the
    /// silent-opaque rule). A named-but-missing curve is an error, once.
    #[test]
    fn timeline_pins_follow_the_resolved_curve() {
        use crate::engine::node_graph::{
            register_std_nodes, validate_curves, DocDescriptors, GraphError, NodeRegistry, PinType,
        };
        use curve_asset::{CurveDoc, Track};

        let mut reg = NodeRegistry::new();
        let _ = register_std_nodes(&mut reg);
        let doc = timeline_doc("curves/x.curve");

        let base = DocDescriptors::new(&doc, &reg);
        let base_outs = base.descriptor(1).expect("descriptor").outputs.len();
        assert!(
            validate_curves(&base).is_empty(),
            "no resolver means no verdict, not a missing-curve error"
        );

        let mut curve = CurveDoc::default();
        curve.tracks = vec![Track::new("height", "Height"), Track::new("lean", "Lean")];
        let mut curves = BTreeMap::new();
        curves.insert("curves/x.curve".to_string(), curve);

        let d = DocDescriptors::new(&doc, &reg).with_curves(&curves);
        let desc = d.descriptor(1).expect("descriptor");
        assert_eq!(desc.outputs.len(), base_outs + 2, "one Float output per track");
        for slug in ["height", "lean"] {
            let pin = desc.output(slug).unwrap_or_else(|| panic!("pin {slug}"));
            assert_eq!(pin.ty, PinType::Float);
        }
        assert!(validate_curves(&d).is_empty());

        // Same resolver, a path it does not hold.
        let dangling = timeline_doc("curves/gone.curve");
        let d = DocDescriptors::new(&dangling, &reg).with_curves(&curves);
        assert_eq!(d.descriptor(1).expect("descriptor").outputs.len(), base_outs);
        assert!(matches!(
            validate_curves(&d).as_slice(),
            [GraphError::MissingCurve { node: 1, path }] if path == "curves/gone.curve"
        ));
    }

    /// The shipped demo pair, end to end: `runner_demo.graph`'s Timeline
    /// resolves `duck_hop.curve` through the editor's own resolver and gains
    /// that curve's tracks as pins. Skipped where the content tree is not on
    /// disk (a packaged build), because the claim is about the assets.
    #[test]
    fn the_demo_timeline_resolves_its_demo_curve() {
        use crate::engine::node_graph::{
            register_std_nodes, validate_curves, DocDescriptors, NodeRegistry,
        };

        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../content");
        let graph_path = root.join("graphs/runner_demo.graph");
        if !graph_path.exists() {
            return;
        }
        let doc = load_graph(&graph_path).expect("runner_demo.graph");
        let mut docs = BTreeMap::new();
        docs.insert("graphs/runner_demo.graph".to_string(), doc.clone());
        let curves = build_curve_docs(std::iter::empty(), &docs, &root);
        assert!(
            curves.contains_key("curves/duck_hop.curve"),
            "the demo graph's curve reference resolves from the content tree"
        );

        let mut reg = NodeRegistry::new();
        let _ = register_std_nodes(&mut reg);
        let d = DocDescriptors::new(&doc, &reg).with_curves(&curves);
        let timeline = doc
            .nodes
            .iter()
            .find(|n| n.type_id == crate::engine::node_graph::TIMELINE_TYPE_ID)
            .expect("a Timeline node");
        let desc = d.descriptor(timeline.id).expect("descriptor");
        for slug in curves["curves/duck_hop.curve"].slugs() {
            assert!(desc.output(slug).is_some(), "track '{slug}' is a pin");
        }
        assert!(validate_curves(&d).is_empty(), "nothing dangling in the demo");
    }

    /// `frame_view` fits + centers the bbox and never magnifies past 1.0×.
    #[test]
    fn frame_view_fits_and_centers() {
        // 400x300 bbox in an 800x600 viewport: raw fit = 1.8, capped at 1.0×.
        let v = frame_view(
            Vec2::new(0.0, 0.0),
            Vec2::new(400.0, 300.0),
            Vec2::new(800.0, 600.0),
            0.25,
            2.5,
        );
        assert!((v.zoom - 1.0).abs() < 1e-4, "capped at 1.0x, got {}", v.zoom);
        // bbox center (200,150) must land at the viewport center (400,300):
        // screen = rect.min + (world - pan) * zoom.
        let sx = (200.0 - v.pan.x) * v.zoom;
        let sy = (150.0 - v.pan.y) * v.zoom;
        assert!((sx - 400.0).abs() < 1e-3 && (sy - 300.0).abs() < 1e-3);
    }

    /// `frame_view` clamps: huge bbox → zoom_min; a single node → 1.0× (not max).
    #[test]
    fn frame_view_clamps_zoom() {
        let vp = Vec2::new(800.0, 600.0);
        let big = frame_view(Vec2::new(0.0, 0.0), Vec2::new(1.0e5, 1.0e5), vp, 0.25, 2.5);
        assert!((big.zoom - 0.25).abs() < 1e-4, "huge bbox clamps to zoom_min");
        let one = frame_view(Vec2::new(0.0, 0.0), Vec2::new(168.0, 60.0), vp, 0.25, 2.5);
        assert!((one.zoom - 1.0).abs() < 1e-4, "single node frames at 1.0x, got {}", one.zoom);
    }

    #[test]
    fn saved_cursor_basic() {
        let mut s = GraphEditStack::new();
        assert!(!s.is_dirty());
        s.record(GraphEdit::Connect(edge(0, 1)));
        assert!(s.is_dirty());
        s.mark_saved();
        assert!(!s.is_dirty());
    }

    #[test]
    fn undo_to_save_point_clears_dirty() {
        let mut doc = GraphDoc::default();
        let mut s = GraphEditStack::new();
        s.mark_saved(); // saved at position 0
        let e = GraphEdit::AddNode(node(0, [0.0, 0.0]));
        e.apply(&mut doc);
        s.record(e);
        assert!(s.is_dirty());
        s.undo(&mut doc);
        assert!(!s.is_dirty(), "undoing back to the save point clears dirty");
        s.redo(&mut doc);
        assert!(s.is_dirty(), "redo past the save point re-dirties");
    }

    #[test]
    fn truncated_save_branch_stays_dirty() {
        let mut doc = GraphDoc::default();
        let mut s = GraphEditStack::new();
        // Edit A, save at position 1.
        let a = GraphEdit::AddNode(node(0, [0.0, 0.0]));
        a.apply(&mut doc);
        s.record(a);
        s.mark_saved();
        assert!(!s.is_dirty());
        // Undo A (position 0), then a new edit B truncates the redo branch
        // that held the save point.
        s.undo(&mut doc);
        assert!(s.is_dirty());
        let b = GraphEdit::AddNode(node(1, [1.0, 1.0]));
        b.apply(&mut doc);
        s.record(b);
        assert!(s.is_dirty(), "save point lost to truncation → dirty");
        // Undoing the new edit does NOT reach the (now-gone) save point.
        s.undo(&mut doc);
        assert!(s.is_dirty(), "truncated save point never re-cleans without save");
    }

    #[test]
    fn resolver_prefers_open_and_loads_disk_closure() {
        use crate::engine::node_graph::save_graph;
        use crate::engine::node_graph::{GraphResolver, IfacePin, PinType};

        // Disk: leaf.subgraph (float iface), mid.subgraph → references leaf.
        let dir = std::env::temp_dir().join("rust_engine_graph_resolver_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("lib")).unwrap();
        let leaf = GraphDoc {
            inputs: vec![IfacePin { slug: "x".into(), label: "X".into(), ty: PinType::Float }],
            ..GraphDoc::default()
        };
        let mut mid = GraphDoc::default();
        mid.nodes.push(NodeInst {
            id: 0,
            type_id: SUBGRAPH_TYPE_ID.to_string(),
            type_version: 1,
            position: [0.0, 0.0],
            properties: std::collections::BTreeMap::new(),
            subgraph: Some("lib/leaf.subgraph".to_string()),
        
        tint: None, title: None,});
        save_graph(&dir.join("lib/leaf.subgraph"), &leaf).unwrap();
        save_graph(&dir.join("lib/mid.subgraph"), &mid).unwrap();

        // Open host references mid; its own open copy has an extra node so we
        // can prove the open doc wins over any disk copy.
        let mut host = GraphDoc::default();
        host.nodes.push(NodeInst {
            id: 9,
            type_id: SUBGRAPH_TYPE_ID.to_string(),
            type_version: 1,
            position: [0.0, 0.0],
            properties: std::collections::BTreeMap::new(),
            subgraph: Some("lib/mid.subgraph".to_string()),
        
        tint: None, title: None,});

        let open = [("main.graph", &host)];
        let docs = build_resolver_docs(open.into_iter().map(|(k, d)| (k, d)), &dir);

        // Open doc present verbatim (wins over disk), disk closure loaded.
        assert_eq!(docs.resolve("main.graph").unwrap().nodes[0].id, 9);
        assert!(docs.resolve("lib/mid.subgraph").is_some());
        assert_eq!(
            docs.resolve("lib/leaf.subgraph").unwrap().inputs.len(),
            1,
            "transitively-referenced leaf loaded from disk"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn move_coalesces_to_single_entry() {
        // A drag records exactly one MoveNodes for the whole gesture: applying
        // one net-delta edit and undoing it restores the start positions.
        let mut doc = GraphDoc::default();
        doc.nodes = vec![node(0, [0.0, 0.0]), node(1, [10.0, 0.0])];
        let mut s = GraphEditStack::new();
        // Simulate live drag: positions already moved to +12,+7.
        for n in doc.nodes.iter_mut() {
            n.position[0] += 12.0;
            n.position[1] += 7.0;
        }
        s.record(GraphEdit::MoveNodes { ids: vec![0, 1], delta: [12.0, 7.0] });
        assert_eq!(s.undo.len(), 1, "one drag = one undo entry");
        s.undo(&mut doc);
        assert_eq!(doc.nodes[0].position, [0.0, 0.0]);
        assert_eq!(doc.nodes[1].position, [10.0, 0.0]);
    }

    // -- GS-3: watches -----------------------------------------------------

    /// A watch reports staleness from when its value last **changed**, not
    /// from when it was last read: a value that keeps arriving unchanged is
    /// exactly what the age tag is about. And the chip's text elides long
    /// values rather than pushing the node off the canvas.
    #[test]
    fn a_watch_ages_from_its_last_change_and_elides_long_values() {
        let mut w = Watch::new(7, "value", true);
        assert!(w.last.is_none());
        assert_eq!(w.stale_for(0.0), None, "nothing has ever arrived");
        assert_eq!(watch_chip_text(w.last.as_deref()), "\u{2014}");

        w.observe("37.5");
        let first = w.changed_at;
        assert_eq!(w.last.as_deref(), Some("37.5"));
        assert!(first.is_some());
        // The same value again does not reset the clock…
        w.observe("37.5");
        assert_eq!(w.changed_at, first);
        // …a different one does.
        w.observe("36.0");
        assert!(w.changed_at != first);
        // Fresh by any real threshold, stale by a zero one.
        assert_eq!(w.stale_for(WATCH_STALE_SECS), None);
        assert!(w.stale_for(0.0).is_some());

        assert!(w.is(7, "value", true));
        assert!(!w.is(7, "value", false), "an input and an output are different pins");
        assert!(!w.is(8, "value", true));

        let long = "Entity(duck_2) at [1.0, 2.0, 3.0] holding a very long spelling";
        let shown = watch_chip_text(Some(long));
        assert_eq!(shown.chars().count(), WATCH_CHARS + 1);
        assert!(shown.ends_with('\u{2026}'));
        assert_eq!(watch_chip_text(Some("37.5")), "37.5");
    }

    // -- GS-2: variables panel ---------------------------------------------

    fn var(slug: &str, ty: PinType, group: Option<&str>) -> VarDecl {
        VarDecl {
            slug: slug.to_string(),
            label: slug.to_string(),
            default: PropValue::zero_of(&ty),
            ty,
            group: group.map(str::to_string),
        }
    }

    fn var_node(id: u64, slug: &str, set: bool) -> NodeInst {
        let mut n = node(id, [0.0, 0.0]);
        n.type_id = if set {
            crate::engine::node_graph::VAR_SET_TYPE_ID
        } else {
            crate::engine::node_graph::VAR_GET_TYPE_ID
        }
        .to_string();
        n.properties.insert(
            crate::engine::node_graph::VAR_PROP.to_string(),
            PropValue::Str(slug.to_string()),
        );
        n
    }

    /// A group is **display metadata**: unset it serializes to nothing at all
    /// (so a document nobody grouped is byte-identical to one from before
    /// groups existed), setting it moves no declaration, and undo restores the
    /// file exactly.
    #[test]
    fn variable_groups_are_metadata_and_round_trip() {
        let reg = NodeRegistry::new();
        let mut st = bare_state();
        st.doc.variables = vec![
            var("health", PinType::Float, None),
            var("score", PinType::Int, None),
        ];
        let bytes = |d: &GraphDoc| crate::engine::node_graph::serialize_graph(d).unwrap();
        let before = st.doc.clone();
        assert!(
            !bytes(&before).contains("group:"),
            "an ungrouped document never writes the field"
        );
        // The claim that matters: a committed file nobody grouped still
        // serializes to exactly the bytes on disk after the field was added.
        let on_disk = Path::new("../content/graphs/runner_demo.graph");
        if let Ok(text) = std::fs::read_to_string(on_disk) {
            let doc = load_graph(on_disk).expect("the committed demo graph parses");
            assert_eq!(bytes(&doc), text, "adding `group` must not rewrite old files");
        }

        assert!(st.set_variable_group("score", Some("State".into()), &reg));
        assert_eq!(st.doc.variables[1].group.as_deref(), Some("State"));
        assert_eq!(
            st.doc.variables.iter().map(|v| v.slug.as_str()).collect::<Vec<_>>(),
            vec!["health", "score"],
            "grouping never reorders"
        );
        assert!(bytes(&st.doc).contains("group: Some(\"State\")"));
        assert_eq!(st.stack.undo_description().as_deref(), Some("Group Variable"));
        st.undo(&reg);
        assert_eq!(bytes(&st.doc), bytes(&before));

        // Clearing is the same edit the other way, with its own label.
        st.redo(&reg);
        assert!(st.set_variable_group("score", None, &reg));
        assert_eq!(st.stack.undo_description().as_deref(), Some("Ungroup Variable"));
        assert_eq!(bytes(&st.doc), bytes(&before));
        // A no-op assignment records nothing.
        assert!(!st.set_variable_group("score", None, &reg));
    }

    /// Reorder is the **only** gesture that changes declaration order, and it
    /// is exactly reversible — the vec order is what a byte-stable save is.
    #[test]
    fn reorder_variable_round_trips_byte_identically() {
        let reg = NodeRegistry::new();
        let mut st = bare_state();
        st.doc.variables = vec![
            var("a", PinType::Float, None),
            var("b", PinType::Int, None),
            var("c", PinType::Bool, None),
        ];
        let bytes = |d: &GraphDoc| crate::engine::node_graph::serialize_graph(d).unwrap();
        let before = st.doc.clone();

        assert!(st.reorder_variable(2, 0, &reg));
        assert_eq!(
            st.doc.variables.iter().map(|v| v.slug.as_str()).collect::<Vec<_>>(),
            vec!["c", "a", "b"]
        );
        assert_eq!(st.stack.undo_description().as_deref(), Some("Reorder Variable"));
        assert_eq!(st.stack.undo_len(), 1, "one drag, one entry");
        st.undo(&reg);
        assert_eq!(bytes(&st.doc), bytes(&before));
        // Degenerate moves are refused rather than recorded.
        assert!(!st.reorder_variable(1, 1, &reg));
        assert!(!st.reorder_variable(0, 9, &reg));
        assert_eq!(st.stack.undo_len(), 0);
    }

    /// Array entries ride the existing default path: one undo entry per
    /// gesture, each named for the gesture, each byte-identical on undo.
    #[test]
    fn array_default_entries_edit_one_gesture_at_a_time() {
        let reg = NodeRegistry::new();
        let mut st = bare_state();
        st.doc.variables = vec![var("path", PinType::Array(Box::new(PinType::Vec3)), None)];
        let bytes = |d: &GraphDoc| crate::engine::node_graph::serialize_graph(d).unwrap();
        let before = st.doc.clone();

        assert!(st.add_array_entry("path", &reg));
        assert!(st.add_array_entry("path", &reg));
        assert_eq!(st.stack.undo_description().as_deref(), Some("Add Array Entry"));
        assert_eq!(st.stack.undo_len(), 2, "one entry per gesture");
        // The new entries take the element type's zero, not a guess.
        assert_eq!(
            st.doc.variables[0].default,
            Some(PropValue::Array(vec![
                PropValue::Vec3([0.0; 3]),
                PropValue::Vec3([0.0; 3])
            ]))
        );

        // A component edit coalesces like any other default drag.
        assert!(st.set_array_entry("path", 1, PropValue::Vec3([1.0, 2.0, 3.0]), &reg));
        st.flush_var_default_edit(&reg);
        assert_eq!(st.stack.undo_description().as_deref(), Some("Set Variable Default"));

        assert!(st.move_array_entry("path", 1, 0, &reg));
        assert_eq!(st.stack.undo_description().as_deref(), Some("Reorder Array Entry"));
        assert_eq!(
            st.doc.variables[0].default,
            Some(PropValue::Array(vec![
                PropValue::Vec3([1.0, 2.0, 3.0]),
                PropValue::Vec3([0.0; 3])
            ]))
        );
        assert!(st.remove_array_entry("path", 0, &reg));
        assert_eq!(st.stack.undo_description().as_deref(), Some("Remove Array Entry"));

        // Every gesture undoes back to the byte-identical start.
        while st.stack.can_undo() {
            st.undo(&reg);
        }
        assert_eq!(bytes(&st.doc), bytes(&before));

        // Out-of-range and non-array targets are refused, never panics.
        assert!(!st.remove_array_entry("path", 7, &reg));
        st.doc.variables.push(var("n", PinType::Float, None));
        assert!(!st.add_array_entry("n", &reg));
        // An element type with no literal has no entry to add.
        st.doc
            .variables
            .push(var("who", PinType::Array(Box::new(PinType::Entity)), None));
        assert!(!st.add_array_entry("who", &reg));
    }

    /// The list the strip draws: group headers over the unchanged declaration
    /// order, filtered rows, dropped empty headers, and the counts the header
    /// and the "hidden" line quote.
    #[test]
    fn variables_view_groups_filters_and_counts() {
        let mut doc = GraphDoc::default();
        doc.variables = vec![
            var("max_health", PinType::Float, Some("Config")),
            var("score", PinType::Int, Some("State")),
            var("loose", PinType::Bool, None),
            var("move_speed", PinType::Float, Some("Config")),
        ];
        let none = BTreeSet::new();

        let v = variables_view(&doc, "", &none);
        assert_eq!(v.total, 4);
        assert_eq!(v.matches, 4);
        assert_eq!(v.hidden, 0);
        // Group order is first-declaration order; ungrouped trails, always.
        assert_eq!(
            v.rows,
            vec![
                VarListRow::Group { name: Some("Config".into()), count: 2, collapsed: false },
                VarListRow::Var(0),
                VarListRow::Var(3),
                VarListRow::Group { name: Some("State".into()), count: 1, collapsed: false },
                VarListRow::Var(1),
                VarListRow::Group { name: None, count: 1, collapsed: false },
                VarListRow::Var(2),
            ]
        );

        // Filtering drops non-matching rows *and* the headers left empty.
        let v = variables_view(&doc, "sc", &none);
        assert_eq!((v.matches, v.hidden), (1, 3));
        assert_eq!(
            v.rows,
            vec![
                VarListRow::Group { name: Some("State".into()), count: 1, collapsed: false },
                VarListRow::Var(1),
            ]
        );
        // The slug counts as a name: filtering is over both.
        assert_eq!(variables_view(&doc, "MAX_", &none).matches, 1);

        // A collapsed group keeps its header and its count, drops its rows.
        let mut collapsed = BTreeSet::new();
        collapsed.insert("Config".to_string());
        let v = variables_view(&doc, "", &collapsed);
        assert_eq!(
            v.rows[0],
            VarListRow::Group { name: Some("Config".into()), count: 2, collapsed: true }
        );
        assert_eq!(v.rows[1], VarListRow::Group { name: Some("State".into()), count: 1, collapsed: false });

        // A document nobody grouped draws no headers at all — the shipped look.
        doc.variables.iter_mut().for_each(|v| v.group = None);
        let v = variables_view(&doc, "", &none);
        assert!(v.rows.iter().all(|r| matches!(r, VarListRow::Var(_))));
        assert_eq!(v.rows.len(), 4);
    }

    /// The in-row reason line is derived from the validation results, so the
    /// row and the canvas cannot disagree about whether something is wrong.
    #[test]
    fn variable_mismatch_names_the_count_and_the_expected_type() {
        let mut doc = GraphDoc::default();
        doc.variables = vec![var("is_dead", PinType::Bool, None)];
        doc.nodes = vec![var_node(1, "is_dead", false), node(2, [0.0, 0.0])];
        let edge = |from: u64, to: u64| Edge {
            from_node: from,
            from_pin: "value".into(),
            to_node: to,
            to_pin: "a".into(),
        };
        let errors = vec![
            GraphError::TypeMismatch {
                edge: edge(1, 2),
                from_ty: PinType::Bool,
                to_ty: PinType::Float,
            },
            GraphError::TypeMismatch {
                edge: edge(1, 3),
                from_ty: PinType::Bool,
                to_ty: PinType::Float,
            },
            // Someone else's problem: not this variable's wire.
            GraphError::TypeMismatch {
                edge: edge(9, 8),
                from_ty: PinType::Int,
                to_ty: PinType::Float,
            },
        ];
        assert_eq!(
            variable_mismatch(&doc, "is_dead", &errors).as_deref(),
            Some("2 wired uses expect Float")
        );
        assert_eq!(variable_mismatch(&doc, "is_dead", &[]), None);
        assert_eq!(variable_mismatch(&doc, "nobody", &errors), None);
        // One use reads as one, and the verb agrees with it.
        let single = vec![errors[0].clone()];
        assert_eq!(
            variable_mismatch(&doc, "is_dead", &single).as_deref(),
            Some("1 wired use expects Float")
        );
    }

    /// Locate walks the uses in document order and wraps — repeat clicks
    /// cycle rather than re-framing the first one forever.
    #[test]
    fn locate_cycles_the_uses_in_document_order() {
        let mut st = bare_state();
        st.doc.variables = vec![var("health", PinType::Float, None)];
        st.doc.nodes = vec![
            var_node(7, "health", false),
            node(8, [0.0, 0.0]),
            var_node(3, "health", true),
        ];
        assert_eq!(variable_node_ids(&st.doc, "health"), vec![7, 3]);
        assert_eq!(st.next_locate("health"), Some(7));
        assert_eq!(st.next_locate("health"), Some(3));
        assert_eq!(st.next_locate("health"), Some(7), "the cycle wraps");
        assert_eq!(st.next_locate("nobody"), None);
    }

    /// The retype dialog promises exactly what the no-coercion rule performs.
    #[test]
    fn retype_outcome_states_what_the_rule_will_do() {
        let mut decl = var("health", PinType::Float, None);
        decl.default = Some(PropValue::Float(100.0));
        // Same shape survives…
        assert_eq!(
            retype_default_outcome(&decl, &PinType::Float).as_deref(),
            Some("Default 100 is kept.")
        );
        // …anything else resets to the new type's zero, and says so.
        assert!(retype_default_outcome(&decl, &PinType::Int)
            .unwrap()
            .contains("reset"));
        // A type with no literal drops it, and says that instead.
        assert!(retype_default_outcome(&decl, &PinType::Entity)
            .unwrap()
            .contains("no literal"));
        decl.default = None;
        assert_eq!(retype_default_outcome(&decl, &PinType::Int), None);
    }

    // -- GS-1: custom-event payload fields ---------------------------------

    /// An `event_custom` node named `event`, with the given payload fields.
    fn event_node(id: u64, event: &str, fields: &[(&str, &str)]) -> NodeInst {
        let mut n = node(id, [0.0, 0.0]);
        n.type_id = EVENT_CUSTOM_TYPE_ID.to_string();
        n.properties
            .insert(EVENT_NAME_PROP.into(), PropValue::Str(event.into()));
        for (slug, ty) in fields {
            n.properties.insert(
                format!("{EVENT_PAYLOAD_PREFIX}{slug}"),
                PropValue::Enum((*ty).into()),
            );
        }
        n
    }

    fn payload_edge(from: u64, slug: &str, to: u64) -> Edge {
        Edge {
            from_node: from,
            from_pin: slug.to_string(),
            to_node: to,
            to_pin: "a".to_string(),
        }
    }

    /// Add / remove / rename are each **one** undo entry, under the gesture's
    /// own name, and each round-trips the document byte-identically.
    #[test]
    fn payload_field_edits_round_trip_as_one_entry() {
        let reg = NodeRegistry::new();
        let mut st = bare_state();
        st.doc.nodes = vec![event_node(0, "Hit", &[("damage", "float")]), node(1, [0.0, 0.0])];
        st.doc.edges = vec![payload_edge(0, "damage", 1)];
        let before = st.doc.clone();
        let bytes = |d: &GraphDoc| crate::engine::node_graph::serialize_graph(d).unwrap();

        // --- add
        assert_eq!(st.add_payload_field(0, "Instigator", &reg).as_deref(), Some("instigator"));
        assert_eq!(
            st.doc.nodes[0].properties.get("payload.instigator"),
            Some(&PropValue::Enum(DEFAULT_PAYLOAD_TYPE.into())),
            "a new field defaults to Float"
        );
        assert_eq!(st.stack.undo_description().as_deref(), Some("Add Payload Field"));
        assert_eq!(st.stack.undo_len(), 1, "one gesture, one entry");
        st.undo(&reg);
        assert_eq!(bytes(&st.doc), bytes(&before));

        // --- rename, carrying this document's wires with it
        assert_eq!(
            st.rename_payload_field(0, "damage", "Hit Damage", &reg).as_deref(),
            Some("hit_damage")
        );
        assert!(!st.doc.nodes[0].properties.contains_key("payload.damage"));
        assert_eq!(
            st.doc.nodes[0].properties.get("payload.hit_damage"),
            Some(&PropValue::Enum("float".into()))
        );
        assert_eq!(
            st.doc.edges,
            vec![payload_edge(0, "hit_damage", 1)],
            "incident edges follow the rename"
        );
        assert_eq!(st.stack.undo_description().as_deref(), Some("Rename Payload Field"));
        assert_eq!(st.stack.undo_len(), 1);
        st.undo(&reg);
        assert_eq!(bytes(&st.doc), bytes(&before), "including the edge order");

        // --- remove: the wire is left pointing at a pin that no longer
        //     exists, which is what makes it a ghost row instead of a silent
        //     unwiring.
        assert!(st.remove_payload_field(0, "damage", &reg));
        assert!(!st.doc.nodes[0].properties.contains_key("payload.damage"));
        assert_eq!(st.doc.edges.len(), 1, "the wire keeps its landing spot");
        assert_eq!(st.stack.undo_description().as_deref(), Some("Remove Payload Field"));
        assert_eq!(st.stack.undo_len(), 1);
        st.undo(&reg);
        assert_eq!(bytes(&st.doc), bytes(&before));
    }

    /// Payload slugs follow the variable rules: snake_case, and a collision
    /// takes a `_2`, `_3` … suffix — on both add and rename, because edges key
    /// by slug and two fields may never share one.
    #[test]
    fn payload_slugs_are_snake_case_and_collision_suffixed() {
        let reg = NodeRegistry::new();
        let mut st = bare_state();
        st.doc.nodes = vec![event_node(0, "Hit", &[])];
        assert_eq!(st.add_payload_field(0, "Damage", &reg).as_deref(), Some("damage"));
        assert_eq!(st.add_payload_field(0, "damage", &reg).as_deref(), Some("damage_2"));
        assert_eq!(st.add_payload_field(0, "Damage!", &reg).as_deref(), Some("damage_3"));
        assert_eq!(st.add_payload_field(0, "", &reg).as_deref(), Some("var"));
        // A rename onto a taken slug disambiguates rather than clobbering.
        assert_eq!(
            st.rename_payload_field(0, "damage_3", "damage", &reg).as_deref(),
            Some("damage_4")
        );
        // A no-op rename records nothing.
        assert_eq!(st.rename_payload_field(0, "damage", "Damage", &reg), None);
        // Not a custom event: no payload fields at all.
        st.doc.nodes.push(node(9, [0.0, 0.0]));
        assert_eq!(st.add_payload_field(9, "x", &reg), None);
    }

    /// The reader count is what the confirmation is *for*: wires out of a
    /// matching payload pin, in this document and in every document the
    /// resolver can enumerate.
    #[test]
    fn payload_reader_count_spans_the_resolvable_documents() {
        let mut here = GraphDoc::default();
        here.nodes = vec![
            event_node(0, "Hit", &[("damage", "float"), ("quiet", "float")]),
            node(1, [0.0, 0.0]),
            node(2, [0.0, 0.0]),
        ];
        here.edges = vec![payload_edge(0, "damage", 1), payload_edge(0, "damage", 2)];

        // Another open graph listening to the same event, reading the field.
        let mut there = GraphDoc::default();
        there.nodes = vec![event_node(7, "Hit", &[("damage", "float")]), node(8, [0.0, 0.0])];
        there.edges = vec![payload_edge(7, "damage", 8)];

        // A third that listens to a *different* event — same slug, no relation.
        let mut other = GraphDoc::default();
        other.nodes = vec![event_node(3, "Land", &[("damage", "float")]), node(4, [0.0, 0.0])];
        other.edges = vec![payload_edge(3, "damage", 4)];

        let mut docs: BTreeMap<String, GraphDoc> = BTreeMap::new();
        // The resolver carries a (possibly stale) copy of the open document
        // too; it must be counted once, from the live one.
        docs.insert("graphs/here.graph".into(), here.clone());
        docs.insert("graphs/there.graph".into(), there);
        docs.insert("graphs/other.graph".into(), other);

        assert_eq!(
            payload_reader_count(&here, "graphs/here.graph", "Hit", "damage", &docs),
            (3, 2),
            "two wires here + one there, across two graphs"
        );
        // An unwired field has no readers, so no ceremony.
        assert_eq!(
            payload_reader_count(&here, "graphs/here.graph", "Hit", "quiet", &docs),
            (0, 0)
        );
        // Same-document only, when the resolver cannot enumerate anything.
        struct Blind;
        impl GraphResolver for Blind {
            fn resolve(&self, _: &str) -> Option<&GraphDoc> {
                None
            }
        }
        assert_eq!(
            payload_reader_count(&here, "graphs/here.graph", "Hit", "damage", &Blind),
            (2, 1),
            "an un-enumerable resolver answers with nothing rather than lying"
        );
    }

    // --- Task 41: an embedded region is part of its owner node. -----------

    /// A one-node rule region, distinctive enough that equality means it
    /// actually round-tripped.
    fn rule_region() -> GraphRegion {
        GraphRegion {
            nodes: vec![NodeInst {
                id: 0,
                type_id: "var_get".to_string(),
                type_version: 1,
                position: [12.0, 34.0],
                properties: [(
                    crate::engine::node_graph::VAR_PROP.to_string(),
                    PropValue::Str("speed".to_string()),
                )]
                .into(),
                subgraph: None,
                tint: None,
                title: None,
            }],
            edges: vec![Edge {
                from_node: 0,
                from_pin: "value".to_string(),
                to_node: 1,
                to_pin: "value".to_string(),
            }],
        }
    }

    /// Deleting a node with an embedded region takes the region with it, and
    /// undo brings the document back exactly — a transition and its rule are
    /// one unit (spec story 21).
    #[test]
    fn deleting_a_node_takes_its_region_and_undo_restores_it() {
        let reg = NodeRegistry::new();
        let mut st = tests_support::empty_state();
        st.doc.nodes = vec![node(0, [0.0, 0.0]), node(1, [50.0, 0.0])];
        st.doc.edges = vec![edge(0, 1)];
        st.doc.regions.insert(1, rule_region());
        let before = st.doc.clone();

        st.selection.insert(1);
        st.delete_selection(&reg);
        assert!(st.doc.regions.is_empty(), "the region died with its owner");
        assert!(st.doc.node(1).is_none());

        st.undo(&reg);
        assert_eq!(st.doc, before, "undo restores nodes, edges and the region");

        st.redo(&reg);
        assert!(st.doc.regions.is_empty(), "redo removes it again");
    }

    /// Duplicating a node clones its region under the duplicate's id; undoing
    /// the paste removes only the clone.
    #[test]
    fn duplicate_carries_the_region_under_the_new_id() {
        let reg = NodeRegistry::new();
        let mut st = tests_support::empty_state();
        st.doc.nodes = vec![node(3, [0.0, 0.0])];
        st.doc.regions.insert(3, rule_region());
        st.selection.insert(3);

        st.duplicate_selection(&reg);
        assert_eq!(st.doc.nodes.len(), 2);
        let dup = st.doc.nodes.last().unwrap().id;
        assert_ne!(dup, 3);
        assert_eq!(
            st.doc.regions.get(&dup),
            Some(&rule_region()),
            "the clone owns an identical region"
        );
        assert_eq!(st.doc.regions.len(), 2);

        st.undo(&reg);
        assert_eq!(st.doc.regions.len(), 1, "undo removes only the clone's region");
        assert!(st.doc.regions.contains_key(&3));
    }

    /// The clipboard fragment carries regions across serialize/parse, and a
    /// pre-region payload (no `regions` field) still parses — the additive-
    /// field rule the container itself follows.
    #[test]
    fn fragments_round_trip_regions_and_accept_old_payloads() {
        let mut frag = GraphFragment {
            nodes: vec![node(5, [0.0, 0.0])],
            ..GraphFragment::default()
        };
        frag.regions.insert(5, rule_region());
        let text = frag.to_ron().unwrap();
        assert_eq!(GraphFragment::from_ron(&text), Some(frag));

        // A region-less fragment serializes without the field at all — its
        // payload is byte-shaped like a pre-region one, and parses.
        let old = GraphFragment {
            nodes: vec![node(5, [0.0, 0.0])],
            ..GraphFragment::default()
        };
        let old_text = old.to_ron().unwrap();
        assert!(!old_text.contains("regions"));
        assert_eq!(GraphFragment::from_ron(&old_text), Some(old));
    }

    /// The state-machine drag: a flow wire dropped state → state (or Any
    /// State → state) reads as "make a transition here"; one undo takes the
    /// whole gesture back.
    #[test]
    fn a_state_to_state_wire_becomes_a_transition() {
        use crate::engine::animation::graph::plan::{
            ANIM_ANY_STATE_TYPE_ID, ANIM_STATE_TYPE_ID, ANIM_TRANSITION_TYPE_ID, STATE_IN_PIN,
            STATE_OUT_PIN,
        };
        let reg = NodeRegistry::new();
        let mut st = test_state("graphs/duck.animgraph");
        let mut n = |id: u64, ty: &str, pos: [f32; 2]| NodeInst {
            id,
            type_id: ty.to_string(),
            type_version: 1,
            position: pos,
            properties: Default::default(),
            subgraph: None,
            tint: None,
            title: None,
        };
        st.doc.nodes = vec![
            n(0, ANIM_STATE_TYPE_ID, [0.0, 0.0]),
            n(1, ANIM_STATE_TYPE_ID, [400.0, 80.0]),
            n(2, ANIM_ANY_STATE_TYPE_ID, [0.0, 200.0]),
        ];
        let flow = |a: u64, b: u64| Edge {
            from_node: a,
            from_pin: STATE_OUT_PIN.to_string(),
            to_node: b,
            to_pin: STATE_IN_PIN.to_string(),
        };
        // The shortcut recognizes both source families…
        assert_eq!(transition_shortcut(&st.doc, &flow(0, 1)), Some((0, 1)));
        assert_eq!(transition_shortcut(&st.doc, &flow(2, 1)), Some((2, 1)));
        // …and nothing else.
        assert_eq!(transition_shortcut(&st.doc, &flow(0, 2)), None, "Any State has no `in`");

        let before = st.doc.clone();
        let t = st.insert_transition_between(0, 1, &reg);
        let node = st.doc.node(t).expect("inserted");
        assert_eq!(node.type_id, ANIM_TRANSITION_TYPE_ID);
        assert_eq!(node.position, [200.0, 40.0], "midway between the states");
        assert_eq!(st.doc.edges.len(), 2, "state → transition → state");
        st.undo(&reg);
        assert_eq!(st.doc, before, "one undo takes the whole gesture back");
    }

    /// The animation compiler's refusal strings anchor to the node they name
    /// — the editor-side half of the "state '<name>': …" contract.
    #[test]
    fn anim_refusals_anchor_to_the_node_they_name() {
        use crate::engine::animation::graph::plan::{
            ANIM_ENTRY_TYPE_ID, ANIM_PLAY_ONCE_TYPE_ID, ANIM_STATE_TYPE_ID,
            ANIM_TRANSITION_TYPE_ID,
        };
        let mut doc = GraphDoc::default();
        let mut n = |id: u64, ty: &str, title: Option<&str>| {
            doc.nodes.push(NodeInst {
                id,
                type_id: ty.to_string(),
                type_version: 1,
                position: [0.0, 0.0],
                properties: Default::default(),
                subgraph: None,
                tint: None,
                title: title.map(str::to_string),
            });
        };
        n(0, ANIM_ENTRY_TYPE_ID, None);
        n(1, ANIM_STATE_TYPE_ID, Some("Idle"));
        n(2, ANIM_STATE_TYPE_ID, None); // display name "State 2"
        n(3, ANIM_TRANSITION_TYPE_ID, None);
        n(4, ANIM_PLAY_ONCE_TYPE_ID, Some("Attack"));

        let a = |msg: &str| anchor_anim_refusal(&doc, msg);
        assert_eq!(a("state 'Idle' names no clip (property `clip`) and has no blend tree"), Some(1));
        assert_eq!(a("state 'State 2': blend tree has no RESULT node"), Some(2));
        assert_eq!(a("transition 3 has no source state"), Some(3));
        assert_eq!(a("transition 3: rule has no RESULT node"), Some(3));
        assert_eq!(a("play-once slot 'Attack' names no clip (property `clip`)"), Some(4));
        assert_eq!(a("the ENTRY node is not wired to a state"), Some(0));
        assert_eq!(a("parameter 'speed' is declared twice"), None);
        assert_eq!(a("transition 99 has no source state"), None, "unknown ids stay unanchored");
    }

    /// An `.animgraph` state recomputes its domain errors on every edit; a
    /// `.graph` never grows any.
    #[test]
    fn after_edit_refreshes_domain_errors_for_the_animation_domain() {
        use crate::engine::animation::graph::new_animgraph_doc;
        let reg = NodeRegistry::new();
        let mut st = test_state("graphs/duck.animgraph");
        assert!(st.domain.is_animation());
        st.doc = new_animgraph_doc();
        st.after_edit(&reg);
        assert_eq!(st.domain_errors.len(), 1);
        assert_eq!(st.domain_errors[0].node, Some(1), "anchored to the clipless state");

        // Naming a clip clears the refusal.
        st.doc.node_mut(1).unwrap().properties.insert(
            crate::engine::animation::graph::plan::CLIP_PROP.to_string(),
            PropValue::Asset("anims/idle.anim".to_string()),
        );
        st.after_edit(&reg);
        assert!(st.domain_errors.is_empty());

        let mut script = test_state("graphs/t.graph");
        script.after_edit(&reg);
        assert!(script.domain_errors.is_empty());
    }
}

/// Rule-scope tests (Task 41 ticket 05): the peek's state machinery —
/// projection, drain-to-parent-history, rebuild-on-undo, gating.
#[cfg(test)]
mod rule_scope_tests {
    use super::*;
    use crate::engine::animation::graph::plan::{
        ANIM_ENTRY_TYPE_ID, ANIM_RULE_RESULT_TYPE_ID, ANIM_STATE_TYPE_ID,
        ANIM_TRANSITION_TYPE_ID, CLIP_PROP, RULE_RESULT_PIN, STATE_IN_PIN, STATE_OUT_PIN,
        TRANSITION_FROM_PIN, TRANSITION_TO_PIN,
    };
    use crate::engine::animation::graph::anim_node_registry;
    use crate::engine::editor::graph_anim_chip::transition_chip;
    use crate::engine::node_graph::{
        GraphRealm, GraphRegion, VarDecl, VAR_GET_TYPE_ID, VAR_PROP, VAR_VALUE_PIN,
    };
    use node_graph_types::std_nodes::COMPARE_FLOAT;

    fn node(id: u64, type_id: &str, title: Option<&str>) -> NodeInst {
        NodeInst {
            id,
            type_id: type_id.to_string(),
            type_version: 1,
            position: [0.0, 0.0],
            properties: Default::default(),
            subgraph: None,
            tint: None,
            title: title.map(str::to_string),
        }
    }

    fn edge(from: u64, from_pin: &str, to: u64, to_pin: &str) -> Edge {
        Edge {
            from_node: from,
            from_pin: from_pin.to_string(),
            to_node: to,
            to_pin: to_pin.to_string(),
        }
    }

    /// ENTRY → Idle —(transition 3)→ Run, `region` as transition 3's rule,
    /// one declared Float parameter `speed`.
    fn machine(region: Option<GraphRegion>) -> GraphEditorState {
        let mut st = super::test_state("graphs/t.animgraph");
        st.doc.realm = GraphRealm::Client;
        let mut idle = node(1, ANIM_STATE_TYPE_ID, Some("Idle"));
        idle.properties
            .insert(CLIP_PROP.to_string(), PropValue::Asset("anims/idle.anim".into()));
        let mut run = node(2, ANIM_STATE_TYPE_ID, Some("Run"));
        run.properties
            .insert(CLIP_PROP.to_string(), PropValue::Asset("anims/run.anim".into()));
        st.doc.nodes = vec![
            node(0, ANIM_ENTRY_TYPE_ID, None),
            idle,
            run,
            node(3, ANIM_TRANSITION_TYPE_ID, None),
        ];
        st.doc.edges = vec![
            edge(0, STATE_OUT_PIN, 1, STATE_IN_PIN),
            edge(1, STATE_OUT_PIN, 3, TRANSITION_FROM_PIN),
            edge(3, TRANSITION_TO_PIN, 2, STATE_IN_PIN),
        ];
        st.doc.variables = vec![VarDecl {
            slug: "speed".into(),
            label: "Speed".into(),
            ty: PinType::Float,
            default: None,
            group: None,
        }];
        if let Some(r) = region {
            st.doc.regions.insert(3, r);
        }
        st.after_edit(&NodeRegistry::new());
        st
    }

    /// A one-comparison rule: var_get(speed) → compare → RESULT.
    fn speed_rule() -> GraphRegion {
        let mut get = node(1, VAR_GET_TYPE_ID, None);
        get.properties
            .insert(VAR_PROP.to_string(), PropValue::Str("speed".into()));
        let mut cmp = node(2, COMPARE_FLOAT, None);
        cmp.properties
            .insert("op".to_string(), PropValue::Enum("greater".into()));
        cmp.properties.insert("b".to_string(), PropValue::Float(3.0));
        GraphRegion {
            nodes: vec![node(0, ANIM_RULE_RESULT_TYPE_ID, None), get, cmp],
            edges: vec![
                edge(1, VAR_VALUE_PIN, 2, "a"),
                edge(2, "result", 0, RULE_RESULT_PIN),
            ],
        }
    }

    /// `InRegion` speaks region-local ids against the real region, creates
    /// the map entry on demand, and prunes an entry left empty — absent and
    /// empty both mean always-true, and only one spelling reaches a save.
    #[test]
    fn in_region_edits_round_trip_and_prune_empty_regions() {
        let mut doc = machine(None).doc;
        let before = doc.clone();
        let add = GraphEdit::InRegion {
            owner: 3,
            edit: Box::new(GraphEdit::AddNode(node(0, ANIM_RULE_RESULT_TYPE_ID, None))),
        };
        add.apply(&mut doc);
        assert_eq!(doc.regions.get(&3).unwrap().nodes.len(), 1);
        add.revert(&mut doc);
        assert!(!doc.regions.contains_key(&3), "an emptied region prunes");
        assert_eq!(doc, before, "undo restores the exact document");
    }

    /// Opening a peek is look-only: the projection mirrors the region plus
    /// the parent's variables, an empty rule gets an unrecorded RESULT seed,
    /// and neither opening nor closing dirties the document.
    #[test]
    fn opening_a_peek_projects_without_dirtying() {
        let reg = NodeRegistry::new();
        let mut st = machine(Some(speed_rule()));
        assert!(st.open_rule_scope(3, &reg));
        {
            let scope = st.rule_scope.as_ref().unwrap();
            assert_eq!(scope.child.doc.nodes.len(), 3);
            assert_eq!(scope.child.doc.variables.len(), 1, "parameters project");
            assert!(scope.seed.is_none(), "a real rule needs no seed");
            assert!(matches!(scope.child.domain, GraphDomain::AnimationRule { owner: 3 }));
        }
        // Only transitions descend.
        assert!(!st.open_rule_scope(1, &reg), "a state is not a rule scope");
        st.close_rule_scope(&reg);
        assert!(!st.dirty && st.stack.undo_len() == 0);

        // An always-true transition opens seeded, and looking costs nothing.
        let mut st = machine(None);
        let before = st.doc.clone();
        assert!(st.open_rule_scope(3, &reg));
        {
            let scope = st.rule_scope.as_ref().unwrap();
            assert_eq!(scope.child.doc.nodes[0].type_id, ANIM_RULE_RESULT_TYPE_ID);
            assert!(scope.seed.is_some());
        }
        st.close_rule_scope(&reg);
        assert_eq!(st.doc, before, "peeking into always-true changed nothing");
        assert!(!st.dirty);
    }

    /// Edits made in the peek drain onto the **parent** stack (seed recorded
    /// first), update the real region — the chip reads them live — and undo
    /// at the machine takes them back entry by entry to the exact bytes.
    #[test]
    fn peek_edits_are_one_history_with_the_machine() {
        let reg = NodeRegistry::new();
        let mut st = machine(None);
        let before = st.doc.clone();
        st.open_rule_scope(3, &reg);
        assert_eq!(transition_chip(&st.doc, 3).text(), "0.00s");

        // The author wires a parameter read straight into the seeded RESULT.
        {
            let scope = st.rule_scope.as_mut().unwrap();
            let mut get = node(1, VAR_GET_TYPE_ID, None);
            get.properties
                .insert(VAR_PROP.to_string(), PropValue::Str("speed".into()));
            scope.child.doc.nodes.push(get.clone());
            scope.child.stack.record(GraphEdit::AddNode(get));
            let e = edge(1, VAR_VALUE_PIN, 0, RULE_RESULT_PIN);
            scope.child.doc.edges.push(e.clone());
            scope.child.stack.record(GraphEdit::Connect(e));
        }
        st.drain_rule_scope(&reg);

        // Seed + two edits, all wrapped, all on the parent stack.
        assert_eq!(st.stack.undo_len(), 3);
        assert!(st.dirty);
        assert_eq!(
            st.rule_scope.as_ref().unwrap().child.stack.undo_len(),
            0,
            "the projection's stack hands everything to the parent"
        );
        let region = st.doc.regions.get(&3).expect("region materialized");
        assert_eq!((region.nodes.len(), region.edges.len()), (2, 1));
        // The chip reads the drained document — "Speed · 0.00s", wired.
        assert_eq!(transition_chip(&st.doc, 3).text(), "Speed \u{b7} 0.00s");

        // Undo at the machine unwinds the rule edit by edit; the projection
        // rebuilds each time and the scope survives.
        st.undo(&reg);
        st.undo(&reg);
        st.undo(&reg);
        assert_eq!(st.doc, before, "three undos return the exact document");
        assert!(st.rule_scope.is_some(), "the peek stays open, re-seeded");
        assert!(st.rule_scope.as_ref().unwrap().seed.is_some());
        assert!(!st.dirty);
    }

    /// Undoing past the transition's own creation closes the scope: a peek
    /// into a node that no longer exists has nothing to show.
    #[test]
    fn undo_that_removes_the_owner_closes_the_scope() {
        let reg = NodeRegistry::new();
        let mut st = machine(Some(speed_rule()));
        // The transition arrives as a recorded paste (as a duplicate would).
        let t = node(9, ANIM_TRANSITION_TYPE_ID, None);
        let paste = GraphEdit::Paste {
            nodes: vec![t.clone()],
            edges: vec![],
            regions: vec![(9, speed_rule())],
        };
        paste.apply(&mut st.doc);
        st.stack.record(paste);
        assert!(st.open_rule_scope(9, &reg));
        st.undo(&reg);
        assert!(st.rule_scope.is_none(), "the owner is gone, so is the peek");
        assert!(st.doc.node(9).is_none());
    }

    /// Variable edits made inside the peek pass through to the document
    /// unwrapped — declarations live on the machine, not in a region.
    #[test]
    fn variable_edits_pass_through_to_the_document() {
        let reg = NodeRegistry::new();
        let mut st = machine(None);
        st.open_rule_scope(3, &reg);
        {
            let scope = st.rule_scope.as_mut().unwrap();
            let decl = VarDecl {
                slug: "grounded".into(),
                label: "Grounded".into(),
                ty: PinType::Bool,
                default: None,
                group: None,
            };
            scope.child.doc.variables.push(decl.clone());
            scope.child.stack.record(GraphEdit::AddVariable(decl));
        }
        st.drain_rule_scope(&reg);
        assert_eq!(st.doc.variables.len(), 2, "the declaration is the machine's");
        assert_eq!(st.stack.undo_len(), 1);
        assert!(
            st.doc.regions.get(&3).is_none(),
            "a variable edit is not region content — no seed, no region"
        );
        st.undo(&reg);
        assert_eq!(st.doc.variables.len(), 1);
    }

    /// Placement gating holds through the clipboard: machine nodes refuse to
    /// paste into a rule canvas, rule nodes refuse to paste onto the machine,
    /// and a legal rule fragment pastes into the projection.
    #[test]
    fn foreign_fragments_refuse_across_animation_canvases() {
        let reg = NodeRegistry::new();
        // Rule nodes onto the machine canvas: refused whole.
        let mut st = machine(None);
        let rule_frag = GraphFragment {
            nodes: vec![node(0, COMPARE_FLOAT, None)],
            ..Default::default()
        };
        st.paste_fragment(&rule_frag, None, &anim_node_registry());
        assert!(st.doc.nodes.iter().all(|n| n.type_id != COMPARE_FLOAT));

        // Machine nodes into the rule projection: refused whole.
        st.open_rule_scope(3, &reg);
        let machine_frag = GraphFragment {
            nodes: vec![node(0, ANIM_STATE_TYPE_ID, Some("Rogue"))],
            ..Default::default()
        };
        {
            let scope = st.rule_scope.as_mut().unwrap();
            let n = scope.child.doc.nodes.len();
            scope.child.paste_fragment(&machine_frag, None, rule_scope_registry());
            assert_eq!(scope.child.doc.nodes.len(), n, "a State cannot enter a rule");
            // A rule fragment is welcome.
            scope.child.paste_fragment(&rule_frag, None, rule_scope_registry());
            assert_eq!(scope.child.doc.nodes.len(), n + 1);
        }
    }

    /// Find reaches inside embedded rules (spec story 22): region hits name
    /// the owning transition and the region-local node, and matching is over
    /// the same title/type/parameter text a user would type.
    #[test]
    fn find_indexes_nodes_inside_rules() {
        let st = machine(Some(speed_rule()));
        let find = |q: &str| FindState { query: q.into(), ..Default::default() };
        assert_eq!(region_find_matches(&st.doc, &find("speed")), vec![(3, 1)]);
        assert_eq!(region_find_matches(&st.doc, &find("compare")), vec![(3, 2)]);
        assert!(region_find_matches(&st.doc, &find("zzz")).is_empty());
    }

    /// A refusal naming a node inside a rule carries both anchors — the
    /// transition for the badge, the region-local node for the descend — and
    /// F8's landing selects that node inside the opened peek.
    #[test]
    fn rule_refusals_descend_into_the_peek() {
        let reg = NodeRegistry::new();
        // Poison the rule: a State node inside the region.
        let mut region = speed_rule();
        region.nodes.push(node(7, ANIM_STATE_TYPE_ID, None));
        let mut st = machine(Some(region));
        assert_eq!(st.domain_errors.len(), 1);
        let e = &st.domain_errors[0];
        assert_eq!((e.node, e.region_node), (Some(3), Some(7)), "{}", e.message);

        assert!(st.open_rule_scope_at(3, 7, &reg));
        let scope = st.rule_scope.as_ref().unwrap();
        assert!(scope.child.selection.contains(&7));
        assert!(scope.child.flash.is_some());
        // The projection's own compiler anchors the same refusal directly.
        assert_eq!(scope.child.domain_errors.len(), 1);
        assert_eq!(scope.child.domain_errors[0].node, Some(7));
    }
}

/// A blank editor state. Test-only, at module level so the sibling test
/// modules (the variables model, P6b) share one definition rather than
/// drifting copies of the constructor call.
#[cfg(test)]
/// Nested sub-state-machine tests (Task 41 ticket 09): the editor half —
/// descend targets, open-request chains, and anchored nested refusals.
#[cfg(test)]
mod nested_graph_tests {
    use super::*;
    use crate::engine::animation::graph::plan::{
        ANIM_ENTRY_TYPE_ID, ANIM_STATE_TYPE_ID, CLIP_PROP, GRAPH_PROP, STATE_IN_PIN,
        STATE_OUT_PIN,
    };
    use crate::engine::node_graph::{GraphRealm, GraphRegion};

    fn state_node(id: u64, props: &[(&str, PropValue)]) -> NodeInst {
        let mut n = NodeInst {
            id,
            type_id: ANIM_STATE_TYPE_ID.to_string(),
            type_version: 1,
            position: [0.0, 0.0],
            properties: Default::default(),
            subgraph: None,
            tint: None,
            title: Some(format!("S{id}")),
        };
        for (k, v) in props {
            n.properties.insert((*k).to_string(), v.clone());
        }
        n
    }

    /// Double-click / PageDown resolution: a nested state descends into its
    /// file (path normalized), a blend-tree region wins over the reference,
    /// and the script domain only ever answers through the `subgraph` field.
    #[test]
    fn animation_states_descend_into_their_nested_graph_file() {
        let mut st = test_state("graphs/host.animgraph");
        st.doc.nodes = vec![
            state_node(1, &[(GRAPH_PROP, PropValue::Asset("graphs\\loco.animgraph".into()))]),
            state_node(2, &[(CLIP_PROP, PropValue::Asset("anims/idle.anim".into()))]),
        ];
        assert_eq!(
            st.file_descend_target(1),
            Some("graphs/loco.animgraph".to_string()),
            "the reference descends, normalized"
        );
        assert_eq!(st.file_descend_target(2), None, "a clip leaf has no file to enter");

        st.select_only(1);
        assert_eq!(st.descend_target(), Some("graphs/loco.animgraph".to_string()));

        // A non-empty tree region takes the state over; the ignored graph
        // reference stops being a descend target too.
        st.doc.regions.insert(
            1,
            GraphRegion {
                nodes: vec![state_node(0, &[])],
                edges: vec![],
            },
        );
        assert_eq!(st.file_descend_target(1), None);

        // Script documents: only the subgraph field answers.
        let mut sc = test_state("graphs/t.graph");
        let mut n = state_node(1, &[(GRAPH_PROP, PropValue::Asset("graphs/x.animgraph".into()))]);
        n.subgraph = Some("lib/util.subgraph".into());
        sc.doc.nodes = vec![n];
        assert_eq!(sc.file_descend_target(1), Some("lib/util.subgraph".to_string()));
    }

    /// The request a descent raises carries the chain for the opened tab:
    /// this tab's ancestors plus this tab — what the breadcrumb renders.
    #[test]
    fn open_requests_extend_the_breadcrumb_chain() {
        let mut st = test_state("graphs/loco.animgraph");
        st.nav_back = vec!["graphs/character.animgraph".into()];
        let req = st.open_request("graphs/legs.animgraph".into());
        assert_eq!(req.path, "graphs/legs.animgraph");
        assert_eq!(
            req.back,
            vec!["graphs/character.animgraph".to_string(), "graphs/loco.animgraph".to_string()]
        );
        assert!(GraphOpenRequest::jump("a.graph".into()).back.is_empty());
    }

    /// A broken nested reference is a domain refusal anchored on the state
    /// that carries it (the file does not exist, so the editor's disk load
    /// fails exactly like the runtime's would).
    #[test]
    fn nested_reference_refusals_anchor_on_the_state() {
        let mut st = test_state("graphs/host.animgraph");
        st.doc.realm = GraphRealm::Client;
        let entry = NodeInst {
            id: 0,
            type_id: ANIM_ENTRY_TYPE_ID.to_string(),
            type_version: 1,
            position: [0.0, 0.0],
            properties: Default::default(),
            subgraph: None,
            tint: None,
            title: None,
        };
        st.doc.nodes = vec![
            entry,
            state_node(
                1,
                &[(GRAPH_PROP, PropValue::Asset("graphs/does-not-exist.animgraph".into()))],
            ),
        ];
        st.doc.edges = vec![Edge {
            from_node: 0,
            from_pin: STATE_OUT_PIN.to_string(),
            to_node: 1,
            to_pin: STATE_IN_PIN.to_string(),
        }];
        st.after_edit(&NodeRegistry::new());
        assert_eq!(st.domain_errors.len(), 1);
        let e = &st.domain_errors[0];
        assert_eq!(e.node, Some(1), "anchored on the referencing state");
        assert!(e.message.contains("could not be loaded"), "{}", e.message);
    }
}

pub(crate) fn test_state(path: &str) -> GraphEditorState {
    GraphEditorState::from_doc(
        path.to_string(),
        GraphDoc::default(),
        GraphDomain::of_path(path),
        &NodeRegistry::new(),
    )
}
