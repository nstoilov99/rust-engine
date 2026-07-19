//! Protocol schema constants and pure acceptance rules.
//!
//! Single source of truth for client and server module; bump
//! `PROTOCOL_VERSION` manually and deliberately on any wire/table change.

use uuid::Uuid;

/// v4: M7 combat schema (hp/mana/alive columns, cooldown/cast tables).
pub const PROTOCOL_VERSION: u32 = 4;

/// M5 runs a single module instance; the realm id exists so sharded modules
/// (M8+) can never collide in identity-keyed structures.
pub const REALM_ID: u32 = 0;

/// Fixed client input send rate (inputs are coalesced, never per-frame).
pub const INPUT_SEND_HZ: u32 = 20;

/// Shared walk speed, consumed by `MotionConfig::default()` on both sides
/// (M6: the server is authoritative; the client predicts with the same
/// controller).
pub const PLAYER_SPEED_MPS: f32 = 4.0;
pub const SPRINT_MULTIPLIER: f32 = 1.6;

/// Tombstones older than this are GC'd server-side; clients prune their
/// local tombstone evidence on the same horizon.
pub const TOMBSTONE_TTL_SECS: u64 = 300;

/// UUIDv5 namespace for GUIDs derived from network identity (contract §2.2).
pub const NET_PROXY_NS: Uuid = Uuid::from_u128(0x7d9c4b62_3f1a_4e08_9b5d_2c8a61f0e4d3);

/// Version handshake: exact match only (contract: mismatch = clean refusal).
pub fn version_compatible(server: u32, client: u32) -> bool {
    server == client
}

/// Server-side input acceptance (contract §5): epoch must match, sequence
/// strictly increasing; gaps legal, everything else silently dropped.
pub fn accept_input(row_epoch: u32, row_last_seq: u32, in_epoch: u32, in_seq: u32) -> bool {
    in_epoch == row_epoch && in_seq > row_last_seq
}

/// Derived `EntityGuid` for a replicated proxy (contract §2.2): UUIDv5 over
/// exactly 16 bytes `realm_id LE ‖ entity_id LE ‖ generation LE`.
pub fn derive_proxy_guid(realm_id: u32, entity_id: u64, generation: u32) -> Uuid {
    let mut name = [0u8; 16];
    name[0..4].copy_from_slice(&realm_id.to_le_bytes());
    name[4..12].copy_from_slice(&entity_id.to_le_bytes());
    name[12..16].copy_from_slice(&generation.to_le_bytes());
    Uuid::new_v5(&NET_PROXY_NS, &name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_exact_match_only() {
        assert!(version_compatible(1, 1));
        assert!(!version_compatible(1, 2));
        assert!(!version_compatible(2, 1));
    }

    #[test]
    fn input_acceptance_rules() {
        // fresh session: epoch 3, nothing accepted yet
        assert!(accept_input(3, 0, 3, 1)); // first input
        assert!(accept_input(3, 5, 3, 6)); // increasing
        assert!(accept_input(3, 5, 3, 9)); // gap is legal
        assert!(!accept_input(3, 5, 3, 5)); // duplicate
        assert!(!accept_input(3, 5, 3, 4)); // stale
        assert!(!accept_input(3, 5, 2, 6)); // old epoch
        assert!(!accept_input(3, 5, 4, 1)); // future epoch (client desync)
    }

    #[test]
    fn proxy_guid_deterministic_and_distinct() {
        let a = derive_proxy_guid(0, 42, 1);
        assert_eq!(a, derive_proxy_guid(0, 42, 1));
        assert_ne!(a, derive_proxy_guid(0, 42, 2)); // generation matters
        assert_ne!(a, derive_proxy_guid(0, 43, 1)); // entity matters
        assert_ne!(a, derive_proxy_guid(1, 42, 1)); // realm matters
        assert_eq!(a.get_version_num(), 5);
    }
}
