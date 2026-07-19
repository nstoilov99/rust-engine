//! Net session wiring (M5): owns the `NetClient`, pumps it once per frame on
//! the main thread and routes drained `NetEvent`s into cache-diff
//! replication. Enabled with `--connect`.

use crate::interp::NetClock;
use crate::prediction::Prediction;
use crate::replication::Replication;
use game_client_net::{local_time_us, SpacetimeNetClient};
use game_shared::net::protocol::{ClientInput, ModuleAddr};
use game_shared::net::traits::{ConnectionState, NetClient, NetEvent};
use rust_engine::engine::ecs::game_world::GameWorld;
use rust_engine::engine::input::action::KeyCode;
use rust_engine::engine::input::InputManager;
use std::time::Instant;

const DEFAULT_HOST: &str = "http://127.0.0.1:3000";
const DEFAULT_MODULE: &str = "rust-engine-dev";

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
}

impl NetSession {
    /// `--connect [host [module]]` — defaults to the local dev standalone.
    pub fn from_args(args: &[String]) -> Option<Self> {
        let idx = args.iter().position(|a| a == "--connect")?;
        // Positionals stop at the first `--flag` so e.g. `--connect --net-id b`
        // doesn't read `b` as the module name.
        let mut positional = args[idx + 1..]
            .iter()
            .take_while(|a| !a.starts_with("--"))
            .cloned();
        let addr = ModuleAddr {
            host: positional.next().unwrap_or_else(|| DEFAULT_HOST.to_string()),
            module: positional
                .next()
                .unwrap_or_else(|| DEFAULT_MODULE.to_string()),
        };
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
        // M6 D1: refuse servers whose collision content differs from ours.
        match rust_engine::assets::asset_source::read_bytes("collision/greybox/manifest.ron") {
            Ok(bytes) => {
                client.set_expected_collision_hash(game_shared::collision::manifest_hash(&bytes));
            }
            Err(e) => println!("net: no local collision manifest ({e}); skipping collision gate"),
        }
        client.connect(&addr);
        Some(Self {
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
        })
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
                } => {
                    self.prediction.on_ack(epoch, seq, pos, vel, yaw, grounded);
                    self.last_ack = Some((epoch, seq));
                }
                NetEvent::ClockSample(s) => self.clock.add_sample(s.offset_us, s.rtt_us),
            }
        }

        if self.client.connection_state() == ConnectionState::InWorld {
            self.prediction
                .update(dt, move_dir, self.yaw, sprint, jump_pressed, &mut self.outgoing);
            for input in self.outgoing.drain(..) {
                self.client.send_input(&input);
            }
            if let Some((pos, yaw)) = self.prediction.visual_pose() {
                self.replication.set_local_pose(world, pos, yaw);
            }
        }

        let server_now = self.clock.server_time_us(local_time_us());
        self.replication.interpolate(world, server_now);
    }

    /// Predicted local-player position (Z-up), once in world.
    #[cfg(not(feature = "editor"))]
    pub fn local_pos(&self) -> Option<glam::Vec3> {
        self.prediction.visual_pose().map(|(p, _)| glam::Vec3::from(p))
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
