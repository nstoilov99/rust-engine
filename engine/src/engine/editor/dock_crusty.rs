//! Crusty-gui dock layout for the editor.
//!
//! Sits on top of crusty-gui's `DockNode`/`DockState`: stable string tab ids
//! map 1:1 to [`EditorTab`], the layout persists as RON, and the default tree
//! is `Hierarchy | Viewport+bottom | Inspector`.
//!
//! # Layout profiles (per-document layouts)
//!
//! Document tabs (viewports, graphs, curves, blend spaces, meshes, input
//! editors) all live in one *document strip*; the panels around it belong to
//! a [`LayoutProfile`] chosen by the focused document's kind. Only the active
//! profile's tree is live (`tree`); every other profile is stored with the
//! marker tab [`DOCUMENTS_TAB`] standing in for the strip. Swapping profiles
//! ([`CrustyDockLayout::swap_profile`]) writes the live tree back in that
//! form, then fills the incoming tree's marker leaf with every document tab.
//! The marker itself never appears in the live tree.

use super::dock_layout::EditorTab;
use super::graph_editor::GraphDomain;
use super::play_settings::PlaySettings;
use super::scene_tab::{DormantScene, SceneId};
use crusty_gui::context::Ui;
use crusty_gui::dock::{DockNode, DockState, Leaf};
use crusty_gui::id::Id;
pub use crusty_gui::math::{Pos2, Rect, Vec2};
use crusty_gui::widgets::Label;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
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
        EditorTab::GraphDetails => "graph_details".to_string(),
        EditorTab::GraphVariables => "graph_variables".to_string(),
        EditorTab::AnimPreview => "anim_preview".to_string(),
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
        "graph_details" => Some(EditorTab::GraphDetails),
        "graph_variables" => Some(EditorTab::GraphVariables),
        "anim_preview" => Some(EditorTab::AnimPreview),
        _ => None,
    }
}

/// The set of side panels around the document strip, keyed by the focused
/// document's kind (doc-layouts spec).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
pub enum LayoutProfile {
    #[default]
    Scene,
    AnimGraph,
    ScriptGraph,
    BlendSpace,
    Curve,
    Mesh,
}

/// Marker tab id standing in for the document strip inside a *stored*
/// profile tree. Never rendered: the live tree has the real document tabs
/// where the marker was.
pub const DOCUMENTS_TAB: &str = "documents";

/// True for tabs that live in the document strip (every such tab has a
/// [`LayoutProfile`]); false for side panels.
pub fn is_document(tab: &EditorTab) -> bool {
    matches!(
        tab,
        EditorTab::Viewport(_)
            | EditorTab::MeshEditor(_)
            | EditorTab::GraphEditor(_)
            | EditorTab::CurveEditor(_)
            | EditorTab::BlendSpace(_)
            | EditorTab::InputActionEditor(_)
            | EditorTab::InputContextEditor(_)
    )
}

/// [`is_document`] on a tab id.
pub fn is_document_id(id: &str) -> bool {
    parse_tab(id).is_some_and(|t| is_document(&t))
}

/// Layout profile of a document tab; `None` for side panels. A graph's
/// profile follows its loaded domain; unknown/unloaded → `ScriptGraph`.
pub fn profile_of(tab: &EditorTab, graph_domain: Option<GraphDomain>) -> Option<LayoutProfile> {
    Some(match tab {
        EditorTab::Viewport(_)
        | EditorTab::InputActionEditor(_)
        | EditorTab::InputContextEditor(_) => LayoutProfile::Scene,
        EditorTab::MeshEditor(_) => LayoutProfile::Mesh,
        EditorTab::CurveEditor(_) => LayoutProfile::Curve,
        EditorTab::BlendSpace(_) => LayoutProfile::BlendSpace,
        EditorTab::GraphEditor(_) => match graph_domain {
            Some(d) if d.is_animation_family() => LayoutProfile::AnimGraph,
            _ => LayoutProfile::ScriptGraph,
        },
        _ => return None,
    })
}

/// A stored (inactive) profile: its tree with the [`DOCUMENTS_TAB`] marker
/// in place of the document strip, plus the dock state it had when it was
/// last live.
#[derive(Serialize, Deserialize)]
pub struct ProfileLayout {
    pub tree: DockNode,
    #[serde(default)]
    pub state: DockState,
}

/// Default tree of a profile, marker in place of the document strip.
pub fn default_tree(profile: LayoutProfile) -> DockNode {
    let docs = DockNode::leaf(DOCUMENTS_TAB);
    let leaf = |t: EditorTab| DockNode::leaf(tab_id(&t));
    let strip = |a: EditorTab, b: EditorTab| DockNode::tabs([tab_id(&a), tab_id(&b)]);
    match profile {
        // Hierarchy (20%) | documents over Console/Profiler (75/25) | Inspector.
        LayoutProfile::Scene => DockNode::split_h(
            0.20,
            leaf(EditorTab::Hierarchy),
            DockNode::split_h(
                0.75,
                DockNode::split_v(0.75, docs, strip(EditorTab::Console, EditorTab::Profiler)),
                leaf(EditorTab::Inspector),
            ),
        ),
        // Variables (18%) | documents over Assets/Console (57%) | Preview over
        // Details (25%, split evenly). Wider than the Scene Inspector (20%)
        // so the preview keeps a usable aspect.
        LayoutProfile::AnimGraph => DockNode::split_h(
            0.18,
            leaf(EditorTab::GraphVariables),
            DockNode::split_h(
                0.70,
                DockNode::split_v(0.75, docs, strip(EditorTab::AssetBrowser, EditorTab::Console)),
                DockNode::split_v(
                    0.5,
                    leaf(EditorTab::AnimPreview),
                    leaf(EditorTab::GraphDetails),
                ),
            ),
        ),
        // Variables (18%) | documents over Assets/Console (62%) | Details (20%,
        // the Scene Inspector's width).
        LayoutProfile::ScriptGraph => DockNode::split_h(
            0.18,
            leaf(EditorTab::GraphVariables),
            DockNode::split_h(
                0.75,
                DockNode::split_v(0.75, docs, strip(EditorTab::AssetBrowser, EditorTab::Console)),
                leaf(EditorTab::GraphDetails),
            ),
        ),
        // These tabs embed their own details/preview: documents over Assets/Console.
        LayoutProfile::BlendSpace | LayoutProfile::Curve | LayoutProfile::Mesh => {
            DockNode::split_v(0.75, docs, strip(EditorTab::AssetBrowser, EditorTab::Console))
        }
    }
}

/// Crusty dock tree + per-frame drag/focus state, persisted together.
///
/// File format v2: `tree`/`state` are the *active* profile's live copy (so a
/// v1 reader still finds a tree); the inactive profiles sit in `profiles`
/// with the [`DOCUMENTS_TAB`] marker. `profiles` never contains `active`.
/// A v1 file (no `version`/`profiles`) loads as the `Scene` profile.
#[derive(Serialize, Deserialize)]
pub struct CrustyDockLayout {
    /// 1 = single tree (pre-profiles), 2 = per-profile trees.
    #[serde(default = "v1")]
    pub version: u32,
    pub tree: DockNode,
    pub state: DockState,
    /// Editor net-play settings (M9.6) — ride along in the layout file so
    /// no extra config file appears; old layouts parse via the default.
    #[serde(default)]
    pub play_settings: PlaySettings,
    /// The profile `tree` belongs to.
    #[serde(default)]
    pub active: LayoutProfile,
    /// Stored trees of the inactive profiles.
    #[serde(default)]
    pub profiles: BTreeMap<LayoutProfile, ProfileLayout>,
}

fn v1() -> u32 {
    1
}

impl Default for CrustyDockLayout {
    fn default() -> Self {
        Self::new()
    }
}

impl CrustyDockLayout {
    /// Default layout: the `Scene` profile with the first scene's viewport
    /// in the document strip.
    pub fn new() -> Self {
        let mut tree = default_tree(LayoutProfile::Scene);
        attach_documents(&mut tree, &[tab_id(&EditorTab::Viewport(SceneId(0)))], None);
        Self {
            version: 2,
            tree,
            state: DockState::default(),
            play_settings: PlaySettings::default(),
            active: LayoutProfile::Scene,
            profiles: BTreeMap::new(),
        }
    }

    /// Reset the *active* profile to its default tree, keeping every
    /// document tab (and the focused one in front), the other profiles and
    /// the play settings (they only share the file, not the "layout" concept).
    pub fn reset(&mut self) {
        let docs = document_tabs(&self.tree);
        let focus = self.state.focused_tab.take().filter(|t| docs.contains(t));
        self.tree = default_tree(self.active);
        attach_documents(&mut self.tree, &docs, focus.as_deref());
        self.state = DockState::default();
        self.state.focused_tab = focus;
    }

    /// [`reset`](Self::reset) plus forget every stored profile, so each
    /// comes back as its default when next activated.
    pub fn reset_all(&mut self) {
        self.profiles.clear();
        self.reset();
    }

    /// Make `to` the active profile with `focus` (a document tab id) in
    /// front of the strip. The live tree is stored back into the outgoing
    /// profile with its strip reduced to the marker; the incoming tree
    /// (stored, or the default) gets every document tab — gathered from
    /// wherever they were docked — in its marker leaf. No-op if already active.
    pub fn swap_profile(&mut self, to: LayoutProfile, focus: &str) {
        if to == self.active {
            return;
        }
        let mut outgoing = std::mem::replace(&mut self.tree, DockNode::leaf(DOCUMENTS_TAB));
        let docs = detach_documents(&mut outgoing);
        let state = std::mem::take(&mut self.state);
        self.profiles
            .insert(self.active, ProfileLayout { tree: outgoing, state });

        let ProfileLayout { mut tree, state } = self.profiles.remove(&to).unwrap_or_else(|| {
            ProfileLayout {
                tree: default_tree(to),
                state: DockState::default(),
            }
        });
        // A stored tree without a marker (hand-edited file) can't host the
        // strip; the default can.
        if !attach_documents(&mut tree, &docs, Some(focus)) {
            tree = default_tree(to);
            attach_documents(&mut tree, &docs, Some(focus));
        }
        self.tree = tree;
        self.state = state;
        self.state.focused_tab = Some(focus.to_string());
        self.active = to;
    }

    /// The document strip's front tab when it is a document, else the
    /// strip's first document (a side panel may be docked in front).
    pub fn active_document(&self) -> Option<String> {
        let mut leaves = Vec::new();
        collect_leaves(&self.tree, &mut leaves);
        let leaf = leaves[documents_leaf_index(&self.tree)?];
        leaf.tabs
            .get(leaf.active)
            .filter(|t| is_document_id(t))
            .or_else(|| leaf.tabs.iter().find(|t| is_document_id(t)))
            .cloned()
    }

    pub fn is_tab_open(&self, tab: &EditorTab) -> bool {
        self.tree.contains_tab(&tab_id(tab))
    }

    /// Open a tab, or activate + focus it if present. Documents join the
    /// document strip; side panels go into the smallest leaf.
    pub fn open_tab(&mut self, tab: EditorTab) {
        let id = tab_id(&tab);
        if !self.tree.contains_tab(&id) {
            if is_document(&tab) {
                push_document(&mut self.tree, id.clone());
            } else {
                self.tree.add_tab(id.clone());
            }
        }
        self.tree.activate_tab(&id);
        self.state.focused_tab = Some(id);
    }

    /// Open a `Viewport(_)` tab in the document strip and focus it.
    pub fn open_viewport_tab(&mut self, scene_id: SceneId) {
        self.open_tab(EditorTab::Viewport(scene_id));
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

    /// Parse a layout file's text; a v1 file becomes the `Scene` profile
    /// with its tree, state and play settings untouched.
    pub fn from_ron(content: &str) -> Option<Self> {
        let mut layout: Self = ron::from_str(content).ok()?;
        layout.version = 2;
        // Invariants a hand-edited file may break: the active profile is
        // never also stored, and the marker never reaches the live tree.
        layout.profiles.remove(&layout.active);
        while layout.tree.close_tab(DOCUMENTS_TAB) {}
        Some(layout)
    }

    pub fn load(path: &Path) -> Option<Self> {
        Self::from_ron(&fs::read_to_string(path).ok()?)
    }

    pub fn load_or_default() -> Self {
        Self::load(&Self::default_layout_path()).unwrap_or_default()
    }
}

/// Every leaf in traversal order.
fn collect_leaves<'a>(node: &'a DockNode, out: &mut Vec<&'a Leaf>) {
    match node {
        DockNode::Leaf(leaf) => out.push(leaf),
        DockNode::Split(s) => {
            collect_leaves(&s.first, out);
            collect_leaves(&s.second, out);
        }
    }
}

/// The `index`-th leaf in traversal order.
fn leaf_mut<'a>(node: &'a mut DockNode, index: &mut usize) -> Option<&'a mut Leaf> {
    match node {
        DockNode::Leaf(leaf) => {
            if *index == 0 {
                Some(leaf)
            } else {
                *index -= 1;
                None
            }
        }
        DockNode::Split(s) => leaf_mut(&mut s.first, index).or_else(|| leaf_mut(&mut s.second, index)),
    }
}

/// The live tree's document strip: the leaf holding the most document tabs
/// (ties → first in traversal order). `None` when no document is docked.
fn documents_leaf_index(tree: &DockNode) -> Option<usize> {
    let mut leaves = Vec::new();
    collect_leaves(tree, &mut leaves);
    let mut best: Option<(usize, usize)> = None;
    for (i, leaf) in leaves.iter().enumerate() {
        let n = leaf.tabs.iter().filter(|t| is_document_id(t)).count();
        if n > 0 && best.is_none_or(|(_, m)| n > m) {
            best = Some((i, n));
        }
    }
    best.map(|(i, _)| i)
}

fn documents_leaf_mut(tree: &mut DockNode) -> Option<&mut Leaf> {
    let mut index = documents_leaf_index(tree)?;
    leaf_mut(tree, &mut index)
}

/// Every document tab in the live tree: the strip's own first (in strip
/// order), then strays docked elsewhere, in traversal order.
pub fn document_tabs(tree: &DockNode) -> Vec<String> {
    let mut leaves = Vec::new();
    collect_leaves(tree, &mut leaves);
    let strip = documents_leaf_index(tree);
    fn docs_of(leaf: &Leaf) -> Vec<String> {
        leaf.tabs.iter().filter(|t| is_document_id(t)).cloned().collect()
    }
    let mut out: Vec<String> = strip.map(|i| docs_of(leaves[i])).unwrap_or_default();
    for (i, leaf) in leaves.iter().enumerate() {
        if Some(i) != strip {
            out.extend(docs_of(leaf));
        }
    }
    out
}

/// Append `id` to the document strip (or the smallest leaf when no
/// document is docked yet), making it the strip's front tab.
fn push_document(tree: &mut DockNode, id: String) {
    match documents_leaf_mut(tree) {
        Some(leaf) => {
            leaf.tabs.push(id);
            leaf.active = leaf.tabs.len() - 1;
        }
        None => tree.add_tab(id),
    }
}

/// Reduce the live tree to its stored form: the marker replaces the strip
/// and every document tab is removed (empty leaves collapse). Returns the
/// document tabs in [`document_tabs`] order.
fn detach_documents(tree: &mut DockNode) -> Vec<String> {
    let docs = document_tabs(tree);
    match documents_leaf_mut(tree) {
        Some(leaf) => leaf.tabs.push(DOCUMENTS_TAB.to_string()),
        None => tree.add_tab(DOCUMENTS_TAB),
    }
    for doc in &docs {
        tree.close_tab(doc);
    }
    docs
}

/// Put `docs` where the marker sits, `focus` (else the first) in front.
/// Returns false — tree untouched — if there is no marker. With no
/// documents the marker leaf collapses, so no empty leaf ever renders.
fn attach_documents(tree: &mut DockNode, docs: &[String], focus: Option<&str>) -> bool {
    if docs.is_empty() {
        return tree.close_tab(DOCUMENTS_TAB);
    }
    let Some(leaf) = tab_leaf_mut(tree, DOCUMENTS_TAB) else {
        return false;
    };
    let at = leaf.tabs.iter().position(|t| t == DOCUMENTS_TAB).unwrap_or(0);
    leaf.tabs.splice(at..at + 1, docs.iter().cloned());
    let focus_at = focus.and_then(|f| docs.iter().position(|d| d == f)).unwrap_or(0);
    leaf.active = at + focus_at;
    true
}

/// The leaf containing `tab`.
fn tab_leaf_mut<'a>(node: &'a mut DockNode, tab: &str) -> Option<&'a mut Leaf> {
    match node {
        DockNode::Leaf(leaf) => leaf.tabs.iter().any(|t| t == tab).then_some(leaf),
        DockNode::Split(s) => {
            tab_leaf_mut(&mut s.first, tab).or_else(|| tab_leaf_mut(&mut s.second, tab))
        }
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

/// Re-dock a tab returning from a float window: documents rejoin the
/// document strip; panels go to the least-crowded leaf that hosts no
/// viewport, so they don't cover the scene view (falls back to
/// `DockNode::add_tab` when every leaf holds a viewport).
pub fn redock_tab(tree: &mut DockNode, tab: impl Into<crusty_gui::dock::TabId>) {
    let tab = tab.into();
    if tree.contains_tab(&tab) {
        return;
    }
    if is_document_id(&tab) {
        push_document(tree, tab);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn ron_of(tree: &DockNode) -> String {
        ron::to_string(tree).unwrap()
    }

    /// Scene layout with the viewport strip holding `viewport:0` + a graph.
    fn scene_with_graph() -> CrustyDockLayout {
        let mut layout = CrustyDockLayout::new();
        layout.open_tab(EditorTab::GraphEditor("graphs/a.animgraph".into()));
        layout
    }

    #[test]
    fn every_default_tree_has_one_marker() {
        for p in [
            LayoutProfile::Scene,
            LayoutProfile::AnimGraph,
            LayoutProfile::ScriptGraph,
            LayoutProfile::BlendSpace,
            LayoutProfile::Curve,
            LayoutProfile::Mesh,
        ] {
            let mut ids = Vec::new();
            default_tree(p).collect_tabs(&mut ids);
            assert_eq!(ids.iter().filter(|t| *t == DOCUMENTS_TAB).count(), 1, "{p:?}");
        }
    }

    /// The spec's default trees (doc-layouts ticket 04): shape, order and
    /// split ratios. Change deliberately — a stored profile never re-reads
    /// these; a profile without a stored tree (fresh file, first activation
    /// after a v1 migration, Reset) does.
    #[test]
    fn graph_defaults_pin_the_spec_trees() {
        use crusty_gui::dock::SplitDirection::{self, Horizontal as H, Vertical as V};
        fn split(n: &DockNode, dir: SplitDirection, ratio: f32) -> (&DockNode, &DockNode) {
            let DockNode::Split(s) = n else { panic!("expected a split, got {n:?}") };
            assert_eq!(s.direction, dir);
            assert!((s.ratio - ratio).abs() < 1e-6, "ratio {} != {ratio}", s.ratio);
            (&s.first, &s.second)
        }
        fn tabs(n: &DockNode) -> Vec<&str> {
            let DockNode::Leaf(l) = n else { panic!("expected a leaf, got {n:?}") };
            l.tabs.iter().map(String::as_str).collect()
        }
        let bottom = ["assets", "console"];

        // AnimGraph: Variables (18%) | docs over Assets/Console | Preview over Details (25%).
        let t = default_tree(LayoutProfile::AnimGraph);
        let (vars, rest) = split(&t, H, 0.18);
        assert_eq!(tabs(vars), ["graph_variables"]);
        let (center, right) = split(rest, H, 0.70);
        let (docs, below) = split(center, V, 0.75);
        assert_eq!(tabs(docs), [DOCUMENTS_TAB]);
        assert_eq!(tabs(below), bottom);
        let (preview, details) = split(right, V, 0.5);
        assert_eq!(tabs(preview), ["anim_preview"]);
        assert_eq!(tabs(details), ["graph_details"]);

        // ScriptGraph: Variables (18%) | docs over Assets/Console | Details (20%).
        let t = default_tree(LayoutProfile::ScriptGraph);
        let (vars, rest) = split(&t, H, 0.18);
        assert_eq!(tabs(vars), ["graph_variables"]);
        let (center, details) = split(rest, H, 0.75);
        let (docs, below) = split(center, V, 0.75);
        assert_eq!(tabs(docs), [DOCUMENTS_TAB]);
        assert_eq!(tabs(below), bottom);
        assert_eq!(tabs(details), ["graph_details"]);

        // Scene: Hierarchy (20%) | docs over Console/Profiler | Inspector (20%).
        let t = default_tree(LayoutProfile::Scene);
        let (hier, rest) = split(&t, H, 0.20);
        assert_eq!(tabs(hier), ["hierarchy"]);
        let (center, inspector) = split(rest, H, 0.75);
        let (docs, below) = split(center, V, 0.75);
        assert_eq!(tabs(docs), [DOCUMENTS_TAB]);
        assert_eq!(tabs(below), ["console", "profiler"]);
        assert_eq!(tabs(inspector), ["inspector"]);

        // Embedded-panel documents: docs over Assets/Console only.
        for p in [LayoutProfile::BlendSpace, LayoutProfile::Curve, LayoutProfile::Mesh] {
            let t = default_tree(p);
            let (docs, below) = split(&t, V, 0.75);
            assert_eq!(tabs(docs), [DOCUMENTS_TAB]);
            assert_eq!(tabs(below), bottom);
        }
    }

    #[test]
    fn open_tab_puts_documents_in_the_strip() {
        let layout = scene_with_graph();
        let (tabs, _) = leaf_tabs(&layout.tree, "viewport:0").unwrap();
        assert_eq!(tabs, ["viewport:0", "graph:graphs/a.animgraph"]);
        assert_eq!(layout.active_document().as_deref(), Some("graph:graphs/a.animgraph"));
        assert_eq!(layout.state.focused_tab.as_deref(), Some("graph:graphs/a.animgraph"));
    }

    #[test]
    fn swap_round_trips_both_profiles_and_never_renders_the_marker() {
        let mut layout = scene_with_graph();
        let graph = "graph:graphs/a.animgraph";
        // Baseline with the viewport in front: the swap back focuses it.
        layout.tree.activate_tab("viewport:0");
        let scene_tree = ron_of(&layout.tree);

        layout.swap_profile(LayoutProfile::AnimGraph, graph);
        assert_eq!(layout.active, LayoutProfile::AnimGraph);
        assert!(!layout.tree.contains_tab(DOCUMENTS_TAB));
        assert!(layout.profiles[&LayoutProfile::Scene].tree.contains_tab(DOCUMENTS_TAB));
        assert!(!layout.profiles.contains_key(&LayoutProfile::AnimGraph));
        assert!(layout.tree.contains_tab("graph_variables"));
        assert!(layout.tree.contains_tab("anim_preview"));
        assert!(!layout.tree.contains_tab("hierarchy"));
        // Both documents came along, the focused one in front.
        let (tabs, idx) = leaf_tabs(&layout.tree, graph).unwrap();
        assert_eq!(tabs, ["viewport:0", graph]);
        assert_eq!(idx, 1);
        assert_eq!(layout.active_document().as_deref(), Some(graph));
        assert_eq!(layout.state.focused_tab.as_deref(), Some(graph));
        let anim_tree = ron_of(&layout.tree);

        // Stored profiles hold no document tabs.
        assert!(document_tabs(&layout.profiles[&LayoutProfile::Scene].tree).is_empty());

        layout.swap_profile(LayoutProfile::Scene, "viewport:0");
        assert_eq!(layout.active, LayoutProfile::Scene);
        assert!(!layout.tree.contains_tab(DOCUMENTS_TAB));
        assert!(!layout.profiles.contains_key(&LayoutProfile::Scene));
        // Same tree as before, splits and all.
        assert_eq!(ron_of(&layout.tree), scene_tree);

        layout.swap_profile(LayoutProfile::AnimGraph, graph);
        assert_eq!(ron_of(&layout.tree), anim_tree);
    }

    #[test]
    fn swap_gathers_documents_docked_in_other_leaves() {
        let mut layout = scene_with_graph();
        // Dock a curve next to the Inspector, a mesh next to Hierarchy.
        tab_leaf_mut(&mut layout.tree, "inspector")
            .unwrap()
            .tabs
            .push("curve:c.curve".into());
        tab_leaf_mut(&mut layout.tree, "hierarchy")
            .unwrap()
            .tabs
            .push("mesh:m.glb".into());
        assert_eq!(
            document_tabs(&layout.tree),
            ["viewport:0", "graph:graphs/a.animgraph", "mesh:m.glb", "curve:c.curve"]
        );

        layout.swap_profile(LayoutProfile::Curve, "curve:c.curve");
        let (tabs, idx) = leaf_tabs(&layout.tree, "curve:c.curve").unwrap();
        assert_eq!(
            tabs,
            ["viewport:0", "graph:graphs/a.animgraph", "mesh:m.glb", "curve:c.curve"]
        );
        assert_eq!(idx, 3);
        // The stray leaves kept their panels and lost only the documents.
        let scene = &layout.profiles[&LayoutProfile::Scene].tree;
        assert_eq!(leaf_tabs(scene, "inspector").unwrap().0, ["inspector"]);
        assert_eq!(leaf_tabs(scene, "hierarchy").unwrap().0, ["hierarchy"]);
        assert_eq!(
            leaf_tabs(scene, DOCUMENTS_TAB).unwrap().0,
            [DOCUMENTS_TAB]
        );
    }

    #[test]
    fn stored_profile_without_marker_falls_back_to_default() {
        let mut layout = scene_with_graph();
        layout.profiles.insert(
            LayoutProfile::Mesh,
            ProfileLayout {
                tree: DockNode::leaf("console"),
                state: DockState::default(),
            },
        );
        layout.swap_profile(LayoutProfile::Mesh, "viewport:0");
        assert_eq!(ron_of(&layout.tree), {
            let mut t = default_tree(LayoutProfile::Mesh);
            attach_documents(
                &mut t,
                &["viewport:0".into(), "graph:graphs/a.animgraph".into()],
                Some("viewport:0"),
            );
            ron_of(&t)
        });
    }

    #[test]
    fn reset_keeps_documents_and_other_profiles() {
        let mut layout = scene_with_graph();
        layout.swap_profile(LayoutProfile::AnimGraph, "graph:graphs/a.animgraph");
        layout.tree.close_tab("graph_details");
        layout.reset();
        assert!(layout.tree.contains_tab("graph_details"));
        assert_eq!(
            document_tabs(&layout.tree),
            ["viewport:0", "graph:graphs/a.animgraph"]
        );
        assert!(layout.profiles.contains_key(&LayoutProfile::Scene));
        layout.reset_all();
        assert!(layout.profiles.is_empty());
        assert_eq!(layout.active, LayoutProfile::AnimGraph);
    }

    #[test]
    fn v1_file_loads_as_the_scene_profile_verbatim() {
        #[derive(Serialize)]
        struct V1 {
            tree: DockNode,
            state: DockState,
            play_settings: PlaySettings,
        }
        let mut tree = DockNode::split_h(
            0.3,
            DockNode::tabs(["hierarchy".into(), "graph:g.graph".into()]),
            DockNode::leaf("viewport:0"),
        );
        tree.activate_tab("graph:g.graph");
        let play_settings = PlaySettings {
            player_count: 3,
            ..PlaySettings::default()
        };
        let mut state = DockState::default();
        state.focused_tab = Some("graph:g.graph".into());
        let text = ron::to_string(&V1 {
            tree: tree.clone(),
            state,
            play_settings: play_settings.clone(),
        })
        .unwrap();
        assert!(!text.contains("version"));

        let layout = CrustyDockLayout::from_ron(&text).unwrap();
        assert_eq!(layout.version, 2);
        assert_eq!(layout.active, LayoutProfile::Scene);
        assert!(layout.profiles.is_empty());
        assert_eq!(ron_of(&layout.tree), ron_of(&tree));
        assert_eq!(layout.state.focused_tab.as_deref(), Some("graph:g.graph"));
        assert_eq!(layout.play_settings, play_settings);
    }

    #[test]
    fn v2_file_round_trips_and_drops_a_stray_marker() {
        let mut layout = scene_with_graph();
        layout.swap_profile(LayoutProfile::AnimGraph, "graph:graphs/a.animgraph");
        let text = ron::ser::to_string_pretty(&layout, Default::default()).unwrap();
        let back = CrustyDockLayout::from_ron(&text).unwrap();
        assert_eq!(back.version, 2);
        assert_eq!(back.active, LayoutProfile::AnimGraph);
        assert_eq!(ron_of(&back.tree), ron_of(&layout.tree));
        assert_eq!(
            ron_of(&back.profiles[&LayoutProfile::Scene].tree),
            ron_of(&layout.profiles[&LayoutProfile::Scene].tree)
        );

        // A marker that leaked into the live tree is dropped on load.
        layout.tree.add_tab(DOCUMENTS_TAB);
        let text = ron::to_string(&layout).unwrap();
        let back = CrustyDockLayout::from_ron(&text).unwrap();
        assert!(!back.tree.contains_tab(DOCUMENTS_TAB));
    }

    #[test]
    fn profiles_of_tabs() {
        let d = |t: &EditorTab| profile_of(t, None);
        assert_eq!(d(&EditorTab::Viewport(SceneId(1))), Some(LayoutProfile::Scene));
        assert_eq!(d(&EditorTab::InputActionEditor("a".into())), Some(LayoutProfile::Scene));
        assert_eq!(d(&EditorTab::MeshEditor("m".into())), Some(LayoutProfile::Mesh));
        assert_eq!(d(&EditorTab::CurveEditor("c".into())), Some(LayoutProfile::Curve));
        assert_eq!(d(&EditorTab::BlendSpace("b".into())), Some(LayoutProfile::BlendSpace));
        assert_eq!(d(&EditorTab::GraphEditor("g".into())), Some(LayoutProfile::ScriptGraph));
        assert_eq!(
            profile_of(&EditorTab::GraphEditor("g".into()), Some(GraphDomain::Animation)),
            Some(LayoutProfile::AnimGraph)
        );
        for side in [
            EditorTab::Hierarchy,
            EditorTab::Inspector,
            EditorTab::GraphDetails,
            EditorTab::AnimPreview,
            EditorTab::Plugin("p".into()),
        ] {
            assert_eq!(d(&side), None, "{side:?}");
            assert!(!is_document(&side));
        }
    }

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
