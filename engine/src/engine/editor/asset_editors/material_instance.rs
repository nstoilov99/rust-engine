//! MaterialInstance editor — edits `.matinst.ron` files (MaterialInstanceDef).
//!
//! **Case B (direct factors):** Task 39 shipped `MaterialInstanceDef` with direct
//! concrete factor values (no `Option<T>` wrapping, no inherits-from-base
//! distinction). Each factor is a plain editable field. The base_material picker
//! exists for traceability; a "Reset to base" button copies the base's current
//! values into the instance.

use std::path::PathBuf;
use std::time::Instant;

use crate::engine::assets::mesh_import::{load_material_ron, MaterialDefinition};
use crate::engine::rendering::rendering_3d::material_manager::MaterialInstanceDef;

/// Per-window editor state for a `.matinst.ron` asset.
pub struct MaterialInstanceEditorState {
    pub path: PathBuf,
    pub loaded: MaterialInstanceDef,
    pub edited: MaterialInstanceDef,
    /// Resolved base material (loaded on demand when base_material path changes).
    pub resolved_base: Option<MaterialDefinition>,
    pub dirty: bool,
    pub last_saved_at: Option<Instant>,
    pub status_message: Option<(String, f64)>,
}

impl MaterialInstanceEditorState {
    /// Load a material instance from disk and create editor state.
    pub fn open(path: PathBuf) -> Result<Self, Box<dyn std::error::Error>> {
        let loaded = MaterialInstanceDef::load(&path)?;
        let edited = loaded.clone();
        let resolved_base = try_load_base(&loaded.base_material, &path);
        Ok(Self {
            path,
            loaded,
            edited,
            resolved_base,
            dirty: false,
            last_saved_at: None,
            status_message: None,
        })
    }

    /// Save edited material instance to disk.
    pub fn save(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        self.edited.save(&self.path)?;
        self.loaded = self.edited.clone();
        self.dirty = false;
        self.last_saved_at = Some(Instant::now());
        self.status_message = Some((
            format!("Saved {}", self.path.display()),
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

    /// Reload the resolved base material from the current base_material path.
    pub fn reload_base(&mut self) {
        self.resolved_base = try_load_base(&self.edited.base_material, &self.path);
    }
}

/// Try to load a base material definition, resolving the path relative to
/// the content root or the instance file's directory.
fn try_load_base(base_material: &str, instance_path: &std::path::Path) -> Option<MaterialDefinition> {
    if base_material.is_empty() {
        return None;
    }
    // Try content-relative path first
    let content_path = std::path::Path::new("content").join(base_material);
    if let Ok(def) = load_material_ron(&content_path) {
        return Some(def);
    }
    // Try relative to instance file's parent directory
    if let Some(parent) = instance_path.parent() {
        let sibling_path = parent.join(base_material);
        if let Ok(def) = load_material_ron(&sibling_path) {
            return Some(def);
        }
    }
    None
}

/// Render the material instance editor UI.
pub fn show_material_instance_editor(
    ui: &mut egui::Ui,
    state: &mut MaterialInstanceEditorState,
) {
    ui.heading("Material Instance Editor");
    ui.label(
        egui::RichText::new(state.path.display().to_string())
            .weak()
            .small(),
    );
    ui.separator();

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // --- Base Material ---
            ui.label(egui::RichText::new("Base Material").strong());
            let mut base_changed = false;
            ui.horizontal(|ui| {
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut state.edited.base_material)
                            .hint_text(".material.ron path")
                            .desired_width(250.0),
                    )
                    .changed()
                {
                    state.dirty = true;
                    base_changed = true;
                }
            });
            if base_changed {
                state.reload_base();
            }
            if let Some(ref base) = state.resolved_base {
                ui.label(
                    egui::RichText::new(format!("Base: {}", base.name))
                        .weak()
                        .small(),
                );
            } else if !state.edited.base_material.is_empty() {
                ui.label(
                    egui::RichText::new("Base material not found")
                        .small()
                        .color(egui::Color32::from_rgb(200, 150, 50)),
                );
            }
            ui.add_space(8.0);

            // --- Factors ---
            ui.label(egui::RichText::new("Factors").strong());
            ui.separator();

            // Base Color
            ui.label("Base Color:");
            let mut color = [
                state.edited.base_color_factor[0],
                state.edited.base_color_factor[1],
                state.edited.base_color_factor[2],
            ];
            ui.horizontal(|ui| {
                if ui.color_edit_button_rgb(&mut color).changed() {
                    state.edited.base_color_factor[0] = color[0];
                    state.edited.base_color_factor[1] = color[1];
                    state.edited.base_color_factor[2] = color[2];
                    state.dirty = true;
                }
                if let Some(ref base) = state.resolved_base {
                    if ui.small_button("Reset to base").clicked() {
                        state.edited.base_color_factor = base.base_color_factor;
                        state.dirty = true;
                    }
                }
            });
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

            // Metallic
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
                if let Some(ref base) = state.resolved_base {
                    if ui.small_button("Reset to base").clicked() {
                        state.edited.metallic_factor = base.metallic_factor;
                        state.dirty = true;
                    }
                }
            });

            // Roughness
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
                if let Some(ref base) = state.resolved_base {
                    if ui.small_button("Reset to base").clicked() {
                        state.edited.roughness_factor = base.roughness_factor;
                        state.dirty = true;
                    }
                }
            });

            // Emissive
            ui.label("Emissive:");
            ui.horizontal(|ui| {
                let mut emissive = state.edited.emissive_factor;
                if ui.color_edit_button_rgb(&mut emissive).changed() {
                    state.edited.emissive_factor = emissive;
                    state.dirty = true;
                }
                if let Some(ref base) = state.resolved_base {
                    if ui.small_button("Reset to base").clicked() {
                        state.edited.emissive_factor = base.emissive_factor;
                        state.dirty = true;
                    }
                }
            });
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
            egui::RichText::new(format!("{}Material Instance", dirty_label))
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
                    log::error!("Failed to save material instance: {}", e);
                    state.status_message = Some((format!("Save failed: {}", e), 0.0));
                }
            }
            if let Some((msg, _)) = &state.status_message {
                ui.label(egui::RichText::new(msg).weak().small());
            }
        });
    });
}
