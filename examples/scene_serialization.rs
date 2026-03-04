use hecs::World;
use nalgebra_glm as glm;
use rust_engine::engine::ecs::components::*;
use rust_engine::engine::scene::{load_scene, save_scene};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Scene Serialization Demo ===\n");

    // Create world and spawn entities
    let mut world = World::new();

    world.spawn((
        Transform::new(glm::vec3(0.0, 5.0, 10.0)),
        Camera::default(),
        Name::new("Main Camera"),
    ));

    world.spawn((
        Transform::new(glm::vec3(0.0, 0.0, 0.0)).with_scale(glm::vec3(0.01, 0.01, 0.01)),
        MeshRenderer {
            mesh_index: 0,
            material_index: 0,
        },
        Name::new("Duck"),
    ));

    world.spawn((
        DirectionalLight {
            direction: glm::vec3(0.0, -1.0, -1.0),
            color: glm::vec3(1.0, 1.0, 1.0),
            intensity: 1.0,
        },
        Name::new("Sun"),
    ));

    println!("Created world with {} entities\n", world.len());

    // Save scene
    save_scene(&world, "assets/scenes/demo.scene.ron", "Demo Scene")?;

    // Clear world
    world.clear();
    println!("\nCleared world: {} entities remaining\n", world.len());

    // Load scene
    let scene_name = load_scene(&mut world, "assets/scenes/demo.scene.ron")?;
    println!(
        "\nLoaded scene '{}' with {} entities\n",
        scene_name,
        world.len()
    );

    // Verify loaded entities
    println!("=== Loaded Entities ===");
    for (id, (transform, name)) in world.query::<(&Transform, &Name)>().iter() {
        println!(
            "Entity {:?}: {} at ({:.2}, {:.2}, {:.2})",
            id, name.0, transform.position.x, transform.position.y, transform.position.z
        );
    }

    Ok(())
}
