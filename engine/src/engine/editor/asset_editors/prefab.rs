//! Prefab placeholder editor.
//!
//! Editor lands when the prefab format ships. This window confirms the asset
//! opens, dock/undock works, and is ready to host the editor when it ships.

use std::path::PathBuf;

/// Per-window state for a prefab placeholder.
pub struct PrefabEditorState {
    pub path: PathBuf,
    pub raw_ron: Option<String>,
    pub show_raw: bool,
}

impl PrefabEditorState {
    pub fn open(path: PathBuf) -> Self {
        let raw_ron = std::fs::read_to_string(&path).ok();
        Self {
            path,
            raw_ron,
            show_raw: false,
        }
    }
}

/// Render the prefab placeholder UI.
pub fn show_prefab_editor(ui: &mut egui::Ui, state: &mut PrefabEditorState) {
    ui.vertical_centered(|ui| {
        ui.add_space(ui.available_height() / 4.0);
        ui.heading("Prefab Editor");
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new("Editor lands when the prefab format ships.")
                .weak(),
        );
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
