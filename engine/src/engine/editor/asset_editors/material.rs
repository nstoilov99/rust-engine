//! Material editor — edits `.material.ron` files (MaterialDefinition).

use std::path::PathBuf;
use std::time::Instant;

use crate::engine::assets::mesh_import::{load_material_ron, MaterialDefinition};

/// Per-window editor state for a `.material.ron` asset.
pub struct MaterialEditorState {
    pub path: PathBuf,
    pub loaded: MaterialDefinition,
    pub edited: MaterialDefinition,
    pub dirty: bool,
    pub last_saved_at: Option<Instant>,
    pub status_message: Option<(String, f64)>,
}

impl MaterialEditorState {
    /// Load a material from disk and create editor state.
    pub fn open(path: PathBuf) -> Result<Self, Box<dyn std::error::Error>> {
        let loaded = load_material_ron(&path)?;
        let edited = loaded.clone();
        Ok(Self {
            path,
            loaded,
            edited,
            dirty: false,
            last_saved_at: None,
            status_message: None,
        })
    }

    /// Save edited material to disk.
    pub fn save(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let pretty = ron::ser::PrettyConfig::new()
            .depth_limit(3)
            .struct_names(true);
        let contents = ron::ser::to_string_pretty(&self.edited, pretty)?;
        std::fs::write(&self.path, contents)?;
        self.loaded = self.edited.clone();
        self.dirty = false;
        self.last_saved_at = Some(Instant::now());
        self.status_message = Some((
            format!("Saved {}", self.path.display()),
            // timestamp for fading
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64(),
        ));
        Ok(())
    }

    /// Revert to the last saved state.
    pub fn revert(&mut self) {
        self.edited = self.loaded.clone();
        self.dirty = false;
    }
}

/// Render the material editor UI.
pub fn show_material_editor(ui: &mut egui::Ui, state: &mut MaterialEditorState) {
    ui.heading("Material Editor");
    ui.label(
        egui::RichText::new(state.path.display().to_string())
            .weak()
            .small(),
    );
    ui.separator();

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // --- Base Color ---
            ui.label(egui::RichText::new("Base Color").strong());
            let mut color = [
                state.edited.base_color_factor[0],
                state.edited.base_color_factor[1],
                state.edited.base_color_factor[2],
            ];
            if ui.color_edit_button_rgb(&mut color).changed() {
                state.edited.base_color_factor[0] = color[0];
                state.edited.base_color_factor[1] = color[1];
                state.edited.base_color_factor[2] = color[2];
                state.dirty = true;
            }
            ui.horizontal(|ui| {
                ui.label("Alpha:");
                if ui
                    .add(egui::Slider::new(
                        &mut state.edited.base_color_factor[3],
                        0.0..=1.0,
                    ))
                    .changed()
                {
                    state.dirty = true;
                }
            });
            ui.add_space(4.0);

            // --- Metallic Factor ---
            ui.horizontal(|ui| {
                ui.label("Metallic:");
                if ui
                    .add(egui::Slider::new(
                        &mut state.edited.metallic_factor,
                        0.0..=1.0,
                    ))
                    .changed()
                {
                    state.dirty = true;
                }
            });

            // --- Roughness Factor ---
            ui.horizontal(|ui| {
                ui.label("Roughness:");
                if ui
                    .add(egui::Slider::new(
                        &mut state.edited.roughness_factor,
                        0.0..=1.0,
                    ))
                    .changed()
                {
                    state.dirty = true;
                }
            });

            // --- Emissive Factor ---
            ui.label(egui::RichText::new("Emissive").strong());
            let mut emissive = state.edited.emissive_factor;
            if ui.color_edit_button_rgb(&mut emissive).changed() {
                state.edited.emissive_factor = emissive;
                state.dirty = true;
            }
            ui.add_space(8.0);

            // --- Texture Slots ---
            ui.label(egui::RichText::new("Texture Slots").strong());
            ui.separator();
            texture_slot_row(ui, "Albedo", &mut state.edited.albedo_texture, &mut state.dirty);
            texture_slot_row(ui, "Normal", &mut state.edited.normal_texture, &mut state.dirty);
            texture_slot_row(
                ui,
                "Metallic/Roughness",
                &mut state.edited.metallic_roughness_texture,
                &mut state.dirty,
            );
            texture_slot_row(ui, "AO", &mut state.edited.ao_texture, &mut state.dirty);
            ui.add_space(8.0);

            // --- Preview placeholder ---
            ui.label(egui::RichText::new("Preview").strong());
            let preview_rect = ui.allocate_space(egui::vec2(256.0, 256.0));
            ui.painter().rect_filled(
                egui::Rect::from_min_size(preview_rect.1.min, egui::vec2(256.0, 256.0)),
                4.0,
                egui::Color32::from_gray(35),
            );
            ui.painter().text(
                preview_rect.1.min + egui::vec2(128.0, 128.0),
                egui::Align2::CENTER_CENTER,
                "Preview lands when\nAssetPreviewRegistry ships\nin Task 39.4 Step 14",
                egui::FontId::proportional(11.0),
                egui::Color32::from_gray(120),
            );
        });

    ui.separator();

    // --- Footer: Save / Revert ---
    ui.horizontal(|ui| {
        let dirty_label = if state.dirty { "* " } else { "" };
        ui.label(
            egui::RichText::new(format!("{}Material", dirty_label))
                .weak()
                .small(),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add_enabled(state.dirty, egui::Button::new("Revert"))
                .clicked()
            {
                state.revert();
            }
            if ui
                .add_enabled(state.dirty, egui::Button::new("Save"))
                .clicked()
            {
                if let Err(e) = state.save() {
                    log::error!("Failed to save material: {}", e);
                    state.status_message = Some((format!("Save failed: {}", e), 0.0));
                }
            }
            if let Some((msg, _)) = &state.status_message {
                ui.label(egui::RichText::new(msg).weak().small());
            }
        });
    });
}

fn texture_slot_row(ui: &mut egui::Ui, label: &str, value: &mut String, dirty: &mut bool) {
    ui.horizontal(|ui| {
        ui.label(format!("{}:", label));
        let display = if value.is_empty() {
            "(none)".to_string()
        } else {
            value.clone()
        };
        if ui
            .add(
                egui::TextEdit::singleline(value)
                    .hint_text("texture path")
                    .desired_width(200.0),
            )
            .changed()
        {
            *dirty = true;
        }
        let _ = display;
    });
}
