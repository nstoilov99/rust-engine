use glam::Vec2;
use std::sync::Arc;
use vulkano::descriptor_set::PersistentDescriptorSet;
use std::collections::HashMap;
use crate::Transform2D;

/// Sprite instance with UV coordinates (for animations)
#[derive(Clone, Copy, Debug)]
pub struct AnimatedSprite {
    pub transform: Transform2D,
    pub uv_rect: [f32; 4],  // [u_min, v_min, u_max, v_max]
}

pub struct SpriteBatch {
    batches: HashMap<usize, Vec<Transform2D>>,
    animated_batches: HashMap<usize, Vec<AnimatedSprite>>,
    descriptor_sets: HashMap<usize, Arc<PersistentDescriptorSet>>,
    next_id: usize,
}

impl SpriteBatch {
    pub fn add_sprite_animated(&mut self, texture_id: usize, transform: Transform2D, uv_rect: [f32; 4]) {
    self.animated_batches
        .entry(texture_id)
        .or_insert_with(Vec::new)
        .push(AnimatedSprite { transform, uv_rect });
    }

    /// Clear all batches (call each frame)
    pub fn clear(&mut self) {
        self.batches.clear();
        self.animated_batches.clear();  // Also clear animated batches
    }

    /// Iterator for animated batches
    pub fn iter_animated_batches(&self) -> impl Iterator<Item = (Arc<PersistentDescriptorSet>, &[AnimatedSprite])> + '_ {
        self.animated_batches.iter().filter_map(move |(id, sprites)| {
            self.descriptor_sets.get(id).map(|desc_set| (desc_set.clone(), sprites.as_slice()))
        })
    }
}

/// Defines a sprite sheet with frame layout
#[derive(Debug, Clone)]
pub struct SpriteSheet {
    pub texture_size: Vec2,        // Full texture size (e.g., 512x512)
    pub frame_size: Vec2,           // Size of one frame (e.g., 64x64)
    pub frames_per_row: u32,        // Frames in each row
    pub total_frames: u32,          // Total number of frames
}

impl SpriteSheet {
    /// Creates a new sprite sheet definition
    pub fn new(
        texture_width: f32,
        texture_height: f32,
        frame_width: f32,
        frame_height: f32,
    ) -> Self {
        let frames_per_row = (texture_width / frame_width) as u32;
        let frames_per_col = (texture_height / frame_height) as u32;

        Self {
            texture_size: Vec2::new(texture_width, texture_height),
            frame_size: Vec2::new(frame_width, frame_height),
            frames_per_row,
            total_frames: frames_per_row * frames_per_col,
        }
    }

    /// Calculate UV coordinates for a specific frame
    pub fn get_frame_uvs(&self, frame_index: u32) -> [Vec2; 4] {
        let frame_index = frame_index.min(self.total_frames - 1);

        let col = frame_index % self.frames_per_row;
        let row = frame_index / self.frames_per_row;

        // Calculate UV coordinates (0.0 to 1.0)
        let u_start = (col as f32 * self.frame_size.x) / self.texture_size.x;
        let v_start = (row as f32 * self.frame_size.y) / self.texture_size.y;
        let u_end = u_start + (self.frame_size.x / self.texture_size.x);
        let v_end = v_start + (self.frame_size.y / self.texture_size.y);

        // Return UV coords for quad vertices [TL, TR, BL, BR]
        [
            Vec2::new(u_start, v_start), // Top-left
            Vec2::new(u_end, v_start),   // Top-right
            Vec2::new(u_start, v_end),   // Bottom-left
            Vec2::new(u_end, v_end),     // Bottom-right
        ]
    }

    /// Get frame index from row and column
    pub fn frame_at(&self, row: u32, col: u32) -> u32 {
        (row * self.frames_per_row + col).min(self.total_frames - 1)
    }
}