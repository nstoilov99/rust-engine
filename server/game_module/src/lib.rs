//! Game server module (M5 Net-A).
//!
//! Identity, session and lifecycle semantics are normative in
//! `docs/roadmap/M5-WORLD-IDENTITY-CONTRACT.md`; protocol constants come
//! from `game_shared::net::schema` (single source of truth).
//!
//! These tables ARE the save system — player rows are never deleted on
//! disconnect.

use game_shared::net::schema::{PROTOCOL_VERSION, REALM_ID, TOMBSTONE_TTL_SECS};
use spacetimedb::{reducer, table, ConnectionId, Identity, ReducerContext, ScheduleAt, Table};
use std::time::Duration;

const TICK_INTERVAL_MS: u64 = 250;

// ---------------------------------------------------------------------------
// Tables
// ---------------------------------------------------------------------------

/// Singleton; in every client's permanent subscription. The version
/// handshake reads it at `on_applied` (plan D3).
#[table(accessor = config, public)]
pub struct Config {
    #[primary_key]
    id: u32, // always 0
    protocol_version: u32,
    realm_id: u32,
}

/// Singleton ID source (contract §1.2): one monotonic namespace for all
/// replicated kinds; a separate sequence for character ids. Never resets.
#[table(accessor = entity_allocator)]
pub struct EntityAllocator {
    #[primary_key]
    id: u32, // always 0
    next_entity_id: u64,
    next_character_id: u64,
}

#[table(accessor = account, public)]
pub struct Account {
    #[primary_key]
    identity: Identity,
    #[unique]
    character_id: u64,
    name: String,
    created_at_micros: i64,
}

/// `#[unique]` on `owner_identity` and `character_id` make a duplicate
/// player row structurally impossible (contract §4.5).
#[table(accessor = player, public)]
pub struct Player {
    #[primary_key]
    entity_id: u64,
    generation: u32,
    #[unique]
    owner_identity: Identity,
    #[unique]
    character_id: u64,
    /// Active session; identity+session auth per contract §4.4.
    session: Option<ConnectionId>,
    x: f32,
    y: f32,
    z: f32,
    vx: f32,
    vy: f32,
    vz: f32,
    yaw: f32,
    #[index(btree)]
    zone_id: u32,
    epoch: u32,
    last_input_seq: u32,
    /// Authoritative timestamp of the last state write (interpolation base).
    last_update_micros: i64,
}

#[table(accessor = npc, public)]
pub struct Npc {
    #[primary_key]
    entity_id: u64,
    generation: u32,
    x: f32,
    y: f32,
    z: f32,
    yaw: f32,
    #[index(btree)]
    zone_id: u32,
    kind: u32,
    last_update_micros: i64,
}

/// Destruction evidence (contract §3): upserted on despawn, GC'd after
/// `TOMBSTONE_TTL_SECS`. In every client's permanent subscription.
#[table(accessor = tombstone, public)]
pub struct Tombstone {
    #[primary_key]
    entity_id: u64,
    generation: u32,
    despawned_at_micros: i64,
}

/// Coarse server time base, updated by the scheduled tick (plan D5).
#[table(accessor = clock, public)]
pub struct Clock {
    #[primary_key]
    id: u32, // always 0
    tick: u64,
    server_time_micros: i64,
}

/// Per-identity ping response (plan D5): client correlates by nonce to
/// complete an NTP-style offset sample.
#[table(accessor = ping_result, public)]
pub struct PingResult {
    #[primary_key]
    identity: Identity,
    nonce: u64,
    server_time_micros: i64,
}

#[table(accessor = tick_timer, scheduled(tick))]
pub struct TickTimer {
    #[primary_key]
    #[auto_inc]
    scheduled_id: u64,
    scheduled_at: ScheduleAt,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn now_micros(ctx: &ReducerContext) -> i64 {
    ctx.timestamp.to_micros_since_unix_epoch()
}

fn alloc_character_id(ctx: &ReducerContext) -> u64 {
    let mut a = ctx
        .db
        .entity_allocator()
        .id()
        .find(0)
        .expect("allocator row must exist after init");
    let id = a.next_character_id;
    a.next_character_id += 1;
    ctx.db.entity_allocator().id().update(a);
    id
}

fn short_name(identity: &Identity) -> String {
    let hex = identity.to_hex();
    format!("Player-{}", &hex.as_str()[..8.min(hex.as_str().len())])
}

// ---------------------------------------------------------------------------
// Lifecycle reducers
// ---------------------------------------------------------------------------

#[reducer(init)]
pub fn init(ctx: &ReducerContext) {
    ctx.db.config().insert(Config {
        id: 0,
        protocol_version: PROTOCOL_VERSION,
        realm_id: REALM_ID,
    });
    ctx.db.entity_allocator().insert(EntityAllocator {
        id: 0,
        next_entity_id: 1,
        next_character_id: 1,
    });
    ctx.db.clock().insert(Clock {
        id: 0,
        tick: 0,
        server_time_micros: now_micros(ctx),
    });
    ctx.db.tick_timer().insert(TickTimer {
        scheduled_id: 0,
        scheduled_at: ScheduleAt::Interval(Duration::from_millis(TICK_INTERVAL_MS).into()),
    });
    log::info!(
        "game_module initialized: protocol v{PROTOCOL_VERSION}, realm {REALM_ID}, tick {TICK_INTERVAL_MS} ms"
    );
}

/// Creates the account on first sight (contract §4.2). Does NOT create the
/// player row or install a session — that is `enter_world` (Package 3).
#[reducer(client_connected)]
pub fn client_connected(ctx: &ReducerContext) {
    if ctx.db.account().identity().find(ctx.sender()).is_none() {
        let character_id = alloc_character_id(ctx);
        ctx.db.account().insert(Account {
            identity: ctx.sender(),
            character_id,
            name: short_name(&ctx.sender()),
            created_at_micros: now_micros(ctx),
        });
    }
}

/// Contract §4.3: clears the session ONLY if the disconnecting connection
/// is the active one — a revoked stale connection's late disconnect must
/// not knock the live session offline. The player row persists.
#[reducer(client_disconnected)]
pub fn client_disconnected(ctx: &ReducerContext) {
    if let Some(mut p) = ctx.db.player().owner_identity().find(ctx.sender()) {
        if p.session.is_some() && p.session == ctx.connection_id() {
            p.session = None;
            ctx.db.player().entity_id().update(p);
        }
    }
}

// ---------------------------------------------------------------------------
// Scheduled tick
// ---------------------------------------------------------------------------

#[reducer]
pub fn tick(ctx: &ReducerContext, _timer: TickTimer) -> Result<(), String> {
    if ctx.sender() != ctx.database_identity() {
        return Err("tick may only be invoked by the scheduler".into());
    }
    let now = now_micros(ctx);

    let mut clock = ctx.db.clock().id().find(0).ok_or("clock row missing")?;
    clock.tick += 1;
    clock.server_time_micros = now;
    ctx.db.clock().id().update(clock);

    // Tombstone GC (contract §3.1).
    let horizon = now - (TOMBSTONE_TTL_SECS as i64) * 1_000_000;
    let expired: Vec<u64> = ctx
        .db
        .tombstone()
        .iter()
        .filter(|t| t.despawned_at_micros < horizon)
        .map(|t| t.entity_id)
        .collect();
    for id in expired {
        ctx.db.tombstone().entity_id().delete(id);
    }

    // NPC wander lands in Package 3.
    Ok(())
}
