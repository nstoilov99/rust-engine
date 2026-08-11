//! The standard node library's **implementations** (Task 45-A D5, P3).
//!
//! Descriptors live in [`node_graph_types::std_nodes`] — the editor needs
//! those and has no business depending on the interpreter. What lives here is
//! behavior, and [`NodeImpls::check_plan`] is what proves the two agree: a
//! node whose implementation is missing, or whose purity disagrees with its
//! descriptor, is reported before the plan runs.
//!
//! Three patterns recur, and they are worth naming once:
//!
//! - **Loops are stateless.** `ForLoop`, `WhileLoop`, `ForEach` and
//!   `Sequence` all re-derive their position from `ctx.loop_frame()` — the
//!   interpreter owns the frame, so the whole loop stack serializes and
//!   suspends with the activation.
//! - **Stateful nodes use `ctx.state()`.** `Gate`, `DoOnce` and `FlipFlop`
//!   hold the one thing they cannot re-derive in the instance's per-node
//!   state slot, not in the implementation, which stays `Sync` and shared.
//! - **Multi-entrance nodes read `ctx.entered()`.** `Gate` behaves
//!   differently per exec input, and only the edge that arrived knows which.

use node_graph_types::std_nodes as ids;
use node_graph_types::{EXEC_OUT_PIN, VAR_GET_TYPE_ID, VAR_SET_TYPE_ID, VAR_VALUE_PIN};

use crate::effect::{Effect, LogLevel, TransformSnapshot};
use crate::node::{
    EvalCtx, ExecError, FireCtx, FireResult, ImpureNode, NodeImpl, NodeImpls, PureNode,
};
use crate::value::{EntityRef, Value};

/// Register every standard implementation, the reserved variable types, and
/// the framework's event entry nodes.
pub fn register_impls(impls: &mut NodeImpls) {
    // --- events -------------------------------------------------------
    for id in [
        node_graph_types::EVENT_BEGIN_PLAY_TYPE_ID,
        node_graph_types::EVENT_CUSTOM_TYPE_ID,
        node_graph_types::EVENT_INPUT_ACTION_TYPE_ID,
    ] {
        impls.insert(id, NodeImpl::impure(EventEntry));
    }
    impls.insert(node_graph_types::EVENT_TICK_TYPE_ID, NodeImpl::impure(TickEntry));

    // --- reserved doc-dependent types ----------------------------------
    impls
        .insert(VAR_GET_TYPE_ID, NodeImpl::pure(VarGet))
        .insert(VAR_SET_TYPE_ID, NodeImpl::impure(VarSet));

    // --- control -------------------------------------------------------
    impls
        .insert(ids::BRANCH, NodeImpl::impure(Branch))
        .insert(ids::SEQUENCE, NodeImpl::impure(Sequence))
        .insert(ids::FOR_LOOP, NodeImpl::impure(ForLoop))
        .insert(ids::WHILE_LOOP, NodeImpl::impure(WhileLoop))
        .insert(ids::GATE, NodeImpl::impure(Gate))
        .insert(ids::DO_ONCE, NodeImpl::impure(DoOnce))
        .insert(ids::FLIP_FLOP, NodeImpl::impure(FlipFlop));
    for id in [ids::FOR_EACH_FLOAT, ids::FOR_EACH_INT, ids::FOR_EACH_ENTITY] {
        impls.insert(id, NodeImpl::impure(ForEach));
    }
    for id in [ids::SELECT_FLOAT, ids::SELECT_INT, ids::SELECT_BOOL, ids::SELECT_STRING] {
        impls.insert(id, NodeImpl::pure(Select));
    }

    // --- logic / math ---------------------------------------------------
    impls
        .insert(ids::AND, NodeImpl::pure(Logic::And))
        .insert(ids::OR, NodeImpl::pure(Logic::Or))
        .insert(ids::NOT, NodeImpl::pure(Logic::Not))
        .insert(ids::COMPARE_INT, NodeImpl::pure(CompareInt))
        .insert(ids::COMPARE_FLOAT, NodeImpl::pure(CompareFloat))
        .insert(ids::ADD_INT, NodeImpl::pure(ArithInt(Op::Add)))
        .insert(ids::SUB_INT, NodeImpl::pure(ArithInt(Op::Sub)))
        .insert(ids::MUL_INT, NodeImpl::pure(ArithInt(Op::Mul)))
        .insert(ids::DIV_INT, NodeImpl::pure(ArithInt(Op::Div)))
        .insert(ids::ADD_FLOAT, NodeImpl::pure(ArithFloat(Op::Add)))
        .insert(ids::SUB_FLOAT, NodeImpl::pure(ArithFloat(Op::Sub)))
        .insert(ids::MUL_FLOAT, NodeImpl::pure(ArithFloat(Op::Mul)))
        .insert(ids::DIV_FLOAT, NodeImpl::pure(ArithFloat(Op::Div)))
        .insert(ids::LERP_FLOAT, NodeImpl::pure(LerpFloat))
        .insert(ids::CLAMP_FLOAT, NodeImpl::pure(ClampFloat))
        .insert(ids::CLAMP_INT, NodeImpl::pure(ClampInt))
        .insert(ids::RANDOM_FLOAT, NodeImpl::pure(RandomFloat))
        .insert(ids::RANDOM_INT, NodeImpl::pure(RandomInt));

    // --- data ------------------------------------------------------------
    impls
        .insert(ids::MAKE_VEC3, NodeImpl::pure(MakeVec3))
        .insert(ids::BREAK_VEC3, NodeImpl::pure(BreakVec3))
        .insert(ids::INT_TO_FLOAT, NodeImpl::pure(IntToFloat))
        .insert(ids::FLOAT_TO_INT, NodeImpl::pure(FloatToInt))
        .insert(ids::INT_TO_STRING, NodeImpl::pure(ToText::Int))
        .insert(ids::FLOAT_TO_STRING, NodeImpl::pure(ToText::Float))
        .insert(ids::BOOL_TO_STRING, NodeImpl::pure(ToText::Bool));

    // --- effects ----------------------------------------------------------
    impls
        .insert(ids::PRINT, NodeImpl::impure(Print))
        .insert(ids::EMIT_EVENT, NodeImpl::impure(EmitEvent))
        .insert(ids::SPAWN_PREFAB, NodeImpl::impure(SpawnPrefab))
        .insert(ids::DESTROY_ENTITY, NodeImpl::impure(DestroyEntity))
        .insert(ids::GET_SELF, NodeImpl::pure(GetSelf))
        .insert(ids::GET_POSITION, NodeImpl::pure(GetPosition))
        .insert(ids::SET_POSITION, NodeImpl::impure(SetPosition))
        .insert(ids::GET_TRANSFORM, NodeImpl::pure(GetTransform))
        .insert(ids::SET_TRANSFORM, NodeImpl::impure(SetTransform));
}

/// An `entity` input pin: unwired it is the instance's own entity, because
/// `Value::zero_of(Entity)` is `self`.
fn entity_of(ctx: &FireCtx<'_>) -> EntityRef {
    ctx.input("entity").as_entity().unwrap_or_else(|| ctx.self_entity())
}

fn entity_of_eval(ctx: &EvalCtx<'_>) -> EntityRef {
    ctx.input("entity").as_entity().unwrap_or_else(|| ctx.self_entity())
}

// ---------------------------------------------------------------------------
// Events and variables
// ---------------------------------------------------------------------------

/// Every event entry node does the same thing when its activation starts:
/// hand control to whatever it is wired to. Its *payload* outputs were
/// already seeded from the delivered event by the interpreter, because the
/// payload belongs to the activation, not to the node.
struct EventEntry;

impl ImpureNode for EventEntry {
    fn fire(&self, _ctx: &mut FireCtx<'_>) -> FireResult {
        FireResult::Continue(EXEC_OUT_PIN)
    }
}

/// Tick additionally publishes the frame delta it was given — the one number
/// a graph cannot get any other way, since nothing in the core reads a clock.
struct TickEntry;

impl ImpureNode for TickEntry {
    fn fire(&self, ctx: &mut FireCtx<'_>) -> FireResult {
        let dt = ctx.tick().dt;
        ctx.set_output("dt", Value::Float(dt));
        FireResult::Continue(EXEC_OUT_PIN)
    }
}

/// `var_get` — pure, one output whose slug is fixed by P1's reserved scheme.
/// Reading a variable is not a side effect, which is why this is
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

impl ImpureNode for VarSet {
    fn fire(&self, ctx: &mut FireCtx<'_>) -> FireResult {
        let v = ctx.input(VAR_VALUE_PIN).clone();
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

// ---------------------------------------------------------------------------
// Control flow
// ---------------------------------------------------------------------------

struct Branch;

impl ImpureNode for Branch {
    fn fire(&self, ctx: &mut FireCtx<'_>) -> FireResult {
        match ctx.bool("condition") {
            Ok(true) => FireResult::Continue("true"),
            Ok(false) => FireResult::Continue("false"),
            Err(e) => FireResult::Stop(e),
        }
    }
}

/// Sequence runs each connected output *to completion* before the next — so
/// it is a loop over its own pins, and it gets the loop protocol for free.
/// Unconnected outputs cost one iteration and nothing else, which is what
/// lets one node serve D5's "Sequence(2–4)".
struct Sequence;

impl ImpureNode for Sequence {
    fn fire(&self, ctx: &mut FireCtx<'_>) -> FireResult {
        let n = ctx.loop_frame().map(|f| f.iteration).unwrap_or(0) as usize;
        match node_graph_types::SEQUENCE_PINS.get(n) {
            Some(pin) => FireResult::Loop(pin),
            None => FireResult::Done,
        }
    }
}

/// The frame protocol in one node: no state of its own, everything
/// re-derived from `first` plus the interpreter's iteration counter.
struct ForLoop;

impl ImpureNode for ForLoop {
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

/// The condition is a *data input*, so it is re-pulled on every firing — and
/// the interpreter re-fires this node once per pass. That is what makes
/// "while" mean while rather than "repeat N times".
struct WhileLoop;

impl ImpureNode for WhileLoop {
    fn fire(&self, ctx: &mut FireCtx<'_>) -> FireResult {
        match ctx.bool("condition") {
            Ok(true) => FireResult::Loop("body"),
            Ok(false) => FireResult::Continue("completed"),
            Err(e) => FireResult::Stop(e),
        }
    }
}

/// One implementation behind all three `for_each_*` descriptors: the element
/// type changes the pins, not the behavior.
struct ForEach;

impl ImpureNode for ForEach {
    fn fire(&self, ctx: &mut FireCtx<'_>) -> FireResult {
        let Some(items) = ctx.input("array").as_array().map(|s| s.to_vec()) else {
            return FireResult::Stop(ExecError::TypeMismatch {
                node: ctx.node_name().to_string(),
                pin: "array".to_string(),
                expected: "Array".to_string(),
            });
        };
        let n = ctx.loop_frame().map(|f| f.iteration).unwrap_or(0) as usize;
        match items.get(n) {
            None => FireResult::Continue("completed"),
            Some(v) => {
                ctx.set_output("element", v.clone());
                ctx.set_output("index", Value::Int(n as i32));
                FireResult::Loop("body")
            }
        }
    }
}

/// Gate is the multi-entrance node: what it does depends entirely on which
/// exec input arrived, which is why the continuation carries the pin.
struct Gate;

impl ImpureNode for Gate {
    fn fire(&self, ctx: &mut FireCtx<'_>) -> FireResult {
        let open = match ctx.state().and_then(|v| v.as_bool()) {
            Some(b) => b,
            // First entry: the start state is a declared input, so an author
            // can flip it without a wire.
            None => !ctx.bool("start_closed").unwrap_or(false),
        };
        match ctx.entered() {
            Some("open") => {
                ctx.set_state(Value::Bool(true));
                FireResult::Done
            }
            Some("close") => {
                ctx.set_state(Value::Bool(false));
                FireResult::Done
            }
            Some("toggle") => {
                ctx.set_state(Value::Bool(!open));
                FireResult::Done
            }
            // "enter", and the entry-point case where nothing named a pin.
            _ => {
                ctx.set_state(Value::Bool(open));
                if open {
                    FireResult::Continue("exit")
                } else {
                    FireResult::Done
                }
            }
        }
    }
}

struct DoOnce;

impl ImpureNode for DoOnce {
    fn fire(&self, ctx: &mut FireCtx<'_>) -> FireResult {
        let spent = match ctx.state().and_then(|v| v.as_bool()) {
            Some(b) => b,
            None => ctx.bool("start_closed").unwrap_or(false),
        };
        if ctx.entered() == Some("reset") {
            ctx.set_state(Value::Bool(false));
            return FireResult::Done;
        }
        if spent {
            ctx.set_state(Value::Bool(true));
            return FireResult::Done;
        }
        ctx.set_state(Value::Bool(true));
        FireResult::Continue("completed")
    }
}

struct FlipFlop;

impl ImpureNode for FlipFlop {
    fn fire(&self, ctx: &mut FireCtx<'_>) -> FireResult {
        // State holds which side *this* entry takes; `true` = A, and A is
        // first because "flip" reads before "flop".
        let is_a = ctx.state().and_then(|v| v.as_bool()).unwrap_or(true);
        ctx.set_state(Value::Bool(!is_a));
        ctx.set_output("is_a", Value::Bool(is_a));
        FireResult::Continue(if is_a { "a" } else { "b" })
    }
}

/// One implementation behind all four `select_*` descriptors.
struct Select;

impl PureNode for Select {
    fn eval(&self, ctx: &mut EvalCtx<'_>) -> Result<(), ExecError> {
        let pick = if ctx.bool("condition")? { "if_true" } else { "if_false" };
        let v = ctx.input(pick).clone();
        ctx.set_output("result", v);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Logic and math
// ---------------------------------------------------------------------------

enum Logic {
    And,
    Or,
    Not,
}

impl PureNode for Logic {
    fn eval(&self, ctx: &mut EvalCtx<'_>) -> Result<(), ExecError> {
        let a = ctx.bool("a")?;
        let r = match self {
            Logic::And => a && ctx.bool("b")?,
            Logic::Or => a || ctx.bool("b")?,
            Logic::Not => !a,
        };
        ctx.set_output("result", Value::Bool(r));
        Ok(())
    }
}

/// The operator of `compare_*`, parsed from the declared-variant `Enum` pin.
/// An unrecognized slug is stale data, and `equal` is the honest fallback:
/// the descriptor's default, and the one the author last saw.
fn compare<T: PartialOrd>(op: &str, a: T, b: T) -> bool {
    match op {
        "not_equal" => a != b,
        "less" => a < b,
        "less_equal" => a <= b,
        "greater" => a > b,
        "greater_equal" => a >= b,
        _ => a == b,
    }
}

struct CompareInt;

impl PureNode for CompareInt {
    fn eval(&self, ctx: &mut EvalCtx<'_>) -> Result<(), ExecError> {
        let op = ctx.input("op").as_str().unwrap_or("equal").to_string();
        let r = compare(&op, ctx.int("a")?, ctx.int("b")?);
        ctx.set_output("result", Value::Bool(r));
        Ok(())
    }
}

struct CompareFloat;

impl PureNode for CompareFloat {
    fn eval(&self, ctx: &mut EvalCtx<'_>) -> Result<(), ExecError> {
        let op = ctx.input("op").as_str().unwrap_or("equal").to_string();
        let r = compare(&op, ctx.float("a")?, ctx.float("b")?);
        ctx.set_output("result", Value::Bool(r));
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum Op {
    Add,
    Sub,
    Mul,
    Div,
}

struct ArithInt(Op);

impl PureNode for ArithInt {
    fn eval(&self, ctx: &mut EvalCtx<'_>) -> Result<(), ExecError> {
        let (a, b) = (ctx.int("a")?, ctx.int("b")?);
        let r = match self.0 {
            // Wrapping, not panicking: a graph is user input, and a division
            // by zero yields 0 for the same reason.
            Op::Add => a.wrapping_add(b),
            Op::Sub => a.wrapping_sub(b),
            Op::Mul => a.wrapping_mul(b),
            Op::Div => {
                if b == 0 {
                    0
                } else {
                    a.wrapping_div(b)
                }
            }
        };
        ctx.set_output("result", Value::Int(r));
        Ok(())
    }
}

struct ArithFloat(Op);

impl PureNode for ArithFloat {
    fn eval(&self, ctx: &mut EvalCtx<'_>) -> Result<(), ExecError> {
        let (a, b) = (ctx.float("a")?, ctx.float("b")?);
        let r = match self.0 {
            Op::Add => a + b,
            Op::Sub => a - b,
            Op::Mul => a * b,
            // 0 rather than an infinity or a NaN: a NaN loose in a graph
            // poisons everything downstream silently, which is worse than a
            // visibly wrong zero.
            Op::Div => {
                if b == 0.0 {
                    0.0
                } else {
                    a / b
                }
            }
        };
        ctx.set_output("result", Value::Float(r));
        Ok(())
    }
}

struct LerpFloat;

impl PureNode for LerpFloat {
    fn eval(&self, ctx: &mut EvalCtx<'_>) -> Result<(), ExecError> {
        let (a, b, t) = (ctx.float("a")?, ctx.float("b")?, ctx.float("alpha")?);
        let t = t.clamp(0.0, 1.0);
        ctx.set_output("result", Value::Float(a + (b - a) * t));
        Ok(())
    }
}

struct ClampFloat;

impl PureNode for ClampFloat {
    fn eval(&self, ctx: &mut EvalCtx<'_>) -> Result<(), ExecError> {
        let (v, lo, hi) = (ctx.float("value")?, ctx.float("min")?, ctx.float("max")?);
        // Inverted bounds are author error, not a panic.
        let r = if lo > hi { lo } else { v.clamp(lo, hi) };
        ctx.set_output("result", Value::Float(r));
        Ok(())
    }
}

struct ClampInt;

impl PureNode for ClampInt {
    fn eval(&self, ctx: &mut EvalCtx<'_>) -> Result<(), ExecError> {
        let (v, lo, hi) = (ctx.int("value")?, ctx.int("min")?, ctx.int("max")?);
        let r = if lo > hi { lo } else { v.clamp(lo, hi) };
        ctx.set_output("result", Value::Int(r));
        Ok(())
    }
}

struct RandomFloat;

impl PureNode for RandomFloat {
    fn eval(&self, ctx: &mut EvalCtx<'_>) -> Result<(), ExecError> {
        let (lo, hi) = (ctx.float("min")?, ctx.float("max")?);
        let t = ctx.rng().next_f32();
        ctx.set_output("value", Value::Float(lo + (hi - lo) * t));
        Ok(())
    }
}

struct RandomInt;

impl PureNode for RandomInt {
    fn eval(&self, ctx: &mut EvalCtx<'_>) -> Result<(), ExecError> {
        let (lo, hi) = (ctx.int("min")?, ctx.int("max")?);
        let v = ctx.rng().next_range(lo, hi);
        ctx.set_output("value", Value::Int(v));
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Data
// ---------------------------------------------------------------------------

struct MakeVec3;

impl PureNode for MakeVec3 {
    fn eval(&self, ctx: &mut EvalCtx<'_>) -> Result<(), ExecError> {
        let v = [ctx.float("x")?, ctx.float("y")?, ctx.float("z")?];
        ctx.set_output("result", Value::Vec3(v));
        Ok(())
    }
}

struct BreakVec3;

impl PureNode for BreakVec3 {
    fn eval(&self, ctx: &mut EvalCtx<'_>) -> Result<(), ExecError> {
        let v = ctx.vec3("value")?;
        ctx.set_output("x", Value::Float(v[0]));
        ctx.set_output("y", Value::Float(v[1]));
        ctx.set_output("z", Value::Float(v[2]));
        Ok(())
    }
}

struct IntToFloat;

impl PureNode for IntToFloat {
    fn eval(&self, ctx: &mut EvalCtx<'_>) -> Result<(), ExecError> {
        let v = ctx.int("value")?;
        ctx.set_output("result", Value::Float(v as f32));
        Ok(())
    }
}

struct FloatToInt;

impl PureNode for FloatToInt {
    fn eval(&self, ctx: &mut EvalCtx<'_>) -> Result<(), ExecError> {
        let v = ctx.float("value")?;
        // Truncation toward zero, saturating: `as` already saturates in Rust,
        // and a NaN becomes 0 rather than something arbitrary.
        ctx.set_output("result", Value::Int(if v.is_nan() { 0 } else { v as i32 }));
        Ok(())
    }
}

/// One implementation behind the three `*_to_string` descriptors: the input
/// pin type differs, the text is `Value`'s own spelling either way — the same
/// spelling `Print` produces, so a value reads identically wherever it lands.
enum ToText {
    Int,
    Float,
    Bool,
}

impl PureNode for ToText {
    fn eval(&self, ctx: &mut EvalCtx<'_>) -> Result<(), ExecError> {
        let text = match self {
            ToText::Int => Value::Int(ctx.int("value")?).to_string(),
            ToText::Float => Value::Float(ctx.float("value")?).to_string(),
            ToText::Bool => Value::Bool(ctx.bool("value")?).to_string(),
        };
        ctx.set_output("text", Value::Str(text));
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Effects
// ---------------------------------------------------------------------------

struct Print;

impl ImpureNode for Print {
    fn fire(&self, ctx: &mut FireCtx<'_>) -> FireResult {
        // Deliberately permissive about the input type: printing a value is
        // the one place where "whatever it is, show it" is the right answer.
        let text = ctx.input("text").to_string();
        ctx.emit(Effect::Log { level: LogLevel::Info, text });
        FireResult::Continue(EXEC_OUT_PIN)
    }
}

struct EmitEvent;

impl ImpureNode for EmitEvent {
    fn fire(&self, ctx: &mut FireCtx<'_>) -> FireResult {
        let name = ctx.input("name").as_str().unwrap_or_default().to_string();
        // Queued on *this* instance, for the *next* tick: same-entity scope
        // (resolved question 3) and no reentrancy (D3).
        ctx.queue_event(&name, Vec::new());
        // …and recorded in the stream so the console and the P7 trace see it.
        // The runner must not deliver it again.
        ctx.emit(Effect::EmitEvent { name, payload: Vec::new() });
        FireResult::Continue(EXEC_OUT_PIN)
    }
}

struct SpawnPrefab;

impl ImpureNode for SpawnPrefab {
    fn fire(&self, ctx: &mut FireCtx<'_>) -> FireResult {
        let path = ctx.input("path").as_str().unwrap_or_default().to_string();
        let position = match ctx.vec3("position") {
            Ok(p) => p,
            Err(e) => return FireResult::Stop(e),
        };
        // The alias is handed out *now*, so the graph can store the handle in
        // a variable and act on it in the same statement. The runner binds it
        // to a real entity when the spawn applies (D1).
        let alias = ctx.new_alias();
        ctx.set_output("spawned", Value::Entity(EntityRef::Spawned(alias)));
        ctx.emit(Effect::SpawnPrefab {
            path,
            alias,
            transform: TransformSnapshot { position, ..Default::default() },
        });
        FireResult::Continue(EXEC_OUT_PIN)
    }
}

struct DestroyEntity;

impl ImpureNode for DestroyEntity {
    fn fire(&self, ctx: &mut FireCtx<'_>) -> FireResult {
        let entity = entity_of(ctx);
        ctx.emit(Effect::DestroyEntity { entity });
        FireResult::Continue(EXEC_OUT_PIN)
    }
}

struct GetSelf;

impl PureNode for GetSelf {
    fn eval(&self, ctx: &mut EvalCtx<'_>) -> Result<(), ExecError> {
        let me = ctx.self_entity();
        ctx.set_output("self", Value::Entity(me));
        Ok(())
    }
}

struct GetPosition;

impl PureNode for GetPosition {
    fn eval(&self, ctx: &mut EvalCtx<'_>) -> Result<(), ExecError> {
        let e = entity_of_eval(ctx);
        let p = ctx.world().position(e).unwrap_or([0.0; 3]);
        ctx.set_output("position", Value::Vec3(p));
        Ok(())
    }
}

struct SetPosition;

impl ImpureNode for SetPosition {
    fn fire(&self, ctx: &mut FireCtx<'_>) -> FireResult {
        match ctx.vec3("position") {
            Ok(position) => {
                let entity = entity_of(ctx);
                ctx.emit(Effect::SetPosition { entity, position });
                FireResult::Continue(EXEC_OUT_PIN)
            }
            Err(e) => FireResult::Stop(e),
        }
    }
}

struct GetTransform;

impl PureNode for GetTransform {
    fn eval(&self, ctx: &mut EvalCtx<'_>) -> Result<(), ExecError> {
        let e = entity_of_eval(ctx);
        let t = ctx.world().transform(e).unwrap_or_default();
        ctx.set_output("position", Value::Vec3(t.position));
        ctx.set_output("rotation", Value::Vec4(t.rotation));
        ctx.set_output("scale", Value::Vec3(t.scale));
        Ok(())
    }
}

struct SetTransform;

impl ImpureNode for SetTransform {
    fn fire(&self, ctx: &mut FireCtx<'_>) -> FireResult {
        let position = match ctx.vec3("position") {
            Ok(p) => p,
            Err(e) => return FireResult::Stop(e),
        };
        let rotation = match ctx.input("rotation") {
            Value::Vec4(q) => *q,
            _ => [0.0, 0.0, 0.0, 1.0],
        };
        let scale = match ctx.vec3("scale") {
            Ok(s) => s,
            Err(e) => return FireResult::Stop(e),
        };
        let entity = entity_of(ctx);
        // Three fine-grained effects rather than one omnibus write (resolved
        // question 1): the runner applies exactly what changed.
        ctx.emit(Effect::SetPosition { entity, position });
        ctx.emit(Effect::SetRotation { entity, rotation });
        ctx.emit(Effect::SetScale { entity, scale });
        FireResult::Continue(EXEC_OUT_PIN)
    }
}

/// The one entry point a consumer needs: `let mut impls = std_impls();`
pub fn std_impls() -> NodeImpls {
    let mut impls = NodeImpls::new();
    register_impls(&mut impls);
    impls
}

// Re-exported so a test or a consumer can name a node without a second
// `use` line reaching into the types crate.
pub use node_graph_types::std_nodes::*;
