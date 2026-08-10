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
            // The honest truth of this codebase (D7 cascade): PlayerInputSystem
            // declares `.after(PhysicsStepSystem)` and CharacterMovementSystem
            // writes `PhysicsWorld` and consumes Rapier handles. Turning
            // physics off therefore turns gameplay off with it — "physics off"
            // is a scene-editing configuration, not a playable one.
            .depends_on([rust_engine::engine::plugins::PHYSICS_RAPIER_ID])
            .internal()
    }

    fn build(&self, ctx: &mut PluginContext) -> Result<(), PluginError> {
        ctx.insert_resource(GameCommandBuffer::new());

        ctx.add_system_with_criteria(
            PlayerInputSystem,
            Stage::Update,
            PlayerInputSystem::descriptor(),
            RunIfPlaying,
        );
        ctx.add_system_with_criteria(
            CharacterMovementSystem,
            Stage::Update,
            CharacterMovementSystem::descriptor(),
            RunIfPlaying,
        );
        ctx.add_system_with_criteria(
            GameCommandExecutor,
            Stage::PostUpdate,
            SystemDescriptor::new("GameCommandExecutor").writes_resource::<GameCommandBuffer>(),
            RunIfPlaying,
        );

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
    #[cfg(feature = "dev_nodes")]
    set.add(rust_engine::engine::plugins::DevNodesPlugin);
    set.add(ClientGamePlugin);
    set
}
