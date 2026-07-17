//! Static-collision cooking (offline) — see
//! `docs/roadmap/VULKANO-M2-COLLISION-PIPELINE.md`.
//!
//! Runtime loading/querying lives in `game_shared::collision`; this module
//! only produces cooked `.ccol` chunks from a scene.

pub mod cook;
