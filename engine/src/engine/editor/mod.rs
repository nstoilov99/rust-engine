//! Editor systems and UI panels
//!
//! ## Step 0 Notes — Integration Points
//!
//! * `game_client/src/app.rs` — main editor application; `EditorServices` constructed
//!   in `App::new` (line ~198) alongside `EditorApp`. `EditorApp` holds viewport,
//!   console, scene, ui, play sub-structures.
//! * `tab_viewer.rs:29` — `EditorContext<'a>` is a borrowed-references struct passed
//!   to `EditorTabViewer` for panel rendering. Modified to carry
//!   `services: &'a mut EditorServices`.
//! * `secondary_window.rs` — multi-window support; secondary windows will receive
//!   `&mut EditorServices` in a future step.
//! * `dock_layout.rs:12` — `LAYOUT_FILE = "editor_layout.ron"`. Step 11 reuses this
//!   constant; no parallel filename.
//! * `commands.rs` — existing undo/redo `Command` trait + `CommandHistory`. The new
//!   module is `command_palette/`, not `commands/`.
//! * `icons.rs` — existing `IconManager` + `ToolbarIcon` + `AssetBrowserIcon`. Step 1
//!   extends this into a full `IconRegistry`.

pub mod command_palette;
pub mod dialogs;
pub mod dirty_state;
pub mod hierarchy_icons;
pub mod icon_classes;
#[cfg(feature = "editor-debug")]
pub mod icon_inspector;
pub mod layout;
pub mod preview;
pub mod services;
#[cfg(feature = "editor-debug")]
pub mod showcase;
pub mod status_bar;
pub mod theme;
pub mod toasts;
pub mod widgets;

pub mod asset_browser;
#[cfg(feature = "crusty")]
pub mod asset_browser_crusty;
pub mod build_dialog;
mod commands;
mod console;
#[cfg(feature = "crusty")]
pub mod console_crusty;
#[cfg(all(feature = "crusty", windows))]
pub mod desktop_sampler;
pub mod console_cmd;
mod dock_layout;
#[cfg(feature = "crusty")]
pub mod command_palette_crusty;
#[cfg(feature = "crusty")]
pub mod dialogs_crusty;
#[cfg(feature = "crusty")]
pub mod dock_crusty;
#[cfg(feature = "crusty")]
pub mod crusty_window;
#[cfg(feature = "crusty")]
pub mod hierarchy_crusty;
mod hierarchy_panel;
pub mod icons;
pub mod import_dialog;
mod input_action_editor;
mod input_context_editor;
mod input_settings_panel;
#[cfg(feature = "crusty")]
pub mod inspector_crusty;
mod inspector_panel;
mod menu_bar;
#[cfg(feature = "crusty")]
pub mod menu_bar_crusty;
pub mod mesh_editor;
pub mod play_mode;
pub mod profiler;
#[cfg(feature = "crusty")]
pub mod profiler_crusty;
pub mod scene_tab;
pub mod secondary_window;
mod selection;
mod tab_viewer;
#[cfg(feature = "crusty")]
pub mod status_bar_crusty;
#[cfg(feature = "crusty")]
pub mod toasts_crusty;
pub mod viewport;
#[cfg(feature = "crusty")]
pub mod viewport_crusty;
mod viewport_texture;
mod window_config;

pub use asset_browser::{
    AssetBrowserEvent, AssetBrowserPanel, AssetDragPayload, AssetEventQueue, AssetFilter,
    AssetRegistry, AssetSelection, FolderNode, GpuThumbnailContext, GridView, ListView,
    RenameTarget, ScanResult, SortCriteria, ThumbnailCache, ViewMode,
};
pub use build_dialog::BuildDialog;
pub use command_palette::{
    dispatch_action, Command, CommandPalette, CommandRegistry, EditorAction,
};
pub use commands::*;
pub use console::{ConsoleLog, LogFilter, LogLevel, LogMessage};
pub use console_cmd::ConsoleCommandSystem;
pub use dialogs::{Dialog, DialogActions, DialogButtons, DialogStack};
pub use dirty_state::{normalize_asset_key, DirtyAsset, DirtyState};
pub use dock_layout::*;
pub use hierarchy_panel::*;
pub use icons::{icon_button, IconManager, ToolbarIcon};
pub use import_dialog::{ImportDialogAction, ImportDialogState, ImportPreview};
pub use input_action_editor::{InputActionEditor, InputActionEditorState};
pub use input_context_editor::{InputContextEditor, InputContextEditorState};
pub use input_settings_panel::InputSettingsPanel;
pub use inspector_panel::*;
pub use menu_bar::*;
pub use preview::{AssetPreview, AssetPreviewRegistry, PreviewId, PreviewKind};
pub use profiler::ProfilerPanel;
pub use scene_tab::{DormantScene, SaveAsDialog, SceneId, SceneRegistry};
pub use secondary_window::{PendingWindowRequest, SecondaryWindow, SecondaryWindowKind};
pub use selection::*;
pub use services::EditorServices;
pub use status_bar::{render_status_bar, StatusBarState};
pub use tab_viewer::*;
pub use toasts::{Toast, ToastKind, ToastStack};
pub use viewport::*;
pub use viewport_texture::*;
pub use window_config::*;
