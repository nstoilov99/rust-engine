//! Command pattern for undo/redo support
//!
//! Each undoable action is represented as a Command that knows how to
//! execute and undo itself.

use crate::engine::ecs::{Name, Transform};
use hecs::{Entity, World};
use nalgebra_glm as glm;

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
}

impl CommandHistory {
    pub fn new(max_history: usize) -> Self {
        Self {
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            max_history,
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
    }

    /// Undo the last command
    pub fn undo(&mut self, world: &mut World) -> Option<String> {
        if let Some(mut command) = self.undo_stack.pop() {
            let desc = command.description().to_string();
            command.undo(world);
            self.redo_stack.push(command);
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
            Some(desc)
        } else {
            None
        }
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
    }

    fn undo(&mut self, world: &mut World) {
        if let Ok(mut transform) = world.get::<&mut Transform>(self.entity) {
            transform.position = self.old_position;
            transform.rotation = self.old_rotation;
            transform.scale = self.old_scale;
        }
    }

    fn description(&self) -> &str {
        "Transform Change"
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
