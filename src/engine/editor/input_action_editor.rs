//! Dedicated editor for individual `.inputaction.ron` files.
//!
//! Opens in a separate OS window when a user double-clicks an input action
//! asset in the asset browser — similar to Unreal Engine's InputAction editor.

use crate::engine::input::enhanced_action::InputActionDefinition;
use crate::engine::input::enhanced_serialization;
use crate::engine::input::value::InputValueType;
use std::collections::HashMap;
use std::path::PathBuf;

/// State for one open input action editor window.
pub struct InputActionEditorState {
    pub definition: InputActionDefinition,
    pub dirty: bool,
    pub file_path: PathBuf,
    pub open: bool,
    pub status_message: Option<(String, f64)>,
}

/// Manages all open input action editor windows.
#[derive(Default)]
pub struct InputActionEditor {
    pub open_actions: HashMap<String, InputActionEditorState>,
}

impl InputActionEditor {
    pub fn new() -> Self {
        Self::default()
    }

    /// Open an input action for editing. Loads from file if not already open.
    /// Returns the editor key (file path string) for use with PendingWindowRequest.
    pub fn open(&mut self, file_path: PathBuf) -> String {
        let key = file_path.to_string_lossy().to_string();
        if !self.open_actions.contains_key(&key) {
            let definition = enhanced_serialization::load_input_action(&file_path)
                .unwrap_or_else(|| InputActionDefinition::new("unnamed", InputValueType::Digital));
            self.open_actions.insert(
                key.clone(),
                InputActionEditorState {
                    definition,
                    dirty: false,
                    file_path,
                    open: true,
                    status_message: None,
                },
            );
        }
        key
    }

    /// Render the input action editor UI into a given Ui area.
    /// Called from the secondary window render loop.
    pub fn show_ui(ui: &mut egui::Ui, state: &mut InputActionEditorState) {
        let action = &mut state.definition;
        let editor_id = ui.id().with("ia_editor");

        // ── Top toolbar ──
        egui::TopBottomPanel::top(editor_id.with("toolbar"))
            .exact_height(32.0)
            .show_inside(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    let save_text = if state.dirty {
                        egui::RichText::new("Save")
                            .color(egui::Color32::WHITE)
                    } else {
                        egui::RichText::new("Save")
                    };
                    if ui.button(save_text).clicked() {
                        match enhanced_serialization::save_input_action(action, &state.file_path) {
                            Ok(()) => {
                                state.dirty = false;
                                state.status_message =
                                    Some(("Saved".to_string(), ui.input(|i| i.time)));
                            }
                            Err(e) => {
                                state.status_message = Some((
                                    format!("Save failed: {e}"),
                                    ui.input(|i| i.time),
                                ));
                            }
                        }
                    }

                    if state.dirty {
                        ui.label(
                            egui::RichText::new("\u{2022} Unsaved changes")
                                .color(egui::Color32::from_rgb(255, 200, 80))
                                .small(),
                        );
                    }

                    // Status message (fades after 3s)
                    if let Some((msg, time)) = &state.status_message {
                        let elapsed = ui.input(|i| i.time) - time;
                        if elapsed < 3.0 {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.label(egui::RichText::new(msg).weak().italics().small());
                            });
                        } else {
                            state.status_message = None;
                        }
                    }
                });
            });

        // ── Bottom status bar ──
        egui::TopBottomPanel::bottom(editor_id.with("statusbar"))
            .exact_height(22.0)
            .show_inside(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.label(
                        egui::RichText::new(state.file_path.to_string_lossy().as_ref())
                            .weak()
                            .small()
                            .monospace(),
                    );
                });
            });

        // ── Central content ──
        egui::CentralPanel::default().show_inside(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.add_space(4.0);

                    // ── Properties ──
                    ui.group(|ui| {
                        ui.label(
                            egui::RichText::new("Properties")
                                .strong()
                                .color(egui::Color32::from_rgb(140, 180, 220)),
                        );
                        ui.add_space(2.0);

                        egui::Grid::new("ia_props")
                            .num_columns(2)
                            .spacing([8.0, 4.0])
                            .show(ui, |ui| {
                                ui.label("Name:");
                                if ui.text_edit_singleline(&mut action.name).changed() {
                                    state.dirty = true;
                                }
                                ui.end_row();

                                ui.label("Value Type:");
                                let before = action.value_type;
                                egui::ComboBox::from_id_salt(editor_id.with("vt"))
                                    .selected_text(format_value_type(action.value_type))
                                    .width(120.0)
                                    .show_ui(ui, |ui| {
                                        for vt in VALUE_TYPES {
                                            let label = format_value_type(*vt);
                                            ui.selectable_value(
                                                &mut action.value_type,
                                                *vt,
                                                label,
                                            );
                                        }
                                    });
                                if action.value_type != before {
                                    state.dirty = true;
                                }
                                ui.end_row();

                                ui.label("Consumes Input:");
                                if ui
                                    .checkbox(&mut action.consumes_input, "")
                                    .on_hover_text(
                                        "When enabled, this action consumes its input sources \
                                         and prevents lower-priority contexts from seeing them.",
                                    )
                                    .changed()
                                {
                                    state.dirty = true;
                                }
                                ui.end_row();
                            });
                    });

                    ui.add_space(6.0);

                    // ── Triggers ──
                    ui.group(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new("Triggers")
                                    .strong()
                                    .color(egui::Color32::from_rgb(180, 200, 140)),
                            );
                            ui.label(
                                egui::RichText::new(format!("({})", action.triggers.len()))
                                    .weak()
                                    .small(),
                            );
                        });
                        ui.add_space(2.0);
                        let before = action.triggers.len();
                        super::input_settings_panel::render_trigger_list_pub(
                            ui,
                            &mut action.triggers,
                            "ia_trig",
                        );
                        if action.triggers.len() != before {
                            state.dirty = true;
                        }
                    });

                    ui.add_space(6.0);

                    // ── Modifiers ──
                    ui.group(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new("Modifiers")
                                    .strong()
                                    .color(egui::Color32::from_rgb(200, 160, 180)),
                            );
                            ui.label(
                                egui::RichText::new(format!("({})", action.modifiers.len()))
                                    .weak()
                                    .small(),
                            );
                        });
                        ui.add_space(2.0);
                        let before = action.modifiers.len();
                        super::input_settings_panel::render_modifier_list_pub(
                            ui,
                            &mut action.modifiers,
                            "ia_mod",
                        );
                        if action.modifiers.len() != before {
                            state.dirty = true;
                        }
                    });
                });
        });
    }
}

fn format_value_type(vt: InputValueType) -> &'static str {
    match vt {
        InputValueType::Digital => "Digital",
        InputValueType::Axis1D => "Axis1D",
        InputValueType::Axis2D => "Axis2D",
        InputValueType::Axis3D => "Axis3D",
    }
}

const VALUE_TYPES: &[InputValueType] = &[
    InputValueType::Digital,
    InputValueType::Axis1D,
    InputValueType::Axis2D,
    InputValueType::Axis3D,
];
