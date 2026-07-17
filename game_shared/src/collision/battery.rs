//! Golden-battery format: a RON-defined set of raycast/shape-cast cases run
//! against a cooked test chunk set (`game_shared/tests/data/collision/`).
//!
//! Defined in the shared crate so the M6 client-vs-server-WASM parity run
//! executes the exact same cases against the exact same bytes. Tolerances per
//! the M2 plan: position/TOI within 1 mm, normal within 0.1°.

use glam::{Quat, Vec3};
use parry3d::shape::{Ball, Capsule, Cuboid, Shape};
use serde::Deserialize;

use super::store::ChunkStore;
use super::TriangleId;

/// Position and TOI tolerance in world units (1 mm).
pub const POSITION_TOLERANCE: f32 = 1e-3;
/// Normal tolerance in degrees.
pub const NORMAL_TOLERANCE_DEG: f32 = 0.1;

#[derive(Debug, Deserialize)]
pub struct Battery {
    pub cases: Vec<Case>,
}

#[derive(Debug, Deserialize)]
pub struct Case {
    pub name: String,
    pub query: Query,
    pub expect: Expect,
}

#[derive(Debug, Deserialize)]
pub enum Query {
    Ray {
        origin: [f32; 3],
        dir: [f32; 3],
        max_toi: f32,
    },
    ShapeCast {
        shape: CastShape,
        position: [f32; 3],
        #[serde(default)]
        rotation: Option<[f32; 4]>,
        delta: [f32; 3],
    },
}

#[derive(Debug, Deserialize)]
pub enum CastShape {
    Ball {
        radius: f32,
    },
    /// Upright capsule (segment along Z).
    Capsule {
        half_height: f32,
        radius: f32,
    },
    Cuboid {
        half_extents: [f32; 3],
    },
}

#[derive(Debug, Deserialize)]
pub enum Expect {
    Miss,
    Hit {
        toi: f32,
        position: [f32; 3],
        normal: [f32; 3],
        id: TriangleId,
    },
}

/// Run every case against `store`; returns one message per failing case
/// (empty = battery passed).
pub fn run(store: &ChunkStore, battery: &Battery) -> Vec<String> {
    let mut failures = Vec::new();
    for case in &battery.cases {
        // (toi, position, normal, id) plus the world length one unit of TOI
        // corresponds to, so the 1 mm tolerance applies to both query kinds.
        let (hit, toi_scale) = match &case.query {
            Query::Ray {
                origin,
                dir,
                max_toi,
            } => {
                let dir = Vec3::from(*dir);
                let hit = store
                    .raycast(Vec3::from(*origin), dir, *max_toi)
                    .map(|h| (h.toi, h.position, h.normal, h.triangle_id));
                (hit, dir.length())
            }
            Query::ShapeCast {
                shape,
                position,
                rotation,
                delta,
            } => {
                let delta = Vec3::from(*delta);
                let rot = rotation
                    .map(|q| Quat::from_xyzw(q[0], q[1], q[2], q[3]))
                    .unwrap_or(Quat::IDENTITY);
                let hit = cast(store, shape, Vec3::from(*position), rot, delta)
                    .map(|h| (h.toi, h.position, h.normal, h.triangle_id));
                (hit, delta.length())
            }
        };

        let name = &case.name;
        match (&case.expect, hit) {
            (Expect::Miss, None) => {}
            (Expect::Miss, Some((toi, .., id))) => {
                failures.push(format!("{name}: expected miss, hit id {id} at toi {toi}"));
            }
            (Expect::Hit { .. }, None) => failures.push(format!("{name}: expected hit, got miss")),
            (
                Expect::Hit {
                    toi,
                    position,
                    normal,
                    id,
                },
                Some((atoi, apos, anorm, aid)),
            ) => {
                if (atoi - toi).abs() * toi_scale > POSITION_TOLERANCE {
                    failures.push(format!("{name}: toi {atoi} != expected {toi}"));
                }
                let epos = Vec3::from(*position);
                if apos.distance(epos) > POSITION_TOLERANCE {
                    failures.push(format!("{name}: position {apos} != expected {epos}"));
                }
                let enorm = Vec3::from(*normal).normalize();
                let angle = anorm
                    .normalize()
                    .dot(enorm)
                    .clamp(-1.0, 1.0)
                    .acos()
                    .to_degrees();
                if angle > NORMAL_TOLERANCE_DEG {
                    failures.push(format!(
                        "{name}: normal {anorm} is {angle:.3}° off expected {enorm}"
                    ));
                }
                if aid != *id {
                    failures.push(format!("{name}: triangle id {aid} != expected {id}"));
                }
            }
        }
    }
    failures
}

fn cast(
    store: &ChunkStore,
    shape: &CastShape,
    position: Vec3,
    rotation: Quat,
    delta: Vec3,
) -> Option<super::ShapeHit> {
    let shape: &dyn Shape = match shape {
        CastShape::Ball { radius } => &Ball::new(*radius),
        CastShape::Capsule {
            half_height,
            radius,
        } => &Capsule::new_z(*half_height, *radius),
        CastShape::Cuboid { half_extents } => &Cuboid::new(parry3d::math::Vector::new(
            half_extents[0],
            half_extents[1],
            half_extents[2],
        )),
    };
    store.cast_shape(shape, position, rotation, delta)
}
