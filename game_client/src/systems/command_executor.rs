//! Drains and processes GameCommandBuffer each frame.

use game_shared::commands::{GameCommand, GameCommandBuffer};
use rust_engine::engine::ecs::resources::Resources;
use rust_engine::engine::ecs::schedule::System;

pub struct GameCommandExecutor;

impl System for GameCommandExecutor {
    fn run(&mut self, _world: &mut hecs::World, resources: &mut Resources) {
        let Some(buffer) = resources.get_mut::<GameCommandBuffer>() else {
            return;
        };
        for cmd in buffer.drain() {
            match cmd {
                GameCommand::ApplyDamage { target, amount } => {
                    log::warn!("GameCommand::ApplyDamage not yet implemented (target={target}, amount={amount})");
                }
                GameCommand::SpawnPrefab { prefab_name, position } => {
                    log::warn!("GameCommand::SpawnPrefab not yet implemented (prefab={prefab_name}, pos={position:?})");
                }
                GameCommand::DespawnEntity { target } => {
                    log::warn!("GameCommand::DespawnEntity not yet implemented (target={target})");
                }
                GameCommand::PlaySound { sound_name, position } => {
                    log::warn!("GameCommand::PlaySound not yet implemented (sound={sound_name}, pos={position:?})");
                }
            }
        }
    }

    fn name(&self) -> &str {
        "GameCommandExecutor"
    }
}
