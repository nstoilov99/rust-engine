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
/// Capsule radius assumed when the entity has no capsule collider.
const DEFAULT_CAPSULE_RADIUS: f32 = 0.4;
/// The step lift aims the feet this far above the step's top.
const STEP_CLEARANCE: f32 = 0.03;
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
/// Gap between the probe hit and the feet that still counts as contact.
const GROUND_SNAP_DEADBAND: f32 = 0.02;
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
            let (feet, radius) = capsule_dims(collider);

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
            let feet_z = centre.z - feet;
            // A step lift in progress ends when the ground under the body
            // centre *is* the step top (the snap would otherwise drop the
            // body back onto the lower tread while the capsule still
            // overhangs the edge), or when the input stops or a jump starts.
            if let Some(target) = cm.step_lift_target {
                let ground_below = ground.as_ref().map(|h| centre.z - h.distance);
                let over_step = ground_below.is_some_and(|z| z >= target - STEP_CLEARANCE - 0.01);
                if over_step || speed <= MIN_STEP_SPEED || cm.jump_hold > 0.0 {
                    cm.step_lift_target = None;
                }
            }
            if let Some(hit) = ground.as_ref() {
                if cm.jump_requested {
                    vz = cm.jump_speed;
                    cm.jump_hold = JUMP_HOLD_SECS;
                    cm.step_lift_target = None;
                } else if let Some(target) = cm.step_lift_target {
                    // Rise until the feet clear the step, then hold height
                    // (no snap) and glide until the centre is over it.
                    vz = if feet_z < target { STEP_ASSIST_VZ } else { 0.0 };
                } else {
                    // The surface to follow is the one under the body — or,
                    // when a walkable slope starts just ahead, that slope,
                    // so the body rides onto it instead of being pressed
                    // into its foot by the flat-ground projection.
                    let mut follow = hit.normal;
                    let mut snap = true;
                    if speed > MIN_STEP_SPEED {
                        let heading = glm::vec3(xy.x / speed, xy.y / speed, 0.0);
                        let knee = centre - glm::vec3(0.0, 0.0, feet - KNEE_ABOVE_FEET);
                        let ahead = physics.raycast_filtered(knee, heading, STEP_PROBE_LEN, Some(handle));
                        match ahead {
                            Some(h) if is_step_face(&h.normal) => {
                                // A steep face at knee height: a step if the
                                // same probe at `step_height` is clear.
                                let step = centre - glm::vec3(0.0, 0.0, feet - cm.step_height);
                                let step_clear = physics
                                    .raycast_filtered(step, heading, STEP_PROBE_LEN, Some(handle))
                                    .is_none();
                                if wants_step_assist(grounded, speed, true, step_clear) {
                                    // Find the step's top: probe down from
                                    // just above step height, ahead of the body.
                                    let above = centre + heading * STEP_PROBE_LEN
                                        - glm::vec3(0.0, 0.0, feet - cm.step_height - 0.01);
                                    let top = physics
                                        .raycast_filtered(above, down, cm.step_height + 0.02, Some(handle))
                                        .map(|h| h.point.z)
                                        .unwrap_or(feet_z + cm.step_height);
                                    cm.step_lift_target = Some(top + STEP_CLEARANCE);
                                }
                            }
                            Some(h) => {
                                // A walkable slope ahead: ride it, no snap
                                // while transitioning onto it.
                                follow = h.normal;
                                snap = false;
                            }
                            None => {}
                        }
                    }
                    vz = match cm.step_lift_target {
                        Some(_) => STEP_ASSIST_VZ,
                        None => {
                            // On a slope the straight-down probe is longer
                            // than the resting distance, so the snap measures
                            // against the slope-corrected feet height.
                            let rest = feet_on_slope(feet, radius, &hit.normal);
                            slope_vz(&xy, &follow, vel.z)
                                + if snap { snap_vz(hit.distance, rest, dt) } else { 0.0 }
                        }
                    };
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

/// `(feet below the body centre, radius)`: the capsule's `half_height +
/// radius` and `radius`, or the D1 defaults when the entity carries no capsule.
pub fn capsule_dims(collider: Option<&Collider>) -> (f32, f32) {
    match collider.map(|c| &c.shape) {
        Some(ColliderShape::Capsule {
            half_height,
            radius,
        }) => (half_height + radius, *radius),
        _ => (DEFAULT_FEET_BELOW_CENTRE, DEFAULT_CAPSULE_RADIUS),
    }
}

/// Feet below the body centre: see [`capsule_dims`].
pub fn feet_below_centre(collider: Option<&Collider>) -> f32 {
    capsule_dims(collider).0
}

/// Vertical distance from the centre to the ground directly below when a
/// capsule rests on a plane with unit `normal`: the contact sits off-axis on
/// a slope, so the straight-down distance grows by `radius (1/n.z − 1)`.
/// Walls (too steep) return `feet` unchanged.
pub fn feet_on_slope(feet: f32, radius: f32, normal: &glm::Vec3) -> f32 {
    if normal.z < MIN_WALKABLE_NZ {
        return feet;
    }
    feet + radius * (1.0 / normal.z - 1.0)
}

/// A knee-probe hit counts as a step face only when it is too steep to
/// walk; a walkable slope ahead is ridden by the slope projection instead.
pub fn is_step_face(normal: &glm::Vec3) -> bool {
    normal.z < MIN_WALKABLE_NZ
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
/// within one frame (rate-limited), zero when already in contact. Gaps
/// inside the dead band are contact-solver slack, not floating: snapping on
/// them would bob the body (and every IK target on it) every frame.
pub fn snap_vz(hit_distance: f32, feet: f32, dt: f32) -> f32 {
    let gap = hit_distance - feet;
    if gap <= GROUND_SNAP_DEADBAND || dt <= 0.0 {
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
    fn slope_rest_distance_grows_with_tilt_and_steps_are_steep_faces() {
        let flat = glm::vec3(0.0, 0.0, 1.0);
        assert_eq!(feet_on_slope(0.9, 0.4, &flat), 0.9);
        let a = 30.0_f32.to_radians();
        let ramp = glm::vec3(-a.sin(), 0.0, a.cos());
        let rest = feet_on_slope(0.9, 0.4, &ramp);
        assert!((rest - (0.9 + 0.4 * (1.0 / a.cos() - 1.0))).abs() < 1e-5, "{rest}");
        assert!(rest > 0.9 && rest < 1.0, "a few cm longer on 30°: {rest}");
        let wall = glm::vec3(-1.0, 0.0, 0.0);
        assert_eq!(feet_on_slope(0.9, 0.4, &wall), 0.9, "walls are not slopes");
        assert!(is_step_face(&wall));
        assert!(!is_step_face(&ramp), "a 30° ramp is walkable, not a step");
        assert!(!is_step_face(&flat));
        assert_eq!(capsule_dims(None), (DEFAULT_FEET_BELOW_CENTRE, DEFAULT_CAPSULE_RADIUS));
        assert_eq!(capsule_dims(Some(&Collider::capsule(0.5, 0.4))), (0.9, 0.4));
    }

    #[test]
    fn ground_snap_closes_a_gap_and_rests_on_contact() {
        assert_eq!(snap_vz(0.9, 0.9, 0.016), 0.0, "in contact");
        assert_eq!(snap_vz(0.85, 0.9, 0.016), 0.0, "penetrating: the solver handles it");
        assert_eq!(snap_vz(0.915, 0.9, 0.016), 0.0, "1.5 cm is solver slack, not floating");
        let v = snap_vz(0.94, 0.9, 0.02);
        assert!((v + 2.0).abs() < 1e-4, "4 cm in 20 ms = -2 m/s: {v}");
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
