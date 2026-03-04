use std::collections::HashSet;
use winit::event::{ElementState, MouseButton};
use winit::keyboard::KeyCode;

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

    /// Raw mouse delta from DeviceEvent::MouseMotion (for camera when cursor is locked)
    raw_mouse_delta: (f32, f32),
    /// Whether to use raw mouse input (when cursor is locked)
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

    /// Call this at the start of each frame
    pub fn new_frame(&mut self) {
        crate::profile_scope!("input_processing");
        self.keys_just_pressed.clear();
        self.keys_just_released.clear();
        self.mouse_delta = (0.0, 0.0);
        self.raw_mouse_delta = (0.0, 0.0);
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

    /// Handle raw mouse motion from DeviceEvent::MouseMotion
    /// This provides delta regardless of cursor position (for camera when cursor is locked)
    pub fn handle_raw_mouse_motion(&mut self, delta_x: f64, delta_y: f64) {
        // Accumulate raw delta (multiple events can arrive per frame)
        self.raw_mouse_delta.0 += delta_x as f32;
        self.raw_mouse_delta.1 += delta_y as f32;
    }

    /// Set whether to use raw mouse input (call when cursor lock state changes)
    pub fn set_use_raw_mouse(&mut self, use_raw: bool) {
        self.use_raw_mouse = use_raw;
        if use_raw {
            // Clear cursor-based delta when switching to raw to avoid jump
            self.mouse_delta = (0.0, 0.0);
        } else {
            // Clear raw delta when switching to cursor-based
            self.raw_mouse_delta = (0.0, 0.0);
        }
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
    /// Returns raw delta when cursor is locked, cursor-based delta otherwise
    pub fn mouse_delta(&self) -> (f32, f32) {
        if self.use_raw_mouse {
            self.raw_mouse_delta
        } else {
            self.mouse_delta
        }
    }

    /// Get mouse wheel scroll delta this frame
    pub fn scroll_delta(&self) -> f32 {
        self.scroll_delta
    }
}
