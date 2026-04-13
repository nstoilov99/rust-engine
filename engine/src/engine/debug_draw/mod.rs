//! Debug draw system — immediate-mode wireframe line rendering.
//!
//! Provides a `DebugDrawBuffer` API for submitting debug lines in Z-up game
//! space, and a `DebugDrawPass` GPU pipeline that renders them as colored
//! line-list primitives in the deferred renderer.
//!
//! # Architecture
//!
//! - **API** (`DebugDrawBuffer`): immediate-mode line submission, gated with
//!   `#[cfg(debug_assertions)]` at the call site.
//! - **Primitives**: pure functions generating wireframe shapes (box, sphere, etc.)
//! - **Renderer** (`DebugDrawPass`, `DebugDrawData`): Vulkan pipelines and GPU
//!   data structures, available in all builds.

pub mod api;
pub mod primitives;
pub mod renderer;

pub use api::DebugDrawBuffer;
pub use primitives::DebugLineData;
pub use renderer::{DebugDrawData, DebugDrawPass, DebugLinePushConstants, DebugLineVertex};
