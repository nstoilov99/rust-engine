//! Engine-owned system name constants for cross-crate ordering.
//!
//! Game code should use these constants in `after()` calls instead of
//! raw strings to avoid silent ordering breakage on renames.

/// Enhanced input system — processes raw input into action values.
pub const ENHANCED_INPUT: &str = "EnhancedInputSystem";

/// Animation update system — advances skeletal animation playback.
pub const ANIMATION_UPDATE: &str = "AnimationUpdateSystem";

/// Physics step system — steps the Rapier physics simulation.
pub const PHYSICS_STEP: &str = "PhysicsStepSystem";

/// Transform propagation system — propagates hierarchy transforms.
pub const TRANSFORM_PROPAGATION: &str = "TransformPropagationSystem";

/// Input action system (legacy) — maps keys to action states.
pub const INPUT_ACTION: &str = "InputActionSystem";

/// Audio system — processes spatial audio and playback.
pub const AUDIO: &str = "AudioSystem";
