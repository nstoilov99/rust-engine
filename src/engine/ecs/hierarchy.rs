//! Parent-child hierarchy components for scene graph
//!
//! These components enable transform inheritance and tree-structured scenes.

use hecs::{Entity, World};
use nalgebra_glm as glm;
use std::collections::{HashMap, HashSet};

use super::components::TransformDirty;
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

/// Check if `ancestor` is an ancestor of `descendant` in the hierarchy.
/// Used to prevent creating cycles when reparenting.
pub fn is_ancestor_of(world: &World, ancestor: Entity, descendant: Entity) -> bool {
    let mut current = descendant;
    while let Ok(parent) = world.get::<&Parent>(current) {
        if parent.0 == ancestor {
            return true;
        }
        current = parent.0;
    }
    false
}

/// Check if reparenting would create a cycle
/// Returns true if the operation is valid (no cycle would be created)
pub fn can_set_parent(world: &World, child: Entity, new_parent: Entity) -> bool {
    // Cannot parent to self
    if child == new_parent {
        return false;
    }
    // Cannot parent to a descendant (would create cycle)
    if is_ancestor_of(world, child, new_parent) {
        return false;
    }
    true
}

/// Set parent-child relationship
/// Returns false if operation would create a cycle (and does nothing in that case)
pub fn set_parent(world: &mut World, child: Entity, parent: Entity) -> bool {
    // Prevent cycles
    if !can_set_parent(world, child, parent) {
        return false;
    }
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

    true
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
    let entities_with_parents: std::collections::HashSet<Entity> =
        world.query::<&Parent>().iter().map(|(e, _)| e).collect();

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

/// Calculate world transform for an entity in Z-up space (recursive up the hierarchy).
///
/// Returns the composed world matrix in the game's native Z-up coordinate system.
/// This correctly handles non-uniform scaling combined with rotation in hierarchies.
///
/// For rendering, convert the result using `render_adapter::world_matrix_to_render()`:
/// ```ignore
/// let world_zup = get_world_transform(world, entity);
/// let render_matrix = render_adapter::world_matrix_to_render(&world_zup);
/// ```
pub fn get_world_transform(world: &World, entity: Entity) -> glm::Mat4 {
    let local_transform = world
        .get::<&Transform>(entity)
        .map(|t| t.local_matrix_zup())
        .unwrap_or_else(|_| glm::identity());

    // Check for parent
    if let Ok(parent) = world.get::<&Parent>(entity) {
        let parent_world = get_world_transform(world, parent.0);
        parent_world * local_transform
    } else {
        local_transform
    }
}

/// Authoritative, single-pass transform cache.
///
/// Call [`TransformCache::propagate`] once per frame (PostUpdate) to walk the
/// hierarchy top-down and compute every entity's world-space matrix.  All
/// downstream consumers (mesh submission, camera sync, editor gizmos) then
/// read from the cache with zero recursion.
///
/// Stores both Z-up world matrices and Y-up render matrices to avoid
/// recomputation each frame.
pub struct TransformCache {
    /// World transforms in Z-up space (for game logic, physics)
    world_cache: HashMap<Entity, glm::Mat4>,
    /// Render transforms in Y-up space (for GPU submission)
    render_cache: HashMap<Entity, glm::Mat4>,
    /// When true, the next propagation must be a full rebuild
    /// (set on spawn, despawn, reparent, scene load, play-mode transitions).
    needs_full: bool,
}

impl TransformCache {
    pub fn new() -> Self {
        Self {
            world_cache: HashMap::new(),
            render_cache: HashMap::new(),
            needs_full: true, // first propagation must be full
        }
    }

    /// Request a full hierarchy rebuild on the next propagation.
    ///
    /// Call this after structural changes: spawn, despawn, reparent,
    /// scene load, or play-mode enter/exit.
    pub fn request_full_propagation(&mut self) {
        self.needs_full = true;
    }

    /// Single-pass top-down propagation of every entity's world matrix.
    ///
    /// Automatically chooses between full and incremental propagation:
    /// - Full rebuild after structural changes (spawn, despawn, reparent, scene load).
    /// - Incremental re-propagation when only transform values have changed.
    /// - No-op when nothing is dirty and no full rebuild is required.
    pub fn propagate(&mut self, world: &mut World) {
        crate::profile_scope!("transform_propagation");

        // Detect structural changes (spawn/despawn/reparent) that happened
        // outside of GameWorld tracking by comparing entity counts.
        let world_entity_count = world.len() as usize;
        if !self.needs_full && world_entity_count != self.world_cache.len() {
            self.needs_full = true;
        }

        if self.needs_full {
            self.propagate_full_inner(world);
            self.needs_full = false;
            // Clear any dirty flags left over — full propagation already handled them.
            let dirty_entities: Vec<Entity> = world
                .query::<&TransformDirty>()
                .iter()
                .map(|(e, _)| e)
                .collect();
            for entity in dirty_entities {
                let _ = world.remove_one::<TransformDirty>(entity);
            }
            return;
        }

        self.propagate_incremental(world);
    }

    /// Full rebuild — clears caches and walks the entire hierarchy.
    ///
    /// Used on first frame and after structural changes.  Kept as a
    /// public method so callers that *know* they need a full rebuild
    /// (e.g. scene load) can invoke it directly.
    pub fn propagate_full(&mut self, world: &mut World) {
        crate::profile_scope!("transform_propagation_full");
        self.propagate_full_inner(world);
        self.needs_full = false;
        // Clear stale dirty flags.
        let dirty_entities: Vec<Entity> = world
            .query::<&TransformDirty>()
            .iter()
            .map(|(e, _)| e)
            .collect();
        for entity in dirty_entities {
            let _ = world.remove_one::<TransformDirty>(entity);
        }
    }

    fn propagate_full_inner(&mut self, world: &World) {
        self.world_cache.clear();
        self.render_cache.clear();

        // Seed with all root entities (no Parent component).
        let mut queue: Vec<(Entity, glm::Mat4)> = Vec::new();

        // Collect entities with parents for the root-detection pass.
        let entities_with_parents: HashSet<Entity> =
            world.query::<&Parent>().iter().map(|(e, _)| e).collect();

        for (entity, _) in world.query::<()>().iter() {
            if !entities_with_parents.contains(&entity) {
                let local = world
                    .get::<&Transform>(entity)
                    .map(|t| t.local_matrix_zup())
                    .unwrap_or_else(|_| glm::identity());
                self.world_cache.insert(entity, local);
                let render =
                    crate::engine::adapters::render_adapter::world_matrix_to_render(&local);
                self.render_cache.insert(entity, render);
                queue.push((entity, local));
            }
        }

        // BFS: propagate down through Children.
        let mut head = 0;
        while head < queue.len() {
            let (parent_entity, parent_world) = queue[head];
            head += 1;

            let children: Vec<Entity> = world
                .get::<&Children>(parent_entity)
                .map(|c| c.0.clone())
                .unwrap_or_default();

            for child in children {
                let local = world
                    .get::<&Transform>(child)
                    .map(|t| t.local_matrix_zup())
                    .unwrap_or_else(|_| glm::identity());
                let child_world = parent_world * local;
                self.world_cache.insert(child, child_world);
                let render =
                    crate::engine::adapters::render_adapter::world_matrix_to_render(&child_world);
                self.render_cache.insert(child, render);
                queue.push((child, child_world));
            }
        }
    }

    /// Incremental propagation — only re-computes subtrees rooted at dirty entities.
    ///
    /// 1. Collect all entities with `TransformDirty`.
    /// 2. Find "dirty roots" — dirty entities whose ancestors are all clean.
    /// 3. BFS-repropagate from each dirty root downward.
    /// 4. Clear all `TransformDirty` markers.
    fn propagate_incremental(&mut self, world: &mut World) {
        // Collect dirty entities.
        let dirty_entities: Vec<Entity> = world
            .query::<&TransformDirty>()
            .iter()
            .map(|(e, _)| e)
            .collect();

        if dirty_entities.is_empty() {
            return; // nothing changed — skip entirely
        }

        let dirty_set: HashSet<Entity> = dirty_entities.iter().copied().collect();

        // Find dirty roots: dirty entities with no dirty ancestor.
        let mut dirty_roots: Vec<Entity> = Vec::new();
        for &entity in &dirty_entities {
            let mut is_root = true;
            let mut current = entity;
            while let Ok(parent) = world.get::<&Parent>(current) {
                let parent_entity = parent.0;
                drop(parent);
                if dirty_set.contains(&parent_entity) {
                    is_root = false;
                    break;
                }
                current = parent_entity;
            }
            if is_root {
                dirty_roots.push(entity);
            }
        }

        // BFS from each dirty root, re-computing world matrices for the subtree.
        for &root in &dirty_roots {
            // Compute parent_world for this root from its (clean) parent's cached matrix.
            let parent_world = if let Ok(parent) = world.get::<&Parent>(root) {
                let parent_entity = parent.0;
                drop(parent);
                self.world_cache
                    .get(&parent_entity)
                    .copied()
                    .unwrap_or_else(glm::identity)
            } else {
                glm::identity()
            };

            let local = world
                .get::<&Transform>(root)
                .map(|t| t.local_matrix_zup())
                .unwrap_or_else(|_| glm::identity());
            let root_world = parent_world * local;
            self.world_cache.insert(root, root_world);
            let render =
                crate::engine::adapters::render_adapter::world_matrix_to_render(&root_world);
            self.render_cache.insert(root, render);

            // BFS children
            let mut queue: Vec<(Entity, glm::Mat4)> = vec![(root, root_world)];
            let mut head = 0;
            while head < queue.len() {
                let (parent_entity, pw) = queue[head];
                head += 1;

                let children: Vec<Entity> = world
                    .get::<&Children>(parent_entity)
                    .map(|c| c.0.clone())
                    .unwrap_or_default();

                for child in children {
                    let child_local = world
                        .get::<&Transform>(child)
                        .map(|t| t.local_matrix_zup())
                        .unwrap_or_else(|_| glm::identity());
                    let child_world = pw * child_local;
                    self.world_cache.insert(child, child_world);
                    let child_render =
                        crate::engine::adapters::render_adapter::world_matrix_to_render(
                            &child_world,
                        );
                    self.render_cache.insert(child, child_render);
                    queue.push((child, child_world));
                }
            }
        }

        // Clear dirty flags.
        for entity in dirty_entities {
            let _ = world.remove_one::<TransformDirty>(entity);
        }
    }

    /// Get cached world transform in Z-up space.
    ///
    /// Returns identity if the entity was not seen during [`propagate`].
    pub fn get_world(&self, entity: Entity) -> glm::Mat4 {
        self.world_cache
            .get(&entity)
            .copied()
            .unwrap_or_else(glm::identity)
    }

    /// Get cached render transform in Y-up space.
    ///
    /// Returns identity if the entity was not seen during [`propagate`].
    pub fn get_render(&self, entity: Entity) -> glm::Mat4 {
        self.render_cache
            .get(&entity)
            .copied()
            .unwrap_or_else(glm::identity)
    }
}

impl Default for TransformCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Resource flag set by `GameWorld` when structural hierarchy changes occur
/// (spawn, despawn, reparent). The `TransformPropagationSystem` reads and
/// clears this flag to trigger a full cache rebuild.
#[derive(Debug, Clone, Default)]
pub struct HierarchyChanged(pub bool);

/// Scheduled system that runs transform propagation from Resources.
///
/// Reads `HierarchyChanged` and clears it, then propagates via `TransformCache`.
pub struct TransformPropagationSystem;

impl super::schedule::System for TransformPropagationSystem {
    fn run(&mut self, world: &mut World, resources: &mut super::resources::Resources) {
        // Check if structural hierarchy changes occurred since last frame.
        let hierarchy_changed = resources
            .get_mut::<HierarchyChanged>()
            .map(|h| {
                let val = h.0;
                h.0 = false;
                val
            })
            .unwrap_or(false);

        if let Some(cache) = resources.get_mut::<TransformCache>() {
            if hierarchy_changed {
                cache.request_full_propagation();
            }
            cache.propagate(world);
        }
    }

    fn name(&self) -> &str {
        "TransformPropagationSystem"
    }
}

/// Convenience: mark a single entity as dirty so incremental propagation
/// picks it up next frame. Silently ignores dead entities.
pub fn mark_transform_dirty(world: &mut World, entity: Entity) {
    if world.contains(entity) && world.get::<&TransformDirty>(entity).is_err() {
        let _ = world.insert_one(entity, TransformDirty);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::ecs::components::Transform;
    use nalgebra_glm as glm;

    /// Spawn a root entity with a transform at the given position.
    fn spawn_root(world: &mut World, x: f32, y: f32, z: f32) -> Entity {
        let t = Transform {
            position: glm::vec3(x, y, z),
            ..Default::default()
        };
        world.spawn((t,))
    }

    /// Spawn a child entity parented to `parent` with a local offset.
    fn spawn_child(world: &mut World, parent: Entity, x: f32, y: f32, z: f32) -> Entity {
        let t = Transform {
            position: glm::vec3(x, y, z),
            ..Default::default()
        };
        let child = world.spawn((t,));
        set_parent(world, child, parent);
        child
    }

    // ── Full propagation ───────────────────────────────────────────

    #[test]
    fn full_propagation_computes_world_matrices() {
        let mut world = World::new();
        let root = spawn_root(&mut world, 10.0, 0.0, 0.0);
        let child = spawn_child(&mut world, root, 0.0, 5.0, 0.0);

        let mut cache = TransformCache::new();
        cache.propagate(&mut world);

        let root_world = cache.get_world(root);
        let child_world = cache.get_world(child);

        // Root should be translated by (10, 0, 0)
        let root_pos = glm::vec4_to_vec3(&(root_world * glm::vec4(0.0, 0.0, 0.0, 1.0)));
        assert!((root_pos.x - 10.0).abs() < 1e-5);
        assert!(root_pos.y.abs() < 1e-5);

        // Child should be at (10, 5, 0) in world space
        let child_pos = glm::vec4_to_vec3(&(child_world * glm::vec4(0.0, 0.0, 0.0, 1.0)));
        assert!((child_pos.x - 10.0).abs() < 1e-5);
        assert!((child_pos.y - 5.0).abs() < 1e-5);
    }

    // ── Dirty leaf updates subtree ─────────────────────────────────

    #[test]
    fn dirty_leaf_updates_subtree() {
        let mut world = World::new();
        let root = spawn_root(&mut world, 0.0, 0.0, 0.0);
        let child = spawn_child(&mut world, root, 0.0, 0.0, 0.0);
        let grandchild = spawn_child(&mut world, child, 0.0, 0.0, 5.0);

        let mut cache = TransformCache::new();
        // Initial full propagation.
        cache.propagate(&mut world);

        // Verify grandchild is at (0, 0, 5).
        let gc_world = cache.get_world(grandchild);
        let gc_pos = glm::vec4_to_vec3(&(gc_world * glm::vec4(0.0, 0.0, 0.0, 1.0)));
        assert!((gc_pos.z - 5.0).abs() < 1e-5);

        // Move the child entity — mark it dirty.
        {
            let mut t = world.get::<&mut Transform>(child).unwrap();
            t.position = glm::vec3(0.0, 10.0, 0.0);
        }
        mark_transform_dirty(&mut world, child);

        // Incremental propagation.
        cache.propagate(&mut world);

        // Grandchild should now be at (0, 10, 5).
        let gc_world = cache.get_world(grandchild);
        let gc_pos = glm::vec4_to_vec3(&(gc_world * glm::vec4(0.0, 0.0, 0.0, 1.0)));
        assert!((gc_pos.y - 10.0).abs() < 1e-5);
        assert!((gc_pos.z - 5.0).abs() < 1e-5);

        // Child should be at (0, 10, 0).
        let child_world = cache.get_world(child);
        let child_pos = glm::vec4_to_vec3(&(child_world * glm::vec4(0.0, 0.0, 0.0, 1.0)));
        assert!((child_pos.y - 10.0).abs() < 1e-5);
    }

    // ── Clean scene is a no-op ─────────────────────────────────────

    #[test]
    fn clean_scene_skips_propagation() {
        let mut world = World::new();
        let root = spawn_root(&mut world, 1.0, 2.0, 3.0);

        let mut cache = TransformCache::new();
        cache.propagate(&mut world);

        // Capture the cached matrix.
        let before = cache.get_world(root);

        // Propagate again without marking anything dirty — should be a no-op.
        cache.propagate(&mut world);

        let after = cache.get_world(root);
        assert_eq!(before, after);
    }

    // ── Full fallback after spawn ──────────────────────────────────

    #[test]
    fn full_fallback_after_spawn() {
        let mut world = World::new();
        let root = spawn_root(&mut world, 0.0, 0.0, 0.0);

        let mut cache = TransformCache::new();
        cache.propagate(&mut world);

        // Spawn a new entity — entity count changes → triggers full rebuild.
        let new_entity = spawn_root(&mut world, 42.0, 0.0, 0.0);

        cache.propagate(&mut world);

        // New entity should be in the cache.
        let new_world = cache.get_world(new_entity);
        let new_pos = glm::vec4_to_vec3(&(new_world * glm::vec4(0.0, 0.0, 0.0, 1.0)));
        assert!((new_pos.x - 42.0).abs() < 1e-5);

        // Old entity should still be correct.
        let root_world = cache.get_world(root);
        let root_pos = glm::vec4_to_vec3(&(root_world * glm::vec4(0.0, 0.0, 0.0, 1.0)));
        assert!(root_pos.x.abs() < 1e-5);
    }

    // ── Full fallback after despawn ────────────────────────────────

    #[test]
    fn full_fallback_after_despawn() {
        let mut world = World::new();
        let a = spawn_root(&mut world, 1.0, 0.0, 0.0);
        let b = spawn_root(&mut world, 2.0, 0.0, 0.0);

        let mut cache = TransformCache::new();
        cache.propagate(&mut world);

        // Despawn one entity — entity count changes → triggers full rebuild.
        let _ = world.despawn(b);

        cache.propagate(&mut world);

        // Remaining entity should still be correct.
        let a_world = cache.get_world(a);
        let a_pos = glm::vec4_to_vec3(&(a_world * glm::vec4(0.0, 0.0, 0.0, 1.0)));
        assert!((a_pos.x - 1.0).abs() < 1e-5);

        // Despawned entity should return identity.
        let b_world = cache.get_world(b);
        assert_eq!(b_world, glm::identity::<f32, 4>());
    }

    // ── request_full_propagation forces full rebuild ───────────────

    #[test]
    fn request_full_forces_rebuild() {
        let mut world = World::new();
        let root = spawn_root(&mut world, 0.0, 0.0, 0.0);
        let child = spawn_child(&mut world, root, 5.0, 0.0, 0.0);

        let mut cache = TransformCache::new();
        cache.propagate(&mut world);

        // Mutate transform without marking dirty.
        {
            let mut t = world.get::<&mut Transform>(child).unwrap();
            t.position = glm::vec3(99.0, 0.0, 0.0);
        }

        // Incremental propagation would miss this (no dirty flag).
        cache.propagate(&mut world);
        let child_world = cache.get_world(child);
        let child_pos = glm::vec4_to_vec3(&(child_world * glm::vec4(0.0, 0.0, 0.0, 1.0)));
        // Still at the old position because no dirty flag was set.
        assert!((child_pos.x - 5.0).abs() < 1e-5);

        // But request_full_propagation forces a full rebuild.
        cache.request_full_propagation();
        cache.propagate(&mut world);
        let child_world = cache.get_world(child);
        let child_pos = glm::vec4_to_vec3(&(child_world * glm::vec4(0.0, 0.0, 0.0, 1.0)));
        assert!((child_pos.x - 99.0).abs() < 1e-5);
    }

    // ── Dirty flags are cleared after propagation ──────────────────

    #[test]
    fn dirty_flags_cleared_after_propagation() {
        let mut world = World::new();
        let root = spawn_root(&mut world, 0.0, 0.0, 0.0);

        let mut cache = TransformCache::new();
        cache.propagate(&mut world);

        mark_transform_dirty(&mut world, root);
        assert!(world.get::<&TransformDirty>(root).is_ok());

        cache.propagate(&mut world);

        // Dirty flag should be removed.
        assert!(world.get::<&TransformDirty>(root).is_err());
    }
}
