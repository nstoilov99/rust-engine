//! Collapsible panel header widget with chevron animation and optional color stripe.

use egui::{Color32, Response, Ui};

use super::UiExt;

/// Render a collapsible panel header.
///
/// - `label`: display name for the section
/// - `color_stripe`: optional 2px color stripe at the left edge
/// - `open`: mutable state for collapsed/expanded
/// - `add_body`: closure called when expanded
///
/// Returns the header response.
pub fn panel_header<R>(
    ui: &mut Ui,
    label: &str,
    color_stripe: Option<Color32>,
    open: &mut bool,
    add_body: impl FnOnce(&mut Ui) -> R,
) -> Response {
    let theme = ui.theme();
    let id = ui.make_persistent_id(label);

    // Animated chevron rotation
    let openness = ui.ctx().animate_bool(id, *open);

    let header_response = ui.horizontal(|ui| {
        // Color stripe
        if let Some(color) = color_stripe {
            let (stripe_rect, _) =
                ui.allocate_exact_size(egui::vec2(2.0, ui.spacing().interact_size.y), egui::Sense::hover());
            if ui.is_rect_visible(stripe_rect) {
                ui.painter().rect_filled(stripe_rect, 0.0, color);
            }
        }

        // Chevron (rotates with animation)
        let chevron_size = egui::vec2(16.0, 16.0);
        let (chevron_rect, chevron_response) =
            ui.allocate_exact_size(chevron_size, egui::Sense::click());

        if chevron_response.clicked() {
            *open = !*open;
        }

        if ui.is_rect_visible(chevron_rect) {
            let center = chevron_rect.center();
            let painter = ui.painter();

            // Draw a rotated triangle based on openness
            let angle = openness * std::f32::consts::FRAC_PI_2; // 0 = right, 90° = down
            let tri_size = 4.0;
            let cos = angle.cos();
            let sin = angle.sin();

            let points = [
                egui::pos2(
                    center.x + tri_size * cos - 0.0 * sin,
                    center.y + tri_size * sin + 0.0 * cos,
                ),
                egui::pos2(
                    center.x - tri_size * 0.5 * cos - tri_size * 0.866 * sin,
                    center.y - tri_size * 0.5 * sin + tri_size * 0.866 * cos,
                ),
                egui::pos2(
                    center.x - tri_size * 0.5 * cos + tri_size * 0.866 * sin,
                    center.y - tri_size * 0.5 * sin - tri_size * 0.866 * cos,
                ),
            ];
            painter.add(egui::Shape::convex_polygon(
                points.to_vec(),
                theme.palette.text_secondary,
                egui::Stroke::NONE,
            ));
        }

        // Header label (also clickable to toggle)
        let label_response = ui.add(
            egui::Label::new(
                egui::RichText::new(label)
                    .strong()
                    .color(theme.palette.text_primary),
            )
            .selectable(false)
            .sense(egui::Sense::click()),
        );

        if label_response.clicked() {
            *open = !*open;
        }

        // Focus ring
        if chevron_response.has_focus() || label_response.has_focus() {
            let combined = chevron_rect.union(label_response.rect);
            ui.painter().rect_stroke(
                combined.expand(1.0),
                4,
                egui::Stroke::new(1.5, theme.palette.focus_ring),
                egui::StrokeKind::Outside,
            );
        }

        chevron_response | label_response
    });

    // Body content (animated height)
    if openness > 0.0 {
        // Use CollapsingState for smooth animation
        ui.scope(|ui| {
            if openness < 1.0 {
                // Partially collapsed — clip
                ui.set_clip_rect(ui.available_rect_before_wrap());
            }
            if *open || openness > 0.01 {
                ui.indent(id, |ui| {
                    add_body(ui);
                });
            }
        });
    }

    header_response.inner
}
