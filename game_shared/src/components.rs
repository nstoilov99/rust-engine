//! Gameplay component types — pure data, zero engine dependencies.
//!
//! Serialized fields are the authoring config; `#[serde(skip)]` fields are
//! per-frame runtime state written by the `game_client` systems and never
//! reach a scene file.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// CharacterMovement
// ---------------------------------------------------------------------------

/// Velocity-set character controller config + runtime state (Task 41.6 D2).
///
/// Lives on the entity that carries the Rapier capsule (`RigidBody` +
/// `Collider`). Written by `PlayerInputSystem` (intent) and
/// `CharacterMovementSystem` (state); read by the anim bridge.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CharacterMovement {
    /// Horizontal speed with no sprint, m/s.
    pub walk_speed: f32,
    /// Horizontal speed while sprinting, m/s.
    pub run_speed: f32,
    /// Horizontal acceleration toward the desired velocity, m/s².
    pub accel: f32,
    /// Horizontal deceleration when there is no input, m/s².
    pub decel: f32,
    /// Vertical velocity set on a grounded jump, m/s.
    pub jump_speed: f32,
    /// Orient-to-movement turn rate, degrees/s.
    pub turn_rate_deg: f32,
    /// Highest ledge the step assist will hop, metres above the feet.
    pub step_height: f32,

    /// Camera-relative movement intent in the XY plane, magnitude ≤ 1.
    #[serde(skip)]
    pub desired_dir: [f32; 2],
    /// Sprint held this frame.
    #[serde(skip)]
    pub run: bool,
    /// Jump pressed this frame; consumed by the movement system.
    #[serde(skip)]
    pub jump_requested: bool,
    /// Ground probe hit this frame.
    #[serde(skip)]
    pub grounded: bool,
    /// Horizontal speed of the velocity set this frame, m/s.
    #[serde(skip)]
    pub horizontal_speed: f32,
    /// Velocity set this frame, Z-up game space, m/s.
    #[serde(skip)]
    pub velocity: [f32; 3],
    /// Seconds left during which the ground probe, ground snap and step
    /// assist are suppressed after a jump (the physics step is fixed-rate,
    /// so the probe would still see the ground for a frame or two).
    #[serde(skip)]
    pub jump_hold: f32,
}

impl Default for CharacterMovement {
    fn default() -> Self {
        Self {
            walk_speed: 1.6,
            run_speed: 4.5,
            accel: 20.0,
            decel: 30.0,
            jump_speed: 8.0,
            turn_rate_deg: 720.0,
            step_height: 0.35,
            desired_dir: [0.0; 2],
            run: false,
            jump_requested: false,
            grounded: false,
            horizontal_speed: 0.0,
            velocity: [0.0; 3],
            jump_hold: 0.0,
        }
    }
}

// ---------------------------------------------------------------------------
// PlayerInput
// ---------------------------------------------------------------------------

/// Input configuration for a player-controlled entity.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PlayerInput {
    pub mapping_context: String,
    pub move_action: String,
    pub look_action: String,
    pub jump_action: String,
    pub sprint_action: String,
    #[serde(skip)]
    pub context_active: bool,
}

impl Default for PlayerInput {
    fn default() -> Self {
        Self {
            mapping_context: "gameplay".to_string(),
            move_action: "move".to_string(),
            look_action: "look".to_string(),
            jump_action: "jump".to_string(),
            sprint_action: "sprint".to_string(),
            context_active: false,
        }
    }
}

// ---------------------------------------------------------------------------
// OrbitCamera
// ---------------------------------------------------------------------------

/// Third-person orbit camera config + runtime state (Task 41.6 D3).
///
/// Lives on the `Camera` entity, not parented to the target — the boom is
/// computed. `target` is the followed entity's GUID (`EntityGuid.0`); `None`
/// means "the first `CharacterMovement` entity".
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OrbitCamera {
    pub target: Option<uuid::Uuid>,
    /// Boom length, metres; scroll zoom clamps it to `[min, max]_distance`.
    pub distance: f32,
    pub min_distance: f32,
    pub max_distance: f32,
    /// Pivot above the target's origin, metres.
    pub pivot_height: f32,
    /// Pivot offset along the camera's right axis, metres.
    pub shoulder: f32,
    /// Radians per `look` axis unit.
    pub sensitivity: f32,
    pub pitch_min_deg: f32,
    pub pitch_max_deg: f32,

    /// Heading in the XY plane, radians (0 = +X). `PlayerInputSystem`
    /// reads this for camera-relative movement.
    #[serde(skip)]
    pub yaw: f32,
    /// Elevation, radians, positive looks up.
    #[serde(skip)]
    pub pitch: f32,
}

impl Default for OrbitCamera {
    fn default() -> Self {
        Self {
            target: None,
            distance: 3.5,
            min_distance: 1.5,
            max_distance: 8.0,
            pivot_height: 1.4,
            shoulder: 0.4,
            sensitivity: 0.003,
            pitch_min_deg: -60.0,
            pitch_max_deg: 70.0,
            yaw: 0.0,
            pitch: 0.0,
        }
    }
}

// ---------------------------------------------------------------------------
// NetProxy
// ---------------------------------------------------------------------------

/// Marker on client-side proxies of replicated net entities (contract §2.1).
/// Spawned/despawned only by the replication system; the scene serializer
/// skips any entity carrying it. Deliberately not serde-serializable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NetProxy {
    pub realm_id: u32,
    pub entity_id: u64,
    pub generation: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_character_movement_values() {
        let cm = CharacterMovement::default();
        assert_eq!(cm.walk_speed, 1.6);
        assert_eq!(cm.run_speed, 4.5);
        assert_eq!(cm.jump_speed, 8.0);
        assert_eq!(cm.step_height, 0.35);
        assert!(!cm.grounded && !cm.jump_requested && !cm.run);
        assert_eq!(cm.desired_dir, [0.0; 2]);
    }

    #[test]
    fn runtime_fields_are_not_serialized_and_missing_config_defaults() {
        let cm = CharacterMovement {
            walk_speed: 2.0,
            jump_requested: true,
            grounded: true,
            ..Default::default()
        };
        let text = ron::to_string(&cm).unwrap();
        assert!(!text.contains("jump_requested") && !text.contains("grounded"));
        let back: CharacterMovement = ron::from_str(&text).unwrap();
        assert_eq!(back.walk_speed, 2.0);
        assert!(!back.jump_requested && !back.grounded);

        let partial: CharacterMovement = ron::from_str("(run_speed: 6.0)").unwrap();
        assert_eq!(partial.run_speed, 6.0);
        assert_eq!(partial.walk_speed, 1.6);
    }

    #[test]
    fn default_player_input_values() {
        let pi = PlayerInput::default();
        assert_eq!(pi.mapping_context, "gameplay");
        assert_eq!(pi.move_action, "move");
        assert!(!pi.context_active);
    }

    #[test]
    fn orbit_camera_defaults_and_target_roundtrip() {
        let oc = OrbitCamera::default();
        assert_eq!(oc.distance, 3.5);
        assert_eq!((oc.min_distance, oc.max_distance), (1.5, 8.0));
        assert_eq!((oc.pitch_min_deg, oc.pitch_max_deg), (-60.0, 70.0));
        assert!(oc.target.is_none());

        let target = uuid::Uuid::from_u128(0xfeed_beef);
        let oc = OrbitCamera {
            target: Some(target),
            yaw: 1.0,
            ..Default::default()
        };
        let text = ron::to_string(&oc).unwrap();
        assert!(!text.contains("yaw"));
        let back: OrbitCamera = ron::from_str(&text).unwrap();
        assert_eq!(back.target, Some(target));
        assert_eq!(back.yaw, 0.0);
    }
}
