//! Parent-child hierarchy components for scene graph
//!
//! These components enable transform inheritance and tree-structured scenes.

use hecs::{Entity, World};
use nalgebra_glm as glm;
use std::collections::HashMap;

use super::Transform;

/// Parent component - references the parent entity
#[derive(Debug, Clone, Copy)]
pub struct Parent(pub Entity);

impl Parent {
    pub fn new(entity: Entity) -> Self {
        Self(entity)
    }

    pub fn entity(&self) -> Entity {
        self.0
    }
}

/// Children component - list of child entities
#[derive(Debug, Clone, Default)]
pub struct Children(pub Vec<Entity>);

impl Children {
    pub fn new() -> Self {
        Self(Vec::new())
    }

    pub fn with_children(children: Vec<Entity>) -> Self {
        Self(children)
    }

    pub fn add(&mut self, child: Entity) {
        if !self.0.contains(&child) {
            self.0.push(child);
        }
    }

    pub fn remove(&mut self, child: Entity) {
        self.0.retain(|&e| e != child);
    }

    pub fn iter(&self) -> impl Iterator<Item = &Entity> {
        self.0.iter()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Insert a child at a specific index
    pub fn insert_at(&mut self, child: Entity, index: usize) {
        if !self.0.contains(&child) {
            let clamped = index.min(self.0.len());
            self.0.insert(clamped, child);
        }
    }

    /// Move existing child to new index
    pub fn move_to_index(&mut self, child: Entity, new_index: usize) -> bool {
        if let Some(current) = self.0.iter().position(|&e| e == child) {
            self.0.remove(current);
            let clamped = new_index.min(self.0.len());
            self.0.insert(clamped, child);
            true
        } else {
            false
        }
    }

    /// Get index of a child
    pub fn index_of(&self, child: Entity) -> Option<usize> {
        self.0.iter().position(|&e| e == child)
    }
}

/// Marker component for root entities (no parent)
/// Optional - entities without Parent component are implicitly roots
#[derive(Debug, Clone, Copy, Default)]
pub struct Root;

/// Set parent-child relationship
pub fn set_parent(world: &mut World, child: Entity, parent: Entity) {
    // Remove from old parent if exists
    if let Ok(old_parent) = world.get::<&Parent>(child) {
        let old_parent_entity = old_parent.0;
        drop(old_parent); // Release borrow before mutable borrow
        if let Ok(mut old_children) = world.get::<&mut Children>(old_parent_entity) {
            old_children.remove(child);
        }
    }

    // Add Parent component to child
    if world.get::<&Parent>(child).is_ok() {
        // Update existing Parent
        if let Ok(mut p) = world.get::<&mut Parent>(child) {
            p.0 = parent;
        }
    } else {
        // Insert new Parent
        let _ = world.insert_one(child, Parent(parent));
    }

    // Add child to parent's Children list
    // Check if Children component exists first
    let has_children = world.get::<&Children>(parent).is_ok();
    if has_children {
        if let Ok(mut children) = world.get::<&mut Children>(parent) {
            children.add(child);
        }
    } else {
        // Create Children component if it doesn't exist
        let mut children = Children::new();
        children.add(child);
        let _ = world.insert_one(parent, children);
    }
}

/// Remove parent (make entity a root)
pub fn remove_parent(world: &mut World, entity: Entity) {
    if let Ok(parent) = world.get::<&Parent>(entity) {
        let parent_entity = parent.0;
        drop(parent); // Release borrow before mutable borrow
        // Remove from parent's children list
        if let Ok(mut children) = world.get::<&mut Children>(parent_entity) {
            children.remove(entity);
        }
    }
    // Remove Parent component
    let _ = world.remove_one::<Parent>(entity);
}

/// Get all root entities (entities without Parent component)
pub fn get_root_entities(world: &World) -> Vec<Entity> {
    let mut roots = Vec::new();
    let entities_with_parents: std::collections::HashSet<Entity> = world
        .query::<&Parent>()
        .iter()
        .map(|(e, _)| e)
        .collect();

    // All entities that don't have a Parent component are roots
    for (entity, _) in world.query::<()>().iter() {
        if !entities_with_parents.contains(&entity) {
            roots.push(entity);
        }
    }
    roots
}

/// Recursively despawn entity and all children
pub fn despawn_recursive(world: &mut World, entity: Entity) {
    // Collect children first to avoid borrow issues
    let children: Vec<Entity> = world
        .get::<&Children>(entity)
        .map(|c| c.0.clone())
        .unwrap_or_default();

    // Recursively despawn children
    for child in children {
        despawn_recursive(world, child);
    }

    // Remove from parent's children list
    remove_parent(world, entity);

    // Despawn the entity
    let _ = world.despawn(entity);
}

/// Calculate world transform for an entity (recursive up the hierarchy)
pub fn get_world_transform(world: &World, entity: Entity) -> glm::Mat4 {
    let local_transform = world
        .get::<&Transform>(entity)
        .map(|t| {
            // Build local transform matrix (translation * rotation * scale)
            let translation = glm::translation(&t.position);
            let rotation = glm::quat_to_mat4(&t.rotation);
            let scale = glm::scaling(&t.scale);
            translation * rotation * scale
        })
        .unwrap_or_else(|_| glm::identity());

    // Check for parent
    if let Ok(parent) = world.get::<&Parent>(entity) {
        let parent_world = get_world_transform(world, parent.0);
        parent_world * local_transform
    } else {
        local_transform
    }
}

/// Cache for world transforms (optional optimization)
pub struct TransformCache {
    cache: HashMap<Entity, glm::Mat4>,
    dirty: bool,
}

impl TransformCache {
    pub fn new() -> Self {
        Self {
            cache: HashMap::new(),
            dirty: true,
        }
    }

    /// Mark cache as dirty (call when transforms change)
    pub fn invalidate(&mut self) {
        self.dirty = true;
    }

    /// Get cached world transform, or calculate and cache
    pub fn get_world_transform(&mut self, world: &World, entity: Entity) -> glm::Mat4 {
        if self.dirty {
            self.cache.clear();
            self.dirty = false;
        }

        if let Some(&cached) = self.cache.get(&entity) {
            return cached;
        }

        let transform = get_world_transform(world, entity);
        self.cache.insert(entity, transform);
        transform
    }
}

impl Default for TransformCache {
    fn default() -> Self {
        Self::new()
    }
}
