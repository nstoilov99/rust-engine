//! M5 Package 5 acceptance suite: bot-driven clients against a live local
//! standalone. Ignored by default because they need infrastructure:
//!
//!   spacetime start                                  (separate terminal)
//!   cd server && pwsh -NoProfile ./publish.ps1 -Wipe
//!   cargo test -p game_client_net --test acceptance -- --ignored --test-threads=1
//!
//! Run serially (`--test-threads=1`): scenarios share the dev database.
//! Player rows persist across runs by design, so every scenario teleports
//! its bots to known positions instead of assuming a fresh world.
//!
//! The fifth plan scenario — a generation bump preventing stale-sample
//! attach — has no server-side path in M5 (entity ids are never reused), so
//! it is covered by unit tests in `game_client::replication` instead
//! (`generation_change_on_live_row_is_replace`,
//! `stale_generation_sample_does_not_attach`).

use std::time::{Duration, Instant};

use game_client_net::SpacetimeNetClient;
use game_shared::net::protocol::{EntityKind, ModuleAddr, WorldSnapshot};
use game_shared::net::traits::{NetClient, NetEvent};

const TIMEOUT: Duration = Duration::from_secs(15);

/// Zone anchor positions (quadrant zones: SW=0, SE=1, NW=2, NE=3).
const ZONE3_POS: [f32; 3] = [16.0, 16.0, 1.0];
const ZONE0_POS: [f32; 3] = [-100.0, -100.0, 1.0];

struct Bot {
    client: SpacetimeNetClient,
    in_world: bool,
    /// Every snapshot seen since connect, newest last.
    snapshots: Vec<WorldSnapshot>,
    tombstones: Vec<(u64, u32)>,
}

impl Bot {
    /// Connects with a distinct persistent identity per `net_id` and waits
    /// until InWorld with a first snapshot.
    fn enter(net_id: &str) -> Self {
        let mut client = SpacetimeNetClient::new();
        client.set_net_id(net_id.to_string());
        client.connect(&ModuleAddr {
            host: "http://127.0.0.1:3000".to_string(),
            module: "rust-engine-dev".to_string(),
        });
        let mut bot = Self {
            client,
            in_world: false,
            snapshots: Vec::new(),
            tombstones: Vec::new(),
        };
        bot.wait("InWorld with a snapshot", |b| {
            b.in_world && !b.snapshots.is_empty()
        });
        bot
    }

    fn pump(&mut self) {
        let mut events = Vec::new();
        self.client.poll(&mut events);
        for ev in events {
            match ev {
                NetEvent::Connected => self.in_world = true,
                NetEvent::Disconnected(reason) => panic!("bot disconnected: {reason:?}"),
                NetEvent::Snapshot(s) => self.snapshots.push(s),
                NetEvent::TombstoneSeen {
                    entity_id,
                    generation,
                } => self.tombstones.push((entity_id, generation)),
                _ => {}
            }
        }
    }

    fn wait(&mut self, what: &str, pred: impl Fn(&Bot) -> bool) {
        let deadline = Instant::now() + TIMEOUT;
        while !pred(self) {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            self.pump();
            std::thread::sleep(Duration::from_millis(16));
        }
    }

    fn latest(&self) -> &WorldSnapshot {
        self.snapshots.last().expect("no snapshot yet")
    }

    fn own_id(&self) -> u64 {
        self.latest().own_entity_id.expect("own row not in snapshot")
    }

    fn own_pos(&self) -> [f32; 3] {
        let own = self.own_id();
        self.latest()
            .entities
            .iter()
            .find(|e| e.entity_id == own)
            .expect("own entity missing from snapshot")
            .pos
    }

    fn sees(&self, entity_id: u64) -> bool {
        self.latest().entities.iter().any(|e| e.entity_id == entity_id)
    }

    /// Teleports via the dev reducer and waits until the own row lands.
    fn teleport_and_settle(&mut self, pos: [f32; 3]) {
        self.client.dev_teleport(pos);
        self.wait("teleport to land", |b| {
            let p = b.own_pos();
            (p[0] - pos[0]).abs() < 0.01 && (p[1] - pos[1]).abs() < 0.01
        });
    }

    /// Clean disconnect without pumping (pump treats Disconnected as fatal).
    fn leave(mut self) {
        self.client.disconnect();
    }
}

#[test]
#[ignore = "needs local spacetime standalone + published module"]
fn reconnect_keeps_identity_without_duplicates() {
    let a = Bot::enter("acc-reconnect");
    let id = a.own_id();
    a.leave();

    let a = Bot::enter("acc-reconnect");
    assert_eq!(a.own_id(), id, "entity id changed across reconnect");
    let rows = a
        .latest()
        .entities
        .iter()
        .filter(|e| e.entity_id == id)
        .count();
    assert_eq!(rows, 1, "duplicate own rows after reconnect");
    a.leave();
}

#[test]
#[ignore = "needs local spacetime standalone + published module"]
fn position_persists_across_disconnect() {
    let mut a = Bot::enter("acc-persist");
    a.teleport_and_settle([10.0, 5.0, 1.0]);
    a.leave();

    let a = Bot::enter("acc-persist");
    let p = a.own_pos();
    assert!(
        (p[0] - 10.0).abs() < 0.01 && (p[1] - 5.0).abs() < 0.01,
        "position not persisted: {p:?}"
    );
    a.leave();
}

#[test]
#[ignore = "needs local spacetime standalone + published module"]
fn zone_swap_overlaps_without_gap() {
    // Park a persistent anchor row in each zone (rows outlive sessions).
    let mut anchor3 = Bot::enter("acc-anchor3");
    anchor3.teleport_and_settle(ZONE3_POS);
    let anchor3_id = anchor3.own_id();
    anchor3.leave();
    let mut anchor0 = Bot::enter("acc-anchor0");
    anchor0.teleport_and_settle(ZONE0_POS);
    let anchor0_id = anchor0.own_id();
    anchor0.leave();

    let mut b = Bot::enter("acc-mover");
    b.teleport_and_settle([20.0, 20.0, 1.0]); // zone 3
    b.wait("zone-3 anchor in scope", |b| b.sees(anchor3_id));
    let baseline = b.snapshots.len();

    b.client.dev_teleport(ZONE0_POS);
    b.wait("swap complete: zone-0 anchor in, zone-3 anchor out", |b| {
        b.sees(anchor0_id) && !b.sees(anchor3_id)
    });

    // The testable no-gap invariant: the new zone was applied before the
    // old one was dropped, so some snapshot held anchors of both zones.
    assert!(
        b.snapshots[baseline..].iter().any(|s| {
            let has = |id| s.entities.iter().any(|e| e.entity_id == id);
            has(anchor3_id) && has(anchor0_id)
        }),
        "no overlap snapshot: old zone dropped before new zone applied"
    );
    // Out-of-scope, not destroyed: no tombstone for the vanished anchor.
    assert!(
        !b.tombstones.iter().any(|&(id, _)| id == anchor3_id),
        "zone eviction must not produce destruction evidence"
    );
    b.leave();
}

#[test]
#[ignore = "needs local spacetime standalone + published module"]
fn despawned_npc_is_destroyed_with_evidence() {
    let mut b = Bot::enter("acc-npc-watch");
    // NPCs wander around the spawn point; watch from the spawn zone.
    b.teleport_and_settle([1.0, 1.0, 1.0]);
    b.wait("an NPC in scope", |b| {
        b.latest().entities.iter().any(|e| e.kind == EntityKind::Npc)
    });
    let npc = b
        .latest()
        .entities
        .iter()
        .find(|e| e.kind == EntityKind::Npc)
        .expect("checked above")
        .entity_id;

    b.client.dev_despawn_npc(npc);
    // Tombstone evidence arrives via the permanent base subscription (§3.2),
    // distinguishing destruction from zone eviction.
    b.wait("tombstone evidence", |b| {
        b.tombstones.iter().any(|&(id, _)| id == npc)
    });
    b.wait("npc row gone", |b| !b.sees(npc));
    b.leave();
}
