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

use game_shared::net::protocol::{ClientInput, ModuleAddr};
use game_shared::net::schema::{version_compatible, PROTOCOL_VERSION};
use game_shared::net::traits::{ConnectionState, DisconnectReason, NetClient, NetEvent};
use spacetimedb_sdk::{credentials, DbContext};

use crate::module_bindings::{ConfigTableAccess, DbConnection, SubscriptionHandle};

/// Spike binding requirement 3: hard client-side cap on reducer calls.
const MAX_REDUCER_CALLS_PER_SEC: f32 = 30.0;

#[derive(Default)]
struct Flags {
    connected: AtomicBool,
    disconnected: AtomicBool,
    base_applied: AtomicBool,
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
    flags: Arc<Flags>,
    pending: Vec<NetEvent>,
    limiter: RateLimiter,
    /// Overridable for acceptance tests (version-mismatch path).
    client_version: u32,
}

impl SpacetimeNetClient {
    pub fn new() -> Self {
        Self {
            state: ConnectionState::Offline,
            conn: None,
            base_sub: None,
            flags: Arc::new(Flags::default()),
            pending: Vec::new(),
            limiter: RateLimiter::new(),
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

    fn teardown(&mut self) {
        self.base_sub = None;
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
        let token = match credentials::File::new(key.clone()).load() {
            Ok(t) => t,
            Err(e) => {
                log::warn!("failed to load stored credentials ({e}); connecting fresh");
                None
            }
        };

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
                on_conn_err.set_error(format!("connect failed: {err}"));
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
            let msg = self
                .flags
                .take_error()
                .unwrap_or_else(|| "connection closed".to_string());
            self.fail(out, DisconnectReason::ConnectionLost(msg));
            return;
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
                    self.state = ConnectionState::InWorld;
                    out.push(NetEvent::Connected);
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
            _ => {}
        }
    }
}
