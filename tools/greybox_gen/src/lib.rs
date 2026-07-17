//! Greybox world generator (M3): deterministic offline content compiler.
//!
//! Emits ordinary engine content — per-cell RMSH terrain meshes, one scene,
//! one world manifest — for the multiplayer-foundation test world. The
//! runtime knows nothing about "greybox"; identical inputs produce
//! byte-identical outputs (see `height` for the determinism rules).

pub mod gym;
pub mod height;
pub mod outputs;
pub mod params;
pub mod scene;
pub mod terrain;
