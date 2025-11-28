//! Scene file format structures for serialization
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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
    pub components: Vec<ComponentData>,
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
    },
    Camera {
        fov: f32,
        near: f32,
        far: f32,
        active: bool,
    },
    DirectionalLight {
        direction: [f32; 3],
        color: [f32; 3],
        intensity: f32,
    },
    PointLight {
        color: [f32; 3],
        intensity: f32,
        radius: f32,
    },
    Player,
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
