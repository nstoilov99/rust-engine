//! Debug gizmo drawing for ParticleEffect components.
//!
//! All coordinates are in Z-up game space.

use super::components::{SpawnShape, ParticleEffect, Transform};
use crate::engine::debug_draw::api::DebugDrawBuffer;
use hecs::World;

const GIZMO_COLOR: [f32; 4] = [0.2, 0.8, 1.0, 1.0]; // Cyan
const GRAVITY_COLOR: [f32; 4] = [1.0, 0.3, 0.1, 0.8]; // Orange-red
const VELOCITY_COLOR: [f32; 4] = [0.3, 1.0, 0.3, 0.8]; // Green

pub fn submit_plankton_debug_draws(world: &World, buffer: &mut DebugDrawBuffer) {
    for (_entity, (transform, effect)) in world.query::<(&Transform, &ParticleEffect)>().iter() {
        if !effect.show_gizmos {
            continue;
        }

        let center = [
            transform.position.x,
            transform.position.y,
            transform.position.z,
        ];

        // Draw shape gizmo
        match effect.spawn_shape {
            SpawnShape::Point => {
                buffer.cross(center, 0.15);
            }
            SpawnShape::Sphere { radius } => {
                buffer.sphere_wireframe(center, radius, GIZMO_COLOR);
            }
            SpawnShape::Cone { angle_rad, radius } => {
                buffer.sphere_wireframe(center, radius * 0.5, GIZMO_COLOR);
                let cone_height = radius * angle_rad.tan().max(0.1);
                let tip = [center[0], center[1], center[2] + cone_height];
                buffer.arrow(center, tip, GIZMO_COLOR);
            }
            SpawnShape::Box { half_extents } => {
                buffer.box_wireframe(
                    center,
                    [half_extents[0], half_extents[1], half_extents[2]],
                    GIZMO_COLOR,
                );
            }
        }

        // Gravity arrow (Z-up game space) — extracted from module stack
        if let Some(grav) = effect.gravity() {
            let grav_len = (grav[0] * grav[0] + grav[1] * grav[1] + grav[2] * grav[2]).sqrt();
            if grav_len > 0.01 {
                let scale = 0.3;
                let end = [
                    center[0] + grav[0] * scale,
                    center[1] + grav[1] * scale,
                    center[2] + grav[2] * scale,
                ];
                buffer.arrow(center, end, GRAVITY_COLOR);
            }
        }

        // Velocity direction indicator
        let vel = effect.initial_velocity;
        let vel_len = (vel[0] * vel[0] + vel[1] * vel[1] + vel[2] * vel[2]).sqrt();
        if vel_len > 0.01 {
            let scale = 0.5;
            let end = [
                center[0] + vel[0] * scale,
                center[1] + vel[1] * scale,
                center[2] + vel[2] * scale,
            ];
            buffer.arrow(center, end, VELOCITY_COLOR);
        }
    }
}
