//! Curve editor panel rendered with crusty-gui (Task 45-A P8b).
//!
//! Three regions: a toolbar row, a track sidebar, and a pan/zoom plot on
//! crusty's [`Canvas`] — so the plot gets middle/right-drag pan and
//! Ctrl+wheel zoom for free, exactly like the graph canvas.
//!
//! Plot world units are pixels at zoom 1: `x = t × PX_PER_SECOND`,
//! `y = −value × PX_PER_UNIT` (screen y grows down, values grow up). Every
//! sample drawn comes from [`curve_asset::Track::sample`] — the same function
//! the interpreter calls — so what the author sees is what the Timeline plays.
//!
//! Gestures: drag a key (t clamped between its neighbours, so key order and
//! therefore every undo index survives), double-click empty plot to add one to
//! the selected track, `Del` to remove the selected key, right-click a key to
//! cycle Constant → Linear → Cubic.

use crusty_gui::context::{Direction, Ui, UiOptions};
use crusty_gui::id::Id;
use crusty_gui::input::{Key as UiKey, Modifiers};
use crusty_gui::math::{Color, Pos2, Rect, Vec2};
use crusty_gui::text::FontFamily;
use crusty_gui::widgets::{show_tooltip_for, Button, Canvas, CanvasScope, ScrollArea, TextEdit};
use curve_asset::Track;

use super::curve_editor::{CurveEditorState, PX_PER_SECOND, PX_PER_UNIT};
use super::theme::tokens::{grid_major, grid_minor, ramp};

// Base metrics, at UI scale 1.0. Every one of them is multiplied by the
// panel's `s` before it reaches a rect — the design system's rule that a
// fixed pixel number is a bug unless it has been through the scale.
const TOOLBAR_H: f32 = 28.0;
const SIDEBAR_W: f32 = 208.0;
/// Key dot radius in screen pixels: constant against the *view* (a keyframe is
/// a handle, not content, so it must not shrink as you zoom out) but not
/// against UI scale.
const KEY_R: f32 = 4.5;
/// Half-extent of a key's grab box. Comfortably larger than the dot without
/// making a dense track a minefield.
const KEY_HIT: f32 = 8.0;
/// Target screen spacing between axis ticks.
const TICK_PX: f32 = 76.0;
/// Screen-space step between curve samples. Two pixels reads as smooth and
/// keeps a wide view from shaping thousands of segments.
const SAMPLE_PX: f32 = 2.0;
/// The unselected tracks' contrast against the selected one.
const DIM: f32 = 0.34;
/// How long a status message stays up. The graph canvas' value, so the two
/// panels' messages behave the same.
const TOAST_MS: f32 = 1800.0;

/// What the panel wants the host to do after the frame. Save is the host's
/// job — only it can reach the plan/curve caches a written `.curve` invalidates
/// (45-A P8b, Timeline pin refresh).
#[derive(Default)]
pub struct CurveEditorOutput {
    pub save_requested: bool,
}

pub struct CurveEditorPanelCtx<'a> {
    pub state: &'a mut CurveEditorState,
    /// `selection.outline` from the live theme. Passed in for the same reason
    /// the graph canvas takes it: crusty's `Style` has no counterpart, and
    /// selection and keyboard focus are different jobs with different tokens.
    pub selection_outline: Color,
    /// This tab is the focused tab of its dock (gates keyboard editing).
    pub focused: bool,
    /// True in float windows (no menu/winit edit path) — the panel handles
    /// keyboard editing itself. False when docked in the main window, where
    /// `EditorAction` routing owns undo/redo/delete/save.
    pub handle_shortcuts: bool,
}

/// A track's plot colour: the theme ramp, strided so consecutive tracks land
/// far apart in hue. Ramp indices, never colours — the same discipline
/// `pin_color` follows.
pub fn track_color(index: usize) -> Color {
    ramp()[(index * 5) % 12].bright
}

pub fn curve_editor_panel(ui: &mut Ui, tab_rect: Rect, ctx: CurveEditorPanelCtx) -> CurveEditorOutput {
    let CurveEditorPanelCtx { state, selection_outline, focused, handle_shortcuts } = ctx;
    let mut out = CurveEditorOutput::default();

    // A release that landed while this tab was not being drawn (tab switch
    // mid-drag) leaves a gesture open; the pointer is already up on re-entry,
    // so finalize it here — before any shortcut can act across it.
    if !ui.ctx().input.pointer_down {
        state.end_drag();
    }

    let panel_id = Id::new("engine_curve_editor").with(state.path.as_str());
    let opts = UiOptions { padding: Vec2::ZERO, spacing: 0.0 };
    ui.run_at(tab_rect, Direction::TopDown, panel_id, opts, |ui| {
        let s = (ui.style().metrics.row_height / 22.0).max(0.1);

        let bar = Rect::from_min_size(tab_rect.min, Vec2::new(tab_rect.width(), TOOLBAR_H * s));
        toolbar(ui, bar, s, state, &mut out);

        let side_w = (SIDEBAR_W * s).min(tab_rect.width() * 0.5);
        let side = Rect::from_min_max(
            Pos2::new(tab_rect.min.x, bar.max.y),
            Pos2::new(tab_rect.min.x + side_w, tab_rect.max.y),
        );
        sidebar(ui, side, s, panel_id, state);

        let plot_rect =
            Rect::from_min_max(Pos2::new(side.max.x, bar.max.y), tab_rect.max);
        plot(ui, plot_rect, s, selection_outline, state);

        if handle_shortcuts && focused {
            handle_panel_keys(ui, state, &mut out);
        }
    });
    out
}

// ── toolbar ─────────────────────────────────────────────────────────────────

fn toolbar(
    ui: &mut Ui,
    rect: Rect,
    s: f32,
    state: &mut CurveEditorState,
    out: &mut CurveEditorOutput,
) {
    let st = ui.style();
    let pad = st.spacing.padding;
    let bh = st.metrics.control_height.min(rect.height() - 4.0 * s);
    let bw = st.metrics.control_height * 2.4;
    let buttons = Rect::from_min_max(
        Pos2::new(rect.max.x - (bw * 2.0 + pad * 3.0), rect.center().y - bh * 0.5),
        Pos2::new(rect.max.x - pad, rect.center().y + bh * 0.5),
    );
    {
        let mut p = ui.painter();
        p.rect_filled(rect, 0.0, st.palette.header);
        p.line_segment(
            Pos2::new(rect.min.x, rect.max.y),
            Pos2::new(rect.max.x, rect.max.y),
            st.metrics.border,
            st.palette.stroke,
        );
        // Path, then the unsaved marker in the *same* warning colour the tab
        // strip and the mesh editor use. One state, one colour, even when the
        // two are inches apart.
        let x = rect.min.x + pad;
        let ty = rect.center().y - st.fonts.mono * 0.62;
        let w = p.text_family(
            Pos2::new(x, ty),
            &state.path,
            st.fonts.mono,
            if state.dirty { st.palette.text } else { st.palette.text_secondary },
            None,
            FontFamily::Mono,
        );
        if state.dirty {
            p.text_family(
                Pos2::new(x + w.x + pad * 0.5, ty),
                "\u{2022}",
                st.fonts.mono,
                super::theme::Palette::invariant_status().warning,
                None,
                FontFamily::Mono,
            );
        }
        // The document's own length, which is what a Timeline plays. Placed
        // off the button block, so it cannot collide with it at any density.
        let dur = format!("{:.2}s", state.doc.duration());
        let w = p.measure_text_family(&dur, st.fonts.mono, None, FontFamily::Mono).x;
        p.text_family(
            Pos2::new(buttons.min.x - w - pad * 2.0, rect.center().y - st.fonts.mono * 0.62),
            &dur,
            st.fonts.mono,
            st.palette.text_secondary,
            None,
            FontFamily::Mono,
        );
    }

    ui.run_at(
        buttons,
        Direction::LeftToRight,
        Id::new("curve_toolbar_actions"),
        UiOptions { padding: Vec2::ZERO, spacing: pad },
        |ui| {
            if Button::new("Frame").exact_size(Vec2::new(bw, bh)).show(ui).clicked {
                state.frame_all();
            }
            let save = Button::new("Save")
                .exact_size(Vec2::new(bw, bh));
            let save = if state.dirty { save.primary() } else { save };
            if save.show(ui).clicked {
                out.save_requested = true;
            }
        },
    );
}

// ── track sidebar ───────────────────────────────────────────────────────────

fn sidebar(ui: &mut Ui, rect: Rect, s: f32, panel_id: Id, state: &mut CurveEditorState) {
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

    let mut add_request: Option<String> = None;
    let mut select: Option<usize> = None;
    let mut delete: Option<usize> = None;
    let mut rename_commit: Option<(usize, String)> = None;

    ui.run_at(
        Rect::from_min_max(rect.min + Vec2::splat(pad), rect.max - Vec2::splat(pad)),
        Direction::TopDown,
        panel_id.with("sidebar"),
        UiOptions { padding: Vec2::ZERO, spacing: pad * 0.6 },
        |ui| {
            let w = rect.width() - pad * 2.0;
            {
                let head = ui.allocate(Vec2::new(w, st.fonts.small * 1.8));
                let mut p = ui.painter();
                // Mono, like the variables strip's own micro-header: in this
                // editor all-caps section labels are a mono face.
                p.text_family(
                    Pos2::new(head.min.x, head.center().y - st.fonts.small * 0.62),
                    "TRACKS",
                    st.fonts.small,
                    st.palette.text_secondary,
                    None,
                    FontFamily::Mono,
                );
            }

            // Add row: name field + button. Submitting the field adds too, so
            // the keyboard path never needs the mouse.
            let row = ui.allocate(Vec2::new(w, st.metrics.control_height));
            let bw = st.metrics.control_height;
            ui.run_at(
                row,
                Direction::LeftToRight,
                panel_id.with("add_row"),
                UiOptions { padding: Vec2::ZERO, spacing: pad * 0.5 },
                |ui| {
                    let out = TextEdit::new(&mut state.new_track)
                        .hint("New track\u{2026}")
                        .width(w - bw - pad * 0.5)
                        .show_full(ui);
                    if out.submitted {
                        add_request = Some(state.new_track.clone());
                    }
                    if Button::new("+")
                        .exact_size(Vec2::new(bw, st.metrics.control_height))
                        .show(ui)
                        .clicked
                    {
                        add_request = Some(state.new_track.clone());
                    }
                },
            );

            let list_h = (rect.max.y - pad) - ui.cursor().y;
            ScrollArea::new(list_h.max(0.0))
                .inset(0.0)
                .spacing(s)
                .auto_shrink(false)
                .show(ui, |ui| {
                    for (i, t) in state.doc.tracks.iter().enumerate() {
                        let renaming =
                            state.rename.as_ref().is_some_and(|r| r.index == i);
                        let row = ui.allocate(Vec2::new(w, st.metrics.row_height));
                        if renaming {
                            // Drawn *into* the row we already allocated:
                            // `TextEdit` allocates its own rect, so letting it
                            // run in the parent flow would open a row-height
                            // hole and shove every track below it down.
                            let r = state.rename.as_mut().expect("renaming");
                            let first = r.first_frame;
                            let mut out = None;
                            ui.run_at(
                                row,
                                Direction::LeftToRight,
                                panel_id.with(("rename", i)),
                                UiOptions { padding: Vec2::ZERO, spacing: 0.0 },
                                |ui| {
                                    out = Some(
                                        TextEdit::new(&mut r.text)
                                            .width(w)
                                            .request_focus(first)
                                            .show_full(ui),
                                    );
                                },
                            );
                            let out = out.expect("rename field");
                            r.first_frame = false;
                            // Enter commits, Escape reverts, and clicking away
                            // commits too — an edit you walked away from is
                            // one you meant, and a field with no way out but
                            // the keyboard is a trap.
                            if out.submitted || (!first && !out.focused) {
                                rename_commit = Some((i, r.text.clone()));
                            } else if out.cancelled {
                                rename_commit = Some((i, String::new()));
                            }
                            continue;
                        }
                        let id = ui.alloc_id(("curve_track_row", i));
                        let resp = ui.interact(id, row);
                        if resp.clicked {
                            select = Some(i);
                        }
                        let sel = state.selected_track == i;
                        let mut p = ui.painter();
                        if sel {
                            p.rect_filled(row, st.rounding.small, st.palette.selection_fill);
                        } else if resp.hovered {
                            p.rect_filled(row, st.rounding.small, st.palette.hover);
                        }
                        // Colour chip: the same colour the plot draws the
                        // track in, so a row and its curve are one thing.
                        let c = st.fonts.small;
                        p.rect_filled(
                            Rect::from_center_size(
                                Pos2::new(row.min.x + pad * 0.5 + c * 0.5, row.center().y),
                                Vec2::splat(c),
                            ),
                            st.rounding.small,
                            track_color(i),
                        );
                        // Key count — swapped out for the delete affordance
                        // on hover, so a bare "3" and a "×" never read as one
                        // token.
                        let count = format!("{}", t.keys.len());
                        let count_w = p
                            .measure_text_family(&count, st.fonts.small, None, FontFamily::Mono)
                            .x
                            .max(st.fonts.small * 2.0);
                        let armed = state.confirm_delete == Some(i);
                        if !resp.hovered && !armed {
                            p.text_family(
                                Pos2::new(
                                    row.max.x - count_w - pad * 1.6,
                                    row.center().y - st.fonts.small * 0.62,
                                ),
                                &count,
                                st.fonts.small,
                                st.palette.text_disabled,
                                None,
                                FontFamily::Mono,
                            );
                        }
                        let label_x = row.min.x + pad + c;
                        p.text(
                            Pos2::new(label_x, row.center().y - st.fonts.body * 0.62),
                            &t.label,
                            st.fonts.body,
                            if sel { st.palette.selection_text } else { st.palette.text },
                            Some(row.width() - (label_x - row.min.x) - count_w - pad * 2.0),
                        );
                        // Delete affordance, on hover — a permanent × on every
                        // row is noise on a list this short. An *armed* row
                        // keeps its × and turns it danger-red, so the "click
                        // again" state lives where the pointer is rather than
                        // only in a message at the far side of the panel.
                        if resp.hovered || armed {
                            let x = Rect::from_center_size(
                                Pos2::new(row.max.x - pad * 0.8, row.center().y),
                                Vec2::splat(st.fonts.small * 1.4),
                            );
                            let xid = ui.alloc_id(("curve_track_del", i));
                            let xr = ui.interact(xid, x);
                            let mut p = ui.painter();
                            p.text(
                                Pos2::new(x.min.x, x.center().y - st.fonts.small * 0.62),
                                "\u{d7}",
                                st.fonts.small,
                                if armed || xr.hovered {
                                    st.palette.danger
                                } else {
                                    st.palette.text_secondary
                                },
                                None,
                            );
                            if xr.clicked {
                                delete = Some(i);
                            }
                        }
                        if resp.double_clicked(ui) {
                            state.rename = Some(super::curve_editor::TrackRename {
                                index: i,
                                text: t.label.clone(),
                                first_frame: true,
                            });
                        }
                    }
                    if state.doc.tracks.is_empty() {
                        let row = ui.allocate(Vec2::new(w, st.metrics.row_height));
                        let mut p = ui.painter();
                        p.text(
                            Pos2::new(row.min.x, row.center().y - st.fonts.small * 0.62),
                            "No tracks yet",
                            st.fonts.small,
                            st.palette.text_disabled,
                            None,
                        );
                    }
                });
        },
    );

    if let Some((i, to)) = rename_commit {
        if !to.is_empty() {
            state.rename_track(i, &to);
        }
        state.rename = None;
    }
    if let Some(i) = select {
        state.selected_track = i;
        state.selected_key = None;
    }
    if let Some(name) = add_request {
        if state.add_track(&name).is_some() {
            state.new_track.clear();
        }
    }
    if let Some(i) = delete {
        // Confirm only when there is something to lose. An empty track is a
        // typo you just made; a keyed one is work.
        let has_keys = state.doc.tracks.get(i).is_some_and(|t| !t.keys.is_empty());
        if has_keys && state.confirm_delete != Some(i) {
            state.confirm_delete = Some(i);
            let label = state.doc.tracks[i].label.clone();
            state.toast(format!("Delete '{label}' and its keys? Click \u{d7} again"));
        } else {
            state.confirm_delete = None;
            state.remove_track(i);
        }
    }
}

// ── plot ────────────────────────────────────────────────────────────────────

fn plot(ui: &mut Ui, rect: Rect, s: f32, selection: Color, state: &mut CurveEditorState) {
    let st = ui.style();
    ui.painter().rect_filled(rect, 0.0, st.palette.window);

    let mut view = state.view;
    // The canvas allocates from the *cursor*, so it needs a child Ui rooted at
    // the plot rect — otherwise it lands back at the panel's top-left and
    // paints over the sidebar.
    ui.run_at(
        rect,
        Direction::TopDown,
        Id::new("curve_plot").with(state.path.as_str()),
        UiOptions { padding: Vec2::ZERO, spacing: 0.0 },
        |ui| {
            Canvas::new()
                .size(rect.size())
                .zoom_range(ZOOM_MIN, ZOOM_MAX)
                .show(ui, &mut view, |ui, scope| {
                    draw_axes(ui, scope, s, &st);
                    draw_tracks(ui, scope, s, selection, state, &st);
                    interact(ui, scope, s, state);
                });
            plot_hint(ui, rect, state, &st);
            draw_toast(ui, rect, state);
        },
    );
    state.view = view;
    // Fitting needs the viewport, which is only known once the plot has been
    // laid out — so an open (or the Frame button) asks here and lands on the
    // next frame, the same deal `frame_all_on_open` strikes in the graph.
    if state.frame_pending {
        state.frame_pending = false;
        state.view = fit_view(state, rect.size());
    }
}

/// Zoom limits. Wider than the graph canvas's: a curve is read at both "the
/// whole timeline" and "this one key's neighbourhood".
const ZOOM_MIN: f32 = 0.05;
const ZOOM_MAX: f32 = 12.0;

/// A view that fits every key with a margin, centred. Never magnifies past
/// 4×, so a two-key curve does not fill the screen with one segment.
pub fn fit_view(state: &CurveEditorState, size: Vec2) -> crusty_gui::widgets::CanvasView {
    let mut t_max = state.doc.duration();
    let (mut v_min, mut v_max) = (0.0_f32, 0.0_f32);
    for k in state.doc.tracks.iter().flat_map(|t| t.keys.iter()) {
        v_min = v_min.min(k.value);
        v_max = v_max.max(k.value);
    }
    // Floors so an empty or flat document still gets a sane scale rather than
    // dividing by zero.
    t_max = t_max.max(1.0);
    if v_max - v_min < 1.0 {
        let c = (v_min + v_max) * 0.5;
        v_min = c - 0.5;
        v_max = c + 0.5;
    }
    let w = t_max * PX_PER_SECOND;
    let h = (v_max - v_min) * PX_PER_UNIT;
    let zoom = ((size.x / w).min(size.y / h) * 0.82).clamp(ZOOM_MIN, 4.0);
    // Centre the content box in the viewport: pan is the world point at the
    // canvas's top-left corner.
    let cx = w * 0.5;
    let cy = -(v_min + v_max) * 0.5 * PX_PER_UNIT;
    crusty_gui::widgets::CanvasView {
        pan: Vec2::new(cx - size.x * 0.5 / zoom, cy - size.y * 0.5 / zoom),
        zoom,
    }
}

/// Screen position of `(t, value)`.
fn to_screen(scope: &CanvasScope, t: f32, value: f32) -> Pos2 {
    scope.world_to_screen(Pos2::new(t * PX_PER_SECOND, -value * PX_PER_UNIT))
}

/// `(t, value)` at a screen position.
fn to_curve(scope: &CanvasScope, p: Pos2) -> (f32, f32) {
    let w = scope.screen_to_world(p);
    (w.x / PX_PER_SECOND, -w.y / PX_PER_UNIT)
}

/// A "nice" tick step (1/2/5 × 10ⁿ) at least `min` wide.
fn nice_step(min: f32) -> f32 {
    if min <= 0.0 || !min.is_finite() {
        return 1.0;
    }
    let mag = 10.0_f32.powf(min.log10().floor());
    for m in [1.0, 2.0, 5.0, 10.0] {
        if mag * m >= min {
            return mag * m;
        }
    }
    mag * 10.0
}

fn draw_axes(ui: &mut Ui, scope: &CanvasScope, s: f32, st: &crusty_gui::style::Style) {
    let r = scope.rect();
    let (t0, v1) = to_curve(scope, r.min);
    let (t1, v0) = to_curve(scope, r.max);
    let t_step = nice_step(TICK_PX * s / (PX_PER_SECOND * scope.zoom()));
    let v_step = nice_step(TICK_PX * s / (PX_PER_UNIT * scope.zoom()));
    let small = st.fonts.small;
    let mut p = ui.painter();

    // Time ticks + labels along the bottom.
    let mut i = (t0 / t_step).floor();
    while i * t_step <= t1 {
        let t = i * t_step;
        i += 1.0;
        let x = to_screen(scope, t, 0.0).x;
        if x < r.min.x || x > r.max.x {
            continue;
        }
        let zero = t.abs() < t_step * 0.5;
        p.line_segment(
            Pos2::new(x, r.min.y),
            Pos2::new(x, r.max.y),
            st.metrics.border,
            if zero { grid_major() } else { grid_minor() },
        );
        let label = format!("{t:.*}s", decimals(t_step));
        p.text_family(
            Pos2::new(x + small * 0.3, r.max.y - small * 1.6),
            &label,
            small,
            st.palette.text_secondary,
            None,
            FontFamily::Mono,
        );
    }

    // Value ticks + labels down the left edge.
    let mut j = (v0 / v_step).floor();
    while j * v_step <= v1 {
        let v = j * v_step;
        j += 1.0;
        let y = to_screen(scope, 0.0, v).y;
        if y < r.min.y || y > r.max.y {
            continue;
        }
        let zero = v.abs() < v_step * 0.5;
        p.line_segment(
            Pos2::new(r.min.x, y),
            Pos2::new(r.max.x, y),
            st.metrics.border,
            if zero { grid_major() } else { grid_minor() },
        );
        let label = format!("{v:.*}", decimals(v_step));
        p.text_family(
            Pos2::new(r.min.x + small * 0.4, y - small * 1.3),
            &label,
            small,
            st.palette.text_secondary,
            None,
            FontFamily::Mono,
        );
    }
}

/// Decimals a tick label needs at this step — 0.5 wants one, 5 wants none.
fn decimals(step: f32) -> usize {
    if step >= 1.0 {
        0
    } else if step >= 0.1 {
        1
    } else {
        2
    }
}

fn draw_tracks(
    ui: &mut Ui,
    scope: &CanvasScope,
    s: f32,
    selection: Color,
    state: &CurveEditorState,
    st: &crusty_gui::style::Style,
) {
    let key_r = KEY_R * s;
    let r = scope.rect();
    let (t0, _) = to_curve(scope, r.min);
    let (t1, _) = to_curve(scope, r.max);
    // Unselected tracks first, so the one being edited draws on top.
    let order: Vec<usize> = (0..state.doc.tracks.len())
        .filter(|i| *i != state.selected_track)
        .chain(std::iter::once(state.selected_track))
        .collect();

    for i in order {
        let Some(track) = state.doc.tracks.get(i) else { continue };
        let sel = i == state.selected_track;
        let color = track_color(i);
        let color = if sel { color } else { color.with_alpha(DIM) };
        let pts = sample_polyline(track, scope, t0, t1);
        if pts.len() >= 2 {
            let mut p = ui.painter();
            p.polyline(&pts, if sel { 1.8 * s } else { 1.2 * s }, color);
        }
        if !sel {
            continue;
        }
        let mut p = ui.painter();
        for (k, key) in track.keys.iter().enumerate() {
            let c = to_screen(scope, key.t, key.value);
            if !r.contains(c) {
                continue;
            }
            p.circle_filled(c, key_r, color);
            // The dot always carries a rim so it reads against the curve it
            // sits on; selection swaps the rim for the theme's selection
            // outline and adds a halo, which is legible at 4px where a fill
            // change is not.
            if state.selected_key == Some((i, k)) {
                p.circle_stroke(c, key_r + 2.6 * s, 1.6 * s, selection);
                p.circle_stroke(c, key_r, 1.6 * s, selection);
            } else {
                p.circle_stroke(c, key_r, st.metrics.border, st.palette.window);
            }
        }
    }
}

/// Sample a track across the visible time range, at the curve's own keys plus
/// a screen-space step between them. `Track::sample` is the interpreter's
/// function, never a local reimplementation.
fn sample_polyline(track: &Track, scope: &CanvasScope, t0: f32, t1: f32) -> Vec<Pos2> {
    if track.keys.is_empty() {
        return Vec::new();
    }
    let dt = SAMPLE_PX / (PX_PER_SECOND * scope.zoom()).max(1.0e-3);
    let mut ts: Vec<f32> = Vec::new();
    let mut t = t0;
    while t < t1 {
        ts.push(t);
        t += dt;
    }
    ts.push(t1);
    // Land on both sides of every visible key. A `Constant` segment ends in a
    // genuine discontinuity: sampling only *at* the key draws the riser as a
    // diagonal across one sample step, so the pair (just-before, exactly-on)
    // is what makes a hold read as a hold.
    let eps = (t1 - t0) * 1.0e-4;
    for k in track.keys.iter().map(|k| k.t).filter(|k| *k >= t0 && *k <= t1) {
        ts.push((k - eps).max(t0));
        ts.push(k);
    }
    ts.sort_by(f32::total_cmp);
    ts.iter().map(|t| to_screen(scope, *t, track.sample(*t))).collect()
}

fn interact(ui: &mut Ui, scope: &CanvasScope, s: f32, state: &mut CurveEditorState) {
    let hit_r = KEY_HIT * s;
    // Escape abandons a live drag and puts the key back — a canvas gesture,
    // not a bound action, so it is handled here rather than in the keymap and
    // works identically docked and floated.
    if state.drag.is_some() && ui.ctx().input.key_pressed(UiKey::Escape) {
        state.cancel_gestures();
        return;
    }
    let ti = state.selected_track;
    let Some(track) = state.doc.tracks.get(ti) else { return };
    let r = scope.rect();
    let hit: Vec<(usize, Rect)> = track
        .keys
        .iter()
        .enumerate()
        .map(|(k, key)| {
            let c = to_screen(scope, key.t, key.value);
            (k, Rect::from_center_size(c, Vec2::splat(hit_r * 2.0)))
        })
        .filter(|(_, b)| r.intersect(*b).width() > 0.0)
        .collect();

    // A live drag owns the pointer: no hit test can steal it mid-gesture.
    if state.drag.is_some() {
        if let Some(p) = ui.ctx().input.pointer_pos {
            let (t, v) = to_curve(scope, p);
            state.drag_to(t, v);
        }
        if !ui.ctx().input.pointer_down {
            state.end_drag();
        }
        return;
    }

    let mut hovered_key: Option<usize> = None;
    for (k, b) in &hit {
        let id = ui.alloc_id(("curve_key", ti, *k));
        let resp = ui.interact(id, *b);
        if resp.hovered {
            hovered_key = Some(*k);
        }
        if resp.pressed {
            state.begin_drag(ti, *k);
            return;
        }
        if resp.clicked {
            state.selected_key = Some((ti, *k));
        }
    }

    if let Some(k) = hovered_key {
        let key = track.keys[k];
        let text = format!("t {:.3}s  v {:.3}  \u{2014}  {}", key.t, key.value, key.interp.label());
        let anchor = Rect::from_center_size(
            to_screen(scope, key.t, key.value),
            Vec2::splat(hit_r * 2.0),
        );
        show_tooltip_for(ui, anchor, &text);
    }

    // Right-click on a key cycles its interpolation. `CanvasScope` has already
    // decided this release was a click and not a pan.
    if let Some(p) = scope.right_clicked() {
        if let Some((k, _)) = hit.iter().find(|(_, b)| b.contains(p)) {
            state.cycle_interp(ti, *k);
            return;
        }
    }

    // Double-click on empty plot adds a key at the cursor.
    let bg = ui.alloc_id("curve_plot_bg");
    let resp = ui.interact(bg, r);
    if resp.double_clicked(ui) && hovered_key.is_none() {
        if let Some(p) = ui.ctx().input.pointer_pos {
            let (t, v) = to_curve(scope, p);
            state.add_key(ti, t, v);
        }
    } else if resp.clicked && hovered_key.is_none() {
        // Clicking a dimmed curve selects its track — the editor-wide "click
        // the thing to select it" grammar, so the sidebar is not the only way
        // in. Nothing under the cursor just clears the key selection.
        match ui.ctx().input.pointer_pos.and_then(|p| track_under(scope, s, state, p)) {
            Some(i) => {
                state.selected_track = i;
                state.selected_key = None;
            }
            None => state.selected_key = None,
        }
    }
}

/// The topmost *unselected* track whose curve passes within a grab radius of
/// `p`. Vertical distance at the cursor's own time — the curve is a function
/// of t, so that is the whole test.
fn track_under(
    scope: &CanvasScope,
    s: f32,
    state: &CurveEditorState,
    p: Pos2,
) -> Option<usize> {
    let (t, _) = to_curve(scope, p);
    let grab = KEY_HIT * s;
    state
        .doc
        .tracks
        .iter()
        .enumerate()
        .filter(|(i, tr)| *i != state.selected_track && !tr.keys.is_empty())
        .map(|(i, tr)| (i, (to_screen(scope, t, tr.sample(t)).y - p.y).abs()))
        .filter(|(_, d)| *d <= grab)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

/// What to do when there is nothing to look at. A bare grid is
/// indistinguishable from a broken panel, and double-click-to-add is the one
/// gesture here that nothing else announces — so the empty states are where
/// it gets said.
fn plot_hint(
    ui: &mut Ui,
    rect: Rect,
    state: &CurveEditorState,
    st: &crusty_gui::style::Style,
) {
    let hint = if state.doc.tracks.is_empty() {
        "No tracks \u{2014} add one on the left"
    } else if state
        .doc
        .tracks
        .get(state.selected_track)
        .is_some_and(|t| t.keys.is_empty())
    {
        "Empty track \u{2014} double-click to add a key"
    } else {
        return;
    };
    let font = st.fonts.body;
    let mut p = ui.painter();
    let w = p.measure_text(hint, font, None).x;
    p.text(
        Pos2::new(rect.center().x - w * 0.5, rect.center().y - font * 0.62),
        hint,
        font,
        st.palette.text_disabled,
        None,
    );
}

/// The transient status line: what the last gesture did, and the
/// confirm-again prompt a keyed track's delete puts up. Bottom-centre and
/// fading, the same place and curve the graph canvas uses — one message at a
/// time rather than a stack, because this panel's gestures are one at a time.
///
/// The message is also the *lifetime* of an armed delete confirmation: when it
/// fades, the confirmation disarms. A prompt you can no longer read must not
/// still be listening.
fn draw_toast(ui: &mut Ui, rect: Rect, state: &mut CurveEditorState) {
    let Some((text, at)) = state.toast.clone() else {
        return;
    };
    let age = at.elapsed().as_millis() as f32 / TOAST_MS;
    if age >= 1.0 {
        state.toast = None;
        state.confirm_delete = None;
        return;
    }
    // Hold, then fade over the last third — the graph canvas' curve, so the
    // two panels' messages behave identically.
    let alpha = (1.0 - (age - 0.66) / 0.34).clamp(0.0, 1.0);
    let st = ui.style();
    let font = st.fonts.small;
    let pad = st.spacing.padding;
    let h = st.metrics.control_height;
    let mut p = ui.painter();
    let w = p.measure_text(&text, font, None).x + pad * 2.0;
    let chip = Rect::from_min_size(
        Pos2::new(rect.center().x - w * 0.5, rect.max.y - pad - h),
        Vec2::new(w, h),
    );
    p.rect_filled(chip, st.rounding.small, st.palette.elevated.with_alpha(alpha));
    p.rect_stroke(
        chip,
        st.rounding.small,
        st.metrics.border,
        st.palette.stroke_strong.with_alpha(alpha),
    );
    p.text(
        Pos2::new(chip.min.x + pad, chip.center().y - font * 0.62),
        &text,
        font,
        st.palette.text.with_alpha(alpha),
        None,
    );
}

// ── keyboard (float windows only) ───────────────────────────────────────────

fn handle_panel_keys(ui: &Ui, state: &mut CurveEditorState, out: &mut CurveEditorOutput) {
    let input = &ui.ctx().input;
    if ui.ctx().text_focused() || ui.ctx().modal_any_open() || state.rename.is_some() {
        // A focused text field owns the keyboard: Del must delete a character.
        return;
    }
    let ctrl = input.modifiers == Modifiers::CTRL;
    if ctrl && input.key_pressed(UiKey::Char('z')) {
        state.undo();
    }
    if (ctrl && input.key_pressed(UiKey::Char('y')))
        || (input.modifiers == Modifiers::CTRL.union(Modifiers::SHIFT)
            && input.key_pressed(UiKey::Char('z')))
    {
        state.redo();
    }
    if ctrl && input.key_pressed(UiKey::Char('s')) {
        out.save_requested = true;
    }
    if input.key_pressed(UiKey::Delete) {
        if let Some((t, k)) = state.selected_key {
            state.remove_key(t, k);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::editor::curve_editor::CurveEditorState;
    use curve_asset::{CurveDoc, Interp, Key, Track};

    fn doc() -> CurveDoc {
        let mut d = CurveDoc::default();
        let mut t = Track::new("h", "H");
        t.keys = vec![
            Key { t: 0.0, value: -1.0, interp: Interp::Linear },
            Key { t: 2.0, value: 3.0, interp: Interp::Linear },
        ];
        d.tracks = vec![t];
        d
    }

    /// Tick steps are 1/2/5 × 10ⁿ and never finer than asked for — the
    /// property that keeps labels from colliding at any zoom.
    #[test]
    fn tick_steps_are_nice_and_never_too_fine() {
        for min in [0.003_f32, 0.02, 0.3, 1.0, 3.0, 47.0, 1200.0] {
            let s = nice_step(min);
            assert!(s >= min, "{s} < {min}");
            let m = s / 10.0_f32.powf(s.log10().floor());
            assert!(
                [1.0, 2.0, 5.0].iter().any(|k| (k - m).abs() < 1e-3),
                "{s} is not a 1/2/5 step (mantissa {m})"
            );
        }
        // Degenerate input answers rather than dividing by zero.
        assert_eq!(nice_step(0.0), 1.0);
        assert_eq!(decimals(5.0), 0);
        assert_eq!(decimals(0.5), 1);
        assert_eq!(decimals(0.05), 2);
    }

    /// A fit puts every key on screen with margin, centred, and never
    /// magnifies past 4×.
    #[test]
    fn fit_view_frames_every_key() {
        let st = CurveEditorState::from_doc(doc(), "t.curve");
        let size = Vec2::new(800.0, 600.0);
        let view = fit_view(&st, size);
        let scope_min = view.pan;
        let scope_max = view.pan + size / view.zoom;
        for k in st.doc.tracks[0].keys.iter() {
            let (x, y) = (k.t * PX_PER_SECOND, -k.value * PX_PER_UNIT);
            assert!(x >= scope_min.x && x <= scope_max.x, "t {} off screen", k.t);
            assert!(y >= scope_min.y && y <= scope_max.y, "v {} off screen", k.value);
        }
        assert!(view.zoom <= 4.0 + 1e-4);

        // An empty document still gets a usable scale rather than NaN.
        let empty = CurveEditorState::from_doc(CurveDoc::default(), "e.curve");
        let v = fit_view(&empty, size);
        assert!(v.zoom.is_finite() && v.zoom > 0.0);
        assert!(v.pan.x.is_finite() && v.pan.y.is_finite());
    }

    /// Track colours come off the ramp and adjacent tracks are distinct —
    /// the sidebar chip and the plot line are the same lookup, so a
    /// collision would make two tracks unreadable in both places at once.
    #[test]
    fn track_colors_are_ramp_entries_and_neighbours_differ() {
        let ramp = ramp();
        for i in 0..24 {
            let c = track_color(i);
            assert!(
                ramp.iter().any(|h| h.bright == c),
                "track {i} is not a ramp hue"
            );
            assert_ne!(c, track_color(i + 1), "tracks {i} and {} collide", i + 1);
        }
    }
}
