use hecs::World;
use rust_engine::engine::ecs::{
    components::*,
    resources::Resources,
    schedule::{FunctionSystem, Schedule, Stage, System},
};

fn main() {
    let mut world = World::new();
    let mut resources = Resources::new();
    resources.insert(rust_engine::engine::ecs::resources::Time::new());
    resources.insert(rust_engine::engine::ecs::resources::EditorState::new());
    let mut cmd = rust_engine::engine::ecs::commands::CommandBuffer::new();

    // Spawn test entities
    world.spawn((
        Transform::new(nalgebra_glm::vec3(0.0, 0.0, 0.0)),
        Player,
        Name::new("Player"),
    ));

    // Build schedule with function systems
    let mut schedule = Schedule::new();
    schedule.add_fn_system("movement", Stage::Update, |world: &mut World, _r: &mut Resources| {
        let speed = 2.0_f32;
        let delta_time = 1.0 / 60.0_f32;
        for (_id, (transform, _player)) in world.query::<(&mut Transform, &Player)>().iter() {
            transform.position.z -= speed * delta_time;
        }
    });

    // Simulate 5 frames
    for frame in 0..5 {
        schedule.run_raw(&mut world, &mut resources, &mut cmd);

        println!("=== Frame {} ===", frame + 1);
        for (_id, (transform, name)) in world.query::<(&Transform, &Name)>().iter() {
            println!(
                "{}: pos=({:.2}, {:.2}, {:.2})",
                name.0, transform.position.x, transform.position.y, transform.position.z
            );
        }
    }
}
