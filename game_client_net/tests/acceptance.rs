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

use game_client_net::{OwnCombat, SpacetimeNetClient};
use game_shared::net::protocol::{EntityKind, EntityState, ModuleAddr, WorldSnapshot};
use game_shared::net::traits::{NetClient, NetEvent};

const TIMEOUT: Duration = Duration::from_secs(15);

/// Ability ids from the shared roster (`game_shared::combat`).
const STRIKE: u16 = 1;
const FIREBOLT: u16 = 2;
const NOVA: u16 = 3;
const HEAL: u16 = 4;

/// Zone anchor positions (quadrant zones: SW=0, SE=1, NW=2, NE=3).
const ZONE3_POS: [f32; 3] = [16.0, 16.0, 1.0];
const ZONE0_POS: [f32; 3] = [-100.0, -100.0, 1.0];

struct Bot {
    client: SpacetimeNetClient,
    in_world: bool,
    /// Every snapshot seen since connect, newest last.
    snapshots: Vec<WorldSnapshot>,
    tombstones: Vec<(u64, u32)>,
    /// Last own-row ack `(epoch, last_applied_seq)` — the exploit battery
    /// forges inputs relative to these.
    ack: Option<(u32, u32)>,
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
            ack: None,
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
                NetEvent::OwnStateAck { epoch, seq, .. } => self.ack = Some((epoch, seq)),
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

    fn entity(&self, entity_id: u64) -> Option<&EntityState> {
        self.latest().entities.iter().find(|e| e.entity_id == entity_id)
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

    /// Pump for a fixed wall-clock span (letting rejected casts prove
    /// themselves by their absence of effects).
    fn settle_secs(&mut self, secs: f32) {
        let deadline = Instant::now() + Duration::from_secs_f32(secs);
        while Instant::now() < deadline {
            self.pump();
            std::thread::sleep(Duration::from_millis(16));
        }
    }

    fn combat(&self) -> OwnCombat {
        self.client.own_combat().expect("own combat state in cache")
    }

    fn gcd(&self) -> u64 {
        self.combat().gcd_until_us
    }

    fn hp_of(&self, entity_id: u64) -> f32 {
        self.entity(entity_id).expect("entity in scope").hp
    }

    /// Player hp never regenerates and rows persist across runs — self-kill
    /// and respawn back to full when a scenario needs hp headroom. Ends at
    /// the spawn point.
    fn normalize_hp(&mut self) {
        let own = self.own_id();
        let (hp, hp_max, gen0) = {
            let e = self.entity(own).expect("own row in snapshot");
            (e.hp, e.hp_max, e.generation)
        };
        if hp < hp_max - 0.01 {
            self.client.dev_damage(own, 1000.0);
            self.wait("normalize-hp respawn", |b| {
                b.entity(own)
                    .is_some_and(|e| e.generation > gen0 && e.alive && e.hp == e.hp_max)
            });
        }
    }

    /// Mana persists across runs and regenerates at 5/s — wait for full
    /// before scenarios that count on exact drain arithmetic.
    fn wait_mana_full(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let c = self.combat();
            if c.mana >= c.mana_max - 0.5 {
                return;
            }
            assert!(Instant::now() < deadline, "mana never refilled");
            self.pump();
            std::thread::sleep(Duration::from_millis(16));
        }
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

/// M7 D5: hp → 0 tombstones the current incarnation (remote proxies destroy
/// through the M5 evidence path) and `tick` respawns the same row after
/// `RESPAWN_SECS` with a fresh generation and full hp.
#[test]
#[ignore = "needs local spacetime standalone + published module"]
fn killed_npc_respawns_with_fresh_generation() {
    let mut b = Bot::enter("acc-npc-kill");
    let npc = b.find_npc();
    let gen0 = b.entity(npc).expect("npc just found").generation;

    b.client.dev_damage(npc, 1000.0);
    b.wait("death tombstone for the killed incarnation", |b| {
        b.tombstones.iter().any(|&(id, g)| id == npc && g == gen0)
    });
    // The corpse doesn't wander, so it respawns in place — still in scope.
    b.wait("fresh-generation respawn with full hp", |b| {
        b.entity(npc).is_some_and(|e| {
            e.generation == gen0 + 1 && e.alive && e.hp == e.hp_max
        })
    });
    b.leave();
}

/// M7 D5: player death leaves the row inert (dead, tombstoned); respawn
/// teleports to the spawn point with a fresh generation and full restore.
#[test]
#[ignore = "needs local spacetime standalone + published module"]
fn player_death_respawns_at_spawn_with_fresh_generation() {
    let mut b = Bot::enter("acc-death");
    b.teleport_and_settle([20.0, 20.0, 9.0]);
    let id = b.own_id();
    let gen0 = b.entity(id).expect("own row in snapshot").generation;

    b.client.dev_damage(id, 1000.0);
    b.wait("own row dead with tombstone", |b| {
        b.entity(id).is_some_and(|e| !e.alive && e.hp == 0.0)
            && b.tombstones.iter().any(|&(tid, g)| tid == id && g == gen0)
    });
    b.wait("respawn at spawn with fresh generation", |b| {
        b.entity(id).is_some_and(|e| {
            e.alive
                && e.generation == gen0 + 1
                && e.hp == e.hp_max
                && e.pos[0].abs() < 0.01
                && e.pos[1].abs() < 0.01
        })
    });
    b.leave();
}

// ---------------------------------------------------------------------------
// M7 D6 exploit battery. Assertions are effect-based — hp/mana/gcd rows —
// because reducer rejections never surface to the caller. A rejected cast
// aborts its transaction, so the GCD timestamp not moving IS the rejection.
// Player targets (not NPCs) keep scenarios deterministic: they don't wander
// out of strike range or zone scope mid-test.
// ---------------------------------------------------------------------------

/// Park bot `b_id` at `b_pos` with full hp (kept connected), and put caster
/// `a_id` at `a_pos` with the target in scope.
fn combat_pair(a_id: &str, b_id: &str, a_pos: [f32; 3], b_pos: [f32; 3]) -> (Bot, Bot, u64) {
    let mut b = Bot::enter(b_id);
    b.normalize_hp();
    b.teleport_and_settle(b_pos);
    let target = b.own_id();
    let mut a = Bot::enter(a_id);
    a.teleport_and_settle(a_pos);
    a.wait("target in scope", |a| a.sees(target));
    (a, b, target)
}

#[test]
#[ignore = "needs local spacetime standalone + published module"]
fn gcd_blocks_back_to_back_strikes() {
    let (mut a, b, target) = combat_pair("acc-gcd-a", "acc-gcd-b", [16.2, 15.0, 9.0], [15.0, 15.0, 9.0]);
    let hp0 = a.hp_of(target);
    a.client.cast_ability(STRIKE, target);
    a.client.cast_ability(STRIKE, target);
    a.wait("exactly one strike lands", |a| (a.hp_of(target) - (hp0 - 15.0)).abs() < 0.01);
    a.settle_secs(1.5);
    assert!(
        (a.hp_of(target) - (hp0 - 15.0)).abs() < 0.01,
        "second immediate strike bypassed the GCD"
    );
    a.leave();
    b.leave();
}

#[test]
#[ignore = "needs local spacetime standalone + published module"]
fn cooldown_blocks_early_nova_recast() {
    let mut a = Bot::enter("acc-nova-cd");
    a.teleport_and_settle([70.0, 15.0, 9.0]); // alone: nova needs no target
    a.wait_mana_full();
    let mana0 = a.combat().mana;
    a.client.cast_ability(NOVA, 0);
    a.wait("nova commits mana", |a| a.combat().mana <= mana0 - 39.0);
    let mana1 = a.combat().mana;
    a.settle_secs(1.2); // GCD over, 8 s cooldown very much not
    let gcd0 = a.gcd();
    a.client.cast_ability(NOVA, 0);
    a.settle_secs(1.0);
    assert_eq!(a.gcd(), gcd0, "recast inside the cooldown started a GCD");
    assert!(a.combat().mana >= mana1, "recast inside the cooldown burned mana");
    a.leave();
}

#[test]
#[ignore = "needs local spacetime standalone + published module"]
fn out_of_range_cast_lands_nothing() {
    // Same zone (NE), 40 m apart: beyond strike (3 m) and firebolt (25 m).
    let (mut a, b, target) = combat_pair("acc-range-a", "acc-range-b", [55.0, 15.0, 9.0], [15.0, 15.0, 9.0]);
    let hp0 = a.hp_of(target);
    let gcd0 = a.gcd();
    a.client.cast_ability(STRIKE, target);
    a.client.cast_ability(FIREBOLT, target);
    a.settle_secs(2.5);
    assert_eq!(a.gcd(), gcd0, "out-of-range cast started a GCD");
    assert!((a.hp_of(target) - hp0).abs() < 0.01, "out-of-range cast dealt damage");
    a.leave();
    b.leave();
}

#[test]
#[ignore = "needs local spacetime standalone + published module"]
fn offline_target_rejected() {
    let (mut a, b, target) = combat_pair("acc-off-a", "acc-off-b", [16.2, 15.0, 9.0], [15.0, 15.0, 9.0]);
    b.leave(); // row persists, session gone
    a.wait("target row still in scope", |a| a.sees(target));
    let hp0 = a.hp_of(target);
    let gcd0 = a.gcd();
    a.client.cast_ability(STRIKE, target);
    a.settle_secs(1.0);
    assert_eq!(a.gcd(), gcd0, "strike on an offline row started a GCD");
    assert!((a.hp_of(target) - hp0).abs() < 0.01, "offline row took damage");
    a.leave();
}

#[test]
#[ignore = "needs local spacetime standalone + published module"]
fn dead_target_and_dead_caster_rejected() {
    let (mut a, b, target) = combat_pair("acc-dead-a", "acc-dead-b", [16.2, 15.0, 9.0], [15.0, 15.0, 9.0]);
    a.client.dev_damage(target, 1000.0);
    a.wait("target dead", |a| a.entity(target).is_some_and(|e| !e.alive));
    let gcd0 = a.gcd();
    a.client.cast_ability(STRIKE, target);
    a.settle_secs(1.0); // inside the 5 s corpse window
    assert_eq!(a.gcd(), gcd0, "strike on a corpse started a GCD");

    let own = a.own_id();
    a.client.dev_damage(own, 1000.0);
    a.wait("caster dead", |a| a.entity(own).is_some_and(|e| !e.alive));
    let gcd1 = a.gcd();
    let mana1 = a.combat().mana; // regen skips the dead: must hold exactly
    a.client.cast_ability(NOVA, 0);
    a.settle_secs(1.0);
    assert_eq!(a.gcd(), gcd1, "dead caster started a GCD");
    assert_eq!(a.combat().mana, mana1, "dead caster's mana changed");
    a.wait("caster respawn (clean exit)", |a| {
        a.entity(own).is_some_and(|e| e.alive)
    });
    a.leave();
    b.leave();
}

#[test]
#[ignore = "needs local spacetime standalone + published module"]
fn mana_floor_stops_casts() {
    // 6 m apart: outside the projectile's 2-tick caster-grace sweep (which
    // skips collision), inside nova's 8 m and firebolt's 25 m.
    let (mut a, b, target) = combat_pair("acc-mana-a", "acc-mana-b", [21.0, 15.0, 9.0], [15.0, 15.0, 9.0]);
    a.wait_mana_full();
    let mana0 = a.combat().mana; // 100

    // Nova (instant, −40) hits the adjacent target too (−15 hp).
    a.client.cast_ability(NOVA, 0);
    a.wait("nova commits mana", |a| a.combat().mana <= mana0 - 39.0);
    a.settle_secs(1.1); // GCD

    // Two firebolts (1.5 s cast, −30 at completion, 25 dmg on hit).
    for round in 0..2 {
        let before = a.combat().mana;
        a.client.cast_ability(FIREBOLT, target);
        a.wait("firebolt cast started", |a| a.combat().active_cast.is_some());
        a.wait("firebolt completion commits mana", |a| {
            a.combat().active_cast.is_none() && a.combat().mana < before - 20.0
        });
        assert!(round == 1 || a.combat().mana >= 30.0, "drain arithmetic drifted");
    }
    let mana_low = a.combat().mana;
    assert!(mana_low < 30.0, "expected sub-cost mana, got {mana_low}");

    // The actual exploit check: a cast the caster cannot afford must not
    // start, commit mana, or touch the GCD.
    let gcd0 = a.gcd();
    a.client.cast_ability(FIREBOLT, target);
    a.settle_secs(1.0);
    assert_eq!(a.gcd(), gcd0, "unaffordable cast started a GCD");
    assert!(a.combat().mana >= mana_low, "unaffordable cast burned mana");
    assert!(a.combat().active_cast.is_none(), "unaffordable cast is casting");

    // Sanity on the damage side: nova 15 + two projectile hits 2×25 = 65.
    a.wait("both projectiles landed", |a| {
        (a.hp_of(target) - 35.0).abs() < 0.01
    });
    a.leave();
    b.leave();
}

#[test]
#[ignore = "needs local spacetime standalone + published module"]
fn movement_input_interrupts_cast() {
    let mut a = Bot::enter("acc-interrupt");
    a.teleport_and_settle([70.0, 40.0, 9.0]);
    a.wait_mana_full();
    let mana0 = a.combat().mana;
    a.client.cast_ability(HEAL, 0);
    a.wait("heal cast started", |a| a.combat().active_cast.is_some());
    let (epoch, seq) = a.ack.expect("own-row ack seen");
    a.client.dev_submit_input_raw(epoch, seq + 1, [1.0, 0.0], 0.0);
    a.wait("movement cancels the cast", |a| a.combat().active_cast.is_none());
    a.settle_secs(2.5); // past would-be completion
    assert!(
        a.combat().mana >= mana0 - 0.01,
        "interrupted heal still committed mana"
    );
    a.leave();
}

#[test]
#[ignore = "needs local spacetime standalone + published module"]
fn forged_inputs_do_not_move() {
    let mut a = Bot::enter("acc-forge");
    a.teleport_and_settle([70.0, 70.0, 9.0]);
    a.settle_secs(0.5);
    let (epoch, seq) = a.ack.expect("own-row ack seen");
    let p0 = a.own_pos();

    // Foreign epoch: dropped before simulation.
    a.client.dev_submit_input_raw(epoch + 7, seq + 1, [1.0, 0.0], 0.0);
    a.settle_secs(1.0);
    let p1 = a.own_pos();
    assert!(
        (p1[0] - p0[0]).abs() < 0.05 && (p1[1] - p0[1]).abs() < 0.05,
        "forged-epoch input moved the player: {p0:?} -> {p1:?}"
    );

    // Replayed (already-consumed) seq under the right epoch: also dropped.
    a.client.dev_submit_input_raw(epoch, seq, [1.0, 0.0], 0.0);
    a.settle_secs(1.0);
    let p2 = a.own_pos();
    assert!(
        (p2[0] - p0[0]).abs() < 0.05 && (p2[1] - p0[1]).abs() < 0.05,
        "replayed-seq input moved the player: {p0:?} -> {p2:?}"
    );
    a.leave();
}
