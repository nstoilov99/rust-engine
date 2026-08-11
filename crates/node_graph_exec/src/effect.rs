//! The world-facing seam (D1): every action a graph takes on the world is an
//! **emitted effect**, and every read goes through [`WorldRead`]. The core
//! mutates nothing else, which is what makes a run replayable and what lets
//! the same interpreter run inside a SpacetimeDB module later.
//!
//! Effects are fine-grained (resolved question 1): `SetPosition`, not
//! "apply this closure".

use serde::{Deserialize, Serialize};

use crate::value::{EntityRef, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LogLevel {
    Info,
    Warning,
    Error,
}

/// One world-touching action. The runner (45-A P5) decides how each is
/// applied — directly for non-structural effects, through the command buffer
/// for structural ones.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Effect {
    /// Console + on-screen print, tagged with the graph by the runner.
    Log { level: LogLevel, text: String },
    /// Non-structural: the runner writes it directly, having declared
    /// `writes::<Transform>()`.
    SetPosition { entity: EntityRef, position: [f32; 3] },
    /// Structural: goes through the command buffer. `alias` is the
    /// instance-local id the graph already holds — the runner writes the real
    /// entity back against it once the spawn runs.
    SpawnPrefab { path: String, alias: u32, position: [f32; 3] },
    /// Structural.
    DestroyEntity { entity: EntityRef },
    /// A custom event, queued for the *next* tick of the same instance
    /// (same-entity scope in v1, resolved question 3).
    EmitEvent { name: String, payload: Vec<(String, Value)> },
}

/// Where emitted effects go. `Vec<Effect>` is the implementation the tests
/// and the determinism harness use; the engine runner supplies its own.
pub trait EffectSink {
    fn emit(&mut self, effect: Effect);
}

impl EffectSink for Vec<Effect> {
    fn emit(&mut self, effect: Effect) {
        self.push(effect);
    }
}

/// A snapshot read of the world. The interpreter never holds this across a
/// tick — it is borrowed for the duration of one `tick` call, so there is no
/// way for a graph to observe a half-applied frame.
pub trait WorldRead {
    fn position(&self, entity: EntityRef) -> Option<[f32; 3]>;
    fn exists(&self, entity: EntityRef) -> bool;
}

/// A world that is not there: every read is absent. The default for headless
/// tests and for a graph running before its entity exists.
pub struct NoWorld;

impl WorldRead for NoWorld {
    fn position(&self, _entity: EntityRef) -> Option<[f32; 3]> {
        None
    }
    fn exists(&self, _entity: EntityRef) -> bool {
        false
    }
}
