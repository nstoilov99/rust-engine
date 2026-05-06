//! Central editor services container
//!
//! `EditorServices` is the canonical owner of editor-only state that doesn't
//! belong to any single panel. Constructed once at editor startup in `app.rs`
//! and threaded through `EditorContext<'a>` to all panels.

use std::sync::Arc;

use super::hierarchy_icons::HierarchyIcons;
use super::theme::EditorTheme;
use super::widgets::IconRegistry;

/// Central owner of editor-wide services and state.
///
/// Fields are added incrementally as Steps 2-14 land their functionality.
pub struct EditorServices {
    /// Canonical theme instance. Also installed into `egui::Context::data()`
    /// via `install_into_context()` so widgets can read via `ui.theme()`.
    pub theme: Arc<EditorTheme>,
    /// Canonical icon registry. Also installed into `egui::Context::data()`.
    pub icons: Arc<IconRegistry>,
    /// Hierarchy panel icon set (auto-discovered SVGs in
    /// `engine/icons/hierarchy/`). Also installed into `egui::Context::data()`
    /// so panels can fetch via `ui.data(...)` without threading a reference.
    pub hierarchy_icons: Arc<HierarchyIcons>,
}

impl EditorServices {
    /// Create a new `EditorServices` with the default dark theme.
    ///
    /// Icons are not loaded yet — call `load_icons()` after egui context
    /// is available, then `install_into_context()` to push into egui data.
    pub fn new() -> Self {
        Self {
            theme: Arc::new(EditorTheme::dark_default()),
            icons: Arc::new(IconRegistry::empty()),
            hierarchy_icons: Arc::new(HierarchyIcons::empty()),
        }
    }

    /// Load icons from disk. Must be called after egui context is available.
    pub fn load_icons(&mut self, ctx: &egui::Context) {
        self.icons = Arc::new(IconRegistry::load(ctx));
        self.hierarchy_icons = Arc::new(HierarchyIcons::load(ctx));
    }

    /// Apply the theme to egui's style/visuals and push the current theme +
    /// icons into `egui::Context::data()` so widgets can read via extension traits.
    ///
    /// Call after construction, after density changes, and after icon reloads.
    pub fn install_into_context(&self, ctx: &egui::Context) {
        // Apply palette, typography, spacing to egui Style/Visuals
        self.theme.apply_to(ctx);

        // Store Arc clones in egui's temp data for widget access
        ctx.data_mut(|d| {
            d.insert_temp::<Arc<EditorTheme>>(egui::Id::NULL, self.theme.clone());
            d.insert_temp::<Arc<IconRegistry>>(egui::Id::NULL, self.icons.clone());
            d.insert_temp::<Arc<HierarchyIcons>>(egui::Id::NULL, self.hierarchy_icons.clone());
        });
    }

    /// Switch density and re-apply.
    pub fn set_density(&mut self, density: super::theme::Density, ctx: &egui::Context) {
        self.theme = Arc::new(self.theme.with_density(density));
        self.install_into_context(ctx);
    }
}

impl Default for EditorServices {
    fn default() -> Self {
        Self::new()
    }
}
