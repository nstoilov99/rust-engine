//! Core ECS components

use nalgebra_glm as glm;

/// 3D transform component
#[derive(Debug, Clone, Copy)]
pub struct Transform {
    pub position: glm::Vec3,
    pub rotation: glm::Quat,
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
    pub fn model_matrix(&self) -> glm::Mat4 {
        let translation = glm::translation(&self.position);
        let rotation = glm::quat_to_mat4(&self.rotation);
        let scale = glm::scaling(&self.scale);
        translation * rotation * scale
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
#[derive(Debug, Clone)]
pub struct MeshRenderer {
    pub mesh_index: usize,
    pub material_index: usize,
}

/// Camera component for 3D rendering
#[derive(Debug, Clone)]
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
#[derive(Debug, Clone, Copy)]
pub struct DirectionalLight {
    pub direction: glm::Vec3,
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
#[derive(Debug, Clone, Copy)]
pub struct PointLight {
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
#[derive(Debug, Clone, Copy)]
pub struct Player;

/// Tag component for naming entities
#[derive(Debug, Clone)]
pub struct Name(pub String);

impl Name {
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }
}