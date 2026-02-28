//! Scene serialization and deserialization

use super::scene_format::{
    CameraProjectionData, ColliderShapeData, ComponentData, EntityData, LightFalloffData,
    RigidBodyTypeData, SceneFile,
};
use crate::engine::ecs::components::*;
use crate::engine::ecs::hierarchy::{set_parent, Children, Parent};
use crate::engine::physics::{
    Collider as PhysCollider, ColliderShape, RigidBody as PhysRigidBody, RigidBodyType,
};
use hecs::{Entity, World};
use nalgebra_glm as glm;
use std::fs;
use log;

/// Serialize a single entity to EntityData
fn serialize_entity(world: &World, entity: Entity) -> Option<EntityData> {
    let mut components = Vec::new();

    // Get entity name first
    let entity_name = world
        .get::<&Name>(entity)
        .map(|name| name.0.clone())
        .unwrap_or_else(|_| format!("Entity_{:?}", entity));

    // Get GUID if present
    let guid = world
        .get::<&EntityGuid>(entity)
        .ok()
        .map(|g| g.0.to_string());

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
            visible: mesh_renderer.visible,
            cast_shadows: mesh_renderer.cast_shadows,
            receive_shadows: mesh_renderer.receive_shadows,
        });
    }

    if let Ok(camera) = world.get::<&Camera>(entity) {
        components.push(ComponentData::Camera {
            fov: camera.fov,
            near: camera.near,
            far: camera.far,
            active: camera.active,
            projection: match camera.projection {
                CameraProjection::Perspective => CameraProjectionData::Perspective,
                CameraProjection::Orthographic { size } => {
                    CameraProjectionData::Orthographic { size }
                }
            },
            clear_color: camera.clear_color,
            priority: camera.priority,
        });
    }

    if let Ok(dir_light) = world.get::<&DirectionalLight>(entity) {
        components.push(ComponentData::DirectionalLight {
            direction: [
                dir_light.direction.x,
                dir_light.direction.y,
                dir_light.direction.z,
            ],
            color: [dir_light.color.x, dir_light.color.y, dir_light.color.z],
            intensity: dir_light.intensity,
            shadow_enabled: dir_light.shadow_enabled,
            shadow_bias: dir_light.shadow_bias,
        });
    }

    if let Ok(point_light) = world.get::<&PointLight>(entity) {
        components.push(ComponentData::PointLight {
            color: [
                point_light.color.x,
                point_light.color.y,
                point_light.color.z,
            ],
            intensity: point_light.intensity,
            radius: point_light.radius,
            shadow_enabled: point_light.shadow_enabled,
            falloff: match point_light.falloff {
                LightFalloff::Linear => LightFalloffData::Linear,
                LightFalloff::Quadratic => LightFalloffData::Quadratic,
                LightFalloff::InverseSquare => LightFalloffData::InverseSquare,
            },
        });
    }

    if world.get::<&Player>(entity).is_ok() {
        components.push(ComponentData::Player);
    }

    if let Ok(rb) = world.get::<&PhysRigidBody>(entity) {
        components.push(ComponentData::RigidBody {
            body_type: match rb.body_type {
                RigidBodyType::Dynamic => RigidBodyTypeData::Dynamic,
                RigidBodyType::Kinematic => RigidBodyTypeData::Kinematic,
                RigidBodyType::Static => RigidBodyTypeData::Static,
            },
            mass: rb.mass,
            linear_damping: rb.linear_damping,
            angular_damping: rb.angular_damping,
            can_sleep: rb.can_sleep,
            gravity_scale: rb.gravity_scale,
            lock_rotation: rb.lock_rotation,
            continuous_collision: rb.continuous_collision,
        });
    }

    if let Ok(col) = world.get::<&PhysCollider>(entity) {
        components.push(ComponentData::Collider {
            shape: match &col.shape {
                ColliderShape::Cuboid { half_extents } => ColliderShapeData::Cuboid {
                    half_extents: [half_extents.x, half_extents.y, half_extents.z],
                },
                ColliderShape::Ball { radius } => ColliderShapeData::Ball { radius: *radius },
                ColliderShape::Capsule {
                    half_height,
                    radius,
                } => ColliderShapeData::Capsule {
                    half_height: *half_height,
                    radius: *radius,
                },
            },
            friction: col.friction,
            restitution: col.restitution,
            is_sensor: col.is_sensor,
        });
    }

    // Save parent relationship (hierarchy)
    if let Ok(parent) = world.get::<&Parent>(entity) {
        // Get parent entity's name for serialization
        if let Ok(parent_name) = world.get::<&Name>(parent.0) {
            let parent_guid = world
                .get::<&EntityGuid>(parent.0)
                .ok()
                .map(|g| g.0.to_string());
            components.push(ComponentData::Parent {
                parent_name: parent_name.0.clone(),
                parent_guid,
            });
        }
    }

    // Only save entities with components
    if !components.is_empty() {
        Some(EntityData {
            name: entity_name,
            guid,
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

/// Serialize ECS world to a RON string (in-memory, no file I/O).
/// Used for play-mode snapshots.
pub fn serialize_scene_to_string(
    world: &World,
    scene_name: &str,
    root_order: &[Entity],
) -> Result<String, Box<dyn std::error::Error>> {
    let mut entities = Vec::new();

    for &root in root_order {
        collect_entities_in_order(world, root, &mut entities);
    }

    let scene_file = SceneFile {
        version: "1.0".to_string(),
        name: scene_name.to_string(),
        entities,
    };

    let ron_string = ron::ser::to_string_pretty(&scene_file, ron::ser::PrettyConfig::default())?;
    Ok(ron_string)
}

/// Load a scene from a RON string (in-memory, no file I/O).
/// Used for play-mode snapshot restore.
/// Returns (scene_name, root_entities_in_order).
pub fn load_scene_from_string(
    world: &mut World,
    ron_string: &str,
) -> Result<(String, Vec<Entity>), Box<dyn std::error::Error>> {
    let scene_file: SceneFile = ron::from_str(ron_string)?;

    world.clear();

    let mut parent_relationships: Vec<(Entity, String, Option<String>)> = Vec::new();
    let mut root_entities: Vec<Entity> = Vec::new();

    for entity_data in &scene_file.entities {
        let entity = spawn_entity_from_data(world, entity_data);

        let mut has_parent = false;
        for component in &entity_data.components {
            if let ComponentData::Parent { parent_name, parent_guid } = component {
                parent_relationships.push((entity, parent_name.clone(), parent_guid.clone()));
                has_parent = true;
            }
        }

        if !has_parent {
            root_entities.push(entity);
        }
    }

    for (child_entity, parent_name, parent_guid) in parent_relationships {
        let parent_entity = resolve_parent(world, &parent_name, parent_guid.as_deref());
        if let Some(parent) = parent_entity {
            set_parent(world, child_entity, parent);
        } else {
            log::warn!(
                "Parent '{}' not found for entity {:?}, entity becomes root",
                parent_name, child_entity
            );
        }
    }

    Ok((scene_file.name, root_entities))
}

/// Deserialize scene file into ECS world
/// Returns (scene_name, root_entities_in_order)
pub fn load_scene(
    world: &mut World,
    path: &str,
) -> Result<(String, Vec<Entity>), Box<dyn std::error::Error>> {
    // Read file
    let ron_string = fs::read_to_string(path)?;

    println!("Loading scene from: {}", path);

    let result = load_scene_from_string(world, &ron_string)?;

    println!(
        "Scene loaded: {} ({} roots)",
        result.0, result.1.len()
    );

    Ok(result)
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
                visible,
                cast_shadows,
                receive_shadows,
            } => {
                builder.add(MeshRenderer {
                    mesh_index: *mesh_index,
                    material_index: *material_index,
                    visible: *visible,
                    cast_shadows: *cast_shadows,
                    receive_shadows: *receive_shadows,
                });
            }
            ComponentData::Camera {
                fov,
                near,
                far,
                active,
                projection,
                clear_color,
                priority,
            } => {
                builder.add(Camera {
                    fov: *fov,
                    near: *near,
                    far: *far,
                    active: *active,
                    projection: match projection {
                        CameraProjectionData::Perspective => CameraProjection::Perspective,
                        CameraProjectionData::Orthographic { size } => {
                            CameraProjection::Orthographic { size: *size }
                        }
                    },
                    clear_color: *clear_color,
                    priority: *priority,
                });
            }
            ComponentData::DirectionalLight {
                direction,
                color,
                intensity,
                shadow_enabled,
                shadow_bias,
            } => {
                builder.add(DirectionalLight {
                    direction: glm::vec3(direction[0], direction[1], direction[2]),
                    color: glm::vec3(color[0], color[1], color[2]),
                    intensity: *intensity,
                    shadow_enabled: *shadow_enabled,
                    shadow_bias: *shadow_bias,
                });
            }
            ComponentData::PointLight {
                color,
                intensity,
                radius,
                shadow_enabled,
                falloff,
            } => {
                builder.add(PointLight {
                    color: glm::vec3(color[0], color[1], color[2]),
                    intensity: *intensity,
                    radius: *radius,
                    shadow_enabled: *shadow_enabled,
                    falloff: match falloff {
                        LightFalloffData::Linear => LightFalloff::Linear,
                        LightFalloffData::Quadratic => LightFalloff::Quadratic,
                        LightFalloffData::InverseSquare => LightFalloff::InverseSquare,
                    },
                });
            }
            ComponentData::RigidBody {
                body_type,
                mass,
                linear_damping,
                angular_damping,
                can_sleep,
                gravity_scale,
                lock_rotation,
                continuous_collision,
            } => {
                builder.add(PhysRigidBody {
                    body_type: match body_type {
                        RigidBodyTypeData::Dynamic => RigidBodyType::Dynamic,
                        RigidBodyTypeData::Kinematic => RigidBodyType::Kinematic,
                        RigidBodyTypeData::Static => RigidBodyType::Static,
                    },
                    mass: *mass,
                    linear_damping: *linear_damping,
                    angular_damping: *angular_damping,
                    can_sleep: *can_sleep,
                    gravity_scale: *gravity_scale,
                    lock_rotation: *lock_rotation,
                    continuous_collision: *continuous_collision,
                    handle: None,
                });
            }
            ComponentData::Collider {
                shape,
                friction,
                restitution,
                is_sensor,
            } => {
                builder.add(PhysCollider {
                    shape: match shape {
                        ColliderShapeData::Cuboid { half_extents } => ColliderShape::Cuboid {
                            half_extents: glm::vec3(
                                half_extents[0],
                                half_extents[1],
                                half_extents[2],
                            ),
                        },
                        ColliderShapeData::Ball { radius } => {
                            ColliderShape::Ball { radius: *radius }
                        }
                        ColliderShapeData::Capsule {
                            half_height,
                            radius,
                        } => ColliderShape::Capsule {
                            half_height: *half_height,
                            radius: *radius,
                        },
                    },
                    friction: *friction,
                    restitution: *restitution,
                    is_sensor: *is_sensor,
                    handle: None,
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

    // Add EntityGuid - use from scene data if present, else generate new
    let entity_guid = entity_data
        .guid
        .as_ref()
        .and_then(|s| EntityGuid::from_string(s))
        .unwrap_or_else(EntityGuid::new);
    builder.add(entity_guid);

    // Spawn the entity
    let entity = world.spawn(builder.build());
    println!("  ↳ Spawned entity: {} ({:?})", entity_data.name, entity);

    entity
}

/// Resolve a parent entity by GUID (preferred) or name (fallback).
fn resolve_parent(world: &World, parent_name: &str, parent_guid: Option<&str>) -> Option<Entity> {
    // Try GUID first
    if let Some(guid_str) = parent_guid {
        if let Some(guid) = EntityGuid::from_string(guid_str) {
            let found = world
                .query::<&EntityGuid>()
                .iter()
                .find(|(_, g)| g.0 == guid.0)
                .map(|(e, _)| e);
            if found.is_some() {
                return found;
            }
        }
    }
    // Fallback to name
    world
        .query::<&Name>()
        .iter()
        .find(|(_, name)| name.0 == parent_name)
        .map(|(entity, _)| entity)
}
