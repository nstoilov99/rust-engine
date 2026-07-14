//! Editor action enum and central dispatcher.
//!
//! All editor commands route through `EditorAction` → `dispatch_action`.
//! This avoids closure-vs-borrow-checker issues by keeping actions as plain data.

use crate::engine::editor::secondary_kind::SecondaryWindowKind;
use crate::engine::editor::theme::Density;
use crate::engine::rendering::rendering_3d::deferred::deferred_renderer::DebugView;

/// All possible editor actions. Each command palette entry, menu item, context
/// menu option, and keyboard shortcut maps to one of these variants.
///
/// Step 7 fills in the `dispatch_action` match arms for every variant.
/// Until then, unhandled variants log a warning.
#[derive(Clone, Debug)]
pub enum EditorAction {
    // --- File ---
    NewScene,
    OpenScene,
    SaveScene,
    SaveSceneAs(Option<std::path::PathBuf>),
    Quit,

    // --- Edit ---
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    Duplicate,
    Delete,

    // --- View ---
    ToggleHierarchy,
    ToggleInspector,
    ToggleAssetBrowser,
    ToggleConsole,
    ToggleProfiler,
    ResetLayoutToDefault,
    SwitchDensity(Density),
    ToggleDevShowcase,

    // --- Selection ---
    SelectAll,
    DeselectAll,
    FocusSelection,
    FindEntityByName,

    // --- Play ---
    TogglePlayMode,
    StepFrame,
    RestartFromEditState,

    // --- Render ---
    SwitchDebugView(DebugView),
    ToggleWireframe,
    ToggleGrid,
    ToggleGizmos,

    // --- Asset ---
    ReimportSelected,
    ShowInExplorer,
    RevealInAssetBrowser,

    // --- Engine ---
    ReloadAllShaders,
    OpenSettings,

    // --- Editor window lifecycle ---
    SaveAndCloseEditor {
        kind: SecondaryWindowKind,
        key: String,
    },
    DiscardAndCloseEditor {
        kind: SecondaryWindowKind,
        key: String,
    },
}

/// Central dispatcher. Single big match; handlers have full mutable access
/// to `EditorServices` and `GameWorld`.
///
/// Step 7 fills in every match arm. Steps 3-5 route context-menu actions
/// through this immediately; until Step 7 handles them, they log a warning.
pub fn dispatch_action(
    action: EditorAction,
    _services: &mut crate::engine::editor::services::EditorServices,
    _world: &mut crate::engine::ecs::game_world::GameWorld,
) {
    // Variants will be implemented as the steps that need them land.
    // Step 7 fills in every arm and removes this catch-all.
    let _ = (_services, _world);
    log::warn!("EditorAction not yet handled: {action:?}");
}
