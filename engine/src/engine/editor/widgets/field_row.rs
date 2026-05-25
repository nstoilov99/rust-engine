//! Field row widget — label + value column layout for the inspector.

use egui::Ui;

/// Default label width ratio (fraction of available width).
const LABEL_RATIO: f32 = 0.35;

/// Render a field row with a label at consistent width, then run the value closure.
///
/// Returns the response from the value widget (or `None` if only the label was rendered).
pub fn field_row<R>(
    ui: &mut Ui,
    label: &str,
    add_value: impl FnOnce(&mut Ui) -> R,
) -> R {
    let available = ui.available_width();
    let label_width = (available * LABEL_RATIO).max(60.0);

    ui.horizontal(|ui| {
        ui.set_min_width(available);

        // Label column
        ui.allocate_ui_with_layout(
            egui::vec2(label_width, ui.spacing().interact_size.y),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.label(
                    egui::RichText::new(label)
                        .color(ui.visuals().widgets.noninteractive.fg_stroke.color),
                );
            },
        );

        // Value column fills remaining space
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), add_value)
            .inner
    })
    .inner
}

/// Render a field row with a label, value widget, and a reset button.
/// `is_default` should be `true` when the value matches its default.
/// Returns `true` if reset was clicked.
pub fn field_row_with_reset<R>(
    ui: &mut Ui,
    label: &str,
    is_default: bool,
    add_value: impl FnOnce(&mut Ui) -> R,
) -> (R, bool) {
    let available = ui.available_width();
    let label_width = (available * LABEL_RATIO).max(60.0);
    let reset_width = if is_default { 0.0 } else { 20.0 };

    let mut reset_clicked = false;

    let result = ui.horizontal(|ui| {
        ui.set_min_width(available);

        // Label column
        ui.allocate_ui_with_layout(
            egui::vec2(label_width, ui.spacing().interact_size.y),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.label(
                    egui::RichText::new(label)
                        .color(ui.visuals().widgets.noninteractive.fg_stroke.color),
                );
            },
        );

        // Value column
        let value_width = available - label_width - reset_width - ui.spacing().item_spacing.x * 2.0;
        let result = ui.allocate_ui_with_layout(
            egui::vec2(value_width.max(40.0), ui.spacing().interact_size.y),
            egui::Layout::left_to_right(egui::Align::Center),
            add_value,
        ).inner;

        // Reset button (only shown when not at default)
        if !is_default {
            let resp = ui.small_button("\u{21BA}"); // ↺
            if resp.clicked() {
                reset_clicked = true;
            }
            resp.on_hover_text("Reset to default");
        }

        result
    })
    .inner;

    (result, reset_clicked)
}
