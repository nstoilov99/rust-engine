//! Pillbox tab bar widget.

use egui::Ui;

use super::UiExt;

/// Render a horizontal tab bar. Returns the index of the newly selected tab (if changed).
///
/// `labels`: slice of tab labels
/// `active`: currently active index
pub fn tab_bar(ui: &mut Ui, labels: &[&str], active: &mut usize) -> Option<usize> {
    let theme = ui.theme();
    let mut changed = None;

    ui.horizontal(|ui| {
        for (i, label) in labels.iter().enumerate() {
            let is_active = i == *active;
            let id = ui.make_persistent_id(format!("tab_{i}"));

            let anim = ui.ctx().animate_bool(id, is_active);

            // Interpolate between surface[2] (inactive) and accent (active)
            let bg = egui::Color32::from_rgba_unmultiplied(
                ((1.0 - anim) * theme.palette.surface[2].r() as f32
                    + anim * theme.palette.accent.r() as f32) as u8,
                ((1.0 - anim) * theme.palette.surface[2].g() as f32
                    + anim * theme.palette.accent.g() as f32) as u8,
                ((1.0 - anim) * theme.palette.surface[2].b() as f32
                    + anim * theme.palette.accent.b() as f32) as u8,
                255,
            );

            let text_color = if is_active {
                egui::Color32::WHITE
            } else {
                theme.palette.text_secondary
            };

            let button = egui::Button::new(
                egui::RichText::new(*label).color(text_color),
            )
            .fill(bg)
            .corner_radius(12);

            let response = ui.add(button);
            if response.clicked() && !is_active {
                *active = i;
                changed = Some(i);
            }

            // Focus ring
            if response.has_focus() {
                ui.painter().rect_stroke(
                    response.rect.expand(1.0),
                    12,
                    egui::Stroke::new(1.5, theme.palette.focus_ring),
                    egui::StrokeKind::Outside,
                );
            }
        }
    });

    changed
}
