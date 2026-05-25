//! AnimationClip editor — read-only display of animation clip metadata.

use std::path::PathBuf;

/// Per-window editor state for an animation clip asset.
pub struct AnimationClipEditorState {
    pub path: PathBuf,
    pub metadata: AnimationClipMetadata,
}

/// Animation clip metadata extracted from the asset.
pub struct AnimationClipMetadata {
    pub duration_secs: f32,
    pub fps: f32,
    pub bone_names: Vec<String>,
}

impl Default for AnimationClipMetadata {
    fn default() -> Self {
        Self {
            duration_secs: 0.0,
            fps: 30.0,
            bone_names: Vec::new(),
        }
    }
}

impl AnimationClipEditorState {
    /// Create editor state. In v1, metadata is placeholder since we don't
    /// have a standardized animation clip file format with header probing.
    pub fn open(path: PathBuf) -> Self {
        let metadata = AnimationClipMetadata::default();
        Self { path, metadata }
    }
}

/// Render the animation clip editor UI.
pub fn show_animation_clip_editor(ui: &mut egui::Ui, state: &mut AnimationClipEditorState) {
    ui.heading("Animation Clip Editor");
    ui.label(
        egui::RichText::new(state.path.display().to_string())
            .weak()
            .small(),
    );
    ui.separator();

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // --- Metadata ---
            ui.label(egui::RichText::new("Clip Info").strong());
            ui.separator();
            ui.label(format!("Duration: {:.2}s", state.metadata.duration_secs));
            ui.label(format!("Frame Rate: {:.0} fps", state.metadata.fps));
            ui.label(format!("Bone Count: {}", state.metadata.bone_names.len()));
            ui.add_space(8.0);

            // --- Timeline placeholder ---
            ui.label(egui::RichText::new("Timeline").strong());
            let timeline_rect = ui.allocate_space(egui::vec2(ui.available_width().min(700.0), 40.0));
            let rect = egui::Rect::from_min_size(
                timeline_rect.1.min,
                egui::vec2(timeline_rect.1.size().x, 40.0),
            );
            ui.painter()
                .rect_filled(rect, 4.0, egui::Color32::from_gray(30));
            // Draw timeline line
            let center_y = rect.center().y;
            ui.painter().line_segment(
                [
                    egui::pos2(rect.left() + 4.0, center_y),
                    egui::pos2(rect.right() - 4.0, center_y),
                ],
                egui::Stroke::new(2.0, egui::Color32::from_gray(80)),
            );
            // Playhead at t=0
            ui.painter().circle_filled(
                egui::pos2(rect.left() + 8.0, center_y),
                4.0,
                egui::Color32::from_rgb(100, 180, 255),
            );
            ui.label(
                egui::RichText::new("Scrubbing lands in Task 41")
                    .weak()
                    .small()
                    .italics(),
            );
            ui.add_space(8.0);

            // --- Bone List ---
            if !state.metadata.bone_names.is_empty() {
                ui.label(egui::RichText::new("Bones").strong());
                ui.separator();
                for name in &state.metadata.bone_names {
                    ui.label(format!("  \u{2022} {}", name));
                }
            } else {
                ui.label(
                    egui::RichText::new("No bone data available")
                        .weak()
                        .italics(),
                );
            }
        });

    ui.separator();

    // --- Footer: info banner ---
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("\u{2139} Full timeline editing lands in Task 41")
                .weak()
                .small(),
        );
    });
}
