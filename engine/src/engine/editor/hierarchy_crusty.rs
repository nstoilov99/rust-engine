//! Hierarchy panel rendered with crusty-gui.
//!
//! Reads/writes the [`HierarchyPanel`] state (shared row building, drop
//! validation, rename/duplicate/delete helpers live in `hierarchy_panel.rs`).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crusty_gui::context::{Align, Direction, Ui, UiOptions};
use crusty_gui::id::Id;
use crusty_gui::input::{Key, Modifiers, Shortcut};
use crusty_gui::math::{Color, Pos2, Rect, Vec2};
use crusty_gui::paint::{PaintCmd, TextureId};
use crusty_gui::widgets::{Button, Label, ScrollArea, TextEdit};
use hecs::{Entity, World};

use super::hierarchy_panel::{
    DropMode, HierarchyPanel, COL_PAD, ICON_SIZE, INDENT, ROW_HEIGHT, VISIBILITY_COL_WIDTH,
};
use super::icon_classes::ChromeState;
use super::widgets::IconRegistry;
use super::Selection;
use crate::engine::ecs::hierarchy::{can_set_parent, set_parent};
use crate::engine::ecs::resources::PlayMode;
use crate::engine::ecs::{EntityGuid, Name, Transform};

/// dnd payload carried while a hierarchy row is dragged.
struct DragEntity(Entity);

/// Borrowed hierarchy state (shared with `hierarchy_panel.rs`).
pub struct HierarchyPanelCtx<'a> {
    pub panel: &'a mut HierarchyPanel,
    pub world: &'a mut World,
    pub selection: &'a mut Selection,
    pub play_mode: PlayMode,
    /// GPU-uploaded hierarchy icon set keyed by SVG file stem
    /// (`RenderEvent::RenderThreadReady::crusty_icons`).
    pub icons: &'a HashMap<String, TextureId>,
    /// Icon palette (per-state tint / size overrides from the Icon Inspector).
    pub registry: &'a IconRegistry,
}

fn gray(v: u8) -> Color {
    Color::from_srgb_u8(v, v, v, 255)
}

const YELLOW: Color = Color::rgb(1.0, 1.0, 0.0);

/// Draw the hierarchy into the dock tab's content rect (physical pixels).
pub fn hierarchy_panel(ui: &mut Ui, tab_rect: Rect, ctx: HierarchyPanelCtx) {
    let rect = tab_rect;
    // Zero item spacing: the header / separators / search stack is placed
    // at explicit offsets via add_space.
    let opts = UiOptions {
        padding: Vec2::new(0.0, 0.0),
        spacing: 0.0,
    };
    let panel_top = rect.min.y;
    ui.run_at(
        rect,
        Direction::TopDown,
        Id::new("engine_hierarchy_panel"),
        opts,
        |ui| {
            let HierarchyPanelCtx {
                panel,
                world,
                selection,
                play_mode,
                icons,
                registry,
            } = ctx;
            let read_only = play_mode != PlayMode::Edit;

            // Escape: cancel rename / drag, else clear selection (mirrors
            // the panel top-of-frame handling).
            if ui.ctx().input.key_pressed(Key::Escape) {
                if panel.renaming_entity.is_some() {
                    panel.renaming_entity = None;
                } else if ui.ctx().dnd.holds::<DragEntity>() {
                    ui.ctx_mut().dnd.clear();
                } else if !selection.is_empty() {
                    selection.clear();
                }
            }

            // Vertical rhythm: separator lines at
            // +29 / +65 from the panel top, search field top at +38, first
            // tree row at +73.
            render_header(ui, panel, world, read_only);
            ui.add_space((panel_top + 29.0 - ui.cursor().y).max(0.0));
            ui.separator();
            ui.add_space((panel_top + 38.0 - ui.cursor().y).max(0.0));
            render_search(ui, panel);
            ui.add_space((panel_top + 65.0 - ui.cursor().y).max(0.0));
            ui.separator();
            ui.add_space((panel_top + 72.0 - ui.cursor().y).max(0.0));
            render_tree(ui, panel, world, selection, icons, registry, read_only);
        },
    );
}

fn render_header(ui: &mut Ui, panel: &mut HierarchyPanel, world: &mut World, read_only: bool) {
    let row_top = ui.cursor();
    ui.horizontal_aligned(Align::Max, |ui| {
        // Dark chip (fill 14,14,17, 20px square).
        let resp = Button::new("+")
            .fill(Color::from_srgb_u8(14, 14, 17, 255))
            // 23x22 pins the rect to the historical button apparent size
            // (a centered 1px stroke adds a row/col on each side).
            .exact_size(Vec2::new(23.0, 22.0))
            .show(ui);
        if resp.hovered {
            ui.tooltip_for(resp.rect, "Add Entity");
        }
        if resp.clicked && !read_only {
            panel.create_empty_entity(world);
        }
    });
    if read_only {
        let style = ui.style();
        let dim = style.palette.text_dim;
        let size = style.fonts.small;
        ui.painter().text(
            Pos2::new(row_top.x, row_top.y + 6.0),
            "(Playing)",
            size,
            dim,
            None,
        );
    }
}

fn render_search(ui: &mut Ui, panel: &mut HierarchyPanel) {
    ui.horizontal(|ui| {
        let dim = ui.style().palette.text_dim;
        Label::new("Search:").color(dim).show(ui);
        ui.add_space(8.0);
        let w = ui.available_size().x;
        let out = TextEdit::new(&mut panel.search_text)
            .width(w)
            .height(18.0)
            .fill(Color::from_srgb_u8(14, 14, 17, 255))
            .hint("Filter entities...")
            .show_full(ui);
        panel.filter_active = !panel.search_text.is_empty();
        if out.cancelled {
            panel.search_text.clear();
            panel.filter_active = false;
        }
    });
}

fn render_tree(
    ui: &mut Ui,
    panel: &mut HierarchyPanel,
    world: &mut World,
    selection: &mut Selection,
    icons: &HashMap<String, TextureId>,
    registry: &IconRegistry,
    read_only: bool,
) {
    panel.sync_root_order(world);
    panel.build_visible_rows(world);
    let total_rows = panel.flat_rows.len();

    // Selected entity's ancestor chain → indices into `flat_rows`, so tree
    // guides can be range-tested per row (same scheme as the historical panel).
    let chain_indices: Vec<usize> = if let Some(primary) = selection.primary() {
        let chain = panel
            .flat_rows
            .iter()
            .find(|r| r.entity == primary)
            .map(|r| r.entity_chain.clone())
            .unwrap_or_default();
        chain
            .iter()
            .map(|&e| {
                panel
                    .flat_rows
                    .iter()
                    .position(|r| r.entity == e)
                    .unwrap_or(usize::MAX)
            })
            .collect()
    } else {
        Vec::new()
    };

    let avail_h = ui.available_size().y;
    ScrollArea::new(avail_h)
        .auto_shrink(false)
        .inset(0.0)
        // Rows are contiguous so consecutive tree-guide segments meet with
        // no visible gap (item_spacing.y = 0 for the same reason).
        .spacing(0.0)
        .show(ui, |ui| {
            if total_rows == 0 {
                let dim = ui.style().palette.text_dim;
                Label::new("No entities in scene").color(dim).show(ui);
                return;
            }
            for idx in 0..panel.flat_rows.len() {
                let row = &panel.flat_rows[idx];
                let entity = row.entity;
                let depth = row.depth;
                let has_children = row.has_children;
                let is_expanded = row.is_expanded;
                let icon_stem = row.icon_stem;
                let icon_tint = row.icon_tint;
                let is_visible = row.is_visible;
                let name = row.name.clone();
                render_row(
                    ui,
                    panel,
                    world,
                    selection,
                    icons,
                    registry,
                    read_only,
                    idx,
                    &chain_indices,
                    RowData {
                        entity,
                        depth,
                        name,
                        has_children,
                        is_expanded,
                        icon_stem,
                        icon_tint,
                        is_visible,
                    },
                );
            }
        });

    handle_keyboard_shortcuts(ui, panel, world, selection, read_only);
}

struct RowData {
    entity: Entity,
    depth: usize,
    name: String,
    has_children: bool,
    is_expanded: bool,
    icon_stem: &'static str,
    icon_tint: Color,
    is_visible: bool,
}

#[allow(clippy::too_many_arguments)]
fn render_row(
    ui: &mut Ui,
    panel: &mut HierarchyPanel,
    world: &mut World,
    selection: &mut Selection,
    icons: &HashMap<String, TextureId>,
    registry: &IconRegistry,
    read_only: bool,
    row_idx: usize,
    chain_indices: &[usize],
    row: RowData,
) {
    let RowData {
        entity,
        depth,
        name,
        has_children,
        is_expanded,
        icon_stem,
        icon_tint,
        is_visible,
    } = row;
    let entity_id = entity.id() as u64;
    let is_selected = selection.is_selected(entity);
    let is_renaming = panel.renaming_entity == Some(entity);

    let width = ui.available().width();
    let row_rect = ui.allocate(Vec2::new(width, ROW_HEIGHT));

    // Cull rows outside the scroll viewport: the allocation above still
    // advances the cursor (so scroll extent stays correct) but paint and
    // interaction are skipped.
    let clip = ui.clip_rect();
    if row_rect.max.y < clip.min.y || row_rect.min.y > clip.max.y {
        return;
    }

    let row_id = Id::new("hierarchy_row").with(entity_id);
    let row_resp = ui.interact(row_id, row_rect);
    let dragging = ui.ctx().dnd.is_dragging();

    // Selection / hover backgrounds — painted first so they sit behind the
    // row's guides, icons and text.
    if is_selected {
        // Subtle accent-tinted fill; full-strength orange lives only in the
        // left indicator bar (Unreal-style) so row text stays readable.
        ui.painter()
            .rect_filled(row_rect, 2.0, Color::from_srgb_u8(196, 110, 36, 80));
        let bar = Rect::from_min_max(
            row_rect.min,
            Pos2::new(row_rect.min.x + 3.0, row_rect.max.y),
        );
        ui.painter()
            .rect_filled(bar, 1.0, Color::from_srgb_u8(245, 143, 46, 255));
    } else if row_resp.hovered && !dragging && !is_renaming {
        ui.painter()
            .rect_filled(row_rect, 2.0, Color::from_srgb_u8(255, 255, 255, 24));
    }

    // Whole-row drag source, same id as the row interact so a completed
    // drag can't double as a click. Chevron / eye presses re-claim the
    // active widget below, which disarms this source.
    if !read_only && !is_renaming {
        ui.dnd_drag_source(row_id, row_rect, name.clone(), || DragEntity(entity));
    }

    draw_tree_guides(ui, row_rect, depth, row_idx, chain_indices);

    let left = row_rect.min.x;
    let indent = depth as f32 * INDENT;
    let center_y = 0.5 * (row_rect.min.y + row_rect.max.y);

    // Chevron column — always reserved so icons align across rows.
    let chevron_rect = Rect::from_min_size(
        Pos2::new(left + indent, center_y - INDENT * 0.5),
        Vec2::splat(INDENT),
    );
    if has_children {
        let ch_resp = ui.interact(row_id.with("chevron"), chevron_rect);
        let center = chevron_rect.center();
        let s = 3.5;
        let color = if ch_resp.hovered {
            Color::WHITE
        } else {
            gray(190)
        };
        let (a, m, b) = if is_expanded {
            (
                Pos2::new(center.x - s, center.y - s * 0.45),
                Pos2::new(center.x, center.y + s * 0.55),
                Pos2::new(center.x + s, center.y - s * 0.45),
            )
        } else {
            (
                Pos2::new(center.x - s * 0.45, center.y - s),
                Pos2::new(center.x + s * 0.55, center.y),
                Pos2::new(center.x - s * 0.45, center.y + s),
            )
        };
        ui.painter().line_segment(a, m, 1.5, color);
        ui.painter().line_segment(m, b, 1.5, color);

        // Tail under the chevron: only when this row is an ancestor of the
        // selection and the trunk continues below.
        let selected_depth = chain_indices.len().saturating_sub(1);
        let chevron_on_path = !chain_indices.is_empty()
            && depth < selected_depth
            && chain_indices.get(depth).copied() == Some(row_idx);
        if is_expanded && chevron_on_path {
            let tail_top = center.y + s + 2.0;
            let tail_bottom = row_rect.max.y + 1.0;
            if tail_bottom > tail_top {
                ui.painter().line_segment(
                    Pos2::new(center.x, tail_top),
                    Pos2::new(center.x, tail_bottom),
                    1.0,
                    gray(180),
                );
            }
        }

        if ch_resp.clicked {
            if is_expanded {
                panel.expanded.remove(&entity_id);
            } else {
                panel.expanded.insert(entity_id);
            }
        }
    }

    // Entity icon: SVG texture when uploaded, faint dot fallback otherwise.
    // x layout: chevron box + 8px item spacing.
    // Tint / size resolve through the IconRegistry (same as the icon
    // panel (per-state palette override → Default override → row default),
    // with hidden entities dimmed via the same gamma factor.
    let row_state = if is_selected {
        ChromeState::Active
    } else {
        ChromeState::Default
    };
    let resolved_tint = registry.panel_icon_tint("hierarchy", icon_stem, row_state, icon_tint);
    // Hidden rows dim by an alpha multiplier (mirrors the historical
    // `Color32::gamma_multiply(0.45)` behaviour closely enough for tint use).
    let icon_draw_tint = if is_visible {
        resolved_tint
    } else {
        resolved_tint.with_alpha(resolved_tint.a * 0.45)
    };
    let icon_size = registry
        .panel_icon_size("hierarchy", icon_stem)
        .unwrap_or(ICON_SIZE)
        .clamp(8.0, ROW_HEIGHT - 2.0);
    let icon_x = left + indent + INDENT + 8.0;
    let icon_rect = Rect::from_min_size(
        Pos2::new(icon_x, center_y - icon_size * 0.5),
        Vec2::splat(icon_size),
    );
    if let Some(&tex) = icons.get(icon_stem) {
        ui.ctx_mut().paint.push(PaintCmd::Image {
            rect: icon_rect,
            uv_min: Pos2::new(0.0, 0.0),
            uv_max: Pos2::new(1.0, 1.0),
            tint: icon_draw_tint,
            texture: tex,
        });
    } else {
        ui.painter()
            .circle_filled(icon_rect.center(), icon_size * 0.18, icon_draw_tint);
    }

    // Label / rename editor. Label x layout: icon + 8px spacing + 4px.
    let label_x = icon_rect.max.x + 12.0;
    if is_renaming {
        let edit_right = (label_x + 108.0).min(row_rect.max.x - VISIBILITY_COL_WIDTH);
        let edit_rect = Rect::from_min_max(
            Pos2::new(label_x, row_rect.min.y),
            Pos2::new(edit_right, row_rect.max.y),
        );
        let request_focus = std::mem::take(&mut panel.rename_needs_focus);
        let (out, _) = ui.run_at(
            edit_rect,
            Direction::TopDown,
            row_id.with("rename"),
            UiOptions {
                padding: Vec2::ZERO,
                spacing: 0.0,
            },
            |ui| {
                TextEdit::new(&mut panel.rename_buffer)
                    .width(100.0)
                    .request_focus(request_focus)
                    .show_full(ui)
            },
        );
        if out.submitted {
            panel.commit_rename(world, entity);
        } else if out.cancelled {
            panel.renaming_entity = None;
        } else if !out.focused && !request_focus {
            panel.commit_rename(world, entity);
        }
    } else {
        let style = ui.style();
        // Default row text is the theme's secondary color (theme's default
        // label color), not the bright primary.
        let color = if !is_visible {
            gray(120)
        } else if is_selected {
            Color::WHITE
        } else {
            style.palette.text_dim
        };
        let size = style.fonts.body;
        let text_h = ui.painter().measure_text(&name, size, None).y;
        ui.painter().text(
            Pos2::new(label_x, center_y - text_h * 0.5),
            &name,
            size,
            color,
            None,
        );
    }

    render_visibility_toggle(
        ui, world, entity, row_id, row_rect, is_visible, icons, read_only,
    );

    // ─── row-wide interactions ────────────────────────────────────────
    if !is_renaming && row_resp.clicked {
        if ui.ctx().input.modifiers.contains(Modifiers::CTRL) {
            selection.toggle(entity);
        } else {
            selection.select(entity);
        }
    }
    if !read_only {
        if row_resp.double_clicked(ui) {
            panel.start_rename(world, entity);
        }
        ui.context_menu_for(("hierarchy_ctx", entity_id), row_rect, |ui| {
            if ui.menu_item("Add Child") {
                let child = world.spawn((
                    Transform::default(),
                    Name::new("New Child"),
                    EntityGuid::new(),
                ));
                set_parent(world, child, entity);
                panel.expanded.insert(entity_id);
            }
            if ui.menu_item("Rename") {
                panel.start_rename(world, entity);
            }
            if ui.menu_item("Duplicate") {
                panel.duplicate_entity(world, entity);
            }
            ui.separator();
            if ui.menu_item("Delete") {
                panel.delete_entity(world, selection, entity);
            }
        });
        handle_drag_drop(
            ui,
            panel,
            world,
            entity,
            entity_id,
            row_rect,
            has_children,
            is_expanded,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_drag_drop(
    ui: &mut Ui,
    panel: &mut HierarchyPanel,
    world: &mut World,
    entity: Entity,
    entity_id: u64,
    row_rect: Rect,
    has_children: bool,
    is_expanded: bool,
) {
    let source = ui.ctx().dnd.peek::<DragEntity>().map(|d| d.0);
    let Some(source) = source else {
        return;
    };
    if source == entity {
        return;
    }

    let is_hovered = ui.contains_pointer(row_rect);
    if is_hovered {
        let mouse_y = ui.ctx().input.pointer_pos.map(|p| p.y).unwrap_or(0.0);
        let mode = panel.calculate_drop_mode(mouse_y, row_rect.min.y, row_rect.max.y);
        let is_valid = panel.is_valid_drop(world, source, entity, mode);

        if is_valid {
            match mode {
                DropMode::InsertAbove => draw_insertion_line(ui, row_rect.min.y, row_rect),
                DropMode::InsertBelow => draw_insertion_line(ui, row_rect.max.y, row_rect),
                DropMode::MakeChild => {
                    let green = Color::from_srgb_u8(100, 200, 100, 255);
                    ui.painter()
                        .rect_filled(row_rect, 2.0, Color::from_srgb_u8(100, 200, 100, 50));
                    ui.painter().rect_stroke(row_rect, 2.0, 2.0, green);
                    let plus_size = 14.0;
                    let sz = ui.painter().measure_text("+", plus_size, None);
                    let center = Pos2::new(row_rect.max.x - 16.0, row_rect.center().y);
                    ui.painter().text(
                        Pos2::new(center.x - sz.x * 0.5, center.y - sz.y * 0.5),
                        "+",
                        plus_size,
                        green,
                        None,
                    );
                }
            }
            if let Some(DragEntity(src)) = ui.dnd_drop_target::<DragEntity>(row_rect) {
                panel.perform_drop(world, src, entity, mode);
            }
        } else {
            ui.painter()
                .rect_stroke(row_rect, 2.0, 2.0, Color::from_srgb_u8(200, 60, 60, 255));
        }

        // Valid-drop tint over everything, across the full row.
        if source != entity && can_set_parent(world, source, entity) {
            ui.painter()
                .rect_filled(row_rect, 2.0, Color::from_srgb_u8(255, 200, 0, 60));

            // Auto-expand a collapsed branch after hovering it for 500ms.
            if has_children && !is_expanded {
                if panel.drag_hover_entity != Some(entity) {
                    panel.drag_hover_entity = Some(entity);
                    panel.drag_hover_start = Some(Instant::now());
                } else if let Some(start) = panel.drag_hover_start {
                    if start.elapsed() > Duration::from_millis(500) {
                        panel.expanded.insert(entity_id);
                        panel.drag_hover_entity = None;
                        panel.drag_hover_start = None;
                    }
                }
            }
        }
    } else if panel.drag_hover_entity == Some(entity) {
        panel.drag_hover_entity = None;
        panel.drag_hover_start = None;
    }
}

fn draw_insertion_line(ui: &mut Ui, line_y: f32, rect: Rect) {
    ui.painter().line_segment(
        Pos2::new(rect.min.x, line_y),
        Pos2::new(rect.max.x, line_y),
        2.0,
        YELLOW,
    );
    let arrow_left = rect.min.x - 8.0;
    let arrow_size = 5.0;
    ui.painter().convex_polygon_filled(
        vec![
            Pos2::new(arrow_left, line_y - arrow_size),
            Pos2::new(arrow_left + arrow_size * 1.5, line_y),
            Pos2::new(arrow_left, line_y + arrow_size),
        ],
        YELLOW,
    );
}

#[allow(clippy::too_many_arguments)]
fn render_visibility_toggle(
    ui: &mut Ui,
    world: &mut World,
    entity: Entity,
    row_id: Id,
    row_rect: Rect,
    is_visible: bool,
    icons: &HashMap<String, TextureId>,
    read_only: bool,
) {
    let center_y = 0.5 * (row_rect.min.y + row_rect.max.y);
    let eye_rect = Rect::from_center_size(
        Pos2::new(row_rect.max.x - VISIBILITY_COL_WIDTH * 0.5, center_y),
        Vec2::new(VISIBILITY_COL_WIDTH, ROW_HEIGHT - 2.0),
    );
    let resp = ui.interact(row_id.with("eye"), eye_rect);

    let tint_alpha: u8 = if is_visible { 255 } else { 110 };
    let base: u8 = if resp.hovered { 255 } else { 200 };
    let tint = Color::from_srgb_u8(base, base, base, tint_alpha);

    let icon_rect = Rect::from_center_size(eye_rect.center(), Vec2::splat(ICON_SIZE));
    if let Some(&tex) = icons.get("visibility") {
        ui.ctx_mut().paint.push(PaintCmd::Image {
            rect: icon_rect,
            uv_min: Pos2::new(0.0, 0.0),
            uv_max: Pos2::new(1.0, 1.0),
            tint,
            texture: tex,
        });
    } else {
        let glyph = if is_visible { "\u{25C9}" } else { "\u{25CB}" };
        let sz = ui.painter().measure_text(glyph, ICON_SIZE, None);
        let c = icon_rect.center();
        ui.painter().text(
            Pos2::new(c.x - sz.x * 0.5, c.y - sz.y * 0.5),
            glyph,
            ICON_SIZE,
            tint,
            None,
        );
    }

    if !is_visible {
        // Faint diagonal slash so a hidden state reads at a glance.
        ui.painter().line_segment(
            Pos2::new(icon_rect.min.x + 1.0, icon_rect.max.y - 1.0),
            Pos2::new(icon_rect.max.x - 1.0, icon_rect.min.y + 1.0),
            1.5,
            Color::from_srgb_u8(255, 120, 120, 200),
        );
    }

    if resp.hovered {
        ui.tooltip_for(
            eye_rect,
            if is_visible {
                "Hide in editor"
            } else {
                "Show in editor"
            },
        );
    }

    if resp.clicked && !read_only {
        HierarchyPanel::set_visibility(world, entity, !is_visible);
    }
}

/// Draw the Godot-style guide path leading to the selected entity — a port
/// of `HierarchyPanel::draw_tree_guides` (see that function for the trunk
/// index-range scheme).
fn draw_tree_guides(
    ui: &mut Ui,
    row_rect: Rect,
    depth: usize,
    row_idx: usize,
    chain_indices: &[usize],
) {
    if depth == 0 || chain_indices.is_empty() {
        return;
    }

    let row_top = row_rect.min.y - 1.0;
    let row_bottom = row_rect.max.y + 1.0;
    let row_middle = 0.5 * (row_rect.min.y + row_rect.max.y);
    let left = row_rect.min.x;

    let stroke_color = gray(180);
    let selected_depth = chain_indices.len().saturating_sub(1);

    let trunk_top = |c: usize| -> bool {
        c + 1 <= selected_depth && chain_indices[c] < row_idx && row_idx <= chain_indices[c + 1]
    };
    let trunk_bottom = |c: usize| -> bool {
        c + 1 <= selected_depth && chain_indices[c] <= row_idx && row_idx < chain_indices[c + 1]
    };

    for c in 0..depth.saturating_sub(1) {
        if trunk_top(c) && trunk_bottom(c) {
            let x = left + COL_PAD + c as f32 * INDENT;
            ui.painter().line_segment(
                Pos2::new(x, row_top),
                Pos2::new(x, row_bottom),
                1.0,
                stroke_color,
            );
        }
    }

    let parent_col = depth - 1;
    let x_parent = left + COL_PAD + parent_col as f32 * INDENT;
    let x_hook_end = left + depth as f32 * INDENT;

    if trunk_top(parent_col) {
        ui.painter().line_segment(
            Pos2::new(x_parent, row_top),
            Pos2::new(x_parent, row_middle),
            1.0,
            stroke_color,
        );
    }

    let hook_lit = depth <= selected_depth && chain_indices.get(depth).copied() == Some(row_idx);
    if hook_lit {
        ui.painter().line_segment(
            Pos2::new(x_parent, row_middle),
            Pos2::new(x_hook_end, row_middle),
            1.0,
            stroke_color,
        );
    }

    if trunk_bottom(parent_col) {
        ui.painter().line_segment(
            Pos2::new(x_parent, row_middle),
            Pos2::new(x_parent, row_bottom),
            1.0,
            stroke_color,
        );
    }
}

fn handle_keyboard_shortcuts(
    ui: &mut Ui,
    panel: &mut HierarchyPanel,
    world: &mut World,
    selection: &mut Selection,
    read_only: bool,
) {
    if read_only {
        return;
    }
    // Don't fire while any text field (search / rename / console) has focus.
    if ui.ctx().memory.focused_widget.is_some() {
        return;
    }
    if ui.ctx().input.key_pressed(Key::Delete) {
        if let Some(entity) = selection.primary() {
            panel.delete_entity(world, selection, entity);
        }
    }
    if ui.ctx().input.key_pressed(Key::F(2)) {
        if let Some(entity) = selection.primary() {
            panel.start_rename(world, entity);
        }
    }
    if ui
        .ctx_mut()
        .input
        .consume_shortcut(Shortcut::ctrl(Key::Char('d')))
    {
        if let Some(entity) = selection.primary() {
            panel.duplicate_entity(world, entity);
        }
    }
}
