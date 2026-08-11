//! The standard node library's **descriptors** (Task 45-A D5, P3).
//!
//! Descriptors live here, beside [`std_events`](crate::std_events), and the
//! implementations live in `node_graph_exec::nodes`. The split is not
//! bureaucratic: the *editor* needs descriptors — for the create menu, pin
//! layout, inline widgets and validation — and the editor has no business
//! depending on the interpreter. `NodeImpls::check_plan` is what keeps the
//! two honest, reporting any node whose implementation is missing or whose
//! purity disagrees with its descriptor.
//!
//! Nothing here registers itself into a live registry; the `graph_scripting`
//! plugin does that in P5.
//!
//! # Naming and granularity
//!
//! Slugs are forever (Task 40 identity rules), so they are boring on purpose:
//! `snake_case`, verb-or-noun, and **type-suffixed wherever D9 forbids
//! polymorphic pins** — `add_int` / `add_float`, `select_bool`. Three rulings
//! shape the list:
//!
//! - **Comparisons collapse into two nodes**, `compare_int` and
//!   `compare_float`, with the operator as an `Enum` input carrying declared
//!   variants. Twelve near-identical nodes (six operators × two types) would
//!   bloat the create menu to say nothing extra; Blueprint only ships them
//!   separately because it has no inline enum dropdown, and we do (P1 gave
//!   `PinDescriptor` its `variants` list for exactly this).
//! - **Arithmetic stays one node per operation**, because Add and Multiply
//!   are recognized shapes, not a parameter — the reverse trade of the above.
//! - **`for_each` and `select` ship one variant per element type** that a v1
//!   graph actually iterates or picks between. A wildcard pin would collapse
//!   them, and a wildcard pin is a stated non-goal.

use crate::doc::{NodeRealm, PinType, PropValue};
use crate::registry::{
    NodeDescriptor, NodeRegistry, PinDescriptor, RegistryError, EXEC_IN_PIN, EXEC_OUT_PIN,
};

// --- control -----------------------------------------------------------
pub const BRANCH: &str = "branch";
pub const SEQUENCE: &str = "sequence";
pub const FOR_LOOP: &str = "for_loop";
pub const WHILE_LOOP: &str = "while_loop";
pub const FOR_EACH_FLOAT: &str = "for_each_float";
pub const FOR_EACH_INT: &str = "for_each_int";
pub const FOR_EACH_ENTITY: &str = "for_each_entity";
pub const DELAY: &str = "delay";
pub const GATE: &str = "gate";
pub const DO_ONCE: &str = "do_once";
pub const FLIP_FLOP: &str = "flip_flop";
pub const SELECT_FLOAT: &str = "select_float";
pub const SELECT_INT: &str = "select_int";
pub const SELECT_BOOL: &str = "select_bool";
pub const SELECT_STRING: &str = "select_string";

// --- logic / math ------------------------------------------------------
pub const AND: &str = "and";
pub const OR: &str = "or";
pub const NOT: &str = "not";
pub const COMPARE_INT: &str = "compare_int";
pub const COMPARE_FLOAT: &str = "compare_float";
pub const ADD_INT: &str = "add_int";
pub const SUB_INT: &str = "sub_int";
pub const MUL_INT: &str = "mul_int";
pub const DIV_INT: &str = "div_int";
pub const ADD_FLOAT: &str = "add_float";
pub const SUB_FLOAT: &str = "sub_float";
pub const MUL_FLOAT: &str = "mul_float";
pub const DIV_FLOAT: &str = "div_float";
pub const LERP_FLOAT: &str = "lerp_float";
pub const CLAMP_FLOAT: &str = "clamp_float";
pub const CLAMP_INT: &str = "clamp_int";
pub const RANDOM_FLOAT: &str = "random_float";
pub const RANDOM_INT: &str = "random_int";

// --- data --------------------------------------------------------------
pub const MAKE_VEC3: &str = "make_vec3";
pub const BREAK_VEC3: &str = "break_vec3";
pub const INT_TO_FLOAT: &str = "int_to_float";
pub const FLOAT_TO_INT: &str = "float_to_int";
pub const INT_TO_STRING: &str = "int_to_string";
pub const FLOAT_TO_STRING: &str = "float_to_string";
pub const BOOL_TO_STRING: &str = "bool_to_string";

// --- effects -----------------------------------------------------------
pub const PRINT: &str = "print";
pub const EMIT_EVENT: &str = "emit_event";
pub const SPAWN_PREFAB: &str = "spawn_prefab";
pub const DESTROY_ENTITY: &str = "destroy_entity";
pub const GET_SELF: &str = "get_self";
pub const GET_POSITION: &str = "get_position";
pub const SET_POSITION: &str = "set_position";
pub const GET_TRANSFORM: &str = "get_transform";
pub const SET_TRANSFORM: &str = "set_transform";

/// The declared operators of `compare_int` / `compare_float`'s `op` pin. The
/// slugs are the serialized identity, so they are spelled out rather than
/// derived from a display name.
pub const COMPARE_OPS: [&str; 6] = [
    "equal",
    "not_equal",
    "less",
    "less_equal",
    "greater",
    "greater_equal",
];

/// The `then_N` exec outputs of [`SEQUENCE`]. Four is the top of D5's
/// "Sequence(2–4)" range; unwired outputs are simply skipped, so one node
/// serves every arity instead of three node types serving one each.
pub const SEQUENCE_PINS: [&str; 4] = ["then_0", "then_1", "then_2", "then_3"];

// ---------------------------------------------------------------------------
// Builders — the descriptors below are data, and reading them should feel
// like reading a table.
// ---------------------------------------------------------------------------

fn exec_in() -> PinDescriptor {
    PinDescriptor::new(EXEC_IN_PIN, "Exec", PinType::Exec)
}

fn exec_out() -> PinDescriptor {
    PinDescriptor::new(EXEC_OUT_PIN, "Exec", PinType::Exec)
}

fn exec(slug: &str, label: &str) -> PinDescriptor {
    PinDescriptor::new(slug, label, PinType::Exec)
}

fn pin(slug: &str, label: &str, ty: PinType) -> PinDescriptor {
    PinDescriptor::new(slug, label, ty)
}

fn f(slug: &str, label: &str, default: f32) -> PinDescriptor {
    PinDescriptor::new(slug, label, PinType::Float).with_default(PropValue::Float(default))
}

fn i(slug: &str, label: &str, default: i32) -> PinDescriptor {
    PinDescriptor::new(slug, label, PinType::Int).with_default(PropValue::Int(default))
}

fn b(slug: &str, label: &str, default: bool) -> PinDescriptor {
    PinDescriptor::new(slug, label, PinType::Bool).with_default(PropValue::Bool(default))
}

fn s(slug: &str, label: &str) -> PinDescriptor {
    PinDescriptor::new(slug, label, PinType::String).with_default(PropValue::Str(String::new()))
}

fn v3(slug: &str, label: &str) -> PinDescriptor {
    PinDescriptor::new(slug, label, PinType::Vec3).with_default(PropValue::Vec3([0.0; 3]))
}

/// An `entity` input. Unwired it evaluates to the instance's own entity —
/// `Value::zero_of(Entity)` is `self` — so "act on me" costs no wire and
/// "act on that one" is the same pin.
fn entity_in() -> PinDescriptor {
    PinDescriptor::new("entity", "Entity", PinType::Entity)
        .with_doc("Leave unconnected to act on this graph's own entity")
}

struct Desc {
    id: &'static str,
    name: &'static str,
    category: &'static str,
    doc: &'static str,
    pure: bool,
    deterministic: bool,
    inputs: Vec<PinDescriptor>,
    outputs: Vec<PinDescriptor>,
}

impl Desc {
    fn build(self) -> NodeDescriptor {
        NodeDescriptor {
            id: self.id.to_string(),
            name: self.name.to_string(),
            category: self.category.to_string(),
            version: 1,
            inputs: self.inputs,
            outputs: self.outputs,
            pure: self.pure,
            realm: NodeRealm::Shared,
            deterministic: self.deterministic,
            doc: Some(self.doc.to_string()),
            preview: None,
        }
    }
}

fn pure(
    id: &'static str,
    name: &'static str,
    category: &'static str,
    doc: &'static str,
    inputs: Vec<PinDescriptor>,
    outputs: Vec<PinDescriptor>,
) -> NodeDescriptor {
    Desc { id, name, category, doc, pure: true, deterministic: true, inputs, outputs }.build()
}

fn impure(
    id: &'static str,
    name: &'static str,
    category: &'static str,
    doc: &'static str,
    inputs: Vec<PinDescriptor>,
    outputs: Vec<PinDescriptor>,
) -> NodeDescriptor {
    Desc { id, name, category, doc, pure: false, deterministic: true, inputs, outputs }.build()
}

/// A pure node whose value is not a function of its inputs alone — instance
/// RNG, or a world read. Marked `deterministic: false`, which the interpreter
/// reads as **volatile**: exempt from statement-scoped memoization, so two
/// pulls in one statement genuinely produce two draws.
fn volatile(
    id: &'static str,
    name: &'static str,
    category: &'static str,
    doc: &'static str,
    inputs: Vec<PinDescriptor>,
    outputs: Vec<PinDescriptor>,
) -> NodeDescriptor {
    Desc { id, name, category, doc, pure: true, deterministic: false, inputs, outputs }.build()
}

/// Every standard node descriptor, in registration order.
pub fn std_node_descriptors() -> Vec<NodeDescriptor> {
    let mut v = Vec::new();
    v.extend(control());
    v.extend(logic_math());
    v.extend(data());
    v.extend(effects());
    v
}

fn control() -> Vec<NodeDescriptor> {
    let mut v = vec![
        impure(
            BRANCH,
            "Branch",
            "Flow",
            "Continues on True or False",
            vec![exec_in(), b("condition", "Condition", false)],
            vec![exec("true", "True"), exec("false", "False")],
        ),
        impure(
            SEQUENCE,
            "Sequence",
            "Flow",
            "Runs each connected output in order, each to completion",
            vec![exec_in()],
            SEQUENCE_PINS
                .iter()
                .enumerate()
                .map(|(n, slug)| exec(slug, &format!("Then {n}")))
                .collect(),
        ),
        impure(
            FOR_LOOP,
            "For Loop",
            "Flow",
            "Runs the body once per index from First to Last inclusive",
            vec![exec_in(), i("first", "First", 0), i("last", "Last", 0)],
            vec![
                exec("body", "Loop Body"),
                i("index", "Index", 0),
                exec("completed", "Completed"),
            ],
        ),
        impure(
            WHILE_LOOP,
            "While Loop",
            "Flow",
            "Runs the body while Condition is true, re-checking it each pass",
            vec![exec_in(), b("condition", "Condition", false)],
            vec![exec("body", "Loop Body"), exec("completed", "Completed")],
        ),
        impure(
            DELAY,
            "Delay",
            "Flow",
            "Waits Duration seconds, then continues — the activation suspends meanwhile",
            vec![exec_in(), f("duration", "Duration", 0.2)],
            vec![PinDescriptor::new(EXEC_OUT_PIN, "Completed", PinType::Exec)],
        ),
        impure(
            GATE,
            "Gate",
            "Flow",
            "Passes Enter through to Exit only while open",
            vec![
                exec("enter", "Enter"),
                exec("open", "Open"),
                exec("close", "Close"),
                exec("toggle", "Toggle"),
                b("start_closed", "Start Closed", false),
            ],
            vec![exec("exit", "Exit")],
        ),
        impure(
            DO_ONCE,
            "Do Once",
            "Flow",
            "Passes the first Enter through, then nothing until Reset",
            vec![
                exec("enter", "Enter"),
                exec("reset", "Reset"),
                b("start_closed", "Start Closed", false),
            ],
            vec![exec("completed", "Completed")],
        ),
        impure(
            FLIP_FLOP,
            "Flip Flop",
            "Flow",
            "Alternates between A and B on each entry",
            vec![exec_in()],
            vec![exec("a", "A"), exec("b", "B"), b("is_a", "Is A", false)],
        ),
    ];
    // ForEach, one variant per element type a v1 graph iterates.
    for (id, name, ty) in [
        (FOR_EACH_FLOAT, "For Each (Float)", PinType::Float),
        (FOR_EACH_INT, "For Each (Int)", PinType::Int),
        (FOR_EACH_ENTITY, "For Each (Entity)", PinType::Entity),
    ] {
        v.push(
            Desc {
                id,
                name,
                category: "Flow",
                doc: "Runs the body once per element of the array",
                pure: false,
                deterministic: true,
                inputs: vec![
                    exec_in(),
                    pin("array", "Array", PinType::Array(Box::new(ty.clone()))),
                ],
                outputs: vec![
                    exec("body", "Loop Body"),
                    pin("element", "Element", ty),
                    i("index", "Index", 0),
                    exec("completed", "Completed"),
                ],
            }
            .build(),
        );
    }
    // Select, one variant per picked type.
    for (id, name, ty, default) in [
        (SELECT_FLOAT, "Select (Float)", PinType::Float, PropValue::Float(0.0)),
        (SELECT_INT, "Select (Int)", PinType::Int, PropValue::Int(0)),
        (SELECT_BOOL, "Select (Bool)", PinType::Bool, PropValue::Bool(false)),
        (SELECT_STRING, "Select (String)", PinType::String, PropValue::Str(String::new())),
    ] {
        v.push(pure(
            id,
            name,
            "Flow",
            "Picks one of two values from a condition",
            vec![
                b("condition", "Condition", false),
                pin("if_true", "If True", ty.clone()).with_default(default.clone()),
                pin("if_false", "If False", ty.clone()).with_default(default),
            ],
            vec![pin("result", "Result", ty)],
        ));
    }
    v
}

fn logic_math() -> Vec<NodeDescriptor> {
    let mut v = vec![
        pure(
            AND,
            "And",
            "Logic",
            "True when both inputs are true",
            vec![b("a", "A", false), b("b", "B", false)],
            vec![b("result", "Result", false)],
        ),
        pure(
            OR,
            "Or",
            "Logic",
            "True when either input is true",
            vec![b("a", "A", false), b("b", "B", false)],
            vec![b("result", "Result", false)],
        ),
        pure(
            NOT,
            "Not",
            "Logic",
            "Inverts a boolean",
            vec![b("a", "A", false)],
            vec![b("result", "Result", false)],
        ),
        pure(
            LERP_FLOAT,
            "Lerp (Float)",
            "Math",
            "Blends A to B by Alpha, clamped to 0..1",
            vec![f("a", "A", 0.0), f("b", "B", 1.0), f("alpha", "Alpha", 0.0)],
            vec![f("result", "Result", 0.0)],
        ),
        pure(
            CLAMP_FLOAT,
            "Clamp (Float)",
            "Math",
            "Constrains a value to Min..Max",
            vec![f("value", "Value", 0.0), f("min", "Min", 0.0), f("max", "Max", 1.0)],
            vec![f("result", "Result", 0.0)],
        ),
        pure(
            CLAMP_INT,
            "Clamp (Int)",
            "Math",
            "Constrains a value to Min..Max",
            vec![i("value", "Value", 0), i("min", "Min", 0), i("max", "Max", 1)],
            vec![i("result", "Result", 0)],
        ),
        volatile(
            RANDOM_FLOAT,
            "Random Float",
            "Math",
            "A random value in Min..Max from the instance's seeded generator",
            vec![f("min", "Min", 0.0), f("max", "Max", 1.0)],
            vec![f("value", "Value", 0.0)],
        ),
        volatile(
            RANDOM_INT,
            "Random Int",
            "Math",
            "A random value in Min..Max inclusive, from the instance's seeded generator",
            vec![i("min", "Min", 0), i("max", "Max", 1)],
            vec![i("value", "Value", 0)],
        ),
    ];
    // Comparisons: two nodes, operator as a declared-variant Enum.
    for (id, name, ty, default) in [
        (COMPARE_INT, "Compare (Int)", PinType::Int, PropValue::Int(0)),
        (COMPARE_FLOAT, "Compare (Float)", PinType::Float, PropValue::Float(0.0)),
    ] {
        v.push(pure(
            id,
            name,
            "Logic",
            "Compares two values with the chosen operator",
            vec![
                pin("a", "A", ty.clone()).with_default(default.clone()),
                pin("b", "B", ty).with_default(default),
                PinDescriptor::new("op", "Operator", PinType::Enum)
                    .with_default(PropValue::Enum("equal".to_string()))
                    .with_variants(COMPARE_OPS)
                    .with_doc("equal, not_equal, less, less_equal, greater, greater_equal"),
            ],
            vec![b("result", "Result", false)],
        ));
    }
    // Arithmetic: one node per operation, per type. No polymorphism (D9).
    for (id, name, doc) in [
        (ADD_INT, "Add (Int)", "a + b"),
        (SUB_INT, "Subtract (Int)", "a - b"),
        (MUL_INT, "Multiply (Int)", "a * b"),
        (DIV_INT, "Divide (Int)", "a / b, and 0 when b is 0"),
    ] {
        v.push(pure(
            id,
            name,
            "Math",
            doc,
            vec![i("a", "A", 0), i("b", "B", if id == DIV_INT || id == MUL_INT { 1 } else { 0 })],
            vec![i("result", "Result", 0)],
        ));
    }
    for (id, name, doc) in [
        (ADD_FLOAT, "Add (Float)", "a + b"),
        (SUB_FLOAT, "Subtract (Float)", "a - b"),
        (MUL_FLOAT, "Multiply (Float)", "a * b"),
        (DIV_FLOAT, "Divide (Float)", "a / b, and 0 when b is 0"),
    ] {
        v.push(pure(
            id,
            name,
            "Math",
            doc,
            vec![
                f("a", "A", 0.0),
                f("b", "B", if id == DIV_FLOAT || id == MUL_FLOAT { 1.0 } else { 0.0 }),
            ],
            vec![f("result", "Result", 0.0)],
        ));
    }
    v
}

fn data() -> Vec<NodeDescriptor> {
    vec![
        pure(
            MAKE_VEC3,
            "Make Vec3",
            "Data",
            "Builds a vector from components — X forward, Y right, Z up",
            vec![f("x", "X", 0.0), f("y", "Y", 0.0), f("z", "Z", 0.0)],
            vec![v3("result", "Result")],
        ),
        pure(
            BREAK_VEC3,
            "Break Vec3",
            "Data",
            "Splits a vector into components — X forward, Y right, Z up",
            vec![v3("value", "Value")],
            vec![f("x", "X", 0.0), f("y", "Y", 0.0), f("z", "Z", 0.0)],
        ),
        pure(
            INT_TO_FLOAT,
            "To Float (Int)",
            "Data",
            "An Int as a Float",
            vec![i("value", "Value", 0)],
            vec![f("result", "Result", 0.0)],
        ),
        pure(
            FLOAT_TO_INT,
            "To Int (Float)",
            "Data",
            "A Float as an Int, truncated toward zero",
            vec![f("value", "Value", 0.0)],
            vec![i("result", "Result", 0)],
        ),
        pure(
            INT_TO_STRING,
            "To String (Int)",
            "Data",
            "An Int as text",
            vec![i("value", "Value", 0)],
            vec![s("text", "Text")],
        ),
        pure(
            FLOAT_TO_STRING,
            "To String (Float)",
            "Data",
            "A Float as text",
            vec![f("value", "Value", 0.0)],
            vec![s("text", "Text")],
        ),
        pure(
            BOOL_TO_STRING,
            "To String (Bool)",
            "Data",
            "A Bool as 'true' or 'false'",
            vec![b("value", "Value", false)],
            vec![s("text", "Text")],
        ),
    ]
}

fn effects() -> Vec<NodeDescriptor> {
    vec![
        impure(
            PRINT,
            "Print",
            "Data",
            "Writes a line to the console",
            vec![exec_in(), s("text", "Text")],
            vec![exec_out()],
        ),
        impure(
            EMIT_EVENT,
            "Emit Event",
            "Event",
            "Queues a custom event on this graph for the next tick",
            vec![exec_in(), s("name", "Name").with_doc("Must match a Custom Event node's name")],
            vec![exec_out()],
        ),
        impure(
            SPAWN_PREFAB,
            "Spawn Prefab",
            "Gameplay",
            "Spawns a prefab and hands back a handle to it",
            vec![
                exec_in(),
                // There is no `Asset` pin type; the path is a String and the
                // stored constant may be a `PropValue::Asset`, which is what
                // the editor's asset field writes.
                s("path", "Prefab").with_doc("Content-relative path to a .prefab"),
                v3("position", "Position"),
            ],
            vec![
                exec_out(),
                pin("spawned", "Spawned", PinType::Entity)
                    .with_doc("Usable immediately; bound to a real entity when the spawn applies"),
            ],
        ),
        impure(
            DESTROY_ENTITY,
            "Destroy Entity",
            "Gameplay",
            "Removes an entity from the world",
            vec![exec_in(), entity_in()],
            vec![exec_out()],
        ),
        pure(
            GET_SELF,
            "Get Self",
            "Gameplay",
            "The entity this graph is attached to",
            vec![],
            vec![pin("self", "Self", PinType::Entity)],
        ),
        volatile(
            GET_POSITION,
            "Get Position",
            "Spatial",
            "An entity's world position",
            vec![entity_in()],
            vec![v3("position", "Position")],
        ),
        impure(
            SET_POSITION,
            "Set Position",
            "Spatial",
            "Moves an entity",
            vec![exec_in(), entity_in(), v3("position", "Position")],
            vec![exec_out()],
        ),
        volatile(
            GET_TRANSFORM,
            "Get Transform",
            "Spatial",
            "An entity's world position, rotation and scale",
            vec![entity_in()],
            vec![
                v3("position", "Position"),
                PinDescriptor::new("rotation", "Rotation", PinType::Vec4)
                    .with_default(PropValue::Vec4([0.0, 0.0, 0.0, 1.0]))
                    .with_doc("Quaternion, xyzw"),
                v3("scale", "Scale"),
            ],
        ),
        impure(
            SET_TRANSFORM,
            "Set Transform",
            "Spatial",
            "Sets an entity's position, rotation and scale",
            vec![
                exec_in(),
                entity_in(),
                v3("position", "Position"),
                PinDescriptor::new("rotation", "Rotation", PinType::Vec4)
                    .with_default(PropValue::Vec4([0.0, 0.0, 0.0, 1.0]))
                    .with_doc("Quaternion, xyzw"),
                PinDescriptor::new("scale", "Scale", PinType::Vec3)
                    .with_default(PropValue::Vec3([1.0; 3])),
            ],
            vec![exec_out()],
        ),
    ]
}

/// Register the standard library directly. The engine reaches these through
/// the 45-A P5 plugin instead, which stages the same descriptors.
pub fn register_std_nodes(reg: &mut NodeRegistry) -> Result<(), RegistryError> {
    for d in std_node_descriptors() {
        reg.register(d)?;
    }
    Ok(())
}

#[cfg(test)]
// Tests build documents the way an author does: start from the default and
// fill in what matters. One giant struct literal per fixture would satisfy
// clippy and read markedly worse.
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// The library registers cleanly — which is a real assertion, because
    /// `register` enforces the descriptor invariants: unique slugs, no
    /// reserved ids, pure nodes without exec pins, impure nodes with them.
    #[test]
    fn the_library_registers_and_holds_its_invariants() {
        let mut reg = NodeRegistry::new();
        register_std_nodes(&mut reg).expect("the standard library must register cleanly");
        crate::std_events::register_std_events(&mut reg).unwrap();

        let all = std_node_descriptors();
        assert!(all.len() >= 40, "the D5 library is ~40 nodes, got {}", all.len());

        let mut ids = BTreeSet::new();
        for d in &all {
            assert!(ids.insert(d.id.clone()), "duplicate id '{}'", d.id);
            assert!(d.doc.is_some(), "'{}' has no doc line — the create menu reads it", d.id);
            assert!(!d.name.is_empty() && !d.category.is_empty(), "{}", d.id);
            // Slugs are forever: keep them boring.
            assert!(
                d.id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
                "'{}' is not a snake_case slug",
                d.id
            );
            for p in d.inputs.iter().chain(d.outputs.iter()) {
                assert!(
                    p.slug.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
                    "'{}' pin '{}' is not a snake_case slug",
                    d.id,
                    p.slug
                );
                assert!(!p.label.is_empty(), "'{}' pin '{}' has no label", d.id, p.slug);
            }
        }
    }

    /// The two comparison nodes declare their operators, so the inline editor
    /// renders a dropdown rather than a free-text chip — the reason twelve
    /// comparison nodes collapse into two.
    #[test]
    fn comparison_operators_are_declared_variants() {
        for id in [COMPARE_INT, COMPARE_FLOAT] {
            let d = std_node_descriptors().into_iter().find(|d| d.id == id).unwrap();
            let op = d.input("op").expect("an operator pin");
            assert_eq!(op.variants, COMPARE_OPS.map(String::from).to_vec());
            assert!(op.accepts_variant("less_equal"));
            assert!(!op.accepts_variant("approximately"));
            assert_eq!(op.default, Some(PropValue::Enum("equal".into())));
        }
    }

    /// Volatile nodes are the ones whose value is not a function of their
    /// inputs: instance RNG and world reads. Everything else is
    /// `deterministic`, which is what lets the interpreter memoize it within
    /// a statement.
    #[test]
    fn only_rng_and_world_reads_are_volatile() {
        let volatile: Vec<String> = std_node_descriptors()
            .into_iter()
            .filter(|d| !d.deterministic)
            .map(|d| d.id)
            .collect();
        assert_eq!(
            volatile,
            vec![RANDOM_FLOAT, RANDOM_INT, GET_POSITION, GET_TRANSFORM],
            "adding a volatile node is a decision, not an accident"
        );
    }
}
