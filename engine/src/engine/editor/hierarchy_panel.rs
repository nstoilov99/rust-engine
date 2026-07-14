//! Scene hierarchy panel state.
//!
//! The egui rendering fns (`show`, `show_contents`, `render_header`,
//! `render_search`, `render_tree`, `render_row`, `draw_tree_guides`,
//! `render_visibility_toggle`, `render_context_menu`, `handle_drag_drop`,
//! `draw_insertion_line`, `render_drag_ghost`, `handle_keyboard_shortcuts`)
//! were removed; the crusty analog lives in `hierarchy_crusty`.

use super::Selection;
use crate::engine::ecs::{
    hierarchy::{can_set_parent, despawn_recursive, get_root_entities, remove_parent, set_parent},
    Camera, Children, DirectionalLight, EditorVisibility, EntityGuid, MeshRenderer, Name, Parent,
    PointLight, Transform,
};
// `egui::Color32` is retained per the "keep egui types on live crusty signatures"
// rule — `entity_icon_stem` returns a tint that the crusty renderer consumes.
use egui::Color32;
use hecs::{Entity, World};
use smallvec::SmallVec;
use std::collections::HashSet;
use std::time::Instant;

/// Drop mode for drag-and-drop operations
/// Determined by mouse Y position within the row
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum DropMode {
    /// Insert above target (top 25% of row) - makes sibling before target
    InsertAbove,
    /// Make child of target (middle 50% of row) - makes child of target
    MakeChild,
    /// Insert below target (bottom 25% of row) - makes sibling after target
    InsertBelow,
}

/// One visible row in the flattened hierarchy list.
#[allow(dead_code)]
pub(crate) struct VisibleRow {
    pub(crate) entity: Entity,
    pub(crate) depth: usize,
    pub(crate) name: String,
    pub(crate) has_children: bool,
    pub(crate) is_expanded: bool,
    /// File stem of the SVG to render for this entity (e.g. `"camera"`,
    /// `"sun"`, `"object"`). Resolved against `HierarchyIcons` at draw time.
    /// Easily extended: pick the stem in `entity_icon_stem` and add the SVG.
    pub(crate) icon_stem: &'static str,
    /// Tint applied when the SVG is rendered. White by default — set it
    /// only when an entity kind needs a brand colour.
    pub(crate) icon_tint: Color32,
    /// Editor visibility flag, mirrored into the row so render code can
    /// dim the entry without re-querying. True == shown.
    pub(crate) is_visible: bool,
    /// Per-ancestor (and self) "is this the last sibling at its depth?" flags.
    /// Index `d` covers the chain element at depth `d`; index `depth` is the
    /// row itself. Used to render Godot-style L / T tree connectors:
    ///   - passing column `d` (0 ≤ d < depth-1) draws a full vertical iff
    ///     `!chain_is_last[d + 1]` (the descendant we belong to has more
    ///     siblings still to come).
    ///   - parent column `depth - 1` draws an L when `chain_is_last[depth]`
    ///     is true, a T otherwise.
    pub(crate) chain_is_last: SmallVec<[bool; 16]>,
    /// Full ancestor chain — entities from root down to `self` inclusive.
    /// Length == `depth + 1`. Used to compute which guide columns at this
    /// row are on the path to the selected entity (so the whole path can be
    /// highlighted, not only the selected row's own hook).
    pub(crate) entity_chain: SmallVec<[Entity; 16]>,
}

pub(crate) const ROW_HEIGHT: f32 = 22.0;
/// Horizontal spacing used for both indentation and tree-guide column centers.
pub(crate) const INDENT: f32 = 16.0;
/// Distance from the row's content left edge to column 0's center.
pub(crate) const COL_PAD: f32 = 8.0;
/// Pixel size icons are drawn at inside the row.
pub(crate) const ICON_SIZE: f32 = 16.0;
/// Width reserved on the right edge for the visibility eye column. Keeps the
/// icon away from the panel's right resize edge so clicking it never grabs
/// the dock separator.
pub(crate) const VISIBILITY_COL_WIDTH: f32 = 22.0;

/// Scene Hierarchy Panel state
pub struct HierarchyPanel {
    /// Search/filter text
    pub(crate) search_text: String,
    /// Entity being renamed (if any)
    pub(crate) renaming_entity: Option<Entity>,
    /// Text buffer for renaming
    pub(crate) rename_buffer: String,
    /// Drag source entity (only written by the crusty tree; kept for future
    /// use by shared drop-mode helpers).
    #[allow(dead_code)]
    pub(crate) drag_source: Option<Entity>,
    /// Show only matching entities when filtering
    pub(crate) filter_active: bool,
    /// Expanded state for entities (by entity id)
    pub(crate) expanded: HashSet<u64>,
    /// Explicit ordering of root entities
    root_order: Vec<Entity>,
    /// Entity being hovered during drag (for auto-expand)
    pub(crate) drag_hover_entity: Option<Entity>,
    /// When drag hover started (for auto-expand delay)
    pub(crate) drag_hover_start: Option<Instant>,
    /// Reusable buffer for flat visible rows (avoids per-frame allocation).
    pub(crate) flat_rows: Vec<VisibleRow>,
    /// One-shot flag: the rename editor should grab keyboard focus on its
    /// next frame. Set by `start_rename`, consumed by the crusty port
    /// (egui re-requests focus every frame instead).
    pub(crate) rename_needs_focus: bool,
}

impl Default for HierarchyPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl HierarchyPanel {
    pub fn new() -> Self {
        Self {
            search_text: String::new(),
            renaming_entity: None,
            rename_buffer: String::new(),
            drag_source: None,
            filter_active: false,
            expanded: HashSet::new(),
            root_order: Vec::new(),
            drag_hover_entity: None,
            drag_hover_start: None,
            flat_rows: Vec::new(),
            rename_needs_focus: false,
        }
    }

    /// Get the current root entity ordering (for scene serialization)
    pub fn root_order(&self) -> &[Entity] {
        &self.root_order
    }

    /// Set the root entity ordering (after scene loading)
    pub fn set_root_order(&mut self, order: Vec<Entity>) {
        self.root_order = order;
    }

    /// Sync `root_order` with the world's actual root entities.
    pub fn sync_root_order(&mut self, world: &World) {
        let current_roots: HashSet<Entity> = get_root_entities(world).into_iter().collect();
        self.root_order.retain(|e| current_roots.contains(e));
        for root in current_roots {
            if !self.root_order.contains(&root) {
                self.root_order.push(root);
            }
        }
    }

    pub(crate) fn move_root(&mut self, entity: Entity, new_index: usize) {
        if let Some(current) = self.root_order.iter().position(|&e| e == entity) {
            self.root_order.remove(current);
            let clamped = new_index.min(self.root_order.len());
            self.root_order.insert(clamped, entity);
        }
    }

    // ─── flat-row construction ─────────────────────────────────────────

    /// Build a flat visible-row list by walking the hierarchy top-down.
    /// Respects expand/collapse and filter state.
    pub(crate) fn build_visible_rows(&mut self, world: &World) {
        self.flat_rows.clear();

        let roots: Vec<Entity> = self.root_order.clone();
        let last_idx = roots.len().saturating_sub(1);
        let parent_is_last: SmallVec<[bool; 16]> = SmallVec::new();
        let parent_entities: SmallVec<[Entity; 16]> = SmallVec::new();
        for (i, root) in roots.into_iter().enumerate() {
            let is_last = i == last_idx;
            self.collect_rows(
                world,
                root,
                0,
                &parent_is_last,
                &parent_entities,
                is_last,
            );
        }
    }

    fn collect_rows(
        &mut self,
        world: &World,
        entity: Entity,
        depth: usize,
        parent_chain_is_last: &[bool],
        parent_entity_chain: &[Entity],
        is_last_sibling: bool,
    ) {
        let name = world
            .get::<&Name>(entity)
            .map(|n| n.0.clone())
            .unwrap_or_else(|_| format!("Entity {:?}", entity.id()));

        if self.filter_active && !self.matches_filter(&name, world, entity) {
            return;
        }

        let children: Vec<Entity> = world
            .get::<&Children>(entity)
            .map(|c| c.0.clone())
            .unwrap_or_default();

        let has_children = !children.is_empty();
        let entity_id = entity.id() as u64;
        let is_expanded = self.expanded.contains(&entity_id);
        let (icon_stem, icon_tint) = Self::entity_icon_stem(world, entity);
        let is_visible = world
            .get::<&EditorVisibility>(entity)
            .map(|v| v.visible)
            .unwrap_or(true);

        // Build this row's chain: parent chain + self's last-sibling flag.
        let mut chain_is_last: SmallVec<[bool; 16]> =
            SmallVec::with_capacity(parent_chain_is_last.len() + 1);
        chain_is_last.extend_from_slice(parent_chain_is_last);
        chain_is_last.push(is_last_sibling);

        // And the entity chain: parent entities + self.
        let mut entity_chain: SmallVec<[Entity; 16]> =
            SmallVec::with_capacity(parent_entity_chain.len() + 1);
        entity_chain.extend_from_slice(parent_entity_chain);
        entity_chain.push(entity);

        self.flat_rows.push(VisibleRow {
            entity,
            depth,
            name,
            has_children,
            is_expanded,
            icon_stem,
            icon_tint,
            is_visible,
            chain_is_last: chain_is_last.clone(),
            entity_chain: entity_chain.clone(),
        });

        if has_children && is_expanded {
            let last_idx = children.len().saturating_sub(1);
            for (i, child) in children.into_iter().enumerate() {
                let child_is_last = i == last_idx;
                self.collect_rows(
                    world,
                    child,
                    depth + 1,
                    &chain_is_last,
                    &entity_chain,
                    child_is_last,
                );
            }
        }
    }
    /// Pick which `engine/icons/hierarchy/<stem>.svg` to render for an entity.
    ///
    /// Add a new entity kind by:
    ///   1. Dropping `engine/icons/hierarchy/<your_name>.svg`,
    ///   2. Adding a branch here returning `("your_name", tint)`.
    /// No registry edit needed — the icon set is auto-discovered at startup.
    pub(crate) fn entity_icon_stem(world: &World, entity: Entity) -> (&'static str, Color32) {
        if world.get::<&Camera>(entity).is_ok() {
            return ("camera", Color32::WHITE);
        }
        if world.get::<&DirectionalLight>(entity).is_ok() || world.get::<&PointLight>(entity).is_ok() {
            return ("sun", Color32::WHITE);
        }
        // Default for everything else (groups, meshes, generics) — covers
        // entries with `Children`, `MeshRenderer`, or no special component.
        let _ = (world.get::<&MeshRenderer>(entity), world.get::<&Children>(entity));
        ("object", Color32::WHITE)
    }
    /// Set (or insert) the `EditorVisibility` component. Shared by the egui
    /// and crusty eye toggles.
    pub(crate) fn set_visibility(world: &mut World, entity: Entity, new_visible: bool) {
        // Scope the mutable component borrow so it's dropped before
        // `insert_one` (which needs a mutable borrow of the world itself).
        let updated = {
            if let Ok(mut existing) = world.get::<&mut EditorVisibility>(entity) {
                existing.visible = new_visible;
                true
            } else {
                false
            }
        };
        if !updated {
            let _ = world.insert_one(
                entity,
                EditorVisibility {
                    visible: new_visible,
                },
            );
        }
    }

    pub(crate) fn matches_filter(&self, name: &str, world: &World, entity: Entity) -> bool {
        let search_lower = self.search_text.to_lowercase();
        if name.to_lowercase().contains(&search_lower) {
            return true;
        }
        if let Ok(children) = world.get::<&Children>(entity) {
            for &child in children.0.iter() {
                let child_name = world
                    .get::<&Name>(child)
                    .map(|n| n.0.clone())
                    .unwrap_or_default();
                if self.matches_filter(&child_name, world, child) {
                    return true;
                }
            }
        }
        false
    }

    pub(crate) fn create_empty_entity(&self, world: &mut World) {
        let count = world.iter().count();
        world.spawn((
            Transform::default(),
            Name::new(format!("Entity {}", count)),
            EntityGuid::new(),
        ));
    }

    pub(crate) fn start_rename(&mut self, world: &World, entity: Entity) {
        self.renaming_entity = Some(entity);
        self.rename_needs_focus = true;
        self.rename_buffer = world
            .get::<&Name>(entity)
            .map(|n| n.0.clone())
            .unwrap_or_default();
    }

    pub(crate) fn commit_rename(&mut self, world: &mut World, entity: Entity) {
        if !self.rename_buffer.is_empty() {
            let has_name = world.get::<&Name>(entity).is_ok();
            if has_name {
                if let Ok(mut name) = world.get::<&mut Name>(entity) {
                    name.0 = self.rename_buffer.clone();
                }
            } else {
                let _ = world.insert_one(entity, Name::new(self.rename_buffer.clone()));
            }
        }
        self.renaming_entity = None;
    }

    pub(crate) fn duplicate_entity(&self, world: &mut World, entity: Entity) {
        let name = world
            .get::<&Name>(entity)
            .map(|n| format!("{} (Copy)", n.0))
            .unwrap_or_else(|_| "Entity (Copy)".to_string());
        let transform = world
            .get::<&Transform>(entity)
            .map(|t| *t)
            .unwrap_or_default();
        world.spawn((transform, Name::new(name), EntityGuid::new()));
    }

    pub(crate) fn delete_entity(
        &self,
        world: &mut World,
        selection: &mut Selection,
        entity: Entity,
    ) {
        selection.remove(entity);
        despawn_recursive(world, entity);
    }

    /// Classify a pointer y against a row spanning `top..bottom`. Takes raw
    /// coordinates (not an egui rect) so the crusty port can share it.
    pub(crate) fn calculate_drop_mode(&self, mouse_y: f32, top: f32, bottom: f32) -> DropMode {
        let row_height = bottom - top;
        let top_zone = top + row_height * 0.25;
        let bottom_zone = bottom - row_height * 0.25;

        if mouse_y < top_zone {
            DropMode::InsertAbove
        } else if mouse_y > bottom_zone {
            DropMode::InsertBelow
        } else {
            DropMode::MakeChild
        }
    }

    pub(crate) fn is_valid_drop(
        &self,
        world: &World,
        source: Entity,
        target: Entity,
        mode: DropMode,
    ) -> bool {
        if source == target {
            return false;
        }
        match mode {
            DropMode::MakeChild => can_set_parent(world, source, target),
            DropMode::InsertAbove | DropMode::InsertBelow => {
                if let Ok(parent) = world.get::<&Parent>(target) {
                    can_set_parent(world, source, parent.0)
                } else {
                    true
                }
            }
        }
    }

    pub(crate) fn perform_drop(
        &mut self,
        world: &mut World,
        source: Entity,
        target: Entity,
        drop_mode: DropMode,
    ) {
        match drop_mode {
            DropMode::MakeChild => {
                set_parent(world, source, target);
                self.expanded.insert(target.id() as u64);
            }
            DropMode::InsertAbove | DropMode::InsertBelow => {
                let drop_above = drop_mode == DropMode::InsertAbove;
                self.perform_sibling_drop(world, source, target, drop_above);
            }
        }
    }

    pub(crate) fn perform_sibling_drop(
        &mut self,
        world: &mut World,
        source: Entity,
        target: Entity,
        drop_above: bool,
    ) {
        let source_parent = world.get::<&Parent>(source).ok().map(|p| p.0);
        let target_parent = world.get::<&Parent>(target).ok().map(|p| p.0);

        if source_parent == target_parent {
            if let Some(parent) = source_parent {
                if let Ok(mut children) = world.get::<&mut Children>(parent) {
                    if let Some(target_idx) = children.index_of(target) {
                        let source_idx = children.index_of(source);
                        let mut insert_idx = if drop_above {
                            target_idx
                        } else {
                            target_idx + 1
                        };
                        if let Some(src_idx) = source_idx {
                            if src_idx < target_idx {
                                insert_idx = insert_idx.saturating_sub(1);
                            }
                        }
                        children.move_to_index(source, insert_idx);
                    }
                }
            } else if let Some(target_idx) = self.root_order.iter().position(|&e| e == target) {
                let source_idx = self.root_order.iter().position(|&e| e == source);
                let mut insert_idx = if drop_above {
                    target_idx
                } else {
                    target_idx + 1
                };
                if let Some(src_idx) = source_idx {
                    if src_idx < target_idx {
                        insert_idx = insert_idx.saturating_sub(1);
                    }
                }
                self.move_root(source, insert_idx);
            }
        } else if let Some(parent) = target_parent {
            set_parent(world, source, parent);
            if let Ok(mut children) = world.get::<&mut Children>(parent) {
                if let Some(target_idx) = children.index_of(target) {
                    let insert_idx = if drop_above {
                        target_idx
                    } else {
                        target_idx + 1
                    };
                    children.move_to_index(source, insert_idx);
                }
            }
        } else {
            remove_parent(world, source);
            if !self.root_order.contains(&source) {
                self.root_order.push(source);
            }
            if let Some(target_idx) = self.root_order.iter().position(|&e| e == target) {
                let insert_idx = if drop_above {
                    target_idx
                } else {
                    target_idx + 1
                };
                self.move_root(source, insert_idx);
            }
        }
    }
}
