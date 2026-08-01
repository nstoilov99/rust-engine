//! Per-asset graph UI state — the user-local sidecar.
//!
//! Pan/zoom and view bookmarks are *how one person is looking at* a graph,
//! not part of it: they belong beside `editor_prefs.ron`, keyed by the
//! asset's content-relative path, so two people editing one graph never fight
//! over whose viewport wins.
//!
//! One gitignored file for every graph, not one file per graph — the same
//! reasoning that kept `WirePrefs` in `editor_prefs.ron`: one save path, one
//! place to delete when something goes wrong.
//!
//! Breakpoints and watches will live here too, and are deliberately absent
//! until Task 45-A gives them meaning — a speculative column is a migration
//! waiting to happen.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crusty_gui::widgets::CanvasView;
use serde::{Deserialize, Serialize};

pub const GRAPH_STATE_FILE: &str = "editor_graph_state.ron";

/// One graph's remembered UI state.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GraphUiState {
    /// Last pan/zoom. `None` = never opened, so the tab frames its content.
    pub view: Option<StoredView>,
    /// The five bookmark slots.
    pub bookmarks: [Option<StoredView>; 5],
}

/// `CanvasView` in a serde-friendly shape (crusty's `Vec2` is not `Serialize`
/// in this build, and a stored file should not track a GUI type anyway).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct StoredView {
    pub pan_x: f32,
    pub pan_y: f32,
    pub zoom: f32,
}

impl StoredView {
    pub fn from_view(v: CanvasView) -> Self {
        Self { pan_x: v.pan.x, pan_y: v.pan.y, zoom: v.zoom }
    }

    pub fn to_view(self) -> CanvasView {
        CanvasView {
            pan: crusty_gui::math::Vec2::new(self.pan_x, self.pan_y),
            zoom: if self.zoom.is_finite() && self.zoom > 0.0 { self.zoom } else { 1.0 },
        }
    }
}

/// The whole sidecar: content-relative path → state.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GraphStateStore {
    pub graphs: BTreeMap<String, GraphUiState>,
}

impl GraphStateStore {
    pub fn path() -> PathBuf {
        PathBuf::from(GRAPH_STATE_FILE)
    }

    /// Load the sidecar, pruning entries whose asset no longer exists under
    /// `content_root`. A missing or unparsable file is simply an empty store:
    /// losing a remembered viewport is not worth an error path.
    pub fn load(content_root: &Path) -> Self {
        let mut store: Self = std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|t| ron::from_str(&t).ok())
            .unwrap_or_default();
        store.prune(content_root);
        store
    }

    /// Drop entries for assets that are gone, so the file cannot grow without
    /// bound across a project's lifetime.
    pub fn prune(&mut self, content_root: &Path) {
        self.graphs.retain(|rel, _| content_root.join(rel).exists());
    }

    pub fn save(&self) -> Result<(), Box<dyn std::error::Error>> {
        let text = ron::ser::to_string_pretty(self, Default::default())?;
        std::fs::write(Self::path(), text)?;
        Ok(())
    }

    pub fn view_for(&self, rel: &str) -> Option<CanvasView> {
        self.graphs.get(rel)?.view.map(StoredView::to_view)
    }

    pub fn bookmarks_for(&self, rel: &str) -> [Option<CanvasView>; 5] {
        let Some(e) = self.graphs.get(rel) else {
            return [None; 5];
        };
        let mut out = [None; 5];
        for (i, slot) in e.bookmarks.iter().enumerate() {
            out[i] = slot.map(|s| s.to_view());
        }
        out
    }

    /// Record a graph's view + bookmarks. Called on tab close and editor exit.
    pub fn store(&mut self, rel: &str, view: CanvasView, bookmarks: &[Option<CanvasView>; 5]) {
        let mut slots: [Option<StoredView>; 5] = [None; 5];
        for (i, b) in bookmarks.iter().enumerate() {
            slots[i] = b.map(StoredView::from_view);
        }
        self.graphs.insert(
            rel.to_string(),
            GraphUiState { view: Some(StoredView::from_view(view)), bookmarks: slots },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crusty_gui::math::Vec2;

    fn view(x: f32, z: f32) -> CanvasView {
        CanvasView { pan: Vec2::new(x, 0.0), zoom: z }
    }

    #[test]
    fn store_round_trips_views_and_bookmarks() {
        let mut s = GraphStateStore::default();
        let mut marks = [None; 5];
        marks[2] = Some(view(30.0, 0.5));
        s.store("graphs/a.graph", view(10.0, 2.0), &marks);

        let text = ron::ser::to_string_pretty(&s, Default::default()).unwrap();
        let back: GraphStateStore = ron::from_str(&text).unwrap();
        assert_eq!(back, s);

        let v = back.view_for("graphs/a.graph").unwrap();
        assert_eq!((v.pan.x, v.zoom), (10.0, 2.0));
        let b = back.bookmarks_for("graphs/a.graph");
        assert_eq!(b[2].map(|v| v.pan.x), Some(30.0));
        assert!(b[0].is_none());

        // An unknown graph is simply absent — the tab frames its content.
        assert!(back.view_for("graphs/never.graph").is_none());
        assert!(back.bookmarks_for("graphs/never.graph").iter().all(Option::is_none));
    }

    #[test]
    fn prune_drops_entries_whose_asset_is_gone() {
        let dir = std::env::temp_dir().join("rust_engine_graph_state_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("graphs")).unwrap();
        std::fs::write(dir.join("graphs/kept.graph"), "()").unwrap();

        let mut s = GraphStateStore::default();
        s.store("graphs/kept.graph", view(1.0, 1.0), &[None; 5]);
        s.store("graphs/deleted.graph", view(2.0, 1.0), &[None; 5]);
        assert_eq!(s.graphs.len(), 2);

        s.prune(&dir);
        assert_eq!(s.graphs.len(), 1, "an entry for a deleted asset is dropped");
        assert!(s.graphs.contains_key("graphs/kept.graph"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A zero or NaN zoom in a hand-edited file must not brick the canvas.
    #[test]
    fn broken_stored_zoom_falls_back() {
        let bad = StoredView { pan_x: 0.0, pan_y: 0.0, zoom: 0.0 };
        assert_eq!(bad.to_view().zoom, 1.0);
        let nan = StoredView { pan_x: 0.0, pan_y: 0.0, zoom: f32::NAN };
        assert_eq!(nan.to_view().zoom, 1.0);
    }
}
