//! Foot placement (Task 41.5 P6, I-D4): rays the ground under each foot
//! chain, writes the chain's `IkTargets` entry, locks planted feet on anim
//! event edges, and drives the cosmetic pelvis drop.
//!
//! Serial by design, scheduled immediately **before** `AnimGraphSystem`: it
//! reads `Resources` (physics world, transform cache), reads last tick's
//! anim event fires off each runtime, and only ever writes component state —
//! `IkTargets` entries, per-foot lock state on the armed chains, the pelvis
//! offsets, and the P4 `force_eval_external` hook (serial-side only, per the
//! throttling contract). The runner does all model-space work; this system
//! never touches `SkeletonInstance` directly — it rays from the *animated*
//! tip positions `apply_ik` recorded last evaluation.
//!
//! Configuration lives on the graph's IK Chain node (`foot`, `ankle_offset`,
//! `pelvis` properties — see `plan.rs`), so a character opts in wherever its
//! chains are declared; no extra component to attach. The system inserts
//! `IkTargets` itself when missing.

use crate::engine::animation::graph::machine::AnimEventFire;
use crate::engine::animation::graph::runner::{
    AnimGraphRuntime, HeldContact, IkTarget, IkTargets,
};
use crate::engine::ecs::components::Transform;
use crate::engine::ecs::hierarchy::TransformCache;
use crate::engine::ecs::resources::{Resources, Time};
use crate::engine::ecs::schedule::System;
use crate::engine::physics::{PhysicsWorld, RigidBody};
use crate::engine::utils::coords::{convert_position_yup_to_zup, convert_position_zup_to_yup};
use nalgebra_glm as glm;

/// Tuning constants for I-D4, in one place. World units ≈ meters.
/// Ray start height above the animated foot (covers a raised step).
const RAY_START_ABOVE: f32 = 0.5;
/// Probe depth below the animated foot (step-down / downhill slope reach).
const STEP_HEIGHT: f32 = 0.6;
/// Total ray length.
const RAY_LENGTH: f32 = RAY_START_ABOVE + STEP_HEIGHT;
/// Knee pole point: this far ahead of the foot along character forward…
const POLE_FORWARD: f32 = 1.0;
/// …and this far up — in front of the knee, so the leg bends forward.
const POLE_UP: f32 = 0.5;
/// The pelvis never drops further than this.
const MAX_PELVIS_DROP: f32 = 0.5;
/// Exponential approach rate (1/s) for the pelvis drop.
const PELVIS_RATE: f32 = 10.0;
const PELVIS_EPSILON: f32 = 1e-3;

/// Anim event name convention: `<chain name>_down` plants, `<chain name>_up`
/// releases.
pub const FOOT_EVENT_DOWN_SUFFIX: &str = "_down";
pub const FOOT_EVENT_UP_SUFFIX: &str = "_up";

/// Did last tick fire `<chain><suffix>`? Allocation-free.
fn fired(events: &[AnimEventFire], chain: &str, suffix: &str) -> bool {
    events
        .iter()
        .any(|e| e.name.strip_prefix(chain).is_some_and(|rest| rest == suffix))
}

/// One entity's foot-placement step — the injectable core (tests script
/// `ray`, the system wires it to [`PhysicsWorld::raycast_filtered`]).
///
/// `ray` takes a world Z-up origin, casts straight down [`RAY_LENGTH`], and
/// returns `(hit point, surface normal)` in world Z-up. `active` is the
/// I-D5 gate: raycasts run only in the top significance bucket — when it is
/// false the stale targets are *removed* (the chains then skip their solves
/// entirely: no target ⇒ no writes), locks release, the pelvis returns, and
/// one forced evaluation snaps the pose back to animated instead of holding
/// a half-corrected pose forever.
pub fn place_feet(
    rt: &mut AnimGraphRuntime,
    targets: &mut IkTargets,
    entity_render: glam::Mat4,
    forward_zup: glam::Vec3,
    dt: f32,
    active: bool,
    ray: &mut dyn FnMut(glam::Vec3) -> Option<(glam::Vec3, glam::Vec3)>,
) {
    if !active {
        let mut changed = false;
        for chain in &mut rt.ik {
            let Some(foot) = &mut chain.foot else { continue };
            changed |= targets.targets.remove(&chain.name).is_some() || foot.locked;
            foot.locked = false;
            foot.held = None;
        }
        if let Some(p) = &mut rt.pelvis {
            changed |= p.offset < -PELVIS_EPSILON;
            p.offset = 0.0;
            p.model_offset = glam::Vec3::ZERO;
        }
        if changed {
            rt.throttle.force_eval_external = true;
        }
        return;
    }

    let inv = entity_render.inverse();
    let entity_z = convert_position_yup_to_zup(entity_render.w_axis.truncate()).z;
    // The lowest contact below the entity's ground plane decides the pelvis
    // drop (0 = nothing below). The reference is the entity's plane, not the
    // foot's current height: clips plant feet on that plane, and the swing
    // foot's lift must not read as ground dropping away.
    let mut lowest = 0.0f32;
    for chain in &mut rt.ik {
        let Some(foot) = &mut chain.foot else { continue };
        // The animated (pre-IK) foot from the last evaluation — absent only
        // before the first one.
        let fresh = chain.animated_tip.and_then(|tip| {
            let foot_world =
                convert_position_yup_to_zup((entity_render * tip.extend(1.0)).truncate());
            let (point, normal) = ray(foot_world + glam::Vec3::Z * RAY_START_ABOVE)?;
            Some(HeldContact {
                target: IkTarget {
                    effector: point + normal * foot.ankle_offset,
                    pole: foot_world + forward_zup * POLE_FORWARD + glam::Vec3::Z * POLE_UP,
                },
                contact_z: point.z,
            })
        });

        // Lock edges (`<chain>_down` / `<chain>_up`, last tick's fires).
        // Either edge forces one full evaluation — the P4 hook, serial-side.
        // A down edge with no ground under the foot does not latch.
        if fired(&rt.events, &chain.name, FOOT_EVENT_DOWN_SUFFIX) {
            if let Some(c) = fresh {
                foot.locked = true;
                foot.held = Some(c);
            }
            rt.throttle.force_eval_external = true;
        }
        if fired(&rt.events, &chain.name, FOOT_EVENT_UP_SUFFIX) {
            foot.locked = false;
            foot.held = None;
            rt.throttle.force_eval_external = true;
        }

        match if foot.locked { foot.held.or(fresh) } else { fresh } {
            Some(c) => {
                // Upsert without allocating at steady state.
                match targets.targets.get_mut(&chain.name) {
                    Some(t) => *t = c.target,
                    None => {
                        targets.targets.insert(chain.name.clone(), c.target);
                    }
                }
                lowest = lowest.min(c.contact_z - entity_z);
            }
            // No ground under the foot (mid-air, past a ledge): no target,
            // so the chain skips its solve — the foot stays animated.
            None => {
                targets.targets.remove(&chain.name);
            }
        }
    }

    if let Some(p) = &mut rt.pelvis {
        let goal = lowest.clamp(-MAX_PELVIS_DROP, 0.0);
        p.offset += (goal - p.offset) * (dt * PELVIS_RATE).min(1.0);
        // World drop → the mesh's Y-up model space through the entity
        // matrix; `apply_ik` adds this to the pelvis bone pre-solve.
        p.model_offset = inv.transform_vector3(convert_position_zup_to_yup(glam::Vec3::new(
            0.0, 0.0, p.offset,
        )));
    }
}

/// The system: [`place_feet`] per entity with armed foot chains, rays
/// through the physics world, excluding the entity's own rigid body.
///
/// Structural licence: inserts a default `IkTargets` on entities that need
/// one — serial work, same terms as `AnimGraphSystem`'s arming.
pub struct FootPlacementSystem {
    /// Entities missing an `IkTargets` (scratch, reused across frames).
    missing: Vec<hecs::Entity>,
}

impl FootPlacementSystem {
    pub fn new() -> Self {
        Self {
            missing: Vec::new(),
        }
    }
}

impl Default for FootPlacementSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl System for FootPlacementSystem {
    fn run(&mut self, world: &mut hecs::World, resources: &mut Resources) {
        crate::profile_scope!("foot_placement");

        let dt = resources
            .get::<Time>()
            .map(|t| t.scaled_delta())
            .unwrap_or(0.0);

        // Entities with armed foot chains get their `IkTargets` created here.
        self.missing.clear();
        self.missing.extend(
            world
                .query::<&AnimGraphRuntime>()
                .iter()
                .filter(|(e, rt)| {
                    rt.disabled.is_none()
                        && rt.ik.iter().any(|c| c.foot.is_some())
                        && world.get::<&IkTargets>(*e).is_err()
                })
                .map(|(e, _)| e),
        );
        for &e in &self.missing {
            let _ = world.insert_one(e, IkTargets::default());
        }

        let physics = resources.get::<PhysicsWorld>();
        let cache = resources.get::<TransformCache>();
        for (e, (rt, targets, transform, body)) in world.query_mut::<(
            &mut AnimGraphRuntime,
            &mut IkTargets,
            Option<&Transform>,
            Option<&RigidBody>,
        )>() {
            if rt.disabled.is_some() || !rt.ik.iter().any(|c| c.foot.is_some()) {
                continue;
            }
            // The entity's Y-up render matrix — previous frame via the
            // cache (the accepted render-path latency), the entity's own
            // `Transform` without one (tests).
            let entity_render = match cache {
                Some(c) => glam::Mat4::from_cols_slice(c.get_render(e).as_slice()),
                None => transform
                    .map(|t| glam::Mat4::from_cols_slice(t.model_matrix().as_slice()))
                    .unwrap_or(glam::Mat4::IDENTITY),
            };
            // Character forward in world Z-up (+X is forward) — the knee
            // pole sits ahead of the foot along it.
            let forward = transform
                .map(|t| {
                    let f = glm::quat_rotate_vec3(&t.rotation, &glm::vec3(1.0, 0.0, 0.0));
                    glam::Vec3::new(f.x, f.y, f.z)
                })
                .unwrap_or(glam::Vec3::X);
            let exclude = body.and_then(|b| b.handle);
            // I-D5: raycasts only in the top significance bucket.
            let active = physics.is_some() && rt.throttle.bucket == 0;
            let mut cast = |origin: glam::Vec3| {
                let hit = physics?.raycast_filtered(
                    glm::vec3(origin.x, origin.y, origin.z),
                    glm::vec3(0.0, 0.0, -1.0),
                    RAY_LENGTH,
                    exclude,
                )?;
                Some((
                    glam::Vec3::new(hit.point.x, hit.point.y, hit.point.z),
                    glam::Vec3::new(hit.normal.x, hit.normal.y, hit.normal.z),
                ))
            };
            place_feet(rt, targets, entity_render, forward, dt, active, &mut cast);
        }
    }

    fn name(&self) -> &str {
        crate::engine::ecs::system_names::FOOT_PLACEMENT
    }
}

// ---------------------------------------------------------------------------
// Tests: lock state machine + pelvis math against a scripted raycast (no
// physics world; the closure *is* the ground). The end-to-end path — armed
// chains, apply_ik consuming the pelvis offset, the throttling pin — lives
// in `graph/acceptance.rs`.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::animation::graph::machine::{
        AnimEventFire, AnimMachine, AnimParams, PlayOnceSlot,
    };
    use crate::engine::animation::graph::plan::{AnimGraphPlan, PlanIkSolver};
    use crate::engine::animation::graph::runner::{
        ArmedIkChain, FootState, PelvisState, ThrottleState,
    };
    use glam::Vec3;
    use std::sync::Arc;

    /// A runtime with one armed foot chain "foot_l" whose animated tip sits
    /// at model Y-up (0, 0.1, 0) — world Z-up (0, 0, 0.1) under an identity
    /// entity matrix — plus a pelvis on bone 0.
    fn foot_rt() -> AnimGraphRuntime {
        let plan = Arc::new(AnimGraphPlan::default());
        AnimGraphRuntime {
            graph: String::new(),
            machine: AnimMachine::new(&plan),
            slot: PlayOnceSlot::new(),
            params: AnimParams::default(),
            events: Vec::new(),
            plan,
            generation: 0,
            disabled: None,
            throttle: ThrottleState::default(),
            ik: vec![ArmedIkChain {
                name: "foot_l".into(),
                bones: vec![0, 1, 2],
                solver: PlanIkSolver::TwoBone,
                weight_param: "ik".into(),
                resolved: None,
                foot: Some(FootState {
                    ankle_offset: 0.1,
                    locked: false,
                    held: None,
                }),
                animated_tip: Some(Vec3::new(0.0, 0.1, 0.0)),
            }],
            pelvis: Some(PelvisState {
                bone: 0,
                offset: 0.0,
                model_offset: Vec3::ZERO,
            }),
            ik_touched: Vec::new(),
        }
    }

    /// Flat ground at world `z`, normal straight up.
    fn ground(z: f32) -> impl FnMut(Vec3) -> Option<(Vec3, Vec3)> {
        move |o| Some((Vec3::new(o.x, o.y, z), Vec3::Z))
    }

    fn place(
        rt: &mut AnimGraphRuntime,
        targets: &mut IkTargets,
        dt: f32,
        active: bool,
        ray: &mut dyn FnMut(Vec3) -> Option<(Vec3, Vec3)>,
    ) {
        place_feet(rt, targets, glam::Mat4::IDENTITY, Vec3::X, dt, active, ray);
    }

    fn fire(rt: &mut AnimGraphRuntime, name: &str) {
        rt.events.push(AnimEventFire {
            name: name.into(),
            weight: 1.0,
        });
    }

    #[test]
    fn a_contact_writes_the_target_with_ankle_offset_and_drops_the_pelvis() {
        let mut rt = foot_rt();
        let mut targets = IkTargets::default();
        // dt 1.0 saturates the smoothing, so the pelvis lands on its goal.
        place(&mut rt, &mut targets, 1.0, true, &mut ground(-0.3));

        let t = targets.targets.get("foot_l").expect("target written");
        assert!(
            (t.effector - Vec3::new(0.0, 0.0, -0.2)).length() < 1e-5,
            "contact −0.3 lifted 0.1 along the normal: {}",
            t.effector
        );
        // Pole: 1 m ahead of the animated foot (world (0,0,0.1)), 0.5 m up.
        assert!(
            (t.pole - Vec3::new(1.0, 0.0, 0.6)).length() < 1e-5,
            "knee-forward pole: {}",
            t.pole
        );
        let p = rt.pelvis.unwrap();
        assert!(
            (p.offset - (-0.3)).abs() < 1e-5,
            "contact 0.3 below the entity plane lowers the pelvis: {}",
            p.offset
        );
        // World Z-up drop → identity-entity model Y-up: (0, −0.3, 0).
        assert!((p.model_offset - Vec3::new(0.0, -0.3, 0.0)).length() < 1e-5);
    }

    #[test]
    fn the_pelvis_ignores_ground_above_the_plane_clamps_and_smooths() {
        let mut rt = foot_rt();
        let mut targets = IkTargets::default();
        // Ground above the entity plane: the leg bends, the pelvis stays.
        place(&mut rt, &mut targets, 1.0, true, &mut ground(0.5));
        assert_eq!(rt.pelvis.unwrap().offset, 0.0);

        // A chasm clamps to the max drop.
        place(&mut rt, &mut targets, 1.0, true, &mut ground(-5.0));
        assert!((rt.pelvis.unwrap().offset - (-0.5)).abs() < 1e-5, "clamped");

        // Small dt approaches exponentially instead of snapping.
        let mut rt = foot_rt();
        let mut targets = IkTargets::default();
        place(&mut rt, &mut targets, 0.05, true, &mut ground(-0.4));
        let off = rt.pelvis.unwrap().offset;
        assert!(
            (off - (-0.2)).abs() < 1e-5,
            "dt 0.05 × rate 10 = half-way: {off}"
        );
    }

    #[test]
    fn a_down_event_latches_the_contact_until_the_up_event() {
        let mut rt = foot_rt();
        let mut targets = IkTargets::default();

        fire(&mut rt, "foot_l_down");
        place(&mut rt, &mut targets, 1.0, true, &mut ground(-0.1));
        assert!(rt.ik[0].foot.as_ref().unwrap().locked);
        assert!(
            std::mem::take(&mut rt.throttle.force_eval_external),
            "the down edge forces one evaluation"
        );
        let held = targets.targets["foot_l"].effector;
        assert!((held.z - 0.0).abs() < 1e-5, "contact −0.1 + offset 0.1");

        // The ground moves; the locked foot does not.
        rt.events.clear();
        place(&mut rt, &mut targets, 1.0, true, &mut ground(-0.4));
        assert_eq!(targets.targets["foot_l"].effector, held, "held while locked");
        assert!(!rt.throttle.force_eval_external, "no edge, no force");

        // The up edge releases: the target follows the fresh contact again.
        fire(&mut rt, "foot_l_up");
        place(&mut rt, &mut targets, 1.0, true, &mut ground(-0.4));
        assert!(!rt.ik[0].foot.as_ref().unwrap().locked);
        assert!(rt.throttle.force_eval_external, "the up edge forces too");
        assert!((targets.targets["foot_l"].effector.z - (-0.3)).abs() < 1e-5);
    }

    #[test]
    fn no_ground_removes_the_target_and_a_down_edge_does_not_latch_air() {
        let mut rt = foot_rt();
        let mut targets = IkTargets::default();
        place(&mut rt, &mut targets, 1.0, true, &mut ground(-0.2));
        assert!(targets.targets.contains_key("foot_l"));

        fire(&mut rt, "foot_l_down");
        place(&mut rt, &mut targets, 1.0, true, &mut |_| None);
        assert!(
            targets.targets.is_empty(),
            "mid-air: no target, the chain skips its solve"
        );
        assert!(!rt.ik[0].foot.as_ref().unwrap().locked, "air never locks");
    }

    #[test]
    fn leaving_the_top_bucket_clears_state_and_forces_exactly_one_eval() {
        let mut rt = foot_rt();
        let mut targets = IkTargets::default();
        fire(&mut rt, "foot_l_down");
        place(&mut rt, &mut targets, 1.0, true, &mut ground(-0.3));
        rt.events.clear();
        rt.throttle.force_eval_external = false;

        // Bucket left: stale targets are removed (the documented policy —
        // the solve skips, the pose returns to animated), locks release,
        // the pelvis resets, and one corrective eval is forced.
        place(&mut rt, &mut targets, 1.0, false, &mut ground(-0.3));
        assert!(targets.targets.is_empty());
        assert!(!rt.ik[0].foot.as_ref().unwrap().locked);
        assert_eq!(rt.pelvis.unwrap().offset, 0.0);
        assert!(std::mem::take(&mut rt.throttle.force_eval_external));

        // Steady state off-bucket: nothing changes, nothing forces.
        place(&mut rt, &mut targets, 1.0, false, &mut ground(-0.3));
        assert!(!rt.throttle.force_eval_external);
    }
}
