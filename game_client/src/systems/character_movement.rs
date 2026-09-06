//! Character movement system — velocity-set locomotion on a Rapier dynamic
//! capsule (Task 41.6 D1/D2).
//!
//! Per frame, per `(Transform, RigidBody, CharacterMovement)`: accelerate
//! the horizontal velocity toward the camera-relative intent, leave Z to
//! physics except for jump and step assist, write the velocity back, and
//! turn the body to face its heading. Runs in PreUpdate before the anim
//! stack and the physics step.
//!
//! Geometry comes from the entity's capsule `Collider` (feet = half_height +
//! radius below the body centre); D1's `0.5 + 0.4` is the fallback.

use game_shared::components::CharacterMovement;
use nalgebra_glm as glm;
use rust_engine::engine::ecs::access::SystemDescriptor;
use rust_engine::engine::ecs::components::{Transform, TransformDirty};
use rust_engine::engine::ecs::hierarchy::mark_transform_dirty;
use rust_engine::engine::ecs::resources::{Resources, Time};
use rust_engine::engine::ecs::schedule::System;
use rust_engine::engine::ecs::system_names;
use rust_engine::engine::physics::{Collider, ColliderShape, PhysicsWorld, RigidBody};
use std::f32::consts::{PI, TAU};

/// Feet below the capsule centre when the entity has no capsule collider.
const DEFAULT_FEET_BELOW_CENTRE: f32 = 0.9;
/// Step-assist knee probe origin: this far above the feet, so risers from
/// a few cm up to `step_height` are all seen.
const KNEE_ABOVE_FEET: f32 = 0.05;
/// Forward reach of the step probes.
const STEP_PROBE_LEN: f32 = 0.5;
/// Vertical velocity applied for one frame when a step is detected.
const STEP_ASSIST_VZ: f32 = 3.0;
/// Below this horizontal speed no step assist is attempted.
const MIN_STEP_SPEED: f32 = 0.1;
/// Below this horizontal speed the character keeps its facing.
const MIN_TURN_SPEED: f32 = 0.2;
/// Ground probe / snap / step assist stay off this long after a jump.
const JUMP_HOLD_SECS: f32 = 0.15;
/// Fastest the ground snap pulls a floating grounded body down, m/s.
const GROUND_SNAP_MAX_VZ: f32 = 3.0;
/// Ground normals flatter than this (cos of the angle to +Z) count as
/// walkable; steeper hits are walls and get no slope projection.
const MIN_WALKABLE_NZ: f32 = 0.5;

pub struct CharacterMovementSystem;

impl System for CharacterMovementSystem {
    fn run(&mut self, world: &mut hecs::World, resources: &mut Resources) {
        let dt = resources.get::<Time>().map_or(0.0, |t| t.delta);
        let Some(physics) = resources.get_mut::<PhysicsWorld>() else {
            return;
        };
        let down = glm::vec3(0.0, 0.0, -1.0);
        let mut turned: Vec<hecs::Entity> = Vec::new();

        for (entity, (transform, rb, cm, collider)) in world.query_mut::<(
            &mut Transform,
            &RigidBody,
            &mut CharacterMovement,
            Option<&Collider>,
        )>() {
            let Some(handle) = rb.physics_handle() else {
                continue;
            };
            let Some(vel) = physics.linear_velocity(handle) else {
                continue;
            };
            let centre = transform.position;
            let feet = feet_below_centre(collider);

            // Right after a jump the fixed-rate step may not have moved the
            // body yet, so the probe would still report ground; hold it off.
            cm.jump_hold = (cm.jump_hold - dt).max(0.0);
            // Probe down to `step_height` past the feet so walking off a
            // tread stays grounded and snaps down instead of free-falling.
            let ground = if cm.jump_hold > 0.0 {
                None
            } else {
                physics.raycast_filtered(centre, down, feet + cm.step_height, Some(handle))
            };
            let grounded = ground.is_some();

            let target_speed = if cm.run { cm.run_speed } else { cm.walk_speed };
            let desired = glm::vec2(cm.desired_dir[0], cm.desired_dir[1]) * target_speed;
            let xy = accelerate_toward(
                glm::vec2(vel.x, vel.y),
                desired,
                cm.accel,
                cm.decel,
                grounded,
                dt,
            );
            let speed = xy.norm();

            let mut vz = vel.z;
            if let Some(hit) = ground.as_ref() {
                if cm.jump_requested {
                    vz = cm.jump_speed;
                    cm.jump_hold = JUMP_HOLD_SECS;
                } else {
                    // Follow the ground: ride the slope with the horizontal
                    // velocity and pull a floating body down onto it.
                    vz = slope_vz(&xy, &hit.normal, vel.z) + snap_vz(hit.distance, feet, dt);
                    if speed > MIN_STEP_SPEED {
                        let heading = glm::vec3(xy.x / speed, xy.y / speed, 0.0);
                        let knee = centre - glm::vec3(0.0, 0.0, feet - KNEE_ABOVE_FEET);
                        let knee_blocked = physics
                            .raycast_filtered(knee, heading, STEP_PROBE_LEN, Some(handle))
                            .is_some();
                        let step_clear = knee_blocked && {
                            let step = centre - glm::vec3(0.0, 0.0, feet - cm.step_height);
                            physics
                                .raycast_filtered(step, heading, STEP_PROBE_LEN, Some(handle))
                                .is_none()
                        };
                        if wants_step_assist(grounded, speed, knee_blocked, step_clear) {
                            vz = STEP_ASSIST_VZ;
                        }
                    }
                }
            }
            cm.jump_requested = false;

            let new_vel = glm::vec3(xy.x, xy.y, vz);
            physics.set_linear_velocity(handle, new_vel);

            if speed > MIN_TURN_SPEED {
                let yaw = turn_toward(
                    yaw_of(&transform.rotation),
                    xy.y.atan2(xy.x),
                    cm.turn_rate_deg.to_radians() * dt,
                );
                let rot = glm::quat_angle_axis(yaw, &glm::vec3(0.0, 0.0, 1.0));
                transform.rotation = rot;
                // The step copies the body's rotation back into the
                // transform, so the body must carry the new yaw too.
                physics.set_rotation(handle, &rot);
                turned.push(entity);
            }

            cm.grounded = grounded;
            cm.horizontal_speed = speed;
            cm.velocity = [new_vel.x, new_vel.y, new_vel.z];
        }

        for entity in turned {
            mark_transform_dirty(world, entity);
        }
    }

    fn name(&self) -> &str {
        system_names::CHARACTER_MOVEMENT
    }
}

impl CharacterMovementSystem {
    pub fn descriptor() -> SystemDescriptor {
        SystemDescriptor::new(system_names::CHARACTER_MOVEMENT)
            .reads_resource::<Time>()
            .writes_resource::<PhysicsWorld>()
            .writes::<Transform>()
            .writes::<TransformDirty>()
            .writes::<CharacterMovement>()
            .reads::<RigidBody>()
            .reads::<Collider>()
            .after(system_names::PLAYER_INPUT)
            // D11: this frame's velocity and facing must be visible to foot
            // placement, the anim graph and the step — all of which touch
            // `PhysicsWorld` or `Transform` in the same stage.
            .before(system_names::FOOT_PLACEMENT)
            .before(system_names::ANIM_GRAPH)
            .before(system_names::PHYSICS_STEP)
    }
}

/// Move the horizontal velocity toward `desired`: at `accel` while there is
/// input, at `decel` when coasting to a stop on the ground. Airborne with no
/// input the velocity is kept — braking mid-jump would kill the arc.
pub fn accelerate_toward(
    current: glm::Vec2,
    desired: glm::Vec2,
    accel: f32,
    decel: f32,
    grounded: bool,
    dt: f32,
) -> glm::Vec2 {
    let has_input = desired.norm_squared() > 1e-6;
    let rate = match (has_input, grounded) {
        (true, _) => accel,
        (false, true) => decel,
        (false, false) => return current,
    };
    let delta = desired - current;
    let dist = delta.norm();
    let max_step = rate * dt;
    if dist <= max_step {
        desired
    } else {
        current + delta * (max_step / dist)
    }
}

/// Feet below the body centre: the capsule's `half_height + radius`, or the
/// D1 default when the entity carries no capsule.
pub fn feet_below_centre(collider: Option<&Collider>) -> f32 {
    match collider.map(|c| &c.shape) {
        Some(ColliderShape::Capsule {
            half_height,
            radius,
        }) => half_height + radius,
        _ => DEFAULT_FEET_BELOW_CENTRE,
    }
}

/// Vertical velocity that keeps a horizontal velocity on the ground plane
/// with unit `normal` (uphill positive, downhill negative). Too-steep
/// normals are walls: the current `vz` is kept.
pub fn slope_vz(xy: &glm::Vec2, normal: &glm::Vec3, current_vz: f32) -> f32 {
    if normal.z < MIN_WALKABLE_NZ {
        return current_vz;
    }
    -(normal.x * xy.x + normal.y * xy.y) / normal.z
}

/// Downward velocity closing the gap between the probe hit and the feet
/// within one frame (rate-limited), zero when already in contact.
pub fn snap_vz(hit_distance: f32, feet: f32, dt: f32) -> f32 {
    let gap = hit_distance - feet;
    if gap <= 0.0 || dt <= 0.0 {
        return 0.0;
    }
    -(gap / dt).min(GROUND_SNAP_MAX_VZ)
}

/// Step assist decision (D2): grounded and moving, a knee-height probe is
/// blocked ahead, and the same probe at `step_height` above the feet is clear.
pub fn wants_step_assist(
    grounded: bool,
    horizontal_speed: f32,
    knee_blocked: bool,
    step_clear: bool,
) -> bool {
    grounded && horizontal_speed > MIN_STEP_SPEED && knee_blocked && step_clear
}

/// Shortest-arc turn of `yaw` toward `target`, at most `max_delta` radians.
pub fn turn_toward(yaw: f32, target: f32, max_delta: f32) -> f32 {
    let diff = wrap_angle(target - yaw);
    wrap_angle(yaw + diff.clamp(-max_delta, max_delta))
}

/// Heading of a Z-up rotation: the angle of its rotated +X in the XY plane.
pub fn yaw_of(rotation: &glm::Quat) -> f32 {
    let fwd = glm::quat_rotate_vec3(rotation, &glm::vec3(1.0, 0.0, 0.0));
    fwd.y.atan2(fwd.x)
}

fn wrap_angle(a: f32) -> f32 {
    (a + PI).rem_euclid(TAU) - PI
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::FRAC_PI_2;

    #[test]
    fn accelerates_toward_desired_and_snaps_when_close() {
        let v = accelerate_toward(glm::vec2(0.0, 0.0), glm::vec2(4.0, 0.0), 20.0, 30.0, true, 0.1);
        assert!((v.x - 2.0).abs() < 1e-5 && v.y.abs() < 1e-5, "{v:?}");
        let v = accelerate_toward(v, glm::vec2(4.0, 0.0), 20.0, 30.0, true, 0.5);
        assert_eq!(v, glm::vec2(4.0, 0.0), "reaches the target without overshoot");
    }

    #[test]
    fn decelerates_on_ground_and_coasts_in_air() {
        let moving = glm::vec2(3.0, 0.0);
        let v = accelerate_toward(moving, glm::vec2(0.0, 0.0), 20.0, 30.0, true, 0.05);
        assert!((v.x - 1.5).abs() < 1e-5, "decel 30 * 0.05 = 1.5 m/s off: {v:?}");
        let v = accelerate_toward(moving, glm::vec2(0.0, 0.0), 20.0, 30.0, false, 0.05);
        assert_eq!(v, moving, "no input in the air keeps the velocity");
        let v = accelerate_toward(moving, glm::vec2(0.0, 1.0), 20.0, 30.0, false, 0.05);
        assert!(v.y > 0.0, "air control with input still steers");
    }

    #[test]
    fn turn_toward_takes_shortest_arc_and_clamps_rate() {
        assert!((turn_toward(0.0, FRAC_PI_2, 0.1) - 0.1).abs() < 1e-6);
        assert!((turn_toward(0.0, -FRAC_PI_2, 0.1) + 0.1).abs() < 1e-6);
        // 170° → -170° is a 20° turn through ±180°, not 340° the long way.
        let from = 170.0_f32.to_radians();
        let to = -170.0_f32.to_radians();
        let mid = turn_toward(from, to, 10.0_f32.to_radians());
        assert!((mid - 180.0_f32.to_radians()).abs() < 1e-5 || (mid + PI).abs() < 1e-5);
        assert!((turn_toward(from, to, 1.0) - to).abs() < 1e-5, "reaches the target");
    }

    #[test]
    fn yaw_of_zup_rotation_roundtrips() {
        for deg in [-170.0_f32, -45.0, 0.0, 30.0, 120.0] {
            let q = glm::quat_angle_axis(deg.to_radians(), &glm::vec3(0.0, 0.0, 1.0));
            assert!((yaw_of(&q) - deg.to_radians()).abs() < 1e-5, "{deg}");
        }
    }

    #[test]
    fn feet_come_from_the_capsule_or_the_default() {
        let capsule = Collider::capsule(0.6, 0.3);
        assert!((feet_below_centre(Some(&capsule)) - 0.9).abs() < 1e-6);
        let cube = Collider::cuboid(1.0, 1.0, 1.0);
        assert_eq!(feet_below_centre(Some(&cube)), DEFAULT_FEET_BELOW_CENTRE);
        assert_eq!(feet_below_centre(None), DEFAULT_FEET_BELOW_CENTRE);
    }

    #[test]
    fn slope_projection_rides_the_ground_and_ignores_walls() {
        let flat = glm::vec3(0.0, 0.0, 1.0);
        assert_eq!(slope_vz(&glm::vec2(3.0, 0.0), &flat, -5.0), 0.0);
        // 30° ramp rising along +X: normal tilts back toward -X.
        let a = 30.0_f32.to_radians();
        let ramp = glm::vec3(-a.sin(), 0.0, a.cos());
        let up = slope_vz(&glm::vec2(4.5, 0.0), &ramp, 0.0);
        assert!((up - 4.5 * a.tan()).abs() < 1e-4, "uphill vz {up}");
        let down = slope_vz(&glm::vec2(-4.5, 0.0), &ramp, 0.0);
        assert!((down + 4.5 * a.tan()).abs() < 1e-4, "downhill vz {down}");
        let wall = glm::vec3(-1.0, 0.0, 0.1);
        assert_eq!(slope_vz(&glm::vec2(1.0, 0.0), &wall, -2.0), -2.0, "wall keeps vz");
    }

    #[test]
    fn ground_snap_closes_a_gap_and_rests_on_contact() {
        assert_eq!(snap_vz(0.9, 0.9, 0.016), 0.0, "in contact");
        assert_eq!(snap_vz(0.85, 0.9, 0.016), 0.0, "penetrating: the solver handles it");
        let v = snap_vz(0.92, 0.9, 0.01);
        assert!((v + 2.0).abs() < 1e-4, "2 cm in 10 ms = -2 m/s: {v}");
        assert_eq!(snap_vz(1.2, 0.9, 0.01), -GROUND_SNAP_MAX_VZ, "rate-limited");
    }

    #[test]
    fn step_assist_needs_ground_motion_and_a_clear_step() {
        assert!(wants_step_assist(true, 1.0, true, true));
        assert!(!wants_step_assist(false, 1.0, true, true), "airborne");
        assert!(!wants_step_assist(true, 0.05, true, true), "standing still");
        assert!(!wants_step_assist(true, 1.0, false, true), "nothing ahead");
        assert!(!wants_step_assist(true, 1.0, true, false), "a wall, not a step");
    }
}
