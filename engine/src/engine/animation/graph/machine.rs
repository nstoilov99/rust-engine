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

use super::plan::{
    AnimGraphPlan, CmpOp, MathOp, ParamDecl, PlanClip, PlanSlot, PlanTree, RuleExpr,
    TransitionFrom,
};

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
    /// The clock spans the last tick advanced, `(state, from, to)` — one per
    /// state [`evaluate_pose`] will sample this frame. What anim-event
    /// crossing detection replays; a state that stopped being sampled this
    /// tick (instant switch, dropped fade) records nothing, so nothing
    /// invisible ever fires. Reused across ticks — no steady-state allocation.
    spans: Vec<(usize, f32, f32)>,
}

impl AnimMachine {
    /// A fresh machine sits in the plan's entry state — active from the very
    /// first tick, no transition needed to get there.
    pub fn new(plan: &AnimGraphPlan) -> Self {
        Self {
            current: plan.entry,
            time: 0.0,
            fade: None,
            spans: Vec::new(),
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
        self.spans.clear();
        // A machine armed against an empty/refused plan has nothing to do —
        // never index into a plan that has no states.
        let Some(state) = plan.states.get(self.current) else {
            return;
        };

        let time_before = self.time;
        self.time += dt * state.speed;
        let mut from_span = None;
        if let Some(fade) = &mut self.fade {
            fade.elapsed += dt;
            if let Some(from) = plan.states.get(fade.from) {
                let from_before = fade.from_time;
                fade.from_time += dt * from.speed;
                from_span = Some((fade.from, from_before, fade.from_time));
            }
            if fade.elapsed >= fade.duration {
                // The retiring fade's outgoing state is not sampled this
                // frame (its weight just hit zero) — no span.
                self.fade = None;
                from_span = None;
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
            // Sampled this frame: the new target (span (0,0) — nothing to
            // cross yet) and, only if a fade keeps it visible, the outgoing
            // state's advance. An instant switch samples the target alone,
            // so the outgoing state's final sliver never fires.
            if t.duration > 0.0 {
                self.spans.push((current, time_before, self.time));
            }
            self.current = t.to;
            self.time = 0.0;
        } else {
            self.spans.push((self.current, time_before, self.time));
            if let Some(span) = from_span {
                self.spans.push(span);
            }
        }
    }

    /// The clock spans the last tick advanced — see the field doc.
    pub fn spans(&self) -> &[(usize, f32, f32)] {
        &self.spans
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

/// Caller-owned scratch buffers for pose evaluation — one per active blend
/// level, grown once and reused, so steady-state frames allocate nothing.
#[derive(Default)]
pub struct PoseScratch {
    bufs: Vec<Vec<LocalBoneTransform>>,
}

impl PoseScratch {
    pub fn new() -> Self {
        Self::default()
    }

    fn take(&mut self, level: usize) -> Vec<LocalBoneTransform> {
        if self.bufs.len() <= level {
            self.bufs.resize_with(level + 1, Vec::new);
        }
        std::mem::take(&mut self.bufs[level])
    }

    fn put(&mut self, level: usize, buf: Vec<LocalBoneTransform>) {
        self.bufs[level] = buf;
    }
}

/// The active child pair of a 1D blend: `(index, Some((index, weight)))`.
/// Endpoints (and anything outside the range) play the nearest child pure; a
/// value between two thresholds blends the bracketing pair proportionally.
fn pick_1d(children: &[(f32, PlanTree)], v: f32) -> (usize, Option<(usize, f32)>) {
    let last = children.len() - 1;
    if v <= children[0].0 {
        return (0, None);
    }
    if v >= children[last].0 {
        return (last, None);
    }
    // Between two thresholds by construction (children sorted, len >= 2).
    let i = children.iter().rposition(|(t, _)| *t <= v).unwrap_or(0);
    let (t0, t1) = (children[i].0, children[i + 1].0);
    let w = (v - t0) / (t1 - t0);
    if w <= 0.0 {
        (i, None)
    } else {
        (i, Some((i + 1, w)))
    }
}

/// The active child pair of a 2D directional blend: the input's angle picks
/// the two angularly-adjacent children (children sorted by angle, wrapping),
/// weighted by the angular fraction between them. Magnitude is ignored; a
/// zero input has no direction to read and holds the first child.
fn pick_2d(children: &[(f32, PlanTree)], x: f32, y: f32) -> (usize, Option<(usize, f32)>) {
    use std::f32::consts::TAU;
    if x * x + y * y < 1e-8 {
        return (0, None);
    }
    let theta = y.atan2(x).rem_euclid(TAU);
    let n = children.len();
    let j = children.iter().position(|(a, _)| *a > theta).unwrap_or(0);
    let i = (j + n - 1) % n;
    let span = (children[j].0 - children[i].0).rem_euclid(TAU);
    if span <= 0.0 {
        return (i, None);
    }
    let w = (theta - children[i].0).rem_euclid(TAU) / span;
    if w <= 0.0 {
        (i, None)
    } else {
        (i, Some((j, w)))
    }
}

/// The shared normalized phase of a blend node's sync group — the minimal
/// form per CONTEXT.md: the first child in sorted order that is a cyclic
/// clip is the phase reference, and every clip child samples at the same
/// normalized phase, so cyclic clips stay aligned however the weights shift
/// (no stutter). Nested blend children keep the raw clock and sync locally.
/// `None` when no clip child resolves (nothing to align).
fn sync_phase<'a, F>(children: &[(f32, PlanTree)], time: f32, clip_for: &F) -> Option<f32>
where
    F: Fn(&PlanClip) -> Option<&'a RawAnimationClip>,
{
    sync_ref_duration(children, clip_for).map(|d| (time / d).rem_euclid(1.0))
}

/// The duration of a blend node's phase-reference clip (the first child in
/// sorted order that is a cyclic clip) — the denominator behind
/// [`sync_phase`], and the clock-to-phase mapping event crossing detection
/// replays.
fn sync_ref_duration<'a, F>(children: &[(f32, PlanTree)], clip_for: &F) -> Option<f32>
where
    F: Fn(&PlanClip) -> Option<&'a RawAnimationClip>,
{
    children.iter().find_map(|(_, t)| match t {
        PlanTree::Clip(c) => clip_for(c)
            .map(|clip| clip.duration_seconds)
            .filter(|d| *d > 0.0),
        _ => None,
    })
}

/// The clock a blend child samples at: clip children follow the sync
/// group's phase, everything else keeps the state clock.
fn child_time<'a, F>(tree: &PlanTree, time: f32, phase: Option<f32>, clip_for: &F) -> f32
where
    F: Fn(&PlanClip) -> Option<&'a RawAnimationClip>,
{
    if let (PlanTree::Clip(c), Some(phase)) = (tree, phase) {
        if let Some(clip) = clip_for(c) {
            if clip.duration_seconds > 0.0 {
                return phase * clip.duration_seconds;
            }
        }
    }
    time
}

/// Evaluate one tree at `time` into `pose` — recursive tree evaluation per
/// the spec. A blend samples its (at most two) active children and mixes
/// them bone-wise through [`LocalBoneTransform::blend`] (lerp positions and
/// scale, slerp rotations). A clip whose asset is unavailable contributes
/// nothing (the pose holds) — the runner refuses to arm on a missing clip,
/// so this only covers races.
fn sample_tree<'a, F>(
    tree: &PlanTree,
    time: f32,
    params: &AnimParams,
    clip_for: &F,
    pose: &mut [LocalBoneTransform],
    scratch: &mut PoseScratch,
    level: usize,
) where
    F: Fn(&PlanClip) -> Option<&'a RawAnimationClip>,
{
    let (children, pick) = match tree {
        PlanTree::Clip(c) => {
            if let Some(clip) = clip_for(c) {
                sampling::sample_channels(&clip.channels, wrapped(time, clip), pose);
            }
            return;
        }
        PlanTree::Blend1D { param, children } => {
            (children, pick_1d(children, params.get_float(param).unwrap_or(0.0)))
        }
        PlanTree::Blend2D {
            param_x,
            param_y,
            children,
        } => (
            children,
            pick_2d(
                children,
                params.get_float(param_x).unwrap_or(0.0),
                params.get_float(param_y).unwrap_or(0.0),
            ),
        ),
    };

    let phase = sync_phase(children, time, clip_for);
    let (a, b) = pick;
    let a = &children[a].1;
    match b {
        None => {
            let t = child_time(a, time, phase, clip_for);
            sample_tree(a, t, params, clip_for, pose, scratch, level);
        }
        Some((b, w)) => {
            let b = &children[b].1;
            // Both sides start from the same pre-sample transforms:
            // `sample_channels` only writes bones a clip animates, and
            // unanimated bones must agree across the blend.
            let mut buf = scratch.take(level);
            buf.clear();
            buf.extend_from_slice(pose);
            let tb = child_time(b, time, phase, clip_for);
            sample_tree(b, tb, params, clip_for, &mut buf, scratch, level + 1);
            let ta = child_time(a, time, phase, clip_for);
            sample_tree(a, ta, params, clip_for, pose, scratch, level + 1);
            for (out, other) in pose.iter_mut().zip(buf.iter()) {
                *out = out.blend(other, w);
            }
            scratch.put(level, buf);
        }
    }
}

/// Evaluate the machine's Pose into `pose` (the skeleton's local bone
/// transforms): the active state's blend tree, mixed with the outgoing
/// state's while a crossfade runs.
///
/// `clip_for` resolves a plan clip reference to its loaded clip. `params`
/// drives blend weights (crossfades already advanced in `tick`). `scratch`
/// is caller-owned so the per-frame path allocates nothing at steady state.
pub fn evaluate_pose<'a, F>(
    machine: &AnimMachine,
    plan: &AnimGraphPlan,
    params: &AnimParams,
    clip_for: F,
    pose: &mut [LocalBoneTransform],
    scratch: &mut PoseScratch,
) where
    F: Fn(&PlanClip) -> Option<&'a RawAnimationClip>,
{
    let fading = machine
        .crossfade()
        .and_then(|f| plan.states.get(f.from).map(|s| (f, s)));

    // The outgoing pose starts from the same pre-sample transforms as the
    // target's — the same agreement `sample_tree` keeps inside a blend.
    if let Some((fade, from_state)) = fading {
        let mut buf = scratch.take(0);
        buf.clear();
        buf.extend_from_slice(pose);
        sample_tree(
            &from_state.tree,
            fade.from_time,
            params,
            &clip_for,
            &mut buf,
            scratch,
            1,
        );
        if let Some(cur) = plan.states.get(machine.current_state()) {
            sample_tree(
                &cur.tree,
                machine.time(),
                params,
                &clip_for,
                pose,
                scratch,
                1,
            );
        }
        let w = fade.weight();
        for (out, from) in pose.iter_mut().zip(buf.iter()) {
            *out = from.blend(out, w);
        }
        scratch.put(0, buf);
    } else if let Some(cur) = plan.states.get(machine.current_state()) {
        sample_tree(
            &cur.tree,
            machine.time(),
            params,
            &clip_for,
            pose,
            scratch,
            0,
        );
    }
}

// ---------------------------------------------------------------------------
// Play-once slot
// ---------------------------------------------------------------------------

/// The play-once slot: v1's single override channel (CONTEXT.md). At most one
/// clip plays over the base result at a time; starting another — the same
/// slot re-triggered or a different one — replaces it. Lives beside the
/// machine, not inside it: the machine's asset-free invariant holds, and the
/// spec's frame order (machine → blend trees → play-once overlay) stays
/// visible in the call sequence.
#[derive(Debug, Clone, Default)]
pub struct PlayOnceSlot {
    playing: Option<SlotPlayback>,
}

#[derive(Debug, Clone)]
struct SlotPlayback {
    /// Index into [`AnimGraphPlan::slots`].
    slot: usize,
    /// The last tick's clock span (event crossing detection replays it).
    prev: f32,
    time: f32,
    /// The clip's duration, captured at start — the one place a duration
    /// crosses over from the assets, via `tick`'s resolver.
    len: f32,
}

impl PlayOnceSlot {
    pub fn new() -> Self {
        Self::default()
    }

    /// Advance the channel and start requested slots. Runs after the
    /// machine's tick (its transitions consume their triggers first).
    ///
    /// Starting is trigger-driven so gameplay stays inside the parameter
    /// contract: the first slot in plan order whose Trigger is set takes the
    /// channel and consumes the trigger (consume-on-start); other set
    /// triggers stay buffered and take the channel — replacing — on a later
    /// tick. A running slot retires one tick after its clock passes the clip
    /// end: past `len` the overlay weight is already 0, and the extra tick
    /// lets markers on the final frame fire before the playback is dropped.
    pub fn tick<'a, F>(
        &mut self,
        plan: &AnimGraphPlan,
        params: &mut AnimParams,
        dt: f32,
        clip_for: &F,
    ) where
        F: Fn(&PlanClip) -> Option<&'a RawAnimationClip>,
    {
        if let Some(p) = &mut self.playing {
            let speed = plan.slots.get(p.slot).map_or(1.0, |s| s.speed);
            if p.prev >= p.len {
                self.playing = None;
            } else {
                p.prev = p.time;
                p.time += dt * speed;
            }
        }
        for (i, slot) in plan.slots.iter().enumerate() {
            if params.trigger_set(&slot.trigger) == Some(true) {
                // Arming refused missing clips; a race just leaves the
                // trigger buffered for the next tick.
                let Some(clip) = clip_for(&slot.clip) else { continue };
                params.consume_trigger(&slot.trigger);
                self.playing = Some(SlotPlayback {
                    slot: i,
                    prev: 0.0,
                    time: 0.0,
                    len: clip.duration_seconds,
                });
                break;
            }
        }
    }

    /// Index into the plan's slots while something plays.
    pub fn playing(&self) -> Option<usize> {
        self.playing.as_ref().map(|p| p.slot)
    }

    /// The overlay's blend weight right now: the fade-in/out envelope while
    /// the clip plays, 0.0 once it has finished (the base result shows
    /// through untouched — "returns").
    pub fn weight(&self, plan: &AnimGraphPlan) -> f32 {
        let Some(p) = &self.playing else { return 0.0 };
        plan.slots
            .get(p.slot)
            .map_or(0.0, |slot| envelope(slot, p.time, p.len))
    }

    /// Overlay the slot's clip onto an already-evaluated base `pose` — the
    /// spec's "play-once slot overlay" stage, after [`evaluate_pose`]. One
    /// shot: the sample clamps at the clip end (never wraps), so the last
    /// frame holds through a fade-out.
    pub fn apply<'a, F>(
        &self,
        plan: &AnimGraphPlan,
        clip_for: &F,
        pose: &mut [LocalBoneTransform],
        scratch: &mut PoseScratch,
    ) where
        F: Fn(&PlanClip) -> Option<&'a RawAnimationClip>,
    {
        let w = self.weight(plan);
        if w <= 0.0 {
            return;
        }
        let Some(p) = &self.playing else { return };
        let Some(slot) = plan.slots.get(p.slot) else { return };
        let Some(clip) = clip_for(&slot.clip) else { return };
        // The overlay starts from the same pre-sample transforms as the base
        // — the agreement every blend in this module keeps.
        let mut buf = scratch.take(0);
        buf.clear();
        buf.extend_from_slice(pose);
        sampling::sample_channels(&clip.channels, p.time.min(p.len), &mut buf);
        for (out, over) in pose.iter_mut().zip(buf.iter()) {
            *out = out.blend(over, w);
        }
        scratch.put(0, buf);
    }
}

/// The slot's overlay-weight envelope at clock position `t`: ramp in over
/// `fade_in`, ramp out over the last `fade_out` seconds, 0 from the clip end
/// on (`t == len` is finished — "returns", not a lingering last frame).
fn envelope(slot: &PlanSlot, t: f32, len: f32) -> f32 {
    if t >= len {
        return 0.0;
    }
    let mut w = 1.0f32;
    if slot.fade_in > 0.0 {
        w = w.min(t / slot.fade_in);
    }
    if slot.fade_out > 0.0 {
        w = w.min((len - t) / slot.fade_out);
    }
    w.clamp(0.0, 1.0)
}

// ---------------------------------------------------------------------------
// Anim events
// ---------------------------------------------------------------------------

/// One anim event firing: a clip's marker was crossed by playback this tick,
/// while the clip was audible. The engine-level event the spec promises —
/// collected per entity into `AnimGraphRuntime::events`, where gameplay reads
/// them (ADR 0002 keeps them off the wire: every client derives its own).
#[derive(Debug, Clone, PartialEq)]
pub struct AnimEventFire {
    /// The marker's name — gameplay's key ("footstep", "hit").
    pub name: String,
    /// The clip's effective blend weight when it fired (blend-tree weights ×
    /// crossfade × play-once envelope) — scale footstep volume with it.
    pub weight: f32,
}

/// Fire once for every marker position `at + k·period` (k ≥ 0 an integer)
/// inside the half-open clock span `[t0, t1)` — a looping clip re-fires on
/// each cycle's crossing. Half-open means a marker fires on the tick that
/// reaches it, exactly once, and a zero-length span (held pose) never fires.
fn crossings(at: f32, period: f32, t0: f32, t1: f32, mut fire: impl FnMut()) {
    if t1 <= t0 || period <= 0.0 {
        return;
    }
    let k = ((t0 - at) / period).ceil().max(0.0);
    let mut t = at + k * period;
    // A float guard: ceil can land one period low on exact boundaries.
    while t < t0 {
        t += period;
    }
    while t < t1 {
        fire();
        t += period;
    }
}

/// Collect the markers a tree's clips crossed while the state clock ran
/// `[t0, t1)`, weighted by the branch weights the *current* parameters give.
/// A branch at weight 0 — fully blended out — is skipped whole: its clips
/// stay silent however their clocks move (the ticket's suppression rule).
fn tree_events<'a, F>(
    tree: &PlanTree,
    t0: f32,
    t1: f32,
    weight: f32,
    params: &AnimParams,
    clip_for: &F,
    out: &mut Vec<AnimEventFire>,
) where
    F: Fn(&PlanClip) -> Option<&'a RawAnimationClip>,
{
    let (children, pick) = match tree {
        PlanTree::Clip(c) => {
            // A leaf clip loops on the raw state clock (`wrapped`).
            if let Some(clip) = clip_for(c) {
                for m in &clip.events {
                    crossings(m.time_seconds, clip.duration_seconds, t0, t1, || {
                        out.push(AnimEventFire {
                            name: m.name.clone(),
                            weight,
                        })
                    });
                }
            }
            return;
        }
        PlanTree::Blend1D { param, children } => {
            (children, pick_1d(children, params.get_float(param).unwrap_or(0.0)))
        }
        PlanTree::Blend2D {
            param_x,
            param_y,
            children,
        } => (
            children,
            pick_2d(
                children,
                params.get_float(param_x).unwrap_or(0.0),
                params.get_float(param_y).unwrap_or(0.0),
            ),
        ),
    };

    // Mirror `sample_tree`'s clocks exactly: clip children run at the sync
    // group's shared phase (marker time ÷ own duration, period one phase
    // cycle), nested blends keep the raw clock and sync locally.
    let ref_duration = sync_ref_duration(children, clip_for);
    let (a, b) = pick;
    let pairs = [Some((a, 1.0 - b.map_or(0.0, |(_, w)| w))), b];
    for (child, w) in pairs.into_iter().flatten() {
        let w = w * weight;
        if w <= 0.0 {
            continue;
        }
        match &children[child].1 {
            PlanTree::Clip(c) => {
                let (Some(clip), Some(d_ref)) = (clip_for(c), ref_duration) else {
                    continue;
                };
                if clip.duration_seconds <= 0.0 {
                    continue;
                }
                for m in &clip.events {
                    crossings(
                        m.time_seconds / clip.duration_seconds,
                        1.0,
                        t0 / d_ref,
                        t1 / d_ref,
                        || {
                            out.push(AnimEventFire {
                                name: m.name.clone(),
                                weight: w,
                            })
                        },
                    );
                }
            }
            nested => tree_events(nested, t0, t1, w, params, clip_for, out),
        }
    }
}

/// Collect every anim event this frame's playback crossed — the machine's
/// states (per the clock spans its tick recorded) and the play-once slot —
/// into `out`. Call after both ticks; `out` is caller-owned and cleared here,
/// so steady state allocates nothing once grown.
pub fn collect_anim_events<'a, F>(
    machine: &AnimMachine,
    slot: &PlayOnceSlot,
    plan: &AnimGraphPlan,
    params: &AnimParams,
    clip_for: F,
    out: &mut Vec<AnimEventFire>,
) where
    F: Fn(&PlanClip) -> Option<&'a RawAnimationClip>,
{
    out.clear();

    // A full-weight overlay hides the base entirely (no bone masks in v1):
    // footsteps from an invisible walk must not fire.
    let base_scale = 1.0 - slot.weight(plan);
    if base_scale > 0.0 {
        let target_weight = machine.blend_weight();
        for &(state, t0, t1) in machine.spans() {
            let w = if state == machine.current_state() {
                target_weight
            } else {
                1.0 - target_weight
            } * base_scale;
            if w <= 0.0 {
                continue;
            }
            if let Some(st) = plan.states.get(state) {
                tree_events(&st.tree, t0, t1, w, params, &clip_for, out);
            }
        }
    }

    // The slot's own clip: one shot, no cycles — each marker fires once as
    // the clock reaches it (hit frames on attack overlays). The weight is the
    // envelope at the marker's own time, so a marker on the final frame is
    // not zeroed by the clock having already overshot the clip end.
    if let Some(p) = &slot.playing {
        if let Some(s) = plan.slots.get(p.slot) {
            if let Some(clip) = clip_for(&s.clip) {
                for m in &clip.events {
                    if p.prev <= m.time_seconds && m.time_seconds < p.time {
                        let w = envelope(s, m.time_seconds, p.len);
                        if w > 0.0 {
                            out.push(AnimEventFire {
                                name: m.name.clone(),
                                weight: w,
                            });
                        }
                    }
                }
            }
        }
    }
}
