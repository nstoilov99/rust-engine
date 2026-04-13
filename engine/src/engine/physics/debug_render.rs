//! Debug visualization for physics colliders.
//!
//! Iterates ECS entities with `Transform + Collider` components and submits
//! wireframe lines for colliders that have `debug_draw_visible` enabled.
//! All geometry is generated directly in Z-up game space from the ECS data.

use crate::engine::debug_draw::DebugDrawBuffer;
use crate::engine::ecs::components::Transform;
use crate::engine::physics::components::{Collider, ColliderShape};
use hecs::World;

const COLLIDER_COLOR: [f32; 4] = [0.0, 1.0, 0.0, 0.5];

/// Submit collider debug wireframes for all entities with `debug_draw_visible`.
///
/// Reads shape and position from ECS components (Z-up game space).
/// No Rapier physics access needed.
pub fn submit_collider_debug_draws(world: &World, buffer: &mut DebugDrawBuffer) {
    for (_entity, (transform, collider)) in world.query::<(&Transform, &Collider)>().iter() {
        if !collider.debug_draw_visible {
            continue;
        }

        let center = [
            transform.position.x,
            transform.position.y,
            transform.position.z,
        ];

        match &collider.shape {
            ColliderShape::Cuboid { half_extents } => {
                buffer.box_wireframe(
                    center,
                    [half_extents.x, half_extents.y, half_extents.z],
                    COLLIDER_COLOR,
                );
            }
            ColliderShape::Ball { radius } => {
                buffer.sphere_wireframe(center, *radius, COLLIDER_COLOR);
            }
            ColliderShape::Capsule {
                half_height,
                radius,
            } => {
                buffer.capsule_wireframe(center, *half_height, *radius, COLLIDER_COLOR);
            }
        }
    }
}
