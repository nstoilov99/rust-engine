//! World management utilities

use hecs::World;

/// Create a new empty ECS world
pub fn create_world() -> World {
    World::new()
}

/// Helper for spawning common entity types
pub struct EntityBuilder {
    world: *mut World,
}

impl EntityBuilder {
    pub fn new(world: &mut World) -> Self {
        Self {
            world: world as *mut World,
        }
    }

    /// Spawn a basic 3D entity with transform and mesh
    pub fn spawn_mesh(
        &mut self,
        position: nalgebra_glm::Vec3,
        mesh_index: usize,
        material_index: usize,
    ) -> hecs::Entity {
        use super::components::*;

        let world = unsafe { &mut *self.world };
        world.spawn((
            Transform::new(position),
            MeshRenderer {
                mesh_index,
                material_index,
                ..Default::default()
            },
        ))
    }

    /// Spawn a camera entity
    pub fn spawn_camera(&mut self, position: nalgebra_glm::Vec3) -> hecs::Entity {
        use super::components::*;

        let world = unsafe { &mut *self.world };
        world.spawn((
            Transform::new(position),
            Camera::default(),
            crate::engine::ecs::components::Name::new("Main Camera"),
        ))
    }

    /// Spawn a directional light
    pub fn spawn_directional_light(
        &mut self,
        direction: nalgebra_glm::Vec3,
        color: nalgebra_glm::Vec3,
        intensity: f32,
    ) -> hecs::Entity {
        use super::components::*;

        let world = unsafe { &mut *self.world };
        world.spawn((
            Transform::default(),
            DirectionalLight {
                direction,
                color,
                intensity,
                ..Default::default()
            },
            crate::engine::ecs::components::Name::new("Directional Light"),
        ))
    }
}
