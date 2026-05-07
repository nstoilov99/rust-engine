//! Scene Hierarchy Panel - tree view of all entities

use super::hierarchy_icons::HierarchyIcons;
use super::icon_classes::ChromeState;
use super::widgets::IconRegistry;
use super::Selection;
use crate::engine::ecs::resources::PlayMode;
use crate::engine::ecs::{
    hierarchy::{can_set_parent, despawn_recursive, get_root_entities, remove_parent, set_parent},
    Camera, Children, DirectionalLight, EditorVisibility, EntityGuid, MeshRenderer, Name, Parent,
    PointLight, Transform,
};
use egui::{pos2, Color32, Context, RichText, ScrollArea, SidePanel, Stroke, TextEdit, Ui};
use hecs::{Entity, World};
use smallvec::SmallVec;
use std::collections::HashSet;
use std::sync::Arc;
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
    /// File stem of the SVG to render for this entity (e.g. `"camera"`,
    /// `"sun"`, `"object"`). Resolved against `HierarchyIcons` at draw time.
    /// Easily extended: pick the stem in `entity_icon_stem` and add the SVG.
    icon_stem: &'static str,
    /// Tint applied when the SVG is rendered. White by default — set it
    /// only when an entity kind needs a brand colour.
    icon_tint: Color32,
    /// Editor visibility flag, mirrored into the row so render code can
    /// dim the entry without re-querying. True == shown.
    is_visible: bool,
    /// Per-ancestor (and self) "is this the last sibling at its depth?" flags.
    /// Index `d` covers the chain element at depth `d`; index `depth` is the
    /// row itself. Used to render Godot-style L / T tree connectors:
    ///   - passing column `d` (0 ≤ d < depth-1) draws a full vertical iff
    ///     `!chain_is_last[d + 1]` (the descendant we belong to has more
    ///     siblings still to come).
    ///   - parent column `depth - 1` draws an L when `chain_is_last[depth]`
    ///     is true, a T otherwise.
    chain_is_last: SmallVec<[bool; 16]>,
    /// Full ancestor chain — entities from root down to `self` inclusive.
    /// Length == `depth + 1`. Used to compute which guide columns at this
    /// row are on the path to the selected entity (so the whole path can be
    /// highlighted, not only the selected row's own hook).
    entity_chain: SmallVec<[Entity; 16]>,
}

const ROW_HEIGHT: f32 = 22.0;
/// Horizontal spacing used for both indentation and tree-guide column centers.
const INDENT: f32 = 16.0;
/// Distance from the row's content left edge to column 0's center.
const COL_PAD: f32 = 8.0;
/// Pixel size icons are drawn at inside the row.
const ICON_SIZE: f32 = 16.0;
/// Width reserved on the right edge for the visibility eye column. Keeps the
/// icon away from the panel's right resize edge so clicking it never grabs
/// the dock separator.
const VISIBILITY_COL_WIDTH: f32 = 22.0;

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

        // Selected entity's chain → indices in `flat_rows`. Each chain
        // element is at some `flat_rows` index; we precompute those once so
        // every row can ask "is the trunk between chain[c-1] and chain[c]
        // currently passing through me?" with a constant-time index compare.
        // Empty when nothing is selected.
        let chain_indices: SmallVec<[usize; 16]> = if let Some(primary) =
            selection.primary()
        {
            let chain = self
                .flat_rows
                .iter()
                .find(|r| r.entity == primary)
                .map(|r| r.entity_chain.clone())
                .unwrap_or_default();
            chain
                .iter()
                .map(|&e| {
                    self.flat_rows
                        .iter()
                        .position(|r| r.entity == e)
                        .unwrap_or(usize::MAX)
                })
                .collect()
        } else {
            SmallVec::new()
        };

        ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, ROW_HEIGHT, total_rows, |ui, row_range| {
                if total_rows == 0 {
                    ui.label(RichText::new("No entities in scene").weak());
                    return;
                }

                // Drop inter-row vertical spacing so tree guides drawn at the
                // top of one row meet the lines drawn at the bottom of the
                // previous one with no visible gap.
                ui.spacing_mut().item_spacing.y = 0.0;

                // Resolve the per-context icon set once per render so the
                // virtualized loop doesn't re-fetch on every row. None when
                // we're rendering inside a secondary window whose ctx data
                // doesn't carry the icons — we fall back to text glyphs.
                let icons = ui
                    .data(|d| d.get_temp::<Arc<HierarchyIcons>>(egui::Id::NULL));
                // Also pick up the icon registry so per-state SVG tint
                // overrides edited in the Icon Inspector show up live.
                let registry = ui
                    .data(|d| d.get_temp::<Arc<IconRegistry>>(egui::Id::NULL));

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
                    let icon_stem = self.flat_rows[idx].icon_stem;
                    let icon_tint = self.flat_rows[idx].icon_tint;
                    let is_visible = self.flat_rows[idx].is_visible;
                    // Clone the name + chains to avoid borrow conflicts.
                    let name = self.flat_rows[idx].name.clone();
                    let chain_is_last = self.flat_rows[idx].chain_is_last.clone();
                    let entity_chain = self.flat_rows[idx].entity_chain.clone();

                    // entity_chain isn't needed for highlighting now (we use
                    // index-range checks against chain_indices instead) but
                    // still has to be cloned out so we can release the
                    // immutable borrow of self.flat_rows before render_row
                    // takes &mut self.
                    drop(entity_chain);

                    self.render_row(
                        ui,
                        world,
                        selection,
                        entity,
                        depth,
                        &name,
                        has_children,
                        is_expanded,
                        icon_stem,
                        icon_tint,
                        is_visible,
                        &chain_is_last,
                        idx,
                        &chain_indices,
                        icons.as_deref(),
                        registry.as_deref(),
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
        icon_stem: &str,
        icon_tint: Color32,
        is_visible: bool,
        chain_is_last: &[bool],
        // row_idx: this row's index in flat_rows.
        // chain_indices: indices of the selected entity's chain elements
        // in flat_rows (root → selected). Empty when no selection.
        row_idx: usize,
        chain_indices: &[usize],
        icons: Option<&HierarchyIcons>,
        registry: Option<&IconRegistry>,
        read_only: bool,
    ) {
        let is_selected = selection.is_selected(entity);
        let is_renaming = self.renaming_entity == Some(entity);
        let entity_id = entity.id() as u64;

        let is_valid_drop_target = self
            .drag_source
            .is_some_and(|source| source != entity && can_set_parent(world, source, entity));

        let row_response = ui.horizontal(|ui| {
            Self::draw_tree_guides(
                ui,
                depth,
                chain_is_last,
                is_selected,
                row_idx,
                chain_indices,
            );

            // Layout shifted right by one INDENT so visual column 0 is the
            // root spine (drawn by draw_tree_guides) — every row, even
            // root-level entries with no children, now has a tree marker
            // on its left.
            let indent = (depth as f32 + 1.0) * INDENT;
            ui.add_space(indent);

            if has_children {
                let (rect, response) =
                    ui.allocate_exact_size(egui::vec2(INDENT, INDENT), egui::Sense::click());

                let center = rect.center();
                let sz = 3.5;

                // The chevron's column carries the selection's path only
                // when this row IS one of the selected entity's ancestors
                // (i.e., row's index matches chain_indices[depth]) AND the
                // selection sits deeper still (so the tail under the
                // chevron is actually part of the chain heading down).
                let selected_depth = chain_indices.len().saturating_sub(1);
                let chevron_on_path = !chain_indices.is_empty()
                    && depth < selected_depth
                    && chain_indices.get(depth).copied() == Some(row_idx);

                let chevron_color = if response.hovered() {
                    Color32::WHITE
                } else if chevron_on_path {
                    Color32::from_gray(230)
                } else {
                    Color32::from_gray(190)
                };

                // Caret-style chevron — two stroked segments forming `>`
                // (collapsed) or `v` (expanded), matching the references.
                // Stroked instead of filled so the row guides reading through
                // it stay visible.
                let chevron_stroke = Stroke::new(if chevron_on_path { 1.7 } else { 1.5 }, chevron_color);
                let s = sz;
                let points = if is_expanded {
                    vec![
                        pos2(center.x - s, center.y - s * 0.45),
                        pos2(center.x, center.y + s * 0.55),
                        pos2(center.x + s, center.y - s * 0.45),
                    ]
                } else {
                    vec![
                        pos2(center.x - s * 0.45, center.y - s),
                        pos2(center.x + s * 0.55, center.y),
                        pos2(center.x - s * 0.45, center.y + s),
                    ]
                };
                ui.painter().add(egui::Shape::line(points, chevron_stroke));

                // Tail: when expanded, run a thin vertical from just below
                // the chevron down to the row's bottom edge, at the chevron's
                // centre x. This sits at the same x as the *children's*
                // parent-column hooks, so the tree reads as a single
                // continuous path from this node into its first child.
                //
                // Highlighted variant when this column is on the selection
                // path (the line really *is* the path going to the selected
                // entity below us) — thicker + brighter, just like the L/T.
                if is_expanded {
                    let row_rect = ui.max_rect();
                    let tail_top = center.y + s + 2.0;
                    let tail_bottom = row_rect.bottom() + 1.0; // +1px overlap
                    if tail_bottom > tail_top {
                        let tail_stroke = if chevron_on_path {
                            Stroke::new(1.7, Color32::from_gray(220))
                        } else {
                            Stroke::new(1.0, Color32::from_gray(95))
                        };
                        ui.painter().line_segment(
                            [pos2(center.x, tail_top), pos2(center.x, tail_bottom)],
                            tail_stroke,
                        );
                    }
                }

                if response.clicked() {
                    if is_expanded {
                        self.expanded.remove(&entity_id);
                    } else {
                        self.expanded.insert(entity_id);
                    }
                }
            } else {
                ui.add_space(INDENT);
            }

            // Entity icon: SVG when available, otherwise a faint dot fallback.
            //
            // Resolved tint walks the chain:
            //   1. user-set per-state override in the IconPalette (Icon Inspector)
            //   2. `(hierarchy, stem, Default)` override (icon-wide colour)
            //   3. `icon_tint` argument — the row-builder default (white).
            //
            // Row state mapping: selected → Active, hovered → Hovered, else
            // Default. Disabled / Accent are reserved for future use.
            let row_state = if is_selected {
                ChromeState::Active
            } else {
                ChromeState::Default
            };
            let resolved_tint = registry
                .map(|r| r.panel_icon_tint("hierarchy", icon_stem, row_state, icon_tint))
                .unwrap_or(icon_tint);
            let icon_draw_tint = if is_visible {
                resolved_tint
            } else {
                // Hidden entities are dimmed so the eye toggle's effect reads
                // immediately in the row.
                resolved_tint.gamma_multiply(0.45)
            };
            let texture = icons.and_then(|i| i.get(icon_stem));
            // Per-icon size override from the Icon Inspector (palette).
            // Clamped to the row's height so a too-large value can't blow out
            // the layout — for bigger icons the user can lift ROW_HEIGHT, but
            // rows stay flush by default.
            let effective_icon_size = registry
                .and_then(|r| r.panel_icon_size("hierarchy", icon_stem))
                .unwrap_or(ICON_SIZE)
                .clamp(8.0, ROW_HEIGHT - 2.0);
            let (icon_rect, _) = ui.allocate_exact_size(
                egui::vec2(effective_icon_size, effective_icon_size),
                egui::Sense::hover(),
            );
            if ui.is_rect_visible(icon_rect) {
                if let Some(tex) = texture {
                    ui.painter().image(
                        tex.id(),
                        icon_rect,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        icon_draw_tint,
                    );
                } else {
                    // Tiny dot fallback (used inside secondary windows where the
                    // icon set isn't installed on the local egui context).
                    ui.painter().circle_filled(
                        icon_rect.center(),
                        effective_icon_size * 0.18,
                        icon_draw_tint,
                    );
                }
            }
            ui.add_space(4.0);

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
                let label_color = if !is_visible {
                    Color32::from_gray(120)
                } else if is_selected {
                    Color32::WHITE
                } else {
                    ui.visuals().text_color()
                };
                let label = if is_selected {
                    RichText::new(name).strong().color(label_color)
                } else {
                    RichText::new(name).color(label_color)
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

            // Right-aligned visibility column. Pinned to the row's right edge
            // so future per-row indicators (lock, prefab badge, …) can stack
            // here in the same gutter.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                Self::render_visibility_toggle(
                    ui,
                    world,
                    entity,
                    is_visible,
                    icons,
                    read_only,
                );
            });
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

    /// Draw Godot-style tree guides for a row at `depth`.
    ///
    /// `chain_is_last` is the row's per-depth "is this chain element the last
    /// sibling at its level?" array (length = `depth + 1`, root at index 0,
    /// self at index `depth`). The guide layout is:
    ///
    /// ```text
    ///  passing columns         parent column      icon
    ///  ───┬─                   ┌──                 │
    ///  …  │   …  (vertical     │  ─── ●            ▼
    ///  ───┴─    if !is_last)   └──    (L when last sibling,
    ///                                  T-with-tail otherwise)
    /// ```
    /// Draws Godot-style L/T tree guides for one row.
    ///
    /// Visual column system (one extra column to the left of the OLD layout):
    ///   - column 0: scene-root spine, present at every row so depth-0 rows
    ///     also have a tree marker.
    ///   - columns 1..=depth: passing trunks for ancestors at logical depths
    ///     1..=depth (drawn iff `!chain_is_last[c]`).
    ///   - column depth+1: this row's chevron / icon area.
    ///
    /// Path highlighting walks `chain_indices` (root → selected, indexed in
    /// flat_rows). Each line segment is checked separately so:
    ///   - the horizontal hook only lights up at *ancestor* rows of the
    ///     selection (where row_idx == chain_indices[depth]),
    ///   - vertical segments light up only while the selection trunk is
    ///     genuinely passing through that y-range,
    /// — so selecting Dust no longer brightens Fire's hook just because they
    /// share the Vfx parent.
    fn draw_tree_guides(
        ui: &mut Ui,
        depth: usize,
        chain_is_last: &[bool],
        is_selected: bool,
        row_idx: usize,
        chain_indices: &[usize],
    ) {
        let row_rect = ui.max_rect();
        // Extend by 1 px above/below so adjacent rows' guides always overlap
        // — combined with `item_spacing.y = 0`, no gap can show through at
        // sub-pixel boundaries.
        let row_top = row_rect.top() - 1.0;
        let row_bottom = row_rect.bottom() + 1.0;
        let row_middle = 0.5 * (row_rect.top() + row_rect.bottom());
        let left = row_rect.left();

        let base_stroke = Stroke::new(1.0, Color32::from_gray(95));
        let highlight_stroke = Stroke::new(1.7, Color32::from_gray(220));
        let stroke_for = |on: bool| if on { highlight_stroke } else { base_stroke };

        let selected_depth = chain_indices.len().saturating_sub(1);
        let has_selection = !chain_indices.is_empty();

        // Trunk-segment "on path" tests. The trunk at visual column c
        // connects chain[c-1]'s chevron tail to chain[c]'s parent-column
        // hook (for c == 0, the upper end is the implicit "scene root"
        // above all rows).
        let segment_top_on_path = |c: usize| -> bool {
            if !has_selection || c > selected_depth {
                return false;
            }
            if c == 0 {
                row_idx <= chain_indices[0]
            } else {
                row_idx > chain_indices[c - 1] && row_idx <= chain_indices[c]
            }
        };
        let segment_bottom_on_path = |c: usize| -> bool {
            if !has_selection || c > selected_depth {
                return false;
            }
            if c == 0 {
                row_idx < chain_indices[0]
            } else {
                row_idx >= chain_indices[c - 1] && row_idx < chain_indices[c]
            }
        };

        // 1) Passing visual columns 0 .. depth (excludes the parent column
        //    `depth` and the chevron column `depth+1`). Visual column 0 is
        //    drawn unconditionally so every row gets the spine; columns
        //    >= 1 use !chain_is_last[c] like before.
        for c in 0..depth {
            let drawn = if c == 0 {
                true
            } else {
                !chain_is_last.get(c).copied().unwrap_or(true)
            };
            if !drawn {
                continue;
            }
            let on_path = segment_top_on_path(c) && segment_bottom_on_path(c);
            let x = left + COL_PAD + c as f32 * INDENT;
            ui.painter().line_segment(
                [pos2(x, row_top), pos2(x, row_bottom)],
                stroke_for(on_path),
            );
        }

        // 2) Parent column = visual column `depth` (the L/T for this row).
        let parent_col = depth;
        let x_parent = left + COL_PAD + parent_col as f32 * INDENT;

        // Top-to-middle vertical — always drawn (carries the line down
        // from the parent above into this row).
        ui.painter().line_segment(
            [pos2(x_parent, row_top), pos2(x_parent, row_middle)],
            stroke_for(segment_top_on_path(parent_col)),
        );

        // Horizontal hook to the chevron column's centre. Lit only when
        // *this row itself* is one of the selected entity's ancestors —
        // i.e., row_idx matches the chain entry at this depth.
        let x_hook_end = left + COL_PAD + (parent_col + 1) as f32 * INDENT;
        let hook_on_path = has_selection
            && depth <= selected_depth
            && chain_indices.get(depth).copied() == Some(row_idx);
        ui.painter().line_segment(
            [pos2(x_parent, row_middle), pos2(x_hook_end, row_middle)],
            stroke_for(hook_on_path || (is_selected && hook_on_path)),
        );

        // Middle-to-bottom vertical — only when this row isn't its
        // parent's last child (T-junction). On path iff the trunk
        // continues *past* this row (which it doesn't for the chain
        // endpoint row, so chain[depth] won't light its lower half).
        let self_is_last = chain_is_last
            .get(depth)
            .copied()
            .unwrap_or(true);
        if !self_is_last {
            ui.painter().line_segment(
                [pos2(x_parent, row_middle), pos2(x_parent, row_bottom)],
                stroke_for(segment_bottom_on_path(parent_col)),
            );
        }
    }

    /// Pick which `engine/icons/hierarchy/<stem>.svg` to render for an entity.
    ///
    /// Add a new entity kind by:
    ///   1. Dropping `engine/icons/hierarchy/<your_name>.svg`,
    ///   2. Adding a branch here returning `("your_name", tint)`.
    /// No registry edit needed — the icon set is auto-discovered at startup.
    fn entity_icon_stem(world: &World, entity: Entity) -> (&'static str, Color32) {
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

    /// Draw the eye icon at the row's right edge and toggle
    /// `EditorVisibility` on click. Falls back to a unicode glyph if the SVG
    /// set isn't available in this egui context.
    fn render_visibility_toggle(
        ui: &mut Ui,
        world: &mut World,
        entity: Entity,
        is_visible: bool,
        icons: Option<&HierarchyIcons>,
        read_only: bool,
    ) {
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(VISIBILITY_COL_WIDTH, ROW_HEIGHT - 2.0),
            egui::Sense::click(),
        );

        let tint_alpha = if is_visible { 255 } else { 110 };
        let base_color = if response.hovered() {
            Color32::WHITE
        } else {
            Color32::from_gray(200)
        };
        let tint = Color32::from_rgba_unmultiplied(
            base_color.r(),
            base_color.g(),
            base_color.b(),
            tint_alpha,
        );

        let icon_rect = egui::Rect::from_center_size(
            rect.center(),
            egui::vec2(ICON_SIZE, ICON_SIZE),
        );

        if ui.is_rect_visible(rect) {
            if let Some(tex) = icons.and_then(|i| i.get("visibility")) {
                ui.painter().image(
                    tex.id(),
                    icon_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    tint,
                );
            } else {
                let glyph = if is_visible { "\u{25C9}" } else { "\u{25CB}" };
                ui.painter().text(
                    icon_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    glyph,
                    egui::FontId::proportional(ICON_SIZE),
                    tint,
                );
            }
        }

        if !is_visible {
            // Faint diagonal slash so a hidden state reads even at a glance,
            // matching the "eye crossed out" pattern in the references.
            let slash_color = Color32::from_rgba_unmultiplied(255, 120, 120, 200);
            ui.painter().line_segment(
                [
                    pos2(icon_rect.left() + 1.0, icon_rect.bottom() - 1.0),
                    pos2(icon_rect.right() - 1.0, icon_rect.top() + 1.0),
                ],
                Stroke::new(1.5, slash_color),
            );
        }

        response.clone().on_hover_text(if is_visible {
            "Hide in editor"
        } else {
            "Show in editor"
        });

        if response.clicked() && !read_only {
            let new_visible = !is_visible;
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
                // Drag ghost is text-only (no SVG sampling on the tooltip
                // layer); just show the entity name.
                let ghost_text = name;

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

