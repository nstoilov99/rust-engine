//! Deterministic terrain height sampling.
//!
//! Heights are sampled at INTEGER global grid coordinates (2 m spacing) —
//! never per-cell float offsets — so shared border vertices of adjacent cells
//! are computed from the same integers and are bit-identical by construction.
//!
//! Determinism rules: only IEEE-exact f32 ops (+ − × ÷, floor, sqrt, min/max)
//! and integer hashing. No `f32::sin` — libm transcendentals differ across
//! platforms; `dsin` below is a fixed polynomial approximation instead.

use crate::params::{Plateau, PLATEAUS, SEED, SPACING};

const PI: f32 = std::f32::consts::PI;
const TAU: f32 = std::f32::consts::TAU;

/// Sine approximation (Bhaskara I) with exact range reduction. Accuracy
/// ~0.002 absolute — irrelevant for terrain shape; determinism is the point.
pub fn dsin(x: f32) -> f32 {
    let t = x * (1.0 / TAU);
    let frac = t - t.floor();
    let mut a = frac * TAU;
    let neg = a > PI;
    if neg {
        a -= PI;
    }
    let s = 16.0 * a * (PI - a) / (5.0 * PI * PI - 4.0 * a * (PI - a));
    if neg {
        -s
    } else {
        s
    }
}

/// 32-bit integer mix (splitmix-style finalizer).
fn mix(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^= x >> 16;
    x
}

/// Hash (seed, a, b) → [0, 1).
fn hash01(seed: u32, a: i32, b: i32) -> f32 {
    let h = mix(seed ^ mix(a as u32).wrapping_add(mix((b as u32).wrapping_mul(0x9e37_79b9))));
    (h >> 8) as f32 / (1u32 << 24) as f32
}

fn smoothstep(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// Value noise over the integer grid: bilinear-smoothstep blend of corner
/// hashes on a `period`-sample lattice. Continuous everywhere (no region
/// seams), deterministic.
fn value_noise(seed: u32, gx: i32, gy: i32, period: i32) -> f32 {
    let cx = gx.div_euclid(period);
    let cy = gy.div_euclid(period);
    let fx = gx.rem_euclid(period) as f32 / period as f32;
    let fy = gy.rem_euclid(period) as f32 / period as f32;
    let (sx, sy) = (smoothstep(fx), smoothstep(fy));
    let h00 = hash01(seed, cx, cy);
    let h10 = hash01(seed, cx + 1, cy);
    let h01 = hash01(seed, cx, cy + 1);
    let h11 = hash01(seed, cx + 1, cy + 1);
    let top = h00 + (h10 - h00) * sx;
    let bot = h01 + (h11 - h01) * sx;
    top + (bot - top) * sy
}

/// Raw terrain height (before plateaus) at integer global grid coords.
fn terrain_base(gx: i32, gy: i32) -> f32 {
    let x = gx as f32 * SPACING;
    let y = gy as f32 * SPACING;
    let p = |i: i32| hash01(SEED, i, -i) * TAU;
    // Gentle by construction: sum of amplitude·frequency products stays
    // below tan(20°) ≈ 0.36.
    let broad = 5.0 * dsin(x * 0.019 + p(1)) * dsin(y * 0.023 + p(2));
    let mid = 2.0 * dsin(x * 0.043 + p(3)) * dsin(y * 0.037 + p(4));
    let amp = 0.5 + 0.75 * value_noise(SEED ^ 0x5eed_0001, gx, gy, 32);
    let fine = amp * dsin(x * 0.09 + p(5)) * dsin(y * 0.10 + p(6));
    8.0 + broad + mid + fine
}

/// Blend weight for a plateau: 1 inside the rect, linear falloff to 0 over
/// `margin` meters outside it (Chebyshev distance to the rect).
fn plateau_weight(pl: &Plateau, x: f32, y: f32) -> f32 {
    let d = (pl.min[0] - x)
        .max(x - pl.max[0])
        .max(pl.min[1] - y)
        .max(y - pl.max[1])
        .max(0.0);
    (1.0 - d / pl.margin).clamp(0.0, 1.0)
}

/// Final terrain height (Z, game space) at integer global grid coords.
pub fn height_at(gx: i32, gy: i32) -> f32 {
    let mut h = terrain_base(gx, gy);
    let x = gx as f32 * SPACING;
    let y = gy as f32 * SPACING;
    for pl in PLATEAUS {
        let w = plateau_weight(pl, x, y);
        h += (pl.height - h) * w;
    }
    h
}

/// Height at a world position on the sample lattice ONLY if it is exactly on
/// integer grid coords; general world positions get bilinear interpolation of
/// the four surrounding samples (used for spawn-point validation, not for
/// mesh emission).
pub fn height_at_world(x: f32, y: f32) -> f32 {
    let gx = (x / SPACING).floor();
    let gy = (y / SPACING).floor();
    let fx = x / SPACING - gx;
    let fy = y / SPACING - gy;
    let (gx, gy) = (gx as i32, gy as i32);
    let h00 = height_at(gx, gy);
    let h10 = height_at(gx + 1, gy);
    let h01 = height_at(gx, gy + 1);
    let h11 = height_at(gx + 1, gy + 1);
    let a = h00 + (h10 - h00) * fx;
    let b = h01 + (h11 - h01) * fx;
    a + (b - a) * fy
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dsin_matches_libm_loosely() {
        for i in -100..100 {
            let x = i as f32 * 0.37;
            assert!((dsin(x) - x.sin()).abs() < 3e-3, "x={x}");
        }
    }

    #[test]
    fn spawn_plateau_is_flat() {
        // Inner region of the origin plateau (margin 8 inside ±32).
        for gx in -10..=10 {
            for gy in -10..=10 {
                assert_eq!(height_at(gx, gy), 8.0);
            }
        }
    }

    #[test]
    fn slopes_stay_gentle() {
        // Sample the whole world; adjacent-sample delta bounds the gradient.
        // Hard bound: everything (including plateau skirts) stays walkable
        // below the 45° controller limit. Soft bound: skirts under 40°.
        let max_dh = SPACING * 45f32.to_radians().tan();
        let soft = SPACING * 40f32.to_radians().tan();
        let open_dh = SPACING * 25f32.to_radians().tan(); // plain terrain
        let in_skirt = |gx: i32, gy: i32| {
            let (x, y) = (gx as f32 * SPACING, gy as f32 * SPACING);
            PLATEAUS.iter().any(|pl| {
                x > pl.min[0] - pl.margin - SPACING
                    && x < pl.max[0] + pl.margin + SPACING
                    && y > pl.min[1] - pl.margin - SPACING
                    && y < pl.max[1] + pl.margin + SPACING
            })
        };
        let (mut worst, mut worst_open): (f32, f32) = (0.0, 0.0);
        for gx in -128..=128 {
            for gy in -128..=128 {
                let h = height_at(gx, gy);
                let dh = (height_at(gx + 1, gy) - h)
                    .abs()
                    .max((height_at(gx, gy + 1) - h).abs());
                worst = worst.max(dh);
                if !in_skirt(gx, gy) {
                    worst_open = worst_open.max(dh);
                }
            }
        }
        assert!(worst < max_dh, "slope exceeds 45°: dh={worst}");
        assert!(worst < soft, "plateau skirt steeper than 40°: dh={worst}");
        assert!(worst_open < open_dh, "open terrain steeper than 25°: dh={worst_open}");
    }
}
