//! SpacetimeDB backend for the `game_shared::net::NetClient` seam (ADR-014).
//!
//! Main-thread pumped: `poll()` calls the SDK's `frame_tick()` once per
//! frame, so all callbacks and cache reads happen on the game thread — no
//! cross-thread channels, no locks around game state (plan D3).
//!
//! Session/identity semantics: `docs/roadmap/M5-WORLD-IDENTITY-CONTRACT.md`.

#[allow(clippy::all)]
pub mod module_bindings;

mod client;

pub use client::SpacetimeNetClient;
