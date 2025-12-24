//! Scene serialization and deserialization

use super::scene_format::{ComponentData, EntityData, SceneFile};
use crate::engine::ecs::components::*;
use crate::engine::ecs::hierarchy::{set_parent, Children, Parent};
use hecs::{Entity, World};
use nalgebra_glm as glm;
use std::fs;

/// Serialize a single entity to EntityData
fn serialize_entity(world: &World, entity: Entity) -> Option<EntityData> {
    let mut components = Vec::new();

    // Get entity name first
    let entity_name = world
        .get::<&Name>(entity)
        .map(|name| name.0.clone())
        .unwrap_or_else(|_| format!("Entity_{:?}", entity));

    // Try to get each component type
    if let Ok(transform) = world.get::<&Transform>(entity) {
        components.push(ComponentData::Transform {
            position: [transform.position.x, transform.position.y, transform.position.z],
            rotation: [
                transform.rotation.coords.x,
                transform.rotation.coords.y,
                transform.rotation.coords.z,
                transform.rotation.coords.w,
            ],
            scale: [transform.scale.x, transform.scale.y, transform.scale.z],
        });
    }

    if let Ok(mesh_renderer) = world.get::<&MeshRenderer>(entity) {
        components.push(ComponentData::MeshRenderer {
            mesh_index: mesh_renderer.mesh_index,
            material_index: mesh_renderer.material_index,
        });
    }

    if let Ok(camera) = world.get::<&Camera>(entity) {
        components.push(ComponentData::Camera {
            fov: camera.fov,
            near: camera.near,
            far: camera.far,
            active: camera.active,
        });
    }

    if let Ok(dir_light) = world.get::<&DirectionalLight>(entity) {
        components.push(ComponentData::DirectionalLight {
            direction: [dir_light.direction.x, dir_light.direction.y, dir_light.direction.z],
            color: [dir_light.color.x, dir_light.color.y, dir_light.color.z],
            intensity: dir_light.intensity,
        });
    }

    if let Ok(point_light) = world.get::<&PointLight>(entity) {
        components.push(ComponentData::PointLight {
            color: [point_light.color.x, point_light.color.y, point_light.color.z],
            intensity: point_light.intensity,
            radius: point_light.radius,
        });
    }

    if world.get::<&Player>(entity).is_ok() {
        components.push(ComponentData::Player);
    }

    // Save parent relationship (hierarchy)
    if let Ok(parent) = world.get::<&Parent>(entity) {
        // Get parent entity's name for serialization
        if let Ok(parent_name) = world.get::<&Name>(parent.0) {
            components.push(ComponentData::Parent {
                parent_name: parent_name.0.clone(),
            });
        }
    }

    // Only save entities with components
    if !components.is_empty() {
        Some(EntityData {
            name: entity_name,
            components,
        })
    } else {
        None
    }
}

/// Recursively collect entities in hierarchy order (parent first, then children in order)
fn collect_entities_in_order(world: &World, entity: Entity, entities: &mut Vec<EntityData>) {
    // Serialize this entity first
    if let Some(entity_data) = serialize_entity(world, entity) {
        entities.push(entity_data);
    }

    // Then serialize children in their order
    if let Ok(children) = world.get::<&Children>(entity) {
        for &child in &children.0 {
            collect_entities_in_order(world, child, entities);
        }
    }
}

/// Serialize ECS world to scene file, preserving hierarchy order
/// root_order: the order of root entities (from HierarchyPanel)
pub fn save_scene(
    world: &World,
    path: &str,
    scene_name: &str,
    root_order: &[Entity],
) -> Result<(), Box<dyn std::error::Error>> {
    let mut entities = Vec::new();

    // Iterate root entities in the specified order
    for &root in root_order {
        collect_entities_in_order(world, root, &mut entities);
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

    println!(
        "Scene saved: {} ({} entities)",
        path,
        scene_file.entities.len()
    );

    Ok(())
}

/// Deserialize scene file into ECS world
/// Returns (scene_name, root_entities_in_order)
pub fn load_scene(
    world: &mut World,
    path: &str,
) -> Result<(String, Vec<Entity>), Box<dyn std::error::Error>> {
    // Read file
    let ron_string = fs::read_to_string(path)?;

    // Deserialize from RON
    let scene_file: SceneFile = ron::from_str(&ron_string)?;

    println!(
        "Loading scene: {} (v{})",
        scene_file.name, scene_file.version
    );

    // Clear existing world
    world.clear();

    // First pass: spawn all entities and collect parent relationships
    let mut parent_relationships: Vec<(Entity, String)> = Vec::new();
    let mut root_entities: Vec<Entity> = Vec::new();

    for entity_data in &scene_file.entities {
        let entity = spawn_entity_from_data(world, entity_data);

        // Check if this entity has a parent reference
        let mut has_parent = false;
        for component in &entity_data.components {
            if let ComponentData::Parent { parent_name } = component {
                parent_relationships.push((entity, parent_name.clone()));
                has_parent = true;
            }
        }

        // Root entities are those without a Parent component
        if !has_parent {
            root_entities.push(entity);
        }
    }

    // Second pass: establish parent-child relationships by name
    for (child_entity, parent_name) in parent_relationships {
        // Find parent entity by name - collect first to avoid borrow issues
        let parent_entity: Option<Entity> = world
            .query::<&Name>()
            .iter()
            .find(|(_, name)| name.0 == parent_name)
            .map(|(entity, _)| entity);

        if let Some(parent) = parent_entity {
            set_parent(world, child_entity, parent);
        } else {
            eprintln!(
                "  Warning: Parent '{}' not found for entity {:?}",
                parent_name, child_entity
            );
        }
    }

    println!(
        "Scene loaded: {} ({} entities, {} roots)",
        scene_file.name,
        scene_file.entities.len(),
        root_entities.len()
    );

    Ok((scene_file.name, root_entities))
}

/// Spawn a single entity from serialized data
/// Returns the spawned Entity for hierarchy restoration
fn spawn_entity_from_data(world: &mut World, entity_data: &EntityData) -> Entity {
    use hecs::EntityBuilder;

    let mut builder = EntityBuilder::new();

    // Add components based on what's in the data
    for component_data in &entity_data.components {
        match component_data {
            ComponentData::Transform {
                position,
                rotation,
                scale,
            } => {
                // Create quaternion from [x, y, z, w] array
                let quat = glm::Quat::new(rotation[3], rotation[0], rotation[1], rotation[2]); // w, x, y, z
                let transform = Transform {
                    position: glm::vec3(position[0], position[1], position[2]),
                    rotation: quat,
                    scale: glm::vec3(scale[0], scale[1], scale[2]),
                };
                builder.add(transform);
            }
            ComponentData::MeshRenderer {
                mesh_index,
                material_index,
            } => {
                builder.add(MeshRenderer {
                    mesh_index: *mesh_index,
                    material_index: *material_index,
                });
            }
            ComponentData::Camera {
                fov,
                near,
                far,
                active,
            } => {
                builder.add(Camera {
                    fov: *fov,
                    near: *near,
                    far: *far,
                    active: *active,
                });
            }
            ComponentData::DirectionalLight {
                direction,
                color,
                intensity,
            } => {
                builder.add(DirectionalLight {
                    direction: glm::vec3(direction[0], direction[1], direction[2]),
                    color: glm::vec3(color[0], color[1], color[2]),
                    intensity: *intensity,
                });
            }
            ComponentData::PointLight {
                color,
                intensity,
                radius,
            } => {
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
                // Parent relationships are handled in second pass of load_scene()
                // Skip here to avoid issues with entity ordering
            }
        }
    }

    // Add the name component
    builder.add(Name::new(entity_data.name.clone()));

    // Spawn the entity
    let entity = world.spawn(builder.build());
    println!("  ↳ Spawned entity: {} ({:?})", entity_data.name, entity);

    entity
}