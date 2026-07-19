//! Game server module (M5 Net-A).
//!
//! Identity, session and lifecycle semantics are normative in
//! `docs/roadmap/M5-WORLD-IDENTITY-CONTRACT.md`; protocol constants come
//! from `game_shared::net::schema` (single source of truth).
//!
//! These tables ARE the save system — player rows are never deleted on
//! disconnect.

use game_shared::collision::{manifest_hash, ChunkStore};
use game_shared::combat::{
    self, AbilityId, AbilityKind, TargetView, NPC_HP_MAX, PLAYER_HP_MAX, PLAYER_MANA_MAX,
};
use game_shared::motion::broadphase::Broadphase;
use game_shared::motion::combat::{
    hitscan, projectile_step, HitKind, Projectile as SimProjectile, SweepOutcome,
};
use game_shared::motion::{self, MotionConfig, MotionState, MoveIntent};
use game_shared::net::schema::{accept_input, PROTOCOL_VERSION, REALM_ID, TOMBSTONE_TTL_SECS};
use game_shared::world_grid::cell_id_from_position;
use glam::Vec3;
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
            // Cold-load cost (bytes → ChunkStore incl. BVH build) shows up in
            // the module log via the console timer (M6 pkg 5 perf gate).
            let _watch = spacetimedb::log_stopwatch::LogStopwatch::new("collision_store_cold_build");
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

/// Movement authority tick (M6 D3): matches the client's 20 Hz input rate;
/// one queued input = one `motion::step` of `MOVE_DT`.
const MOVE_TICK_MS: u64 = 50;
/// Catch-up bound per tick; deeper backlogs wait (or get dropped, below).
const MAX_STEPS_PER_TICK: usize = 4;
/// Queue depth beyond this drops oldest inputs (reconciliation snaps).
const MAX_QUEUE_DEPTH: usize = 8;
/// Empty-queue grace: repeat the last move intent for this many ticks
/// (250 ms) before zeroing — bridges ordinary packet gaps without rubber-
/// banding. Gravity integrates regardless.
const HELD_INTENT_GRACE_TICKS: u32 = 5;

/// Dev spawn point (Z-up, meters). The world manifest is runtime-loaded RON
/// the WASM module cannot read; a manifest-driven spawn is deferred.
const SPAWN_POINT: [f32; 3] = [0.0, 0.0, 1.0];
/// Player spawn is a capsule *center*, dropped just above the greybox
/// surface at the origin (ground z = 8, see the recorded golden trace) —
/// with real gravity the old z = 1 would be under the terrain.
const PLAYER_SPAWN_Z: f32 = 9.0;
/// NPC capsule center, same convention as players (M7: dummies must sit on
/// the greybox surface — the old z = 1 put them ~7 m underground, unhittable
/// once 3D range/LoS checks exist).
const NPC_SPAWN_Z: f32 = 9.0;
const NPC_SEED_COUNT: u32 = 4;
const NPC_WANDER_RADIUS: f32 = 8.0;
const NPC_SPEED_MPS: f32 = 1.5;

// M8 D3 coarse-tier write policy (ruled 2026-07-20): movement upserts only
// on cell change or ≥ 2 m moved, capped at one write per entity per second.
const COARSE_MOVE_M: f32 = 2.0;
const COARSE_MIN_INTERVAL_MICROS: i64 = 1_000_000;
const COARSE_KIND_PLAYER: u32 = 0;
const COARSE_KIND_NPC: u32 = 1;

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
    /// M6: controller grounding, part of the atomic own-state ack.
    grounded: bool,
    // M7 combat state (plan D2). Times are server micros; 0 = unset.
    hp: f32,
    hp_max: f32,
    mana: f32,
    mana_max: f32,
    alive: bool,
    respawn_at_micros: i64,
    gcd_until_micros: i64,
    #[index(btree)]
    cell_id: u64,
    epoch: u32,
    /// Highest *received* seq (acceptance guard, M5 semantics).
    last_input_seq: u32,
    /// Highest seq *consumed by the movement tick* — what client prediction
    /// reconciles against (M6 D3).
    last_applied_seq: u32,
    /// Authoritative timestamp of the last state write (interpolation base).
    last_update_micros: i64,
}

/// Accepted-but-not-yet-simulated inputs (M6 D3). Private: never replicated.
/// A queue (not a latest-intent mailbox) keeps client prediction steps and
/// server steps 1:1 per seq and preserves `jump` edges.
#[table(accessor = pending_input)]
pub struct PendingInput {
    #[primary_key]
    #[auto_inc]
    id: u64,
    #[index(btree)]
    entity_id: u64,
    seq: u32,
    move_x: f32,
    move_y: f32,
    yaw: f32,
    sprint: bool,
    jump: bool,
}

/// Last consumed move intent per player (M6 D3 grace). Private.
#[table(accessor = held_intent)]
pub struct HeldIntent {
    #[primary_key]
    entity_id: u64,
    move_x: f32,
    move_y: f32,
    yaw: f32,
    sprint: bool,
    grace_ticks: u32,
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
    // M7: target-dummy combat state (plan D2). Dead rows are retained until
    // `tick` respawns them.
    hp: f32,
    hp_max: f32,
    alive: bool,
    respawn_at_micros: i64,
    #[index(btree)]
    cell_id: u64,
    kind: u32,
    last_update_micros: i64,
}

/// Per-(entity, ability) cooldown end time (plan D2). PK is the
/// deterministic `combat::cooldown_key` — upsert-by-PK, no auto_inc + scan.
/// End-times (not durations) so replication latency can't skew HUD sweeps.
/// Expired rows are GC'd in `tick`.
#[table(accessor = ability_cooldown, public)]
pub struct AbilityCooldown {
    #[primary_key]
    key: u64,
    #[index(btree)]
    entity_id: u64,
    ability_id: u16,
    ready_at_micros: i64,
}

/// At most one in-progress cast per entity (new cast replaces). Public so
/// the target frame can show the target's cast bar; `cell_id` is copied from
/// the caster at cast time so the row fits the cell-scoped subscriptions
/// (mid-cast cell changes are impossible — moving cancels).
#[table(accessor = active_cast, public)]
pub struct ActiveCast {
    #[primary_key]
    entity_id: u64,
    ability_id: u16,
    target_entity_id: u64,
    #[index(btree)]
    cell_id: u64,
    start_micros: i64,
    finish_micros: i64,
}

/// In-flight firebolt (M7 D4): spawned at cast completion, stepped in
/// `move_tick` with `projectile_step`, deleted on hit or after
/// `PROJECTILE_LIFETIME_SECS`. Public — clients render by extrapolating from
/// the last server step. `cell_id` is recomputed per step (M8: a firebolt
/// crosses a 64 m cell in ~2 s, so fixed-at-spawn would misfile it).
#[table(accessor = projectile, public)]
pub struct Projectile {
    #[primary_key]
    entity_id: u64,
    caster_entity_id: u64,
    ability_id: u16,
    x: f32,
    y: f32,
    z: f32,
    vx: f32,
    vy: f32,
    vz: f32,
    #[index(btree)]
    cell_id: u64,
    spawned_at_micros: i64,
    last_update_micros: i64,
}

/// M8 D3: far-tier position row — whole-row replication means "position
/// only" is a second, smaller table, not a projection. Movement upserts are
/// rate-limited (`maybe_coarse`); death deletes the row, respawn re-upserts
/// it with the bumped generation so far observers see corpses vanish.
#[table(accessor = entity_coarse, public)]
pub struct EntityCoarse {
    #[primary_key]
    entity_id: u64,
    generation: u32,
    /// 0 = player, 1 = npc (client maps to `EntityKind`).
    kind: u32,
    #[index(btree)]
    cell_id: u64,
    x: f32,
    y: f32,
    z: f32,
    updated_micros: i64,
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

/// M8 D4: per-table cumulative row-write counters for the load harness.
/// Deliberately NOT `public`-visible to ordinary clients (owner-only): a
/// counter in `config` would broadcast a write to the whole population each
/// tick, defeating the idle-delivery goal it measures.
#[table(accessor = metrics)]
pub struct Metrics {
    #[primary_key]
    id: u32, // always 0
    player_rows_written: u64,
    npc_rows_written: u64,
    projectile_rows_written: u64,
    coarse_rows_written: u64,
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

#[table(accessor = move_timer, scheduled(move_tick))]
pub struct MoveTimer {
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

/// Drop all queued/held input for a player (epoch bump, teleport, teardown).
fn clear_input_queue(ctx: &ReducerContext, entity_id: u64) {
    ctx.db.pending_input().entity_id().delete(entity_id);
    ctx.db.held_intent().entity_id().delete(entity_id);
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
    ctx.db.move_timer().insert(MoveTimer {
        scheduled_id: 0,
        scheduled_at: ScheduleAt::Interval(Duration::from_millis(MOVE_TICK_MS).into()),
    });
    log::info!(
        "game_module initialized: protocol v{PROTOCOL_VERSION}, realm {REALM_ID}, tick {TICK_INTERVAL_MS} ms, move tick {MOVE_TICK_MS} ms"
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
            let entity_id = p.entity_id;
            ctx.db.player().entity_id().update(p);
            clear_input_queue(ctx, entity_id);
            // M7 D3: disconnect teardown deletes any in-progress cast.
            ctx.db.active_cast().entity_id().delete(entity_id);
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
            p.last_applied_seq = 0;
            p.last_update_micros = now_micros(ctx);
            let entity_id = p.entity_id;
            ctx.db.player().entity_id().update(p);
            clear_input_queue(ctx, entity_id);
        }
        None => {
            let entity_id = alloc_entity_id(ctx);
            write_coarse(
                ctx,
                entity_id,
                1,
                COARSE_KIND_PLAYER,
                SPAWN_POINT[0],
                SPAWN_POINT[1],
                PLAYER_SPAWN_Z,
                now_micros(ctx),
            );
            ctx.db.player().insert(Player {
                entity_id,
                generation: 1,
                owner_identity: ctx.sender(),
                character_id: account.character_id,
                session: Some(conn),
                x: SPAWN_POINT[0],
                y: SPAWN_POINT[1],
                z: PLAYER_SPAWN_Z,
                vx: 0.0,
                vy: 0.0,
                vz: 0.0,
                yaw: 0.0,
                grounded: false,
                hp: PLAYER_HP_MAX,
                hp_max: PLAYER_HP_MAX,
                mana: PLAYER_MANA_MAX,
                mana_max: PLAYER_MANA_MAX,
                alive: true,
                respawn_at_micros: 0,
                gcd_until_micros: 0,
                cell_id: cell_id_from_position(SPAWN_POINT[0], SPAWN_POINT[1]),
                epoch: 1,
                last_input_seq: 0,
                last_applied_seq: 0,
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
/// M6 (D3): intent-only. Accepted inputs are queued for the movement tick;
/// this reducer moves nothing. `last_input_seq` advances at acceptance
/// (reorder/replay guard); `last_applied_seq` advances when the tick
/// consumes the input.
#[reducer]
pub fn submit_input(
    ctx: &ReducerContext,
    epoch: u32,
    seq: u32,
    move_x: f32,
    move_y: f32,
    yaw: f32,
    sprint: bool,
    jump: bool,
) -> Result<(), String> {
    let Some(mut p) = ctx.db.player().owner_identity().find(ctx.sender()) else {
        return Ok(());
    };
    // §4.4: only the active session may act.
    if p.session.is_none() || p.session != ctx.connection_id() {
        return Ok(());
    }
    // M7 D5: corpses don't move — dropped silently like any stale input.
    if !p.alive {
        return Ok(());
    }
    if !accept_input(p.epoch, p.last_input_seq, epoch, seq) {
        return Ok(());
    }
    if ![move_x, move_y, yaw].iter().all(|v| v.is_finite()) {
        return Ok(());
    }
    p.last_input_seq = seq;
    let entity_id = p.entity_id;
    ctx.db.player().entity_id().update(p);

    ctx.db.pending_input().insert(PendingInput {
        id: 0, // auto_inc
        entity_id,
        seq,
        move_x,
        move_y,
        yaw,
        sprint,
        jump,
    });
    // Depth cap: drop oldest — reconciliation snaps the client past the gap.
    let mut queued: Vec<PendingInput> = ctx.db.pending_input().entity_id().filter(entity_id).collect();
    if queued.len() > MAX_QUEUE_DEPTH {
        queued.sort_by_key(|i| i.seq);
        for stale in &queued[..queued.len() - MAX_QUEUE_DEPTH] {
            ctx.db.pending_input().id().delete(stale.id);
        }
    }
    Ok(())
}

/// Dev/test reducer (unauthenticated like `despawn_npc`): teleports the
/// caller's player. Lets acceptance tests cross cell borders
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
    // M6: stale queued intent must not walk the player away from the target;
    // grounding is re-established by the next movement tick. Seq counters
    // stay intact (the session did not restart).
    p.grounded = false;
    p.cell_id = cell_id_from_position(x, y);
    p.last_update_micros = now_micros(ctx);
    let (entity_id, generation) = (p.entity_id, p.generation);
    ctx.db.player().entity_id().update(p);
    write_coarse(ctx, entity_id, generation, COARSE_KIND_PLAYER, x, y, z, now_micros(ctx));
    clear_input_queue(ctx, entity_id);
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
    upsert_tombstone(ctx, entity_id, npc.generation, now_micros(ctx));
    ctx.db.npc().entity_id().delete(entity_id);
    ctx.db.entity_coarse().entity_id().delete(entity_id);
    Ok(())
}

/// Unconditional coarse upsert — spawn/teleport/respawn sites, where the
/// row must reflect the new position (or generation) immediately.
fn write_coarse(
    ctx: &ReducerContext,
    entity_id: u64,
    generation: u32,
    kind: u32,
    x: f32,
    y: f32,
    z: f32,
    now: i64,
) {
    let row = EntityCoarse {
        entity_id,
        generation,
        kind,
        cell_id: cell_id_from_position(x, y),
        x,
        y,
        z,
        updated_micros: now,
    };
    if ctx.db.entity_coarse().entity_id().find(entity_id).is_some() {
        ctx.db.entity_coarse().entity_id().update(row);
    } else {
        ctx.db.entity_coarse().insert(row);
    }
}

/// Movement-path coarse upsert under the D3 write policy: cell change or
/// ≥ `COARSE_MOVE_M` moved, hard-capped at one write per entity per second.
/// Returns whether a row was written (D4 metrics).
fn maybe_coarse(
    ctx: &ReducerContext,
    entity_id: u64,
    generation: u32,
    kind: u32,
    x: f32,
    y: f32,
    z: f32,
    now: i64,
) -> bool {
    let Some(c) = ctx.db.entity_coarse().entity_id().find(entity_id) else {
        write_coarse(ctx, entity_id, generation, kind, x, y, z, now);
        return true;
    };
    if now - c.updated_micros < COARSE_MIN_INTERVAL_MICROS {
        return false;
    }
    let moved2 = (x - c.x).powi(2) + (y - c.y).powi(2) + (z - c.z).powi(2);
    if c.cell_id == cell_id_from_position(x, y) && moved2 < COARSE_MOVE_M * COARSE_MOVE_M {
        return false;
    }
    write_coarse(ctx, entity_id, generation, kind, x, y, z, now);
    true
}

/// M8 D4: cumulative per-table row-write counters for the load harness.
/// Written only on change, so an idle population costs zero metric writes.
fn bump_metrics(ctx: &ReducerContext, player: u64, npc: u64, projectile: u64, coarse: u64) {
    if player + npc + projectile + coarse == 0 {
        return;
    }
    match ctx.db.metrics().id().find(0) {
        Some(mut m) => {
            m.player_rows_written += player;
            m.npc_rows_written += npc;
            m.projectile_rows_written += projectile;
            m.coarse_rows_written += coarse;
            ctx.db.metrics().id().update(m);
        }
        None => {
            ctx.db.metrics().insert(Metrics {
                id: 0,
                player_rows_written: player,
                npc_rows_written: npc,
                projectile_rows_written: projectile,
                coarse_rows_written: coarse,
            });
        }
    }
}

fn upsert_tombstone(ctx: &ReducerContext, entity_id: u64, generation: u32, now: i64) {
    let ts = Tombstone {
        entity_id,
        generation,
        despawned_at_micros: now,
    };
    if ctx.db.tombstone().entity_id().find(entity_id).is_some() {
        ctx.db.tombstone().entity_id().update(ts);
    } else {
        ctx.db.tombstone().insert(ts);
    }
}

/// Dev/test reducer (unauthenticated like `despawn_npc`): direct damage to
/// any entity, so death/respawn is testable without a combat rotation.
#[reducer]
pub fn dev_damage(ctx: &ReducerContext, entity_id: u64, amount: f32) -> Result<(), String> {
    if !amount.is_finite() {
        return Err("non-finite amount".into());
    }
    damage_entity(ctx, entity_id, amount, now_micros(ctx));
    Ok(())
}

// ---------------------------------------------------------------------------
// Combat (M7 D3)
// ---------------------------------------------------------------------------

/// Combat-relevant view of a target row (player or NPC) plus its position.
struct ResolvedTarget {
    view: TargetView,
    pos: Vec3,
}

fn resolve_target(ctx: &ReducerContext, entity_id: u64) -> Option<ResolvedTarget> {
    if let Some(p) = ctx.db.player().entity_id().find(entity_id) {
        return Some(ResolvedTarget {
            view: TargetView {
                entity_id,
                alive: p.alive,
                connected: p.session.is_some(),
            },
            pos: Vec3::new(p.x, p.y, p.z),
        });
    }
    ctx.db.npc().entity_id().find(entity_id).map(|n| ResolvedTarget {
        view: TargetView {
            entity_id,
            alive: n.alive,
            connected: true,
        },
        pos: Vec3::new(n.x, n.y, n.z),
    })
}

fn eye(capsule_center: Vec3) -> Vec3 {
    capsule_center + Vec3::Z * combat::EYE_OFFSET_M
}

/// Eye-to-eye world LoS (plan D3: world blocks, entities don't).
fn line_of_sight(store: &ChunkStore, from: Vec3, to: Vec3) -> bool {
    let delta = eye(to) - eye(from);
    let dist = delta.length();
    dist <= f32::EPSILON || store.raycast(eye(from), delta / dist, dist).is_none()
}

fn upsert_cooldown(ctx: &ReducerContext, entity_id: u64, def: &combat::AbilityDef, now: i64) {
    if def.cooldown_secs <= 0.0 {
        return; // GCD-limited only
    }
    let row = AbilityCooldown {
        key: combat::cooldown_key(entity_id, def.id),
        entity_id,
        ability_id: def.id.0,
        ready_at_micros: now + combat::micros(def.cooldown_secs) as i64,
    };
    if ctx.db.ability_cooldown().key().find(row.key).is_some() {
        ctx.db.ability_cooldown().key().update(row);
    } else {
        ctx.db.ability_cooldown().insert(row);
    }
}

fn upsert_active_cast(ctx: &ReducerContext, row: ActiveCast) {
    if ctx.db.active_cast().entity_id().find(row.entity_id).is_some() {
        ctx.db.active_cast().entity_id().update(row); // new cast replaces
    } else {
        ctx.db.active_cast().insert(row);
    }
}

/// Subtract hp on whichever row carries the target. hp 0 is the death
/// transition (plan D5): the row stays, inert, tombstoned under the current
/// generation (remote proxies destroy through the M5 evidence path); respawn
/// is scheduled via `respawn_at_micros`.
fn damage_entity(ctx: &ReducerContext, entity_id: u64, amount: f32, now: i64) {
    if let Some(mut p) = ctx.db.player().entity_id().find(entity_id) {
        if !p.alive {
            return;
        }
        p.hp = (p.hp - amount).max(0.0);
        p.last_update_micros = now;
        if p.hp <= 0.0 {
            p.alive = false;
            p.respawn_at_micros = now + combat::micros(combat::RESPAWN_SECS) as i64;
            (p.vx, p.vy, p.vz) = (0.0, 0.0, 0.0);
            upsert_tombstone(ctx, entity_id, p.generation, now);
            clear_input_queue(ctx, entity_id);
            ctx.db.active_cast().entity_id().delete(entity_id);
            // M8 D3: far observers see the corpse vanish.
            ctx.db.entity_coarse().entity_id().delete(entity_id);
        }
        ctx.db.player().entity_id().update(p);
    } else if let Some(mut n) = ctx.db.npc().entity_id().find(entity_id) {
        if !n.alive {
            return;
        }
        n.hp = (n.hp - amount).max(0.0);
        n.last_update_micros = now;
        if n.hp <= 0.0 {
            n.alive = false;
            n.respawn_at_micros = now + combat::micros(combat::RESPAWN_SECS) as i64;
            upsert_tombstone(ctx, entity_id, n.generation, now);
            ctx.db.entity_coarse().entity_id().delete(entity_id);
        }
        ctx.db.npc().entity_id().update(n);
    }
}

/// Plan D5: teleport to spawn, full restore, `generation += 1` (every remote
/// client destroy-and-respawns the proxy via the tested generation-replace
/// path) and `epoch += 1` (the own client restarts prediction exactly like a
/// reconnect).
fn respawn_player(ctx: &ReducerContext, mut p: Player, now: i64) {
    p.x = SPAWN_POINT[0];
    p.y = SPAWN_POINT[1];
    p.z = PLAYER_SPAWN_Z;
    (p.vx, p.vy, p.vz) = (0.0, 0.0, 0.0);
    p.grounded = false;
    p.hp = p.hp_max;
    p.mana = p.mana_max;
    p.alive = true;
    p.respawn_at_micros = 0;
    p.gcd_until_micros = 0;
    p.generation += 1;
    p.epoch += 1;
    p.last_input_seq = 0;
    p.last_applied_seq = 0;
    p.cell_id = cell_id_from_position(p.x, p.y);
    p.last_update_micros = now;
    let (entity_id, generation) = (p.entity_id, p.generation);
    let (x, y, z) = (p.x, p.y, p.z);
    ctx.db.player().entity_id().update(p);
    write_coarse(ctx, entity_id, generation, COARSE_KIND_PLAYER, x, y, z, now);
    clear_input_queue(ctx, entity_id);
}

fn cast_err(e: combat::CastError) -> String {
    format!("cast rejected: {e:?}")
}

/// Melee/bolt hit re-check through `hitscan` with the target as the single
/// candidate — the world can still block the hit even after eye-to-eye LoS.
fn hits_target(
    cfg: &MotionConfig,
    store: &ChunkStore,
    caster_pos: Vec3,
    target: &ResolvedTarget,
) -> bool {
    let mut bp = Broadphase::new();
    bp.insert(target.view.entity_id, target.pos);
    let origin = eye(caster_pos);
    let dir = eye(target.pos) - origin;
    let max_dist = dir.length() + cfg.capsule_half_seg + cfg.capsule_radius;
    matches!(
        hitscan(cfg, store, &bp, origin, dir, max_dist),
        Some(h) if h.kind == (HitKind::Entity { entity_id: target.view.entity_id })
    )
}

/// M7 D3: server-authoritative cast entry. Validation chain (first failure
/// aborts the transaction — the failure IS the rejection, no partial state):
/// session → alive/GCD/cooldown/mana → target legality → range → LoS.
/// Instant kinds resolve here; cast kinds insert `ActiveCast` + start the
/// GCD and commit mana/cooldown at completion (`move_tick`).
#[reducer]
pub fn cast_ability(
    ctx: &ReducerContext,
    ability_id: u16,
    target_entity_id: u64,
) -> Result<(), String> {
    let mut caster = ctx
        .db
        .player()
        .owner_identity()
        .find(ctx.sender())
        .ok_or("no player for caller")?;
    // §4.4: only the active session may act.
    if caster.session.is_none() || caster.session != ctx.connection_id() {
        return Err("not the active session".into());
    }
    let now = now_micros(ctx);
    let def = combat::ability(AbilityId(ability_id)).ok_or("unknown ability")?;
    let ready_at = ctx
        .db
        .ability_cooldown()
        .key()
        .find(combat::cooldown_key(caster.entity_id, def.id))
        .map_or(0, |c| c.ready_at_micros);
    combat::can_cast(
        def,
        now.max(0) as u64,
        caster.gcd_until_micros.max(0) as u64,
        ready_at.max(0) as u64,
        caster.mana,
        caster.alive,
    )
    .map_err(cast_err)?;

    let target = if target_entity_id == 0 {
        None
    } else {
        Some(resolve_target(ctx, target_entity_id).ok_or(cast_err(combat::CastError::TargetNotFound))?)
    };
    combat::target_legal(def.kind, caster.entity_id, target.as_ref().map(|t| &t.view))
        .map_err(cast_err)?;

    let caster_pos = Vec3::new(caster.x, caster.y, caster.z);
    if def.kind.hostile_targeted() {
        let t = target.as_ref().expect("target_legal enforced presence");
        if caster_pos.distance(t.pos) > def.range_m {
            return Err(cast_err(combat::CastError::OutOfRange));
        }
        let store = collision_store().ok_or("collision unavailable")?;
        if !line_of_sight(store, caster_pos, t.pos) {
            return Err(cast_err(combat::CastError::NoLineOfSight));
        }
    }

    caster.gcd_until_micros = now + combat::micros(combat::GCD_SECS) as i64;

    if def.cast_secs > 0.0 {
        upsert_active_cast(
            ctx,
            ActiveCast {
                entity_id: caster.entity_id,
                ability_id,
                target_entity_id,
                cell_id: caster.cell_id,
                start_micros: now,
                finish_micros: now + combat::micros(def.cast_secs) as i64,
            },
        );
        ctx.db.player().entity_id().update(caster);
        return Ok(());
    }

    // Instant kinds: all remaining validation happens before any row write.
    match def.kind {
        AbilityKind::Strike => {
            let t = target.as_ref().expect("target_legal enforced presence");
            let store = collision_store().ok_or("collision unavailable")?;
            if !hits_target(&MotionConfig::default(), store, caster_pos, t) {
                return Err(cast_err(combat::CastError::NoLineOfSight));
            }
            caster.mana -= def.mana_cost;
            upsert_cooldown(ctx, caster.entity_id, def, now);
            ctx.db.player().entity_id().update(caster);
            damage_entity(ctx, t.view.entity_id, def.amount, now);
        }
        AbilityKind::NovaAoe => {
            // Per-transaction broadphase over live players + NPCs (caster
            // excluded — nothing hits itself), then a true radius filter.
            let mut bp = Broadphase::new();
            for p in ctx.db.player().iter() {
                if p.session.is_some() && p.alive && p.entity_id != caster.entity_id {
                    bp.insert(p.entity_id, Vec3::new(p.x, p.y, p.z));
                }
            }
            for n in ctx.db.npc().iter() {
                if n.alive {
                    bp.insert(n.entity_id, Vec3::new(n.x, n.y, n.z));
                }
            }
            caster.mana -= def.mana_cost;
            upsert_cooldown(ctx, caster.entity_id, def, now);
            ctx.db.player().entity_id().update(caster);
            for id in bp.aoe_candidates(caster_pos, def.range_m) {
                if let Some(t) = resolve_target(ctx, id) {
                    if caster_pos.distance(t.pos) <= def.range_m {
                        damage_entity(ctx, id, def.amount, now);
                    }
                }
            }
        }
        AbilityKind::Projectile | AbilityKind::Heal => unreachable!("cast kinds handled above"),
    }
    Ok(())
}

/// Cast completion (plan D3): runs from `move_tick` AFTER movement/interrupt
/// consumption. Re-validates what can have changed mid-cast; failures fizzle
/// silently (a scheduled transaction must not abort per cast) — mana and
/// cooldown only commit on success.
fn complete_cast(
    ctx: &ReducerContext,
    cfg: &MotionConfig,
    store: &ChunkStore,
    cast: &ActiveCast,
    now: i64,
) {
    let Some(def) = combat::ability(AbilityId(cast.ability_id)) else {
        return;
    };
    let Some(mut caster) = ctx.db.player().entity_id().find(cast.entity_id) else {
        return;
    };
    if !caster.alive || caster.session.is_none() || caster.mana < def.mana_cost {
        return;
    }
    let caster_pos = Vec3::new(caster.x, caster.y, caster.z);
    match def.kind {
        AbilityKind::Heal => {
            caster.mana -= def.mana_cost;
            caster.hp = (caster.hp + def.amount).min(caster.hp_max);
            caster.last_update_micros = now;
            upsert_cooldown(ctx, cast.entity_id, def, now);
            ctx.db.player().entity_id().update(caster);
        }
        AbilityKind::Projectile => {
            let Some(t) = resolve_target(ctx, cast.target_entity_id) else {
                return;
            };
            if !t.view.alive
                || !t.view.connected
                || caster_pos.distance(t.pos) > def.range_m
                || !line_of_sight(store, caster_pos, t.pos)
            {
                return;
            }
            caster.mana -= def.mana_cost;
            upsert_cooldown(ctx, cast.entity_id, def, now);
            let cell_id = caster.cell_id;
            ctx.db.player().entity_id().update(caster);
            // Dumb-fire eye-to-eye (D4): aim above the target eye by ½·g·t²
            // (first-order gravity compensation over the straight-line flight
            // time). No homing — the target can outrun or dodge it.
            let origin = eye(caster_pos);
            let dist = origin.distance(eye(t.pos)).max(0.001);
            let t_flight = dist / def.projectile_speed_mps;
            let aim = eye(t.pos) + Vec3::Z * 0.5 * cfg.gravity * t_flight * t_flight;
            let vel = (aim - origin).normalize() * def.projectile_speed_mps;
            ctx.db.projectile().insert(Projectile {
                entity_id: alloc_entity_id(ctx),
                caster_entity_id: cast.entity_id,
                ability_id: cast.ability_id,
                x: origin.x,
                y: origin.y,
                z: origin.z,
                vx: vel.x,
                vy: vel.y,
                vz: vel.z,
                cell_id,
                spawned_at_micros: now,
                last_update_micros: now,
            });
        }
        AbilityKind::Strike | AbilityKind::NovaAoe => {} // instant; never queued
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
    // Measured, not nominal: scheduled ticks can run late and fixed ×0.1
    // regen would silently lose that time.
    let elapsed_secs = ((now - clock.server_time_micros).max(0) as f32) / 1_000_000.0;
    clock.tick += 1;
    clock.server_time_micros = now;
    ctx.db.clock().id().update(clock);

    // M7 D3: mana regen for live players; expired-cooldown GC.
    let thirsty: Vec<Player> = ctx
        .db
        .player()
        .iter()
        .filter(|p| p.session.is_some() && p.alive && p.mana < p.mana_max)
        .collect();
    for mut p in thirsty {
        p.mana = (p.mana + combat::MANA_REGEN_PER_SEC * elapsed_secs).min(p.mana_max);
        ctx.db.player().entity_id().update(p);
    }
    let expired_cd: Vec<u64> = ctx
        .db
        .ability_cooldown()
        .iter()
        .filter(|c| c.ready_at_micros <= now)
        .map(|c| c.key)
        .collect();
    for key in expired_cd {
        ctx.db.ability_cooldown().key().delete(key);
    }

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
        let mut npc_writes = 0u64;
        let mut coarse_writes = 0u64;
        for mut n in npcs {
            if !n.alive {
                if now >= n.respawn_at_micros {
                    // In-place respawn (the wander disc is the anchor);
                    // fresh generation so clients destroy-and-respawn.
                    n.z = NPC_SPAWN_Z;
                    n.hp = n.hp_max;
                    n.alive = true;
                    n.respawn_at_micros = 0;
                    n.generation += 1;
                    n.last_update_micros = now;
                    write_coarse(
                        ctx,
                        n.entity_id,
                        n.generation,
                        COARSE_KIND_NPC,
                        n.x,
                        n.y,
                        n.z,
                        now,
                    );
                    ctx.db.npc().entity_id().update(n);
                }
                continue;
            }
            // Per-entity heading drift; steer home outside the radius.
            let before = (n.x, n.y, n.yaw);
            n.yaw += dt * (0.3 + 0.15 * ((n.entity_id % 5) as f32));
            let dx = n.x - SPAWN_POINT[0];
            let dy = n.y - SPAWN_POINT[1];
            if dx * dx + dy * dy > NPC_WANDER_RADIUS * NPC_WANDER_RADIUS {
                n.yaw = (-dy).atan2(-dx);
            }
            n.yaw = n.yaw.rem_euclid(std::f32::consts::TAU);
            n.x += n.yaw.cos() * NPC_SPEED_MPS * dt;
            n.y += n.yaw.sin() * NPC_SPEED_MPS * dt;
            // M8 D4: changed-only guard (insurance — wander moves every
            // tick today, so NPC write load is activity-bound by design).
            if (n.x, n.y, n.yaw) == before {
                continue;
            }
            n.cell_id = cell_id_from_position(n.x, n.y);
            n.last_update_micros = now;
            if maybe_coarse(ctx, n.entity_id, n.generation, COARSE_KIND_NPC, n.x, n.y, n.z, now) {
                coarse_writes += 1;
            }
            ctx.db.npc().entity_id().update(n);
            npc_writes += 1;
        }
        bump_metrics(ctx, 0, npc_writes, 0, coarse_writes);
    }
    Ok(())
}

/// Movement authority (M6 D3): one transaction per 50 ms tick. Per player
/// with a live session: pop queued inputs in seq order (bounded), one
/// `motion::step` per input; empty queue runs a held/zero intent step so
/// gravity always integrates. One row write per player carries the complete
/// atomic ack `(epoch, last_applied_seq, pos, vel, yaw, grounded)`.
#[reducer]
pub fn move_tick(ctx: &ReducerContext, _timer: MoveTimer) -> Result<(), String> {
    if ctx.sender() != ctx.database_identity() {
        return Err("move_tick may only be invoked by the scheduler".into());
    }
    let Some(store) = collision_store() else {
        return Ok(()); // load failure already logged; never panic per tick
    };
    let cfg = MotionConfig::default();
    let now = now_micros(ctx);

    let players: Vec<Player> = ctx
        .db
        .player()
        .iter()
        .filter(|p| p.session.is_some())
        .collect();
    // Per-tick duration lands in the module log; p95/p99 are aggregated from
    // there (no in-module clock). Only ticks that simulate someone are timed.
    let _watch = (!players.is_empty())
        .then(|| spacetimedb::log_stopwatch::LogStopwatch::new("move_tick"));
    let mut player_writes = 0u64;
    let mut projectile_writes = 0u64;
    let mut coarse_writes = 0u64;
    for mut p in players {
        // M7 D5: dead rows are inert (no gravity for corpses) until the
        // respawn timer fires.
        if !p.alive {
            if now >= p.respawn_at_micros {
                respawn_player(ctx, p, now);
            }
            continue;
        }
        let mut state = MotionState {
            pos: Vec3::new(p.x, p.y, p.z),
            vel: Vec3::new(p.vx, p.vy, p.vz),
            yaw: p.yaw,
            grounded: p.grounded,
            ground_ref: None,
        };
        let before = (state, p.last_applied_seq);

        let mut queued: Vec<PendingInput> =
            ctx.db.pending_input().entity_id().filter(p.entity_id).collect();
        queued.sort_by_key(|i| i.seq);
        let consumed = queued.len().min(MAX_STEPS_PER_TICK);
        // M7 D3: movement interrupts casting — any consumed input with real
        // intent cancels, BEFORE due casts resolve below (cancel wins the
        // same-tick tie by construction).
        if queued[..consumed]
            .iter()
            .any(|i| i.jump || i.move_x != 0.0 || i.move_y != 0.0)
        {
            ctx.db.active_cast().entity_id().delete(p.entity_id);
        }
        for input in &queued[..consumed] {
            let intent = MoveIntent {
                move_dir: [input.move_x, input.move_y],
                yaw: input.yaw,
                sprint: input.sprint,
                jump: input.jump,
            };
            state = motion::step(&cfg, &state, &intent, store);
            p.last_applied_seq = input.seq;
            ctx.db.pending_input().id().delete(input.id);
        }

        if consumed > 0 {
            let last = &queued[consumed - 1];
            upsert_held_intent(
                ctx,
                HeldIntent {
                    entity_id: p.entity_id,
                    move_x: last.move_x,
                    move_y: last.move_y,
                    yaw: last.yaw,
                    sprint: last.sprint,
                    grace_ticks: HELD_INTENT_GRACE_TICKS,
                },
            );
        } else {
            // No input this tick: held intent inside the grace window, else
            // zero move. Advances no seq; gravity always integrates.
            let mut intent = MoveIntent {
                yaw: state.yaw,
                ..MoveIntent::IDLE
            };
            if let Some(mut held) = ctx.db.held_intent().entity_id().find(p.entity_id) {
                if held.grace_ticks > 0 {
                    held.grace_ticks -= 1;
                    intent.move_dir = [held.move_x, held.move_y];
                    intent.yaw = held.yaw;
                    intent.sprint = held.sprint;
                    ctx.db.held_intent().entity_id().update(held);
                }
            }
            state = motion::step(&cfg, &state, &intent, store);
        }

        if (state, p.last_applied_seq) != before {
            p.x = state.pos.x;
            p.y = state.pos.y;
            p.z = state.pos.z;
            p.vx = state.vel.x;
            p.vy = state.vel.y;
            p.vz = state.vel.z;
            p.yaw = state.yaw;
            p.grounded = state.grounded;
            p.cell_id = cell_id_from_position(p.x, p.y);
            p.last_update_micros = now;
            if maybe_coarse(
                ctx,
                p.entity_id,
                p.generation,
                COARSE_KIND_PLAYER,
                p.x,
                p.y,
                p.z,
                now,
            ) {
                coarse_writes += 1;
            }
            ctx.db.player().entity_id().update(p);
            player_writes += 1;
        }
    }

    // M7 D4: step in-flight projectiles against one shared broadphase of
    // live targets. Runs before due-cast completion, so fresh spawns take
    // their first step next tick (never in their spawn transaction).
    let projectiles: Vec<Projectile> = ctx.db.projectile().iter().collect();
    if !projectiles.is_empty() {
        let mut bp = Broadphase::new();
        for p in ctx.db.player().iter() {
            if p.session.is_some() && p.alive {
                bp.insert(p.entity_id, Vec3::new(p.x, p.y, p.z));
            }
        }
        for n in ctx.db.npc().iter() {
            if n.alive {
                bp.insert(n.entity_id, Vec3::new(n.x, n.y, n.z));
            }
        }
        let lifetime = combat::micros(combat::PROJECTILE_LIFETIME_SECS) as i64;
        // Caster transparency window: the bolt spawns at the caster's eye,
        // inside their own capsule — ignore caster hits for two ticks.
        let grace = 2 * (MOVE_TICK_MS as i64) * 1000;
        for mut row in projectiles {
            if now - row.spawned_at_micros >= lifetime {
                ctx.db.projectile().entity_id().delete(row.entity_id);
                continue;
            }
            let dt = ((now - row.last_update_micros).max(0) as f32) / 1_000_000.0;
            let sim = SimProjectile {
                pos: Vec3::new(row.x, row.y, row.z),
                vel: Vec3::new(row.vx, row.vy, row.vz),
            };
            let flying = match projectile_step(&cfg, store, &bp, sim, cfg.gravity, dt) {
                SweepOutcome::Hit(hit) => match hit.kind {
                    HitKind::Entity { entity_id }
                        if entity_id == row.caster_entity_id
                            && now - row.spawned_at_micros < grace =>
                    {
                        // Integrate through the caster as if nothing was hit.
                        let vel = sim.vel - Vec3::Z * cfg.gravity * dt;
                        Some(SimProjectile {
                            pos: sim.pos + vel * dt,
                            vel,
                        })
                    }
                    HitKind::Entity { entity_id } => {
                        let amount = combat::ability(AbilityId(row.ability_id))
                            .map_or(0.0, |d| d.amount);
                        damage_entity(ctx, entity_id, amount, now);
                        None
                    }
                    HitKind::World { .. } => None,
                },
                SweepOutcome::Flying(p) => Some(p),
            };
            match flying {
                Some(p) => {
                    row.x = p.pos.x;
                    row.y = p.pos.y;
                    row.z = p.pos.z;
                    row.vx = p.vel.x;
                    row.vy = p.vel.y;
                    row.vz = p.vel.z;
                    row.cell_id = cell_id_from_position(p.pos.x, p.pos.y);
                    row.last_update_micros = now;
                    ctx.db.projectile().entity_id().update(row);
                    projectile_writes += 1;
                }
                None => {
                    ctx.db.projectile().entity_id().delete(row.entity_id);
                }
            }
        }
    }

    // M7 D3: due casts resolve after movement consumption (normative order).
    let due: Vec<ActiveCast> = ctx
        .db
        .active_cast()
        .iter()
        .filter(|c| c.finish_micros <= now)
        .collect();
    for cast in due {
        ctx.db.active_cast().entity_id().delete(cast.entity_id);
        complete_cast(ctx, &cfg, store, &cast, now);
    }
    bump_metrics(ctx, player_writes, 0, projectile_writes, coarse_writes);
    Ok(())
}

fn upsert_held_intent(ctx: &ReducerContext, row: HeldIntent) {
    if ctx.db.held_intent().entity_id().find(row.entity_id).is_some() {
        ctx.db.held_intent().entity_id().update(row);
    } else {
        ctx.db.held_intent().insert(row);
    }
}

fn seed_npcs(ctx: &ReducerContext, now: i64) {
    for i in 0..NPC_SEED_COUNT {
        let angle = (i as f32) * std::f32::consts::TAU / NPC_SEED_COUNT as f32;
        let x = SPAWN_POINT[0] + angle.cos() * 4.0;
        let y = SPAWN_POINT[1] + angle.sin() * 4.0;
        let entity_id = alloc_entity_id(ctx);
        write_coarse(ctx, entity_id, 1, COARSE_KIND_NPC, x, y, NPC_SPAWN_Z, now);
        ctx.db.npc().insert(Npc {
            entity_id,
            generation: 1,
            x,
            y,
            z: NPC_SPAWN_Z,
            yaw: angle,
            hp: NPC_HP_MAX,
            hp_max: NPC_HP_MAX,
            alive: true,
            respawn_at_micros: 0,
            cell_id: cell_id_from_position(x, y),
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

/// Dev/test reducer (M6 D5): replays a named embedded motion trace against
/// the embedded collision — actual-WASM parity. `replay` errors (which fail
/// the reducer) if any step diverges from the natively recorded expected
/// positions by more than `TRACE_TOLERANCE`; success records the outcome in
/// `parity_result` for the native harness to cross-check.
#[reducer]
pub fn run_parity_trace(ctx: &ReducerContext, trace_id: String) -> Result<(), String> {
    let store = collision_store().ok_or("embedded collision failed to load")?;
    let trace = motion::trace::load_embedded(&trace_id)?;
    let report = {
        let _watch = spacetimedb::log_stopwatch::LogStopwatch::new("parity_trace_replay");
        trace.replay(store)?
    };
    log::info!(
        "parity trace {trace_id:?}: {} steps, end {:?}, max err {:.6}",
        report.steps,
        report.end_pos,
        report.max_error
    );
    let row = ParityResult {
        trace_id,
        steps: report.steps,
        end_x: report.end_pos.x,
        end_y: report.end_pos.y,
        end_z: report.end_pos.z,
        state_hash: report.state_hash,
    };
    if ctx.db.parity_result().trace_id().find(&row.trace_id).is_some() {
        ctx.db.parity_result().trace_id().update(row);
    } else {
        ctx.db.parity_result().insert(row);
    }
    Ok(())
}
