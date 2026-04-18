//! Debug gizmo drawing for PlanktonEmitter components.
//!
//! All coordinates are in Z-up game space.

use super::components::{EmissionShape, PlanktonEmitter, Transform};
use crate::engine::debug_draw::api::DebugDrawBuffer;
use hecs::World;

const GIZMO_COLOR: [f32; 4] = [0.2, 0.8, 1.0, 1.0]; // Cyan
const GRAVITY_COLOR: [f32; 4] = [1.0, 0.3, 0.1, 0.8]; // Orange-red
const VELOCITY_COLOR: [f32; 4] = [0.3, 1.0, 0.3, 0.8]; // Green

pub fn submit_plankton_debug_draws(world: &World, buffer: &mut DebugDrawBuffer) {
    for (_entity, (transform, emitter)) in world.query::<(&Transform, &PlanktonEmitter)>().iter() {
        if !emitter.show_gizmos {
            continue;
        }

        let center = [
            transform.position.x,
            transform.position.y,
            transform.position.z,
        ];

        // Draw shape gizmo
        match emitter.emission_shape {
            EmissionShape::Point => {
                buffer.cross(center, 0.15);
            }
            EmissionShape::Sphere { radius } => {
                buffer.sphere_wireframe(center, radius, GIZMO_COLOR);
            }
            EmissionShape::Cone { angle_rad, radius } => {
                // Sphere wireframe at base radius for the cone opening
                buffer.sphere_wireframe(center, radius * 0.5, GIZMO_COLOR);
                // Arrow along Z axis showing cone direction
                let cone_height = radius * angle_rad.tan().max(0.1);
                let tip = [center[0], center[1], center[2] + cone_height];
                buffer.arrow(center, tip, GIZMO_COLOR);
            }
            EmissionShape::Box { half_extents } => {
                buffer.box_wireframe(
                    center,
                    [half_extents[0], half_extents[1], half_extents[2]],
                    GIZMO_COLOR,
                );
            }
        }

        // Gravity arrow (Z-up game space)
        let grav = emitter.gravity;
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

        // Velocity direction indicator
        let vel = emitter.initial_velocity;
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
