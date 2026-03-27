//! Dock-based editor layout system
//!
//! Provides a dockable panel layout with drag-and-drop rearrangement,
//! tabbed panels, and persistent layouts.

use egui_dock::{DockState, NodeIndex};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Layout file name stored in the current directory
const LAYOUT_FILE: &str = "editor_layout.ron";

/// All available editor panels/tabs
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EditorTab {
    /// 3D viewport - renders the scene
    Viewport,
    /// Scene hierarchy tree view
    Hierarchy,
    /// Property inspector for selected entities
    Inspector,
    /// Asset browser with thumbnails (placeholder)
    AssetBrowser,
    /// Console/log output (placeholder)
    Console,
    /// Performance profiler
    Profiler,
}

impl EditorTab {
    /// Display name for the tab.
    pub fn title_string(&self) -> String {
        match self {
            EditorTab::Viewport => "Viewport".to_string(),
            EditorTab::Hierarchy => "Hierarchy".to_string(),
            EditorTab::Inspector => "Inspector".to_string(),
            EditorTab::AssetBrowser => "Assets".to_string(),
            EditorTab::Console => "Console".to_string(),
            EditorTab::Profiler => "Profiler".to_string(),
        }
    }

    /// Whether this tab can be closed
    pub fn closable(&self) -> bool {
        true
    }
}

/// Editor dock state wrapper
pub struct EditorDockState {
    pub dock_state: DockState<EditorTab>,
}

impl Default for EditorDockState {
    fn default() -> Self {
        Self::new()
    }
}

impl EditorDockState {
    /// Create a new dock state with default layout
    pub fn new() -> Self {
        Self {
            dock_state: create_default_dock_state(),
        }
    }

    /// Reset to default layout
    pub fn reset(&mut self) {
        self.dock_state = create_default_dock_state();
    }

    /// Check if a tab is currently open in the dock
    pub fn is_tab_open(&self, tab: &EditorTab) -> bool {
        for (_surface_index, node) in self.dock_state.iter_all_nodes() {
            if let egui_dock::Node::Leaf(leaf_data) = node {
                if leaf_data.tabs.contains(tab) {
                    return true;
                }
            }
        }
        false
    }

    /// Open a tab, or focus it if already present
    pub fn open_tab(&mut self, tab: EditorTab) {
        if let Some(location) = self.dock_state.find_tab(&tab) {
            self.dock_state.set_active_tab(location);
            self.dock_state
                .set_focused_node_and_surface((location.0, location.1));
            return;
        }
        self.dock_state.push_to_focused_leaf(tab);
    }

    /// Get the default layout file path
    pub fn default_layout_path() -> PathBuf {
        PathBuf::from(LAYOUT_FILE)
    }

    /// Save layout to file
    pub fn save(&self, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        let ron_str = ron::ser::to_string_pretty(&self.dock_state, Default::default())?;
        fs::write(path, ron_str)?;
        Ok(())
    }

    /// Save layout to the default file path
    pub fn save_to_default(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.save(&Self::default_layout_path())
    }

    /// Load layout from file, returning None if file doesn't exist or is invalid
    pub fn load(path: &Path) -> Option<Self> {
        let content = fs::read_to_string(path).ok()?;
        let dock_state: DockState<EditorTab> = ron::from_str(&content).ok()?;
        Some(EditorDockState { dock_state })
    }

    /// Load layout from the default file path, or create a new default layout
    pub fn load_or_default() -> Self {
        Self::load(&Self::default_layout_path()).unwrap_or_default()
    }
}

/// Create the default dock layout
///
/// Layout:
/// ```text
/// +------------+------------------+------------+
/// | Hierarchy  |    Viewport      | Inspector  |
/// |   (20%)    |     (60%)        |   (20%)    |
/// |            |                  |            |
/// +------------+------------------+------------+
/// |        Console / Profiler (tabs)           |
/// +--------------------------------------------+
/// ```
pub fn create_default_dock_state() -> DockState<EditorTab> {
    // Start with viewport in the center
    let mut dock_state = DockState::new(vec![EditorTab::Viewport]);

    // Split: Add hierarchy on the left (20% width)
    let [_hierarchy_node, center_node] = dock_state.main_surface_mut().split_left(
        NodeIndex::root(),
        0.20,
        vec![EditorTab::Hierarchy],
    );

    // Split: Add inspector on the right (25% of remaining = ~20% total)
    let [viewport_node, _inspector_node] =
        dock_state
            .main_surface_mut()
            .split_right(center_node, 0.75, vec![EditorTab::Inspector]);

    // Split: Add bottom panel with console and profiler as tabs below viewport (25% height)
    let [_viewport_final, _bottom_node] = dock_state.main_surface_mut().split_below(
        viewport_node,
        0.75,
        vec![EditorTab::Console, EditorTab::Profiler],
    );

    dock_state
}

/// Create a custom dock style matching the editor theme
pub fn create_editor_dock_style(ctx: &egui::Context) -> egui_dock::Style {
    let mut style = egui_dock::Style::from_egui(ctx.style().as_ref());

    // Customize tab bar appearance
    style.tab_bar.fill_tab_bar = true;
    style.tab_bar.height = 24.0;

    // Customize tab appearance (egui 0.31+ uses i8 for Margin)
    style.tab.tab_body.inner_margin = egui::Margin::same(4);

    // Customize separator appearance
    style.separator.width = 2.0;

    style
}
