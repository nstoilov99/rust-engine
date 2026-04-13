//! Character movement system — executes movement via physics forces.

use game_shared::components::{CharacterMovement, LookController, MovementMode};
use nalgebra_glm as glm;
use rust_engine::engine::adapters::physics_adapter::velocity_to_physics;
use rust_engine::engine::ecs::access::SystemDescriptor;
use rust_engine::engine::ecs::components::{Camera, Transform, TransformDirty};
use rust_engine::engine::ecs::hierarchy::Children;
use rust_engine::engine::ecs::resources::{Resources, Time};
use rust_engine::engine::ecs::schedule::System;
use rust_engine::engine::physics::{PhysicsWorld, RigidBody, RigidBodyHandle};

pub struct CharacterMovementSystem;

impl System for CharacterMovementSystem {
    fn run(&mut self, world: &mut hecs::World, resources: &mut Resources) {
        struct MoveData {
            entity: hecs::Entity,
            handle: RigidBodyHandle,
            desired_velocity: glm::Vec3,
            move_speed: f32,
            is_sprinting: bool,
            sprint_multiplier: f32,
            jump_requested: bool,
            jump_impulse: f32,
            position: glm::Vec3,
            ground_check_dist: f32,
            movement_mode: MovementMode,
            pitch: Option<f32>,
            camera_children: Vec<hecs::Entity>,
        }

        let mut moves: Vec<MoveData> = Vec::new();

        for (entity, (transform, rb, cm, look, children)) in world.query_mut::<(
            &mut Transform,
            &RigidBody,
            &mut CharacterMovement,
            Option<&LookController>,
            Option<&Children>,
        )>() {
            let Some(handle) = rb.physics_handle() else {
                continue;
            };

            if let Some(look) = look {
                transform.rotation =
                    glm::quat_angle_axis(look.yaw, &glm::vec3(0.0, 0.0, 1.0));
            }

            let camera_children = children
                .map(|c| c.0.clone())
                .unwrap_or_default();

            moves.push(MoveData {
                entity,
                handle,
                desired_velocity: glm::vec3(
                    cm.desired_velocity[0],
                    cm.desired_velocity[1],
                    cm.desired_velocity[2],
                ),
                move_speed: cm.move_speed,
                is_sprinting: cm.is_sprinting,
                sprint_multiplier: cm.sprint_multiplier,
                jump_requested: cm.jump_requested,
                jump_impulse: cm.jump_impulse,
                position: transform.position,
                ground_check_dist: cm.ground_check_dist,
                movement_mode: cm.movement_mode,
                pitch: look.map(|l| l.pitch),
                camera_children,
            });

            cm.desired_velocity = [0.0; 3];
            cm.jump_requested = false;
        }

        let mut grounding_updates: Vec<(hecs::Entity, bool)> = Vec::new();
        let mut dirty_entities: Vec<hecs::Entity> = Vec::new();

        for mv in &moves {
            let is_grounded = if mv.movement_mode == MovementMode::Walking {
                resources
                    .get::<PhysicsWorld>()
                    .and_then(|physics| {
                        physics.raycast(
                            mv.position,
                            glm::vec3(0.0, 0.0, -1.0),
                            mv.ground_check_dist,
                        )
                    })
                    .is_some()
            } else {
                false
            };
            grounding_updates.push((mv.entity, is_grounded));

            let has_input = mv.desired_velocity.magnitude_squared() > 0.0001;
            if has_input {
                let mut speed = mv.move_speed;
                if mv.is_sprinting {
                    speed *= mv.sprint_multiplier;
                }
                let force = velocity_to_physics(&(mv.desired_velocity * speed));
                if let Some(physics) = resources.get_mut::<PhysicsWorld>() {
                    physics.apply_force(mv.handle, force);
                }
            }

            if mv.jump_requested && is_grounded {
                let impulse = velocity_to_physics(&glm::vec3(0.0, 0.0, mv.jump_impulse));
                if let Some(physics) = resources.get_mut::<PhysicsWorld>() {
                    physics.apply_impulse(mv.handle, impulse);
                }
            }

            dirty_entities.push(mv.entity);

            if let Some(pitch) = mv.pitch {
                for &child in &mv.camera_children {
                    if world.get::<&Camera>(child).is_ok() {
                        if let Ok(mut cam_transform) = world.get::<&mut Transform>(child) {
                            cam_transform.rotation =
                                glm::quat_angle_axis(pitch, &glm::vec3(0.0, 1.0, 0.0));
                        }
                        dirty_entities.push(child);
                    }
                }
            }
        }

        for (entity, is_grounded) in grounding_updates {
            if let Ok(mut cm) = world.get::<&mut CharacterMovement>(entity) {
                cm.is_grounded = is_grounded;
            }
        }

        for entity in dirty_entities {
            let _ = world.insert_one(entity, TransformDirty);
        }
    }

    fn name(&self) -> &str {
        "CharacterMovementSystem"
    }
}

impl CharacterMovementSystem {
    pub fn descriptor() -> SystemDescriptor {
        SystemDescriptor::new("CharacterMovementSystem")
            .reads_resource::<Time>()
            .writes_resource::<PhysicsWorld>()
            .writes::<Transform>()
            .writes::<CharacterMovement>()
            .reads::<LookController>()
            .reads::<RigidBody>()
            .reads::<Camera>()
            .reads::<Children>()
            .after("PlayerInputSystem")
    }
}
