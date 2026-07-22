//! Net session wiring (M5): owns the `NetClient`, pumps it once per frame on
//! the main thread and routes drained `NetEvent`s into cache-diff
//! replication. Enabled with `--connect`.

use crate::interp::NetClock;
use crate::prediction::Prediction;
use crate::replication::Replication;
use game_client_net::{local_time_us, SpacetimeNetClient};
use game_shared::motion::MotionConfig;
use game_shared::net::protocol::{ClientInput, ModuleAddr};
use game_shared::net::traits::{ConnectionState, NetClient, NetEvent};
use nalgebra_glm as glm;
use rust_engine::engine::ecs::components::{MeshRenderer, Name, Transform};
use rust_engine::engine::ecs::game_world::GameWorld;
use rust_engine::engine::input::action::KeyCode;
use rust_engine::engine::input::InputManager;
use rust_engine::engine::rendering::rendering_3d::mesh::PRIMITIVE_SPHERE;
use std::collections::HashMap;
use std::time::Instant;

const DEFAULT_HOST: &str = "http://127.0.0.1:3000";
const DEFAULT_MODULE: &str = "rust-engine-dev";

/// Optional `net_config.ron` beside the exe (M9 D1): lets a shipped client
/// be repointed at another server with a text editor, no recompile.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(default)]
pub struct NetConfig {
    pub host: String,
    pub module: String,
    pub auto_connect: bool,
}

impl Default for NetConfig {
    fn default() -> Self {
        Self {
            host: DEFAULT_HOST.to_string(),
            module: DEFAULT_MODULE.to_string(),
            auto_connect: false,
        }
    }
}

impl NetConfig {
    pub fn parse(s: &str) -> Result<Self, ron::error::SpannedError> {
        // Hand-edited configs (Notepad) often carry a UTF-8 BOM, which RON
        // rejects at 1:1.
        ron::from_str(s.trim_start_matches('\u{feff}'))
    }

    /// `<exe dir>/net_config.ron` (same discovery rule as `game.pak`).
    /// Absent file = defaults; malformed file = warn + defaults, never
    /// crash a shipped client.
    pub fn load() -> Self {
        let path = match std::env::current_exe() {
            Ok(exe) => match exe.parent() {
                Some(dir) => dir.join("net_config.ron"),
                None => return Self::default(),
            },
            Err(_) => return Self::default(),
        };
        match std::fs::read_to_string(&path) {
            Ok(s) => Self::parse(&s).unwrap_or_else(|e| {
                println!("net: malformed {} ({e}); using defaults", path.display());
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }
}

/// Field-wise precedence (M9 D1): CLI positional > config value. Compiled
/// defaults are already folded into the config's `Default`.
fn resolve_addr(positionals: &[String], cfg: &NetConfig) -> ModuleAddr {
    ModuleAddr {
        host: positionals.first().cloned().unwrap_or_else(|| cfg.host.clone()),
        module: positionals.get(1).cloned().unwrap_or_else(|| cfg.module.clone()),
    }
}

pub struct NetSession {
    client: SpacetimeNetClient,
    events: Vec<NetEvent>,
    replication: Replication,
    clock: NetClock,
    prediction: Prediction,
    /// Reused buffer for the input samples one frame's prediction produced.
    outgoing: Vec<ClientInput>,
    last_update: Option<Instant>,
    /// Last acked (epoch, seq) from the own row, for the status line.
    last_ack: Option<(u32, u32)>,
    /// Facing carried across idle frames (yaw only changes while moving).
    yaw: f32,
    /// Space state last frame, for the jump edge trigger.
    space_was_down: bool,
    /// Rendered projectile spheres by server entity id (M7 D4).
    projectiles: HashMap<u64, hecs::Entity>,
    /// Client-side target selection (M7 D6): sent per cast, never stored
    /// server-side. Cleared when the proxy leaves scope or dies.
    target: Option<u64>,
    /// Scene the server announced (M9.6, `Config.world_scene`); `Some` once
    /// the handshake gates pass. Consumers poll `world_scene()`.
    world_scene: Option<String>,
    /// Monotonic server-time estimate for HUD timers: the EWMA clock offset
    /// can step backwards, which would make cooldown sweeps jitter.
    #[cfg(all(not(feature = "editor"), feature = "hud"))]
    hud_server_now: u64,
}

/// Everything the standalone HUD draws in one frame (M7 D7).
#[cfg(all(not(feature = "editor"), feature = "hud"))]
pub struct HudState {
    pub status: String,
    pub server_now_us: u64,
    pub own: Option<game_client_net::OwnCombat>,
    pub target: Option<HudTarget>,
}

#[cfg(all(not(feature = "editor"), feature = "hud"))]
pub struct HudTarget {
    pub label: String,
    pub hp: f32,
    pub hp_max: f32,
    pub alive: bool,
    pub cast: Option<game_client_net::ActiveCastView>,
}

impl NetSession {
    /// `--connect [host [module]]` — missing positionals fall back to
    /// `net_config.ron` beside the exe, then compiled defaults (M9 D1).
    pub fn from_args(args: &[String]) -> Option<Self> {
        let idx = args.iter().position(|a| a == "--connect")?;
        // Positionals stop at the first `--flag` so e.g. `--connect --net-id b`
        // doesn't read `b` as the module name.
        let positionals: Vec<String> = args[idx + 1..]
            .iter()
            .take_while(|a| !a.starts_with("--"))
            .cloned()
            .collect();
        Some(Self::connect(
            resolve_addr(&positionals, &NetConfig::load()),
            args,
        ))
    }

    /// Standalone startup (M9 D1): `--connect` wins; without it, a config
    /// with `auto_connect: true` connects using its own host/module —
    /// double-click-the-exe multiplayer. Editor builds never auto-connect.
    #[cfg(not(feature = "editor"))]
    pub fn from_args_or_config(args: &[String]) -> Option<Self> {
        if args.iter().any(|a| a == "--connect") {
            return Self::from_args(args);
        }
        let cfg = NetConfig::load();
        cfg.auto_connect
            .then(|| Self::connect(resolve_addr(&[], &cfg), args))
    }

    fn connect(addr: ModuleAddr, args: &[String]) -> Self {
        println!("net: connecting to {} / {}", addr.host, addr.module);
        let mut client = SpacetimeNetClient::new();
        // `--net-id <name>`: distinct identity per name for local multi-
        // instance testing (default: one shared identity per module).
        if let Some(i) = args.iter().position(|a| a == "--net-id") {
            if let Some(id) = args.get(i + 1).filter(|a| !a.starts_with("--")) {
                println!("net: using identity slot '{id}'");
                client.set_net_id(id.clone());
            }
        }
        // M9 D4: advisory build stamp (short git hash from build.rs).
        client.set_expected_build_id(env!("GIT_HASH").to_string());
        // M6 D1: refuse servers whose collision content differs from ours.
        match rust_engine::assets::asset_source::read_bytes("collision/greybox/manifest.ron") {
            Ok(bytes) => {
                client.set_expected_collision_hash(game_shared::collision::manifest_hash(&bytes));
            }
            Err(e) => println!("net: no local collision manifest ({e}); skipping collision gate"),
        }
        client.connect(&addr);
        Self {
            client,
            events: Vec::new(),
            replication: Replication::default(),
            clock: NetClock::default(),
            prediction: Prediction::new(),
            outgoing: Vec::new(),
            last_update: None,
            last_ack: None,
            yaw: 0.0,
            space_was_down: false,
            projectiles: HashMap::new(),
            target: None,
            world_scene: None,
            #[cfg(all(not(feature = "editor"), feature = "hud"))]
            hud_server_now: 0,
        }
    }

    /// Pump once per frame on the main thread: drain events (acks reconcile
    /// prediction), run fixed-step prediction from raw WASD+Space (M6 D4),
    /// forward its input samples, and evaluate proxy interpolation at
    /// delayed server time. Events drain BEFORE inputs are sent, so an epoch
    /// change can never race a stale-seq sample into the new epoch.
    pub fn update(&mut self, game_world: &mut GameWorld) {
        let now = Instant::now();
        let dt = self
            .last_update
            .map_or(0.0, |t| now.duration_since(t).as_secs_f32());
        self.last_update = Some(now);

        let (move_dir, sprint, space_down) = game_world
            .resource::<InputManager>()
            .map_or(([0.0; 2], false, false), read_move_keys);
        let (tab_pressed, cast_slot) = game_world
            .resource::<InputManager>()
            .map_or((false, None), read_combat_keys);
        let jump_pressed = space_down && !self.space_was_down;
        self.space_was_down = space_down;
        if move_dir != [0.0; 2] {
            self.yaw = move_dir[1].atan2(move_dir[0]);
        }

        let world = game_world.hecs_mut();
        self.client.poll(&mut self.events);
        for ev in self.events.drain(..) {
            match ev {
                NetEvent::Connected => println!(
                    "net: in world as {}",
                    self.client.identity_hex().unwrap_or_default()
                ),
                NetEvent::Disconnected(reason) => println!("net: disconnected: {reason:?}"),
                NetEvent::TombstoneSeen {
                    entity_id,
                    generation,
                } => self.replication.record_tombstone(entity_id, generation),
                NetEvent::Snapshot(snapshot) => self.replication.apply_snapshot(world, &snapshot),
                NetEvent::StateUpdate(state) => self.replication.push_sample(&state),
                NetEvent::OwnStateAck {
                    epoch,
                    seq,
                    pos,
                    vel,
                    yaw,
                    grounded,
                    alive,
                } => {
                    self.prediction.on_ack(epoch, seq, pos, vel, yaw, grounded, alive);
                    self.last_ack = Some((epoch, seq));
                }
                NetEvent::ClockSample(s) => self.clock.add_sample(s.offset_us, s.rtt_us),
                NetEvent::BuildMismatch { server, client } => println!(
                    "net: WARNING: build mismatch (server {server}, client {client}) — \
                     same protocol, different builds; consider republishing/updating"
                ),
                NetEvent::WorldScene(scene) => {
                    println!("net: server world scene '{scene}'");
                    self.world_scene = Some(scene);
                }
            }
        }

        if self.client.connection_state() == ConnectionState::InWorld {
            self.handle_combat_keys(tab_pressed, cast_slot);
            self.prediction
                .update(dt, move_dir, self.yaw, sprint, jump_pressed, &mut self.outgoing);
            for input in self.outgoing.drain(..) {
                self.client.send_input(&input);
            }
            if let Some((pos, yaw)) = self.prediction.visual_pose() {
                self.client.set_interest_hint([pos[0], pos[1]]);
                self.replication.set_local_pose(world, pos, yaw);
            }
        }

        let server_now = self.clock.server_time_us(local_time_us());
        self.replication.interpolate(world, server_now);
        self.update_projectiles(world, server_now);
    }

    /// M7 D6: Tab cycles alive proxies nearest-first; keys 1–4 cast the
    /// roster slots. Targeting is pure client selection state — the server
    /// re-checks legality against the id sent with each cast.
    fn handle_combat_keys(&mut self, tab_pressed: bool, cast_slot: Option<usize>) {
        let candidates = self.replication.targetables();
        if self.target.is_some_and(|t| !candidates.iter().any(|(id, _)| *id == t)) {
            self.target = None;
        }
        if tab_pressed && !candidates.is_empty() {
            let own = self
                .prediction
                .visual_pose()
                .map_or([0.0; 3], |(p, _)| p);
            let d2 = |p: [f32; 3]| {
                (p[0] - own[0]).powi(2) + (p[1] - own[1]).powi(2) + (p[2] - own[2]).powi(2)
            };
            let mut sorted = candidates;
            sorted.sort_by(|a, b| d2(a.1).total_cmp(&d2(b.1)));
            let next = match self.target.and_then(|t| sorted.iter().position(|(id, _)| *id == t)) {
                Some(i) => sorted[(i + 1) % sorted.len()].0,
                None => sorted[0].0,
            };
            self.target = Some(next);
            println!("net: target {next}");
        }
        if let Some(slot) = cast_slot {
            let def = &game_shared::combat::ABILITIES[slot];
            let target = if def.kind.hostile_targeted() {
                self.target.unwrap_or(0)
            } else {
                0
            };
            self.client.cast_ability(def.id.0, target);
        }
    }

    /// M7 D4: small spheres extrapolated ballistically from the last server
    /// step — `pos + vel·dt − ½·g·dt²·ẑ` at estimated server time, dt capped
    /// so a stale row can't fly off before its delete arrives.
    fn update_projectiles(&mut self, world: &mut hecs::World, server_now: Option<u64>) {
        const MAX_EXTRAPOLATE_SECS: f32 = 0.25;
        let gravity = MotionConfig::default().gravity;
        let views = self.client.projectiles();
        for v in &views {
            // Unsynced clock: dt 0 renders at the last server step.
            let dt = (server_now.unwrap_or(0).saturating_sub(v.server_time_us) as f32
                / 1_000_000.0)
                .min(MAX_EXTRAPOLATE_SECS);
            let pos = glm::vec3(
                v.pos[0] + v.vel[0] * dt,
                v.pos[1] + v.vel[1] * dt,
                v.pos[2] + v.vel[2] * dt - 0.5 * gravity * dt * dt,
            );
            match self.projectiles.get(&v.entity_id) {
                Some(&e) => {
                    if let Ok(mut t) = world.get::<&mut Transform>(e) {
                        t.position = pos;
                    }
                    rust_engine::engine::ecs::hierarchy::mark_transform_dirty(world, e);
                }
                None => {
                    let e = world.spawn((
                        Transform::new(pos).with_scale(glm::vec3(0.25, 0.25, 0.25)),
                        MeshRenderer {
                            mesh_path: PRIMITIVE_SPHERE.to_string(),
                            ..Default::default()
                        },
                        Name::new(format!("Net Projectile {}", v.entity_id)),
                    ));
                    self.projectiles.insert(v.entity_id, e);
                }
            }
        }
        self.projectiles.retain(|id, e| {
            let live = views.iter().any(|v| v.entity_id == *id);
            if !live {
                let _ = world.despawn(*e);
            }
            live
        });
    }

    /// Predicted local-player position (Z-up), once in world.
    #[cfg(not(feature = "editor"))]
    pub fn local_pos(&self) -> Option<glam::Vec3> {
        self.prediction.visual_pose().map(|(p, _)| glam::Vec3::from(p))
    }

    /// Per-frame HUD snapshot (M7 D7). `&mut` only for the monotonic clamp.
    #[cfg(all(not(feature = "editor"), feature = "hud"))]
    pub fn hud_state(&mut self) -> HudState {
        use game_shared::net::protocol::EntityKind;
        let estimate = self.clock.server_time_us(local_time_us()).unwrap_or(0);
        self.hud_server_now = self.hud_server_now.max(estimate);
        let target = self.target.and_then(|id| {
            let (kind, combat) = self.replication.target_info(id)?;
            let label = match kind {
                EntityKind::Player => format!("Player {id}"),
                EntityKind::Npc => format!("Dummy #{id}"),
            };
            let (hp, hp_max, alive) =
                combat.map_or((0.0, 1.0, true), |c| (c.hp, c.hp_max, c.alive));
            Some(HudTarget {
                label,
                hp,
                hp_max,
                alive,
                cast: self.client.active_cast_of(id),
            })
        });
        HudState {
            status: self.status_line(),
            server_now_us: self.hud_server_now,
            own: self.client.own_combat(),
            target,
        }
    }

    /// Scene the server announced (M9.6); `Some` once the handshake gates pass.
    pub fn world_scene(&self) -> Option<&str> {
        self.world_scene.as_deref()
    }

    /// True once the connection is dead (handshake timeout, refusal, loss).
    pub fn is_disconnected(&self) -> bool {
        matches!(
            self.client.connection_state(),
            ConnectionState::Disconnected | ConnectionState::Offline
        )
    }

    pub fn disconnect(&mut self) {
        self.client.disconnect();
    }

    pub fn status_line(&self) -> String {
        let state = self.client.connection_state();
        if state == ConnectionState::InWorld {
            let id = self.client.identity_hex().unwrap_or_default();
            let mut line = format!("Net: InWorld ({})", id.get(..8).unwrap_or(&id));
            if let Some((epoch, seq)) = self.last_ack {
                line += &format!(" ack e{epoch}#{seq}");
            }
            if self.clock.synced() {
                line += &format!(" rtt {}ms", self.clock.rtt_us() / 1000);
            }
            line
        } else {
            format!("Net: {state:?}")
        }
    }
}

/// Tab (target cycle) + 1–4 (ability slots), edge-triggered.
fn read_combat_keys(im: &InputManager) -> (bool, Option<usize>) {
    let slot = [
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
    ]
    .iter()
    .position(|&k| im.is_key_just_pressed(k));
    (im.is_key_just_pressed(KeyCode::Tab), slot)
}

/// Raw WASD + Shift (sprint) + Space (jump) on the XY ground plane (Z-up:
/// X forward, Y right). Deliberately bypasses Enhanced Input for now.
fn read_move_keys(im: &InputManager) -> ([f32; 2], bool, bool) {
    let axis = |neg, pos| (im.is_key_pressed(pos) as i8 - im.is_key_pressed(neg) as i8) as f32;
    (
        [
            axis(KeyCode::KeyS, KeyCode::KeyW),
            axis(KeyCode::KeyA, KeyCode::KeyD),
        ],
        im.is_key_pressed(KeyCode::ShiftLeft),
        im.is_key_pressed(KeyCode::Space),
    )
}

#[cfg(test)]
mod net_config_tests {
    use super::*;

    fn args(s: &[&str]) -> Vec<String> {
        s.iter().map(|a| a.to_string()).collect()
    }

    #[test]
    fn parse_full_config() {
        let c = NetConfig::parse(
            r#"NetConfig(host: "http://example.com", module: "my-game", auto_connect: true)"#,
        )
        .unwrap();
        assert_eq!(c.host, "http://example.com");
        assert_eq!(c.module, "my-game");
        assert!(c.auto_connect);
    }

    #[test]
    fn parse_partial_config_keeps_defaults() {
        let c = NetConfig::parse(r#"NetConfig(module: "my-game")"#).unwrap();
        assert_eq!(c.host, DEFAULT_HOST);
        assert_eq!(c.module, "my-game");
        assert!(!c.auto_connect);
    }

    #[test]
    fn parse_tolerates_utf8_bom() {
        let c = NetConfig::parse("\u{feff}NetConfig(module: \"my-game\")").unwrap();
        assert_eq!(c.module, "my-game");
    }

    #[test]
    fn parse_malformed_is_err() {
        assert!(NetConfig::parse("NetConfig(host: )").is_err());
        assert!(NetConfig::parse("").is_err());
    }

    #[test]
    fn resolve_field_wise_precedence() {
        let cfg = NetConfig {
            host: "http://cfg:3000".into(),
            module: "cfg-module".into(),
            auto_connect: false,
        };
        // Both positionals: CLI wins outright.
        let a = resolve_addr(&args(&["http://cli:3000", "cli-module"]), &cfg);
        assert_eq!((a.host.as_str(), a.module.as_str()), ("http://cli:3000", "cli-module"));
        // Host only: module still comes from the config.
        let a = resolve_addr(&args(&["http://cli:3000"]), &cfg);
        assert_eq!((a.host.as_str(), a.module.as_str()), ("http://cli:3000", "cfg-module"));
        // Bare --connect: both from the config.
        let a = resolve_addr(&[], &cfg);
        assert_eq!((a.host.as_str(), a.module.as_str()), ("http://cfg:3000", "cfg-module"));
    }

    #[test]
    fn resolve_defaults_without_config() {
        let a = resolve_addr(&[], &NetConfig::default());
        assert_eq!((a.host.as_str(), a.module.as_str()), (DEFAULT_HOST, DEFAULT_MODULE));
    }
}
