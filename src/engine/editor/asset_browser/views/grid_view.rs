//! Grid view for asset browser
//!
//! Displays assets as a grid of cards with thumbnails and names.
//! Implements virtualization for large asset collections.

use crate::engine::assets::{AssetId, AssetMetadata, AssetType};
use crate::engine::editor::asset_browser::selection::AssetSelection;
use crate::engine::editor::asset_browser::thumbnail::ThumbnailCache;
use crate::engine::editor::icons::{AssetBrowserIcon, IconManager};
use egui::{Color32, Pos2, Rect, RichText, Sense, Stroke, Ui, Vec2};

/// Context menu action for deferred handling
enum ContextAction {
    Open,
    Rename,
    RevealInExplorer,
    Delete,
}

/// Response from grid view interactions
#[derive(Debug, Default)]
pub struct GridViewResponse {
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

/// Grid view for displaying assets
pub struct GridView {
    /// Size of each grid item (width = height for thumbnails)
    pub item_size: f32,
    /// Spacing between items
    pub spacing: f32,
    /// Padding inside each card
    pub padding: f32,
    /// Height reserved for the label below thumbnail
    pub label_height: f32,
}

impl Default for GridView {
    fn default() -> Self {
        Self::new(96.0)
    }
}

impl GridView {
    /// Create a new grid view with the given item size
    pub fn new(item_size: f32) -> Self {
        Self {
            item_size,
            spacing: 8.0,
            padding: 4.0,
            label_height: 20.0,
        }
    }

    /// Get the total height of a single card (thumbnail + label)
    pub fn card_height(&self) -> f32 {
        self.item_size + self.label_height + self.padding * 2.0
    }

    /// Get the total width of a single card
    pub fn card_width(&self) -> f32 {
        self.item_size + self.padding * 2.0
    }

    /// Render the grid view
    ///
    /// `renaming_asset` is Some((id, current_text)) if an asset is being renamed
    /// `icon_manager` is optional - if provided, PNG icons will be used for file types
    pub fn show(
        &mut self,
        ui: &mut Ui,
        assets: &[&AssetMetadata],
        thumbnails: &mut ThumbnailCache,
        selection: &mut AssetSelection,
        renaming_asset: &mut Option<(AssetId, String)>,
        icon_manager: Option<&IconManager>,
    ) -> GridViewResponse {
        let mut response = GridViewResponse::default();

        if assets.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label(RichText::new("No assets in this folder").weak());
            });
            return response;
        }

        let available_width = ui.available_width();
        let card_width = self.card_width() + self.spacing;
        let columns = ((available_width / card_width).floor() as usize).max(1);
        let rows = assets.len().div_ceil(columns);

        let card_height = self.card_height() + self.spacing;
        let total_height = rows as f32 * card_height;

        // Use ScrollArea with virtualization
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_viewport(ui, |ui, viewport| {
                // Reserve space for all items
                let (_, painter_rect) =
                    ui.allocate_space(Vec2::new(columns as f32 * card_width, total_height));

                // Calculate visible row range
                let start_row = (viewport.top() / card_height).floor() as usize;
                let end_row = ((viewport.bottom() / card_height).ceil() as usize + 1).min(rows);

                // Render only visible items
                for row in start_row..end_row {
                    for col in 0..columns {
                        let idx = row * columns + col;
                        if idx >= assets.len() {
                            break;
                        }

                        let asset = assets[idx];
                        let x = painter_rect.min.x + col as f32 * card_width;
                        let y = painter_rect.min.y + row as f32 * card_height;

                        let card_rect = Rect::from_min_size(
                            Pos2::new(x, y),
                            Vec2::new(self.card_width(), self.card_height()),
                        );

                        // Check if this asset is being renamed
                        let is_renaming = renaming_asset
                            .as_ref()
                            .map(|(id, _)| *id == asset.id)
                            .unwrap_or(false);

                        if let Some(r) = self.render_asset_card(
                            ui,
                            card_rect,
                            asset,
                            thumbnails,
                            selection,
                            is_renaming,
                            renaming_asset,
                            icon_manager,
                        ) {
                            // Merge responses
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
                }
            });

        response
    }

    #[allow(clippy::too_many_arguments)]
    fn render_asset_card(
        &self,
        ui: &mut Ui,
        rect: Rect,
        asset: &AssetMetadata,
        thumbnails: &mut ThumbnailCache,
        selection: &AssetSelection,
        is_renaming: bool,
        renaming_text: &mut Option<(AssetId, String)>,
        icon_manager: Option<&IconManager>,
    ) -> Option<GridViewResponse> {
        let mut response = GridViewResponse::default();

        // Only render if visible
        if !ui.is_rect_visible(rect) {
            return None;
        }

        let is_selected = selection.is_selected(asset.id);
        let id = ui.make_persistent_id(asset.id.0);

        // Create interaction area
        let sense = Sense::click_and_drag();
        let ui_response = ui.interact(rect, id, sense);

        let painter = ui.painter();

        // Background
        let bg_color = if is_selected {
            Color32::from_rgba_unmultiplied(60, 120, 200, 100)
        } else if ui_response.hovered() {
            Color32::from_rgba_unmultiplied(100, 100, 100, 50)
        } else {
            Color32::TRANSPARENT
        };

        painter.rect_filled(rect, 4.0, bg_color);

        // Selection border
        if is_selected {
            painter.rect_stroke(
                rect,
                4.0,
                Stroke::new(2.0, Color32::from_rgb(60, 120, 200)),
                egui::epaint::StrokeKind::Outside,
            );
        }

        // Thumbnail area
        let thumb_rect = Rect::from_min_size(
            Pos2::new(rect.min.x + self.padding, rect.min.y + self.padding),
            Vec2::new(self.item_size, self.item_size),
        );

        // Draw thumbnail or placeholder
        if let Some(texture_id) = thumbnails.get_texture_id(ui.ctx(), asset) {
            painter.image(
                texture_id,
                thumb_rect,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        } else {
            // Draw type icon as placeholder
            painter.rect_filled(thumb_rect, 2.0, Color32::from_gray(40));

            // Try to use PNG icon, fall back to Unicode
            let icon_rendered = if let Some(manager) = icon_manager {
                if let Some(icon_type) = get_asset_browser_icon(asset.asset_type) {
                    if let Some(texture) = manager.get_asset_icon(icon_type) {
                        // Center the icon in the thumbnail area (32x32 icon)
                        let icon_size = Vec2::new(32.0, 32.0);
                        let icon_rect = Rect::from_center_size(thumb_rect.center(), icon_size);
                        painter.image(
                            texture.id(),
                            icon_rect,
                            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                            Color32::from_gray(180),
                        );
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            } else {
                false
            };

            if !icon_rendered {
                let icon = get_type_icon(asset.asset_type);
                painter.text(
                    thumb_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    icon,
                    egui::FontId::proportional(32.0),
                    Color32::from_gray(120),
                );
            }
        }

        // Asset type badge (small icon in corner)
        let badge_size = 18.0;
        let badge_pos = Pos2::new(
            thumb_rect.max.x - badge_size - 2.0,
            thumb_rect.max.y - badge_size - 2.0,
        );
        let badge_rect = Rect::from_min_size(badge_pos, Vec2::new(badge_size, badge_size));
        painter.rect_filled(
            badge_rect,
            2.0,
            Color32::from_rgba_unmultiplied(0, 0, 0, 180),
        );

        // Try to render PNG icon for badge, fall back to text
        let badge_icon_rendered = if let Some(manager) = icon_manager {
            if let Some(icon_type) = get_asset_browser_icon(asset.asset_type) {
                if let Some(texture) = manager.get_asset_icon(icon_type) {
                    let icon_rect = badge_rect.shrink(2.0);
                    painter.image(
                        texture.id(),
                        icon_rect,
                        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                        Color32::WHITE,
                    );
                    true
                } else {
                    false
                }
            } else {
                false
            }
        } else {
            false
        };

        if !badge_icon_rendered {
            painter.text(
                badge_rect.center(),
                egui::Align2::CENTER_CENTER,
                get_type_icon_small(asset.asset_type),
                egui::FontId::proportional(10.0),
                get_type_color(asset.asset_type),
            );
        }

        // Label area
        let label_rect = Rect::from_min_size(
            Pos2::new(
                rect.min.x + self.padding,
                rect.min.y + self.padding + self.item_size + 2.0,
            ),
            Vec2::new(self.item_size, self.label_height),
        );

        // Render label or rename text edit
        if is_renaming {
            // Show text edit for renaming
            if let Some((rename_id, ref mut rename_text)) = renaming_text {
                if *rename_id == asset.id {
                    // Create a child UI for the text edit at the label position
                    let text_edit_id = ui.make_persistent_id(("rename", asset.id.0));
                    let mut text_edit_rect = label_rect;
                    text_edit_rect.set_height(18.0);

                    let text_response = ui.put(
                        text_edit_rect,
                        egui::TextEdit::singleline(rename_text)
                            .id(text_edit_id)
                            .desired_width(self.item_size - 4.0)
                            .font(egui::FontId::proportional(11.0))
                            .horizontal_align(egui::Align::Center),
                    );

                    // Request focus on first render
                    if !text_response.has_focus() {
                        text_response.request_focus();
                    }

                    // Handle Enter to confirm WHILE focused (before focus is lost)
                    if text_response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        response.rename_confirmed = Some((asset.id, rename_text.clone()));
                    } else if text_response.lost_focus() {
                        // Lost focus without Enter = cancel
                        response.rename_cancelled = true;
                    }
                }
            }
        } else {
            // Normal label rendering - truncate if too long
            let label_text = truncate_label(&asset.display_name, self.item_size, 11.0);
            painter.text(
                Pos2::new(label_rect.center().x, label_rect.min.y + 10.0),
                egui::Align2::CENTER_CENTER,
                &label_text,
                egui::FontId::proportional(11.0),
                if is_selected {
                    Color32::WHITE
                } else {
                    Color32::from_gray(200)
                },
            );
        }

        // Handle interactions
        if ui_response.clicked() {
            response.clicked = Some(asset.id);
        }

        if ui_response.double_clicked() {
            response.double_clicked = Some(asset.id);
        }

        // Context menu on right-click - use deferred action pattern
        // because closure mutations don't propagate to the response struct
        let mut context_action: Option<ContextAction> = None;

        ui_response.context_menu(|ui| {
            if ui.button("Open").clicked() {
                context_action = Some(ContextAction::Open);
                ui.close();
            }

            if ui.button("Rename").clicked() {
                context_action = Some(ContextAction::Rename);
                ui.close();
            }

            ui.separator();

            if ui.button("Reveal in Explorer").clicked() {
                context_action = Some(ContextAction::RevealInExplorer);
                ui.close();
            }

            if ui.button("Copy Path").clicked() {
                ui.ctx().copy_text(asset.path.to_string_lossy().to_string());
                ui.close();
            }

            ui.separator();

            if ui
                .button(RichText::new("Delete").color(Color32::from_rgb(220, 80, 80)))
                .clicked()
            {
                context_action = Some(ContextAction::Delete);
                ui.close();
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
        if ui_response.drag_started() {
            response.drag_started = Some(asset.id);
            // Set the DnD payload so folder tree can detect it
            ui_response.dnd_set_drag_payload(asset.id);
        }

        // Tooltip on hover (context_menu auto-closes tooltip)
        if ui_response.hovered() {
            egui::containers::Tooltip::always_open(
                ui.ctx().clone(),
                ui.layer_id(),
                egui::Id::new("asset_tooltip"),
                egui::containers::PopupAnchor::Pointer,
            )
            .show(|ui| {
                ui.label(&asset.display_name);
                ui.label(RichText::new(asset.asset_type.display_name()).weak());
                ui.label(RichText::new(format!("Size: {}", asset.formatted_size())).weak());
                ui.label(RichText::new(format!("Path: {}", asset.path.display())).weak());
            });
        }

        Some(response)
    }
}

/// Get icon character for asset type
fn get_type_icon(asset_type: AssetType) -> &'static str {
    match asset_type {
        AssetType::Texture => "\u{1F5BC}",  // 🖼
        AssetType::Model => "\u{1F4E6}",    // 📦
        AssetType::Scene => "\u{1F3AC}",    // 🎬
        AssetType::Material => "\u{1F3A8}", // 🎨
        AssetType::Audio => "\u{1F50A}",    // 🔊
        AssetType::Shader => "\u{2728}",    // ✨
        AssetType::Prefab => "\u{1F4CB}",   // 📋
        AssetType::Unknown => "\u{2753}",   // ❓
    }
}

/// Get small letter icon for badge
fn get_type_icon_small(asset_type: AssetType) -> &'static str {
    match asset_type {
        AssetType::Texture => "T",
        AssetType::Model => "M",
        AssetType::Scene => "S",
        AssetType::Material => "Ma",
        AssetType::Audio => "A",
        AssetType::Shader => "Sh",
        AssetType::Prefab => "P",
        AssetType::Unknown => "?",
    }
}

/// Get color for asset type
fn get_type_color(asset_type: AssetType) -> Color32 {
    match asset_type {
        AssetType::Texture => Color32::from_rgb(100, 180, 100), // Green
        AssetType::Model => Color32::from_rgb(100, 150, 220),   // Blue
        AssetType::Scene => Color32::from_rgb(220, 180, 100),   // Gold
        AssetType::Material => Color32::from_rgb(200, 100, 180), // Purple
        AssetType::Audio => Color32::from_rgb(220, 120, 100),   // Orange-red
        AssetType::Shader => Color32::from_rgb(150, 220, 220),  // Cyan
        AssetType::Prefab => Color32::from_rgb(180, 180, 100),  // Olive
        AssetType::Unknown => Color32::from_gray(150),
    }
}

/// Truncate label to fit within width
fn truncate_label(text: &str, max_width: f32, font_size: f32) -> String {
    // Rough estimate: each character is about 0.6 * font_size wide
    let char_width = font_size * 0.55;
    let max_chars = (max_width / char_width) as usize;

    if text.len() <= max_chars {
        text.to_string()
    } else if max_chars > 3 {
        format!("{}...", &text[..max_chars - 3])
    } else {
        "...".to_string()
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
