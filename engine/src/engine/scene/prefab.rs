//! Prefab system for reusable entity templates

use super::scene_format::{
    CameraProjectionData, ColliderShapeData, ComponentData, EntityData, LightFalloffData,
    RigidBodyTypeData,
};
use crate::engine::audio::{AudioBus, AudioEmitter, AudioListener};
use crate::engine::ecs::components::*;
use crate::engine::physics::{
    Collider as PhysCollider, ColliderShape, RigidBody as PhysRigidBody, RigidBodyType,
};
use hecs::{Entity, EntityBuilder, World};
use nalgebra_glm as glm;
use serde::{Deserialize, Serialize};
use std::fs;

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
    /// Load prefab from file (uses asset source for pak support).
    pub fn load(path: &str) -> Result<Self, Box<dyn std::error::Error>> {
        use crate::engine::assets::asset_source;

        let ron_string = if asset_source::is_pak() {
            let relative = asset_source::to_content_relative(path);
            asset_source::read_string(&relative)?
        } else {
            fs::read_to_string(path)?
        };
        let prefab: Prefab = ron::from_str(&ron_string)?;
        println!("Loaded prefab: {} from {}", prefab.name, path);
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
                ComponentData::Transform {
                    position,
                    rotation,
                    scale,
                } => {
                    // glm::quat takes parameters in order (x, y, z, w), NOT (w, x, y, z)!
                    // rotation array is [x, y, z, w]
                    let mut transform = Transform {
                        position: glm::vec3(position[0], position[1], position[2]),
                        rotation: glm::quat(rotation[0], rotation[1], rotation[2], rotation[3]),
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
                ComponentData::MeshRenderer {
                    mesh_path,
                    material_paths,
                    material_path,
                    mesh_index,
                    material_index,
                    visible,
                    cast_shadows,
                    receive_shadows,
                    base_color_factor,
                    metallic_factor,
                    roughness_factor,
                    emissive_factor,
                } => {
                    let mut mr = MeshRenderer {
                        mesh_path: mesh_path.clone(),
                        material_paths: material_paths.clone(),
                        material_path: material_path.clone(),
                        mesh_index: *mesh_index,
                        material_index: *material_index,
                        visible: *visible,
                        cast_shadows: *cast_shadows,
                        receive_shadows: *receive_shadows,
                        base_color_factor: *base_color_factor,
                        metallic_factor: *metallic_factor,
                        roughness_factor: *roughness_factor,
                        emissive_factor: *emissive_factor,
                    };
                    mr.migrate_legacy_material_path();
                    builder.add(mr);
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
                        debug_draw_visible: false,
                        handle: None,
                    });
                }
                ComponentData::Player => {
                    builder.add(Player);
                }
                ComponentData::AudioEmitter {
                    clip_path,
                    bus,
                    volume_db,
                    pitch,
                    looping,
                    auto_play,
                    spatial,
                    max_distance,
                    hide_range_in_game,
                } => {
                    let bus_val = match bus.as_str() {
                        "Music" => AudioBus::Music,
                        "Ambient" => AudioBus::Ambient,
                        _ => AudioBus::SFX,
                    };
                    builder.add(AudioEmitter {
                        clip_path: clip_path.clone(),
                        bus: bus_val,
                        volume_db: *volume_db,
                        pitch: *pitch,
                        looping: *looping,
                        auto_play: *auto_play,
                        spatial: *spatial,
                        max_distance: *max_distance,
                        hide_range_in_game: *hide_range_in_game,
                    });
                }
                ComponentData::AudioListener { active } => {
                    builder.add(AudioListener { active: *active });
                }
                ComponentData::ParticleEffect { .. } => {
                    // ParticleEffect prefab instantiation handled via scene serializer
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
