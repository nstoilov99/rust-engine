//! Core ECS components
use nalgebra_glm as glm;
use serde::{Deserialize, Serialize};

/// 3D transform component
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Transform {
    #[serde(with = "vec3_serde")]
    pub position: glm::Vec3,
    #[serde(with = "quat_serde")]
    pub rotation: glm::Quat,
    #[serde(with = "vec3_serde")]
    pub scale: glm::Vec3,
}

impl Transform {
    pub fn new(position: glm::Vec3) -> Self {
        Self {
            position,
            rotation: glm::quat_identity(),
            scale: glm::vec3(1.0, 1.0, 1.0),
        }
    }

    pub fn with_rotation(mut self, rotation: glm::Quat) -> Self {
        self.rotation = rotation;
        self
    }

    pub fn with_scale(mut self, scale: glm::Vec3) -> Self {
        self.scale = scale;
        self
    }

    /// Build the local transform matrix in Z-up space (no coordinate conversion).
    ///
    /// This composes Translation * Rotation * Scale in the game's native Z-up
    /// coordinate system. Use this for:
    /// - Hierarchy composition (parent * child matrices)
    /// - Physics calculations
    /// - Any game logic that needs world-space matrices
    ///
    /// For rendering, convert the result using `render_adapter::world_matrix_to_render()`.
    pub fn local_matrix_zup(&self) -> glm::Mat4 {
        let translation = glm::translation(&self.position);
        let rotation = glm::quat_to_mat4(&self.rotation);
        let scale = glm::scaling(&self.scale);
        translation * rotation * scale
    }

    /// Calculate the model matrix for this transform, converted to render space (Y-up).
    ///
    /// **Note**: This returns only the LOCAL transform. For entities with parents,
    /// use `hierarchy::get_world_transform()` followed by `render_adapter::world_matrix_to_render()`.
    ///
    /// ECS uses Z-up coordinates. Vulkan uses Y-up for rendering.
    /// Delegates to render_adapter for centralized coordinate conversion.
    pub fn model_matrix(&self) -> glm::Mat4 {
        crate::engine::adapters::render_adapter::transform_to_model_matrix(self)
    }
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            position: glm::vec3(0.0, 0.0, 0.0),
            rotation: glm::quat_identity(),
            scale: glm::vec3(1.0, 1.0, 1.0),
        }
    }
}

/// Mesh renderer component
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeshRenderer {
    pub mesh_index: usize,
    pub material_index: usize,
    /// Whether this mesh is rendered
    #[serde(default = "default_true")]
    pub visible: bool,
    /// Whether this mesh casts shadows (when shadow mapping is active)
    #[serde(default = "default_true")]
    pub cast_shadows: bool,
    /// Whether this mesh receives shadows from other objects
    #[serde(default = "default_true")]
    pub receive_shadows: bool,
}

impl Default for MeshRenderer {
    fn default() -> Self {
        Self {
            mesh_index: 0,
            material_index: 0,
            visible: true,
            cast_shadows: true,
            receive_shadows: true,
        }
    }
}

/// Camera projection type
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum CameraProjection {
    Perspective,
    Orthographic { size: f32 },
}

impl Default for CameraProjection {
    fn default() -> Self {
        Self::Perspective
    }
}

/// Camera component for 3D rendering
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Camera {
    pub fov: f32,
    pub near: f32,
    pub far: f32,
    pub active: bool,
    /// Projection type (perspective or orthographic)
    #[serde(default)]
    pub projection: CameraProjection,
    /// Background clear color (RGB)
    #[serde(default = "default_clear_color")]
    pub clear_color: [f32; 3],
    /// Render priority (higher renders on top)
    #[serde(default)]
    pub priority: i32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            fov: 60.0,
            near: 0.1,
            far: 1000.0,
            active: true,
            projection: CameraProjection::default(),
            clear_color: default_clear_color(),
            priority: 0,
        }
    }
}

/// Directional light component
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct DirectionalLight {
    #[serde(with = "vec3_serde")]
    pub direction: glm::Vec3,
    #[serde(with = "vec3_serde")]
    pub color: glm::Vec3,
    pub intensity: f32,
    /// Enable shadow mapping for this light
    #[serde(default)]
    pub shadow_enabled: bool,
    /// Shadow bias to prevent shadow acne artifacts
    #[serde(default = "default_shadow_bias")]
    pub shadow_bias: f32,
}

impl Default for DirectionalLight {
    fn default() -> Self {
        Self {
            direction: glm::vec3(0.0, -1.0, 0.0),
            color: glm::vec3(1.0, 1.0, 1.0),
            intensity: 1.0,
            shadow_enabled: false,
            shadow_bias: default_shadow_bias(),
        }
    }
}

/// Light attenuation falloff model
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum LightFalloff {
    /// Light fades linearly with distance
    Linear,
    /// Physically-based quadratic falloff
    #[default]
    Quadratic,
    /// Realistic inverse-square law
    InverseSquare,
}

/// Point light component
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct PointLight {
    #[serde(with = "vec3_serde")]
    pub color: glm::Vec3,
    pub intensity: f32,
    pub radius: f32,
    /// Enable shadow mapping for this light
    #[serde(default)]
    pub shadow_enabled: bool,
    /// Attenuation falloff model
    #[serde(default)]
    pub falloff: LightFalloff,
}

impl Default for PointLight {
    fn default() -> Self {
        Self {
            color: glm::vec3(1.0, 1.0, 1.0),
            intensity: 1.0,
            radius: 10.0,
            shadow_enabled: false,
            falloff: LightFalloff::default(),
        }
    }
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

/// Tag component for player-controlled entities
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Player;

/// Tag component for naming entities
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Name(pub String);

impl Name {
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }
}

/// Globally unique identifier for entities.
/// Survives serialization round-trips and is used for snapshot restore.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EntityGuid(pub uuid::Uuid);

impl EntityGuid {
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }

    pub fn from_string(s: &str) -> Option<Self> {
        uuid::Uuid::parse_str(s).ok().map(Self)
    }
}

impl Default for EntityGuid {
    fn default() -> Self {
        Self::new()
    }
}

// ========== Custom Serde for nalgebra-glm types ==========

mod vec3_serde {
    use nalgebra_glm as glm;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    #[derive(Serialize, Deserialize)]
    struct Vec3Surrogate {
        x: f32,
        y: f32,
        z: f32,
    }

    pub fn serialize<S>(vec: &glm::Vec3, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let surrogate = Vec3Surrogate {
            x: vec.x,
            y: vec.y,
            z: vec.z,
        };
        surrogate.serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<glm::Vec3, D::Error>
    where
        D: Deserializer<'de>,
    {
        let surrogate = Vec3Surrogate::deserialize(deserializer)?;
        Ok(glm::vec3(surrogate.x, surrogate.y, surrogate.z))
    }
}

mod quat_serde {
    use nalgebra_glm as glm;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    #[derive(Serialize, Deserialize)]
    struct QuatSurrogate {
        x: f32,
        y: f32,
        z: f32,
        w: f32,
    }

    pub fn serialize<S>(quat: &glm::Quat, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let surrogate = QuatSurrogate {
            x: quat.coords.x,
            y: quat.coords.y,
            z: quat.coords.z,
            w: quat.coords.w,
        };
        surrogate.serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<glm::Quat, D::Error>
    where
        D: Deserializer<'de>,
    {
        let surrogate = QuatSurrogate::deserialize(deserializer)?;
        // glm::quat takes parameters in order (x, y, z, w), NOT (w, x, y, z)!
        Ok(glm::quat(
            surrogate.x,
            surrogate.y,
            surrogate.z,
            surrogate.w,
        ))
    }
}
