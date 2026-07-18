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

/// Zone containing an XY position (M5 zone scoping). The greybox world's
/// four zones are the axis quadrants of the cell grid (SW=0, SE=1, NW=2,
/// NE=3), so the boundary sits at 0.0 on each axis; positions exactly on a
/// boundary belong to the higher quadrant (matches `chunk_coord` floor
/// semantics). The formula extends past the authored 8×8 cells, so
/// out-of-world positions still map to the nearest quadrant's zone.
/// Frozen for M5; manifest-driven server zones are future work.
pub fn zone_id_from_position(x: f32, y: f32) -> u32 {
    (x >= 0.0) as u32 | (((y >= 0.0) as u32) << 1)
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
    fn zone_quadrants_match_manifest() {
        // greybox.world.ron: SW=0, SE=1, NW=2, NE=3.
        assert_eq!(zone_id_from_position(-1.0, -1.0), 0);
        assert_eq!(zone_id_from_position(1.0, -1.0), 1);
        assert_eq!(zone_id_from_position(-1.0, 1.0), 2);
        assert_eq!(zone_id_from_position(1.0, 1.0), 3);
        // Boundaries belong to the higher quadrant; spawn (0,0) is zone 3.
        assert_eq!(zone_id_from_position(0.0, 0.0), 3);
        assert_eq!(zone_id_from_position(0.0, -0.1), 1);
        // Extends beyond the authored 8×8 cells.
        assert_eq!(zone_id_from_position(-10_000.0, 10_000.0), 2);
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
