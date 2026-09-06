//! Game client plugin — registers gameplay systems with the engine.
//!
//! Runs through `PluginSet` like every other plugin, but is flagged
//! `internal`: no engine lists the game's own module as a toggleable plugin,
//! so it never appears in the Plugin Manager and the manifest cannot switch it
//! off. (Its `depends_on: ["physics_rapier"]` arrives with the Rapier plugin.)

use game_shared::commands::GameCommandBuffer;
use rust_engine::engine::ecs::access::SystemDescriptor;
use rust_engine::engine::ecs::schedule::{RunIfPlaying, Stage};
use rust_engine::engine::plugins::{
    EnginePlugin, PluginContext, PluginError, PluginKind, PluginManifest, PluginOrigin, PluginSet,
};

use crate::systems::{CharacterMovementSystem, GameCommandExecutor, PlayerInputSystem};

/// Client-side game plugin that registers player input, movement, and command systems.
pub struct ClientGamePlugin;

/// The project module's plugin id. Permanent, and referenced by the editor's
/// "gameplay disabled" hint, so it lives here rather than as a loose literal.
pub const GAME_CLIENT_ID: &str = "game_client";

impl EnginePlugin for ClientGamePlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new(GAME_CLIENT_ID, "Game Client")
            .with_module_path("game_client/src/plugin.rs")
            .with_description("Player input, character movement and game command execution.")
            .with_origin(PluginOrigin::Project)
            .with_kind(PluginKind::Runtime)
            // The honest truth of this codebase (D7 cascade):
            // CharacterMovementSystem declares `.before(PhysicsStepSystem)`,
            // writes `PhysicsWorld` and consumes Rapier handles. Turning
            // physics off therefore turns gameplay off with it — "physics off"
            // is a scene-editing configuration, not a playable one.
            .depends_on([rust_engine::engine::plugins::PHYSICS_RAPIER_ID])
            .internal()
    }

    fn build(&self, ctx: &mut PluginContext) -> Result<(), PluginError> {
        ctx.insert_resource(GameCommandBuffer::new());

        // Task 41.6 D11: PreUpdate, ahead of the anim stack and the physics
        // step, so this frame's velocity reaches the step and the anim
        // bridge sees this frame's state.
        ctx.add_system_with_criteria(
            PlayerInputSystem,
            Stage::PreUpdate,
            PlayerInputSystem::descriptor(),
            RunIfPlaying,
        );
        ctx.add_system_with_criteria(
            CharacterMovementSystem,
            Stage::PreUpdate,
            CharacterMovementSystem::descriptor(),
            RunIfPlaying,
        );
        ctx.add_system_with_criteria(
            GameCommandExecutor,
            Stage::PostUpdate,
            SystemDescriptor::new("GameCommandExecutor").writes_resource::<GameCommandBuffer>(),
            RunIfPlaying,
        );
        // The Task 41 tracer demo writer (ticket 01) is retired: the real
        // parameter bridge lives in `anim_bridge` (net characters, ADR 0002).

        Ok(())
    }
}

/// The plugin set both binaries run. Plugin *inclusion* is Rust code plus
/// Cargo features; *activation* is the `project.ron` manifest.
///
/// Insertion order here is the "manifest order" tie-break for dependency
/// sorting, so engine plugins go in before the project's own module.
pub fn client_plugin_set() -> PluginSet {
    let mut set = PluginSet::new();
    set.add(rust_engine::engine::plugins::RapierPhysicsPlugin);
    #[cfg(feature = "graph-scripting")]
    set.add(rust_engine::engine::plugins::GraphScriptingPlugin::new(
        rust_engine::engine::assets::content_root::content_root(),
    ));
    #[cfg(feature = "dev_nodes")]
    set.add(rust_engine::engine::plugins::DevNodesPlugin);
    set.add(ClientGamePlugin);
    set
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_engine::engine::ecs::resources::{EditorState, Resources};
    use rust_engine::engine::ecs::schedule::{Schedule, Stage, System};
    use rust_engine::engine::ecs::system_names;
    use rust_engine::engine::plugins::PluginTargets;

    struct Stub(&'static str);
    impl System for Stub {
        fn run(&mut self, _w: &mut hecs::World, _r: &mut Resources) {}
        fn name(&self) -> &str {
            self.0
        }
    }

    /// The gameplay systems share PreUpdate with the anim stack and the
    /// physics step. Only a launch builds the runtime schedule, so this
    /// mirrors both hosts' PreUpdate registrations (same descriptors as
    /// `app.rs` / `standalone.rs`) and runs the validator over the real
    /// plugin set: every overlapping access must be declared and ordered.
    #[test]
    fn gameplay_systems_validate_against_the_host_preupdate_stage() {
        use rust_engine::engine::animation::graph::{AnimGraphRunner, AnimGraphRuntime, IkTargets};
        use rust_engine::engine::animation::{AnimationPlayer, SkeletonInstance};
        use rust_engine::engine::ecs::components::Transform;
        use rust_engine::engine::ecs::resources::Time;
        use rust_engine::engine::ecs::hierarchy::TransformCache;
        use rust_engine::engine::physics::{PhysicsWorld, RigidBody};

        let mut schedule = Schedule::new();
        schedule.add_system_described(
            Stub(system_names::ANIMATION_UPDATE),
            Stage::PreUpdate,
            SystemDescriptor::new(system_names::ANIMATION_UPDATE)
                .reads_resource::<Time>()
                .writes::<AnimationPlayer>()
                .writes::<SkeletonInstance>(),
        );
        schedule.add_system_described(
            Stub(system_names::FOOT_PLACEMENT),
            Stage::PreUpdate,
            SystemDescriptor::new(system_names::FOOT_PLACEMENT)
                .reads_resource::<Time>()
                .reads_resource::<PhysicsWorld>()
                .reads_resource::<TransformCache>()
                .reads::<Transform>()
                .reads::<RigidBody>()
                .writes::<AnimGraphRuntime>()
                .writes::<IkTargets>()
                .after(system_names::ANIMATION_UPDATE)
                .before(system_names::ANIM_GRAPH),
        );
        schedule.add_system_described(
            Stub(system_names::ANIM_GRAPH),
            Stage::PreUpdate,
            SystemDescriptor::new(system_names::ANIM_GRAPH)
                .reads_resource::<Time>()
                .reads_resource::<TransformCache>()
                .reads::<AnimGraphRunner>()
                .reads::<Transform>()
                .writes::<AnimGraphRuntime>()
                .writes::<SkeletonInstance>()
                .after(system_names::ANIMATION_UPDATE),
        );

        let mut resources = Resources::new();
        resources.insert(PhysicsWorld::new());
        resources.insert(EditorState::new());
        let mut registry = rust_engine::engine::node_graph::NodeRegistry::new();

        let mut set = client_plugin_set();
        set.build_all(
            PluginTargets {
                schedule: &mut schedule,
                resources: &mut resources,
                node_registry: &mut registry,
            },
            None,
        );
        assert!(set.failures().is_empty(), "{:?}", set.failures());

        let errors = schedule.validate();
        assert!(errors.is_empty(), "schedule validation: {errors:?}");
    }
}
