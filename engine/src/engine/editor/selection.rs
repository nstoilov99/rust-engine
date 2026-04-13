//! Entity selection state for editor

use hecs::Entity;
use std::collections::HashSet;

/// Selection state for editor
#[derive(Debug, Default)]
pub struct Selection {
    /// Currently selected entities
    selected: HashSet<Entity>,
    /// Primary selected entity (for single-select operations)
    primary: Option<Entity>,
}

impl Selection {
    pub fn new() -> Self {
        Self::default()
    }

    /// Select a single entity (clears previous selection)
    pub fn select(&mut self, entity: Entity) {
        self.selected.clear();
        self.selected.insert(entity);
        self.primary = Some(entity);
    }

    /// Add entity to selection (multi-select)
    pub fn add(&mut self, entity: Entity) {
        self.selected.insert(entity);
        if self.primary.is_none() {
            self.primary = Some(entity);
        }
    }

    /// Remove entity from selection
    pub fn remove(&mut self, entity: Entity) {
        self.selected.remove(&entity);
        if self.primary == Some(entity) {
            self.primary = self.selected.iter().next().copied();
        }
    }

    /// Toggle entity selection
    pub fn toggle(&mut self, entity: Entity) {
        if self.selected.contains(&entity) {
            self.remove(entity);
        } else {
            self.add(entity);
        }
    }

    /// Clear all selection
    pub fn clear(&mut self) {
        self.selected.clear();
        self.primary = None;
    }

    /// Check if entity is selected
    pub fn is_selected(&self, entity: Entity) -> bool {
        self.selected.contains(&entity)
    }

    /// Get primary selected entity
    pub fn primary(&self) -> Option<Entity> {
        self.primary
    }

    /// Get all selected entities
    pub fn all(&self) -> impl Iterator<Item = &Entity> {
        self.selected.iter()
    }

    /// Get selected entity count
    pub fn count(&self) -> usize {
        self.selected.len()
    }

    /// Check if selection is empty
    pub fn is_empty(&self) -> bool {
        self.selected.is_empty()
    }
}
