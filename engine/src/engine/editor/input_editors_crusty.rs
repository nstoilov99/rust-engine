//! Input settings / input action / mapping context editors rendered with
//! crusty-gui (Phase 16 port).
//!
//! Reads/writes the same state structs as the egui versions
//! (`InputSettingsPanel`, `InputActionEditorState`, `InputContextEditorState`);
//! shared labels/variant tables come from `input_settings_panel`.

use crusty_gui::context::{Direction, Ui, UiOptions};
use crusty_gui::id::Id;
use crusty_gui::input::{Key as CKey, Modifiers as CModifiers, MouseButton as CMouseButton};
use crusty_gui::math::{Color, Pos2, Rect, Vec2};
use crusty_gui::widgets::{
    show_tooltip_for, Button, Checkbox, CollapsingHeader, ComboBox, DragValue, Grid, Label,
    ScrollArea, SelectableValue, TextEdit,
};

use super::input_action_editor::{InputActionEditor, InputActionEditorState};
use super::input_context_editor::{InputContextEditor, InputContextEditorState};
use super::input_settings_panel::{self as isp, InputSettingsPanel};
use crate::engine::input::action::{InputSource, KeyCode, MouseButton};
use crate::engine::input::enhanced_action::{
    EnhancedBinding, InputActionDefinition, InputActionSet, MappingContext, MappingContextEntry,
};
use crate::engine::input::enhanced_defaults::default_action_set;
use crate::engine::input::enhanced_serialization;
use crate::engine::input::modifier::{CurveType, DeadZoneKind, InputModifier, SwizzleOrder};
use crate::engine::input::trigger::InputTrigger;
use crate::engine::input::value::InputValueType;
use crate::engine::input::action::{GamepadAxisType, GamepadButton, MouseAxisType};

// ── small helpers ───────────────────────────────────────────────────────────

fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::from_srgb_u8(r, g, b, 255)
}

fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color {
    Color::from_srgb_u8(r, g, b, a)
}

fn red() -> Color {
    rgb(200, 80, 80)
}

/// Top-left y so text of `font` px sits vertically centered in `rect`.
fn text_y(rect: Rect, font: f32) -> f32 {
    rect.min.y + (rect.height() - font * 1.25) * 0.5
}

/// A label vertically centered against button-height rows.
fn row_label(ui: &mut Ui, text: &str, font: f32, color: Color) {
    let style = ui.style();
    let row_h = style.fonts.body * 1.25 + style.spacing.button_padding.y * 2.0;
    let w = ui.text_mut().measure(text, font, None).x;
    let rect = ui.allocate(Vec2::new(w, row_h));
    ui.painter()
        .text(Pos2::new(rect.min.x, text_y(rect, font)), text, font, color, None);
}

fn small_button(ui: &mut Ui, text: &str) -> bool {
    let small = ui.style().fonts.small;
    Button::new(text).size(small).show(ui).clicked
}

fn small_red_button(ui: &mut Ui, text: &str) -> bool {
    let small = ui.style().fonts.small;
    Button::new(text).size(small).text_color(red()).show(ui).clicked
}

/// Stroke-framed group (egui `ui.group` analogue): full available width,
/// content padded, 1px stroke around the used height.
fn group<R>(ui: &mut Ui, id_src: &str, f: impl FnOnce(&mut Ui) -> R) -> R {
    let style = ui.style();
    let top = ui.cursor();
    let avail = ui.available();
    let pad = 6.0;
    let inner = Rect::from_min_max(top, Pos2::new(avail.max.x, avail.max.y));
    let opts = UiOptions {
        padding: Vec2::new(pad, pad),
        spacing: style.spacing.item,
    };
    let id = ui.alloc_id(("input_group", id_src));
    let (r, used) = ui.run_at(inner, Direction::TopDown, id, opts, f);
    // Fit the frame to the content (egui group behavior), not the full width.
    let bg = Rect::from_min_max(top, Pos2::new(used.max.x + pad, used.max.y + pad));
    ui.painter().rect_stroke(bg, 4.0, 1.0, style.palette.stroke);
    ui.allocate(Vec2::new(bg.width(), bg.height()));
    r
}

/// Indented block (egui `ui.indent` analogue).
fn indent<R>(ui: &mut Ui, f: impl FnOnce(&mut Ui) -> R) -> R {
    ui.horizontal(|ui| {
        ui.add_space(16.0);
        ui.vertical(f)
    })
}

/// One clickable row in a combo popup ("selectable_label" pattern).
fn combo_item(ui: &mut Ui, selected: bool, label: &str) -> bool {
    let mut sel = selected;
    SelectableValue::new(&mut sel, true, label).show(ui).clicked
}

/// Debug-formatted enum combo (egui `render_enum_combo` analogue).
fn enum_combo<T: Copy + PartialEq + std::fmt::Debug>(
    ui: &mut Ui,
    id: String,
    current: &mut T,
    variants: &[T],
    width: f32,
) {
    ComboBox::new(id)
        .selected_text(format!("{:?}", current))
        .width(width)
        .show_ui(ui, |ui| {
            for v in variants {
                SelectableValue::new(current, *v, format!("{:?}", v)).show(ui);
            }
        });
}

fn value_type_combo(ui: &mut Ui, id: String, vt: &mut InputValueType, width: f32) {
    ComboBox::new(id)
        .selected_text(isp::format_value_type(*vt))
        .width(width)
        .show_ui(ui, |ui| {
            for v in isp::VALUE_TYPES {
                SelectableValue::new(vt, *v, isp::format_value_type(*v)).show(ui);
            }
        });
}

/// Combo over string options. Returns true if the value changed.
fn string_combo(
    ui: &mut Ui,
    id: String,
    value: &mut String,
    options: &[String],
    width: f32,
) -> bool {
    let mut changed = false;
    ComboBox::new(id)
        .selected_text(value.clone())
        .width(width)
        .show_ui(ui, |ui| {
            for opt in options {
                if combo_item(ui, *opt == *value, opt) {
                    *value = opt.clone();
                    changed = true;
                }
            }
        });
    changed
}

fn key_group(ui: &mut Ui, title: &str, keys: &[KeyCode], current: &mut KeyCode) {
    let style = ui.style();
    Label::new(title).size(style.fonts.small).show(ui);
    for k in keys {
        SelectableValue::new(current, *k, format!("{:?}", k)).show(ui);
    }
}

/// Key-code combo with grouped sections (egui `render_key_combo` analogue).
/// Tall content scrolls via the ComboBox popup height cap.
fn key_combo(ui: &mut Ui, id: String, current: &mut KeyCode) {
    ComboBox::new(id)
        .selected_text(format!("{:?}", current))
        .width(110.0)
        .show_ui(ui, |ui| {
            key_group(ui, "Letters", isp::KEY_LETTERS, current);
            ui.separator();
            key_group(ui, "Digits", isp::KEY_DIGITS, current);
            ui.separator();
            key_group(ui, "Function", isp::KEY_FUNCTION, current);
            ui.separator();
            key_group(ui, "Navigation", isp::KEY_NAV, current);
            ui.separator();
            key_group(ui, "Modifiers", isp::KEY_MODIFIERS, current);
            ui.separator();
            key_group(ui, "Punctuation", isp::KEY_PUNCTUATION, current);
        });
}

/// i32 DragValue (crusty's DragValue is f32-only). Returns true on change.
fn drag_i32(ui: &mut Ui, v: &mut i32, min: i32, max: i32, tooltip: Option<&str>) -> bool {
    let mut f = *v as f32;
    let r = DragValue::new(&mut f)
        .speed(1.0)
        .decimals(0)
        .range(min as f32..=max as f32)
        .show(ui);
    if let Some(tip) = tooltip {
        if r.hovered {
            show_tooltip_for(ui, r.rect, tip);
        }
    }
    let nv = f.round() as i32;
    let changed = nv != *v;
    *v = nv;
    changed
}

fn drag_u32(ui: &mut Ui, v: &mut u32, prefix: &str) {
    let mut f = *v as f32;
    DragValue::new(&mut f)
        .speed(1.0)
        .decimals(0)
        .range(0.0..=1_000_000.0)
        .prefix(prefix)
        .show(ui);
    *v = f.round().max(0.0) as u32;
}

// ── Input Settings dock panel ───────────────────────────────────────────────

/// Draw the Input Settings panel into the dock tab's content rect.
pub fn input_settings_panel(
    ui: &mut Ui,
    tab_rect: egui::Rect,
    ppp: f32,
    panel: &mut InputSettingsPanel,
    resource_set: Option<&InputActionSet>,
) {
    let rect = super::dock_crusty::rect_px(tab_rect, ppp);
    let style = ui.style();
    let opts = UiOptions {
        padding: Vec2::new(6.0, 4.0),
        spacing: style.spacing.item,
    };
    ui.run_at(
        rect,
        Direction::TopDown,
        Id::new("engine_input_settings"),
        opts,
        |ui| settings_contents(ui, panel, resource_set),
    );
}

fn settings_contents(
    ui: &mut Ui,
    panel: &mut InputSettingsPanel,
    resource_set: Option<&InputActionSet>,
) {
    if panel.working_copy.is_none() {
        panel.working_copy = resource_set.cloned();
    }

    let style = ui.style();
    Label::new("Input Settings").size(style.fonts.title).show(ui);
    ui.separator();

    // Toolbar
    let (apply, save, reload, reset) = ui.horizontal(|ui| {
        (
            Button::new("Apply").show(ui).clicked,
            Button::new("Save to File").show(ui).clicked,
            Button::new("Reload from Resource").show(ui).clicked,
            Button::new("Reset to Defaults").show(ui).clicked,
        )
    });
    let now = ui.ctx().time();
    if apply {
        if let Some(set) = &panel.working_copy {
            panel.pending_apply = Some(set.clone());
            panel.status_message = Some(("Applied to runtime".to_string(), now));
        }
    }
    if save {
        if let Some(set) = &panel.working_copy {
            let path = enhanced_serialization::default_action_set_path();
            let msg = match enhanced_serialization::save_action_set(set, &path) {
                Ok(()) => format!("Saved to {}", path.display()),
                Err(e) => format!("Save failed: {e}"),
            };
            panel.status_message = Some((msg, now));
        }
    }
    if reload {
        panel.working_copy = resource_set.cloned();
        panel.status_message = Some(("Reloaded from runtime resource".to_string(), now));
    }
    if reset {
        panel.working_copy = Some(default_action_set());
        panel.status_message = Some(("Reset to defaults (click Apply to use)".to_string(), now));
    }

    // Status message (fades after 3s)
    let mut clear_status = false;
    if let Some((msg, time)) = &panel.status_message {
        if now - time < 3.0 {
            Label::new(msg.as_str())
                .size(style.fonts.small)
                .color(style.palette.text_dim)
                .show(ui);
        } else {
            clear_status = true;
        }
    }
    if clear_status {
        panel.status_message = None;
    }

    ui.separator();

    let Some(set) = panel.working_copy.as_mut() else {
        Label::new("No InputActionSet available.").show(ui);
        return;
    };

    let h = ui.available_size().y;
    ScrollArea::new(h).auto_shrink(false).show(ui, |ui| {
        let title = ui.style().fonts.title;
        CollapsingHeader::new("Actions")
            .default_open(true)
            .text_size(title)
            .show(ui, |ui| actions_section(ui, set));
        ui.add_space(8.0);
        CollapsingHeader::new("Mapping Contexts")
            .default_open(true)
            .text_size(title)
            .show(ui, |ui| contexts_section(ui, set));
    });
}

fn actions_section(ui: &mut Ui, set: &mut InputActionSet) {
    let style = ui.style();
    let mut action_names: Vec<String> = set.actions.keys().cloned().collect();
    action_names.sort();

    let mut remove_action: Option<String> = None;

    for name in &action_names {
        let Some(action) = set.actions.get_mut(name) else {
            continue;
        };
        let id = format!("action_{name}");
        CollapsingHeader::new(name.as_str()).show(ui, |ui| {
            ui.horizontal(|ui| {
                row_label(ui, "Value Type:", style.fonts.body, style.palette.text);
                value_type_combo(ui, format!("{id}_vt"), &mut action.value_type, 80.0);
                Checkbox::new(&mut action.consumes_input, "Consumes Input").show(ui);
            });

            Label::new("Triggers:").show(ui);
            trigger_list(ui, &mut action.triggers, &format!("{id}_trig"));

            Label::new("Modifiers:").show(ui);
            modifier_list(ui, &mut action.modifiers, &format!("{id}_mod"));

            if small_red_button(ui, "Delete Action") {
                remove_action = Some(name.clone());
            }
        });
    }

    if let Some(name) = remove_action {
        set.actions.remove(&name);
    }

    if Button::new("+ New Action").show(ui).clicked {
        let name = format!("new_action_{}", set.actions.len());
        set.add_action(InputActionDefinition::new(&name, InputValueType::Digital));
    }
}

fn contexts_section(ui: &mut Ui, set: &mut InputActionSet) {
    let style = ui.style();
    let action_names: Vec<String> = set.actions.keys().cloned().collect();
    let value_types: std::collections::HashMap<String, InputValueType> = set
        .actions
        .iter()
        .map(|(k, v)| (k.clone(), v.value_type))
        .collect();

    let mut remove_ctx = None;
    for ctx_idx in 0..set.contexts.len() {
        let mctx = &mut set.contexts[ctx_idx];
        let ctx_id = format!("ctx_{}", mctx.name);
        let header = format!("{} (priority: {})", mctx.name, mctx.priority);

        CollapsingHeader::new(header).default_open(true).show(ui, |ui| {
            ui.horizontal(|ui| {
                row_label(ui, "Priority:", style.fonts.body, style.palette.text);
                drag_i32(ui, &mut mctx.priority, -1000, 1000, None);
            });

            let mut remove_entry = None;
            for entry_idx in 0..mctx.entries.len() {
                let entry = &mut mctx.entries[entry_idx];
                let entry_id = format!("{}_{}", ctx_id, entry.action_name);

                CollapsingHeader::new(entry.action_name.as_str()).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        row_label(ui, "Action:", style.fonts.body, style.palette.text);
                        string_combo(
                            ui,
                            format!("{entry_id}_an"),
                            &mut entry.action_name,
                            &action_names,
                            120.0,
                        );
                    });

                    Label::new("Bindings:").show(ui);
                    let value_type = value_types
                        .get(&entry.action_name)
                        .copied()
                        .unwrap_or(InputValueType::Digital);

                    let mut remove_bind = None;
                    for (bi, binding) in entry.bindings.iter_mut().enumerate() {
                        let bind_id = format!("{entry_id}_b{bi}");
                        ui.horizontal(|ui| {
                            binding_editor(ui, binding, &bind_id, value_type);
                            if small_red_button(ui, "X") {
                                remove_bind = Some(bi);
                            }
                        });

                        if !binding.modifiers.is_empty() || !binding.triggers.is_empty() {
                            indent(ui, |ui| {
                                if !binding.modifiers.is_empty() {
                                    Label::new("Modifiers:").size(style.fonts.small).show(ui);
                                    modifier_list(
                                        ui,
                                        &mut binding.modifiers,
                                        &format!("{bind_id}_mod"),
                                    );
                                }
                                if !binding.triggers.is_empty() {
                                    Label::new("Triggers:").size(style.fonts.small).show(ui);
                                    trigger_list(
                                        ui,
                                        &mut binding.triggers,
                                        &format!("{bind_id}_trig"),
                                    );
                                }
                            });
                        }
                    }
                    if let Some(idx) = remove_bind {
                        entry.bindings.remove(idx);
                    }

                    if small_button(ui, "+ Add Binding") {
                        entry
                            .bindings
                            .push(EnhancedBinding::digital(InputSource::Key(KeyCode::Space)));
                    }
                    if small_red_button(ui, "Remove Entry") {
                        remove_entry = Some(entry_idx);
                    }
                });
            }
            if let Some(idx) = remove_entry {
                mctx.entries.remove(idx);
            }

            if small_button(ui, "+ Add Entry") {
                let action_name = action_names
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "unnamed".to_string());
                mctx.entries.push(MappingContextEntry::new(action_name));
            }
            if small_red_button(ui, "Delete Context") {
                remove_ctx = Some(ctx_idx);
            }
        });
    }
    if let Some(idx) = remove_ctx {
        set.contexts.remove(idx);
    }

    if Button::new("+ New Context").show(ui).clicked {
        let name = format!("context_{}", set.contexts.len());
        set.add_context(MappingContext::new(name, 0));
    }
}

// ── Binding editor ──────────────────────────────────────────────────────────

fn binding_editor(ui: &mut Ui, binding: &mut EnhancedBinding, id: &str, value_type: InputValueType) {
    // Source type selector
    let src_label = isp::source_type_label(&binding.source);
    ComboBox::new(format!("{id}_type"))
        .selected_text(src_label)
        .width(90.0)
        .show_ui(ui, |ui| {
            if combo_item(ui, matches!(binding.source, InputSource::Key(_)), "Key") {
                binding.source = InputSource::Key(KeyCode::Space);
            }
            if combo_item(
                ui,
                matches!(binding.source, InputSource::MouseButton(_)),
                "Mouse Btn",
            ) {
                binding.source = InputSource::MouseButton(MouseButton::Left);
            }
            if combo_item(
                ui,
                matches!(binding.source, InputSource::MouseAxis(_)),
                "Mouse Axis",
            ) {
                binding.source = InputSource::MouseAxis(MouseAxisType::MoveX);
            }
            if combo_item(
                ui,
                matches!(binding.source, InputSource::GamepadButton(_)),
                "GP Button",
            ) {
                binding.source = InputSource::GamepadButton(GamepadButton::South);
            }
            if combo_item(
                ui,
                matches!(binding.source, InputSource::GamepadAxis(_)),
                "GP Axis",
            ) {
                binding.source = InputSource::GamepadAxis(GamepadAxisType::LeftStickX);
            }
        });

    // Value selector
    let val_id = format!("{id}_val");
    match &mut binding.source {
        InputSource::Key(key) => key_combo(ui, val_id, key),
        InputSource::MouseButton(btn) => enum_combo(ui, val_id, btn, isp::MOUSE_BUTTONS, 110.0),
        InputSource::MouseAxis(axis) => enum_combo(ui, val_id, axis, isp::MOUSE_AXES, 110.0),
        InputSource::GamepadButton(btn) => enum_combo(ui, val_id, btn, isp::GAMEPAD_BUTTONS, 110.0),
        InputSource::GamepadAxis(axis) => enum_combo(ui, val_id, axis, isp::GAMEPAD_AXES, 110.0),
    }

    // Axis contribution
    match value_type {
        InputValueType::Axis1D => {
            DragValue::new(&mut binding.axis_contribution.0)
                .range(-10.0..=10.0)
                .speed(0.1)
                .prefix("val: ")
                .show(ui);
        }
        InputValueType::Axis2D | InputValueType::Axis3D => {
            DragValue::new(&mut binding.axis_contribution.0)
                .range(-10.0..=10.0)
                .speed(0.1)
                .prefix("x: ")
                .show(ui);
            DragValue::new(&mut binding.axis_contribution.1)
                .range(-10.0..=10.0)
                .speed(0.1)
                .prefix("y: ")
                .show(ui);
        }
        InputValueType::Digital => {}
    }
}

// ── Modifier list editor ────────────────────────────────────────────────────

fn modifier_list(ui: &mut Ui, modifiers: &mut Vec<InputModifier>, id: &str) {
    let style = ui.style();
    let mut remove_idx = None;
    for (i, modifier) in modifiers.iter_mut().enumerate() {
        let mod_id = format!("{id}_{i}");
        ui.horizontal(|ui| {
            row_label(ui, isp::modifier_label(modifier), style.fonts.small, style.palette.text);
            modifier_params(ui, modifier, &mod_id);
            if small_red_button(ui, "X") {
                remove_idx = Some(i);
            }
        });
    }
    if let Some(idx) = remove_idx {
        modifiers.remove(idx);
    }

    ComboBox::new(format!("{id}_add"))
        .selected_text("+ Add Modifier")
        .width(120.0)
        .show_ui(ui, |ui| {
            if combo_item(ui, false, "Negate") {
                modifiers.push(InputModifier::Negate {
                    x: true,
                    y: false,
                    z: false,
                });
            }
            if combo_item(ui, false, "Swizzle") {
                modifiers.push(InputModifier::Swizzle {
                    order: SwizzleOrder::YXZ,
                });
            }
            if combo_item(ui, false, "DeadZone") {
                modifiers.push(InputModifier::DeadZone {
                    lower: 0.15,
                    upper: 0.95,
                    kind: DeadZoneKind::Radial,
                });
            }
            if combo_item(ui, false, "Scale") {
                modifiers.push(InputModifier::Scale {
                    factor: glam::Vec3::ONE,
                });
            }
            if combo_item(ui, false, "Smooth") {
                modifiers.push(InputModifier::Smooth {
                    speed: 10.0,
                    previous: None,
                });
            }
            if combo_item(ui, false, "ResponseCurve") {
                modifiers.push(InputModifier::ResponseCurve {
                    curve: CurveType::Linear,
                });
            }
            if combo_item(ui, false, "Clamp") {
                modifiers.push(InputModifier::Clamp {
                    min: glam::Vec3::splat(-1.0),
                    max: glam::Vec3::splat(1.0),
                });
            }
        });
}

fn modifier_params(ui: &mut Ui, modifier: &mut InputModifier, id: &str) {
    match modifier {
        InputModifier::Negate { x, y, z } => {
            Checkbox::new(x, "X").show(ui);
            Checkbox::new(y, "Y").show(ui);
            Checkbox::new(z, "Z").show(ui);
        }
        InputModifier::DeadZone { lower, upper, kind } => {
            DragValue::new(lower).range(0.0..=1.0).speed(0.01).prefix("lo:").show(ui);
            DragValue::new(upper).range(0.0..=1.0).speed(0.01).prefix("hi:").show(ui);
            enum_combo(
                ui,
                format!("{id}_dzk"),
                kind,
                &[DeadZoneKind::Radial, DeadZoneKind::PerAxis],
                60.0,
            );
        }
        InputModifier::Scale { factor } => {
            DragValue::new(&mut factor.x).speed(0.1).prefix("x:").show(ui);
            DragValue::new(&mut factor.y).speed(0.1).prefix("y:").show(ui);
            DragValue::new(&mut factor.z).speed(0.1).prefix("z:").show(ui);
        }
        InputModifier::Smooth { speed, .. } => {
            DragValue::new(speed).range(0.1..=100.0).speed(0.5).prefix("spd:").show(ui);
        }
        InputModifier::ResponseCurve { curve } => {
            ComboBox::new(format!("{id}_curve"))
                .selected_text(isp::curve_label(curve))
                .width(80.0)
                .show_ui(ui, |ui| {
                    if combo_item(ui, matches!(curve, CurveType::Linear), "Linear") {
                        *curve = CurveType::Linear;
                    }
                    if combo_item(ui, matches!(curve, CurveType::Quadratic), "Quadratic") {
                        *curve = CurveType::Quadratic;
                    }
                    if combo_item(ui, matches!(curve, CurveType::Cubic), "Cubic") {
                        *curve = CurveType::Cubic;
                    }
                });
        }
        InputModifier::Clamp { min, max } => {
            DragValue::new(&mut min.x).speed(0.1).prefix("min:").show(ui);
            DragValue::new(&mut max.x).speed(0.1).prefix("max:").show(ui);
        }
        InputModifier::Swizzle { order } => {
            enum_combo(ui, format!("{id}_sw"), order, isp::SWIZZLE_ORDERS, 60.0);
        }
    }
}

// ── Trigger list editor ─────────────────────────────────────────────────────

fn trigger_list(ui: &mut Ui, triggers: &mut Vec<InputTrigger>, id: &str) {
    let style = ui.style();
    let mut remove_idx = None;
    for (i, trigger) in triggers.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            row_label(ui, isp::trigger_label(trigger), style.fonts.small, style.palette.text);
            trigger_params(ui, trigger);
            if small_red_button(ui, "X") {
                remove_idx = Some(i);
            }
        });
    }
    if let Some(idx) = remove_idx {
        triggers.remove(idx);
    }

    ComboBox::new(format!("{id}_add"))
        .selected_text("+ Add Trigger")
        .width(110.0)
        .show_ui(ui, |ui| {
            if combo_item(ui, false, "Down") {
                triggers.push(InputTrigger::Down);
            }
            if combo_item(ui, false, "Pressed") {
                triggers.push(InputTrigger::Pressed);
            }
            if combo_item(ui, false, "Released") {
                triggers.push(InputTrigger::Released);
            }
            if combo_item(ui, false, "Held") {
                triggers.push(InputTrigger::Held {
                    duration: 0.5,
                    elapsed: 0.0,
                    fired: false,
                });
            }
            if combo_item(ui, false, "Tap") {
                triggers.push(InputTrigger::Tap {
                    max_duration: 0.3,
                    elapsed: 0.0,
                    was_active: false,
                });
            }
            if combo_item(ui, false, "Pulse") {
                triggers.push(InputTrigger::Pulse {
                    interval: 0.5,
                    trigger_limit: 0,
                    elapsed: 0.0,
                    pulse_count: 0,
                });
            }
            if combo_item(ui, false, "ChordAction") {
                triggers.push(InputTrigger::ChordAction {
                    action_name: String::new(),
                });
            }
        });
}

fn trigger_params(ui: &mut Ui, trigger: &mut InputTrigger) {
    match trigger {
        InputTrigger::Held { duration, .. } => {
            DragValue::new(duration)
                .range(0.01..=10.0)
                .speed(0.05)
                .prefix("dur: ")
                .suffix("s")
                .show(ui);
        }
        InputTrigger::Tap { max_duration, .. } => {
            DragValue::new(max_duration)
                .range(0.01..=5.0)
                .speed(0.05)
                .prefix("max: ")
                .suffix("s")
                .show(ui);
        }
        InputTrigger::Pulse {
            interval,
            trigger_limit,
            ..
        } => {
            DragValue::new(interval)
                .range(0.01..=10.0)
                .speed(0.05)
                .prefix("int: ")
                .suffix("s")
                .show(ui);
            drag_u32(ui, trigger_limit, "lim: ");
        }
        InputTrigger::ChordAction { action_name } => {
            TextEdit::new(action_name).width(120.0).show(ui);
        }
        _ => {}
    }
}

// ── Shared toolbar / status bar for the per-asset editors ──────────────────

/// Save button + unsaved marker + right-aligned fading status. Returns
/// whether Save was clicked.
fn editor_toolbar(ui: &mut Ui, dirty: bool, status: &mut Option<(String, f64)>) -> bool {
    let style = ui.style();
    let now = ui.ctx().time();
    let row_top = ui.cursor();
    let right = ui.available().max.x;

    let save_clicked = ui.horizontal(|ui| {
        let btn = if dirty {
            Button::new("Save").text_color(Color::rgba(1.0, 1.0, 1.0, 1.0))
        } else {
            Button::new("Save")
        };
        let clicked = btn.show(ui).clicked;
        if dirty {
            row_label(ui, "\u{2022} Unsaved changes", style.fonts.small, rgb(255, 200, 80));
        }
        clicked
    });

    let mut clear = false;
    if let Some((msg, time)) = &*status {
        if now - time < 3.0 {
            let msg = msg.clone();
            let w = ui.text_mut().measure(&msg, style.fonts.small, None).x;
            let row_h = style.fonts.body * 1.25 + style.spacing.button_padding.y * 2.0;
            let y = row_top.y + (row_h - style.fonts.small * 1.25) * 0.5;
            ui.painter().text(
                Pos2::new(right - w - 6.0, y),
                &msg,
                style.fonts.small,
                style.palette.text_dim,
                None,
            );
        } else {
            clear = true;
        }
    }
    if clear {
        *status = None;
    }
    save_clicked
}

fn file_path_bar(ui: &mut Ui, path: &std::path::Path) {
    let style = ui.style();
    ui.separator();
    Label::new(path.to_string_lossy().as_ref())
        .size(style.fonts.small)
        .color(style.palette.text_dim)
        .show(ui);
}

// ── Input Action editor tab ─────────────────────────────────────────────────

/// Draw one input action editor into the dock tab's content rect. `key`
/// namespaces widget ids so several open action tabs don't collide.
pub fn input_action_panel(
    ui: &mut Ui,
    tab_rect: egui::Rect,
    ppp: f32,
    key: &str,
    state: &mut InputActionEditorState,
) {
    let rect = super::dock_crusty::rect_px(tab_rect, ppp);
    let style = ui.style();
    let opts = UiOptions {
        padding: Vec2::new(6.0, 4.0),
        spacing: style.spacing.item,
    };
    ui.run_at(
        rect,
        Direction::TopDown,
        Id::new(("engine_input_action", key)),
        opts,
        |ui| {
            let save = editor_toolbar(ui, state.dirty, &mut state.status_message);
            if save {
                let now = ui.ctx().time();
                match InputActionEditor::save_state(state) {
                    Ok(()) => state.status_message = Some(("Saved".to_string(), now)),
                    Err(e) => {
                        state.status_message = Some((format!("Save failed: {e}"), now));
                    }
                }
            }
            ui.separator();

            let bottom_h = style.fonts.small * 1.25 + 1.0 + style.spacing.item * 2.0;
            let h = (ui.available_size().y - bottom_h - style.spacing.item).max(50.0);
            let action = &mut state.definition;
            let mut dirty = state.dirty;
            ScrollArea::new(h).auto_shrink(false).show(ui, |ui| {
                ui.add_space(4.0);

                group(ui, "ia_props", |ui| {
                    Label::new("Properties").color(rgb(140, 180, 220)).show(ui);
                    ui.add_space(2.0);
                    Grid::new("ia_props_grid").spacing(Vec2::new(8.0, 4.0)).show(ui, |g| {
                        g.cell(|ui| row_label(ui, "Name:", style.fonts.body, style.palette.text));
                        g.cell(|ui| {
                            let before = action.name.clone();
                            TextEdit::new(&mut action.name).width(280.0).show(ui);
                            if action.name != before {
                                dirty = true;
                            }
                        });
                        g.end_row();

                        g.cell(|ui| {
                            row_label(ui, "Value Type:", style.fonts.body, style.palette.text)
                        });
                        g.cell(|ui| {
                            let before = action.value_type;
                            value_type_combo(ui, "ia_vt".to_string(), &mut action.value_type, 120.0);
                            if action.value_type != before {
                                dirty = true;
                            }
                        });
                        g.end_row();

                        g.cell(|ui| {
                            row_label(ui, "Consumes Input:", style.fonts.body, style.palette.text)
                        });
                        g.cell(|ui| {
                            let r = Checkbox::new(&mut action.consumes_input, "").show(ui);
                            if r.hovered {
                                show_tooltip_for(
                                    ui,
                                    r.rect,
                                    "When enabled, this action consumes its input sources \
                                     and prevents lower-priority contexts from seeing them.",
                                );
                            }
                            if r.clicked {
                                dirty = true;
                            }
                        });
                        g.end_row();
                    });
                });

                ui.add_space(6.0);

                group(ui, "ia_triggers", |ui| {
                    ui.horizontal(|ui| {
                        row_label(ui, "Triggers", style.fonts.body, rgb(180, 200, 140));
                        row_label(
                            ui,
                            &format!("({})", action.triggers.len()),
                            style.fonts.small,
                            style.palette.text_dim,
                        );
                    });
                    ui.add_space(2.0);
                    let before = action.triggers.len();
                    trigger_list(ui, &mut action.triggers, "ia_trig");
                    if action.triggers.len() != before {
                        dirty = true;
                    }
                });

                ui.add_space(6.0);

                group(ui, "ia_modifiers", |ui| {
                    ui.horizontal(|ui| {
                        row_label(ui, "Modifiers", style.fonts.body, rgb(200, 160, 180));
                        row_label(
                            ui,
                            &format!("({})", action.modifiers.len()),
                            style.fonts.small,
                            style.palette.text_dim,
                        );
                    });
                    ui.add_space(2.0);
                    let before = action.modifiers.len();
                    modifier_list(ui, &mut action.modifiers, "ia_mod");
                    if action.modifiers.len() != before {
                        dirty = true;
                    }
                });
            });
            state.dirty = dirty;

            file_path_bar(ui, &state.file_path);
        },
    );
}

// ── Mapping Context editor tab ──────────────────────────────────────────────

/// Draw one mapping context editor into the dock tab's content rect.
pub fn input_context_panel(
    ui: &mut Ui,
    tab_rect: egui::Rect,
    ppp: f32,
    key: &str,
    state: &mut InputContextEditorState,
    available_actions: &[String],
) {
    let rect = super::dock_crusty::rect_px(tab_rect, ppp);
    let style = ui.style();
    let opts = UiOptions {
        padding: Vec2::new(6.0, 4.0),
        spacing: style.spacing.item,
    };
    ui.run_at(
        rect,
        Direction::TopDown,
        Id::new(("engine_input_context", key)),
        opts,
        |ui| {
            // ── Input detection (before any widgets consume events) ──
            if let Some((entry_idx, bind_idx)) = state.listening_binding {
                let source = state
                    .pending_external_input
                    .take()
                    .or_else(|| detect_input(ui))
                    .or_else(|| detect_modifier_press(ui, &state.listen_start_modifiers));
                if let Some(source) = source {
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

            let save = editor_toolbar(ui, state.dirty, &mut state.status_message);
            if save {
                let now = ui.ctx().time();
                match InputContextEditor::save_state(state) {
                    Ok(()) => state.status_message = Some(("Saved".to_string(), now)),
                    Err(e) => {
                        state.status_message = Some((format!("Save failed: {e}"), now));
                    }
                }
            }
            ui.separator();

            let bottom_h = style.fonts.small * 1.25 + 1.0 + style.spacing.item * 2.0;
            let h = (ui.available_size().y - bottom_h - style.spacing.item).max(50.0);
            let listening = state.listening_binding;
            let mut dirty = state.dirty;
            let mut start_listen: Option<(usize, usize)> = None;
            let mctx = &mut state.context;

            ScrollArea::new(h).auto_shrink(false).show(ui, |ui| {
                ui.add_space(4.0);

                group(ui, "mc_props", |ui| {
                    Label::new("Context Properties").color(rgb(140, 180, 220)).show(ui);
                    ui.add_space(2.0);
                    Grid::new("mc_props_grid").spacing(Vec2::new(8.0, 4.0)).show(ui, |g| {
                        g.cell(|ui| row_label(ui, "Name:", style.fonts.body, style.palette.text));
                        g.cell(|ui| {
                            let before = mctx.name.clone();
                            TextEdit::new(&mut mctx.name).width(280.0).show(ui);
                            if mctx.name != before {
                                dirty = true;
                            }
                        });
                        g.end_row();

                        g.cell(|ui| {
                            row_label(ui, "Priority:", style.fonts.body, style.palette.text)
                        });
                        g.cell(|ui| {
                            if drag_i32(
                                ui,
                                &mut mctx.priority,
                                -1000,
                                1000,
                                Some(
                                    "Higher priority contexts are processed first. Actions \
                                     that consume input will block lower-priority contexts.",
                                ),
                            ) {
                                dirty = true;
                            }
                        });
                        g.end_row();
                    });
                });

                ui.add_space(6.0);

                group(ui, "mc_entries", |ui| {
                    ui.horizontal(|ui| {
                        row_label(ui, "Action Mappings", style.fonts.body, rgb(180, 200, 140));
                        row_label(
                            ui,
                            &format!("({})", mctx.entries.len()),
                            style.fonts.small,
                            style.palette.text_dim,
                        );
                    });
                    ui.add_space(2.0);

                    let mut remove_entry = None;
                    for entry_idx in 0..mctx.entries.len() {
                        let entry = &mut mctx.entries[entry_idx];
                        let entry_id = format!("mc_entry_{entry_idx}");
                        let header = format!(
                            "{} \u{2014} {} binding{}",
                            entry.action_name,
                            entry.bindings.len(),
                            if entry.bindings.len() == 1 { "" } else { "s" }
                        );

                        CollapsingHeader::new(header).default_open(true).show(ui, |ui| {
                            ui.horizontal(|ui| {
                                row_label(ui, "Action:", style.fonts.body, style.palette.text);
                                if string_combo(
                                    ui,
                                    format!("{entry_id}_an"),
                                    &mut entry.action_name,
                                    available_actions,
                                    180.0,
                                ) {
                                    dirty = true;
                                }
                            });

                            ui.add_space(4.0);
                            Label::new("Bindings:").show(ui);

                            let mut remove_bind = None;
                            for (bi, binding) in entry.bindings.iter_mut().enumerate() {
                                let bind_id = format!("{entry_id}_b{bi}");
                                let is_listening = listening == Some((entry_idx, bi));

                                group(ui, &bind_id, |ui| {
                                    ui.horizontal(|ui| {
                                        row_label(
                                            ui,
                                            &format!("#{}", bi + 1),
                                            style.fonts.small,
                                            style.palette.text_dim,
                                        );

                                        if is_listening {
                                            Button::new("...")
                                                .text_color(rgb(100, 200, 255))
                                                .show(ui);
                                        } else {
                                            let r =
                                                Button::new("\u{1F3A7}").size(14.0).show(ui);
                                            if r.hovered {
                                                show_tooltip_for(
                                                    ui,
                                                    r.rect,
                                                    "Click to listen for key/mouse input",
                                                );
                                            }
                                            if r.clicked {
                                                start_listen = Some((entry_idx, bi));
                                            }
                                        }

                                        binding_editor(
                                            ui,
                                            binding,
                                            &bind_id,
                                            InputValueType::Digital,
                                        );

                                        let small = style.fonts.small;
                                        let r = Button::new("\u{2716}")
                                            .size(small)
                                            .text_color(red())
                                            .show(ui);
                                        if r.hovered {
                                            show_tooltip_for(ui, r.rect, "Remove binding");
                                        }
                                        if r.clicked {
                                            remove_bind = Some(bi);
                                            dirty = true;
                                        }
                                    });

                                    let details = format!(
                                        "Modifiers ({}) / Triggers ({})",
                                        binding.modifiers.len(),
                                        binding.triggers.len()
                                    );
                                    CollapsingHeader::new(details)
                                        .default_open(
                                            !binding.modifiers.is_empty()
                                                || !binding.triggers.is_empty(),
                                        )
                                        .text_size(style.fonts.small)
                                        .text_color(style.palette.text_dim)
                                        .fit_width(true)
                                        .show(ui, |ui| {
                                            Label::new("Modifiers:")
                                                .size(style.fonts.small)
                                                .show(ui);
                                            let before = binding.modifiers.len();
                                            modifier_list(
                                                ui,
                                                &mut binding.modifiers,
                                                &format!("{bind_id}_mod"),
                                            );
                                            if binding.modifiers.len() != before {
                                                dirty = true;
                                            }

                                            ui.add_space(2.0);

                                            Label::new("Triggers:")
                                                .size(style.fonts.small)
                                                .show(ui);
                                            let before = binding.triggers.len();
                                            trigger_list(
                                                ui,
                                                &mut binding.triggers,
                                                &format!("{bind_id}_trig"),
                                            );
                                            if binding.triggers.len() != before {
                                                dirty = true;
                                            }
                                        });
                                });
                            }
                            if let Some(idx) = remove_bind {
                                entry.bindings.remove(idx);
                            }

                            ui.horizontal(|ui| {
                                if Button::new("+ Add Binding").show(ui).clicked {
                                    entry.bindings.push(EnhancedBinding::digital(
                                        InputSource::Key(KeyCode::Space),
                                    ));
                                    dirty = true;
                                }
                                // Right-aligned Remove Entry
                                let w = ui.text_mut().measure("Remove Entry", style.fonts.body, None).x
                                    + style.spacing.button_padding.x * 2.0;
                                ui.set_cursor(Pos2::new(
                                    ui.available().max.x - w,
                                    ui.cursor().y,
                                ));
                                if Button::new("Remove Entry")
                                    .text_color(red())
                                    .show(ui)
                                    .clicked
                                {
                                    remove_entry = Some(entry_idx);
                                    dirty = true;
                                }
                            });
                        });
                    }
                    if let Some(idx) = remove_entry {
                        mctx.entries.remove(idx);
                    }

                    ui.add_space(4.0);
                    if Button::new("+ Add Entry").show(ui).clicked {
                        let action_name = available_actions
                            .first()
                            .cloned()
                            .unwrap_or_else(|| "unnamed".to_string());
                        mctx.entries.push(MappingContextEntry::new(action_name));
                        dirty = true;
                    }
                });
            });
            state.dirty = dirty;
            if let Some(lb) = start_listen {
                state.listening_binding = Some(lb);
                state.listen_start_modifiers =
                    crusty_mods_to_egui(ui.ctx().input.modifiers);
            }

            file_path_bar(ui, &state.file_path);

            // ── Listening overlay (drawn last so it paints on top) ──
            if state.listening_binding.is_some() {
                listening_overlay(ui, rect, &mut state.listening_binding);
            }
        },
    );
}

/// Centered "Listening for input..." modal box over the panel.
fn listening_overlay(ui: &mut Ui, panel_rect: Rect, listening: &mut Option<(usize, usize)>) {
    let style = ui.style();
    let title = "Listening for input...";
    let subtitle = "Press any key, mouse button, or gamepad input";
    let title_size = 18.0;
    let title_w = ui.text_mut().measure(title, title_size, None).x;
    let sub_w = ui.text_mut().measure(subtitle, style.fonts.body, None).x;
    let btn_h = style.fonts.body * 1.25 + style.spacing.button_padding.y * 2.0;
    let margin = 24.0;
    let content_w = title_w.max(sub_w);
    let content_h =
        4.0 + title_size * 1.25 + 8.0 + style.fonts.body * 1.25 + 8.0 + btn_h;
    let box_size = Vec2::new(content_w + margin * 2.0, content_h + margin * 2.0);
    let center = panel_rect.center();
    let box_rect = Rect::from_min_size(
        Pos2::new(center.x - box_size.x * 0.5, center.y - box_size.y * 0.5),
        box_size,
    );

    let cx = box_rect.center().x;
    let mut y = box_rect.min.y + margin + 4.0;
    {
        let mut p = ui.painter();
        p.rect_filled(box_rect, 8.0, rgba(20, 20, 30, 220));
        p.rect_stroke(box_rect, 8.0, 2.0, rgb(100, 160, 255));
        p.text(
            Pos2::new(cx - title_w * 0.5, y),
            title,
            title_size,
            Color::rgba(1.0, 1.0, 1.0, 1.0),
            None,
        );
        y += title_size * 1.25 + 8.0;
        p.text(
            Pos2::new(cx - sub_w * 0.5, y),
            subtitle,
            style.fonts.body,
            rgb(180, 180, 200),
            None,
        );
        y += style.fonts.body * 1.25 + 8.0;
    }

    let btn_w = ui.text_mut().measure("Cancel", style.fonts.body, None).x
        + style.spacing.button_padding.x * 2.0;
    let btn_rect = Rect::from_min_size(Pos2::new(cx - btn_w * 0.5, y), Vec2::new(btn_w, btn_h));
    let opts = UiOptions {
        padding: Vec2::ZERO,
        spacing: 0.0,
    };
    let id = ui.alloc_id("listen_cancel");
    let clicked = ui
        .run_at(btn_rect, Direction::TopDown, id, opts, |ui| {
            Button::new("Cancel").show(ui).clicked
        })
        .0;
    if clicked {
        *listening = None;
    }
}

// ── Input detection (crusty analogues of the egui helpers) ─────────────────

/// Scan this frame's crusty input for a key or mouse button press.
fn detect_input(ui: &Ui) -> Option<InputSource> {
    let input = &ui.ctx().input;
    for kp in &input.key_presses {
        if kp.repeat {
            continue;
        }
        if let Some(kc) = crusty_key_to_keycode(kp.key) {
            return Some(InputSource::Key(kc));
        }
    }
    for b in &input.button_presses {
        if let Some(mb) = crusty_button_to_mouse(*b) {
            return Some(InputSource::MouseButton(mb));
        }
    }
    None
}

/// Detect modifier-only presses (crusty maps modifier keys to `Key::Unknown`,
/// so — like egui — they only show up in the modifier flags). Compares against
/// the state captured when listening started.
fn detect_modifier_press(ui: &Ui, start: &egui::Modifiers) -> Option<InputSource> {
    let m = ui.ctx().input.modifiers;
    if m.contains(CModifiers::SHIFT) && !start.shift {
        return Some(InputSource::Key(KeyCode::ShiftLeft));
    }
    if m.contains(CModifiers::CTRL) && !start.ctrl {
        return Some(InputSource::Key(KeyCode::ControlLeft));
    }
    if m.contains(CModifiers::ALT) && !start.alt {
        return Some(InputSource::Key(KeyCode::AltLeft));
    }
    if m.contains(CModifiers::META) && !start.command {
        return Some(InputSource::Key(KeyCode::SuperLeft));
    }
    None
}

/// crusty modifiers → egui modifiers (the state struct stores the egui type
/// while both UIs coexist).
fn crusty_mods_to_egui(m: CModifiers) -> egui::Modifiers {
    egui::Modifiers {
        alt: m.contains(CModifiers::ALT),
        ctrl: m.contains(CModifiers::CTRL),
        shift: m.contains(CModifiers::SHIFT),
        mac_cmd: false,
        command: m.contains(CModifiers::META),
    }
}

fn crusty_key_to_keycode(key: CKey) -> Option<KeyCode> {
    match key {
        CKey::Char(c) => char_to_keycode(c),
        CKey::F(n @ 1..=12) => Some(isp::KEY_FUNCTION[(n - 1) as usize]),
        CKey::Escape => Some(KeyCode::Escape),
        CKey::Tab => Some(KeyCode::Tab),
        CKey::Backspace => Some(KeyCode::Backspace),
        CKey::Enter => Some(KeyCode::Enter),
        CKey::Space => Some(KeyCode::Space),
        CKey::ArrowLeft => Some(KeyCode::ArrowLeft),
        CKey::ArrowRight => Some(KeyCode::ArrowRight),
        CKey::ArrowUp => Some(KeyCode::ArrowUp),
        CKey::ArrowDown => Some(KeyCode::ArrowDown),
        CKey::Home => Some(KeyCode::Home),
        CKey::End => Some(KeyCode::End),
        CKey::PageUp => Some(KeyCode::PageUp),
        CKey::PageDown => Some(KeyCode::PageDown),
        CKey::Delete => Some(KeyCode::Delete),
        CKey::Insert => Some(KeyCode::Insert),
        _ => None,
    }
}

fn char_to_keycode(c: char) -> Option<KeyCode> {
    let c = c.to_ascii_lowercase();
    match c {
        'a'..='z' => Some(isp::KEY_LETTERS[(c as u8 - b'a') as usize]),
        '0'..='9' => Some(isp::KEY_DIGITS[(c as u8 - b'0') as usize]),
        ',' => Some(KeyCode::Comma),
        '.' => Some(KeyCode::Period),
        ';' => Some(KeyCode::Semicolon),
        '\'' => Some(KeyCode::Quote),
        '[' => Some(KeyCode::BracketLeft),
        ']' => Some(KeyCode::BracketRight),
        '\\' => Some(KeyCode::Backslash),
        '/' => Some(KeyCode::Slash),
        '-' => Some(KeyCode::Minus),
        '=' | '+' => Some(KeyCode::Equal),
        '`' => Some(KeyCode::Backquote),
        _ => None,
    }
}

fn crusty_button_to_mouse(b: CMouseButton) -> Option<MouseButton> {
    match b {
        CMouseButton::Primary => Some(MouseButton::Left),
        CMouseButton::Secondary => Some(MouseButton::Right),
        CMouseButton::Middle => Some(MouseButton::Middle),
        CMouseButton::Back => Some(MouseButton::Back),
        CMouseButton::Forward => Some(MouseButton::Forward),
        CMouseButton::Other(_) => None,
    }
}
