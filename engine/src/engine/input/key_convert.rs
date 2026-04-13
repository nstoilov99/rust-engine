//! Conversion helpers from winit types to engine-owned input types.

use super::action;

/// Convert a winit `KeyCode` to the engine's `KeyCode`.
/// Returns `None` for unmapped/exotic keys.
pub fn from_winit_keycode(winit_key: winit::keyboard::KeyCode) -> Option<action::KeyCode> {
    use winit::keyboard::KeyCode as W;
    let mapped = match winit_key {
        W::KeyA => action::KeyCode::KeyA,
        W::KeyB => action::KeyCode::KeyB,
        W::KeyC => action::KeyCode::KeyC,
        W::KeyD => action::KeyCode::KeyD,
        W::KeyE => action::KeyCode::KeyE,
        W::KeyF => action::KeyCode::KeyF,
        W::KeyG => action::KeyCode::KeyG,
        W::KeyH => action::KeyCode::KeyH,
        W::KeyI => action::KeyCode::KeyI,
        W::KeyJ => action::KeyCode::KeyJ,
        W::KeyK => action::KeyCode::KeyK,
        W::KeyL => action::KeyCode::KeyL,
        W::KeyM => action::KeyCode::KeyM,
        W::KeyN => action::KeyCode::KeyN,
        W::KeyO => action::KeyCode::KeyO,
        W::KeyP => action::KeyCode::KeyP,
        W::KeyQ => action::KeyCode::KeyQ,
        W::KeyR => action::KeyCode::KeyR,
        W::KeyS => action::KeyCode::KeyS,
        W::KeyT => action::KeyCode::KeyT,
        W::KeyU => action::KeyCode::KeyU,
        W::KeyV => action::KeyCode::KeyV,
        W::KeyW => action::KeyCode::KeyW,
        W::KeyX => action::KeyCode::KeyX,
        W::KeyY => action::KeyCode::KeyY,
        W::KeyZ => action::KeyCode::KeyZ,

        W::Digit0 => action::KeyCode::Digit0,
        W::Digit1 => action::KeyCode::Digit1,
        W::Digit2 => action::KeyCode::Digit2,
        W::Digit3 => action::KeyCode::Digit3,
        W::Digit4 => action::KeyCode::Digit4,
        W::Digit5 => action::KeyCode::Digit5,
        W::Digit6 => action::KeyCode::Digit6,
        W::Digit7 => action::KeyCode::Digit7,
        W::Digit8 => action::KeyCode::Digit8,
        W::Digit9 => action::KeyCode::Digit9,

        W::F1 => action::KeyCode::F1,
        W::F2 => action::KeyCode::F2,
        W::F3 => action::KeyCode::F3,
        W::F4 => action::KeyCode::F4,
        W::F5 => action::KeyCode::F5,
        W::F6 => action::KeyCode::F6,
        W::F7 => action::KeyCode::F7,
        W::F8 => action::KeyCode::F8,
        W::F9 => action::KeyCode::F9,
        W::F10 => action::KeyCode::F10,
        W::F11 => action::KeyCode::F11,
        W::F12 => action::KeyCode::F12,

        W::Escape => action::KeyCode::Escape,
        W::Space => action::KeyCode::Space,
        W::Enter => action::KeyCode::Enter,
        W::Backspace => action::KeyCode::Backspace,
        W::Tab => action::KeyCode::Tab,
        W::Delete => action::KeyCode::Delete,
        W::Insert => action::KeyCode::Insert,
        W::Home => action::KeyCode::Home,
        W::End => action::KeyCode::End,
        W::PageUp => action::KeyCode::PageUp,
        W::PageDown => action::KeyCode::PageDown,

        W::ArrowUp => action::KeyCode::ArrowUp,
        W::ArrowDown => action::KeyCode::ArrowDown,
        W::ArrowLeft => action::KeyCode::ArrowLeft,
        W::ArrowRight => action::KeyCode::ArrowRight,

        W::ShiftLeft => action::KeyCode::ShiftLeft,
        W::ShiftRight => action::KeyCode::ShiftRight,
        W::ControlLeft => action::KeyCode::ControlLeft,
        W::ControlRight => action::KeyCode::ControlRight,
        W::AltLeft => action::KeyCode::AltLeft,
        W::AltRight => action::KeyCode::AltRight,
        W::SuperLeft => action::KeyCode::SuperLeft,
        W::SuperRight => action::KeyCode::SuperRight,

        W::Comma => action::KeyCode::Comma,
        W::Period => action::KeyCode::Period,
        W::Semicolon => action::KeyCode::Semicolon,
        W::Quote => action::KeyCode::Quote,
        W::BracketLeft => action::KeyCode::BracketLeft,
        W::BracketRight => action::KeyCode::BracketRight,
        W::Backslash => action::KeyCode::Backslash,
        W::Slash => action::KeyCode::Slash,
        W::Minus => action::KeyCode::Minus,
        W::Equal => action::KeyCode::Equal,
        W::Backquote => action::KeyCode::Backquote,

        _ => return None,
    };
    Some(mapped)
}

/// Convert an engine `KeyCode` back to a winit `KeyCode`.
pub fn to_winit_keycode(key: action::KeyCode) -> winit::keyboard::KeyCode {
    use winit::keyboard::KeyCode as W;
    match key {
        action::KeyCode::KeyA => W::KeyA,
        action::KeyCode::KeyB => W::KeyB,
        action::KeyCode::KeyC => W::KeyC,
        action::KeyCode::KeyD => W::KeyD,
        action::KeyCode::KeyE => W::KeyE,
        action::KeyCode::KeyF => W::KeyF,
        action::KeyCode::KeyG => W::KeyG,
        action::KeyCode::KeyH => W::KeyH,
        action::KeyCode::KeyI => W::KeyI,
        action::KeyCode::KeyJ => W::KeyJ,
        action::KeyCode::KeyK => W::KeyK,
        action::KeyCode::KeyL => W::KeyL,
        action::KeyCode::KeyM => W::KeyM,
        action::KeyCode::KeyN => W::KeyN,
        action::KeyCode::KeyO => W::KeyO,
        action::KeyCode::KeyP => W::KeyP,
        action::KeyCode::KeyQ => W::KeyQ,
        action::KeyCode::KeyR => W::KeyR,
        action::KeyCode::KeyS => W::KeyS,
        action::KeyCode::KeyT => W::KeyT,
        action::KeyCode::KeyU => W::KeyU,
        action::KeyCode::KeyV => W::KeyV,
        action::KeyCode::KeyW => W::KeyW,
        action::KeyCode::KeyX => W::KeyX,
        action::KeyCode::KeyY => W::KeyY,
        action::KeyCode::KeyZ => W::KeyZ,

        action::KeyCode::Digit0 => W::Digit0,
        action::KeyCode::Digit1 => W::Digit1,
        action::KeyCode::Digit2 => W::Digit2,
        action::KeyCode::Digit3 => W::Digit3,
        action::KeyCode::Digit4 => W::Digit4,
        action::KeyCode::Digit5 => W::Digit5,
        action::KeyCode::Digit6 => W::Digit6,
        action::KeyCode::Digit7 => W::Digit7,
        action::KeyCode::Digit8 => W::Digit8,
        action::KeyCode::Digit9 => W::Digit9,

        action::KeyCode::F1 => W::F1,
        action::KeyCode::F2 => W::F2,
        action::KeyCode::F3 => W::F3,
        action::KeyCode::F4 => W::F4,
        action::KeyCode::F5 => W::F5,
        action::KeyCode::F6 => W::F6,
        action::KeyCode::F7 => W::F7,
        action::KeyCode::F8 => W::F8,
        action::KeyCode::F9 => W::F9,
        action::KeyCode::F10 => W::F10,
        action::KeyCode::F11 => W::F11,
        action::KeyCode::F12 => W::F12,

        action::KeyCode::Escape => W::Escape,
        action::KeyCode::Space => W::Space,
        action::KeyCode::Enter => W::Enter,
        action::KeyCode::Backspace => W::Backspace,
        action::KeyCode::Tab => W::Tab,
        action::KeyCode::Delete => W::Delete,
        action::KeyCode::Insert => W::Insert,
        action::KeyCode::Home => W::Home,
        action::KeyCode::End => W::End,
        action::KeyCode::PageUp => W::PageUp,
        action::KeyCode::PageDown => W::PageDown,

        action::KeyCode::ArrowUp => W::ArrowUp,
        action::KeyCode::ArrowDown => W::ArrowDown,
        action::KeyCode::ArrowLeft => W::ArrowLeft,
        action::KeyCode::ArrowRight => W::ArrowRight,

        action::KeyCode::ShiftLeft => W::ShiftLeft,
        action::KeyCode::ShiftRight => W::ShiftRight,
        action::KeyCode::ControlLeft => W::ControlLeft,
        action::KeyCode::ControlRight => W::ControlRight,
        action::KeyCode::AltLeft => W::AltLeft,
        action::KeyCode::AltRight => W::AltRight,
        action::KeyCode::SuperLeft => W::SuperLeft,
        action::KeyCode::SuperRight => W::SuperRight,

        action::KeyCode::Comma => W::Comma,
        action::KeyCode::Period => W::Period,
        action::KeyCode::Semicolon => W::Semicolon,
        action::KeyCode::Quote => W::Quote,
        action::KeyCode::BracketLeft => W::BracketLeft,
        action::KeyCode::BracketRight => W::BracketRight,
        action::KeyCode::Backslash => W::Backslash,
        action::KeyCode::Slash => W::Slash,
        action::KeyCode::Minus => W::Minus,
        action::KeyCode::Equal => W::Equal,
        action::KeyCode::Backquote => W::Backquote,
    }
}

/// Convert a winit `MouseButton` to the engine's `MouseButton`.
pub fn from_winit_mouse_button(
    winit_btn: winit::event::MouseButton,
) -> Option<action::MouseButton> {
    match winit_btn {
        winit::event::MouseButton::Left => Some(action::MouseButton::Left),
        winit::event::MouseButton::Right => Some(action::MouseButton::Right),
        winit::event::MouseButton::Middle => Some(action::MouseButton::Middle),
        winit::event::MouseButton::Back => Some(action::MouseButton::Back),
        winit::event::MouseButton::Forward => Some(action::MouseButton::Forward),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_keycode() {
        let keys = [
            action::KeyCode::KeyA,
            action::KeyCode::Space,
            action::KeyCode::F12,
            action::KeyCode::ArrowUp,
            action::KeyCode::ControlLeft,
        ];
        for key in keys {
            let winit_key = to_winit_keycode(key);
            let back = from_winit_keycode(winit_key);
            assert_eq!(back, Some(key), "roundtrip failed for {key:?}");
        }
    }

    #[test]
    fn unmapped_winit_key_returns_none() {
        assert_eq!(from_winit_keycode(winit::keyboard::KeyCode::CapsLock), None);
    }

    #[test]
    fn mouse_button_conversion() {
        assert_eq!(
            from_winit_mouse_button(winit::event::MouseButton::Left),
            Some(action::MouseButton::Left)
        );
        assert_eq!(
            from_winit_mouse_button(winit::event::MouseButton::Right),
            Some(action::MouseButton::Right)
        );
    }
}
