//! Dock-based editor layout system
//!
//! Provides a dockable panel layout with drag-and-drop rearrangement,
//! tabbed panels, and persistent layouts.

use super::scene_tab::SceneId;
use egui_dock::{DockState, NodeIndex};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Layout file name stored in the current directory
const LAYOUT_FILE: &str = "editor_layout.ron";

/// All available editor panels/tabs
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EditorTab {
    /// 3D viewport - renders a scene tab (carries its scene id).
    Viewport(SceneId),
    /// Scene hierarchy tree view
    Hierarchy,
    /// Property inspector for selected entities
    Inspector,
    /// Asset browser with thumbnails
    AssetBrowser,
    /// Console/log output
    Console,
    /// Performance profiler
    Profiler,
    /// Input settings / action map editor
    InputSettings,
    /// Per-file mesh editor (keyed by content-relative mesh path)
    MeshEditor(String),
    /// Per-file input action editor (keyed by file path)
    InputActionEditor(String),
    /// Per-file mapping context editor (keyed by file path)
    InputContextEditor(String),
    /// Per-file material editor (keyed by content-relative path)
    MaterialEditor(String),
    /// Per-file material instance editor (keyed by content-relative path)
    MaterialInstanceEditor(String),
    /// Per-file texture editor (keyed by content-relative path)
    TextureEditor(String),
    /// Per-file animation clip editor (keyed by content-relative path)
    AnimationClipEditor(String),
    /// Per-file animation graph editor (keyed by content-relative path)
    AnimationGraphEditor(String),
    /// Per-file material graph editor (keyed by content-relative path)
    MaterialGraphEditor(String),
    /// Per-file audio editor (keyed by content-relative path)
    AudioEditor(String),
    /// Per-file prefab editor (keyed by content-relative path)
    PrefabEditor(String),
}

impl EditorTab {
    /// Display name for the tab.
    pub fn title_string(&self) -> String {
        match self {
            EditorTab::Viewport(_) => "Viewport".to_string(),
            EditorTab::Hierarchy => "Hierarchy".to_string(),
            EditorTab::Inspector => "Inspector".to_string(),
            EditorTab::AssetBrowser => "Assets".to_string(),
            EditorTab::Console => "Console".to_string(),
            EditorTab::Profiler => "Profiler".to_string(),
            EditorTab::InputSettings => "Input Settings".to_string(),
            EditorTab::MeshEditor(key) => {
                let name = std::path::Path::new(key)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| key.clone());
                format!("Mesh \u{2014} {}", name)
            }
            EditorTab::InputActionEditor(key) => {
                let name = std::path::Path::new(key)
                    .file_stem()
                    .and_then(|s| std::path::Path::new(s).file_stem())
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| key.clone());
                format!("IA \u{2014} {}", name)
            }
            EditorTab::InputContextEditor(key) => {
                let name = std::path::Path::new(key)
                    .file_stem()
                    .and_then(|s| std::path::Path::new(s).file_stem())
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| key.clone());
                format!("MC \u{2014} {}", name)
            }
            EditorTab::MaterialEditor(key) => {
                let name = std::path::Path::new(key)
                    .file_stem()
                    .and_then(|s| std::path::Path::new(s).file_stem())
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| key.clone());
                format!("Material \u{2014} {}", name)
            }
            EditorTab::MaterialInstanceEditor(key) => {
                let name = std::path::Path::new(key)
                    .file_stem()
                    .and_then(|s| std::path::Path::new(s).file_stem())
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| key.clone());
                format!("MatInst \u{2014} {}", name)
            }
            EditorTab::TextureEditor(key) => {
                let name = std::path::Path::new(key)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| key.clone());
                format!("Texture \u{2014} {}", name)
            }
            EditorTab::AnimationClipEditor(key) => {
                let name = std::path::Path::new(key)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| key.clone());
                format!("Clip \u{2014} {}", name)
            }
            EditorTab::AnimationGraphEditor(key) => {
                let name = std::path::Path::new(key)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| key.clone());
                format!("AnimGraph \u{2014} {}", name)
            }
            EditorTab::MaterialGraphEditor(key) => {
                let name = std::path::Path::new(key)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| key.clone());
                format!("MatGraph \u{2014} {}", name)
            }
            EditorTab::AudioEditor(key) => {
                let name = std::path::Path::new(key)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| key.clone());
                format!("Audio \u{2014} {}", name)
            }
            EditorTab::PrefabEditor(key) => {
                let name = std::path::Path::new(key)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| key.clone());
                format!("Prefab \u{2014} {}", name)
            }
        }
    }

    /// Whether this tab can be closed
    pub fn closable(&self) -> bool {
        true
    }

    /// Unique string ID for this tab (used for egui ID generation).
    pub fn id_string(&self) -> String {
        match self {
            EditorTab::Viewport(id) => format!("tab_viewport_{}", id.0),
            EditorTab::Hierarchy => "tab_hierarchy".to_string(),
            EditorTab::Inspector => "tab_inspector".to_string(),
            EditorTab::AssetBrowser => "tab_assets".to_string(),
            EditorTab::Console => "tab_console".to_string(),
            EditorTab::Profiler => "tab_profiler".to_string(),
            EditorTab::InputSettings => "tab_input_settings".to_string(),
            EditorTab::MeshEditor(key) => format!("tab_mesh_{}", key),
            EditorTab::InputActionEditor(key) => format!("tab_ia_{}", key),
            EditorTab::InputContextEditor(key) => format!("tab_mc_{}", key),
            EditorTab::MaterialEditor(key) => format!("tab_mat_{}", key),
            EditorTab::MaterialInstanceEditor(key) => format!("tab_matinst_{}", key),
            EditorTab::TextureEditor(key) => format!("tab_tex_{}", key),
            EditorTab::AnimationClipEditor(key) => format!("tab_animclip_{}", key),
            EditorTab::AnimationGraphEditor(key) => format!("tab_animgraph_{}", key),
            EditorTab::MaterialGraphEditor(key) => format!("tab_matgraph_{}", key),
            EditorTab::AudioEditor(key) => format!("tab_audio_{}", key),
            EditorTab::PrefabEditor(key) => format!("tab_prefab_{}", key),
        }
    }

    /// Convert this tab to a `SecondaryWindowKind` + editor key for undocking.
    /// Returns `None` for Viewport (cannot be undocked).
    pub fn to_window_kind(&self) -> Option<(super::SecondaryWindowKind, String)> {
        use super::SecondaryWindowKind as K;
        match self {
            EditorTab::Viewport(_) => None,
            EditorTab::Hierarchy => Some((K::Hierarchy, String::new())),
            EditorTab::Inspector => Some((K::Inspector, String::new())),
            EditorTab::AssetBrowser => Some((K::AssetBrowser, String::new())),
            EditorTab::Console => Some((K::Console, String::new())),
            EditorTab::Profiler => Some((K::Profiler, String::new())),
            EditorTab::InputSettings => Some((K::InputSettings, String::new())),
            EditorTab::MeshEditor(key) => Some((K::Mesh, key.clone())),
            EditorTab::InputActionEditor(key) => Some((K::InputAction, key.clone())),
            EditorTab::InputContextEditor(key) => Some((K::InputContext, key.clone())),
            EditorTab::MaterialEditor(key) => Some((K::Material, key.clone())),
            EditorTab::MaterialInstanceEditor(key) => Some((K::MaterialInstance, key.clone())),
            EditorTab::TextureEditor(key) => Some((K::Texture, key.clone())),
            EditorTab::AnimationClipEditor(key) => Some((K::AnimationClip, key.clone())),
            EditorTab::AnimationGraphEditor(key) => Some((K::AnimationGraph, key.clone())),
            EditorTab::MaterialGraphEditor(key) => Some((K::MaterialGraph, key.clone())),
            EditorTab::AudioEditor(key) => Some((K::Audio, key.clone())),
            EditorTab::PrefabEditor(key) => Some((K::Prefab, key.clone())),
        }
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

    /// Open a `Viewport(_)` tab in the dock leaf that already hosts other viewport
    /// tabs (so all scenes share the same tab strip), and focus it.
    /// Falls back to `open_tab` if no leaf currently contains a viewport.
    pub fn open_viewport_tab(&mut self, scene_id: SceneId) {
        let target = self.dock_state.iter_all_nodes().find_map(|(surface, node)| {
            if let egui_dock::Node::Leaf(leaf) = node {
                if leaf
                    .tabs
                    .iter()
                    .any(|t| matches!(t, EditorTab::Viewport(_)))
                {
                    // Find this leaf's NodeIndex by enumerating the surface's tree.
                    return self.dock_state[surface]
                        .iter()
                        .enumerate()
                        .find_map(|(idx, n)| {
                            let ni = NodeIndex(idx);
                            if std::ptr::eq(n, node) {
                                Some((surface, ni))
                            } else if let egui_dock::Node::Leaf(other) = n {
                                if other.tabs.as_slice() == leaf.tabs.as_slice() {
                                    Some((surface, ni))
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        });
                }
            }
            None
        });

        let tab = EditorTab::Viewport(scene_id);
        if let Some((surface, node)) = target {
            self.dock_state.set_focused_node_and_surface((surface, node));
            self.dock_state.push_to_focused_leaf(tab);
        } else {
            self.open_tab(tab);
        }
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

    /// Remove a specific tab from the dock. Returns true if found and removed.
    pub fn remove_tab(&mut self, tab: &EditorTab) -> bool {
        if let Some(location) = self.dock_state.find_tab(tab) {
            self.dock_state.remove_tab(location);
            true
        } else {
            false
        }
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
    let mut dock_state = DockState::new(vec![EditorTab::Viewport(SceneId(0))]);

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

    // Tabs sized to content (Godot/Unity/Unreal style), not stretched across the bar.
    style.tab_bar.fill_tab_bar = false;
    style.tab_bar.height = 28.0;

    // Give tabs a Godot-like minimum width so labels aren't cramped.
    style.tab.minimum_width = Some(120.0);
    style.tab.spacing = 2.0;

    // Customize tab appearance (egui 0.31+ uses i8 for Margin)
    style.tab.tab_body.inner_margin = egui::Margin::same(4);

    // Customize separator appearance
    style.separator.width = 2.0;

    style
}
