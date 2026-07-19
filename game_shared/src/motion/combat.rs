//! Hitscan and projectile-sweep primitives (M6 D6, combat groundwork).
//!
//! Pure queries: static world via `ChunkStore::raycast`, entities as the
//! shared motion capsule at `Broadphase` candidate positions. Server-side
//! only in M6 — no replication; M7 decides projectile representation.

use super::broadphase::Broadphase;
use super::MotionConfig;
use crate::collision::{ChunkStore, TriangleFlags, TriangleId};
use glam::Vec3;
use parry3d::math::{Point, Vector};
use parry3d::query::{Ray, RayCast};
use parry3d::shape::Capsule;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HitKind {
    World {
        triangle_id: TriangleId,
        flags: TriangleFlags,
    },
    Entity {
        entity_id: u64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hit {
    /// Meters from the ray origin (direction is normalized internally).
    pub distance: f32,
    pub position: Vec3,
    /// Surface normal at the hit, pointing back toward the origin side.
    pub normal: Vec3,
    pub kind: HitKind,
}

/// First hit along `origin + t * normalize(dir)`, `t ∈ [0, max_dist]`:
/// world triangles, or entity capsules (`cfg`'s dimensions, upright at the
/// candidate's capsule-center position). Earliest distance wins; ties keep
/// the world hit, then the lowest entity id (candidates arrive id-sorted).
pub fn hitscan(
    cfg: &MotionConfig,
    store: &ChunkStore,
    targets: &Broadphase,
    origin: Vec3,
    dir: Vec3,
    max_dist: f32,
) -> Option<Hit> {
    let dir = dir.try_normalize()?;
    let mut best = store.raycast(origin, dir, max_dist).map(|h| Hit {
        distance: h.toi,
        position: h.position,
        normal: h.normal,
        kind: HitKind::World {
            triangle_id: h.triangle_id,
            flags: h.flags,
        },
    });

    let end = origin + dir * max_dist;
    // A capsule center can sit this far from the segment and still touch it.
    let reach = Vec3::splat(cfg.capsule_half_seg + cfg.capsule_radius);
    let capsule = Capsule::new_z(cfg.capsule_half_seg, cfg.capsule_radius);
    for (entity_id, pos) in targets.in_aabb(origin.min(end) - reach, origin.max(end) + reach) {
        // Capsule-local frame = translation only (upright, no rotation).
        let local = origin - pos;
        let ray = Ray::new(
            Point::new(local.x, local.y, local.z),
            Vector::new(dir.x, dir.y, dir.z),
        );
        let Some(h) = capsule.cast_local_ray_and_get_normal(&ray, max_dist, true) else {
            continue;
        };
        if best.map_or(true, |b| h.time_of_impact < b.distance) {
            best = Some(Hit {
                distance: h.time_of_impact,
                position: origin + dir * h.time_of_impact,
                normal: Vec3::new(h.normal.x, h.normal.y, h.normal.z),
                kind: HitKind::Entity { entity_id },
            });
        }
    }
    best
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Projectile {
    pub pos: Vec3,
    pub vel: Vec3,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SweepOutcome {
    Hit(Hit),
    Flying(Projectile),
}

/// One fixed-step projectile advance (the server's movement tick, 50 ms):
/// integrate gravity into velocity, then sweep the step's segment with the
/// same world + capsule tests as `hitscan`.
pub fn projectile_step(
    cfg: &MotionConfig,
    store: &ChunkStore,
    targets: &Broadphase,
    p: Projectile,
    gravity: f32,
    dt: f32,
) -> SweepOutcome {
    let vel = p.vel - Vec3::Z * gravity * dt;
    let delta = vel * dt;
    if let Some(hit) = hitscan(cfg, store, targets, p.pos, delta, delta.length()) {
        return SweepOutcome::Hit(hit);
    }
    SweepOutcome::Flying(Projectile {
        pos: p.pos + delta,
        vel,
    })
}
