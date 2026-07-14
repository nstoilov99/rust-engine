//! Asset Browser Panel state.
//!
//! The old rendering fns (`show`, `render_toolbar`, `render_folder_tree`,
//! `render_content`, `render_breadcrumb`, `render_delete_confirmation`,
//! `handle_view_response`, `handle_keyboard`) and the `views` submodule
//! (`FolderTreeView`, `GridView`, `ListView`) were removed as part of the UI
//! teardown. The crusty analog lives in `asset_browser_crusty`. This module
//! now only exports the panel state + supporting types.

mod events;
mod registry;
mod selection;
mod thumbnail;
pub mod thumbnail_renderer;

pub use events::{AssetBrowserEvent, AssetEventQueue};
pub use registry::{AssetFilter, AssetRegistry, FolderNode, ScanResult, SortCriteria};
pub use selection::AssetSelection;
pub use thumbnail::{ThumbnailCache, ThumbnailCacheStats, THUMBNAIL_SIZE};
pub use thumbnail_renderer::GpuThumbnailContext;

use crate::engine::assets::{AssetId, AssetMetadata, AssetType};
use std::collections::HashSet;
use std::path::PathBuf;

/// View mode for the asset browser
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewMode {
    #[default]
    Grid,
    List,
}

/// Target for rename operation (asset or folder)
#[derive(Debug, Clone)]
pub enum RenameTarget {
    /// Renaming an asset file
    Asset { id: AssetId, current_name: String },
    /// Renaming a folder
    Folder { path: PathBuf, current_name: String },
}

/// Target for delete operation
#[derive(Debug, Clone)]
pub enum DeleteTarget {
    /// Deleting an asset file
    Asset { id: AssetId, path: PathBuf },
    /// Deleting a folder
    Folder { path: PathBuf, is_empty: bool },
}

/// State for delete confirmation dialog
#[derive(Debug, Clone)]
pub struct DeleteConfirmation {
    /// What is being deleted
    pub target: DeleteTarget,
    /// Number of files in folder (for non-empty folder warnings)
    pub file_count: usize,
}

/// Payload for drag-and-drop operations
#[derive(Debug, Clone)]
pub enum DragPayload {
    /// Dragging an asset
    Asset(AssetDragPayload),
    /// Dragging a folder
    Folder { path: PathBuf, name: String },
}

/// Main asset browser panel
pub struct AssetBrowserPanel {
    /// Asset registry with metadata
    pub registry: AssetRegistry,
    /// Thumbnail cache
    pub thumbnails: ThumbnailCache,
    /// Current asset selection
    pub selection: AssetSelection,
    /// Current view mode
    pub view_mode: ViewMode,
    /// Current folder being viewed
    pub current_folder: PathBuf,
    /// Expanded folders in the tree
    pub folder_expanded: HashSet<PathBuf>,
    /// Search text filter
    pub search_text: String,
    /// Type filter (None = all types)
    pub type_filter: Option<AssetType>,
    /// Grid item size (zoom)
    pub grid_item_size: f32,
    /// Asset or folder being renamed
    pub renaming: Option<RenameTarget>,
    /// Delete confirmation dialog state
    pub delete_confirmation: Option<DeleteConfirmation>,
    /// Current drag payload (if dragging)
    pub drag_payload: Option<DragPayload>,
    /// Folder being hovered for drop target
    pub drop_target_folder: Option<PathBuf>,
    /// Event queue for cross-panel communication
    pub events: AssetEventQueue,
    /// Show folder panel
    pub show_folders: bool,
    /// Folder panel width
    pub folder_panel_width: f32,
    /// Needs rescan flag
    needs_rescan: bool,
    /// Asset paths hidden from the browser.
    hidden_paths: HashSet<PathBuf>,
}

impl AssetBrowserPanel {
    /// Create a new asset browser panel
    pub fn new(assets_root: PathBuf, gpu_ctx: Option<GpuThumbnailContext>) -> Self {
        let mut registry = AssetRegistry::new(assets_root.clone());

        // Initial scan
        let _ = registry.scan_directory();

        Self {
            registry,
            thumbnails: ThumbnailCache::new(assets_root, gpu_ctx),
            selection: AssetSelection::new(),
            view_mode: ViewMode::Grid,
            current_folder: PathBuf::new(),
            folder_expanded: HashSet::new(),
            search_text: String::new(),
            type_filter: None,
            grid_item_size: 96.0,
            renaming: None,
            delete_confirmation: None,
            drag_payload: None,
            drop_target_folder: None,
            events: AssetEventQueue::new(),
            show_folders: true,
            folder_panel_width: 180.0,
            needs_rescan: false,
            hidden_paths: HashSet::new(),
        }
    }

    /// Hide specific asset paths from the browser UI.
    pub fn set_hidden_paths<I>(&mut self, hidden_paths: I)
    where
        I: IntoIterator<Item = PathBuf>,
    {
        self.hidden_paths = hidden_paths.into_iter().collect();
    }

    /// Read the current hidden-path set (used by the crusty view to filter
    /// listings).
    pub fn hidden_paths(&self) -> &HashSet<PathBuf> {
        &self.hidden_paths
    }

    /// Request a rescan of the assets directory
    pub fn request_rescan(&mut self) {
        self.needs_rescan = true;
    }

    /// Run a pending rescan, if one was requested.
    pub(crate) fn process_rescan(&mut self) {
        if self.needs_rescan {
            self.needs_rescan = false;
            let _ = self.registry.scan_directory();
        }
    }

    /// Build an `AssetFilter` from the panel's current search / type / folder
    /// state. Used by the crusty view to run registry queries.
    pub(crate) fn build_filter(&self) -> AssetFilter {
        AssetFilter {
            search_text: if self.search_text.is_empty() {
                None
            } else {
                Some(self.search_text.clone())
            },
            asset_types: self.type_filter.map(|t| vec![t]),
            tags: None,
            folder: if self.current_folder.as_os_str().is_empty() {
                None
            } else {
                Some(self.current_folder.clone())
            },
            include_subfolders: true,
            sort_by: SortCriteria::Name,
            sort_ascending: true,
            excluded_paths: self.hidden_paths.iter().cloned().collect(),
        }
    }

    /// Get selected asset metadata
    pub fn selected_assets(&self) -> Vec<&AssetMetadata> {
        self.selection
            .all()
            .filter_map(|id| self.registry.get(id))
            .collect()
    }

    /// Navigate to a specific folder
    pub fn navigate_to_folder(&mut self, folder: PathBuf) {
        self.current_folder = folder.clone();

        // Expand parent folders
        let mut current = folder.as_path();
        while let Some(parent) = current.parent() {
            if !parent.as_os_str().is_empty() {
                self.folder_expanded.insert(parent.to_path_buf());
            }
            current = parent;
        }
    }

    /// Focus on a specific asset
    pub fn focus_asset(&mut self, id: AssetId) {
        if let Some(metadata) = self.registry.get(id) {
            // Navigate to the asset's folder
            if let Some(folder) = metadata.path.parent() {
                self.navigate_to_folder(folder.to_path_buf());
            }

            // Select the asset
            self.selection.select(id);
        }
    }

    /// Get drag payload for viewport drop
    pub fn get_drag_payload(&self) -> Option<AssetDragPayload> {
        self.selection.primary().and_then(|id| {
            self.registry.get(id).map(|metadata| AssetDragPayload {
                asset_id: id,
                asset_type: metadata.asset_type,
                path: metadata.path.clone(),
            })
        })
    }
}

impl Default for AssetBrowserPanel {
    fn default() -> Self {
        Self::new(PathBuf::from("assets"), None)
    }
}

/// Payload for drag-and-drop operations
#[derive(Debug, Clone)]
pub struct AssetDragPayload {
    pub asset_id: AssetId,
    pub asset_type: AssetType,
    pub path: PathBuf,
}
