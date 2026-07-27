//! Graph editor canvas panel (Task 40, P5).
//!
//! Draws the pan/zoom node canvas — world grid, nodes (header tinted by
//! category), typed pins, bezier wires, selection, and a validation-error
//! overlay — and handles all mouse interaction (drag nodes, connect pins,
//! marquee select, right-click create menu). Keyboard editing (undo/redo,
//! delete, copy/paste/duplicate, save) is handled here only for float windows
//! (`handle_shortcuts`); docked graphs route through the main window's menu /
//! winit path. Colors come from the theme (no raw color literals — M10 grep
//! gate); status colors use the theme's invariant status group, matching
//! `console_crusty`.

use crusty_gui::context::Ui;
use crusty_gui::input::{Key, Modifiers};
use crusty_gui::math::{Color, Pos2, Rect, Vec2};
use crusty_gui::paint::Painter;
use crusty_gui::style::Style;
use crusty_gui::widgets::{Canvas, CanvasScope, TextEdit};

use super::graph_editor::{
    prop_display, ConnectDrag, GraphEdit, GraphEditorState, GraphFragment, NodeDrag,
};
use super::theme::Palette;
use crate::engine::node_graph::{Edge, GraphError, NodeRegistry, PinType, SUBGRAPH_TYPE_ID};

// Node metrics, world-space units (≈ pixels at zoom 1.0).
const NODE_W: f32 = 168.0;
const HEADER_H: f32 = 22.0;
const ROW_H: f32 = 18.0;
const BODY_PAD: f32 = 6.0;
const PIN_R: f32 = 4.5;

/// Everything the panel needs, bundled so the signature stays small.
pub struct GraphEditorPanelCtx<'a> {
    pub state: &'a mut GraphEditorState,
    pub registry: &'a NodeRegistry,
    pub clipboard: &'a mut Option<GraphFragment>,
    /// This tab is the focused tab of its dock (gates keyboard editing).
    pub focused: bool,
    /// True in float windows (no menu/winit edit path) — the panel handles
    /// keyboard editing itself. False when docked in the main window.
    pub handle_shortcuts: bool,
}

// ---------------------------------------------------------------------------
// Geometry
// ---------------------------------------------------------------------------

struct PinGeom {
    slug: String,
    label: String,
    ty: PinType,
    center: Pos2,
    output: bool,
}

struct NodeGeom {
    id: u64,
    rect: Rect,
    header: String,
    category: Option<String>,
    missing: bool,
    pins: Vec<PinGeom>,
}

impl NodeGeom {
    fn pin_center(&self, slug: &str, output: bool) -> Option<Pos2> {
        self.pins
            .iter()
            .find(|p| p.output == output && p.slug == slug)
            .map(|p| p.center)
    }
}

fn build_geoms(state: &GraphEditorState, registry: &NodeRegistry) -> Vec<NodeGeom> {
    state
        .doc
        .nodes
        .iter()
        .map(|n| {
            let min = Pos2::new(n.position[0], n.position[1]);
            let desc = (n.type_id != SUBGRAPH_TYPE_ID)
                .then(|| registry.get(&n.type_id))
                .flatten();
            #[allow(clippy::type_complexity)]
            let (header, category, missing, inputs, outputs): (
                String,
                Option<String>,
                bool,
                Vec<(String, String, PinType)>,
                Vec<(String, String, PinType)>,
            ) = if n.type_id == SUBGRAPH_TYPE_ID {
                let name = n
                    .subgraph
                    .as_deref()
                    .and_then(|p| std::path::Path::new(p).file_stem())
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| "Subgraph".to_string());
                (name, Some("Subgraph".to_string()), false, vec![], vec![])
            } else if let Some(d) = desc {
                let ins = d
                    .inputs
                    .iter()
                    .map(|p| (p.slug.clone(), p.label.clone(), p.ty.clone()))
                    .collect();
                let outs = d
                    .outputs
                    .iter()
                    .map(|p| (p.slug.clone(), p.label.clone(), p.ty.clone()))
                    .collect();
                (d.name.clone(), Some(d.category.clone()), false, ins, outs)
            } else {
                (n.type_id.clone(), None, true, vec![], vec![])
            };

            let rows = inputs.len().max(outputs.len()).max(1);
            let height = HEADER_H + rows as f32 * ROW_H + BODY_PAD;
            let rect = Rect::from_min_size(min, Vec2::new(NODE_W, height));
            let row_y = |i: usize| min.y + HEADER_H + i as f32 * ROW_H + ROW_H * 0.5;

            let mut pins = Vec::new();
            for (i, (slug, label, ty)) in inputs.into_iter().enumerate() {
                pins.push(PinGeom { slug, label, ty, center: Pos2::new(min.x, row_y(i)), output: false });
            }
            for (i, (slug, label, ty)) in outputs.into_iter().enumerate() {
                pins.push(PinGeom {
                    slug,
                    label,
                    ty,
                    center: Pos2::new(min.x + NODE_W, row_y(i)),
                    output: true,
                });
            }
            NodeGeom { id: n.id, rect, header, category, missing, pins }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Colors (theme-only)
// ---------------------------------------------------------------------------

fn pin_color(st: &Style, ty: &PinType) -> Color {
    let tc = Palette::invariant_type_colors();
    match ty {
        PinType::Exec => st.palette.text,
        PinType::Float => tc.physics,
        PinType::Vec2 | PinType::Vec3 | PinType::Vec4 => tc.geometry,
        PinType::Color => tc.materials,
        PinType::Bool => tc.scripting,
        PinType::Enum => tc.animation,
        PinType::Texture => tc.vfx,
        PinType::Mesh => tc.lights,
        PinType::Entity => tc.cameras,
        PinType::Domain(_) => tc.ui,
    }
}

/// Deterministic header tint per category, drawn from the invariant type
/// colors (stable across presets, zero literals).
fn category_color(category: &str) -> Color {
    let tc = Palette::invariant_type_colors();
    let palette = [
        tc.geometry,
        tc.lights,
        tc.cameras,
        tc.vfx,
        tc.audio,
        tc.animation,
        tc.materials,
        tc.scripting,
        tc.physics,
        tc.ui,
    ];
    let h = category
        .bytes()
        .fold(0u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
    palette[(h as usize) % palette.len()]
}

// ---------------------------------------------------------------------------
// Connection validity
// ---------------------------------------------------------------------------

/// If a wire between two pins is legal, return the normalized edge
/// (output → input). No implicit conversions; exec only to exec (type
/// equality covers both); the input side must be currently unconnected.
#[allow(clippy::too_many_arguments)]
fn validate_connection(
    state: &GraphEditorState,
    a_node: u64,
    a_slug: &str,
    a_out: bool,
    a_ty: &PinType,
    b_node: u64,
    b_slug: &str,
    b_out: bool,
    b_ty: &PinType,
) -> Option<Edge> {
    if a_node == b_node || a_out == b_out || a_ty != b_ty {
        return None;
    }
    let (from_node, from_pin, to_node, to_pin) = if a_out {
        (a_node, a_slug, b_node, b_slug)
    } else {
        (b_node, b_slug, a_node, a_slug)
    };
    if state
        .doc
        .edges
        .iter()
        .any(|e| e.to_node == to_node && e.to_pin == to_pin)
    {
        return None;
    }
    Some(Edge {
        from_node,
        from_pin: from_pin.to_string(),
        to_node,
        to_pin: to_pin.to_string(),
    })
}

// ---------------------------------------------------------------------------
// Panel
// ---------------------------------------------------------------------------

pub fn graph_editor_panel(ui: &mut Ui, ctx: GraphEditorPanelCtx) {
    let GraphEditorPanelCtx { state, registry, clipboard, focused, handle_shortcuts } = ctx;

    if handle_shortcuts && focused {
        handle_panel_keys(ui, state, registry, clipboard);
    }

    // Canvas needs `&mut CanvasView`; `CanvasView` is Copy, so pass a local
    // copy and write it back — keeps `state` fully borrowable in the body.
    let mut view = state.view;
    let mut menu_open_at: Option<Pos2> = None;
    let out = Canvas::new().zoom_range(0.25, 2.5).show(ui, &mut view, |ui, scope| {
        draw_and_interact(ui, scope, state, registry, &mut menu_open_at);
    });
    state.view = view;

    create_menu(ui, state, registry, menu_open_at);
    error_overlay(ui, out.rect, &state.errors);
}

fn handle_panel_keys(
    ui: &Ui,
    state: &mut GraphEditorState,
    registry: &NodeRegistry,
    clipboard: &mut Option<GraphFragment>,
) {
    let (ctrl, shift, del, z, y, c, v, d, s) = {
        let input = &ui.ctx().input;
        let ctrl = input.modifiers.contains(Modifiers::CTRL);
        (
            ctrl,
            input.modifiers.contains(Modifiers::SHIFT),
            input.key_pressed(Key::Delete),
            input.key_pressed(Key::Char('z')),
            input.key_pressed(Key::Char('y')),
            input.key_pressed(Key::Char('c')),
            input.key_pressed(Key::Char('v')),
            input.key_pressed(Key::Char('d')),
            input.key_pressed(Key::Char('s')),
        )
    };
    if del {
        state.delete_selection(registry);
    }
    if ctrl && z {
        if shift {
            state.redo(registry);
        } else {
            state.undo(registry);
        }
    }
    if ctrl && y {
        state.redo(registry);
    }
    if ctrl && c {
        state.copy_selection(clipboard);
    }
    if ctrl && v {
        state.paste_clipboard(clipboard, registry);
    }
    if ctrl && d {
        state.duplicate_selection(registry);
    }
    if ctrl && s {
        let abs = std::path::Path::new("content").join(&state.path);
        if let Err(e) = state.save(&abs) {
            log::warn!("graph save failed: {e}");
        }
    }
}

fn draw_and_interact(
    ui: &mut Ui,
    scope: &CanvasScope,
    state: &mut GraphEditorState,
    registry: &NodeRegistry,
    menu_open_at: &mut Option<Pos2>,
) {
    let st = ui.style();
    let zoom = scope.zoom();
    let vis = scope.visible_world_rect();
    let geoms = build_geoms(state, registry);

    // Background + world grid.
    {
        let mut p = ui.painter();
        p.rect_filled(scope.rect(), 0.0, st.palette.input);
        let step = 40.0;
        for i in (vis.min.x / step).floor() as i32..=(vis.max.x / step).ceil() as i32 {
            let wx = i as f32 * step;
            p.line_segment(
                scope.world_to_screen(Pos2::new(wx, vis.min.y)),
                scope.world_to_screen(Pos2::new(wx, vis.max.y)),
                1.0,
                st.palette.stroke,
            );
        }
        for i in (vis.min.y / step).floor() as i32..=(vis.max.y / step).ceil() as i32 {
            let wy = i as f32 * step;
            p.line_segment(
                scope.world_to_screen(Pos2::new(vis.min.x, wy)),
                scope.world_to_screen(Pos2::new(vis.max.x, wy)),
                1.0,
                st.palette.stroke,
            );
        }
    }

    // Existing wires.
    {
        let mut p = ui.painter();
        for e in &state.doc.edges {
            let from = geoms
                .iter()
                .find(|g| g.id == e.from_node)
                .and_then(|g| g.pin_center(&e.from_pin, true));
            let to = geoms
                .iter()
                .find(|g| g.id == e.to_node)
                .and_then(|g| g.pin_center(&e.to_pin, false));
            if let (Some(a), Some(b)) = (from, to) {
                draw_wire(
                    &mut p,
                    scope.world_to_screen(a),
                    scope.world_to_screen(b),
                    zoom,
                    st.palette.accent_active,
                );
            }
        }
    }

    // Nodes + pins.
    let status = Palette::invariant_status();
    for g in &geoms {
        let clip = g.rect.intersect(vis);
        if clip.width() <= 0.0 || clip.height() <= 0.0 {
            continue;
        }
        let srect = scope.world_rect_to_screen(g.rect);
        let selected = state.selection.contains(&g.id);
        let round = 4.0 * zoom;
        let header_col = if g.missing {
            status.error
        } else {
            g.category.as_deref().map(category_color).unwrap_or(st.palette.header)
        };
        let stroke_col = if selected {
            st.palette.selection_fill
        } else if g.missing {
            status.error
        } else {
            st.palette.stroke_strong
        };

        let mut p = ui.painter();
        p.rect_filled(srect, round, st.palette.elevated);
        let header = Rect::from_min_size(srect.min, Vec2::new(srect.width(), HEADER_H * zoom));
        p.rect_filled(header, round, header_col);
        p.rect_stroke(srect, round, if selected { 2.0 } else { 1.0 } * zoom, stroke_col);

        if let Some(px) = scope.label_size(st.fonts.body) {
            p.text(
                srect.min + Vec2::new(6.0 * zoom, (HEADER_H * zoom - px) * 0.5),
                &g.header,
                px,
                st.palette.text,
                None,
            );
        }

        let label_px = scope.label_size(st.fonts.small);
        for pin in &g.pins {
            let c = scope.world_to_screen(pin.center);
            p.circle_filled(c, PIN_R * zoom, pin_color(&st, &pin.ty));
            let Some(px) = label_px else {
                continue;
            };
            if pin.output {
                let w = p.measure_text(&pin.label, px, None).x;
                p.text(
                    Pos2::new(srect.max.x - 8.0 * zoom - w, c.y - px * 0.5),
                    &pin.label,
                    px,
                    st.palette.text_secondary,
                    None,
                );
            } else {
                let connected = state
                    .doc
                    .edges
                    .iter()
                    .any(|e| e.to_node == g.id && e.to_pin == pin.slug);
                let text = if !connected {
                    match state.doc.node(g.id).and_then(|n| n.properties.get(&pin.slug)) {
                        Some(v) => format!("{}: {}", pin.label, prop_display(v)),
                        None => pin.label.clone(),
                    }
                } else {
                    pin.label.clone()
                };
                p.text(
                    Pos2::new(srect.min.x + 8.0 * zoom, c.y - px * 0.5),
                    &text,
                    px,
                    st.palette.text_secondary,
                    None,
                );
            }
        }
    }

    // Interactions.
    let pointer_world = scope.pointer_world(ui);
    let pointer_down = ui.ctx().input.pointer_down;
    let pointer_pressed = ui.ctx().input.pointer_pressed;
    let released = ui.ctx().input.pointer_released;
    let right_pressed = ui.ctx().input.right_pressed;
    let shift = ui.ctx().input.modifiers.contains(Modifiers::SHIFT);

    // Advance / finish a live node drag. Snapshot the drag data first so the
    // `node_drag` borrow ends before mutating the doc.
    let drag_snapshot = state
        .node_drag
        .as_ref()
        .map(|d| (d.origin_world, d.originals.clone()));
    if let Some((origin, originals)) = drag_snapshot {
        if let Some(pw) = pointer_world {
            let (dx, dy) = (pw.x - origin[0], pw.y - origin[1]);
            for (id, start) in &originals {
                if let Some(n) = state.doc.node_mut(*id) {
                    n.position = [start[0] + dx, start[1] + dy];
                }
            }
        }
        if !pointer_down {
            finish_node_drag(state, registry);
        }
    }

    // Pins take precedence over the node body.
    let mut pin_claimed = false;
    for g in &geoms {
        for pin in &g.pins {
            let wr = Rect::from_center_size(pin.center, Vec2::splat(PIN_R * 3.0));
            let id = ui.alloc_id(("graph_pin", g.id, &pin.slug, pin.output));
            let resp = scope.interact(ui, id, wr);
            if resp.pressed && state.connect_drag.is_none() && state.node_drag.is_none() {
                state.connect_drag = Some(ConnectDrag {
                    from_node: g.id,
                    from_pin: pin.slug.clone(),
                    from_output: pin.output,
                });
                pin_claimed = true;
            }
        }
    }

    // Node body: select + start drag.
    let mut begin_drag = false;
    for g in &geoms {
        let id = ui.alloc_id(("graph_node", g.id));
        let resp = scope.interact(ui, id, g.rect);
        if resp.pressed
            && !pin_claimed
            && state.node_drag.is_none()
            && state.connect_drag.is_none()
            && !begin_drag
            && !shift
        {
            if !state.selection.contains(&g.id) {
                state.selection.clear();
                state.selection.insert(g.id);
            }
            begin_drag = true;
        }
        if resp.clicked && shift {
            if state.selection.contains(&g.id) {
                state.selection.remove(&g.id);
            } else {
                state.selection.insert(g.id);
            }
        }
    }
    if begin_drag {
        if let Some(pw) = pointer_world {
            let originals = state
                .selection
                .iter()
                .filter_map(|id| state.doc.node(*id).map(|n| (*id, n.position)))
                .collect();
            state.node_drag = Some(NodeDrag { origin_world: [pw.x, pw.y], originals });
        }
    }

    // Connection drag: live wire tinted by validity, resolved on release.
    // Snapshot first so the `connect_drag` borrow ends before any mutation.
    let connect_snapshot = state
        .connect_drag
        .as_ref()
        .map(|d| (d.from_node, d.from_pin.clone(), d.from_output));
    if let Some((from_node, from_pin, from_output)) = connect_snapshot {
        let src = geoms
            .iter()
            .find(|g| g.id == from_node)
            .and_then(|g| g.pin_center(&from_pin, from_output));
        if let (Some(src), Some(pw)) = (src, pointer_world) {
            let src_ty = pin_ty(&geoms, from_node, &from_pin, from_output);
            let tint = match (pin_under(&geoms, pw), src_ty.as_ref()) {
                (Some(h), Some(sty)) => {
                    if validate_connection(
                        state, from_node, &from_pin, from_output, sty, h.0, &h.1, h.3, &h.2,
                    )
                    .is_some()
                    {
                        status.success
                    } else {
                        status.error
                    }
                }
                _ => st.palette.accent_active,
            };
            let mut p = ui.painter();
            draw_wire(&mut p, scope.world_to_screen(src), scope.world_to_screen(pw), zoom, tint);
        }
        if released {
            resolve_connection(state, &geoms, pointer_world, registry);
            state.connect_drag = None;
        } else if !pointer_down {
            state.connect_drag = None;
        }
    }

    // Marquee box-select on empty canvas.
    handle_marquee(
        ui,
        scope,
        state,
        &geoms,
        pointer_world,
        pointer_pressed,
        pointer_down,
        released,
        pin_claimed,
        &st,
    );

    // Right-click empty space → open the create menu at the pointer.
    if right_pressed {
        if let Some(pw) = pointer_world {
            if pin_under(&geoms, pw).is_none() && node_under(&geoms, pw).is_none() {
                state.create_menu_world = Some([pw.x, pw.y]);
                state.create_menu_search.clear();
                *menu_open_at = ui.ctx().input.pointer_pos;
            }
        }
    }
}

fn pin_ty(geoms: &[NodeGeom], node: u64, slug: &str, output: bool) -> Option<PinType> {
    geoms
        .iter()
        .find(|g| g.id == node)?
        .pins
        .iter()
        .find(|p| p.output == output && p.slug == slug)
        .map(|p| p.ty.clone())
}

fn pin_under(geoms: &[NodeGeom], pw: Pos2) -> Option<(u64, String, PinType, bool)> {
    for g in geoms {
        for pin in &g.pins {
            if Rect::from_center_size(pin.center, Vec2::splat(PIN_R * 3.0)).contains(pw) {
                return Some((g.id, pin.slug.clone(), pin.ty.clone(), pin.output));
            }
        }
    }
    None
}

fn node_under(geoms: &[NodeGeom], pw: Pos2) -> Option<u64> {
    geoms.iter().rev().find(|g| g.rect.contains(pw)).map(|g| g.id)
}

fn resolve_connection(
    state: &mut GraphEditorState,
    geoms: &[NodeGeom],
    pointer_world: Option<Pos2>,
    registry: &NodeRegistry,
) {
    let Some((from_node, from_pin, from_output)) = state
        .connect_drag
        .as_ref()
        .map(|d| (d.from_node, d.from_pin.clone(), d.from_output))
    else {
        return;
    };
    let (Some(pw), Some(src_ty)) =
        (pointer_world, pin_ty(geoms, from_node, &from_pin, from_output))
    else {
        return;
    };
    let Some((tn, ts, tty, to)) = pin_under(geoms, pw) else {
        return;
    };
    if let Some(edge) = validate_connection(
        state, from_node, &from_pin, from_output, &src_ty, tn, &ts, to, &tty,
    ) {
        state.doc.edges.push(edge.clone());
        state.commit(GraphEdit::Connect(edge), registry);
    }
}

fn finish_node_drag(state: &mut GraphEditorState, registry: &NodeRegistry) {
    let Some(drag) = state.node_drag.take() else {
        return;
    };
    let ids: Vec<u64> = drag.originals.iter().map(|(id, _)| *id).collect();
    let delta = drag
        .originals
        .first()
        .and_then(|(id, start)| {
            state
                .doc
                .node(*id)
                .map(|n| [n.position[0] - start[0], n.position[1] - start[1]])
        })
        .unwrap_or([0.0, 0.0]);
    if delta[0].abs() > f32::EPSILON || delta[1].abs() > f32::EPSILON {
        state.commit(GraphEdit::MoveNodes { ids, delta }, registry);
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_marquee(
    ui: &mut Ui,
    scope: &CanvasScope,
    state: &mut GraphEditorState,
    geoms: &[NodeGeom],
    pointer_world: Option<Pos2>,
    pointer_pressed: bool,
    pointer_down: bool,
    released: bool,
    pin_claimed: bool,
    st: &Style,
) {
    if pointer_pressed
        && state.node_drag.is_none()
        && state.connect_drag.is_none()
        && !pin_claimed
    {
        if let Some(pw) = pointer_world {
            if node_under(geoms, pw).is_none() && pin_under(geoms, pw).is_none() {
                state.marquee = Some([pw.x, pw.y]);
            }
        }
    }
    let Some(start) = state.marquee else {
        return;
    };
    let pw = pointer_world.unwrap_or(Pos2::new(start[0], start[1]));
    let world_rect = Rect::from_min_max(
        Pos2::new(start[0].min(pw.x), start[1].min(pw.y)),
        Pos2::new(start[0].max(pw.x), start[1].max(pw.y)),
    );
    {
        let srect = scope.world_rect_to_screen(world_rect);
        let mut p = ui.painter();
        p.rect_filled(srect, 0.0, st.palette.selection_fill.with_alpha(0.18));
        p.rect_stroke(srect, 0.0, 1.0, st.palette.selection_fill);
    }
    if released || !pointer_down {
        state.selection = geoms
            .iter()
            .filter(|g| {
                let i = g.rect.intersect(world_rect);
                i.width() > 0.0 && i.height() > 0.0
            })
            .map(|g| g.id)
            .collect();
        state.marquee = None;
    }
}

fn draw_wire(p: &mut Painter, a: Pos2, b: Pos2, zoom: f32, color: Color) {
    let dx = ((b.x - a.x).abs() * 0.5)
        .max((b.y - a.y).abs() * 0.4)
        .max(24.0 * zoom);
    p.bezier_cubic(a, a + Vec2::new(dx, 0.0), b - Vec2::new(dx, 0.0), b, 2.0 * zoom, color);
}

fn create_menu(
    ui: &mut Ui,
    state: &mut GraphEditorState,
    registry: &NodeRegistry,
    open_at: Option<Pos2>,
) {
    let world = state.create_menu_world;
    let search = &mut state.create_menu_search;
    let mut chosen: Option<String> = None;
    crusty_gui::widgets::context_menu_at(ui, "graph_create_menu", open_at, |ui| {
        ui.menu_group_header("Add Node");
        TextEdit::new(search).hint("Search\u{2026}").width(170.0).show(ui);
        let needle = search.to_lowercase();
        for (cat, descs) in registry.by_category() {
            let rows: Vec<_> = descs
                .iter()
                .filter(|d| {
                    needle.is_empty()
                        || d.name.to_lowercase().contains(&needle)
                        || d.id.to_lowercase().contains(&needle)
                })
                .collect();
            if rows.is_empty() {
                continue;
            }
            ui.menu_group_header(cat);
            for d in rows {
                if ui.menu_item(&d.name) {
                    chosen = Some(d.id.clone());
                }
            }
        }
    });
    if let (Some(type_id), Some(pos)) = (chosen, world) {
        state.add_node(&type_id, pos, registry);
        state.create_menu_world = None;
    }
}

/// Compact validation summary pinned to the canvas's top-left corner.
fn error_overlay(ui: &mut Ui, rect: Rect, errors: &[GraphError]) {
    if errors.is_empty() {
        return;
    }
    let st = ui.style();
    let status = Palette::invariant_status();
    let font = st.fonts.small;
    let pad = 6.0;
    const MAX_LINES: usize = 3;
    let header = format!(
        "{} validation error{}",
        errors.len(),
        if errors.len() == 1 { "" } else { "s" }
    );
    let mut lines: Vec<String> = errors.iter().take(MAX_LINES).map(|e| format!("{e}")).collect();
    if errors.len() > MAX_LINES {
        lines.push(format!("+{} more\u{2026}", errors.len() - MAX_LINES));
    }

    let mut p = ui.painter();
    let mut w = p.measure_text(&header, font, None).x;
    for l in &lines {
        w = w.max(p.measure_text(l, font, None).x);
    }
    let line_h = font * 1.35;
    let box_h = pad * 2.0 + line_h * (lines.len() as f32 + 1.0);
    let box_rect = Rect::from_min_size(
        rect.min + Vec2::new(8.0, 8.0),
        Vec2::new(w + pad * 2.0, box_h),
    );
    p.rect_filled(box_rect, st.rounding.small, st.palette.elevated);
    p.rect_stroke(box_rect, st.rounding.small, 1.0, status.error);
    let mut y = box_rect.min.y + pad;
    p.text(Pos2::new(box_rect.min.x + pad, y), &header, font, status.error, None);
    y += line_h;
    for l in &lines {
        p.text(Pos2::new(box_rect.min.x + pad, y), l, font, st.palette.text_secondary, None);
        y += line_h;
    }
}
