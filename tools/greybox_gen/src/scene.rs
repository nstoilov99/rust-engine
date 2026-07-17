//! Scene + world-manifest emission.
//!
//! v2 (M4): terrain cells live only in the world manifest — the scene holds
//! just the sun, camera and gym. The streamer spawns cell entities at runtime
//! (identity transform — world placement is baked into the mesh) using the
//! manifest's `root_entity_guid`. GUIDs are derived deterministically from
//! (seed, kind, coord) so regeneration preserves identity (M4/M5 rely on
//! stable GUIDs for spawn/despawn and replication).

use game_shared::world_manifest::{
    CellEntry, ControllerRef, SpawnPoint, WorldManifest, ZoneEntry,
};
use rust_engine::engine::scene::scene_format::{ComponentData, EntityData, SceneFile};

use crate::params::{
    CELL_SIZE, JUMP_GAP, MAX_CELL, MIN_CELL, SEED, SLOPE_LIMIT_DEG, STEP_HEIGHT,
};
use crate::terrain::cell_mesh_path;

pub const SCENE_PATH: &str = "scenes/greybox.scene";
pub const MANIFEST_PATH: &str = "world/greybox.world.ron";

fn fnv1a(bytes: &[u8]) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for &b in bytes {
        h = (h ^ b as u32).wrapping_mul(0x0100_0193);
    }
    h
}

fn mix(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^= x >> 16;
    x
}

/// Deterministic GUID from (seed, kind, a, b), formatted as a version-4
/// variant-1 UUID string so it is indistinguishable from editor-made GUIDs.
pub fn det_guid(kind: &str, a: i32, b: i32) -> String {
    let base = mix(SEED ^ fnv1a(kind.as_bytes()))
        ^ mix(a as u32).rotate_left(1)
        ^ mix((b as u32).wrapping_mul(0x9e37_79b9)).rotate_left(7);
    let mut bytes = [0u8; 16];
    for (i, chunk) in bytes.chunks_mut(4).enumerate() {
        chunk.copy_from_slice(&mix(base.wrapping_add(i as u32 * 0x517c_c1b7)).to_le_bytes());
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40; // version 4
    bytes[8] = (bytes[8] & 0x3f) | 0x80; // variant 1
    let h = |r: std::ops::Range<usize>| {
        bytes[r].iter().map(|b| format!("{b:02x}")).collect::<String>()
    };
    format!(
        "{}-{}-{}-{}-{}",
        h(0..4),
        h(4..6),
        h(6..8),
        h(8..10),
        h(10..16)
    )
}

/// Quadrant zone: 0=(−,−) 1=(+,−) 2=(−,+) 3=(+,+).
pub fn zone_of(cx: i32, cy: i32) -> u32 {
    (if cx >= 0 { 1 } else { 0 }) + (if cy >= 0 { 2 } else { 0 })
}

pub(crate) fn mesh_renderer(mesh_path: String, base_color: [f32; 4]) -> ComponentData {
    ComponentData::MeshRenderer {
        mesh_path,
        material_paths: Vec::new(),
        material_path: String::new(),
        mesh_index: 0,
        material_index: 0,
        visible: true,
        cast_shadows: true,
        receive_shadows: true,
        base_color_factor: base_color,
        metallic_factor: 0.0,
        roughness_factor: 0.9,
        emissive_factor: [0.0; 3],
    }
}

pub fn build_scene() -> SceneFile {
    let mut entities = Vec::new();

    entities.push(EntityData {
        name: "Sun".into(),
        guid: Some(det_guid("sun", 0, 0)),
        components: vec![ComponentData::DirectionalLight {
            direction: [0.4, 0.3, -0.85],
            color: [1.0, 1.0, 1.0],
            intensity: 1.0,
            shadow_enabled: false,
            shadow_bias: 0.005,
        }],
    });
    entities.push(EntityData {
        name: "Main Camera".into(),
        guid: Some(det_guid("camera", 0, 0)),
        components: vec![
            ComponentData::Transform {
                position: [-24.0, 0.0, 14.0],
                rotation: [0.0, 0.0, 0.0, 1.0],
                scale: [1.0; 3],
            },
            ComponentData::Camera {
                fov: 60.0,
                near: 0.1,
                far: 1000.0,
                active: true,
                projection: Default::default(),
                clear_color: [0.1, 0.1, 0.15],
                priority: 0,
            },
        ],
    });

    entities.extend(crate::gym::gym_entities());

    SceneFile {
        version: "1.0".into(),
        name: "Greybox World".into(),
        entities,
    }
}

pub fn build_manifest() -> WorldManifest {
    let mut cells = Vec::new();
    let mut zones: Vec<ZoneEntry> = (0..4).map(|id| ZoneEntry { id, cells: Vec::new() }).collect();

    for cy in MIN_CELL..MAX_CELL {
        for cx in MIN_CELL..MAX_CELL {
            let zone_id = zone_of(cx, cy);
            zones[zone_id as usize].cells.push([cx, cy]);
            cells.push(CellEntry {
                coord: [cx, cy],
                zone_id,
                root_entity_guid: det_guid("cell", cx, cy),
                mesh: cell_mesh_path(cx, cy),
                collision_chunk: format!("collision/greybox/{cx}_{cy}.ccol"),
            });
        }
    }

    WorldManifest {
        version: 1,
        scene: SCENE_PATH.into(),
        cell_size: CELL_SIZE,
        controller_ref: ControllerRef {
            slope_limit_deg: SLOPE_LIMIT_DEG,
            step_height: STEP_HEIGHT,
            jump_gap: JUMP_GAP,
        },
        cells,
        zones,
        spawn_points: {
            let mut sp = vec![
                SpawnPoint {
                    name: "default_spawn".into(),
                    position: [0.0, 0.0, 8.0],
                },
                SpawnPoint {
                    name: "route_north".into(),
                    position: [0.0, 152.0, 8.0],
                },
                SpawnPoint {
                    name: "route_west".into(),
                    position: [-152.0, 0.0, 7.0],
                },
            ];
            sp.extend(crate::gym::gym_spawn_points());
            sp
        },
    }
}

/// RON bytes exactly as the engine's own scene serializer would write them.
pub fn scene_bytes(scene: &SceneFile) -> Vec<u8> {
    ron::ser::to_string_pretty(scene, ron::ser::PrettyConfig::default())
        .expect("scene serialization cannot fail")
        .into_bytes()
}

pub fn manifest_bytes(manifest: &WorldManifest) -> Vec<u8> {
    ron::ser::to_string_pretty(manifest, ron::ser::PrettyConfig::default())
        .expect("manifest serialization cannot fail")
        .into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guids_are_stable_and_distinct() {
        assert_eq!(det_guid("cell", -4, 3), det_guid("cell", -4, 3));
        assert_ne!(det_guid("cell", -4, 3), det_guid("cell", 3, -4));
        assert_ne!(det_guid("cell", 0, 0), det_guid("sun", 0, 0));
        let g = det_guid("cell", 1, 2);
        assert_eq!(g.len(), 36);
        assert_eq!(&g[14..15], "4"); // version nibble
    }

    #[test]
    fn cells_live_only_in_the_manifest() {
        let scene = build_scene();
        let manifest = build_manifest();
        assert_eq!(manifest.cells.len(), 64);
        for cell in &manifest.cells {
            assert!(
                !scene
                    .entities
                    .iter()
                    .any(|e| e.guid.as_deref() == Some(cell.root_entity_guid.as_str())),
                "cell {:?} must not be a scene entity (streamed at runtime)",
                cell.coord
            );
        }
    }

    #[test]
    fn zones_partition_the_cell_set() {
        let manifest = build_manifest();
        let total: usize = manifest.zones.iter().map(|z| z.cells.len()).sum();
        assert_eq!(total, 64);
        for z in &manifest.zones {
            assert_eq!(z.cells.len(), 16, "zone {}", z.id);
            for c in &z.cells {
                assert_eq!(zone_of(c[0], c[1]), z.id);
            }
        }
    }

    #[test]
    fn scene_round_trips_through_engine_loader() {
        use rust_engine::engine::scene::scene_serializer::load_scene_from_string;
        let bytes = scene_bytes(&build_scene());
        let mut world = rust_engine::engine::ecs::World::new();
        let (name, roots) =
            load_scene_from_string(&mut world, std::str::from_utf8(&bytes).unwrap()).unwrap();
        assert_eq!(name, "Greybox World");
        assert_eq!(roots.len(), 2 + crate::gym::gym_entities().len());
    }
}
