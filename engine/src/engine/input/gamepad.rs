//! Gamepad input via gilrs.
//!
//! Wraps `gilrs::Gilrs` as an ECS resource. Polls gilrs events each frame
//! and tracks the active gamepad (first connected).

use super::action::{GamepadAxisType, GamepadButton};
use std::collections::HashSet;

/// Maps a gilrs button to the engine's GamepadButton.
fn map_button(gilrs_btn: gilrs::Button) -> Option<GamepadButton> {
    match gilrs_btn {
        gilrs::Button::South => Some(GamepadButton::South),
        gilrs::Button::East => Some(GamepadButton::East),
        gilrs::Button::West => Some(GamepadButton::West),
        gilrs::Button::North => Some(GamepadButton::North),
        gilrs::Button::LeftTrigger => Some(GamepadButton::LeftBumper),
        gilrs::Button::RightTrigger => Some(GamepadButton::RightBumper),
        gilrs::Button::LeftTrigger2 => Some(GamepadButton::LeftTrigger),
        gilrs::Button::RightTrigger2 => Some(GamepadButton::RightTrigger),
        gilrs::Button::Select => Some(GamepadButton::Select),
        gilrs::Button::Start => Some(GamepadButton::Start),
        gilrs::Button::LeftThumb => Some(GamepadButton::LeftStick),
        gilrs::Button::RightThumb => Some(GamepadButton::RightStick),
        gilrs::Button::DPadUp => Some(GamepadButton::DPadUp),
        gilrs::Button::DPadDown => Some(GamepadButton::DPadDown),
        gilrs::Button::DPadLeft => Some(GamepadButton::DPadLeft),
        gilrs::Button::DPadRight => Some(GamepadButton::DPadRight),
        _ => None,
    }
}

/// Maps an engine GamepadAxisType to a gilrs axis.
fn engine_axis_to_gilrs(axis: GamepadAxisType) -> gilrs::Axis {
    match axis {
        GamepadAxisType::LeftStickX => gilrs::Axis::LeftStickX,
        GamepadAxisType::LeftStickY => gilrs::Axis::LeftStickY,
        GamepadAxisType::RightStickX => gilrs::Axis::RightStickX,
        GamepadAxisType::RightStickY => gilrs::Axis::RightStickY,
        GamepadAxisType::LeftTrigger => gilrs::Axis::LeftZ,
        GamepadAxisType::RightTrigger => gilrs::Axis::RightZ,
    }
}

/// Gamepad input resource wrapping gilrs.
///
/// Tracks one active gamepad (the first connected controller).
pub struct GamepadState {
    gilrs: gilrs::Gilrs,
    active_gamepad: Option<gilrs::GamepadId>,
    buttons_pressed: HashSet<GamepadButton>,
}

// SAFETY: GamepadState is only accessed on the main thread via the game loop.
// gilrs::Gilrs is !Sync due to internal mpsc::Receiver, but we never share
// it across threads — it lives in ECS Resources which requires Send+Sync bounds.
unsafe impl Sync for GamepadState {}

impl GamepadState {
    /// Create a new GamepadState. Returns None if gilrs fails to initialize.
    pub fn try_new() -> Option<Self> {
        match gilrs::Gilrs::new() {
            Ok(gilrs) => {
                // Find the first connected gamepad
                let active = gilrs.gamepads().next().map(|(id, _)| id);
                if let Some(id) = active {
                    let gp = gilrs.gamepad(id);
                    log::info!("Gamepad connected: {} ({:?})", gp.name(), gp.uuid());
                }
                Some(Self {
                    gilrs,
                    active_gamepad: active,
                    buttons_pressed: HashSet::new(),
                })
            }
            Err(e) => {
                log::warn!("Failed to initialize gilrs (gamepad support disabled): {e}");
                None
            }
        }
    }

    /// Poll gilrs events. Call once per frame in `begin_frame()`.
    pub fn update(&mut self) {
        while let Some(event) = self.gilrs.next_event() {
            match event.event {
                gilrs::EventType::Connected => {
                    if self.active_gamepad.is_none() {
                        self.active_gamepad = Some(event.id);
                        let gp = self.gilrs.gamepad(event.id);
                        log::info!("Gamepad connected: {} ({:?})", gp.name(), gp.uuid());
                    }
                }
                gilrs::EventType::Disconnected => {
                    if self.active_gamepad == Some(event.id) {
                        log::info!("Active gamepad disconnected");
                        self.active_gamepad = None;
                        self.buttons_pressed.clear();
                        // Find another connected gamepad
                        for (id, gp) in self.gilrs.gamepads() {
                            if gp.is_connected() && id != event.id {
                                self.active_gamepad = Some(id);
                                log::info!(
                                    "Switched to gamepad: {} ({:?})",
                                    gp.name(),
                                    gp.uuid()
                                );
                                break;
                            }
                        }
                    }
                }
                gilrs::EventType::ButtonPressed(btn, _) => {
                    if Some(event.id) == self.active_gamepad {
                        if let Some(engine_btn) = map_button(btn) {
                            self.buttons_pressed.insert(engine_btn);
                        }
                    }
                }
                gilrs::EventType::ButtonReleased(btn, _) => {
                    if Some(event.id) == self.active_gamepad {
                        if let Some(engine_btn) = map_button(btn) {
                            self.buttons_pressed.remove(&engine_btn);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    /// Is a gamepad button currently pressed?
    pub fn is_pressed(&self, button: GamepadButton) -> bool {
        self.buttons_pressed.contains(&button)
    }

    /// Get the value of a gamepad axis [-1.0, 1.0].
    pub fn axis_value(&self, axis: GamepadAxisType) -> f32 {
        let Some(id) = self.active_gamepad else {
            return 0.0;
        };
        let gp = self.gilrs.gamepad(id);
        let gilrs_axis = engine_axis_to_gilrs(axis);
        gp.axis_data(gilrs_axis)
            .map(|d| d.value())
            .unwrap_or(0.0)
    }

    /// Whether any gamepad is connected.
    pub fn has_gamepad(&self) -> bool {
        self.active_gamepad.is_some()
    }
}
