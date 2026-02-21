//! Frame-based component change detection.
//!
//! Tracks when components are added or modified using frame numbers.
//! This is opt-in: only components explicitly marked via `mark_added`
//! or `mark_changed` are tracked. Does NOT wrap hecs storage.

use hecs::Entity;
use std::any::TypeId;
use std::collections::HashMap;

/// Tracks when components were added or changed, by frame number.
///
/// Keyed by `(Entity, TypeId)` with frame stamps. Supports querying
/// whether a component was added or changed within a given number of frames.
pub struct ChangeTicks {
    /// Frame at which a component was first added to an entity.
    added: HashMap<(Entity, TypeId), u64>,
    /// Frame at which a component was last changed on an entity.
    changed: HashMap<(Entity, TypeId), u64>,
    /// Current frame counter (incremented each frame).
    current_frame: u64,
}

impl ChangeTicks {
    pub fn new() -> Self {
        Self {
            added: HashMap::new(),
            changed: HashMap::new(),
            current_frame: 0,
        }
    }

    /// Advance to next frame. Call at start of each frame.
    pub fn new_frame(&mut self) {
        self.current_frame += 1;
    }

    /// Current frame number.
    pub fn current_frame(&self) -> u64 {
        self.current_frame
    }

    /// Mark a component as just added on an entity.
    /// Also marks it as changed (added implies changed).
    pub fn mark_added<T: 'static>(&mut self, entity: Entity) {
        let key = (entity, TypeId::of::<T>());
        self.added.insert(key, self.current_frame);
        self.changed.insert(key, self.current_frame);
    }

    /// Mark a component as changed on an entity (without marking added).
    pub fn mark_changed<T: 'static>(&mut self, entity: Entity) {
        let key = (entity, TypeId::of::<T>());
        self.changed.insert(key, self.current_frame);
    }

    /// Check if a component was added within `max_age` frames.
    pub fn is_added<T: 'static>(&self, entity: Entity, max_age: u64) -> bool {
        let key = (entity, TypeId::of::<T>());
        self.added
            .get(&key)
            .map_or(false, |&frame| self.current_frame.saturating_sub(frame) <= max_age)
    }

    /// Check if a component was changed within `max_age` frames.
    pub fn is_changed<T: 'static>(&self, entity: Entity, max_age: u64) -> bool {
        let key = (entity, TypeId::of::<T>());
        self.changed
            .get(&key)
            .map_or(false, |&frame| self.current_frame.saturating_sub(frame) <= max_age)
    }

    /// Remove all tracking data for an entity (call on despawn).
    pub fn remove_entity(&mut self, entity: Entity) {
        self.added.retain(|&(e, _), _| e != entity);
        self.changed.retain(|&(e, _), _| e != entity);
    }

    /// Prune entries older than `max_age` frames to prevent unbounded growth.
    pub fn prune(&mut self, max_age: u64) {
        let cutoff = self.current_frame.saturating_sub(max_age);
        self.added.retain(|_, &mut frame| frame >= cutoff);
        self.changed.retain(|_, &mut frame| frame >= cutoff);
    }

    /// Total number of tracked entries (for diagnostics).
    pub fn tracked_count(&self) -> usize {
        self.added.len() + self.changed.len()
    }
}

impl Default for ChangeTicks {
    fn default() -> Self {
        Self::new()
    }
}
