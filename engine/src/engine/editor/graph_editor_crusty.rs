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

use crusty_gui::context::{Direction, Ui, UiOptions};
use crusty_gui::id::Id;
use crusty_gui::input::{Key, Modifiers};
use crusty_gui::math::{Color, Pos2, Rect, Vec2};
use crusty_gui::paint::Painter;
use crusty_gui::style::Style;
use crusty_gui::widgets::{Canvas, CanvasScope, TextEdit};

use super::graph_editor::{
    nodes_captured_by_rect, prop_display, AnnotationDrag, AnnotationEdit, ConnectDrag, GraphEdit,
    GraphEditorState, GraphFragment, NodeDrag,
};
use super::theme::Palette;
use crate::engine::node_graph::{
    Edge, GraphError, GraphResolver, NodeRegistry, PinType, SUBGRAPH_TYPE_ID,
};

// Node metrics, world-space units (≈ pixels at zoom 1.0).
const NODE_W: f32 = 168.0;
const HEADER_H: f32 = 22.0;
const ROW_H: f32 = 18.0;
const BODY_PAD: f32 = 6.0;
const PIN_R: f32 = 4.5;
// Annotation + minimap metrics (P7).
const COMMENT_HEADER_H: f32 = 18.0;
const GROUP_TITLE_H: f32 = 20.0;
const MINIMAP_W: f32 = 180.0;
const MINIMAP_H: f32 = 120.0;
const MINIMAP_MARGIN: f32 = 10.0;
const MINIMAP_BTN: f32 = 24.0;

/// Everything the panel needs, bundled so the signature stays small.
pub struct GraphEditorPanelCtx<'a> {
    pub state: &'a mut GraphEditorState,
    pub registry: &'a NodeRegistry,
    pub clipboard: &'a mut Option<GraphFragment>,
    /// Resolves subgraph references (open docs + disk) for pin derivation.
    pub resolver: &'a dyn GraphResolver,
    /// Content-relative paths of known `.subgraph` assets (create menu).
    pub subgraph_assets: &'a [String],
    /// Set to a content-relative path when a subgraph node is double-clicked;
    /// the host opens it as a tab (P6 open-in-tab navigation).
    pub open_subgraph: &'a mut Option<String>,
    /// Canvas zoom limits (EditorPrefs, P9).
    pub zoom_min: f32,
    pub zoom_max: f32,
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

fn build_geoms(
    state: &GraphEditorState,
    registry: &NodeRegistry,
    resolver: &dyn GraphResolver,
) -> Vec<NodeGeom> {
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
                // Pins derive from the referenced doc's declared interface;
                // an unresolvable reference renders in the missing-node style.
                match n.subgraph.as_deref().and_then(|p| resolver.resolve(p)) {
                    Some(sub) => {
                        let iface = |pins: &[crate::engine::node_graph::IfacePin]| {
                            pins.iter()
                                .map(|p| (p.slug.clone(), p.label.clone(), p.ty.clone()))
                                .collect::<Vec<_>>()
                        };
                        (name, Some("Subgraph".to_string()), false, iface(&sub.inputs), iface(&sub.outputs))
                    }
                    None => (name, Some("Subgraph".to_string()), true, vec![], vec![]),
                }
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
    let GraphEditorPanelCtx {
        state,
        registry,
        clipboard,
        resolver,
        subgraph_assets,
        open_subgraph,
        zoom_min,
        zoom_max,
        focused,
        handle_shortcuts,
    } = ctx;

    if handle_shortcuts && focused {
        handle_panel_keys(ui, state, registry, clipboard);
    }

    // Finalize a drag orphaned by a release that landed while this tab was not
    // being drawn (e.g. the user switched tabs mid-drag): the pointer is
    // already up on re-entry, so `draw_and_interact`'s in-body finish never
    // ran. Finalizing (vs reverting) keeps the node/annotation where the user
    // dragged it — the simpler correct choice. No-op when nothing is dragging.
    if !ui.ctx().input.pointer_down {
        finish_node_drag(state, registry);
        finish_annotation_drag(state, registry);
    }

    // Canvas needs `&mut CanvasView`; `CanvasView` is Copy, so pass a local
    // copy and write it back — keeps `state` fully borrowable in the body.
    let mut view = state.view;
    let mut menu_open_at: Option<Pos2> = None;
    let mut minimap_pan: Option<Vec2> = None;
    let out = Canvas::new().zoom_range(zoom_min, zoom_max).show(ui, &mut view, |ui, scope| {
        draw_and_interact(
            ui,
            scope,
            state,
            registry,
            resolver,
            &mut menu_open_at,
            open_subgraph,
            &mut minimap_pan,
        );
    });
    // Minimap click re-centers the view (applied after the canvas ran).
    if let Some(pan) = minimap_pan {
        view.pan = pan;
    }
    state.view = view;

    create_menu(ui, state, registry, subgraph_assets, menu_open_at);
    edit_popup(ui, state, registry, out.rect);
    error_overlay(ui, out.rect, &state.errors, &state.ref_errors);
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

#[allow(clippy::too_many_arguments)]
fn draw_and_interact(
    ui: &mut Ui,
    scope: &CanvasScope,
    state: &mut GraphEditorState,
    registry: &NodeRegistry,
    resolver: &dyn GraphResolver,
    menu_open_at: &mut Option<Pos2>,
    open_subgraph: &mut Option<String>,
    minimap_pan: &mut Option<Vec2>,
) {
    let st = ui.style();
    let zoom = scope.zoom();
    let vis = scope.visible_world_rect();
    let geoms = build_geoms(state, registry, resolver);

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

    // Group frames + comment boxes (behind nodes/wires) — P7.
    {
        let mut p = ui.painter();
        for (i, g) in state.doc.groups.iter().enumerate() {
            let wr = Rect::from_min_size(Pos2::new(g.rect[0], g.rect[1]), Vec2::new(g.rect[2], g.rect[3]));
            if wr.intersect(vis).width() <= 0.0 || wr.intersect(vis).height() <= 0.0 {
                continue;
            }
            let sr = scope.world_rect_to_screen(wr);
            let sel = state.sel_group == Some(i);
            let round = 4.0 * zoom;
            p.rect_filled(sr, round, st.palette.panel.with_alpha(0.12));
            p.rect_stroke(
                sr,
                round,
                if sel { 2.0 } else { 1.0 } * zoom,
                if sel { st.palette.selection_fill } else { st.palette.stroke_strong },
            );
            let bar = Rect::from_min_size(sr.min, Vec2::new(sr.width(), GROUP_TITLE_H * zoom));
            p.rect_filled(bar, round, st.palette.header);
            if let Some(px) = scope.label_size(st.fonts.body) {
                p.text(
                    sr.min + Vec2::new(6.0 * zoom, (GROUP_TITLE_H * zoom - px) * 0.5),
                    &g.title,
                    px,
                    st.palette.text,
                    None,
                );
            }
        }
        for (i, c) in state.doc.comments.iter().enumerate() {
            let wr = Rect::from_min_size(Pos2::new(c.rect[0], c.rect[1]), Vec2::new(c.rect[2], c.rect[3]));
            if wr.intersect(vis).width() <= 0.0 || wr.intersect(vis).height() <= 0.0 {
                continue;
            }
            let sr = scope.world_rect_to_screen(wr);
            let sel = state.sel_comment == Some(i);
            let round = 4.0 * zoom;
            p.rect_filled(sr, round, st.palette.elevated.with_alpha(0.35));
            p.rect_stroke(
                sr,
                round,
                if sel { 2.0 } else { 1.0 } * zoom,
                if sel { st.palette.selection_fill } else { st.palette.stroke },
            );
            let header = Rect::from_min_size(sr.min, Vec2::new(sr.width(), COMMENT_HEADER_H * zoom));
            p.rect_filled(header, round, st.palette.header.with_alpha(0.6));
            if let Some(px) = scope.label_size(st.fonts.small) {
                p.text(
                    sr.min + Vec2::new(6.0 * zoom, COMMENT_HEADER_H * zoom + 2.0 * zoom),
                    &c.text,
                    px,
                    st.palette.text_secondary,
                    Some((sr.width() - 12.0 * zoom).max(1.0)),
                );
            }
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
    let pointer_pos = ui.ctx().input.pointer_pos;
    let pointer_down = ui.ctx().input.pointer_down;
    let pointer_pressed = ui.ctx().input.pointer_pressed;
    let released = ui.ctx().input.pointer_released;
    let right_pressed = ui.ctx().input.right_pressed;
    let shift = ui.ctx().input.modifiers.contains(Modifiers::SHIFT);

    // Minimap/toggle screen rects (deterministic from the canvas rect); the
    // overlay claims the pointer so it doesn't fall through to the canvas.
    let (mm_btn, mm_rect) = minimap_rects(scope.rect(), state.minimap_open);
    let over_overlay = pointer_pos
        .map(|p| mm_btn.contains(p) || mm_rect.is_some_and(|r| r.contains(p)))
        .unwrap_or(false);

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

    // Advance / finish a live comment/group drag (P7).
    let ann_snapshot = state.annotation_drag.as_ref().map(|d| {
        (d.is_group, d.index, d.origin_world, d.rect_min0, d.captured.clone())
    });
    if let Some((is_group, index, origin, rect0, captured)) = ann_snapshot {
        if let Some(pw) = pointer_world {
            let (dx, dy) = (pw.x - origin[0], pw.y - origin[1]);
            if is_group {
                if let Some(g) = state.doc.groups.get_mut(index) {
                    g.rect[0] = rect0[0] + dx;
                    g.rect[1] = rect0[1] + dy;
                }
                for (id, start) in &captured {
                    if let Some(n) = state.doc.node_mut(*id) {
                        n.position = [start[0] + dx, start[1] + dy];
                    }
                }
            } else if let Some(c) = state.doc.comments.get_mut(index) {
                c.rect[0] = rect0[0] + dx;
                c.rect[1] = rect0[1] + dy;
            }
        }
        if !pointer_down {
            finish_annotation_drag(state, registry);
        }
    }

    // Pins take precedence over the node body.
    let mut pin_claimed = false;
    for g in &geoms {
        for pin in &g.pins {
            let wr = Rect::from_center_size(pin.center, Vec2::splat(PIN_R * 3.0));
            let id = ui.alloc_id(("graph_pin", g.id, &pin.slug, pin.output));
            let resp = scope.interact(ui, id, wr);
            if resp.pressed
                && !over_overlay
                && state.connect_drag.is_none()
                && state.node_drag.is_none()
            {
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
    let mut node_pressed = false;
    for g in &geoms {
        let id = ui.alloc_id(("graph_node", g.id));
        let resp = scope.interact(ui, id, g.rect);
        if resp.pressed && !over_overlay {
            node_pressed = true;
        }
        if resp.pressed
            && !over_overlay
            && !pin_claimed
            && state.node_drag.is_none()
            && state.connect_drag.is_none()
            && !begin_drag
            && !shift
        {
            if !state.selection.contains(&g.id) {
                state.clear_selection();
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
        // Double-click a subgraph node → open its referenced doc as a tab.
        if resp.double_clicked(ui) {
            if let Some(path) = state.doc.node(g.id).and_then(|n| n.subgraph.clone()) {
                *open_subgraph = Some(path);
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

    // Group title bars + comment headers: select, drag, double-click to edit.
    // Behind nodes, so only when no node/pin claimed the press this frame.
    let ann_free = !node_pressed
        && !pin_claimed
        && !over_overlay
        && state.node_drag.is_none()
        && state.annotation_drag.is_none();
    // Groups first (front-most annotation is the last drawn = highest index).
    for i in (0..state.doc.groups.len()).rev() {
        let r = state.doc.groups[i].rect;
        let bar = Rect::from_min_size(Pos2::new(r[0], r[1]), Vec2::new(r[2], GROUP_TITLE_H));
        let id = ui.alloc_id(("graph_group", i));
        let resp = scope.interact(ui, id, bar);
        if resp.double_clicked(ui) {
            begin_annotation_edit(state, true, i);
        } else if resp.pressed && ann_free && pointer_world.is_some() {
            state.clear_selection();
            state.sel_group = Some(i);
            let pw = pointer_world.unwrap();
            let captured = nodes_captured_by_rect(&node_centers(&geoms), r);
            let originals = captured
                .iter()
                .filter_map(|id| state.doc.node(*id).map(|n| (*id, n.position)))
                .collect();
            state.annotation_drag = Some(AnnotationDrag {
                is_group: true,
                index: i,
                origin_world: [pw.x, pw.y],
                rect_min0: [r[0], r[1]],
                captured: originals,
            });
        }
    }
    for i in (0..state.doc.comments.len()).rev() {
        let r = state.doc.comments[i].rect;
        let bar = Rect::from_min_size(Pos2::new(r[0], r[1]), Vec2::new(r[2], COMMENT_HEADER_H));
        let id = ui.alloc_id(("graph_comment", i));
        let resp = scope.interact(ui, id, bar);
        if resp.double_clicked(ui) {
            begin_annotation_edit(state, false, i);
        } else if resp.pressed && ann_free && state.annotation_drag.is_none() && pointer_world.is_some() {
            state.clear_selection();
            state.sel_comment = Some(i);
            let pw = pointer_world.unwrap();
            state.annotation_drag = Some(AnnotationDrag {
                is_group: false,
                index: i,
                origin_world: [pw.x, pw.y],
                rect_min0: [r[0], r[1]],
                captured: Vec::new(),
            });
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

    // Marquee box-select on empty canvas (suppressed while an overlay/node/
    // annotation owns the gesture).
    handle_marquee(
        ui,
        scope,
        state,
        &geoms,
        pointer_world,
        pointer_pressed,
        pointer_down,
        released,
        pin_claimed || over_overlay || node_pressed,
        &st,
    );

    // Right-click empty space → open the create menu at the pointer.
    if right_pressed && !over_overlay {
        if let Some(pw) = pointer_world {
            if pin_under(&geoms, pw).is_none() && node_under(&geoms, pw).is_none() {
                state.create_menu_world = Some([pw.x, pw.y]);
                state.create_menu_search.clear();
                *menu_open_at = ui.ctx().input.pointer_pos;
            }
        }
    }

    // Minimap overlay + toggle button (drawn on top; interaction claimed above).
    draw_minimap(ui, scope, state, &geoms, mm_btn, mm_rect, minimap_pan);
}

/// Node centers (world) for group capture / minimap.
fn node_centers(geoms: &[NodeGeom]) -> Vec<(u64, [f32; 2])> {
    geoms
        .iter()
        .map(|g| (g.id, [g.rect.center().x, g.rect.center().y]))
        .collect()
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

/// Finish a comment/group drag, recording one coalesced Move edit (P7).
fn finish_annotation_drag(state: &mut GraphEditorState, registry: &NodeRegistry) {
    let Some(d) = state.annotation_drag.take() else {
        return;
    };
    let min = if d.is_group {
        state.doc.groups.get(d.index).map(|g| [g.rect[0], g.rect[1]])
    } else {
        state.doc.comments.get(d.index).map(|c| [c.rect[0], c.rect[1]])
    };
    let Some(min) = min else {
        return;
    };
    let delta = [min[0] - d.rect_min0[0], min[1] - d.rect_min0[1]];
    if delta[0].abs() > f32::EPSILON || delta[1].abs() > f32::EPSILON {
        let edit = if d.is_group {
            GraphEdit::MoveGroup {
                index: d.index,
                node_ids: d.captured.iter().map(|(id, _)| *id).collect(),
                delta,
            }
        } else {
            GraphEdit::MoveComment { index: d.index, delta }
        };
        state.commit(edit, registry);
    }
}

/// Open the inline text editor for a comment/group (P7).
fn begin_annotation_edit(state: &mut GraphEditorState, is_group: bool, index: usize) {
    let (text, anchor) = if is_group {
        match state.doc.groups.get(index) {
            Some(g) => (g.title.clone(), [g.rect[0], g.rect[1]]),
            None => return,
        }
    } else {
        match state.doc.comments.get(index) {
            Some(c) => (c.text.clone(), [c.rect[0], c.rect[1]]),
            None => return,
        }
    };
    state.clear_selection();
    if is_group {
        state.sel_group = Some(index);
    } else {
        state.sel_comment = Some(index);
    }
    state.annotation_drag = None;
    state.editing = Some(AnnotationEdit {
        is_group,
        index,
        buffer: text.clone(),
        original: text,
        anchor_world: anchor,
        first_frame: true,
    });
}

/// Toggle-button and minimap screen rects, derived from the canvas rect.
fn minimap_rects(canvas: Rect, open: bool) -> (Rect, Option<Rect>) {
    let btn = Rect::from_min_size(
        Pos2::new(
            canvas.max.x - MINIMAP_MARGIN - MINIMAP_BTN,
            canvas.max.y - MINIMAP_MARGIN - MINIMAP_BTN,
        ),
        Vec2::splat(MINIMAP_BTN),
    );
    let mm = open.then(|| {
        Rect::from_min_size(
            Pos2::new(canvas.max.x - MINIMAP_MARGIN - MINIMAP_W, btn.min.y - 6.0 - MINIMAP_H),
            Vec2::new(MINIMAP_W, MINIMAP_H),
        )
    });
    (btn, mm)
}

/// World-space bounding box `[min_x, min_y, w, h]` of all content, or `None`.
fn doc_bbox(state: &GraphEditorState, geoms: &[NodeGeom]) -> Option<[f32; 4]> {
    let (mut minx, mut miny, mut maxx, mut maxy) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    let mut acc = |x0: f32, y0: f32, x1: f32, y1: f32| {
        minx = minx.min(x0);
        miny = miny.min(y0);
        maxx = maxx.max(x1);
        maxy = maxy.max(y1);
    };
    for g in geoms {
        acc(g.rect.min.x, g.rect.min.y, g.rect.max.x, g.rect.max.y);
    }
    for g in &state.doc.groups {
        acc(g.rect[0], g.rect[1], g.rect[0] + g.rect[2], g.rect[1] + g.rect[3]);
    }
    for c in &state.doc.comments {
        acc(c.rect[0], c.rect[1], c.rect[0] + c.rect[2], c.rect[1] + c.rect[3]);
    }
    if maxx <= minx || maxy <= miny {
        return None;
    }
    Some([minx, miny, maxx - minx, maxy - miny])
}

/// Toggle button (always) + minimap overlay (when open): node/group rects at a
/// fitted transform, a current-view indicator, and click-to-recenter (P7).
#[allow(clippy::too_many_arguments)]
fn draw_minimap(
    ui: &mut Ui,
    scope: &CanvasScope,
    state: &mut GraphEditorState,
    geoms: &[NodeGeom],
    btn: Rect,
    mm: Option<Rect>,
    minimap_pan: &mut Option<Vec2>,
) {
    let st = ui.style();
    let btn_id = ui.alloc_id("graph_minimap_btn");
    let btn_resp = ui.interact(btn_id, btn);
    if btn_resp.clicked {
        state.minimap_open = !state.minimap_open;
    }
    {
        let mut p = ui.painter();
        let fill = if btn_resp.hovered { st.palette.hover } else { st.palette.elevated };
        p.rect_filled(btn, st.rounding.small, fill);
        p.rect_stroke(btn, st.rounding.small, 1.0, st.palette.stroke_strong);
        let inner = Rect::from_center_size(btn.center(), Vec2::splat(MINIMAP_BTN * 0.45));
        let glyph = if state.minimap_open {
            st.palette.accent_active
        } else {
            st.palette.text_secondary
        };
        p.rect_stroke(inner, 0.0, 1.0, glyph);
    }

    let Some(mm) = mm else {
        return;
    };
    let bbox = doc_bbox(state, geoms).unwrap_or_else(|| {
        let v = scope.visible_world_rect();
        [v.min.x, v.min.y, v.width(), v.height()]
    });
    let pad = 6.0;
    let area = Rect::from_min_max(mm.min + Vec2::splat(pad), mm.max - Vec2::splat(pad));
    let scale = (area.width() / bbox[2]).min(area.height() / bbox[3]).max(f32::MIN_POSITIVE);
    let ox = area.min.x + (area.width() - bbox[2] * scale) * 0.5;
    let oy = area.min.y + (area.height() - bbox[3] * scale) * 0.5;
    let to_mm = |wx: f32, wy: f32| Pos2::new(ox + (wx - bbox[0]) * scale, oy + (wy - bbox[1]) * scale);
    {
        let mut p = ui.painter();
        p.rect_filled(mm, st.rounding.small, st.palette.window.with_alpha(0.92));
        p.rect_stroke(mm, st.rounding.small, 1.0, st.palette.stroke_strong);
        for g in &state.doc.groups {
            let a = to_mm(g.rect[0], g.rect[1]);
            let b = to_mm(g.rect[0] + g.rect[2], g.rect[1] + g.rect[3]);
            p.rect_stroke(Rect::from_min_max(a, b), 0.0, 1.0, st.palette.stroke);
        }
        for g in geoms {
            let a = to_mm(g.rect.min.x, g.rect.min.y);
            let b = to_mm(g.rect.max.x, g.rect.max.y);
            p.rect_filled(Rect::from_min_max(a, b), 0.0, st.palette.text_secondary);
        }
        let v = scope.visible_world_rect();
        let a = to_mm(v.min.x, v.min.y);
        let b = to_mm(v.max.x, v.max.y);
        p.rect_stroke(Rect::from_min_max(a, b), 0.0, 1.0, st.palette.accent_active);
    }
    let mm_id = ui.alloc_id("graph_minimap");
    let resp = ui.interact(mm_id, mm);
    if resp.pressed {
        if let Some(pp) = ui.ctx().input.pointer_pos {
            let wx = bbox[0] + (pp.x - ox) / scale;
            let wy = bbox[1] + (pp.y - oy) / scale;
            let half = scope.rect().size() / (2.0 * scope.zoom());
            *minimap_pan = Some(Vec2::new(wx - half.x, wy - half.y));
        }
    }
}

/// Inline text-edit popup for the annotation in `state.editing` (P7).
fn edit_popup(
    ui: &mut Ui,
    state: &mut GraphEditorState,
    registry: &NodeRegistry,
    canvas_rect: Rect,
) {
    let Some((is_group, index, anchor_world, first_frame, original)) = state
        .editing
        .as_ref()
        .map(|e| (e.is_group, e.index, e.anchor_world, e.first_frame, e.original.clone()))
    else {
        return;
    };
    let view = state.view;
    let screen = Pos2::new(
        canvas_rect.min.x + (anchor_world[0] - view.pan.x) * view.zoom,
        canvas_rect.min.y + (anchor_world[1] - view.pan.y) * view.zoom,
    );
    let rect = Rect::from_min_size(screen, Vec2::new(200.0, 26.0));
    {
        let st = ui.style();
        let mut p = ui.painter();
        p.rect_filled(rect, st.rounding.small, st.palette.elevated);
        p.rect_stroke(rect, st.rounding.small, 1.0, st.palette.accent_active);
    }
    let mut buffer = state.editing.as_ref().map(|e| e.buffer.clone()).unwrap_or_default();
    let (out, _) = ui.run_at(
        rect,
        Direction::TopDown,
        Id::new(("graph_annot_edit", is_group, index)),
        UiOptions { padding: Vec2::splat(2.0), spacing: 0.0 },
        |ui| {
            TextEdit::new(&mut buffer)
                .width(196.0)
                .request_focus(first_frame)
                .show_full(ui)
        },
    );
    if out.cancelled {
        state.editing = None;
        return;
    }
    let commit = out.submitted || (!out.focused && !first_frame);
    if commit {
        state.editing = None;
        // Only record the edit if the target still exists — the annotation may
        // have been deleted while the editor was open (guard the index so
        // undo/redo never indexes out of bounds).
        let applied = if buffer == original {
            false
        } else if is_group {
            match state.doc.groups.get_mut(index) {
                Some(g) => {
                    g.title = buffer.clone();
                    true
                }
                None => false,
            }
        } else {
            match state.doc.comments.get_mut(index) {
                Some(c) => {
                    c.text = buffer.clone();
                    true
                }
                None => false,
            }
        };
        if applied {
            let edit = if is_group {
                GraphEdit::SetGroupTitle { index, old: original, new: buffer }
            } else {
                GraphEdit::SetCommentText { index, old: original, new: buffer }
            };
            state.commit(edit, registry);
        }
    } else if let Some(e) = state.editing.as_mut() {
        e.buffer = buffer;
        e.first_frame = false;
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
        && state.annotation_drag.is_none()
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
    subgraph_assets: &[String],
    open_at: Option<Pos2>,
) {
    let world = state.create_menu_world;
    let has_selection = !state.selection.is_empty();
    let search = &mut state.create_menu_search;
    let mut chosen: Option<String> = None;
    let mut chosen_subgraph: Option<String> = None;
    let mut add_comment = false;
    let mut add_group = false;
    crusty_gui::widgets::context_menu_at(ui, "graph_create_menu", open_at, |ui| {
        ui.menu_group_header("Add Node");
        TextEdit::new(search).hint("Search\u{2026}").width(170.0).show(ui);
        let needle = search.to_lowercase();
        if needle.is_empty() {
            ui.menu_group_header("Annotate");
            if ui.menu_item("Add Comment") {
                add_comment = true;
            }
            if ui.menu_item_enabled("Add Group around selection", has_selection) {
                add_group = true;
            }
        }
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
        // Subgraph assets (`.subgraph`) as instance nodes.
        let subs: Vec<(&String, String)> = subgraph_assets
            .iter()
            .map(|p| {
                let stem = std::path::Path::new(p)
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| p.clone());
                (p, stem)
            })
            .filter(|(p, stem)| {
                needle.is_empty()
                    || stem.to_lowercase().contains(&needle)
                    || p.to_lowercase().contains(&needle)
            })
            .collect();
        if !subs.is_empty() {
            ui.menu_group_header("Subgraph");
            for (path, stem) in subs {
                if ui.menu_item(&stem) {
                    chosen_subgraph = Some(path.clone());
                }
            }
        }
    });
    if let Some(pos) = world {
        if let Some(type_id) = chosen {
            state.add_node(&type_id, pos, registry);
            state.create_menu_world = None;
        } else if let Some(path) = chosen_subgraph {
            state.add_subgraph_node(&path, pos, registry);
            state.create_menu_world = None;
        } else if add_comment {
            state.add_comment(pos, registry);
            state.create_menu_world = None;
        } else if add_group {
            state.add_group_around_selection(registry);
            state.create_menu_world = None;
        }
    }
}

/// Compact validation summary pinned to the canvas's top-left corner. Shows
/// doc-local and cross-asset (subgraph) errors together.
fn error_overlay(ui: &mut Ui, rect: Rect, doc_errors: &[GraphError], ref_errors: &[GraphError]) {
    let total = doc_errors.len() + ref_errors.len();
    if total == 0 {
        return;
    }
    let st = ui.style();
    let status = Palette::invariant_status();
    let font = st.fonts.small;
    let pad = 6.0;
    const MAX_LINES: usize = 3;
    let header = format!(
        "{} validation error{}",
        total,
        if total == 1 { "" } else { "s" }
    );
    let mut lines: Vec<String> = doc_errors
        .iter()
        .chain(ref_errors.iter())
        .take(MAX_LINES)
        .map(|e| format!("{e}"))
        .collect();
    if total > MAX_LINES {
        lines.push(format!("+{} more\u{2026}", total - MAX_LINES));
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
