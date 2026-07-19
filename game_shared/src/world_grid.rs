//! World grid shared by collision chunks (M2) and interest cells (M8).
//!
//! The grid tiles the XY plane (Z-up world); vertical extent is unbounded
//! within a chunk. Chunk geometry is stored chunk-local (relative to
//! `chunk_origin`) to keep float precision flat across the world.

use glam::{IVec2, Vec2, Vec3};

/// Chunk edge length in meters. Matches the M0 interest-cell size; M8 reuses
/// this grid.
pub const CHUNK_SIZE: f32 = 64.0;

/// Chunk containing the given world position. Positions exactly on a border
/// belong to the higher-coordinate chunk (floor semantics).
pub fn chunk_coord(world: Vec3) -> IVec2 {
    IVec2::new(
        (world.x / CHUNK_SIZE).floor() as i32,
        (world.y / CHUNK_SIZE).floor() as i32,
    )
}

/// World-space origin (min XY corner) of a chunk.
pub fn chunk_origin(coord: IVec2) -> Vec2 {
    coord.as_vec2() * CHUNK_SIZE
}

/// World position -> chunk-local position (Z passes through).
pub fn world_to_local(coord: IVec2, world: Vec3) -> Vec3 {
    let o = chunk_origin(coord);
    Vec3::new(world.x - o.x, world.y - o.y, world.z)
}

/// Chunk-local position -> world position (Z passes through).
pub fn local_to_world(coord: IVec2, local: Vec3) -> Vec3 {
    let o = chunk_origin(coord);
    Vec3::new(local.x + o.x, local.y + o.y, local.z)
}

/// Re-anchor margin (M8 interest hysteresis): the anchor cell only moves
/// once the position is this far outside its AABB, so border wiggle never
/// swaps subscriptions.
pub const INTEREST_HYSTERESIS_M: f32 = 8.0;

/// Packed interest-cell key for one indexable equality column: high 32 bits
/// x, low 32 bits y; negative coords round-trip through the u32 cast.
pub fn cell_key(coord: IVec2) -> u64 {
    ((coord.x as u32 as u64) << 32) | (coord.y as u32 as u64)
}

/// Inverse of `cell_key`.
pub fn cell_coord(key: u64) -> IVec2 {
    IVec2::new((key >> 32) as u32 as i32, key as u32 as i32)
}

/// Interest cell key for an XY position (M8; replaces the M5 quadrant
/// zones on the net path — world streaming zones are unrelated).
pub fn cell_id_from_position(x: f32, y: f32) -> u64 {
    cell_key(chunk_coord(Vec3::new(x, y, 0.0)))
}

/// New anchor cell when `pos` has moved more than `INTEREST_HYSTERESIS_M`
/// outside the anchor cell's AABB (distance to the box, so corners don't
/// double-count); `None` while it hugs the anchor.
pub fn re_anchor(anchor: IVec2, pos: Vec2) -> Option<IVec2> {
    let o = chunk_origin(anchor);
    let dx = (o.x - pos.x).max(pos.x - (o.x + CHUNK_SIZE)).max(0.0);
    let dy = (o.y - pos.y).max(pos.y - (o.y + CHUNK_SIZE)).max(0.0);
    (dx * dx + dy * dy > INTEREST_HYSTERESIS_M * INTEREST_HYSTERESIS_M)
        .then(|| chunk_coord(pos.extend(0.0)))
}

/// Full-detail interest ring: the anchor cell and its 8 neighbors (3×3).
pub fn near_cells(anchor: IVec2) -> impl Iterator<Item = IVec2> {
    ring_cells(anchor, 0, 1)
}

/// Coarse interest ring: Chebyshev distance 2..=3 from the anchor (7×7
/// minus the near 3×3 — 40 cells).
pub fn far_cells(anchor: IVec2) -> impl Iterator<Item = IVec2> {
    ring_cells(anchor, 2, 3)
}

fn ring_cells(anchor: IVec2, min_d: i32, max_d: i32) -> impl Iterator<Item = IVec2> {
    (-max_d..=max_d).flat_map(move |dy| {
        (-max_d..=max_d).filter_map(move |dx| {
            (dx.abs().max(dy.abs()) >= min_d).then_some(anchor + IVec2::new(dx, dy))
        })
    })
}

/// All chunks whose XY footprint overlaps the world-space AABB `[min, max]`.
pub fn chunks_overlapping(min: Vec3, max: Vec3) -> impl Iterator<Item = IVec2> {
    let lo = chunk_coord(min);
    let hi = chunk_coord(max);
    (lo.y..=hi.y).flat_map(move |y| (lo.x..=hi.x).map(move |x| IVec2::new(x, y)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coord_floor_semantics() {
        assert_eq!(chunk_coord(Vec3::new(0.0, 0.0, 5.0)), IVec2::new(0, 0));
        assert_eq!(chunk_coord(Vec3::new(63.9, 0.0, 0.0)), IVec2::new(0, 0));
        assert_eq!(chunk_coord(Vec3::new(64.0, 0.0, 0.0)), IVec2::new(1, 0));
        assert_eq!(chunk_coord(Vec3::new(-0.1, -64.0, 0.0)), IVec2::new(-1, -1));
    }

    #[test]
    fn local_world_roundtrip() {
        let coord = IVec2::new(-3, 7);
        let world = Vec3::new(-150.25, 460.5, 12.0);
        let local = world_to_local(coord, world);
        assert_eq!(local_to_world(coord, local), world);
        let own = chunk_coord(world);
        let l = world_to_local(own, world);
        assert!(l.x >= 0.0 && l.x < CHUNK_SIZE && l.y >= 0.0 && l.y < CHUNK_SIZE);
    }

    #[test]
    fn cell_key_roundtrip() {
        for coord in [
            IVec2::new(0, 0),
            IVec2::new(3, -7),
            IVec2::new(-1, -1),
            IVec2::new(i32::MIN, i32::MAX),
        ] {
            assert_eq!(cell_coord(cell_key(coord)), coord);
        }
        // Distinct coords never collide on the packed key.
        assert_ne!(cell_key(IVec2::new(-1, 0)), cell_key(IVec2::new(0, -1)));
        assert_eq!(
            cell_id_from_position(-0.1, 64.0),
            cell_key(IVec2::new(-1, 1))
        );
    }

    #[test]
    fn re_anchor_hysteresis() {
        let a = IVec2::new(0, 0); // AABB [0,64)²
        // Border wiggle around x=64: inside margin, never re-anchors.
        assert_eq!(re_anchor(a, Vec2::new(63.9, 10.0)), None);
        assert_eq!(re_anchor(a, Vec2::new(64.0 + 7.9, 10.0)), None);
        // Committed crossing re-anchors to the position's cell.
        assert_eq!(
            re_anchor(a, Vec2::new(64.0 + 8.1, 10.0)),
            Some(IVec2::new(1, 0))
        );
        // Corner: 7 m past on both axes is √98 > 8 m box distance.
        assert_eq!(
            re_anchor(a, Vec2::new(71.0, 71.0)),
            Some(IVec2::new(1, 1))
        );
        // 5 m past on both axes is √50 < 8 m: still anchored.
        assert_eq!(re_anchor(a, Vec2::new(69.0, 69.0)), None);
        // Negative side too.
        assert_eq!(re_anchor(a, Vec2::new(-8.1, 0.0)), Some(IVec2::new(-1, 0)));
    }

    #[test]
    fn interest_rings() {
        let a = IVec2::new(2, -3);
        let near: Vec<_> = near_cells(a).collect();
        let far: Vec<_> = far_cells(a).collect();
        assert_eq!(near.len(), 9);
        assert_eq!(far.len(), 40);
        assert!(near.contains(&a));
        assert!(!far.contains(&a));
        assert!(near.iter().all(|c| !far.contains(c)));
        let d = |c: &IVec2| (*c - a).abs().max_element();
        assert!(near.iter().all(|c| d(c) <= 1));
        assert!(far.iter().all(|c| (2..=3).contains(&d(c))));
    }

    #[test]
    fn overlap_iteration() {
        let chunks: Vec<_> =
            chunks_overlapping(Vec3::new(-1.0, -1.0, 0.0), Vec3::new(65.0, 1.0, 10.0)).collect();
        assert_eq!(chunks.len(), 6); // x in {-1,0,1}, y in {-1,0}
        assert!(chunks.contains(&IVec2::new(-1, -1)));
        assert!(chunks.contains(&IVec2::new(1, 0)));
    }
}
