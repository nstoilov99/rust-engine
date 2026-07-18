//! The `NetClient` seam (ADR-014): everything the game simulates or renders
//! from the network arrives as `NetEvent`s from a `NetClient` implementation.

use super::protocol::{ClientInput, ClockSample, EntityState, ModuleAddr, WorldSnapshot};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Offline,
    Connecting,
    /// Connected; waiting for the permanent subscription to apply.
    AwaitBaseSub,
    /// Base subscription applied; checking `config.protocol_version`.
    VersionCheck,
    /// Version accepted; `enter_world` sent, waiting for the own player row.
    EnteringWorld,
    /// Own player row visible; replication live.
    InWorld,
    Disconnected,
    Reconnecting,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisconnectReason {
    /// Server/client protocol mismatch — clean refusal, no retry (contract).
    VersionMismatch { server: u32, client: u32 },
    /// Another connection of the same identity took the session (§4.3).
    SessionReplaced,
    /// Transport-level loss (socket death, server gone).
    ConnectionLost(String),
    /// Local request.
    UserRequested,
}

/// Events drained once per frame on the main thread. Spawn/despawn decisions
/// are made by the cache-diff replication system, not by the backend; the
/// backend only reports connection lifecycle, samples, acks and clock data.
#[derive(Debug, Clone, PartialEq)]
pub enum NetEvent {
    Connected,
    Disconnected(DisconnectReason),
    /// Replicated rows changed; full current view for the cache-diff (D3).
    Snapshot(WorldSnapshot),
    /// Tombstone evidence observed (contract §3.2). Fed from row callbacks,
    /// so destruction + GC arriving in one pump cannot erase the evidence.
    TombstoneSeen { entity_id: u64, generation: u32 },
    /// A fresh state sample for a live entity (feeds interpolation buffers).
    StateUpdate(EntityState),
    /// Own-row ack advanced (contract §5: replication of the row IS the ack).
    InputAck { epoch: u32, seq: u32 },
    ClockSample(ClockSample),
}

/// Backend-neutral client. The SpacetimeDB implementation lives in
/// `game_client_net`; a renet/QUIC fallback would implement the same trait.
///
/// Implementations MUST rate-limit outgoing reducer calls internally (spike
/// binding requirement 3) and MUST NOT panic on a dead connection.
pub trait NetClient {
    fn connect(&mut self, addr: &ModuleAddr);
    fn disconnect(&mut self);
    fn connection_state(&self) -> ConnectionState;
    /// Send (or coalesce toward the next send slot) one input sample.
    /// Silently dropped unless `InWorld`.
    fn send_input(&mut self, input: &ClientInput);
    /// Pump the connection and drain pending events into `out`.
    /// Called exactly once per frame on the main thread.
    fn poll(&mut self, out: &mut Vec<NetEvent>);
}
