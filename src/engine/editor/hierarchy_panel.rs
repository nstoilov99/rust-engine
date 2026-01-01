//! Scene Hierarchy Panel - tree view of all entities

use super::Selection;
use crate::engine::ecs::{
    hierarchy::{can_set_parent, despawn_recursive, get_root_entities, remove_parent, set_parent},
    Camera, Children, DirectionalLight, MeshRenderer, Name, Parent, PointLight, Transform,
};
use egui::{pos2, Color32, Context, RichText, ScrollArea, SidePanel, Stroke, TextEdit, Ui};
use hecs::{Entity, World};
use std::collections::HashSet;
use std::time::{Duration, Instant};

/// Drop mode for drag-and-drop operations
/// Determined by mouse Y position within the row
#[derive(Debug, Clone, Copy, PartialEq)]
enum DropMode {
    /// Insert above target (top 25% of row) - makes sibling before target
    InsertAbove,
    /// Make child of target (middle 50% of row) - makes child of target
    MakeChild,
    /// Insert below target (bottom 25% of row) - makes sibling after target
    InsertBelow,
}

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
    /// Entity being hovered during drag (for auto-expand)
    drag_hover_entity: Option<Entity>,
    /// When drag hover started (for auto-expand delay)
    drag_hover_start: Option<Instant>,
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

    /// Render the hierarchy panel as a side panel
    pub fn show(&mut self, ctx: &Context, world: &mut World, selection: &mut Selection) {
        SidePanel::left("hierarchy_panel")
            .resizable(true)
            .default_width(250.0)
            .min_width(150.0)
            .show(ctx, |ui| {
                self.show_contents(ui, world, selection);
            });
    }

    /// Render just the contents (for use inside dock tabs)
    pub fn show_contents(&mut self, ui: &mut Ui, world: &mut World, selection: &mut Selection) {
        self.render_header(ui, world);
        ui.separator();
        self.render_search(ui);
        ui.separator();
        self.render_tree(ui, world, selection);
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

        // ESC to cancel drag
        if self.drag_source.is_some() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.drag_source = None;
            self.drag_hover_entity = None;
            self.drag_hover_start = None;
        }

        ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if self.root_order.is_empty() {
                    ui.label(RichText::new("No entities in scene").weak());
                    return;
                }

                // Render roots in explicit order (Entity is Copy, so we can iterate by value)
                for i in 0..self.root_order.len() {
                    let root = self.root_order[i]; // Entity is Copy
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
            self.drag_hover_entity = None;
            self.drag_hover_start = None;
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

        // Check if this is a valid drop target (for visual feedback)
        let is_valid_drop_target = self.drag_source.map_or(false, |source| {
            source != entity && can_set_parent(world, source, entity)
        });

        // Create a horizontal layout for the row
        let row_response = ui.horizontal(|ui| {
            // Draw tree guide lines for depth
            self.draw_tree_guides(ui, depth);

            // Indentation
            let indent = depth as f32 * 16.0;
            ui.add_space(indent);

            // Expand/collapse arrow for entities with children
            if has_children {
                // Minimalist painted triangle instead of button
                let (rect, response) =
                    ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::click());

                let center = rect.center();
                let size = 3.5;
                let color = if response.hovered() {
                    Color32::WHITE
                } else {
                    Color32::from_gray(165)  // Increased from 140 for better visibility
                };

                let points = if is_expanded {
                    // Down arrow
                    vec![
                        pos2(center.x - size, center.y - size * 0.4),
                        pos2(center.x + size, center.y - size * 0.4),
                        pos2(center.x, center.y + size * 0.6),
                    ]
                } else {
                    // Right arrow
                    vec![
                        pos2(center.x - size * 0.4, center.y - size),
                        pos2(center.x + size * 0.6, center.y),
                        pos2(center.x - size * 0.4, center.y + size),
                    ]
                };

                ui.painter()
                    .add(egui::Shape::convex_polygon(points, color, Stroke::NONE));

                if response.clicked() {
                    if is_expanded {
                        self.expanded.remove(&entity_id);
                    } else {
                        self.expanded.insert(entity_id);
                    }
                }
            } else {
                // Spacer for alignment
                ui.add_space(16.0);
            }

            // Entity icon with color
            let (icon, icon_color) = self.get_entity_icon_with_color(world, entity);
            ui.label(RichText::new(icon).color(icon_color));

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
                // Custom selection rendering (avoids egui's harsh cyan default)
                let label = if is_selected {
                    RichText::new(&name).strong().color(Color32::WHITE)
                } else {
                    RichText::new(&name)
                };

                // Use regular label with click+drag sensing
                let response = ui.label(label).interact(egui::Sense::click_and_drag());

                // Draw custom selection background (muted blue-gray, increased alpha for visibility)
                if is_selected {
                    ui.painter().rect_filled(
                        response.rect.expand(2.0),
                        3.0,
                        Color32::from_rgba_unmultiplied(60, 90, 140, 160),
                    );
                }

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
                self.handle_drag_drop(ui, &response, world, entity, is_valid_drop_target, has_children, is_expanded);
            }
        });

        // Draw row hover/drop target highlight
        let row_rect = row_response.response.rect;
        let pointer_pos = ui.input(|i| i.pointer.hover_pos());
        let is_hovered = pointer_pos.map(|p| row_rect.contains(p)).unwrap_or(false);

        // Hover highlight (when not dragging) - increased alpha for visibility
        if is_hovered && self.drag_source.is_none() && !is_renaming {
            ui.painter().rect_filled(
                row_rect,
                2.0,
                Color32::from_rgba_unmultiplied(255, 255, 255, 40),
            );
        }

        // Drop target highlight (when dragging) - increased alpha for visibility
        if is_hovered && is_valid_drop_target {
            ui.painter().rect_filled(
                row_rect,
                2.0,
                Color32::from_rgba_unmultiplied(255, 200, 0, 60),
            );
        }

        // Auto-expand on drag hover (after 500ms)
        if self.drag_source.is_some() && is_hovered && has_children && !is_expanded && is_valid_drop_target {
            if self.drag_hover_entity != Some(entity) {
                self.drag_hover_entity = Some(entity);
                self.drag_hover_start = Some(Instant::now());
            } else if let Some(start) = self.drag_hover_start {
                if start.elapsed() > Duration::from_millis(500) {
                    self.expanded.insert(entity_id);
                    self.drag_hover_entity = None;
                    self.drag_hover_start = None;
                }
            }
        } else if self.drag_hover_entity == Some(entity) && !is_hovered {
            self.drag_hover_entity = None;
            self.drag_hover_start = None;
        }

        // Render children (if expanded and has children)
        if has_children && is_expanded {
            for child in children {
                self.render_entity_node(ui, world, selection, child, depth + 1);
            }
        }
    }

    /// Draw tree guide lines to show hierarchy depth
    fn draw_tree_guides(&self, ui: &mut Ui, depth: usize) {
        if depth == 0 {
            return;
        }

        let row_rect = ui.max_rect();
        let guide_color = Color32::from_gray(75);

        for d in 0..depth {
            let x = 8.0 + (d as f32 * 16.0);
            ui.painter().line_segment(
                [pos2(x, row_rect.top()), pos2(x, row_rect.bottom())],
                Stroke::new(1.0, guide_color),
            );
        }
    }

    /// Get icon and color for entity based on its components
    fn get_entity_icon_with_color(&self, world: &World, entity: Entity) -> (&'static str, Color32) {
        // Check for specific component types (using Unicode symbols for cleaner look)
        if world.get::<&Camera>(entity).is_ok() {
            return ("\u{1F3A5}", Color32::from_rgb(100, 180, 255)); // 🎥 Camera - Blue
        }
        if world.get::<&DirectionalLight>(entity).is_ok() {
            return ("\u{2600}", Color32::from_rgb(255, 220, 100)); // ☀ Sun - Yellow
        }
        if world.get::<&PointLight>(entity).is_ok() {
            return ("\u{1F4A1}", Color32::from_rgb(255, 180, 100)); // 💡 Light bulb - Orange
        }
        if world.get::<&MeshRenderer>(entity).is_ok() {
            return ("\u{25A6}", Color32::from_rgb(150, 150, 255)); // ▦ Mesh grid - Purple
        }
        if world.get::<&Children>(entity).is_ok() {
            return ("\u{1F4C1}", Color32::from_rgb(180, 180, 180)); // 📁 Folder - Gray
        }
        ("\u{25CB}", Color32::from_rgb(140, 140, 140)) // ○ Circle - Default dim gray
    }

    /// Get icon for entity based on its components (legacy, for ghost)
    fn get_entity_icon(&self, world: &World, entity: Entity) -> &'static str {
        self.get_entity_icon_with_color(world, entity).0
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
            ui.close();
        }

        if ui.button("Rename").clicked() {
            self.start_rename(world, entity);
            ui.close();
        }

        if ui.button("Duplicate").clicked() {
            self.duplicate_entity(world, entity);
            ui.close();
        }

        ui.separator();

        if ui.button("Delete").clicked() {
            self.delete_entity(world, selection, entity);
            ui.close();
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

    /// Calculate drop mode from mouse Y position within the row
    fn calculate_drop_mode(&self, mouse_y: f32, rect: &egui::Rect) -> DropMode {
        let row_height = rect.height();
        let top_zone = rect.top() + row_height * 0.25;
        let bottom_zone = rect.bottom() - row_height * 0.25;

        if mouse_y < top_zone {
            DropMode::InsertAbove
        } else if mouse_y > bottom_zone {
            DropMode::InsertBelow
        } else {
            DropMode::MakeChild
        }
    }

    /// Check if a drop is valid for the given mode
    fn is_valid_drop(&self, world: &World, source: Entity, target: Entity, mode: DropMode) -> bool {
        if source == target {
            return false;
        }

        match mode {
            DropMode::MakeChild => {
                // Check if source can become a child of target
                can_set_parent(world, source, target)
            }
            DropMode::InsertAbove | DropMode::InsertBelow => {
                // For sibling insert, check if we can become child of target's parent
                if let Ok(parent) = world.get::<&Parent>(target) {
                    can_set_parent(world, source, parent.0)
                } else {
                    // Target is root - always valid to become a root sibling
                    true
                }
            }
        }
    }

    /// Handle drag-and-drop for reordering and reparenting
    fn handle_drag_drop(
        &mut self,
        ui: &mut Ui,
        response: &egui::Response,
        world: &mut World,
        entity: Entity,
        _is_valid_drop_target: bool, // No longer used - we calculate per-mode
        _has_children: bool,
        _is_expanded: bool,
    ) {
        // Make this item a drag source - use dragged() which is more reliable
        if response.dragged() {
            self.drag_source = Some(entity);
        }

        // Drop target detection with position feedback
        if let Some(source) = self.drag_source {
            if source == entity {
                return; // Can't drop on self
            }

            // Manually check if pointer is over this item's rect (response.hovered() doesn't work during drag)
            let pointer_pos = ui.input(|i| i.pointer.hover_pos());
            let is_hovered = pointer_pos.map(|p| response.rect.contains(p)).unwrap_or(false);

            if is_hovered {
                let mouse_y = pointer_pos.map(|p| p.y).unwrap_or(0.0);
                let drop_mode = self.calculate_drop_mode(mouse_y, &response.rect);
                let is_valid = self.is_valid_drop(world, source, entity, drop_mode);

                if is_valid {
                    // Visual feedback based on drop mode
                    match drop_mode {
                        DropMode::InsertAbove => {
                            // Line at top of row
                            self.draw_insertion_line(ui, response.rect.top(), &response.rect);
                        }
                        DropMode::InsertBelow => {
                            // Line at bottom of row
                            self.draw_insertion_line(ui, response.rect.bottom(), &response.rect);
                        }
                        DropMode::MakeChild => {
                            // Highlight the entire row + indent indicator
                            ui.painter().rect_filled(
                                response.rect,
                                2.0,
                                Color32::from_rgba_unmultiplied(100, 200, 100, 50),
                            );
                            ui.painter().rect_stroke(
                                response.rect,
                                2.0,
                                Stroke::new(2.0, Color32::from_rgb(100, 200, 100)),
                                egui::epaint::StrokeKind::Outside,
                            );
                            // Draw a small "+" icon to indicate "add as child"
                            let icon_pos = pos2(response.rect.right() - 16.0, response.rect.center().y);
                            ui.painter().text(
                                icon_pos,
                                egui::Align2::CENTER_CENTER,
                                "+",
                                egui::FontId::proportional(14.0),
                                Color32::from_rgb(100, 200, 100),
                            );
                        }
                    }
                } else {
                    // Show "invalid" indicator
                    ui.painter().rect_stroke(
                        response.rect,
                        2.0,
                        Stroke::new(2.0, Color32::from_rgb(200, 60, 60)),
                        egui::epaint::StrokeKind::Outside,
                    );
                }
            }
        }

        // Complete the drop when mouse is released
        if ui.input(|i| i.pointer.any_released()) {
            if let Some(source) = self.drag_source {
                if source == entity {
                    return;
                }

                // Use interact_pos() instead of hover_pos() - hover_pos() returns None on release
                let pointer_pos = ui.input(|i| i.pointer.interact_pos());
                let is_hovered = pointer_pos.map(|p| response.rect.contains(p)).unwrap_or(false);

                if is_hovered {
                    let mouse_y = pointer_pos.map(|p| p.y).unwrap_or(0.0);
                    let drop_mode = self.calculate_drop_mode(mouse_y, &response.rect);
                    let is_valid = self.is_valid_drop(world, source, entity, drop_mode);

                    if is_valid {
                        self.perform_drop(world, source, entity, drop_mode);
                    }
                }
            }
            // Note: drag_source is cleared in render_tree() after all entities are processed
        }
    }

    /// Draw insertion line indicator for sibling drops
    fn draw_insertion_line(&self, ui: &mut Ui, line_y: f32, rect: &egui::Rect) {
        ui.painter().hline(
            rect.x_range(),
            line_y,
            Stroke::new(2.0, Color32::YELLOW),
        );

        // Add a small triangle/arrow at the left to show insertion point
        let arrow_left = rect.left() - 8.0;
        let arrow_size = 5.0;
        let arrow_points = vec![
            pos2(arrow_left, line_y - arrow_size),
            pos2(arrow_left + arrow_size * 1.5, line_y),
            pos2(arrow_left, line_y + arrow_size),
        ];
        ui.painter().add(egui::Shape::convex_polygon(
            arrow_points,
            Color32::YELLOW,
            Stroke::NONE,
        ));
    }

    /// Perform the drop operation based on drop mode
    fn perform_drop(
        &mut self,
        world: &mut World,
        source: Entity,
        target: Entity,
        drop_mode: DropMode,
    ) {
        match drop_mode {
            DropMode::MakeChild => {
                // Make source a child of target
                set_parent(world, source, target);
                // Expand target to show the new child
                self.expanded.insert(target.id() as u64);
            }
            DropMode::InsertAbove | DropMode::InsertBelow => {
                let drop_above = drop_mode == DropMode::InsertAbove;
                self.perform_sibling_drop(world, source, target, drop_above);
            }
        }
    }

    /// Perform sibling drop (insert above/below target)
    fn perform_sibling_drop(
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
            // Different parents = reparent as sibling of target
            if let Some(parent) = target_parent {
                // Target has a parent - make source a child of that parent (sibling of target)
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
                    egui::epaint::StrokeKind::Outside,
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
