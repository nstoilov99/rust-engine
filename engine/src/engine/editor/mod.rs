//! Editor systems and UI panels
//!
//! The runtime is crusty-gui. Legacy panel state (hierarchy,
//! inspector, mesh_editor, input_*, console, asset_browser, dock_layout,
//! profiler, viewport) still lives here and is drawn by the `*_crusty`
//! sibling modules. Retired legacy widgets/renders were removed as part of
//! the editor teardown; see the crusty modules below.

pub mod command_palette;
pub mod dialogs;
pub mod dirty_state;
pub mod hierarchy_icons;
pub mod icon_classes;
pub mod layout;
pub mod services;
pub mod theme;
pub mod toasts;
pub mod widgets;

pub mod asset_browser;
#[cfg(feature = "editor")]
pub mod asset_browser_crusty;
pub mod build_dialog;
#[cfg(feature = "editor")]
pub mod command_palette_crusty;
pub mod commands;
mod console;
pub mod console_cmd;
#[cfg(feature = "editor")]
pub mod console_crusty;
#[cfg(feature = "editor")]
pub mod crusty_window;
#[cfg(all(feature = "editor", windows))]
pub mod desktop_sampler;
#[cfg(feature = "editor")]
pub mod dialogs_crusty;
#[cfg(feature = "editor")]
pub mod dock_crusty;
mod dock_layout;
#[cfg(feature = "editor")]
pub mod hierarchy_crusty;
mod hierarchy_panel;
pub mod import_dialog;
mod input_action_editor;
mod input_context_editor;
#[cfg(feature = "editor")]
pub mod input_editors_crusty;
mod input_settings_panel;
#[cfg(feature = "editor")]
pub mod inspector_crusty;
mod inspector_panel;
mod menu_bar;
#[cfg(feature = "editor")]
pub mod menu_bar_crusty;
pub mod mesh_editor;
#[cfg(feature = "editor")]
pub mod mesh_editor_crusty;
pub mod graph_editor;
pub mod graph_prefs;
pub mod graph_wire_router;
#[cfg(feature = "editor")]
pub mod graph_editor_crusty;
pub mod editor_prefs;
pub mod play_mode;
pub mod play_settings;
pub mod profiler;
pub mod project_config;
#[cfg(feature = "editor")]
pub mod profiler_crusty;
pub mod scene_tab;
pub mod secondary_kind;
mod selection;
#[cfg(feature = "editor")]
pub mod settings_crusty;
#[cfg(feature = "editor")]
pub mod status_bar_crusty;
#[cfg(feature = "editor")]
pub mod toasts_crusty;
pub mod viewport;
#[cfg(feature = "editor")]
pub mod viewport_crusty;
mod viewport_texture;
mod window_config;
pub mod world_object;

pub use asset_browser::{
    AssetBrowserEvent, AssetBrowserPanel, AssetDragPayload, AssetEventQueue, AssetFilter,
    AssetRegistry, AssetSelection, FolderNode, GpuThumbnailContext, RenameTarget, ScanResult,
    SortCriteria, ThumbnailCache, ViewMode,
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
pub use import_dialog::{ImportDialogAction, ImportDialogState, ImportPreview};
pub use input_action_editor::{InputActionEditor, InputActionEditorState};
pub use input_context_editor::{InputContextEditor, InputContextEditorState};
pub use input_settings_panel::InputSettingsPanel;
pub use inspector_panel::*;
pub use menu_bar::*;
pub use profiler::ProfilerPanel;
pub use scene_tab::{DormantScene, SaveAsDialog, SceneId, SceneRegistry};
pub use secondary_kind::SecondaryWindowKind;
pub use selection::*;
pub use services::EditorServices;
pub use toasts::{Toast, ToastKind, ToastStack};
pub use viewport::*;
pub use viewport_texture::*;
pub use window_config::*;
pub use world_object::WorldObjectInfo;
