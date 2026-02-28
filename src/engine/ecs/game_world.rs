//! Central game world that aggregates all ECS state.
//!
//! ```text
//! GameWorld
//! +-- hecs::World          (entity/component storage)
//! +-- Resources            (global typed state)
//! +-- EventStorage         (double-buffered events)
//! +-- CommandBuffer        (deferred structural mutations)
//! +-- ChangeTicks          (component change tracking)
//! ```
//!
//! Systems receive decomposed borrows via `Schedule::run_raw()`.
//! External code (editor, physics, rendering) accesses the hecs world
//! through `hecs()` / `hecs_mut()` for backward compatibility.

use super::change_detection::ChangeTicks;
use super::commands::CommandBuffer;
use super::events::{EntityDeleted, EntitySpawned, Event, EventStorage};
use super::resources::{EditorState, Resources, Time};
use super::schedule::Schedule;
use hecs::Entity;

/// Central game world containing all ECS data.
pub struct GameWorld {
    hecs_world: hecs::World,
    resources: Resources,
    events: EventStorage,
    command_buffer: CommandBuffer,
    change_ticks: ChangeTicks,
}

impl GameWorld {
    pub fn new() -> Self {
        let mut resources = Resources::new();
        resources.insert(Time::new());
        resources.insert(EditorState::new());

        Self {
            hecs_world: hecs::World::new(),
            resources,
            events: EventStorage::new(),
            command_buffer: CommandBuffer::new(),
            change_ticks: ChangeTicks::new(),
        }
    }

    // === Frame lifecycle ===

    /// Call at the start of each frame.
    /// Advances change ticks and swaps event buffers.
    pub fn begin_frame(&mut self) {
        self.change_ticks.new_frame();
        self.events.update_all();
    }

    // === Direct hecs access (backward compatibility) ===

    /// Immutable access to the underlying hecs::World.
    pub fn hecs(&self) -> &hecs::World {
        &self.hecs_world
    }

    /// Mutable access to the underlying hecs::World.
    pub fn hecs_mut(&mut self) -> &mut hecs::World {
        &mut self.hecs_world
    }

    // === Resource access ===

    pub fn resources(&self) -> &Resources {
        &self.resources
    }

    pub fn resources_mut(&mut self) -> &mut Resources {
        &mut self.resources
    }

    /// Convenience: get an immutable resource reference.
    pub fn resource<T: Send + Sync + 'static>(&self) -> Option<&T> {
        self.resources.get::<T>()
    }

    /// Convenience: get a mutable resource reference.
    pub fn resource_mut<T: Send + Sync + 'static>(&mut self) -> Option<&mut T> {
        self.resources.get_mut::<T>()
    }

    // === Event access ===

    /// Send an event (will be readable next frame after update).
    pub fn send_event<T: Event>(&mut self, event: T) {
        self.events.send(event);
    }

    /// Get event storage for reading.
    pub fn events(&self) -> &EventStorage {
        &self.events
    }

    /// Get mutable event storage.
    pub fn events_mut(&mut self) -> &mut EventStorage {
        &mut self.events
    }

    // === Command buffer access ===

    /// Get the command buffer for queueing deferred operations.
    pub fn commands(&mut self) -> &mut CommandBuffer {
        &mut self.command_buffer
    }

    /// Apply all pending commands to the hecs world.
    pub fn apply_commands(&mut self) {
        self.command_buffer.apply(&mut self.hecs_world);
    }

    // === Change ticks access ===

    pub fn change_ticks(&self) -> &ChangeTicks {
        &self.change_ticks
    }

    pub fn change_ticks_mut(&mut self) -> &mut ChangeTicks {
        &mut self.change_ticks
    }

    /// Check if a component was added this frame.
    pub fn is_added<T: 'static>(&self, entity: Entity) -> bool {
        self.change_ticks.is_added::<T>(entity, 0)
    }

    /// Check if a component was changed this frame.
    pub fn is_changed<T: 'static>(&self, entity: Entity) -> bool {
        self.change_ticks.is_changed::<T>(entity, 0)
    }

    // === Entity operations (convenience, with tracking) ===

    /// Spawn an entity with tracking. Sends EntitySpawned event.
    /// Auto-assigns EntityGuid if not already present.
    /// Prefer using CommandBuffer during system execution.
    pub fn spawn(&mut self, bundle: impl hecs::DynamicBundle) -> Entity {
        let entity = self.hecs_world.spawn(bundle);
        // Auto-assign GUID if the bundle didn't include one
        if self.hecs_world.get::<&super::components::EntityGuid>(entity).is_err() {
            let _ = self.hecs_world.insert_one(entity, super::components::EntityGuid::new());
        }
        self.events.send(EntitySpawned {
            entity,
            name: None,
        });
        entity
    }

    /// Spawn an entity with tracking and a name. Sends EntitySpawned event.
    /// Auto-assigns EntityGuid if not already present.
    pub fn spawn_named(
        &mut self,
        bundle: impl hecs::DynamicBundle,
        name: impl Into<String>,
    ) -> Entity {
        let entity = self.hecs_world.spawn(bundle);
        // Auto-assign GUID if the bundle didn't include one
        if self.hecs_world.get::<&super::components::EntityGuid>(entity).is_err() {
            let _ = self.hecs_world.insert_one(entity, super::components::EntityGuid::new());
        }
        self.events.send(EntitySpawned {
            entity,
            name: Some(name.into()),
        });
        entity
    }

    /// Despawn an entity (immediately, with hierarchy cleanup).
    /// Sends EntityDeleted event, cleans up change ticks.
    pub fn despawn(&mut self, entity: Entity) {
        self.change_ticks.remove_entity(entity);
        self.events.send(EntityDeleted { entity });
        super::hierarchy::despawn_recursive(&mut self.hecs_world, entity);
    }

    /// Insert a component on an entity, marking it as Added in ChangeTicks.
    pub fn insert<T: hecs::Component>(
        &mut self,
        entity: Entity,
        component: T,
    ) -> Result<(), hecs::NoSuchEntity> {
        let result = self.hecs_world.insert_one(entity, component);
        if result.is_ok() {
            self.change_ticks.mark_added::<T>(entity);
        }
        result
    }

    /// Remove a component from an entity.
    pub fn remove<T: hecs::Component>(&mut self, entity: Entity) -> Result<T, hecs::ComponentError> {
        self.hecs_world.remove_one::<T>(entity)
    }

    // === Component operations ===

    /// Get a component immutably.
    pub fn get<T: Send + Sync + 'static>(
        &self,
        entity: Entity,
    ) -> Result<hecs::Ref<'_, T>, hecs::ComponentError> {
        self.hecs_world.get::<&T>(entity)
    }

    /// Get a component mutably (does NOT auto-track change).
    pub fn get_component_mut<T: Send + Sync + 'static>(
        &mut self,
        entity: Entity,
    ) -> Result<hecs::RefMut<'_, T>, hecs::ComponentError> {
        self.hecs_world.get::<&mut T>(entity)
    }

    /// Get a component mutably and mark it as changed in ChangeTicks.
    pub fn get_mut_tracked<T: Send + Sync + 'static>(
        &mut self,
        entity: Entity,
    ) -> Result<hecs::RefMut<'_, T>, hecs::ComponentError> {
        self.change_ticks.mark_changed::<T>(entity);
        self.hecs_world.get::<&mut T>(entity)
    }

    // === Query operations (delegate to hecs) ===

    /// Query entities immutably (delegates to hecs).
    pub fn query<Q: hecs::Query>(&self) -> hecs::QueryBorrow<'_, Q> {
        self.hecs_world.query::<Q>()
    }

    /// Query entities mutably (delegates to hecs).
    pub fn query_mut<Q: hecs::Query>(&mut self) -> hecs::QueryMut<'_, Q> {
        self.hecs_world.query_mut::<Q>()
    }

    // === Schedule integration ===

    /// Run a schedule against this GameWorld.
    /// Decomposes self into parts for systems, applies commands between stages.
    pub fn run_schedule(&mut self, schedule: &mut Schedule) {
        schedule.run_raw(
            &mut self.hecs_world,
            &mut self.resources,
            &mut self.command_buffer,
        );
    }

    // === Transient state reset ===

    /// Reset all transient ECS state (commands, events, change ticks).
    /// Used during play mode transitions to prevent stale state from leaking.
    /// If `flush_commands` is true, pending commands are applied before clearing;
    /// if false, they are discarded.
    pub fn reset_transients(&mut self, flush_commands: bool) {
        if flush_commands {
            self.apply_commands();
        }
        self.command_buffer.clear();
        self.events.clear_all();
        self.change_ticks = ChangeTicks::new();
    }

    // === Pruning ===

    /// Prune old change tick data. Call periodically (e.g., every 1000 frames).
    pub fn prune_change_ticks(&mut self, max_age: u64) {
        self.change_ticks.prune(max_age);
    }
}

impl Default for GameWorld {
    fn default() -> Self {
        Self::new()
    }
}
