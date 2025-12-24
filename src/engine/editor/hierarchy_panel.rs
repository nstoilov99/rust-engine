//! Scene Hierarchy Panel - tree view of all entities

use super::Selection;
use crate::engine::ecs::{
    hierarchy::{despawn_recursive, get_root_entities, remove_parent, set_parent},
    Camera, Children, DirectionalLight, MeshRenderer, Name, Parent, PointLight, Transform,
};
use egui::{Context, RichText, ScrollArea, SidePanel, TextEdit, Ui};
use hecs::{Entity, World};
use std::collections::HashSet;

/// Scene Hierarchy Panel state
pub struct HierarchyPanel {
    /// Search/filter text
    search_text: String,
    /// Entity being renamed (if any)
    renaming_entity: Option<Entity>,
    /// Text buffer for renaming
    rename_buffer: String,
    /// Drag source entity
    drag_source: Option<Entity>,
    /// Show only matching entities when filtering
    filter_active: bool,
    /// Expanded state for entities (by entity id)
    expanded: HashSet<u64>,
    /// Explicit ordering of root entities
    root_order: Vec<Entity>,
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

    /// Render the hierarchy panel
    pub fn show(&mut self, ctx: &Context, world: &mut World, selection: &mut Selection) {
        SidePanel::left("hierarchy_panel")
            .resizable(true)
            .default_width(250.0)
            .min_width(150.0)
            .show(ctx, |ui| {
                self.render_header(ui, world);
                ui.separator();
                self.render_search(ui);
                ui.separator();
                self.render_tree(ui, world, selection);
            });
    }

    /// Render panel header with title and add button
    fn render_header(&mut self, ui: &mut Ui, world: &mut World) {
        ui.horizontal(|ui| {
            ui.heading("Hierarchy");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("+").on_hover_text("Add Entity").clicked() {
                    self.create_empty_entity(world);
                }
            });
        });
    }

    /// Render search box
    fn render_search(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label("Search:");
            let response = ui.add(
                TextEdit::singleline(&mut self.search_text)
                    .hint_text("Filter entities...")
                    .desired_width(ui.available_width()),
            );
            self.filter_active = !self.search_text.is_empty();

            // Clear search on Escape
            if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.search_text.clear();
                self.filter_active = false;
            }
        });
    }

    /// Sync root_order with world state (call at start of render)
    fn sync_root_order(&mut self, world: &World) {
        let current_roots: HashSet<Entity> = get_root_entities(world).into_iter().collect();

        // Remove entities that are no longer roots
        self.root_order.retain(|e| current_roots.contains(e));

        // Add new roots that aren't in our order list (at the end)
        for root in current_roots {
            if !self.root_order.contains(&root) {
                self.root_order.push(root);
            }
        }
    }

    /// Move a root entity to a new index
    fn move_root(&mut self, entity: Entity, new_index: usize) {
        if let Some(current) = self.root_order.iter().position(|&e| e == entity) {
            self.root_order.remove(current);
            let clamped = new_index.min(self.root_order.len());
            self.root_order.insert(clamped, entity);
        }
    }

    /// Render the entity tree
    fn render_tree(&mut self, ui: &mut Ui, world: &mut World, selection: &mut Selection) {
        // Sync root order with world state first
        self.sync_root_order(world);

        ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if self.root_order.is_empty() {
                    ui.label(RichText::new("No entities in scene").italics().weak());
                    return;
                }

                // Render roots in explicit order
                let roots = self.root_order.clone();
                for root in roots {
                    self.render_entity_node(ui, world, selection, root, 0);
                }
            });

        // Handle keyboard shortcuts
        self.handle_keyboard_shortcuts(ui, world, selection);

        // Render drag ghost (floating element following cursor)
        self.render_drag_ghost(ui, world);

        // Clear drag source after mouse release (once per frame, not per entity)
        if ui.input(|i| i.pointer.any_released()) {
            self.drag_source = None;
        }
    }

    /// Render a single entity node in the tree
    fn render_entity_node(
        &mut self,
        ui: &mut Ui,
        world: &mut World,
        selection: &mut Selection,
        entity: Entity,
        depth: usize,
    ) {
        // Get entity name
        let name = world
            .get::<&Name>(entity)
            .map(|n| n.0.clone())
            .unwrap_or_else(|_| format!("Entity {:?}", entity.id()));

        // Check if entity matches filter
        if self.filter_active && !self.matches_filter(&name, world, entity) {
            return;
        }

        // Get children
        let children: Vec<Entity> = world
            .get::<&Children>(entity)
            .map(|c| c.0.clone())
            .unwrap_or_default();

        let has_children = !children.is_empty();
        let is_selected = selection.is_selected(entity);
        let is_renaming = self.renaming_entity == Some(entity);
        let entity_id = entity.id() as u64;
        let is_expanded = self.expanded.contains(&entity_id);

        // Create a horizontal layout for the row
        ui.horizontal(|ui| {
            // Indentation
            let indent = depth as f32 * 16.0;
            ui.add_space(indent);

            // Expand/collapse arrow for entities with children
            if has_children {
                let arrow = if is_expanded { "v" } else { ">" };
                if ui.small_button(arrow).clicked() {
                    if is_expanded {
                        self.expanded.remove(&entity_id);
                    } else {
                        self.expanded.insert(entity_id);
                    }
                }
            } else {
                // Spacer for alignment
                ui.add_space(20.0);
            }

            // Entity icon based on components
            let icon = self.get_entity_icon(world, entity);
            ui.label(icon);

            // Entity name (or rename field)
            if is_renaming {
                let response = ui.add(TextEdit::singleline(&mut self.rename_buffer).desired_width(100.0));

                // Auto-focus on first frame
                response.request_focus();

                // Check Enter while we have focus - commit rename
                if response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    self.commit_rename(world, entity);
                    self.renaming_entity = None;
                }
                // Check Escape - cancel rename
                else if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    self.renaming_entity = None;
                }
                // Lost focus (clicked elsewhere) - commit rename
                else if response.lost_focus() {
                    self.commit_rename(world, entity);
                    self.renaming_entity = None;
                }
            } else {
                // Selectable label
                let label = if is_selected {
                    RichText::new(&name).strong()
                } else {
                    RichText::new(&name)
                };

                let response = ui.selectable_label(is_selected, label);
                // Enable drag sensing (selectable_label only senses clicks by default)
                let response = response.interact(egui::Sense::drag());

                // Handle selection
                if response.clicked() {
                    if ui.input(|i| i.modifiers.ctrl) {
                        selection.toggle(entity);
                    } else {
                        selection.select(entity);
                    }
                }

                // Double-click to rename
                if response.double_clicked() {
                    self.start_rename(world, entity);
                }

                // Right-click context menu
                response.context_menu(|ui| {
                    self.render_context_menu(ui, world, selection, entity);
                });

                // Drag-and-drop
                self.handle_drag_drop(ui, &response, world, entity);
            }
        });

        // Render children (if expanded and has children)
        if has_children && is_expanded {
            for child in children {
                self.render_entity_node(ui, world, selection, child, depth + 1);
            }
        }
    }

    /// Get icon for entity based on its components
    fn get_entity_icon(&self, world: &World, entity: Entity) -> &'static str {
        // Check for specific component types
        if world.get::<&Camera>(entity).is_ok() {
            return "CAM";
        }
        if world.get::<&DirectionalLight>(entity).is_ok() {
            return "SUN";
        }
        if world.get::<&PointLight>(entity).is_ok() {
            return "LIT";
        }
        if world.get::<&MeshRenderer>(entity).is_ok() {
            return "MSH";
        }
        if world.get::<&Children>(entity).is_ok() {
            return "GRP";
        }
        "ENT" // Default icon
    }

    /// Check if entity matches current filter
    fn matches_filter(&self, name: &str, world: &World, entity: Entity) -> bool {
        let search_lower = self.search_text.to_lowercase();

        // Check name
        if name.to_lowercase().contains(&search_lower) {
            return true;
        }

        // Check children recursively
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

    /// Create a new empty entity
    fn create_empty_entity(&self, world: &mut World) {
        let count = world.iter().count();
        world.spawn((Transform::default(), Name::new(format!("Entity {}", count))));
    }

    /// Start renaming an entity
    fn start_rename(&mut self, world: &World, entity: Entity) {
        self.renaming_entity = Some(entity);
        self.rename_buffer = world
            .get::<&Name>(entity)
            .map(|n| n.0.clone())
            .unwrap_or_default();
    }

    /// Commit the rename
    fn commit_rename(&mut self, world: &mut World, entity: Entity) {
        if !self.rename_buffer.is_empty() {
            // Check if Name component exists first
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

    /// Render right-click context menu
    fn render_context_menu(
        &mut self,
        ui: &mut Ui,
        world: &mut World,
        selection: &mut Selection,
        entity: Entity,
    ) {
        if ui.button("Add Child").clicked() {
            let child = world.spawn((Transform::default(), Name::new("New Child")));
            set_parent(world, child, entity);
            // Expand parent to show new child
            self.expanded.insert(entity.id() as u64);
            ui.close_menu();
        }

        if ui.button("Rename").clicked() {
            self.start_rename(world, entity);
            ui.close_menu();
        }

        if ui.button("Duplicate").clicked() {
            self.duplicate_entity(world, entity);
            ui.close_menu();
        }

        ui.separator();

        if ui.button("Delete").clicked() {
            self.delete_entity(world, selection, entity);
            ui.close_menu();
        }
    }

    /// Duplicate an entity
    fn duplicate_entity(&self, world: &mut World, entity: Entity) {
        // Get components to copy
        let name = world
            .get::<&Name>(entity)
            .map(|n| format!("{} (Copy)", n.0))
            .unwrap_or_else(|_| "Entity (Copy)".to_string());

        let transform = world.get::<&Transform>(entity).map(|t| *t).unwrap_or_default();

        // Create duplicate (simple - doesn't copy all components)
        world.spawn((transform, Name::new(name)));
    }

    /// Delete an entity
    fn delete_entity(&self, world: &mut World, selection: &mut Selection, entity: Entity) {
        selection.remove(entity);
        despawn_recursive(world, entity);
    }

    /// Handle drag-and-drop for reordering and reparenting
    fn handle_drag_drop(
        &mut self,
        ui: &mut Ui,
        response: &egui::Response,
        world: &mut World,
        entity: Entity,
    ) {
        // Make this item a drag source - use dragged() which is more reliable
        if response.dragged() {
            self.drag_source = Some(entity);
        }

        // Drop target detection with position feedback
        if let Some(source) = self.drag_source {
            // Manually check if pointer is over this item's rect (response.hovered() doesn't work during drag)
            let pointer_pos = ui.input(|i| i.pointer.hover_pos());
            let is_hovered = pointer_pos.map(|p| response.rect.contains(p)).unwrap_or(false);
            if source != entity && is_hovered {
                // Determine drop position (above or below center of target)
                let mouse_y = ui.input(|i| i.pointer.hover_pos().map(|p| p.y).unwrap_or(0.0));
                let center_y = response.rect.center().y;
                let drop_above = mouse_y < center_y;

                // Visual feedback - show line above or below to indicate insertion point
                let line_y = if drop_above {
                    response.rect.top()
                } else {
                    response.rect.bottom()
                };
                ui.painter().hline(
                    response.rect.x_range(),
                    line_y,
                    egui::Stroke::new(2.0, egui::Color32::YELLOW),
                );

                // Add a small triangle/arrow at the left to show insertion point
                let arrow_left = response.rect.left() - 8.0;
                let arrow_size = 5.0;
                let arrow_points = vec![
                    egui::pos2(arrow_left, line_y - arrow_size),
                    egui::pos2(arrow_left + arrow_size * 1.5, line_y),
                    egui::pos2(arrow_left, line_y + arrow_size),
                ];
                ui.painter().add(egui::Shape::convex_polygon(
                    arrow_points,
                    egui::Color32::YELLOW,
                    egui::Stroke::NONE,
                ));
            }
        }

        // Complete the drop when mouse is released
        if ui.input(|i| i.pointer.any_released()) {
            if let Some(source) = self.drag_source {
                // Check if we're hovering over a valid target
                // Use interact_pos() instead of hover_pos() - hover_pos() returns None on release
                let pointer_pos = ui.input(|i| i.pointer.interact_pos());
                let is_hovered = pointer_pos.map(|p| response.rect.contains(p)).unwrap_or(false);
                if source != entity && is_hovered {
                    let mouse_y = pointer_pos.map(|p| p.y).unwrap_or(0.0);
                    let center_y = response.rect.center().y;
                    let drop_above = mouse_y < center_y;

                    self.perform_drop(world, source, entity, drop_above);
                }
            }
            // Note: drag_source is cleared in render_tree() after all entities are processed
        }
    }

    /// Perform the drop operation - either sibling reorder or reparent
    fn perform_drop(
        &mut self,
        world: &mut World,
        source: Entity,
        target: Entity,
        drop_above: bool,
    ) {
        // Get parents
        let source_parent = world.get::<&Parent>(source).ok().map(|p| p.0);
        let target_parent = world.get::<&Parent>(target).ok().map(|p| p.0);

        // Same parent = sibling reorder
        if source_parent == target_parent {
            if let Some(parent) = source_parent {
                // Both have same parent - reorder within Children
                if let Ok(mut children) = world.get::<&mut Children>(parent) {
                    if let Some(target_idx) = children.index_of(target) {
                        // Calculate insert index, accounting for source position
                        let source_idx = children.index_of(source);
                        let mut insert_idx = if drop_above { target_idx } else { target_idx + 1 };

                        // If source is before target and we're moving down, adjust index
                        if let Some(src_idx) = source_idx {
                            if src_idx < target_idx {
                                insert_idx = insert_idx.saturating_sub(1);
                            }
                        }

                        children.move_to_index(source, insert_idx);
                    }
                }
            } else {
                // Both are roots - reorder in root_order
                if let Some(target_idx) = self.root_order.iter().position(|&e| e == target) {
                    let source_idx = self.root_order.iter().position(|&e| e == source);
                    let mut insert_idx = if drop_above { target_idx } else { target_idx + 1 };

                    // Adjust for source position
                    if let Some(src_idx) = source_idx {
                        if src_idx < target_idx {
                            insert_idx = insert_idx.saturating_sub(1);
                        }
                    }

                    self.move_root(source, insert_idx);
                }
            }
        } else {
            // Different parents = reparent
            // Make source a sibling of target (same parent as target)
            if let Some(parent) = target_parent {
                // Target has a parent - make source a child of that parent
                set_parent(world, source, parent);
                // Then reorder to be near target
                if let Ok(mut children) = world.get::<&mut Children>(parent) {
                    if let Some(target_idx) = children.index_of(target) {
                        let insert_idx = if drop_above { target_idx } else { target_idx + 1 };
                        children.move_to_index(source, insert_idx);
                    }
                }
            } else {
                // Target is root - make source a root too
                remove_parent(world, source);
                // Add source to root_order if not present
                if !self.root_order.contains(&source) {
                    self.root_order.push(source);
                }
                // Reorder in root list
                if let Some(target_idx) = self.root_order.iter().position(|&e| e == target) {
                    let insert_idx = if drop_above { target_idx } else { target_idx + 1 };
                    self.move_root(source, insert_idx);
                }
            }
            self.expanded.insert(target.id() as u64);
        }
    }

    /// Render floating ghost element during drag operation
    fn render_drag_ghost(&self, ui: &mut Ui, world: &World) {
        if let Some(source) = self.drag_source {
            if let Some(pointer_pos) = ui.ctx().pointer_hover_pos() {
                // Get entity name for the ghost label
                let name = world
                    .get::<&Name>(source)
                    .map(|n| n.0.clone())
                    .unwrap_or_else(|_| format!("Entity {:?}", source.id()));

                // Get entity icon
                let icon = self.get_entity_icon(world, source);

                // Create ghost text with icon
                let ghost_text = format!("{} {}", icon, name);

                // Use a top-level layer to ensure ghost is above everything
                let layer_id =
                    egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("drag_ghost"));
                let painter = ui.ctx().layer_painter(layer_id);

                // Draw ghost slightly offset from cursor
                let ghost_pos = pointer_pos + egui::vec2(12.0, 0.0);

                // Measure text
                let galley = painter.layout_no_wrap(
                    ghost_text.clone(),
                    egui::FontId::default(),
                    egui::Color32::WHITE,
                );

                let bg_rect = egui::Rect::from_min_size(
                    ghost_pos - egui::vec2(4.0, galley.size().y / 2.0 + 4.0),
                    galley.size() + egui::vec2(8.0, 8.0),
                );

                // Semi-transparent background
                painter.rect_filled(
                    bg_rect,
                    4.0,
                    egui::Color32::from_rgba_unmultiplied(30, 30, 30, 220),
                );

                // Yellow border (matches drop indicator)
                painter.rect_stroke(
                    bg_rect,
                    4.0,
                    egui::Stroke::new(1.5, egui::Color32::YELLOW),
                );

                // Ghost text
                painter.text(
                    ghost_pos,
                    egui::Align2::LEFT_CENTER,
                    ghost_text,
                    egui::FontId::default(),
                    egui::Color32::WHITE,
                );
            }
        }
    }

    /// Handle keyboard shortcuts
    fn handle_keyboard_shortcuts(
        &mut self,
        ui: &mut Ui,
        world: &mut World,
        selection: &mut Selection,
    ) {
        // Delete key
        if ui.input(|i| i.key_pressed(egui::Key::Delete)) {
            if let Some(entity) = selection.primary() {
                self.delete_entity(world, selection, entity);
            }
        }

        // F2 to rename
        if ui.input(|i| i.key_pressed(egui::Key::F2)) {
            if let Some(entity) = selection.primary() {
                self.start_rename(world, entity);
            }
        }

        // Ctrl+D to duplicate
        if ui.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::D)) {
            if let Some(entity) = selection.primary() {
                self.duplicate_entity(world, entity);
            }
        }
    }
}
