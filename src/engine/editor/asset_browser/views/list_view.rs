//! List view for asset browser
//!
//! Displays assets in a table format with columns for name, type, size, and date.

use crate::engine::assets::{AssetId, AssetMetadata, AssetType};
use crate::engine::editor::asset_browser::selection::AssetSelection;
use crate::engine::editor::icons::{AssetBrowserIcon, IconManager};
use egui::{Color32, RichText, Ui};
use std::time::SystemTime;

/// Context menu action for deferred handling
enum ContextAction {
    Open,
    Rename,
    RevealInExplorer,
    Delete,
}

/// Response from list view interactions
#[derive(Debug, Default)]
pub struct ListViewResponse {
    /// Asset that was clicked
    pub clicked: Option<AssetId>,
    /// Asset that was double-clicked
    pub double_clicked: Option<AssetId>,
    /// Asset that was right-clicked (context menu)
    pub context_menu: Option<AssetId>,
    /// Asset being dragged
    pub drag_started: Option<AssetId>,
    /// Request to start renaming an asset
    pub rename_requested: Option<AssetId>,
    /// Rename was confirmed (asset id, new name without extension)
    pub rename_confirmed: Option<(AssetId, String)>,
    /// Rename was cancelled
    pub rename_cancelled: bool,
    /// Request to reveal asset in file explorer
    pub reveal_in_explorer: Option<AssetId>,
    /// Request to delete an asset
    pub delete_requested: Option<AssetId>,
}

/// Column to sort by
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortColumn {
    #[default]
    Name,
    Type,
    Size,
    DateModified,
}

/// List view for displaying assets in table format
pub struct ListView {
    /// Row height
    pub row_height: f32,
    /// Current sort column
    pub sort_column: SortColumn,
    /// Sort ascending
    pub sort_ascending: bool,
}

impl Default for ListView {
    fn default() -> Self {
        Self::new()
    }
}

impl ListView {
    /// Create a new list view
    pub fn new() -> Self {
        Self {
            row_height: 24.0,
            sort_column: SortColumn::Name,
            sort_ascending: true,
        }
    }

    /// Render the list view
    ///
    /// `renaming_asset` is Some((id, current_text)) if an asset is being renamed
    /// `icon_manager` is optional - if provided, PNG icons will be used for file types
    pub fn show(
        &mut self,
        ui: &mut Ui,
        assets: &[&AssetMetadata],
        selection: &mut AssetSelection,
        renaming_asset: &mut Option<(AssetId, String)>,
        icon_manager: Option<&IconManager>,
    ) -> ListViewResponse {
        let mut response = ListViewResponse::default();

        if assets.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label(RichText::new("No assets in this folder").weak());
            });
            return response;
        }

        // Header row
        ui.horizontal(|ui| {
            self.render_header(ui, "Name", SortColumn::Name, 200.0);
            self.render_header(ui, "Type", SortColumn::Type, 80.0);
            self.render_header(ui, "Size", SortColumn::Size, 80.0);
            self.render_header(ui, "Modified", SortColumn::DateModified, 120.0);
        });

        ui.separator();

        // Asset rows with virtualization
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, self.row_height, assets.len(), |ui, row_range| {
                for row in row_range {
                    if row >= assets.len() {
                        break;
                    }

                    let asset = assets[row];

                    // Check if this asset is being renamed
                    let is_renaming = renaming_asset.as_ref()
                        .map(|(id, _)| *id == asset.id)
                        .unwrap_or(false);

                    if let Some(r) = self.render_row(ui, asset, selection, is_renaming, renaming_asset, icon_manager) {
                        if r.clicked.is_some() {
                            response.clicked = r.clicked;
                        }
                        if r.double_clicked.is_some() {
                            response.double_clicked = r.double_clicked;
                        }
                        if r.context_menu.is_some() {
                            response.context_menu = r.context_menu;
                        }
                        if r.drag_started.is_some() {
                            response.drag_started = r.drag_started;
                        }
                        if r.rename_requested.is_some() {
                            response.rename_requested = r.rename_requested;
                        }
                        if r.rename_confirmed.is_some() {
                            response.rename_confirmed = r.rename_confirmed;
                        }
                        if r.rename_cancelled {
                            response.rename_cancelled = true;
                        }
                        if r.reveal_in_explorer.is_some() {
                            response.reveal_in_explorer = r.reveal_in_explorer;
                        }
                        if r.delete_requested.is_some() {
                            response.delete_requested = r.delete_requested;
                        }
                    }
                }
            });

        response
    }

    fn render_header(&mut self, ui: &mut Ui, label: &str, column: SortColumn, width: f32) {
        let is_sorted = self.sort_column == column;
        let arrow = if is_sorted {
            if self.sort_ascending {
                " \u{25B2}" // ▲
            } else {
                " \u{25BC}" // ▼
            }
        } else {
            ""
        };

        let text = format!("{}{}", label, arrow);
        let text = if is_sorted {
            RichText::new(text).strong()
        } else {
            RichText::new(text)
        };

        let response = ui.add_sized(
            egui::vec2(width, 20.0),
            egui::Button::new(text).frame(false),
        );

        if response.clicked() {
            if self.sort_column == column {
                self.sort_ascending = !self.sort_ascending;
            } else {
                self.sort_column = column;
                self.sort_ascending = true;
            }
        }
    }

    fn render_row(
        &self,
        ui: &mut Ui,
        asset: &AssetMetadata,
        selection: &AssetSelection,
        is_renaming: bool,
        renaming_text: &mut Option<(AssetId, String)>,
        icon_manager: Option<&IconManager>,
    ) -> Option<ListViewResponse> {
        let mut response = ListViewResponse::default();
        let is_selected = selection.is_selected(asset.id);

        let row_response = ui.horizontal(|ui| {
            // Background for selection
            let rect = ui.available_rect_before_wrap();
            let row_rect = egui::Rect::from_min_size(
                rect.min,
                egui::vec2(ui.available_width(), self.row_height),
            );

            if is_selected {
                ui.painter().rect_filled(
                    row_rect,
                    0.0,
                    Color32::from_rgba_unmultiplied(60, 120, 200, 100),
                );
            }

            // Icon and name - with rename support
            if is_renaming {
                // Show text edit for renaming
                if let Some((rename_id, ref mut rename_text)) = renaming_text {
                    if *rename_id == asset.id {
                        // Icon first (PNG or fallback)
                        render_file_type_icon(ui, asset.asset_type, icon_manager);

                        // Text edit for name
                        let text_edit_id = ui.make_persistent_id(("rename", asset.id.0));
                        let text_edit_response = ui.add_sized(
                            egui::vec2(180.0, self.row_height - 4.0),
                            egui::TextEdit::singleline(rename_text)
                                .id(text_edit_id)
                                .font(egui::FontId::proportional(12.0)),
                        );

                        // Request focus on first render
                        if !text_edit_response.has_focus() {
                            text_edit_response.request_focus();
                        }

                        // Handle Enter to confirm WHILE focused (before focus is lost)
                        if text_edit_response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            response.rename_confirmed = Some((asset.id, rename_text.clone()));
                        } else if text_edit_response.lost_focus() {
                            // Lost focus without Enter = cancel
                            response.rename_cancelled = true;
                        }
                    }
                }
            } else {
                // Normal label rendering with icon
                render_file_type_icon(ui, asset.asset_type, icon_manager);
                ui.add_sized(
                    egui::vec2(184.0, self.row_height),  // Slightly smaller to account for icon
                    egui::Label::new(
                        RichText::new(&asset.display_name).color(if is_selected {
                            Color32::WHITE
                        } else {
                            Color32::from_gray(220)
                        }),
                    )
                    .truncate(),
                );
            }

            // Type
            ui.add_sized(
                egui::vec2(80.0, self.row_height),
                egui::Label::new(
                    RichText::new(asset.asset_type.display_name())
                        .color(get_type_color(asset.asset_type))
                        .small(),
                ),
            );

            // Size
            ui.add_sized(
                egui::vec2(80.0, self.row_height),
                egui::Label::new(RichText::new(asset.formatted_size()).weak().small()),
            );

            // Modified date
            let date_str = format_date(asset.last_modified);
            ui.add_sized(
                egui::vec2(120.0, self.row_height),
                egui::Label::new(RichText::new(date_str).weak().small()),
            );
        });

        let row_response = row_response.response;

        // Interaction
        let sense_response = ui.interact(
            row_response.rect,
            ui.make_persistent_id(asset.id.0),
            egui::Sense::click_and_drag(),
        );

        if sense_response.clicked() {
            response.clicked = Some(asset.id);
        }

        if sense_response.double_clicked() {
            response.double_clicked = Some(asset.id);
        }

        // Context menu on right-click - use deferred action pattern
        // because closure mutations don't propagate to the response struct
        let mut context_action: Option<ContextAction> = None;

        sense_response.context_menu(|ui| {
            if ui.button("Open").clicked() {
                context_action = Some(ContextAction::Open);
                ui.close_menu();
            }

            if ui.button("Rename").clicked() {
                context_action = Some(ContextAction::Rename);
                ui.close_menu();
            }

            ui.separator();

            if ui.button("Reveal in Explorer").clicked() {
                context_action = Some(ContextAction::RevealInExplorer);
                ui.close_menu();
            }

            if ui.button("Copy Path").clicked() {
                ui.ctx().copy_text(asset.path.to_string_lossy().to_string());
                ui.close_menu();
            }

            ui.separator();

            if ui.button(RichText::new("Delete").color(Color32::from_rgb(220, 80, 80))).clicked() {
                context_action = Some(ContextAction::Delete);
                ui.close_menu();
            }
        });

        // Handle deferred context menu action
        if let Some(action) = context_action {
            response.context_menu = Some(asset.id);
            match action {
                ContextAction::Open => {
                    response.double_clicked = Some(asset.id);
                }
                ContextAction::Rename => {
                    response.rename_requested = Some(asset.id);
                }
                ContextAction::RevealInExplorer => {
                    response.reveal_in_explorer = Some(asset.id);
                }
                ContextAction::Delete => {
                    response.delete_requested = Some(asset.id);
                }
            }
        }

        // Drag start - use egui's DnD system for reliable drop detection
        if sense_response.drag_started() {
            response.drag_started = Some(asset.id);
            // Set the DnD payload so folder tree can detect it
            sense_response.dnd_set_drag_payload(asset.id);
        }

        Some(response)
    }
}

fn get_type_icon(asset_type: AssetType) -> &'static str {
    match asset_type {
        AssetType::Texture => "\u{1F5BC}",
        AssetType::Model => "\u{1F4E6}",
        AssetType::Scene => "\u{1F3AC}",
        AssetType::Material => "\u{1F3A8}",
        AssetType::Audio => "\u{1F50A}",
        AssetType::Shader => "\u{2728}",
        AssetType::Prefab => "\u{1F4CB}",
        AssetType::Unknown => "\u{2753}",
    }
}

fn get_type_color(asset_type: AssetType) -> Color32 {
    match asset_type {
        AssetType::Texture => Color32::from_rgb(100, 180, 100),
        AssetType::Model => Color32::from_rgb(100, 150, 220),
        AssetType::Scene => Color32::from_rgb(220, 180, 100),
        AssetType::Material => Color32::from_rgb(200, 100, 180),
        AssetType::Audio => Color32::from_rgb(220, 120, 100),
        AssetType::Shader => Color32::from_rgb(150, 220, 220),
        AssetType::Prefab => Color32::from_rgb(180, 180, 100),
        AssetType::Unknown => Color32::from_gray(150),
    }
}

fn format_date(time: SystemTime) -> String {
    // Simple date formatting
    match time.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(duration) => {
            let secs = duration.as_secs();
            // Convert to rough date (very simplified)
            let days = secs / 86400;
            let years = days / 365;
            let year = 1970 + years;
            let remaining_days = days % 365;
            let month = remaining_days / 30 + 1;
            let day = remaining_days % 30 + 1;

            format!("{:04}-{:02}-{:02}", year, month.min(12), day.min(31))
        }
        Err(_) => "Unknown".to_string(),
    }
}

/// Map asset type to asset browser icon type
fn get_asset_browser_icon(asset_type: AssetType) -> Option<AssetBrowserIcon> {
    match asset_type {
        AssetType::Texture => Some(AssetBrowserIcon::FileImage),
        AssetType::Model => Some(AssetBrowserIcon::FileMesh),
        AssetType::Scene => Some(AssetBrowserIcon::FileDocument),
        AssetType::Material => Some(AssetBrowserIcon::FileDocument),
        AssetType::Audio => Some(AssetBrowserIcon::FileDocument),
        AssetType::Shader => Some(AssetBrowserIcon::FileCode),
        AssetType::Prefab => Some(AssetBrowserIcon::FileDocument),
        AssetType::Unknown => Some(AssetBrowserIcon::FileDocument),
    }
}

/// Render a file type icon (PNG or Unicode fallback)
fn render_file_type_icon(ui: &mut Ui, asset_type: AssetType, icon_manager: Option<&IconManager>) {
    let icon_size = egui::vec2(16.0, 16.0);

    if let Some(manager) = icon_manager {
        if let Some(icon_type) = get_asset_browser_icon(asset_type) {
            if let Some(texture) = manager.get_asset_icon(icon_type) {
                ui.image((texture.id(), icon_size));
                return;
            }
        }
    }

    // Fallback to Unicode
    let fallback = get_type_icon(asset_type);
    ui.label(fallback);
}
