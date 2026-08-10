//! Physics system using Rapier 3D
//!
//! Provides physics simulation integrated with the ECS.
//!
//! # Quick Start
//! ```ignore
//! use rust_engine::engine::physics::*;
//!
//! // Create physics world
//! let mut physics = PhysicsWorld::new();
//!
//! // Spawn an entity with physics components
//! world.spawn((
//!     Transform::new(glm::vec3(0.0, 10.0, 0.0)),
//!     RigidBody::dynamic(),
//!     Collider::ball(1.0),
//! ));
//!
//! // Register entities with physics
//! for (_, (t, rb, col)) in world.query::<(&Transform, &mut RigidBody, &mut Collider)>().iter() {
//!     physics.register_entity(t, rb, col);
//! }
//!
//! // In game loop
//! physics.step(delta_time, &mut world);
//! ```

mod components;
pub mod debug_render;
pub mod system;
mod world;

pub use components::*;
pub use debug_render::submit_collider_debug_draws;
pub use system::PhysicsStepSystem;
pub use world::PhysicsWorld;

/// Clear every Rapier body/collider and re-register the ECS world's physics
/// entities from scratch.
///
/// This is the *only* body-registration entry point (Task 39.8 P2): every
/// content moment funnels through it via the world-population helper, so
/// P5 has exactly one call to move into `RapierPhysicsPlugin`. Clearing
/// first makes it idempotent — running it twice on the same world cannot
/// double-register.
pub fn rebuild_bodies_from_world(physics_world: &mut PhysicsWorld, world: &mut hecs::World) {
    use crate::engine::ecs::components::Transform;

    physics_world.rigid_body_set = rapier3d::prelude::RigidBodySet::new();
    physics_world.collider_set = rapier3d::prelude::ColliderSet::new();
    physics_world.island_manager = rapier3d::prelude::IslandManager::new();
    physics_world.broad_phase = rapier3d::prelude::DefaultBroadPhase::new();
    physics_world.narrow_phase = rapier3d::prelude::NarrowPhase::new();
    physics_world.impulse_joint_set = rapier3d::prelude::ImpulseJointSet::new();
    physics_world.multibody_joint_set = rapier3d::prelude::MultibodyJointSet::new();
    physics_world.ccd_solver = rapier3d::prelude::CCDSolver::new();
    physics_world.query_pipeline = rapier3d::prelude::QueryPipeline::new();
    physics_world.reset_accumulator();

    // Handles from the previous population are dangling now.
    for (_, rigidbody) in world.query::<&mut RigidBody>().iter() {
        rigidbody.handle = None;
    }
    for (_, collider) in world.query::<&mut Collider>().iter() {
        collider.handle = None;
    }

    for (_, (transform, rigidbody, collider)) in world
        .query::<(&Transform, &mut RigidBody, &mut Collider)>()
        .iter()
    {
        physics_world.register_entity(transform, rigidbody, collider);
    }
}

// Re-export useful Rapier types
pub use rapier3d::prelude::{ColliderHandle, RigidBodyHandle};

// Serde helper for Vec3 (reused pattern from ecs/components.rs)
pub(crate) mod vec3_serde {
    use nalgebra_glm as glm;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    #[derive(Serialize, Deserialize)]
    struct Vec3Surrogate {
        x: f32,
        y: f32,
        z: f32,
    }

    pub fn serialize<S>(vec: &glm::Vec3, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        Vec3Surrogate {
            x: vec.x,
            y: vec.y,
            z: vec.z,
        }
        .serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<glm::Vec3, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = Vec3Surrogate::deserialize(deserializer)?;
        Ok(glm::vec3(s.x, s.y, s.z))
    }
}
