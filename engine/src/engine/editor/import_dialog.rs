//! Model import dialog — egui modal for configuring import settings.
//!
//! Shown when model files (FBX, OBJ, glTF) are dropped onto the editor.
//! Follows the same `Option<DialogState>` pattern as `DeleteConfirmation`.

use crate::engine::assets::mesh_import::{MeshImportSettings, UpAxis};
use egui::RichText;
use std::path::PathBuf;

/// Preview information extracted from the source model.
#[derive(Debug, Clone)]
pub struct ImportPreview {
    pub mesh_count: usize,
    pub total_vertices: u32,
    pub total_indices: u32,
    pub material_count: usize,
    pub bone_count: usize,
    pub animation_count: usize,
}

/// State for the model import dialog.
#[derive(Debug, Clone)]
pub struct ImportDialogState {
    /// Source files being imported.
    pub source_files: Vec<PathBuf>,
    /// Index of the file currently being configured.
    pub current_file_index: usize,
    /// Import settings (shared for all files in batch, user can adjust).
    pub settings: MeshImportSettings,
    /// Content-relative target folder for output .mesh files.
    pub target_folder: PathBuf,
    /// Preview info (populated lazily on first render).
    pub preview: Option<ImportPreview>,
    /// Whether preview loading has been attempted.
    pub preview_attempted: bool,
}

impl ImportDialogState {
    /// Create a new import dialog for the given source files.
    pub fn new(source_files: Vec<PathBuf>, target_folder: PathBuf) -> Self {
        Self {
            source_files,
            current_file_index: 0,
            settings: MeshImportSettings::default(),
            target_folder,
            preview: None,
            preview_attempted: false,
        }
    }

    /// Get the current source file being configured.
    pub fn current_file(&self) -> Option<&PathBuf> {
        self.source_files.get(self.current_file_index)
    }

    /// Get the display name of the current file.
    pub fn current_file_name(&self) -> String {
        self.current_file()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "Unknown".to_string())
    }

    /// Get the format name based on extension.
    pub fn format_name(&self) -> &'static str {
        self.current_file()
            .and_then(|p| p.extension())
            .and_then(|e| e.to_str())
            .map(|ext| match ext.to_ascii_lowercase().as_str() {
                "gltf" => "glTF Text",
                "glb" => "glTF Binary",
                "obj" => "Wavefront OBJ",
                "fbx" => "Autodesk FBX",
                _ => "Unknown",
            })
            .unwrap_or("Unknown")
    }
}

/// Result from rendering the import dialog.
pub enum ImportDialogAction {
    /// No action taken (dialog still open).
    None,
    /// User cancelled the import.
    Cancel,
    /// User confirmed import for the current file.
    Import,
}

/// Render the import dialog. Returns the user's action.
pub fn render_import_dialog(
    ctx: &egui::Context,
    state: &mut ImportDialogState,
) -> ImportDialogAction {
    let mut action = ImportDialogAction::None;

    let title = if state.source_files.len() > 1 {
        format!(
            "Import Model ({}/{}): {}",
            state.current_file_index + 1,
            state.source_files.len(),
            state.current_file_name()
        )
    } else {
        format!("Import Model: {}", state.current_file_name())
    };

    egui::Window::new(title)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .default_width(380.0)
        .show(ctx, |ui| {
            ui.vertical(|ui| {
                // ── Source info ──
                ui.add_space(4.0);
                egui::Grid::new("import_source_info")
                    .num_columns(2)
                    .spacing([8.0, 4.0])
                    .show(ui, |ui| {
                        ui.label("Source:");
                        let source_text = state
                            .current_file()
                            .map(|p| {
                                // Truncate long paths
                                let s = p.to_string_lossy();
                                if s.len() > 50 {
                                    format!("...{}", &s[s.len() - 47..])
                                } else {
                                    s.to_string()
                                }
                            })
                            .unwrap_or_default();
                        ui.label(RichText::new(source_text).weak());
                        ui.end_row();

                        ui.label("Format:");
                        ui.label(RichText::new(state.format_name()).weak());
                        ui.end_row();

                        // Preview stats
                        if let Some(ref preview) = state.preview {
                            ui.label("Meshes:");
                            ui.label(
                                RichText::new(format!("{}", preview.mesh_count)).weak(),
                            );
                            ui.end_row();

                            ui.label("Vertices:");
                            ui.label(
                                RichText::new(format!(
                                    "{} ({} triangles)",
                                    preview.total_vertices,
                                    preview.total_indices / 3
                                ))
                                .weak(),
                            );
                            ui.end_row();

                            ui.label("Materials:");
                            ui.label(
                                RichText::new(format!("{}", preview.material_count)).weak(),
                            );
                            ui.end_row();

                            if preview.bone_count > 0 {
                                ui.label("Bones:");
                                ui.label(
                                    RichText::new(format!("{}", preview.bone_count)).weak(),
                                );
                                ui.end_row();
                            }

                            if preview.animation_count > 0 {
                                ui.label("Animations:");
                                ui.label(
                                    RichText::new(format!("{}", preview.animation_count)).weak(),
                                );
                                ui.end_row();
                            }
                        }
                    });

                ui.add_space(8.0);
                ui.separator();
                ui.add_space(4.0);

                // ── Import Settings ──
                ui.label(RichText::new("Import Settings").strong());
                ui.add_space(4.0);

                egui::Grid::new("import_settings")
                    .num_columns(2)
                    .spacing([8.0, 6.0])
                    .show(ui, |ui| {
                        // Scale
                        ui.label("Scale:");
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::DragValue::new(&mut state.settings.scale)
                                    .speed(0.01)
                                    .range(0.001..=1000.0)
                                    .fixed_decimals(3),
                            );
                            if ui.small_button("1x").clicked() {
                                state.settings.scale = 1.0;
                            }
                            if ui.small_button("0.01x").clicked() {
                                state.settings.scale = 0.01;
                            }
                        });
                        ui.end_row();

                        // Up Axis
                        ui.label("Up Axis:");
                        egui::ComboBox::from_id_salt("import_up_axis")
                            .selected_text(match state.settings.up_axis {
                                UpAxis::YUp => "Y-Up",
                                UpAxis::ZUp => "Z-Up (convert to Y-Up)",
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut state.settings.up_axis,
                                    UpAxis::YUp,
                                    "Y-Up",
                                );
                                ui.selectable_value(
                                    &mut state.settings.up_axis,
                                    UpAxis::ZUp,
                                    "Z-Up (convert to Y-Up)",
                                );
                            });
                        ui.end_row();
                    });

                ui.add_space(4.0);
                ui.checkbox(&mut state.settings.generate_tangents, "Generate Tangents");
                ui.checkbox(&mut state.settings.import_materials, "Import Materials");
                ui.checkbox(&mut state.settings.flip_uvs, "Flip UVs (V = 1-V)");

                let has_animations = state
                    .preview
                    .as_ref()
                    .is_some_and(|p| p.animation_count > 0);
                if has_animations {
                    ui.checkbox(
                        &mut state.settings.import_animations,
                        "Import Animations (.anim)",
                    );
                }

                ui.add_space(8.0);
                ui.separator();
                ui.add_space(4.0);

                // ── Output path preview ──
                let output_name = state
                    .current_file()
                    .and_then(|p| p.file_stem())
                    .and_then(|s| s.to_str())
                    .unwrap_or("model");
                let output_display = if state.target_folder.as_os_str().is_empty() {
                    format!("{}.mesh", output_name)
                } else {
                    format!("{}/{}.mesh", state.target_folder.display(), output_name)
                };
                ui.horizontal(|ui| {
                    ui.label("Output:");
                    ui.label(RichText::new(output_display).weak().italics());
                });

                if has_animations && state.settings.import_animations {
                    let anim_display = if state.target_folder.as_os_str().is_empty() {
                        format!("{}.anim", output_name)
                    } else {
                        format!("{}/{}.anim", state.target_folder.display(), output_name)
                    };
                    ui.horizontal(|ui| {
                        ui.add_space(ui.spacing().indent); // align with Output: label
                        ui.label(RichText::new(anim_display).weak().italics());
                    });
                }

                ui.add_space(12.0);

                // ── Buttons ──
                let button_width = 90.0;
                let spacing = 16.0;
                let total_width = button_width * 2.0 + spacing;
                let available = ui.available_width();
                let padding = ((available - total_width) / 2.0).max(0.0);

                ui.horizontal(|ui| {
                    ui.add_space(padding);

                    if ui
                        .add(
                            egui::Button::new("Cancel")
                                .min_size(egui::vec2(button_width, 28.0)),
                        )
                        .clicked()
                    {
                        action = ImportDialogAction::Cancel;
                    }

                    ui.add_space(spacing);

                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("Import").color(egui::Color32::WHITE),
                            )
                            .fill(egui::Color32::from_rgb(50, 120, 200))
                            .min_size(egui::vec2(button_width, 28.0)),
                        )
                        .clicked()
                    {
                        action = ImportDialogAction::Import;
                    }
                });

                ui.add_space(4.0);
            });
        });

    action
}
