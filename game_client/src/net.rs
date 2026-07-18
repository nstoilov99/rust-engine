//! Net session wiring (M5): owns the `NetClient`, pumps it once per frame on
//! the main thread and routes drained `NetEvent`s into cache-diff
//! replication. Enabled with `--connect`.

use crate::replication::Replication;
use game_client_net::SpacetimeNetClient;
use game_shared::net::protocol::ModuleAddr;
use game_shared::net::traits::{ConnectionState, NetClient, NetEvent};

const DEFAULT_HOST: &str = "http://127.0.0.1:3000";
const DEFAULT_MODULE: &str = "rust-engine-dev";

pub struct NetSession {
    client: SpacetimeNetClient,
    events: Vec<NetEvent>,
    replication: Replication,
}

impl NetSession {
    /// `--connect [host [module]]` — defaults to the local dev standalone.
    pub fn from_args(args: &[String]) -> Option<Self> {
        let idx = args.iter().position(|a| a == "--connect")?;
        let positional = |offset: usize| {
            args.get(idx + offset)
                .filter(|a| !a.starts_with("--"))
                .cloned()
        };
        let addr = ModuleAddr {
            host: positional(1).unwrap_or_else(|| DEFAULT_HOST.to_string()),
            module: positional(2).unwrap_or_else(|| DEFAULT_MODULE.to_string()),
        };
        println!("net: connecting to {} / {}", addr.host, addr.module);
        let mut client = SpacetimeNetClient::new();
        client.connect(&addr);
        Some(Self {
            client,
            events: Vec::new(),
            replication: Replication::default(),
        })
    }

    /// Pump once per frame on the main thread.
    pub fn update(&mut self, world: &mut hecs::World) {
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
                NetEvent::Snapshot(snapshot) => {
                    self.replication.apply_snapshot(world, &snapshot)
                }
                _ => {}
            }
        }
    }

    pub fn status_line(&self) -> String {
        let state = self.client.connection_state();
        if state == ConnectionState::InWorld {
            let id = self.client.identity_hex().unwrap_or_default();
            format!("Net: InWorld ({})", id.get(..8).unwrap_or(&id))
        } else {
            format!("Net: {state:?}")
        }
    }
}
