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

use crusty_gui::widgets::CanvasView;

use crate::engine::node_graph::{
    load_graph, migrate_doc, save_graph, validate_doc, Edge, GraphDoc, GraphError, NodeInst,
    NodeRegistry, PropValue, SUBGRAPH_TYPE_ID,
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
    /// Nodes and their incident edges were removed together.
    RemoveNodes { nodes: Vec<NodeInst>, edges: Vec<Edge> },
    /// An edge was created.
    Connect(Edge),
    /// An edge was removed.
    Disconnect(Edge),
    /// Nodes moved by a fixed world-space delta (drag-coalesced).
    MoveNodes { ids: Vec<u64>, delta: [f32; 2] },
    /// A fragment (nodes + internal edges) was pasted/duplicated.
    Paste { nodes: Vec<NodeInst>, edges: Vec<Edge> },
}

impl GraphEdit {
    /// Redo direction — the edit as originally performed.
    fn apply(&self, doc: &mut GraphDoc) {
        match self {
            GraphEdit::AddNode(n) => doc.nodes.push(n.clone()),
            GraphEdit::RemoveNodes { nodes, edges } => {
                let ids: BTreeSet<u64> = nodes.iter().map(|n| n.id).collect();
                doc.nodes.retain(|n| !ids.contains(&n.id));
                doc.edges.retain(|e| !edges.contains(e));
            }
            GraphEdit::Connect(e) => doc.edges.push(e.clone()),
            GraphEdit::Disconnect(e) => doc.edges.retain(|x| x != e),
            GraphEdit::MoveNodes { ids, delta } => move_nodes(doc, ids, *delta),
            GraphEdit::Paste { nodes, edges } => {
                doc.nodes.extend(nodes.iter().cloned());
                doc.edges.extend(edges.iter().cloned());
            }
        }
    }

    /// Undo direction — the inverse of [`apply`](Self::apply).
    fn revert(&self, doc: &mut GraphDoc) {
        match self {
            GraphEdit::AddNode(n) => doc.nodes.retain(|x| x.id != n.id),
            GraphEdit::RemoveNodes { nodes, edges } => {
                doc.nodes.extend(nodes.iter().cloned());
                doc.edges.extend(edges.iter().cloned());
            }
            GraphEdit::Connect(e) => doc.edges.retain(|x| x != e),
            GraphEdit::Disconnect(e) => doc.edges.push(e.clone()),
            GraphEdit::MoveNodes { ids, delta } => {
                move_nodes(doc, ids, [-delta[0], -delta[1]])
            }
            GraphEdit::Paste { nodes, edges } => {
                let ids: BTreeSet<u64> = nodes.iter().map(|n| n.id).collect();
                doc.nodes.retain(|n| !ids.contains(&n.id));
                doc.edges.retain(|e| !edges.contains(e));
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
            GraphEdit::Disconnect(_) => "Disconnect".to_string(),
            GraphEdit::MoveNodes { ids, .. } => {
                format!("Move {} Node{}", ids.len(), plural(ids.len()))
            }
            GraphEdit::Paste { nodes, .. } => {
                format!("Paste {} Node{}", nodes.len(), plural(nodes.len()))
            }
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
    /// Doc-local undo/redo.
    pub stack: GraphEditStack,
    /// In-flight node drag (session-only).
    pub node_drag: Option<NodeDrag>,
    /// In-flight pin→pin connection drag (session-only).
    pub connect_drag: Option<ConnectDrag>,
    /// In-flight marquee box-select origin, world space (session-only).
    pub marquee: Option<[f32; 2]>,
    /// World position captured when the node-create menu opened.
    pub create_menu_world: Option<[f32; 2]>,
    /// Search text of the node-create menu.
    pub create_menu_search: String,
}

/// In-flight node drag: original positions so live movement is absolute
/// (no drift) and the net delta is recorded once on release.
pub struct NodeDrag {
    pub origin_world: [f32; 2],
    pub originals: Vec<(u64, [f32; 2])>,
}

/// In-flight connection drag from a source pin toward the pointer.
pub struct ConnectDrag {
    pub from_node: u64,
    pub from_pin: String,
    /// True if the grabbed pin is an output (wire flows out); false = input.
    pub from_output: bool,
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
            stack: GraphEditStack::new(),
            node_drag: None,
            connect_drag: None,
            marquee: None,
            create_menu_world: None,
            create_menu_search: String::new(),
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

    /// Drop selection entries whose node no longer exists (after undo/redo/
    /// delete).
    fn prune_selection(&mut self) {
        self.selection.retain(|id| self.doc.node(*id).is_some());
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
        };
        let id = node.id;
        self.doc.nodes.push(node.clone());
        self.selection.clear();
        self.selection.insert(id);
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
        };
        let id = node.id;
        self.doc.nodes.push(node.clone());
        self.selection.clear();
        self.selection.insert(id);
        self.commit(GraphEdit::AddNode(node), registry);
    }

    /// Delete the current selection and its incident edges.
    pub fn delete_selection(&mut self, registry: &NodeRegistry) {
        if self.selection.is_empty() {
            return;
        }
        let ids = self.selection.clone();
        let nodes: Vec<NodeInst> = self
            .doc
            .nodes
            .iter()
            .filter(|n| ids.contains(&n.id))
            .cloned()
            .collect();
        if nodes.is_empty() {
            return;
        }
        let edges: Vec<Edge> = self
            .doc
            .edges
            .iter()
            .filter(|e| ids.contains(&e.from_node) || ids.contains(&e.to_node))
            .cloned()
            .collect();
        let edit = GraphEdit::RemoveNodes { nodes, edges };
        edit.apply(&mut self.doc);
        self.selection.clear();
        self.commit(edit, registry);
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
        self.selection = nodes.iter().map(|n| n.id).collect();
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

    fn node(id: u64, pos: [f32; 2]) -> NodeInst {
        NodeInst {
            id,
            type_id: "test_add".to_string(),
            type_version: 1,
            position: pos,
            properties: std::collections::BTreeMap::new(),
            subgraph: None,
        }
    }

    fn edge(a: u64, b: u64) -> Edge {
        Edge {
            from_node: a,
            from_pin: "sum".to_string(),
            to_node: b,
            to_pin: "a".to_string(),
        }
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
                nodes: vec![node(1, [10.0, 10.0])],
                edges: vec![edge(0, 1)],
            },
            GraphEdit::Connect(Edge {
                from_node: 0,
                from_pin: "sum".to_string(),
                to_node: 1,
                to_pin: "b".to_string(),
            }),
            GraphEdit::Disconnect(edge(0, 1)),
            GraphEdit::MoveNodes { ids: vec![0, 1], delta: [3.0, -4.0] },
            GraphEdit::Paste {
                nodes: vec![node(7, [1.0, 1.0]), node(8, [2.0, 2.0])],
                edges: vec![edge(7, 8)],
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
        });
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
        });

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
