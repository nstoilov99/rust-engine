//! Input reader abstraction for the action system.

use super::action::{GamepadAxisType, GamepadButton, KeyCode, MouseButton};

/// Trait for reading raw input state from keyboard, mouse, and gamepad.
pub trait InputReader {
    fn is_key_pressed(&self, key: KeyCode) -> bool;
    fn is_key_just_pressed(&self, key: KeyCode) -> bool;
    fn is_mouse_pressed(&self, button: MouseButton) -> bool;
    fn mouse_delta(&self) -> (f32, f32);
    fn scroll_delta(&self) -> f32;
    fn is_gamepad_pressed(&self, button: GamepadButton) -> bool;
    fn gamepad_axis(&self, axis: GamepadAxisType) -> f32;
}

/// Reads keyboard and mouse from `InputManager`. Gamepad always returns zero.
pub struct KeyboardMouseReader<'a> {
    pub input: &'a super::InputManager,
}

impl InputReader for KeyboardMouseReader<'_> {
    fn is_key_pressed(&self, key: KeyCode) -> bool {
        self.input.is_key_pressed(key)
    }
    fn is_key_just_pressed(&self, key: KeyCode) -> bool {
        self.input.is_key_just_pressed(key)
    }
    fn is_mouse_pressed(&self, button: MouseButton) -> bool {
        self.input.is_mouse_button_pressed(button)
    }
    fn mouse_delta(&self) -> (f32, f32) {
        self.input.mouse_delta()
    }
    fn scroll_delta(&self) -> f32 {
        self.input.scroll_delta()
    }
    fn is_gamepad_pressed(&self, _button: GamepadButton) -> bool {
        false
    }
    fn gamepad_axis(&self, _axis: GamepadAxisType) -> f32 {
        0.0
    }
}

/// Full input reader wrapping keyboard/mouse and gamepad.
pub struct FullInputReader<'a> {
    pub input: &'a super::InputManager,
    pub gamepad: Option<&'a super::gamepad::GamepadState>,
}

impl InputReader for FullInputReader<'_> {
    fn is_key_pressed(&self, key: KeyCode) -> bool {
        self.input.is_key_pressed(key)
    }
    fn is_key_just_pressed(&self, key: KeyCode) -> bool {
        self.input.is_key_just_pressed(key)
    }
    fn is_mouse_pressed(&self, button: MouseButton) -> bool {
        self.input.is_mouse_button_pressed(button)
    }
    fn mouse_delta(&self) -> (f32, f32) {
        self.input.mouse_delta()
    }
    fn scroll_delta(&self) -> f32 {
        self.input.scroll_delta()
    }
    fn is_gamepad_pressed(&self, button: GamepadButton) -> bool {
        self.gamepad.is_some_and(|gp| gp.is_pressed(button))
    }
    fn gamepad_axis(&self, axis: GamepadAxisType) -> f32 {
        self.gamepad.map_or(0.0, |gp| gp.axis_value(axis))
    }
}
