//! Game client plugin — registers gameplay systems with the engine.

use game_shared::commands::GameCommandBuffer;
use rust_engine::engine::ecs::access::SystemDescriptor;
use rust_engine::engine::ecs::resources::Resources;
use rust_engine::engine::ecs::schedule::{RunIfPlaying, Schedule, Stage};
use rust_engine::engine::plugin::GamePlugin;

use crate::systems::{CharacterMovementSystem, GameCommandExecutor, PlayerInputSystem};

/// Client-side game plugin that registers player input, movement, and command systems.
pub struct ClientGamePlugin;

impl GamePlugin for ClientGamePlugin {
    fn build(&self, schedule: &mut Schedule, resources: &mut Resources) {
        resources.insert(GameCommandBuffer::new());

        schedule.add_system_described_with_criteria(
            PlayerInputSystem,
            Stage::Update,
            PlayerInputSystem::descriptor(),
            RunIfPlaying,
        );
        schedule.add_system_described_with_criteria(
            CharacterMovementSystem,
            Stage::Update,
            CharacterMovementSystem::descriptor(),
            RunIfPlaying,
        );
        schedule.add_system_described_with_criteria(
            GameCommandExecutor,
            Stage::PostUpdate,
            SystemDescriptor::new("GameCommandExecutor")
                .writes_resource::<GameCommandBuffer>(),
            RunIfPlaying,
        );
    }

    fn name(&self) -> &str {
        "ClientGamePlugin"
    }
}
