//! `SpacetimeNetClient`: the SpacetimeDB implementation of `NetClient`.
//!
//! State machine (plan D3): Offline → Connecting → AwaitBaseSub →
//! VersionCheck → InWorld, with any state collapsing to Disconnected.
//! SDK callbacks run inside `frame_tick()` on the game thread, but must be
//! `Send + 'static`, so they only write `Flags`; all decisions happen in
//! `poll()`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use game_shared::net::protocol::{
    ClientInput, EntityKind, EntityState, ModuleAddr, WorldSnapshot,
};
use game_shared::net::schema::{version_compatible, PROTOCOL_VERSION};
use game_shared::net::traits::{ConnectionState, DisconnectReason, NetClient, NetEvent};
use spacetimedb_sdk::{credentials, DbContext, Table, TableWithPrimaryKey};

use crate::module_bindings::{
    enter_world, ConfigTableAccess, DbConnection, NpcTableAccess, PlayerTableAccess,
    SubscriptionHandle, TombstoneTableAccess,
};

/// Spike binding requirement 3: hard client-side cap on reducer calls.
const MAX_REDUCER_CALLS_PER_SEC: f32 = 30.0;

#[derive(Default)]
struct Flags {
    connected: AtomicBool,
    disconnected: AtomicBool,
    base_applied: AtomicBool,
    repl_applied: AtomicBool,
    /// Replicated rows changed since the last snapshot (plan D3: callbacks
    /// only mark dirty; the cache-diff is the sole spawn/despawn authority).
    cache_dirty: AtomicBool,
    /// The presented stored token was rejected (contract §4.1: delete it).
    auth_rejected: AtomicBool,
    /// Tombstone evidence observed by row callbacks (contract §3.2).
    tombstones: Mutex<Vec<(u64, u32)>>,
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

pub struct SpacetimeNetClient {
    state: ConnectionState,
    conn: Option<DbConnection>,
    base_sub: Option<SubscriptionHandle>,
    repl_sub: Option<SubscriptionHandle>,
    flags: Arc<Flags>,
    pending: Vec<NetEvent>,
    limiter: RateLimiter,
    enter_world_sent: bool,
    /// Credentials file key of the current connection (token cleanup).
    credentials_key: Option<String>,
    /// Overridable for acceptance tests (version-mismatch path).
    client_version: u32,
}

impl SpacetimeNetClient {
    pub fn new() -> Self {
        Self {
            state: ConnectionState::Offline,
            conn: None,
            base_sub: None,
            repl_sub: None,
            flags: Arc::new(Flags::default()),
            pending: Vec::new(),
            limiter: RateLimiter::new(),
            enter_world_sent: false,
            credentials_key: None,
            client_version: PROTOCOL_VERSION,
        }
    }

    /// Test hook: pretend to be a different protocol version.
    pub fn set_client_version(&mut self, version: u32) {
        self.client_version = version;
    }

    pub fn identity_hex(&self) -> Option<String> {
        self.conn
            .as_ref()
            .and_then(|c| c.try_identity())
            .map(|id| id.to_hex().to_string())
    }

    fn credentials_key(module: &str) -> String {
        format!("rust-engine-{module}")
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
        self.base_sub = None;
        self.repl_sub = None;
        self.enter_world_sent = false;
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

    /// Package 3 replication scope: all player/NPC rows (populations are
    /// tiny). Package 5 replaces this with zone-scoped queries swapped on
    /// `ZoneChanged`.
    fn subscribe_replication(&mut self) {
        let conn = self
            .conn
            .as_ref()
            .expect("subscribe_replication requires a connection");
        let queries = vec![
            "SELECT * FROM player".to_string(),
            "SELECT * FROM npc".to_string(),
        ];
        let applied = self.flags.clone();
        let errored = self.flags.clone();
        let handle = conn
            .subscription_builder()
            .on_applied(move |_ctx| {
                applied.repl_applied.store(true, Ordering::Release);
                applied.mark_dirty();
            })
            .on_error(move |_ctx, err| {
                errored.set_error(format!("replication subscription failed: {err}"));
                errored.disconnected.store(true, Ordering::Release);
            })
            .subscribe(queries);
        self.repl_sub = Some(handle);
    }

    /// Row callbacks only mark dirty / record tombstone evidence; all
    /// spawn/despawn decisions belong to the cache-diff (plan D3).
    fn register_row_callbacks(conn: &DbConnection, flags: &Arc<Flags>) {
        macro_rules! dirty_on {
            ($table:expr) => {{
                let f = flags.clone();
                $table.on_insert(move |_, _| f.mark_dirty());
                let f = flags.clone();
                $table.on_update(move |_, _, _| f.mark_dirty());
                let f = flags.clone();
                $table.on_delete(move |_, _| f.mark_dirty());
            }};
        }
        dirty_on!(conn.db.player());
        dirty_on!(conn.db.npc());
        let f = flags.clone();
        conn.db.tombstone().on_insert(move |_, row| {
            f.push_tombstone(row.entity_id, row.generation);
        });
        let f = flags.clone();
        conn.db.tombstone().on_update(move |_, _, row| {
            f.push_tombstone(row.entity_id, row.generation);
        });
    }

    fn build_snapshot(conn: &DbConnection) -> WorldSnapshot {
        let own_identity = conn.try_identity();
        let mut entities = Vec::new();
        let mut own_entity_id = None;
        for p in conn.db.player().iter() {
            if own_identity == Some(p.owner_identity) {
                own_entity_id = Some(p.entity_id);
            }
            entities.push(EntityState {
                entity_id: p.entity_id,
                generation: p.generation,
                kind: EntityKind::Player,
                pos: [p.x, p.y, p.z],
                vel: [p.vx, p.vy, p.vz],
                yaw: p.yaw,
                zone_id: p.zone_id,
                server_time_us: p.last_update_micros.max(0) as u64,
            });
        }
        for n in conn.db.npc().iter() {
            entities.push(EntityState {
                entity_id: n.entity_id,
                generation: n.generation,
                kind: EntityKind::Npc,
                pos: [n.x, n.y, n.z],
                vel: [0.0; 3],
                yaw: n.yaw,
                zone_id: n.zone_id,
                server_time_us: n.last_update_micros.max(0) as u64,
            });
        }
        WorldSnapshot {
            entities,
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

        let key = Self::credentials_key(&addr.module);
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

    fn send_input(&mut self, _input: &ClientInput) {
        if self.state != ConnectionState::InWorld {
            return;
        }
        if !self.limiter.allow() {
            return;
        }
        // The input reducer lands in Package 4; until then this is only the
        // gate (state + rate limit) that all reducer calls will pass through.
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
                if version_compatible(config.protocol_version, self.client_version) {
                    self.subscribe_replication();
                    self.state = ConnectionState::EnteringWorld;
                } else {
                    self.fail(
                        out,
                        DisconnectReason::VersionMismatch {
                            server: config.protocol_version,
                            client: self.client_version,
                        },
                    );
                }
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
                let own_session_live = conn
                    .try_identity()
                    .and_then(|id| conn.db.player().owner_identity().find(&id))
                    .is_some_and(|p| p.session.is_some() && p.session == conn.try_connection_id());
                if own_session_live && self.flags.repl_applied.load(Ordering::Acquire) {
                    self.state = ConnectionState::InWorld;
                    self.flags.mark_dirty();
                    out.push(NetEvent::Connected);
                }
            }
            ConnectionState::InWorld => {
                // Session revocation (contract §4.3): our own row's session
                // no longer names this connection → another connection of
                // the same identity took over.
                if let (Some(identity), Some(my_conn)) =
                    (conn.try_identity(), conn.try_connection_id())
                {
                    if let Some(own) = conn.db.player().owner_identity().find(&identity) {
                        if own.session != Some(my_conn) {
                            self.fail(out, DisconnectReason::SessionReplaced);
                            return;
                        }
                    }
                }
                if self.flags.cache_dirty.swap(false, Ordering::AcqRel) {
                    out.push(NetEvent::Snapshot(Self::build_snapshot(conn)));
                }
            }
            _ => {}
        }
    }
}
