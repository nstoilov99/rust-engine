//! Deferred structural operations for the ECS world.
//!
//! Commands are queued during system execution and applied between stages
//! by GameWorld. This prevents borrow conflicts during iteration.

use hecs::Entity;

/// A deferred command to be applied to the hecs world.
///
/// Named `EcsCommand` to avoid collision with the editor's undo/redo `Command`.
pub enum EcsCommand {
    /// Spawn an entity using a closure that calls world.spawn(...).
    Spawn(Box<dyn FnOnce(&mut hecs::World) -> Entity + Send + Sync>),

    /// Despawn an entity and all its children (via hierarchy::despawn_recursive).
    Despawn(Entity),

    /// Insert a component on an existing entity.
    InsertComponent {
        entity: Entity,
        insert_fn: Box<dyn FnOnce(&mut hecs::World) + Send + Sync>,
    },

    /// Remove a component from an entity.
    RemoveComponent {
        entity: Entity,
        remove_fn: Box<dyn FnOnce(&mut hecs::World) + Send + Sync>,
    },

    /// Execute an arbitrary closure against the world.
    Custom(Box<dyn FnOnce(&mut hecs::World) + Send + Sync>),
}

/// Buffer for deferred ECS commands.
///
/// Systems push commands here during execution. GameWorld applies them
/// between stages via `apply()`.
///
/// **Lives ONLY in GameWorld** — not in Resources, not duplicated.
pub struct CommandBuffer {
    commands: Vec<EcsCommand>,
}

impl CommandBuffer {
    pub fn new() -> Self {
        Self {
            commands: Vec::new(),
        }
    }

    /// Queue spawning an entity from a component bundle closure.
    pub fn spawn<F>(&mut self, spawn_fn: F)
    where
        F: FnOnce(&mut hecs::World) -> Entity + Send + Sync + 'static,
    {
        self.commands.push(EcsCommand::Spawn(Box::new(spawn_fn)));
    }

    /// Queue despawning an entity (and its children via despawn_recursive).
    pub fn despawn(&mut self, entity: Entity) {
        self.commands.push(EcsCommand::Despawn(entity));
    }

    /// Queue inserting a component on an entity.
    pub fn insert<T: Send + Sync + 'static>(&mut self, entity: Entity, component: T) {
        self.commands.push(EcsCommand::InsertComponent {
            entity,
            insert_fn: Box::new(move |world: &mut hecs::World| {
                let _ = world.insert_one(entity, component);
            }),
        });
    }

    /// Queue removing a component from an entity.
    pub fn remove<T: Send + Sync + 'static>(&mut self, entity: Entity) {
        self.commands.push(EcsCommand::RemoveComponent {
            entity,
            remove_fn: Box::new(move |world: &mut hecs::World| {
                let _ = world.remove_one::<T>(entity);
            }),
        });
    }

    /// Queue a custom operation.
    pub fn custom<F>(&mut self, op: F)
    where
        F: FnOnce(&mut hecs::World) + Send + Sync + 'static,
    {
        self.commands.push(EcsCommand::Custom(Box::new(op)));
    }

    /// Apply all buffered commands to the world, then clear the buffer.
    pub fn apply(&mut self, world: &mut hecs::World) {
        for command in self.commands.drain(..) {
            match command {
                EcsCommand::Spawn(spawn_fn) => {
                    spawn_fn(world);
                }
                EcsCommand::Despawn(entity) => {
                    super::hierarchy::despawn_recursive(world, entity);
                }
                EcsCommand::InsertComponent { insert_fn, .. } => {
                    insert_fn(world);
                }
                EcsCommand::RemoveComponent { remove_fn, .. } => {
                    remove_fn(world);
                }
                EcsCommand::Custom(op) => {
                    op(world);
                }
            }
        }
    }

    /// Number of pending commands.
    pub fn len(&self) -> usize {
        self.commands.len()
    }

    /// Whether the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    /// Clear without applying.
    pub fn clear(&mut self) {
        self.commands.clear();
    }
}

impl Default for CommandBuffer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hecs::World;

    #[test]
    fn spawn_via_command_buffer() {
        let mut world = World::new();
        let mut buf = CommandBuffer::new();

        buf.spawn(|w: &mut World| w.spawn((42_i32,)));
        assert_eq!(buf.len(), 1);

        buf.apply(&mut world);
        assert_eq!(world.len(), 1);
        assert!(buf.is_empty());
    }

    #[test]
    fn despawn_via_command_buffer() {
        let mut world = World::new();
        let entity = world.spawn((100_i32,));
        assert_eq!(world.len(), 1);

        let mut buf = CommandBuffer::new();
        buf.despawn(entity);
        buf.apply(&mut world);

        assert_eq!(world.len(), 0);
    }

    #[test]
    fn insert_component_via_command_buffer() {
        let mut world = World::new();
        let entity = world.spawn((42_i32,));

        let mut buf = CommandBuffer::new();
        buf.insert(entity, 2.72_f32);
        buf.apply(&mut world);

        let val = world
            .get::<&f32>(entity)
            .expect("f32 component should exist");
        assert!((*val - 2.72).abs() < f32::EPSILON);
    }

    #[test]
    fn remove_component_via_command_buffer() {
        let mut world = World::new();
        let entity = world.spawn((42_i32, 2.72_f32));

        let mut buf = CommandBuffer::new();
        buf.remove::<f32>(entity);
        buf.apply(&mut world);

        assert!(world.get::<&f32>(entity).is_err());
        assert!(world.get::<&i32>(entity).is_ok());
    }

    #[test]
    fn custom_command() {
        let mut world = World::new();
        let entity = world.spawn((0_i32,));

        let mut buf = CommandBuffer::new();
        buf.custom(move |w: &mut World| {
            if let Ok(mut val) = w.get::<&mut i32>(entity) {
                *val = 999;
            }
        });
        buf.apply(&mut world);

        let val = world.get::<&i32>(entity).expect("i32 should exist");
        assert_eq!(*val, 999);
    }

    #[test]
    fn commands_applied_in_order() {
        let mut world = World::new();
        let entity = world.spawn((0_i32,));

        let mut buf = CommandBuffer::new();
        // First set to 10
        buf.custom(move |w: &mut World| {
            if let Ok(mut val) = w.get::<&mut i32>(entity) {
                *val = 10;
            }
        });
        // Then multiply by 2
        buf.custom(move |w: &mut World| {
            if let Ok(mut val) = w.get::<&mut i32>(entity) {
                *val *= 2;
            }
        });
        buf.apply(&mut world);

        let val = world.get::<&i32>(entity).expect("i32 should exist");
        assert_eq!(*val, 20);
    }

    #[test]
    fn empty_buffer_apply_is_noop() {
        let mut world = World::new();
        let _entity = world.spawn((42_i32,));

        let mut buf = CommandBuffer::new();
        assert!(buf.is_empty());
        buf.apply(&mut world);

        assert_eq!(world.len(), 1);
    }

    #[test]
    fn clear_discards_commands() {
        let mut buf = CommandBuffer::new();
        buf.spawn(|w: &mut World| w.spawn((1_i32,)));
        buf.spawn(|w: &mut World| w.spawn((2_i32,)));
        assert_eq!(buf.len(), 2);

        buf.clear();
        assert!(buf.is_empty());

        let mut world = World::new();
        buf.apply(&mut world);
        assert_eq!(world.len(), 0);
    }
}
