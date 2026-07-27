//! Graph editor per-document state (Task 40, P4).
//!
//! Holds one open `.graph`/`.subgraph` document, its doc-local validation
//! errors, and a dirty flag — mirroring how `mesh_editor` owns per-document
//! state. P4 is the tab/save/dirty shell only; the canvas UI, undo/redo, and
//! editing ops land in P5, extending this struct.

use crate::engine::node_graph::{
    load_graph, migrate_doc, save_graph, validate_doc, GraphDoc, GraphError, NodeRegistry,
};

/// State for one open graph document.
pub struct GraphEditorState {
    /// Content-relative path (forward slashes), e.g. `"graphs/foo.graph"`.
    /// This is the tab key; the on-disk file is `content/<path>`.
    pub path: String,
    /// The loaded (and migrated) document.
    pub doc: GraphDoc,
    /// Doc-local validation errors from the last load/save. Never fatal —
    /// an unknown node type or type-mismatched edge is surfaced, not rejected,
    /// so the doc round-trips without data loss.
    pub errors: Vec<GraphError>,
    /// Unsaved changes flag (saved-cursor discipline arrives with P5's undo).
    pub dirty: bool,
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
            dirty: false,
        })
    }

    /// Serialize and write the doc back to disk, clearing the dirty flag.
    pub fn save(&mut self, abs_path: &std::path::Path) -> Result<(), String> {
        save_graph(abs_path, &self.doc).map_err(|e| e.to_string())?;
        self.dirty = false;
        Ok(())
    }
}
