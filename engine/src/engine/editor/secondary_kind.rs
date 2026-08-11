//! Editor tab / editor-window kind tag.
//!
//! Formerly lived in `secondary_window.rs` (deleted along with the old
//! secondary-OS-window system). The enum survives because `EditorAction`
//! and per-editor save/dirty state key off it.
//!
//! The variants historically named individual secondary-window kinds; some
//! (Hierarchy, Inspector, AssetBrowser, Console, Profiler, InputSettings)
//! were only ever meaningful as OS windows and are now unused. Kept for the
//! `EditorAction` payload's stable shape.

/// Which editor slice a per-file save/close request targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SecondaryWindowKind {
    Mesh,
    Graph,
    /// A `.curve` document editor (45-A P8b).
    Curve,
    Hierarchy,
    Inspector,
    AssetBrowser,
    Console,
    Profiler,
    InputSettings,
    InputAction,
    InputContext,
    /// A plugin-contributed panel (Task 39.8 D6), keyed by panel id.
    Plugin,
    #[cfg(feature = "editor-debug")]
    IconInspector,
}
