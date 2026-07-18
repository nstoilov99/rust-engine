//! Wire/row shapes as plain Rust types (backend-neutral).
//!
//! All positions are Z-up world space in meters, matching `world_grid`.
//! Server table rows mirror these field-for-field inside the module crate
//! (rows must derive SpacetimeDB traits; this crate stays backend-free).

use serde::{Deserialize, Serialize};

/// Network identity of a replicated entity (contract §1.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NetId {
    pub realm_id: u32,
    pub entity_id: u64,
    pub generation: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntityKind {
    Player,
    Npc,
}

/// One replicated state sample. `server_time_us` is the authoritative
/// timestamp stamped by the reducer that produced it — the interpolation
/// time base (plan D5/D6).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EntityState {
    pub entity_id: u64,
    pub generation: u32,
    pub kind: EntityKind,
    pub pos: [f32; 3],
    pub vel: [f32; 3],
    pub yaw: f32,
    pub zone_id: u32,
    pub server_time_us: u64,
}

/// Complete view of the replicated entity rows currently visible to the
/// client — the cache-diff input (plan D3). Emitted by the backend on dirty
/// frames only. `own_entity_id` marks the local player's row so replication
/// binds it to the local entity instead of proxying it (contract §2.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorldSnapshot {
    pub entities: Vec<EntityState>,
    pub own_entity_id: Option<u64>,
}

/// Client input sample, session-scoped (contract §5). Coalesced client-side
/// and sent at `INPUT_SEND_HZ`, never per-frame.
///
/// M5 is trust-the-client: `pos` is the client's integrated position and the
/// server accepts it after sanity checks (finite, per-input distance cap).
/// M6 replaces this with server-authoritative movement + reconciliation.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ClientInput {
    /// Stamped by the `NetClient` implementation at send time (it owns the
    /// authoritative epoch from the own player row); caller values ignored.
    pub epoch: u32,
    /// Stamped by the `NetClient` implementation at send time.
    pub seq: u32,
    pub pos: [f32; 3],
    /// Desired movement direction on the XY ground plane (Z-up), unnormalized
    /// is legal; the server normalizes and rejects non-finite input.
    pub move_dir: [f32; 2],
    pub yaw: f32,
    pub sprint: bool,
    pub jump: bool,
}

/// Completed clock sync sample (plan D5): the backend finishes the NTP-style
/// ping round trip and reports the raw measurement; filtering (outlier
/// rejection, EWMA) is game-side policy in `NetClock`.
///
/// `offset_us` is defined so that
/// `estimated_server_time_us = local_time_us() + offset_us`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClockSample {
    pub offset_us: i64,
    pub rtt_us: u64,
}

/// Address of one module instance. Part of the seam so zone→module routing
/// later is a lookup, not a client rewrite (spike binding requirement 2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleAddr {
    /// e.g. "http://127.0.0.1:3000"
    pub host: String,
    /// Database/module name, e.g. "rust-engine-dev"
    pub module: String,
}
