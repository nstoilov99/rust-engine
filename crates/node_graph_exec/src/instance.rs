//! Instance state (D1): variables, live execution threads, the queued event
//! FIFO and the RNG — **all plain serializable data**, so an instance can be
//! written to disk mid-wait (the P4 case) and resumed.

use std::collections::{BTreeMap, VecDeque};

use node_graph_types::EventPhase;
use serde::{Deserialize, Serialize};

use crate::node::{ExecError, Suspension};
use crate::plan::Plan;
use crate::value::{EntityRef, Value};

/// Unique within one instance, never reused. Latent state is keyed by this,
/// which is what lets two concurrent activations of the same Delay node
/// suspend independently (D1).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub struct ActivationId(pub u64);

/// One loop frame, owned by the interpreter. The node re-derives its state
/// from `iteration`, so implementations stay stateless and the whole stack
/// serializes (D1's Delay-inside-ForLoop requirement).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Frame {
    /// The plan node that owns this loop.
    pub node: usize,
    /// Which body run this is: 0 on the first.
    pub iteration: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ThreadState {
    /// Ready to continue at `cursor`.
    Running,
    /// Waiting; the runner resumes it when the condition is met (P4).
    Suspended(Suspension),
    /// Finished normally.
    Finished,
    /// Stopped by an error. Kept rather than dropped so the editor can show
    /// which activation died and why.
    Failed(ExecError),
}

/// One execution thread: an activation of one entry point.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Activation {
    pub id: ActivationId,
    /// The entry node this started from, for reporting.
    pub entry: usize,
    /// Continuation point: the plan node to fire next, and the exec input
    /// pin it is being entered through. The pin is part of the continuation
    /// because a node with several exec inputs (Gate's open/close/enter)
    /// behaves differently per entrance, and only the edge knows which one
    /// was taken.
    pub cursor: Option<usize>,
    pub entered: Option<String>,
    /// Loop/call frames, innermost last.
    pub frames: Vec<Frame>,
    /// Data outputs impure nodes have produced in this activation. Pure
    /// outputs are *not* here — they are statement-scoped and re-evaluated
    /// per firing (D1, no cross-statement caching in v1).
    pub locals: BTreeMap<usize, BTreeMap<String, Value>>,
    /// Payload delivered by the event that started this activation, exposed
    /// through the entry node's payload output pins.
    pub payload: BTreeMap<String, Value>,
    /// The latent node and exec output pin this activation is parked on, set
    /// while suspended (45-A review, finding 7).
    ///
    /// Suspension resolves its continuation immediately but does not *take*
    /// it until the activation is due — so the edge is known at suspend time
    /// and travelled at wake time, and only the second is the truth a pulse
    /// should show. Recording it here is what lets the wake light the wire
    /// out of a `Delay` at the moment control actually leaves it. Plain data,
    /// so an instance still serializes mid-wait.
    #[serde(default)]
    pub resume_edge: Option<(usize, String)>,
    pub state: ThreadState,
}

/// A queued event, waiting for the next tick.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueuedEvent {
    pub phase: EventPhase,
    /// The custom-event or input-action name this addresses. `None` matches
    /// every entry of the phase.
    pub name: Option<String>,
    pub payload: BTreeMap<String, Value>,
}

/// SplitMix64 — small, fast, and identical on every target, which is the
/// whole requirement (D8: RNG is a seeded per-instance PRNG, and iteration
/// order is stable). Not cryptographic and not trying to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rng {
    state: u64,
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `[0, 1)`. Built from the top 24 bits so the result is
    /// exactly representable in `f32` on every target.
    pub fn next_f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u32 << 24) as f32
    }

    /// Uniform in `[min, max]`. `min > max` yields `min` rather than
    /// panicking: a graph is user input.
    pub fn next_range(&mut self, min: i32, max: i32) -> i32 {
        if min >= max {
            return min;
        }
        let span = (max as i64 - min as i64 + 1) as u64;
        (min as i64 + (self.next_u64() % span) as i64) as i32
    }
}

/// Everything one running graph owns.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphInstance {
    pub variables: BTreeMap<String, Value>,
    pub threads: Vec<Activation>,
    pub queue: VecDeque<QueuedEvent>,
    pub rng: Rng,
    /// Per-node persistent state, keyed by plan index — the one thing a
    /// stateful node (Gate's open/closed, DoOnce's fired, FlipFlop's side)
    /// cannot re-derive from its inputs and its frame. It lives here rather
    /// than in the implementation so implementations stay stateless and the
    /// whole instance stays serializable.
    pub node_state: BTreeMap<usize, Value>,
    /// Spawn aliases handed out but not yet bound to a real entity. The
    /// runner drains this after applying the tick's effects and records its
    /// own alias -> entity mapping; real ids never enter this crate (D1).
    pub pending_aliases: Vec<u32>,
    /// The entity this instance is attached to, as the core may see it.
    pub self_entity: EntityRef,
    /// BeginPlay fires exactly once per instance lifetime (D3). Restart
    /// re-arms it because the instance itself is recreated, not reset.
    pub begin_play_pending: bool,
    /// Instance time, advanced by the runner's `dt`.
    pub time: f64,
    pub next_activation: u64,
    pub next_alias: u32,
    /// Set when the budget or an error stopped the instance. A stopped
    /// instance ticks no further; the runner reports it once.
    pub halted: Option<ExecError>,
}

impl GraphInstance {
    /// A fresh instance of `plan`, with variables seeded from their declared
    /// defaults and BeginPlay armed.
    pub fn new(plan: &Plan, self_entity: EntityRef, seed: u64) -> Self {
        Self {
            variables: plan
                .variables
                .iter()
                .map(|v| (v.slug.clone(), v.initial.clone()))
                .collect(),
            threads: Vec::new(),
            queue: VecDeque::new(),
            rng: Rng::new(seed),
            node_state: BTreeMap::new(),
            pending_aliases: Vec::new(),
            self_entity,
            begin_play_pending: true,
            time: 0.0,
            next_activation: 0,
            next_alias: 0,
            halted: None,
        }
    }

    /// Queue an event for the *next* tick. Emitting during a tick never
    /// re-enters the graph (D3: no reentrancy).
    pub fn queue_event(&mut self, phase: EventPhase, name: Option<String>, payload: BTreeMap<String, Value>) {
        self.queue.push_back(QueuedEvent { phase, name, payload });
    }

    /// Take the aliases the runner still has to bind. Draining is the
    /// handshake: what is left in the list is what has not been spawned yet.
    pub fn take_pending_aliases(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.pending_aliases)
    }

    pub(crate) fn next_activation_id(&mut self) -> ActivationId {
        let id = ActivationId(self.next_activation);
        self.next_activation += 1;
        id
    }

    /// Activations still doing something. Everything else is history the
    /// editor may still want to show.
    pub fn live_threads(&self) -> usize {
        self.threads
            .iter()
            .filter(|t| !matches!(t.state, ThreadState::Finished | ThreadState::Failed(_)))
            .count()
    }
}
