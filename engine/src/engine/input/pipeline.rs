//! Input processing pipeline: raw collection, modifier chains, value promotion.

use glam::Vec2;

use super::action::{InputSource, MouseAxisType};
use super::input_reader::InputReader;
use super::modifier::InputModifier;
use super::trigger::{InputTrigger, TriggerState};
use super::value::{InputValue, InputValueType};

/// Collect the raw value from a single input source.
pub fn collect_raw_value(source: &InputSource, reader: &dyn InputReader) -> InputValue {
    match source {
        InputSource::Key(k) => InputValue::Digital(reader.is_key_pressed(*k)),
        InputSource::MouseButton(b) => InputValue::Digital(reader.is_mouse_pressed(*b)),
        InputSource::MouseAxis(axis) => {
            let raw = match axis {
                MouseAxisType::MoveX => reader.mouse_delta().0,
                MouseAxisType::MoveY => reader.mouse_delta().1,
                MouseAxisType::ScrollY => reader.scroll_delta(),
            };
            InputValue::Axis1D(raw)
        }
        InputSource::GamepadButton(b) => InputValue::Digital(reader.is_gamepad_pressed(*b)),
        InputSource::GamepadAxis(axis) => InputValue::Axis1D(reader.gamepad_axis(*axis)),
    }
}

/// Apply a chain of modifiers to a value.
pub fn apply_modifiers(value: InputValue, modifiers: &mut [InputModifier], dt: f32) -> InputValue {
    let mut v = value;
    for m in modifiers.iter_mut() {
        v = m.apply(v, dt);
    }
    v
}

/// Promote a value to the target type, using axis_contribution for digital-to-axis promotion.
pub fn promote_value(
    raw: InputValue,
    target: InputValueType,
    axis_contribution: (f32, f32),
) -> InputValue {
    match (raw, target) {
        // Same type — pass through
        (InputValue::Digital(_), InputValueType::Digital) => raw,
        (InputValue::Axis1D(_), InputValueType::Axis1D) => raw,
        (InputValue::Axis2D(_), InputValueType::Axis2D) => raw,
        (InputValue::Axis3D(_), InputValueType::Axis3D) => raw,

        // Digital → Axis1D: use axis_contribution.0 when active
        (InputValue::Digital(active), InputValueType::Axis1D) => {
            InputValue::Axis1D(if active { axis_contribution.0 } else { 0.0 })
        }
        // Digital → Axis2D: use (axis_contribution.0, axis_contribution.1) when active
        (InputValue::Digital(active), InputValueType::Axis2D) => {
            if active {
                InputValue::Axis2D(Vec2::new(axis_contribution.0, axis_contribution.1))
            } else {
                InputValue::Axis2D(Vec2::ZERO)
            }
        }
        // Axis1D → Axis2D: put value in X
        (InputValue::Axis1D(v), InputValueType::Axis2D) => {
            InputValue::Axis2D(Vec2::new(v * axis_contribution.0, v * axis_contribution.1))
        }
        // All other promotions: use the as_* converters
        (v, InputValueType::Digital) => InputValue::Digital(v.as_bool()),
        (v, InputValueType::Axis1D) => InputValue::Axis1D(v.as_f32()),
        (v, InputValueType::Axis2D) => InputValue::Axis2D(v.as_vec2()),
        (v, InputValueType::Axis3D) => InputValue::Axis3D(v.as_vec3()),
    }
}

/// Evaluate a set of triggers against a value.
///
/// Returns the combined trigger state. If multiple triggers are present,
/// all must be at least `Ongoing` for the combined result to be `Triggered`.
/// (AND logic for multiple triggers, matching Unreal's behavior.)
pub fn evaluate_triggers(
    triggers: &mut [InputTrigger],
    value: &InputValue,
    was_active: bool,
    dt: f32,
    chord_lookup: &dyn Fn(&str) -> bool,
) -> TriggerState {
    if triggers.is_empty() {
        // Implicit Down trigger
        return if value.is_active() {
            TriggerState::Triggered
        } else {
            TriggerState::Idle
        };
    }

    if triggers.len() == 1 {
        return triggers[0].evaluate(value, was_active, dt, chord_lookup);
    }

    // Multiple triggers: AND logic
    // All must be Triggered for the combined to be Triggered.
    // If any is Ongoing, combined is Ongoing.
    // If any is Idle, combined is Idle.
    let mut any_ongoing = false;
    let mut all_triggered = true;

    for trigger in triggers.iter_mut() {
        let state = trigger.evaluate(value, was_active, dt, chord_lookup);
        match state {
            TriggerState::Idle => return TriggerState::Idle,
            TriggerState::Ongoing => {
                any_ongoing = true;
                all_triggered = false;
            }
            TriggerState::Triggered => {}
        }
    }

    if all_triggered {
        TriggerState::Triggered
    } else if any_ongoing {
        TriggerState::Ongoing
    } else {
        TriggerState::Idle
    }
}

/// Generate a stable hash key for an input source (for consumed-source tracking).
pub fn source_id(source: &InputSource) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    std::mem::discriminant(source).hash(&mut hasher);
    match source {
        InputSource::Key(k) => std::mem::discriminant(k).hash(&mut hasher),
        InputSource::MouseButton(b) => std::mem::discriminant(b).hash(&mut hasher),
        InputSource::MouseAxis(a) => std::mem::discriminant(a).hash(&mut hasher),
        InputSource::GamepadButton(b) => std::mem::discriminant(b).hash(&mut hasher),
        InputSource::GamepadAxis(a) => std::mem::discriminant(a).hash(&mut hasher),
    }
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::input::action::{GamepadAxisType, GamepadButton, KeyCode, MouseButton};
    use glam::Vec2;

    struct MockReader {
        pressed_keys: Vec<KeyCode>,
        mouse_delta: (f32, f32),
    }

    impl InputReader for MockReader {
        fn is_key_pressed(&self, key: KeyCode) -> bool {
            self.pressed_keys.contains(&key)
        }
        fn is_key_just_pressed(&self, _: KeyCode) -> bool { false }
        fn is_mouse_pressed(&self, _: MouseButton) -> bool { false }
        fn mouse_delta(&self) -> (f32, f32) { self.mouse_delta }
        fn scroll_delta(&self) -> f32 { 0.0 }
        fn is_gamepad_pressed(&self, _: GamepadButton) -> bool { false }
        fn gamepad_axis(&self, _: GamepadAxisType) -> f32 { 0.0 }
    }

    #[test]
    fn collect_key_press() {
        let reader = MockReader {
            pressed_keys: vec![KeyCode::Space],
            mouse_delta: (0.0, 0.0),
        };
        let v = collect_raw_value(&InputSource::Key(KeyCode::Space), &reader);
        assert_eq!(v, InputValue::Digital(true));

        let v = collect_raw_value(&InputSource::Key(KeyCode::KeyA), &reader);
        assert_eq!(v, InputValue::Digital(false));
    }

    #[test]
    fn promote_digital_to_axis2d() {
        let v = promote_value(InputValue::Digital(true), InputValueType::Axis2D, (0.0, 1.0));
        assert_eq!(v, InputValue::Axis2D(Vec2::new(0.0, 1.0)));

        let v = promote_value(InputValue::Digital(false), InputValueType::Axis2D, (0.0, 1.0));
        assert_eq!(v, InputValue::Axis2D(Vec2::ZERO));
    }

    #[test]
    fn implicit_down_trigger() {
        let mut triggers: Vec<InputTrigger> = vec![];
        let state = evaluate_triggers(&mut triggers, &InputValue::Digital(true), false, 0.016, &|_| false);
        assert_eq!(state, TriggerState::Triggered);

        let state = evaluate_triggers(&mut triggers, &InputValue::Digital(false), false, 0.016, &|_| false);
        assert_eq!(state, TriggerState::Idle);
    }
}
