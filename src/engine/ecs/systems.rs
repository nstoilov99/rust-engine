//! Legacy ECS systems (retained for backward compatibility).
//!
//! New systems should implement `schedule::System` instead.

use super::components::*;
use hecs::World;

/// Legacy system trait (superseded by `schedule::System`).
pub trait LegacySystem {
    fn update(&mut self, world: &mut World, delta_time: f32);
}

/// Example: Transform update system
pub struct TransformSystem;

impl LegacySystem for TransformSystem {
    fn update(&mut self, world: &mut World, _delta_time: f32) {
        crate::profile_scope!("transform_system");
        let mut clamped = Vec::new();
        for (id, transform) in world.query_mut::<&mut Transform>() {
            let before = transform.scale;
            transform.scale.x = transform.scale.x.max(0.001);
            transform.scale.y = transform.scale.y.max(0.001);
            transform.scale.z = transform.scale.z.max(0.001);
            if transform.scale != before {
                clamped.push(id);
            }
        }
        for entity in clamped {
            super::hierarchy::mark_transform_dirty(world, entity);
        }
    }
}

/// Legacy system scheduler (superseded by `schedule::Schedule`).
pub struct LegacySystemScheduler {
    systems: Vec<Box<dyn LegacySystem>>,
}

impl LegacySystemScheduler {
    pub fn new() -> Self {
        Self {
            systems: Vec::new(),
        }
    }

    pub fn add_system(&mut self, system: Box<dyn LegacySystem>) {
        self.systems.push(system);
    }

    pub fn update(&mut self, world: &mut World, delta_time: f32) {
        crate::profile_function!();
        for system in &mut self.systems {
            system.update(world, delta_time);
        }
    }
}

impl Default for LegacySystemScheduler {
    fn default() -> Self {
        Self::new()
    }
}

/// Example: Movement system
pub struct MovementSystem {
    pub speed: f32,
}

impl LegacySystem for MovementSystem {
    fn update(&mut self, world: &mut World, delta_time: f32) {
        crate::profile_scope!("movement_system");
        use super::components::{Player, Transform};

        let mut moved = Vec::new();
        for (id, (transform, _player)) in world.query::<(&mut Transform, &Player)>().iter() {
            transform.position.z -= self.speed * delta_time;
            moved.push(id);
        }
        for entity in moved {
            super::hierarchy::mark_transform_dirty(world, entity);
        }
    }
}

/// Example: Rotation system
pub struct RotationSystem {
    pub rotation_speed: f32,
}

impl LegacySystem for RotationSystem {
    fn update(&mut self, world: &mut World, delta_time: f32) {
        crate::profile_scope!("rotation_system");
        use super::components::Transform;
        use nalgebra_glm as glm;

        let mut rotated = Vec::new();
        for (id, transform) in world.query_mut::<&mut Transform>() {
            let rotation = glm::quat_angle_axis(
                self.rotation_speed * delta_time,
                &glm::vec3(0.0, 1.0, 0.0),
            );
            transform.rotation = rotation * transform.rotation;
            rotated.push(id);
        }
        for entity in rotated {
            super::hierarchy::mark_transform_dirty(world, entity);
        }
    }
}
