//! Coordinate system adapters for boundary conversion
//!
//! This module provides centralized coordinate conversion between:
//! - **Game/ECS space**: Z-up (X=forward, Y=right, Z=up)
//! - **Render space**: Y-up (X=right, Y=up, Z=forward) - Vulkan/OpenGL
//! - **Physics space**: Y-up (X=right, Y=up, Z=forward) - Rapier
//!
//! All conversion logic is centralized here to avoid scattered conversions
//! throughout the codebase and provide a single source of truth.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────┐
//! │         GAME CODE (Z-up)                    │
//! │    Transform { position, rotation, scale }  │
//! └─────────────────────────────────────────────┘
//!                        │
//!        ┌───────────────┴───────────────┐
//!        ▼                               ▼
//! ┌──────────────────┐         ┌──────────────────┐
//! │  render_adapter  │         │  physics_adapter │
//! │  to_model_matrix │         │  to_physics_pos  │
//! └──────────────────┘         └──────────────────┘
//!        │                               │
//!        ▼                               ▼
//! ┌──────────────────┐         ┌──────────────────┐
//! │  VULKAN (Y-up)   │         │  RAPIER (Y-up)   │
//! └──────────────────┘         └──────────────────┘
//! ```

pub mod physics_adapter;
pub mod render_adapter;

pub use physics_adapter::*;
pub use render_adapter::*;
