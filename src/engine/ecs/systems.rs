//! ECS systems for game logic

use super::components::*;
use hecs::World;

/// System trait for organizing game logic
pub trait System {
    fn update(&mut self, world: &mut World, delta_time: f32);
}

/// Example: Transform update system
pub struct TransformSystem;

impl System for TransformSystem {
    fn update(&mut self, world: &mut World, _delta_time: f32) {
        // Example: validate transforms, compute hierarchies, etc.
        for (_id, transform) in world.query_mut::<&mut Transform>() {
            // Clamp scale to prevent negative values
            transform.scale.x = transform.scale.x.max(0.001);
            transform.scale.y = transform.scale.y.max(0.001);
            transform.scale.z = transform.scale.z.max(0.001);
        }
    }
}


/// System scheduler that runs systems in order
pub struct SystemScheduler {
    systems: Vec<Box<dyn System>>,
}

impl SystemScheduler {
    pub fn new() -> Self {
        Self {
            systems: Vec::new(),
        }
    }

    /// Add a system to the scheduler
    pub fn add_system(&mut self, system: Box<dyn System>) {
        self.systems.push(system);
    }

    /// Run all systems in order
    pub fn update(&mut self, world: &mut World, delta_time: f32) {
        for system in &mut self.systems {
            system.update(world, delta_time);
        }
    }
}

impl Default for SystemScheduler {
    fn default() -> Self {
        Self::new()
    }
}

/// Example: Movement system
pub struct MovementSystem {
    pub speed: f32,
}

impl System for MovementSystem {
    fn update(&mut self, world: &mut World, delta_time: f32) {
        use super::components::{Transform, Player};

        // Move player entities
        for (_id, (transform, _player)) in world.query::<(&mut Transform, &Player)>().iter() {
            // Example: simple forward movement
            transform.position.z -= self.speed * delta_time;
        }
    }
}

/// Example: Rotation system
pub struct RotationSystem {
    pub rotation_speed: f32,
}

impl System for RotationSystem {
    fn update(&mut self, world: &mut World, delta_time: f32) {
        use super::components::Transform;
        use nalgebra_glm as glm;

        for (_id, transform) in world.query_mut::<&mut Transform>() {
            // Rotate around Y axis
            let rotation = glm::quat_angle_axis(
                self.rotation_speed * delta_time,
                &glm::vec3(0.0, 1.0, 0.0),
            );
            transform.rotation = rotation * transform.rotation;
        }
    }
}