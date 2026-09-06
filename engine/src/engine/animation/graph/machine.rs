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
    AnimGraphPlan, CmpOp, MathOp, ParamDecl, PlanClip, PlanSlot, PlanSpace, PlanTree,
    PoseSource, RuleExpr, TransitionFrom,
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
/// is ticked with. A nested state (ticket 09) owns a child machine in
/// `subs`, recursively: the sub-machine is that state's Pose source, restarts
/// at its own ENTRY whenever the host (re)enters the state, and keeps
/// running while the state blends out.
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
    /// The last transition that fired — `(plan transition index, seconds
    /// since)`. A read surface for the editor's live preview (ticket 06);
    /// nothing in evaluation consults it.
    fired: Option<(usize, f32)>,
    /// Child machines, index-aligned with the plan's states — `Some` exactly
    /// where the plan's source is [`PoseSource::Machine`]. The recursion
    /// mirrors the plan's, which the compiler guaranteed finite (cycle
    /// refusal), and reuses its allocations across restarts ([`Self::reset`]).
    subs: Vec<Option<Box<AnimMachine>>>,
    /// Per-state smoothed blend-space input, index-aligned with the plan's
    /// states (only meaningful where the source is a [`PlanTree::Space`]).
    /// `None` = no memory yet: the next advance snaps to the raw target,
    /// which is what (re)entering a state does so it never blends from stale
    /// input.
    inputs: Vec<Option<[f32; 2]>>,
}

impl AnimMachine {
    /// A fresh machine sits in the plan's entry state — active from the very
    /// first tick, no transition needed to get there.
    pub fn new(plan: &AnimGraphPlan) -> Self {
        let mut m = Self {
            current: plan.entry,
            time: 0.0,
            fade: None,
            spans: Vec::new(),
            fired: None,
            subs: Vec::new(),
            inputs: Vec::new(),
        };
        m.reset(plan);
        m
    }

    /// Back to the plan's entry state, wholesale — what (re)entering a nested
    /// state does to its sub-machine. Reuses every buffer already grown, so a
    /// transition into a nested state allocates nothing once warm.
    fn reset(&mut self, plan: &AnimGraphPlan) {
        self.current = plan.entry;
        self.time = 0.0;
        self.fade = None;
        self.spans.clear();
        self.fired = None;
        self.inputs.clear();
        self.inputs.resize(plan.states.len(), None);
        if self.subs.len() != plan.states.len() {
            self.subs = plan.states.iter().map(|_| None).collect();
        }
        for (i, s) in plan.states.iter().enumerate() {
            if let PoseSource::Machine { plan: child, .. } = &s.source {
                match &mut self.subs[i] {
                    Some(sub) => sub.reset(child),
                    slot => *slot = Some(Box::new(AnimMachine::new(child))),
                }
            }
        }
    }

    /// The child machine of a nested state, by plan state index. The read
    /// surface pose evaluation, event collection and the editor's preview
    /// use; `None` for tree states.
    pub fn sub(&self, state: usize) -> Option<&AnimMachine> {
        self.subs.get(state).and_then(|s| s.as_deref())
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

    /// True while this machine — or the sampled chain of nested machines —
    /// is mid-crossfade or fired a transition this very tick (instant
    /// switches included). The S-D4 forced-evaluation condition: an
    /// update-rate-throttled entity must not hold a stale pose through a
    /// transition. Only the active state's sub is consulted; a frozen sub
    /// (instant-switched away) keeps its fade but is not sampled, so it must
    /// not pin the entity at full rate.
    pub fn transition_activity(&self) -> bool {
        self.fade.is_some()
            || self.fired.is_some_and(|(_, age)| age == 0.0)
            || self
                .sub(self.current)
                .is_some_and(AnimMachine::transition_activity)
    }

    /// The last transition that fired: `(plan transition index, seconds
    /// since)`. `None` until the first fire. The editor's live highlight
    /// reads this to light the firing transition; age lets an instant
    /// (zero-duration) fire still flash for a moment.
    pub fn last_fired(&self) -> Option<(usize, f32)> {
        self.fired
    }

    /// The active state's blend weight: 1.0 at rest, the fade's target
    /// weight while crossfading.
    pub fn blend_weight(&self) -> f32 {
        self.fade.as_ref().map(Crossfade::weight).unwrap_or(1.0)
    }

    /// The (smoothed) blend-space input a state samples at, as of the last
    /// tick; the raw parameter target before the state's first advance. The
    /// editor's preview reads this to place the live point.
    pub fn space_input(&self, state: usize, plan: &AnimGraphPlan, params: &AnimParams) -> [f32; 2] {
        match plan.states.get(state).map(|s| &s.source) {
            Some(PoseSource::Tree(PlanTree::Space(sp))) => self
                .inputs
                .get(state)
                .copied()
                .flatten()
                .unwrap_or_else(|| sp.target(params)),
            _ => [0.0; 2],
        }
    }

    /// Advance a blend-space state's input memory toward the raw target:
    /// exponential approach over the space's smoothing time, a snap when
    /// smoothing is off or the state has no memory yet (just entered).
    fn advance_input(&mut self, state: usize, plan: &AnimGraphPlan, params: &AnimParams, dt: f32) {
        let Some(PoseSource::Tree(PlanTree::Space(sp))) =
            plan.states.get(state).map(|s| &s.source)
        else {
            return;
        };
        let Some(slot) = self.inputs.get_mut(state) else { return };
        let target = sp.target(params);
        *slot = Some(match *slot {
            Some(mut x) if sp.smoothing > 0.0 => {
                let k = 1.0 - (-dt / sp.smoothing).exp();
                for (v, t) in x.iter_mut().zip(target) {
                    *v += (t - *v) * k;
                }
                x
            }
            _ => target,
        });
    }

    /// Advance one frame: clip clocks first, then transition rules.
    ///
    /// While a crossfade runs no transition may start — every transition
    /// waits for the fade to finish (state aliases compile to ordinary
    /// transitions, so nothing has an interrupt right). `params` is mutable
    /// because firing consumes the triggers the firing rule reads
    /// (consume-on-transition); rules themselves stay pure.
    pub fn tick(&mut self, plan: &AnimGraphPlan, params: &mut AnimParams, dt: f32) {
        self.spans.clear();
        if let Some((_, age)) = &mut self.fired {
            *age += dt;
        }
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
        // candidate whose rule passes wins, deterministically. Alias-expanded
        // transitions sit in the same list on the same priority scale.
        let fading = self.fade.is_some();
        let current = self.current;
        let fired = plan
            .transitions
            .iter()
            .enumerate()
            .filter(|(_, t)| match t.from {
                TransitionFrom::State(s) => !fading && s == current,
            })
            .find(|(_, t)| t.rule.as_ref().is_none_or(|r| r.expr.eval_bool(params)));
        let entered = fired.is_some();
        if let Some((ti, t)) = fired {
            self.fired = Some((ti, 0.0));
            // Consume-on-transition: the fire spends every trigger the rule
            // reads, exactly once — they were consulted, they are consumed.
            if let Some(rule) = &t.rule {
                for slug in &rule.triggers {
                    params.consume_trigger(slug);
                }
            }
            // Nothing fires mid-fade, so this never replaces a running fade.
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
            // Entering a blend-space state forgets its smoothed input: the
            // first advance below snaps to the raw target.
            if let Some(slot) = self.inputs.get_mut(t.to) {
                *slot = None;
            }
            // Entering a nested state restarts its sub-machine at the child's
            // ENTRY — re-entry never resumes mid-flight, the same rule as the
            // state clock resetting to 0.
            if let Some(PoseSource::Machine { plan: child, .. }) =
                plan.states.get(t.to).map(|s| &s.source)
            {
                if let Some(Some(sub)) = self.subs.get_mut(t.to) {
                    sub.reset(child);
                }
            }
        } else {
            self.spans.push((self.current, time_before, self.time));
            if let Some(span) = from_span {
                self.spans.push(span);
            }
        }

        // Nested states tick their sub-machines — only the ones this frame
        // samples, so a sub frozen by an instant switch stays exactly where
        // it stopped (and restarts at ENTRY on re-entry). The active state's
        // sub skips the frame the host entered it (it was just reset: its
        // entry pose this frame, exactly as a clip state samples t = 0). The
        // outgoing state's sub keeps running while the fade keeps it visible.
        // Order is the determinism statement for shared-blackboard triggers:
        // host rules consumed theirs first, then the active sub, then the
        // dying one. A self-transition's two sides share one sub instance —
        // the fade blends the restarted machine with itself, the nested
        // reading of the accepted restart semantics.
        if !entered {
            self.tick_sub(self.current, plan, params, dt);
        }
        if let Some(from) = self.fade.as_ref().map(|f| f.from) {
            if from != self.current {
                self.tick_sub(from, plan, params, dt);
            }
        }

        // Blend-space inputs follow their parameters for every state sampled
        // this frame — the active one and, while a fade keeps it visible, the
        // outgoing one (which keeps smoothing on its own memory).
        self.advance_input(self.current, plan, params, dt);
        if let Some(from) = self.fade.as_ref().map(|f| f.from) {
            if from != self.current {
                self.advance_input(from, plan, params, dt);
            }
        }
    }

    /// Tick the child machine of `state`, if it has one, on the state's
    /// speed-scaled clock.
    fn tick_sub(&mut self, state: usize, plan: &AnimGraphPlan, params: &mut AnimParams, dt: f32) {
        let Some(st) = plan.states.get(state) else { return };
        let PoseSource::Machine { plan: child, .. } = &st.source else { return };
        if let Some(Some(sub)) = self.subs.get_mut(state) {
            sub.tick(child, params, dt * st.speed);
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
        // Reached only for a hand-built plan: a state's space goes through
        // `sample_state`, which supplies the machine's smoothed input.
        PlanTree::Space(sp) => {
            return sample_space(sp, sp.target(params), time, clip_for, pose, scratch, level);
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

/// A blend space's sync group reference: the duration of the first sample in
/// the space's order that resolves to a cyclic clip (the blend node's rule).
/// Every cyclic sample then runs on the reference's cycle — sample `i` at
/// clock `time · rate_i · d_i / d_ref` — so equal rates are phase-matched
/// (walk/run feet agree) and a sample's `rate_scale` is its speed relative
/// to a rate-1 sample. `None` = nothing cyclic: each sample runs its own
/// rate on the raw clock.
fn space_sync<'a, F>(sp: &PlanSpace, clip_for: &F) -> Option<f32>
where
    F: Fn(&PlanClip) -> Option<&'a RawAnimationClip>,
{
    sp.samples
        .iter()
        .find_map(|(c, _)| Some(clip_for(c)?.duration_seconds).filter(|d| *d > 0.0))
}

/// The clock sample `i` reads at state clock `time` (see [`space_sync`]).
/// Unwrapped on purpose: the caller wraps by the clip, which keeps a
/// non-integer rate ratio continuous across the reference's cycle boundary.
fn space_sample_time(sp: &PlanSpace, i: usize, time: f32, d_ref: Option<f32>, d_i: f32) -> f32 {
    let rate_i = sp.samples[i].1;
    match d_ref {
        Some(d_ref) if d_i > 0.0 => time * rate_i * d_i / d_ref,
        _ => time * rate_i,
    }
}

/// Evaluate a blend space at `input` into `pose`: the (at most three)
/// contributing samples, mixed bone-wise in one accumulating pass — the same
/// pre-sample agreement and scratch discipline `sample_tree` keeps.
fn sample_space<'a, F>(
    sp: &PlanSpace,
    input: [f32; 2],
    time: f32,
    clip_for: &F,
    pose: &mut [LocalBoneTransform],
    scratch: &mut PoseScratch,
    level: usize,
) where
    F: Fn(&PlanClip) -> Option<&'a RawAnimationClip>,
{
    let weights = sp.space.weights(input);
    let sync = space_sync(sp, clip_for);
    let mut buf = scratch.take(level);
    // Accumulated weight so far: mixing sample k in at `w_k / (acc + w_k)`
    // yields the exact normalized blend after the last one.
    let mut acc = 0.0f32;
    for &(i, w) in weights.as_slice() {
        let Some(clip) = sp.samples.get(i).and_then(|(c, _)| clip_for(c)) else {
            continue;
        };
        let t = space_sample_time(sp, i, time, sync, clip.duration_seconds);
        if acc <= 0.0 {
            sampling::sample_channels(&clip.channels, wrapped(t, clip), pose);
        } else {
            buf.clear();
            buf.extend_from_slice(pose);
            sampling::sample_channels(&clip.channels, wrapped(t, clip), &mut buf);
            let k = w / (acc + w);
            for (out, other) in pose.iter_mut().zip(buf.iter()) {
                *out = out.blend(other, k);
            }
        }
        acc += w;
    }
    scratch.put(level, buf);
}

/// One state's Pose at `time`: its blend tree, or — nested — its whole
/// sub-machine, evaluated recursively (the sub owns its clocks, so `time` is
/// only for trees).
#[allow(clippy::too_many_arguments)]
fn sample_state<'a, F>(
    machine: &AnimMachine,
    plan: &AnimGraphPlan,
    state: usize,
    time: f32,
    params: &AnimParams,
    clip_for: &F,
    pose: &mut [LocalBoneTransform],
    scratch: &mut PoseScratch,
    level: usize,
) where
    F: Fn(&PlanClip) -> Option<&'a RawAnimationClip>,
{
    let Some(st) = plan.states.get(state) else { return };
    match &st.source {
        PoseSource::Tree(PlanTree::Space(sp)) => {
            let input = machine.space_input(state, plan, params);
            sample_space(sp, input, time, clip_for, pose, scratch, level)
        }
        PoseSource::Tree(tree) => {
            sample_tree(tree, time, params, clip_for, pose, scratch, level)
        }
        PoseSource::Machine { plan: child, .. } => {
            if let Some(sub) = machine.sub(state) {
                eval_machine(sub, child, params, clip_for, pose, scratch, level);
            }
        }
    }
}

/// One machine level: the active state's Pose, mixed with the outgoing
/// state's while a crossfade runs. `level` offsets the scratch pool so
/// nested machines never alias a parent's buffers.
fn eval_machine<'a, F>(
    machine: &AnimMachine,
    plan: &AnimGraphPlan,
    params: &AnimParams,
    clip_for: &F,
    pose: &mut [LocalBoneTransform],
    scratch: &mut PoseScratch,
    level: usize,
) where
    F: Fn(&PlanClip) -> Option<&'a RawAnimationClip>,
{
    let fading = machine
        .crossfade()
        .filter(|f| plan.states.get(f.from).is_some());

    // The outgoing pose starts from the same pre-sample transforms as the
    // target's — the same agreement `sample_tree` keeps inside a blend.
    if let Some(fade) = fading {
        let mut buf = scratch.take(level);
        buf.clear();
        buf.extend_from_slice(pose);
        sample_state(
            machine,
            plan,
            fade.from,
            fade.from_time,
            params,
            clip_for,
            &mut buf,
            scratch,
            level + 1,
        );
        sample_state(
            machine,
            plan,
            machine.current_state(),
            machine.time(),
            params,
            clip_for,
            pose,
            scratch,
            level + 1,
        );
        let w = fade.weight();
        for (out, from) in pose.iter_mut().zip(buf.iter()) {
            *out = from.blend(out, w);
        }
        scratch.put(level, buf);
    } else {
        sample_state(
            machine,
            plan,
            machine.current_state(),
            machine.time(),
            params,
            clip_for,
            pose,
            scratch,
            level,
        );
    }
}

/// Evaluate the machine's Pose into `pose` (the skeleton's local bone
/// transforms): the active state's blend tree, mixed with the outgoing
/// state's while a crossfade runs; a nested state contributes its whole
/// sub-machine's result the same way.
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
    eval_machine(machine, plan, params, &clip_for, pose, scratch, 0);
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
        PlanTree::Space(sp) => {
            return space_events(sp, sp.target(params), t0, t1, weight, clip_for, out);
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

/// The markers a blend space's contributing samples crossed while the state
/// clock ran `[t0, t1)` — the mirror of [`space_sample_time`]'s clocks, in
/// each sample's own cycle units.
fn space_events<'a, F>(
    sp: &PlanSpace,
    input: [f32; 2],
    t0: f32,
    t1: f32,
    weight: f32,
    clip_for: &F,
    out: &mut Vec<AnimEventFire>,
) where
    F: Fn(&PlanClip) -> Option<&'a RawAnimationClip>,
{
    let sync = space_sync(sp, clip_for);
    for &(i, w) in sp.space.weights(input).as_slice() {
        let w = w * weight;
        let Some((clip, rate_i)) = sp
            .samples
            .get(i)
            .and_then(|(c, rate)| clip_for(c).map(|cl| (cl, *rate)))
        else {
            continue;
        };
        if w <= 0.0 || clip.duration_seconds <= 0.0 {
            continue;
        }
        let d = clip.duration_seconds;
        // Sample-i clock as a function of the state clock, unwrapped: either
        // the synced cycle (`T · rate_i / d_ref` cycles) or the raw rate.
        let (period, s0, s1) = match sync {
            Some(d_ref) => (1.0, t0 * rate_i / d_ref, t1 * rate_i / d_ref),
            None => (d, t0 * rate_i, t1 * rate_i),
        };
        for m in &clip.events {
            let at = if sync.is_some() { m.time_seconds / d } else { m.time_seconds };
            crossings(at, period, s0, s1, || {
                out.push(AnimEventFire {
                    name: m.name.clone(),
                    weight: w,
                })
            });
        }
    }
}

/// One machine level's event crossings, scaled by `scale` (the weight the
/// level itself is heard at). Tree states replay their recorded clock spans;
/// nested states recurse into their sub-machine — whose spans are fresh
/// exactly when the host sampled it this frame (the sub only ticks then),
/// so a frozen sub can never re-fire stale crossings.
fn machine_events<'a, F>(
    machine: &AnimMachine,
    plan: &AnimGraphPlan,
    params: &AnimParams,
    scale: f32,
    clip_for: &F,
    out: &mut Vec<AnimEventFire>,
) where
    F: Fn(&PlanClip) -> Option<&'a RawAnimationClip>,
{
    let target_weight = machine.blend_weight();
    for &(state, t0, t1) in machine.spans() {
        let w = if state == machine.current_state() {
            target_weight
        } else {
            1.0 - target_weight
        } * scale;
        if w <= 0.0 {
            continue;
        }
        match plan.states.get(state).map(|s| &s.source) {
            Some(PoseSource::Tree(PlanTree::Space(sp))) => {
                let input = machine.space_input(state, plan, params);
                space_events(sp, input, t0, t1, w, clip_for, out)
            }
            Some(PoseSource::Tree(tree)) => {
                tree_events(tree, t0, t1, w, params, clip_for, out)
            }
            Some(PoseSource::Machine { plan: child, .. }) => {
                if let Some(sub) = machine.sub(state) {
                    machine_events(sub, child, params, w, clip_for, out);
                }
            }
            None => {}
        }
    }
}

/// Collect every anim event this frame's playback crossed — the machine's
/// states (per the clock spans its tick recorded, nested sub-machines
/// included) and the play-once slot — into `out`. Call after both ticks;
/// `out` is caller-owned and cleared here, so steady state allocates nothing
/// once grown.
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
        machine_events(machine, plan, params, base_scale, &clip_for, out);
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
