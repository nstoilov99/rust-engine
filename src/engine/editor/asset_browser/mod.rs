//! Asset Browser Panel
//!
//! Provides a comprehensive UI for browsing, searching, and managing project assets.
//! Features include:
//! - Grid and list view modes with virtualization
//! - Folder tree navigation
//! - Search and filtering by type, tags, and name
//! - Async thumbnail generation with caching
//! - Drag and drop support for viewport integration

mod events;
mod registry;
mod selection;
mod thumbnail;
mod views;

pub use events::{AssetBrowserEvent, AssetEventQueue};
pub use registry::{AssetFilter, AssetRegistry, FolderNode, ScanResult, SortCriteria};
pub use selection::AssetSelection;
pub use thumbnail::{ThumbnailCache, ThumbnailCacheStats, THUMBNAIL_SIZE};
pub use views::{FolderTreeView, GridView, ListView};

use crate::engine::assets::{AssetId, AssetMetadata, AssetType};
use crate::engine::editor::icons::IconManager;
use egui::{RichText, Ui};
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
    Asset {
        id: AssetId,
        current_name: String,
    },
    /// Renaming a folder
    Folder {
        path: PathBuf,
        current_name: String,
    },
}

/// Target for delete operation
#[derive(Debug, Clone)]
pub enum DeleteTarget {
    /// Deleting an asset file
    Asset {
        id: AssetId,
        path: PathBuf,
    },
    /// Deleting a folder
    Folder {
        path: PathBuf,
        is_empty: bool,
    },
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
    Folder {
        path: PathBuf,
        name: String,
    },
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
    /// Folder tree view state
    folder_tree: FolderTreeView,
    /// Grid view state
    grid_view: GridView,
    /// List view state
    list_view: ListView,
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
    pub fn new(assets_root: PathBuf) -> Self {
        let mut registry = AssetRegistry::new(assets_root.clone());

        // Initial scan
        let _ = registry.scan_directory();

        Self {
            registry,
            thumbnails: ThumbnailCache::new(assets_root),
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
            folder_tree: FolderTreeView::new(),
            grid_view: GridView::new(96.0),
            list_view: ListView::new(),
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

    /// Request a rescan of the assets directory
    pub fn request_rescan(&mut self) {
        self.needs_rescan = true;
    }

    /// Render the asset browser panel contents
    /// `icon_manager` is optional - if provided, PNG icons will be used
    pub fn show(&mut self, ui: &mut Ui, icon_manager: Option<&IconManager>) {
        // Poll for completed thumbnails
        self.thumbnails.poll(ui.ctx());

        // Handle rescan if needed
        if self.needs_rescan {
            self.needs_rescan = false;
            let result = self.registry.scan_directory();
            if result.added > 0 || result.updated > 0 || result.removed > 0 {
                // Log scan results
            }
        }

        // Top toolbar
        self.render_toolbar(ui);

        ui.separator();

        // Breadcrumb navigation
        self.render_breadcrumb(ui);

        ui.separator();

        // Handle keyboard navigation
        self.handle_keyboard(ui);

        // Main content area with optional folder panel
        if self.show_folders {
            // Split view with folder tree and content
            egui::SidePanel::left("asset_folder_panel")
                .resizable(true)
                .default_width(self.folder_panel_width)
                .width_range(120.0..=300.0)
                .show_inside(ui, |ui| {
                    self.render_folder_tree(ui, icon_manager);
                });
        }

        // Content area
        egui::CentralPanel::default().show_inside(ui, |ui| {
            self.render_content(ui, icon_manager);
        });

        // Render delete confirmation dialog if active
        self.render_delete_confirmation(ui);
    }

    /// Render delete confirmation dialog for non-empty folders
    fn render_delete_confirmation(&mut self, ui: &mut Ui) {
        if self.delete_confirmation.is_none() {
            return;
        }

        let mut close_dialog = false;
        let mut confirm_delete = false;

        if let Some(ref confirmation) = self.delete_confirmation {
            let (title, message) = match &confirmation.target {
                DeleteTarget::Asset { path, .. } => {
                    ("Delete Asset".to_string(), format!(
                        "Are you sure you want to delete '{}'?\n\nThis action cannot be undone.",
                        path.file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_else(|| path.display().to_string())
                    ))
                }
                DeleteTarget::Folder { path, is_empty } => {
                    let folder_name = path.file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| path.display().to_string());

                    if *is_empty {
                        ("Delete Folder".to_string(), format!(
                            "Are you sure you want to delete the folder '{}'?",
                            folder_name
                        ))
                    } else {
                        ("Delete Non-Empty Folder".to_string(), format!(
                            "The folder '{}' contains {} items.\n\n\
                            Are you sure you want to delete this folder and ALL its contents?\n\n\
                            This action cannot be undone!",
                            folder_name, confirmation.file_count
                        ))
                    }
                }
            };

            egui::Window::new(&title)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ui.ctx(), |ui| {
                    ui.vertical(|ui| {
                        ui.add_space(8.0);
                        ui.label(&message);
                        ui.add_space(16.0);

                        // Center the buttons by calculating width and adding padding
                        let button_width = 80.0;
                        let spacing = 16.0;
                        let total_buttons_width = (button_width * 2.0) + spacing;
                        let available_width = ui.available_width();
                        let padding = (available_width - total_buttons_width) / 2.0;

                        ui.horizontal(|ui| {
                            if padding > 0.0 {
                                ui.add_space(padding);
                            }

                            // Cancel button
                            if ui.add(
                                egui::Button::new("Cancel")
                                    .min_size(egui::vec2(button_width, 28.0))
                            ).clicked() {
                                close_dialog = true;
                            }

                            ui.add_space(spacing);

                            // Red delete button
                            if ui.add(
                                egui::Button::new(
                                    RichText::new("Delete").color(egui::Color32::WHITE)
                                )
                                .fill(egui::Color32::from_rgb(180, 60, 60))
                                .min_size(egui::vec2(button_width, 28.0))
                            ).clicked() {
                                confirm_delete = true;
                            }
                        });
                    });
                });
        }

        if confirm_delete {
            // Handle the confirmed deletion
            if let Some(confirmation) = self.delete_confirmation.take() {
                match confirmation.target {
                    DeleteTarget::Asset { id, path } => {
                        self.events.push(AssetBrowserEvent::AssetDeleted { id, path });
                    }
                    DeleteTarget::Folder { path, .. } => {
                        // For non-empty folders, we need to use FolderDeleted with force flag
                        // The app.rs handler will use remove_dir_all for this
                        self.events.push(AssetBrowserEvent::FolderDeleted { path });
                    }
                }
            }
        } else if close_dialog {
            self.delete_confirmation = None;
        }
    }

    fn render_toolbar(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            // Folder toggle
            if ui.selectable_label(self.show_folders, "\u{1F4C1}").on_hover_text("Toggle folders").clicked() {
                self.show_folders = !self.show_folders;
            }

            ui.separator();

            // Search box
            ui.label("\u{1F50D}"); // 🔍
            let search_response = ui.add(
                egui::TextEdit::singleline(&mut self.search_text)
                    .hint_text("Search...")
                    .desired_width(150.0),
            );
            if search_response.changed() {
                // Search changed
            }

            // Clear search button
            if !self.search_text.is_empty() {
                if ui.small_button("\u{2715}").on_hover_text("Clear search").clicked() {
                    self.search_text.clear();
                }
            }

            ui.separator();

            // Type filter
            egui::ComboBox::from_id_salt("type_filter")
                .selected_text(match &self.type_filter {
                    Some(t) => t.display_name(),
                    None => "All Types",
                })
                .show_ui(ui, |ui| {
                    if ui.selectable_value(&mut self.type_filter, None, "All Types").clicked() {}
                    ui.separator();
                    for asset_type in AssetType::all() {
                        if ui.selectable_value(&mut self.type_filter, Some(*asset_type), asset_type.display_name()).clicked() {}
                    }
                });

            ui.separator();

            // View mode toggle
            if ui.selectable_label(self.view_mode == ViewMode::Grid, "\u{25A6}") // Grid icon
                .on_hover_text("Grid view")
                .clicked()
            {
                self.view_mode = ViewMode::Grid;
            }
            if ui.selectable_label(self.view_mode == ViewMode::List, "\u{2630}") // List icon
                .on_hover_text("List view")
                .clicked()
            {
                self.view_mode = ViewMode::List;
            }

            ui.separator();

            // Zoom slider (grid view only)
            if self.view_mode == ViewMode::Grid {
                ui.label("Size:");
                let slider = egui::Slider::new(&mut self.grid_item_size, 48.0..=192.0)
                    .show_value(false);
                if ui.add(slider).changed() {
                    self.grid_view.item_size = self.grid_item_size;
                }
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Rescan button
                if ui.button("\u{27F3}").on_hover_text("Rescan assets").clicked() {
                    self.request_rescan();
                }

                // Asset count
                let filter = self.build_filter();
                let count = self.registry.query(&filter).len();
                ui.label(RichText::new(format!("{} assets", count)).weak());
            });
        });
    }

    fn render_folder_tree(&mut self, ui: &mut Ui, icon_manager: Option<&IconManager>) {
        ui.heading("Folders");
        ui.separator();

        // Extract folder renaming state
        let mut renaming_folder = match &self.renaming {
            Some(RenameTarget::Folder { path, current_name }) => Some((path.clone(), current_name.clone())),
            _ => None,
        };

        let folder_tree = self.registry.get_folder_tree();
        let response = self.folder_tree.show(
            ui,
            &folder_tree,
            &self.current_folder,
            &mut self.folder_expanded,
            &mut renaming_folder,
            icon_manager,
        );

        // Sync folder renaming text back (if modified)
        if let Some((path, text)) = renaming_folder {
            if let Some(RenameTarget::Folder { path: rename_path, current_name }) = &mut self.renaming {
                if *rename_path == path {
                    *current_name = text;
                }
            }
        }

        // Handle folder rename confirmed
        if let Some((old_path, new_name)) = response.folder_rename_confirmed {
            // Compute new path: same parent directory with new folder name
            let new_path = if let Some(parent) = old_path.parent() {
                parent.join(&new_name)
            } else {
                // Folder is at root level
                PathBuf::from(&new_name)
            };
            self.events.push(AssetBrowserEvent::FolderRenamed {
                old_path,
                new_path,
            });
            self.renaming = None;
        }

        // Handle folder rename cancelled - only if the cancelled folder matches current renaming folder
        // This prevents cancelling a new rename when the old TextEdit loses focus
        if response.folder_rename_cancelled {
            if let Some(RenameTarget::Folder { path: current_path, .. }) = &self.renaming {
                if response.folder_rename_cancelled_path.as_ref() == Some(current_path) {
                    self.renaming = None;
                }
            }
        }

        // Update drop target for visual feedback
        self.drop_target_folder = response.drop_target;

        // Handle asset dropped on folder (using egui's DnD system)
        if let Some((target_folder, asset_id)) = response.asset_dropped {
            // Get asset metadata to find the source path
            if let Some(metadata) = self.registry.get(asset_id) {
                // Move asset to target folder
                self.events.push(AssetBrowserEvent::AssetMoved {
                    id: asset_id,
                    old_path: metadata.path.clone(),
                    new_path: target_folder.join(
                        metadata.path.file_name().unwrap_or_default()
                    ),
                });
            }
            // Clear drag state after drop
            self.drag_payload = None;
        }

        // Handle folder dropped on folder (move folder into another folder)
        if let Some((target_folder, source_folder)) = response.folder_dropped {
            // Get the folder name to construct the new path
            if let Some(folder_name) = source_folder.file_name() {
                let new_path = target_folder.join(folder_name);
                self.events.push(AssetBrowserEvent::FolderMoved {
                    old_path: source_folder,
                    new_path,
                });
            }
        }

        // Handle folder navigation
        if let Some(clicked) = response.clicked {
            self.current_folder = clicked.clone();
            self.events.push(AssetBrowserEvent::FolderChanged { path: clicked });
        }

        // Handle folder context menu actions
        if let Some((path, action)) = response.context_action {
            use views::FolderContextAction;
            match action {
                FolderContextAction::NewFolder => {
                    self.events.push(AssetBrowserEvent::CreateFolder { parent_path: path });
                }
                FolderContextAction::Rename => {
                    // Start folder rename
                    if let Some((rename_path, current_name)) = response.rename_requested {
                        self.renaming = Some(RenameTarget::Folder {
                            path: rename_path,
                            current_name,
                        });
                    }
                }
                FolderContextAction::Delete => {
                    // Always show confirmation dialog for folder deletion
                    let full_path = self.registry.root_path().join(&path);
                    let file_count = std::fs::read_dir(&full_path)
                        .map(|entries| entries.count())
                        .unwrap_or(0);
                    let is_empty = file_count == 0;

                    self.delete_confirmation = Some(DeleteConfirmation {
                        target: DeleteTarget::Folder { path, is_empty },
                        file_count,
                    });
                }
                FolderContextAction::RevealInExplorer => {
                    self.events.push(AssetBrowserEvent::RevealFolderInExplorer { path });
                }
            }
        }
    }

    fn render_content(&mut self, ui: &mut Ui, icon_manager: Option<&IconManager>) {
        // Build filter
        let filter = self.build_filter();
        let assets = self.registry.query(&filter);

        // Show empty folder message if no assets
        if assets.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(50.0);
                ui.label(RichText::new("No assets found").weak().size(16.0));
                if !self.search_text.is_empty() || self.type_filter.is_some() {
                    ui.label(RichText::new("Try adjusting your filters").weak());
                } else if !self.current_folder.as_os_str().is_empty() {
                    ui.label(RichText::new("This folder is empty").weak());
                }
            });
            return;
        }

        // Extract renaming asset state for passing to views
        let mut renaming_asset = match &self.renaming {
            Some(RenameTarget::Asset { id, current_name }) => Some((*id, current_name.clone())),
            _ => None,
        };

        match self.view_mode {
            ViewMode::Grid => {
                let response = self.grid_view.show(ui, &assets, &mut self.thumbnails, &mut self.selection, &mut renaming_asset, icon_manager);
                self.handle_view_response(response.clicked, response.double_clicked, response.context_menu, ui);

                // Handle rename request from context menu
                if let Some(id) = response.rename_requested {
                    if let Some(metadata) = self.registry.get(id) {
                        self.renaming = Some(RenameTarget::Asset {
                            id,
                            current_name: metadata.display_name.clone(),
                        });
                    }
                }

                // Handle rename confirmed
                if let Some((id, new_name)) = response.rename_confirmed {
                    if let Some(metadata) = self.registry.get(id) {
                        let old_name = metadata.display_name.clone();
                        self.events.push(AssetBrowserEvent::AssetRenamed {
                            id,
                            old_name,
                            new_name,
                        });
                    }
                    self.renaming = None;
                }

                // Handle rename cancelled
                if response.rename_cancelled {
                    self.renaming = None;
                }

                // Handle reveal in explorer from context menu
                if let Some(id) = response.reveal_in_explorer {
                    if let Some(metadata) = self.registry.get(id) {
                        self.events.push(AssetBrowserEvent::RevealInExplorer {
                            path: metadata.path.clone(),
                        });
                    }
                }

                // Handle delete from context menu
                if let Some(id) = response.delete_requested {
                    if let Some(metadata) = self.registry.get(id) {
                        self.events.push(AssetBrowserEvent::AssetDeleted {
                            id,
                            path: metadata.path.clone(),
                        });
                    }
                }

                // Handle drag started - store drag payload
                if let Some(id) = response.drag_started {
                    if let Some(metadata) = self.registry.get(id) {
                        self.drag_payload = Some(DragPayload::Asset(AssetDragPayload {
                            asset_id: id,
                            asset_type: metadata.asset_type,
                            path: metadata.path.clone(),
                        }));
                    }
                }
            }
            ViewMode::List => {
                let response = self.list_view.show(ui, &assets, &mut self.selection, &mut renaming_asset, icon_manager);
                self.handle_view_response(response.clicked, response.double_clicked, response.context_menu, ui);

                // Handle rename request from context menu
                if let Some(id) = response.rename_requested {
                    if let Some(metadata) = self.registry.get(id) {
                        self.renaming = Some(RenameTarget::Asset {
                            id,
                            current_name: metadata.display_name.clone(),
                        });
                    }
                }

                // Handle rename confirmed
                if let Some((id, new_name)) = response.rename_confirmed {
                    if let Some(metadata) = self.registry.get(id) {
                        let old_name = metadata.display_name.clone();
                        self.events.push(AssetBrowserEvent::AssetRenamed {
                            id,
                            old_name,
                            new_name,
                        });
                    }
                    self.renaming = None;
                }

                // Handle rename cancelled
                if response.rename_cancelled {
                    self.renaming = None;
                }

                // Handle reveal in explorer from context menu
                if let Some(id) = response.reveal_in_explorer {
                    if let Some(metadata) = self.registry.get(id) {
                        self.events.push(AssetBrowserEvent::RevealInExplorer {
                            path: metadata.path.clone(),
                        });
                    }
                }

                // Handle delete from context menu
                if let Some(id) = response.delete_requested {
                    if let Some(metadata) = self.registry.get(id) {
                        self.events.push(AssetBrowserEvent::AssetDeleted {
                            id,
                            path: metadata.path.clone(),
                        });
                    }
                }

                // Handle drag started - store drag payload
                if let Some(id) = response.drag_started {
                    if let Some(metadata) = self.registry.get(id) {
                        self.drag_payload = Some(DragPayload::Asset(AssetDragPayload {
                            asset_id: id,
                            asset_type: metadata.asset_type,
                            path: metadata.path.clone(),
                        }));
                    }
                }
            }
        }

        // Clear drag payload if mouse released without dropping on folder
        if self.drag_payload.is_some() && ui.input(|i| i.pointer.any_released()) {
            // Only clear if we didn't drop on a folder (drop_target_folder would be set)
            if self.drop_target_folder.is_none() {
                self.drag_payload = None;
            }
        }

        // Sync renaming_asset changes back (if text was modified)
        if let Some((id, text)) = renaming_asset {
            if let Some(RenameTarget::Asset { id: rename_id, current_name }) = &mut self.renaming {
                if *rename_id == id {
                    *current_name = text;
                }
            }
        }
    }

    fn handle_view_response(
        &mut self,
        clicked: Option<AssetId>,
        double_clicked: Option<AssetId>,
        context_menu: Option<AssetId>,
        ui: &Ui,
    ) {
        // Handle click
        if let Some(id) = clicked {
            let modifiers = ui.input(|i| i.modifiers);
            if modifiers.ctrl {
                self.selection.toggle(id);
            } else if modifiers.shift {
                // Range select
                let filter = self.build_filter();
                let assets = self.registry.query(&filter);
                let visible_ids: Vec<AssetId> = assets.iter().map(|a| a.id).collect();
                self.selection.select_range(&visible_ids, id);
            } else {
                self.selection.select(id);
            }
            self.events.push(AssetBrowserEvent::AssetSelected { id });
        }

        // Handle double-click
        if let Some(id) = double_clicked {
            self.events.push(AssetBrowserEvent::AssetOpened { id });
        }

        // Handle context menu
        if let Some(id) = context_menu {
            // Make sure it's selected
            if !self.selection.is_selected(id) {
                self.selection.select(id);
            }
            // Context menu is handled in render_context_menu
        }
    }

    fn build_filter(&self) -> AssetFilter {
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

    /// Render breadcrumb navigation showing current folder path
    fn render_breadcrumb(&mut self, ui: &mut Ui) {
        // Collect components first to avoid borrow conflicts
        let current_folder = self.current_folder.clone();
        let components: Vec<_> = current_folder.components().collect();

        let mut clicked_path: Option<PathBuf> = None;

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;

            // Root "Content" button (displayed with capital C for industry standard naming)
            let root_selected = current_folder.as_os_str().is_empty();
            if ui.selectable_label(root_selected, "Content").clicked() {
                clicked_path = Some(PathBuf::new());
            }

            // Path components
            if !components.is_empty() {
                let mut accumulated = PathBuf::new();
                for component in &components {
                    ui.label(RichText::new("/").weak());
                    accumulated.push(component.as_os_str());
                    let name = component.as_os_str().to_string_lossy();
                    let is_last = accumulated == current_folder;

                    if ui.selectable_label(is_last, name.as_ref()).clicked() && !is_last {
                        clicked_path = Some(accumulated.clone());
                    }
                }
            }
        });

        // Apply navigation after the UI closure
        if let Some(path) = clicked_path {
            self.current_folder = path.clone();
            self.events.push(AssetBrowserEvent::FolderChanged { path });
        }
    }

    /// Handle keyboard navigation for the asset browser
    fn handle_keyboard(&mut self, ui: &Ui) {
        // Don't process keyboard shortcuts when actively renaming
        // The rename TextEdit handles Enter/Escape itself
        if self.renaming.is_some() {
            // Only allow Escape to cancel from outside the TextEdit
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.renaming = None;
                self.delete_confirmation = None;
            }
            return;
        }

        // Skip keyboard handling if a text field (like search box) has focus
        // This prevents F2, Delete, etc. from triggering while typing
        if ui.ctx().wants_keyboard_input() {
            return;
        }

        // Enter key - open selected asset
        if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            if let Some(id) = self.selection.primary() {
                self.events.push(AssetBrowserEvent::AssetOpened { id });
            }
        }

        // Delete key - show confirmation dialog before deleting
        if ui.input(|i| i.key_pressed(egui::Key::Delete)) {
            // Check if an asset is selected first
            if let Some(id) = self.selection.primary() {
                if let Some(metadata) = self.registry.get(id) {
                    // Show confirmation dialog for asset
                    self.delete_confirmation = Some(DeleteConfirmation {
                        target: DeleteTarget::Asset {
                            id,
                            path: metadata.path.clone(),
                        },
                        file_count: 1,
                    });
                }
            } else if !self.current_folder.as_os_str().is_empty() {
                // No asset selected, but we have a non-root folder selected
                // Show confirmation dialog for folder
                let full_path = self.registry.root_path().join(&self.current_folder);
                if full_path.exists() {
                    let file_count = std::fs::read_dir(&full_path)
                        .map(|entries| entries.count())
                        .unwrap_or(0);
                    self.delete_confirmation = Some(DeleteConfirmation {
                        target: DeleteTarget::Folder {
                            path: self.current_folder.clone(),
                            is_empty: file_count == 0,
                        },
                        file_count,
                    });
                }
            }
        }

        // F2 - start rename for selected asset OR current folder
        if ui.input(|i| i.key_pressed(egui::Key::F2)) {
            // First priority: rename selected asset
            if let Some(id) = self.selection.primary() {
                if let Some(metadata) = self.registry.get(id) {
                    self.renaming = Some(RenameTarget::Asset {
                        id,
                        current_name: metadata.display_name.clone(),
                    });
                }
            } else if !self.current_folder.as_os_str().is_empty() {
                // Second priority: rename current folder (if not at root)
                let folder_name = self.current_folder
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                if !folder_name.is_empty() {
                    self.renaming = Some(RenameTarget::Folder {
                        path: self.current_folder.clone(),
                        current_name: folder_name,
                    });
                }
            }
        }

        // Escape - priority chain: dialog → search → selection
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            if self.delete_confirmation.is_some() {
                // First priority: close delete confirmation dialog
                self.delete_confirmation = None;
            } else if !self.search_text.is_empty() {
                // Second priority: clear search filter
                self.search_text.clear();
            } else if !self.selection.is_empty() {
                // Third priority: clear asset selection
                self.selection.clear();
            }
        }

        // Backspace - navigate up one folder level
        if ui.input(|i| i.key_pressed(egui::Key::Backspace)) {
            if !self.current_folder.as_os_str().is_empty() {
                if let Some(parent) = self.current_folder.parent() {
                    let parent_path = parent.to_path_buf();
                    self.current_folder = parent_path.clone();
                    self.events.push(AssetBrowserEvent::FolderChanged { path: parent_path });
                }
            }
        }

        // F5 - refresh
        if ui.input(|i| i.key_pressed(egui::Key::F5)) {
            self.request_rescan();
        }

        // Arrow navigation and Ctrl+A require visible assets list
        let filter = self.build_filter();
        let visible_assets: Vec<AssetId> = self.registry.query(&filter)
            .iter()
            .map(|m| m.id)
            .collect();

        if visible_assets.is_empty() {
            return;
        }

        // Arrow key navigation
        if ui.input(|i| i.key_pressed(egui::Key::ArrowDown)) {
            if let Some(primary) = self.selection.primary() {
                if let Some(idx) = visible_assets.iter().position(|&id| id == primary) {
                    if idx + 1 < visible_assets.len() {
                        self.selection.select(visible_assets[idx + 1]);
                    }
                }
            } else {
                self.selection.select(visible_assets[0]);
            }
        }

        if ui.input(|i| i.key_pressed(egui::Key::ArrowUp)) {
            if let Some(primary) = self.selection.primary() {
                if let Some(idx) = visible_assets.iter().position(|&id| id == primary) {
                    if idx > 0 {
                        self.selection.select(visible_assets[idx - 1]);
                    }
                }
            } else {
                self.selection.select(visible_assets[visible_assets.len() - 1]);
            }
        }

        // Ctrl+A - select all
        if ui.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::A)) {
            for id in &visible_assets {
                self.selection.add(*id);
            }
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
        Self::new(PathBuf::from("assets"))
    }
}

/// Payload for drag-and-drop operations
#[derive(Debug, Clone)]
pub struct AssetDragPayload {
    pub asset_id: AssetId,
    pub asset_type: AssetType,
    pub path: PathBuf,
}
