//! MaterialGraph placeholder editor.
//!
//! The node-graph editor lands in Task 50. This window confirms the asset
//! opens, dock/undock works, and is ready to host the editor when it ships.

use std::path::PathBuf;

/// Per-window state for a material graph placeholder.
pub struct MaterialGraphEditorState {
    pub path: PathBuf,
    pub raw_ron: Option<String>,
    pub show_raw: bool,
}

impl MaterialGraphEditorState {
    pub fn open(path: PathBuf) -> Self {
        let raw_ron = std::fs::read_to_string(&path).ok();
        Self {
            path,
            raw_ron,
            show_raw: false,
        }
    }
}

/// Render the material graph placeholder UI.
pub fn show_material_graph_editor(ui: &mut egui::Ui, state: &mut MaterialGraphEditorState) {
    ui.vertical_centered(|ui| {
        ui.add_space(ui.available_height() / 4.0);
        ui.heading("Material Graph Editor");
        ui.add_space(8.0);
        ui.label(egui::RichText::new("Editor lands in Task 50.").weak());
        ui.label(
            egui::RichText::new(
                "This window confirms the asset opens, dock/undock works,\nand is ready to host the editor when it ships.",
            )
            .weak()
            .small(),
        );
        ui.add_space(16.0);

        if state.raw_ron.is_some()
            && ui
                .selectable_label(state.show_raw, "\u{25B6} Show raw RON")
                .clicked()
        {
            state.show_raw = !state.show_raw;
        }
    });

    if state.show_raw {
        if let Some(ref ron) = state.raw_ron {
            ui.separator();
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut ron.as_str())
                            .code_editor()
                            .desired_width(f32::INFINITY),
                    );
                });
        }
    }
}
