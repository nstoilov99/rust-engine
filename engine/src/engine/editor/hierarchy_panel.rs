//! Scene Hierarchy Panel - tree view of all entities

use super::Selection;
use crate::engine::ecs::resources::PlayMode;
use crate::engine::ecs::{
    hierarchy::{can_set_parent, despawn_recursive, get_root_entities, remove_parent, set_parent},
    Camera, Children, DirectionalLight, EntityGuid, MeshRenderer, Name, Parent, PointLight,
    Transform,
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

/// One visible row in the flattened hierarchy list.
struct VisibleRow {
    entity: Entity,
    depth: usize,
    name: String,
    has_children: bool,
    is_expanded: bool,
    icon: &'static str,
    icon_color: Color32,
}

const ROW_HEIGHT: f32 = 22.0;

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
    /// Reusable buffer for flat visible rows (avoids per-frame allocation).
    flat_rows: Vec<VisibleRow>,
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
    pub fn show(
        &mut self,
        ctx: &Context,
        world: &mut World,
        selection: &mut Selection,
        play_mode: PlayMode,
    ) {
        SidePanel::left("hierarchy_panel")
            .resizable(true)
            .default_width(250.0)
            .min_width(150.0)
            .show(ctx, |ui| {
                self.show_contents(ui, world, selection, play_mode);
            });
    }

    /// Render just the contents (for use inside dock tabs)
    pub fn show_contents(
        &mut self,
        ui: &mut Ui,
        world: &mut World,
        selection: &mut Selection,
        play_mode: PlayMode,
    ) {
        let read_only = play_mode != PlayMode::Edit;

        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            if self.renaming_entity.is_some() {
                self.renaming_entity = None;
            } else if !selection.is_empty() {
                selection.clear();
            }
        }

        self.render_header(ui, world, read_only);
        ui.separator();
        self.render_search(ui);
        ui.separator();
        self.render_tree(ui, world, selection, read_only);
    }

    fn render_header(&mut self, ui: &mut Ui, world: &mut World, read_only: bool) {
        ui.horizontal(|ui| {
            ui.heading("Hierarchy");
            if read_only {
                ui.label(RichText::new("(Playing)").weak().italics().small());
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_enabled_ui(!read_only, |ui| {
                    if ui.button("+").on_hover_text("Add Entity").clicked() {
                        self.create_empty_entity(world);
                    }
                });
            });
        });
    }

    fn render_search(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label("Search:");
            let response = ui.add(
                TextEdit::singleline(&mut self.search_text)
                    .hint_text("Filter entities...")
                    .desired_width(ui.available_width()),
            );
            self.filter_active = !self.search_text.is_empty();

            if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.search_text.clear();
                self.filter_active = false;
            }
        });
    }

    /// Sync root_order with world state.
    pub fn sync_root_order(&mut self, world: &World) {
        let current_roots: HashSet<Entity> = get_root_entities(world).into_iter().collect();
        self.root_order.retain(|e| current_roots.contains(e));
        for root in current_roots {
            if !self.root_order.contains(&root) {
                self.root_order.push(root);
            }
        }
    }

    fn move_root(&mut self, entity: Entity, new_index: usize) {
        if let Some(current) = self.root_order.iter().position(|&e| e == entity) {
            self.root_order.remove(current);
            let clamped = new_index.min(self.root_order.len());
            self.root_order.insert(clamped, entity);
        }
    }

    // ─── flat-row construction ─────────────────────────────────────────

    /// Build a flat visible-row list by walking the hierarchy top-down.
    /// Respects expand/collapse and filter state.
    fn build_visible_rows(&mut self, world: &World) {
        self.flat_rows.clear();

        let roots: Vec<Entity> = self.root_order.clone();
        for root in roots {
            self.collect_rows(world, root, 0);
        }
    }

    fn collect_rows(&mut self, world: &World, entity: Entity, depth: usize) {
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
        let (icon, icon_color) = Self::entity_icon(world, entity);

        self.flat_rows.push(VisibleRow {
            entity,
            depth,
            name,
            has_children,
            is_expanded,
            icon,
            icon_color,
        });

        if has_children && is_expanded {
            for child in children {
                self.collect_rows(world, child, depth + 1);
            }
        }
    }

    // ─── render tree with virtualization ────────────────────────────────

    fn render_tree(
        &mut self,
        ui: &mut Ui,
        world: &mut World,
        selection: &mut Selection,
        read_only: bool,
    ) {
        self.sync_root_order(world);

        if read_only {
            self.drag_source = None;
        }
        if self.drag_source.is_some() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.drag_source = None;
            self.drag_hover_entity = None;
            self.drag_hover_start = None;
        }

        self.build_visible_rows(world);

        let total_rows = self.flat_rows.len();

        ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, ROW_HEIGHT, total_rows, |ui, row_range| {
                if total_rows == 0 {
                    ui.label(RichText::new("No entities in scene").weak());
                    return;
                }

                for idx in row_range {
                    if idx >= self.flat_rows.len() {
                        break;
                    }
                    // Extract data we need (flat_rows is borrowed immutably for data,
                    // but we may mutate self for expand/collapse/drag).
                    let entity = self.flat_rows[idx].entity;
                    let depth = self.flat_rows[idx].depth;
                    let has_children = self.flat_rows[idx].has_children;
                    let is_expanded = self.flat_rows[idx].is_expanded;
                    let icon = self.flat_rows[idx].icon;
                    let icon_color = self.flat_rows[idx].icon_color;
                    // Clone the name to avoid borrow conflicts.
                    let name = self.flat_rows[idx].name.clone();

                    self.render_row(
                        ui,
                        world,
                        selection,
                        entity,
                        depth,
                        &name,
                        has_children,
                        is_expanded,
                        icon,
                        icon_color,
                        read_only,
                    );
                }
            });

        self.handle_keyboard_shortcuts(ui, world, selection, read_only);

        if !read_only {
            self.render_drag_ghost(ui, world);
        }

        if ui.input(|i| i.pointer.any_released()) {
            self.drag_source = None;
            self.drag_hover_entity = None;
            self.drag_hover_start = None;
        }
    }

    /// Render a single flattened row.
    #[allow(clippy::too_many_arguments)]
    fn render_row(
        &mut self,
        ui: &mut Ui,
        world: &mut World,
        selection: &mut Selection,
        entity: Entity,
        depth: usize,
        name: &str,
        has_children: bool,
        is_expanded: bool,
        icon: &str,
        icon_color: Color32,
        read_only: bool,
    ) {
        let is_selected = selection.is_selected(entity);
        let is_renaming = self.renaming_entity == Some(entity);
        let entity_id = entity.id() as u64;

        let is_valid_drop_target = self
            .drag_source
            .is_some_and(|source| source != entity && can_set_parent(world, source, entity));

        let row_response = ui.horizontal(|ui| {
            Self::draw_tree_guides(ui, depth);

            let indent = depth as f32 * 16.0;
            ui.add_space(indent);

            if has_children {
                let (rect, response) =
                    ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::click());

                let center = rect.center();
                let sz = 3.5;
                let color = if response.hovered() {
                    Color32::WHITE
                } else {
                    Color32::from_gray(165)
                };

                let points = if is_expanded {
                    vec![
                        pos2(center.x - sz, center.y - sz * 0.4),
                        pos2(center.x + sz, center.y - sz * 0.4),
                        pos2(center.x, center.y + sz * 0.6),
                    ]
                } else {
                    vec![
                        pos2(center.x - sz * 0.4, center.y - sz),
                        pos2(center.x + sz * 0.6, center.y),
                        pos2(center.x - sz * 0.4, center.y + sz),
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
                ui.add_space(16.0);
            }

            ui.label(RichText::new(icon).color(icon_color));

            if is_renaming {
                let response =
                    ui.add(TextEdit::singleline(&mut self.rename_buffer).desired_width(100.0));
                response.request_focus();

                if response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    self.commit_rename(world, entity);
                    self.renaming_entity = None;
                } else if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    self.renaming_entity = None;
                } else if response.lost_focus() {
                    self.commit_rename(world, entity);
                    self.renaming_entity = None;
                }
            } else {
                let label = if is_selected {
                    RichText::new(name).strong().color(Color32::WHITE)
                } else {
                    RichText::new(name)
                };

                let response = ui.label(label).interact(egui::Sense::click_and_drag());

                if is_selected {
                    ui.painter().rect_filled(
                        response.rect.expand(2.0),
                        3.0,
                        Color32::from_rgba_unmultiplied(60, 90, 140, 160),
                    );
                }

                if response.clicked() {
                    if ui.input(|i| i.modifiers.ctrl) {
                        selection.toggle(entity);
                    } else {
                        selection.select(entity);
                    }
                }

                if !read_only {
                    if response.double_clicked() {
                        self.start_rename(world, entity);
                    }

                    response.context_menu(|ui| {
                        self.render_context_menu(ui, world, selection, entity);
                    });

                    self.handle_drag_drop(ui, &response, world, entity);
                }
            }
        });

        let row_rect = row_response.response.rect;
        let pointer_pos = ui.input(|i| i.pointer.hover_pos());
        let is_hovered = pointer_pos.map(|p| row_rect.contains(p)).unwrap_or(false);

        if is_hovered && self.drag_source.is_none() && !is_renaming {
            ui.painter().rect_filled(
                row_rect,
                2.0,
                Color32::from_rgba_unmultiplied(255, 255, 255, 40),
            );
        }

        if is_hovered && is_valid_drop_target {
            ui.painter().rect_filled(
                row_rect,
                2.0,
                Color32::from_rgba_unmultiplied(255, 200, 0, 60),
            );
        }

        if self.drag_source.is_some()
            && is_hovered
            && has_children
            && !is_expanded
            && is_valid_drop_target
        {
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
    }

    // ─── helpers ───────────────────────────────────────────────────────

    fn draw_tree_guides(ui: &mut Ui, depth: usize) {
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

    fn entity_icon(world: &World, entity: Entity) -> (&'static str, Color32) {
        if world.get::<&Camera>(entity).is_ok() {
            return ("\u{1F3A5}", Color32::from_rgb(100, 180, 255));
        }
        if world.get::<&DirectionalLight>(entity).is_ok() {
            return ("\u{2600}", Color32::from_rgb(255, 220, 100));
        }
        if world.get::<&PointLight>(entity).is_ok() {
            return ("\u{1F4A1}", Color32::from_rgb(255, 180, 100));
        }
        if world.get::<&MeshRenderer>(entity).is_ok() {
            return ("\u{25A6}", Color32::from_rgb(150, 150, 255));
        }
        if world.get::<&Children>(entity).is_ok() {
            return ("\u{1F4C1}", Color32::from_rgb(180, 180, 180));
        }
        ("\u{25CB}", Color32::from_rgb(140, 140, 140))
    }

    fn matches_filter(&self, name: &str, world: &World, entity: Entity) -> bool {
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

    fn create_empty_entity(&self, world: &mut World) {
        let count = world.iter().count();
        world.spawn((
            Transform::default(),
            Name::new(format!("Entity {}", count)),
            EntityGuid::new(),
        ));
    }

    fn start_rename(&mut self, world: &World, entity: Entity) {
        self.renaming_entity = Some(entity);
        self.rename_buffer = world
            .get::<&Name>(entity)
            .map(|n| n.0.clone())
            .unwrap_or_default();
    }

    fn commit_rename(&mut self, world: &mut World, entity: Entity) {
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

    fn render_context_menu(
        &mut self,
        ui: &mut Ui,
        world: &mut World,
        selection: &mut Selection,
        entity: Entity,
    ) {
        if ui.button("Add Child").clicked() {
            let child = world.spawn((
                Transform::default(),
                Name::new("New Child"),
                EntityGuid::new(),
            ));
            set_parent(world, child, entity);
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

    fn duplicate_entity(&self, world: &mut World, entity: Entity) {
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

    fn delete_entity(&self, world: &mut World, selection: &mut Selection, entity: Entity) {
        selection.remove(entity);
        despawn_recursive(world, entity);
    }

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

    fn is_valid_drop(&self, world: &World, source: Entity, target: Entity, mode: DropMode) -> bool {
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

    fn handle_drag_drop(
        &mut self,
        ui: &mut Ui,
        response: &egui::Response,
        world: &mut World,
        entity: Entity,
    ) {
        if response.dragged() {
            self.drag_source = Some(entity);
        }

        if let Some(source) = self.drag_source {
            if source == entity {
                return;
            }

            let pointer_pos = ui.input(|i| i.pointer.hover_pos());
            let is_hovered = pointer_pos
                .map(|p| response.rect.contains(p))
                .unwrap_or(false);

            if is_hovered {
                let mouse_y = pointer_pos.map(|p| p.y).unwrap_or(0.0);
                let drop_mode = self.calculate_drop_mode(mouse_y, &response.rect);
                let is_valid = self.is_valid_drop(world, source, entity, drop_mode);

                if is_valid {
                    match drop_mode {
                        DropMode::InsertAbove => {
                            self.draw_insertion_line(ui, response.rect.top(), &response.rect);
                        }
                        DropMode::InsertBelow => {
                            self.draw_insertion_line(ui, response.rect.bottom(), &response.rect);
                        }
                        DropMode::MakeChild => {
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
                            let icon_pos =
                                pos2(response.rect.right() - 16.0, response.rect.center().y);
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
                    ui.painter().rect_stroke(
                        response.rect,
                        2.0,
                        Stroke::new(2.0, Color32::from_rgb(200, 60, 60)),
                        egui::epaint::StrokeKind::Outside,
                    );
                }
            }
        }

        if ui.input(|i| i.pointer.any_released()) {
            if let Some(source) = self.drag_source {
                if source == entity {
                    return;
                }

                let pointer_pos = ui.input(|i| i.pointer.interact_pos());
                let is_hovered = pointer_pos
                    .map(|p| response.rect.contains(p))
                    .unwrap_or(false);

                if is_hovered {
                    let mouse_y = pointer_pos.map(|p| p.y).unwrap_or(0.0);
                    let drop_mode = self.calculate_drop_mode(mouse_y, &response.rect);
                    let is_valid = self.is_valid_drop(world, source, entity, drop_mode);

                    if is_valid {
                        self.perform_drop(world, source, entity, drop_mode);
                    }
                }
            }
        }
    }

    fn draw_insertion_line(&self, ui: &mut Ui, line_y: f32, rect: &egui::Rect) {
        ui.painter()
            .hline(rect.x_range(), line_y, Stroke::new(2.0, Color32::YELLOW));
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

    fn perform_drop(
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

    fn perform_sibling_drop(
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

    fn render_drag_ghost(&self, ui: &mut Ui, world: &World) {
        if let Some(source) = self.drag_source {
            if let Some(pointer_pos) = ui.ctx().pointer_hover_pos() {
                let name = world
                    .get::<&Name>(source)
                    .map(|n| n.0.clone())
                    .unwrap_or_else(|_| format!("Entity {:?}", source.id()));
                let (icon, _) = Self::entity_icon(world, source);
                let ghost_text = format!("{} {}", icon, name);

                let layer_id =
                    egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("drag_ghost"));
                let painter = ui.ctx().layer_painter(layer_id);

                let ghost_pos = pointer_pos + egui::vec2(12.0, 0.0);
                let galley = painter.layout_no_wrap(
                    ghost_text.clone(),
                    egui::FontId::default(),
                    Color32::WHITE,
                );

                let bg_rect = egui::Rect::from_min_size(
                    ghost_pos - egui::vec2(4.0, galley.size().y / 2.0 + 4.0),
                    galley.size() + egui::vec2(8.0, 8.0),
                );

                painter.rect_filled(
                    bg_rect,
                    4.0,
                    Color32::from_rgba_unmultiplied(30, 30, 30, 220),
                );
                painter.rect_stroke(
                    bg_rect,
                    4.0,
                    egui::Stroke::new(1.5, Color32::YELLOW),
                    egui::epaint::StrokeKind::Outside,
                );
                painter.text(
                    ghost_pos,
                    egui::Align2::LEFT_CENTER,
                    ghost_text,
                    egui::FontId::default(),
                    Color32::WHITE,
                );
            }
        }
    }

    fn handle_keyboard_shortcuts(
        &mut self,
        ui: &mut Ui,
        world: &mut World,
        selection: &mut Selection,
        read_only: bool,
    ) {
        if read_only {
            return;
        }
        if ui.input(|i| i.key_pressed(egui::Key::Delete)) {
            if let Some(entity) = selection.primary() {
                self.delete_entity(world, selection, entity);
            }
        }
        if ui.input(|i| i.key_pressed(egui::Key::F2)) {
            if let Some(entity) = selection.primary() {
                self.start_rename(world, entity);
            }
        }
        if ui.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::D)) {
            if let Some(entity) = selection.primary() {
                self.duplicate_entity(world, entity);
            }
        }
    }
}
