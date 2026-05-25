//! AnimationGraph placeholder editor.
//!
//! The interactive graph editor lands in Task 41. This window confirms the
//! asset opens, dock/undock works, and is ready to host the editor when it ships.

use std::path::PathBuf;

/// Per-window state for an animation graph placeholder.
pub struct AnimationGraphEditorState {
    pub path: PathBuf,
    pub raw_ron: Option<String>,
    pub show_raw: bool,
}

impl AnimationGraphEditorState {
    pub fn open(path: PathBuf) -> Self {
        let raw_ron = std::fs::read_to_string(&path).ok();
        Self {
            path,
            raw_ron,
            show_raw: false,
        }
    }
}

/// Render the animation graph placeholder UI.
pub fn show_animation_graph_editor(ui: &mut egui::Ui, state: &mut AnimationGraphEditorState) {
    ui.vertical_centered(|ui| {
        ui.add_space(ui.available_height() / 4.0);
        ui.heading("Animation Graph Editor");
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new("The interactive graph editor lands in Task 41.")
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
