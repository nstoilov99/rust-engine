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
