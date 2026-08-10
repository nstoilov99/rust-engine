//! Game setup functions for scene initialization, physics, and assets
//!
//! Extracts setup code from main.rs for better organization.

use hecs::Entity;
use hecs::World;
use nalgebra_glm as glm;
use rust_engine::assets::AssetManager;
#[cfg(feature = "editor")]
use rust_engine::assets::{HotReloadWatcher, ReloadEvent};
use rust_engine::engine::ecs::components::DirectionalLight as EcsDirectionalLight;
use rust_engine::engine::ecs::components::{Camera, MeshRenderer, Name, Transform};
use rust_engine::engine::physics::{Collider, RigidBody};
use rust_engine::engine::rendering::rendering_3d::mesh::{
    create_primitive, PRIMITIVE_CUBE, PRIMITIVE_PLANE, PRIMITIVE_SPHERE,
};
use rust_engine::engine::scene::load_scene;
#[cfg(feature = "editor")]
use rust_engine::Renderer;
#[cfg(feature = "editor")]
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;

/// Setup asset manager and hot-reload system (editor only)
#[cfg(feature = "editor")]
#[allow(clippy::type_complexity)]
pub fn setup_asset_system(
    renderer: &Renderer,
) -> Result<(Arc<AssetManager>, HotReloadWatcher, Receiver<ReloadEvent>), Box<dyn std::error::Error>>
{
    let asset_manager = Arc::new(AssetManager::new(
        renderer.gpu.device.clone(),
        renderer.gpu.queue.clone(),
        renderer.gpu.memory_allocator.clone(),
        renderer.gpu.command_buffer_allocator.clone(),
    ));

    // Setup hot-reload channel
    let (reload_tx, reload_rx): (Sender<ReloadEvent>, Receiver<ReloadEvent>) = mpsc::channel();

    // Setup hot-reload watcher
    let mut hot_reload = HotReloadWatcher::new(asset_manager.clone(), reload_tx);
    let content_dir = rust_engine::assets::content_root();
    hot_reload.watch_directory(&content_dir.to_string_lossy())?;
    let duck_fs_path = rust_engine::assets::asset_source::resolve("models/Duck.glb");
    hot_reload.track_asset(&duck_fs_path.to_string_lossy());

    Ok((asset_manager, hot_reload, reload_rx))
}

/// Load model and create procedural meshes (registered with named paths).
pub fn load_assets(
    asset_manager: &Arc<AssetManager>,
) -> Result<(Vec<usize>, usize, usize), Box<dyn std::error::Error>> {
    let (mesh_indices, _duck_model) = asset_manager.load_model_gpu("models/Duck.glb")?;

    let upload_primitive = |path: &str| -> Result<usize, Box<dyn std::error::Error>> {
        let (verts, idx) = create_primitive(path).ok_or("unknown primitive")?;
        asset_manager.upload_procedural_mesh_named(&verts, &idx, Some(path))
    };
    let plane_mesh_index = upload_primitive(PRIMITIVE_PLANE)?;
    let cube_mesh_index = upload_primitive(PRIMITIVE_CUBE)?;
    upload_primitive(PRIMITIVE_SPHERE)?;

    Ok((mesh_indices, plane_mesh_index, cube_mesh_index))
}

/// Create default scene with camera, duck, and light
pub fn create_default_scene(world: &mut World, mesh_index: usize) {
    // Spawn Camera entity
    world.spawn((
        Transform::new(glm::vec3(0.0, 5.0, 10.0)),
        Camera::default(),
        Name::new("Main Camera"),
    ));

    // Spawn Duck entity with 180° rotation around X-axis to flip upside-down models
    let flip_rotation = glm::quat_angle_axis(std::f32::consts::PI, &glm::vec3(1.0, 0.0, 0.0));
    world.spawn((
        Transform::new(glm::vec3(0.0, 0.0, 0.0))
            .with_rotation(flip_rotation)
            .with_scale(glm::vec3(0.01, 0.01, 0.01)),
        MeshRenderer {
            mesh_index,
            material_index: 0,
            ..Default::default()
        },
        Name::new("Duck"),
    ));

    // Spawn Directional Light
    world.spawn((
        EcsDirectionalLight {
            direction: glm::vec3(0.0, -1.0, -1.0),
            color: glm::vec3(1.0, 1.0, 1.0),
            intensity: 1.0,
            ..Default::default()
        },
        Name::new("Sun"),
    ));
}

/// Load scene from file or create default
/// Returns (scene_was_loaded, root_entities_in_order)
/// - scene_was_loaded: true if loaded from file, false if default was created
/// - root_entities_in_order: order of root entities (for HierarchyPanel)
pub fn load_or_create_scene(
    world: &mut World,
    mesh_index: usize,
    scene_relative: &str,
) -> Result<(bool, Vec<Entity>), Box<dyn std::error::Error>> {
    if rust_engine::assets::asset_source::exists(scene_relative) {
        let (_scene_name, root_entities) = load_scene(world, scene_relative)?;
        Ok((true, root_entities))
    } else if scene_relative == "scenes/main.scene"
        && rust_engine::assets::asset_source::exists("scenes/main.scene.ron")
    {
        log::warn!(
            "Loading legacy 'scenes/main.scene.ron' — rename via tools/migrate_asset_extensions"
        );
        let (_scene_name, root_entities) = load_scene(world, "scenes/main.scene.ron")?;
        Ok((true, root_entities))
    } else {
        create_default_scene(world, mesh_index);
        Ok((false, Vec::new()))
    }
}

/// Configuration for spawning a physics test object
pub struct PhysicsObjectConfig {
    pub position: glm::Vec3,
    pub scale: f32,
    pub mass: f32,
    pub restitution: f32,
    pub is_ball: bool,
    pub mesh_index: usize,
    pub name: &'static str,
}

/// Spawn a physics test object (helper to avoid duplication)
fn spawn_physics_object(world: &mut World, config: PhysicsObjectConfig) {
    let half_extent = config.scale / 2.0;
    let collider = if config.is_ball {
        Collider::ball(half_extent).with_restitution(config.restitution)
    } else {
        Collider::cuboid(half_extent, half_extent, half_extent).with_restitution(config.restitution)
    };

    world.spawn((
        Transform::new(config.position).with_scale(glm::vec3(
            config.scale,
            config.scale,
            config.scale,
        )),
        MeshRenderer {
            mesh_index: config.mesh_index,
            material_index: 0,
            ..Default::default()
        },
        RigidBody::dynamic().with_mass(config.mass),
        collider,
        Name::new(config.name),
    ));
}

/// Spawn physics test objects (ground and falling cubes)
///
/// Now uses Z-up coordinates: objects spawn at Z heights and fall in -Z direction.
pub fn spawn_physics_test_objects(world: &mut World, plane_mesh: usize, cube_mesh: usize) {
    // Ground plane (static - never moves)
    // In Z-up: ground is at Z = -0.5
    world.spawn((
        Transform::new(glm::vec3(0.0, 0.0, -0.5)).with_scale(glm::vec3(10.0, 1.0, 10.0)),
        MeshRenderer {
            mesh_index: plane_mesh,
            material_index: 0,
            ..Default::default()
        },
        RigidBody::fixed(),
        Collider::cuboid(5.0, 5.0, 0.01),
        Name::new("Ground"),
    ));

    // Falling cubes - use helper to avoid duplication
    // In Z-up: objects spawn at Z heights (3.0, 5.0, 7.0) and fall in -Z direction
    let cubes = [
        PhysicsObjectConfig {
            position: glm::vec3(0.0, 0.0, 3.0), // Z-up: height is Z
            scale: 0.5,
            mass: 1.0,
            restitution: 0.7,
            is_ball: false,
            mesh_index: cube_mesh,
            name: "FallingCube1",
        },
        PhysicsObjectConfig {
            position: glm::vec3(1.0, 0.5, 5.0), // Z-up: height is Z
            scale: 0.4,
            mass: 0.5,
            restitution: 0.5,
            is_ball: false,
            mesh_index: cube_mesh,
            name: "FallingCube2",
        },
        PhysicsObjectConfig {
            position: glm::vec3(-1.0, 0.0, 7.0), // Z-up: height is Z
            scale: 0.6,
            mass: 2.0,
            restitution: 0.9,
            is_ball: true,
            mesh_index: cube_mesh,
            name: "BouncyBox",
        },
    ];

    for config in cubes {
        spawn_physics_object(world, config);
    }
}

// `register_physics_entities` lived here until 39.8 P2. Body registration is
// now `engine::physics::rebuild_bodies_from_world`, reached only through
// `world_population::after_world_populated`, so there is exactly one seam for
// P5 to move into `RapierPhysicsPlugin`.

#[cfg(feature = "editor")]
pub fn print_controls() {
    // Controls are now shown in the Engine Stats panel instead of console
}
