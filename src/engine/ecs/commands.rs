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
