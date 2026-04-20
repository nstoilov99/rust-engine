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
                    other => Err(de::Error::unknown_variant(
                        other,
                        &["Perspective", "Orthographic"],
                    )),
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
                        _ => {
                            let _ = map.next_value::<serde::de::IgnoredAny>()?;
                        }
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
                    other => Err(E::unknown_variant(
                        other,
                        &["Linear", "Quadratic", "InverseSquare"],
                    )),
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
                    other => Err(E::unknown_variant(
                        other,
                        &["Dynamic", "Kinematic", "Static"],
                    )),
                }
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok(RigidBodyTypeData::default())
            }
        }
        deserializer.deserialize_any(Visitor)
    }
}

/// Collider shape for scene serialization.
///
/// Custom serde keeps existing saved scenes loadable while writing an explicit
/// `type` field for new files.
#[derive(Debug, Clone)]
pub enum ColliderShapeData {
    Cuboid { half_extents: [f32; 3] },
    Ball { radius: f32 },
    Capsule { half_height: f32, radius: f32 },
}

impl Serialize for ColliderShapeData {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;

        let mut map = serializer.serialize_map(None)?;
        match self {
            ColliderShapeData::Cuboid { half_extents } => {
                map.serialize_entry("type", "Cuboid")?;
                map.serialize_entry("half_extents", half_extents)?;
            }
            ColliderShapeData::Ball { radius } => {
                map.serialize_entry("type", "Ball")?;
                map.serialize_entry("radius", radius)?;
            }
            ColliderShapeData::Capsule {
                half_height,
                radius,
            } => {
                map.serialize_entry("type", "Capsule")?;
                map.serialize_entry("half_height", half_height)?;
                map.serialize_entry("radius", radius)?;
            }
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for ColliderShapeData {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use ron::Value;
        use serde::de::Error;

        let value = Value::deserialize(deserializer)?;
        match value {
            Value::Map(mut map) => {
                if let Some(kind) = take_string_field(&mut map, "type")? {
                    return collider_shape_from_fields(kind.as_str(), map);
                }

                if let Some((kind, inner_map)) = take_variant_map(&mut map)? {
                    return collider_shape_from_fields(kind.as_str(), inner_map);
                }

                let inferred_kind = if has_field(&map, "half_extents") {
                    "Cuboid"
                } else if has_field(&map, "half_height") {
                    "Capsule"
                } else if has_field(&map, "radius") {
                    "Ball"
                } else {
                    return Err(Error::custom("unrecognized collider shape"));
                };

                collider_shape_from_fields(inferred_kind, map)
            }
            _ => Err(Error::custom("expected collider shape map")),
        }
    }
}

fn collider_shape_from_fields<E: serde::de::Error>(
    kind: &str,
    mut fields: ron::Map,
) -> Result<ColliderShapeData, E> {
    match kind {
        "Cuboid" => Ok(ColliderShapeData::Cuboid {
            half_extents: take_required_field(&mut fields, "half_extents")?,
        }),
        "Ball" => Ok(ColliderShapeData::Ball {
            radius: take_required_field(&mut fields, "radius")?,
        }),
        "Capsule" => Ok(ColliderShapeData::Capsule {
            half_height: take_required_field(&mut fields, "half_height")?,
            radius: take_required_field(&mut fields, "radius")?,
        }),
        other => Err(E::unknown_variant(other, &["Cuboid", "Ball", "Capsule"])),
    }
}

fn take_variant_map<E: serde::de::Error>(
    map: &mut ron::Map,
) -> Result<Option<(String, ron::Map)>, E> {
    if map.len() != 1 {
        return Ok(None);
    }

    let Some((key, _value)) = map.iter().next() else {
        return Ok(None);
    };

    let ron::Value::String(kind) = key else {
        return Ok(None);
    };
    if !matches!(kind.as_str(), "Cuboid" | "Ball" | "Capsule") {
        return Ok(None);
    }

    let kind = kind.clone();
    let value = map
        .remove(&ron::Value::String(kind.clone()))
        .ok_or_else(|| E::custom("missing collider variant payload"))?;
    let ron::Value::Map(inner_map) = value else {
        return Err(E::custom("expected collider variant field map"));
    };

    Ok(Some((kind, inner_map)))
}

fn take_string_field<E: serde::de::Error>(
    fields: &mut ron::Map,
    name: &str,
) -> Result<Option<String>, E> {
    let Some(value) = fields.remove(&ron::Value::String(name.to_string())) else {
        return Ok(None);
    };
    value
        .into_rust::<String>()
        .map(Some)
        .map_err(|error| E::custom(error.to_string()))
}

fn take_required_field<T, E: serde::de::Error>(fields: &mut ron::Map, name: &str) -> Result<T, E>
where
    T: serde::de::DeserializeOwned,
{
    let value = fields
        .remove(&ron::Value::String(name.to_string()))
        .ok_or_else(|| E::custom(format!("missing field '{name}'")))?;
    let value = match value {
        ron::Value::Option(Some(inner)) => *inner,
        other => other,
    };
    value
        .into_rust::<T>()
        .map_err(|error| E::custom(error.to_string()))
}

fn has_field(fields: &ron::Map, name: &str) -> bool {
    fields
        .iter()
        .any(|(key, _)| matches!(key, ron::Value::String(value) if value == name))
}

fn default_true() -> bool {
    true
}

fn default_base_color_factor() -> [f32; 4] {
    [1.0, 1.0, 1.0, 1.0]
}

fn default_one() -> f32 {
    1.0
}

fn default_half() -> f32 {
    0.5
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

fn default_audio_bus() -> String {
    "SFX".to_string()
}

fn default_pitch() -> f32 {
    1.0
}

fn default_max_distance() -> f32 {
    50.0
}

fn default_plankton_capacity() -> u32 { 2048 }
fn default_plankton_emission_rate() -> f32 { 20.0 }
fn default_plankton_lifetime_min() -> f32 { 1.0 }
fn default_plankton_lifetime_max() -> f32 { 2.0 }
fn default_plankton_initial_velocity() -> [f32; 3] { [0.0, 0.0, 2.0] }

/// Spawn shape for scene serialization.
/// Custom serde impl for the same RON internally-tagged enum reason as CameraProjectionData.
#[derive(Debug, Clone, Copy)]
pub enum SpawnShapeData {
    Point,
    Sphere { radius: f32 },
    Cone { angle_rad: f32, radius: f32 },
    Box { half_extents: [f32; 3] },
}

impl Default for SpawnShapeData {
    fn default() -> Self {
        Self::Point
    }
}

impl Serialize for SpawnShapeData {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        match self {
            SpawnShapeData::Point => serializer.serialize_str("Point"),
            SpawnShapeData::Sphere { radius } => {
                let mut map = serializer.serialize_map(Some(2))?;
                map.serialize_entry("shape", "Sphere")?;
                map.serialize_entry("radius", radius)?;
                map.end()
            }
            SpawnShapeData::Cone { angle_rad, radius } => {
                let mut map = serializer.serialize_map(Some(3))?;
                map.serialize_entry("shape", "Cone")?;
                map.serialize_entry("angle_rad", angle_rad)?;
                map.serialize_entry("radius", radius)?;
                map.end()
            }
            SpawnShapeData::Box { half_extents } => {
                let mut map = serializer.serialize_map(Some(2))?;
                map.serialize_entry("shape", "Box")?;
                map.serialize_entry("half_extents", half_extents)?;
                map.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for SpawnShapeData {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de;

        struct Visitor;
        impl<'de> de::Visitor<'de> for Visitor {
            type Value = SpawnShapeData;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("\"Point\" or a map with shape-specific params")
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                match v {
                    "Point" => Ok(SpawnShapeData::Point),
                    other => Err(de::Error::unknown_variant(
                        other,
                        &["Point", "Sphere", "Cone", "Box"],
                    )),
                }
            }

            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(SpawnShapeData::Point)
            }

            fn visit_map<A: de::MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut shape: Option<String> = None;
                let mut radius: Option<f32> = None;
                let mut angle_rad: Option<f32> = None;
                let mut half_extents: Option<[f32; 3]> = None;
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "shape" => shape = Some(map.next_value()?),
                        "radius" => radius = Some(map.next_value()?),
                        "angle_rad" => angle_rad = Some(map.next_value()?),
                        "half_extents" => half_extents = Some(map.next_value()?),
                        _ => { let _ = map.next_value::<de::IgnoredAny>()?; }
                    }
                }
                let kind = shape.as_deref().unwrap_or("Point");
                match kind {
                    "Point" => Ok(SpawnShapeData::Point),
                    "Sphere" => Ok(SpawnShapeData::Sphere {
                        radius: radius.unwrap_or(1.0),
                    }),
                    "Cone" => Ok(SpawnShapeData::Cone {
                        angle_rad: angle_rad.unwrap_or(0.5),
                        radius: radius.unwrap_or(0.5),
                    }),
                    "Box" => Ok(SpawnShapeData::Box {
                        half_extents: half_extents.unwrap_or([0.5, 0.5, 0.5]),
                    }),
                    other => Err(de::Error::unknown_variant(
                        other,
                        &["Point", "Sphere", "Cone", "Box"],
                    )),
                }
            }
        }

        deserializer.deserialize_any(Visitor)
    }
}

/// Composable update module for scene serialization.
/// Uses the same custom serde pattern as other scene enums.
#[derive(Debug, Clone, Copy)]
pub enum UpdateModuleData {
    Gravity([f32; 3]),
    Drag(f32),
    Wind([f32; 3]),
    CurlNoise { strength: f32, scale: f32, speed: f32 },
    ColorOverLife { start: [f32; 4], end: [f32; 4] },
    SizeOverLife { start: f32, end: f32 },
}

impl Serialize for UpdateModuleData {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(None)?;
        match self {
            UpdateModuleData::Gravity(v) => {
                map.serialize_entry("module", "Gravity")?;
                map.serialize_entry("value", v)?;
            }
            UpdateModuleData::Drag(v) => {
                map.serialize_entry("module", "Drag")?;
                map.serialize_entry("value", v)?;
            }
            UpdateModuleData::Wind(v) => {
                map.serialize_entry("module", "Wind")?;
                map.serialize_entry("value", v)?;
            }
            UpdateModuleData::CurlNoise { strength, scale, speed } => {
                map.serialize_entry("module", "CurlNoise")?;
                map.serialize_entry("strength", strength)?;
                map.serialize_entry("scale", scale)?;
                map.serialize_entry("speed", speed)?;
            }
            UpdateModuleData::ColorOverLife { start, end } => {
                map.serialize_entry("module", "ColorOverLife")?;
                map.serialize_entry("start", start)?;
                map.serialize_entry("end", end)?;
            }
            UpdateModuleData::SizeOverLife { start, end } => {
                map.serialize_entry("module", "SizeOverLife")?;
                map.serialize_entry("start", start)?;
                map.serialize_entry("end", end)?;
            }
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for UpdateModuleData {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de;

        struct Visitor;
        impl<'de> de::Visitor<'de> for Visitor {
            type Value = UpdateModuleData;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a map with a 'module' field")
            }

            fn visit_map<A: de::MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut module: Option<String> = None;
                let mut value_f32: Option<f32> = None;
                let mut value_vec3: Option<[f32; 3]> = None;
                let mut strength: Option<f32> = None;
                let mut scale: Option<f32> = None;
                let mut speed: Option<f32> = None;
                let mut start_f32: Option<f32> = None;
                let mut end_f32: Option<f32> = None;
                let mut start_color: Option<[f32; 4]> = None;
                let mut end_color: Option<[f32; 4]> = None;

                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "module" => module = Some(map.next_value()?),
                        "value" => {
                            // Could be f32 or [f32; 3] — try deserializing as ron::Value
                            let v: ron::Value = map.next_value()?;
                            if let Ok(f) = v.clone().into_rust::<f32>() {
                                value_f32 = Some(f);
                            } else if let Ok(arr) = v.into_rust::<[f32; 3]>() {
                                value_vec3 = Some(arr);
                            }
                        }
                        "strength" => strength = Some(map.next_value()?),
                        "scale" => scale = Some(map.next_value()?),
                        "speed" => speed = Some(map.next_value()?),
                        "start" => {
                            let v: ron::Value = map.next_value()?;
                            if let Ok(f) = v.clone().into_rust::<f32>() {
                                start_f32 = Some(f);
                            } else if let Ok(arr) = v.into_rust::<[f32; 4]>() {
                                start_color = Some(arr);
                            }
                        }
                        "end" => {
                            let v: ron::Value = map.next_value()?;
                            if let Ok(f) = v.clone().into_rust::<f32>() {
                                end_f32 = Some(f);
                            } else if let Ok(arr) = v.into_rust::<[f32; 4]>() {
                                end_color = Some(arr);
                            }
                        }
                        _ => { let _ = map.next_value::<de::IgnoredAny>()?; }
                    }
                }
                let kind = module.as_deref().ok_or_else(|| de::Error::missing_field("module"))?;
                match kind {
                    "Gravity" => Ok(UpdateModuleData::Gravity(
                        value_vec3.unwrap_or([0.0, 0.0, -9.8]),
                    )),
                    "Drag" => Ok(UpdateModuleData::Drag(value_f32.unwrap_or(0.0))),
                    "Wind" => Ok(UpdateModuleData::Wind(
                        value_vec3.unwrap_or([0.0, 0.0, 0.0]),
                    )),
                    "CurlNoise" => Ok(UpdateModuleData::CurlNoise {
                        strength: strength.unwrap_or(1.0),
                        scale: scale.unwrap_or(1.0),
                        speed: speed.unwrap_or(0.0),
                    }),
                    "ColorOverLife" => Ok(UpdateModuleData::ColorOverLife {
                        start: start_color.unwrap_or([1.0, 1.0, 1.0, 1.0]),
                        end: end_color.unwrap_or([1.0, 1.0, 1.0, 0.0]),
                    }),
                    "SizeOverLife" => Ok(UpdateModuleData::SizeOverLife {
                        start: start_f32.unwrap_or(0.1),
                        end: end_f32.unwrap_or(0.0),
                    }),
                    other => Err(de::Error::unknown_variant(
                        other,
                        &["Gravity", "Drag", "Wind", "CurlNoise", "ColorOverLife", "SizeOverLife"],
                    )),
                }
            }
        }

        deserializer.deserialize_any(Visitor)
    }
}

/// Render mode for scene serialization.
#[derive(Debug, Clone, Copy, Default)]
pub enum RenderModeData {
    #[default]
    Billboard,
}

impl Serialize for RenderModeData {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            RenderModeData::Billboard => serializer.serialize_str("Billboard"),
        }
    }
}

impl<'de> Deserialize<'de> for RenderModeData {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = RenderModeData;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("\"Billboard\"")
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                match v {
                    "Billboard" => Ok(RenderModeData::Billboard),
                    other => Err(E::unknown_variant(other, &["Billboard"])),
                }
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok(RenderModeData::Billboard)
            }
        }
        deserializer.deserialize_any(Visitor)
    }
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
        #[serde(default)]
        mesh_path: String,
        #[serde(default)]
        material_paths: Vec<String>,
        /// Backward-compat: old single material_path (migrated to material_paths[0] on load)
        #[serde(default, skip_serializing)]
        material_path: String,
        /// Kept for backward compat with old scenes (ignored if mesh_path is set)
        #[serde(default)]
        mesh_index: usize,
        #[serde(default)]
        material_index: usize,
        #[serde(default = "default_true")]
        visible: bool,
        #[serde(default = "default_true")]
        cast_shadows: bool,
        #[serde(default = "default_true")]
        receive_shadows: bool,
        #[serde(default = "default_base_color_factor")]
        base_color_factor: [f32; 4],
        #[serde(default = "default_one")]
        metallic_factor: f32,
        #[serde(default = "default_half")]
        roughness_factor: f32,
        #[serde(default)]
        emissive_factor: [f32; 3],
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
    AudioEmitter {
        #[serde(default)]
        clip_path: String,
        #[serde(default = "default_audio_bus")]
        bus: String,
        #[serde(default)]
        volume_db: f32,
        #[serde(default = "default_pitch")]
        pitch: f32,
        #[serde(default)]
        looping: bool,
        #[serde(default)]
        auto_play: bool,
        #[serde(default)]
        spatial: bool,
        #[serde(default = "default_max_distance")]
        max_distance: f32,
        #[serde(default = "default_true")]
        hide_range_in_game: bool,
    },
    AudioListener {
        #[serde(default = "default_true")]
        active: bool,
    },
    ParticleEffect {
        #[serde(default = "default_true")]
        enabled: bool,
        #[serde(default = "default_plankton_capacity")]
        capacity: u32,
        #[serde(default = "default_plankton_emission_rate")]
        emission_rate: f32,
        #[serde(default)]
        burst_count: u32,
        #[serde(default)]
        burst_interval: f32,
        #[serde(default)]
        spawn_shape: SpawnShapeData,
        #[serde(default = "default_plankton_lifetime_min")]
        lifetime_min: f32,
        #[serde(default = "default_plankton_lifetime_max")]
        lifetime_max: f32,
        #[serde(default = "default_plankton_initial_velocity")]
        initial_velocity: [f32; 3],
        #[serde(default)]
        velocity_variance: f32,
        #[serde(default)]
        update_modules: Vec<UpdateModuleData>,
        #[serde(default)]
        render_mode: RenderModeData,
        #[serde(default)]
        texture_path: String,
        #[serde(default)]
        soft_fade_distance: f32,
        #[serde(default)]
        show_gizmos: bool,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collider_shape_loads_legacy_external_variant() {
        let shape: ColliderShapeData =
            ron::from_str("Cuboid(half_extents: (1.0, 2.0, 3.0))").unwrap();

        match shape {
            ColliderShapeData::Cuboid { half_extents } => {
                assert_eq!(half_extents, [1.0, 2.0, 3.0]);
            }
            _ => panic!("expected cuboid shape"),
        }
    }

    #[test]
    fn collider_component_roundtrips_with_explicit_type_map() {
        let component = ComponentData::Collider {
            shape: ColliderShapeData::Capsule {
                half_height: 1.5,
                radius: 0.75,
            },
            friction: 0.5,
            restitution: 0.25,
            is_sensor: false,
        };

        let ron_string = ron::ser::to_string(&component).unwrap();
        assert!(ron_string.contains("Capsule"));

        let decoded: ComponentData = ron::from_str(&ron_string).unwrap();
        match decoded {
            ComponentData::Collider {
                shape:
                    ColliderShapeData::Capsule {
                        half_height,
                        radius,
                    },
                ..
            } => {
                assert!((half_height - 1.5).abs() < 0.001);
                assert!((radius - 0.75).abs() < 0.001);
            }
            _ => panic!("expected capsule collider"),
        }
    }
}
