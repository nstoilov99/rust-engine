//! Editor status bar helpers.

#[derive(Debug, Clone)]
pub struct StatusBarState {
    pub left_text: String,
    pub right_text: String,
}

impl Default for StatusBarState {
    fn default() -> Self {
        Self {
            left_text: "Ready".to_string(),
            right_text: String::new(),
        }
    }
}

impl StatusBarState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_left(&mut self, text: impl Into<String>) {
        self.left_text = text.into();
    }

    pub fn set_right(&mut self, text: impl Into<String>) {
        self.right_text = text.into();
    }
}

pub fn render_status_bar(ctx: &egui::Context, state: &StatusBarState) {
    egui::TopBottomPanel::bottom("editor_status_bar")
        .exact_height(22.0)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(state.left_text.as_str()).weak());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if !state.right_text.is_empty() {
                        ui.label(egui::RichText::new(state.right_text.as_str()).weak());
                    }
                });
            });
        });
}
