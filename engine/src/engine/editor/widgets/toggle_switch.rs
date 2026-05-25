//! Animated toggle switch widget.

use egui::{pos2, vec2, Response, Sense, Ui};

use super::UiExt;

/// Render an animated toggle switch. Returns the response; `on` is mutated on click.
///
/// Keyboard contract: focusable, Enter toggles, Tab cycles focus.
pub fn toggle_switch(ui: &mut Ui, on: &mut bool) -> Response {
    let theme = ui.theme();
    let desired_size = ui.spacing().interact_size.y * vec2(2.0, 1.0);
    let (rect, mut response) = ui.allocate_exact_size(desired_size, Sense::click());

    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }

    // Also handle Enter key when focused
    if response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
        *on = !*on;
        response.mark_changed();
    }

    if ui.is_rect_visible(rect) {
        let how_on = ui.ctx().animate_bool(response.id, *on);

        // Background track
        let bg_color = egui::Color32::from_rgba_unmultiplied(
            ((1.0 - how_on) * theme.palette.surface[2].r() as f32
                + how_on * theme.palette.accent.r() as f32) as u8,
            ((1.0 - how_on) * theme.palette.surface[2].g() as f32
                + how_on * theme.palette.accent.g() as f32) as u8,
            ((1.0 - how_on) * theme.palette.surface[2].b() as f32
                + how_on * theme.palette.accent.b() as f32) as u8,
            255,
        );
        ui.painter()
            .rect_filled(rect, rect.height() / 2.0, bg_color);

        // Thumb
        let thumb_x = egui::lerp(
            rect.left() + rect.height() / 2.0..=rect.right() - rect.height() / 2.0,
            how_on,
        );
        let thumb_center = pos2(thumb_x, rect.center().y);
        ui.painter().circle_filled(
            thumb_center,
            rect.height() / 2.5,
            theme.palette.text_primary,
        );

        // Focus ring
        if response.has_focus() {
            ui.painter().rect_stroke(
                rect.expand(1.0),
                rect.height() / 2.0,
                egui::Stroke::new(1.5, theme.palette.focus_ring),
                egui::StrokeKind::Outside,
            );
        }
    }

    response
}
