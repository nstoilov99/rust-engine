//! Game server module (M5 Net-A).
//!
//! Identity, session and lifecycle semantics are normative in
//! `docs/roadmap/M5-WORLD-IDENTITY-CONTRACT.md`; protocol constants come
//! from `game_shared::net::schema` (single source of truth).
//!
//! These tables ARE the save system — player rows are never deleted on
//! disconnect.

use game_shared::collision::{manifest_hash, ChunkStore};
use game_shared::motion::{self, MotionConfig, MotionState, MoveIntent};
use game_shared::net::schema::{accept_input, PROTOCOL_VERSION, REALM_ID, TOMBSTONE_TTL_SECS};
use game_shared::world_grid::zone_id_from_position;
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
    /// M6: controller grounding, part of the atomic own-state ack.
    grounded: bool,
    #[index(btree)]
    zone_id: u32,
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
            ctx.db.player().insert(Player {
                entity_id: alloc_entity_id(ctx),
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
                zone_id: zone_id_from_position(SPAWN_POINT[0], SPAWN_POINT[1]),
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
    // M6: stale queued intent must not walk the player away from the target;
    // grounding is re-established by the next movement tick. Seq counters
    // stay intact (the session did not restart).
    p.grounded = false;
    p.zone_id = zone_id_from_position(x, y);
    p.last_update_micros = now_micros(ctx);
    let entity_id = p.entity_id;
    ctx.db.player().entity_id().update(p);
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
    for mut p in players {
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
            p.zone_id = zone_id_from_position(p.x, p.y);
            p.last_update_micros = now;
            ctx.db.player().entity_id().update(p);
        }
    }
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
