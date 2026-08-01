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
use crusty_gui::widgets::CanvasView;

use crate::engine::node_graph::{
    endpoint_type, load_graph, migrate_doc, save_graph, validate_doc, CommentBox, Edge,
    GraphDoc, IfacePin,
    GraphError, GroupBox, NodeInst, NodeRegistry, PropValue, REROUTE_IN, REROUTE_OUT,
    REROUTE_TYPE_ID, SUBGRAPH_TYPE_ID,
};

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
    /// a deleted node dies with it, and comes back with it.
    RemoveNodes {
        nodes: Vec<(usize, NodeInst)>,
        edges: Vec<(usize, Edge)>,
        comments: Vec<(usize, CommentBox)>,
    },
    /// An edge was created.
    Connect(Edge),
    /// Edges were removed together (wire selection + Delete). Each carries
    /// its original index so undo restores the exact vec order — a graph must
    /// serialize byte-identically after an undo.
    Disconnect { edges: Vec<(usize, Edge)> },
    /// Nodes moved by a fixed world-space delta (drag-coalesced).
    MoveNodes { ids: Vec<u64>, delta: [f32; 2] },
    /// A fragment (nodes + internal edges) was pasted/duplicated.
    Paste { nodes: Vec<NodeInst>, edges: Vec<Edge> },
    /// An inline input constant changed (P2 canvas widgets). `None` on either
    /// side means "no stored property", so setting a first value and clearing
    /// one back to the descriptor default are both round-trippable.
    SetProperty {
        node: u64,
        key: String,
        old: Option<PropValue>,
        new: Option<PropValue>,
    },
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
    /// Redo direction — the edit as originally performed.
    fn apply(&self, doc: &mut GraphDoc) {
        match self {
            GraphEdit::AddNode(n) => doc.nodes.push(n.clone()),
            GraphEdit::RemoveNodes { nodes, edges, comments } => {
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
            }
            GraphEdit::Connect(e) => doc.edges.push(e.clone()),
            GraphEdit::Disconnect { edges } => {
                doc.edges.retain(|x| !edges.iter().any(|(_, e)| e == x))
            }
            GraphEdit::MoveNodes { ids, delta } => move_nodes(doc, ids, *delta),
            GraphEdit::Paste { nodes, edges } => {
                doc.nodes.extend(nodes.iter().cloned());
                doc.edges.extend(edges.iter().cloned());
            }
            GraphEdit::SetProperty { node, key, new, .. } => set_prop(doc, *node, key, new),
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
        }
    }

    /// Undo direction — the inverse of [`apply`](Self::apply).
    fn revert(&self, doc: &mut GraphDoc) {
        match self {
            GraphEdit::AddNode(n) => doc.nodes.retain(|x| x.id != n.id),
            GraphEdit::RemoveNodes { nodes, edges, comments } => {
                // Reinsert at original indices, ascending so each index still
                // refers to the correct slot once earlier ones are back.
                reinsert_indexed(&mut doc.nodes, nodes);
                reinsert_indexed(&mut doc.edges, edges);
                reinsert_indexed(&mut doc.comments, comments);
            }
            GraphEdit::Connect(e) => doc.edges.retain(|x| x != e),
            GraphEdit::Disconnect { edges } => reinsert_indexed(&mut doc.edges, edges),
            GraphEdit::MoveNodes { ids, delta } => {
                move_nodes(doc, ids, [-delta[0], -delta[1]])
            }
            GraphEdit::Paste { nodes, edges } => {
                let ids: BTreeSet<u64> = nodes.iter().map(|n| n.id).collect();
                doc.nodes.retain(|n| !ids.contains(&n.id));
                doc.edges.retain(|e| !edges.contains(e));
            }
            GraphEdit::SetProperty { node, key, old, .. } => set_prop(doc, *node, key, old),
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
        }
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

/// A copied slice of a graph: nodes plus the edges internal to them. Ids are
/// remapped on paste, so a fragment can be pasted into any document.
#[derive(Debug, Clone, Default)]
pub struct GraphFragment {
    pub nodes: Vec<NodeInst>,
    pub edges: Vec<Edge>,
}

impl GraphFragment {
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Produce a copy with fresh ids (starting at `first_id`) and positions
    /// offset by `offset`. Returns the remapped nodes + edges (edges keep only
    /// those whose both endpoints are in the fragment).
    fn instantiate(&self, first_id: u64, offset: [f32; 2]) -> (Vec<NodeInst>, Vec<Edge>) {
        use std::collections::BTreeMap;
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
        let edges = self
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
        (nodes, edges)
    }
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
}

/// How many view bookmarks a graph keeps.
pub const BOOKMARK_SLOTS: usize = 5;

/// The align & distribute operations offered for 3+ selected nodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlignMode {
    Left,
    Right,
    Top,
    Bottom,
    DistributeHorizontally,
    DistributeVertically,
}

impl AlignMode {
    pub const ALL: [AlignMode; 6] = [
        AlignMode::Left,
        AlignMode::Right,
        AlignMode::Top,
        AlignMode::Bottom,
        AlignMode::DistributeHorizontally,
        AlignMode::DistributeVertically,
    ];

    pub fn label(self) -> &'static str {
        match self {
            AlignMode::Left => "Align Left",
            AlignMode::Right => "Align Right",
            AlignMode::Top => "Align Top",
            AlignMode::Bottom => "Align Bottom",
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

/// In-flight node drag: original positions so live movement is absolute
/// (no drift) and the net delta is recorded once on release.
pub struct NodeDrag {
    pub origin_world: [f32; 2],
    pub originals: Vec<(u64, [f32; 2])>,
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
        let errors = validate_doc(&doc, registry);
        Ok(Self {
            path: content_rel_key.to_string(),
            doc,
            errors,
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
        })
    }

    /// Serialize and write the doc back to disk, clearing the dirty flag.
    pub fn save(&mut self, abs_path: &std::path::Path) -> Result<(), String> {
        save_graph(abs_path, &self.doc).map_err(|e| e.to_string())?;
        self.stack.mark_saved();
        self.dirty = false;
        self.last_saved_at = Some(Instant::now());
        Ok(())
    }

    /// Re-validate the doc and refresh the dirty flag. Call after every edit.
    pub fn after_edit(&mut self, registry: &NodeRegistry) {
        self.errors = validate_doc(&self.doc, registry);
        self.dirty = self.stack.is_dirty();
    }

    /// Record an already-applied edit, then re-validate.
    pub fn commit(&mut self, edit: GraphEdit, registry: &NodeRegistry) {
        self.stack.record(edit);
        self.after_edit(registry);
    }

    pub fn undo(&mut self, registry: &NodeRegistry) {
        if self.stack.undo(&mut self.doc).is_some() {
            self.prune_selection();
            self.after_edit(registry);
        }
    }

    pub fn redo(&mut self, registry: &NodeRegistry) {
        if self.stack.redo(&mut self.doc).is_some() {
            self.prune_selection();
            self.after_edit(registry);
        }
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
        // An in-flight drag holds pre-undo positions/indices; cancel it so the
        // next frame doesn't overwrite the undone state or commit a bogus move.
        self.cancel_interactions();
    }

    /// Add a comment box at `pos` (default size + placeholder text), select it.
    pub fn add_comment(&mut self, pos: [f32; 2], registry: &NodeRegistry) {
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
        
        tint: None,};
        let id = node.id;
        self.doc.nodes.push(node.clone());
        self.select_only(id);
        self.commit(GraphEdit::AddNode(node), registry);
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
        
        tint: None,};
        let id = node.id;
        self.doc.nodes.push(node.clone());
        self.select_only(id);
        self.commit(GraphEdit::AddNode(node), registry);
    }

    /// Delete the current selection: a selected comment or group frame (frame
    /// only — member nodes stay), else the selected nodes and their edges.
    pub fn delete_selection(&mut self, registry: &NodeRegistry) {
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
        let edit = GraphEdit::RemoveNodes { nodes, edges, comments };
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
        let doomed = std::mem::take(&mut self.selected_edges);
        let edges: Vec<(usize, Edge)> = self
            .doc
            .edges
            .iter()
            .enumerate()
            .filter(|(_, e)| doomed.contains(e))
            .map(|(i, e)| (i, e.clone()))
            .collect();
        if edges.is_empty() {
            return false;
        }
        let n = edges.len();
        let edit = GraphEdit::Disconnect { edges };
        edit.apply(&mut self.doc);
        self.commit(edit, registry);
        // The toast system lands with the rest of the break gestures; until
        // then the count still gets reported.
        println!("graph: broke {n} link{}", if n == 1 { "" } else { "s" });
        true
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
        let Some(index) = self.doc.edges.iter().position(|e| e == edge) else {
            return;
        };
        let id = self.doc.next_node_id();
        let node = NodeInst {
            id,
            type_id: REROUTE_TYPE_ID.to_string(),
            type_version: 1,
            position: pos,
            properties: Default::default(),
            subgraph: None,
            tint: None,
        };
        let edit = GraphEdit::Composite {
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
        };
        edit.apply(&mut self.doc);
        self.select_only(id);
        self.commit(edit, registry);
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
        if rects.len() < 3 {
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
        // replaced by the interface declaration.
        let sub = GraphDoc {
            realm: self.doc.realm,
            nodes: nodes.clone(),
            edges: self
                .doc
                .edges
                .iter()
                .filter(|e| inside.contains(&e.from_node) && inside.contains(&e.to_node))
                .cloned()
                .collect(),
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
            },
            GraphEdit::AddNode(NodeInst {
                id,
                type_id: SUBGRAPH_TYPE_ID.to_string(),
                type_version: 1,
                position: anchor,
                properties: Default::default(),
                subgraph: Some(rel.clone()),
                tint: None,
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
                // An unregistered type is not evidence of uselessness.
                registry.get(&n.type_id).map(|d| !d.pure).unwrap_or(true)
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
            edits: vec![GraphEdit::RemoveNodes { nodes, edges, comments }],
        };
        edit.apply(&mut self.doc);
        self.selection.retain(|id| !set.contains(id));
        self.commit(edit, registry);
    }

    /// Cancel any in-flight drag / inline edit — indices they hold become
    /// invalid on structural edits and on undo/redo.
    pub fn cancel_interactions(&mut self) {
        self.node_drag = None;
        self.annotation_drag = None;
        self.connect_drag = None;
        self.marquee = None;
        self.marquee_mode = MarqueeMode::Replace;
        self.editing = None;
        self.annotation_resize = None;
        // Dropped, not flushed: the value it references may already be gone.
        self.prop_edit = None;
    }

    /// Copy the selection into `clipboard` (nodes + internal edges).
    pub fn copy_selection(&self, clipboard: &mut Option<GraphFragment>) {
        if let Some(frag) = self.selection_fragment() {
            *clipboard = Some(frag);
        }
    }

    /// Build a fragment from the current selection, or `None` if empty.
    fn selection_fragment(&self) -> Option<GraphFragment> {
        if self.selection.is_empty() {
            return None;
        }
        let nodes: Vec<NodeInst> = self
            .doc
            .nodes
            .iter()
            .filter(|n| self.selection.contains(&n.id))
            .cloned()
            .collect();
        if nodes.is_empty() {
            return None;
        }
        let edges: Vec<Edge> = self
            .doc
            .edges
            .iter()
            .filter(|e| {
                self.selection.contains(&e.from_node) && self.selection.contains(&e.to_node)
            })
            .cloned()
            .collect();
        Some(GraphFragment { nodes, edges })
    }

    /// Paste `clipboard` at a fixed offset, selecting the new nodes.
    pub fn paste_clipboard(
        &mut self,
        clipboard: &Option<GraphFragment>,
        registry: &NodeRegistry,
    ) {
        if let Some(frag) = clipboard {
            self.paste_fragment(frag, registry);
        }
    }

    /// Duplicate the selection in place (does not touch the shared clipboard).
    pub fn duplicate_selection(&mut self, registry: &NodeRegistry) {
        if let Some(frag) = self.selection_fragment() {
            self.paste_fragment(&frag, registry);
        }
    }

    const PASTE_OFFSET: [f32; 2] = [30.0, 30.0];

    fn paste_fragment(&mut self, frag: &GraphFragment, registry: &NodeRegistry) {
        if frag.is_empty() {
            return;
        }
        let (nodes, edges) = frag.instantiate(self.doc.next_node_id(), Self::PASTE_OFFSET);
        let edit = GraphEdit::Paste { nodes: nodes.clone(), edges: edges.clone() };
        edit.apply(&mut self.doc);
        // Move selection to the pasted nodes; clear any annotation selection so
        // a following Delete hits the pasted nodes, not an off-screen comment.
        self.clear_selection();
        self.selection = nodes.iter().map(|n| n.id).collect();
        self.primary = nodes.last().map(|n| n.id);
        self.commit(edit, registry);
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
        PropValue::Vec2(a) => format!("({}, {})", a[0], a[1]),
        PropValue::Vec3(a) => format!("({}, {}, {})", a[0], a[1], a[2]),
        PropValue::Vec4(a) => format!("({}, {}, {}, {})", a[0], a[1], a[2], a[3]),
        PropValue::Color(a) => format!("#{:.2},{:.2},{:.2},{:.2}", a[0], a[1], a[2], a[3]),
        PropValue::Bool(b) => b.to_string(),
        PropValue::Enum(s) => s.clone(),
        PropValue::Asset(s) => s.clone(),
        PropValue::Raw(s) => s.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::node_graph::doc::GraphDoc;
    use crate::engine::node_graph::NodeDescriptor;

    fn node(id: u64, pos: [f32; 2]) -> NodeInst {
        NodeInst {
            id,
            type_id: "test_add".to_string(),
            type_version: 1,
            position: pos,
            properties: std::collections::BTreeMap::new(),
            subgraph: None,
        
        tint: None,}
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
            tint: None,
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

        // Fewer than three is not a distribution.
        st.align_nodes(&rects[..2], AlignMode::Left, &reg);
        assert!(!st.stack.can_undo());
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
            tint: None,
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

        // Asset: internal edge kept, interface derived from the cut edges.
        let written =
            crate::engine::node_graph::load_graph(&dir.join(&rel)).expect("reload");
        assert_eq!(written.nodes.len(), 2);
        assert_eq!(written.edges.len(), 1, "only the internal edge came along");
        assert_eq!(written.inputs.len(), 1);
        assert_eq!(written.outputs.len(), 1);
        assert_eq!(written.inputs[0].slug, "sum", "input slug comes from its source pin");
        assert_eq!(written.outputs[0].slug, "sum");

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
        GraphEditorState {
            path: "t.graph".into(),
            doc: GraphDoc::default(),
            errors: vec![],
            ref_errors: vec![],
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
        }
    }

    /// Regression (finding 3): undo/redo cancels a live drag so the next frame
    /// can't overwrite the undone state or commit a bogus delta.
    #[test]
    fn undo_cancels_live_drag() {
        let reg = NodeRegistry::new();
        let mut st = bare_state();
        st.doc.nodes = vec![node(0, [0.0, 0.0])];
        st.doc.nodes[0].position = [50.0, 0.0];
        st.stack.record(GraphEdit::MoveNodes { ids: vec![0], delta: [50.0, 0.0] });
        st.node_drag = Some(NodeDrag {
            origin_world: [0.0, 0.0],
            originals: vec![(0, [0.0, 0.0])],
            anchored: Vec::new(),
        });
        st.undo(&reg);
        assert!(st.node_drag.is_none(), "undo must cancel the live node drag");
        assert_eq!(st.doc.nodes[0].position, [0.0, 0.0]);
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
        let frag = GraphFragment { nodes: vec![node(5, [0.0, 0.0])], edges: vec![] };
        st.paste_clipboard(&Some(frag), &reg);
        assert!(st.sel_comment.is_none(), "paste must clear annotation selection");
        assert!(!st.selection.is_empty(), "pasted nodes become the selection");
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
        
        tint: None,});
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
        
        tint: None,});

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
}
