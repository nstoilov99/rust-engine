//! The proving node set (P2): just enough real nodes to exercise the whole
//! contract end to end — control flow with an interpreter-owned frame, a pure
//! chain, an effect, a world read, and variables. The rest of D5's library is
//! P3, built on this now-proven contract rather than beside it.
//!
//! Descriptors and implementations live together on purpose: a descriptor
//! whose pins disagree with its implementation is the failure mode this
//! package exists to rule out, and [`register`] cross-checks the two.

use node_graph_types::{
    NodeDescriptor, NodeRealm, NodeRegistry, PinDescriptor, PinType, PropValue, RegistryError,
    EXEC_IN_PIN, EXEC_OUT_PIN, VAR_GET_TYPE_ID, VAR_SET_TYPE_ID, VAR_VALUE_PIN,
};

use crate::effect::{Effect, LogLevel};
use crate::node::{EvalCtx, ExecError, FireCtx, FireResult, NodeImpl, NodeImpls, PureNode};
use crate::value::Value;

pub const BRANCH: &str = "branch";
pub const FOR_LOOP: &str = "for_loop";
pub const ADD_INT: &str = "add_int";
pub const PRINT: &str = "print";
pub const INT_TO_STRING: &str = "int_to_string";
pub const GET_POSITION: &str = "get_position";
pub const SET_POSITION: &str = "set_position";

/// The descriptors for the P2 node set. `var_get`/`var_set` are absent by
/// design — they are reserved doc-dependent types whose pins come from the
/// document's `VarDecl` (P1), so they have no descriptor to register.
pub fn descriptors() -> Vec<NodeDescriptor> {
    let exec_in = || PinDescriptor::new(EXEC_IN_PIN, "Exec", PinType::Exec);
    let exec_out = || PinDescriptor::new(EXEC_OUT_PIN, "Exec", PinType::Exec);
    vec![
        NodeDescriptor {
            id: BRANCH.into(),
            name: "Branch".into(),
            category: "Flow".into(),
            version: 1,
            inputs: vec![
                exec_in(),
                PinDescriptor::new("condition", "Condition", PinType::Bool)
                    .with_default(PropValue::Bool(false)),
            ],
            outputs: vec![
                PinDescriptor::new("true", "True", PinType::Exec),
                PinDescriptor::new("false", "False", PinType::Exec),
            ],
            pure: false,
            realm: NodeRealm::Shared,
            deterministic: true,
            doc: Some("Continues on True or False".into()),
            preview: None,
        },
        NodeDescriptor {
            id: FOR_LOOP.into(),
            name: "For Loop".into(),
            category: "Flow".into(),
            version: 1,
            inputs: vec![
                exec_in(),
                PinDescriptor::new("first", "First", PinType::Int).with_default(PropValue::Int(0)),
                PinDescriptor::new("last", "Last", PinType::Int).with_default(PropValue::Int(0)),
            ],
            outputs: vec![
                PinDescriptor::new("body", "Loop Body", PinType::Exec),
                PinDescriptor::new("index", "Index", PinType::Int),
                PinDescriptor::new("completed", "Completed", PinType::Exec),
            ],
            pure: false,
            realm: NodeRealm::Shared,
            deterministic: true,
            doc: Some("Runs the body once per index from First to Last inclusive".into()),
            preview: None,
        },
        NodeDescriptor {
            id: ADD_INT.into(),
            name: "Add (Int)".into(),
            category: "Math".into(),
            version: 1,
            inputs: vec![
                PinDescriptor::new("a", "A", PinType::Int).with_default(PropValue::Int(0)),
                PinDescriptor::new("b", "B", PinType::Int).with_default(PropValue::Int(0)),
            ],
            outputs: vec![PinDescriptor::new("sum", "Sum", PinType::Int)],
            pure: true,
            realm: NodeRealm::Shared,
            deterministic: true,
            doc: Some("a + b".into()),
            preview: None,
        },
        NodeDescriptor {
            id: INT_TO_STRING.into(),
            name: "To String (Int)".into(),
            category: "Data".into(),
            version: 1,
            inputs: vec![
                PinDescriptor::new("value", "Value", PinType::Int).with_default(PropValue::Int(0))
            ],
            outputs: vec![PinDescriptor::new("text", "Text", PinType::String)],
            pure: true,
            realm: NodeRealm::Shared,
            deterministic: true,
            // There are no implicit conversions in v1 (D9), so wiring an Int
            // into a String pin is a validation error and this node is how you
            // fix it. Naming the type in the slug is deliberate: polymorphic
            // pins are a stated non-goal.
            doc: Some("An Int as text".into()),
            preview: None,
        },
        NodeDescriptor {
            id: PRINT.into(),
            name: "Print".into(),
            category: "Data".into(),
            version: 1,
            inputs: vec![
                exec_in(),
                PinDescriptor::new("text", "Text", PinType::String)
                    .with_default(PropValue::Str(String::new())),
            ],
            outputs: vec![exec_out()],
            pure: false,
            realm: NodeRealm::Shared,
            deterministic: true,
            doc: Some("Writes a line to the console".into()),
            preview: None,
        },
        NodeDescriptor {
            id: GET_POSITION.into(),
            name: "Get Position".into(),
            category: "Spatial".into(),
            version: 1,
            inputs: vec![],
            outputs: vec![PinDescriptor::new("position", "Position", PinType::Vec3)],
            pure: true,
            realm: NodeRealm::Shared,
            // A world read is not a function of the node's inputs, so it must
            // not be memoized across a statement.
            deterministic: false,
            doc: Some("This entity's world position".into()),
            preview: None,
        },
        NodeDescriptor {
            id: SET_POSITION.into(),
            name: "Set Position".into(),
            category: "Spatial".into(),
            version: 1,
            inputs: vec![
                exec_in(),
                PinDescriptor::new("position", "Position", PinType::Vec3)
                    .with_default(PropValue::Vec3([0.0; 3])),
            ],
            outputs: vec![exec_out()],
            pure: false,
            realm: NodeRealm::Shared,
            deterministic: true,
            doc: Some("Moves this entity".into()),
            preview: None,
        },
    ]
}

/// Register the descriptors into `reg`.
pub fn register_descriptors(reg: &mut NodeRegistry) -> Result<(), RegistryError> {
    for d in descriptors() {
        reg.register(d)?;
    }
    Ok(())
}

/// Register the implementations: the P2 node set, the two reserved variable
/// types, and the framework's event entry nodes.
///
/// The event nodes belong here rather than in `node_graph_types` because a
/// *descriptor* is data and an *implementation* is behavior — the types crate
/// declares what an entry node looks like, this crate decides what happens
/// when one fires.
pub fn register_impls(impls: &mut NodeImpls) {
    for id in [
        node_graph_types::EVENT_BEGIN_PLAY_TYPE_ID,
        node_graph_types::EVENT_CUSTOM_TYPE_ID,
        node_graph_types::EVENT_INPUT_ACTION_TYPE_ID,
    ] {
        impls.insert(id, NodeImpl::impure(EventEntry));
    }
    impls.insert(node_graph_types::EVENT_TICK_TYPE_ID, NodeImpl::impure(TickEntry));
    impls
        .insert(BRANCH, NodeImpl::impure(Branch))
        .insert(FOR_LOOP, NodeImpl::impure(ForLoop))
        .insert(ADD_INT, NodeImpl::pure(AddInt))
        .insert(INT_TO_STRING, NodeImpl::pure(IntToString))
        .insert(PRINT, NodeImpl::impure(Print))
        .insert(GET_POSITION, NodeImpl::pure(GetPosition))
        .insert(SET_POSITION, NodeImpl::impure(SetPosition))
        .insert(VAR_GET_TYPE_ID, NodeImpl::pure(VarGet))
        .insert(VAR_SET_TYPE_ID, NodeImpl::impure(VarSet));
}

// ---------------------------------------------------------------------------
// Implementations
// ---------------------------------------------------------------------------

/// Every event entry node does the same thing when its activation starts:
/// hand control to whatever it is wired to. Its *payload* outputs were
/// already seeded from the delivered event by the interpreter, because the
/// payload belongs to the activation, not to the node.
struct EventEntry;

impl crate::node::ImpureNode for EventEntry {
    fn fire(&self, _ctx: &mut FireCtx<'_>) -> FireResult {
        FireResult::Continue(EXEC_OUT_PIN)
    }
}

/// Tick additionally publishes the frame delta it was given — the one number
/// a graph cannot get any other way, since nothing in the core reads a clock.
struct TickEntry;

impl crate::node::ImpureNode for TickEntry {
    fn fire(&self, ctx: &mut FireCtx<'_>) -> FireResult {
        let dt = ctx.tick().dt;
        ctx.set_output("dt", Value::Float(dt));
        FireResult::Continue(EXEC_OUT_PIN)
    }
}

struct Branch;

impl crate::node::ImpureNode for Branch {
    fn fire(&self, ctx: &mut FireCtx<'_>) -> FireResult {
        match ctx.bool("condition") {
            Ok(true) => FireResult::Continue("true"),
            Ok(false) => FireResult::Continue("false"),
            Err(e) => FireResult::Stop(e),
        }
    }
}

/// The frame protocol in one node: no state of its own, everything
/// re-derived from `first` plus the interpreter's iteration counter.
struct ForLoop;

impl crate::node::ImpureNode for ForLoop {
    fn fire(&self, ctx: &mut FireCtx<'_>) -> FireResult {
        let (first, last) = match (ctx.int("first"), ctx.int("last")) {
            (Ok(a), Ok(b)) => (a, b),
            (Err(e), _) | (_, Err(e)) => return FireResult::Stop(e),
        };
        let iteration = ctx.loop_frame().map(|f| f.iteration).unwrap_or(0);
        let index = first as i64 + iteration as i64;
        if index > last as i64 {
            // `Continue` pops the frame — leaving a loop is how a loop ends.
            return FireResult::Continue("completed");
        }
        ctx.set_output("index", Value::Int(index as i32));
        FireResult::Loop("body")
    }
}

struct AddInt;

impl PureNode for AddInt {
    fn eval(&self, ctx: &mut EvalCtx<'_>) -> Result<(), ExecError> {
        let sum = ctx.int("a")?.wrapping_add(ctx.int("b")?);
        ctx.set_output("sum", Value::Int(sum));
        Ok(())
    }
}

struct IntToString;

impl PureNode for IntToString {
    fn eval(&self, ctx: &mut EvalCtx<'_>) -> Result<(), ExecError> {
        let v = ctx.int("value")?;
        ctx.set_output("text", Value::Str(v.to_string()));
        Ok(())
    }
}

struct Print;

impl crate::node::ImpureNode for Print {
    fn fire(&self, ctx: &mut FireCtx<'_>) -> FireResult {
        // Deliberately permissive about the input type: printing a value is
        // the one place where "whatever it is, show it" is the right answer.
        let text = ctx.input("text").to_string();
        ctx.emit(Effect::Log { level: LogLevel::Info, text });
        FireResult::Continue(EXEC_OUT_PIN)
    }
}

struct GetPosition;

impl PureNode for GetPosition {
    fn eval(&self, ctx: &mut EvalCtx<'_>) -> Result<(), ExecError> {
        let p = ctx
            .world()
            .position(ctx.self_entity())
            .unwrap_or([0.0; 3]);
        ctx.set_output("position", Value::Vec3(p));
        Ok(())
    }
}

struct SetPosition;

impl crate::node::ImpureNode for SetPosition {
    fn fire(&self, ctx: &mut FireCtx<'_>) -> FireResult {
        match ctx.vec3("position") {
            Ok(position) => {
                let entity = ctx.self_entity();
                ctx.emit(Effect::SetPosition { entity, position });
                FireResult::Continue(EXEC_OUT_PIN)
            }
            Err(e) => FireResult::Stop(e),
        }
    }
}

/// `var_get` — pure, one output whose slug is fixed by P1's reserved scheme.
/// The variable it names was resolved at compile time into
/// `PlanNode::variable`, so the implementation parses nothing and holds
/// nothing. Reading a variable is not a side effect, which is why this is
/// legitimately pure: every *write* happens at an impure firing, and that is
/// exactly when the statement memo is cleared.
struct VarGet;

impl PureNode for VarGet {
    fn eval(&self, ctx: &mut EvalCtx<'_>) -> Result<(), ExecError> {
        let Some(slug) = ctx.variable().map(|s| s.to_string()) else {
            return Err(ExecError::UnknownVariable {
                node: ctx.node_name().to_string(),
                slug: String::new(),
            });
        };
        let v = ctx.var(&slug).cloned().ok_or_else(|| ExecError::UnknownVariable {
            node: ctx.node_name().to_string(),
            slug: slug.clone(),
        })?;
        ctx.set_output(VAR_VALUE_PIN, v);
        Ok(())
    }
}

struct VarSet;

impl crate::node::ImpureNode for VarSet {
    fn fire(&self, ctx: &mut FireCtx<'_>) -> FireResult {
        let v = ctx.input(VAR_VALUE_PIN).clone();
        // The slug is carried on the context by the interpreter, which read
        // it off the plan node — the implementation stays stateless.
        let Some(slug) = ctx.variable().map(|s| s.to_string()) else {
            return FireResult::Stop(ExecError::UnknownVariable {
                node: ctx.node_name().to_string(),
                slug: String::new(),
            });
        };
        match ctx.set_var(&slug, v) {
            Ok(()) => FireResult::Continue(EXEC_OUT_PIN),
            Err(e) => FireResult::Stop(e),
        }
    }
}
