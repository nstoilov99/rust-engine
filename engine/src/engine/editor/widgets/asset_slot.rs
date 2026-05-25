//! Asset slot widget — drop target for asset references in the inspector.

use egui::{Response, Ui};

use super::UiExt;

/// Render an asset slot. Shows the asset name/thumbnail when populated,
/// or an empty state placeholder when empty.
///
/// Returns the response for drag-drop detection.
pub fn asset_slot(
    ui: &mut Ui,
    label: &str,
    current_asset: Option<&str>,
) -> AssetSlotResponse {
    let theme = ui.theme();
    let height = 32.0;

    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height),
        egui::Sense::click_and_drag(),
    );

    let mut cleared = false;

    if ui.is_rect_visible(rect) {
        let painter = ui.painter();

        // Background
        let bg = if response.hovered() {
            theme.palette.surface[3]
        } else {
            theme.palette.surface[1]
        };
        painter.rect_filled(rect, 4, bg);

        // Border (dashed when empty, solid when populated)
        let stroke_color = if current_asset.is_some() {
            theme.palette.stroke
        } else {
            theme.palette.text_disabled
        };
        painter.rect_stroke(
            rect,
            4,
            egui::Stroke::new(1.0, stroke_color),
            egui::StrokeKind::Inside,
        );

        // Content
        if let Some(asset_name) = current_asset {
            // Show asset name
            painter.text(
                egui::pos2(rect.left() + 8.0, rect.center().y),
                egui::Align2::LEFT_CENTER,
                asset_name,
                egui::FontId::proportional(11.0),
                theme.palette.text_primary,
            );

            // Clear button on hover
            if response.hovered() {
                let clear_rect = egui::Rect::from_min_size(
                    egui::pos2(rect.right() - 20.0, rect.top()),
                    egui::vec2(20.0, height),
                );
                painter.text(
                    clear_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "\u{2715}",
                    egui::FontId::proportional(10.0),
                    theme.palette.text_secondary,
                );

                // Check click on clear area
                if response.clicked() {
                    if let Some(pos) = response.interact_pointer_pos() {
                        if clear_rect.contains(pos) {
                            cleared = true;
                        }
                    }
                }
            }
        } else {
            // Empty placeholder
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                format!("Drop {label} here…"),
                egui::FontId::proportional(11.0),
                theme.palette.text_disabled,
            );
        }

        // Focus ring
        if response.has_focus() {
            painter.rect_stroke(
                rect.expand(1.0),
                4,
                egui::Stroke::new(1.5, theme.palette.focus_ring),
                egui::StrokeKind::Outside,
            );
        }
    }

    AssetSlotResponse {
        response,
        cleared,
    }
}

/// Response from an asset slot widget.
pub struct AssetSlotResponse {
    /// The egui response for the slot area.
    pub response: Response,
    /// Whether the clear button was clicked.
    pub cleared: bool,
}
