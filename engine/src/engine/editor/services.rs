//! Central editor services container
//!
//! `EditorServices` is the canonical owner of editor-only state that doesn't
//! belong to any single panel. Constructed once at editor startup in `app.rs`
//! and threaded through `EditorContext<'a>` to all panels.
//!
//! The old helpers (`load_icons`, `install_into_context`, `set_density`
//! taking a UI context reference) and the `AssetPreviewRegistry`/`HierarchyIcons`
//! sub-services were removed as part of the UI teardown. The crusty path
//! uploads its own textures on the render thread (see
//! `rendering::render_thread::load_hierarchy_icons`) and calls
//! `load_icons_crusty` here to get the persisted palette.

use std::sync::Arc;

use super::command_palette::{CommandPalette, CommandRegistry};
use super::dialogs::DialogStack;
use super::dirty_state::DirtyState;
use super::keymap::Keymap;
use super::theme::EditorTheme;
use super::toasts::ToastStack;
use super::widgets::IconRegistry;

/// Central owner of editor-wide services and state.
pub struct EditorServices {
    /// Canonical theme instance.
    pub theme: Arc<EditorTheme>,
    /// Canonical icon registry (palette only in the crusty path).
    pub icons: Arc<IconRegistry>,
    /// Global dirty-state tracker for editor-owned assets.
    pub dirty: DirtyState,
    /// Central modal stack for editor confirmation flows.
    pub dialogs: DialogStack,
    /// Toast notifications shown over the editor UI.
    pub toasts: ToastStack,
    /// Keyboard bindings as data — the compiled-in preset overlaid with the
    /// user's `keymap.ron`. Nothing dispatches through it yet; the consumers
    /// (menus, tooltips, preferences, the cheat sheet) land in B2.
    pub keymap: Keymap,
    /// Central command registry and palette UI state.
    pub command_registry: CommandRegistry,
    pub command_palette: CommandPalette,
}

impl EditorServices {
    /// Create a new `EditorServices` with the default dark theme.
    pub fn new() -> Self {
        Self {
            theme: Arc::new(EditorTheme::dark_default()),
            icons: Arc::new(IconRegistry::empty()),
            dirty: DirtyState::new(),
            dialogs: DialogStack::new(),
            toasts: ToastStack::new(),
            keymap: Keymap::default(),
            command_registry: CommandRegistry::new(),
            command_palette: CommandPalette::new(),
        }
    }

    /// Read `keymap.ron` over the compiled-in preset, surfacing any problem on
    /// the console *and* as a toast. A bad or conflicting file is never fatal:
    /// [`Keymap::load`] hands back working defaults, because an editor that
    /// refuses to open is a worse outcome than one with the stock bindings.
    pub fn load_keymap(&mut self) {
        let (keymap, problems) = Keymap::load();
        self.keymap = keymap;
        for p in &problems {
            println!("keymap: {p}");
        }
        if let Some(first) = problems.first() {
            let msg = if problems.len() > 1 {
                format!("{first} (+{} more — see the console)", problems.len() - 1)
            } else {
                first.clone()
            };
            self.toasts.warning(msg);
        }
    }

    /// Load palette-only icon state (no GPU textures here) for the crusty path.
    /// Crusty draws its own textures via the render thread's registry.
    pub fn load_icons_crusty(&mut self) {
        self.icons = Arc::new(IconRegistry::empty_with_default_palette());
    }

}

impl Default for EditorServices {
    fn default() -> Self {
        Self::new()
    }
}
