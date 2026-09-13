//! Foot placement (Task 41.5 P6, I-D4; Task 41.6 P6): rays the ground under
//! each foot chain, writes the chain's `IkTargets` entry as a terrain
//! *offset* of the animated foot, locks planted feet on anim event edges,
//! and drives the cosmetic pelvis drop.
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
    AnimGraphRuntime, HeldContact, IkTarget, IkTargets, RELEASE_SECS,
};
use crate::engine::ecs::components::Transform;
use crate::engine::ecs::hierarchy::{Parent, TransformCache};
use crate::engine::ecs::resources::{Resources, Time};
use crate::engine::ecs::schedule::System;
use crate::engine::physics::{PhysicsWorld, RigidBody, RigidBodyHandle};
use crate::engine::utils::coords::{convert_position_yup_to_zup, convert_position_zup_to_yup};
use nalgebra_glm as glm;

/// Tuning constants for I-D4, in one place. World units ≈ meters.
/// Ray start height above the animated foot (covers a raised step).
const RAY_START_ABOVE: f32 = 0.5;
/// Probe depth below the animated foot (step-down / downhill slope reach).
const STEP_HEIGHT: f32 = 0.6;
/// Total ray length.
const RAY_LENGTH: f32 = RAY_START_ABOVE + STEP_HEIGHT;
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

/// This frame's ground reading under one foot (Task 41.6 P6, F1).
#[derive(Clone, Copy)]
struct Contact {
    /// World Z-up displacement of the animated foot: the terrain height
    /// difference from the entity's ground plane (clamped to what the
    /// pelvis drop / step reach can absorb) plus the ankle offset tilted
    /// onto the surface normal — zero on flat ground at the entity's level.
    delta: glam::Vec3,
    /// The animated foot shifted by `delta`: the clip's plant pose on this
    /// terrain, what a `_down` edge latches.
    plant: glam::Vec3,
    /// Raw contact height, world Z.
    contact_z: f32,
}

/// Terrain delta for a foot: `contact_z − entity_z` clamped to
/// `[−MAX_PELVIS_DROP, +STEP_HEIGHT]` along Z, plus the ankle offset
/// re-aimed from straight up to the surface normal.
pub fn terrain_delta(
    contact_z: f32,
    entity_z: f32,
    normal: glam::Vec3,
    ankle_offset: f32,
) -> glam::Vec3 {
    let dz = (contact_z - entity_z).clamp(-MAX_PELVIS_DROP, STEP_HEIGHT);
    glam::Vec3::Z * dz + (normal - glam::Vec3::Z) * ankle_offset
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
///
/// Per foot (Task 41.6 P6): the ray starts above the last evaluation's
/// animated foot; the chain's `IkTargets` entry is the resulting
/// [`terrain_delta`] as an `Offset` goal (the pole comes from the chain's
/// own knee in the runner), so an unlocked foot keeps the clip's swing and
/// only conforms to the terrain. Lock state lives on the chain's
/// `FootState`: a `_down` edge latches the shifted plant pose, `_up` (or a
/// runner-side reach failure) releases it into a [`RELEASE_SECS`] blend
/// toward the offset target.
pub fn place_feet(
    rt: &mut AnimGraphRuntime,
    targets: &mut IkTargets,
    entity_render: glam::Mat4,
    dt: f32,
    active: bool,
    ray: &mut dyn FnMut(glam::Vec3) -> Option<(glam::Vec3, glam::Vec3)>,
) {
    if !active {
        let mut changed = false;
        for chain in &mut rt.ik {
            let Some(foot) = &mut chain.foot else { continue };
            changed |= targets.targets.remove(&chain.name).is_some()
                || foot.locked
                || foot.release.is_some();
            foot.clear();
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
            let delta = terrain_delta(point.z, entity_z, normal, foot.ankle_offset);
            Some(Contact {
                delta,
                plant: foot_world + delta,
                contact_z: point.z,
            })
        });

        // The runner found the held point out of reach last evaluation:
        // let go now, blending off it.
        if std::mem::take(&mut foot.release_requested) && foot.locked {
            foot.unlock();
            rt.throttle.force_eval_external = true;
        }
        // Lock edges (`<chain>_down` / `<chain>_up`, last tick's fires).
        // Either edge forces one full evaluation — the P4 hook, serial-side.
        // A down edge with no ground under the foot does not latch.
        if fired(&rt.events, &chain.name, FOOT_EVENT_DOWN_SUFFIX) {
            if let Some(c) = fresh {
                foot.locked = true;
                foot.held = Some(HeldContact {
                    point: c.plant,
                    contact_z: c.contact_z,
                });
                foot.release = None;
            }
            rt.throttle.force_eval_external = true;
        }
        if fired(&rt.events, &chain.name, FOOT_EVENT_UP_SUFFIX) {
            foot.unlock();
            rt.throttle.force_eval_external = true;
        }
        if let Some(rel) = &mut foot.release {
            rel.blend += dt / RELEASE_SECS;
            if rel.blend >= 1.0 {
                foot.release = None;
            }
        }

        match fresh {
            Some(c) => {
                // Upsert without allocating at steady state.
                let target = IkTarget::offset(c.delta);
                match targets.targets.get_mut(&chain.name) {
                    Some(t) => *t = target,
                    None => {
                        targets.targets.insert(chain.name.clone(), target);
                    }
                }
            }
            // No ground under the foot (mid-air, past a ledge): no target,
            // so the chain skips its solve — the foot stays animated (a
            // locked foot still holds its point).
            None => {
                targets.targets.remove(&chain.name);
            }
        }
        let contact_z = if foot.locked {
            foot.held.map(|h| h.contact_z)
        } else {
            None
        }
        .or(fresh.map(|c| c.contact_z));
        if let Some(z) = contact_z {
            lowest = lowest.min(z - entity_z);
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

/// Which body the ground rays must ignore (Task 41.6 D8): the rig entity's
/// own, else its parent's. A character's capsule lives on the gameplay root
/// while the animated rig is a child carrying no collider — without the
/// fallback every ray would hit the capsule the feet stand inside.
pub fn exclude_handle(
    own: Option<&RigidBody>,
    parent: Option<&RigidBody>,
) -> Option<RigidBodyHandle> {
    own.and_then(|b| b.handle)
        .or_else(|| parent.and_then(|b| b.handle))
}

/// The system: [`place_feet`] per entity with armed foot chains, rays
/// through the physics world, excluding the entity's own rigid body (or
/// its parent's — see [`exclude_handle`]).
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
        // `query` (not `query_mut`) so the parent's body can be read
        // through `world.get` inside the loop — shared borrows on
        // `RigidBody` only, so hecs' runtime check is satisfied.
        for (e, (rt, targets, transform, body, parent)) in world
            .query::<(
                &mut AnimGraphRuntime,
                &mut IkTargets,
                Option<&Transform>,
                Option<&RigidBody>,
                Option<&Parent>,
            )>()
            .iter()
        {
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
            // No `RigidBody` on the rig or its parent ⇒ no exclusion
            // filter: a character whose collider isn't backed by an ECS
            // RigidBody can ray-hit itself. Shipping characters attach
            // colliders via RigidBody (on the root, D8).
            let parent_body = parent.and_then(|p| world.get::<&RigidBody>(p.0).ok());
            let exclude = exclude_handle(body, parent_body.as_deref());
            // I-D5: raycasts only in the top significance bucket. `bucket`
            // is written by AnimGraphSystem step 2.5, which runs *after*
            // this system — one frame of latency entering/leaving bucket 0
            // (harmless: the cleanup path forces an eval). The cleanup also
            // zeroes the pelvis instantly rather than easing out — a pop,
            // but only when the entity is already far/small on screen.
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
            place_feet(rt, targets, entity_render, dt, active, &mut cast);
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
        ArmedIkChain, FootState, IkGoal, PelvisState, ThrottleState,
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
                foot: Some(FootState::new(0.1)),
                animated_tip: Some(Vec3::new(0.0, 0.1, 0.0)),
                animated_root: Some(Vec3::new(0.0, 0.9, 0.0)),
                animated_mid: Some(Vec3::new(0.0, 0.5, 0.2)),
                pole_dir: None,
            }],
            pelvis: Some(PelvisState {
                bone: 0,
                offset: 0.0,
                model_offset: Vec3::ZERO,
            }),
            ik_touched: Vec::new(),
            clips: Default::default(),
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
        place_feet(rt, targets, glam::Mat4::IDENTITY, dt, active, ray);
    }

    fn fire(rt: &mut AnimGraphRuntime, name: &str) {
        rt.events.push(AnimEventFire {
            name: name.into(),
            weight: 1.0,
        });
    }

    fn foot(rt: &AnimGraphRuntime) -> &FootState {
        rt.ik[0].foot.as_ref().unwrap()
    }

    fn delta(targets: &IkTargets) -> Vec3 {
        match targets.targets.get("foot_l").expect("target written") {
            IkTarget {
                goal: IkGoal::Offset(d),
                pole: None,
            } => *d,
            other => panic!("feet write pole-less offsets: {other:?}"),
        }
    }

    #[test]
    fn a_contact_writes_the_terrain_delta_and_drops_the_pelvis() {
        let mut rt = foot_rt();
        let mut targets = IkTargets::default();
        // dt 1.0 saturates the smoothing, so the pelvis lands on its goal.
        place(&mut rt, &mut targets, 1.0, true, &mut ground(-0.3));

        let d = delta(&targets);
        assert!(
            (d - Vec3::new(0.0, 0.0, -0.3)).length() < 1e-5,
            "ground 0.3 below the entity plane, flat: the foot shifts down 0.3: {d}"
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
    fn flat_ground_at_the_entity_plane_is_a_zero_delta() {
        let mut rt = foot_rt();
        let mut targets = IkTargets::default();
        place(&mut rt, &mut targets, 1.0, true, &mut ground(0.0));
        assert_eq!(delta(&targets), Vec3::ZERO, "nothing to conform to");
        assert_eq!(rt.pelvis.unwrap().offset, 0.0);
    }

    #[test]
    fn the_delta_clamps_and_tilts_the_ankle_offset_onto_the_normal() {
        // Deep chasm: the drop is capped at the pelvis reach.
        let d = terrain_delta(-5.0, 0.0, Vec3::Z, 0.1);
        assert!((d - Vec3::new(0.0, 0.0, -MAX_PELVIS_DROP)).length() < 1e-6, "{d}");
        // Tall step: capped at the step reach.
        let d = terrain_delta(2.0, 0.0, Vec3::Z, 0.1);
        assert!((d - Vec3::new(0.0, 0.0, STEP_HEIGHT)).length() < 1e-6, "{d}");
        // A slope tilts the 0.1 ankle lift from Z onto the normal.
        let n = Vec3::new(-0.6, 0.0, 0.8);
        let d = terrain_delta(0.0, 0.0, n, 0.1);
        assert!((d - (n - Vec3::Z) * 0.1).length() < 1e-6, "{d}");
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
    fn a_down_event_latches_the_plant_pose_until_the_up_event_then_blends_out() {
        let mut rt = foot_rt();
        let mut targets = IkTargets::default();

        fire(&mut rt, "foot_l_down");
        place(&mut rt, &mut targets, 1.0, true, &mut ground(-0.1));
        assert!(foot(&rt).locked);
        assert!(
            std::mem::take(&mut rt.throttle.force_eval_external),
            "the down edge forces one evaluation"
        );
        let held = foot(&rt).held.expect("latched");
        assert!(
            (held.point - Vec3::new(0.0, 0.0, 0.0)).length() < 1e-5,
            "animated foot (0,0,0.1) shifted by the −0.1 delta: {}",
            held.point
        );
        assert!((held.contact_z - (-0.1)).abs() < 1e-6);

        // The ground moves; the lock does not, the offset entry follows
        // (it is the runner's fallback), the pelvis measures the held contact.
        rt.events.clear();
        place(&mut rt, &mut targets, 1.0, true, &mut ground(-0.4));
        assert_eq!(foot(&rt).held.unwrap().point, held.point, "held while locked");
        assert!((delta(&targets).z - (-0.4)).abs() < 1e-5, "fresh offset still written");
        assert!(
            (rt.pelvis.unwrap().offset - (-0.1)).abs() < 1e-5,
            "pelvis on the held contact"
        );
        assert!(!rt.throttle.force_eval_external, "no edge, no force");

        // The up edge releases into a blend from the held point.
        fire(&mut rt, "foot_l_up");
        place(&mut rt, &mut targets, 0.05, true, &mut ground(-0.4));
        assert!(!foot(&rt).locked && foot(&rt).held.is_none());
        assert!(rt.throttle.force_eval_external, "the up edge forces too");
        let rel = foot(&rt).release.expect("release blend started");
        assert_eq!(rel.from, held.point);
        assert!((rel.blend - 0.5).abs() < 1e-5, "0.05 s of a 0.1 s blend: {}", rel.blend);
        assert!(
            (rt.pelvis.unwrap().offset - (-0.25)).abs() < 1e-5,
            "pelvis easing (half-way at dt 0.05) toward the fresh contact: {}",
            rt.pelvis.unwrap().offset
        );

        rt.events.clear();
        place(&mut rt, &mut targets, 0.05, true, &mut ground(-0.4));
        assert!(foot(&rt).release.is_none(), "blend complete");
    }

    #[test]
    fn a_runner_reach_failure_releases_the_lock_next_frame() {
        let mut rt = foot_rt();
        let mut targets = IkTargets::default();
        fire(&mut rt, "foot_l_down");
        place(&mut rt, &mut targets, 1.0, true, &mut ground(-0.1));
        rt.events.clear();
        rt.throttle.force_eval_external = false;

        // What `apply_ik` sets when the held point is past 98 % of the leg.
        rt.ik[0].foot.as_mut().unwrap().release_requested = true;
        place(&mut rt, &mut targets, 0.01, true, &mut ground(-0.1));
        let f = foot(&rt);
        assert!(!f.locked && f.held.is_none(), "unlocked");
        assert!(!f.release_requested, "consumed");
        assert!(f.release.is_some(), "blending off the held point");
        assert!(rt.throttle.force_eval_external, "one forced evaluation");
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
        assert!(!foot(&rt).locked, "air never locks");
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
        // the solve skips, the pose returns to animated), locks release
        // without a blend, the pelvis resets, and one corrective eval is
        // forced.
        place(&mut rt, &mut targets, 1.0, false, &mut ground(-0.3));
        assert!(targets.targets.is_empty());
        assert!(!foot(&rt).locked && foot(&rt).release.is_none());
        assert_eq!(rt.pelvis.unwrap().offset, 0.0);
        assert!(std::mem::take(&mut rt.throttle.force_eval_external));

        // Steady state off-bucket: nothing changes, nothing forces.
        place(&mut rt, &mut targets, 1.0, false, &mut ground(-0.3));
        assert!(!rt.throttle.force_eval_external);
    }

    fn body(handle: Option<RigidBodyHandle>) -> RigidBody {
        RigidBody {
            handle,
            ..RigidBody::default()
        }
    }

    #[test]
    fn exclude_prefers_the_rigs_own_body_then_falls_back_to_the_parents() {
        let own = RigidBodyHandle::from_raw_parts(1, 0);
        let parents = RigidBodyHandle::from_raw_parts(2, 0);
        assert_eq!(
            exclude_handle(Some(&body(Some(own))), Some(&body(Some(parents)))),
            Some(own),
            "a rig with its own body excludes that"
        );
        assert_eq!(
            exclude_handle(None, Some(&body(Some(parents)))),
            Some(parents),
            "D8: the rig child carries no body — the parent's capsule is excluded"
        );
        assert_eq!(
            exclude_handle(Some(&body(None)), Some(&body(Some(parents)))),
            Some(parents),
            "an unregistered own body (no handle yet) still falls back"
        );
        assert_eq!(exclude_handle(None, None), None, "no body anywhere: no filter");
        assert_eq!(exclude_handle(None, Some(&body(None))), None);
    }
}
