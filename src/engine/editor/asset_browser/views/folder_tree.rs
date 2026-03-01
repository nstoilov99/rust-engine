//! Folder tree view for asset browser
//!
//! Displays the folder hierarchy as a collapsible tree, allowing
//! navigation through the asset directory structure.

use crate::engine::assets::AssetId;
use crate::engine::editor::asset_browser::registry::FolderNode;
use crate::engine::editor::icons::{AssetBrowserIcon, IconManager};
use egui::{Color32, RichText, Ui};
use std::collections::HashSet;
use std::path::PathBuf;

/// Context menu action for folders
#[derive(Debug, Clone)]
pub enum FolderContextAction {
    /// Create a new folder inside this folder
    NewFolder,
    /// Rename this folder
    Rename,
    /// Delete this folder
    Delete,
    /// Reveal this folder in file explorer
    RevealInExplorer,
}

/// Response from folder tree interactions
#[derive(Debug, Default)]
pub struct FolderTreeResponse {
    /// Folder that was clicked (navigate to it)
    pub clicked: Option<PathBuf>,
    /// Context menu action requested
    pub context_action: Option<(PathBuf, FolderContextAction)>,
    /// Folder rename was requested (path, current name)
    pub rename_requested: Option<(PathBuf, String)>,
    /// Folder rename confirmed (old_path, new_name)
    pub folder_rename_confirmed: Option<(PathBuf, String)>,
    /// Folder rename was cancelled
    pub folder_rename_cancelled: bool,
    /// Path of folder whose rename was cancelled (for matching)
    pub folder_rename_cancelled_path: Option<PathBuf>,
    /// Asset was dropped on this folder (target folder, asset id)
    pub asset_dropped: Option<(PathBuf, AssetId)>,
    /// Folder was dropped on this folder (target folder, source folder)
    pub folder_dropped: Option<(PathBuf, PathBuf)>,
    /// Folder is currently highlighted as drop target
    pub drop_target: Option<PathBuf>,
}

/// Renders a folder tree sidebar
pub struct FolderTreeView {
    /// Currently hovered folder (for hover effects)
    hovered_folder: Option<PathBuf>,
    /// Currently dragged folder (tracked for drop detection since egui clears payload on release)
    dragged_folder: Option<PathBuf>,
}

impl Default for FolderTreeView {
    fn default() -> Self {
        Self::new()
    }
}

impl FolderTreeView {
    /// Create a new folder tree view
    pub fn new() -> Self {
        Self {
            hovered_folder: None,
            dragged_folder: None,
        }
    }

    /// Render the folder tree
    ///
    /// Returns FolderTreeResponse with clicked folder and any context menu actions.
    /// `renaming_folder` is Some((path, current_text)) if a folder is being renamed
    /// `icon_manager` is optional - if provided, PNG icons will be used instead of Unicode
    pub fn show(
        &mut self,
        ui: &mut Ui,
        root: &FolderNode,
        current_folder: &PathBuf,
        expanded: &mut HashSet<PathBuf>,
        renaming_folder: &mut Option<(PathBuf, String)>,
        icon_manager: Option<&IconManager>,
    ) -> FolderTreeResponse {
        let mut response = FolderTreeResponse::default();

        // Track the currently dragged folder BEFORE rendering (while egui still has the payload)
        // This is critical because egui clears the payload on mouse release
        if let Some(dragged_path) = egui::DragAndDrop::payload::<PathBuf>(ui.ctx()) {
            self.dragged_folder = Some((*dragged_path).clone());
        }

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                // Render root node
                let node_response = self.render_folder_node(ui, root, current_folder, expanded, 0, renaming_folder, icon_manager);
                if node_response.clicked.is_some() {
                    response.clicked = node_response.clicked;
                }
                if node_response.context_action.is_some() {
                    response.context_action = node_response.context_action;
                }
                if node_response.rename_requested.is_some() {
                    response.rename_requested = node_response.rename_requested;
                }
                if node_response.folder_rename_confirmed.is_some() {
                    response.folder_rename_confirmed = node_response.folder_rename_confirmed;
                }
                if node_response.folder_rename_cancelled {
                    response.folder_rename_cancelled = true;
                    response.folder_rename_cancelled_path = node_response.folder_rename_cancelled_path;
                }
                if node_response.asset_dropped.is_some() {
                    response.asset_dropped = node_response.asset_dropped;
                }
                if node_response.folder_dropped.is_some() {
                    response.folder_dropped = node_response.folder_dropped;
                }
                if node_response.drop_target.is_some() {
                    response.drop_target = node_response.drop_target;
                }
            });

        // Paint floating drag preview near cursor if a folder is being dragged
        if let Some(pointer_pos) = ui.ctx().pointer_hover_pos() {
            if let Some(dragged_path) = egui::DragAndDrop::payload::<PathBuf>(ui.ctx()) {
                // Get folder name from path
                let folder_name = dragged_path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "Folder".to_string());

                // Paint a tooltip-like label at cursor
                let label_text = format!("\u{1F4C1} {}", folder_name);
                let galley = ui.painter().layout_no_wrap(
                    label_text,
                    egui::FontId::proportional(12.0),
                    Color32::WHITE,
                );

                let padding = egui::vec2(6.0, 4.0);
                let label_size = galley.size() + padding * 2.0;
                let label_pos = pointer_pos + egui::vec2(12.0, 12.0); // Offset from cursor
                let label_rect = egui::Rect::from_min_size(label_pos, label_size);

                // Draw background
                ui.painter().rect_filled(
                    label_rect,
                    4.0,
                    Color32::from_rgba_unmultiplied(40, 40, 40, 230),
                );
                // Draw border
                ui.painter().rect_stroke(
                    label_rect,
                    4.0,
                    egui::Stroke::new(1.0, Color32::from_rgb(100, 180, 255)),
                    egui::epaint::StrokeKind::Inside,
                );
                // Draw text
                ui.painter().galley(
                    label_rect.min + padding,
                    galley,
                    Color32::WHITE,
                );
            }
        }

        // Clear dragged_folder after a successful drop or when no drag is active
        if response.folder_dropped.is_some() {
            self.dragged_folder = None;
        } else if egui::DragAndDrop::payload::<PathBuf>(ui.ctx()).is_none() {
            // No active drag, clear the stored folder
            self.dragged_folder = None;
        }

        response
    }

    fn render_folder_node(
        &mut self,
        ui: &mut Ui,
        node: &FolderNode,
        current_folder: &PathBuf,
        expanded: &mut HashSet<PathBuf>,
        depth: usize,
        renaming_folder: &mut Option<(PathBuf, String)>,
        icon_manager: Option<&IconManager>,
    ) -> FolderTreeResponse {
        let mut tree_response = FolderTreeResponse::default();
        let has_children = !node.children.is_empty();
        let is_expanded = expanded.contains(&node.path);
        let is_selected = current_folder == &node.path ||
            (node.path.as_os_str().is_empty() && current_folder.as_os_str().is_empty());
        let is_root = node.path.as_os_str().is_empty();

        // Check if this folder is being renamed
        let is_renaming = renaming_folder.as_ref()
            .map(|(path, _)| path == &node.path)
            .unwrap_or(false);

        // Calculate indentation
        let indent = depth as f32 * 16.0;

        // Context menu action (deferred pattern to avoid closure borrow issues)
        let mut context_action: Option<FolderContextAction> = None;

        // Track if this folder is a drop target
        let mut is_drop_target = false;

        // Icon sizes for folder tree
        let arrow_size = egui::vec2(10.0, 10.0);  // Arrow icons
        let folder_icon_size = egui::vec2(16.0, 16.0);  // Folder icons

        // Render the row content inside horizontal layout
        // Return arrow_clicked from closure to avoid mutable capture issues
        let row_response = ui.horizontal(|ui| {
            ui.add_space(indent);

            // Expand/collapse arrow - capture click state to return
            let arrow_clicked = if has_children {
                render_arrow_icon(ui, is_expanded, icon_manager, arrow_size)
            } else {
                // Allocate invisible placeholder same size as arrow for proper alignment
                ui.allocate_exact_size(arrow_size, egui::Sense::hover());
                false
            };

            // Render folder name (normal label or rename TextEdit)
            if is_renaming {
                // Show folder icon first
                render_folder_icon(ui, is_expanded && has_children, icon_manager, folder_icon_size);

                // Show TextEdit for renaming
                if let Some((rename_path, ref mut rename_text)) = renaming_folder {
                    if rename_path == &node.path {
                        let text_edit_id = ui.make_persistent_id(("folder_rename", node.path.as_os_str()));
                        let text_response = ui.add(
                            egui::TextEdit::singleline(rename_text)
                                .id(text_edit_id)
                                .desired_width(100.0)
                                .font(egui::FontId::proportional(12.0)),
                        );

                        // Request focus on first render
                        if !text_response.has_focus() {
                            text_response.request_focus();
                        }

                        // Handle Enter to confirm WHILE focused (before focus is lost)
                        if text_response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            tree_response.folder_rename_confirmed = Some((node.path.clone(), rename_text.clone()));
                        } else if text_response.lost_focus() {
                            // Lost focus without Enter = cancel
                            tree_response.folder_rename_cancelled = true;
                            tree_response.folder_rename_cancelled_path = Some(node.path.clone());
                        }
                    }
                }
            } else {
                // Normal folder label - render icon and text
                render_folder_icon(ui, is_expanded && has_children, icon_manager, folder_icon_size);
                let text_color = if is_selected { Color32::WHITE } else { ui.style().visuals.text_color() };
                ui.label(RichText::new(&node.name).color(text_color));
            }

            // Return arrow_clicked as the inner value
            arrow_clicked
        });

        // Extract arrow click state from the horizontal response
        let arrow_clicked = row_response.inner;

        // Handle arrow click to toggle expand/collapse
        if arrow_clicked {
            if is_expanded {
                expanded.remove(&node.path);
            } else {
                expanded.insert(node.path.clone());
            }
        }

        // Track if this folder is being dragged (for visual dimming)
        let mut is_being_dragged = false;

        // AFTER horizontal: Create interaction on the row EXCLUDING the arrow area
        // This allows arrow clicks to work (egui processes later interactions first)
        if !is_renaming {
            let row_rect = row_response.response.rect;

            // Create a rect that EXCLUDES the arrow area, so arrow clicks aren't consumed
            let arrow_area_width = indent + arrow_size.x;
            let interaction_rect = egui::Rect::from_min_max(
                egui::pos2(row_rect.min.x + arrow_area_width, row_rect.min.y),
                row_rect.max,
            );

            let folder_id = ui.make_persistent_id(("folder_row", node.path.as_os_str()));
            let response = ui.interact(interaction_rect, folder_id, egui::Sense::click_and_drag());

            // Draw selection highlight over full row
            if response.hovered() || is_selected {
                let bg_color = if is_selected {
                    Color32::from_rgba_unmultiplied(60, 120, 200, 100)
                } else {
                    Color32::from_rgba_unmultiplied(100, 100, 100, 50)
                };
                ui.painter().rect_filled(row_rect, 2.0, bg_color);
            }

            // Handle click - but NOT if arrow was clicked (arrow handles its own toggle)
            if response.clicked() && !arrow_clicked {
                tree_response.clicked = Some(node.path.clone());
            }

            // Double-click to expand/collapse
            if response.double_clicked() && has_children {
                if is_expanded {
                    expanded.remove(&node.path);
                } else {
                    expanded.insert(node.path.clone());
                }
            }

            // Context menu on right-click
            response.context_menu(|ui| {
                // New Folder - always available
                if ui.button("New Folder").clicked() {
                    context_action = Some(FolderContextAction::NewFolder);
                    ui.close();
                }

                // Only show these for non-root folders
                if !is_root {
                    if ui.button("Rename").clicked() {
                        context_action = Some(FolderContextAction::Rename);
                        ui.close();
                    }

                    ui.separator();

                    if ui.button("Reveal in Explorer").clicked() {
                        context_action = Some(FolderContextAction::RevealInExplorer);
                        ui.close();
                    }

                    ui.separator();

                    // Delete button - styled in red
                    if ui.button(RichText::new("Delete").color(Color32::from_rgb(220, 80, 80))).clicked() {
                        context_action = Some(FolderContextAction::Delete);
                        ui.close();
                    }
                } else {
                    // Root folder can still be revealed
                    ui.separator();
                    if ui.button("Reveal in Explorer").clicked() {
                        context_action = Some(FolderContextAction::RevealInExplorer);
                        ui.close();
                    }
                }
            });

            // Show asset count on hover
            if response.hovered() {
                self.hovered_folder = Some(node.path.clone());
            }

            // Make non-root folders draggable
            if !is_root && response.drag_started() {
                response.dnd_set_drag_payload(node.path.clone());
            }

            // Check if this folder is currently being dragged (dim it)
            is_being_dragged = if !is_root {
                if let Some(dragged_path) = egui::DragAndDrop::payload::<PathBuf>(ui.ctx()) {
                    *dragged_path == node.path
                } else {
                    false
                }
            } else {
                false
            };

            // Check for DnD hover - asset being dragged over this folder
            if response.dnd_hover_payload::<AssetId>().is_some() {
                is_drop_target = true;
            }

            // Check for DnD hover - folder being dragged over this folder
            if let Some(dragged_folder) = response.dnd_hover_payload::<PathBuf>() {
                // Don't allow dropping folder onto itself or into its own children
                let dragged_path: &PathBuf = &*dragged_folder;
                if dragged_path != &node.path && !node.path.starts_with(dragged_path) {
                    is_drop_target = true;
                }
            }

            // Check for DnD release - asset was dropped on this folder
            if let Some(dropped_asset_id) = response.dnd_release_payload::<AssetId>() {
                tree_response.asset_dropped = Some((node.path.clone(), *dropped_asset_id));
            }

            // Check for DnD release - folder was dropped on this folder
            // First try the standard dnd_release_payload
            if let Some(dropped_folder) = response.dnd_release_payload::<PathBuf>() {
                // Validate: can't drop into self or children
                let dropped_path: &PathBuf = &*dropped_folder;
                if dropped_path != &node.path && !node.path.starts_with(dropped_path) {
                    tree_response.folder_dropped = Some((node.path.clone(), dropped_path.clone()));
                }
            }

            // Fallback: Check for pointer release while hovering over this folder
            // This handles cases where dnd_release_payload doesn't fire properly
            // We use self.dragged_folder (captured before render) because egui clears payload on release
            if tree_response.folder_dropped.is_none() {
                let pointer_released = ui.ctx().input(|i| i.pointer.any_released());
                if pointer_released {
                    // Use latest_pos - it persists even after pointer release (unlike pointer_hover_pos)
                    if let Some(pointer_pos) = ui.ctx().input(|i| i.pointer.latest_pos()) {
                        let row_rect = row_response.response.rect;
                        if row_rect.contains(pointer_pos) {
                            // Use stored dragged_folder instead of egui payload (which is cleared on release)
                            if let Some(ref dropped_path) = self.dragged_folder {
                                // Validate: can't drop into self or children
                                if dropped_path != &node.path && !node.path.starts_with(dropped_path) {
                                    tree_response.folder_dropped = Some((node.path.clone(), dropped_path.clone()));
                                }
                            }
                        }
                    }
                }
            }

            // Show tooltip with asset count
            response.on_hover_text(format!(
                "{} assets ({} total)",
                node.asset_count, node.total_asset_count
            ));
        }

        // Draw visual feedback for drag-and-drop
        let rect = row_response.response.rect;

        // Dim the folder if it's being dragged
        if is_being_dragged {
            ui.painter().rect_filled(
                rect,
                2.0,
                Color32::from_rgba_unmultiplied(80, 80, 80, 150),
            );
        }

        // Highlight drop target with filled background and border
        if is_drop_target {
            // Semi-transparent blue fill
            ui.painter().rect_filled(
                rect,
                2.0,
                Color32::from_rgba_unmultiplied(100, 180, 255, 60),
            );
            // Blue border
            ui.painter().rect_stroke(
                rect,
                2.0,
                egui::Stroke::new(2.0, Color32::from_rgb(100, 180, 255)),
                egui::epaint::StrokeKind::Inside,
            );
            tree_response.drop_target = Some(node.path.clone());
        }

        // Handle deferred context action
        if let Some(action) = context_action {
            match &action {
                FolderContextAction::Rename => {
                    tree_response.rename_requested = Some((node.path.clone(), node.name.clone()));
                }
                _ => {}
            }
            tree_response.context_action = Some((node.path.clone(), action));
        }

        // Render children if expanded
        if is_expanded {
            for child in &node.children {
                let child_response = self.render_folder_node(ui, child, current_folder, expanded, depth + 1, renaming_folder, icon_manager);
                if child_response.clicked.is_some() {
                    tree_response.clicked = child_response.clicked;
                }
                if child_response.context_action.is_some() {
                    tree_response.context_action = child_response.context_action;
                }
                if child_response.rename_requested.is_some() {
                    tree_response.rename_requested = child_response.rename_requested;
                }
                if child_response.folder_rename_confirmed.is_some() {
                    tree_response.folder_rename_confirmed = child_response.folder_rename_confirmed;
                }
                if child_response.folder_rename_cancelled {
                    tree_response.folder_rename_cancelled = true;
                    tree_response.folder_rename_cancelled_path = child_response.folder_rename_cancelled_path;
                }
                if child_response.asset_dropped.is_some() {
                    tree_response.asset_dropped = child_response.asset_dropped;
                }
                if child_response.folder_dropped.is_some() {
                    tree_response.folder_dropped = child_response.folder_dropped;
                }
                if child_response.drop_target.is_some() {
                    tree_response.drop_target = child_response.drop_target;
                }
            }
        }

        tree_response
    }
}

/// Render a folder icon (PNG or Unicode fallback)
fn render_folder_icon(ui: &mut Ui, is_open: bool, icon_manager: Option<&IconManager>, size: egui::Vec2) {
    let icon_type = if is_open {
        AssetBrowserIcon::FolderOpen
    } else {
        AssetBrowserIcon::Folder
    };

    if let Some(manager) = icon_manager {
        if let Some(texture) = manager.get_asset_icon(icon_type) {
            ui.image((texture.id(), size));
            return;
        }
    }

    // Fallback to Unicode
    let fallback = if is_open { "\u{1F4C2}" } else { "\u{1F4C1}" }; // 📂 or 📁
    ui.label(fallback);
}

/// Render an expand/collapse arrow icon (PNG or Unicode fallback)
/// Returns true if clicked
fn render_arrow_icon(ui: &mut Ui, is_expanded: bool, icon_manager: Option<&IconManager>, size: egui::Vec2) -> bool {
    let icon_type = if is_expanded {
        AssetBrowserIcon::ArrowDown
    } else {
        AssetBrowserIcon::ArrowRight
    };

    if let Some(manager) = icon_manager {
        if let Some(texture) = manager.get_asset_icon(icon_type) {
            let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
            if ui.is_rect_visible(rect) {
                ui.painter().image(
                    texture.id(),
                    rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    Color32::from_gray(200),
                );
            }
            return response.clicked();
        }
    }

    // Fallback to Unicode button
    let arrow = if is_expanded { "\u{25BC}" } else { "\u{25B6}" }; // ▼ or ▶
    let arrow_response = ui.add(
        egui::Button::new(RichText::new(arrow).size(10.0))
            .frame(false)
            .min_size(size),
    );
    arrow_response.clicked()
}
