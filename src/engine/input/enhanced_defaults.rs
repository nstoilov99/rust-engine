//! Default enhanced input configuration.
//!
//! Provides a sensible default `InputActionSet` with gameplay and global contexts.

use super::action::*;
use super::enhanced_action::*;
use super::modifier::{DeadZoneKind, InputModifier};
use super::trigger::InputTrigger;
use super::value::InputValueType;

/// Create the default enhanced input action set.
pub fn default_action_set() -> InputActionSet {
    let mut set = InputActionSet::new();

    // --- Action Definitions ---

    // Movement (Axis2D, fires every frame while active)
    set.add_action(InputActionDefinition::new("move", InputValueType::Axis2D));

    // Camera look (Axis2D, fires every frame)
    set.add_action(InputActionDefinition::new("look", InputValueType::Axis2D));

    // Jump (Digital, fires once on press)
    set.add_action(
        InputActionDefinition::new("jump", InputValueType::Digital)
            .with_trigger(InputTrigger::Pressed),
    );

    // Sprint (Digital, fires every frame while held)
    set.add_action(InputActionDefinition::new("sprint", InputValueType::Digital));

    // Interact (Digital, fires once on press)
    set.add_action(
        InputActionDefinition::new("interact", InputValueType::Digital)
            .with_trigger(InputTrigger::Pressed),
    );

    // Pause (Digital, fires once on press)
    set.add_action(
        InputActionDefinition::new("pause", InputValueType::Digital)
            .with_trigger(InputTrigger::Pressed)
            .with_consumes(false),
    );

    // Screenshot (Digital, fires once on press)
    set.add_action(
        InputActionDefinition::new("screenshot", InputValueType::Digital)
            .with_trigger(InputTrigger::Pressed)
            .with_consumes(false),
    );

    // Debug overlay toggle (Digital, fires once on press)
    set.add_action(
        InputActionDefinition::new("debug_overlay", InputValueType::Digital)
            .with_trigger(InputTrigger::Pressed)
            .with_consumes(false),
    );

    // --- Global Context (priority 100 — processed first) ---

    let global = MappingContext::new("global", 100)
        .with_entry(
            MappingContextEntry::new("pause")
                .with_binding(EnhancedBinding::digital(InputSource::Key(KeyCode::Escape)))
                .with_binding(EnhancedBinding::digital(InputSource::GamepadButton(
                    GamepadButton::Start,
                ))),
        )
        .with_entry(
            MappingContextEntry::new("screenshot")
                .with_binding(EnhancedBinding::digital(InputSource::Key(KeyCode::F2))),
        )
        .with_entry(
            MappingContextEntry::new("debug_overlay")
                .with_binding(EnhancedBinding::digital(InputSource::Key(KeyCode::F3))),
        );
    set.add_context(global);

    // --- Gameplay Context (priority 0) ---

    let gameplay = MappingContext::new("gameplay", 0)
        .with_entry(
            MappingContextEntry::new("move")
                // WASD keyboard bindings
                .with_binding(EnhancedBinding::axis_2d(
                    InputSource::Key(KeyCode::KeyW),
                    0.0,
                    1.0,
                ))
                .with_binding(EnhancedBinding::axis_2d(
                    InputSource::Key(KeyCode::KeyS),
                    0.0,
                    -1.0,
                ))
                .with_binding(EnhancedBinding::axis_2d(
                    InputSource::Key(KeyCode::KeyA),
                    -1.0,
                    0.0,
                ))
                .with_binding(EnhancedBinding::axis_2d(
                    InputSource::Key(KeyCode::KeyD),
                    1.0,
                    0.0,
                ))
                // Left stick
                .with_binding(
                    EnhancedBinding::axis_2d(
                        InputSource::GamepadAxis(GamepadAxisType::LeftStickX),
                        1.0,
                        0.0,
                    )
                    .with_modifier(InputModifier::DeadZone {
                        lower: 0.15,
                        upper: 0.95,
                        kind: DeadZoneKind::Radial,
                    }),
                )
                .with_binding(
                    EnhancedBinding::axis_2d(
                        InputSource::GamepadAxis(GamepadAxisType::LeftStickY),
                        0.0,
                        1.0,
                    )
                    .with_modifier(InputModifier::DeadZone {
                        lower: 0.15,
                        upper: 0.95,
                        kind: DeadZoneKind::Radial,
                    }),
                ),
        )
        .with_entry(
            MappingContextEntry::new("look")
                // Mouse
                .with_binding(EnhancedBinding::axis_2d(
                    InputSource::MouseAxis(MouseAxisType::MoveX),
                    1.0,
                    0.0,
                ))
                .with_binding(EnhancedBinding::axis_2d(
                    InputSource::MouseAxis(MouseAxisType::MoveY),
                    0.0,
                    1.0,
                ))
                // Right stick
                .with_binding(
                    EnhancedBinding::axis_2d(
                        InputSource::GamepadAxis(GamepadAxisType::RightStickX),
                        1.0,
                        0.0,
                    )
                    .with_modifier(InputModifier::DeadZone {
                        lower: 0.15,
                        upper: 0.95,
                        kind: DeadZoneKind::Radial,
                    }),
                )
                .with_binding(
                    EnhancedBinding::axis_2d(
                        InputSource::GamepadAxis(GamepadAxisType::RightStickY),
                        0.0,
                        1.0,
                    )
                    .with_modifier(InputModifier::DeadZone {
                        lower: 0.15,
                        upper: 0.95,
                        kind: DeadZoneKind::Radial,
                    }),
                ),
        )
        .with_entry(
            MappingContextEntry::new("jump")
                .with_binding(EnhancedBinding::digital(InputSource::Key(KeyCode::Space)))
                .with_binding(EnhancedBinding::digital(InputSource::GamepadButton(
                    GamepadButton::South,
                ))),
        )
        .with_entry(
            MappingContextEntry::new("sprint")
                .with_binding(EnhancedBinding::digital(InputSource::Key(
                    KeyCode::ShiftLeft,
                )))
                .with_binding(EnhancedBinding::digital(InputSource::GamepadButton(
                    GamepadButton::LeftStick,
                ))),
        )
        .with_entry(
            MappingContextEntry::new("interact")
                .with_binding(EnhancedBinding::digital(InputSource::Key(KeyCode::KeyE)))
                .with_binding(EnhancedBinding::digital(InputSource::GamepadButton(
                    GamepadButton::West,
                ))),
        );
    set.add_context(gameplay);

    set
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_set_has_contexts() {
        let set = default_action_set();
        assert!(set.context("global").is_some());
        assert!(set.context("gameplay").is_some());
    }

    #[test]
    fn default_set_has_actions() {
        let set = default_action_set();
        assert!(set.actions.contains_key("move"));
        assert!(set.actions.contains_key("look"));
        assert!(set.actions.contains_key("jump"));
        assert!(set.actions.contains_key("sprint"));
        assert!(set.actions.contains_key("interact"));
        assert!(set.actions.contains_key("pause"));
    }

    #[test]
    fn global_has_higher_priority() {
        let set = default_action_set();
        let global = set.context("global").unwrap();
        let gameplay = set.context("gameplay").unwrap();
        assert!(global.priority > gameplay.priority);
    }

    #[test]
    fn move_has_wasd_and_gamepad() {
        let set = default_action_set();
        let gameplay = set.context("gameplay").unwrap();
        let move_entry = gameplay
            .entries
            .iter()
            .find(|e| e.action_name == "move")
            .expect("move entry");
        // WASD (4) + left stick X/Y (2) = 6 bindings
        assert_eq!(move_entry.bindings.len(), 6);
    }
}
