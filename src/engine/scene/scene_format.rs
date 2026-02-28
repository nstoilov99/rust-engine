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

/// Camera projection for scene serialization
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub enum CameraProjectionData {
    #[default]
    Perspective,
    Orthographic {
        size: f32,
    },
}

/// Light falloff model for scene serialization
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub enum LightFalloffData {
    Linear,
    #[default]
    Quadratic,
    InverseSquare,
}

/// Rigidbody type for scene serialization
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub enum RigidBodyTypeData {
    #[default]
    Dynamic,
    Kinematic,
    Static,
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
