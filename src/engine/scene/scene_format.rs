//! Scene file format structures for serialization
use serde::{Deserialize, Serialize};

/// Top-level scene file structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneFile {
    pub version: String,
    pub name: String,
    pub entities: Vec<EntityData>,
}

/// Entity data for serialization
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityData {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guid: Option<String>,
    pub components: Vec<ComponentData>,
}

/// Camera projection for scene serialization.
///
/// Uses a custom serde implementation to work around RON's handling of
/// unit enum variants inside internally-tagged (`#[serde(tag = "type")]`)
/// structs. RON serializes `Perspective` as a bare identifier, but serde's
/// internal tagging expects a string — causing "Expected string or map but
/// found a unit value" on load.
#[derive(Debug, Clone, Copy, Default)]
pub enum CameraProjectionData {
    #[default]
    Perspective,
    Orthographic {
        size: f32,
    },
}

impl Serialize for CameraProjectionData {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            CameraProjectionData::Perspective => serializer.serialize_str("Perspective"),
            CameraProjectionData::Orthographic { size } => {
                use serde::ser::SerializeMap;
                let mut map = serializer.serialize_map(Some(2))?;
                map.serialize_entry("type", "Orthographic")?;
                map.serialize_entry("size", size)?;
                map.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for CameraProjectionData {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de;

        struct Visitor;
        impl<'de> de::Visitor<'de> for Visitor {
            type Value = CameraProjectionData;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("\"Perspective\" or a map with \"size\" for Orthographic")
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                match v {
                    "Perspective" => Ok(CameraProjectionData::Perspective),
                    other => Err(de::Error::unknown_variant(other, &["Perspective", "Orthographic"])),
                }
            }

            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(CameraProjectionData::Perspective)
            }

            fn visit_map<A: de::MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut size: Option<f32> = None;
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "size" => size = Some(map.next_value()?),
                        _ => { let _ = map.next_value::<serde::de::IgnoredAny>()?; }
                    }
                }
                let size = size.ok_or_else(|| de::Error::missing_field("size"))?;
                Ok(CameraProjectionData::Orthographic { size })
            }
        }

        deserializer.deserialize_any(Visitor)
    }
}

/// Light falloff model for scene serialization.
/// Custom serde impl for the same RON internally-tagged enum reason as CameraProjectionData.
#[derive(Debug, Clone, Copy, Default)]
pub enum LightFalloffData {
    Linear,
    #[default]
    Quadratic,
    InverseSquare,
}

impl Serialize for LightFalloffData {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            LightFalloffData::Linear => serializer.serialize_str("Linear"),
            LightFalloffData::Quadratic => serializer.serialize_str("Quadratic"),
            LightFalloffData::InverseSquare => serializer.serialize_str("InverseSquare"),
        }
    }
}

impl<'de> Deserialize<'de> for LightFalloffData {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = LightFalloffData;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("\"Linear\", \"Quadratic\", or \"InverseSquare\"")
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                match v {
                    "Linear" => Ok(LightFalloffData::Linear),
                    "Quadratic" => Ok(LightFalloffData::Quadratic),
                    "InverseSquare" => Ok(LightFalloffData::InverseSquare),
                    other => Err(E::unknown_variant(other, &["Linear", "Quadratic", "InverseSquare"])),
                }
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok(LightFalloffData::default())
            }
        }
        deserializer.deserialize_any(Visitor)
    }
}

/// Rigidbody type for scene serialization.
/// Custom serde impl for the same RON internally-tagged enum reason as CameraProjectionData.
#[derive(Debug, Clone, Copy, Default)]
pub enum RigidBodyTypeData {
    #[default]
    Dynamic,
    Kinematic,
    Static,
}

impl Serialize for RigidBodyTypeData {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            RigidBodyTypeData::Dynamic => serializer.serialize_str("Dynamic"),
            RigidBodyTypeData::Kinematic => serializer.serialize_str("Kinematic"),
            RigidBodyTypeData::Static => serializer.serialize_str("Static"),
        }
    }
}

impl<'de> Deserialize<'de> for RigidBodyTypeData {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = RigidBodyTypeData;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("\"Dynamic\", \"Kinematic\", or \"Static\"")
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                match v {
                    "Dynamic" => Ok(RigidBodyTypeData::Dynamic),
                    "Kinematic" => Ok(RigidBodyTypeData::Kinematic),
                    "Static" => Ok(RigidBodyTypeData::Static),
                    other => Err(E::unknown_variant(other, &["Dynamic", "Kinematic", "Static"])),
                }
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok(RigidBodyTypeData::default())
            }
        }
        deserializer.deserialize_any(Visitor)
    }
}

/// Collider shape for scene serialization
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ColliderShapeData {
    Cuboid { half_extents: [f32; 3] },
    Ball { radius: f32 },
    Capsule { half_height: f32, radius: f32 },
}

fn default_true() -> bool {
    true
}

fn default_clear_color() -> [f32; 3] {
    [0.1, 0.1, 0.15]
}

fn default_shadow_bias() -> f32 {
    0.005
}

fn default_mass() -> f32 {
    1.0
}

fn default_gravity_scale() -> f32 {
    1.0
}

fn default_friction() -> f32 {
    0.5
}

fn default_restitution() -> f32 {
    0.3
}

/// Component data enum for all component types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ComponentData {
    Transform {
        position: [f32; 3],
        rotation: [f32; 4], // quaternion [x, y, z, w]
        scale: [f32; 3],
    },
    MeshRenderer {
        mesh_index: usize,
        material_index: usize,
        #[serde(default = "default_true")]
        visible: bool,
        #[serde(default = "default_true")]
        cast_shadows: bool,
        #[serde(default = "default_true")]
        receive_shadows: bool,
    },
    Camera {
        fov: f32,
        near: f32,
        far: f32,
        active: bool,
        #[serde(default)]
        projection: CameraProjectionData,
        #[serde(default = "default_clear_color")]
        clear_color: [f32; 3],
        #[serde(default)]
        priority: i32,
    },
    DirectionalLight {
        direction: [f32; 3],
        color: [f32; 3],
        intensity: f32,
        #[serde(default)]
        shadow_enabled: bool,
        #[serde(default = "default_shadow_bias")]
        shadow_bias: f32,
    },
    PointLight {
        color: [f32; 3],
        intensity: f32,
        radius: f32,
        #[serde(default)]
        shadow_enabled: bool,
        #[serde(default)]
        falloff: LightFalloffData,
    },
    RigidBody {
        #[serde(default)]
        body_type: RigidBodyTypeData,
        #[serde(default = "default_mass")]
        mass: f32,
        #[serde(default)]
        linear_damping: f32,
        #[serde(default)]
        angular_damping: f32,
        #[serde(default = "default_true")]
        can_sleep: bool,
        #[serde(default = "default_gravity_scale")]
        gravity_scale: f32,
        #[serde(default)]
        lock_rotation: [bool; 3],
        #[serde(default)]
        continuous_collision: bool,
    },
    Collider {
        shape: ColliderShapeData,
        #[serde(default = "default_friction")]
        friction: f32,
        #[serde(default = "default_restitution")]
        restitution: f32,
        #[serde(default)]
        is_sensor: bool,
    },
    Player,
    Parent {
        parent_name: String, // Reference parent entity by name
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_guid: Option<String>, // Reference parent entity by GUID (preferred)
    },
}

impl Default for SceneFile {
    fn default() -> Self {
        Self {
            version: "1.0".to_string(),
            name: "Untitled Scene".to_string(),
            entities: Vec::new(),
        }
    }
}
