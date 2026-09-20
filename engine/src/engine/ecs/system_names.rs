//! Engine-owned system name constants for cross-crate ordering.
//!
//! Game code should use these constants in `after()` calls instead of
//! raw strings to avoid silent ordering breakage on renames.

/// Enhanced input system — processes raw input into action values.
pub const ENHANCED_INPUT: &str = "EnhancedInputSystem";

/// Animation update system — advances skeletal animation playback.
pub const ANIMATION_UPDATE: &str = "AnimationUpdateSystem";

/// Foot placement system — ground raycasts into IK targets, runs
/// immediately before the animation graph system.
pub const FOOT_PLACEMENT: &str = "FootPlacementSystem";

/// Animation graph system — ticks `.animgraph` state machines into skeletons.
pub const ANIM_GRAPH: &str = "AnimGraphSystem";

/// Physics step system — steps the Rapier physics simulation.
pub const PHYSICS_STEP: &str = "PhysicsStepSystem";

/// Transform propagation system — propagates hierarchy transforms.
pub const TRANSFORM_PROPAGATION: &str = "TransformPropagationSystem";

/// Input action system (legacy) — maps keys to action states.
pub const INPUT_ACTION: &str = "InputActionSystem";

/// Audio system — processes spatial audio and playback.
pub const AUDIO: &str = "AudioSystem";

// --- game_client gameplay systems (Task 41.6). The engine never registers
// these; the constants exist so engine plugins and the game order against
// one spelling.

/// Player input system — Enhanced Input → `CharacterMovement` intent
/// (PreUpdate; stages sequence it after `ENHANCED_INPUT`).
pub const PLAYER_INPUT: &str = "PlayerInputSystem";

/// Character movement system — velocity-set capsule controller, runs
/// before the anim stack and `PHYSICS_STEP`.
pub const CHARACTER_MOVEMENT: &str = "CharacterMovementSystem";

/// Offline anim bridge — `CharacterMovement` state → anim graph params,
/// between `CHARACTER_MOVEMENT` and `FOOT_PLACEMENT`.
pub const CHARACTER_ANIM_BRIDGE: &str = "CharacterAnimBridgeSystem";

/// Orbit camera system — writes the Camera entity's transform in Update,
/// before `TRANSFORM_PROPAGATION`.
pub const ORBIT_CAMERA: &str = "OrbitCameraSystem";
