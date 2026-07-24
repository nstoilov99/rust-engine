//! Crusty-gui dock layout for the editor.
//!
//! Sits on top of crusty-gui's `DockNode`/`DockState`: stable string tab ids
//! map 1:1 to [`EditorTab`], the layout persists as RON, and the default tree
//! is `Hierarchy | Viewport+bottom | Inspector`.

use super::dock_layout::EditorTab;
use super::play_settings::PlaySettings;
use super::scene_tab::{DormantScene, SceneId};
use crusty_gui::context::{Direction, Ui, UiOptions};
use crusty_gui::dock::{DockNode, DockState, Leaf};
use crusty_gui::id::Id;
pub use crusty_gui::math::{Pos2, Rect, Vec2};
use crusty_gui::widgets::{Button, Label};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

pub use crusty_gui::dock::{DockArea, DockResponse, ExternalDrag, TabBarSlot};

/// Layout file name stored in the current directory.
const LAYOUT_FILE: &str = "editor_layout_crusty.ron";

/// Stable string id for a tab, used as the crusty dock `TabId`.
pub fn tab_id(tab: &EditorTab) -> String {
    match tab {
        EditorTab::Viewport(id) => format!("viewport:{}", id.0),
        EditorTab::Hierarchy => "hierarchy".to_string(),
        EditorTab::Inspector => "inspector".to_string(),
        EditorTab::AssetBrowser => "assets".to_string(),
        EditorTab::Console => "console".to_string(),
        EditorTab::Profiler => "profiler".to_string(),
        EditorTab::InputSettings => "input_settings".to_string(),
        EditorTab::MeshEditor(key) => format!("mesh:{key}"),
        EditorTab::InputActionEditor(key) => format!("ia:{key}"),
        EditorTab::InputContextEditor(key) => format!("mc:{key}"),
    }
}

/// Inverse of [`tab_id`]. `None` for ids this build doesn't know.
pub fn parse_tab(id: &str) -> Option<EditorTab> {
    if let Some((kind, key)) = id.split_once(':') {
        return match kind {
            "viewport" => key.parse().ok().map(|n| EditorTab::Viewport(SceneId(n))),
            "mesh" => Some(EditorTab::MeshEditor(key.to_string())),
            "ia" => Some(EditorTab::InputActionEditor(key.to_string())),
            "mc" => Some(EditorTab::InputContextEditor(key.to_string())),
            _ => None,
        };
    }
    match id {
        "hierarchy" => Some(EditorTab::Hierarchy),
        "inspector" => Some(EditorTab::Inspector),
        "assets" => Some(EditorTab::AssetBrowser),
        "console" => Some(EditorTab::Console),
        "profiler" => Some(EditorTab::Profiler),
        "input_settings" => Some(EditorTab::InputSettings),
        _ => None,
    }
}

/// Crusty dock tree + per-frame drag/focus state, persisted together.
#[derive(Serialize, Deserialize)]
pub struct CrustyDockLayout {
    pub tree: DockNode,
    pub state: DockState,
    /// Editor net-play settings (M9.6) — ride along in the layout file so
    /// no extra config file appears; old layouts parse via the default.
    #[serde(default)]
    pub play_settings: PlaySettings,
}

impl Default for CrustyDockLayout {
    fn default() -> Self {
        Self::new()
    }
}

impl CrustyDockLayout {
    /// Default layout: Hierarchy (20%) | Viewport over Console/Profiler
    /// (75/25) | Inspector.
    pub fn new() -> Self {
        let center = DockNode::split_v(
            0.75,
            DockNode::leaf(tab_id(&EditorTab::Viewport(SceneId(0)))),
            DockNode::tabs([tab_id(&EditorTab::Console), tab_id(&EditorTab::Profiler)]),
        );
        let tree = DockNode::split_h(
            0.20,
            DockNode::leaf(tab_id(&EditorTab::Hierarchy)),
            DockNode::split_h(0.75, center, DockNode::leaf(tab_id(&EditorTab::Inspector))),
        );
        Self {
            tree,
            state: DockState::default(),
            play_settings: PlaySettings::default(),
        }
    }

    /// Reset the dock tree, keeping the play settings (they only share the
    /// file, not the "layout" concept).
    pub fn reset(&mut self) {
        let play_settings = self.play_settings.clone();
        *self = Self::new();
        self.play_settings = play_settings;
    }

    pub fn is_tab_open(&self, tab: &EditorTab) -> bool {
        self.tree.contains_tab(&tab_id(tab))
    }

    /// Open a tab (into the smallest leaf), or activate + focus it if present.
    pub fn open_tab(&mut self, tab: EditorTab) {
        let id = tab_id(&tab);
        if !self.tree.contains_tab(&id) {
            self.tree.add_tab(id.clone());
        }
        self.tree.activate_tab(&id);
        self.state.focused_tab = Some(id);
    }

    /// Open a `Viewport(_)` tab in the leaf that already hosts other viewport
    /// tabs (so all scenes share one tab strip), and focus it.
    pub fn open_viewport_tab(&mut self, scene_id: SceneId) {
        let id = tab_id(&EditorTab::Viewport(scene_id));
        if self.tree.contains_tab(&id) {
            self.tree.activate_tab(&id);
            self.state.focused_tab = Some(id);
            return;
        }
        if let Some(leaf) = viewport_leaf_mut(&mut self.tree) {
            leaf.tabs.push(id.clone());
            leaf.active = leaf.tabs.len() - 1;
            self.state.focused_tab = Some(id);
        } else {
            self.open_tab(EditorTab::Viewport(scene_id));
        }
    }

    pub fn remove_tab(&mut self, tab: &EditorTab) -> bool {
        self.tree.close_tab(&tab_id(tab))
    }

    /// The scene id of the focused viewport tab, if a viewport has focus.
    pub fn focused_viewport_id(&self) -> Option<SceneId> {
        match self.state.focused_tab.as_deref().and_then(parse_tab) {
            Some(EditorTab::Viewport(id)) => Some(id),
            _ => None,
        }
    }

    pub fn default_layout_path() -> PathBuf {
        PathBuf::from(LAYOUT_FILE)
    }

    pub fn save(&self, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        let ron_str = ron::ser::to_string_pretty(self, Default::default())?;
        fs::write(path, ron_str)?;
        Ok(())
    }

    pub fn save_to_default(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.save(&Self::default_layout_path())
    }

    pub fn load(path: &Path) -> Option<Self> {
        let content = fs::read_to_string(path).ok()?;
        ron::from_str(&content).ok()
    }

    pub fn load_or_default() -> Self {
        Self::load(&Self::default_layout_path()).unwrap_or_default()
    }
}

/// Display titles for every tab in the tree; viewport tabs show their scene
/// name. `extra` covers a tab currently torn off the tree (ghost drag) so its
/// card still shows the display title. The second return value is the set of
/// dirty tabs, shown as a warning dot in the tab strip.
pub fn tab_titles(
    tree: &DockNode,
    active_id: SceneId,
    active_name: &str,
    active_dirty: bool,
    dormant: &[DormantScene],
    extra: Option<&str>,
) -> (HashMap<String, String>, std::collections::HashSet<String>) {
    let mut ids = Vec::new();
    tree.collect_tabs(&mut ids);
    if let Some(extra) = extra {
        ids.push(extra.to_string());
    }
    let mut dirty_set = std::collections::HashSet::new();
    let titles = ids
        .into_iter()
        .map(|id| {
            let title = match parse_tab(&id) {
                Some(EditorTab::Viewport(vid)) => {
                    let (name, dirty) = if vid == active_id {
                        (active_name, active_dirty)
                    } else {
                        dormant
                            .iter()
                            .find(|d| d.id == vid)
                            .map(|d| (d.display_name.as_str(), d.dirty))
                            .unwrap_or(("(missing)", false))
                    };
                    let name = if name.is_empty() {
                        "Untitled Scene"
                    } else {
                        name
                    };
                    if dirty {
                        dirty_set.insert(id.clone());
                    }
                    name.to_string()
                }
                Some(tab) => tab.title_string(),
                None => id.clone(),
            };
            (id, title)
        })
        .collect();
    (titles, dirty_set)
}

/// This frame's [`ExternalDrag`] for an in-window ghost re-drag (a tab that
/// was torn off the dock and follows the cursor until dropped).
pub fn ghost_drag(ui: &Ui, tab_id: &str) -> ExternalDrag {
    let input = &ui.ctx().input;
    ExternalDrag {
        tab_id: tab_id.to_string(),
        pointer: input.pointer_pos.unwrap_or(Pos2::new(-10_000.0, -10_000.0)),
        grab: Vec2::new(60.0, 14.0),
        released: input.pointer_released,
        force: false,
        ghost: true,
    }
}

/// "+" new-scene button drawn after the last tab of `slot`'s bar. Returns
/// true on click.
pub fn new_tab_button(ui: &mut Ui, slot: &TabBarSlot) -> bool {
    let s = slot.bar_rect.height() - 8.0;
    let rect = Rect::from_min_size(
        Pos2::new(slot.end_x + 4.0, slot.bar_rect.min.y + 4.0),
        Vec2::splat(s),
    );
    let opts = UiOptions {
        padding: Vec2::ZERO,
        spacing: 0.0,
    };
    ui.run_at(
        rect,
        Direction::TopDown,
        Id::ROOT.with("dock_new_tab"),
        opts,
        |ui| Button::new("+").min_size(Vec2::splat(s)).show(ui).clicked,
    )
    .0
}

/// Dim placeholder body for tabs whose panel isn't ported yet.
pub fn placeholder_panel(ui: &mut Ui, text: &str) {
    let dim = ui.style().palette.text_secondary;
    Label::new(text).color(dim).show(ui);
}

/// Add `tab` to the least-crowded leaf that hosts no viewport, so panels
/// returning from float windows don't cover the scene view. Falls back to
/// `DockNode::add_tab` when every leaf holds a viewport.
pub fn redock_tab(tree: &mut DockNode, tab: impl Into<crusty_gui::dock::TabId>) {
    let tab = tab.into();
    if tree.contains_tab(&tab) {
        return;
    }
    fn smallest_non_viewport(node: &mut DockNode) -> Option<&mut Leaf> {
        match node {
            DockNode::Leaf(leaf) => {
                (!leaf.tabs.iter().any(|t| t.starts_with("viewport:"))).then_some(leaf)
            }
            DockNode::Split(s) => match (
                smallest_non_viewport(&mut s.first),
                smallest_non_viewport(&mut s.second),
            ) {
                (Some(a), Some(b)) => Some(if b.tabs.len() < a.tabs.len() { b } else { a }),
                (a, b) => a.or(b),
            },
        }
    }
    match smallest_non_viewport(tree) {
        Some(leaf) => {
            leaf.tabs.push(tab);
            leaf.active = leaf.tabs.len() - 1;
        }
        None => tree.add_tab(tab),
    }
}

/// The first leaf containing a viewport tab.
fn viewport_leaf_mut(node: &mut DockNode) -> Option<&mut Leaf> {
    match node {
        DockNode::Leaf(leaf) => leaf
            .tabs
            .iter()
            .any(|t| t.starts_with("viewport:"))
            .then_some(leaf),
        DockNode::Split(s) => {
            viewport_leaf_mut(&mut s.first).or_else(|| viewport_leaf_mut(&mut s.second))
        }
    }
}
