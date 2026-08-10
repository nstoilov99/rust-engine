//! The staging context handed to [`EnginePlugin::build`].
//!
//! Nothing here touches live engine state. Systems land in a scratch
//! [`Schedule`], resources in a scratch [`Resources`], node types in a
//! [`StagedRegistry`], and lifecycle callbacks in a plain `Vec`. `PluginSet`
//! preflights and commits the whole stage at once, or throws all of it away —
//! so a plugin that errors half-way through `build()` cannot leave a partial
//! registration behind.
//!
//! [`EnginePlugin::build`]: super::EnginePlugin::build

use std::any::TypeId;
use std::collections::HashMap;

use crate::engine::ecs::access::SystemDescriptor;
use crate::engine::ecs::resources::Resources;
use crate::engine::ecs::schedule::{RunCriteria, Schedule, Stage, System};
use crate::engine::node_graph::migrate::MigrationCtx;
use crate::engine::node_graph::registry::{NodeDescriptor, StagedRegistry};

/// A callback run after any world population (editor initial scene, scene
/// open, standalone `load_world`, benchmark load, play-mode reset).
///
/// `FnMut` because a plugin's content hook legitimately keeps state between
/// loads (Rapier's handle bookkeeping, for one).
pub type WorldLoadedFn = Box<dyn FnMut(&mut hecs::World, &mut Resources) + Send + Sync>;

/// What a plugin actually registered — collected during staging so the Plugin
/// Manager gets its per-plugin counts with zero bookkeeping from authors.
///
/// `node_types` is the *committed* count: descriptors skipped for id collision
/// (see [`crate::engine::node_graph::NodeRegistry::merge_staged`]) are not
/// counted, they show up as warnings instead.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PluginCounts {
    pub systems: usize,
    pub resources: usize,
    pub node_types: usize,
    pub domain_pins: usize,
    pub migrations: usize,
    pub world_loaded_callbacks: usize,
}

/// Per-plugin scratch space. See the module docs.
#[derive(Default)]
pub struct PluginContext {
    pub(super) schedule: Schedule,
    pub(super) resources: Resources,
    /// TypeId -> type name for every staged resource. Doubles as the staged
    /// resource key set (`Resources` does not expose its keys) and gives
    /// collision errors something readable to print.
    pub(super) resource_names: HashMap<TypeId, &'static str>,
    pub(super) registry: StagedRegistry,
    pub(super) world_loaded: Vec<WorldLoadedFn>,
}

impl PluginContext {
    pub fn new() -> Self {
        Self::default()
    }

    // === ECS ===

    /// Register a system with an access descriptor.
    pub fn add_system<S: System + 'static>(
        &mut self,
        system: S,
        stage: Stage,
        descriptor: SystemDescriptor,
    ) -> &mut Self {
        self.schedule.add_system_described(system, stage, descriptor);
        self
    }

    /// Register a system with an access descriptor and a run criteria.
    pub fn add_system_with_criteria<S, C>(
        &mut self,
        system: S,
        stage: Stage,
        descriptor: SystemDescriptor,
        criteria: C,
    ) -> &mut Self
    where
        S: System + 'static,
        C: RunCriteria + 'static,
    {
        self.schedule
            .add_system_described_with_criteria(system, stage, descriptor, criteria);
        self
    }

    /// Insert a resource. Colliding with a core or earlier-plugin resource is
    /// an error at commit time — a plugin never silently replaces one.
    pub fn insert_resource<T: Send + Sync + 'static>(&mut self, resource: T) -> &mut Self {
        self.resource_names
            .insert(TypeId::of::<T>(), std::any::type_name::<T>());
        self.resources.insert(resource);
        self
    }

    // === Node graph ===

    /// Register a node type. Descriptor invariants are checked at commit; an
    /// id already taken is skipped with a warning rather than failing the
    /// plugin (a duplicate node type must not cost you the whole plugin).
    pub fn register_node(&mut self, descriptor: NodeDescriptor) -> &mut Self {
        self.registry.nodes.push(descriptor);
        self
    }

    /// Register a consumer domain pin type, hue derived from the slug hash.
    pub fn register_domain_pin(&mut self, slug: &str) -> &mut Self {
        self.registry.domain_pins.push((slug.to_string(), None));
        self
    }

    /// Same, pinned to an explicit ramp index (0..12).
    pub fn register_domain_pin_keyed(&mut self, slug: &str, ramp_index: u8) -> &mut Self {
        self.registry
            .domain_pins
            .push((slug.to_string(), Some(ramp_index)));
        self
    }

    /// Register the migration step for `type_id` from `from_version` to
    /// `from_version + 1`.
    pub fn register_migration(
        &mut self,
        type_id: &str,
        from_version: u32,
        step: impl Fn(&mut MigrationCtx) + Send + Sync + 'static,
    ) -> &mut Self {
        self.registry
            .migrations
            .push(((type_id.to_string(), from_version), Box::new(step)));
        self
    }

    // === Lifecycle ===

    /// Run `callback` after every world population. Registration alone is not
    /// enough for plugins whose work happens at content moments, not startup.
    pub fn on_world_loaded(
        &mut self,
        callback: impl FnMut(&mut hecs::World, &mut Resources) + Send + Sync + 'static,
    ) -> &mut Self {
        self.world_loaded.push(Box::new(callback));
        self
    }

    // === Introspection ===

    /// Counts as staged. `PluginSet` adjusts `node_types` down for skipped
    /// descriptors before recording them.
    pub(super) fn staged_counts(&self) -> PluginCounts {
        PluginCounts {
            systems: self.schedule.system_count(),
            resources: self.resource_names.len(),
            node_types: self.registry.nodes.len(),
            domain_pins: self.registry.domain_pins.len(),
            migrations: self.registry.migrations.len(),
            world_loaded_callbacks: self.world_loaded.len(),
        }
    }

    /// Does this stage touch the node registry at all? Used to decide whether
    /// a missing registry is fatal for this plugin.
    pub(super) fn touches_node_registry(&self) -> bool {
        !self.registry.nodes.is_empty()
            || !self.registry.domain_pins.is_empty()
            || !self.registry.migrations.is_empty()
    }
}
