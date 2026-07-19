//! Shared kinematic character controller (M6 D2, ADR-015).
//!
//! Pure functions over `ChunkStore` — no I/O, no ECS; compiled into both the
//! client (prediction) and the server WASM module (authority). Parity is
//! tolerance-based, never bit-exact: fixed `MOVE_DT`, deterministic candidate
//! ordering in the collision layer, and conservative epsilons keep
//! native/WASM divergence inside the trace-suite envelope (D5).
//!
//! Normative step order (both sides must match): depenetrate → wish velocity
//! → jump → gravity → horizontal collide-and-slide (with one step-up
//! attempt) → vertical pass → ground snap.

pub mod trace;

use crate::collision::{ChunkStore, FLAG_BLOCKING, FLAG_WALKABLE, TriangleFlags};
use crate::net::schema::{PLAYER_SPEED_MPS, SPRINT_MULTIPLIER};
use glam::{Quat, Vec2, Vec3};
use parry3d::shape::Capsule;
use serde::{Deserialize, Serialize};

/// One controller step per input sequence / server movement tick (20 Hz).
pub const MOVE_DT: f32 = 1.0 / 20.0;

/// Movements shorter than this are noise; casts are skipped.
const MIN_MOVE: f32 = 1e-5;
const MAX_SLIDE_ITERATIONS: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MotionConfig {
    pub capsule_radius: f32,
    /// Half-length of the capsule's core segment (total height =
    /// `2 * (capsule_half_seg + capsule_radius)`).
    pub capsule_half_seg: f32,
    pub walk_speed: f32,
    pub sprint_mult: f32,
    /// Positive; applied along −Z.
    pub gravity: f32,
    pub jump_speed: f32,
    pub terminal_fall_speed: f32,
    pub max_slope_deg: f32,
    pub step_height: f32,
    /// Max downward snap that keeps a walking character grounded over dips.
    pub snap_dist: f32,
    /// Contact offset: casts stop this far short of surfaces.
    pub skin: f32,
}

impl Default for MotionConfig {
    fn default() -> Self {
        Self {
            capsule_radius: 0.4,
            capsule_half_seg: 0.5, // total height 1.8 m
            walk_speed: PLAYER_SPEED_MPS,
            sprint_mult: SPRINT_MULTIPLIER,
            gravity: 20.0,
            jump_speed: 8.0, // apex ≈ 1.6 m at gravity 20
            terminal_fall_speed: 30.0,
            max_slope_deg: 50.0,
            step_height: 0.35,
            snap_dist: 0.3,
            skin: 0.02,
        }
    }
}

/// Grounding seam (ADR-015): reserved in M6 (always `None`), used by M6.5
/// moving platforms. Serialized so platforms are purely additive.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GroundRef {
    pub entity_id: u64,
    pub generation: u32,
    pub local_anchor: [f32; 3],
    pub inherited_velocity: [f32; 3],
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionState {
    /// Capsule center, world Z-up.
    pub pos: Vec3,
    pub vel: Vec3,
    /// Pass-through from intent, normalized to (−π, π].
    pub yaw: f32,
    pub grounded: bool,
    pub ground_ref: Option<GroundRef>,
}

impl MotionState {
    /// Standing rest state with the capsule bottom at `feet` (world Z-up).
    pub fn standing_at(cfg: &MotionConfig, feet: Vec3) -> Self {
        Self {
            pos: feet + Vec3::Z * (cfg.capsule_half_seg + cfg.capsule_radius + cfg.skin),
            vel: Vec3::ZERO,
            yaw: 0.0,
            grounded: true,
            ground_ref: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MoveIntent {
    /// Desired movement on the world XY plane; clamped to unit length.
    pub move_dir: [f32; 2],
    pub yaw: f32,
    pub sprint: bool,
    /// Edge-triggered: the caller (server queue / prediction) consumes it.
    pub jump: bool,
}

impl MoveIntent {
    pub const IDLE: Self = Self {
        move_dir: [0.0, 0.0],
        yaw: 0.0,
        sprint: false,
        jump: false,
    };
}

fn capsule(cfg: &MotionConfig) -> Capsule {
    Capsule::new_z(cfg.capsule_half_seg, cfg.capsule_radius)
}

fn walkable(cfg: &MotionConfig, normal: Vec3, flags: TriangleFlags) -> bool {
    if flags & FLAG_BLOCKING != 0 {
        return false;
    }
    if flags & FLAG_WALKABLE != 0 {
        return true;
    }
    normal.z >= cfg.max_slope_deg.to_radians().cos()
}

/// Wrap to (−π, π]. Non-finite input falls back to `fallback`.
fn wrap_yaw(yaw: f32, fallback: f32) -> f32 {
    if !yaw.is_finite() {
        return fallback;
    }
    let y = yaw.rem_euclid(std::f32::consts::TAU);
    if y > std::f32::consts::PI {
        y - std::f32::consts::TAU
    } else {
        y
    }
}

/// One fixed `MOVE_DT` controller step. Pure: same inputs → same output
/// (modulo float lowering, which the parity envelope covers).
pub fn step(
    cfg: &MotionConfig,
    state: &MotionState,
    intent: &MoveIntent,
    store: &ChunkStore,
) -> MotionState {
    let cap = capsule(cfg);
    let rot = Quat::IDENTITY;
    let mut pos = state.pos;
    let mut vel = state.vel;
    let was_grounded = state.grounded;

    // 1. Depenetrate (bounded, so a bad spawn can't explode).
    let mut push = Vec3::ZERO;
    for c in store.contacts(&cap, pos, rot, 0.0) {
        if c.distance < 0.0 {
            push += c.normal * (-c.distance);
        }
    }
    let max_push = 2.0 * cfg.skin;
    if push.length() > max_push {
        push = push.normalize() * max_push;
    }
    pos += push;

    // 2. Horizontal wish velocity (instantaneous; no acceleration in M6).
    let mut dir = Vec2::new(intent.move_dir[0], intent.move_dir[1]);
    if !dir.is_finite() {
        dir = Vec2::ZERO;
    }
    if dir.length_squared() > 1.0 {
        dir = dir.normalize();
    }
    let speed = cfg.walk_speed * if intent.sprint { cfg.sprint_mult } else { 1.0 };
    vel.x = dir.x * speed;
    vel.y = dir.y * speed;

    // 3. Jump (edge-triggered by the caller).
    let mut jumped = false;
    if was_grounded && intent.jump {
        vel.z = cfg.jump_speed;
        jumped = true;
    }

    // 4. Gravity.
    vel.z = (vel.z - cfg.gravity * MOVE_DT).max(-cfg.terminal_fall_speed);

    // 5. Horizontal collide-and-slide.
    let mut delta = Vec3::new(vel.x, vel.y, 0.0) * MOVE_DT;
    let mut stepped_up = false;
    for _ in 0..MAX_SLIDE_ITERATIONS {
        let len = delta.length();
        if len < MIN_MOVE {
            break;
        }
        let dirn = delta / len;
        let Some(hit) = store.cast_shape(&cap, pos, rot, delta) else {
            pos += delta;
            break;
        };
        let travelled = (hit.toi * len - cfg.skin).max(0.0);
        pos += dirn * travelled;
        if was_grounded && !jumped && !stepped_up && !walkable(cfg, hit.normal, hit.flags) {
            stepped_up = true; // one attempt per step, successful or not
            if let Some(stepped) = try_step_up(cfg, &cap, store, pos, delta - dirn * travelled) {
                pos = stepped;
                break;
            }
        }
        // Slide the remainder along the surface plane.
        let remaining = delta - dirn * travelled;
        delta = remaining - hit.normal * remaining.dot(hit.normal);
    }

    // 6. Vertical pass.
    let mut grounded = false;
    let dz = vel.z * MOVE_DT;
    if dz.abs() > MIN_MOVE {
        let vdelta = Vec3::new(0.0, 0.0, dz);
        match store.cast_shape(&cap, pos, rot, vdelta) {
            None => pos.z += dz,
            Some(hit) => {
                let travelled = (hit.toi * dz.abs() - cfg.skin).max(0.0);
                pos.z += dz.signum() * travelled;
                if dz < 0.0 {
                    if walkable(cfg, hit.normal, hit.flags) {
                        grounded = true;
                        vel.z = 0.0;
                    } else {
                        // Steep landing: slide the remainder once (checked).
                        let remaining = vdelta - Vec3::new(0.0, 0.0, dz.signum() * travelled);
                        let slide = remaining - hit.normal * remaining.dot(hit.normal);
                        let slen = slide.length();
                        if slen > MIN_MOVE {
                            match store.cast_shape(&cap, pos, rot, slide) {
                                None => pos += slide,
                                Some(h2) => {
                                    pos += slide / slen * (h2.toi * slen - cfg.skin).max(0.0);
                                }
                            }
                        }
                    }
                } else {
                    vel.z = 0.0; // ceiling
                }
            }
        }
    }

    // 7. Ground snap: keep walking characters attached over small dips.
    if was_grounded && !jumped && !grounded && vel.z <= 0.0 {
        let dist = cfg.snap_dist + cfg.skin;
        if let Some(hit) = store.cast_shape(&cap, pos, rot, Vec3::NEG_Z * dist) {
            if walkable(cfg, hit.normal, hit.flags) {
                pos.z -= (hit.toi * dist - cfg.skin).max(0.0);
                grounded = true;
                vel.z = 0.0;
            }
        }
    }

    MotionState {
        pos,
        vel,
        yaw: wrap_yaw(intent.yaw, state.yaw),
        grounded,
        // M6: static world only; M6.5 platforms will set this from the hit.
        ground_ref: None,
    }
}

/// One step-up attempt: headroom check, re-cast the remaining horizontal
/// delta from `step_height` up, then land back down on a walkable surface.
fn try_step_up(
    cfg: &MotionConfig,
    cap: &Capsule,
    store: &ChunkStore,
    pos: Vec3,
    remaining: Vec3,
) -> Option<Vec3> {
    let len = remaining.length();
    if len < MIN_MOVE {
        return None;
    }
    let up = Vec3::Z * cfg.step_height;
    if store.cast_shape(cap, pos, Quat::IDENTITY, up).is_some() {
        return None; // no headroom
    }
    let raised = pos + up;
    let dirn = remaining / len;
    let forward = match store.cast_shape(cap, raised, Quat::IDENTITY, remaining) {
        None => len,
        Some(h) => (h.toi * len - cfg.skin).max(0.0),
    };
    if forward <= cfg.skin {
        return None; // still blocked: a wall, not a step
    }
    let fwd_pos = raised + dirn * forward;
    let down = cfg.step_height + cfg.skin;
    let hit = store.cast_shape(cap, fwd_pos, Quat::IDENTITY, Vec3::NEG_Z * down)?;
    if !walkable(cfg, hit.normal, hit.flags) {
        return None;
    }
    Some(fwd_pos - Vec3::Z * (hit.toi * down - cfg.skin).max(0.0))
}

#[cfg(test)]
mod tests;
