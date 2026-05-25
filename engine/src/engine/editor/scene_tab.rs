//! Multi-scene tab support.
//!
//! The active scene's live state lives in the existing slots:
//! - `CoreApp.game_world` for the ECS world
//! - `SceneEditorState.{selection, command_history, hierarchy_panel, current_scene_*, active_dirty}`
//!
//! Inactive scenes are parked in [`DormantScene`] records inside [`SceneRegistry`].
//! Switching tabs is a swap dance: the active state is moved into a `DormantScene`
//! for the previously-active id, and the target id's `DormantScene` is consumed
//! back into the active slots.

use super::{CommandHistory, Selection};
use crate::engine::ecs::game_world::GameWorld;
use hecs::Entity;
use serde::{Deserialize, Serialize};

/// Stable identifier for an open scene tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SceneId(pub u32);

/// Modal state for "Save As" — collecting a filename for an untitled scene.
pub struct SaveAsDialog {
    /// Editable filename (relative to `content/scenes/`, without extension).
    pub filename: String,
    /// Set to true to commit the save next frame.
    pub commit: bool,
}

impl SaveAsDialog {
    pub fn new(initial_name: &str) -> Self {
        // Sanitise initial name: lowercase, replace spaces with underscores.
        let mut sanitised = initial_name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                    c.to_ascii_lowercase()
                } else {
                    '_'
                }
            })
            .collect::<String>();
        if sanitised.is_empty() {
            sanitised = "untitled".to_string();
        }
        Self {
            filename: sanitised,
            commit: false,
        }
    }
}

/// Parked state for a non-active scene tab.
pub struct DormantScene {
    pub id: SceneId,
    pub relative_path: String,
    pub display_name: String,
    pub dirty: bool,
    pub world: GameWorld,
    pub selection: Selection,
    pub command_history: CommandHistory,
    pub hierarchy_root_order: Vec<Entity>,
}

/// Registry of dormant scene tabs plus active id allocation.
pub struct SceneRegistry {
    pub active_id: SceneId,
    pub next_id: SceneId,
    pub dormant: Vec<DormantScene>,
}

impl SceneRegistry {
    pub fn new(initial_active_id: SceneId) -> Self {
        Self {
            active_id: initial_active_id,
            next_id: SceneId(initial_active_id.0 + 1),
            dormant: Vec::new(),
        }
    }

    /// Allocate a fresh id (does not register a scene).
    pub fn allocate_id(&mut self) -> SceneId {
        let id = self.next_id;
        self.next_id = SceneId(self.next_id.0 + 1);
        id
    }

    /// Find a dormant scene by its file path. Used to focus an already-open tab.
    pub fn find_dormant_by_path(&self, relative_path: &str) -> Option<SceneId> {
        if relative_path.is_empty() {
            return None;
        }
        self.dormant
            .iter()
            .find(|d| d.relative_path == relative_path)
            .map(|d| d.id)
    }

    /// Take a dormant scene out of the registry by id.
    pub fn take_dormant(&mut self, id: SceneId) -> Option<DormantScene> {
        let pos = self.dormant.iter().position(|d| d.id == id)?;
        Some(self.dormant.remove(pos))
    }

    /// Park a dormant scene back into the registry.
    pub fn park(&mut self, dormant: DormantScene) {
        self.dormant.push(dormant);
    }

    /// Drop a dormant scene without restoring it (used when closing a non-active tab).
    pub fn drop_dormant(&mut self, id: SceneId) -> bool {
        let pos = self.dormant.iter().position(|d| d.id == id);
        if let Some(p) = pos {
            self.dormant.remove(p);
            true
        } else {
            false
        }
    }
}
