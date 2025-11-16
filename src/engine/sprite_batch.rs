// Sprite batching - groups sprites by texture for efficient rendering

use std::sync::Arc;
use std::collections::HashMap;
use vulkano::descriptor_set::PersistentDescriptorSet;
use crate::engine::components::Transform2D;

/// Sprite batch - groups sprites by texture
pub struct SpriteBatch {
    // Map from texture ID to list of transforms
    batches: HashMap<usize, Vec<Transform2D>>,
    descriptor_sets: HashMap<usize, Arc<PersistentDescriptorSet>>,
    next_id: usize,
}

impl Default for SpriteBatch {
    fn default() -> Self {
        Self::new()
    }
}

impl SpriteBatch {
    pub fn new() -> Self {
        Self {
            batches: HashMap::new(),
            descriptor_sets: HashMap::new(),
            next_id: 0,   
        }
    }

    /// Add sprite to batch
    pub fn add_sprite(&mut self, texture_id: usize, transform: Transform2D) {
        self.batches
            .entry(texture_id)
            .or_insert_with(Vec::new)
            .push(transform);
    }

    /// Clear all sprites
    pub fn clear(&mut self) {
        self.batches.clear();
    }

    /// Get batches for rendering (returns descriptor set + transforms)
    pub fn iter_batches(&self) -> impl Iterator<Item = (Arc<PersistentDescriptorSet>, &[Transform2D])> + '_ {
        self.batches.iter().filter_map(move |(id, transforms)| {
            self.descriptor_sets.get(id).map(|desc_set| (desc_set.clone(), transforms.as_slice()))
        })
    }

    /// Get sprite count
    pub fn sprite_count(&self) -> usize {
        self.batches.values().map(|v| v.len()).sum()
    }

    /// Get batch count
    pub fn batch_count(&self) -> usize {
        self.batches.len()
    }
    
    /// Register a texture and get its ID
    pub fn register_texture(&mut self, descriptor_set: Arc<PersistentDescriptorSet>) -> usize {
    let id = self.next_id;
    self.descriptor_sets.insert(id, descriptor_set);
    self.next_id += 1;
    id
}
}