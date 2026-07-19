//! Cell broadphase over entity positions (M6 D6, combat groundwork).
//!
//! The server rebuilds one per tick from the player/NPC rows it already
//! holds; cells reuse the `world_grid` 64 m XY tiling so combat, collision
//! chunks and interest cells (M8) share one grid. Coarse by design: results
//! are candidates for narrow-phase tests, not hits.

use crate::world_grid;
use glam::{IVec2, Vec3};
use std::collections::HashMap;

#[derive(Debug, Default, Clone)]
pub struct Broadphase {
    cells: HashMap<IVec2, Vec<(u64, Vec3)>>,
}

impl Broadphase {
    pub fn new() -> Self {
        Self::default()
    }

    /// Each entity is inserted once; duplicate ids are the caller's bug.
    pub fn insert(&mut self, entity_id: u64, pos: Vec3) {
        self.cells
            .entry(world_grid::chunk_coord(pos))
            .or_default()
            .push((entity_id, pos));
    }

    /// Candidate ids for a sphere AoE: everything in cells overlapping the
    /// sphere's AABB, sorted by id. No distance filter — narrow phase is the
    /// caller's job.
    pub fn aoe_candidates(&self, center: Vec3, radius: f32) -> Vec<u64> {
        let r = Vec3::splat(radius);
        self.in_aabb(center - r, center + r)
            .into_iter()
            .map(|(id, _)| id)
            .collect()
    }

    /// Entities in cells overlapping the world AABB, sorted by id (stable
    /// candidate order keeps downstream tie-breaks deterministic).
    pub(crate) fn in_aabb(&self, min: Vec3, max: Vec3) -> Vec<(u64, Vec3)> {
        let mut out: Vec<(u64, Vec3)> = world_grid::chunks_overlapping(min, max)
            .filter_map(|c| self.cells.get(&c))
            .flatten()
            .copied()
            .collect();
        out.sort_unstable_by_key(|&(id, _)| id);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aoe_candidates_by_cell_overlap() {
        let mut bp = Broadphase::new();
        bp.insert(1, Vec3::new(10.0, 10.0, 0.0)); // cell (0,0)
        bp.insert(2, Vec3::new(70.0, 10.0, 0.0)); // cell (1,0)
        bp.insert(3, Vec3::new(-10.0, 10.0, 0.0)); // cell (-1,0)
        bp.insert(4, Vec3::new(10.0, 200.0, 0.0)); // cell (0,3) — far

        // Radius reaching only cell (0,0).
        assert_eq!(bp.aoe_candidates(Vec3::new(30.0, 30.0, 0.0), 5.0), [1]);
        // AABB spanning the x=0 border pulls in cell (-1,0) too.
        assert_eq!(bp.aoe_candidates(Vec3::new(2.0, 10.0, 0.0), 5.0), [1, 3]);
        // Large radius covers all three neighboring cells; far cell excluded.
        assert_eq!(bp.aoe_candidates(Vec3::new(30.0, 10.0, 0.0), 64.0), [1, 2, 3]);
    }

    #[test]
    fn candidates_sorted_by_id_across_cells() {
        let mut bp = Broadphase::new();
        bp.insert(9, Vec3::new(70.0, 10.0, 0.0));
        bp.insert(2, Vec3::new(10.0, 10.0, 0.0));
        bp.insert(5, Vec3::new(10.0, 12.0, 0.0));
        let got = bp.in_aabb(Vec3::new(0.0, 0.0, -1.0), Vec3::new(80.0, 20.0, 1.0));
        let ids: Vec<u64> = got.into_iter().map(|(id, _)| id).collect();
        assert_eq!(ids, [2, 5, 9]);
    }
}
