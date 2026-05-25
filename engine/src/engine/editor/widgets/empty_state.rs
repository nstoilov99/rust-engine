//! Empty state placeholder widget.

use egui::Ui;

use super::UiExt;
use super::IconKind;

/// Configuration for an empty state display.
pub struct EmptyState {
    pub icon: Option<IconKind>,
    pub title: String,
    pub subtitle: Option<String>,
    pub action_label: Option<String>,
}

impl EmptyState {
    /// Create a new empty state with just a title.
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            icon: None,
            title: title.into(),
            subtitle: None,
            action_label: None,
        }
    }

    /// Set the icon.
    pub fn with_icon(mut self, icon: IconKind) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Set the subtitle.
    pub fn with_subtitle(mut self, subtitle: impl Into<String>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }

    /// Set an action button label.
    pub fn with_action(mut self, label: impl Into<String>) -> Self {
        self.action_label = Some(label.into());
        self
    }
}

/// Render an empty state placeholder. Returns `true` if the action button was clicked.
pub fn empty_state(ui: &mut Ui, state: &EmptyState) -> bool {
    let theme = ui.theme();
    let mut action_clicked = false;

    ui.vertical_centered(|ui| {
        ui.add_space(40.0);

        // Icon (large, dimmed)
        if let Some(icon) = state.icon {
            let text = super::IconRegistry::fallback_text(icon);
            ui.label(
                egui::RichText::new(text)
                    .size(32.0)
                    .color(theme.palette.text_disabled),
            );
            ui.add_space(8.0);
        }

        // Title
        ui.label(
            egui::RichText::new(&state.title)
                .size(14.0)
                .color(theme.palette.text_secondary),
        );

        // Subtitle
        if let Some(subtitle) = &state.subtitle {
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(subtitle)
                    .size(11.0)
                    .color(theme.palette.text_disabled),
            );
        }

        // Action button
        if let Some(label) = &state.action_label {
            ui.add_space(12.0);
            if ui
                .add(
                    egui::Button::new(
                        egui::RichText::new(label).color(egui::Color32::WHITE),
                    )
                    .fill(theme.palette.accent),
                )
                .clicked()
            {
                action_clicked = true;
            }
        }

        ui.add_space(40.0);
    });

    action_clicked
}
