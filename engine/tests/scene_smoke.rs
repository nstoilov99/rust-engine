//! Smoke test: build a large scene with 50+ entities, serialize and deserialize
//! without panicking. Verifies that the full pipeline handles diverse component
//! combinations at scale.

mod common;

use rust_engine::engine::ecs::components::*;
use rust_engine::engine::ecs::hierarchy::set_parent;
use rust_engine::engine::physics::{Collider, RigidBody};
use rust_engine::engine::scene::{load_scene_from_string, serialize_scene_to_string};

#[test]
fn smoke_test_50_entities_roundtrip() {
    let mut world = hecs::World::new();
    let mut roots = Vec::new();

    // 20 mesh entities
    for i in 0..20 {
        let entity = world.spawn((
            Name::new(format!("Mesh_{}", i)),
            EntityGuid::new(),
            Transform::new(nalgebra_glm::vec3(i as f32, 0.0, 0.0)),
            MeshRenderer::default(),
        ));
        roots.push(entity);
    }

    // 10 physics entities
    for i in 0..10 {
        let entity = world.spawn((
            Name::new(format!("Phys_{}", i)),
            EntityGuid::new(),
            Transform::new(nalgebra_glm::vec3(0.0, i as f32, 5.0)),
            RigidBody::dynamic(),
            Collider::cuboid(0.5, 0.5, 0.5),
        ));
        roots.push(entity);
    }

    // 5 lights
    for i in 0..3 {
        let entity = world.spawn((
            Name::new(format!("PointLight_{}", i)),
            EntityGuid::new(),
            Transform::new(nalgebra_glm::vec3(0.0, 0.0, 10.0 + i as f32)),
            PointLight::default(),
        ));
        roots.push(entity);
    }
    for i in 0..2 {
        let entity = world.spawn((
            Name::new(format!("DirLight_{}", i)),
            EntityGuid::new(),
            Transform::default(),
            DirectionalLight::default(),
        ));
        roots.push(entity);
    }

    // 1 camera
    let cam = world.spawn((
        Name::new("Camera"),
        EntityGuid::new(),
        Transform::default(),
        Camera::default(),
    ));
    roots.push(cam);

    // 10 hierarchy entities (5 parents with 1 child each)
    for i in 0..5 {
        let parent = world.spawn((
            Name::new(format!("HParent_{}", i)),
            EntityGuid::new(),
            Transform::new(nalgebra_glm::vec3(i as f32 * 3.0, 0.0, 0.0)),
        ));
        roots.push(parent);

        let child = world.spawn((
            Name::new(format!("HChild_{}", i)),
            EntityGuid::new(),
            Transform::new(nalgebra_glm::vec3(0.0, 1.0, 0.0)),
        ));
        set_parent(&mut world, child, parent);
    }

    // 5 varied component combos
    let varied = world.spawn((
        Name::new("VarCapsule"),
        EntityGuid::new(),
        Transform::new(nalgebra_glm::vec3(100.0, 0.0, 0.0)),
        RigidBody::kinematic(),
        Collider::capsule(1.0, 0.5),
    ));
    roots.push(varied);

    let varied2 = world.spawn((
        Name::new("VarBall"),
        EntityGuid::new(),
        Transform::default(),
        RigidBody::fixed(),
        Collider::ball(2.0),
    ));
    roots.push(varied2);

    let varied3 = world.spawn((
        Name::new("VarPlayer"),
        EntityGuid::new(),
        Transform::default(),
        Player,
    ));
    roots.push(varied3);

    let varied4 = world.spawn((
        Name::new("VarOrthoCamera"),
        EntityGuid::new(),
        Transform::default(),
        Camera {
            projection: CameraProjection::Orthographic { size: 10.0 },
            ..Default::default()
        },
    ));
    roots.push(varied4);

    let varied5 = world.spawn((
        Name::new("VarMeshPhys"),
        EntityGuid::new(),
        Transform::default(),
        MeshRenderer {
            mesh_index: 5,
            material_index: 2,
            ..Default::default()
        },
        RigidBody::dynamic().with_mass(10.0),
        Collider::cuboid(1.0, 1.0, 1.0),
    ));
    roots.push(varied5);

    // Total: 20 + 10 + 5 + 1 + 5*2 + 5 = 51 entities
    assert!(
        world.len() >= 50,
        "scene should have 50+ entities, got {}",
        world.len()
    );

    // Serialize
    let ron_string = serialize_scene_to_string(&world, "smoke_test", &roots)
        .expect("serialize should not panic");

    // Deserialize into fresh world
    let mut new_world = hecs::World::new();
    let (name, _new_roots) =
        load_scene_from_string(&mut new_world, &ron_string).expect("deserialize should not panic");

    assert_eq!(name, "smoke_test");
    assert!(
        new_world.len() >= 50,
        "restored scene should have 50+ entities, got {}",
        new_world.len()
    );

    // Verify at least some entities have expected components
    let transform_count = new_world.query::<&Transform>().iter().count();
    assert!(
        transform_count >= 50,
        "most entities should have transforms, got {}",
        transform_count
    );

    let mesh_count = new_world.query::<&MeshRenderer>().iter().count();
    assert!(
        mesh_count >= 20,
        "should have mesh renderers, got {}",
        mesh_count
    );

    let rb_count = new_world.query::<&RigidBody>().iter().count();
    assert!(rb_count >= 10, "should have rigidbodies, got {}", rb_count);
}
