//! Blend space editor panel rendered with crusty-gui (Task 41.5 ticket 04).
//!
//! Two regions: a **details column** on the left (Axes, Samples, Smoothing —
//! every field commits at end-edit as one undo entry) and the **canvas** on
//! the right, a [`Canvas`] scope that draws the axis frame, grid lines at
//! the divisions, the compiled triangulation and the samples as dots. A
//! footer line under the canvas carries the compile status.
//!
//! This ticket's canvas is static: hit-testing, dragging and the preview
//! point are ticket 05, which adds them inside [`canvas`]'s scope body
//! (interact first, draw second — the curve plot's order) using
//! [`doc_to_world`] / [`world_to_doc`] for the mapping.
//!
//! Policy lives in `blend_space_editor`: field gestures, edits and the
//! compiled space are decided — and tested — there. This file draws.

use crusty_gui::context::{Direction, Ui, UiOptions};
use crusty_gui::id::Id;
use crusty_gui::input::{Key as UiKey, Modifiers};
use crusty_gui::math::{Color, Pos2, Rect, Vec2};
use crusty_gui::style::Style;
use crusty_gui::text::FontFamily;
use crusty_gui::widgets::{
    Button, Canvas, CanvasScope, CanvasView, ComboBox, DragValue, ScrollArea, SelectableValue,
    TextEdit,
};

use super::blend_space_editor::{BlendSpaceEditorState, Field, FieldEvent};
use super::theme::tokens::{asset_color, grid_major, grid_minor};
use super::theme::Palette;
use super::widgets::segmented_control;
use crate::engine::animation::blend_space::{BlendAxis, BlendSample, BlendSpaceDoc};

// Base metrics at UI scale 1.0; multiplied by the panel's `s` before use.
const DETAILS_W: f32 = 300.0;
const LABEL_W: f32 = 84.0;
const FOOTER_H: f32 = 24.0;
/// Canvas world box the axis ranges map onto (pixels at zoom 1).
pub const WORLD_W: f32 = 800.0;
pub const WORLD_H: f32 = 500.0;
/// World height of the one-axis strip.
pub const WORLD_H_1D: f32 = 120.0;
/// Screen margin the fit leaves for the border labels.
const FIT_MARGIN: f32 = 56.0;
const SAMPLE_R: f32 = 5.0;
const ZOOM_MIN: f32 = 0.1;
const ZOOM_MAX: f32 = 8.0;
const TOAST_MS: f32 = 1800.0;

/// What the panel wants the host to do after the frame. Save is the host's
/// job — only it can reach the caches a written `.blendspace` invalidates.
#[derive(Default)]
pub struct BlendSpaceEditorOutput {
    pub save_requested: bool,
}

pub struct BlendSpaceEditorPanelCtx<'a> {
    pub state: &'a mut BlendSpaceEditorState,
    /// Content-relative `.anim` paths from the asset registry — the Clip
    /// dropdown's rows.
    pub anim_assets: &'a [String],
    /// `selection.outline` from the live theme (crusty's `Style` has none).
    pub selection_outline: Color,
    /// This tab is the focused tab of its dock (gates keyboard editing).
    pub focused: bool,
    /// True in float windows — the panel handles undo/redo/save/delete
    /// itself. False when docked, where `EditorAction` routing owns them.
    pub handle_shortcuts: bool,
}

pub fn blend_space_editor_panel(
    ui: &mut Ui,
    tab_rect: Rect,
    ctx: BlendSpaceEditorPanelCtx,
) -> BlendSpaceEditorOutput {
    let BlendSpaceEditorPanelCtx { state, anim_assets, selection_outline, focused, handle_shortcuts } =
        ctx;
    let mut out = BlendSpaceEditorOutput::default();
    let panel_id = Id::new("engine_blend_space_editor").with(state.path.as_str());
    let opts = UiOptions { padding: Vec2::ZERO, spacing: 0.0 };
    ui.run_at(tab_rect, Direction::TopDown, panel_id, opts, |ui| {
        let s = (ui.style().metrics.row_height / 22.0).max(0.1);
        let side_w = (DETAILS_W * s).min(tab_rect.width() * 0.5);
        let side = Rect::from_min_size(tab_rect.min, Vec2::new(side_w, tab_rect.height()));
        details(ui, side, s, panel_id, state, anim_assets);

        let foot = Rect::from_min_max(
            Pos2::new(side.max.x, tab_rect.max.y - FOOTER_H * s),
            tab_rect.max,
        );
        let plot = Rect::from_min_max(Pos2::new(side.max.x, tab_rect.min.y), Pos2::new(tab_rect.max.x, foot.min.y));
        canvas(ui, plot, s, selection_outline, state);
        footer(ui, foot, s, state);
        draw_toast(ui, plot, state);

        if focused {
            handle_panel_keys(ui, state, handle_shortcuts, &mut out);
        }
    });
    out
}

// ── details column ──────────────────────────────────────────────────────────

fn details(
    ui: &mut Ui,
    rect: Rect,
    s: f32,
    panel_id: Id,
    state: &mut BlendSpaceEditorState,
    anim_assets: &[String],
) {
    let st = ui.style();
    let pad = st.spacing.padding;
    {
        let mut p = ui.painter();
        p.rect_filled(rect, 0.0, st.palette.panel);
        p.line_segment(
            Pos2::new(rect.max.x, rect.min.y),
            Pos2::new(rect.max.x, rect.max.y),
            st.metrics.border,
            st.palette.stroke,
        );
    }
    let inner = Rect::from_min_max(rect.min + Vec2::splat(pad), rect.max - Vec2::splat(pad));
    ui.run_at(
        inner,
        Direction::TopDown,
        panel_id.with("details"),
        UiOptions { padding: Vec2::ZERO, spacing: pad * 0.6 },
        |ui| {
            let w = inner.width();
            ScrollArea::new(inner.height()).inset(0.0).spacing(s).auto_shrink(false).show(ui, |ui| {
                axes_section(ui, w, s, &st, state);
                ui.add_space(pad);
                samples_section(ui, w, s, &st, state, anim_assets);
                ui.add_space(pad);
                section_header(ui, w, &st, "SMOOTHING");
                let before = state.doc.input_smoothing;
                if let Some(ev) = num_row(ui, w, s, &st, state, "Input (s)", Field::Smoothing, before, 0.0..=5.0, 2) {
                    match ev {
                        FieldEvent::Live(v) => state.doc.input_smoothing = v,
                        FieldEvent::Commit { from, to } => {
                            state.doc.input_smoothing = to;
                            state.record_smoothing(from);
                        }
                        FieldEvent::None => {}
                    }
                }
            });
        },
    );
}

fn axes_section(ui: &mut Ui, w: f32, s: f32, st: &Style, state: &mut BlendSpaceEditorState) {
    section_header(ui, w, st, "AXES");
    let row = ui.allocate(Vec2::new(w, st.metrics.control_height));
    let active = if state.doc.is_2d() { 1 } else { 0 };
    let ctl = Rect::from_min_size(Pos2::new(row.min.x + LABEL_W * s, row.min.y), Vec2::new(row.width() - LABEL_W * s, row.height()));
    row_label_at(ui, row, s, st, "Axes");
    if let Some(i) = segmented_control(ui, "bs_axis_count", ctl, &["1D", "2D"], active, true) {
        state.set_axis_count(i as u32 + 1);
    }
    for axis in 0..state.doc.active_axes().len() {
        ui.add_space(st.spacing.padding * 0.5);
        section_header(ui, w, st, if axis == 0 { "X AXIS" } else { "Y AXIS" });
        let a = state.doc.axes[axis].clone();
        if let Some(v) = text_row(ui, w, s, st, state, "Name", Field::AxisName(axis), &a.name, None) {
            state.set_axis(axis, BlendAxis { name: v, ..a.clone() }, "Set Axis Name");
        }
        if let Some(v) = text_row(ui, w, s, st, state, "Parameter", Field::AxisParam(axis), &a.param, Some(&a.name)) {
            state.set_axis(axis, BlendAxis { param: v, ..a.clone() }, "Set Axis Parameter");
        }
        let numeric: [(&str, Field, f32, std::ops::RangeInclusive<f32>, usize, &str); 3] = [
            ("Min", Field::AxisMin(axis), a.min, -1.0e6..=1.0e6, 2, "Set Axis Min"),
            ("Max", Field::AxisMax(axis), a.max, -1.0e6..=1.0e6, 2, "Set Axis Max"),
            ("Grid", Field::AxisGrid(axis), a.grid_divisions as f32, 1.0..=64.0, 0, "Set Grid Divisions"),
        ];
        for (label, field, value, range, decimals, verb) in numeric {
            let Some(ev) = num_row(ui, w, s, st, state, label, field, value, range, decimals) else { continue };
            let write = |a: &mut BlendAxis, v: f32| match field {
                Field::AxisMin(_) => a.min = v,
                Field::AxisMax(_) => a.max = v,
                _ => a.grid_divisions = v.round().max(1.0) as u32,
            };
            match ev {
                FieldEvent::Live(v) => write(&mut state.doc.axes[axis], v),
                FieldEvent::Commit { from, to } => {
                    let mut before = state.doc.axes[axis].clone();
                    write(&mut before, from);
                    write(&mut state.doc.axes[axis], to);
                    state.record_axis(axis, before, verb);
                }
                FieldEvent::None => {}
            }
        }
    }
}

fn samples_section(
    ui: &mut Ui,
    w: f32,
    s: f32,
    st: &Style,
    state: &mut BlendSpaceEditorState,
    anim_assets: &[String],
) {
    section_header(ui, w, st, "SAMPLES");
    let two_d = state.doc.is_2d();
    let warning = Palette::invariant_status().warning;
    let mut delete: Option<usize> = None;
    for i in 0..state.doc.samples.len() {
        let sample = state.doc.samples[i].clone();
        let selected = state.selection == Some(i);
        // Card: clip row, optional clip-name row, position/rate row.
        let stem = clip_stem(&sample.clip);
        let card_top = ui.cursor().y;
        ui.horizontal(|ui| {
            let btn_w = st.metrics.control_height;
            let combo_w = w - btn_w - st.spacing.item;
            let mut pick: Option<String> = None;
            ComboBox::new(format!("bs_clip_{i}"))
                .selected_text(if sample.clip.is_empty() { "(no clip)".to_string() } else { stem.clone() })
                .width(combo_w)
                .popup_width(combo_w.max(260.0 * s))
                .show_ui(ui, |ui| {
                    for path in anim_assets {
                        let mut sel = *path == sample.clip;
                        if SelectableValue::new(&mut sel, true, path.as_str()).show(ui).clicked {
                            pick = Some(path.clone());
                        }
                    }
                });
            if let Some(clip) = pick {
                if clip != sample.clip {
                    state.set_sample(i, BlendSample { clip, clip_name: None, ..sample.clone() }, "Set Sample Clip");
                }
            }
            if Button::new("\u{00D7}").exact_size(Vec2::splat(btn_w)).danger_outline().show(ui).clicked {
                delete = Some(i);
            }
        });
        if !sample.clip.is_empty() && !anim_assets.iter().any(|p| *p == sample.clip) {
            let r = ui.allocate(Vec2::new(w, st.fonts.small * 1.6));
            ui.painter().text(
                Pos2::new(r.min.x, r.min.y),
                &format!("\u{26A0} {} not found in project", sample.clip),
                st.fonts.small,
                warning,
                Some(w),
            );
        }
        let names: Vec<String> = if sample.clip.is_empty() { Vec::new() } else { state.clip_names(&sample.clip).to_vec() };
        if names.len() > 1 {
            let row = ui.allocate(Vec2::new(w, st.metrics.control_height));
            row_label_at(ui, row, s, st, "Clip name");
            let mut pick: Option<String> = None;
            let id = ui.alloc_id(("bs_clip_name_row", i));
            ui.run_at(
                Rect::from_min_max(Pos2::new(row.min.x + LABEL_W * s, row.min.y), row.max),
                Direction::LeftToRight,
                id,
                UiOptions { padding: Vec2::ZERO, spacing: 0.0 },
                |ui| {
                    let current = sample.clip_name.clone().unwrap_or_else(|| names[0].clone());
                    ComboBox::new(format!("bs_clip_name_{i}"))
                        .selected_text(current.clone())
                        .width(row.width() - LABEL_W * s)
                        .show_ui(ui, |ui| {
                            for n in &names {
                                let mut sel = *n == current;
                                if SelectableValue::new(&mut sel, true, n.as_str()).show(ui).clicked {
                                    pick = Some(n.clone());
                                }
                            }
                        });
                },
            );
            if let Some(n) = pick {
                let clip_name = (n != names[0]).then_some(n);
                state.set_sample(i, BlendSample { clip_name, ..sample.clone() }, "Set Sample Clip Name");
            }
        }
        let mut fields: Vec<(&str, Field, f32, usize, &str)> = vec![
            ("X", Field::SampleX(i), sample.x, 2, "Move Sample"),
        ];
        if two_d {
            fields.push(("Y", Field::SampleY(i), sample.y, 2, "Move Sample"));
        }
        fields.push(("Rate", Field::SampleRate(i), sample.rate_scale, 2, "Set Rate Scale"));
        let row = ui.allocate(Vec2::new(w, st.metrics.control_height));
        let n = fields.len() as f32;
        let cell_w = (row.width() - st.spacing.item * (n - 1.0)) / n;
        for (k, (label, field, value, decimals, verb)) in fields.into_iter().enumerate() {
            let cell = Rect::from_min_size(
                Pos2::new(row.min.x + (cell_w + st.spacing.item) * k as f32, row.min.y),
                Vec2::new(cell_w, row.height()),
            );
            let lab_w = st.fonts.small * 2.4;
            ui.painter().text(
                Pos2::new(cell.min.x, cell.center().y - st.fonts.small * 0.62),
                label,
                st.fonts.small,
                st.palette.text_secondary,
                None,
            );
            let mut ev = FieldEvent::None;
            let id = ui.alloc_id(("bs_sample_num", i, k));
            ui.run_at(
                Rect::from_min_max(Pos2::new(cell.min.x + lab_w, cell.min.y), cell.max),
                Direction::LeftToRight,
                id,
                UiOptions { padding: Vec2::ZERO, spacing: 0.0 },
                |ui| {
                    ev = num_field(ui, cell.width() - lab_w, s, state, field, value, -1.0e6..=1.0e6, decimals);
                },
            );
            let write = |sm: &mut BlendSample, v: f32| match field {
                Field::SampleX(_) => sm.x = v,
                Field::SampleY(_) => sm.y = v,
                _ => sm.rate_scale = v,
            };
            match ev {
                FieldEvent::Live(v) => write(&mut state.doc.samples[i], v),
                FieldEvent::Commit { from, to } => {
                    let mut before = state.doc.samples[i].clone();
                    write(&mut before, from);
                    write(&mut state.doc.samples[i], to);
                    state.record_sample(i, before, verb);
                }
                FieldEvent::None => {}
            }
        }
        if selected {
            let card = Rect::from_min_max(
                Pos2::new(ui.cursor().x - st.spacing.padding * 0.5, card_top - st.spacing.padding * 0.3),
                Pos2::new(ui.cursor().x + w + st.spacing.padding * 0.5, ui.cursor().y + st.spacing.padding * 0.3),
            );
            ui.painter().rect_stroke(card, st.rounding.small, st.metrics.border, st.palette.focus_ring);
        }
        ui.add_space(st.spacing.padding * 0.6);
    }
    if let Some(i) = delete {
        state.remove_sample(i);
    }
    if Button::new("+ Add Sample").min_size(Vec2::new(w, st.metrics.control_height)).show(ui).clicked {
        state.add_sample();
    }
}

// ── field helpers ───────────────────────────────────────────────────────────

fn section_header(ui: &mut Ui, w: f32, st: &Style, text: &str) {
    let head = ui.allocate(Vec2::new(w, st.fonts.small * 1.8));
    ui.painter().text_family(
        Pos2::new(head.min.x, head.center().y - st.fonts.small * 0.62),
        text,
        st.fonts.small,
        st.palette.text_secondary,
        None,
        FontFamily::Mono,
    );
}

fn row_label_at(ui: &mut Ui, row: Rect, s: f32, st: &Style, label: &str) {
    ui.painter().text(
        Pos2::new(row.min.x, row.center().y - st.fonts.body * 0.62),
        label,
        st.fonts.body,
        st.palette.text,
        Some(LABEL_W * s),
    );
}

/// Label + text field on one row; returns the committed value once per edit.
fn text_row(
    ui: &mut Ui,
    w: f32,
    s: f32,
    st: &Style,
    state: &mut BlendSpaceEditorState,
    label: &str,
    field: Field,
    current: &str,
    hint: Option<&str>,
) -> Option<String> {
    let row = ui.allocate(Vec2::new(w, st.metrics.control_height));
    row_label_at(ui, row, s, st, label);
    let mut buf = state.text_buffer(field, current);
    let mut out = None;
    let id = ui.alloc_id(("bs_text", field));
    ui.run_at(
        Rect::from_min_max(Pos2::new(row.min.x + LABEL_W * s, row.min.y), row.max),
        Direction::LeftToRight,
        id,
        UiOptions { padding: Vec2::ZERO, spacing: 0.0 },
        |ui| {
            let mut te = TextEdit::new(&mut buf).width(row.width() - LABEL_W * s);
            if let Some(h) = hint {
                te = te.hint(h);
            }
            out = Some(te.show_full(ui));
        },
    );
    let o = out?;
    state.text_event(field, current, buf, o.focused, o.submitted, o.cancelled)
}

/// Label + numeric field on one row.
#[allow(clippy::too_many_arguments)]
fn num_row(
    ui: &mut Ui,
    w: f32,
    s: f32,
    st: &Style,
    state: &mut BlendSpaceEditorState,
    label: &str,
    field: Field,
    value: f32,
    range: std::ops::RangeInclusive<f32>,
    decimals: usize,
) -> Option<FieldEvent> {
    let row = ui.allocate(Vec2::new(w, st.metrics.control_height));
    row_label_at(ui, row, s, st, label);
    let mut ev = FieldEvent::None;
    let id = ui.alloc_id(("bs_num", field));
    ui.run_at(
        Rect::from_min_max(Pos2::new(row.min.x + LABEL_W * s, row.min.y), row.max),
        Direction::LeftToRight,
        id,
        UiOptions { padding: Vec2::ZERO, spacing: 0.0 },
        |ui| ev = num_field(ui, row.width() - LABEL_W * s, s, state, field, value, range, decimals),
    );
    (ev != FieldEvent::None).then_some(ev)
}

/// A `DragValue` folded through the state's gesture rule.
#[allow(clippy::too_many_arguments)]
fn num_field(
    ui: &mut Ui,
    width: f32,
    s: f32,
    state: &mut BlendSpaceEditorState,
    field: Field,
    value: f32,
    range: std::ops::RangeInclusive<f32>,
    decimals: usize,
) -> FieldEvent {
    let mut v = value;
    let resp = DragValue::new(&mut v)
        .width(width)
        .height(ui.style().metrics.control_height)
        .range(range)
        .decimals(decimals)
        .speed(0.01 * if decimals == 0 { 10.0 } else { 1.0 } * s as f64)
        .show(ui);
    state.numeric_event(field, value, v, resp.pressed)
}

fn clip_stem(clip: &str) -> String {
    std::path::Path::new(clip)
        .file_stem()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| clip.to_string())
}

// ── canvas ──────────────────────────────────────────────────────────────────

/// World size of the axis box for this document.
pub fn world_size(doc: &BlendSpaceDoc) -> Vec2 {
    Vec2::new(WORLD_W, if doc.is_2d() { WORLD_H } else { WORLD_H_1D })
}

fn norm(a: &BlendAxis, v: f32) -> f32 {
    let span = a.max - a.min;
    if span.abs() < 1e-6 { 0.5 } else { (v - a.min) / span }
}

/// Axis units → canvas world. `y` is the strip's centreline for one axis.
pub fn doc_to_world(doc: &BlendSpaceDoc, p: [f32; 2]) -> Pos2 {
    let size = world_size(doc);
    let y = if doc.is_2d() { (1.0 - norm(&doc.axes[1], p[1])) * size.y } else { size.y * 0.5 };
    Pos2::new(norm(&doc.axes[0], p[0]) * size.x, y)
}

/// Canvas world → axis units (unclamped). One-axis spaces report `y = 0`.
pub fn world_to_doc(doc: &BlendSpaceDoc, w: Pos2) -> [f32; 2] {
    let size = world_size(doc);
    let ax = &doc.axes[0];
    let x = ax.min + (w.x / size.x) * (ax.max - ax.min);
    let y = if doc.is_2d() {
        let ay = &doc.axes[1];
        ay.min + (1.0 - w.y / size.y) * (ay.max - ay.min)
    } else {
        0.0
    };
    [x, y]
}

/// A view that fits the axis box with room for the border labels.
pub fn fit_view(doc: &BlendSpaceDoc, size: Vec2, s: f32) -> CanvasView {
    let world = world_size(doc);
    let m = FIT_MARGIN * s;
    let zoom = ((size.x - 2.0 * m) / world.x).min((size.y - 2.0 * m) / world.y).clamp(ZOOM_MIN, ZOOM_MAX);
    CanvasView {
        pan: Vec2::new(world.x * 0.5 - size.x * 0.5 / zoom, world.y * 0.5 - size.y * 0.5 / zoom),
        zoom,
    }
}

fn canvas(ui: &mut Ui, rect: Rect, s: f32, selection: Color, state: &mut BlendSpaceEditorState) {
    let st = ui.style();
    ui.painter().rect_filled(rect, 0.0, st.palette.window);
    if state.frame_pending {
        state.frame_pending = false;
        state.view = fit_view(&state.doc, rect.size(), s);
    }
    let mut view = state.view;
    ui.run_at(
        rect,
        Direction::TopDown,
        Id::new("blend_space_canvas").with(state.path.as_str()),
        UiOptions { padding: Vec2::ZERO, spacing: 0.0 },
        |ui| {
            Canvas::new().size(rect.size()).zoom_range(ZOOM_MIN, ZOOM_MAX).show(ui, &mut view, |ui, scope| {
                // Ticket 05: interact here, before drawing.
                draw_frame_and_grid(ui, scope, s, state, &st);
                draw_triangulation(ui, scope, state, &st);
                draw_samples(ui, scope, s, selection, state, &st);
            });
        },
    );
    state.view = view;
}

fn draw_frame_and_grid(ui: &mut Ui, scope: &CanvasScope, s: f32, state: &BlendSpaceEditorState, st: &Style) {
    let doc = &state.doc;
    let size = world_size(doc);
    let frame = scope.world_rect_to_screen(Rect::from_min_size(Pos2::new(0.0, 0.0), size));
    let bg = st.palette.window;
    let small = st.fonts.small;
    let mut p = ui.painter();
    p.rect_filled(frame, 0.0, st.palette.panel);

    let ax = &doc.axes[0];
    for i in 1..ax.grid_divisions.max(1) {
        let x = scope.world_to_screen(Pos2::new(size.x * i as f32 / ax.grid_divisions as f32, 0.0)).x;
        p.line_segment(Pos2::new(x, frame.min.y), Pos2::new(x, frame.max.y), st.metrics.border, grid_minor(bg));
    }
    if doc.is_2d() {
        let ay = &doc.axes[1];
        for j in 1..ay.grid_divisions.max(1) {
            let y = scope.world_to_screen(Pos2::new(0.0, size.y * j as f32 / ay.grid_divisions as f32)).y;
            p.line_segment(Pos2::new(frame.min.x, y), Pos2::new(frame.max.x, y), st.metrics.border, grid_minor(bg));
        }
    } else {
        // One axis: the strip's centreline is where the samples sit.
        let y = frame.center().y;
        p.line_segment(Pos2::new(frame.min.x, y), Pos2::new(frame.max.x, y), st.metrics.border, grid_major(bg, false));
    }
    p.rect_stroke(frame, 0.0, st.metrics.border, grid_major(bg, true));

    // Border labels: range ends in mono, axis name (and the parameter it
    // reads, when that differs) centred on the edge.
    let mono = FontFamily::Mono;
    let secondary = st.palette.text_secondary;
    let fmt = |v: f32| format!("{v:.2}").trim_end_matches('0').trim_end_matches('.').to_string();
    let below = frame.max.y + small * 0.5;
    p.text_family(Pos2::new(frame.min.x, below), &fmt(ax.min), small, secondary, None, mono);
    let max_label = fmt(ax.max);
    let mw = p.measure_text_family(&max_label, small, None, mono).x;
    p.text_family(Pos2::new(frame.max.x - mw, below), &max_label, small, secondary, None, mono);
    let title = axis_title(ax);
    let tw = p.measure_text(&title, st.fonts.body, None).x;
    p.text(Pos2::new(frame.center().x - tw * 0.5, below + small * 1.4 * s), &title, st.fonts.body, st.palette.text, None);

    if doc.is_2d() {
        let ay = &doc.axes[1];
        let right = frame.min.x - small * 0.5;
        let lo = fmt(ay.min);
        let lw = p.measure_text_family(&lo, small, None, mono).x;
        p.text_family(Pos2::new(right - lw, frame.max.y - small * 1.2), &lo, small, secondary, None, mono);
        let hi = fmt(ay.max);
        let hw = p.measure_text_family(&hi, small, None, mono).x;
        p.text_family(Pos2::new(right - hw, frame.min.y), &hi, small, secondary, None, mono);
        let title = axis_title(ay);
        p.text(Pos2::new(frame.min.x, frame.min.y - st.fonts.body * 1.5 * s), &title, st.fonts.body, st.palette.text, None);
    }
}

fn axis_title(a: &BlendAxis) -> String {
    if a.param.is_empty() || a.param == a.name {
        a.name.clone()
    } else {
        format!("{} \u{2190} {}", a.name, a.param)
    }
}

fn draw_triangulation(ui: &mut Ui, scope: &CanvasScope, state: &BlendSpaceEditorState, st: &Style) {
    let Ok(space) = &state.compiled else { return };
    let doc = &state.doc;
    let at = |i: usize| scope.world_to_screen(doc_to_world(doc, space.points()[i]));
    let color = st.palette.stroke_strong;
    let mut p = ui.painter();
    if space.triangles().is_empty() {
        // Line-shaped set: hull is its two extremes.
        let hull = space.hull();
        if hull.len() == 2 {
            p.line_segment(at(hull[0]), at(hull[1]), st.metrics.border, color);
        }
        return;
    }
    for t in space.triangles() {
        p.polygon_stroke(&[at(t[0]), at(t[1]), at(t[2])], st.metrics.border, color);
    }
}

fn draw_samples(ui: &mut Ui, scope: &CanvasScope, s: f32, selection: Color, state: &BlendSpaceEditorState, st: &Style) {
    let doc = &state.doc;
    let fill = asset_color("animation");
    let r = SAMPLE_R * s;
    let label_px = scope.label_size(st.fonts.small);
    let mut p = ui.painter();
    for (i, sm) in doc.samples.iter().enumerate() {
        let c = scope.world_to_screen(doc_to_world(doc, [sm.x, sm.y]));
        p.circle_filled(c, r, fill);
        if state.selection == Some(i) {
            p.circle_stroke(c, r + 2.0 * s, 1.5 * s, selection);
        }
        if let Some(px) = label_px {
            let mut label = if sm.clip.is_empty() { "\u{2014}".to_string() } else { clip_stem(&sm.clip) };
            if let Some(n) = &sm.clip_name {
                label = format!("{label}:{n}");
            }
            let w = p.measure_text(&label, px, None).x;
            p.text(Pos2::new(c.x - w * 0.5, c.y + r + 2.0 * s), &label, px, st.palette.text_secondary, None);
        }
    }
    if let Some(pt) = state.preview_point {
        let c = scope.world_to_screen(doc_to_world(doc, pt));
        p.circle_filled(c, r * 0.9, Palette::invariant_status().success);
    }
}

fn footer(ui: &mut Ui, rect: Rect, s: f32, state: &BlendSpaceEditorState) {
    let st = ui.style();
    let mut p = ui.painter();
    p.rect_filled(rect, 0.0, st.palette.panel);
    p.line_segment(rect.min, Pos2::new(rect.max.x, rect.min.y), st.metrics.border, st.palette.stroke);
    let (text, color) = match &state.compiled {
        Ok(space) => (
            format!(
                "{} sample{} \u{00B7} {} triangle{}",
                doc_len(state),
                if doc_len(state) == 1 { "" } else { "s" },
                space.triangles().len(),
                if space.triangles().len() == 1 { "" } else { "s" }
            ),
            st.palette.text_secondary,
        ),
        Err(e) => (format!("\u{26A0} {e}"), Palette::invariant_status().warning),
    };
    p.text_family(
        Pos2::new(rect.min.x + st.spacing.padding, rect.center().y - st.fonts.small * 0.62),
        &text,
        st.fonts.small,
        color,
        Some(rect.width() - st.spacing.padding * 2.0 * s),
        FontFamily::Mono,
    );
}

fn doc_len(state: &BlendSpaceEditorState) -> usize {
    state.doc.samples.len()
}

fn draw_toast(ui: &mut Ui, rect: Rect, state: &mut BlendSpaceEditorState) {
    let Some((msg, at)) = state.toast.clone() else { return };
    if at.elapsed().as_secs_f32() * 1000.0 > TOAST_MS {
        state.toast = None;
        return;
    }
    let st = ui.style();
    let pad = st.spacing.padding;
    let mut p = ui.painter();
    let size = p.measure_text(&msg, st.fonts.body, None) + Vec2::splat(pad * 2.0);
    let r = Rect::from_min_size(Pos2::new(rect.max.x - size.x - pad, rect.min.y + pad), size);
    p.rect_filled(r, st.rounding.small, st.palette.elevated);
    p.rect_stroke(r, st.rounding.small, st.metrics.border, st.palette.stroke);
    p.text(r.min + Vec2::splat(pad), &msg, st.fonts.body, st.palette.text, None);
}

fn handle_panel_keys(
    ui: &Ui,
    state: &mut BlendSpaceEditorState,
    handle_shortcuts: bool,
    out: &mut BlendSpaceEditorOutput,
) {
    let input = &ui.ctx().input;
    if ui.ctx().text_focused() || ui.ctx().modal_any_open() || !handle_shortcuts {
        return;
    }
    let ctrl = input.modifiers == Modifiers::CTRL;
    if ctrl && input.key_pressed(UiKey::Char('z')) {
        state.undo();
    }
    if (ctrl && input.key_pressed(UiKey::Char('y')))
        || (input.modifiers == Modifiers::CTRL.union(Modifiers::SHIFT) && input.key_pressed(UiKey::Char('z')))
    {
        state.redo();
    }
    if ctrl && input.key_pressed(UiKey::Char('s')) {
        out.save_requested = true;
    }
    if input.key_pressed(UiKey::Delete) {
        state.delete_selection();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc_2d() -> BlendSpaceDoc {
        let mut d = BlendSpaceDoc::default();
        d.axis_count = 2;
        d.axes[0] = BlendAxis::new("Speed", 0.0, 6.0);
        d.axes[1] = BlendAxis::new("Direction", -1.0, 1.0);
        d
    }

    #[test]
    fn doc_and_world_are_inverses_and_y_grows_up() {
        let d = doc_2d();
        let w = doc_to_world(&d, [3.0, 1.0]);
        assert_eq!(w, Pos2::new(WORLD_W * 0.5, 0.0), "max y is the top edge");
        let back = world_to_doc(&d, w);
        assert!((back[0] - 3.0).abs() < 1e-5 && (back[1] - 1.0).abs() < 1e-5);
    }

    #[test]
    fn one_axis_maps_onto_the_strip_centreline() {
        let mut d = doc_2d();
        d.axis_count = 1;
        assert_eq!(doc_to_world(&d, [6.0, 0.7]).y, WORLD_H_1D * 0.5);
        assert_eq!(world_to_doc(&d, Pos2::new(WORLD_W, 3.0))[1], 0.0);
    }

    #[test]
    fn fit_view_centres_the_box_inside_the_margin() {
        let d = doc_2d();
        let size = Vec2::new(1000.0, 700.0);
        let v = fit_view(&d, size, 1.0);
        let scale = |w: Pos2| Pos2::new((w.x - v.pan.x) * v.zoom, (w.y - v.pan.y) * v.zoom);
        let min = scale(Pos2::new(0.0, 0.0));
        let max = scale(Pos2::new(WORLD_W, WORLD_H));
        assert!(min.x >= FIT_MARGIN - 1e-3 && min.y >= FIT_MARGIN - 1e-3);
        assert!(max.x <= size.x - FIT_MARGIN + 1e-3 && max.y <= size.y - FIT_MARGIN + 1e-3);
        assert!(((min.x + max.x) * 0.5 - size.x * 0.5).abs() < 1e-3, "centred");
    }
}
