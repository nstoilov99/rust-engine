//! Command pattern for undo/redo support
//!
//! Each undoable action is represented as a Command that knows how to
//! execute and undo itself.

use crate::engine::ecs::components::EntityGuid;
use crate::engine::ecs::hierarchy::despawn_recursive;
use crate::engine::ecs::{Name, Transform};
use crate::engine::scene::scene_format::EntityData;
use crate::engine::scene::scene_serializer::{serialize_subtree, spawn_subtree};
use hecs::{Entity, World};
use nalgebra_glm as glm;

/// Resolve an entity by its GUID string (entities are respawned across
/// undo/redo, so commands track GUIDs instead of `Entity` ids).
fn find_by_guid(world: &World, guid: &str) -> Option<Entity> {
    let guid = EntityGuid::from_string(guid)?;
    world
        .query::<&EntityGuid>()
        .iter()
        .find(|(_, g)| g.0 == guid.0)
        .map(|(e, _)| e)
}

/// "Delete Duck", "Cut 3 Entities", … for undo labels.
pub fn verb_object_label(verb: &str, world: &World, roots: &[Entity]) -> String {
    match roots {
        [single] => {
            let name = world
                .get::<&Name>(*single)
                .map(|n| n.0.clone())
                .unwrap_or_else(|_| "Entity".to_string());
            format!("{verb} {name}")
        }
        many => format!("{verb} {} Entities", many.len()),
    }
}

/// Trait for undoable commands
pub trait Command: Send + Sync {
    /// Execute the command
    fn execute(&mut self, world: &mut World);

    /// Undo the command
    fn undo(&mut self, world: &mut World);

    /// Get a description of the command for display
    fn description(&self) -> &str;
}

/// Command history for undo/redo
pub struct CommandHistory {
    undo_stack: Vec<Box<dyn Command>>,
    redo_stack: Vec<Box<dyn Command>>,
    max_history: usize,
    /// `true` when there are unsaved modifications since the last `mark_saved` call.
    dirty: bool,
}

impl CommandHistory {
    pub fn new(max_history: usize) -> Self {
        Self {
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            max_history,
            dirty: false,
        }
    }

    /// Change the history cap; trims the oldest entries if over it.
    pub fn set_max_history(&mut self, max: usize) {
        self.max_history = max.max(1);
        while self.undo_stack.len() > self.max_history {
            self.undo_stack.remove(0);
        }
    }

    /// Execute a command and add it to history
    pub fn execute(&mut self, mut command: Box<dyn Command>, world: &mut World) {
        command.execute(world);
        self.undo_stack.push(command);
        self.redo_stack.clear(); // Clear redo stack on new action

        // Limit history size
        if self.undo_stack.len() > self.max_history {
            self.undo_stack.remove(0);
        }
        self.dirty = true;
    }

    /// Record an already-executed command (for commands whose results the
    /// caller needs to inspect before handing them over, e.g. Paste).
    pub fn push_executed(&mut self, command: Box<dyn Command>) {
        self.undo_stack.push(command);
        self.redo_stack.clear();
        if self.undo_stack.len() > self.max_history {
            self.undo_stack.remove(0);
        }
        self.dirty = true;
    }

    /// Undo the last command
    pub fn undo(&mut self, world: &mut World) -> Option<String> {
        if let Some(mut command) = self.undo_stack.pop() {
            let desc = command.description().to_string();
            command.undo(world);
            self.redo_stack.push(command);
            self.dirty = true;
            Some(desc)
        } else {
            None
        }
    }

    /// Redo the last undone command
    pub fn redo(&mut self, world: &mut World) -> Option<String> {
        if let Some(mut command) = self.redo_stack.pop() {
            let desc = command.description().to_string();
            command.execute(world);
            self.undo_stack.push(command);
            self.dirty = true;
            Some(desc)
        } else {
            None
        }
    }

    /// Whether the scene has unsaved modifications.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Mark the history as saved (clears the dirty flag).
    pub fn mark_saved(&mut self) {
        self.dirty = false;
    }

    /// Set the dirty flag externally (for non-command edits like hierarchy drag-reorder).
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    pub fn undo_description(&self) -> Option<&str> {
        self.undo_stack.last().map(|c| c.description())
    }

    pub fn redo_description(&self) -> Option<&str> {
        self.redo_stack.last().map(|c| c.description())
    }

    pub fn clear(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.dirty = false;
    }
}

impl Default for CommandHistory {
    fn default() -> Self {
        Self::new(100) // Default to 100 undo levels
    }
}

/// Command for changing a Transform
pub struct TransformChangeCommand {
    entity: Entity,
    old_position: glm::Vec3,
    old_rotation: glm::Quat,
    old_scale: glm::Vec3,
    new_position: glm::Vec3,
    new_rotation: glm::Quat,
    new_scale: glm::Vec3,
}

impl TransformChangeCommand {
    pub fn new(entity: Entity, old: &Transform, new: &Transform) -> Self {
        Self {
            entity,
            old_position: old.position,
            old_rotation: old.rotation,
            old_scale: old.scale,
            new_position: new.position,
            new_rotation: new.rotation,
            new_scale: new.scale,
        }
    }
}

impl Command for TransformChangeCommand {
    fn execute(&mut self, world: &mut World) {
        if let Ok(mut transform) = world.get::<&mut Transform>(self.entity) {
            transform.position = self.new_position;
            transform.rotation = self.new_rotation;
            transform.scale = self.new_scale;
        }
        crate::engine::ecs::hierarchy::mark_transform_dirty(world, self.entity);
    }

    fn undo(&mut self, world: &mut World) {
        if let Ok(mut transform) = world.get::<&mut Transform>(self.entity) {
            transform.position = self.old_position;
            transform.rotation = self.old_rotation;
            transform.scale = self.old_scale;
        }
        crate::engine::ecs::hierarchy::mark_transform_dirty(world, self.entity);
    }

    fn description(&self) -> &str {
        "Transform Entity"
    }
}

/// Command for renaming an entity
pub struct RenameCommand {
    entity: Entity,
    old_name: String,
    new_name: String,
}

impl RenameCommand {
    pub fn new(entity: Entity, old_name: String, new_name: String) -> Self {
        Self {
            entity,
            old_name,
            new_name,
        }
    }
}

impl Command for RenameCommand {
    fn execute(&mut self, world: &mut World) {
        if let Ok(mut name) = world.get::<&mut Name>(self.entity) {
            name.0 = self.new_name.clone();
        }
    }

    fn undo(&mut self, world: &mut World) {
        if let Ok(mut name) = world.get::<&mut Name>(self.entity) {
            name.0 = self.old_name.clone();
        }
    }

    fn description(&self) -> &str {
        "Rename Entity"
    }
}

/// Command for adding a component to an entity
pub struct AddComponentCommand<T: Clone + Send + Sync + 'static> {
    entity: Entity,
    component: Option<T>,
    description_str: &'static str,
}

impl<T: Clone + Send + Sync + 'static> AddComponentCommand<T> {
    pub fn new(entity: Entity, component: T, description: &'static str) -> Self {
        Self {
            entity,
            component: Some(component),
            description_str: description,
        }
    }
}

impl<T: Clone + Send + Sync + 'static> Command for AddComponentCommand<T> {
    fn execute(&mut self, world: &mut World) {
        if let Some(component) = self.component.take() {
            let _ = world.insert_one(self.entity, component);
        }
    }

    fn undo(&mut self, world: &mut World) {
        if let Ok(component) = world.remove_one::<T>(self.entity) {
            self.component = Some(component);
        }
    }

    fn description(&self) -> &str {
        self.description_str
    }
}

/// Command for removing a component from an entity
pub struct RemoveComponentCommand<T: Clone + Send + Sync + 'static> {
    entity: Entity,
    component: Option<T>,
    description_str: &'static str,
}

impl<T: Clone + Send + Sync + 'static> RemoveComponentCommand<T> {
    pub fn new(entity: Entity, description: &'static str) -> Self {
        Self {
            entity,
            component: None,
            description_str: description,
        }
    }
}

impl<T: Clone + Send + Sync + 'static> Command for RemoveComponentCommand<T> {
    fn execute(&mut self, world: &mut World) {
        if let Ok(component) = world.remove_one::<T>(self.entity) {
            self.component = Some(component);
        }
    }

    fn undo(&mut self, world: &mut World) {
        if let Some(component) = self.component.take() {
            let _ = world.insert_one(self.entity, component);
        }
    }

    fn description(&self) -> &str {
        self.description_str
    }
}

/// Undoable delete of entity subtrees (also the delete half of Cut — the
/// clipboard write itself is not undone, matching common editors).
///
/// Subtrees are serialized at construction. Execute despawns them, resolving
/// entities by GUID so redo still works after an undo respawned them; undo
/// respawns with the original GUIDs and re-links surviving external parents.
pub struct DeleteSubtreeCommand {
    subtrees: Vec<Vec<EntityData>>,
    root_guids: Vec<String>,
    description: String,
}

impl DeleteSubtreeCommand {
    /// `roots` must be top-most (no root an ancestor of another).
    pub fn new(world: &World, roots: &[Entity], verb: &str) -> Self {
        Self {
            subtrees: roots.iter().map(|&r| serialize_subtree(world, r)).collect(),
            root_guids: roots
                .iter()
                .filter_map(|&r| world.get::<&EntityGuid>(r).ok().map(|g| g.0.to_string()))
                .collect(),
            description: verb_object_label(verb, world, roots),
        }
    }
}

impl Command for DeleteSubtreeCommand {
    fn execute(&mut self, world: &mut World) {
        for guid in &self.root_guids {
            if let Some(entity) = find_by_guid(world, guid) {
                despawn_recursive(world, entity);
            }
        }
    }

    fn undo(&mut self, world: &mut World) {
        for subtree in &self.subtrees {
            spawn_subtree(world, subtree, false);
        }
    }

    fn description(&self) -> &str {
        &self.description
    }
}

/// Undoable paste of the entity clipboard as siblings of the paste target.
///
/// The first execute spawns with fresh GUIDs and re-serializes the result so
/// redo restores the *same* pasted entities (GUIDs stay stable across
/// undo/redo). The target parent is tracked by GUID; if it no longer exists
/// the entities paste as scene roots.
pub struct PasteCommand {
    data: Vec<EntityData>,
    target_parent_guid: Option<String>,
    root_guids: Vec<String>,
    description: String,
}

impl PasteCommand {
    pub fn new(world: &World, clipboard: Vec<EntityData>, target_parent: Option<Entity>) -> Self {
        Self {
            data: clipboard,
            target_parent_guid: target_parent
                .and_then(|p| world.get::<&EntityGuid>(p).ok().map(|g| g.0.to_string())),
            root_guids: Vec::new(),
            description: "Paste".to_string(),
        }
    }

    /// The pasted subtree roots (valid after execute).
    pub fn roots(&self, world: &World) -> Vec<Entity> {
        self.root_guids
            .iter()
            .filter_map(|g| find_by_guid(world, g))
            .collect()
    }
}

impl Command for PasteCommand {
    fn execute(&mut self, world: &mut World) {
        if self.root_guids.is_empty() {
            // First execute: fresh GUIDs, then reparent + snapshot the result.
            let (_, roots) = spawn_subtree(world, &self.data, true);
            let target = self
                .target_parent_guid
                .as_deref()
                .and_then(|g| find_by_guid(world, g));
            if let Some(target) = target {
                for &root in &roots {
                    crate::engine::ecs::hierarchy::set_parent(world, root, target);
                }
            }
            self.root_guids = roots
                .iter()
                .filter_map(|&r| world.get::<&EntityGuid>(r).ok().map(|g| g.0.to_string()))
                .collect();
            self.data = roots
                .iter()
                .flat_map(|&r| serialize_subtree(world, r))
                .collect();
            self.description = verb_object_label("Paste", world, &roots);
        } else {
            // Redo: restore the exact entities the first paste created.
            spawn_subtree(world, &self.data, false);
        }
    }

    fn undo(&mut self, world: &mut World) {
        for guid in &self.root_guids {
            if let Some(entity) = find_by_guid(world, guid) {
                despawn_recursive(world, entity);
            }
        }
    }

    fn description(&self) -> &str {
        &self.description
    }
}
