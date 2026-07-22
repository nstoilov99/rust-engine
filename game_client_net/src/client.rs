//! `SpacetimeNetClient`: the SpacetimeDB implementation of `NetClient`.
//!
//! State machine (plan D3): Offline → Connecting → AwaitBaseSub →
//! VersionCheck → InWorld, with any state collapsing to Disconnected.
//! SDK callbacks run inside `frame_tick()` on the game thread, but must be
//! `Send + 'static`, so they only write `Flags`; all decisions happen in
//! `poll()`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use game_shared::net::protocol::{
    ClientInput, ClockSample, CoarseState, EntityKind, EntityState, ModuleAddr, WorldSnapshot,
};
use game_shared::net::schema::{version_compatible, PROTOCOL_VERSION};
use game_shared::net::traits::{ConnectionState, DisconnectReason, NetClient, NetEvent};
use game_shared::world_grid::{cell_key, chunk_coord, far_cells, near_cells, re_anchor};
use glam::{IVec2, Vec2, Vec3};
use spacetimedb_sdk::{
    credentials, DbContext, SubscriptionHandle as _, Table, TableWithPrimaryKey,
};

use crate::module_bindings::{
    cast_ability, despawn_npc, dev_damage, dev_teleport, enter_world, ping, run_parity_trace,
    submit_input, AbilityCooldownTableAccess, ActiveCastTableAccess, ConfigTableAccess,
    DbConnection, EntityCoarseTableAccess, Npc, NpcTableAccess, ParityResultTableAccess,
    PingResultTableAccess, Player, PlayerTableAccess, ProjectileTableAccess, SubscriptionHandle,
    TombstoneTableAccess,
};

/// Spike binding requirement 3: hard client-side cap on reducer calls.
const MAX_REDUCER_CALLS_PER_SEC: f32 = 30.0;

/// Clock sync ping cadence (plan D5).
const PING_INTERVAL_SECS: f32 = 2.0;

/// M9.6: connect-to-InWorld budget. Generous vs the observed WAN handshake
/// (~2 s to Maincloud) but finite, so a dead host surfaces as a disconnect.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);

/// Monotonic local timeline in microseconds. The backend stamps ping
/// round-trips on it and `ClockSample.offset_us` is relative to it, so the
/// game-side `NetClock` must use this same function.
pub fn local_time_us() -> u64 {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    EPOCH.get_or_init(Instant::now).elapsed().as_micros() as u64
}

/// Last-server-step view of an in-flight projectile (M7 D4): the renderer
/// extrapolates `pos + vel·dt − ½·g·dt²·ẑ` from `server_time_us`.
#[derive(Debug, Clone, Copy)]
pub struct ProjectileView {
    pub entity_id: u64,
    pub pos: [f32; 3],
    pub vel: [f32; 3],
    pub server_time_us: u64,
}

/// Own combat state polled per frame by the HUD (M7 D6) — no events;
/// the HUD re-reads everything each frame anyway. Times are server micros.
#[derive(Debug, Clone, Default)]
pub struct OwnCombat {
    pub hp: f32,
    pub hp_max: f32,
    pub mana: f32,
    pub mana_max: f32,
    pub alive: bool,
    pub gcd_until_us: u64,
    pub respawn_at_us: u64,
    /// `(ability_id, ready_at_us)` for every live cooldown row.
    pub cooldowns: Vec<(u16, u64)>,
    pub active_cast: Option<ActiveCastView>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ActiveCastView {
    pub ability_id: u16,
    pub target_entity_id: u64,
    pub start_us: u64,
    pub finish_us: u64,
}

#[derive(Default)]
struct Flags {
    connected: AtomicBool,
    disconnected: AtomicBool,
    base_applied: AtomicBool,
    /// The in-flight cell subscription's `on_applied` fired (one at a time).
    pending_repl_applied: AtomicBool,
    /// Replicated rows changed since the last snapshot (plan D3: callbacks
    /// only mark dirty; the cache-diff is the sole spawn/despawn authority).
    cache_dirty: AtomicBool,
    /// The presented stored token was rejected (contract §4.1: delete it).
    auth_rejected: AtomicBool,
    /// Tombstone evidence observed by row callbacks (contract §3.2).
    tombstones: Mutex<Vec<(u64, u32)>>,
    /// Interpolation samples queued by player/NPC row callbacks (plan D6);
    /// drained after the snapshot so samples never precede proxy creation.
    samples: Mutex<Vec<EntityState>>,
    /// Completed ping rows: (nonce, server_time_us, recv_local_us).
    pongs: Mutex<Vec<(u64, u64, u64)>>,
    /// Last transport/subscription error message, if any.
    error: Mutex<Option<String>>,
}

impl Flags {
    fn set_error(&self, msg: String) {
        if let Ok(mut e) = self.error.lock() {
            *e = Some(msg);
        }
    }

    fn take_error(&self) -> Option<String> {
        self.error.lock().ok().and_then(|mut e| e.take())
    }

    fn mark_dirty(&self) {
        self.cache_dirty.store(true, Ordering::Release);
    }

    fn push_tombstone(&self, entity_id: u64, generation: u32) {
        if let Ok(mut ts) = self.tombstones.lock() {
            ts.push((entity_id, generation));
        }
    }

    fn push_sample(&self, state: EntityState) {
        if let Ok(mut s) = self.samples.lock() {
            s.push(state);
        }
    }

    fn push_pong(&self, nonce: u64, server_time_us: u64) {
        if let Ok(mut p) = self.pongs.lock() {
            p.push((nonce, server_time_us, local_time_us()));
        }
    }
}

struct RateLimiter {
    tokens: f32,
    last: Instant,
}

impl RateLimiter {
    fn new() -> Self {
        Self {
            tokens: MAX_REDUCER_CALLS_PER_SEC,
            last: Instant::now(),
        }
    }

    fn allow(&mut self) -> bool {
        let now = Instant::now();
        let dt = now.duration_since(self.last).as_secs_f32();
        self.last = now;
        self.tokens = (self.tokens + dt * MAX_REDUCER_CALLS_PER_SEC).min(MAX_REDUCER_CALLS_PER_SEC);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// (epoch, last_applied_seq, pos, vel, yaw, grounded, alive) — the own-row
/// fields that constitute one atomic ack (M6 D4; alive per M7 D5).
type OwnState = (u32, u32, [f32; 3], [f32; 3], f32, bool, bool);

pub struct SpacetimeNetClient {
    state: ConnectionState,
    conn: Option<DbConnection>,
    base_sub: Option<SubscriptionHandle>,
    /// Active interest subscription and the anchor cell it is centered on
    /// (M8 D2: 3×3 full-detail ring around the anchor).
    repl_sub: Option<SubscriptionHandle>,
    repl_anchor: Option<IVec2>,
    /// In-flight anchor swap: the new set's subscription, promoted to
    /// `repl_sub` (and the old one dropped) only once applied — plan D3 §6
    /// apply-new-then-drop-old, so scoped rows never gap.
    pending_repl: Option<(IVec2, SubscriptionHandle)>,
    /// Latest predicted own XY from `set_interest_hint`; anchors lead the
    /// authoritative row so subscribe latency hides behind movement.
    interest_hint: Option<[f32; 2]>,
    /// Completed anchor swaps (test observability: hysteresis must keep
    /// border wiggle from thrashing this).
    interest_swaps: u32,
    flags: Arc<Flags>,
    pending: Vec<NetEvent>,
    limiter: RateLimiter,
    enter_world_sent: bool,
    /// Queued input samples (M6 D4: one per prediction step, seq stamped by
    /// the caller). Drained in `poll`; cleared on epoch change so a rejoin
    /// can never send stale-seq inputs under the new epoch.
    pending_inputs: Vec<ClientInput>,
    /// Epoch stamped onto outgoing inputs, mirrored from the own row.
    input_epoch: u32,
    /// Last own-row state emitted as `OwnStateAck` — `None` until the row
    /// is first seen, so the first sight always emits.
    last_own_state: Option<OwnState>,
    last_ping: Option<Instant>,
    ping_nonce: u64,
    /// In-flight ping: (nonce, send_local_us). One at a time; a new ping
    /// abandons an unanswered one (stale pongs are nonce-rejected).
    pending_ping: Option<(u64, u64)>,
    /// Credentials file key of the current connection (token cleanup).
    credentials_key: Option<String>,
    /// Extra key suffix so multiple local instances get distinct identities.
    net_id: Option<String>,
    /// Overridable for acceptance tests (version-mismatch path).
    client_version: u32,
    /// Local collision manifest hash (M6 D1). `None` = no local collision
    /// content → the gate is skipped (dev tools, bots).
    expected_collision_hash: Option<u64>,
    /// Local build stamp (M9 D4). `None` = check skipped. Mismatch is
    /// advisory (`NetEvent::BuildMismatch`), never a refusal.
    expected_build_id: Option<String>,
    /// M9.6: the handshake must complete by this instant or the connection
    /// fails — without it a dead host leaves the client in `Connecting`
    /// forever (no elapsed-time check anywhere else in `poll`).
    handshake_deadline: Option<Instant>,
}

impl SpacetimeNetClient {
    pub fn new() -> Self {
        Self {
            state: ConnectionState::Offline,
            conn: None,
            base_sub: None,
            repl_sub: None,
            repl_anchor: None,
            pending_repl: None,
            interest_hint: None,
            interest_swaps: 0,
            flags: Arc::new(Flags::default()),
            pending: Vec::new(),
            limiter: RateLimiter::new(),
            enter_world_sent: false,
            pending_inputs: Vec::new(),
            input_epoch: 0,
            last_own_state: None,
            last_ping: None,
            ping_nonce: 0,
            pending_ping: None,
            credentials_key: None,
            net_id: None,
            client_version: PROTOCOL_VERSION,
            expected_collision_hash: None,
            expected_build_id: None,
            handshake_deadline: None,
        }
    }

    /// Gate the connection on the server's collision manifest hash matching
    /// `hash` (of the locally loaded manifest, M6 D1). Mismatch = clean
    /// refusal at version check — the two sides would simulate against
    /// different geometry.
    pub fn set_expected_collision_hash(&mut self, hash: u64) {
        self.expected_collision_hash = Some(hash);
    }

    /// Enable the advisory build-stamp comparison (M9 D4): a differing
    /// server `build_id` emits `NetEvent::BuildMismatch` but still connects.
    pub fn set_expected_build_id(&mut self, id: String) {
        self.expected_build_id = Some(id);
    }

    /// Use a separate credentials file (→ separate identity) per name, so
    /// multiple local instances can be in world as different players.
    pub fn set_net_id(&mut self, id: String) {
        self.net_id = Some(id);
    }

    /// Test hook: pretend to be a different protocol version.
    pub fn set_client_version(&mut self, version: u32) {
        self.client_version = version;
    }

    /// Dev/test hook for the unauthenticated `dev_teleport` reducer:
    /// teleports the caller's own player (acceptance tests cross cell
    /// borders deterministically with this).
    pub fn dev_teleport(&mut self, pos: [f32; 3]) {
        if let Some(conn) = &self.conn {
            let _ = conn.reducers.dev_teleport(pos[0], pos[1], pos[2]);
        }
    }

    /// Dev/test hook for the unauthenticated `despawn_npc` reducer
    /// (destroyed-vs-out-of-scope acceptance scenario).
    pub fn dev_despawn_npc(&mut self, entity_id: u64) {
        if let Some(conn) = &self.conn {
            let _ = conn.reducers.despawn_npc(entity_id);
        }
    }

    /// Dev/test hook for the unauthenticated `dev_damage` reducer
    /// (death/respawn acceptance scenarios).
    pub fn dev_damage(&mut self, entity_id: u64, amount: f32) {
        if let Some(conn) = &self.conn {
            let _ = conn.reducers.dev_damage(entity_id, amount);
        }
    }

    /// M7 D6: request a cast. Fire-and-forget — rejections are server-side
    /// only; the client observes effects (hp/mana/cooldown rows).
    pub fn cast_ability(&mut self, ability_id: u16, target_entity_id: u64) {
        if let Some(conn) = &self.conn {
            let _ = conn.reducers.cast_ability(ability_id, target_entity_id);
        }
    }

    /// Own combat state from the client cache (M7 D6), polled per frame.
    /// `None` until the own row is in the cache.
    pub fn own_combat(&self) -> Option<OwnCombat> {
        let conn = self.conn.as_ref()?;
        let identity = conn.try_identity()?;
        let p = conn.db.player().owner_identity().find(&identity)?;
        let cooldowns = conn
            .db
            .ability_cooldown()
            .iter()
            .filter(|c| c.entity_id == p.entity_id)
            .map(|c| (c.ability_id, c.ready_at_micros.max(0) as u64))
            .collect();
        let active_cast = conn
            .db
            .active_cast()
            .entity_id()
            .find(&p.entity_id)
            .map(|c| ActiveCastView {
                ability_id: c.ability_id,
                target_entity_id: c.target_entity_id,
                start_us: c.start_micros.max(0) as u64,
                finish_us: c.finish_micros.max(0) as u64,
            });
        Some(OwnCombat {
            hp: p.hp,
            hp_max: p.hp_max,
            mana: p.mana,
            mana_max: p.mana_max,
            alive: p.alive,
            gcd_until_us: p.gcd_until_micros.max(0) as u64,
            respawn_at_us: p.respawn_at_micros.max(0) as u64,
            cooldowns,
            active_cast,
        })
    }

    /// Active cast of any entity in scope (target frame cast bar).
    pub fn active_cast_of(&self, entity_id: u64) -> Option<ActiveCastView> {
        let conn = self.conn.as_ref()?;
        conn.db
            .active_cast()
            .entity_id()
            .find(&entity_id)
            .map(|c| ActiveCastView {
                ability_id: c.ability_id,
                target_entity_id: c.target_entity_id,
                start_us: c.start_micros.max(0) as u64,
                finish_us: c.finish_micros.max(0) as u64,
            })
    }

    /// Dev/test hook: raw `submit_input` with caller-controlled epoch AND
    /// seq, bypassing the own-row epoch stamping — the exploit battery
    /// forges stale and foreign values through this.
    pub fn dev_submit_input_raw(&mut self, epoch: u32, seq: u32, move_dir: [f32; 2], yaw: f32) {
        if let Some(conn) = &self.conn {
            let _ = conn
                .reducers
                .submit_input(epoch, seq, move_dir[0], move_dir[1], yaw, false, false);
        }
    }

    /// In-flight projectiles from the client cache (cell-scoped sub). The
    /// renderer extrapolates from the last server step (M7 D4) — polled per
    /// frame, no events.
    pub fn projectiles(&self) -> Vec<ProjectileView> {
        let Some(conn) = &self.conn else {
            return Vec::new();
        };
        conn.db
            .projectile()
            .iter()
            .map(|p| ProjectileView {
                entity_id: p.entity_id,
                pos: [p.x, p.y, p.z],
                vel: [p.vx, p.vy, p.vz],
                server_time_us: p.last_update_micros.max(0) as u64,
            })
            .collect()
    }

    /// Dev/test hook: replay the embedded parity trace inside the WASM
    /// module (M6 D5); the result upserts into `parity_result`.
    pub fn dev_run_parity_trace(&mut self, trace_id: &str) {
        if let Some(conn) = &self.conn {
            let _ = conn.reducers.run_parity_trace(trace_id.to_string());
        }
    }

    /// Dev/test reader for the WASM-side replay result:
    /// `(steps, end_pos, state_hash)`.
    pub fn dev_parity_result(&self, trace_id: &str) -> Option<(u32, [f32; 3], u64)> {
        let conn = self.conn.as_ref()?;
        conn.db
            .parity_result()
            .trace_id()
            .find(&trace_id.to_string())
            .map(|r| (r.steps, [r.end_x, r.end_y, r.end_z], r.state_hash))
    }

    pub fn identity_hex(&self) -> Option<String> {
        self.conn
            .as_ref()
            .and_then(|c| c.try_identity())
            .map(|id| id.to_hex().to_string())
    }

    fn credentials_key(&self, module: &str) -> String {
        match &self.net_id {
            Some(id) => format!("rust-engine-{module}-{id}"),
            None => format!("rust-engine-{module}"),
        }
    }

    /// The SDK has no delete API; remove the token file it writes under
    /// `~/.spacetimedb_client_credentials/<key>` (contract §4.1: a rejected
    /// token is deleted so the next connect is fresh).
    fn delete_credentials(key: &str) {
        let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"));
        if let Some(home) = home {
            let path = std::path::Path::new(&home)
                .join(".spacetimedb_client_credentials")
                .join(key);
            let _ = std::fs::remove_file(path);
        }
    }

    fn teardown(&mut self) {
        self.handshake_deadline = None;
        self.base_sub = None;
        self.repl_sub = None;
        self.repl_anchor = None;
        self.pending_repl = None;
        self.enter_world_sent = false;
        self.pending_inputs.clear();
        self.input_epoch = 0;
        self.last_own_state = None;
        self.last_ping = None;
        self.pending_ping = None;
        if let Some(conn) = self.conn.take() {
            conn.disconnect().ok();
        }
        self.state = ConnectionState::Disconnected;
    }

    fn fail(&mut self, out: &mut Vec<NetEvent>, reason: DisconnectReason) {
        out.push(NetEvent::Disconnected(reason));
        self.teardown();
    }

    fn subscribe_base(&mut self) {
        let conn = self.conn.as_ref().expect("subscribe_base requires a connection");
        let hex = conn
            .try_identity()
            .expect("subscribe_base requires an identity")
            .to_hex()
            .to_string();
        // Permanent subscription set (plan D3): config + tombstones + own rows.
        let queries = vec![
            "SELECT * FROM config".to_string(),
            "SELECT * FROM tombstone".to_string(),
            format!("SELECT * FROM account WHERE identity = 0x{hex}"),
            format!("SELECT * FROM player WHERE owner_identity = 0x{hex}"),
            format!("SELECT * FROM ping_result WHERE identity = 0x{hex}"),
            // Dev/test only (≤10 tiny rows): WASM parity replay results.
            "SELECT * FROM parity_result".to_string(),
        ];
        let applied = self.flags.clone();
        let errored = self.flags.clone();
        let handle = conn
            .subscription_builder()
            .on_applied(move |_ctx| applied.base_applied.store(true, Ordering::Release))
            .on_error(move |_ctx, err| {
                errored.set_error(format!("base subscription failed: {err}"));
                errored.disconnected.store(true, Ordering::Release);
            })
            .subscribe(queries);
        self.base_sub = Some(handle);
    }

    /// M8 D2 interest pump: subscriptions follow a hysteresis-stabilized
    /// anchor cell (`re_anchor`: the anchor only moves once the position is
    /// > 8 m outside its AABB, so border wiggle never thrashes). The anchor
    /// tracks the *predicted* position when a hint is set, the authoritative
    /// own row otherwise.
    ///
    /// Promotes an applied pending swap first; the SDK ref-counts rows under
    /// overlapping queries, so shared rows never leave the cache during the
    /// overlap and out-of-ring rows are dropped (out-of-scope, no tombstone)
    /// only after the new set is fully applied.
    fn pump_interest(&mut self, own_pos: [f32; 2]) {
        if self.pending_repl.is_some() {
            if self.flags.pending_repl_applied.load(Ordering::Acquire) {
                let (anchor, handle) = self.pending_repl.take().expect("checked above");
                if let Some(old) = self.repl_sub.take() {
                    let _ = old.unsubscribe();
                }
                self.repl_sub = Some(handle);
                self.repl_anchor = Some(anchor);
                self.interest_swaps += 1;
                self.flags.mark_dirty();
            }
            return; // one swap in flight; re-checked next poll
        }
        let pos = Vec2::from(self.interest_hint.unwrap_or(own_pos));
        match self.repl_anchor {
            None => {
                let anchor = chunk_coord(Vec3::new(pos.x, pos.y, 0.0));
                self.start_interest_sub(anchor);
            }
            Some(anchor) => {
                if let Some(new_anchor) = re_anchor(anchor, pos) {
                    self.start_interest_sub(new_anchor);
                }
            }
        }
    }

    /// Completed interest swaps (acceptance-test observability).
    pub fn interest_swap_count(&self) -> u32 {
        self.interest_swaps
    }

    fn start_interest_sub(&mut self, anchor: IVec2) {
        let conn = self
            .conn
            .as_ref()
            .expect("start_interest_sub requires a connection");
        let mut queries = Vec::with_capacity(9 * 4 + 40 + 1);
        for cell in near_cells(anchor) {
            let key = cell_key(cell);
            queries.push(format!("SELECT * FROM player WHERE cell_id = {key}"));
            queries.push(format!("SELECT * FROM npc WHERE cell_id = {key}"));
            queries.push(format!("SELECT * FROM projectile WHERE cell_id = {key}"));
            // M7 D6: in-scope cast bars (rows carry the caster's cell).
            queries.push(format!("SELECT * FROM active_cast WHERE cell_id = {key}"));
        }
        // M8 D3: far ring gets coarse position rows only. The query-set size
        // (~77) is the acknowledged risk the package-5 harness measures.
        for cell in far_cells(anchor) {
            let key = cell_key(cell);
            queries.push(format!("SELECT * FROM entity_coarse WHERE cell_id = {key}"));
        }
        // Own cooldown rows (HUD sweeps). The own row is always in the cache
        // by the time the first interest sub starts (it carries the pos).
        if let Some(p) = conn
            .try_identity()
            .and_then(|id| conn.db.player().owner_identity().find(&id))
        {
            queries.push(format!(
                "SELECT * FROM ability_cooldown WHERE entity_id = {}",
                p.entity_id
            ));
        }
        self.flags
            .pending_repl_applied
            .store(false, Ordering::Release);
        let applied = self.flags.clone();
        let errored = self.flags.clone();
        let handle = conn
            .subscription_builder()
            .on_applied(move |_ctx| {
                applied.pending_repl_applied.store(true, Ordering::Release);
                applied.mark_dirty();
            })
            .on_error(move |_ctx, err| {
                errored.set_error(format!("interest subscription failed: {err}"));
                errored.disconnected.store(true, Ordering::Release);
            })
            .subscribe(queries);
        self.pending_repl = Some((anchor, handle));
    }

    /// Row callbacks mark dirty / record tombstone evidence / queue
    /// interpolation samples; all spawn/despawn decisions still belong to
    /// the cache-diff (plan D3).
    fn register_row_callbacks(conn: &DbConnection, flags: &Arc<Flags>) {
        macro_rules! sample_on {
            ($table:expr, $to_state:expr) => {{
                let f = flags.clone();
                $table.on_insert(move |_, row| {
                    f.push_sample($to_state(row));
                    f.mark_dirty();
                });
                let f = flags.clone();
                $table.on_update(move |_, _, row| {
                    f.push_sample($to_state(row));
                    f.mark_dirty();
                });
                let f = flags.clone();
                $table.on_delete(move |_, _| f.mark_dirty());
            }};
        }
        sample_on!(conn.db.player(), Self::player_state);
        sample_on!(conn.db.npc(), Self::npc_state);
        // Coarse rows only dirty the snapshot — no interp samples (M8 D3).
        let f = flags.clone();
        conn.db.entity_coarse().on_insert(move |_, _| f.mark_dirty());
        let f = flags.clone();
        conn.db.entity_coarse().on_update(move |_, _, _| f.mark_dirty());
        let f = flags.clone();
        conn.db.entity_coarse().on_delete(move |_, _| f.mark_dirty());
        let f = flags.clone();
        conn.db.tombstone().on_insert(move |_, row| {
            f.push_tombstone(row.entity_id, row.generation);
        });
        let f = flags.clone();
        conn.db.tombstone().on_update(move |_, _, row| {
            f.push_tombstone(row.entity_id, row.generation);
        });
        let f = flags.clone();
        conn.db.ping_result().on_insert(move |_, row| {
            f.push_pong(row.nonce, row.server_time_micros.max(0) as u64);
        });
        let f = flags.clone();
        conn.db.ping_result().on_update(move |_, _, row| {
            f.push_pong(row.nonce, row.server_time_micros.max(0) as u64);
        });
    }

    fn player_state(p: &Player) -> EntityState {
        EntityState {
            entity_id: p.entity_id,
            generation: p.generation,
            kind: EntityKind::Player,
            pos: [p.x, p.y, p.z],
            vel: [p.vx, p.vy, p.vz],
            yaw: p.yaw,
            cell_id: p.cell_id,
            hp: p.hp,
            hp_max: p.hp_max,
            alive: p.alive,
            server_time_us: p.last_update_micros.max(0) as u64,
        }
    }

    fn npc_state(n: &Npc) -> EntityState {
        EntityState {
            entity_id: n.entity_id,
            generation: n.generation,
            kind: EntityKind::Npc,
            pos: [n.x, n.y, n.z],
            vel: [0.0; 3],
            yaw: n.yaw,
            cell_id: n.cell_id,
            hp: n.hp,
            hp_max: n.hp_max,
            alive: n.alive,
            server_time_us: n.last_update_micros.max(0) as u64,
        }
    }

    fn build_snapshot(conn: &DbConnection) -> WorldSnapshot {
        let own_identity = conn.try_identity();
        let mut entities = Vec::new();
        let mut own_entity_id = None;
        for p in conn.db.player().iter() {
            if own_identity == Some(p.owner_identity) {
                own_entity_id = Some(p.entity_id);
            }
            entities.push(Self::player_state(&p));
        }
        for n in conn.db.npc().iter() {
            entities.push(Self::npc_state(&n));
        }
        let coarse = conn
            .db
            .entity_coarse()
            .iter()
            .map(|c| CoarseState {
                entity_id: c.entity_id,
                generation: c.generation,
                kind: if c.kind == 1 {
                    EntityKind::Npc
                } else {
                    EntityKind::Player
                },
                pos: [c.x, c.y, c.z],
                server_time_us: c.updated_micros.max(0) as u64,
            })
            .collect();
        WorldSnapshot {
            entities,
            coarse,
            own_entity_id,
        }
    }
}

impl Default for SpacetimeNetClient {
    fn default() -> Self {
        Self::new()
    }
}

impl NetClient for SpacetimeNetClient {
    fn connect(&mut self, addr: &ModuleAddr) {
        if self.conn.is_some() {
            return;
        }
        self.flags = Arc::new(Flags::default());
        self.pending.clear();

        let key = self.credentials_key(&addr.module);
        self.credentials_key = Some(key.clone());
        let token = match credentials::File::new(key.clone()).load() {
            Ok(t) => t,
            Err(e) => {
                log::warn!("failed to load stored credentials ({e}); connecting fresh");
                None
            }
        };
        let presented_token = token.is_some();

        let on_connect = self.flags.clone();
        let on_conn_err = self.flags.clone();
        let on_disc = self.flags.clone();
        let result = DbConnection::builder()
            .with_uri(addr.host.clone())
            .with_database_name(addr.module.clone())
            .with_token(token)
            .on_connect(move |_conn, _identity, tok| {
                if let Err(e) = credentials::File::new(key).save(tok) {
                    log::warn!("failed to persist credentials: {e}");
                }
                on_connect.connected.store(true, Ordering::Release);
            })
            .on_connect_error(move |_ctx, err| {
                let msg = format!("connect failed: {err}");
                // Contract §4.1: a rejected stored token is deleted so the
                // next attempt connects fresh.
                let lower = msg.to_lowercase();
                if presented_token && (lower.contains("token") || lower.contains("auth")) {
                    on_conn_err.auth_rejected.store(true, Ordering::Release);
                }
                on_conn_err.set_error(msg);
                on_conn_err.disconnected.store(true, Ordering::Release);
            })
            .on_disconnect(move |_ctx, err| {
                if let Some(err) = err {
                    on_disc.set_error(err.to_string());
                }
                on_disc.disconnected.store(true, Ordering::Release);
            })
            .build();

        match result {
            Ok(conn) => {
                Self::register_row_callbacks(&conn, &self.flags);
                self.conn = Some(conn);
                self.state = ConnectionState::Connecting;
                self.handshake_deadline = Some(Instant::now() + HANDSHAKE_TIMEOUT);
            }
            Err(e) => {
                self.state = ConnectionState::Disconnected;
                self.pending.push(NetEvent::Disconnected(DisconnectReason::ConnectionLost(
                    format!("connect failed: {e}"),
                )));
            }
        }
    }

    fn disconnect(&mut self) {
        if self.conn.is_none() {
            return;
        }
        self.teardown();
        self.pending
            .push(NetEvent::Disconnected(DisconnectReason::UserRequested));
    }

    fn connection_state(&self) -> ConnectionState {
        self.state
    }

    fn send_input(&mut self, input: &ClientInput) {
        // One sample per prediction step (M6 D4); seq is caller-owned,
        // epoch is stamped in `poll` from the authoritative own row.
        if self.state == ConnectionState::InWorld {
            self.pending_inputs.push(*input);
        }
    }

    fn set_interest_hint(&mut self, pos: [f32; 2]) {
        if pos[0].is_finite() && pos[1].is_finite() {
            self.interest_hint = Some(pos);
        }
    }

    fn poll(&mut self, out: &mut Vec<NetEvent>) {
        out.append(&mut self.pending);

        let Some(conn) = self.conn.as_ref() else {
            return;
        };

        if let Err(e) = conn.frame_tick() {
            let msg = self.flags.take_error().unwrap_or_else(|| e.to_string());
            self.fail(out, DisconnectReason::ConnectionLost(msg));
            return;
        }

        if self.flags.disconnected.load(Ordering::Acquire) {
            if self.flags.auth_rejected.load(Ordering::Acquire) {
                if let Some(key) = &self.credentials_key {
                    log::warn!("stored auth token rejected; deleting credentials '{key}'");
                    Self::delete_credentials(key);
                }
            }
            let msg = self
                .flags
                .take_error()
                .unwrap_or_else(|| "connection closed".to_string());
            self.fail(out, DisconnectReason::ConnectionLost(msg));
            return;
        }

        // M9.6: handshake deadline — every pre-InWorld state waits on some
        // asynchronous progress (identity, subscription, own row); a host
        // that stops responding mid-handshake would otherwise hang forever.
        if self.handshake_deadline.is_some_and(|d| Instant::now() > d) {
            self.fail(
                out,
                DisconnectReason::ConnectionLost(format!(
                    "handshake timeout after {}s",
                    HANDSHAKE_TIMEOUT.as_secs()
                )),
            );
            return;
        }

        // Tombstone evidence is drained in every state (contract §3.2):
        // destruction + GC arriving in one pump must still reach the client.
        if let Ok(mut ts) = self.flags.tombstones.lock() {
            for (entity_id, generation) in ts.drain(..) {
                out.push(NetEvent::TombstoneSeen {
                    entity_id,
                    generation,
                });
            }
        }

        match self.state {
            ConnectionState::Connecting => {
                let identity_ready = self.flags.connected.load(Ordering::Acquire)
                    && self.conn.as_ref().is_some_and(|c| c.try_identity().is_some());
                if identity_ready {
                    self.subscribe_base();
                    self.state = ConnectionState::AwaitBaseSub;
                }
            }
            ConnectionState::AwaitBaseSub => {
                if self.flags.base_applied.load(Ordering::Acquire) {
                    self.state = ConnectionState::VersionCheck;
                }
            }
            ConnectionState::VersionCheck => {
                let Some(config) = conn.db.config().id().find(&0) else {
                    self.fail(
                        out,
                        DisconnectReason::ConnectionLost(
                            "config row missing after base subscription".to_string(),
                        ),
                    );
                    return;
                };
                if !version_compatible(config.protocol_version, self.client_version) {
                    self.fail(
                        out,
                        DisconnectReason::VersionMismatch {
                            server: config.protocol_version,
                            client: self.client_version,
                        },
                    );
                    return;
                }
                // M6 D1: refuse to simulate against different geometry.
                if let Some(expected) = self.expected_collision_hash {
                    if config.collision_manifest_hash != expected {
                        self.fail(
                            out,
                            DisconnectReason::CollisionMismatch {
                                server: config.collision_manifest_hash,
                                client: expected,
                            },
                        );
                        return;
                    }
                }
                // M9 D4: soft stamp — warn-only; protocol version above is
                // the hard gate.
                if let Some(expected) = &self.expected_build_id {
                    if &config.build_id != expected {
                        out.push(NetEvent::BuildMismatch {
                            server: config.build_id.clone(),
                            client: expected.clone(),
                        });
                    }
                }
                // M9.6: gates passed — announce the server's world so the
                // game can load the matching scene.
                out.push(NetEvent::WorldScene(config.world_scene.clone()));
                self.state = ConnectionState::EnteringWorld;
            }
            ConnectionState::EnteringWorld => {
                // Liveness-guarded (we only get here past the disconnect
                // checks) and rate-limited like every reducer call.
                if !self.enter_world_sent {
                    if !self.limiter.allow() {
                        return;
                    }
                    if let Err(e) = conn.reducers.enter_world() {
                        self.fail(
                            out,
                            DisconnectReason::ConnectionLost(format!("enter_world failed: {e}")),
                        );
                        return;
                    }
                    self.enter_world_sent = true;
                }
                // Wait for OUR session on the own row, not just the row: a
                // stale row from a previous connection would otherwise trip
                // the InWorld session-replaced check before our own
                // `enter_world` round-trips.
                let own = conn
                    .try_identity()
                    .and_then(|id| conn.db.player().owner_identity().find(&id));
                let own_session_live = own
                    .as_ref()
                    .is_some_and(|p| p.session.is_some() && p.session == conn.try_connection_id());
                if own_session_live {
                    // The own row carries the authoritative position, so the
                    // first interest subscription can only start here.
                    let p = own.expect("session live implies own row");
                    self.pump_interest([p.x, p.y]);
                    if self.repl_anchor.is_some() {
                        self.state = ConnectionState::InWorld;
                        self.handshake_deadline = None;
                        self.flags.mark_dirty();
                        out.push(NetEvent::Connected);
                    }
                }
            }
            ConnectionState::InWorld => {
                // Session revocation (contract §4.3): our own row's session
                // no longer names this connection → another connection of
                // the same identity took over.
                let own = conn
                    .try_identity()
                    .and_then(|id| conn.db.player().owner_identity().find(&id));
                if let (Some(own), Some(my_conn)) = (&own, conn.try_connection_id()) {
                    if own.session != Some(my_conn) {
                        self.fail(out, DisconnectReason::SessionReplaced);
                        return;
                    }
                }

                // Snapshot before samples so a sample can never reach the
                // game before the proxy it belongs to exists (plan D6).
                if self.flags.cache_dirty.swap(false, Ordering::AcqRel) {
                    out.push(NetEvent::Snapshot(Self::build_snapshot(conn)));
                }
                if let Ok(mut samples) = self.flags.samples.lock() {
                    for state in samples.drain(..) {
                        out.push(NetEvent::StateUpdate(state));
                    }
                }

                if let Some(own) = &own {
                    // Row epoch change (rejoin) restarts the sequence space
                    // (contract §5). Drop queued inputs — their seqs belong
                    // to the old epoch.
                    if own.epoch != self.input_epoch {
                        self.input_epoch = own.epoch;
                        self.pending_inputs.clear();
                    }
                    // Replication of the own row IS the ack (M6 D4): emit
                    // whenever its simulated state or counters change,
                    // including the first time the row is seen.
                    let state: OwnState = (
                        own.epoch,
                        own.last_applied_seq,
                        [own.x, own.y, own.z],
                        [own.vx, own.vy, own.vz],
                        own.yaw,
                        own.grounded,
                        own.alive,
                    );
                    if self.last_own_state != Some(state) {
                        self.last_own_state = Some(state);
                        out.push(NetEvent::OwnStateAck {
                            epoch: state.0,
                            seq: state.1,
                            pos: state.2,
                            vel: state.3,
                            yaw: state.4,
                            grounded: state.5,
                            alive: state.6,
                        });
                    }

                    // Forward queued input samples without re-stamping or
                    // coalescing (M6 D4). A failed call is dropped, never
                    // retried (contract §5).
                    for input in self.pending_inputs.drain(..) {
                        if !self.limiter.allow() {
                            break;
                        }
                        let _ = conn.reducers.submit_input(
                            self.input_epoch,
                            input.seq,
                            input.move_dir[0],
                            input.move_dir[1],
                            input.yaw,
                            input.sprint,
                            input.jump,
                        );
                    }
                }

                // Clock sync ping (plan D5): one in flight; a new ping
                // abandons an unanswered one.
                let ping_due = self
                    .last_ping
                    .is_none_or(|t| t.elapsed().as_secs_f32() >= PING_INTERVAL_SECS);
                if ping_due && self.limiter.allow() {
                    self.ping_nonce += 1;
                    if conn.reducers.ping(self.ping_nonce).is_ok() {
                        self.pending_ping = Some((self.ping_nonce, local_time_us()));
                    }
                    self.last_ping = Some(Instant::now());
                }
                let pongs: Vec<(u64, u64, u64)> = self
                    .flags
                    .pongs
                    .lock()
                    .map(|mut p| p.drain(..).collect())
                    .unwrap_or_default();
                for (nonce, server_us, recv_us) in pongs {
                    let Some((sent_nonce, send_us)) = self.pending_ping else {
                        continue;
                    };
                    if nonce != sent_nonce || recv_us < send_us {
                        continue; // stale/foreign pong
                    }
                    self.pending_ping = None;
                    out.push(NetEvent::ClockSample(ClockSample {
                        offset_us: server_us as i64 - ((send_us + recv_us) / 2) as i64,
                        rtt_us: recv_us - send_us,
                    }));
                }

                // Interest anchored on the predicted position (hint) with
                // the authoritative own row as fallback.
                if let Some(pos) = own.map(|p| [p.x, p.y]) {
                    self.pump_interest(pos);
                }
            }
            _ => {}
        }
    }
}
