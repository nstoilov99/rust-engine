//! Editor extension points (Task 39.8 D6): plugin panels and settings pages.
//!
//! A minimal open seam, deliberately **not** a panel-system rewrite — Task
//! 45.5 restructures editor code later and this must not pre-empt it.
//!
//! ## Why a per-frame context struct rather than `&mut App`
//!
//! Both dispatch sites (`app.rs`: the docked `DockArea::show_in` closure and
//! the floating `fw.frame` closure) run *inside* closures where `App` has
//! already been split into ~25 disjoint field borrows — `world`, `sel`,
//! `hierarchy`, `inspector`, … . There is no `&mut App` to hand a panel, and
//! reassembling one would mean un-splitting those borrows, i.e. exactly the
//! "threading half of `App` through" that the plan's risk §6.2 warns against.
//!
//! So panels get a narrow [`PluginPanelCtx`] built at the dispatch site — the
//! same shape every existing panel already uses (`ConsolePanelCtx`,
//! `HierarchyPanelCtx`, …) and the same shape as Checkpoint #6's
//! `PassContext`. The trait stays object-safe, which is also what tier 2
//! needs.

use crusty_gui::context::Ui;
use crusty_gui::math::Rect;

use crate::engine::ecs::resources::{PlayMode, Resources};
use crate::engine::editor::project_config::ProjectConfig;

/// What a plugin panel may touch, assembled fresh each frame at the dispatch
/// site. Adding a field later is additive; nothing here is a god-object.
pub struct PluginPanelCtx<'a> {
    pub world: &'a mut hecs::World,
    pub resources: &'a mut Resources,
    pub play_mode: PlayMode,
}

/// A panel contributed by a plugin. The implementor *is* the panel's state.
pub trait PluginPanel: Send + Sync {
    /// Draw into `rect`. Called from both the docked and the floating host.
    fn draw(&mut self, ui: &mut Ui, rect: Rect, ctx: &mut PluginPanelCtx<'_>);
}

/// Produces a panel instance. A factory rather than a value so tier 2 (and a
/// future per-window panel state) has somewhere to hook in.
pub type PluginPanelFactory = Box<dyn Fn() -> Box<dyn PluginPanel> + Send + Sync>;

/// What a plugin settings page may touch.
///
/// `project` is the live [`ProjectConfig`] the Project Settings window is
/// editing — dirty tracking and Ctrl+S are the window's job, exactly as for
/// built-in rows. This is also what lets P6 build the Plugin Manager as a
/// settings page that edits `project.plugins`.
pub struct PluginSettingsCtx<'a> {
    pub project: &'a mut ProjectConfig,
}

/// A Project Settings page contributed by a plugin.
pub trait PluginSettingsPage: Send + Sync {
    fn draw(&mut self, ui: &mut Ui, ctx: &mut PluginSettingsCtx<'_>);
}

pub type PluginSettingsFactory = Box<dyn Fn() -> Box<dyn PluginSettingsPage> + Send + Sync>;

/// A live panel, owned by `PluginSet` after its plugin's stage was committed.
pub struct PluginPanelEntry {
    /// Which plugin registered it — the manager groups by this, and disabling
    /// the plugin removes the entry.
    pub plugin_id: String,
    /// Panel id; the dock tab is `plugin:<id>`.
    pub id: String,
    pub title: String,
    pub panel: Box<dyn PluginPanel>,
}

impl PluginPanelEntry {
    /// The dock tab id this panel answers to.
    pub fn tab_id(&self) -> String {
        format!("plugin:{}", self.id)
    }
}

/// A live settings page, owned by `PluginSet`.
pub struct PluginSettingsEntry {
    pub plugin_id: String,
    pub id: String,
    pub title: String,
    pub page: Box<dyn PluginSettingsPage>,
}
