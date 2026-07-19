//! Game server module (M5 Net-A).
//!
//! Identity, session and lifecycle semantics are normative in
//! `docs/roadmap/M5-WORLD-IDENTITY-CONTRACT.md`; protocol constants come
//! from `game_shared::net::schema` (single source of truth).
//!
//! These tables ARE the save system — player rows are never deleted on
//! disconnect.

use game_shared::collision::{manifest_hash, ChunkStore};
use game_shared::net::schema::{
    accept_input, MAX_INPUT_STEP_M, PROTOCOL_VERSION, REALM_ID, TOMBSTONE_TTL_SECS,
};
use game_shared::world_grid::zone_id_from_position;
use spacetimedb::{reducer, table, ConnectionId, Identity, ReducerContext, ScheduleAt, Table};
use std::sync::OnceLock;
use std::time::Duration;

/// Cooked greybox collision embedded at build time (M6 D1, `build.rs`).
mod collision_registry {
    include!(concat!(env!("OUT_DIR"), "/collision_registry.rs"));
}

/// World collision, built lazily on first use and kept for the lifetime of
/// the module instance. `None` = embedded data failed validation; callers
/// must skip simulation (with a log) instead of panicking every transaction.
static COLLISION: OnceLock<Option<ChunkStore>> = OnceLock::new();

fn collision_store() -> Option<&'static ChunkStore> {
    COLLISION
        .get_or_init(|| {
            let mut store = ChunkStore::new();
            for bytes in collision_registry::COLLISION_CHUNKS {
                if let Err(e) = store.insert_chunk(bytes) {
                    log::error!("embedded collision chunk rejected: {e}");
                    return None;
                }
            }
            log::info!(
                "collision store built: {} chunks, {} embedded bytes",
                store.len(),
                collision_registry::COLLISION_CHUNKS
                    .iter()
                    .map(|c| c.len())
                    .sum::<usize>()
            );
            Some(store)
        })
        .as_ref()
}

/// 10 Hz: NPC sample spacing must stay well under the client's ~150 ms
/// interpolation delay or remote motion stalls between samples.
const TICK_INTERVAL_MS: u64 = 100;

/// Dev spawn point (Z-up, meters). The world manifest is runtime-loaded RON
/// the WASM module cannot read; a manifest-driven spawn is deferred.
const SPAWN_POINT: [f32; 3] = [0.0, 0.0, 1.0];
const NPC_SEED_COUNT: u32 = 4;
const NPC_WANDER_RADIUS: f32 = 8.0;
const NPC_SPEED_MPS: f32 = 1.5;

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
    /// FNV-1a 64 of the embedded collision manifest (M6 D1). Clients compare
    /// against their local manifest at connect; mismatch = refuse, since the
    /// two sides would simulate against different geometry.
    collision_manifest_hash: u64,
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

fn alloc_entity_id(ctx: &ReducerContext) -> u64 {
    let mut a = ctx
        .db
        .entity_allocator()
        .id()
        .find(0)
        .expect("allocator row must exist after init");
    let id = a.next_entity_id;
    a.next_entity_id += 1;
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
        collision_manifest_hash: manifest_hash(collision_registry::COLLISION_MANIFEST),
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
// World entry & despawn
// ---------------------------------------------------------------------------

/// Contract §4.5: creates or resumes the caller's player and installs the
/// calling connection as the active session (last-wins revocation, §4.3).
/// Idempotent from the live connection: no epoch bump, no state change.
#[reducer]
pub fn enter_world(ctx: &ReducerContext) -> Result<(), String> {
    let conn = ctx
        .connection_id()
        .ok_or("enter_world requires a client connection")?;
    let account = ctx
        .db
        .account()
        .identity()
        .find(ctx.sender())
        .ok_or("no account for caller")?;

    match ctx.db.player().owner_identity().find(ctx.sender()) {
        Some(mut p) => {
            if p.session == Some(conn) {
                return Ok(());
            }
            p.session = Some(conn);
            p.epoch += 1;
            p.last_input_seq = 0;
            p.last_update_micros = now_micros(ctx);
            ctx.db.player().entity_id().update(p);
        }
        None => {
            ctx.db.player().insert(Player {
                entity_id: alloc_entity_id(ctx),
                generation: 1,
                owner_identity: ctx.sender(),
                character_id: account.character_id,
                session: Some(conn),
                x: SPAWN_POINT[0],
                y: SPAWN_POINT[1],
                z: SPAWN_POINT[2],
                vx: 0.0,
                vy: 0.0,
                vz: 0.0,
                yaw: 0.0,
                zone_id: zone_id_from_position(SPAWN_POINT[0], SPAWN_POINT[1]),
                epoch: 1,
                last_input_seq: 0,
                last_update_micros: now_micros(ctx),
            });
        }
    }
    Ok(())
}

/// Contract §5 + §4.4: session-scoped input. Anything not acceptable is
/// dropped silently — never an error (stale/duplicate/wrong-epoch input is
/// normal during reconnect churn).
///
/// M5 trust-the-client movement: the client's position is accepted after
/// sanity checks. Isolated here on purpose — M6 replaces this body with
/// server-authoritative integration.
#[reducer]
pub fn submit_input(
    ctx: &ReducerContext,
    epoch: u32,
    seq: u32,
    x: f32,
    y: f32,
    z: f32,
    yaw: f32,
) -> Result<(), String> {
    let Some(mut p) = ctx.db.player().owner_identity().find(ctx.sender()) else {
        return Ok(());
    };
    // §4.4: only the active session may act.
    if p.session.is_none() || p.session != ctx.connection_id() {
        return Ok(());
    }
    if !accept_input(p.epoch, p.last_input_seq, epoch, seq) {
        return Ok(());
    }
    if ![x, y, z, yaw].iter().all(|v| v.is_finite()) {
        return Ok(());
    }
    // Anti-teleport sanity: clamp (never drop) the step so a lag spike can't
    // deadlock the row behind the cap — the row converges at cap × send rate.
    let (mut dx, mut dy, mut dz) = (x - p.x, y - p.y, z - p.z);
    let dist_sq = dx * dx + dy * dy + dz * dz;
    if dist_sq > MAX_INPUT_STEP_M * MAX_INPUT_STEP_M {
        let scale = MAX_INPUT_STEP_M / dist_sq.sqrt();
        dx *= scale;
        dy *= scale;
        dz *= scale;
    }

    let now = now_micros(ctx);
    let dt = ((now - p.last_update_micros).max(1) as f32) / 1_000_000.0;
    p.vx = dx / dt;
    p.vy = dy / dt;
    p.vz = dz / dt;
    p.x += dx;
    p.y += dy;
    p.z += dz;
    p.yaw = yaw;
    p.zone_id = zone_id_from_position(p.x, p.y);
    p.last_input_seq = seq;
    p.last_update_micros = now;
    ctx.db.player().entity_id().update(p);
    Ok(())
}

/// Dev/test reducer (unauthenticated like `despawn_npc`): teleports the
/// caller's player. Lets acceptance tests cross zone borders
/// deterministically without simulating walks.
#[reducer]
pub fn dev_teleport(ctx: &ReducerContext, x: f32, y: f32, z: f32) -> Result<(), String> {
    let mut p = ctx
        .db
        .player()
        .owner_identity()
        .find(ctx.sender())
        .ok_or("no player for caller")?;
    if ![x, y, z].iter().all(|v| v.is_finite()) {
        return Err("non-finite position".into());
    }
    p.x = x;
    p.y = y;
    p.z = z;
    (p.vx, p.vy, p.vz) = (0.0, 0.0, 0.0);
    p.zone_id = zone_id_from_position(x, y);
    p.last_update_micros = now_micros(ctx);
    ctx.db.player().entity_id().update(p);
    Ok(())
}

/// Plan D5: upsert the caller's ping row; the client completes the
/// NTP-style sample from the row update's arrival time.
#[reducer]
pub fn ping(ctx: &ReducerContext, nonce: u64) -> Result<(), String> {
    let row = PingResult {
        identity: ctx.sender(),
        nonce,
        server_time_micros: now_micros(ctx),
    };
    if ctx.db.ping_result().identity().find(ctx.sender()).is_some() {
        ctx.db.ping_result().identity().update(row);
    } else {
        ctx.db.ping_result().insert(row);
    }
    Ok(())
}

/// Contract §3.1: tombstone upsert + row delete in one transaction — both or
/// neither. Dev/test reducer (unauthenticated on purpose): lets the CLI and
/// acceptance tests exercise destroyed-vs-out-of-scope classification.
#[reducer]
pub fn despawn_npc(ctx: &ReducerContext, entity_id: u64) -> Result<(), String> {
    let npc = ctx
        .db
        .npc()
        .entity_id()
        .find(entity_id)
        .ok_or("no such npc")?;
    let ts = Tombstone {
        entity_id,
        generation: npc.generation,
        despawned_at_micros: now_micros(ctx),
    };
    if ctx.db.tombstone().entity_id().find(entity_id).is_some() {
        ctx.db.tombstone().entity_id().update(ts);
    } else {
        ctx.db.tombstone().insert(ts);
    }
    ctx.db.npc().entity_id().delete(entity_id);
    Ok(())
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

    // NPC wander: deterministic bounded walk around the spawn point.
    // Seeding here (not in `init`) is idempotent across incremental
    // publishes of an already-initialized database.
    let npcs: Vec<Npc> = ctx.db.npc().iter().collect();
    if npcs.is_empty() {
        seed_npcs(ctx, now);
    } else {
        let dt = TICK_INTERVAL_MS as f32 / 1000.0;
        for mut n in npcs {
            // Per-entity heading drift; steer home outside the radius.
            n.yaw += dt * (0.3 + 0.15 * ((n.entity_id % 5) as f32));
            let dx = n.x - SPAWN_POINT[0];
            let dy = n.y - SPAWN_POINT[1];
            if dx * dx + dy * dy > NPC_WANDER_RADIUS * NPC_WANDER_RADIUS {
                n.yaw = (-dy).atan2(-dx);
            }
            n.yaw = n.yaw.rem_euclid(std::f32::consts::TAU);
            n.x += n.yaw.cos() * NPC_SPEED_MPS * dt;
            n.y += n.yaw.sin() * NPC_SPEED_MPS * dt;
            n.zone_id = zone_id_from_position(n.x, n.y);
            n.last_update_micros = now;
            ctx.db.npc().entity_id().update(n);
        }
    }
    Ok(())
}

fn seed_npcs(ctx: &ReducerContext, now: i64) {
    for i in 0..NPC_SEED_COUNT {
        let angle = (i as f32) * std::f32::consts::TAU / NPC_SEED_COUNT as f32;
        let x = SPAWN_POINT[0] + angle.cos() * 4.0;
        let y = SPAWN_POINT[1] + angle.sin() * 4.0;
        ctx.db.npc().insert(Npc {
            entity_id: alloc_entity_id(ctx),
            generation: 1,
            x,
            y,
            z: SPAWN_POINT[2],
            yaw: angle,
            zone_id: zone_id_from_position(x, y),
            kind: 0,
            last_update_micros: now,
        });
    }
}

// ---------------------------------------------------------------------------
// Parity harness (M6 D5)
// ---------------------------------------------------------------------------

/// Result of one `run_parity_trace` call, read by the native test harness
/// (public so the harness can subscribe). One row per trace id, overwritten
/// on re-run.
#[table(accessor = parity_result, public)]
pub struct ParityResult {
    #[primary_key]
    trace_id: String,
    steps: u32,
    end_x: f32,
    end_y: f32,
    end_z: f32,
    /// Order-sensitive hash over per-step positions (defined in package 5).
    state_hash: u64,
}

/// Dev/test reducer (M6 D5): replays a named motion trace against the
/// embedded collision and records the outcome in `parity_result`. The trace
/// list ships with the shared controller (packages 2/5); until then this
/// only proves the embedded store builds inside the module.
#[reducer]
pub fn run_parity_trace(_ctx: &ReducerContext, trace_id: String) -> Result<(), String> {
    let store = collision_store().ok_or("embedded collision failed to load")?;
    Err(format!(
        "unknown parity trace {trace_id:?} ({} collision chunks loaded)",
        store.len()
    ))
}
