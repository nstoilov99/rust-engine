//! Themed button widgets for the editor.

use egui::{Response, Ui, WidgetText};

use super::UiExt;
use super::IconKind;

/// Visual variant for themed buttons.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ButtonVariant {
    #[default]
    Primary,
    Secondary,
    Danger,
}

/// Render a themed button with the default (Primary) variant.
pub fn themed_button(ui: &mut Ui, label: impl Into<WidgetText>) -> Response {
    themed_button_variant(ui, ButtonVariant::Primary, label)
}

/// Render a themed button with an icon and label.
pub fn themed_button_with_icon(
    ui: &mut Ui,
    icon: IconKind,
    label: impl Into<WidgetText>,
) -> Response {
    let theme = ui.theme();
    let label: WidgetText = label.into();

    let text = IconRegistry::fallback_text(icon);
    let combined = format!("{text} {}", label.text());

    let button = egui::Button::new(combined);
    let response = ui.add(button);

    // Draw focus ring if keyboard-focused
    if response.has_focus() {
        let rect = response.rect;
        ui.painter().rect_stroke(
            rect.expand(1.0),
            4,
            egui::Stroke::new(1.5, theme.palette.focus_ring),
            egui::StrokeKind::Outside,
        );
    }

    response
}

use super::IconRegistry;

/// Render a themed button with the specified variant.
pub fn themed_button_variant(
    ui: &mut Ui,
    variant: ButtonVariant,
    label: impl Into<WidgetText>,
) -> Response {
    let theme = ui.theme();
    let label: WidgetText = label.into();

    let (fill, text_color) = match variant {
        ButtonVariant::Primary => (theme.palette.accent, egui::Color32::WHITE),
        ButtonVariant::Secondary => (theme.palette.surface[2], theme.palette.text_primary),
        ButtonVariant::Danger => (theme.palette.semantic.error, egui::Color32::WHITE),
    };

    let button = egui::Button::new(
        egui::RichText::new(label.text()).color(text_color),
    )
    .fill(fill);

    let response = ui.add(button);

    // Draw focus ring if keyboard-focused
    if response.has_focus() {
        let rect = response.rect;
        ui.painter().rect_stroke(
            rect.expand(1.0),
            4,
            egui::Stroke::new(1.5, theme.palette.focus_ring),
            egui::StrokeKind::Outside,
        );
    }

    response
}
