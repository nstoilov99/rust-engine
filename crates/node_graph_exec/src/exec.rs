//! The interpreter (D1): drain the event queue in the documented order,
//! advance every running activation, and stop hard when the budget runs out.
//!
//! **Pure evaluation is statement-scoped.** During one impure firing each
//! pure node evaluates at most once (memo keyed by plan index, cleared at the
//! next firing). There is no cross-statement caching in v1 — correctness
//! first — and volatile nodes (`deterministic: false`, i.e. anything reading
//! instance RNG) are exempt from the memo entirely. Pure evaluations count
//! against the tick budget, so a pathological pure chain cannot hide from it.

use std::collections::BTreeMap;

use node_graph_types::{EventPhase, EVENT_DRAIN_ORDER};

use crate::effect::{EffectSink, WorldRead};
use crate::instance::{Activation, Frame, GraphInstance, QueuedEvent, ThreadState};
use crate::node::{
    EvalCtx, ExecError, FireCtx, FireResult, LoopFrameView, NodeImpl, NodeImpls, TickInput,
};
use crate::plan::{InputSource, Plan};
use crate::value::Value;

/// The default per-tick firing budget (D1). Generous enough that no honest
/// graph reaches it, small enough that hitting it costs a frame, not a hang.
pub const DEFAULT_BUDGET: u32 = 100_000;

/// What one tick did. The counts are what the editor's execution view shows
/// (P7) and what a budget test asserts on.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TickReport {
    /// Node firings plus pure evaluations — everything the budget counts.
    pub steps: u32,
    /// Activations started this tick.
    pub activations: u32,
    /// The error that halted the instance, if one did.
    pub halted: Option<ExecError>,
}

/// Run one tick of `instance` against `plan`.
///
/// The drain order is [`EVENT_DRAIN_ORDER`] — BeginPlay (once per instance
/// lifetime) → due latents → input actions → custom events → Tick — and is
/// taken from the P1 constants rather than restated here, so the interpreter
/// and the spec cannot drift.
pub fn tick(
    plan: &Plan,
    instance: &mut GraphInstance,
    impls: &NodeImpls,
    tick: TickInput,
    world: &dyn WorldRead,
    effects: &mut dyn EffectSink,
) -> TickReport {
    tick_with_budget(plan, instance, impls, tick, world, effects, DEFAULT_BUDGET)
}

pub fn tick_with_budget(
    plan: &Plan,
    instance: &mut GraphInstance,
    impls: &NodeImpls,
    tick_in: TickInput,
    world: &dyn WorldRead,
    effects: &mut dyn EffectSink,
    budget: u32,
) -> TickReport {
    let mut report = TickReport::default();
    if instance.halted.is_some() {
        report.halted = instance.halted.clone();
        return report;
    }
    instance.time += tick_in.dt as f64;

    // --- start activations, in drain order --------------------------------
    // The queue is drained into a per-phase working set first: an event
    // emitted *during* this tick lands in `instance.queue` for the next one,
    // which is what "no reentrancy" means.
    let pending: Vec<QueuedEvent> = instance.queue.drain(..).collect();
    for phase in EVENT_DRAIN_ORDER {
        match phase {
            EventPhase::BeginPlay => {
                if std::mem::take(&mut instance.begin_play_pending) {
                    report.activations +=
                        start_phase(plan, instance, phase, None, &BTreeMap::new());
                }
            }
            EventPhase::Latent => {
                // Resume suspended activations whose time has come, ordered
                // by due time then activation id (D3). P4 puts real latent
                // nodes behind this; the ordering is fixed here so it does
                // not have to change when they land.
                let mut due: Vec<(u64, u64, usize)> = Vec::new();
                for (i, t) in instance.threads.iter().enumerate() {
                    if let ThreadState::Suspended(crate::node::Suspension::Until { time }) = t.state
                    {
                        if instance.time >= time {
                            due.push((time.to_bits(), t.id.0, i));
                        }
                    }
                }
                due.sort();
                for (_, _, i) in due {
                    instance.threads[i].state = ThreadState::Running;
                }
            }
            _ => {
                for ev in pending.iter().filter(|e| e.phase == phase) {
                    report.activations +=
                        start_phase(plan, instance, phase, ev.name.as_deref(), &ev.payload);
                }
                if phase == EventPhase::Tick {
                    report.activations += start_phase(plan, instance, phase, None, &BTreeMap::new());
                }
            }
        }
    }

    // --- advance every running activation ---------------------------------
    // Index order, which is start order: a stable iteration order over
    // activations is part of the determinism contract (D8).
    let mut i = 0;
    while i < instance.threads.len() {
        if !matches!(instance.threads[i].state, ThreadState::Running) {
            i += 1;
            continue;
        }
        match advance(plan, instance, i, impls, tick_in, world, effects, budget, &mut report) {
            Ok(()) => {}
            Err(e) => {
                // A budget kill stops the whole instance, not just the
                // activation: the graph is not in a state anyone should trust.
                instance.threads[i].state = ThreadState::Failed(e.clone());
                if matches!(e, ExecError::BudgetExceeded { .. }) {
                    instance.halted = Some(e.clone());
                    report.halted = Some(e);
                    return report;
                }
            }
        }
        i += 1;
    }
    report
}

/// Start one activation per entry node matching `phase` (and `name`, when the
/// phase carries one). Returns how many started.
fn start_phase(
    plan: &Plan,
    instance: &mut GraphInstance,
    phase: EventPhase,
    name: Option<&str>,
    payload: &BTreeMap<String, Value>,
) -> u32 {
    let matching: Vec<usize> = plan
        .entries
        .iter()
        .filter(|e| e.phase == phase)
        .filter(|e| match (name, e.name.as_deref()) {
            (Some(want), Some(have)) => want == have,
            (Some(_), None) => false,
            (None, _) => true,
        })
        .map(|e| e.node)
        .collect();
    let mut started = 0;
    for node in matching {
        let id = instance.next_activation_id();
        instance.threads.push(Activation {
            id,
            entry: node,
            cursor: Some(node),
            entered: None,
            frames: Vec::new(),
            locals: BTreeMap::new(),
            payload: payload.clone(),
            state: ThreadState::Running,
        });
        started += 1;
    }
    started
}

/// Run one activation until it finishes, suspends, or the budget dies.
#[allow(clippy::too_many_arguments)]
fn advance(
    plan: &Plan,
    instance: &mut GraphInstance,
    thread: usize,
    impls: &NodeImpls,
    tick_in: TickInput,
    world: &dyn WorldRead,
    effects: &mut dyn EffectSink,
    budget: u32,
    report: &mut TickReport,
) -> Result<(), ExecError> {
    loop {
        let Some(node_ix) = instance.threads[thread].cursor else {
            instance.threads[thread].state = ThreadState::Finished;
            return Ok(());
        };
        let node = &plan.nodes[node_ix];

        // The budget is charged per firing *and* per pure evaluation, so an
        // exec cycle and a pathological pure chain both hit the same wall.
        report.steps += 1;
        if report.steps > budget {
            return Err(ExecError::BudgetExceeded { node: node.name.clone(), budget });
        }

        // Pull the data inputs backward through the pure chains. The memo is
        // created here and dropped at the end of this firing — that *is*
        // "statement-scoped".
        //
        // The RNG travels as a local across the pull so that a volatile pure
        // node (RandomFloat) genuinely *advances* it: handing the pull phase a
        // throwaway copy would make every draw return the same number, which
        // is deterministic in the least useful sense.
        let mut memo: BTreeMap<usize, BTreeMap<String, Value>> = BTreeMap::new();
        let mut rng = instance.rng;
        let pulled = pull_inputs(
            plan, instance, thread, node_ix, impls, tick_in, world, budget, report, &mut memo,
            &mut rng,
        );
        instance.rng = rng;
        let inputs = pulled?;

        let Some(NodeImpl::Impure(imp)) = impls.get(&node.type_id) else {
            return Err(ExecError::MissingImpl {
                node: node.name.clone(),
                type_id: node.type_id.clone(),
            });
        };

        // The frame view: `Some` only when this firing is the interpreter
        // re-entering the node after its own loop body finished.
        let frame = instance.threads[thread]
            .frames
            .last()
            .filter(|f| f.node == node_ix)
            .map(|f| LoopFrameView { iteration: f.iteration });

        let entered = instance.threads[thread].entered.clone();
        let node_state = instance.node_state.get(&node_ix).cloned();
        let mut outputs: BTreeMap<String, Value> = instance.threads[thread]
            .locals
            .get(&node_ix)
            .cloned()
            .unwrap_or_default();
        // The entry node's payload pins are its outputs.
        if node_ix == instance.threads[thread].entry {
            for (k, v) in instance.threads[thread].payload.clone() {
                outputs.entry(k).or_insert(v);
            }
        }

        let (result, new_state) = {
            let mut ctx = FireCtx {
                node: &node.name,
                inputs: &inputs,
                outputs: &mut outputs,
                vars: &mut instance.variables,
                variable: node.variable.as_deref(),
                effects,
                world,
                tick: tick_in,
                rng: &mut instance.rng,
                self_entity: instance.self_entity,
                next_alias: &mut instance.next_alias,
                pending_aliases: &mut instance.pending_aliases,
                queue: &mut instance.queue,
                state: node_state,
                entered: entered.as_deref(),
                frame,
            };
            let r = imp.fire(&mut ctx);
            (r, ctx.state)
        };
        instance.threads[thread].locals.insert(node_ix, outputs);
        if let Some(v) = new_state {
            instance.node_state.insert(node_ix, v);
        }

        match result {
            FireResult::Continue(pin) => {
                // Leaving a loop is how a loop ends: drop the frame this node
                // owns, if it owns one.
                pop_own_frame(&mut instance.threads[thread], node_ix);
                let next = plan.nodes[node_ix].exec.get(pin);
                instance.threads[thread].cursor = next.map(|t| t.node);
                instance.threads[thread].entered = next.map(|t| t.pin.clone());
                if next.is_none() {
                    unwind(plan, instance, thread);
                }
            }
            FireResult::Loop(body) => {
                let frames = &mut instance.threads[thread].frames;
                if frames.last().map(|f| f.node) != Some(node_ix) {
                    frames.push(Frame { node: node_ix, iteration: 0 });
                }
                match plan.nodes[node_ix].exec.get(body) {
                    // An empty body is legal — the loop still counts down,
                    // which is what makes an unwired ForLoop (or a Sequence
                    // with gaps) terminate rather than spin.
                    None => {
                        bump_frame(&mut instance.threads[thread], node_ix);
                        instance.threads[thread].cursor = Some(node_ix);
                        instance.threads[thread].entered = None;
                    }
                    Some(t) => {
                        instance.threads[thread].cursor = Some(t.node);
                        instance.threads[thread].entered = Some(t.pin.clone());
                    }
                }
            }
            FireResult::Done => {
                pop_own_frame(&mut instance.threads[thread], node_ix);
                instance.threads[thread].cursor = None;
                instance.threads[thread].entered = None;
                unwind(plan, instance, thread);
            }
            FireResult::Suspend(s) => {
                // The cursor stays where it is: resuming re-fires this node,
                // which re-derives its state from the frame stack. Latent
                // state therefore needs no storage of its own (P4).
                instance.threads[thread].state = ThreadState::Suspended(s);
                return Ok(());
            }
            FireResult::Stop(e) => return Err(e),
        }
    }
}

/// A statement ended. Hand control back to the innermost loop frame — that is
/// the point of the frame: the interpreter, not the node, remembers where a
/// body returns to.
fn unwind(plan: &Plan, instance: &mut GraphInstance, thread: usize) {
    let _ = plan;
    let t = &mut instance.threads[thread];
    if t.cursor.is_some() {
        return;
    }
    if let Some(f) = t.frames.last_mut() {
        f.iteration += 1;
        t.cursor = Some(f.node);
        // Re-entering a loop node from its own body is not an entrance
        // through one of its exec inputs.
        t.entered = None;
    } else {
        t.state = ThreadState::Finished;
    }
}

fn pop_own_frame(t: &mut Activation, node_ix: usize) {
    if t.frames.last().map(|f| f.node) == Some(node_ix) {
        t.frames.pop();
    }
}

fn bump_frame(t: &mut Activation, node_ix: usize) {
    if let Some(f) = t.frames.last_mut() {
        if f.node == node_ix {
            f.iteration += 1;
        }
    }
}

/// Resolve one node's data inputs, evaluating the pure chain behind them.
#[allow(clippy::too_many_arguments)]
fn pull_inputs(
    plan: &Plan,
    instance: &GraphInstance,
    thread: usize,
    node_ix: usize,
    impls: &NodeImpls,
    tick_in: TickInput,
    world: &dyn WorldRead,
    budget: u32,
    report: &mut TickReport,
    memo: &mut BTreeMap<usize, BTreeMap<String, Value>>,
    rng: &mut crate::instance::Rng,
) -> Result<BTreeMap<String, Value>, ExecError> {
    let mut out = BTreeMap::new();
    let sources: Vec<(String, InputSource)> = plan.nodes[node_ix]
        .inputs
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    for (slug, src) in sources {
        let v = match src {
            InputSource::Constant(v) => v,
            InputSource::Pin { node, pin } => pull_output(
                plan, instance, thread, node, &pin, impls, tick_in, world, budget, report, memo,
                rng,
            )?,
        };
        out.insert(slug, v);
    }
    Ok(out)
}

/// One output pin's value: an impure node's stored local, or a pure node's
/// evaluation (memoized for this statement unless the node is volatile).
#[allow(clippy::too_many_arguments)]
fn pull_output(
    plan: &Plan,
    instance: &GraphInstance,
    thread: usize,
    node_ix: usize,
    pin: &str,
    impls: &NodeImpls,
    tick_in: TickInput,
    world: &dyn WorldRead,
    budget: u32,
    report: &mut TickReport,
    memo: &mut BTreeMap<usize, BTreeMap<String, Value>>,
    rng: &mut crate::instance::Rng,
) -> Result<Value, ExecError> {
    let node = &plan.nodes[node_ix];
    if !node.pure {
        // An impure node's outputs are values it *stored* when it fired.
        // Absent means it has not fired yet in this activation, which is a
        // zero rather than an error — the same answer Blueprint gives.
        return Ok(instance.threads[thread]
            .locals
            .get(&node_ix)
            .and_then(|m| m.get(pin))
            .cloned()
            .unwrap_or(Value::Unit));
    }
    if !node.volatile {
        if let Some(v) = memo.get(&node_ix).and_then(|m| m.get(pin)) {
            return Ok(v.clone());
        }
    }

    report.steps += 1;
    if report.steps > budget {
        return Err(ExecError::BudgetExceeded { node: node.name.clone(), budget });
    }

    // Data-edge cycles are rejected at validation (P1's `DataCycle` rule), so
    // this recursion is over a DAG by construction.
    let inputs = pull_inputs(
        plan, instance, thread, node_ix, impls, tick_in, world, budget, report, memo, rng,
    )?;

    let Some(NodeImpl::Pure(imp)) = impls.get(&node.type_id) else {
        return Err(ExecError::MissingImpl {
            node: node.name.clone(),
            type_id: node.type_id.clone(),
        });
    };

    let mut outputs = BTreeMap::new();
    {
        let mut ctx = EvalCtx {
            node: &node.name,
            inputs: &inputs,
            outputs: &mut outputs,
            vars: &instance.variables,
            variable: node.variable.as_deref(),
            tick: tick_in,
            rng,
            world,
            self_entity: instance.self_entity,
        };
        imp.eval(&mut ctx)?;
    }
    let v = outputs.get(pin).cloned().unwrap_or(Value::Unit);
    if !node.volatile {
        memo.insert(node_ix, outputs);
    }
    Ok(v)
}
