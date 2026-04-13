//! Game command types for cross-boundary intent communication.
//!
//! `GameCommand` is for cross-boundary intent only — local systems should
//! mutate components directly. Commands are useful when:
//! - A system in one crate needs to trigger behavior owned by another crate
//! - Deferred execution is needed (e.g., spawn/despawn during iteration)
//! - Network replication requires a serializable action log

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A serializable command representing a gameplay intent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GameCommand {
    /// Apply damage to an entity.
    ApplyDamage {
        target: Uuid,
        amount: f32,
    },
    /// Spawn an entity from a prefab.
    SpawnPrefab {
        prefab_name: String,
        position: [f32; 3],
    },
    /// Despawn an entity by GUID.
    DespawnEntity {
        target: Uuid,
    },
    /// Play a sound effect.
    PlaySound {
        sound_name: String,
        position: Option<[f32; 3]>,
    },
}

/// Buffer for queueing game commands during a frame.
///
/// Systems push commands; the `GameCommandExecutor` system drains and
/// processes them each frame.
#[derive(Default)]
pub struct GameCommandBuffer {
    commands: Vec<GameCommand>,
}

impl GameCommandBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Queue a command for execution.
    pub fn push(&mut self, cmd: GameCommand) {
        self.commands.push(cmd);
    }

    /// Drain all queued commands.
    pub fn drain(&mut self) -> std::vec::Drain<'_, GameCommand> {
        self.commands.drain(..)
    }

    /// Check if there are pending commands.
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_buffer_push_and_drain() {
        let mut buf = GameCommandBuffer::new();
        assert!(buf.is_empty());
        buf.push(GameCommand::PlaySound {
            sound_name: "boom".into(),
            position: None,
        });
        assert!(!buf.is_empty());
        let cmds: Vec<_> = buf.drain().collect();
        assert_eq!(cmds.len(), 1);
        assert!(buf.is_empty());
    }
}
