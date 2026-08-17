//! The state machine core: parameter blackboard, active state, crossfades,
//! and pose evaluation.
//!
//! This is the evaluator seam the acceptance tests drive: a plan plus
//! parameter writes plus frame ticks in, active state and blend weights out
//! — pure CPU, no assets, no ECS. Clip sampling reuses the existing keyframe
//! functions ([`crate::engine::animation::sampling`]); a Pose here is the
//! same `[LocalBoneTransform]` the single-clip player writes.

use std::collections::BTreeMap;

use crate::engine::animation::components::LocalBoneTransform;
use crate::engine::animation::sampling;
use crate::engine::assets::model_loader::RawAnimationClip;

use super::plan::{AnimGraphPlan, ParamDecl, TransitionCondition};

// ---------------------------------------------------------------------------
// Parameters
// ---------------------------------------------------------------------------

/// A parameter's runtime value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ParamValue {
    Float(f32),
    Bool(bool),
}

/// The typed parameter blackboard — gameplay's entire surface onto an
/// animation graph (ADR 0002 posture: parameters in, never states).
///
/// Writes are checked against the declarations: an undeclared slug or a
/// mistyped write is refused (`false`) rather than silently creating an
/// entry, because the declaration is the contract the graph author published.
#[derive(Debug, Clone, Default)]
pub struct AnimParams {
    values: BTreeMap<String, ParamValue>,
}

impl AnimParams {
    pub fn from_decls(decls: &[ParamDecl]) -> Self {
        Self {
            values: decls
                .iter()
                .map(|d| (d.slug.clone(), d.default))
                .collect(),
        }
    }

    /// Write a Float parameter. `false` = refused (undeclared or not Float).
    pub fn set_float(&mut self, slug: &str, value: f32) -> bool {
        match self.values.get_mut(slug) {
            Some(v @ ParamValue::Float(_)) => {
                *v = ParamValue::Float(value);
                true
            }
            _ => false,
        }
    }

    /// Write a Bool parameter. `false` = refused (undeclared or not Bool).
    pub fn set_bool(&mut self, slug: &str, value: bool) -> bool {
        match self.values.get_mut(slug) {
            Some(v @ ParamValue::Bool(_)) => {
                *v = ParamValue::Bool(value);
                true
            }
            _ => false,
        }
    }

    pub fn get_float(&self, slug: &str) -> Option<f32> {
        match self.values.get(slug) {
            Some(ParamValue::Float(f)) => Some(*f),
            _ => None,
        }
    }

    pub fn get_bool(&self, slug: &str) -> Option<bool> {
        match self.values.get(slug) {
            Some(ParamValue::Bool(b)) => Some(*b),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Machine
// ---------------------------------------------------------------------------

/// An in-flight crossfade: the outgoing state keeps playing and blending out
/// while the (already-current) target blends in — both clips advance, unlike
/// the single-clip player's static-snapshot fade.
#[derive(Debug, Clone, PartialEq)]
pub struct Crossfade {
    /// State index being blended out.
    pub from: usize,
    /// The outgoing state's clip clock (keeps advancing during the fade).
    pub from_time: f32,
    pub elapsed: f32,
    pub duration: f32,
}

impl Crossfade {
    /// The **target** state's weight, 0→1 linearly over the duration — the
    /// same curve [`crate::engine::animation::CrossfadeState`] uses, so the
    /// two players feel identical.
    pub fn weight(&self) -> f32 {
        if self.duration <= 0.0 {
            1.0
        } else {
            (self.elapsed / self.duration).clamp(0.0, 1.0)
        }
    }
}

/// One running machine instance: active state, clip clock, optional
/// crossfade. Owns nothing document-shaped — it walks the compiled plan it
/// is ticked with.
#[derive(Debug, Clone)]
pub struct AnimMachine {
    current: usize,
    /// Seconds into the current state's clip (speed already applied; the
    /// sampler wraps by clip duration, so this only grows).
    time: f32,
    fade: Option<Crossfade>,
}

impl AnimMachine {
    /// A fresh machine sits in the plan's entry state — active from the very
    /// first tick, no transition needed to get there.
    pub fn new(plan: &AnimGraphPlan) -> Self {
        Self {
            current: plan.entry,
            time: 0.0,
            fade: None,
        }
    }

    /// Index of the active (target, if fading) state.
    pub fn current_state(&self) -> usize {
        self.current
    }

    /// Seconds into the active state's clip.
    pub fn time(&self) -> f32 {
        self.time
    }

    pub fn crossfade(&self) -> Option<&Crossfade> {
        self.fade.as_ref()
    }

    /// The active state's blend weight: 1.0 at rest, the fade's target
    /// weight while crossfading.
    pub fn blend_weight(&self) -> f32 {
        self.fade.as_ref().map(Crossfade::weight).unwrap_or(1.0)
    }

    /// Advance one frame: clip clocks first, then transition rules.
    ///
    /// Interruption rule v1: while a crossfade runs, no ordinary transition
    /// may start — they wait for it to finish. (Any State transitions, the
    /// one sanctioned interrupter, arrive with the rule-graph slice.)
    pub fn tick(&mut self, plan: &AnimGraphPlan, params: &AnimParams, dt: f32) {
        // A machine armed against an empty/refused plan has nothing to do —
        // never index into a plan that has no states.
        let Some(state) = plan.states.get(self.current) else {
            return;
        };

        self.time += dt * state.speed;
        if let Some(fade) = &mut self.fade {
            fade.elapsed += dt;
            if let Some(from) = plan.states.get(fade.from) {
                fade.from_time += dt * from.speed;
            }
            if fade.elapsed >= fade.duration {
                self.fade = None;
            }
        }

        if self.fade.is_some() {
            return;
        }
        // Transitions are pre-sorted (priority, then node id): the first
        // passing rule out of the current state wins, deterministically.
        let fired = plan
            .transitions
            .iter()
            .filter(|t| t.from == self.current)
            .find(|t| match &t.condition {
                TransitionCondition::Always => true,
                TransitionCondition::BoolParam(slug) => params.get_bool(slug).unwrap_or(false),
            });
        if let Some(t) = fired {
            self.fade = (t.duration > 0.0).then(|| Crossfade {
                from: self.current,
                from_time: self.time,
                elapsed: 0.0,
                duration: t.duration,
            });
            self.current = t.to;
            self.time = 0.0;
        }
    }
}

// ---------------------------------------------------------------------------
// Pose evaluation
// ---------------------------------------------------------------------------

/// Wrap a clip clock into the clip (cyclic playback — states loop in this
/// slice).
fn wrapped(time: f32, clip: &RawAnimationClip) -> f32 {
    if clip.duration_seconds > 0.0 {
        time.rem_euclid(clip.duration_seconds)
    } else {
        time
    }
}

/// Evaluate the machine's Pose into `pose` (the skeleton's local bone
/// transforms).
///
/// `clip_for` resolves a state index to its clip; a state whose clip is
/// unavailable contributes nothing (the pose holds), which is the degraded
/// behavior — the runner refuses to arm on a missing clip, so this only
/// covers races. `scratch` is caller-owned so the per-frame path allocates
/// nothing at steady state.
pub fn evaluate_pose<'a>(
    machine: &AnimMachine,
    clip_for: impl Fn(usize) -> Option<&'a RawAnimationClip>,
    pose: &mut [LocalBoneTransform],
    scratch: &mut Vec<LocalBoneTransform>,
) {
    let from_clip = machine
        .crossfade()
        .and_then(|f| clip_for(f.from).map(|c| (f, c)));

    // The outgoing pose starts from the same pre-sample transforms as the
    // target's: `sample_channels` only writes bones the clip animates, and
    // unanimated bones must agree on both sides of the blend.
    if let Some((fade, clip)) = from_clip {
        scratch.clear();
        scratch.extend_from_slice(pose);
        sampling::sample_channels(&clip.channels, wrapped(fade.from_time, clip), scratch);
        if let Some(clip) = clip_for(machine.current_state()) {
            sampling::sample_channels(&clip.channels, wrapped(machine.time(), clip), pose);
        }
        let w = fade.weight();
        for (out, from) in pose.iter_mut().zip(scratch.iter()) {
            *out = from.blend(out, w);
        }
    } else if let Some(clip) = clip_for(machine.current_state()) {
        sampling::sample_channels(&clip.channels, wrapped(machine.time(), clip), pose);
    }
}
