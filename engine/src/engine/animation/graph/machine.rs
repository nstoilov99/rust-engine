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

use super::plan::{AnimGraphPlan, CmpOp, MathOp, ParamDecl, RuleExpr, TransitionFrom};

// ---------------------------------------------------------------------------
// Parameters
// ---------------------------------------------------------------------------

/// A parameter's runtime value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ParamValue {
    Float(f32),
    Bool(bool),
    /// A one-shot: `true` = set (fired by gameplay, not yet consumed). It
    /// stays set across frames until a transition whose rule reads it fires
    /// — consume-on-transition, so one-shots are never silently lost.
    Trigger(bool),
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

    /// Set a Trigger parameter. `false` = refused (undeclared or not a
    /// Trigger). Idempotent while set: firing twice before a consume is one
    /// buffered shot, not two.
    pub fn fire_trigger(&mut self, slug: &str) -> bool {
        match self.values.get_mut(slug) {
            Some(v @ ParamValue::Trigger(_)) => {
                *v = ParamValue::Trigger(true);
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

    /// Is the Trigger currently set (fired and not yet consumed)?
    pub fn trigger_set(&self, slug: &str) -> Option<bool> {
        match self.values.get(slug) {
            Some(ParamValue::Trigger(b)) => Some(*b),
            _ => None,
        }
    }

    /// Spend a Trigger. The machine's seam, called when a transition whose
    /// rule reads the trigger fires — gameplay never consumes.
    pub(crate) fn consume_trigger(&mut self, slug: &str) {
        if let Some(v @ ParamValue::Trigger(_)) = self.values.get_mut(slug) {
            *v = ParamValue::Trigger(false);
        }
    }
}

// ---------------------------------------------------------------------------
// Rule evaluation
// ---------------------------------------------------------------------------

impl RuleExpr {
    /// Evaluate a Bool-typed expression. The compiler guarantees the typing,
    /// so the Float-only variants land in the benign `false` arm only if a
    /// plan was built by hand against the rules.
    pub fn eval_bool(&self, params: &AnimParams) -> bool {
        match self {
            RuleExpr::ConstBool(b) => *b,
            RuleExpr::ParamBool(slug) => params.get_bool(slug).unwrap_or(false),
            RuleExpr::ParamTrigger(slug) => params.trigger_set(slug).unwrap_or(false),
            RuleExpr::Compare(op, a, b) => {
                let (a, b) = (a.eval_float(params), b.eval_float(params));
                match op {
                    CmpOp::Equal => a == b,
                    CmpOp::NotEqual => a != b,
                    CmpOp::Less => a < b,
                    CmpOp::LessEqual => a <= b,
                    CmpOp::Greater => a > b,
                    CmpOp::GreaterEqual => a >= b,
                }
            }
            RuleExpr::And(a, b) => a.eval_bool(params) && b.eval_bool(params),
            RuleExpr::Or(a, b) => a.eval_bool(params) || b.eval_bool(params),
            RuleExpr::Not(a) => !a.eval_bool(params),
            RuleExpr::ConstFloat(_) | RuleExpr::ParamFloat(_) | RuleExpr::Math(..) => false,
        }
    }

    /// Evaluate a Float-typed expression (same typing guarantee, `0.0` arm).
    fn eval_float(&self, params: &AnimParams) -> f32 {
        match self {
            RuleExpr::ConstFloat(f) => *f,
            RuleExpr::ParamFloat(slug) => params.get_float(slug).unwrap_or(0.0),
            RuleExpr::Math(op, a, b) => {
                let (a, b) = (a.eval_float(params), b.eval_float(params));
                match op {
                    MathOp::Add => a + b,
                    MathOp::Sub => a - b,
                    MathOp::Mul => a * b,
                    // The std library's division answer: 0 when b is 0.
                    MathOp::Div => {
                        if b == 0.0 {
                            0.0
                        } else {
                            a / b
                        }
                    }
                }
            }
            _ => 0.0,
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
    /// Interruption rule v1: while a crossfade runs, only an **Any State**
    /// transition may start — ordinary transitions wait for the fade to
    /// finish. `params` is mutable because firing consumes the triggers the
    /// firing rule reads (consume-on-transition); rules themselves stay pure.
    pub fn tick(&mut self, plan: &AnimGraphPlan, params: &mut AnimParams, dt: f32) {
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

        // Transitions are pre-sorted (priority, then node id): the first
        // candidate whose rule passes wins, deterministically — Any State
        // competes with ordinary transitions on the same priority scale.
        let fading = self.fade.is_some();
        let current = self.current;
        let fired = plan
            .transitions
            .iter()
            .filter(|t| match t.from {
                TransitionFrom::State(s) => !fading && s == current,
                // Skipping self-targets is what keeps a held Any State rule
                // (Died = true) from restarting its state every frame — and,
                // mid-fade, from re-interrupting into the fade's own target.
                TransitionFrom::AnyState => t.to != current,
            })
            .find(|t| t.rule.as_ref().is_none_or(|r| r.expr.eval_bool(params)));
        if let Some(t) = fired {
            // Consume-on-transition: the fire spends every trigger the rule
            // reads, exactly once — they were consulted, they are consumed.
            if let Some(rule) = &t.rule {
                for slug in &rule.triggers {
                    params.consume_trigger(slug);
                }
            }
            // On an Any State interrupt this replaces the running fade: the
            // new outgoing side is the interrupted fade's *target* (the
            // dominant clip by then); the old outgoing state's residual
            // contribution is dropped — the accepted v1 simplification.
            self.fade = (t.duration > 0.0).then(|| Crossfade {
                from: current,
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
