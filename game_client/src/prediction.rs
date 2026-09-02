//! Client-side prediction and reconciliation (M6 D4).
//!
//! Owns the input sequence space: one seq per fixed prediction step, stepped
//! with the same `game_shared::motion` controller and collision content as
//! the server. `OwnStateAck` events reconcile — matching acks just trim the
//! record buffer; mispredictions reset to server state and replay unacked
//! inputs, with the positional jump absorbed by a decaying visual offset.

use game_shared::collision::format::CollisionManifest;
use game_shared::collision::ChunkStore;
use game_shared::motion::{self, MotionConfig, MotionState, MoveIntent, MOVE_DT};
use game_shared::net::protocol::ClientInput;
use glam::Vec3;
use std::collections::VecDeque;

/// Bounded record buffer: 40 steps = 2 s at 20 Hz, far beyond ack RTT.
const MAX_RECORDS: usize = 40;
/// Catch-up cap per frame; excess accumulated time is dropped (hitch, not
/// spiral).
const MAX_CATCHUP_STEPS: u32 = 5;
/// Reconciliation tolerances (plan D4): 1 cm position, 0.1 rad yaw.
const POS_EPSILON: f32 = 0.01;
const YAW_EPSILON: f32 = 0.1;
/// Visual error offset decays to ~zero in about this long.
const ERROR_DECAY_SECS: f32 = 0.2;

struct Record {
    seq: u32,
    intent: MoveIntent,
    /// Predicted state *after* applying `intent`.
    state: MotionState,
}

pub struct Prediction {
    cfg: MotionConfig,
    /// Prediction's own collision store (independent of the render/streaming
    /// `CollisionWorld`), loaded once from the cooked greybox content.
    store: Option<ChunkStore>,
    /// Sequence-space epoch from the server; `None` until the first ack.
    epoch: Option<u32>,
    next_seq: u32,
    state: Option<MotionState>,
    /// State before the newest fixed step; the render pose interpolates
    /// prev → state by the accumulator fraction so 20 Hz steps don't
    /// stair-step at display rate.
    prev_state: Option<MotionState>,
    records: VecDeque<Record>,
    accumulator: f32,
    /// Visual-only offset added to the predicted pose, absorbing
    /// reconciliation snaps; decays over `ERROR_DECAY_SECS`.
    error_offset: Vec3,
    /// Jump press latched until a prediction step consumes it.
    pending_jump: bool,
    /// A stop sample (zero move) is still owed to the server.
    stop_pending: bool,
    /// M7 D5: dead freezes prediction (no steps, no input samples) on the
    /// authoritative corpse state; respawn arrives as an epoch bump.
    alive: bool,
}

impl Prediction {
    pub fn new() -> Self {
        Self {
            cfg: MotionConfig::default(),
            store: load_store(),
            epoch: None,
            next_seq: 1,
            state: None,
            prev_state: None,
            records: VecDeque::new(),
            accumulator: 0.0,
            error_offset: Vec3::ZERO,
            pending_jump: false,
            stop_pending: false,
            alive: true,
        }
    }

    /// Predicted pose for the local player entity: `(pos, yaw)` with the
    /// visual error offset applied, interpolated between the last two fixed
    /// steps by the accumulator fraction. `None` until the first ack seeds
    /// state.
    pub fn visual_pose(&self) -> Option<([f32; 3], f32)> {
        let s = self.state.as_ref()?;
        let (pos, yaw) = self.lerped_pose(s);
        Some(((pos + self.error_offset).to_array(), yaw))
    }

    /// Controller-state sample for animation derivation (ADR 0002): velocity
    /// and grounded from the newest fixed step. `None` until the first ack
    /// seeds state.
    pub fn anim_motion(&self) -> Option<([f32; 3], bool)> {
        self.state.as_ref().map(|s| (s.vel.to_array(), s.grounded))
    }

    /// Authoritative own-row alive flag (M7 D5); `true` before the first ack.
    pub fn alive(&self) -> bool {
        self.alive
    }

    fn lerped_pose(&self, s: &MotionState) -> (Vec3, f32) {
        match &self.prev_state {
            Some(p) => {
                let alpha = (self.accumulator / MOVE_DT).clamp(0.0, 1.0);
                (
                    p.pos.lerp(s.pos, alpha),
                    p.yaw + angle_diff(s.yaw, p.yaw) * alpha,
                )
            }
            None => (s.pos, s.yaw),
        }
    }

    /// Authoritative own-row state (replication IS the ack). An epoch change
    /// restarts the sequence space; otherwise trim acked records and check
    /// the prediction made at `seq` against the server's result.
    pub fn on_ack(&mut self, epoch: u32, seq: u32, pos: [f32; 3], vel: [f32; 3], yaw: f32, grounded: bool, alive: bool) {
        let server = MotionState {
            pos: Vec3::from(pos),
            vel: Vec3::from(vel),
            yaw,
            grounded,
            ground_ref: None,
        };
        self.alive = alive;
        if self.epoch != Some(epoch) {
            self.epoch = Some(epoch);
            self.next_seq = 1;
            self.records.clear();
            self.accumulator = 0.0;
            self.error_offset = Vec3::ZERO;
            self.state = Some(server);
            self.prev_state = None;
            return;
        }
        if !alive {
            // Death snap: the server row is inert until respawn — freeze on
            // the corpse state and discard everything in flight.
            self.records.clear();
            self.accumulator = 0.0;
            self.pending_jump = false;
            self.stop_pending = false;
            self.error_offset = Vec3::ZERO;
            self.state = Some(server);
            self.prev_state = None;
            return;
        }
        let Some(state) = self.state else {
            self.state = Some(server);
            self.prev_state = None;
            return;
        };

        // Drop records the server has consumed, keeping the prediction that
        // corresponds to this ack. If none matches (idle freeze: seq stays
        // put while the server may still simulate), compare the live state.
        let mut predicted = state;
        while let Some(front) = self.records.front() {
            if front.seq > seq {
                break;
            }
            let r = self.records.pop_front().expect("front checked");
            if r.seq == seq {
                predicted = r.state;
            }
        }

        let pos_err = (predicted.pos - server.pos).length();
        let yaw_err = angle_diff(predicted.yaw, server.yaw).abs();
        if pos_err <= POS_EPSILON && yaw_err <= YAW_EPSILON {
            return;
        }

        // Mispredict: reset to the server state, replay unacked inputs, and
        // move the visual discontinuity into the decaying error offset.
        let old_visual = self.lerped_pose(&state).0 + self.error_offset;
        let mut new_state = server;
        if let Some(store) = &self.store {
            for rec in self.records.iter_mut() {
                new_state = motion::step(&self.cfg, &new_state, &rec.intent, store);
                rec.state = new_state;
            }
        }
        self.error_offset = old_visual - new_state.pos;
        self.state = Some(new_state);
        self.prev_state = None;
    }

    /// Fixed-step prediction: consume accumulated time, one input sample per
    /// step pushed to `out` (seq owned here; epoch stamped by the backend).
    /// Idle freeze: while grounded with no input and no stop sample owed,
    /// no steps run and the seq space stands still — both sides are at a
    /// stable rest state, so there is nothing to predict.
    pub fn update(&mut self, dt: f32, move_dir: [f32; 2], yaw: f32, sprint: bool, jump_pressed: bool, out: &mut Vec<ClientInput>) {
        if !self.alive {
            return;
        }
        if jump_pressed {
            self.pending_jump = true;
        }
        let (Some(mut state), Some(store), Some(epoch)) = (self.state, self.store.as_ref(), self.epoch) else {
            return;
        };

        let moving = move_dir != [0.0, 0.0];
        if moving {
            self.stop_pending = true;
        }
        if !(moving || self.pending_jump || !state.grounded || self.stop_pending) {
            // At rest both sides agree; render the rest pose exactly.
            self.accumulator = 0.0;
            self.prev_state = None;
            self.decay_error(dt);
            return;
        }

        self.accumulator += dt;
        let mut steps = 0;
        while self.accumulator >= MOVE_DT && steps < MAX_CATCHUP_STEPS {
            self.accumulator -= MOVE_DT;
            steps += 1;
            self.prev_state = Some(state);
            let intent = MoveIntent {
                move_dir,
                yaw,
                sprint,
                jump: std::mem::take(&mut self.pending_jump),
            };
            state = motion::step(&self.cfg, &state, &intent, store);
            if !moving && state.grounded {
                self.stop_pending = false;
            }
            let seq = self.next_seq;
            self.next_seq += 1;
            self.records.push_back(Record { seq, intent, state });
            if self.records.len() > MAX_RECORDS {
                self.records.pop_front();
            }
            out.push(ClientInput {
                epoch,
                seq,
                move_dir: intent.move_dir,
                yaw: intent.yaw,
                sprint: intent.sprint,
                jump: intent.jump,
            });
        }
        if self.accumulator >= MOVE_DT {
            // Catch-up cap hit: drop the excess instead of spiraling.
            self.accumulator %= MOVE_DT;
        }
        self.state = Some(state);
        self.decay_error(dt);
    }

    fn decay_error(&mut self, dt: f32) {
        self.error_offset *= (1.0 - dt / ERROR_DECAY_SECS).max(0.0);
        if self.error_offset.length_squared() < 1e-8 {
            self.error_offset = Vec3::ZERO;
        }
    }
}

fn angle_diff(a: f32, b: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    (a - b + PI).rem_euclid(TAU) - PI
}

/// Load the cooked greybox collision content prediction simulates against —
/// the same chunk set whose manifest hash gated the connection (M6 D1).
fn load_store() -> Option<ChunkStore> {
    use rust_engine::assets::asset_source;
    let manifest = asset_source::read_string("collision/greybox/manifest.ron")
        .ok()
        .and_then(|s| ron::from_str::<CollisionManifest>(&s).ok())?;
    let mut store = ChunkStore::new();
    for chunk in &manifest.chunks {
        let rel = format!("collision/greybox/{}_{}.ccol", chunk.coord[0], chunk.coord[1]);
        match asset_source::read_bytes(&rel) {
            Ok(bytes) => {
                if let Err(e) = store.insert_chunk(&bytes) {
                    println!("net: prediction chunk {rel} failed to load: {e:?}");
                }
            }
            Err(e) => println!("net: prediction chunk {rel} unreadable: {e}"),
        }
    }
    println!("net: prediction collision store ready ({} chunks)", store.len());
    Some(store)
}
