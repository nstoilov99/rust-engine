//! Default action map with global and gameplay contexts.

use super::action::*;
use super::action_map::*;

/// Create the default action map with standard bindings.
pub fn default_action_map() -> ActionMap {
    let mut map = ActionMap::new();

    let global = InputContext::new("global")
        .with_action(
            ActionDefinition::new("pause", ActionType::Digital)
                .with_binding(ActionBinding::digital(InputSource::Key(KeyCode::Escape)))
                .with_binding(ActionBinding::digital(InputSource::GamepadButton(GamepadButton::Start))),
        )
        .with_action(
            ActionDefinition::new("screenshot", ActionType::Digital)
                .with_binding(ActionBinding::digital(InputSource::Key(KeyCode::F2))),
        )
        .with_action(
            ActionDefinition::new("debug_overlay", ActionType::Digital)
                .with_binding(ActionBinding::digital(InputSource::Key(KeyCode::F3))),
        );
    map.add_context(global);

    let gameplay = InputContext::new("gameplay")
        .with_action(
            ActionDefinition::new("move", ActionType::Axis2D)
                .with_binding(ActionBinding::axis_2d(InputSource::Key(KeyCode::KeyW), 0.0, 1.0))
                .with_binding(ActionBinding::axis_2d(InputSource::Key(KeyCode::KeyS), 0.0, -1.0))
                .with_binding(ActionBinding::axis_2d(InputSource::Key(KeyCode::KeyA), -1.0, 0.0))
                .with_binding(ActionBinding::axis_2d(InputSource::Key(KeyCode::KeyD), 1.0, 0.0))
                .with_gamepad_stick(GamepadStick2D {
                    axis_x: GamepadAxisType::LeftStickX,
                    axis_y: GamepadAxisType::LeftStickY,
                }),
        )
        .with_action(
            ActionDefinition::new("look", ActionType::Axis2D)
                .with_binding(ActionBinding::axis_2d(InputSource::MouseAxis(MouseAxisType::MoveX), 1.0, 0.0))
                .with_binding(ActionBinding::axis_2d(InputSource::MouseAxis(MouseAxisType::MoveY), 0.0, 1.0))
                .with_gamepad_stick(GamepadStick2D {
                    axis_x: GamepadAxisType::RightStickX,
                    axis_y: GamepadAxisType::RightStickY,
                }),
        )
        .with_action(
            ActionDefinition::new("jump", ActionType::Digital)
                .with_binding(ActionBinding::digital(InputSource::Key(KeyCode::Space)))
                .with_binding(ActionBinding::digital(InputSource::GamepadButton(GamepadButton::South))),
        )
        .with_action(
            ActionDefinition::new("sprint", ActionType::Digital)
                .with_binding(ActionBinding::digital(InputSource::Key(KeyCode::ShiftLeft)))
                .with_binding(ActionBinding::digital(InputSource::GamepadButton(GamepadButton::LeftStick))),
        )
        .with_action(
            ActionDefinition::new("interact", ActionType::Digital)
                .with_binding(ActionBinding::digital(InputSource::Key(KeyCode::KeyE)))
                .with_binding(ActionBinding::digital(InputSource::GamepadButton(GamepadButton::West))),
        );
    map.add_context(gameplay);

    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_map_has_global_and_gameplay() {
        let map = default_action_map();
        assert!(map.context("global").is_some());
        assert!(map.context("gameplay").is_some());
    }

    #[test]
    fn gameplay_has_move_action_with_wasd() {
        let map = default_action_map();
        let gameplay = map.context("gameplay").expect("gameplay context");
        let move_action = gameplay.actions.iter().find(|a| a.name == "move").expect("move action");
        assert_eq!(move_action.action_type, ActionType::Axis2D);
        assert_eq!(move_action.bindings.len(), 4);
        assert!(move_action.gamepad_stick.is_some());
    }
}
