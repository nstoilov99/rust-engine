//! Runtime chunk store and Z-up collision queries.
//!
//! Owns loaded `.ccol` chunks and answers raycasts, shape-casts and contact
//! probes against them. No I/O here — callers hand in chunk bytes
//! (client: file/pak; server M5: its own storage) and drive insert/remove
//! (M4 streaming).
//!
//! All queries take and return **Z-up world space**. Geometry is stored
//! chunk-local (see `world_grid`); queries are translated per chunk so float
//! precision stays flat across the world.
//!
//! Triangle attribution: parry's whole-trimesh casts return no triangle id,
//! so every query is BVH traversal (candidates under the swept AABB) →
//! per-triangle cast → earliest TOI, tie-broken on the lowest stable triangle
//! id. Border-duplicated triangles therefore resolve to one consistent hit.

use std::collections::HashMap;

use glam::{IVec2, Quat, Vec3};
use parry3d::bounding_volume::{Aabb, BoundingVolume};
use parry3d::math::{Isometry, Point, Vector};
use parry3d::na;
use parry3d::query::{cast_shapes, contact, Ray, RayCast, ShapeCastOptions};
use parry3d::shape::{Shape, TriMesh, TriMeshBuilderError};

use super::format::{self, FormatError};
use super::{TriangleFlags, TriangleId};
use crate::world_grid;

/// Margin added to query AABBs before BVH traversal (float safety only).
const AABB_MARGIN: f32 = 1e-3;

#[derive(Debug)]
pub enum ChunkLoadError {
    Format(FormatError),
    Mesh(TriMeshBuilderError),
}

impl std::fmt::Display for ChunkLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Format(e) => write!(f, "chunk format error: {e}"),
            Self::Mesh(e) => write!(f, "trimesh build error: {e}"),
        }
    }
}

pub struct LoadedChunk {
    trimesh: TriMesh,
    triangle_ids: Vec<TriangleId>,
    triangle_flags: Vec<TriangleFlags>,
}

impl LoadedChunk {
    pub fn num_triangles(&self) -> usize {
        self.trimesh.num_triangles()
    }

    /// Chunk-local triangle vertices (debug draw / inspection).
    pub fn triangle_local(&self, i: u32) -> [Vec3; 3] {
        let tri = self.trimesh.triangle(i);
        [vec_from_na(tri.a), vec_from_na(tri.b), vec_from_na(tri.c)]
    }

    /// Chunk-local geometry AABB as (min, max).
    pub fn local_aabb(&self) -> (Vec3, Vec3) {
        let aabb = self.trimesh.local_aabb();
        (vec_from_na(aabb.mins), vec_from_na(aabb.maxs))
    }
}

/// A ray intersection, world Z-up.
#[derive(Debug, Clone, Copy)]
pub struct RayHit {
    /// Hit point = `origin + dir * toi` (in units of `dir`'s length).
    pub toi: f32,
    pub position: Vec3,
    pub normal: Vec3,
    pub triangle_id: TriangleId,
    pub flags: TriangleFlags,
}

/// A shape-cast impact, world Z-up.
#[derive(Debug, Clone, Copy)]
pub struct ShapeHit {
    /// Fraction of `delta` travelled at impact, in `[0, 1]`. Zero means the
    /// shape already touches/penetrates at the start pose.
    pub toi: f32,
    /// Contact point on the geometry.
    pub position: Vec3,
    /// Geometry surface normal at the contact (points toward the shape).
    pub normal: Vec3,
    pub triangle_id: TriangleId,
    pub flags: TriangleFlags,
}

/// A contact/overlap probe result, world Z-up.
#[derive(Debug, Clone, Copy)]
pub struct ContactHit {
    /// Signed distance between shape and triangle; negative = penetration depth.
    pub distance: f32,
    /// Contact point on the geometry.
    pub position: Vec3,
    /// Geometry normal at the contact (points toward the shape).
    pub normal: Vec3,
    pub triangle_id: TriangleId,
    pub flags: TriangleFlags,
}

#[derive(Default)]
pub struct ChunkStore {
    chunks: HashMap<IVec2, LoadedChunk>,
}

impl ChunkStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Parse `.ccol` bytes, build the BVH and insert (replacing any chunk at
    /// the same coordinate). Returns the chunk coordinate.
    pub fn insert_chunk(&mut self, bytes: &[u8]) -> Result<IVec2, ChunkLoadError> {
        let data = format::read_chunk(bytes).map_err(ChunkLoadError::Format)?;
        let vertices: Vec<Point<f32>> = data
            .trimesh
            .vertices
            .iter()
            .map(|v| Point::new(v[0], v[1], v[2]))
            .collect();
        let trimesh =
            TriMesh::new(vertices, data.trimesh.indices.clone()).map_err(ChunkLoadError::Mesh)?;
        self.chunks.insert(
            data.coord,
            LoadedChunk {
                trimesh,
                triangle_ids: data.trimesh.triangle_ids,
                triangle_flags: data.trimesh.triangle_flags,
            },
        );
        Ok(data.coord)
    }

    pub fn remove_chunk(&mut self, coord: IVec2) -> bool {
        self.chunks.remove(&coord).is_some()
    }

    pub fn clear(&mut self) {
        self.chunks.clear();
    }

    pub fn len(&self) -> usize {
        self.chunks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }

    pub fn contains(&self, coord: IVec2) -> bool {
        self.chunks.contains_key(&coord)
    }

    pub fn coords(&self) -> impl Iterator<Item = IVec2> + '_ {
        self.chunks.keys().copied()
    }

    /// Loaded chunks in arbitrary order (debug draw / inspection).
    pub fn chunks(&self) -> impl Iterator<Item = (IVec2, &LoadedChunk)> + '_ {
        self.chunks.iter().map(|(c, chunk)| (*c, chunk))
    }

    /// Loaded chunks overlapping a world AABB, in deterministic (y, x) order.
    fn chunks_for_aabb(&self, min: Vec3, max: Vec3) -> Vec<(IVec2, &LoadedChunk)> {
        let mut out: Vec<(IVec2, &LoadedChunk)> = world_grid::chunks_overlapping(min, max)
            .filter_map(|c| self.chunks.get(&c).map(|chunk| (c, chunk)))
            .collect();
        out.sort_by_key(|(c, _)| (c.y, c.x));
        out
    }

    /// Cast a ray; `toi` is in units of `dir`'s length, capped at `max_toi`.
    /// Earliest hit wins; equal-TOI ties break to the lowest triangle id.
    pub fn raycast(&self, origin: Vec3, dir: Vec3, max_toi: f32) -> Option<RayHit> {
        let end = origin + dir * max_toi;
        let (min, max) = (origin.min(end), origin.max(end));

        let mut best: Option<RayHit> = None;
        for (coord, chunk) in self.chunks_for_aabb(min, max) {
            let local_origin = world_grid::world_to_local(coord, origin);
            let local_end = world_grid::world_to_local(coord, end);
            let aabb = Aabb::new(
                Point::from(vec_na(local_origin.min(local_end))),
                Point::from(vec_na(local_origin.max(local_end))),
            )
            .loosened(AABB_MARGIN);

            let mut candidates: Vec<u32> = Vec::new();
            chunk.trimesh.qbvh().intersect_aabb(&aabb, &mut candidates);

            let ray = Ray::new(Point::from(vec_na(local_origin)), vec_na(dir));
            for i in candidates {
                let tri = chunk.trimesh.triangle(i);
                let Some(hit) = tri.cast_local_ray_and_get_normal(&ray, max_toi, true) else {
                    continue;
                };
                let id = chunk.triangle_ids[i as usize];
                if better(
                    hit.time_of_impact,
                    id,
                    &best.map(|b| (b.toi, b.triangle_id)),
                ) {
                    best = Some(RayHit {
                        toi: hit.time_of_impact,
                        position: origin + dir * hit.time_of_impact,
                        normal: vec_glam(hit.normal),
                        triangle_id: id,
                        flags: chunk.triangle_flags[i as usize],
                    });
                }
            }
        }
        best
    }

    /// Sweep `shape` (posed at `position`/`rotation`) along `delta`. `toi` is
    /// the fraction of `delta` travelled at impact. Impacts at `toi == 0`
    /// (already touching/penetrating) are reported with valid contact
    /// geometry. Earliest hit wins; ties break to the lowest triangle id.
    pub fn cast_shape(
        &self,
        shape: &dyn Shape,
        position: Vec3,
        rotation: Quat,
        delta: Vec3,
    ) -> Option<ShapeHit> {
        let world_pose = isometry(position, rotation);
        let start = shape.compute_aabb(&world_pose);
        let swept = start
            .merged(&start.translated(&vec_na(delta)))
            .loosened(AABB_MARGIN);

        let options = ShapeCastOptions {
            max_time_of_impact: 1.0,
            target_distance: 0.0,
            stop_at_penetration: true,
            compute_impact_geometry_on_penetration: true,
        };

        let mut best: Option<ShapeHit> = None;
        for (coord, chunk) in self.chunks_for_aabb(vec_from_na(swept.mins), vec_from_na(swept.maxs))
        {
            let local_pose = isometry(world_grid::world_to_local(coord, position), rotation);
            let local_aabb = shape.compute_aabb(&local_pose);
            let local_swept = local_aabb
                .merged(&local_aabb.translated(&vec_na(delta)))
                .loosened(AABB_MARGIN);

            let mut candidates: Vec<u32> = Vec::new();
            chunk
                .trimesh
                .qbvh()
                .intersect_aabb(&local_swept, &mut candidates);

            for i in candidates {
                let tri = chunk.trimesh.triangle(i);
                let Ok(Some(hit)) = cast_shapes(
                    &local_pose,
                    &vec_na(delta),
                    shape,
                    &Isometry::identity(),
                    &Vector::zeros(),
                    &tri,
                    options,
                ) else {
                    continue;
                };
                let id = chunk.triangle_ids[i as usize];
                if better(
                    hit.time_of_impact,
                    id,
                    &best.map(|b| (b.toi, b.triangle_id)),
                ) {
                    best = Some(ShapeHit {
                        toi: hit.time_of_impact,
                        position: world_grid::local_to_world(coord, vec_from_na(hit.witness2)),
                        normal: vec_glam(*hit.normal2),
                        triangle_id: id,
                        flags: chunk.triangle_flags[i as usize],
                    });
                }
            }
        }
        best
    }

    /// Contact/overlap probe: all triangles within `prediction` distance of
    /// `shape` (negative `distance` = penetration). Border-duplicated
    /// triangles are deduplicated by stable id, keeping the deepest contact.
    /// Results are sorted by (distance, triangle id) — deepest first.
    pub fn contacts(
        &self,
        shape: &dyn Shape,
        position: Vec3,
        rotation: Quat,
        prediction: f32,
    ) -> Vec<ContactHit> {
        let world_pose = isometry(position, rotation);
        let aabb = shape
            .compute_aabb(&world_pose)
            .loosened(prediction + AABB_MARGIN);

        let mut by_id: HashMap<TriangleId, ContactHit> = HashMap::new();
        for (coord, chunk) in self.chunks_for_aabb(vec_from_na(aabb.mins), vec_from_na(aabb.maxs)) {
            let local_pose = isometry(world_grid::world_to_local(coord, position), rotation);
            let local_aabb = shape
                .compute_aabb(&local_pose)
                .loosened(prediction + AABB_MARGIN);

            let mut candidates: Vec<u32> = Vec::new();
            chunk
                .trimesh
                .qbvh()
                .intersect_aabb(&local_aabb, &mut candidates);

            for i in candidates {
                let tri = chunk.trimesh.triangle(i);
                let Ok(Some(c)) =
                    contact(&local_pose, shape, &Isometry::identity(), &tri, prediction)
                else {
                    continue;
                };
                let id = chunk.triangle_ids[i as usize];
                let hit = ContactHit {
                    distance: c.dist,
                    position: world_grid::local_to_world(coord, vec_from_na(c.point2)),
                    normal: vec_glam(*c.normal2),
                    triangle_id: id,
                    flags: chunk.triangle_flags[i as usize],
                };
                by_id
                    .entry(id)
                    .and_modify(|existing| {
                        if hit.distance < existing.distance {
                            *existing = hit;
                        }
                    })
                    .or_insert(hit);
            }
        }
        let mut out: Vec<ContactHit> = by_id.into_values().collect();
        out.sort_by(|a, b| {
            (a.distance, a.triangle_id)
                .partial_cmp(&(b.distance, b.triangle_id))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        out
    }
}

/// Earliest-TOI comparison with deterministic tie-break on lowest triangle id.
fn better(toi: f32, id: TriangleId, best: &Option<(f32, TriangleId)>) -> bool {
    match best {
        None => true,
        Some((best_toi, best_id)) => toi < *best_toi || (toi == *best_toi && id < *best_id),
    }
}

fn isometry(position: Vec3, rotation: Quat) -> Isometry<f32> {
    Isometry::from_parts(
        na::Translation3::new(position.x, position.y, position.z),
        na::UnitQuaternion::from_quaternion(na::Quaternion::new(
            rotation.w, rotation.x, rotation.y, rotation.z,
        )),
    )
}

fn vec_na(v: Vec3) -> Vector<f32> {
    Vector::new(v.x, v.y, v.z)
}

fn vec_glam(v: Vector<f32>) -> Vec3 {
    Vec3::new(v.x, v.y, v.z)
}

fn vec_from_na(p: Point<f32>) -> Vec3 {
    Vec3::new(p.x, p.y, p.z)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collision::format::{write_chunk, ChunkData, TriMeshSection};
    use crate::world_grid::CHUNK_SIZE;
    use parry3d::shape::Ball;

    /// A chunk at `coord` with a full-chunk ground quad at world z = 0
    /// (two triangles, ids `base_id` / `base_id + 1`, flags 7).
    fn ground_chunk(coord: IVec2, base_id: u64) -> Vec<u8> {
        let s = CHUNK_SIZE;
        let chunk = ChunkData {
            coord,
            local_aabb: ([0.0, 0.0, 0.0], [s, s, 0.0]),
            trimesh: TriMeshSection {
                vertices: vec![[0.0, 0.0, 0.0], [s, 0.0, 0.0], [s, s, 0.0], [0.0, s, 0.0]],
                indices: vec![[0, 1, 2], [0, 2, 3]],
                triangle_ids: vec![base_id, base_id + 1],
                triangle_flags: vec![7, 7],
            },
        };
        write_chunk(&chunk, 1)
    }

    fn store_with_ground() -> ChunkStore {
        let mut store = ChunkStore::new();
        store
            .insert_chunk(&ground_chunk(IVec2::new(0, 0), 100))
            .unwrap();
        store
    }

    #[test]
    fn raycast_down_hits_ground() {
        let store = store_with_ground();
        let hit = store
            .raycast(Vec3::new(10.0, 20.0, 5.0), Vec3::new(0.0, 0.0, -1.0), 100.0)
            .unwrap();
        assert!((hit.toi - 5.0).abs() < 1e-5);
        assert!(hit.position.abs_diff_eq(Vec3::new(10.0, 20.0, 0.0), 1e-4));
        assert!(hit.normal.abs_diff_eq(Vec3::Z, 1e-5));
        assert_eq!(hit.flags, 7);
    }

    #[test]
    fn raycast_miss_and_range_cap() {
        let store = store_with_ground();
        // Outside the chunk on XY.
        assert!(store
            .raycast(
                Vec3::new(-10.0, -10.0, 5.0),
                Vec3::new(0.0, 0.0, -1.0),
                100.0
            )
            .is_none());
        // Too short to reach the ground.
        assert!(store
            .raycast(Vec3::new(10.0, 10.0, 5.0), Vec3::new(0.0, 0.0, -1.0), 2.0)
            .is_none());
    }

    #[test]
    fn raycast_tie_breaks_to_lowest_id() {
        let store = store_with_ground();
        // The quad diagonal (x == y) is shared by both triangles: equal TOI.
        let hit = store
            .raycast(Vec3::new(30.0, 30.0, 5.0), Vec3::new(0.0, 0.0, -1.0), 10.0)
            .unwrap();
        assert_eq!(hit.triangle_id, 100);
    }

    #[test]
    fn shape_cast_ball_onto_ground() {
        let store = store_with_ground();
        let hit = store
            .cast_shape(
                &Ball::new(0.5),
                Vec3::new(10.0, 10.0, 5.0),
                Quat::IDENTITY,
                Vec3::new(0.0, 0.0, -10.0),
            )
            .unwrap();
        assert!((hit.toi - 0.45).abs() < 1e-4);
        assert!(hit.position.abs_diff_eq(Vec3::new(10.0, 10.0, 0.0), 1e-3));
        assert!(hit.normal.abs_diff_eq(Vec3::Z, 1e-4));
        assert_eq!(hit.flags, 7);
    }

    #[test]
    fn shape_cast_reports_initial_penetration_at_zero_toi() {
        let store = store_with_ground();
        let hit = store
            .cast_shape(
                &Ball::new(0.5),
                // Strictly inside one triangle (off the quad diagonal): a
                // per-triangle EPA against an edge gives a lateral normal.
                Vec3::new(10.0, 20.0, 0.2),
                Quat::IDENTITY,
                Vec3::new(0.0, 0.0, -1.0),
            )
            .unwrap();
        assert_eq!(hit.toi, 0.0);
        assert!(hit.normal.abs_diff_eq(Vec3::Z, 1e-4));
    }

    #[test]
    fn contacts_report_penetration_depth() {
        let store = store_with_ground();
        let hits = store.contacts(
            &Ball::new(0.5),
            Vec3::new(10.0, 10.0, 0.3),
            Quat::IDENTITY,
            0.0,
        );
        assert!(!hits.is_empty());
        assert!((hits[0].distance + 0.2).abs() < 1e-4);
        assert!(hits[0].normal.abs_diff_eq(Vec3::Z, 1e-4));
    }

    #[test]
    fn border_duplicates_resolve_to_single_consistent_hit() {
        // Both chunks carry a copy of the same border geometry with the same
        // stable ids — mimicking cooked border duplication.
        let s = CHUNK_SIZE;
        let mut store = ChunkStore::new();
        for coord in [IVec2::new(0, 0), IVec2::new(1, 0)] {
            // World quad x ∈ [62, 66], y ∈ [0, 64], z = 0, expressed chunk-local.
            let ox = coord.x as f32 * s;
            let verts = [
                [62.0 - ox, 0.0, 0.0],
                [66.0 - ox, 0.0, 0.0],
                [66.0 - ox, s, 0.0],
                [62.0 - ox, s, 0.0],
            ];
            let chunk = ChunkData {
                coord,
                local_aabb: ([verts[0][0], 0.0, 0.0], [verts[1][0], s, 0.0]),
                trimesh: TriMeshSection {
                    vertices: verts.to_vec(),
                    indices: vec![[0, 1, 2], [0, 2, 3]],
                    triangle_ids: vec![500, 501],
                    triangle_flags: vec![1, 1],
                },
            };
            store.insert_chunk(&write_chunk(&chunk, 1)).unwrap();
        }

        // Ray straight down onto the duplicated strip, right at the border.
        let hit = store
            .raycast(Vec3::new(64.0, 32.0, 5.0), Vec3::new(0.0, 0.0, -1.0), 10.0)
            .unwrap();
        assert!((hit.toi - 5.0).abs() < 1e-4);
        assert_eq!(hit.triangle_id, 500);

        // Sweep crossing the border sees one hit with consistent attribution.
        let sweep = store
            .cast_shape(
                &Ball::new(0.5),
                Vec3::new(60.0, 32.0, 5.0),
                Quat::IDENTITY,
                Vec3::new(8.0, 0.0, -10.0),
            )
            .unwrap();
        assert!(sweep.toi > 0.0 && sweep.toi <= 1.0);
        assert!(sweep.triangle_id == 500 || sweep.triangle_id == 501);
    }

    #[test]
    fn insert_rejects_corrupt_bytes() {
        let mut store = ChunkStore::new();
        assert!(matches!(
            store.insert_chunk(b"not a chunk"),
            Err(ChunkLoadError::Format(_))
        ));
        assert!(store.is_empty());
    }

    #[test]
    fn remove_and_clear() {
        let mut store = store_with_ground();
        assert!(store.contains(IVec2::new(0, 0)));
        assert!(store.remove_chunk(IVec2::new(0, 0)));
        assert!(!store.remove_chunk(IVec2::new(0, 0)));
        assert!(store.is_empty());
    }
}
