//! Integration tests for scene serialization round-trips.
//!
//! These tests verify that serializable ECS components survive a
//! serialize → deserialize cycle through the RON scene format.

mod common;

use rust_engine::engine::ecs::components::*;
use rust_engine::engine::ecs::hierarchy::Parent;
use rust_engine::engine::physics::{Collider, RigidBody};
use rust_engine::engine::scene::{load_scene_from_string, serialize_scene_to_string};

use common::assert_approx_eq;

use common::{spawn_child_entity, spawn_named_entity};

/// Serialize a world to RON and deserialize into a fresh world.
fn roundtrip(world: &hecs::World, root_order: &[hecs::Entity]) -> hecs::World {
    let ron_string = serialize_scene_to_string(world, "test_scene", root_order)
        .expect("serialize should succeed");
    let mut new_world = hecs::World::new();
    let (_name, _roots) =
        load_scene_from_string(&mut new_world, &ron_string).expect("deserialize should succeed");
    new_world
}

#[test]
fn transform_roundtrip() {
    let mut world = hecs::World::new();
    let entity = spawn_named_entity(&mut world, "Cube", 1.0, 2.0, 3.0);
    // Set non-default scale
    {
        let mut t = world.get::<&mut Transform>(entity).expect("transform");
        t.scale = nalgebra_glm::vec3(0.5, 1.5, 2.5);
    }

    let new_world = roundtrip(&world, &[entity]);
    assert_eq!(new_world.len(), 1);

    for (_, t) in new_world.query::<&Transform>().iter() {
        common::assert_approx_eq(t.position.x, 1.0, 0.001);
        common::assert_approx_eq(t.position.y, 2.0, 0.001);
        common::assert_approx_eq(t.position.z, 3.0, 0.001);
        common::assert_approx_eq(t.scale.x, 0.5, 0.001);
        common::assert_approx_eq(t.scale.y, 1.5, 0.001);
        common::assert_approx_eq(t.scale.z, 2.5, 0.001);
    }
}

#[test]
fn mesh_renderer_roundtrip() {
    let mut world = hecs::World::new();
    let entity = world.spawn((
        Name::new("MeshEntity"),
        EntityGuid::new(),
        Transform::default(),
        MeshRenderer {
            mesh_path: String::new(),
            material_paths: vec![],
            material_path: String::new(),
            mesh_index: 3,
            material_index: 7,
            visible: false,
            cast_shadows: true,
            receive_shadows: false,
            ..Default::default()
        },
    ));

    let new_world = roundtrip(&world, &[entity]);
    for (_, mr) in new_world.query::<&MeshRenderer>().iter() {
        assert_eq!(mr.mesh_index, 3);
        assert_eq!(mr.material_index, 7);
        assert!(!mr.visible);
        assert!(mr.cast_shadows);
        assert!(!mr.receive_shadows);
    }
}

#[test]
fn camera_roundtrip() {
    let mut world = hecs::World::new();
    let entity = world.spawn((
        Name::new("Cam"),
        EntityGuid::new(),
        Transform::default(),
        Camera {
            fov: 90.0,
            near: 0.5,
            far: 500.0,
            active: true,
            projection: CameraProjection::Perspective,
            clear_color: [0.2, 0.3, 0.4],
            priority: 5,
        },
    ));

    let new_world = roundtrip(&world, &[entity]);
    for (_, cam) in new_world.query::<&Camera>().iter() {
        common::assert_approx_eq(cam.fov, 90.0, 0.01);
        common::assert_approx_eq(cam.near, 0.5, 0.01);
        common::assert_approx_eq(cam.far, 500.0, 0.01);
        assert!(cam.active);
        assert_eq!(cam.priority, 5);
    }
}

#[test]
fn directional_light_roundtrip() {
    let mut world = hecs::World::new();
    let entity = world.spawn((
        Name::new("Sun"),
        EntityGuid::new(),
        Transform::default(),
        DirectionalLight {
            direction: nalgebra_glm::vec3(0.0, -1.0, -0.5),
            color: nalgebra_glm::vec3(1.0, 0.9, 0.8),
            intensity: 2.5,
            shadow_enabled: true,
            shadow_bias: 0.01,
        },
    ));

    let new_world = roundtrip(&world, &[entity]);
    for (_, light) in new_world.query::<&DirectionalLight>().iter() {
        common::assert_approx_eq(light.intensity, 2.5, 0.01);
        assert!(light.shadow_enabled);
    }
}

#[test]
fn point_light_roundtrip() {
    let mut world = hecs::World::new();
    let entity = world.spawn((
        Name::new("Lamp"),
        EntityGuid::new(),
        Transform::default(),
        PointLight {
            color: nalgebra_glm::vec3(1.0, 0.5, 0.0),
            intensity: 5.0,
            radius: 20.0,
            shadow_enabled: false,
            falloff: LightFalloff::InverseSquare,
        },
    ));

    let new_world = roundtrip(&world, &[entity]);
    for (_, light) in new_world.query::<&PointLight>().iter() {
        common::assert_approx_eq(light.intensity, 5.0, 0.01);
        common::assert_approx_eq(light.radius, 20.0, 0.01);
    }
}

#[test]
fn rigidbody_collider_roundtrip() {
    let mut world = hecs::World::new();
    let entity = world.spawn((
        Name::new("PhysBox"),
        EntityGuid::new(),
        Transform::default(),
        RigidBody::dynamic().with_mass(5.0),
        Collider::cuboid(1.0, 2.0, 3.0),
    ));

    let new_world = roundtrip(&world, &[entity]);
    for (_, rb) in new_world.query::<&RigidBody>().iter() {
        common::assert_approx_eq(rb.mass, 5.0, 0.01);
    }
    for (_, col) in new_world.query::<&Collider>().iter() {
        // Collider shape is preserved
        match &col.shape {
            rust_engine::engine::physics::ColliderShape::Cuboid { half_extents } => {
                common::assert_approx_eq(half_extents.x, 1.0, 0.01);
                common::assert_approx_eq(half_extents.y, 2.0, 0.01);
                common::assert_approx_eq(half_extents.z, 3.0, 0.01);
            }
            _ => panic!("expected cuboid collider"),
        }
    }
}

#[test]
fn hierarchy_preserved_across_roundtrip() {
    let mut world = hecs::World::new();
    let parent = spawn_named_entity(&mut world, "Parent", 0.0, 0.0, 0.0);
    let _child = spawn_child_entity(&mut world, parent, "Child", 1.0, 0.0, 0.0);

    let new_world = roundtrip(&world, &[parent]);

    // Find the child by name
    let mut found_child = false;
    for (entity, name) in new_world.query::<&Name>().iter() {
        if name.0 == "Child" {
            assert!(
                new_world.get::<&Parent>(entity).is_ok(),
                "child should have Parent component after roundtrip"
            );
            found_child = true;
        }
    }
    assert!(found_child, "child entity should exist after roundtrip");
}

#[test]
fn guid_preserved_across_roundtrip() {
    let mut world = hecs::World::new();
    let entity = spawn_named_entity(&mut world, "GuidTest", 0.0, 0.0, 0.0);
    let original_guid = world
        .get::<&EntityGuid>(entity)
        .expect("guid should exist")
        .0;

    let new_world = roundtrip(&world, &[entity]);

    let mut found = false;
    for (_, guid) in new_world.query::<&EntityGuid>().iter() {
        if guid.0 == original_guid {
            found = true;
        }
    }
    assert!(found, "GUID should be preserved across roundtrip");
}

#[test]
fn empty_scene_roundtrip() {
    let world = hecs::World::new();
    let ron_string =
        serialize_scene_to_string(&world, "empty", &[]).expect("serialize empty should succeed");

    let mut new_world = hecs::World::new();
    let (name, roots) =
        load_scene_from_string(&mut new_world, &ron_string).expect("deserialize should succeed");

    assert_eq!(name, "empty");
    assert!(roots.is_empty());
    assert_eq!(new_world.len(), 0);
}

#[test]
fn invalid_ron_returns_error() {
    let mut world = hecs::World::new();
    let result = load_scene_from_string(&mut world, "this is not valid RON!!!");
    assert!(result.is_err(), "invalid RON should return error");
}

#[test]
fn player_component_roundtrip() {
    let mut world = hecs::World::new();
    let entity = world.spawn((
        Name::new("PlayerEntity"),
        EntityGuid::new(),
        Transform::default(),
        Player,
    ));

    let new_world = roundtrip(&world, &[entity]);
    let mut found_player = false;
    for (_, _) in new_world.query::<&Player>().iter() {
        found_player = true;
    }
    assert!(found_player, "Player component should survive roundtrip");
}

#[test]
fn multiple_entities_roundtrip() {
    let mut world = hecs::World::new();
    let e1 = spawn_named_entity(&mut world, "A", 1.0, 0.0, 0.0);
    let e2 = spawn_named_entity(&mut world, "B", 2.0, 0.0, 0.0);
    let e3 = spawn_named_entity(&mut world, "C", 3.0, 0.0, 0.0);

    let new_world = roundtrip(&world, &[e1, e2, e3]);
    assert_eq!(new_world.len(), 3);

    // Verify names exist
    let names: Vec<String> = new_world
        .query::<&Name>()
        .iter()
        .map(|(_, n)| n.0.clone())
        .collect();
    assert!(names.contains(&"A".to_string()));
    assert!(names.contains(&"B".to_string()));
    assert!(names.contains(&"C".to_string()));
}

#[test]
fn particle_effect_roundtrip() {
    let mut world = hecs::World::new();
    let entity = world.spawn((
        Name::new("ParticleEmitter"),
        EntityGuid::new(),
        Transform::default(),
        ParticleEffect {
            enabled: true,
            capacity: 1024,
            emission_rate: 50.0,
            burst_count: 10,
            burst_interval: 0.5,
            spawn_shape: SpawnShape::Sphere { radius: 2.0 },
            lifetime_min: 0.5,
            lifetime_max: 3.0,
            initial_velocity: [1.0, 2.0, 3.0],
            velocity_variance: 1.5,
            update_modules: vec![
                UpdateModule::Gravity([0.0, 0.0, -9.8]),
                UpdateModule::Wind([1.0, 0.0, 0.0]),
                UpdateModule::Drag(0.5),
                UpdateModule::CurlNoise { strength: 2.0, scale: 0.5, speed: 1.0 },
                UpdateModule::ColorOverLife {
                    start: [1.0, 0.5, 0.0, 1.0],
                    end: [1.0, 0.0, 0.0, 0.0],
                },
                UpdateModule::SizeOverLife { start: 0.2, end: 0.05 },
            ],
            render_mode: RenderMode::Billboard,
            texture_path: "textures/spark.png".to_string(),
            soft_fade_distance: 0.5,
            show_gizmos: true,
        },
    ));

    let new_world = roundtrip(&world, &[entity]);

    let mut found = false;
    for (_, effect) in new_world.query::<&ParticleEffect>().iter() {
        found = true;
        assert!(effect.enabled);
        assert_eq!(effect.capacity, 1024);
        assert_approx_eq(effect.emission_rate, 50.0, 0.01);
        assert_eq!(effect.burst_count, 10);
        assert_approx_eq(effect.burst_interval, 0.5, 0.01);
        match effect.spawn_shape {
            SpawnShape::Sphere { radius } => {
                assert_approx_eq(radius, 2.0, 0.01);
            }
            _ => panic!("expected Sphere spawn shape"),
        }
        assert_approx_eq(effect.lifetime_min, 0.5, 0.01);
        assert_approx_eq(effect.lifetime_max, 3.0, 0.01);
        assert_approx_eq(effect.initial_velocity[0], 1.0, 0.01);
        assert_approx_eq(effect.initial_velocity[1], 2.0, 0.01);
        assert_approx_eq(effect.initial_velocity[2], 3.0, 0.01);
        assert_approx_eq(effect.velocity_variance, 1.5, 0.01);

        // Verify modules survived roundtrip
        assert_eq!(effect.update_modules.len(), 6);
        assert_approx_eq(effect.gravity().unwrap()[2], -9.8, 0.01);
        assert_approx_eq(effect.wind().unwrap()[0], 1.0, 0.01);
        assert_approx_eq(effect.drag().unwrap(), 0.5, 0.01);
        let (turb_str, turb_scale, turb_speed) = effect.curl_noise().unwrap();
        assert_approx_eq(turb_str, 2.0, 0.01);
        assert_approx_eq(turb_scale, 0.5, 0.01);
        assert_approx_eq(turb_speed, 1.0, 0.01);
        let (color_start, color_end) = effect.color_over_life().unwrap();
        assert_approx_eq(color_start[0], 1.0, 0.01);
        assert_approx_eq(color_start[1], 0.5, 0.01);
        assert_approx_eq(color_end[3], 0.0, 0.01);
        let (size_start, size_end) = effect.size_over_life().unwrap();
        assert_approx_eq(size_start, 0.2, 0.01);
        assert_approx_eq(size_end, 0.05, 0.01);

        assert_eq!(effect.texture_path, "textures/spark.png");
        assert_approx_eq(effect.soft_fade_distance, 0.5, 0.01);
        assert!(effect.show_gizmos);
    }
    assert!(found, "ParticleEffect should survive roundtrip");
}

#[test]
fn particle_effect_capacity_clamped_on_deserialize() {
    let mut world = hecs::World::new();
    let entity = world.spawn((
        Name::new("ClampTest"),
        EntityGuid::new(),
        Transform::default(),
        ParticleEffect {
            capacity: 8192, // above max
            ..ParticleEffect::default()
        },
    ));

    let new_world = roundtrip(&world, &[entity]);
    for (_, effect) in new_world.query::<&ParticleEffect>().iter() {
        assert_eq!(effect.capacity, 4096, "capacity should be clamped to max");
    }
}
