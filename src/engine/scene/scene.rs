use std::collections::HashMap;
use crate::engine::scene::Transform2D;
use crate::engine::rendering::rendering_2d::SpriteBatch;
use crate::engine::scene::{SpriteSheet, AnimationController};

/// Entity ID (unique per entity)
pub type EntityId = usize;

/// Sprite component (references a texture)
#[derive(Clone, Copy, Debug)]
pub struct SpriteComponent {
    pub texture_id: usize,  // Which texture to use
    pub layer: i32,         // Drawing order (higher = drawn on top)
}

/// Entity with components
#[derive(Clone, Debug)]
pub struct Entity {
    pub id: EntityId,
    pub transform: Transform2D,
    pub sprite: Option<SpriteComponent>,
    pub animation: Option<AnimationController>,
    pub sprite_sheet: Option<SpriteSheet>,  
    pub active: bool,
}

/// Scene - manages all entities
pub struct Scene {
    entities: HashMap<EntityId, Entity>,
    next_id: EntityId,
}

impl Scene {
    pub fn new() -> Self {
        Self {
            entities: HashMap::new(),
            next_id: 0,
        }
    }

    /// Add entity to scene
    pub fn add_entity(
        &mut self,
        transform: Transform2D,
        sprite: Option<SpriteComponent>,
        animation: Option<AnimationController>,
        sprite_sheet: Option<SpriteSheet>,  
    ) -> EntityId {
        let id = self.next_id;
        self.next_id += 1;

        self.entities.insert(id, Entity {
            id,
            transform,
            sprite,
            animation,
            sprite_sheet, 
            active: true,
        });

        id
    }

    /// Remove entity from scene
    pub fn remove_entity(&mut self, id: EntityId) {
        self.entities.remove(&id);
    }

    /// Get entity (mutable)
    pub fn get_entity_mut(&mut self, id: EntityId) -> Option<&mut Entity> {
        self.entities.get_mut(&id)
    }

    /// Get entity (immutable)
    pub fn get_entity(&self, id: EntityId) -> Option<&Entity> {
        self.entities.get(&id)
    }

    /// Iterate all entities
    pub fn iter_entities(&self) -> impl Iterator<Item = &Entity> {
        self.entities.values().filter(|e| e.active)
    }

    /// Iterate all entities (mutable)
    pub fn iter_entities_mut(&mut self) -> impl Iterator<Item = &mut Entity> {
        self.entities.values_mut().filter(|e| e.active)
    }

    /// Submit all sprites to a batch for rendering (sorted by layer)
    pub fn submit_to_batch(&self, batch: &mut SpriteBatch, get_uv_rect: impl Fn(&Entity) -> [f32; 4]) {
        // Collect sprites with layers
        let mut sprites: Vec<_> = self.entities.values()
            .filter(|e| e.active && e.sprite.is_some())
            .collect();

        // Sort by layer (lower layers drawn first)
        sprites.sort_by_key(|e| e.sprite.as_ref().unwrap().layer);

        // Add to batch
        for entity in sprites {
            if let Some(sprite) = &entity.sprite {
                let uv_rect = get_uv_rect(entity);
                batch.add_sprite_animated(sprite.texture_id, entity.transform, uv_rect);
            }
        }
    }

    /// Update all entity animations
    pub fn update_animations(&mut self, delta_time: f32) {
        crate::profile_scope!("scene_update_animations");
        for entity in self.entities.values_mut().filter(|e| e.active) {
            if let Some(anim) = &mut entity.animation {
                anim.update(delta_time);
            }
        }
    }

    /// Get entity count
    pub fn entity_count(&self) -> usize {
        self.entities.values().filter(|e| e.active).count()
    }

    /// Clear all entities
    pub fn clear(&mut self) {
        self.entities.clear();
        self.next_id = 0;
    }
}

impl Default for Scene {
    fn default() -> Self {
        Self::new()
    }
}