//! Prefab system for reusable entity templates

use super::scene_format::{EntityData, ComponentData};
use crate::engine::ecs::components::*;
use hecs::{World, Entity, EntityBuilder};
use serde::{Serialize, Deserialize};
use std::fs;
use nalgebra_glm as glm;

/// Prefab file format
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Prefab {
    pub name: String,
    pub description: String,
    pub template: EntityData,
}

/// Component override type
#[derive(Debug, Clone)]
pub enum ComponentOverride {
    Transform(Transform),
    Position(glm::Vec3),
    Rotation(glm::Quat),
    Scale(glm::Vec3),
}

/// Prefab instance with overrides
#[derive(Debug, Clone)]
pub struct PrefabInstance {
    pub prefab_name: String,
    pub overrides: Vec<ComponentOverride>,
}

impl Prefab {
    /// Load prefab from file
    pub fn load(path: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let ron_string = fs::read_to_string(path)?;
        let prefab: Prefab = ron::from_str(&ron_string)?;
        println!("📦 Loaded prefab: {} from {}", prefab.name, path);
        Ok(prefab)
    }

    /// Save prefab to file
    pub fn save(&self, path: &str) -> Result<(), Box<dyn std::error::Error>> {
        let ron_string = ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())?;
        fs::write(path, ron_string)?;
        println!("💾 Saved prefab: {} to {}", self.name, path);
        Ok(())
    }

    /// Instantiate prefab into world
    pub fn instantiate(&self, world: &mut World) -> Entity {
        self.instantiate_with_overrides(world, Vec::new())
    }

    /// Instantiate prefab with component overrides
    pub fn instantiate_with_overrides(
        &self,
        world: &mut World,
        overrides: Vec<ComponentOverride>,
    ) -> Entity {
        let mut builder = EntityBuilder::new();

        // Apply template components
        for component_data in &self.template.components {
            match component_data {
                ComponentData::Transform { position, rotation, scale } => {
                    let mut transform = Transform {
                        position: glm::vec3(position[0], position[1], position[2]),
                        rotation: glm::quat(rotation[3], rotation[0], rotation[1], rotation[2]),
                        scale: glm::vec3(scale[0], scale[1], scale[2]),
                    };

                    // Apply overrides
                    for override_item in &overrides {
                        match override_item {
                            ComponentOverride::Transform(t) => transform = *t,
                            ComponentOverride::Position(p) => transform.position = *p,
                            ComponentOverride::Rotation(r) => transform.rotation = *r,
                            ComponentOverride::Scale(s) => transform.scale = *s,
                        }
                    }

                    builder.add(transform);
                }
                ComponentData::MeshRenderer { mesh_index, material_index } => {
                    builder.add(MeshRenderer {
                        mesh_index: *mesh_index,
                        material_index: *material_index,
                    });
                }
                ComponentData::Camera { fov, near, far, active } => {
                    builder.add(Camera {
                        fov: *fov,
                        near: *near,
                        far: *far,
                        active: *active,
                    });
                }
                ComponentData::DirectionalLight { direction, color, intensity } => {
                    builder.add(DirectionalLight {
                        direction: glm::vec3(direction[0], direction[1], direction[2]),
                        color: glm::vec3(color[0], color[1], color[2]),
                        intensity: *intensity,
                    });
                }
                ComponentData::PointLight { color, intensity, radius } => {
                    builder.add(PointLight {
                        color: glm::vec3(color[0], color[1], color[2]),
                        intensity: *intensity,
                        radius: *radius,
                    });
                }
                ComponentData::Player => {
                    builder.add(Player);
                }
                ComponentData::Parent { .. } => {
                    // Parent relationships are not applicable for prefabs
                    // They are handled separately during scene loading
                }
            }
        }

        // Add entity name
        builder.add(Name::new(self.template.name.clone()));

        world.spawn(builder.build())
    }
}