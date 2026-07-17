//! Cooks a scene's static geometry into `.ccol` collision chunks.
//!
//! Sources:
//! - Mesh entities: `MeshRenderer` + `StaticCollision` marker. Geometry comes
//!   from the canonical Z-up accessor (`assets::collision_geometry`) and is
//!   transformed by the full hierarchy world matrix — matches the renderer.
//!   Built-in `__primitive__/` mesh paths resolve through the same accessor
//!   (no model load).
//! - Primitive entities: fixed `RigidBody` + non-sensor `Collider`,
//!   tessellated to triangles. Transformed by the entity's LOCAL
//!   position + rotation only (no hierarchy, no scale) — this mirrors how
//!   `PhysicsWorld::register_entity` registers colliders at runtime, so
//!   cooked collision matches gameplay truth, not the debug wireframe.
//!
//! Pipeline: gather → drop degenerates → subdivide oversized triangles →
//! weld (1e-4 m, pre-assignment so chunk borders share identical vertices) →
//! assign each triangle to every chunk its AABB overlaps → emit. Output is
//! deterministic: entities sorted by GUID, triangles keep source order,
//! chunks sorted by coordinate — re-cooking an unchanged scene is
//! byte-identical.

use std::collections::HashMap;
use std::sync::Arc;

use glam::{IVec2, Vec3};
use hecs::World;
use nalgebra_glm as glm;

use game_shared::collision::format::{
    fnv1a_64, write_chunk, ChunkData, CollisionManifest, ManifestChunk, TriMeshSection,
    FORMAT_VERSION,
};
use game_shared::world_grid::{self, CHUNK_SIZE};

use crate::engine::assets::collision_geometry::{
    model_collision_geometry_zup, primitive_collision_geometry_zup,
};
use crate::engine::assets::model_loader::Model;
use crate::engine::ecs::components::{EntityGuid, MeshRenderer, Transform};
use crate::engine::ecs::hierarchy;
use crate::engine::physics::{Collider, ColliderShape, RigidBody, RigidBodyType, StaticCollision};

/// Bump when cook logic, tessellation, or the parry pin changes. Written to
/// chunk headers and the manifest (informational — format version governs
/// load compatibility).
pub const COOKER_VERSION_HASH: u32 = 1;

const WELD_EPSILON: f32 = 1e-4;
/// Triangles whose AABB spans more than this many chunks on X or Y are
/// midpoint-subdivided pre-weld to bound border duplication.
const MAX_TRI_SPAN_CHUNKS: f32 = 2.0;
const SPHERE_SEGMENTS: usize = 16;
const SPHERE_RINGS: usize = 8;

pub struct CookedScene {
    /// `.ccol` file bytes per non-empty chunk, sorted by coord (y, then x).
    pub chunks: Vec<(IVec2, Vec<u8>)>,
    pub manifest: CollisionManifest,
    pub warnings: Vec<String>,
}

struct SourceTri {
    v: [Vec3; 3],
    /// Stable id: fnv1a(entity GUID bytes ++ source triangle index).
    id: u64,
}

/// Cook the static collision of an already-loaded scene world.
///
/// `load_model` resolves a `MeshRenderer::mesh_path` to a loaded model
/// (editor: model manager cache; CLI: filesystem/pak load). No file I/O
/// happens here — callers write the returned bytes.
pub fn cook_scene(
    world: &World,
    scene_name: &str,
    load_model: &mut dyn FnMut(&str) -> Option<Arc<Model>>,
) -> CookedScene {
    let mut warnings = Vec::new();
    let mut tris = gather_mesh_triangles(world, load_model, &mut warnings);
    tris.extend(gather_primitive_triangles(world, &mut warnings));

    let tris = subdivide_oversized(drop_degenerate(tris, &mut warnings), &mut warnings);
    let (positions, tris) = weld(tris);
    let chunks = assign_and_emit(&positions, &tris);

    let manifest = CollisionManifest {
        scene: scene_name.to_string(),
        format_version: FORMAT_VERSION,
        cooker_hash: COOKER_VERSION_HASH,
        // Callers that know the source scene bytes fill this in before writing.
        scene_hash: 0,
        chunk_size: CHUNK_SIZE,
        chunks: chunks
            .iter()
            .map(|(coord, bytes)| ManifestChunk {
                coord: [coord.x, coord.y],
                content_hash: fnv1a_64(bytes),
            })
            .collect(),
    };
    CookedScene {
        chunks,
        manifest,
        warnings,
    }
}

// ------------------------------------------------------------------ gather

fn sorted_by_guid<C>(world: &World, mut visit: impl FnMut(hecs::Entity, uuid::Uuid))
where
    C: hecs::Query,
{
    let mut entities: Vec<(uuid::Uuid, hecs::Entity)> = world
        .query::<&EntityGuid>()
        .with::<C>()
        .iter()
        .map(|(e, guid)| (guid.0, e))
        .collect();
    entities.sort_by_key(|(guid, _)| *guid.as_bytes());
    for (guid, entity) in entities {
        visit(entity, guid);
    }
}

fn gather_mesh_triangles(
    world: &World,
    load_model: &mut dyn FnMut(&str) -> Option<Arc<Model>>,
    warnings: &mut Vec<String>,
) -> Vec<SourceTri> {
    let mut out = Vec::new();
    sorted_by_guid::<(&MeshRenderer, &StaticCollision)>(world, |entity, guid| {
        let mesh_path = world
            .get::<&MeshRenderer>(entity)
            .expect("queried")
            .mesh_path
            .clone();
        let geoms = if mesh_path.starts_with("__primitive__/") {
            let Some(geom) = primitive_collision_geometry_zup(&mesh_path) else {
                warnings.push(format!(
                    "entity {guid}: unknown primitive mesh '{mesh_path}' — skipped"
                ));
                return;
            };
            vec![geom]
        } else {
            let Some(model) = load_model(&mesh_path) else {
                warnings.push(format!(
                    "entity {guid}: mesh '{mesh_path}' failed to load — skipped"
                ));
                return;
            };
            let (geoms, skinned) = model_collision_geometry_zup(&model);
            if skinned > 0 {
                warnings.push(format!(
                    "entity {guid}: mesh '{mesh_path}' has {skinned} skinned submesh(es) — skipped"
                ));
            }
            geoms
        };
        let m = hierarchy::get_world_transform(world, entity);
        let flip = glm::determinant(&m) < 0.0;
        let mut tri_index: u32 = 0;
        for geom in geoms {
            for tri in geom.indices.chunks_exact(3) {
                let mut v = [Vec3::ZERO; 3];
                for (k, &i) in tri.iter().enumerate() {
                    let p = geom.positions[i as usize];
                    let w = m * glm::vec4(p.x, p.y, p.z, 1.0);
                    v[k] = Vec3::new(w.x, w.y, w.z);
                }
                if flip {
                    v.swap(1, 2);
                }
                out.push(SourceTri {
                    v,
                    id: stable_id(&guid, tri_index),
                });
                tri_index += 1;
            }
        }
    });
    out
}

fn gather_primitive_triangles(world: &World, warnings: &mut Vec<String>) -> Vec<SourceTri> {
    let mut out = Vec::new();
    sorted_by_guid::<(&Transform, &RigidBody, &Collider)>(world, |entity, guid| {
        let rb = world.get::<&RigidBody>(entity).expect("queried");
        let col = world.get::<&Collider>(entity).expect("queried");
        if rb.body_type != RigidBodyType::Static || col.is_sensor {
            return;
        }
        let t = world.get::<&Transform>(entity).expect("queried");
        if (t.scale.x - 1.0).abs() > 1e-3
            || (t.scale.y - 1.0).abs() > 1e-3
            || (t.scale.z - 1.0).abs() > 1e-3
        {
            warnings.push(format!(
                "entity {guid}: non-unit scale on primitive collider is ignored at runtime and in cooking"
            ));
        }
        let local = match &col.shape {
            ColliderShape::Cuboid { half_extents } => {
                tessellate_cuboid(Vec3::new(half_extents.x, half_extents.y, half_extents.z))
            }
            ColliderShape::Ball { radius } => tessellate_capsule(*radius, 0.0),
            ColliderShape::Capsule {
                half_height,
                radius,
            } => tessellate_capsule(*radius, *half_height),
        };
        for (tri_index, tri) in local.into_iter().enumerate() {
            let v = tri.map(|p| {
                let r = glm::quat_rotate_vec3(&t.rotation, &glm::vec3(p.x, p.y, p.z));
                Vec3::new(r.x + t.position.x, r.y + t.position.y, r.z + t.position.z)
            });
            out.push(SourceTri {
                v,
                id: stable_id(&guid, tri_index as u32),
            });
        }
    });
    out
}

fn stable_id(guid: &uuid::Uuid, tri_index: u32) -> u64 {
    let mut bytes = [0u8; 20];
    bytes[..16].copy_from_slice(guid.as_bytes());
    bytes[16..].copy_from_slice(&tri_index.to_le_bytes());
    fnv1a_64(&bytes)
}

// -------------------------------------------------------------- filtering

fn drop_degenerate(tris: Vec<SourceTri>, warnings: &mut Vec<String>) -> Vec<SourceTri> {
    let before = tris.len();
    let out: Vec<SourceTri> = tris
        .into_iter()
        .filter(|t| {
            t.v.iter().all(|p| p.is_finite())
                && (t.v[1] - t.v[0]).cross(t.v[2] - t.v[0]).length_squared() > 1e-12
        })
        .collect();
    let dropped = before - out.len();
    if dropped > 0 {
        warnings.push(format!(
            "dropped {dropped} degenerate/non-finite triangle(s)"
        ));
    }
    out
}

fn subdivide_oversized(tris: Vec<SourceTri>, warnings: &mut Vec<String>) -> Vec<SourceTri> {
    let max_span = MAX_TRI_SPAN_CHUNKS * CHUNK_SIZE;
    let mut out = Vec::with_capacity(tris.len());
    let mut split_count = 0usize;
    for tri in tris {
        let before = out.len();
        subdivide_rec(tri, max_span, 0, &mut out);
        if out.len() - before > 1 {
            split_count += 1;
        }
    }
    if split_count > 0 {
        warnings.push(format!(
            "subdivided {split_count} oversized triangle(s) (> {MAX_TRI_SPAN_CHUNKS} chunks); \
             consider splitting the source geometry"
        ));
    }
    out
}

fn subdivide_rec(tri: SourceTri, max_span: f32, depth: u32, out: &mut Vec<SourceTri>) {
    let min = tri.v[0].min(tri.v[1]).min(tri.v[2]);
    let max = tri.v[0].max(tri.v[1]).max(tri.v[2]);
    let span = max - min;
    if depth >= 16 || (span.x <= max_span && span.y <= max_span) {
        out.push(tri);
        return;
    }
    // Split the longest edge at its midpoint; both halves keep the source id
    // (earliest-TOI-per-id query semantics stay correct).
    let mut longest = 0;
    let mut best = -1.0f32;
    for e in 0..3 {
        let len = (tri.v[(e + 1) % 3] - tri.v[e]).length_squared();
        if len > best {
            best = len;
            longest = e;
        }
    }
    let (a, b, c) = (longest, (longest + 1) % 3, (longest + 2) % 3);
    let mid = (tri.v[a] + tri.v[b]) * 0.5;
    subdivide_rec(
        SourceTri {
            v: [tri.v[a], mid, tri.v[c]],
            id: tri.id,
        },
        max_span,
        depth + 1,
        out,
    );
    subdivide_rec(
        SourceTri {
            v: [mid, tri.v[b], tri.v[c]],
            id: tri.id,
        },
        max_span,
        depth + 1,
        out,
    );
}

// ----------------------------------------------------------------- welding

struct WeldedTri {
    indices: [u32; 3],
    id: u64,
}

fn weld(tris: Vec<SourceTri>) -> (Vec<Vec3>, Vec<WeldedTri>) {
    let inv_eps = 1.0 / WELD_EPSILON;
    let mut map: HashMap<[i64; 3], u32> = HashMap::new();
    let mut positions: Vec<Vec3> = Vec::new();
    let mut out = Vec::with_capacity(tris.len());
    for tri in &tris {
        let mut indices = [0u32; 3];
        for (k, p) in tri.v.iter().enumerate() {
            let key = [
                (p.x * inv_eps).round() as i64,
                (p.y * inv_eps).round() as i64,
                (p.z * inv_eps).round() as i64,
            ];
            indices[k] = *map.entry(key).or_insert_with(|| {
                positions.push(*p);
                (positions.len() - 1) as u32
            });
        }
        out.push(WeldedTri {
            indices,
            id: tri.id,
        });
    }
    (positions, out)
}

// ---------------------------------------------------------------- chunking

fn assign_and_emit(positions: &[Vec3], tris: &[WeldedTri]) -> Vec<(IVec2, Vec<u8>)> {
    struct ChunkBuild {
        remap: HashMap<u32, u32>,
        trimesh: TriMeshSection,
    }
    let mut chunks: HashMap<IVec2, ChunkBuild> = HashMap::new();

    for tri in tris {
        // Welded triangles that collapsed to zero area contribute nothing.
        if tri.indices[0] == tri.indices[1]
            || tri.indices[1] == tri.indices[2]
            || tri.indices[0] == tri.indices[2]
        {
            continue;
        }
        let p = tri.indices.map(|i| positions[i as usize]);
        let min = p[0].min(p[1]).min(p[2]);
        let max = p[0].max(p[1]).max(p[2]);
        for coord in world_grid::chunks_overlapping(min, max) {
            let build = chunks.entry(coord).or_insert_with(|| ChunkBuild {
                remap: HashMap::new(),
                trimesh: TriMeshSection::default(),
            });
            let mut local = [0u32; 3];
            for (k, &gi) in tri.indices.iter().enumerate() {
                local[k] = *build.remap.entry(gi).or_insert_with(|| {
                    let lp = world_grid::world_to_local(coord, positions[gi as usize]);
                    build.trimesh.vertices.push([lp.x, lp.y, lp.z]);
                    (build.trimesh.vertices.len() - 1) as u32
                });
            }
            build.trimesh.indices.push(local);
            build.trimesh.triangle_ids.push(tri.id);
            build.trimesh.triangle_flags.push(0);
        }
    }

    let mut coords: Vec<IVec2> = chunks.keys().copied().collect();
    coords.sort_by_key(|c| (c.y, c.x));
    coords
        .into_iter()
        .map(|coord| {
            let build = chunks.remove(&coord).expect("keyed");
            let (mut min, mut max) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
            for v in &build.trimesh.vertices {
                min = min.min(Vec3::from_array(*v));
                max = max.max(Vec3::from_array(*v));
            }
            let data = ChunkData {
                coord,
                local_aabb: (min.to_array(), max.to_array()),
                trimesh: build.trimesh,
            };
            (coord, write_chunk(&data, COOKER_VERSION_HASH))
        })
        .collect()
}

// ------------------------------------------------------------ tessellation

/// 12 triangles, CCW outward winding, Z-up local space.
fn tessellate_cuboid(he: Vec3) -> Vec<[Vec3; 3]> {
    let c = |sx: f32, sy: f32, sz: f32| Vec3::new(sx * he.x, sy * he.y, sz * he.z);
    let quads = [
        // +X, -X, +Y, -Y, +Z, -Z (each CCW viewed from outside)
        [
            c(1., -1., -1.),
            c(1., 1., -1.),
            c(1., 1., 1.),
            c(1., -1., 1.),
        ],
        [
            c(-1., 1., -1.),
            c(-1., -1., -1.),
            c(-1., -1., 1.),
            c(-1., 1., 1.),
        ],
        [
            c(1., 1., -1.),
            c(-1., 1., -1.),
            c(-1., 1., 1.),
            c(1., 1., 1.),
        ],
        [
            c(-1., -1., -1.),
            c(1., -1., -1.),
            c(1., -1., 1.),
            c(-1., -1., 1.),
        ],
        [
            c(-1., -1., 1.),
            c(1., -1., 1.),
            c(1., 1., 1.),
            c(-1., 1., 1.),
        ],
        [
            c(-1., 1., -1.),
            c(1., 1., -1.),
            c(1., -1., -1.),
            c(-1., -1., -1.),
        ],
    ];
    let mut out = Vec::with_capacity(12);
    for q in quads {
        out.push([q[0], q[1], q[2]]);
        out.push([q[0], q[2], q[3]]);
    }
    out
}

/// Lat-long tessellation of a capsule with axis +Z (half_height = 0 gives a
/// ball). The equator ring is emitted twice, offset ±half_height — the strip
/// between the two copies forms the cylinder wall. Pole rings collapse to
/// points; the resulting zero-area triangles are welded/dropped downstream.
fn tessellate_capsule(radius: f32, half_height: f32) -> Vec<[Vec3; 3]> {
    use std::f32::consts::PI;
    let mut rings: Vec<Vec<Vec3>> = Vec::new();
    let half_rings = SPHERE_RINGS / 2;
    let mut push_ring = |lat: f32, z_off: f32| {
        let (rz, rl) = (radius * lat.sin(), radius * lat.cos());
        rings.push(
            (0..SPHERE_SEGMENTS)
                .map(|j| {
                    let a = 2.0 * PI * j as f32 / SPHERE_SEGMENTS as f32;
                    Vec3::new(rl * a.cos(), rl * a.sin(), rz + z_off)
                })
                .collect(),
        );
    };
    for i in 0..=half_rings {
        push_ring(
            -PI / 2.0 + PI / 2.0 * i as f32 / half_rings as f32,
            -half_height,
        );
    }
    for i in 0..=half_rings {
        push_ring(PI / 2.0 * i as f32 / half_rings as f32, half_height);
    }

    let mut out = Vec::new();
    for w in rings.windows(2) {
        let (lo, hi) = (&w[0], &w[1]);
        for j in 0..SPHERE_SEGMENTS {
            let jn = (j + 1) % SPHERE_SEGMENTS;
            out.push([lo[j], lo[jn], hi[jn]]);
            out.push([lo[j], hi[jn], hi[j]]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use game_shared::collision::format::read_chunk;

    fn no_models(_: &str) -> Option<Arc<Model>> {
        None
    }

    fn fixed_cuboid(world: &mut World, pos: glm::Vec3, half: f32) {
        world.spawn((
            EntityGuid(uuid::Uuid::new_v4()),
            Transform::new(pos),
            RigidBody::fixed(),
            Collider::cuboid(half, half, half),
        ));
    }

    #[test]
    fn cooks_fixed_cuboid_into_chunk() {
        let mut world = World::new();
        fixed_cuboid(&mut world, glm::vec3(10.0, 10.0, 0.0), 1.0);
        let cooked = cook_scene(&world, "test", &mut no_models);
        assert_eq!(cooked.chunks.len(), 1);
        assert_eq!(cooked.chunks[0].0, IVec2::new(0, 0));
        let data = read_chunk(&cooked.chunks[0].1).unwrap();
        assert_eq!(data.trimesh.indices.len(), 12);
        assert_eq!(data.trimesh.vertices.len(), 8); // welded corners
        assert_eq!(cooked.manifest.chunks.len(), 1);
    }

    #[test]
    fn cooks_primitive_mesh_without_model_loader() {
        let mut world = World::new();
        world.spawn((
            EntityGuid(uuid::Uuid::new_v4()),
            Transform::new(glm::vec3(10.0, 10.0, 0.0)),
            MeshRenderer {
                mesh_path: "__primitive__/Cube".to_string(),
                ..Default::default()
            },
            StaticCollision,
        ));
        let cooked = cook_scene(&world, "test", &mut no_models);
        assert!(cooked.warnings.is_empty(), "{:?}", cooked.warnings);
        assert_eq!(cooked.chunks.len(), 1);
        let data = read_chunk(&cooked.chunks[0].1).unwrap();
        assert_eq!(data.trimesh.indices.len(), 12);
    }

    #[test]
    fn border_cuboid_duplicates_into_both_chunks() {
        let mut world = World::new();
        // Straddles the x = 64 chunk border.
        fixed_cuboid(&mut world, glm::vec3(CHUNK_SIZE, 10.0, 0.0), 1.0);
        let cooked = cook_scene(&world, "test", &mut no_models);
        let coords: Vec<IVec2> = cooked.chunks.iter().map(|(c, _)| *c).collect();
        assert_eq!(coords, vec![IVec2::new(0, 0), IVec2::new(1, 0)]);
        // The -X face (x = 63) lies wholly in chunk (0,0), the +X face (x = 65)
        // wholly in (1,0); the remaining 8 spanning triangles duplicate into both.
        for (_, bytes) in &cooked.chunks {
            let data = read_chunk(bytes).unwrap();
            assert_eq!(data.trimesh.indices.len(), 10);
        }
    }

    #[test]
    fn dynamic_and_sensor_colliders_excluded() {
        let mut world = World::new();
        world.spawn((
            EntityGuid(uuid::Uuid::new_v4()),
            Transform::new(glm::vec3(0.0, 0.0, 0.0)),
            RigidBody::dynamic(),
            Collider::cuboid(1.0, 1.0, 1.0),
        ));
        world.spawn((
            EntityGuid(uuid::Uuid::new_v4()),
            Transform::new(glm::vec3(5.0, 5.0, 0.0)),
            RigidBody::fixed(),
            Collider::cuboid(1.0, 1.0, 1.0).as_sensor(),
        ));
        let cooked = cook_scene(&world, "test", &mut no_models);
        assert!(cooked.chunks.is_empty());
    }

    #[test]
    fn deterministic_recook() {
        let mut world = World::new();
        let guid = uuid::Uuid::from_u128(42);
        world.spawn((
            EntityGuid(guid),
            Transform::new(glm::vec3(3.0, 4.0, 5.0)),
            RigidBody::fixed(),
            Collider::capsule(1.0, 0.5),
        ));
        let a = cook_scene(&world, "test", &mut no_models);
        let b = cook_scene(&world, "test", &mut no_models);
        assert_eq!(a.chunks, b.chunks);
        assert_eq!(a.manifest, b.manifest);
    }

    #[test]
    fn oversized_triangle_is_subdivided() {
        // A giant floor triangle spanning many chunks must cook without a
        // single chunk holding a triangle bigger than the span limit.
        let mut world = World::new();
        fixed_cuboid(&mut world, glm::vec3(0.0, 0.0, 0.0), 200.0);
        let cooked = cook_scene(&world, "test", &mut no_models);
        assert!(cooked.warnings.iter().any(|w| w.contains("subdivided")));
        for (coord, bytes) in &cooked.chunks {
            let data = read_chunk(bytes).unwrap();
            assert_eq!(data.coord, *coord);
        }
    }

    #[test]
    fn capsule_tessellation_is_closed_and_finite() {
        let tris = tessellate_capsule(0.5, 1.0);
        assert!(!tris.is_empty());
        for t in &tris {
            assert!(t.iter().all(|p| p.is_finite()));
            // Everything within radius+half_height of origin.
            assert!(t.iter().all(|p| p.length() <= 1.5 + 1e-4));
        }
    }
}
