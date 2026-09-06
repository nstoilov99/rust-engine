//! Third-person orbit camera (Task 41.6 D3).
//!
//! Per `(Transform, Camera, OrbitCamera)` entity: the `look` axis turns
//! yaw/pitch, scroll zooms the boom, the pivot sits `pivot_height` above the
//! target and `shoulder` to its right, and the boom is shortened by a ray
//! from the pivot so walls never sit between camera and character. The
//! Camera entity's `Transform` is written directly — no hierarchy, the boom
//! is computed.
//!
//! Rotation convention: both hosts derive the view from the entity's render
//! matrix as forward = −column 2, which in Z-up is the entity's **local +X**.
//! `look_rotation` therefore aims local +X at the pivot with local +Z kept
//! roll-free (right = local +Y).
//!
//! Runs in `Update`: after the physics step (the target's body has moved)
//! and before transform propagation (the viewport follows this frame).

use game_shared::components::{CharacterMovement, OrbitCamera, PlayerInput};
use nalgebra_glm as glm;
use rust_engine::engine::ecs::access::SystemDescriptor;
use rust_engine::engine::ecs::components::{Camera, EntityGuid, Transform, TransformDirty};
use rust_engine::engine::ecs::hierarchy::mark_transform_dirty;
use rust_engine::engine::ecs::resources::Resources;
use rust_engine::engine::ecs::schedule::System;
use rust_engine::engine::ecs::system_names;
use rust_engine::engine::input::subsystem::InputSubsystem;
use rust_engine::engine::physics::{PhysicsWorld, RigidBody, RigidBodyHandle};
use rust_engine::InputManager;
use uuid::Uuid;

/// Clearance kept between a boom hit and the camera, metres.
const BOOM_PADDING: f32 = 0.2;
/// Boom length change per scroll line, metres (scroll up zooms in).
const ZOOM_PER_LINE: f32 = 0.5;
/// Enhanced Input action used when the target carries no `PlayerInput`.
const DEFAULT_LOOK_ACTION: &str = "look";
/// The graph runner writes `Transform` in the same stage; a graph should see
/// this frame's movement and the camera should follow the graph's result.
#[cfg(feature = "graph-scripting")]
const GRAPH_SCRIPT_RUNNER: &str = "GraphScriptRunnerSystem";

/// What the camera needs from the entity it follows.
struct Target {
    position: glm::Vec3,
    body: Option<RigidBodyHandle>,
    look_action: String,
}

pub struct OrbitCameraSystem;

impl System for OrbitCameraSystem {
    fn run(&mut self, world: &mut hecs::World, resources: &mut Resources) {
        let cams: Vec<(hecs::Entity, Option<Uuid>)> = world
            .query::<(&Camera, &OrbitCamera)>()
            .with::<&Transform>()
            .iter()
            .map(|(e, (_, oc))| (e, oc.target))
            .collect();
        if cams.is_empty() {
            return;
        }
        let scroll = resources
            .get::<InputManager>()
            .map_or(0.0, |im| im.scroll_delta());
        let Some(input) = resources.get::<InputSubsystem>() else {
            return;
        };
        let Some(physics) = resources.get::<PhysicsWorld>() else {
            return;
        };

        let mut moved = Vec::with_capacity(cams.len());
        for (entity, target_guid) in cams {
            let Some(target) = resolve_target(world, target_guid) else {
                continue;
            };
            let look = input.axis_2d(&target.look_action);
            let Ok((transform, oc)) =
                world.query_one_mut::<(&mut Transform, &mut OrbitCamera)>(entity)
            else {
                continue;
            };

            oc.yaw = wrap_angle(oc.yaw + look.0 * oc.sensitivity);
            oc.pitch = clamp_pitch(
                oc.pitch - look.1 * oc.sensitivity,
                oc.pitch_min_deg,
                oc.pitch_max_deg,
            );
            oc.distance = (oc.distance - scroll * ZOOM_PER_LINE)
                .clamp(oc.min_distance, oc.max_distance);

            let pivot = boom_pivot(target.position, oc.yaw, oc.pivot_height, oc.shoulder);
            let fwd = forward(oc.yaw, oc.pitch);
            let hit = physics
                .raycast_filtered(pivot, -fwd, oc.distance, target.body)
                .map(|h| h.distance);
            let len = boom_length(oc.distance, hit, oc.min_distance);

            transform.position = pivot - fwd * len;
            transform.rotation = look_rotation(oc.yaw, oc.pitch);
            moved.push(entity);
        }

        for entity in moved {
            mark_transform_dirty(world, entity);
        }
    }

    fn name(&self) -> &str {
        system_names::ORBIT_CAMERA
    }
}

impl OrbitCameraSystem {
    pub fn descriptor() -> SystemDescriptor {
        let d = SystemDescriptor::new(system_names::ORBIT_CAMERA)
            .reads_resource::<InputSubsystem>()
            // Scroll is not an Enhanced Input action; read the raw delta.
            .reads_resource::<InputManager>()
            .reads_resource::<PhysicsWorld>()
            .writes::<Transform>()
            .writes::<TransformDirty>()
            .writes::<OrbitCamera>()
            .reads::<Camera>()
            .reads::<EntityGuid>()
            .reads::<RigidBody>()
            .reads::<CharacterMovement>()
            .reads::<PlayerInput>()
            .after(system_names::PHYSICS_STEP)
            .before(system_names::TRANSFORM_PROPAGATION);
        #[cfg(feature = "graph-scripting")]
        let d = d.after(GRAPH_SCRIPT_RUNNER);
        d
    }
}

/// The followed entity: the one whose `EntityGuid` matches `target`, else
/// the first `CharacterMovement` entity. Its position is `Transform.position`
/// (the player is a root entity), its body excludes itself from the boom ray.
fn resolve_target(world: &hecs::World, target: Option<Uuid>) -> Option<Target> {
    let make = |t: &Transform, rb: Option<&RigidBody>, pi: Option<&PlayerInput>| Target {
        position: t.position,
        body: rb.and_then(RigidBody::physics_handle),
        look_action: pi.map_or(DEFAULT_LOOK_ACTION.to_string(), |p| p.look_action.clone()),
    };
    if let Some(guid) = target {
        let found = world
            .query::<(&EntityGuid, &Transform, Option<&RigidBody>, Option<&PlayerInput>)>()
            .iter()
            .find(|(_, (g, ..))| g.0 == guid)
            .map(|(_, (_, t, rb, pi))| make(t, rb, pi));
        if found.is_some() {
            return found;
        }
    }
    world
        .query::<(&CharacterMovement, &Transform, Option<&RigidBody>, Option<&PlayerInput>)>()
        .iter()
        .next()
        .map(|(_, (_, t, rb, pi))| make(t, rb, pi))
}

/// Unit view direction for a heading `yaw` (0 = +X) and elevation `pitch`
/// (positive = up), Z-up.
pub fn forward(yaw: f32, pitch: f32) -> glm::Vec3 {
    let (sy, cy) = yaw.sin_cos();
    let (sp, cp) = pitch.sin_cos();
    glm::vec3(cp * cy, cp * sy, sp)
}

/// Camera right for a heading `yaw`: +Y at yaw 0.
pub fn right(yaw: f32) -> glm::Vec3 {
    let (sy, cy) = yaw.sin_cos();
    glm::vec3(-sy, cy, 0.0)
}

/// Pivot: `pivot_height` above the target's origin, `shoulder` along the
/// camera's right.
pub fn boom_pivot(target: glm::Vec3, yaw: f32, pivot_height: f32, shoulder: f32) -> glm::Vec3 {
    target + glm::vec3(0.0, 0.0, pivot_height) + right(yaw) * shoulder
}

/// Boom length after collision: a hit at `hit_distance` pulls the camera to
/// `BOOM_PADDING` before it, never closer than `min_distance` and never
/// further than `distance`.
pub fn boom_length(distance: f32, hit_distance: Option<f32>, min_distance: f32) -> f32 {
    match hit_distance {
        Some(h) => (h - BOOM_PADDING).max(min_distance).min(distance),
        None => distance,
    }
}

/// Roll-free rotation aiming local +X along `forward(yaw, pitch)` with local
/// +Y kept horizontal (the render side reads forward from local +X).
pub fn look_rotation(yaw: f32, pitch: f32) -> glm::Quat {
    // Pitch about local Y first (positive pitch = nose up = rotate X toward
    // +Z, which is a negative rotation about +Y), then yaw about world Z.
    glm::quat_angle_axis(yaw, &glm::vec3(0.0, 0.0, 1.0))
        * glm::quat_angle_axis(-pitch, &glm::vec3(0.0, 1.0, 0.0))
}

/// Clamp a pitch in radians to degree limits.
pub fn clamp_pitch(pitch: f32, min_deg: f32, max_deg: f32) -> f32 {
    pitch.clamp(min_deg.to_radians(), max_deg.to_radians())
}

fn wrap_angle(a: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    (a + PI).rem_euclid(TAU) - PI
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::FRAC_PI_2;

    fn close(a: glm::Vec3, b: glm::Vec3) -> bool {
        (a - b).norm() < 1e-5
    }

    #[test]
    fn forward_and_right_follow_the_zup_convention() {
        assert!(close(forward(0.0, 0.0), glm::vec3(1.0, 0.0, 0.0)));
        assert!(close(forward(FRAC_PI_2, 0.0), glm::vec3(0.0, 1.0, 0.0)));
        assert!(close(forward(0.0, FRAC_PI_2), glm::vec3(0.0, 0.0, 1.0)), "pitch up is +Z");
        assert!(close(right(0.0), glm::vec3(0.0, 1.0, 0.0)));
        assert!(close(right(FRAC_PI_2), glm::vec3(-1.0, 0.0, 0.0)));
    }

    #[test]
    fn pivot_is_above_and_to_the_right_of_the_target() {
        let p = boom_pivot(glm::vec3(1.0, 2.0, 0.0), 0.0, 1.4, 0.4);
        assert!(close(p, glm::vec3(1.0, 2.4, 1.4)));
        let p = boom_pivot(glm::vec3(0.0, 0.0, 0.0), FRAC_PI_2, 1.4, 0.4);
        assert!(close(p, glm::vec3(-0.4, 0.0, 1.4)));
    }

    #[test]
    fn boom_shortens_on_hit_with_padding_and_floors_at_min() {
        assert_eq!(boom_length(3.5, None, 1.5), 3.5);
        assert!((boom_length(3.5, Some(2.0), 1.5) - 1.8).abs() < 1e-6);
        assert_eq!(boom_length(3.5, Some(0.1), 1.5), 1.5, "never below min_distance");
        assert_eq!(boom_length(3.5, Some(3.5), 1.5), 3.3);
    }

    #[test]
    fn look_rotation_aims_local_x_at_forward_and_stays_level() {
        for (yaw, pitch) in [(0.0, 0.0), (0.7, 0.3), (-2.0, -0.9), (3.0, 1.1)] {
            let q = look_rotation(yaw, pitch);
            let fwd = glm::quat_rotate_vec3(&q, &glm::vec3(1.0, 0.0, 0.0));
            assert!(close(fwd, forward(yaw, pitch)), "yaw {yaw} pitch {pitch}: {fwd:?}");
            let rgt = glm::quat_rotate_vec3(&q, &glm::vec3(0.0, 1.0, 0.0));
            assert!(close(rgt, right(yaw)), "right stays horizontal: {rgt:?}");
            let up = glm::quat_rotate_vec3(&q, &glm::vec3(0.0, 0.0, 1.0));
            assert!(up.z > 0.0, "never upside down");
        }
    }

    #[test]
    fn camera_sits_behind_the_pivot() {
        let pivot = boom_pivot(glm::vec3(0.0, 0.0, 0.0), 0.0, 1.4, 0.0);
        let cam = pivot - forward(0.0, 0.0) * 3.5;
        assert!(close(cam, glm::vec3(-3.5, 0.0, 1.4)));
    }

    #[test]
    fn pitch_clamps_to_degree_limits() {
        assert!((clamp_pitch(2.0, -60.0, 70.0) - 70.0_f32.to_radians()).abs() < 1e-6);
        assert!((clamp_pitch(-2.0, -60.0, 70.0) + 60.0_f32.to_radians()).abs() < 1e-6);
        assert_eq!(clamp_pitch(0.1, -60.0, 70.0), 0.1);
    }
}
