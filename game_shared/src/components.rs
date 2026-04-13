//! Gameplay component types — pure data, zero engine dependencies.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// MovementMode
// ---------------------------------------------------------------------------

/// Movement mode for `CharacterMovement`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum MovementMode {
    #[default]
    Walking,
    Flying,
}

// ---------------------------------------------------------------------------
// CharacterMovement
// ---------------------------------------------------------------------------

/// Movement configuration and runtime physics state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterMovement {
    #[serde(default = "default_move_speed")]
    pub move_speed: f32,
    #[serde(default = "default_sprint_mult")]
    pub sprint_multiplier: f32,
    #[serde(default = "default_jump_impulse")]
    pub jump_impulse: f32,
    #[serde(default = "default_ground_dist")]
    pub ground_check_dist: f32,
    #[serde(default)]
    pub movement_mode: MovementMode,
    #[serde(skip)]
    pub desired_velocity: [f32; 3],
    #[serde(skip)]
    pub is_sprinting: bool,
    #[serde(skip)]
    pub jump_requested: bool,
    #[serde(skip)]
    pub is_grounded: bool,
}

fn default_move_speed() -> f32 { 50.0 }
fn default_sprint_mult() -> f32 { 1.8 }
fn default_jump_impulse() -> f32 { 5.0 }
fn default_ground_dist() -> f32 { 1.05 }

impl Default for CharacterMovement {
    fn default() -> Self {
        Self {
            move_speed: default_move_speed(),
            sprint_multiplier: default_sprint_mult(),
            jump_impulse: default_jump_impulse(),
            ground_check_dist: default_ground_dist(),
            movement_mode: MovementMode::default(),
            desired_velocity: [0.0; 3],
            is_sprinting: false,
            jump_requested: false,
            is_grounded: false,
        }
    }
}

// ---------------------------------------------------------------------------
// LookController
// ---------------------------------------------------------------------------

/// Look/aim configuration and runtime yaw/pitch state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LookController {
    #[serde(default = "default_sensitivity")]
    pub mouse_sensitivity: f32,
    #[serde(default = "default_pitch_min")]
    pub pitch_min: f32,
    #[serde(default = "default_pitch_max")]
    pub pitch_max: f32,
    #[serde(skip)]
    pub yaw: f32,
    #[serde(skip)]
    pub pitch: f32,
}

fn default_sensitivity() -> f32 { 0.003 }
fn default_pitch_min() -> f32 { -89.0_f32.to_radians() }
fn default_pitch_max() -> f32 { 89.0_f32.to_radians() }

impl Default for LookController {
    fn default() -> Self {
        Self {
            mouse_sensitivity: default_sensitivity(),
            pitch_min: default_pitch_min(),
            pitch_max: default_pitch_max(),
            yaw: 0.0,
            pitch: 0.0,
        }
    }
}

// ---------------------------------------------------------------------------
// PlayerInput
// ---------------------------------------------------------------------------

/// Input configuration for a player-controlled entity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerInput {
    #[serde(default = "default_mapping_context")]
    pub mapping_context: String,
    #[serde(default = "default_move_action")]
    pub move_action: String,
    #[serde(default = "default_look_action")]
    pub look_action: String,
    #[serde(default = "default_jump_action")]
    pub jump_action: String,
    #[serde(default = "default_sprint_action")]
    pub sprint_action: String,
    #[serde(skip)]
    pub context_active: bool,
}

fn default_mapping_context() -> String { "gameplay".to_string() }
fn default_move_action() -> String { "move".to_string() }
fn default_look_action() -> String { "look".to_string() }
fn default_jump_action() -> String { "jump".to_string() }
fn default_sprint_action() -> String { "sprint".to_string() }

impl Default for PlayerInput {
    fn default() -> Self {
        Self {
            mapping_context: default_mapping_context(),
            move_action: default_move_action(),
            look_action: default_look_action(),
            jump_action: default_jump_action(),
            sprint_action: default_sprint_action(),
            context_active: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_character_movement_values() {
        let cm = CharacterMovement::default();
        assert_eq!(cm.move_speed, 50.0);
        assert_eq!(cm.sprint_multiplier, 1.8);
        assert_eq!(cm.jump_impulse, 5.0);
        assert_eq!(cm.movement_mode, MovementMode::Walking);
        assert!(!cm.is_grounded);
        assert!(!cm.jump_requested);
        assert!(!cm.is_sprinting);
    }

    #[test]
    fn default_look_controller_values() {
        let look = LookController::default();
        assert_eq!(look.mouse_sensitivity, 0.003);
        assert_eq!(look.yaw, 0.0);
        assert_eq!(look.pitch, 0.0);
        assert!((look.pitch_min - (-89.0_f32.to_radians())).abs() < 0.001);
        assert!((look.pitch_max - 89.0_f32.to_radians()).abs() < 0.001);
    }

    #[test]
    fn pitch_clamp_values() {
        let mut look = LookController::default();
        look.pitch = 100.0_f32.to_radians();
        look.pitch = look.pitch.clamp(look.pitch_min, look.pitch_max);
        assert!((look.pitch - 89.0_f32.to_radians()).abs() < 0.001);
    }

    #[test]
    fn movement_mode_default_is_walking() {
        assert_eq!(MovementMode::default(), MovementMode::Walking);
    }

    #[test]
    fn default_player_input_values() {
        let pi = PlayerInput::default();
        assert_eq!(pi.mapping_context, "gameplay");
        assert_eq!(pi.move_action, "move");
        assert!(!pi.context_active);
    }
}
