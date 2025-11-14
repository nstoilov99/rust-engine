// Sprite batching - groups sprites by texture for efficient rendering

use std::sync::Arc;
use std::collections::HashMap;
use vulkano::descriptor_set::PersistentDescriptorSet;
use crate::engine::components::Transform2D;

/// Sprite batch - groups sprites by texture
pub struct SpriteBatch {
    // Map from texture ID to list of transforms
    batches: HashMap<usize, Vec<Transform2D>>,
}

impl SpriteBatch {
    pub fn new() -> Self {
        Self {
            batches: HashMap::new(),
        }
    }

    /// Add sprite to batch
    pub fn add(&mut self, texture_id: usize, transform: Transform2D) {
        self.batches
            .entry(texture_id)
            .or_insert_with(Vec::new)
            .push(transform);
    }

    /// Clear all sprites
    pub fn clear(&mut self) {
        self.batches.clear();
    }

    /// Get batches for rendering
    pub fn get_batches(&self) -> &HashMap<usize, Vec<Transform2D>> {
        &self.batches
    }

    /// Get sprite count
    pub fn sprite_count(&self) -> usize {
        self.batches.values().map(|v| v.len()).sum()
    }

    /// Get batch count
    pub fn batch_count(&self) -> usize {
        self.batches.len()
    }
}