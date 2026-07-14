//! Editor tab identifier enum shared between the (now-removed) old dock
//! layout and the crusty dock layout.
//!
//! `EditorTab` is used by `dock_crusty::tab_id` / `parse_tab` and by
//! `game_client::app` for opening and toggling tabs.
//! The old `EditorDockState`, `create_default_dock_state`,
//! `create_editor_dock_style`, and the `editor_layout.ron` LAYOUT_FILE
//! persistence path were removed; the crusty dock persists separately as
//! `editor_layout_crusty.ron` (see `dock_crusty`).

use super::scene_tab::SceneId;
use serde::{Deserialize, Serialize};

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
        }
    }

    /// Whether this tab can be closed
    pub fn closable(&self) -> bool {
        true
    }

    /// Unique string ID for this tab.
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
        }
    }
}
