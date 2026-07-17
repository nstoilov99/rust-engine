//! Backend-neutral networking seam (M5 Net-A, ADR-014).
//!
//! Pure data and pure logic only — no socket, no SpacetimeDB types. The
//! client backend (`game_client_net`) and the server module both depend on
//! this; a renet/QUIC fallback backend would implement the same trait.
//!
//! Identity semantics are defined by `docs/roadmap/M5-WORLD-IDENTITY-CONTRACT.md`.

pub mod protocol;
pub mod schema;
pub mod traits;

pub use protocol::*;
pub use schema::*;
pub use traits::*;
