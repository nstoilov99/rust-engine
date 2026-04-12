//! Player controller components and systems — Unreal/Unity-style architecture.
//!
//! # Components
//!
//! - [`CharacterMovement`] — movement config + runtime physics state
//!   (like Unreal's `UCharacterMovementComponent`)
//! - [`LookController`] — decoupled look/aim state
//!   (like Unreal's controller rotation)
//!
//! # Systems
//!
//! - [`PlayerInputSystem`] — reads Enhanced Input → writes intent to components
//!   (only for entities with `Player` tag; AI characters skip this)
//! - [`CharacterMovementSystem`] — executes movement intent via physics
//!   (works for both player and AI entities)
//!
//! # Camera setup
//!
//! Parent a `Camera` entity to the player. The camera's local `Transform.position`
//! is the eye-height offset (e.g. `[0, 0, 1.7]`). `CharacterMovementSystem` applies
//! pitch rotation to the camera child's local transform. The existing
//! `TransformPropagationSystem` combines player world × camera local automatically.
//!
//! # Components
//!
//! - [`PlayerInput`] — mapping context + action name references
//!   (like Unreal's `APlayerController` input setup / `UInputMappingContext` reference)
//! - [`CharacterMovement`] — movement config + runtime physics state
//!   (like Unreal's `UCharacterMovementComponent`)
//! - [`LookController`] — decoupled look/aim state
//!   (like Unreal's controller rotation)
//!
//! # Entity setup
//!
//! ```text
//! Player Entity:
//!   Transform + Player + PlayerInput + CharacterMovement + LookController
//!   + RigidBody + Collider
//!   └── Camera Child Entity:
//!       Transform (position = eye height offset) + Camera (active = true)
//! ```

use nalgebra_glm as glm;
use serde::{Deserialize, Serialize};

use crate::engine::adapters::physics_adapter::velocity_to_physics;
use crate::engine::ecs::access::SystemDescriptor;
use crate::engine::ecs::components::{Camera, Player, Transform, TransformDirty};
use crate::engine::ecs::hierarchy::Children;
use crate::engine::ecs::resources::{Resources, Time};
use crate::engine::ecs::schedule::System;
use crate::engine::input::subsystem::InputSubsystem;
use crate::engine::physics::{PhysicsWorld, RigidBody};

// ---------------------------------------------------------------------------
// MovementMode
// ---------------------------------------------------------------------------

/// Movement mode for `CharacterMovement`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum MovementMode {
    /// Ground-based movement with gravity and ground detection.
    #[default]
    Walking,
    /// Free-flight movement — gravity disabled, vertical input allowed.
    Flying,
}

// ---------------------------------------------------------------------------
// CharacterMovement
// ---------------------------------------------------------------------------

/// Movement configuration and runtime physics state.
///
/// Attach to any entity with a `RigidBody` + `Collider` to give it
/// physics-based movement. For player control, also add `Player` +
/// `LookController`. For AI, set `desired_velocity` directly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterMovement {
    /// Horizontal movement force magnitude.
    #[serde(default = "default_move_speed")]
    pub move_speed: f32,
    /// Sprint speed multiplier.
    #[serde(default = "default_sprint_mult")]
    pub sprint_multiplier: f32,
    /// Upward impulse applied on jump.
    #[serde(default = "default_jump_impulse")]
    pub jump_impulse: f32,
    /// Raycast distance below entity center for ground detection.
    #[serde(default = "default_ground_dist")]
    pub ground_check_dist: f32,
    /// Current movement mode.
    #[serde(default)]
    pub movement_mode: MovementMode,

    // -- runtime state (not saved) --
    /// Desired movement direction (world-space, set by input or AI each frame).
    #[serde(skip)]
    pub desired_velocity: [f32; 3],
    /// Whether sprint is active this frame.
    #[serde(skip)]
    pub is_sprinting: bool,
    /// Whether a jump was requested this frame.
    #[serde(skip)]
    pub jump_requested: bool,
    /// Whether the entity was on the ground last frame.
    #[serde(skip)]
    pub is_grounded: bool,
}

fn default_move_speed() -> f32 {
    50.0
}
fn default_sprint_mult() -> f32 {
    1.8
}
fn default_jump_impulse() -> f32 {
    5.0
}
fn default_ground_dist() -> f32 {
    1.05
}

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
///
/// Decoupled from movement — the player entity rotates by yaw,
/// the camera child rotates by pitch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LookController {
    /// Mouse / stick sensitivity.
    #[serde(default = "default_sensitivity")]
    pub mouse_sensitivity: f32,
    /// Minimum pitch angle in radians (looking down).
    #[serde(default = "default_pitch_min")]
    pub pitch_min: f32,
    /// Maximum pitch angle in radians (looking up).
    #[serde(default = "default_pitch_max")]
    pub pitch_max: f32,

    // -- runtime state (not saved) --
    /// Accumulated yaw (radians, around Z-up axis).
    #[serde(skip)]
    pub yaw: f32,
    /// Accumulated pitch (radians, clamped to [pitch_min, pitch_max]).
    #[serde(skip)]
    pub pitch: f32,
}

fn default_sensitivity() -> f32 {
    0.003
}
fn default_pitch_min() -> f32 {
    -89.0_f32.to_radians()
}
fn default_pitch_max() -> f32 {
    89.0_f32.to_radians()
}

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
///
/// References a mapping context (created in the editor) and maps action names
/// to gameplay functions. Like Unreal's `APlayerController` input setup:
/// - `mapping_context` → which `UInputMappingContext` to activate
/// - `move_action`, etc. → which `UInputAction` assets to bind
///
/// The `PlayerInputSystem` automatically pushes the mapping context to the
/// `InputSubsystem` when play starts. The action names are used to query
/// input values each frame.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerInput {
    /// Name of the mapping context to activate (must match an editor-created context).
    #[serde(default = "default_mapping_context")]
    pub mapping_context: String,
    /// Action name for 2D movement (Axis2D).
    #[serde(default = "default_move_action")]
    pub move_action: String,
    /// Action name for 2D look/aim (Axis2D).
    #[serde(default = "default_look_action")]
    pub look_action: String,
    /// Action name for jump (Digital).
    #[serde(default = "default_jump_action")]
    pub jump_action: String,
    /// Action name for sprint (Digital).
    #[serde(default = "default_sprint_action")]
    pub sprint_action: String,

    /// Whether the mapping context has been activated this play session.
    #[serde(skip)]
    pub context_active: bool,
}

fn default_mapping_context() -> String {
    "gameplay".to_string()
}
fn default_move_action() -> String {
    "move".to_string()
}
fn default_look_action() -> String {
    "look".to_string()
}
fn default_jump_action() -> String {
    "jump".to_string()
}
fn default_sprint_action() -> String {
    "sprint".to_string()
}

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

// ===========================================================================
// PlayerInputSystem
// ===========================================================================

/// Reads Enhanced Input actions via `PlayerInput` configuration and writes
/// intent to `CharacterMovement` + `LookController`.
///
/// On first run, activates the mapping context specified in `PlayerInput`.
/// Uses the configured action names (not hardcoded) to query input values.
///
/// AI-controlled characters don't need `PlayerInput` — they set
/// `CharacterMovement::desired_velocity` directly.
pub struct PlayerInputSystem;

impl System for PlayerInputSystem {
    fn run(&mut self, world: &mut hecs::World, resources: &mut Resources) {
        // First pass: activate mapping contexts for any PlayerInput that hasn't yet
        {
            let mut contexts_to_add: Vec<String> = Vec::new();
            for (_entity, pi) in world.query_mut::<&mut PlayerInput>() {
                if !pi.context_active {
                    contexts_to_add.push(pi.mapping_context.clone());
                    pi.context_active = true;
                }
            }
            if !contexts_to_add.is_empty() {
                if let Some(subsystem) = resources.get_mut::<InputSubsystem>() {
                    for ctx in &contexts_to_add {
                        if !subsystem.has_context(ctx) {
                            subsystem.add_context(ctx);
                        }
                    }
                }
            }
        }

        // Second pass: read input and write to movement/look components
        // Collect input data per PlayerInput first to avoid borrow conflicts
        struct InputData {
            move_xy: (f32, f32),
            look_xy: (f32, f32),
            jump: bool,
            sprint: bool,
        }

        let input_data = {
            // All PlayerInput entities share the same subsystem, so we can read once
            // per unique action name set. For simplicity (and since there's typically
            // one player), we read per entity.
            let Some(subsystem) = resources.get::<InputSubsystem>() else {
                return;
            };

            let mut data_map: Vec<(hecs::Entity, InputData)> = Vec::new();
            for (entity, pi) in world.query::<&PlayerInput>().iter() {
                data_map.push((
                    entity,
                    InputData {
                        move_xy: subsystem.axis_2d(&pi.move_action),
                        look_xy: subsystem.axis_2d(&pi.look_action),
                        jump: subsystem.just_pressed(&pi.jump_action),
                        sprint: subsystem.digital(&pi.sprint_action),
                    },
                ));
            }
            data_map
        };

        // Apply input to CharacterMovement + LookController
        for (entity, input) in &input_data {
            let Ok(mut cm) = world.get::<&mut CharacterMovement>(*entity) else {
                continue;
            };
            let Ok(mut look) = world.get::<&mut LookController>(*entity) else {
                continue;
            };

            let (move_x, move_y) = input.move_xy;
            let (look_x, look_y) = input.look_xy;

            // Update look angles
            look.yaw -= look_x * look.mouse_sensitivity;
            look.pitch -= look_y * look.mouse_sensitivity;
            look.pitch = look.pitch.clamp(look.pitch_min, look.pitch_max);

            // Calculate movement directions from yaw (Z-up: X=forward, Y=right)
            let forward = glm::vec3(look.yaw.cos(), look.yaw.sin(), 0.0);
            let right = glm::vec3(-look.yaw.sin(), look.yaw.cos(), 0.0);

            // Compute desired velocity in world space
            let has_input = move_x.abs() > 0.01 || move_y.abs() > 0.01;
            if has_input {
                let desired = forward * move_y + right * move_x;
                cm.desired_velocity = [desired.x, desired.y, desired.z];
            } else {
                cm.desired_velocity = [0.0; 3];
            }

            // For flying mode, use look direction for vertical component
            if cm.movement_mode == MovementMode::Flying && has_input {
                let look_forward = glm::vec3(
                    look.yaw.cos() * look.pitch.cos(),
                    look.yaw.sin() * look.pitch.cos(),
                    look.pitch.sin(),
                );
                let desired = look_forward * move_y + right * move_x;
                cm.desired_velocity = [desired.x, desired.y, desired.z];
            }

            cm.jump_requested = input.jump;
            cm.is_sprinting = input.sprint;
        }
    }

    fn name(&self) -> &str {
        "PlayerInputSystem"
    }
}

impl PlayerInputSystem {
    pub fn descriptor() -> SystemDescriptor {
        SystemDescriptor::new("PlayerInputSystem")
            .reads_resource::<InputSubsystem>()
            .writes_resource::<InputSubsystem>()
            .writes::<PlayerInput>()
            .writes::<CharacterMovement>()
            .writes::<LookController>()
            .reads::<Transform>()
            .reads::<Player>()
            .after("PhysicsStepSystem")
    }
}

// ===========================================================================
// CharacterMovementSystem
// ===========================================================================

/// Executes movement intent via physics forces.
///
/// For each entity with `CharacterMovement`:
/// 1. Ground detection (raycast downward)
/// 2. Apply horizontal movement force from `desired_velocity`
/// 3. Apply jump impulse if requested and grounded
/// 4. Apply yaw rotation from `LookController` (if present)
/// 5. Apply pitch rotation to Camera child (if present)
/// 6. Clear transient state (`desired_velocity`, `jump_requested`)
pub struct CharacterMovementSystem;

impl System for CharacterMovementSystem {
    fn run(&mut self, world: &mut hecs::World, resources: &mut Resources) {
        // Collect movement data first (avoids borrow conflicts with physics)
        struct MoveData {
            entity: hecs::Entity,
            handle: rapier3d::dynamics::RigidBodyHandle,
            desired_velocity: glm::Vec3,
            move_speed: f32,
            is_sprinting: bool,
            sprint_multiplier: f32,
            jump_requested: bool,
            jump_impulse: f32,
            position: glm::Vec3,
            ground_check_dist: f32,
            movement_mode: MovementMode,
            // Pitch from LookController (if present) — applied to camera child
            pitch: Option<f32>,
            // Camera child entities
            camera_children: Vec<hecs::Entity>,
        }

        let mut moves: Vec<MoveData> = Vec::new();

        for (entity, (transform, rb, cm, look, children)) in world.query_mut::<(
            &mut Transform,
            &RigidBody,
            &mut CharacterMovement,
            Option<&LookController>,
            Option<&Children>,
        )>() {
            let Some(handle) = rb.handle else {
                continue;
            };

            // Apply yaw rotation to player entity
            if let Some(look) = look {
                transform.rotation =
                    glm::quat_angle_axis(look.yaw, &glm::vec3(0.0, 0.0, 1.0));
            }

            // Find camera children
            let camera_children = children
                .map(|c| c.0.clone())
                .unwrap_or_default();

            moves.push(MoveData {
                entity,
                handle,
                desired_velocity: glm::vec3(
                    cm.desired_velocity[0],
                    cm.desired_velocity[1],
                    cm.desired_velocity[2],
                ),
                move_speed: cm.move_speed,
                is_sprinting: cm.is_sprinting,
                sprint_multiplier: cm.sprint_multiplier,
                jump_requested: cm.jump_requested,
                jump_impulse: cm.jump_impulse,
                position: transform.position,
                ground_check_dist: cm.ground_check_dist,
                movement_mode: cm.movement_mode,
                pitch: look.map(|l| l.pitch),
                camera_children,
            });

            // Clear transient state
            cm.desired_velocity = [0.0; 3];
            cm.jump_requested = false;
        }

        // Apply physics and ground detection
        let mut grounding_updates: Vec<(hecs::Entity, bool)> = Vec::new();
        let mut dirty_entities: Vec<hecs::Entity> = Vec::new();

        for mv in &moves {
            // Ground detection: raycast downward
            let is_grounded = if mv.movement_mode == MovementMode::Walking {
                resources
                    .get::<PhysicsWorld>()
                    .and_then(|physics| {
                        physics.raycast(
                            mv.position,
                            glm::vec3(0.0, 0.0, -1.0),
                            mv.ground_check_dist,
                        )
                    })
                    .is_some()
            } else {
                false // flying mode: never grounded
            };
            grounding_updates.push((mv.entity, is_grounded));

            // Apply movement force
            let has_input = mv.desired_velocity.magnitude_squared() > 0.0001;
            if has_input {
                let mut speed = mv.move_speed;
                if mv.is_sprinting {
                    speed *= mv.sprint_multiplier;
                }
                let force = velocity_to_physics(&(mv.desired_velocity * speed));
                if let Some(physics) = resources.get_mut::<PhysicsWorld>() {
                    physics.apply_force(mv.handle, force);
                }
            }

            // Jump
            if mv.jump_requested && is_grounded {
                let impulse = velocity_to_physics(&glm::vec3(0.0, 0.0, mv.jump_impulse));
                if let Some(physics) = resources.get_mut::<PhysicsWorld>() {
                    physics.apply_impulse(mv.handle, impulse);
                }
            }

            dirty_entities.push(mv.entity);

            // Apply pitch to camera children.
            // The camera is a child of the player entity, so it inherits the
            // player's yaw rotation through the hierarchy. The camera's local
            // rotation is pure pitch around the local right axis (Y in Z-up).
            if let Some(pitch) = mv.pitch {
                for &child in &mv.camera_children {
                    if world.get::<&Camera>(child).is_ok() {
                        if let Ok(mut cam_transform) = world.get::<&mut Transform>(child) {
                            cam_transform.rotation =
                                glm::quat_angle_axis(pitch, &glm::vec3(0.0, 1.0, 0.0));
                        }
                        dirty_entities.push(child);
                    }
                }
            }
        }

        // Write back grounding state
        for (entity, is_grounded) in grounding_updates {
            if let Ok(mut cm) = world.get::<&mut CharacterMovement>(entity) {
                cm.is_grounded = is_grounded;
            }
        }

        // Mark transforms dirty for hierarchy propagation
        for entity in dirty_entities {
            let _ = world.insert_one(entity, TransformDirty);
        }
    }

    fn name(&self) -> &str {
        "CharacterMovementSystem"
    }
}

impl CharacterMovementSystem {
    pub fn descriptor() -> SystemDescriptor {
        SystemDescriptor::new("CharacterMovementSystem")
            .reads_resource::<Time>()
            .writes_resource::<PhysicsWorld>()
            .writes::<Transform>()
            .writes::<CharacterMovement>()
            .reads::<LookController>()
            .reads::<RigidBody>()
            .reads::<Camera>()
            .reads::<Children>()
            .after("PlayerInputSystem")
    }
}

// ===========================================================================
// Tests
// ===========================================================================

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
    fn character_movement_serde_roundtrip() {
        let cm = CharacterMovement {
            move_speed: 12.0,
            sprint_multiplier: 2.5,
            jump_impulse: 8.0,
            ground_check_dist: 0.2,
            movement_mode: MovementMode::Flying,
            desired_velocity: [1.0, 2.0, 3.0], // runtime, should be skipped
            is_sprinting: true,                  // runtime, should be skipped
            jump_requested: true,                // runtime, should be skipped
            is_grounded: true,                   // runtime, should be skipped
        };
        let ron = ron::to_string(&cm).unwrap();
        let deserialized: CharacterMovement = ron::from_str(&ron).unwrap();
        assert_eq!(deserialized.move_speed, 12.0);
        assert_eq!(deserialized.sprint_multiplier, 2.5);
        assert_eq!(deserialized.movement_mode, MovementMode::Flying);
        assert_eq!(deserialized.desired_velocity, [0.0; 3]); // runtime state reset
        assert!(!deserialized.is_sprinting);
        assert!(!deserialized.is_grounded);
    }

    #[test]
    fn look_controller_serde_roundtrip() {
        let look = LookController {
            mouse_sensitivity: 0.005,
            pitch_min: -80.0_f32.to_radians(),
            pitch_max: 80.0_f32.to_radians(),
            yaw: 1.5,   // runtime, should be skipped
            pitch: 0.5, // runtime, should be skipped
        };
        let ron = ron::to_string(&look).unwrap();
        let deserialized: LookController = ron::from_str(&ron).unwrap();
        assert_eq!(deserialized.mouse_sensitivity, 0.005);
        assert_eq!(deserialized.yaw, 0.0); // runtime state reset
        assert_eq!(deserialized.pitch, 0.0);
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
        assert_eq!(pi.look_action, "look");
        assert_eq!(pi.jump_action, "jump");
        assert_eq!(pi.sprint_action, "sprint");
        assert!(!pi.context_active);
    }

    #[test]
    fn player_input_serde_roundtrip() {
        let pi = PlayerInput {
            mapping_context: "vehicle".to_string(),
            move_action: "drive".to_string(),
            look_action: "camera".to_string(),
            jump_action: "boost".to_string(),
            sprint_action: "nitro".to_string(),
            context_active: true, // runtime, should be skipped
        };
        let ron = ron::to_string(&pi).unwrap();
        let deserialized: PlayerInput = ron::from_str(&ron).unwrap();
        assert_eq!(deserialized.mapping_context, "vehicle");
        assert_eq!(deserialized.move_action, "drive");
        assert_eq!(deserialized.look_action, "camera");
        assert!(!deserialized.context_active); // runtime state reset
    }
}
