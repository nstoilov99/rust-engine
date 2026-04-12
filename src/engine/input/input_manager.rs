//! Tracks input state (keyboard and mouse).
//!
//! Uses engine-owned `KeyCode` and `MouseButton` internally.
//! Conversion from winit types happens at the event boundary.

use std::collections::HashSet;

use super::action;
use super::key_convert;

/// Tracks input state (keyboard and mouse).
pub struct InputManager {
    keys_pressed: HashSet<action::KeyCode>,
    keys_just_pressed: HashSet<action::KeyCode>,
    keys_just_released: HashSet<action::KeyCode>,

    mouse_buttons_pressed: HashSet<action::MouseButton>,
    mouse_position: (f32, f32),
    mouse_delta: (f32, f32),
    scroll_delta: f32,

    raw_mouse_delta: (f32, f32),
    use_raw_mouse: bool,
}

impl Default for InputManager {
    fn default() -> Self {
        Self::new()
    }
}

impl InputManager {
    pub fn new() -> Self {
        Self {
            keys_pressed: HashSet::new(),
            keys_just_pressed: HashSet::new(),
            keys_just_released: HashSet::new(),
            mouse_buttons_pressed: HashSet::new(),
            mouse_position: (0.0, 0.0),
            mouse_delta: (0.0, 0.0),
            scroll_delta: 0.0,
            raw_mouse_delta: (0.0, 0.0),
            use_raw_mouse: false,
        }
    }

    pub fn new_frame(&mut self) {
        crate::profile_scope!("input_processing");
        self.keys_just_pressed.clear();
        self.keys_just_released.clear();
        self.mouse_delta = (0.0, 0.0);
        self.raw_mouse_delta = (0.0, 0.0);
        self.scroll_delta = 0.0;
    }

    /// Handle keyboard input from winit. Converts to engine KeyCode at the boundary.
    pub fn handle_keyboard(
        &mut self,
        keycode: Option<winit::keyboard::KeyCode>,
        state: winit::event::ElementState,
    ) {
        let Some(winit_key) = keycode else { return };
        let Some(engine_key) = key_convert::from_winit_keycode(winit_key) else { return };

        match state {
            winit::event::ElementState::Pressed => {
                if !self.keys_pressed.contains(&engine_key) {
                    self.keys_just_pressed.insert(engine_key);
                }
                self.keys_pressed.insert(engine_key);
            }
            winit::event::ElementState::Released => {
                self.keys_pressed.remove(&engine_key);
                self.keys_just_released.insert(engine_key);
            }
        }
    }

    /// Handle mouse button input from winit.
    pub fn handle_mouse_button(
        &mut self,
        button: winit::event::MouseButton,
        state: winit::event::ElementState,
    ) {
        let Some(engine_btn) = key_convert::from_winit_mouse_button(button) else { return };

        match state {
            winit::event::ElementState::Pressed => {
                self.mouse_buttons_pressed.insert(engine_btn);
            }
            winit::event::ElementState::Released => {
                self.mouse_buttons_pressed.remove(&engine_btn);
            }
        }
    }

    pub fn handle_mouse_move(&mut self, x: f32, y: f32) {
        let old_pos = self.mouse_position;
        self.mouse_position = (x, y);
        self.mouse_delta = (x - old_pos.0, y - old_pos.1);
    }

    pub fn handle_mouse_wheel(&mut self, delta: f32) {
        self.scroll_delta += delta;
    }

    pub fn handle_raw_mouse_motion(&mut self, delta_x: f64, delta_y: f64) {
        self.raw_mouse_delta.0 += delta_x as f32;
        self.raw_mouse_delta.1 += delta_y as f32;
    }

    pub fn set_use_raw_mouse(&mut self, use_raw: bool) {
        self.use_raw_mouse = use_raw;
        if use_raw {
            self.mouse_delta = (0.0, 0.0);
        } else {
            self.raw_mouse_delta = (0.0, 0.0);
        }
    }

    // === Engine KeyCode query methods ===

    pub fn is_key_pressed(&self, keycode: action::KeyCode) -> bool {
        self.keys_pressed.contains(&keycode)
    }

    pub fn is_key_just_pressed(&self, keycode: action::KeyCode) -> bool {
        self.keys_just_pressed.contains(&keycode)
    }

    pub fn is_key_just_released(&self, keycode: action::KeyCode) -> bool {
        self.keys_just_released.contains(&keycode)
    }

    /// Is mouse button pressed (engine MouseButton)?
    pub fn is_mouse_button_pressed(&self, button: action::MouseButton) -> bool {
        self.mouse_buttons_pressed.contains(&button)
    }

    /// Is mouse button pressed (winit compat)?
    pub fn is_mouse_pressed(&self, button: winit::event::MouseButton) -> bool {
        key_convert::from_winit_mouse_button(button)
            .is_some_and(|b| self.mouse_buttons_pressed.contains(&b))
    }

    pub fn mouse_position(&self) -> (f32, f32) {
        self.mouse_position
    }

    pub fn mouse_delta(&self) -> (f32, f32) {
        if self.use_raw_mouse { self.raw_mouse_delta } else { self.mouse_delta }
    }

    pub fn scroll_delta(&self) -> f32 {
        self.scroll_delta
    }

    // === Winit-compatible query methods ===

    pub fn is_winit_key_pressed(&self, keycode: winit::keyboard::KeyCode) -> bool {
        key_convert::from_winit_keycode(keycode).is_some_and(|k| self.keys_pressed.contains(&k))
    }

    pub fn is_winit_key_just_pressed(&self, keycode: winit::keyboard::KeyCode) -> bool {
        key_convert::from_winit_keycode(keycode).is_some_and(|k| self.keys_just_pressed.contains(&k))
    }

    pub fn is_winit_key_just_released(&self, keycode: winit::keyboard::KeyCode) -> bool {
        key_convert::from_winit_keycode(keycode).is_some_and(|k| self.keys_just_released.contains(&k))
    }
}
