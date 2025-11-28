//! Scene serialization and deserialization

use super::scene_format::{SceneFile, EntityData, ComponentData};
use crate::engine::ecs::components::*;
use hecs::World;
use std::fs;
use nalgebra_glm as glm;

/// Serialize ECS world to scene file
pub fn save_scene(world: &World, path: &str, scene_name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut entities = Vec::new();

    // Iterate all entities
    for entity_ref in world.iter() {
        let entity_id = entity_ref.entity();
        let mut components = Vec::new();

        // Get entity name first
        let entity_name = entity_ref.get::<&Name>()
            .map(|name| name.0.clone())
            .unwrap_or_else(|| format!("Entity_{:?}", entity_id));

        // Try to get each component type
        if let Some(transform) = entity_ref.get::<&Transform>() {
            components.push(ComponentData::Transform {
                position: [transform.position.x, transform.position.y, transform.position.z],
                rotation: [transform.rotation.coords.x, transform.rotation.coords.y, transform.rotation.coords.z, transform.rotation.coords.w],
                scale: [transform.scale.x, transform.scale.y, transform.scale.z],
            });
        }

        if let Some(mesh_renderer) = entity_ref.get::<&MeshRenderer>() {
            components.push(ComponentData::MeshRenderer {
                mesh_index: mesh_renderer.mesh_index,
                material_index: mesh_renderer.material_index,
            });
        }

        if let Some(camera) = entity_ref.get::<&Camera>() {
            components.push(ComponentData::Camera {
                fov: camera.fov,
                near: camera.near,
                far: camera.far,
                active: camera.active,
            });
        }

        if let Some(dir_light) = entity_ref.get::<&DirectionalLight>() {
            components.push(ComponentData::DirectionalLight {
                direction: [dir_light.direction.x, dir_light.direction.y, dir_light.direction.z],
                color: [dir_light.color.x, dir_light.color.y, dir_light.color.z],
                intensity: dir_light.intensity,
            });
        }

        if let Some(point_light) = entity_ref.get::<&PointLight>() {
            components.push(ComponentData::PointLight {
                color: [point_light.color.x, point_light.color.y, point_light.color.z],
                intensity: point_light.intensity,
                radius: point_light.radius,
            });
        }

        if entity_ref.get::<&Player>().is_some() {
            components.push(ComponentData::Player);
        }

        // Only save entities with components
        if !components.is_empty() {
            entities.push(EntityData {
                name: entity_name,
                components,
            });
        }
    }

    let scene_file = SceneFile {
        version: "1.0".to_string(),
        name: scene_name.to_string(),
        entities,
    };

    // Serialize to RON format
    let ron_string = ron::ser::to_string_pretty(&scene_file, ron::ser::PrettyConfig::default())?;

    // Create parent directory if it doesn't exist
    if let Some(parent) = std::path::Path::new(path).parent() {
        fs::create_dir_all(parent)?;
    }

    // Write to file
    fs::write(path, ron_string)?;

    println!("💾 Scene saved: {} ({} entities)", path, scene_file.entities.len());

    Ok(())
}

/// Deserialize scene file into ECS world
pub fn load_scene(world: &mut World, path: &str) -> Result<String, Box<dyn std::error::Error>> {
    // Read file
    let ron_string = fs::read_to_string(path)?;

    // Deserialize from RON
    let scene_file: SceneFile = ron::from_str(&ron_string)?;

    println!("📂 Loading scene: {} (v{})", scene_file.name, scene_file.version);

    // Clear existing world
    world.clear();

    // Spawn entities
    for entity_data in &scene_file.entities {
        spawn_entity_from_data(world, entity_data);
    }

    println!("✅ Scene loaded: {} ({} entities)", scene_file.name, scene_file.entities.len());

    Ok(scene_file.name)
}

/// Spawn a single entity from serialized data
fn spawn_entity_from_data(world: &mut World, entity_data: &EntityData) {
    use hecs::EntityBuilder;

    let mut builder = EntityBuilder::new();

    // Add components based on what's in the data
    for component_data in &entity_data.components {
        match component_data {
            ComponentData::Transform { position, rotation, scale } => {
                // Create quaternion from [x, y, z, w] array
                let quat = glm::Quat::new(rotation[3], rotation[0], rotation[1], rotation[2]); // w, x, y, z
                let transform = Transform {
                    position: glm::vec3(position[0], position[1], position[2]),
                    rotation: quat,
                    scale: glm::vec3(scale[0], scale[1], scale[2]),
                };
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
        }
    }

    // Add the name component
    builder.add(Name::new(entity_data.name.clone()));

    // Spawn the entity
    let entity = world.spawn(builder.build());
    println!("  ↳ Spawned entity: {} ({:?})", entity_data.name, entity);
}