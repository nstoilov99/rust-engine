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

    /// NPCs orbit in small deterministic loops near the spawn point, which
    /// sits on the corner of all four zones — any given quadrant may hold
    /// none of them. Hop the quadrants until one is in scope.
    fn find_npc(&mut self) -> u64 {
        let quadrants = [
            [1.0, 1.0, 1.0],
            [1.0, -1.0, 1.0],
            [-1.0, -1.0, 1.0],
            [-1.0, 1.0, 1.0],
        ];
        let deadline = Instant::now() + Duration::from_secs(45);
        loop {
            for pos in quadrants {
                self.teleport_and_settle(pos);
                let dwell = Instant::now() + Duration::from_secs(2);
                while Instant::now() < dwell {
                    self.pump();
                    if let Some(npc) = self
                        .latest()
                        .entities
                        .iter()
                        .find(|e| e.kind == EntityKind::Npc)
                    {
                        return npc.entity_id;
                    }
                    std::thread::sleep(Duration::from_millis(16));
                }
            }
            assert!(Instant::now() < deadline, "no NPC found in any quadrant");
        }
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

/// M6 D5 parity: every embedded trace replayed inside the server WASM module
/// must land within tolerance of the same trace replayed natively. Each side
/// is already bounded to 1 mm of the recorded `expected` per step (a replay
/// error fails the reducer / the native unwrap), so cross-checking the end
/// positions bounds native↔WASM divergence to 2·TRACE_TOLERANCE.
///
/// Rows are deterministic upserts, so a stale row from a pre-divergence
/// module could mask a fresh reducer failure — run after a fresh publish
/// (the suite header's `publish.ps1 -Wipe`) for a strict result.
#[test]
#[ignore = "needs local spacetime standalone + published module"]
fn wasm_parity_traces_match_native() {
    use game_shared::collision::ChunkStore;
    use game_shared::motion::trace::{load_embedded, EMBEDDED_TRACES, TRACE_TOLERANCE};

    // Native store from the same cooked chunks the module embeds.
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../content/collision/greybox");
    let mut paths: Vec<_> = std::fs::read_dir(dir)
        .expect("content/collision/greybox present")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "ccol"))
        .collect();
    paths.sort();
    let mut store = ChunkStore::new();
    for p in &paths {
        store.insert_chunk(&std::fs::read(p).unwrap()).unwrap();
    }

    let mut b = Bot::enter("acc-parity");
    for (id, _) in EMBEDDED_TRACES {
        b.client.dev_run_parity_trace(id);
    }
    for (id, _) in EMBEDDED_TRACES {
        let native = load_embedded(id).unwrap().replay(&store).unwrap();
        b.wait(&format!("parity result for {id}"), |b| {
            b.client.dev_parity_result(id).is_some()
        });
        let (steps, end, _hash) = b.client.dev_parity_result(id).unwrap();
        assert_eq!(steps, native.steps, "trace {id}: step count mismatch");
        let d = ((native.end_pos.x - end[0]).powi(2)
            + (native.end_pos.y - end[1]).powi(2)
            + (native.end_pos.z - end[2]).powi(2))
        .sqrt();
        assert!(
            d <= 2.0 * TRACE_TOLERANCE,
            "trace {id}: native end {:?} vs WASM end {end:?} diverge by {d}",
            native.end_pos
        );
    }
    b.leave();
}

#[test]
#[ignore = "needs local spacetime standalone + published module"]
fn despawned_npc_is_destroyed_with_evidence() {
    let mut b = Bot::enter("acc-npc-watch");
    let npc = b.find_npc();

    b.client.dev_despawn_npc(npc);
    // Tombstone evidence arrives via the permanent base subscription (§3.2),
    // distinguishing destruction from zone eviction.
    b.wait("tombstone evidence", |b| {
        b.tombstones.iter().any(|&(id, _)| id == npc)
    });
    b.wait("npc row gone", |b| !b.sees(npc));
    b.leave();
}
