//! Crusty-gui dock layout for the editor.
//!
//! Sits on top of crusty-gui's `DockNode`/`DockState`: stable string tab ids
//! map 1:1 to [`EditorTab`], the layout persists as RON, and the default tree
//! is `Hierarchy | Viewport+bottom | Inspector`.

use super::dock_layout::EditorTab;
use super::play_settings::PlaySettings;
use super::scene_tab::{DormantScene, SceneId};
use crusty_gui::context::Ui;
use crusty_gui::dock::{DockNode, DockState, Leaf};
use crusty_gui::id::Id;
pub use crusty_gui::math::{Pos2, Rect, Vec2};
use crusty_gui::widgets::Label;
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
        EditorTab::GraphEditor(key) => format!("graph:{key}"),
        EditorTab::CurveEditor(key) => format!("curve:{key}"),
        EditorTab::BlendSpace(key) => format!("blendspace:{key}"),
        EditorTab::InputActionEditor(key) => format!("ia:{key}"),
        EditorTab::InputContextEditor(key) => format!("mc:{key}"),
        EditorTab::Plugin(id) => format!("plugin:{id}"),
    }
}

/// Every tab id in a dock tree, in traversal order.
///
/// The tree has `contains_tab` but no enumeration, and restoring a layout
/// needs one: a per-file editor tab read back from disk has no state behind
/// it until something walks the tree and opens its document.
pub fn collect_tabs(node: &DockNode, out: &mut Vec<String>) {
    match node {
        DockNode::Leaf(leaf) => out.extend(leaf.tabs.iter().map(|t| t.to_string())),
        DockNode::Split(split) => {
            collect_tabs(&split.first, out);
            collect_tabs(&split.second, out);
        }
    }
}

/// Inverse of [`tab_id`]. `None` for ids this build doesn't know.
pub fn parse_tab(id: &str) -> Option<EditorTab> {
    if let Some((kind, key)) = id.split_once(':') {
        return match kind {
            "viewport" => key.parse().ok().map(|n| EditorTab::Viewport(SceneId(n))),
            "mesh" => Some(EditorTab::MeshEditor(key.to_string())),
            "graph" => Some(EditorTab::GraphEditor(key.to_string())),
            "curve" => Some(EditorTab::CurveEditor(key.to_string())),
            "blendspace" => Some(EditorTab::BlendSpace(key.to_string())),
            "ia" => Some(EditorTab::InputActionEditor(key.to_string())),
            "mc" => Some(EditorTab::InputContextEditor(key.to_string())),
            // Always parses, even for a panel id nothing registers this
            // session: the tab must keep its place in the tree and degrade
            // visibly, not vanish and quietly reshape the user's layout.
            "plugin" => Some(EditorTab::Plugin(key.to_string())),
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
        // Atomic (39.8 §5.6) — the relaunch flow writes this then exits.
        super::atomic_file::atomic_write(path, &ron_str)?;
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
/// card still shows the display title. `editor_dirty` carries the tab ids of
/// per-file editors (mesh, graph, …) with unsaved changes — their dirty dot is
/// driven from the host's editor state rather than the scene registry. The
/// second return value is the set of dirty tabs, shown as a warning dot in the
/// tab strip.
/// Everything [`tab_titles`] needs besides the tree itself.
pub struct TabTitlesCtx<'a> {
    pub active_id: SceneId,
    pub active_name: &'a str,
    pub active_dirty: bool,
    pub dormant: &'a [DormantScene],
    /// A tab currently torn off the tree (ghost drag), so its card still
    /// shows a display title.
    pub extra: Option<&'a str>,
    /// Tab ids of per-file editors (mesh, graph, …) with unsaved changes.
    pub editor_dirty: &'a std::collections::HashSet<String>,
    /// Plugin panel *tab id* -> registered title. An id missing from this map
    /// is a panel no plugin registered this session; its tab keeps the id as
    /// its label, so the placeholder body has something to name.
    pub plugin_titles: &'a HashMap<String, String>,
}

pub fn tab_titles(
    tree: &DockNode,
    ctx: TabTitlesCtx<'_>,
) -> (HashMap<String, String>, std::collections::HashSet<String>) {
    let TabTitlesCtx {
        active_id,
        active_name,
        active_dirty,
        dormant,
        extra,
        editor_dirty,
        plugin_titles,
    } = ctx;
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
                Some(EditorTab::Plugin(_)) => plugin_titles
                    .get(&id)
                    .cloned()
                    .unwrap_or_else(|| id.clone()),
                Some(tab) => {
                    // Per-file editor tabs (mesh, graph) get their dirty dot
                    // from the host's editor state, keyed by tab id.
                    if editor_dirty.contains(&id) {
                        dirty_set.insert(id.clone());
                    }
                    tab.title_string()
                }
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
    // Plain 24px glyph, no button chrome (mockup); hover gets a quiet fill.
    let s = 24.0_f32.min(slot.bar_rect.height() - 4.0);
    let rect = Rect::from_min_size(
        Pos2::new(slot.end_x + 4.0, slot.bar_rect.center().y - s * 0.5),
        Vec2::splat(s),
    );
    let resp = ui.interact(Id::ROOT.with("dock_new_tab"), rect);
    let style = ui.style();
    if resp.hovered {
        ui.painter()
            .rect_filled(rect, style.rounding.small, style.palette.hover);
    }
    let color = if resp.hovered {
        style.palette.text
    } else {
        style.palette.text_disabled
    };
    let font = 14.0;
    let tw = ui.painter().measure_text("+", font, None);
    ui.painter().text(
        Pos2::new(
            rect.center().x - tw.x * 0.5,
            rect.center().y - font * 1.25 * 0.5,
        ),
        "+",
        font,
        color,
        None,
    );
    resp.clicked
}

/// Eye button at the right end of `slot`'s tab bar — hides the leaf's tab
/// strip (restored via the corner triangle). Returns true on click.
pub fn hide_tabs_button(
    ui: &mut Ui,
    slot: &TabBarSlot,
    icons: &HashMap<String, crusty_gui::paint::TextureId>,
) -> bool {
    let bar = slot.bar_rect;
    let size = Vec2::new(26.0, 24.0);
    let rect = Rect::from_min_size(
        Pos2::new(bar.max.x - 4.0 - size.x, bar.center().y - size.y * 0.5),
        size,
    );
    // Don't draw over the tabs when the strip is packed.
    if rect.min.x < slot.end_x + 4.0 {
        return false;
    }
    let resp = ui.interact(Id::ROOT.with(("dock_hide_tabs", &slot.anchor)), rect);
    let style = ui.style();
    if resp.hovered {
        ui.painter()
            .rect_filled(rect, style.rounding.small, style.palette.hover);
    }
    let tint = if resp.hovered {
        style.palette.text
    } else {
        style.palette.text_disabled
    };
    if let Some(&tex) = icons.get("visibility") {
        ui.ctx_mut().paint.push(crusty_gui::paint::PaintCmd::Image {
            rect: Rect::from_center_size(rect.center(), Vec2::splat(12.0)),
            uv_min: Pos2::new(0.0, 0.0),
            uv_max: Pos2::new(1.0, 1.0),
            tint,
            texture: tex,
        });
    }
    if resp.hovered {
        crusty_gui::widgets::show_tooltip_for(ui, rect, "Hide tabs");
    }
    resp.clicked
}

/// Ordered tabs of the leaf containing `tab`, and `tab`'s index in it.
pub fn leaf_tabs(tree: &DockNode, tab: &str) -> Option<(Vec<String>, usize)> {
    match tree {
        DockNode::Leaf(leaf) => leaf
            .tabs
            .iter()
            .position(|t| t == tab)
            .map(|i| (leaf.tabs.clone(), i)),
        DockNode::Split(s) => {
            leaf_tabs(&s.first, tab).or_else(|| leaf_tabs(&s.second, tab))
        }
    }
}

/// Actions committed from the tab context menu; the host applies them.
#[derive(Default)]
pub struct TabMenuActions {
    /// Hide the strip of the leaf containing this tab.
    pub hide_tabs: Option<String>,
    /// Tabs to close (each goes through the host's close/veto path).
    pub close: Vec<String>,
}

/// Right-click tab menu (mockup "Tab options"): Hide Tabs first, the close
/// family, then rows shipping disabled until Task 58.5. `open` is this
/// frame's dock report; `target` persists the subject tab across frames.
pub fn tab_context_menu(
    ui: &mut Ui,
    open: Option<(String, Pos2)>,
    target: &mut Option<String>,
    tree: &DockNode,
) -> TabMenuActions {
    if let Some((tab, _)) = &open {
        *target = Some(tab.clone());
    }
    let mut act = TabMenuActions::default();
    let Some(tab) = target.clone() else {
        return act;
    };
    let (tabs, idx) = leaf_tabs(tree, &tab).unwrap_or((vec![tab.clone()], 0));
    let open_at = open.map(|(_, p)| p);
    crusty_gui::widgets::context_menu_at(ui, "dock_tab_ctx", open_at, |ui| {
        ui.menu_group_header("Tab Options");
        if ui.menu_item("Hide Tabs") {
            act.hide_tabs = Some(tab.clone());
        }
        if ui.menu_item("Close") {
            act.close.push(tab.clone());
        }
        if ui.menu_item_enabled("Close Tabs to the Left", idx > 0) {
            act.close.extend(tabs[..idx].iter().cloned());
        }
        if ui.menu_item_enabled("Close Tabs to the Right", idx + 1 < tabs.len()) {
            act.close.extend(tabs[idx + 1..].iter().cloned());
        }
        if ui.menu_item_enabled("Close Other Tabs", tabs.len() > 1) {
            act.close
                .extend(tabs.iter().filter(|t| **t != tab).cloned());
        }
        ui.separator();
        // Ship disabled until Task 58.5 (multi-window viewport & tabs v2);
        // rows stay in place, dimmed, so the menu never changes shape.
        let _ = ui.menu_item_enabled("Unpin", false);
        let _ = ui.menu_item_enabled("Split Right", false);
        let _ = ui.menu_item_enabled("Move to New Window", false);
        ui.separator();
        let _ = ui.menu_item_enabled("Focus Viewport", false);
    });
    act
}

/// Every tab living in a leaf whose strip is hidden — panels use this to
/// make room for the corner triangle.
pub fn hidden_tabs(tree: &DockNode) -> std::collections::HashSet<String> {
    fn walk(node: &DockNode, out: &mut std::collections::HashSet<String>) {
        match node {
            DockNode::Leaf(leaf) => {
                if leaf.tabs_hidden {
                    out.extend(leaf.tabs.iter().cloned());
                }
            }
            DockNode::Split(s) => {
                walk(&s.first, out);
                walk(&s.second, out);
            }
        }
    }
    let mut out = std::collections::HashSet::new();
    walk(tree, &mut out);
    out
}

/// Dim placeholder body for tabs whose panel isn't ported yet.
pub fn placeholder_panel(ui: &mut Ui, text: &str) {
    let dim = ui.style().palette.text_secondary;
    Label::new(text).color(dim).show(ui);
}

/// Placeholder for a per-file editor tab whose document could not be loaded.
///
/// Names the asset and, when known, why: a tab that says only "not loaded"
/// leaves the user with nothing to act on, and these tabs survive a restart,
/// so the explanation has to live in the tab itself.
pub fn missing_document_panel(ui: &mut Ui, what: &str, key: &str, reason: Option<&str>) {
    let style = ui.style();
    Label::new(format!("{what} not loaded"))
        .color(style.palette.text_secondary)
        .show(ui);
    ui.add_space(style.spacing.item);
    Label::new(key).color(style.palette.text_mono).show(ui);
    if let Some(reason) = reason {
        ui.add_space(style.spacing.item);
        Label::new(reason)
            .color(super::theme::Palette::invariant_status().error)
            .show(ui);
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn per_file_tab_ids_round_trip() {
        for tab in [
            EditorTab::CurveEditor("curves/a.curve".into()),
            EditorTab::BlendSpace("blendspaces/locomotion.blendspace".into()),
            EditorTab::GraphEditor("graphs/x.animgraph".into()),
        ] {
            assert_eq!(parse_tab(&tab_id(&tab)), Some(tab));
        }
        assert_eq!(
            tab_id(&EditorTab::BlendSpace("b/l.blendspace".into())),
            "blendspace:b/l.blendspace"
        );
    }
}
