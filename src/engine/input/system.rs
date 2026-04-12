//! Input action system: resolves raw input into action values each frame.

use super::action::{ActionType, ActionValue, InputSource};
use super::action_map::{ActionDefinition, ActionMap};
use super::action_state::ActionState;
use super::gamepad::GamepadState;
use super::input_reader::{FullInputReader, InputReader};
use super::InputManager;
use crate::engine::ecs::access::SystemDescriptor;
use crate::engine::ecs::resources::Resources;
use crate::engine::ecs::schedule::{Stage, System};

/// Resolves a single action definition against input reader state.
fn resolve_action(def: &ActionDefinition, reader: &dyn InputReader) -> ActionValue {
    match def.action_type {
        ActionType::Digital => {
            let active = def.bindings.iter().any(|b| match &b.source {
                InputSource::Key(k) => reader.is_key_pressed(*k),
                InputSource::MouseButton(mb) => reader.is_mouse_pressed(*mb),
                InputSource::GamepadButton(gb) => reader.is_gamepad_pressed(*gb),
                _ => false,
            });
            ActionValue::Digital(active)
        }
        ActionType::Axis1D => {
            let mut value = 0.0f32;
            for binding in &def.bindings {
                match &binding.source {
                    InputSource::Key(k) => {
                        if reader.is_key_pressed(*k) {
                            value += binding.axis_contribution.0;
                        }
                    }
                    InputSource::MouseButton(mb) => {
                        if reader.is_mouse_pressed(*mb) {
                            value += binding.axis_contribution.0;
                        }
                    }
                    InputSource::MouseAxis(axis) => {
                        use super::action::MouseAxisType;
                        let raw = match axis {
                            MouseAxisType::MoveX => reader.mouse_delta().0,
                            MouseAxisType::MoveY => reader.mouse_delta().1,
                            MouseAxisType::ScrollY => reader.scroll_delta(),
                        };
                        value += raw * binding.axis_contribution.0;
                    }
                    InputSource::GamepadAxis(axis) => {
                        value += reader.gamepad_axis(*axis) * binding.axis_contribution.0;
                    }
                    InputSource::GamepadButton(gb) => {
                        if reader.is_gamepad_pressed(*gb) {
                            value += binding.axis_contribution.0;
                        }
                    }
                }
            }
            ActionValue::Axis1D(value.clamp(-1.0, 1.0))
        }
        ActionType::Axis2D => {
            let mut x = 0.0f32;
            let mut y = 0.0f32;
            for binding in &def.bindings {
                match &binding.source {
                    InputSource::Key(k) => {
                        if reader.is_key_pressed(*k) {
                            x += binding.axis_contribution.0;
                            y += binding.axis_contribution.1;
                        }
                    }
                    InputSource::MouseAxis(axis) => {
                        use super::action::MouseAxisType;
                        let raw = match axis {
                            MouseAxisType::MoveX => reader.mouse_delta().0,
                            MouseAxisType::MoveY => reader.mouse_delta().1,
                            MouseAxisType::ScrollY => reader.scroll_delta(),
                        };
                        x += raw * binding.axis_contribution.0;
                        y += raw * binding.axis_contribution.1;
                    }
                    InputSource::GamepadAxis(axis) => {
                        let raw = reader.gamepad_axis(*axis);
                        x += raw * binding.axis_contribution.0;
                        y += raw * binding.axis_contribution.1;
                    }
                    InputSource::GamepadButton(gb) => {
                        if reader.is_gamepad_pressed(*gb) {
                            x += binding.axis_contribution.0;
                            y += binding.axis_contribution.1;
                        }
                    }
                    InputSource::MouseButton(mb) => {
                        if reader.is_mouse_pressed(*mb) {
                            x += binding.axis_contribution.0;
                            y += binding.axis_contribution.1;
                        }
                    }
                }
            }

            // Gamepad stick 2D binding
            if let Some(ref stick) = def.gamepad_stick {
                x += reader.gamepad_axis(stick.axis_x);
                y += reader.gamepad_axis(stick.axis_y);
            }

            // Normalize if magnitude > 1 (diagonal keyboard input)
            let mag = (x * x + y * y).sqrt();
            if mag > 1.0 {
                x /= mag;
                y /= mag;
            }

            ActionValue::Axis2D(x, y)
        }
    }
}

/// ECS system that resolves actions each frame.
///
/// Resolution order: global context first, then each context in the stack
/// from bottom to top. Stack contexts override global on name collision.
pub struct InputActionSystem;

impl System for InputActionSystem {
    fn run(&mut self, _world: &mut hecs::World, resources: &mut Resources) {
        crate::profile_scope!("input_action_system");

        let Some(input_manager) = resources.get::<InputManager>() else {
            return;
        };
        let gamepad_state = resources.get::<GamepadState>();
        let reader = FullInputReader {
            input: input_manager,
            gamepad: gamepad_state,
        };

        // Collect action definitions to resolve
        let mut actions_to_resolve: Vec<(String, ActionDefinition)> = Vec::new();

        if let Some(action_map) = resources.get::<ActionMap>() {
            // Global context first
            if let Some(global) = action_map.context("global") {
                for action in &global.actions {
                    actions_to_resolve.push((action.name.clone(), action.clone()));
                }
            }

            // Stack contexts (bottom to top — later entries override)
            if let Some(action_state) = resources.get::<ActionState>() {
                let stack: Vec<String> = action_state.context_stack().to_vec();
                for ctx_name in &stack {
                    if let Some(ctx) = action_map.context(ctx_name) {
                        for action in &ctx.actions {
                            if let Some(existing) = actions_to_resolve.iter_mut().find(|(n, _)| n == &action.name) {
                                existing.1 = action.clone();
                            } else {
                                actions_to_resolve.push((action.name.clone(), action.clone()));
                            }
                        }
                    }
                }
            }
        }

        // Resolve all actions
        let resolved: Vec<(String, ActionType, ActionValue)> = actions_to_resolve
            .iter()
            .map(|(name, def)| (name.clone(), def.action_type, resolve_action(def, &reader)))
            .collect();

        // Write results
        if let Some(action_state) = resources.get_mut::<ActionState>() {
            action_state.begin_frame();
            for (name, action_type, value) in resolved {
                action_state.set_value(&name, action_type, value);
            }
        }
    }

    fn name(&self) -> &str {
        "InputActionSystem"
    }
}

impl InputActionSystem {
    pub fn descriptor() -> SystemDescriptor {
        SystemDescriptor::new("InputActionSystem")
            .reads_resource::<InputManager>()
            .reads_resource::<ActionMap>()
            .reads_resource::<GamepadState>()
            .writes_resource::<ActionState>()
    }

    pub fn stage() -> Stage {
        Stage::First
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::input::action::{ActionBinding, GamepadAxisType, GamepadButton, KeyCode as EKeyCode, MouseButton};

    struct TestReader {
        keys: Vec<EKeyCode>,
    }

    impl InputReader for TestReader {
        fn is_key_pressed(&self, key: EKeyCode) -> bool { self.keys.contains(&key) }
        fn is_key_just_pressed(&self, _: EKeyCode) -> bool { false }
        fn is_mouse_pressed(&self, _: MouseButton) -> bool { false }
        fn mouse_delta(&self) -> (f32, f32) { (0.0, 0.0) }
        fn scroll_delta(&self) -> f32 { 0.0 }
        fn is_gamepad_pressed(&self, _: GamepadButton) -> bool { false }
        fn gamepad_axis(&self, _: GamepadAxisType) -> f32 { 0.0 }
    }

    #[test]
    fn resolve_digital_action() {
        let def = ActionDefinition::new("jump", ActionType::Digital)
            .with_binding(ActionBinding::digital(InputSource::Key(EKeyCode::Space)));
        let reader = TestReader { keys: vec![] };
        assert_eq!(resolve_action(&def, &reader), ActionValue::Digital(false));

        let reader = TestReader { keys: vec![EKeyCode::Space] };
        assert_eq!(resolve_action(&def, &reader), ActionValue::Digital(true));
    }

    #[test]
    fn resolve_axis2d_normalizes_diagonal() {
        let def = ActionDefinition::new("move", ActionType::Axis2D)
            .with_binding(ActionBinding::axis_2d(InputSource::Key(EKeyCode::KeyW), 0.0, 1.0))
            .with_binding(ActionBinding::axis_2d(InputSource::Key(EKeyCode::KeyD), 1.0, 0.0));

        let reader = TestReader { keys: vec![EKeyCode::KeyW, EKeyCode::KeyD] };
        if let ActionValue::Axis2D(x, y) = resolve_action(&def, &reader) {
            let mag = (x * x + y * y).sqrt();
            assert!((mag - 1.0).abs() < 0.01, "diagonal should be normalized, got mag={mag}");
        } else {
            panic!("expected Axis2D");
        }
    }
}
