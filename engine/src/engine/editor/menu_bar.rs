//! Editor menu-bar action enum.
//!
//! The old menu-bar rendering fns were removed; the crusty analog lives in
//! `menu_bar_crusty`. This file now only exports the `MenuAction` enum shared
//! between both paths.

/// Actions that can be triggered from the menu bar
#[derive(Debug, Clone, PartialEq)]
pub enum MenuAction {
    /// No action
    None,
    /// Create a fresh empty scene
    NewScene,
    /// Save the current scene
    SaveScene,
    /// Cook the current scene's static collision to `content/collision/<scene>/`
    CookCollision,
    /// Exit the application
    Exit,
    /// Undo the last action
    Undo,
    /// Redo the last undone action
    Redo,
    /// Duplicate the selected entity
    Duplicate,
    /// Delete the selected entity
    Delete,
    /// Save the current layout
    SaveLayout,
    /// Reset dock layout to default
    ResetLayout,
    /// Replace the current world with the deterministic benchmark scene.
    LoadBenchmarkScene,
    /// Launch the standalone benchmark runner.
    RunBenchmark,
    /// Enter play mode (Edit -> Playing)
    Play,
    /// Pause play mode (Playing -> Paused)
    Pause,
    /// Resume play mode (Paused -> Playing)
    Resume,
    /// Stop play mode and restore snapshot (Playing|Paused -> Edit)
    Stop,
    /// Rebuild all registered shader pipelines from source.
    RebuildShaders,
    /// Toggle wireframe debug draw of cooked collision chunks.
    ToggleCollisionChunkDraw,
    /// Toggle the collision chunk-grid overlay.
    ToggleCollisionGridDraw,
    /// Toggle editor world streaming between full-world and camera rings.
    ToggleStreamAroundCamera,
    /// Toggle the Icon Inspector window (editor-debug only).
    #[cfg(feature = "editor-debug")]
    ToggleIconInspector,
    /// Toggle the Widget Showcase window (editor-debug only).
    #[cfg(feature = "editor-debug")]
    ToggleShowcase,
}
