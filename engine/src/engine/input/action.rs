//! Core action types for the input action system.
//!
//! Defines action types, values, engine-owned key/button enums,
//! and input source bindings.

use serde::{Deserialize, Serialize};

/// The kind of value an action produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ActionType {
    /// On/off (e.g., jump, interact).
    Digital,
    /// Single axis (e.g., throttle, scroll).
    Axis1D,
    /// Two-axis (e.g., movement, look).
    Axis2D,
}

/// Runtime value produced by an action.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ActionValue {
    Digital(bool),
    Axis1D(f32),
    Axis2D(f32, f32),
}

impl ActionValue {
    pub fn zero(action_type: ActionType) -> Self {
        match action_type {
            ActionType::Digital => ActionValue::Digital(false),
            ActionType::Axis1D => ActionValue::Axis1D(0.0),
            ActionType::Axis2D => ActionValue::Axis2D(0.0, 0.0),
        }
    }
}

impl Default for ActionValue {
    fn default() -> Self {
        ActionValue::Digital(false)
    }
}

/// Engine-owned keyboard key codes, decoupled from winit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum KeyCode {
    KeyA,
    KeyB,
    KeyC,
    KeyD,
    KeyE,
    KeyF,
    KeyG,
    KeyH,
    KeyI,
    KeyJ,
    KeyK,
    KeyL,
    KeyM,
    KeyN,
    KeyO,
    KeyP,
    KeyQ,
    KeyR,
    KeyS,
    KeyT,
    KeyU,
    KeyV,
    KeyW,
    KeyX,
    KeyY,
    KeyZ,

    Digit0,
    Digit1,
    Digit2,
    Digit3,
    Digit4,
    Digit5,
    Digit6,
    Digit7,
    Digit8,
    Digit9,

    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,

    Escape,
    Space,
    Enter,
    Backspace,
    Tab,
    Delete,
    Insert,
    Home,
    End,
    PageUp,
    PageDown,

    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,

    ShiftLeft,
    ShiftRight,
    ControlLeft,
    ControlRight,
    AltLeft,
    AltRight,
    SuperLeft,
    SuperRight,

    Comma,
    Period,
    Semicolon,
    Quote,
    BracketLeft,
    BracketRight,
    Backslash,
    Slash,
    Minus,
    Equal,
    Backquote,
}

/// Engine-owned mouse button identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Back,
    Forward,
}

/// Mouse axis types for analog input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MouseAxisType {
    MoveX,
    MoveY,
    ScrollY,
}

/// Gamepad button identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GamepadButton {
    South,
    East,
    West,
    North,
    LeftBumper,
    RightBumper,
    LeftTrigger,
    RightTrigger,
    Select,
    Start,
    LeftStick,
    RightStick,
    DPadUp,
    DPadDown,
    DPadLeft,
    DPadRight,
}

/// Gamepad axis types for analog input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GamepadAxisType {
    LeftStickX,
    LeftStickY,
    RightStickX,
    RightStickY,
    LeftTrigger,
    RightTrigger,
}

/// An input source that can be bound to an action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InputSource {
    Key(KeyCode),
    MouseButton(MouseButton),
    MouseAxis(MouseAxisType),
    GamepadButton(GamepadButton),
    GamepadAxis(GamepadAxisType),
}

/// A binding that maps an input source to an action contribution.
///
/// For digital actions, a key press maps to `true`.
/// For axis actions, `axis_contribution` specifies the value added when the source is active
/// (e.g., Key W → +1.0 on Y axis for forward movement).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionBinding {
    pub source: InputSource,
    /// For Axis1D: the value to contribute when this digital source is active.
    /// For Axis2D: the (x, y) contribution. Only the relevant component is used.
    pub axis_contribution: (f32, f32),
}

impl ActionBinding {
    /// Create a digital binding (key or button press).
    pub fn digital(source: InputSource) -> Self {
        Self {
            source,
            axis_contribution: (0.0, 0.0),
        }
    }

    /// Create a 1D axis binding with a contribution value.
    pub fn axis_1d(source: InputSource, value: f32) -> Self {
        Self {
            source,
            axis_contribution: (value, 0.0),
        }
    }

    /// Create a 2D axis binding with (x, y) contribution.
    pub fn axis_2d(source: InputSource, x: f32, y: f32) -> Self {
        Self {
            source,
            axis_contribution: (x, y),
        }
    }
}

/// A 2D stick binding combining two gamepad axes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GamepadStick2D {
    pub axis_x: GamepadAxisType,
    pub axis_y: GamepadAxisType,
}
