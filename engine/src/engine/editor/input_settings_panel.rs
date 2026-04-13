//! Editor panel for viewing and editing the Enhanced Input action set.
//!
//! Holds an editable working copy of the InputActionSet. Changes are applied
//! back to the InputSubsystem when the user clicks "Apply".

use crate::engine::input::action::{
    GamepadAxisType, GamepadButton, InputSource, KeyCode, MouseAxisType, MouseButton,
};
use crate::engine::input::enhanced_action::{
    EnhancedBinding, InputActionDefinition, InputActionSet, MappingContext, MappingContextEntry,
};
use crate::engine::input::enhanced_defaults::default_action_set;
use crate::engine::input::enhanced_serialization;
use crate::engine::input::modifier::{CurveType, DeadZoneKind, InputModifier, SwizzleOrder};
use crate::engine::input::trigger::InputTrigger;
use crate::engine::input::value::InputValueType;
use egui::Ui;

/// Editor panel for enhanced input settings.
#[derive(Default)]
pub struct InputSettingsPanel {
    working_copy: Option<InputActionSet>,
    pending_apply: Option<InputActionSet>,
    status_message: Option<(String, f64)>,
}

impl InputSettingsPanel {
    pub fn new() -> Self {
        Self::default()
    }

    /// Take the pending apply. Caller writes it back to the InputSubsystem.
    pub fn take_pending_apply(&mut self) -> Option<InputActionSet> {
        self.pending_apply.take()
    }

    /// Render the panel contents.
    pub fn show_contents(&mut self, ui: &mut Ui, resource_set: Option<&InputActionSet>) {
        if self.working_copy.is_none() {
            self.working_copy = resource_set.cloned();
        }

        ui.heading("Input Settings");
        ui.separator();

        // Toolbar
        ui.horizontal(|ui| {
            if ui.button("Apply").clicked() {
                if let Some(set) = &self.working_copy {
                    self.pending_apply = Some(set.clone());
                    self.set_status(ui, "Applied to runtime");
                }
            }

            if ui.button("Save to File").clicked() {
                if let Some(set) = &self.working_copy {
                    let path = enhanced_serialization::default_action_set_path();
                    match enhanced_serialization::save_action_set(set, &path) {
                        Ok(()) => self.set_status(ui, &format!("Saved to {}", path.display())),
                        Err(e) => self.set_status(ui, &format!("Save failed: {e}")),
                    }
                }
            }

            if ui.button("Reload from Resource").clicked() {
                self.working_copy = resource_set.cloned();
                self.set_status(ui, "Reloaded from runtime resource");
            }

            if ui.button("Reset to Defaults").clicked() {
                self.working_copy = Some(default_action_set());
                self.set_status(ui, "Reset to defaults (click Apply to use)");
            }
        });

        // Status message
        if let Some((msg, time)) = &self.status_message {
            let elapsed = ui.input(|i| i.time) - time;
            if elapsed < 3.0 {
                ui.label(egui::RichText::new(msg).weak().italics());
            } else {
                self.status_message = None;
            }
        }

        ui.separator();

        let Some(set) = &mut self.working_copy else {
            ui.label("No InputActionSet available.");
            return;
        };

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                // --- Actions Section ---
                egui::CollapsingHeader::new(egui::RichText::new("Actions").heading())
                    .default_open(true)
                    .show(ui, |ui| {
                        render_actions_section(ui, set);
                    });

                ui.add_space(8.0);

                // --- Mapping Contexts Section ---
                egui::CollapsingHeader::new(egui::RichText::new("Mapping Contexts").heading())
                    .default_open(true)
                    .show(ui, |ui| {
                        render_contexts_section(ui, set);
                    });
            });
    }

    fn set_status(&mut self, ui: &Ui, msg: &str) {
        self.status_message = Some((msg.to_string(), ui.input(|i| i.time)));
    }
}

// ── Actions Section ──

fn render_actions_section(ui: &mut Ui, set: &mut InputActionSet) {
    let mut action_names: Vec<String> = set.actions.keys().cloned().collect();
    action_names.sort();

    let mut remove_action = None;

    for name in &action_names {
        let Some(action) = set.actions.get_mut(name) else {
            continue;
        };
        let id = format!("action_{}", name);

        egui::CollapsingHeader::new(egui::RichText::new(name).monospace())
            .id_salt(&id)
            .default_open(false)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Value Type:");
                    let vt_id = format!("{}_vt", id);
                    egui::ComboBox::from_id_salt(&vt_id)
                        .selected_text(format_value_type(action.value_type))
                        .width(80.0)
                        .show_ui(ui, |ui| {
                            for vt in VALUE_TYPES {
                                ui.selectable_value(
                                    &mut action.value_type,
                                    *vt,
                                    format_value_type(*vt),
                                );
                            }
                        });

                    ui.checkbox(&mut action.consumes_input, "Consumes Input");
                });

                // Per-action triggers
                ui.label(egui::RichText::new("Triggers:").strong());
                render_trigger_list(ui, &mut action.triggers, &format!("{}_trig", id));

                // Per-action modifiers
                ui.label(egui::RichText::new("Modifiers:").strong());
                render_modifier_list(ui, &mut action.modifiers, &format!("{}_mod", id));

                if ui
                    .small_button(egui::RichText::new("Delete Action").color(egui::Color32::from_rgb(200, 80, 80)))
                    .clicked()
                {
                    remove_action = Some(name.clone());
                }
            });
    }

    if let Some(name) = remove_action {
        set.actions.remove(&name);
    }

    // Add new action
    ui.horizontal(|ui| {
        if ui.button("+ New Action").clicked() {
            let name = format!("new_action_{}", set.actions.len());
            set.add_action(InputActionDefinition::new(&name, InputValueType::Digital));
        }
    });
}

// ── Mapping Contexts Section ──

fn render_contexts_section(ui: &mut Ui, set: &mut InputActionSet) {
    let mut remove_ctx = None;
    let action_names: Vec<String> = set.actions.keys().cloned().collect();

    for ctx_idx in 0..set.contexts.len() {
        let ctx = &mut set.contexts[ctx_idx];
        let ctx_id = format!("ctx_{}", ctx.name);

        let header = format!("{} (priority: {})", ctx.name, ctx.priority);
        egui::CollapsingHeader::new(egui::RichText::new(header).strong())
            .id_salt(&ctx_id)
            .default_open(true)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Priority:");
                    ui.add(
                        egui::DragValue::new(&mut ctx.priority)
                            .speed(1)
                            .range(-1000..=1000),
                    );
                });

                // Entries
                let mut remove_entry = None;
                for entry_idx in 0..ctx.entries.len() {
                    let entry = &mut ctx.entries[entry_idx];
                    let entry_id = format!("{}_{}", ctx_id, entry.action_name);

                    egui::CollapsingHeader::new(&entry.action_name)
                        .id_salt(&entry_id)
                        .default_open(false)
                        .show(ui, |ui| {
                            // Action name selector
                            ui.horizontal(|ui| {
                                ui.label("Action:");
                                let an_id = format!("{}_an", entry_id);
                                egui::ComboBox::from_id_salt(&an_id)
                                    .selected_text(&entry.action_name)
                                    .width(120.0)
                                    .show_ui(ui, |ui| {
                                        for name in &action_names {
                                            ui.selectable_value(
                                                &mut entry.action_name,
                                                name.clone(),
                                                name,
                                            );
                                        }
                                    });
                            });

                            // Bindings
                            ui.label(egui::RichText::new("Bindings:").strong());
                            let value_type = set
                                .actions
                                .get(&entry.action_name)
                                .map(|a| a.value_type)
                                .unwrap_or(InputValueType::Digital);

                            let mut remove_bind = None;
                            for (bi, binding) in entry.bindings.iter_mut().enumerate() {
                                let bind_id = format!("{}_b{}", entry_id, bi);
                                ui.horizontal(|ui| {
                                    render_binding_editor(ui, binding, &bind_id, value_type);
                                    if ui
                                        .small_button(
                                            egui::RichText::new("X")
                                                .color(egui::Color32::from_rgb(200, 80, 80)),
                                        )
                                        .clicked()
                                    {
                                        remove_bind = Some(bi);
                                    }
                                });

                                // Per-binding modifiers (collapsible, inline)
                                if !binding.modifiers.is_empty() || !binding.triggers.is_empty() {
                                    ui.indent(format!("{}_details", bind_id), |ui| {
                                        if !binding.modifiers.is_empty() {
                                            ui.label(egui::RichText::new("Modifiers:").small());
                                            render_modifier_list(
                                                ui,
                                                &mut binding.modifiers,
                                                &format!("{}_mod", bind_id),
                                            );
                                        }
                                        if !binding.triggers.is_empty() {
                                            ui.label(egui::RichText::new("Triggers:").small());
                                            render_trigger_list(
                                                ui,
                                                &mut binding.triggers,
                                                &format!("{}_trig", bind_id),
                                            );
                                        }
                                    });
                                }
                            }

                            if let Some(idx) = remove_bind {
                                entry.bindings.remove(idx);
                            }

                            if ui.small_button("+ Add Binding").clicked() {
                                entry.bindings.push(EnhancedBinding::digital(InputSource::Key(
                                    KeyCode::Space,
                                )));
                            }

                            if ui
                                .small_button(
                                    egui::RichText::new("Remove Entry")
                                        .color(egui::Color32::from_rgb(200, 80, 80)),
                                )
                                .clicked()
                            {
                                remove_entry = Some(entry_idx);
                            }
                        });
                }

                if let Some(idx) = remove_entry {
                    ctx.entries.remove(idx);
                }

                // Add entry button
                if ui.small_button("+ Add Entry").clicked() {
                    let action_name = action_names
                        .first()
                        .cloned()
                        .unwrap_or_else(|| "unnamed".to_string());
                    ctx.entries.push(MappingContextEntry::new(action_name));
                }

                if ui
                    .small_button(
                        egui::RichText::new("Delete Context")
                            .color(egui::Color32::from_rgb(200, 80, 80)),
                    )
                    .clicked()
                {
                    remove_ctx = Some(ctx_idx);
                }
            });
    }

    if let Some(idx) = remove_ctx {
        set.contexts.remove(idx);
    }

    if ui.button("+ New Context").clicked() {
        let name = format!("context_{}", set.contexts.len());
        set.add_context(MappingContext::new(name, 0));
    }
}

// ── Binding Editor ──

fn render_binding_editor(
    ui: &mut Ui,
    binding: &mut EnhancedBinding,
    id: &str,
    value_type: InputValueType,
) {
    let source_type = source_type_label(&binding.source);

    // Source type selector
    let type_id = format!("{}_type", id);
    egui::ComboBox::from_id_salt(&type_id)
        .selected_text(source_type)
        .width(90.0)
        .show_ui(ui, |ui| {
            if ui
                .selectable_label(matches!(&binding.source, InputSource::Key(_)), "Key")
                .clicked()
            {
                binding.source = InputSource::Key(KeyCode::Space);
            }
            if ui
                .selectable_label(
                    matches!(&binding.source, InputSource::MouseButton(_)),
                    "Mouse Btn",
                )
                .clicked()
            {
                binding.source = InputSource::MouseButton(MouseButton::Left);
            }
            if ui
                .selectable_label(
                    matches!(&binding.source, InputSource::MouseAxis(_)),
                    "Mouse Axis",
                )
                .clicked()
            {
                binding.source = InputSource::MouseAxis(MouseAxisType::MoveX);
            }
            if ui
                .selectable_label(
                    matches!(&binding.source, InputSource::GamepadButton(_)),
                    "GP Button",
                )
                .clicked()
            {
                binding.source = InputSource::GamepadButton(GamepadButton::South);
            }
            if ui
                .selectable_label(
                    matches!(&binding.source, InputSource::GamepadAxis(_)),
                    "GP Axis",
                )
                .clicked()
            {
                binding.source = InputSource::GamepadAxis(GamepadAxisType::LeftStickX);
            }
        });

    // Value selector
    let val_id = format!("{}_val", id);
    match &mut binding.source {
        InputSource::Key(key) => render_key_combo(ui, key, &val_id),
        InputSource::MouseButton(btn) => render_enum_combo(ui, btn, &val_id, MOUSE_BUTTONS),
        InputSource::MouseAxis(axis) => render_enum_combo(ui, axis, &val_id, MOUSE_AXES),
        InputSource::GamepadButton(btn) => render_enum_combo(ui, btn, &val_id, GAMEPAD_BUTTONS),
        InputSource::GamepadAxis(axis) => render_enum_combo(ui, axis, &val_id, GAMEPAD_AXES),
    }

    // Axis contribution
    match value_type {
        InputValueType::Axis1D => {
            ui.add(
                egui::DragValue::new(&mut binding.axis_contribution.0)
                    .range(-10.0..=10.0)
                    .speed(0.1)
                    .prefix("val: "),
            );
        }
        InputValueType::Axis2D | InputValueType::Axis3D => {
            ui.add(
                egui::DragValue::new(&mut binding.axis_contribution.0)
                    .range(-10.0..=10.0)
                    .speed(0.1)
                    .prefix("x: "),
            );
            ui.add(
                egui::DragValue::new(&mut binding.axis_contribution.1)
                    .range(-10.0..=10.0)
                    .speed(0.1)
                    .prefix("y: "),
            );
        }
        InputValueType::Digital => {}
    }
}

// ── Modifier List Editor ──

fn render_modifier_list(ui: &mut Ui, modifiers: &mut Vec<InputModifier>, id: &str) {
    let mut remove_idx = None;
    for (i, modifier) in modifiers.iter_mut().enumerate() {
        let mod_id = format!("{}_{}", id, i);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(modifier_label(modifier)).monospace().small());
            render_modifier_params(ui, modifier, &mod_id);
            if ui
                .small_button(egui::RichText::new("X").color(egui::Color32::from_rgb(200, 80, 80)))
                .clicked()
            {
                remove_idx = Some(i);
            }
        });
    }
    if let Some(idx) = remove_idx {
        modifiers.remove(idx);
    }

    // Add modifier
    let add_id = format!("{}_add", id);
    egui::ComboBox::from_id_salt(&add_id)
        .selected_text("+ Add Modifier")
        .width(120.0)
        .show_ui(ui, |ui| {
            if ui.selectable_label(false, "Negate").clicked() {
                modifiers.push(InputModifier::Negate {
                    x: true,
                    y: false,
                    z: false,
                });
            }
            if ui.selectable_label(false, "Swizzle").clicked() {
                modifiers.push(InputModifier::Swizzle {
                    order: SwizzleOrder::YXZ,
                });
            }
            if ui.selectable_label(false, "DeadZone").clicked() {
                modifiers.push(InputModifier::DeadZone {
                    lower: 0.15,
                    upper: 0.95,
                    kind: DeadZoneKind::Radial,
                });
            }
            if ui.selectable_label(false, "Scale").clicked() {
                modifiers.push(InputModifier::Scale {
                    factor: glam::Vec3::ONE,
                });
            }
            if ui.selectable_label(false, "Smooth").clicked() {
                modifiers.push(InputModifier::Smooth {
                    speed: 10.0,
                    previous: None,
                });
            }
            if ui.selectable_label(false, "ResponseCurve").clicked() {
                modifiers.push(InputModifier::ResponseCurve {
                    curve: CurveType::Linear,
                });
            }
            if ui.selectable_label(false, "Clamp").clicked() {
                modifiers.push(InputModifier::Clamp {
                    min: glam::Vec3::splat(-1.0),
                    max: glam::Vec3::splat(1.0),
                });
            }
        });
}

fn modifier_label(m: &InputModifier) -> &'static str {
    match m {
        InputModifier::Negate { .. } => "Negate",
        InputModifier::Swizzle { .. } => "Swizzle",
        InputModifier::DeadZone { .. } => "DeadZone",
        InputModifier::Scale { .. } => "Scale",
        InputModifier::Smooth { .. } => "Smooth",
        InputModifier::ResponseCurve { .. } => "Curve",
        InputModifier::Clamp { .. } => "Clamp",
    }
}

fn render_modifier_params(ui: &mut Ui, modifier: &mut InputModifier, id: &str) {
    match modifier {
        InputModifier::Negate { x, y, z } => {
            ui.checkbox(x, "X");
            ui.checkbox(y, "Y");
            ui.checkbox(z, "Z");
        }
        InputModifier::DeadZone { lower, upper, kind } => {
            ui.add(egui::DragValue::new(lower).range(0.0..=1.0).speed(0.01).prefix("lo:"));
            ui.add(egui::DragValue::new(upper).range(0.0..=1.0).speed(0.01).prefix("hi:"));
            let kind_id = format!("{}_dzk", id);
            egui::ComboBox::from_id_salt(&kind_id)
                .selected_text(format!("{:?}", kind))
                .width(60.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(kind, DeadZoneKind::Radial, "Radial");
                    ui.selectable_value(kind, DeadZoneKind::PerAxis, "PerAxis");
                });
        }
        InputModifier::Scale { factor } => {
            ui.add(egui::DragValue::new(&mut factor.x).speed(0.1).prefix("x:"));
            ui.add(egui::DragValue::new(&mut factor.y).speed(0.1).prefix("y:"));
            ui.add(egui::DragValue::new(&mut factor.z).speed(0.1).prefix("z:"));
        }
        InputModifier::Smooth { speed, .. } => {
            ui.add(egui::DragValue::new(speed).range(0.1..=100.0).speed(0.5).prefix("spd:"));
        }
        InputModifier::ResponseCurve { curve } => {
            let curve_id = format!("{}_curve", id);
            egui::ComboBox::from_id_salt(&curve_id)
                .selected_text(curve_label(curve))
                .width(80.0)
                .show_ui(ui, |ui| {
                    if ui.selectable_label(matches!(curve, CurveType::Linear), "Linear").clicked() {
                        *curve = CurveType::Linear;
                    }
                    if ui
                        .selectable_label(matches!(curve, CurveType::Quadratic), "Quadratic")
                        .clicked()
                    {
                        *curve = CurveType::Quadratic;
                    }
                    if ui.selectable_label(matches!(curve, CurveType::Cubic), "Cubic").clicked() {
                        *curve = CurveType::Cubic;
                    }
                });
        }
        InputModifier::Clamp { min, max } => {
            ui.add(egui::DragValue::new(&mut min.x).speed(0.1).prefix("min:"));
            ui.add(egui::DragValue::new(&mut max.x).speed(0.1).prefix("max:"));
        }
        InputModifier::Swizzle { order } => {
            let sw_id = format!("{}_sw", id);
            egui::ComboBox::from_id_salt(&sw_id)
                .selected_text(format!("{:?}", order))
                .width(60.0)
                .show_ui(ui, |ui| {
                    for o in SWIZZLE_ORDERS {
                        ui.selectable_value(order, *o, format!("{:?}", o));
                    }
                });
        }
    }
}

fn curve_label(c: &CurveType) -> &'static str {
    match c {
        CurveType::Linear => "Linear",
        CurveType::Quadratic => "Quadratic",
        CurveType::Cubic => "Cubic",
        CurveType::Custom(_) => "Custom",
    }
}

// ── Trigger List Editor ──

fn render_trigger_list(ui: &mut Ui, triggers: &mut Vec<InputTrigger>, id: &str) {
    let mut remove_idx = None;
    for (i, trigger) in triggers.iter_mut().enumerate() {
        let trig_id = format!("{}_{}", id, i);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(trigger_label(trigger)).monospace().small());
            render_trigger_params(ui, trigger, &trig_id);
            if ui
                .small_button(egui::RichText::new("X").color(egui::Color32::from_rgb(200, 80, 80)))
                .clicked()
            {
                remove_idx = Some(i);
            }
        });
    }
    if let Some(idx) = remove_idx {
        triggers.remove(idx);
    }

    // Add trigger
    let add_id = format!("{}_add", id);
    egui::ComboBox::from_id_salt(&add_id)
        .selected_text("+ Add Trigger")
        .width(110.0)
        .show_ui(ui, |ui| {
            if ui.selectable_label(false, "Down").clicked() {
                triggers.push(InputTrigger::Down);
            }
            if ui.selectable_label(false, "Pressed").clicked() {
                triggers.push(InputTrigger::Pressed);
            }
            if ui.selectable_label(false, "Released").clicked() {
                triggers.push(InputTrigger::Released);
            }
            if ui.selectable_label(false, "Held").clicked() {
                triggers.push(InputTrigger::Held {
                    duration: 0.5,
                    elapsed: 0.0,
                    fired: false,
                });
            }
            if ui.selectable_label(false, "Tap").clicked() {
                triggers.push(InputTrigger::Tap {
                    max_duration: 0.3,
                    elapsed: 0.0,
                    was_active: false,
                });
            }
            if ui.selectable_label(false, "Pulse").clicked() {
                triggers.push(InputTrigger::Pulse {
                    interval: 0.5,
                    trigger_limit: 0,
                    elapsed: 0.0,
                    pulse_count: 0,
                });
            }
            if ui.selectable_label(false, "ChordAction").clicked() {
                triggers.push(InputTrigger::ChordAction {
                    action_name: String::new(),
                });
            }
        });
}

fn trigger_label(t: &InputTrigger) -> &'static str {
    match t {
        InputTrigger::Down => "Down",
        InputTrigger::Pressed => "Pressed",
        InputTrigger::Released => "Released",
        InputTrigger::Held { .. } => "Held",
        InputTrigger::Tap { .. } => "Tap",
        InputTrigger::Pulse { .. } => "Pulse",
        InputTrigger::ChordAction { .. } => "Chord",
    }
}

fn render_trigger_params(ui: &mut Ui, trigger: &mut InputTrigger, _id: &str) {
    match trigger {
        InputTrigger::Held { duration, .. } => {
            ui.add(
                egui::DragValue::new(duration)
                    .range(0.01..=10.0)
                    .speed(0.05)
                    .prefix("dur: ")
                    .suffix("s"),
            );
        }
        InputTrigger::Tap { max_duration, .. } => {
            ui.add(
                egui::DragValue::new(max_duration)
                    .range(0.01..=5.0)
                    .speed(0.05)
                    .prefix("max: ")
                    .suffix("s"),
            );
        }
        InputTrigger::Pulse {
            interval,
            trigger_limit,
            ..
        } => {
            ui.add(
                egui::DragValue::new(interval)
                    .range(0.01..=10.0)
                    .speed(0.05)
                    .prefix("int: ")
                    .suffix("s"),
            );
            ui.add(
                egui::DragValue::new(trigger_limit)
                    .speed(1)
                    .prefix("lim: "),
            );
        }
        InputTrigger::ChordAction { action_name } => {
            ui.text_edit_singleline(action_name);
        }
        _ => {}
    }
}

// ── Shared helpers ──

fn format_value_type(vt: InputValueType) -> &'static str {
    match vt {
        InputValueType::Digital => "Digital",
        InputValueType::Axis1D => "Axis1D",
        InputValueType::Axis2D => "Axis2D",
        InputValueType::Axis3D => "Axis3D",
    }
}

fn source_type_label(source: &InputSource) -> &'static str {
    match source {
        InputSource::Key(_) => "Key",
        InputSource::MouseButton(_) => "Mouse Btn",
        InputSource::MouseAxis(_) => "Mouse Axis",
        InputSource::GamepadButton(_) => "GP Button",
        InputSource::GamepadAxis(_) => "GP Axis",
    }
}

fn render_enum_combo<T: Copy + PartialEq + std::fmt::Debug>(
    ui: &mut Ui,
    current: &mut T,
    id: &str,
    variants: &[T],
) {
    egui::ComboBox::from_id_salt(id)
        .selected_text(format!("{:?}", current))
        .width(110.0)
        .show_ui(ui, |ui| {
            for v in variants {
                ui.selectable_value(current, *v, format!("{:?}", v));
            }
        });
}

fn render_key_combo(ui: &mut Ui, current: &mut KeyCode, id: &str) {
    egui::ComboBox::from_id_salt(id)
        .selected_text(format!("{:?}", current))
        .width(110.0)
        .show_ui(ui, |ui| {
            ui.label(egui::RichText::new("Letters").small().strong());
            for k in KEY_LETTERS { ui.selectable_value(current, *k, format!("{:?}", k)); }
            ui.separator();
            ui.label(egui::RichText::new("Digits").small().strong());
            for k in KEY_DIGITS { ui.selectable_value(current, *k, format!("{:?}", k)); }
            ui.separator();
            ui.label(egui::RichText::new("Function").small().strong());
            for k in KEY_FUNCTION { ui.selectable_value(current, *k, format!("{:?}", k)); }
            ui.separator();
            ui.label(egui::RichText::new("Navigation").small().strong());
            for k in KEY_NAV { ui.selectable_value(current, *k, format!("{:?}", k)); }
            ui.separator();
            ui.label(egui::RichText::new("Modifiers").small().strong());
            for k in KEY_MODIFIERS { ui.selectable_value(current, *k, format!("{:?}", k)); }
            ui.separator();
            ui.label(egui::RichText::new("Punctuation").small().strong());
            for k in KEY_PUNCTUATION { ui.selectable_value(current, *k, format!("{:?}", k)); }
        });
}

// ── Enum variant tables ──

const VALUE_TYPES: &[InputValueType] = &[
    InputValueType::Digital,
    InputValueType::Axis1D,
    InputValueType::Axis2D,
    InputValueType::Axis3D,
];

const SWIZZLE_ORDERS: &[SwizzleOrder] = &[
    SwizzleOrder::YXZ,
    SwizzleOrder::ZYX,
    SwizzleOrder::XZY,
    SwizzleOrder::YZX,
    SwizzleOrder::ZXY,
];

const MOUSE_BUTTONS: &[MouseButton] = &[
    MouseButton::Left, MouseButton::Right, MouseButton::Middle,
    MouseButton::Back, MouseButton::Forward,
];

const MOUSE_AXES: &[MouseAxisType] = &[
    MouseAxisType::MoveX, MouseAxisType::MoveY, MouseAxisType::ScrollY,
];

const GAMEPAD_BUTTONS: &[GamepadButton] = &[
    GamepadButton::South, GamepadButton::East, GamepadButton::West, GamepadButton::North,
    GamepadButton::LeftBumper, GamepadButton::RightBumper,
    GamepadButton::LeftTrigger, GamepadButton::RightTrigger,
    GamepadButton::Select, GamepadButton::Start,
    GamepadButton::LeftStick, GamepadButton::RightStick,
    GamepadButton::DPadUp, GamepadButton::DPadDown, GamepadButton::DPadLeft, GamepadButton::DPadRight,
];

const GAMEPAD_AXES: &[GamepadAxisType] = &[
    GamepadAxisType::LeftStickX, GamepadAxisType::LeftStickY,
    GamepadAxisType::RightStickX, GamepadAxisType::RightStickY,
    GamepadAxisType::LeftTrigger, GamepadAxisType::RightTrigger,
];

const KEY_LETTERS: &[KeyCode] = &[
    KeyCode::KeyA, KeyCode::KeyB, KeyCode::KeyC, KeyCode::KeyD,
    KeyCode::KeyE, KeyCode::KeyF, KeyCode::KeyG, KeyCode::KeyH,
    KeyCode::KeyI, KeyCode::KeyJ, KeyCode::KeyK, KeyCode::KeyL,
    KeyCode::KeyM, KeyCode::KeyN, KeyCode::KeyO, KeyCode::KeyP,
    KeyCode::KeyQ, KeyCode::KeyR, KeyCode::KeyS, KeyCode::KeyT,
    KeyCode::KeyU, KeyCode::KeyV, KeyCode::KeyW, KeyCode::KeyX,
    KeyCode::KeyY, KeyCode::KeyZ,
];

const KEY_DIGITS: &[KeyCode] = &[
    KeyCode::Digit0, KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3,
    KeyCode::Digit4, KeyCode::Digit5, KeyCode::Digit6, KeyCode::Digit7,
    KeyCode::Digit8, KeyCode::Digit9,
];

const KEY_FUNCTION: &[KeyCode] = &[
    KeyCode::F1, KeyCode::F2, KeyCode::F3, KeyCode::F4,
    KeyCode::F5, KeyCode::F6, KeyCode::F7, KeyCode::F8,
    KeyCode::F9, KeyCode::F10, KeyCode::F11, KeyCode::F12,
];

const KEY_NAV: &[KeyCode] = &[
    KeyCode::Escape, KeyCode::Space, KeyCode::Enter, KeyCode::Backspace,
    KeyCode::Tab, KeyCode::Delete, KeyCode::Insert, KeyCode::Home,
    KeyCode::End, KeyCode::PageUp, KeyCode::PageDown,
    KeyCode::ArrowUp, KeyCode::ArrowDown, KeyCode::ArrowLeft, KeyCode::ArrowRight,
];

const KEY_MODIFIERS: &[KeyCode] = &[
    KeyCode::ShiftLeft, KeyCode::ShiftRight,
    KeyCode::ControlLeft, KeyCode::ControlRight,
    KeyCode::AltLeft, KeyCode::AltRight,
    KeyCode::SuperLeft, KeyCode::SuperRight,
];

const KEY_PUNCTUATION: &[KeyCode] = &[
    KeyCode::Comma, KeyCode::Period, KeyCode::Semicolon, KeyCode::Quote,
    KeyCode::BracketLeft, KeyCode::BracketRight, KeyCode::Backslash,
    KeyCode::Slash, KeyCode::Minus, KeyCode::Equal, KeyCode::Backquote,
];

// ── Public API for reuse by per-asset editors ──

/// Render a trigger list editor. Public wrapper for use by InputActionEditor etc.
pub fn render_trigger_list_pub(ui: &mut Ui, triggers: &mut Vec<InputTrigger>, id: &str) {
    render_trigger_list(ui, triggers, id);
}

/// Render a modifier list editor. Public wrapper for use by InputActionEditor etc.
pub fn render_modifier_list_pub(ui: &mut Ui, modifiers: &mut Vec<InputModifier>, id: &str) {
    render_modifier_list(ui, modifiers, id);
}

/// Render a binding editor row. Public wrapper for use by InputContextEditor etc.
pub fn render_binding_editor_pub(
    ui: &mut Ui,
    binding: &mut EnhancedBinding,
    id: &str,
    value_type: InputValueType,
) {
    render_binding_editor(ui, binding, id, value_type);
}
