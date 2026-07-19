//! Recorded motion traces (M6 D5): a config, a start state, and a fixed
//! intent sequence with expected per-step positions. Replayed natively in
//! tests and inside the server WASM module (`run_parity_trace`) to bound
//! native/WASM divergence. RON on disk; this module does no I/O.

use super::{step, MotionConfig, MotionState, MoveIntent};
use crate::collision::ChunkStore;
use glam::Vec3;
use serde::{Deserialize, Serialize};

/// Positional tolerance per step (1 mm): parity is envelope-based, not
/// bit-exact (float lowering differs between native and WASM).
pub const TRACE_TOLERANCE: f32 = 1e-3;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceStart {
    pub pos: [f32; 3],
    pub vel: [f32; 3],
    pub yaw: f32,
    pub grounded: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MotionTrace {
    pub name: String,
    pub config: MotionConfig,
    pub start: TraceStart,
    pub intents: Vec<MoveIntent>,
    /// Capsule-center position after each step; same length as `intents`.
    pub expected: Vec<[f32; 3]>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TraceReport {
    pub steps: u32,
    pub end_pos: Vec3,
    pub max_error: f32,
    /// Order-sensitive FNV-1a over the replayed per-step position bit
    /// patterns. Diagnostic only — never compared across native/WASM (parity
    /// is tolerance-based); a differing hash localizes which run diverged.
    pub state_hash: u64,
}

/// The checked-in parity trace battery (D5), embedded so the server WASM
/// module can replay it without I/O. Ids match the file stems.
pub const EMBEDDED_TRACES: &[(&str, &str)] = &[
    ("greybox_walk", include_str!("traces/greybox_walk.ron")),
    ("slope_up_down", include_str!("traces/slope_up_down.ron")),
    ("slope_blocked", include_str!("traces/slope_blocked.ron")),
    ("step_up", include_str!("traces/step_up.ron")),
    ("step_blocked", include_str!("traces/step_blocked.ron")),
    ("jump_gap", include_str!("traces/jump_gap.ron")),
    ("fall_land", include_str!("traces/fall_land.ron")),
    ("wall_slide", include_str!("traces/wall_slide.ron")),
    ("chunk_seam", include_str!("traces/chunk_seam.ron")),
    ("long_run", include_str!("traces/long_run.ron")),
];

fn fnv1a_extend(mut hash: u64, pos: &Vec3) -> u64 {
    for f in [pos.x, pos.y, pos.z] {
        for b in f.to_le_bytes() {
            hash ^= b as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
    }
    hash
}

pub fn load_embedded(trace_id: &str) -> Result<MotionTrace, String> {
    let (_, text) = EMBEDDED_TRACES
        .iter()
        .find(|(id, _)| *id == trace_id)
        .ok_or_else(|| format!("unknown parity trace {trace_id:?}"))?;
    ron::from_str(text).map_err(|e| format!("trace {trace_id:?} failed to parse: {e}"))
}

impl MotionTrace {
    fn start_state(&self) -> MotionState {
        MotionState {
            pos: Vec3::from(self.start.pos),
            vel: Vec3::from(self.start.vel),
            yaw: self.start.yaw,
            grounded: self.start.grounded,
            ground_ref: None,
        }
    }

    /// Record expected positions by running the controller over the intents.
    pub fn record(&mut self, store: &ChunkStore) {
        let mut state = self.start_state();
        self.expected.clear();
        for intent in &self.intents {
            state = step(&self.config, &state, intent, store);
            self.expected.push(state.pos.to_array());
        }
    }

    /// Replay against `store`; `Err` names the first step whose position
    /// diverges from `expected` by more than `TRACE_TOLERANCE`.
    pub fn replay(&self, store: &ChunkStore) -> Result<TraceReport, String> {
        if self.expected.len() != self.intents.len() {
            return Err(format!(
                "trace '{}': {} intents but {} expected positions",
                self.name,
                self.intents.len(),
                self.expected.len()
            ));
        }
        let mut state = self.start_state();
        let mut max_error = 0.0f32;
        let mut hash: u64 = 0xcbf29ce484222325; // FNV-1a offset basis
        for (i, (intent, expected)) in self.intents.iter().zip(&self.expected).enumerate() {
            state = step(&self.config, &state, intent, store);
            hash = fnv1a_extend(hash, &state.pos);
            let err = state.pos.distance(Vec3::from(*expected));
            max_error = max_error.max(err);
            if err > TRACE_TOLERANCE {
                return Err(format!(
                    "trace '{}' diverged at step {i}: got {:?}, expected {expected:?} (err {err})",
                    self.name, state.pos
                ));
            }
        }
        Ok(TraceReport {
            steps: self.intents.len() as u32,
            end_pos: state.pos,
            max_error,
            state_hash: hash,
        })
    }
}
