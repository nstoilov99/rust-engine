//! Editor panel state + label helpers for the Enhanced Input action set.
//!
//! Holds an editable working copy of the InputActionSet. Changes are applied
//! back to the InputSubsystem when the user clicks "Apply".
//!
//! The old rendering fns were removed; the crusty analog lives in
//! `input_editors_crusty`. Small label helpers and enum-variant tables that
//! the crusty code re-uses are kept here as `pub(crate)` items.

use crate::engine::input::action::{
    GamepadAxisType, GamepadButton, InputSource, KeyCode, MouseAxisType, MouseButton,
};
use crate::engine::input::enhanced_action::InputActionSet;
use crate::engine::input::modifier::{CurveType, InputModifier, SwizzleOrder};
use crate::engine::input::trigger::InputTrigger;
use crate::engine::input::value::InputValueType;

/// Editor panel for enhanced input settings.
#[derive(Default)]
pub struct InputSettingsPanel {
    pub(crate) working_copy: Option<InputActionSet>,
    pub(crate) pending_apply: Option<InputActionSet>,
    pub(crate) status_message: Option<(String, f64)>,
}

impl InputSettingsPanel {
    pub fn new() -> Self {
        Self::default()
    }

    /// Take the pending apply. Caller writes it back to the InputSubsystem.
    pub fn take_pending_apply(&mut self) -> Option<InputActionSet> {
        self.pending_apply.take()
    }
}

// ── Enum variant tables ──

pub(crate) const VALUE_TYPES: &[InputValueType] = &[
    InputValueType::Digital,
    InputValueType::Axis1D,
    InputValueType::Axis2D,
    InputValueType::Axis3D,
];

pub(crate) const SWIZZLE_ORDERS: &[SwizzleOrder] = &[
    SwizzleOrder::YXZ,
    SwizzleOrder::ZYX,
    SwizzleOrder::XZY,
    SwizzleOrder::YZX,
    SwizzleOrder::ZXY,
];

pub(crate) const MOUSE_BUTTONS: &[MouseButton] = &[
    MouseButton::Left, MouseButton::Right, MouseButton::Middle,
    MouseButton::Back, MouseButton::Forward,
];

pub(crate) const MOUSE_AXES: &[MouseAxisType] = &[
    MouseAxisType::MoveX, MouseAxisType::MoveY, MouseAxisType::ScrollY,
];

pub(crate) const GAMEPAD_BUTTONS: &[GamepadButton] = &[
    GamepadButton::South, GamepadButton::East, GamepadButton::West, GamepadButton::North,
    GamepadButton::LeftBumper, GamepadButton::RightBumper,
    GamepadButton::LeftTrigger, GamepadButton::RightTrigger,
    GamepadButton::Select, GamepadButton::Start,
    GamepadButton::LeftStick, GamepadButton::RightStick,
    GamepadButton::DPadUp, GamepadButton::DPadDown, GamepadButton::DPadLeft, GamepadButton::DPadRight,
];

pub(crate) const GAMEPAD_AXES: &[GamepadAxisType] = &[
    GamepadAxisType::LeftStickX, GamepadAxisType::LeftStickY,
    GamepadAxisType::RightStickX, GamepadAxisType::RightStickY,
    GamepadAxisType::LeftTrigger, GamepadAxisType::RightTrigger,
];

pub(crate) const KEY_LETTERS: &[KeyCode] = &[
    KeyCode::KeyA, KeyCode::KeyB, KeyCode::KeyC, KeyCode::KeyD,
    KeyCode::KeyE, KeyCode::KeyF, KeyCode::KeyG, KeyCode::KeyH,
    KeyCode::KeyI, KeyCode::KeyJ, KeyCode::KeyK, KeyCode::KeyL,
    KeyCode::KeyM, KeyCode::KeyN, KeyCode::KeyO, KeyCode::KeyP,
    KeyCode::KeyQ, KeyCode::KeyR, KeyCode::KeyS, KeyCode::KeyT,
    KeyCode::KeyU, KeyCode::KeyV, KeyCode::KeyW, KeyCode::KeyX,
    KeyCode::KeyY, KeyCode::KeyZ,
];

pub(crate) const KEY_DIGITS: &[KeyCode] = &[
    KeyCode::Digit0, KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3,
    KeyCode::Digit4, KeyCode::Digit5, KeyCode::Digit6, KeyCode::Digit7,
    KeyCode::Digit8, KeyCode::Digit9,
];

pub(crate) const KEY_FUNCTION: &[KeyCode] = &[
    KeyCode::F1, KeyCode::F2, KeyCode::F3, KeyCode::F4,
    KeyCode::F5, KeyCode::F6, KeyCode::F7, KeyCode::F8,
    KeyCode::F9, KeyCode::F10, KeyCode::F11, KeyCode::F12,
];

pub(crate) const KEY_NAV: &[KeyCode] = &[
    KeyCode::Escape, KeyCode::Space, KeyCode::Enter, KeyCode::Backspace,
    KeyCode::Tab, KeyCode::Delete, KeyCode::Insert, KeyCode::Home,
    KeyCode::End, KeyCode::PageUp, KeyCode::PageDown,
    KeyCode::ArrowUp, KeyCode::ArrowDown, KeyCode::ArrowLeft, KeyCode::ArrowRight,
];

pub(crate) const KEY_MODIFIERS: &[KeyCode] = &[
    KeyCode::ShiftLeft, KeyCode::ShiftRight,
    KeyCode::ControlLeft, KeyCode::ControlRight,
    KeyCode::AltLeft, KeyCode::AltRight,
    KeyCode::SuperLeft, KeyCode::SuperRight,
];

pub(crate) const KEY_PUNCTUATION: &[KeyCode] = &[
    KeyCode::Comma, KeyCode::Period, KeyCode::Semicolon, KeyCode::Quote,
    KeyCode::BracketLeft, KeyCode::BracketRight, KeyCode::Backslash,
    KeyCode::Slash, KeyCode::Minus, KeyCode::Equal, KeyCode::Backquote,
];

// ── Label helpers (shared with the crusty editors) ──

pub(crate) fn modifier_label(m: &InputModifier) -> &'static str {
    match m {
        InputModifier::Negate { .. } => "Negate",
        InputModifier::Swizzle { .. } => "Swizzle",
        InputModifier::DeadZone { .. } => "DeadZone",
        InputModifier::Scale { .. } => "Scale",
        InputModifier::Smooth { .. } => "Smooth",
        InputModifier::ResponseCurve { .. } => "Curve",
        InputModifier::Clamp { .. } => "Clamp",
    }
}

pub(crate) fn curve_label(c: &CurveType) -> &'static str {
    match c {
        CurveType::Linear => "Linear",
        CurveType::Quadratic => "Quadratic",
        CurveType::Cubic => "Cubic",
        CurveType::Custom(_) => "Custom",
    }
}

pub(crate) fn trigger_label(t: &InputTrigger) -> &'static str {
    match t {
        InputTrigger::Down => "Down",
        InputTrigger::Pressed => "Pressed",
        InputTrigger::Released => "Released",
        InputTrigger::Held { .. } => "Held",
        InputTrigger::Tap { .. } => "Tap",
        InputTrigger::Pulse { .. } => "Pulse",
        InputTrigger::ChordAction { .. } => "Chord",
    }
}

pub(crate) fn format_value_type(vt: InputValueType) -> &'static str {
    match vt {
        InputValueType::Digital => "Digital",
        InputValueType::Axis1D => "Axis1D",
        InputValueType::Axis2D => "Axis2D",
        InputValueType::Axis3D => "Axis3D",
    }
}

pub(crate) fn source_type_label(source: &InputSource) -> &'static str {
    match source {
        InputSource::Key(_) => "Key",
        InputSource::MouseButton(_) => "Mouse Btn",
        InputSource::MouseAxis(_) => "Mouse Axis",
        InputSource::GamepadButton(_) => "GP Button",
        InputSource::GamepadAxis(_) => "GP Axis",
    }
}
