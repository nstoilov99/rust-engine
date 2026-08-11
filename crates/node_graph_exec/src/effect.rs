//! The world-facing seam (D1): every action a graph takes on the world is an
//! **emitted effect**, and every read goes through [`WorldRead`]. The core
//! mutates nothing else, which is what makes a run replayable and what lets
//! the same interpreter run inside a SpacetimeDB module later.
//!
//! Effects are fine-grained (resolved question 1): `SetPosition`, not "apply
//! this closure" and not one omnibus `SetTransform` — a node that only moves
//! something must not force the runner to re-write its rotation.

use serde::{Deserialize, Serialize};

use crate::value::{EntityRef, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LogLevel {
    Info,
    Warning,
    Error,
}

/// A world transform as the portable core sees it: Z-up, quaternion `xyzw`,
/// plain arrays. No `glam`, no engine types — conversion is the runner's job
/// and happens on one side of the seam only.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TransformSnapshot {
    pub position: [f32; 3],
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
}

impl Default for TransformSnapshot {
    fn default() -> Self {
        Self {
            position: [0.0; 3],
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0; 3],
        }
    }
}

/// One world-touching action. The runner (45-A P5) decides how each is
/// applied — directly for non-structural effects, through the command buffer
/// for structural ones.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Effect {
    /// Console + on-screen print, tagged with the graph by the runner.
    Log { level: LogLevel, text: String },
    /// Non-structural: the runner writes these directly, having declared
    /// `writes::<Transform>()`.
    SetPosition { entity: EntityRef, position: [f32; 3] },
    /// Quaternion, `xyzw`.
    SetRotation { entity: EntityRef, rotation: [f32; 4] },
    SetScale { entity: EntityRef, scale: [f32; 3] },
    /// Structural: goes through the command buffer. `alias` is the
    /// instance-local id the graph already holds — the runner binds it to the
    /// real entity once the spawn runs, and the instance lists it in
    /// `pending_aliases` until then.
    SpawnPrefab {
        path: String,
        alias: u32,
        transform: TransformSnapshot,
    },
    /// Structural.
    DestroyEntity { entity: EntityRef },
    /// A custom event. **The core has already queued this on the emitting
    /// instance** (same-entity scope, resolved question 3); the effect is in
    /// the stream so the console and the P7 trace can see it. A runner must
    /// not deliver it a second time.
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
    fn transform(&self, entity: EntityRef) -> Option<TransformSnapshot>;

    fn exists(&self, entity: EntityRef) -> bool;

    /// Convenience over [`WorldRead::transform`]; implementors rarely need to
    /// override it.
    fn position(&self, entity: EntityRef) -> Option<[f32; 3]> {
        self.transform(entity).map(|t| t.position)
    }
}

/// A world that is not there: every read is absent. The default for headless
/// tests and for a graph running before its entity exists.
pub struct NoWorld;

impl WorldRead for NoWorld {
    fn transform(&self, _entity: EntityRef) -> Option<TransformSnapshot> {
        None
    }
    fn exists(&self, _entity: EntityRef) -> bool {
        false
    }
}
