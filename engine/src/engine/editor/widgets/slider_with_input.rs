//! Combined drag-value slider + numeric input widget.

use egui::{Response, Ui};

/// Render a combined slider + numeric input.
///
/// When not focused, acts as a drag-value (drag to change). When text-focused,
/// switches to keyboard input mode.
pub fn slider_with_input(
    ui: &mut Ui,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    label: &str,
) -> Response {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(
            egui::DragValue::new(value)
                .range(range)
                .speed(0.01)
                .max_decimals(3),
        )
    })
    .inner
}

/// Render a combined slider + numeric input for an integer value.
pub fn slider_with_input_i32(
    ui: &mut Ui,
    value: &mut i32,
    range: std::ops::RangeInclusive<i32>,
    label: &str,
) -> Response {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(egui::DragValue::new(value).range(range))
    })
    .inner
}
