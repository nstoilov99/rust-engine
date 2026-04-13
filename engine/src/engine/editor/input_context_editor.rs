//! Dedicated editor for individual `.mappingcontext.ron` files.
//!
//! Opens in a separate OS window when a user double-clicks a mapping context
//! asset in the asset browser — similar to Unreal Engine's InputMappingContext editor.

use crate::engine::input::action::{InputSource, KeyCode, MouseButton};
use crate::engine::input::enhanced_action::{
    EnhancedBinding, MappingContext, MappingContextEntry,
};
use crate::engine::input::enhanced_serialization;
use crate::engine::input::value::InputValueType;
use std::collections::HashMap;
use std::path::PathBuf;

/// State for one open mapping context editor window.
pub struct InputContextEditorState {
    pub context: MappingContext,
    pub dirty: bool,
    pub file_path: PathBuf,
    pub open: bool,
    pub status_message: Option<(String, f64)>,
    /// When set, the editor is listening for input on this (entry_idx, binding_idx).
    pub listening_binding: Option<(usize, usize)>,
    /// Modifier state captured when listening started, to detect new modifier-key presses.
    pub listen_start_modifiers: egui::Modifiers,
    /// Input detected from external sources (gamepad). Set by the main loop.
    pub pending_external_input: Option<InputSource>,
}

/// Manages all open mapping context editor windows.
#[derive(Default)]
pub struct InputContextEditor {
    pub open_contexts: HashMap<String, InputContextEditorState>,
    /// Action names discovered from .inputaction.ron files in content/.
    pub available_actions: Vec<String>,
}

impl InputContextEditor {
    pub fn new() -> Self {
        Self::default()
    }

    /// Refresh the list of available action names (call after asset rescan).
    pub fn refresh_action_names(&mut self, content_dir: &std::path::Path) {
        self.available_actions = enhanced_serialization::scan_action_names(content_dir);
    }

    /// Open a mapping context for editing. Loads from file if not already open.
    /// Returns the editor key (file path string) for use with PendingWindowRequest.
    pub fn open(&mut self, file_path: PathBuf) -> String {
        let key = file_path.to_string_lossy().to_string();
        if !self.open_contexts.contains_key(&key) {
            let context = enhanced_serialization::load_mapping_context(&file_path)
                .unwrap_or_else(|| MappingContext::new("unnamed", 0));
            self.open_contexts.insert(
                key.clone(),
                InputContextEditorState {
                    context,
                    dirty: false,
                    file_path,
                    open: true,
                    status_message: None,
                    listening_binding: None,
                    listen_start_modifiers: egui::Modifiers::NONE,
                    pending_external_input: None,
                },
            );
        }
        key
    }

    /// Render the mapping context editor UI into a given Ui area.
    /// Called from the secondary window render loop.
    pub fn show_ui(
        ui: &mut egui::Ui,
        state: &mut InputContextEditorState,
        available_actions: &[String],
    ) {
        // ── Input detection (must run before any widgets consume events) ──
        if let Some((entry_idx, bind_idx)) = state.listening_binding {
            // Priority: external (gamepad) → egui key/mouse → modifier-only keys
            let source = state
                .pending_external_input
                .take()
                .or_else(|| detect_input(ui))
                .or_else(|| detect_modifier_press(ui, &state.listen_start_modifiers));

            if let Some(source) = source {
                // Apply detected input to the binding
                if entry_idx < state.context.entries.len() {
                    let entry = &mut state.context.entries[entry_idx];
                    if bind_idx < entry.bindings.len() {
                        entry.bindings[bind_idx].source = source;
                        state.dirty = true;
                    }
                }
                state.listening_binding = None;
            }
        }

        let ctx = &mut state.context;
        let editor_id = ui.id().with("mc_editor");

        // ── Top toolbar ──
        egui::TopBottomPanel::top(editor_id.with("toolbar"))
            .exact_height(32.0)
            .show_inside(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    let save_text = if state.dirty {
                        egui::RichText::new("Save").color(egui::Color32::WHITE)
                    } else {
                        egui::RichText::new("Save")
                    };
                    if ui.button(save_text).clicked() {
                        match enhanced_serialization::save_mapping_context(ctx, &state.file_path) {
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

                    if let Some((msg, time)) = &state.status_message {
                        let elapsed = ui.input(|i| i.time) - time;
                        if elapsed < 3.0 {
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(egui::RichText::new(msg).weak().italics().small());
                                },
                            );
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

        // ── Listening overlay ──
        if state.listening_binding.is_some() {
            // Draw a semi-transparent overlay over the central panel
            egui::Area::new(egui::Id::new("listen_overlay"))
                .order(egui::Order::Foreground)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ui.ctx(), |ui| {
                    egui::Frame::new()
                        .fill(egui::Color32::from_rgba_unmultiplied(20, 20, 30, 220))
                        .corner_radius(8.0)
                        .inner_margin(egui::Margin::same(24))
                        .stroke(egui::Stroke::new(
                            2.0,
                            egui::Color32::from_rgb(100, 160, 255),
                        ))
                        .show(ui, |ui| {
                            ui.vertical_centered(|ui| {
                                ui.add_space(4.0);
                                ui.label(
                                    egui::RichText::new("Listening for input...")
                                        .size(18.0)
                                        .color(egui::Color32::WHITE),
                                );
                                ui.add_space(8.0);
                                ui.label(
                                    egui::RichText::new(
                                        "Press any key, mouse button, or gamepad input",
                                    )
                                    .color(egui::Color32::from_rgb(180, 180, 200)),
                                );
                                ui.add_space(8.0);
                                if ui.button("Cancel").clicked() {
                                    state.listening_binding = None;
                                }
                            });
                        });
                });
            // Request continuous repaint while listening
            ui.ctx().request_repaint();
        }

        // ── Central content ──
        let listening = state.listening_binding;
        egui::CentralPanel::default().show_inside(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.add_space(4.0);

                    // ── Context Properties ──
                    ui.group(|ui| {
                        ui.label(
                            egui::RichText::new("Context Properties")
                                .strong()
                                .color(egui::Color32::from_rgb(140, 180, 220)),
                        );
                        ui.add_space(2.0);

                        egui::Grid::new("mc_props")
                            .num_columns(2)
                            .spacing([8.0, 4.0])
                            .show(ui, |ui| {
                                ui.label("Name:");
                                if ui.text_edit_singleline(&mut ctx.name).changed() {
                                    state.dirty = true;
                                }
                                ui.end_row();

                                ui.label("Priority:");
                                if ui
                                    .add(
                                        egui::DragValue::new(&mut ctx.priority)
                                            .speed(1)
                                            .range(-1000..=1000),
                                    )
                                    .on_hover_text(
                                        "Higher priority contexts are processed first. \
                                         Actions that consume input will block lower-priority contexts.",
                                    )
                                    .changed()
                                {
                                    state.dirty = true;
                                }
                                ui.end_row();
                            });
                    });

                    ui.add_space(6.0);

                    // ── Entries ──
                    ui.group(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new("Action Mappings")
                                    .strong()
                                    .color(egui::Color32::from_rgb(180, 200, 140)),
                            );
                            ui.label(
                                egui::RichText::new(format!("({})", ctx.entries.len()))
                                    .weak()
                                    .small(),
                            );
                        });
                        ui.add_space(2.0);

                        let mut remove_entry = None;
                        let entries_len = ctx.entries.len();
                        for entry_idx in 0..entries_len {
                            let entry = &mut ctx.entries[entry_idx];
                            let entry_id = format!("mc_entry_{}", entry_idx);

                            let header = format!(
                                "{} \u{2014} {} binding{}",
                                entry.action_name,
                                entry.bindings.len(),
                                if entry.bindings.len() == 1 { "" } else { "s" }
                            );

                            egui::CollapsingHeader::new(
                                egui::RichText::new(header).strong().monospace(),
                            )
                            .id_salt(&entry_id)
                            .default_open(true)
                            .show(ui, |ui| {
                                // Action name selector
                                egui::Grid::new(format!("{}_grid", entry_id))
                                    .num_columns(2)
                                    .spacing([8.0, 4.0])
                                    .show(ui, |ui| {
                                        ui.label("Action:");
                                        let before = entry.action_name.clone();
                                        let an_id = format!("{}_an", entry_id);
                                        egui::ComboBox::from_id_salt(&an_id)
                                            .selected_text(&entry.action_name)
                                            .width(180.0)
                                            .show_ui(ui, |ui| {
                                                for name in available_actions {
                                                    ui.selectable_value(
                                                        &mut entry.action_name,
                                                        name.clone(),
                                                        name,
                                                    );
                                                }
                                            });
                                        if entry.action_name != before {
                                            state.dirty = true;
                                        }
                                        ui.end_row();
                                    });

                                ui.add_space(4.0);

                                // Bindings
                                ui.label(egui::RichText::new("Bindings:").strong());

                                let mut remove_bind = None;
                                for (bi, binding) in entry.bindings.iter_mut().enumerate() {
                                    let bind_id = format!("{}_b{}", entry_id, bi);
                                    let is_listening =
                                        listening == Some((entry_idx, bi));

                                    ui.group(|ui| {
                                        ui.horizontal(|ui| {
                                            ui.label(
                                                egui::RichText::new(format!("#{}", bi + 1))
                                                    .weak()
                                                    .small()
                                                    .monospace(),
                                            );

                                            // Listen button
                                            if is_listening {
                                                ui.add(egui::Button::new(
                                                    egui::RichText::new("...")
                                                        .color(egui::Color32::from_rgb(
                                                            100, 200, 255,
                                                        )),
                                                ));
                                            } else {
                                                let listen_btn = ui
                                                    .button(
                                                        egui::RichText::new("\u{1F3A7}")
                                                            .size(14.0),
                                                    )
                                                    .on_hover_text(
                                                        "Click to listen for key/mouse input",
                                                    );
                                                if listen_btn.clicked() {
                                                    state.listening_binding =
                                                        Some((entry_idx, bi));
                                                    state.listen_start_modifiers =
                                                        ui.input(|i| i.modifiers);
                                                }
                                            }

                                            super::input_settings_panel::render_binding_editor_pub(
                                                ui,
                                                binding,
                                                &bind_id,
                                                InputValueType::Digital,
                                            );
                                            if ui
                                                .small_button(
                                                    egui::RichText::new("\u{2716}")
                                                        .color(egui::Color32::from_rgb(
                                                            200, 80, 80,
                                                        )),
                                                )
                                                .on_hover_text("Remove binding")
                                                .clicked()
                                            {
                                                remove_bind = Some(bi);
                                                state.dirty = true;
                                            }
                                        });

                                        // Per-binding modifiers and triggers (always expandable)
                                        egui::CollapsingHeader::new(
                                            egui::RichText::new(format!(
                                                "Modifiers ({}) / Triggers ({})",
                                                binding.modifiers.len(),
                                                binding.triggers.len()
                                            ))
                                            .small()
                                            .weak(),
                                        )
                                        .id_salt(format!("{}_details", bind_id))
                                        .default_open(
                                            !binding.modifiers.is_empty()
                                                || !binding.triggers.is_empty(),
                                        )
                                        .show(ui, |ui| {
                                            ui.label(egui::RichText::new("Modifiers:").small());
                                            let before = binding.modifiers.len();
                                            super::input_settings_panel::render_modifier_list_pub(
                                                ui,
                                                &mut binding.modifiers,
                                                &format!("{}_mod", bind_id),
                                            );
                                            if binding.modifiers.len() != before {
                                                state.dirty = true;
                                            }

                                            ui.add_space(2.0);

                                            ui.label(egui::RichText::new("Triggers:").small());
                                            let before = binding.triggers.len();
                                            super::input_settings_panel::render_trigger_list_pub(
                                                ui,
                                                &mut binding.triggers,
                                                &format!("{}_trig", bind_id),
                                            );
                                            if binding.triggers.len() != before {
                                                state.dirty = true;
                                            }
                                        });
                                    });
                                }

                                if let Some(idx) = remove_bind {
                                    entry.bindings.remove(idx);
                                }

                                ui.horizontal(|ui| {
                                    if ui.button("+ Add Binding").clicked() {
                                        entry.bindings.push(EnhancedBinding::digital(
                                            InputSource::Key(KeyCode::Space),
                                        ));
                                        state.dirty = true;
                                    }

                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            if ui
                                                .button(
                                                    egui::RichText::new("Remove Entry")
                                                        .color(egui::Color32::from_rgb(
                                                            200, 80, 80,
                                                        )),
                                                )
                                                .clicked()
                                            {
                                                remove_entry = Some(entry_idx);
                                                state.dirty = true;
                                            }
                                        },
                                    );
                                });
                            });
                        }

                        if let Some(idx) = remove_entry {
                            ctx.entries.remove(idx);
                        }

                        ui.add_space(4.0);
                        if ui.button("+ Add Entry").clicked() {
                            let action_name = available_actions
                                .first()
                                .cloned()
                                .unwrap_or_else(|| "unnamed".to_string());
                            ctx.entries.push(MappingContextEntry::new(action_name));
                            state.dirty = true;
                        }
                    });
                });
        });
    }
}

// ── Input detection helpers ──

/// Check egui input events for a key press or mouse button press.
/// Returns the detected `InputSource`, or `None` if nothing was pressed.
fn detect_input(ui: &egui::Ui) -> Option<InputSource> {
    ui.input(|input| {
        for event in &input.events {
            match event {
                egui::Event::Key {
                    key,
                    pressed: true,
                    repeat: false,
                    ..
                } => {
                    if let Some(kc) = egui_key_to_engine_keycode(*key) {
                        return Some(InputSource::Key(kc));
                    }
                }
                egui::Event::PointerButton {
                    button,
                    pressed: true,
                    ..
                } => {
                    if let Some(mb) = egui_pointer_to_mouse_button(*button) {
                        return Some(InputSource::MouseButton(mb));
                    }
                }
                _ => {}
            }
        }
        None
    })
}

/// Detect modifier-only key presses (Shift, Ctrl, Alt, Super/Windows).
/// egui doesn't emit Key events for modifiers — they're only tracked via `Modifiers`.
/// Compares current modifiers against the state captured when listening started.
fn detect_modifier_press(
    ui: &egui::Ui,
    start_mods: &egui::Modifiers,
) -> Option<InputSource> {
    ui.input(|input| {
        let current = input.modifiers;
        if current.shift && !start_mods.shift {
            return Some(InputSource::Key(KeyCode::ShiftLeft));
        }
        if current.ctrl && !start_mods.ctrl {
            return Some(InputSource::Key(KeyCode::ControlLeft));
        }
        if current.alt && !start_mods.alt {
            return Some(InputSource::Key(KeyCode::AltLeft));
        }
        if current.command && !start_mods.command {
            return Some(InputSource::Key(KeyCode::SuperLeft));
        }
        None
    })
}

/// Map egui::Key to our engine's KeyCode.
fn egui_key_to_engine_keycode(key: egui::Key) -> Option<KeyCode> {
    use egui::Key as K;
    match key {
        K::A => Some(KeyCode::KeyA),
        K::B => Some(KeyCode::KeyB),
        K::C => Some(KeyCode::KeyC),
        K::D => Some(KeyCode::KeyD),
        K::E => Some(KeyCode::KeyE),
        K::F => Some(KeyCode::KeyF),
        K::G => Some(KeyCode::KeyG),
        K::H => Some(KeyCode::KeyH),
        K::I => Some(KeyCode::KeyI),
        K::J => Some(KeyCode::KeyJ),
        K::K => Some(KeyCode::KeyK),
        K::L => Some(KeyCode::KeyL),
        K::M => Some(KeyCode::KeyM),
        K::N => Some(KeyCode::KeyN),
        K::O => Some(KeyCode::KeyO),
        K::P => Some(KeyCode::KeyP),
        K::Q => Some(KeyCode::KeyQ),
        K::R => Some(KeyCode::KeyR),
        K::S => Some(KeyCode::KeyS),
        K::T => Some(KeyCode::KeyT),
        K::U => Some(KeyCode::KeyU),
        K::V => Some(KeyCode::KeyV),
        K::W => Some(KeyCode::KeyW),
        K::X => Some(KeyCode::KeyX),
        K::Y => Some(KeyCode::KeyY),
        K::Z => Some(KeyCode::KeyZ),
        K::Num0 => Some(KeyCode::Digit0),
        K::Num1 => Some(KeyCode::Digit1),
        K::Num2 => Some(KeyCode::Digit2),
        K::Num3 => Some(KeyCode::Digit3),
        K::Num4 => Some(KeyCode::Digit4),
        K::Num5 => Some(KeyCode::Digit5),
        K::Num6 => Some(KeyCode::Digit6),
        K::Num7 => Some(KeyCode::Digit7),
        K::Num8 => Some(KeyCode::Digit8),
        K::Num9 => Some(KeyCode::Digit9),
        K::F1 => Some(KeyCode::F1),
        K::F2 => Some(KeyCode::F2),
        K::F3 => Some(KeyCode::F3),
        K::F4 => Some(KeyCode::F4),
        K::F5 => Some(KeyCode::F5),
        K::F6 => Some(KeyCode::F6),
        K::F7 => Some(KeyCode::F7),
        K::F8 => Some(KeyCode::F8),
        K::F9 => Some(KeyCode::F9),
        K::F10 => Some(KeyCode::F10),
        K::F11 => Some(KeyCode::F11),
        K::F12 => Some(KeyCode::F12),
        K::ArrowDown => Some(KeyCode::ArrowDown),
        K::ArrowLeft => Some(KeyCode::ArrowLeft),
        K::ArrowRight => Some(KeyCode::ArrowRight),
        K::ArrowUp => Some(KeyCode::ArrowUp),
        K::Escape => Some(KeyCode::Escape),
        K::Tab => Some(KeyCode::Tab),
        K::Backspace => Some(KeyCode::Backspace),
        K::Enter => Some(KeyCode::Enter),
        K::Space => Some(KeyCode::Space),
        K::Delete => Some(KeyCode::Delete),
        K::Insert => Some(KeyCode::Insert),
        K::Home => Some(KeyCode::Home),
        K::End => Some(KeyCode::End),
        K::PageUp => Some(KeyCode::PageUp),
        K::PageDown => Some(KeyCode::PageDown),
        K::Minus => Some(KeyCode::Minus),
        K::Plus => Some(KeyCode::Equal), // Plus/Equal share key
        K::Comma => Some(KeyCode::Comma),
        K::Period => Some(KeyCode::Period),
        K::Semicolon => Some(KeyCode::Semicolon),
        K::Backtick => Some(KeyCode::Backquote),
        K::Backslash => Some(KeyCode::Backslash),
        K::Slash => Some(KeyCode::Slash),
        K::OpenBracket => Some(KeyCode::BracketLeft),
        K::CloseBracket => Some(KeyCode::BracketRight),
        K::Quote => Some(KeyCode::Quote),
        _ => None,
    }
}

/// Map egui::PointerButton to our engine's MouseButton.
fn egui_pointer_to_mouse_button(btn: egui::PointerButton) -> Option<MouseButton> {
    match btn {
        egui::PointerButton::Primary => Some(MouseButton::Left),
        egui::PointerButton::Secondary => Some(MouseButton::Right),
        egui::PointerButton::Middle => Some(MouseButton::Middle),
        egui::PointerButton::Extra1 => Some(MouseButton::Back),
        egui::PointerButton::Extra2 => Some(MouseButton::Forward),
    }
}
