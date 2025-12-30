use winit::event::{ElementState, MouseButton};
use winit::keyboard::KeyCode;
use std::collections::HashSet;

/// Tracks input state (keyboard and mouse)
/// Updated for winit 0.30 - uses KeyCode instead of VirtualKeyCode
pub struct InputManager {
    keys_pressed: HashSet<KeyCode>,
    keys_just_pressed: HashSet<KeyCode>,
    keys_just_released: HashSet<KeyCode>,

    mouse_buttons_pressed: HashSet<MouseButton>,
    mouse_position: (f32, f32),
    mouse_delta: (f32, f32),
    scroll_delta: f32,
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
        }
    }

    /// Call this at the start of each frame
    pub fn new_frame(&mut self) {
        self.keys_just_pressed.clear();
        self.keys_just_released.clear();
        self.mouse_delta = (0.0, 0.0);
        self.scroll_delta = 0.0;
    }

    /// Handle keyboard input (winit 0.30)
    pub fn handle_keyboard(&mut self, keycode: Option<KeyCode>, state: ElementState) {
        if let Some(keycode) = keycode {
            match state {
                ElementState::Pressed => {
                    if !self.keys_pressed.contains(&keycode) {
                        self.keys_just_pressed.insert(keycode);
                    }
                    self.keys_pressed.insert(keycode);
                }
                ElementState::Released => {
                    self.keys_pressed.remove(&keycode);
                    self.keys_just_released.insert(keycode);
                }
            }
        }
    }

    /// Handle mouse button input
    pub fn handle_mouse_button(&mut self, button: MouseButton, state: ElementState) {
        match state {
            ElementState::Pressed => {
                self.mouse_buttons_pressed.insert(button);
            }
            ElementState::Released => {
                self.mouse_buttons_pressed.remove(&button);
            }
        }
    }

    /// Handle mouse movement
    pub fn handle_mouse_move(&mut self, x: f32, y: f32) {
        let old_pos = self.mouse_position;
        self.mouse_position = (x, y);
        self.mouse_delta = (x - old_pos.0, y - old_pos.1);
    }

    /// Handle mouse wheel
    pub fn handle_mouse_wheel(&mut self, delta: f32) {
        self.scroll_delta += delta;
    }

    // Query methods

    /// Is key currently held down?
    pub fn is_key_pressed(&self, keycode: KeyCode) -> bool {
        self.keys_pressed.contains(&keycode)
    }

    /// Was key just pressed this frame?
    pub fn is_key_just_pressed(&self, keycode: KeyCode) -> bool {
        self.keys_just_pressed.contains(&keycode)
    }

    /// Was key just released this frame?
    pub fn is_key_just_released(&self, keycode: KeyCode) -> bool {
        self.keys_just_released.contains(&keycode)
    }

    /// Is mouse button pressed?
    pub fn is_mouse_pressed(&self, button: MouseButton) -> bool {
        self.mouse_buttons_pressed.contains(&button)
    }

    /// Get current mouse position
    pub fn mouse_position(&self) -> (f32, f32) {
        self.mouse_position
    }

    /// Get mouse movement delta this frame
    pub fn mouse_delta(&self) -> (f32, f32) {
        self.mouse_delta
    }

    /// Get mouse wheel scroll delta this frame
    pub fn scroll_delta(&self) -> f32 {
        self.scroll_delta
    }
}
