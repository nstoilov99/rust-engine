use hecs::World;
use rust_engine::engine::ecs::{components::*, EntityBuilder};

fn main() {
    // Create ECS world
    let mut world = World::new();

    println!("=== ECS Demo ===\n");

    // Spawn entities using the world directly
    let player = world.spawn((
        Transform::new(nalgebra_glm::vec3(0.0, 0.0, 0.0)),
        Player,
        Name::new("Hero"),
    ));

    let enemy1 = world.spawn((
        Transform::new(nalgebra_glm::vec3(5.0, 0.0, 0.0)),
        Name::new("Goblin"),
    ));

    let enemy2 = world.spawn((
        Transform::new(nalgebra_glm::vec3(-5.0, 0.0, 0.0)),
        Name::new("Orc"),
    ));

    // Spawn a camera
    let camera = world.spawn((
        Transform::new(nalgebra_glm::vec3(0.0, 5.0, 10.0)),
        Camera::default(),
        Name::new("Main Camera"),
    ));

    // Spawn a light
    let light = world.spawn((
        DirectionalLight {
            direction: nalgebra_glm::vec3(0.0, -1.0, -1.0),
            color: nalgebra_glm::vec3(1.0, 1.0, 1.0),
            intensity: 1.0,
        },
        Name::new("Sun"),
    ));

    println!("Spawned {} entities\n", world.len());

    // Query all entities with Transform and Name
    println!("=== All Named Entities ===");
    for (id, (transform, name)) in world.query::<(&Transform, &Name)>().iter() {
        println!(
            "Entity {:?}: {} at position ({:.2}, {:.2}, {:.2})",
            id, name.0, transform.position.x, transform.position.y, transform.position.z
        );
    }

    // Query only player entities
    println!("\n=== Player Entities ===");
    for (id, (transform, _player, name)) in world.query::<(&Transform, &Player, &Name)>().iter() {
        println!(
            "Player {:?}: {} at position ({:.2}, {:.2}, {:.2})",
            id, name.0, transform.position.x, transform.position.y, transform.position.z
        );
    }

    // Query cameras
    println!("\n=== Camera Entities ===");
    for (id, (camera, name)) in world.query::<(&Camera, &Name)>().iter() {
        println!(
            "Camera {:?}: {} (FOV: {}, Active: {})",
            id, name.0, camera.fov, camera.active
        );
    }

    // Modify transforms
    println!("\n=== Moving Entities ===");
    for (_id, transform) in world.query_mut::<&mut Transform>() {
        transform.position.y += 1.0; // Move up by 1 unit
    }

    println!("All entities moved up by 1 unit");

    // Show updated positions
    println!("\n=== Updated Positions ===");
    for (id, (transform, name)) in world.query::<(&Transform, &Name)>().iter() {
        println!(
            "Entity {:?}: {} now at ({:.2}, {:.2}, {:.2})",
            id, name.0, transform.position.x, transform.position.y, transform.position.z
        );
    }

    // Remove an entity
    world.despawn(enemy1).unwrap();
    println!("\n=== After Despawning Enemy ===");
    println!("Remaining entities: {}", world.len());

    // Add component to existing entity
    world.insert_one(player, PointLight::default()).unwrap();
    println!("\nAdded PointLight component to player");

    // Check if entity has component
    if world.get::<&PointLight>(player).is_ok() {
        println!("Player now has a PointLight component!");
    }
}