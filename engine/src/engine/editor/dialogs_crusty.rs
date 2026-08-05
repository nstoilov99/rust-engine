//! Crusty-gui modal dialogs: the central [`DialogStack`], the model import
//! dialog, the Save Scene As dialog and the file-drop overlay. Reads state
//! from `dialogs/mod.rs`, `import_dialog.rs` and `game_client/src/app.rs`.

use super::command_palette::EditorAction;
use super::dialogs::{DialogButtons, DialogStack};
use super::import_dialog::{ImportDialogAction, ImportDialogState};
use super::scene_tab::SaveAsDialog;
use crate::engine::assets::mesh_import::UpAxis;
use crusty_gui::context::Ui;
use crusty_gui::math::{Pos2, Rect, Rounding, Vec2};
use crusty_gui::widgets::{
    Button, Checkbox, ComboBox, DragValue, Grid, Label, SelectableValue, TextEdit, Window,
};

const BUTTON_SIZE: Vec2 = Vec2::new(80.0, 26.0);

fn dialog_window(title: &str, width: f32) -> Window<'static> {
    Window::new(title)
        .modal(true)
        .resizable(false)
        .collapsible(false)
        .anchor_center(true)
        .default_size(Vec2::new(width, 120.0))
        .auto_size(true)
}

fn message_lines(ui: &mut Ui, message: &str) {
    for line in message.split('\n') {
        if line.is_empty() {
            ui.add_space(6.0);
        } else {
            Label::new(line).show(ui);
        }
    }
}

/// Show the top dialog of the stack as a centered modal. Clicking a button
/// pops the dialog and returns its bound actions.
pub fn dialog_stack_panel(ui: &mut Ui, stack: &mut DialogStack) -> Vec<EditorAction> {
    let Some(dialog) = stack.top().cloned() else {
        return Vec::new();
    };

    let mut close = false;
    let mut actions = Vec::new();
    let mut clicked = |action: &Option<EditorAction>| {
        if let Some(action) = action.clone() {
            actions.push(action);
        }
        close = true;
    };

    dialog_window(&dialog.title, 360.0).show(ui, |ui| {
        message_lines(ui, &dialog.message);
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            let button = |ui: &mut Ui, b: Button| {
                let r = b.min_size(BUTTON_SIZE).show(ui).clicked;
                ui.add_space(8.0);
                r
            };
            match dialog.buttons {
                DialogButtons::Ok => {
                    if button(ui, Button::new("OK").primary()) {
                        clicked(&dialog.actions.primary);
                    }
                }
                DialogButtons::OkCancel => {
                    if button(ui, Button::new("OK").primary()) {
                        clicked(&dialog.actions.primary);
                    }
                    if button(ui, Button::new("Cancel").ghost()) {
                        clicked(&dialog.actions.cancel);
                    }
                }
                DialogButtons::SaveDiscardCancel => {
                    if button(ui, Button::new("Save").primary()) {
                        clicked(&dialog.actions.primary);
                    }
                    if button(ui, Button::new("Discard").danger()) {
                        clicked(&dialog.actions.secondary);
                    }
                    if button(ui, Button::new("Cancel").ghost()) {
                        clicked(&dialog.actions.cancel);
                    }
                }
            }
        });
    });

    if close {
        stack.pop_top();
    }
    actions
}

/// Save Scene As: filename entry. Sets `dlg.commit` on Save/Enter; returns
/// true when the dialog should close without saving (Cancel/Escape).
pub fn save_as_dialog_panel(ui: &mut Ui, dlg: &mut SaveAsDialog) -> bool {
    let mut cancel = false;
    dialog_window("Save Scene As", 360.0).show(ui, |ui| {
        Label::new("Filename (saved to content/scenes/<name>.scene):").show(ui);
        let out = TextEdit::new(&mut dlg.filename).width(300.0).show_full(ui);
        if out.submitted {
            dlg.commit = true;
        }
        if out.cancelled {
            cancel = true;
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if Button::new("Save")
                .primary()
                .min_size(BUTTON_SIZE)
                .show(ui)
                .clicked
            {
                dlg.commit = true;
            }
            ui.add_space(8.0);
            if Button::new("Cancel")
                .ghost()
                .min_size(BUTTON_SIZE)
                .show(ui)
                .clicked
            {
                cancel = true;
            }
        });
    });
    cancel
}

/// Model import dialog: source stats, import settings, output preview.
pub fn import_dialog_panel(ui: &mut Ui, state: &mut ImportDialogState) -> ImportDialogAction {
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

    let dim = ui.style().palette.text_secondary;
    let dim_label = |ui: &mut Ui, text: String| {
        Label::new(text).color(dim).show(ui);
    };
    // Key column: no wrapping — rows appear once the preview loads, and a
    // wrapped label would lock the grid column at its too-narrow width.
    let key_label = |ui: &mut Ui, text: &str| {
        Label::new(text).wrap(false).show(ui);
    };

    dialog_window(&title, 420.0).show(ui, |ui| {
        // ── Source info
        ui.add_space(4.0);
        Grid::new("import_source_info")
            .spacing(Vec2::new(8.0, 4.0))
            .show(ui, |g| {
                g.cell(|ui| key_label(ui, "Source:"));
                let source_text = state
                    .current_file()
                    .map(|p| {
                        let s = p.to_string_lossy();
                        if s.len() > 50 {
                            format!("...{}", &s[s.len() - 47..])
                        } else {
                            s.to_string()
                        }
                    })
                    .unwrap_or_default();
                g.cell(|ui| dim_label(ui, source_text));
                g.end_row();

                g.cell(|ui| key_label(ui, "Format:"));
                g.cell(|ui| dim_label(ui, state.format_name().to_string()));
                g.end_row();

                if let Some(ref preview) = state.preview {
                    g.cell(|ui| key_label(ui, "Meshes:"));
                    g.cell(|ui| dim_label(ui, preview.mesh_count.to_string()));
                    g.end_row();

                    g.cell(|ui| key_label(ui, "Vertices:"));
                    g.cell(|ui| {
                        dim_label(
                            ui,
                            format!(
                                "{} ({} triangles)",
                                preview.total_vertices,
                                preview.total_indices / 3
                            ),
                        )
                    });
                    g.end_row();

                    g.cell(|ui| key_label(ui, "Materials:"));
                    g.cell(|ui| dim_label(ui, preview.material_count.to_string()));
                    g.end_row();

                    if preview.bone_count > 0 {
                        g.cell(|ui| key_label(ui, "Bones:"));
                        g.cell(|ui| dim_label(ui, preview.bone_count.to_string()));
                        g.end_row();
                    }
                    if preview.animation_count > 0 {
                        g.cell(|ui| key_label(ui, "Animations:"));
                        g.cell(|ui| dim_label(ui, preview.animation_count.to_string()));
                        g.end_row();
                    }
                }
            });

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);

        // ── Import settings
        Label::new("Import Settings").show(ui);
        ui.add_space(4.0);

        Grid::new("import_settings")
            .spacing(Vec2::new(8.0, 6.0))
            .show(ui, |g| {
                g.cell(|ui| key_label(ui, "Scale:"));
                g.cell(|ui| {
                    ui.horizontal(|ui| {
                        DragValue::new(&mut state.settings.scale)
                            .speed(0.01)
                            .range(0.001..=1000.0)
                            .decimals(3)
                            .min_decimals(3)
                            .width(70.0)
                            .show(ui);
                        if Button::new("1x").show(ui).clicked {
                            state.settings.scale = 1.0;
                        }
                        if Button::new("0.01x").show(ui).clicked {
                            state.settings.scale = 0.01;
                        }
                    });
                });
                g.end_row();

                g.cell(|ui| key_label(ui, "Up Axis:"));
                g.cell(|ui| {
                    ComboBox::new("import_up_axis")
                        .selected_text(match state.settings.up_axis {
                            UpAxis::YUp => "Y-Up",
                            UpAxis::ZUp => "Z-Up (convert to Y-Up)",
                        })
                        .width(180.0)
                        .show_ui(ui, |ui| {
                            SelectableValue::new(&mut state.settings.up_axis, UpAxis::YUp, "Y-Up")
                                .show(ui);
                            SelectableValue::new(
                                &mut state.settings.up_axis,
                                UpAxis::ZUp,
                                "Z-Up (convert to Y-Up)",
                            )
                            .show(ui);
                        });
                });
                g.end_row();
            });

        ui.add_space(4.0);
        Checkbox::new(&mut state.settings.generate_tangents, "Generate Tangents").show(ui);
        Checkbox::new(&mut state.settings.import_materials, "Import Materials").show(ui);
        Checkbox::new(&mut state.settings.flip_uvs, "Flip UVs (V = 1-V)").show(ui);

        let has_animations = state
            .preview
            .as_ref()
            .is_some_and(|p| p.animation_count > 0);
        if has_animations {
            Checkbox::new(
                &mut state.settings.import_animations,
                "Import Animations (.anim)",
            )
            .show(ui);
        }

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);

        // ── Output path preview
        let output_name = state
            .current_file()
            .and_then(|p| p.file_stem())
            .and_then(|s| s.to_str())
            .unwrap_or("model");
        let output_display = if state.target_folder.as_os_str().is_empty() {
            format!("{output_name}.mesh")
        } else {
            format!("{}/{}.mesh", state.target_folder.display(), output_name)
        };
        ui.horizontal(|ui| {
            Label::new("Output:").show(ui);
            dim_label(ui, output_display);
        });
        if has_animations && state.settings.import_animations {
            let anim_display = if state.target_folder.as_os_str().is_empty() {
                format!("{output_name}.anim")
            } else {
                format!("{}/{}.anim", state.target_folder.display(), output_name)
            };
            ui.horizontal(|ui| {
                ui.add_space(48.0); // align under the Output: value
                dim_label(ui, anim_display);
            });
        }

        ui.add_space(12.0);

        // ── Buttons (centered)
        let button = Vec2::new(90.0, 28.0);
        let spacing = 16.0;
        let pad = ((ui.available_size().x - (button.x * 2.0 + spacing)) * 0.5).max(0.0);
        ui.horizontal(|ui| {
            ui.add_space(pad);
            if Button::new("Cancel").ghost().min_size(button).show(ui).clicked {
                action = ImportDialogAction::Cancel;
            }
            ui.add_space(spacing);
            if Button::new("Import")
                .primary()
                .min_size(button)
                .show(ui)
                .clicked
            {
                action = ImportDialogAction::Import;
            }
        });
        ui.add_space(4.0);
    });

    action
}

/// Full-screen tint + border + hint while external files hover the window.
pub fn file_drop_overlay(ui: &mut Ui, screen: Rect) {
    let palette = ui.style().palette;
    let mut p = ui.overlay_painter();
    // A 12% wash over the whole editor: it has to read as "drop here", not as
    // a curtain drawn across the window.
    p.rect_filled_translucent(
        screen,
        Rounding::ZERO,
        palette.accent_active.with_alpha(0.12),
    );
    p.rect_stroke(
        screen.shrink(4.0),
        Rounding::same(8.0),
        3.0,
        palette.accent_active,
    );
    let text = "Drop files to import into current folder";
    let size = p.measure_text(text, 24.0, None);
    p.text(
        Pos2::new(
            screen.center().x - size.x * 0.5,
            screen.center().y - size.y * 0.5,
        ),
        text,
        24.0,
        palette.text,
        None,
    );
}
