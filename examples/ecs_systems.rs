use hecs::World;
use rust_engine::engine::ecs::{
    components::*,
    systems::{System, SystemScheduler, MovementSystem, RotationSystem},
};

fn main() {
    let mut world = World::new();
    let mut scheduler = SystemScheduler::new();

    // Spawn test entities
    world.spawn((
        Transform::new(nalgebra_glm::vec3(0.0, 0.0, 0.0)),
        Player,
        Name::new("Player"),
    ));

    // Add systems
    scheduler.add_system(Box::new(MovementSystem { speed: 2.0 }));
    scheduler.add_system(Box::new(RotationSystem { rotation_speed: 1.0 }));

    // Simulate 5 frames
    let delta_time = 1.0 / 60.0; // 60 FPS
    for frame in 0..5 {
        scheduler.update(&mut world, delta_time);

        println!("=== Frame {} ===", frame + 1);
        for (_id, (transform, name)) in world.query::<(&Transform, &Name)>().iter() {
            println!(
                "{}: pos=({:.2}, {:.2}, {:.2})",
                name.0, transform.position.x, transform.position.y, transform.position.z
            );
        }
    }
}