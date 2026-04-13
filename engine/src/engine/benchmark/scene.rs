use super::BenchmarkConfig;
use crate::engine::ecs::components::{
    Camera, DirectionalLight, EntityGuid, MeshRenderer, Name, PointLight, Transform,
};
use crate::engine::ecs::hierarchy::{get_root_entities, set_parent};
use crate::engine::physics::{Collider, PhysicsWorld, RigidBody};
use crate::engine::scene::load_scene;
use hecs::World;
use nalgebra_glm as glm;

pub const BENCHMARK_SCENE_RELATIVE: &str = "scenes/benchmark.scene.ron";

/// Load the saved benchmark scene if it exists, otherwise spawn the default one.
pub fn load_or_create_benchmark_scene(
    world: &mut World,
    physics_world: &mut PhysicsWorld,
    config: &BenchmarkConfig,
    cube_mesh_index: usize,
) -> Result<Vec<hecs::Entity>, Box<dyn std::error::Error>> {
    world.clear();
    *physics_world = PhysicsWorld::new();

    let roots = if crate::engine::assets::asset_source::exists(BENCHMARK_SCENE_RELATIVE) {
        let (_, roots) = load_scene(world, BENCHMARK_SCENE_RELATIVE)?;
        roots
    } else {
        spawn_benchmark_scene(world, config, cube_mesh_index)
    };

    register_scene_physics(world, physics_world);
    Ok(roots)
}

/// Spawn a deterministic benchmark scene into the world.
pub fn spawn_benchmark_scene(
    world: &mut World,
    config: &BenchmarkConfig,
    cube_mesh_index: usize,
) -> Vec<hecs::Entity> {
    let mut rng = SimpleRng::new(config.seed);

    let point_light_count = 12u32.min((config.entity_count / 20).max(4));
    let hierarchy_chain_count = 25u32.min((config.entity_count / 20).max(5));
    let hierarchy_depth = 5u32;
    let hierarchy_entity_count = hierarchy_chain_count * hierarchy_depth;
    let physics_count = 75u32.min((config.entity_count / 6).max(10));
    let reserved = 3 + point_light_count + hierarchy_entity_count + physics_count;
    let mesh_count = config.entity_count.saturating_sub(reserved);

    let camera_rotation = glm::quat_angle_axis(0.37, &glm::vec3(0.0, 1.0, 0.0));
    spawn_with_guid(
        world,
        (
            Transform::new(glm::vec3(-90.0, 0.0, 35.0)).with_rotation(camera_rotation),
            Camera {
                far: 500.0,
                ..Default::default()
            },
            Name::new("BenchmarkCamera"),
        ),
    );

    spawn_with_guid(
        world,
        (
            DirectionalLight {
                direction: glm::normalize(&glm::vec3(1.0, -0.35, -1.0)),
                intensity: 1.3,
                ..Default::default()
            },
            Name::new("BenchmarkSun"),
        ),
    );

    for index in 0..point_light_count {
        spawn_with_guid(
            world,
            (
                Transform::new(glm::vec3(
                    rng.range_f32(-40.0, 40.0),
                    rng.range_f32(-40.0, 40.0),
                    rng.range_f32(8.0, 20.0),
                )),
                PointLight {
                    color: glm::vec3(
                        rng.range_f32(0.4, 1.0),
                        rng.range_f32(0.4, 1.0),
                        rng.range_f32(0.4, 1.0),
                    ),
                    intensity: rng.range_f32(3.0, 8.0),
                    radius: rng.range_f32(8.0, 16.0),
                    ..Default::default()
                },
                Name::new(format!("BenchmarkLight{index}")),
            ),
        );
    }

    spawn_with_guid(
        world,
        (
            Transform::new(glm::vec3(0.0, 0.0, -0.5)).with_scale(glm::vec3(120.0, 120.0, 1.0)),
            MeshRenderer {
                mesh_index: cube_mesh_index,
                material_index: 0,
                ..Default::default()
            },
            RigidBody::fixed(),
            Collider::cuboid(60.0, 60.0, 0.5),
            Name::new("BenchmarkGround"),
        ),
    );

    for index in 0..mesh_count {
        let yaw = glm::quat_angle_axis(
            rng.range_f32(-std::f32::consts::PI, std::f32::consts::PI),
            &glm::vec3(0.0, 0.0, 1.0),
        );
        let pitch = glm::quat_angle_axis(rng.range_f32(-0.5, 0.5), &glm::vec3(0.0, 1.0, 0.0));
        let scale = rng.range_f32(0.4, 1.6);
        spawn_with_guid(
            world,
            (
                Transform::new(glm::vec3(
                    rng.range_f32(-55.0, 55.0),
                    rng.range_f32(-55.0, 55.0),
                    rng.range_f32(0.0, 12.0),
                ))
                .with_rotation(yaw * pitch)
                .with_scale(glm::vec3(scale, scale, scale)),
                MeshRenderer {
                    mesh_index: cube_mesh_index,
                    material_index: (index % 8) as usize,
                    ..Default::default()
                },
                Name::new(format!("BenchmarkMesh{index}")),
            ),
        );
    }

    for chain_index in 0..hierarchy_chain_count {
        let root = spawn_with_guid(
            world,
            (
                Transform::new(glm::vec3(
                    -35.0 + chain_index as f32 * 2.8,
                    rng.range_f32(-20.0, 20.0),
                    1.5,
                )),
                MeshRenderer {
                    mesh_index: cube_mesh_index,
                    material_index: (chain_index % 8) as usize,
                    ..Default::default()
                },
                Name::new(format!("BenchmarkChain{chain_index}_0")),
            ),
        );

        let mut parent = root;
        for depth in 1..hierarchy_depth {
            let child = spawn_with_guid(
                world,
                (
                    Transform::new(glm::vec3(
                        1.4 + rng.range_f32(-0.2, 0.2),
                        rng.range_f32(-0.3, 0.3),
                        0.9 + rng.range_f32(-0.2, 0.2),
                    ))
                    .with_rotation(glm::quat_angle_axis(
                        rng.range_f32(-0.35, 0.35),
                        &glm::vec3(0.0, 1.0, 0.0),
                    ))
                    .with_scale(glm::vec3(0.75, 0.75, 0.75)),
                    MeshRenderer {
                        mesh_index: cube_mesh_index,
                        material_index: ((chain_index + depth) % 8) as usize,
                        ..Default::default()
                    },
                    Name::new(format!("BenchmarkChain{chain_index}_{depth}")),
                ),
            );
            set_parent(world, child, parent);
            parent = child;
        }
    }

    for index in 0..physics_count {
        let height = 4.0 + (index / 5) as f32 * 1.15;
        spawn_with_guid(
            world,
            (
                Transform::new(glm::vec3(
                    -8.0 + (index % 5) as f32 * 1.8,
                    -6.0 + ((index / 5) % 5) as f32 * 1.8,
                    height,
                ))
                .with_scale(glm::vec3(0.75, 0.75, 0.75)),
                MeshRenderer {
                    mesh_index: cube_mesh_index,
                    material_index: (index % 8) as usize,
                    ..Default::default()
                },
                RigidBody::dynamic()
                    .with_mass(rng.range_f32(0.5, 3.0))
                    .with_linear_damping(0.1),
                Collider::cuboid(0.375, 0.375, 0.375).with_restitution(rng.range_f32(0.1, 0.6)),
                Name::new(format!("BenchmarkBody{index}")),
            ),
        );
    }

    get_root_entities(world)
}

fn spawn_with_guid(world: &mut World, bundle: impl hecs::DynamicBundle) -> hecs::Entity {
    let entity = world.spawn(bundle);
    let _ = world.insert_one(entity, EntityGuid::new());
    entity
}

fn register_scene_physics(world: &mut World, physics_world: &mut PhysicsWorld) {
    for (_, (transform, rigidbody, collider)) in world
        .query::<(&Transform, &mut RigidBody, &mut Collider)>()
        .iter()
    {
        physics_world.register_entity(transform, rigidbody, collider);
    }
}

struct SimpleRng {
    state: u64,
}

impl SimpleRng {
    fn new(seed: u64) -> Self {
        Self { state: seed.max(1) }
    }

    fn next_u64(&mut self) -> u64 {
        self.state ^= self.state << 13;
        self.state ^= self.state >> 7;
        self.state ^= self.state << 17;
        self.state
    }

    fn next_f32(&mut self) -> f32 {
        (self.next_u64() % 10_000) as f32 / 10_000.0
    }

    fn range_f32(&mut self, min: f32, max: f32) -> f32 {
        min + self.next_f32() * (max - min)
    }
}
