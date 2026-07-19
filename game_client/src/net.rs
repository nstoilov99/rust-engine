//! Net session wiring (M5): owns the `NetClient`, pumps it once per frame on
//! the main thread and routes drained `NetEvent`s into cache-diff
//! replication. Enabled with `--connect`.

use crate::interp::NetClock;
use crate::replication::Replication;
use game_client_net::{local_time_us, SpacetimeNetClient};
use game_shared::net::protocol::ModuleAddr;
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
    last_update: Option<Instant>,
    /// Last acked (epoch, seq) from the own row, for the status line.
    last_ack: Option<(u32, u32)>,
    /// True while the previous frame had movement input — lets one final
    /// stop sample through so the server sees the rest pose.
    was_moving: bool,
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
            last_update: None,
            last_ack: None,
            was_moving: false,
        })
    }

    /// Pump once per frame on the main thread: drain events, drive the
    /// local player from raw WASD (M5 trust-the-client; M6 replaces this),
    /// and evaluate proxy interpolation at delayed server time.
    pub fn update(&mut self, game_world: &mut GameWorld) {
        let now = Instant::now();
        let dt = self
            .last_update
            .map_or(0.0, |t| now.duration_since(t).as_secs_f32());
        self.last_update = Some(now);

        let (move_dir, sprint) = game_world
            .resource::<InputManager>()
            .map_or(([0.0; 2], false), read_move_keys);

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
                NetEvent::InputAck { epoch, seq } => self.last_ack = Some((epoch, seq)),
                NetEvent::ClockSample(s) => self.clock.add_sample(s.offset_us, s.rtt_us),
            }
        }

        if self.client.connection_state() == ConnectionState::InWorld {
            let moving = move_dir != [0.0; 2];
            if let Some(input) = self
                .replication
                .drive_local_player(world, move_dir, sprint, dt)
            {
                // Coalesced by the backend; only offer samples while moving
                // (plus one stop sample) so idle clients stay quiet.
                if moving || self.was_moving {
                    self.client.send_input(&input);
                }
            }
            self.was_moving = moving;
        }

        let server_now = self.clock.server_time_us(local_time_us());
        self.replication.interpolate(world, server_now);
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

/// Raw WASD + Shift on the XY ground plane (Z-up: X forward, Y right).
/// Deliberately bypasses Enhanced Input — M6's server-authoritative player
/// replaces this whole path.
fn read_move_keys(im: &InputManager) -> ([f32; 2], bool) {
    let axis = |neg, pos| (im.is_key_pressed(pos) as i8 - im.is_key_pressed(neg) as i8) as f32;
    (
        [
            axis(KeyCode::KeyS, KeyCode::KeyW),
            axis(KeyCode::KeyA, KeyCode::KeyD),
        ],
        im.is_key_pressed(KeyCode::ShiftLeft),
    )
}
