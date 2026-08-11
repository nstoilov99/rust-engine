//! The `NodeImpl` contract (D1).
//!
//! The pure/impure split is **structural**, not a flag on one trait: a pure
//! node computes outputs from inputs and cannot touch the world, an impure
//! node performs an effect and names its continuation. Registering the wrong
//! one against a descriptor is a compile error at the call site rather than a
//! runtime surprise, and the compiler cross-checks the two anyway.
//!
//! Both context types receive their inputs **pre-pulled**. That is not an
//! optimization — it is the reference semantics from the plan: an impure node
//! "fires, pulls its data inputs backward through pure chains, performs its
//! effect, then names the exec output(s) to continue on". Pulling happens
//! before `fire` is entered, so an implementation cannot re-enter the
//! interpreter and cannot observe a half-evaluated statement.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::effect::{Effect, EffectSink, WorldRead};
use crate::instance::Rng;
use crate::value::{EntityRef, Value};

/// Time, injected. Nothing in this crate reads a clock (D8).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct TickInput {
    /// Seconds since the previous tick.
    pub dt: f32,
    /// Seconds since the instance started. Monotonic, supplied by the runner.
    pub time: f64,
}

/// Why an activation stopped early.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecError {
    /// The per-tick firing budget ran out — a runaway loop or an exec cycle.
    /// Names the node it was executing so the report can point at it.
    BudgetExceeded { node: String, budget: u32 },
    /// A node's input was not the type its implementation expected. Only
    /// reachable through a document that skipped validation, or a node
    /// implementation disagreeing with its own descriptor.
    TypeMismatch { node: String, pin: String, expected: String },
    /// No implementation registered for a node type the plan contains.
    MissingImpl { node: String, type_id: String },
    /// A variable the plan does not declare. Validation catches this at edit
    /// time; the runtime still refuses rather than inventing a slot.
    UnknownVariable { node: String, slug: String },
    /// A node stopped its own activation deliberately.
    Stopped { node: String, reason: String },
}

impl std::fmt::Display for ExecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecError::BudgetExceeded { node, budget } => write!(
                f,
                "node '{node}': iteration budget of {budget} firings exhausted \
                 (runaway loop or exec cycle)"
            ),
            ExecError::TypeMismatch { node, pin, expected } => {
                write!(f, "node '{node}': input '{pin}' is not a {expected}")
            }
            ExecError::MissingImpl { node, type_id } => {
                write!(f, "node '{node}': no implementation for type '{type_id}'")
            }
            ExecError::UnknownVariable { node, slug } => {
                write!(f, "node '{node}': no variable '{slug}'")
            }
            ExecError::Stopped { node, reason } => write!(f, "node '{node}': {reason}"),
        }
    }
}

/// A suspended activation's resume condition **and its continuation**.
///
/// Carrying the continuation is what makes resuming cheap and honest: at the
/// moment a node suspends, the interpreter resolves `resume` through the plan
/// exactly as it would resolve a [`FireResult::Continue`], parks the resulting
/// cursor on the activation, and marks it suspended. Waking up is then a
/// state flip — no plan lookup, no re-firing of the latent node, and no way
/// for a Delay to suspend itself twice.
///
/// It is plain data, because the whole point is that an instance can be
/// written to disk mid-wait and resumed (D1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Suspension {
    /// Instance time at which the activation becomes due. Instance time is
    /// the sum of the `dt`s the runner has supplied — nothing in this crate
    /// reads a clock (D8).
    pub until: f64,
    /// The suspending node's exec output to continue on when due. `None`
    /// means the activation unwinds on resume: back to its enclosing loop
    /// frame, or finished.
    pub resume: Option<String>,
}

impl Suspension {
    /// Suspend until `until`, continuing on `pin`.
    pub fn until(until: f64, pin: &str) -> Self {
        Self { until, resume: Some(pin.to_string()) }
    }
}

/// What an impure node's firing decided.
///
/// The loop protocol is the interesting half: **frames belong to the
/// interpreter, implementations stay stateless.** A loop node returns
/// [`FireResult::Loop`], which tells the interpreter to push (or keep) a
/// frame for that node and continue into the body. When the body's exec chain
/// runs out, the interpreter bumps the frame's `iteration` and **re-fires the
/// same node**, which re-derives its state from `iteration` plus its inputs
/// and decides whether to loop again or continue past. Nothing is stored in
/// the implementation, so the whole loop stack is plain serializable data and
/// suspends with the activation (the P4 Delay-inside-ForLoop case).
#[derive(Debug, Clone, PartialEq)]
pub enum FireResult {
    /// The statement ends here: the interpreter unwinds to the enclosing
    /// frame, or finishes the activation if there is none.
    Done,
    /// Continue on this exec output pin. Pops the node's own loop frame if it
    /// owns one — leaving a loop is how a loop ends.
    Continue(&'static str),
    /// Run this exec output as a loop body owned by the firing node.
    Loop(&'static str),
    /// Suspend the activation until its due time, then continue on the pin
    /// the suspension names. **An activation has one continuation, so it has
    /// at most one outstanding suspension** — two simultaneous latents in one
    /// thread are impossible by construction, not by policy.
    Suspend(Suspension),
    /// Stop the activation with a reported error.
    Stop(ExecError),
}

/// A pure node: values in, values out. Called at most once per impure firing
/// (statement-scoped memoization, D1) unless its descriptor says
/// `deterministic: false`, in which case it is volatile and re-evaluated
/// every time it is pulled.
pub trait PureNode: Send + Sync {
    fn eval(&self, ctx: &mut EvalCtx<'_>) -> Result<(), ExecError>;
}

/// An impure node: performs an effect, names a continuation.
pub trait ImpureNode: Send + Sync {
    fn fire(&self, ctx: &mut FireCtx<'_>) -> FireResult;
}

/// One registered implementation. The variant *is* the purity claim, and
/// [`NodeImpls::check_plan`] proves it matches what the plan compiled.
pub enum NodeImpl {
    Pure(Box<dyn PureNode>),
    Impure(Box<dyn ImpureNode>),
}

impl NodeImpl {
    pub fn pure(n: impl PureNode + 'static) -> NodeImpl {
        NodeImpl::Pure(Box::new(n))
    }
    pub fn impure(n: impl ImpureNode + 'static) -> NodeImpl {
        NodeImpl::Impure(Box::new(n))
    }
    pub fn is_pure(&self) -> bool {
        matches!(self, NodeImpl::Pure(_))
    }
}

/// Implementations by node-type slug. Deliberately a plain map owned by the
/// caller — like `NodeRegistry`, there is no global singleton.
#[derive(Default)]
pub struct NodeImpls {
    map: BTreeMap<String, NodeImpl>,
}

impl NodeImpls {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, type_id: &str, imp: NodeImpl) -> &mut Self {
        self.map.insert(type_id.to_string(), imp);
        self
    }

    pub fn get(&self, type_id: &str) -> Option<&NodeImpl> {
        self.map.get(type_id)
    }

    pub fn contains(&self, type_id: &str) -> bool {
        self.map.contains_key(type_id)
    }

    /// Cross-check a compiled plan against these implementations *before*
    /// running it: every node type has an implementation, and its purity
    /// matches the descriptor the plan compiled from. Both failures are
    /// otherwise only discovered when execution reaches that node — which,
    /// inside a rarely-taken Branch, could be days later.
    pub fn check_plan(&self, plan: &crate::plan::Plan) -> Vec<ExecError> {
        let mut out = Vec::new();
        for n in &plan.nodes {
            match self.map.get(&n.type_id) {
                None => out.push(ExecError::MissingImpl {
                    node: n.name.clone(),
                    type_id: n.type_id.clone(),
                }),
                Some(imp) if imp.is_pure() != n.pure => out.push(ExecError::Stopped {
                    node: n.name.clone(),
                    reason: format!(
                        "descriptor says {}, implementation is {}",
                        if n.pure { "pure" } else { "impure" },
                        if imp.is_pure() { "pure" } else { "impure" }
                    ),
                }),
                Some(_) => {}
            }
        }
        out
    }
}

// ---------------------------------------------------------------------------
// Contexts
// ---------------------------------------------------------------------------

/// What a pure node may do.
pub struct EvalCtx<'a> {
    pub(crate) node: &'a str,
    pub(crate) inputs: &'a BTreeMap<String, Value>,
    pub(crate) outputs: &'a mut BTreeMap<String, Value>,
    /// Read-only. Reading a variable is not a side effect, so a pure node may
    /// do it — and the statement-scoped memo stays correct because every
    /// *write* happens at an impure firing, which clears the memo.
    pub(crate) vars: &'a BTreeMap<String, Value>,
    pub(crate) variable: Option<&'a str>,
    pub(crate) tick: TickInput,
    pub(crate) rng: &'a mut Rng,
    pub(crate) world: &'a dyn WorldRead,
    pub(crate) self_entity: EntityRef,
}

/// What an impure node may do.
pub struct FireCtx<'a> {
    pub(crate) node: &'a str,
    pub(crate) inputs: &'a BTreeMap<String, Value>,
    pub(crate) outputs: &'a mut BTreeMap<String, Value>,
    pub(crate) vars: &'a mut BTreeMap<String, Value>,
    /// The variable slug a `var_get`/`var_set` instance names, resolved at
    /// compile time (`PlanNode::variable`) so the implementation stays
    /// stateless and never parses a property at run time.
    pub(crate) variable: Option<&'a str>,
    pub(crate) effects: &'a mut dyn EffectSink,
    pub(crate) world: &'a dyn WorldRead,
    pub(crate) tick: TickInput,
    pub(crate) rng: &'a mut Rng,
    pub(crate) self_entity: EntityRef,
    pub(crate) next_alias: &'a mut u32,
    pub(crate) pending_aliases: &'a mut Vec<u32>,
    pub(crate) queue: &'a mut std::collections::VecDeque<crate::instance::QueuedEvent>,
    /// This node's persistent state slot, read in and written back by the
    /// interpreter around the firing.
    pub(crate) state: Option<Value>,
    /// Which exec input pin this firing was entered through.
    pub(crate) entered: Option<&'a str>,
    /// Instance time, for latent nodes.
    pub(crate) now: f64,
    /// The frame this node owns, when it is being re-fired by the interpreter
    /// after its body finished. `None` on the first firing.
    pub(crate) frame: Option<LoopFrameView>,
}

/// The read-only view a loop node gets of the frame the interpreter owns for
/// it. One number, because a stateless implementation only needs to know
/// which iteration this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoopFrameView {
    /// 0 on the first body run, 1 on the second, …
    pub iteration: u32,
}

/// Input accessors, shared by both contexts. A missing or wrongly-typed input
/// is an `ExecError` naming the pin rather than a silent zero: a graph that
/// type-checked cannot hit it, so hitting it means something upstream lied.
macro_rules! input_accessors {
    () => {
        pub fn input(&self, pin: &str) -> &Value {
            self.inputs.get(pin).unwrap_or(&Value::Unit)
        }

        pub fn int(&self, pin: &str) -> Result<i32, ExecError> {
            self.input(pin).as_int().ok_or_else(|| ExecError::TypeMismatch {
                node: self.node.to_string(),
                pin: pin.to_string(),
                expected: "Int".to_string(),
            })
        }

        pub fn float(&self, pin: &str) -> Result<f32, ExecError> {
            self.input(pin).as_float().ok_or_else(|| ExecError::TypeMismatch {
                node: self.node.to_string(),
                pin: pin.to_string(),
                expected: "Float".to_string(),
            })
        }

        pub fn bool(&self, pin: &str) -> Result<bool, ExecError> {
            self.input(pin).as_bool().ok_or_else(|| ExecError::TypeMismatch {
                node: self.node.to_string(),
                pin: pin.to_string(),
                expected: "Bool".to_string(),
            })
        }

        pub fn vec3(&self, pin: &str) -> Result<[f32; 3], ExecError> {
            self.input(pin).as_vec3().ok_or_else(|| ExecError::TypeMismatch {
                node: self.node.to_string(),
                pin: pin.to_string(),
                expected: "Vec3".to_string(),
            })
        }

        /// The node's plan-level name, for error reporting.
        pub fn node_name(&self) -> &str {
            self.node
        }

        pub fn set_output(&mut self, pin: &str, v: Value) {
            self.outputs.insert(pin.to_string(), v);
        }

        pub fn tick(&self) -> TickInput {
            self.tick
        }

        /// The instance's seeded PRNG. Reading it makes a node *volatile*:
        /// its descriptor must say `deterministic: false`, and the
        /// interpreter then exempts it from statement memoization.
        pub fn rng(&mut self) -> &mut Rng {
            self.rng
        }

        pub fn self_entity(&self) -> EntityRef {
            self.self_entity
        }

        pub fn world(&self) -> &dyn WorldRead {
            self.world
        }

        /// Read a graph variable.
        pub fn var(&self, slug: &str) -> Option<&Value> {
            self.vars.get(slug)
        }

        /// The variable this instance names, for the reserved variable types.
        pub fn variable(&self) -> Option<&str> {
            self.variable
        }
    };
}

impl EvalCtx<'_> {
    input_accessors!();
}

impl FireCtx<'_> {
    input_accessors!();

    pub fn emit(&mut self, effect: Effect) {
        self.effects.emit(effect);
    }

    /// Write a graph variable. Refuses to create one: the set of variables is
    /// the document's, not the runtime's.
    pub fn set_var(&mut self, slug: &str, v: Value) -> Result<(), ExecError> {
        match self.vars.get_mut(slug) {
            Some(slot) => {
                *slot = v;
                Ok(())
            }
            None => Err(ExecError::UnknownVariable {
                node: self.node.to_string(),
                slug: slug.to_string(),
            }),
        }
    }

    /// Reserve an instance-local alias for something about to be spawned.
    /// The graph can hold it in a variable immediately; the runner binds it
    /// to a real entity when the spawn effect is applied.
    pub fn new_alias(&mut self) -> u32 {
        let a = *self.next_alias;
        *self.next_alias += 1;
        self.pending_aliases.push(a);
        a
    }

    /// Queue a custom event on **this** instance, for the next tick. Same-
    /// entity scope in v1 (resolved question 3), and never re-entrant: the
    /// event lands in the queue, not in the current tick.
    pub fn queue_event(&mut self, name: &str, payload: Vec<(String, Value)>) {
        self.queue.push_back(crate::instance::QueuedEvent {
            phase: node_graph_types::EventPhase::Custom,
            name: Some(name.to_string()),
            payload: payload.into_iter().collect(),
        });
    }

    /// Which exec input pin this firing was entered through. `None` for the
    /// first firing of an activation (an entry node is not entered by a wire)
    /// and for a loop node's re-entry from its own body.
    pub fn entered(&self) -> Option<&str> {
        self.entered
    }

    /// This node's persistent state — the one thing a stateful node cannot
    /// re-derive. Survives across firings, activations and ticks, and
    /// serializes with the instance.
    pub fn state(&self) -> Option<&Value> {
        self.state.as_ref()
    }

    pub fn set_state(&mut self, v: Value) {
        self.state = Some(v);
    }

    /// The loop frame the interpreter owns for this node, when it is being
    /// re-fired after its body finished.
    pub fn loop_frame(&self) -> Option<LoopFrameView> {
        self.frame
    }

    /// Instance time: the accumulated total of every `dt` the runner has
    /// supplied since this instance started. **This is the clock latent nodes
    /// measure against** — the same one the due-latent drain compares — and it
    /// is injected, never read.
    pub fn now(&self) -> f64 {
        self.now
    }
}
