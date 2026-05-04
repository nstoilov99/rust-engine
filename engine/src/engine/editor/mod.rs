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

pub mod icon_classes;
pub mod services;
pub mod theme;
pub mod widgets;
pub mod command_palette;
pub mod dialogs;
pub mod toasts;
pub mod status_bar;
pub mod dirty_state;
pub mod layout;
pub mod preview;
#[cfg(feature = "editor-debug")]
pub mod icon_inspector;
#[cfg(feature = "editor-debug")]
pub mod showcase;

pub mod asset_browser;
pub mod build_dialog;
mod commands;
mod console;
pub mod console_cmd;
mod dock_layout;
mod hierarchy_panel;
pub mod icons;
pub mod import_dialog;
mod inspector_panel;
mod input_action_editor;
mod input_context_editor;
mod input_settings_panel;
mod menu_bar;
pub mod mesh_editor;
pub mod play_mode;
pub mod profiler;
pub mod secondary_window;
mod selection;
mod tab_viewer;
pub mod viewport;
mod viewport_texture;
mod window_config;

pub use asset_browser::{
    AssetBrowserEvent, AssetBrowserPanel, AssetDragPayload, AssetEventQueue, AssetFilter,
    AssetRegistry, AssetSelection, FolderNode, GridView, GpuThumbnailContext, ListView,
    RenameTarget, ScanResult, SortCriteria, ThumbnailCache, ViewMode,
};
pub use build_dialog::BuildDialog;
pub use import_dialog::{ImportDialogAction, ImportDialogState, ImportPreview};
pub use commands::*;
pub use console::{ConsoleLog, LogFilter, LogLevel, LogMessage};
pub use console_cmd::ConsoleCommandSystem;
pub use dock_layout::*;
pub use hierarchy_panel::*;
pub use icons::{icon_button, IconManager, ToolbarIcon};
pub use inspector_panel::*;
pub use menu_bar::*;
pub use profiler::ProfilerPanel;
pub use secondary_window::{PendingWindowRequest, SecondaryWindow, SecondaryWindowKind};
pub use selection::*;
pub use services::EditorServices;
pub use tab_viewer::*;
pub use input_action_editor::{InputActionEditor, InputActionEditorState};
pub use input_context_editor::{InputContextEditor, InputContextEditorState};
pub use input_settings_panel::InputSettingsPanel;
pub use viewport::*;
pub use viewport_texture::*;
pub use window_config::*;
