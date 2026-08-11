//! Runtime values.
//!
//! Distinct from [`PropValue`] on purpose. `PropValue` is the *stored
//! constant* form — what an unconnected input pin holds in the asset — and it
//! deliberately has no `Entity` variant, because an entity has no constant
//! spelling. A running graph does hold entities, so the runtime type has one;
//! it holds an **instance-local alias**, never a real entity id (D1: real ids
//! never enter the portable core).

use node_graph_types::{PinType, PropValue};
use serde::{Deserialize, Serialize};

/// An entity as the portable core is allowed to see it.
///
/// `Spawned` is an alias handed out by a spawn effect; the runner resolves it
/// to a real entity when the spawn actually happens and writes the mapping
/// back into the instance. The core never learns the real id, so a replay on
/// another machine — or in the SpacetimeDB module — produces the same
/// effect stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum EntityRef {
    /// The entity this graph instance is attached to.
    SelfEntity,
    /// An entity this instance spawned, by alias.
    Spawned(u32),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Value {
    /// No value: an exec pin, or an output a node chose not to write.
    Unit,
    Bool(bool),
    Int(i32),
    Float(f32),
    Str(String),
    /// Enum variant slug.
    Enum(String),
    /// Content-relative asset path.
    Asset(String),
    Vec2([f32; 2]),
    Vec3([f32; 3]),
    Vec4([f32; 4]),
    Color([f32; 4]),
    Entity(EntityRef),
    Array(Vec<Value>),
}

impl Value {
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_int(&self) -> Option<i32> {
        match self {
            Value::Int(i) => Some(*i),
            _ => None,
        }
    }

    pub fn as_float(&self) -> Option<f32> {
        match self {
            Value::Float(f) => Some(*f),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) | Value::Enum(s) | Value::Asset(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn as_vec3(&self) -> Option<[f32; 3]> {
        match self {
            Value::Vec3(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_entity(&self) -> Option<EntityRef> {
        match self {
            Value::Entity(e) => Some(*e),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(v) => Some(v.as_slice()),
            _ => None,
        }
    }

    /// The zero of a pin type — what an input with no constant and no wire
    /// evaluates to, and what a `VarDecl` with no declared default starts at.
    /// Stated here rather than in the schema, because "the type's zero" is a
    /// runtime question (45-A P1 ruling on `VarDecl::default`).
    pub fn zero_of(ty: &PinType) -> Value {
        match ty {
            PinType::Exec => Value::Unit,
            PinType::Bool => Value::Bool(false),
            PinType::Int => Value::Int(0),
            PinType::Float => Value::Float(0.0),
            PinType::String => Value::Str(String::new()),
            PinType::Enum => Value::Enum(String::new()),
            PinType::Vec2 => Value::Vec2([0.0; 2]),
            PinType::Vec3 => Value::Vec3([0.0; 3]),
            PinType::Vec4 => Value::Vec4([0.0; 4]),
            PinType::Color => Value::Color([0.0, 0.0, 0.0, 1.0]),
            PinType::Entity => Value::Entity(EntityRef::SelfEntity),
            PinType::Array(_) => Value::Array(Vec::new()),
            // A texture/mesh/domain pin with no wire has nothing to stand in
            // for: `Unit` says "absent", which is honest.
            PinType::Texture | PinType::Mesh | PinType::Domain(_) => Value::Unit,
        }
    }

    /// Lift a stored constant into a runtime value. `Raw` — forward-compat
    /// data this build does not understand — becomes `Unit` rather than a
    /// guess; it still round-trips in the document untouched.
    pub fn from_prop(p: &PropValue) -> Value {
        match p {
            PropValue::Float(f) => Value::Float(*f),
            PropValue::Int(i) => Value::Int(*i),
            PropValue::Vec2(v) => Value::Vec2(*v),
            PropValue::Vec3(v) => Value::Vec3(*v),
            PropValue::Vec4(v) => Value::Vec4(*v),
            PropValue::Color(c) => Value::Color(*c),
            PropValue::Bool(b) => Value::Bool(*b),
            PropValue::Enum(s) => Value::Enum(s.clone()),
            PropValue::Str(s) => Value::Str(s.clone()),
            PropValue::Asset(s) => Value::Asset(s.clone()),
            PropValue::Array(v) => Value::Array(v.iter().map(Value::from_prop).collect()),
            PropValue::Raw(_) => Value::Unit,
        }
    }
}

impl std::fmt::Display for Value {
    /// The spelling `Print` puts in the console. Deliberately terse and
    /// stable: it is part of the effect stream the determinism test compares.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        fn nums(f: &mut std::fmt::Formatter<'_>, v: &[f32]) -> std::fmt::Result {
            write!(f, "(")?;
            for (i, x) in v.iter().enumerate() {
                if i > 0 {
                    write!(f, ", ")?;
                }
                write!(f, "{x}")?;
            }
            write!(f, ")")
        }
        match self {
            Value::Unit => write!(f, "-"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Int(i) => write!(f, "{i}"),
            Value::Float(x) => write!(f, "{x}"),
            Value::Str(s) | Value::Enum(s) | Value::Asset(s) => write!(f, "{s}"),
            Value::Vec2(v) => nums(f, v),
            Value::Vec3(v) => nums(f, v),
            Value::Vec4(v) | Value::Color(v) => nums(f, v),
            Value::Entity(EntityRef::SelfEntity) => write!(f, "self"),
            Value::Entity(EntityRef::Spawned(a)) => write!(f, "#{a}"),
            Value::Array(v) => {
                write!(f, "[")?;
                for (i, x) in v.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{x}")?;
                }
                write!(f, "]")
            }
        }
    }
}
