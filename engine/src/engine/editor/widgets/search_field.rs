//! Search field widget with clear button and underline animation.

use egui::{Response, Ui};

use super::UiExt;

/// Render a search field with a clear button.
///
/// Returns the text edit response. The `query` string is mutated in place.
pub fn search_field(ui: &mut Ui, query: &mut String) -> Response {
    let theme = ui.theme();
    let id = ui.make_persistent_id("search_field");

    let response = ui.horizontal(|ui| {
        // Search icon
        ui.label(
            egui::RichText::new("\u{1F50D}")
                .color(theme.palette.text_secondary)
                .size(12.0),
        );

        // Text input
        let te_response = ui.add(
            egui::TextEdit::singleline(query)
                .hint_text("Search…")
                .desired_width(ui.available_width() - 20.0),
        );

        // Clear button (only when text present)
        if !query.is_empty() {
            let clear = ui.small_button("\u{2715}"); // ✕
            if clear.clicked() {
                query.clear();
                te_response.request_focus();
            }
        }

        te_response
    });

    let te_response = response.inner;

    // Animated underline when focused
    if te_response.has_focus() {
        let focus_anim = ui.ctx().animate_bool(id, true);
        let rect = te_response.rect;
        let line_width = rect.width() * focus_anim;
        let center_x = rect.center().x;

        ui.painter().line_segment(
            [
                egui::pos2(center_x - line_width / 2.0, rect.bottom()),
                egui::pos2(center_x + line_width / 2.0, rect.bottom()),
            ],
            egui::Stroke::new(2.0, theme.palette.accent),
        );
    } else {
        // Animate out
        ui.ctx().animate_bool(id, false);
    }

    te_response
}
