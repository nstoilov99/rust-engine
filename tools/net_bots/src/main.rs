//! M8 D5 load harness: N headless bots through the real client path —
//! connection, base + interest subscriptions, cache-diff snapshots, the
//! 20 Hz epoch/seq input stream and the cast pipeline.
//!
//! One process hosts up to ~50 bots (an SDK panic on a dead socket kills
//! the whole process — bounded blast radius); scenario scripts spawn
//! several processes with distinct `--prefix` values.
//!
//! Scenarios (plan D5):
//! - `uniform`: bots teleport to deterministic spots across the world and
//!   wander locally.
//! - `hotspot`: bots converge on `--center`, then disperse after
//!   `--disperse` seconds.
//! - `churn`: uniform wander + each bot disconnects/reconnects on a
//!   10–20 s cycle.
//! - `thrash`: bots pace across the cell border at `--border`, exercising
//!   anchor hysteresis at oscillation rate.

use std::time::{Duration, Instant};

use game_client_net::SpacetimeNetClient;
use game_shared::net::protocol::{ClientInput, ModuleAddr, WorldSnapshot};
use game_shared::net::traits::{NetClient, NetEvent};

const STRIKE: u16 = 1;
const NOVA: u16 = 3;
const INPUT_INTERVAL: Duration = Duration::from_millis(50); // 20 Hz

#[derive(Clone, Copy, PartialEq)]
enum Scenario {
    Uniform,
    Hotspot,
    Churn,
    Thrash,
}

struct Args {
    bots: usize,
    prefix: String,
    scenario: Scenario,
    duration: f32,
    host: String,
    module: String,
    center: [f32; 2],
    disperse: f32,
    area: f32,
    border: f32,
}

fn parse_args() -> Args {
    let mut a = Args {
        bots: 50,
        prefix: "bot".to_string(),
        scenario: Scenario::Uniform,
        duration: 120.0,
        host: "http://127.0.0.1:3000".to_string(),
        module: "rust-engine-dev".to_string(),
        center: [32.0, 32.0],
        disperse: 0.0, // 0 → duration/2
        area: 200.0,
        border: 64.0,
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut val = || it.next().unwrap_or_else(|| panic!("missing value for {flag}"));
        match flag.as_str() {
            "--bots" => a.bots = val().parse().expect("--bots"),
            "--prefix" => a.prefix = val(),
            "--scenario" => {
                a.scenario = match val().as_str() {
                    "uniform" => Scenario::Uniform,
                    "hotspot" => Scenario::Hotspot,
                    "churn" => Scenario::Churn,
                    "thrash" => Scenario::Thrash,
                    s => panic!("unknown scenario {s}"),
                }
            }
            "--duration" => a.duration = val().parse().expect("--duration"),
            "--host" => a.host = val(),
            "--module" => a.module = val(),
            "--center" => {
                let v = val();
                let (x, y) = v.split_once(',').expect("--center X,Y");
                a.center = [x.parse().expect("center x"), y.parse().expect("center y")];
            }
            "--disperse" => a.disperse = val().parse().expect("--disperse"),
            "--area" => a.area = val().parse().expect("--area"),
            "--border" => a.border = val().parse().expect("--border"),
            f => panic!("unknown flag {f}"),
        }
    }
    if a.disperse == 0.0 {
        a.disperse = a.duration / 2.0;
    }
    a
}

/// splitmix64 — deterministic per-bot randomness without a rand dep.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Uniform in [0, 1).
    fn f32(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }

    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.f32()
    }
}

struct Bot {
    client: SpacetimeNetClient,
    rng: Rng,
    events: Vec<NetEvent>,
    // Own-row authority (from acks).
    epoch: u32,
    seq: u32,
    pos: [f32; 3],
    alive: bool,
    in_world: bool,
    teleported: bool,
    /// Scenario home: uniform/churn wander center, hotspot disperse target.
    home: [f32; 2],
    heading: f32,
    next_heading: Instant,
    next_input: Instant,
    next_cast: Instant,
    /// Churn: next connect/disconnect flip; None outside churn.
    next_flip: Option<Instant>,
    connected: bool,
    /// Latest snapshot's live full-tier targets (excluding self).
    targets: Vec<u64>,
    own_entity_id: Option<u64>,
    // Stats.
    snapshots: u64,
    state_updates: u64,
    coarse_updates: u64,
    coarse_seen: std::collections::HashMap<u64, u64>,
    rtts_us: Vec<u64>,
    disconnects: u64,
    reconnects: u64,
}

impl Bot {
    fn new(index: usize, args: &Args, now: Instant) -> Self {
        let mut rng = Rng(0x5eed ^ (index as u64) << 8 ^ args.prefix.len() as u64);
        let home = match args.scenario {
            // Deterministic scatter; margin keeps bots on cooked terrain.
            Scenario::Uniform | Scenario::Churn | Scenario::Hotspot => [
                rng.range(-args.area, args.area),
                rng.range(-args.area, args.area),
            ],
            Scenario::Thrash => [args.border, rng.range(-args.area, args.area)],
        };
        let mut client = SpacetimeNetClient::new();
        client.set_net_id(format!("{}-{index}", args.prefix));
        client.connect(&ModuleAddr {
            host: args.host.clone(),
            module: args.module.clone(),
        });
        Self {
            client,
            heading: rng.range(0.0, std::f32::consts::TAU),
            rng,
            events: Vec::new(),
            epoch: 0,
            seq: 0,
            pos: [0.0; 3],
            alive: true,
            in_world: false,
            teleported: false,
            home,
            next_heading: now,
            next_input: now + Duration::from_millis((index % 20) as u64 * 2),
            next_cast: now + Duration::from_secs_f32(3.0 + (index % 7) as f32 * 0.5),
            next_flip: None,
            connected: true,
            targets: Vec::new(),
            own_entity_id: None,
            snapshots: 0,
            state_updates: 0,
            coarse_updates: 0,
            coarse_seen: std::collections::HashMap::new(),
            rtts_us: Vec::new(),
            disconnects: 0,
            reconnects: 0,
        }
    }

    fn absorb_snapshot(&mut self, s: &WorldSnapshot) {
        self.snapshots += 1;
        self.own_entity_id = s.own_entity_id;
        self.targets = s
            .entities
            .iter()
            .filter(|e| e.alive && Some(e.entity_id) != s.own_entity_id)
            .map(|e| e.entity_id)
            .collect();
        for c in &s.coarse {
            let last = self.coarse_seen.entry(c.entity_id).or_insert(0);
            if c.server_time_us > *last {
                *last = c.server_time_us;
                self.coarse_updates += 1;
            }
        }
    }

    fn pump(&mut self, now: Instant) {
        let mut events = std::mem::take(&mut self.events);
        self.client.poll(&mut events);
        for ev in events.drain(..) {
            match ev {
                NetEvent::Snapshot(s) => self.absorb_snapshot(&s),
                NetEvent::OwnStateAck {
                    epoch, pos, alive, ..
                } => {
                    if epoch != self.epoch {
                        self.epoch = epoch;
                        self.seq = 0;
                    }
                    self.pos = pos;
                    self.alive = alive;
                    self.in_world = true;
                }
                NetEvent::StateUpdate(_) => self.state_updates += 1,
                NetEvent::ClockSample(c) => self.rtts_us.push(c.rtt_us),
                NetEvent::Disconnected(_) => {
                    self.disconnects += 1;
                    self.in_world = false;
                    self.teleported = false;
                }
                _ => {}
            }
        }
        self.events = events;
        let _ = now;
    }

    fn move_dir(&mut self, now: Instant, args: &Args, elapsed: f32) -> [f32; 2] {
        match args.scenario {
            Scenario::Uniform | Scenario::Churn => {
                if now >= self.next_heading {
                    self.heading += self.rng.range(-1.2, 1.2);
                    self.next_heading = now + Duration::from_secs_f32(self.rng.range(2.0, 4.0));
                }
                // Steer home beyond a small wander disc.
                let (dx, dy) = (self.home[0] - self.pos[0], self.home[1] - self.pos[1]);
                if dx * dx + dy * dy > 20.0 * 20.0 {
                    self.heading = dy.atan2(dx);
                }
                [self.heading.cos(), self.heading.sin()]
            }
            Scenario::Hotspot => {
                let goal = if elapsed < args.disperse {
                    args.center
                } else {
                    self.home
                };
                let (dx, dy) = (goal[0] - self.pos[0], goal[1] - self.pos[1]);
                if dx * dx + dy * dy < 4.0 {
                    // Mill around the goal.
                    if now >= self.next_heading {
                        self.heading = self.rng.range(0.0, std::f32::consts::TAU);
                        self.next_heading = now + Duration::from_secs_f32(self.rng.range(1.0, 2.0));
                    }
                } else {
                    self.heading = dy.atan2(dx);
                }
                [self.heading.cos(), self.heading.sin()]
            }
            Scenario::Thrash => {
                // Pace across the border: flip direction ±6 m either side.
                if self.pos[0] > args.border + 6.0 {
                    self.heading = std::f32::consts::PI;
                } else if self.pos[0] < args.border - 6.0 {
                    self.heading = 0.0;
                }
                [self.heading.cos(), self.heading.sin()]
            }
        }
    }

    fn step(&mut self, now: Instant, args: &Args, elapsed: f32) {
        // Churn lifecycle first: a disconnected bot only waits for its
        // reconnect slot.
        if args.scenario == Scenario::Churn {
            let flip = *self
                .next_flip
                .get_or_insert_with(|| now + Duration::from_secs_f32(self.rng.range(10.0, 20.0)));
            if now >= flip {
                if self.connected {
                    self.client.disconnect();
                    self.connected = false;
                    self.next_flip = Some(now + Duration::from_secs_f32(self.rng.range(1.0, 3.0)));
                } else {
                    self.client.connect(&ModuleAddr {
                        host: args.host.clone(),
                        module: args.module.clone(),
                    });
                    self.connected = true;
                    self.reconnects += 1;
                    self.next_flip =
                        Some(now + Duration::from_secs_f32(self.rng.range(10.0, 20.0)));
                }
            }
        }

        self.pump(now);
        if !self.connected || !self.in_world {
            return;
        }

        if !self.teleported {
            // Drop from above the terrain surface (~8±8 m) at the scenario
            // start point; the interest anchor follows the own row.
            let start = match args.scenario {
                Scenario::Thrash => [args.border - 5.0, self.home[1]],
                _ => self.home,
            };
            self.client.dev_teleport([start[0], start[1], 30.0]);
            self.teleported = true;
            return;
        }

        if now >= self.next_input && self.alive {
            self.next_input = now + INPUT_INTERVAL;
            let dir = self.move_dir(now, args, elapsed);
            self.seq += 1;
            self.client.send_input(&ClientInput {
                epoch: self.epoch, // re-stamped by the client
                seq: self.seq,
                move_dir: dir,
                yaw: dir[1].atan2(dir[0]),
                sprint: false,
                jump: false,
            });
            self.client.set_interest_hint([self.pos[0], self.pos[1]]);
        }

        if now >= self.next_cast && self.alive {
            self.next_cast = now + Duration::from_secs_f32(self.rng.range(3.0, 6.0));
            match self.targets.first() {
                Some(&t) => self.client.cast_ability(STRIKE, t),
                None => self.client.cast_ability(NOVA, 0),
            }
        }
    }
}

fn percentile(sorted: &[u64], p: f32) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let idx = ((sorted.len() - 1) as f32 * p).round() as usize;
    sorted[idx]
}

fn main() {
    let args = parse_args();
    let start = Instant::now();
    let mut bots: Vec<Bot> = (0..args.bots).map(|i| Bot::new(i, &args, start)).collect();
    let deadline = start + Duration::from_secs_f32(args.duration);
    eprintln!(
        "net_bots: {} bots, scenario {}, {}s",
        args.bots,
        match args.scenario {
            Scenario::Uniform => "uniform",
            Scenario::Hotspot => "hotspot",
            Scenario::Churn => "churn",
            Scenario::Thrash => "thrash",
        },
        args.duration
    );

    let mut last_progress = start;
    while Instant::now() < deadline {
        let now = Instant::now();
        let elapsed = (now - start).as_secs_f32();
        for bot in &mut bots {
            bot.step(now, &args, elapsed);
        }
        if now - last_progress >= Duration::from_secs(10) {
            last_progress = now;
            let in_world = bots.iter().filter(|b| b.in_world).count();
            eprintln!("t={elapsed:.0}s in_world={in_world}/{}", bots.len());
        }
        std::thread::sleep(Duration::from_millis(2));
    }

    // Report.
    let dur = args.duration as f64;
    let mut rtts: Vec<u64> = bots.iter().flat_map(|b| b.rtts_us.iter().copied()).collect();
    rtts.sort_unstable();
    let snapshots: u64 = bots.iter().map(|b| b.snapshots).sum();
    let updates: u64 = bots.iter().map(|b| b.state_updates).sum();
    let coarse: u64 = bots.iter().map(|b| b.coarse_updates).sum();
    let disconnects: u64 = bots.iter().map(|b| b.disconnects).sum();
    let reconnects: u64 = bots.iter().map(|b| b.reconnects).sum();
    let swaps: Vec<u32> = bots.iter().map(|b| b.client.interest_swap_count()).collect();
    let n = bots.len() as f64;
    println!("--- net_bots report ({} bots, {dur:.0}s) ---", bots.len());
    println!(
        "rtt_ms p50={:.1} p95={:.1} n={}",
        percentile(&rtts, 0.50) as f64 / 1000.0,
        percentile(&rtts, 0.95) as f64 / 1000.0,
        rtts.len()
    );
    println!(
        "per-bot/s: snapshots={:.2} full_row_updates={:.2} coarse_updates={:.2}",
        snapshots as f64 / n / dur,
        updates as f64 / n / dur,
        coarse as f64 / n / dur
    );
    println!(
        "interest swaps/bot: min={} max={} mean={:.1}",
        swaps.iter().min().copied().unwrap_or(0),
        swaps.iter().max().copied().unwrap_or(0),
        swaps.iter().map(|&s| s as f64).sum::<f64>() / n
    );
    println!("disconnects={disconnects} reconnects={reconnects}");
    for mut bot in bots {
        bot.client.disconnect();
    }
}
