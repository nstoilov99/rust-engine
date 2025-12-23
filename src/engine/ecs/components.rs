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

    /// Calculate the model matrix for this transform
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
}

/// Camera component for 3D rendering
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Camera {
    pub fov: f32,
    pub near: f32,
    pub far: f32,
    pub active: bool,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            fov: 60.0,
            near: 0.1,
            far: 1000.0,
            active: true,
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
}

impl Default for DirectionalLight {
    fn default() -> Self {
        Self {
            direction: glm::vec3(0.0, -1.0, 0.0),
            color: glm::vec3(1.0, 1.0, 1.0),
            intensity: 1.0,
        }
    }
}

/// Point light component
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct PointLight {
    #[serde(with = "vec3_serde")]
    pub color: glm::Vec3,
    pub intensity: f32,
    pub radius: f32,
}

impl Default for PointLight {
    fn default() -> Self {
        Self {
            color: glm::vec3(1.0, 1.0, 1.0),
            intensity: 1.0,
            radius: 10.0,
        }
    }
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
        Ok(glm::quat(
            surrogate.w,
            surrogate.x,
            surrogate.y,
            surrogate.z,
        ))
    }
}
