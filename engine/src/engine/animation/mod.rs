//! Skeletal animation system.
//!
//! Provides ECS components (`SkeletonInstance`, `AnimationPlayer`) for
//! runtime skeletal animation, keyframe sampling with lerp/slerp, and
//! forward kinematics to compute world-space bone palettes for GPU skinning.

pub mod components;
#[cfg(debug_assertions)]
pub mod debug_draw;
pub mod sampling;
pub mod system;

pub use components::{AnimationPlayer, CrossfadeState, PlaybackState, SkeletonInstance};
pub use system::AnimationUpdateSystem;
