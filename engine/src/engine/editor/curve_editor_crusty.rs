//! Curve editor panel rendered with crusty-gui (Task 45-A P8b, restructured
//! in GS-5b).
//!
//! Four regions, top to bottom: a **toolbar** (framing, snap toggles, the
//! time/loop readout), a **track sidebar** (ramp colour, counts, eye + lock),
//! a pan/zoom **plot** on crusty's [`Canvas`], and a **footer detail bar** that
//! gives every gesture a visible, typeable equivalent.
//!
//! Plot world units are pixels at zoom 1: `x = t × PX_PER_SECOND`,
//! `y = −value × PX_PER_UNIT` (screen y grows down, values grow up). Every
//! sample drawn comes from [`curve_asset::Track::sample`] — the same function
//! the interpreter calls — so what the author sees is what the Timeline plays,
//! and the playhead's readouts are literally the runtime's numbers.
//!
//! Gestures: drag keys (the whole selection moves as a body, time clamped
//! against the keys that are *not* moving), box-select on empty plot, Shift to
//! extend, double-click to add, `Del` to remove the selection, drag a tangent
//! arm to shape a cubic segment, right-click for the key menu (which replaced
//! P8b's blind interpolation cycle). Esc abandons whatever is in flight.
//!
//! Policy lives in `curve_editor`: glyphs, footer state, snapping and clamping
//! are decided — and tested — there. This file draws what those say.

use crusty_gui::context::{Direction, Ui, UiOptions};
use crusty_gui::id::Id;
use crusty_gui::input::{Key as UiKey, Modifiers};
use crusty_gui::math::{Color, Pos2, Rect, Rounding, Vec2};
use crusty_gui::paint::Painter;
use crusty_gui::style::Style;
use crusty_gui::text::FontFamily;
use crusty_gui::widgets::{show_tooltip_for, Button, Canvas, CanvasScope, ScrollArea, TextEdit};
use curve_asset::{Interp, Track};

use super::curve_editor::{
    key_glyph, ArmSide, CurveEditorState, FooterField, FooterState, KeyGlyph, TangentMode,
    MIN_KEY_GAP, PX_PER_SECOND, SNAP_TIME, SNAP_VALUE, VALUE_PX_MAX, VALUE_PX_MIN,
};
use super::theme::tokens::{grid_major, grid_minor, ramp};
use super::widgets::segmented_control;

// Base metrics, at UI scale 1.0. Every one of them is multiplied by the
// panel's `s` before it reaches a rect — the design system's rule that a
// fixed pixel number is a bug unless it has been through the scale.
const TOOLBAR_H: f32 = 28.0;
const FOOTER_H: f32 = 34.0;
const SIDEBAR_W: f32 = 210.0;
/// The strip along the top of the plot that carries the loop wash and owns the
/// playhead drag.
const AXIS_STRIP_H: f32 = 16.0;
/// Key glyph radius in screen pixels: constant against the *view* (a keyframe
/// is a handle, not content, so it must not shrink as you zoom out) but not
/// against UI scale.
const KEY_R: f32 = 4.5;
/// Half-extent of a key's grab box. Comfortably larger than the glyph without
/// making a dense track a minefield.
const KEY_HIT: f32 = 8.0;
/// Screen length of a tangent arm. Fixed, because an arm is a handle: its
/// *angle* is the data, and giving it a length would imply weighted tangents,
/// which the design explicitly punted.
const ARM_LEN: f32 = 46.0;
/// Target screen spacing between axis ticks. Generous on purpose: the grid is
/// a backdrop for curves and handles, and a half-second cadence competes with
/// both.
const TICK_PX: f32 = 112.0;
/// Screen-space step between curve samples. Two pixels reads as smooth and
/// keeps a wide view from shaping thousands of segments.
const SAMPLE_PX: f32 = 2.0;
/// The unselected tracks' contrast against the selected one — applied to
/// strokes *and* fills, so a dimmed track dims as one thing (the GS-2 lesson:
/// a half-dimmed row reads as a rendering bug).
const DIM: f32 = 0.4;
/// Movement before a press becomes a box select rather than a click.
const MARQUEE_ARM_PX: f32 = 4.0;
/// Marquee fill alpha — the graph canvas' value, so the two sweeps match.
const MARQUEE_FILL_ALPHA: f32 = 0.08;
/// The loop-extent wash on the axis strip. Small because the compositor
/// blends in **linear** light: measured on a capture, 0.10 here lands at ~25%
/// against the plot's near-black — a band, not a wash. This is the GS-2
/// lesson, priced.
const LOOP_WASH_ALPHA: f32 = 0.025;
const LOOP_LINE_ALPHA: f32 = 0.35;
/// A ghost arm (a key whose mode is derived, not hand-set) against a real one.
const GHOST_ARM_ALPHA: f32 = 0.55;
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
    /// undo/redo/save/delete itself. False when docked in the main window,
    /// where `EditorAction` routing owns them. The key menu's *numeric*
    /// shortcuts are the panel's either way: nothing else in the editor binds
    /// bare digits, and they only mean anything with keys selected.
    pub handle_shortcuts: bool,
}

/// A track's plot colour.
///
/// Anchored on the **Float pin hue** — a curve track *is* a float output pin
/// on a Timeline node, so a track and the pin it becomes read as the same
/// thing — then strided by 5, which is coprime with 12 and so walks all twelve
/// hues before repeating while leaving neighbours ~150° apart. Ramp indices,
/// never colours: the same discipline `pin_color` follows.
pub fn track_color(index: usize) -> Color {
    ramp()[(FLOAT_HUE + index * 5) % 12].bright
}

/// `PALETTES.pins[Float]` — kept as a named constant here rather than reaching
/// through `pin_color`, which needs a `PinType` and a registry to answer.
const FLOAT_HUE: usize = 4;

pub fn curve_editor_panel(ui: &mut Ui, tab_rect: Rect, ctx: CurveEditorPanelCtx) -> CurveEditorOutput {
    let CurveEditorPanelCtx { state, selection_outline, focused, handle_shortcuts } = ctx;
    let mut out = CurveEditorOutput::default();

    // A release that landed while this tab was not being drawn (tab switch
    // mid-drag) leaves a gesture open; the pointer is already up on re-entry,
    // so finalize it here — before any shortcut can act across it.
    if !ui.ctx().input.pointer_down {
        state.end_drag();
        state.end_arm_drag();
        state.marquee = None;
        state.playhead_drag = false;
    }

    let panel_id = Id::new("engine_curve_editor").with(state.path.as_str());
    let opts = UiOptions { padding: Vec2::ZERO, spacing: 0.0 };
    ui.run_at(tab_rect, Direction::TopDown, panel_id, opts, |ui| {
        let s = (ui.style().metrics.row_height / 22.0).max(0.1);

        let bar = Rect::from_min_size(tab_rect.min, Vec2::new(tab_rect.width(), TOOLBAR_H * s));
        toolbar(ui, bar, s, state, &mut out);

        let foot = Rect::from_min_max(
            Pos2::new(tab_rect.min.x, tab_rect.max.y - FOOTER_H * s),
            tab_rect.max,
        );

        let side_w = (SIDEBAR_W * s).min(tab_rect.width() * 0.5);
        let side = Rect::from_min_max(
            Pos2::new(tab_rect.min.x, bar.max.y),
            Pos2::new(tab_rect.min.x + side_w, foot.min.y),
        );
        sidebar(ui, side, s, panel_id, state);

        let plot_rect =
            Rect::from_min_max(Pos2::new(side.max.x, bar.max.y), Pos2::new(tab_rect.max.x, foot.min.y));
        let menu_at = plot(ui, plot_rect, s, selection_outline, state);
        key_menu(ui, state, menu_at);

        footer(ui, foot, s, panel_id, state);

        if focused {
            handle_panel_keys(ui, state, handle_shortcuts, &mut out);
        }
    });
    out
}

// ── shared bar chrome ───────────────────────────────────────────────────────

/// A toolbar pill: the design system's toggled-tool treatment when `on`, a
/// quiet bordered control when not. Actions (Fit all / Fit track) use it with
/// `on = false` — same family, same height, no second vocabulary in one bar.
fn pill(
    ui: &mut Ui,
    id_source: (&str, usize),
    rect: Rect,
    label: &str,
    mono: Option<&str>,
    on: bool,
) -> bool {
    let st = ui.style();
    let id = ui.alloc_id(id_source);
    let resp = ui.interact(id, rect);
    let pad = st.spacing.padding;
    let mut p = ui.painter();
    if on {
        p.rect_filled(rect, st.rounding.small, st.palette.accent_soft);
    } else if resp.hovered {
        p.rect_filled(rect, st.rounding.small, st.palette.hover);
    }
    p.rect_stroke(
        rect,
        st.rounding.small,
        st.metrics.border,
        if on { st.palette.accent_active } else { st.palette.stroke },
    );
    let color = if on {
        st.palette.accent_active
    } else if resp.hovered {
        st.palette.text
    } else {
        st.palette.text_secondary
    };
    let font = st.fonts.small;
    let mut x = rect.min.x + pad;
    let y = rect.center().y - font * 0.62;
    let w = p.text(Pos2::new(x, y), label, font, color, None).x;
    if let Some(m) = mono {
        x += w + pad * 0.5;
        p.text_family(Pos2::new(x, y), m, font, color, None, FontFamily::Mono);
    }
    resp.clicked
}

fn pill_width(ui: &mut Ui, label: &str, mono: Option<&str>, st: &Style) -> f32 {
    let pad = st.spacing.padding;
    let mut p = ui.painter();
    let mut w = p.measure_text(label, st.fonts.small, None).x;
    if let Some(m) = mono {
        w += pad * 0.5 + p.measure_text_family(m, st.fonts.small, None, FontFamily::Mono).x;
    }
    w + pad * 2.0
}

/// The 1px vertical rule that separates groups in a bar.
fn bar_divider(p: &mut Painter, x: f32, rect: Rect, st: &Style) {
    let h = rect.height() * 0.5;
    p.line_segment(
        Pos2::new(x, rect.center().y - h * 0.5),
        Pos2::new(x, rect.center().y + h * 0.5),
        st.metrics.border,
        st.palette.stroke,
    );
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
    let bh = (st.metrics.control_height * 0.8).min(rect.height() - 4.0 * s);
    let save_w = st.metrics.control_height * 2.4;
    {
        let mut p = ui.painter();
        p.rect_filled(rect, 0.0, st.palette.header);
        p.line_segment(
            Pos2::new(rect.min.x, rect.max.y),
            Pos2::new(rect.max.x, rect.max.y),
            st.metrics.border,
            st.palette.stroke,
        );
    }

    // Document identity, then the unsaved marker in the *same* warning colour
    // the tab strip and the mesh editor use. One state, one colour, even when
    // the two are inches apart.
    let mut x = rect.min.x + pad;
    {
        let mut p = ui.painter();
        let ty = rect.center().y - st.fonts.small * 0.62;
        let w = p.text(
            Pos2::new(x, ty),
            &state.path,
            st.fonts.small,
            if state.dirty { st.palette.text } else { st.palette.text_secondary },
            None,
        );
        x += w.x + pad * 0.6;
        if state.dirty {
            p.circle_filled(
                Pos2::new(x + 2.5 * s, rect.center().y),
                2.5 * s,
                super::theme::Palette::invariant_status().warning,
            );
            x += 6.0 * s;
        }
        x += pad;
        bar_divider(&mut p, x, rect, &st);
        x += pad;
    }

    let y = rect.center().y - bh * 0.5;
    for (i, (label, track_only)) in [("Fit all", false), ("Fit track", true)].iter().enumerate() {
        let w = pill_width(ui, label, None, &st);
        let r = Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, bh));
        if pill(ui, ("curve_fit", i), r, label, None, false) {
            if *track_only {
                state.frame_track();
            } else {
                state.frame_all();
            }
        }
        x += w + pad * 0.5;
    }

    {
        x += pad * 0.5;
        let mut p = ui.painter();
        bar_divider(&mut p, x, rect, &st);
        x += pad;
    }

    // Snap toggles own the resting state; the hint says what Ctrl does, since
    // an inversion is not something a toggle can show.
    let t_label = format!("{SNAP_TIME:.1}s");
    let w = pill_width(ui, "Snap", Some(&t_label), &st);
    let r = Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, bh));
    if pill(ui, ("curve_snap", 0), r, "Snap", Some(&t_label), state.snap.time) {
        state.snap.time = !state.snap.time;
    }
    x += w + pad * 0.5;
    let v_label = format!("{SNAP_VALUE:.2}");
    let w = pill_width(ui, "Snap", Some(&v_label), &st);
    let r = Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, bh));
    if pill(ui, ("curve_snap", 1), r, "Snap", Some(&v_label), state.snap.value) {
        state.snap.value = !state.snap.value;
    }
    x += w + pad;

    // Right end: the readout, then Save. Laid out from the right edge so the
    // hint in the middle is the only thing that ever gets squeezed.
    let readout = format!("t {:.2}  \u{b7}  loop {:.1}s", state.playhead, state.doc.duration());
    let save = Rect::from_min_size(
        Pos2::new(rect.max.x - pad - save_w, rect.center().y - bh * 0.5),
        Vec2::new(save_w, bh),
    );
    {
        let mut p = ui.painter();
        let rw = p
            .measure_text_family(&readout, st.fonts.small, None, FontFamily::Mono)
            .x;
        let rx = save.min.x - pad * 1.5 - rw;
        p.text_family(
            Pos2::new(rx, rect.center().y - st.fonts.small * 0.62),
            &readout,
            st.fonts.small,
            st.palette.text_mono,
            None,
            FontFamily::Mono,
        );
        // The hint fills what is left, and is dropped rather than truncated
        // when the bar is tight.
        let hint = "hold Ctrl to invert";
        let hw = p.measure_text(hint, st.fonts.small, None).x;
        if rx - x > hw + pad * 2.0 {
            p.text(
                Pos2::new(x, rect.center().y - st.fonts.small * 0.62),
                hint,
                st.fonts.small,
                st.palette.text_disabled,
                None,
            );
        }
    }
    ui.run_at(
        save,
        Direction::LeftToRight,
        Id::new("curve_toolbar_save"),
        UiOptions { padding: Vec2::ZERO, spacing: 0.0 },
        |ui| {
            let b = Button::new("Save").exact_size(save.size());
            let b = if state.dirty { b.primary() } else { b };
            if b.show(ui).clicked {
                out.save_requested = true;
            }
        },
    );
}

// ── track sidebar ───────────────────────────────────────────────────────────

/// An eye, drawn rather than fetched: the icon registry is a texture service
/// the panel does not take, and two 12px glyphs are not worth plumbing it for.
fn draw_eye(p: &mut Painter, c: Pos2, r: f32, w: f32, color: Color, open: bool) {
    p.bezier_quadratic(
        Pos2::new(c.x - r, c.y),
        Pos2::new(c.x, c.y - r * 1.5),
        Pos2::new(c.x + r, c.y),
        w,
        color,
    );
    p.bezier_quadratic(
        Pos2::new(c.x - r, c.y),
        Pos2::new(c.x, c.y + r * 1.5),
        Pos2::new(c.x + r, c.y),
        w,
        color,
    );
    if open {
        p.circle_filled(c, r * 0.36, color);
    } else {
        // Struck through: the lens is still there, it is just not looking.
        p.line_segment(
            Pos2::new(c.x - r * 0.9, c.y + r * 0.9),
            Pos2::new(c.x + r * 0.9, c.y - r * 0.9),
            w,
            color,
        );
    }
}

/// A padlock. Closed = filled body + closed shackle; open = outlined body and
/// a shackle that has swung off one shoulder.
fn draw_lock(p: &mut Painter, c: Pos2, r: f32, w: f32, color: Color, locked: bool) {
    let body = Rect::from_min_max(
        Pos2::new(c.x - r * 0.8, c.y - r * 0.1),
        Pos2::new(c.x + r * 0.8, c.y + r * 0.9),
    );
    if locked {
        p.rect_filled(body, 0.0, color);
        p.bezier_quadratic(
            Pos2::new(c.x - r * 0.5, c.y - r * 0.1),
            Pos2::new(c.x, c.y - r * 1.4),
            Pos2::new(c.x + r * 0.5, c.y - r * 0.1),
            w,
            color,
        );
    } else {
        p.rect_stroke(body, 0.0, w, color);
        p.bezier_quadratic(
            Pos2::new(c.x - r * 0.5, c.y - r * 0.1),
            Pos2::new(c.x - r * 0.2, c.y - r * 1.4),
            Pos2::new(c.x + r * 0.6, c.y - r * 0.7),
            w,
            color,
        );
    }
}

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
    let mut hide: Option<usize> = None;
    let mut lock: Option<usize> = None;
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

            let list_h = (rect.max.y - pad) - ui.cursor().y - st.metrics.control_height - pad;
            ScrollArea::new(list_h.max(0.0))
                .inset(0.0)
                .spacing(s)
                .auto_shrink(false)
                .show(ui, |ui| {
                    for i in 0..state.doc.tracks.len() {
                        let renaming = state.rename.as_ref().is_some_and(|r| r.index == i);
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
                        let armed = state.confirm_delete == Some(i);
                        let hidden = state.is_hidden(i);
                        let locked = state.is_locked(i);
                        let (label, keys, dur) = {
                            let t = &state.doc.tracks[i];
                            (t.label.clone(), t.keys.len(), t.duration())
                        };
                        {
                            let mut p = ui.painter();
                            if sel {
                                p.rect_filled(row, st.rounding.small, st.palette.selection_fill);
                            } else if resp.hovered {
                                p.rect_filled(row, st.rounding.small, st.palette.hover);
                            }
                            // Colour bar: the same lookup the plot strokes the
                            // curve with, so a row and its curve are one
                            // thing. Dimmed with the track it names.
                            let c = track_color(i);
                            let c = if hidden { c.with_alpha(DIM) } else { c };
                            p.rect_filled(
                                Rect::from_center_size(
                                    Pos2::new(row.min.x + pad * 0.5, row.center().y),
                                    Vec2::new(3.0 * s, row.height() * 0.62),
                                ),
                                st.rounding.small,
                                c,
                            );
                        }

                        // Right-hand controls, laid out from the edge inward:
                        // [×/Delete?] [lock] [eye] [counts].
                        let ih = st.fonts.body;
                        let mut rx = row.max.x - pad * 0.4;
                        if resp.hovered || armed {
                            let (dw, dh) = if armed {
                                (
                                    {
                                        let mut p = ui.painter();
                                        p.measure_text("Delete?", st.fonts.small, None).x
                                            + pad
                                    },
                                    row.height() * 0.72,
                                )
                            } else {
                                (ih, ih)
                            };
                            let x = Rect::from_min_size(
                                Pos2::new(rx - dw, row.center().y - dh * 0.5),
                                Vec2::new(dw, dh),
                            );
                            let xid = ui.alloc_id(("curve_track_del", i));
                            let xr = ui.interact(xid, x);
                            let mut p = ui.painter();
                            if armed {
                                // The armed state names itself where the
                                // pointer is, rather than only in a message at
                                // the far side of the panel.
                                p.rect_filled(x, st.rounding.small, st.palette.danger);
                                p.text(
                                    Pos2::new(
                                        x.min.x + pad * 0.5,
                                        x.center().y - st.fonts.small * 0.62,
                                    ),
                                    "Delete?",
                                    st.fonts.small,
                                    // Not `accent_text`: that token is dark by
                                    // design (it sits on the bright accent),
                                    // and dark-on-danger is the one place it
                                    // fails its own contrast job.
                                    st.palette.text,
                                    None,
                                );
                            } else {
                                p.text(
                                    Pos2::new(x.min.x, x.center().y - st.fonts.small * 0.62),
                                    "\u{d7}",
                                    st.fonts.small,
                                    if xr.hovered {
                                        st.palette.danger
                                    } else {
                                        st.palette.text_secondary
                                    },
                                    None,
                                );
                            }
                            if xr.clicked {
                                delete = Some(i);
                            }
                            rx -= dw + pad * 0.4;
                        }

                        let lock_rect =
                            Rect::from_center_size(Pos2::new(rx - ih * 0.5, row.center().y), Vec2::splat(ih));
                        let lock_id = ui.alloc_id(("curve_track_lock", i));
                        let lr = ui.interact(lock_id, lock_rect);
                        rx -= ih + pad * 0.3;
                        let eye_rect =
                            Rect::from_center_size(Pos2::new(rx - ih * 0.5, row.center().y), Vec2::splat(ih));
                        let eye_id = ui.alloc_id(("curve_track_eye", i));
                        let er = ui.interact(eye_id, eye_rect);
                        rx -= ih + pad * 0.5;
                        if lr.clicked {
                            lock = Some(i);
                        }
                        if er.clicked {
                            hide = Some(i);
                        }
                        {
                            let mut p = ui.painter();
                            let bw = st.metrics.border;
                            draw_lock(
                                &mut p,
                                lock_rect.center(),
                                ih * 0.42,
                                bw,
                                if locked {
                                    super::theme::Palette::invariant_status().warning
                                } else if lr.hovered {
                                    st.palette.text
                                } else {
                                    st.palette.text_disabled
                                },
                                locked,
                            );
                            draw_eye(
                                &mut p,
                                eye_rect.center(),
                                ih * 0.42,
                                bw,
                                if hidden {
                                    st.palette.text_disabled
                                } else if sel || er.hovered {
                                    st.palette.text
                                } else {
                                    st.palette.text_secondary
                                },
                                !hidden,
                            );

                            // Counts, mono and in-row: how much work is on this
                            // track and how long it runs, without opening it.
                            let counts = format!("{keys}k \u{b7} {dur:.1}s");
                            let cw = p
                                .measure_text_family(&counts, st.fonts.small, None, FontFamily::Mono)
                                .x;
                            p.text_family(
                                Pos2::new(rx - cw, row.center().y - st.fonts.small * 0.62),
                                &counts,
                                st.fonts.small,
                                if sel { st.palette.text_mono } else { st.palette.text_disabled },
                                None,
                                FontFamily::Mono,
                            );
                            let label_x = row.min.x + pad * 1.4;
                            p.text(
                                Pos2::new(label_x, row.center().y - st.fonts.body * 0.62),
                                &label,
                                st.fonts.body,
                                if sel {
                                    st.palette.selection_text
                                } else if hidden {
                                    st.palette.text_disabled
                                } else {
                                    st.palette.text
                                },
                                Some((rx - cw - label_x - pad * 0.5).max(1.0)),
                            );
                        }
                        if resp.double_clicked(ui) {
                            state.rename = Some(super::curve_editor::TrackRename {
                                index: i,
                                text: label,
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

            // Add row, pinned to the bottom of the sidebar (the mockup's
            // "+ Track" foot): the list is the panel's subject, the way to
            // grow it is chrome. Submitting the field adds too, so the
            // keyboard path never needs the mouse.
            let row = Rect::from_min_size(
                Pos2::new(rect.min.x + pad, rect.max.y - pad - st.metrics.control_height),
                Vec2::new(w, st.metrics.control_height),
            );
            {
                let mut p = ui.painter();
                p.line_segment(
                    Pos2::new(rect.min.x, row.min.y - pad * 0.5),
                    Pos2::new(rect.max.x, row.min.y - pad * 0.5),
                    st.metrics.border,
                    st.palette.stroke,
                );
            }
            if !state.adding_track {
                // Resting state: one quiet action, the weight the mockup gives
                // it. A permanent field would outweigh the list it grows.
                let id = ui.alloc_id("curve_add_track");
                let resp = ui.interact(id, row);
                let mut p = ui.painter();
                if resp.hovered {
                    p.rect_filled(row, st.rounding.small, st.palette.hover);
                }
                p.text(
                    Pos2::new(row.min.x + pad * 0.5, row.center().y - st.fonts.body * 0.62),
                    "+ Track",
                    st.fonts.body,
                    if resp.hovered { st.palette.text } else { st.palette.text_secondary },
                    None,
                );
                if resp.clicked {
                    state.adding_track = true;
                    state.new_track.clear();
                }
            } else {
                let bw = st.metrics.control_height;
                let mut out = None;
                ui.run_at(
                    row,
                    Direction::LeftToRight,
                    panel_id.with("add_row"),
                    UiOptions { padding: Vec2::ZERO, spacing: pad * 0.5 },
                    |ui| {
                        // Focus is asked for while the field is still empty —
                        // the first frame after the action was clicked.
                        let want_focus = state.new_track.is_empty();
                        out = Some(
                            TextEdit::new(&mut state.new_track)
                                .hint("New track\u{2026}")
                                .width(w - bw - pad * 0.5)
                                .request_focus(want_focus)
                                .show_full(ui),
                        );
                        if Button::new("+")
                            .exact_size(Vec2::new(bw, st.metrics.control_height))
                            .show(ui)
                            .clicked
                        {
                            add_request = Some(state.new_track.clone());
                        }
                    },
                );
                let out = out.expect("add field");
                if out.submitted {
                    add_request = Some(state.new_track.clone());
                }
                // Escape (or walking away from an empty field) puts the foot
                // back to its resting action rather than leaving a field open
                // for the rest of the session.
                if out.cancelled || (!out.focused && state.new_track.is_empty()) {
                    state.adding_track = false;
                }
            }
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
        state.clear_selection();
    }
    if let Some(i) = hide {
        state.toggle_hidden(i);
    }
    if let Some(i) = lock {
        state.toggle_locked(i);
    }
    if let Some(name) = add_request {
        if state.add_track(&name).is_some() {
            state.new_track.clear();
            state.adding_track = false;
        }
    }
    if let Some(i) = delete {
        // Confirm only when there is something to lose. An empty track is a
        // typo you just made; a keyed one is work.
        let has_keys = state.doc.tracks.get(i).is_some_and(|t| !t.keys.is_empty());
        if has_keys && state.confirm_delete != Some(i) {
            state.confirm_delete = Some(i);
            let label = state.doc.tracks[i].label.clone();
            state.toast(format!("Delete '{label}' and its keys? Click again"));
        } else {
            state.confirm_delete = None;
            state.remove_track(i);
        }
    }
}

// ── footer detail bar ───────────────────────────────────────────────────────

/// Format a footer number: mono, three decimals, `\u{2014}` when the selection
/// disagrees. Public so the tests can pin the em-dash rule.
pub fn footer_field_text(v: Option<f32>) -> String {
    match v {
        Some(v) => format!("{v:.3}"),
        None => "\u{2014}".to_string(),
    }
}

fn footer(ui: &mut Ui, rect: Rect, s: f32, panel_id: Id, state: &mut CurveEditorState) {
    let st = ui.style();
    let pad = st.spacing.padding;
    let f = state.footer();
    {
        let mut p = ui.painter();
        p.rect_filled(rect, 0.0, st.palette.header);
        p.line_segment(
            Pos2::new(rect.min.x, rect.min.y),
            Pos2::new(rect.max.x, rect.min.y),
            st.metrics.border,
            st.palette.stroke,
        );
    }
    let h = (st.metrics.control_height * 0.92).min(rect.height() - 6.0 * s);
    let y = rect.center().y - h * 0.5;
    let ty = rect.center().y - st.fonts.small * 0.62;
    let mut x = rect.min.x + pad;

    // Count, then the fields, then the two segmented controls. Nothing here
    // hides: with no selection the whole bar renders disabled rather than
    // vanishing, so the plot never jumps by a bar height.
    {
        let mut p = ui.painter();
        let count = match f.count {
            0 => "no selection".to_string(),
            1 => "1 key".to_string(),
            n => format!("{n} keys"),
        };
        let w = p
            .measure_text_family(&count, st.fonts.small, None, FontFamily::Mono)
            .x;
        p.text_family(
            Pos2::new(x, ty),
            &count,
            st.fonts.small,
            if f.count > 0 { st.palette.text_mono } else { st.palette.text_disabled },
            None,
            FontFamily::Mono,
        );
        x += w + pad;
        bar_divider(&mut p, x, rect, &st);
        x += pad;
    }

    // t / value fields. The buffers are refreshed from the selection on every
    // frame they are not being typed into — otherwise a live selection change
    // would fight the caret.
    let field_w = 68.0 * s;
    for (which, value) in [(FooterField::Time, f.t), (FooterField::Value, f.value)] {
        let label = if which == FooterField::Time { "t" } else { "value" };
        let lw = {
            let mut p = ui.painter();
            let w = p.measure_text(label, st.fonts.small, None).x;
            p.text(
                Pos2::new(x, ty),
                label,
                st.fonts.small,
                if f.count > 0 { st.palette.text_secondary } else { st.palette.text_disabled },
                None,
            );
            w
        };
        x += lw + pad * 0.5;
        let r = Rect::from_min_size(Pos2::new(x, y), Vec2::new(field_w, h));
        if state.field_focus != Some(which) {
            let text = footer_field_text(value);
            match which {
                FooterField::Time => state.field_t = text,
                FooterField::Value => state.field_v = text,
            }
        }
        if f.count == 0 {
            // Disabled, not hidden — and not interactive, because there is
            // nothing for a typed number to land on.
            let mut p = ui.painter();
            p.rect_filled(r, st.rounding.small, st.palette.input);
            p.rect_stroke(r, st.rounding.small, st.metrics.border, st.palette.stroke.with_alpha(0.5));
        } else {
            let mut out = None;
            ui.run_at(
                r,
                Direction::LeftToRight,
                panel_id.with(("footer_field", label)),
                UiOptions { padding: Vec2::ZERO, spacing: 0.0 },
                |ui| {
                    let buf = match which {
                        FooterField::Time => &mut state.field_t,
                        FooterField::Value => &mut state.field_v,
                    };
                    out = Some(TextEdit::new(buf).width(field_w).show_full(ui));
                },
            );
            let out = out.expect("footer field");
            if out.focused {
                state.field_focus = Some(which);
            } else if state.field_focus == Some(which) {
                state.field_focus = None;
            }
            if out.submitted {
                let text = match which {
                    FooterField::Time => state.field_t.clone(),
                    FooterField::Value => state.field_v.clone(),
                };
                if let Ok(v) = text.trim().parse::<f32>() {
                    match which {
                        FooterField::Time => state.set_selection_time(v),
                        FooterField::Value => state.set_selection_value(v),
                    }
                }
                state.field_focus = None;
            } else if out.cancelled {
                state.field_focus = None;
            }
        }
        x += field_w + pad;
    }

    {
        let mut p = ui.painter();
        bar_divider(&mut p, x, rect, &st);
        x += pad;
    }

    // Interp — always live: every key has one, and a mixed selection unifies
    // on click rather than refusing.
    let interp_labels: Vec<&str> = Interp::ALL.iter().map(|i| i.label()).collect();
    let seg_w = 50.0 * s * interp_labels.len() as f32;
    x = segmented_group(
        ui,
        &st,
        SegGroup {
            label: "Interp",
            rect: Rect::from_min_size(Pos2::new(x, y), Vec2::new(seg_w, h)),
            text_y: ty,
            labels: &interp_labels,
            active: f.interp.map(|i| Interp::ALL.iter().position(|a| *a == i).unwrap_or(0)),
            enabled: f.count > 0,
        },
    )
    .map(|(nx, clicked)| {
        if let Some(i) = clicked {
            state.set_selection_interp(Interp::ALL[i]);
        }
        nx
    })
    .unwrap_or(x);

    let tan_labels: Vec<&str> = TangentMode::ALL.iter().map(|m| m.label()).collect();
    let seg_w = 46.0 * s * tan_labels.len() as f32;
    x = segmented_group(
        ui,
        &st,
        SegGroup {
            label: "Tangent",
            rect: Rect::from_min_size(Pos2::new(x, y), Vec2::new(seg_w, h)),
            text_y: ty,
            labels: &tan_labels,
            active: f
                .tangent
                .map(|m| TangentMode::ALL.iter().position(|a| *a == m).unwrap_or(0)),
            enabled: f.count > 0 && f.tangent_enabled,
        },
    )
    .map(|(nx, clicked)| {
        if let Some(i) = clicked {
            state.set_selection_tangent(TangentMode::ALL[i]);
        }
        nx
    })
    .unwrap_or(x);

    let mut p = ui.painter();
    if f.tangent_mixed() && f.tangent_enabled {
        // Mixed is a *state*, so it gets a word, not an unexplained dark
        // control: the segments stay unlit and this says why.
        let w = p.measure_text_family("mixed", st.fonts.small, None, FontFamily::Mono).x;
        p.text_family(
            Pos2::new(x, ty),
            "mixed",
            st.fonts.small,
            super::theme::Palette::invariant_status().warning,
            None,
            FontFamily::Mono,
        );
        x += w + pad;
    }
    let hint = if f.count > 0 {
        "right-click opens the key menu \u{2014} the footer keeps state visible"
    } else {
        "select a key to edit it \u{2014} drag a box on the plot to take several"
    };
    let hw = p.measure_text(hint, st.fonts.small, None).x;
    if rect.max.x - pad - hw > x {
        p.text(
            Pos2::new(rect.max.x - pad - hw, ty),
            hint,
            st.fonts.small,
            st.palette.text_disabled,
            None,
        );
    }
}

/// Where a labelled segmented control goes, and what it should show.
struct SegGroup<'a> {
    label: &'a str,
    rect: Rect,
    text_y: f32,
    labels: &'a [&'a str],
    /// `None` is the mixed state: nothing lit.
    active: Option<usize>,
    enabled: bool,
}

/// A labelled segmented control. Returns the x cursor after it and the segment
/// that was clicked.
fn segmented_group(ui: &mut Ui, st: &Style, g: SegGroup) -> Option<(f32, Option<usize>)> {
    let SegGroup { label, rect, text_y, labels, active, enabled } = g;
    let pad = st.spacing.padding;
    let lw = {
        let mut p = ui.painter();
        let w = p.measure_text(label, st.fonts.small, None).x;
        p.text(
            Pos2::new(rect.min.x, text_y),
            label,
            st.fonts.small,
            if enabled { st.palette.text_secondary } else { st.palette.text_disabled },
            None,
        );
        w
    };
    let seg = Rect::from_min_size(
        Pos2::new(rect.min.x + lw + pad * 0.5, rect.min.y),
        Vec2::new(rect.width(), rect.height()),
    );
    // `usize::MAX` is "no segment": the control lights nothing, which is the
    // mixed state's whole visual claim.
    let clicked = segmented_control(
        ui,
        &format!("curve_seg_{label}"),
        seg,
        labels,
        active.unwrap_or(usize::MAX),
        enabled,
    );
    Some((seg.max.x + pad, clicked))
}

// ── plot ────────────────────────────────────────────────────────────────────

fn plot(
    ui: &mut Ui,
    rect: Rect,
    s: f32,
    selection: Color,
    state: &mut CurveEditorState,
) -> Option<Pos2> {
    let st = ui.style();
    ui.painter().rect_filled(rect, 0.0, st.palette.window);

    let mut view = state.view;
    let mut menu_at = None;
    // The canvas allocates from the *cursor*, so it needs a child Ui rooted at
    // the plot rect — otherwise it lands back at the panel's top-left and
    // paints over the sidebar.
    ui.run_at(
        rect,
        Direction::TopDown,
        Id::new("curve_plot").with(state.path.as_str()),
        UiOptions { padding: Vec2::ZERO, spacing: 0.0 },
        |ui| {
            let out = Canvas::new()
                .size(rect.size())
                .zoom_range(ZOOM_MIN, ZOOM_MAX)
                .show(ui, &mut view, |ui, scope| {
                    // Interact first, draw second: a drag then renders at the
                    // position it just reached rather than a frame behind it.
                    let menu = interact(ui, scope, s, state);
                    draw_axes(ui, scope, state.value_px, s, &st);
                    draw_loop_extent(ui, scope, s, state, &st);
                    draw_snap_guides(ui, scope, s, state, &st);
                    draw_tracks(ui, scope, s, selection, state, &st);
                    draw_marquee(ui, scope, state, &st);
                    draw_playhead(ui, scope, s, state, &st);
                    menu
                });
            menu_at = out.inner;
            plot_hint(ui, rect, state, &st);
            draw_toast(ui, rect, state);
        },
    );
    state.view = view;
    // Fitting needs the viewport, which is only known once the plot has been
    // laid out — so an open (or a Fit button) asks here and lands on the next
    // frame, the same deal `frame_all_on_open` strikes in the graph.
    if state.frame_pending {
        state.frame_pending = false;
        let (view, value_px) = fit_view(state, rect.size());
        state.view = view;
        state.value_px = value_px;
    }
    menu_at
}

/// Zoom limits. Wider than the graph canvas's: a curve is read at both "the
/// whole timeline" and "this one key's neighbourhood".
const ZOOM_MIN: f32 = 0.05;
const ZOOM_MAX: f32 = 12.0;

/// A view that fits every key with a margin, centred — the whole document, or
/// just the selected track when Fit track asked. Never magnifies past 4×, so a
/// two-key curve does not fill the screen with one segment.
///
/// Returns the view **and the value scale it chose**: a canvas has one zoom
/// for both axes and curve data does not, so the vertical mapping is what
/// makes a fit fill the plot rather than leave the curve in a band across the
/// middle.
pub fn fit_view(state: &CurveEditorState, size: Vec2) -> (crusty_gui::widgets::CanvasView, f32) {
    let tracks: Vec<&Track> = if state.frame_track_only {
        state.doc.tracks.get(state.selected_track).into_iter().collect()
    } else {
        state.doc.tracks.iter().filter(|t| !t.keys.is_empty()).collect()
    };
    let mut t_max = tracks.iter().map(|t| t.duration()).fold(0.0, f32::max);
    let (mut v_min, mut v_max) = (0.0_f32, 0.0_f32);
    for k in tracks.iter().flat_map(|t| t.keys.iter()) {
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
    // Time decides the zoom (it is the axis whose unit is fixed and shared
    // with the runtime); the value scale then fills the height that is left.
    let zoom = (size.x * 0.88 / w).clamp(ZOOM_MIN, 4.0);
    let value_px =
        (size.y * 0.80 / (zoom * (v_max - v_min))).clamp(VALUE_PX_MIN, VALUE_PX_MAX);
    // Centre the content box in the viewport: pan is the world point at the
    // canvas's top-left corner.
    let cx = w * 0.5;
    let cy = -(v_min + v_max) * 0.5 * value_px;
    (
        crusty_gui::widgets::CanvasView {
            pan: Vec2::new(cx - size.x * 0.5 / zoom, cy - size.y * 0.5 / zoom),
            zoom,
        },
        value_px,
    )
}

/// Screen position of `(t, value)`. `vpu` is the document's fitted value
/// scale (`CurveEditorState::value_px`) — the vertical half of the mapping,
/// which is per document rather than a constant.
fn to_screen(scope: &CanvasScope, vpu: f32, t: f32, value: f32) -> Pos2 {
    scope.world_to_screen(Pos2::new(t * PX_PER_SECOND, -value * vpu))
}

/// `(t, value)` at a screen position.
fn to_curve(scope: &CanvasScope, vpu: f32, p: Pos2) -> (f32, f32) {
    let w = scope.screen_to_world(p);
    (w.x / PX_PER_SECOND, -w.y / vpu)
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

fn draw_axes(ui: &mut Ui, scope: &CanvasScope, vpu: f32, s: f32, st: &Style) {
    let r = scope.rect();
    let (t0, v1) = to_curve(scope, vpu, r.min);
    let (t1, v0) = to_curve(scope, vpu, r.max);
    let t_step = nice_step(TICK_PX * s / (PX_PER_SECOND * scope.zoom()));
    let v_step = nice_step(TICK_PX * s / (vpu * scope.zoom()));
    let small = st.fonts.small;
    let mut p = ui.painter();

    // Time ticks + labels along the bottom.
    let mut i = (t0 / t_step).floor();
    while i * t_step <= t1 {
        let t = i * t_step;
        i += 1.0;
        let x = to_screen(scope, vpu, t, 0.0).x;
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
        let y = to_screen(scope, vpu, 0.0, v).y;
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

/// The loop extent: the document's own duration, washed along the axis strip
/// and dropped as a dashed line. What a Timeline set to Loop actually replays,
/// stated once and not repeated per track.
fn draw_loop_extent(ui: &mut Ui, scope: &CanvasScope, s: f32, state: &CurveEditorState, st: &Style) {
    let vpu = state.value_px;
    let dur = state.doc.duration();
    if dur <= 0.0 {
        return;
    }
    let r = scope.rect();
    let strip = Rect::from_min_max(r.min, Pos2::new(r.max.x, r.min.y + AXIS_STRIP_H * s));
    let x0 = to_screen(scope, vpu, 0.0, 0.0).x.max(r.min.x);
    let x1 = to_screen(scope, vpu, dur, 0.0).x.min(r.max.x);
    let mut p = ui.painter();
    if x1 > x0 {
        p.rect_filled_translucent(
            Rect::from_min_max(Pos2::new(x0, strip.min.y), Pos2::new(x1, strip.max.y)),
            Rounding::ZERO,
            st.palette.accent_active.with_alpha(LOOP_WASH_ALPHA),
        );
    }
    let x = to_screen(scope, vpu, dur, 0.0).x;
    if x >= r.min.x && x <= r.max.x {
        dashed_v_line(
            &mut p,
            x,
            r.min.y,
            r.max.y,
            st.metrics.border,
            st.palette.accent_active.with_alpha(LOOP_LINE_ALPHA),
            s,
        );
        let label = format!("LOOP {dur:.1}s");
        let w = p
            .measure_text_family(&label, st.fonts.small, None, FontFamily::Mono)
            .x;
        p.text_family(
            Pos2::new((x - w - 4.0 * s).max(r.min.x), strip.min.y + 2.0 * s),
            &label,
            st.fonts.small,
            st.palette.text_disabled,
            None,
            FontFamily::Mono,
        );
    }
}

fn dashed_v_line(p: &mut Painter, x: f32, y0: f32, y1: f32, w: f32, color: Color, s: f32) {
    let dash = 3.0 * s;
    let gap = 4.0 * s;
    let mut y = y0;
    while y < y1 {
        let e = (y + dash).min(y1);
        p.line_segment(Pos2::new(x, y), Pos2::new(x, e), w, color);
        y = e + gap;
    }
}

/// While a gesture is snapping, the grid line it is snapping to lights up —
/// the feedback that tells you snapping is on *without* reading the toolbar.
fn draw_snap_guides(ui: &mut Ui, scope: &CanvasScope, s: f32, state: &CurveEditorState, st: &Style) {
    let vpu = state.value_px;
    let Some(d) = state.drag.as_ref() else { return };
    let snap = state.snap.effective(ui.ctx().input.modifiers.contains(Modifiers::CTRL));
    if !snap.time && !snap.value {
        return;
    }
    let Some(key) = state
        .doc
        .tracks
        .get(d.track)
        .and_then(|t| t.keys.get(d.anchor))
        .copied()
    else {
        return;
    };
    let r = scope.rect();
    let c = to_screen(scope, vpu, key.t, key.value);
    let mut p = ui.painter();
    let color = st.palette.accent_active.with_alpha(0.7);
    if snap.time {
        p.line_segment(Pos2::new(c.x, r.min.y), Pos2::new(c.x, r.max.y), st.metrics.border, color);
    }
    if snap.value {
        p.line_segment(Pos2::new(r.min.x, c.y), Pos2::new(r.max.x, c.y), st.metrics.border, color);
    }
    // Committed numbers, at the cursor: the chip says what the release will
    // write, not where the pointer happens to be.
    let chip = format!("{:.3} \u{b7} {:.2}", key.t, key.value);
    let pad = st.spacing.padding;
    let w = p.measure_text_family(&chip, st.fonts.small, None, FontFamily::Mono).x + pad;
    let h = st.fonts.small * 1.7;
    let cr = Rect::from_min_size(
        Pos2::new(c.x + KEY_HIT * s, c.y - h - KEY_HIT * s),
        Vec2::new(w, h),
    );
    p.rect_filled(cr, st.rounding.small, st.palette.elevated);
    p.rect_stroke(cr, st.rounding.small, st.metrics.border, st.palette.stroke_strong);
    p.text_family(
        Pos2::new(cr.min.x + pad * 0.5, cr.center().y - st.fonts.small * 0.62),
        &chip,
        st.fonts.small,
        st.palette.text_mono,
        None,
        FontFamily::Mono,
    );
}

/// The screen direction of key `i`'s tangent on `side`, as a unit vector.
fn arm_dir(scope: &CanvasScope, vpu: f32, track: &Track, i: usize, side: ArmSide) -> Vec2 {
    let slope = match side {
        ArmSide::In => track.in_tangent(i),
        ArmSide::Out => track.out_tangent(i),
    };
    let zoom = scope.zoom();
    let d = Vec2::new(PX_PER_SECOND * zoom, -slope * vpu * zoom);
    let len = (d.x * d.x + d.y * d.y).sqrt().max(1.0e-3);
    let unit = Vec2::new(d.x / len, d.y / len);
    match side {
        ArmSide::Out => unit,
        ArmSide::In => Vec2::new(-unit.x, -unit.y),
    }
}

/// Where an arm's handle sits, in screen pixels.
fn arm_handle(scope: &CanvasScope, vpu: f32, track: &Track, i: usize, side: ArmSide, s: f32) -> Option<Pos2> {
    let key = track.keys.get(i)?;
    let c = to_screen(scope, vpu, key.t, key.value);
    let d = arm_dir(scope, vpu, track, i, side);
    Some(Pos2::new(c.x + d.x * ARM_LEN * s, c.y + d.y * ARM_LEN * s))
}

/// Does this side of the key touch a cubic segment? An arm on a linear segment
/// would promise an edit that changes nothing.
fn arm_live(track: &Track, i: usize, side: ArmSide) -> bool {
    match side {
        ArmSide::Out => {
            track.keys.get(i).is_some_and(|k| k.interp == Interp::Cubic) && i + 1 < track.keys.len()
        }
        ArmSide::In => i
            .checked_sub(1)
            .and_then(|p| track.keys.get(p))
            .is_some_and(|k| k.interp == Interp::Cubic),
    }
}

fn draw_tracks(
    ui: &mut Ui,
    scope: &CanvasScope,
    s: f32,
    selection: Color,
    state: &CurveEditorState,
    st: &Style,
) {
    let vpu = state.value_px;
    let key_r = KEY_R * s;
    let r = scope.rect();
    let (t0, _) = to_curve(scope, vpu, r.min);
    let (t1, _) = to_curve(scope, vpu, r.max);
    // Unselected tracks first, so the one being edited draws on top.
    let order: Vec<usize> = (0..state.doc.tracks.len())
        .filter(|i| *i != state.selected_track)
        .chain(std::iter::once(state.selected_track))
        .collect();

    for i in order {
        let Some(track) = state.doc.tracks.get(i) else { continue };
        if state.is_hidden(i) {
            continue;
        }
        let sel = i == state.selected_track;
        let base = track_color(i);
        // Fills *and* strokes dim together: a dimmed curve with full-strength
        // keys reads as a bug, not as depth.
        let color = if sel { base } else { base.with_alpha(DIM) };
        let pts = sample_polyline(track, scope, vpu, t0, t1);
        if pts.len() >= 2 {
            let mut p = ui.painter();
            p.polyline(&pts, if sel { 1.8 * s } else { 1.2 * s }, color);
        }
        if !sel {
            // A dimmed track still shows *where* its keys are — that is how
            // you decide which track to open — but not their modes.
            let mut p = ui.painter();
            for key in track.keys.iter() {
                let c = to_screen(scope, vpu, key.t, key.value);
                if r.contains(c) {
                    p.circle_filled(c, key_r * 0.8, color);
                }
            }
            continue;
        }

        for (k, key) in track.keys.iter().enumerate() {
            let c = to_screen(scope, vpu, key.t, key.value);
            if !r.contains(c) {
                continue;
            }
            let selected = state.is_selected(k);
            // Arms come first so the glyph sits on top of its own handles.
            if selected {
                let mode = TangentMode::of(&key.tangent);
                for side in [ArmSide::In, ArmSide::Out] {
                    if !arm_live(track, k, side) {
                        continue;
                    }
                    let Some(h) = arm_handle(scope, vpu, track, k, side, s) else { continue };
                    // A derived mode (Auto/Flat/Linear) still shows where its
                    // slope points — as a *ghost*, which is both an honest
                    // readout and the affordance that makes "drag to shape it"
                    // discoverable (grabbing one promotes the key to User).
                    // Only on a *single* selected key, though: on a
                    // multi-selection the design wants arms to mean "this key
                    // is in an arm mode", and a plot full of ghosts would say
                    // the opposite.
                    let ghost = !mode.has_arms();
                    if ghost && state.selection.len() > 1 {
                        continue;
                    }
                    let col =
                        if ghost { selection.with_alpha(GHOST_ARM_ALPHA) } else { selection };
                    let mut p = ui.painter();
                    // Thicker than a border: an arm lies *along* the curve it
                    // shapes, so at hairline width it disappears under the
                    // stroke it is sitting on.
                    p.line_segment(c, h, 1.6 * s, col);
                    let hb = Rect::from_center_size(h, Vec2::splat(6.4 * s));
                    if ghost {
                        p.rect_stroke(hb, 0.0, st.metrics.border, col);
                    } else {
                        p.rect_filled(hb, 0.0, col);
                    }
                }
            }
            let mut p = ui.painter();
            match key_glyph(&key.tangent) {
                KeyGlyph::Circle => {
                    p.circle_filled(c, key_r, color);
                    if selected {
                        p.circle_stroke(c, key_r + 2.6 * s, 1.6 * s, selection);
                    } else {
                        p.circle_stroke(c, key_r, st.metrics.border, st.palette.window);
                    }
                }
                KeyGlyph::Square => {
                    let b = Rect::from_center_size(c, Vec2::splat(key_r * 1.8));
                    p.rect_filled(b, 0.0, color);
                    if selected {
                        p.rect_stroke(
                            Rect::from_center_size(c, Vec2::splat(key_r * 1.8 + 5.2 * s)),
                            0.0,
                            1.6 * s,
                            selection,
                        );
                    } else {
                        p.rect_stroke(b, 0.0, st.metrics.border, st.palette.window);
                    }
                }
                KeyGlyph::Diamond => {
                    p.convex_polygon_filled(diamond(c, key_r * 1.25), color);
                    if selected {
                        p.polygon_stroke(&diamond(c, key_r * 1.25 + 3.4 * s), 1.6 * s, selection);
                    } else {
                        p.polygon_stroke(&diamond(c, key_r * 1.25), st.metrics.border, st.palette.window);
                    }
                }
            }
        }
    }
}

fn diamond(c: Pos2, r: f32) -> Vec<Pos2> {
    vec![
        Pos2::new(c.x, c.y - r),
        Pos2::new(c.x + r, c.y),
        Pos2::new(c.x, c.y + r),
        Pos2::new(c.x - r, c.y),
    ]
}

/// Sample a track across the visible time range, at the curve's own keys plus
/// a screen-space step between them. `Track::sample` is the interpreter's
/// function, never a local reimplementation.
fn sample_polyline(track: &Track, scope: &CanvasScope, vpu: f32, t0: f32, t1: f32) -> Vec<Pos2> {
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
    ts.iter().map(|t| to_screen(scope, vpu, *t, track.sample(*t))).collect()
}

fn draw_marquee(ui: &mut Ui, scope: &CanvasScope, state: &CurveEditorState, st: &Style) {
    let vpu = state.value_px;
    let Some(m) = state.marquee else { return };
    if !m.armed {
        return;
    }
    let a = to_screen(scope, vpu, m.start.0, m.start.1);
    let b = to_screen(scope, vpu, m.cur.0, m.cur.1);
    let r = Rect::from_min_max(
        Pos2::new(a.x.min(b.x), a.y.min(b.y)),
        Pos2::new(a.x.max(b.x), a.y.max(b.y)),
    );
    let mut p = ui.painter();
    // Translucent, not glass: the sweep has to reveal the keys it is picking
    // up (the graph canvas learned this the hard way).
    p.rect_filled_translucent(
        r,
        Rounding::ZERO,
        st.palette.accent_active.with_alpha(MARQUEE_FILL_ALPHA),
    );
    p.rect_stroke(r, Rounding::ZERO, st.metrics.border, st.palette.accent_active);
}

/// The playhead: a draggable accent cursor, its time, and what every visible
/// track evaluates to under it — each chip edged in that track's colour, which
/// is what ties sidebar, curve and readout into one identity.
fn draw_playhead(ui: &mut Ui, scope: &CanvasScope, s: f32, state: &CurveEditorState, st: &Style) {
    let vpu = state.value_px;
    let r = scope.rect();
    let x = to_screen(scope, vpu, state.playhead, 0.0).x;
    if x < r.min.x - 1.0 || x > r.max.x + 1.0 {
        return;
    }
    let pad = st.spacing.padding;
    let strip_h = AXIS_STRIP_H * s;
    let mut p = ui.painter();
    p.line_segment(
        Pos2::new(x, r.min.y),
        Pos2::new(x, r.max.y),
        st.metrics.border,
        st.palette.accent_active,
    );
    p.triangle(
        Pos2::new(x - 5.0 * s, r.min.y),
        Pos2::new(x + 5.0 * s, r.min.y),
        Pos2::new(x, r.min.y + 7.0 * s),
        st.palette.accent_active,
    );

    let h = st.fonts.small * 1.9;
    let mut y = r.min.y + strip_h + 5.0 * s;
    let mut chip = |p: &mut Painter, text: &str, edge: Option<Color>, fg: Color| {
        let w = p.measure_text_family(text, st.fonts.small, None, FontFamily::Mono).x + pad;
        // Flip to the left of the cursor when the right side runs out — a
        // readout you cannot read is not a readout.
        let left = x + 10.0 * s + w > r.max.x;
        let cx = if left { x - 10.0 * s - w } else { x + 10.0 * s };
        let cr = Rect::from_min_size(Pos2::new(cx, y), Vec2::new(w, h));
        p.rect_filled(cr, st.rounding.small, st.palette.elevated);
        p.rect_stroke(
            cr,
            st.rounding.small,
            st.metrics.border,
            match edge {
                // A readout is a chip, not a swatch: neutral outline, and the
                // track's identity carried on one edge only.
                Some(_) => st.palette.stroke_strong,
                None => st.palette.accent_active,
            },
        );
        if let Some(c) = edge {
            // The track's own colour on the leading edge, and nowhere else:
            // the chip stays a chip.
            p.rect_filled(
                Rect::from_min_size(cr.min, Vec2::new(st.metrics.edge_accent, cr.height())),
                0.0,
                c,
            );
        }
        p.text_family(
            Pos2::new(cr.min.x + pad * 0.7, cr.center().y - st.fonts.small * 0.62),
            text,
            st.fonts.small,
            fg,
            None,
            FontFamily::Mono,
        );
        y += h + 4.0 * s;
    };
    let time = format!("{:.2}", state.playhead);
    chip(&mut p, &time, None, st.palette.accent_active);
    for (i, v) in state.playhead_readouts() {
        chip(&mut p, &format!("{v:.2}"), Some(track_color(i)), st.palette.text_mono);
    }
}

/// Everything the pointer does inside the plot. Returns the position at which
/// the key menu should open, if this frame's right-click asked for one.
fn interact(ui: &mut Ui, scope: &CanvasScope, s: f32, state: &mut CurveEditorState) -> Option<Pos2> {
    let vpu = state.value_px;
    let hit_r = KEY_HIT * s;
    let r = scope.rect();
    let input_ctrl = ui.ctx().input.modifiers.contains(Modifiers::CTRL);
    let shift = ui.ctx().input.modifiers.contains(Modifiers::SHIFT);
    let snap = state.snap.effective(input_ctrl);
    let pointer = ui.ctx().input.pointer_pos;
    let pointer_down = ui.ctx().input.pointer_down;

    // Escape abandons whatever is in flight and puts the document back — a
    // canvas gesture, not a bound action, so it is handled here rather than in
    // the keymap and works identically docked and floated.
    if ui.ctx().input.key_pressed(UiKey::Escape) {
        if state.gesture_in_flight() {
            state.cancel_gestures();
            return None;
        }
        state.menu_anchor = None;
    }

    // A live gesture owns the pointer: no hit test can steal it mid-drag.
    if state.arm.is_some() {
        if let Some(p) = pointer {
            let a = state.arm.expect("arm");
            if let Some(key) = state.doc.tracks.get(a.track).and_then(|t| t.keys.get(a.index)) {
                let (t, v) = to_curve(scope, vpu, p);
                let dt = t - key.t;
                // Value-per-second, the schema's unit. Close to vertical the
                // slope is meaningless, so the arm holds rather than flipping.
                if dt.abs() > MIN_KEY_GAP {
                    state.arm_to((v - key.value) / dt);
                }
            }
        }
        if !pointer_down {
            state.end_arm_drag();
        }
        return None;
    }
    if state.drag.is_some() {
        if let Some(p) = pointer {
            let (t, v) = to_curve(scope, vpu, p);
            state.drag_to(t, v, snap);
        }
        if !pointer_down {
            state.end_drag();
        }
        return None;
    }
    if state.playhead_drag {
        if let Some(p) = pointer {
            state.set_playhead(to_curve(scope, vpu, p).0, snap);
        }
        if !pointer_down {
            state.playhead_drag = false;
        }
        return None;
    }
    if let Some(m) = state.marquee {
        let mut m = m;
        if let Some(p) = pointer {
            m.cur = to_curve(scope, vpu, p);
            let a = to_screen(scope, vpu, m.start.0, m.start.1);
            if (p.x - a.x).abs() > MARQUEE_ARM_PX * s || (p.y - a.y).abs() > MARQUEE_ARM_PX * s {
                m.armed = true;
            }
        }
        state.marquee = Some(m);
        if !pointer_down {
            if m.armed {
                state.select_in_box((m.start.0, m.cur.0), (m.start.1, m.cur.1), m.add);
            }
            state.marquee = None;
        }
        return None;
    }

    // The playhead strip owns the top band, so grabbing the cursor never
    // fights a key that happens to be up there.
    let strip = Rect::from_min_max(r.min, Pos2::new(r.max.x, r.min.y + AXIS_STRIP_H * s));
    let strip_id = ui.alloc_id("curve_playhead_strip");
    let strip_resp = ui.interact(strip_id, strip);
    if strip_resp.pressed {
        state.playhead_drag = true;
        if let Some(p) = pointer {
            state.set_playhead(to_curve(scope, vpu, p).0, snap);
        }
        return None;
    }

    let ti = state.selected_track;
    let locked = state.is_locked(ti);
    // A snapshot, so hit-testing can read the track while the gestures below
    // write to the document. Keys are `Copy` and a hand-authored curve is
    // small — this is cheaper than threading indices through five closures.
    let track = state.doc.tracks.get(ti).cloned()?;
    let track = &track;
    let hit: Vec<(usize, Rect)> = track
        .keys
        .iter()
        .enumerate()
        .map(|(k, key)| {
            let c = to_screen(scope, vpu, key.t, key.value);
            (k, Rect::from_center_size(c, Vec2::splat(hit_r * 2.0)))
        })
        .filter(|(_, b)| r.intersect(*b).width() > 0.0)
        .collect();

    // Arms first: they belong to the selected keys and sit away from them, so
    // grabbing one must not be intercepted by the key it points at.
    if !locked {
        let arms: Vec<(usize, ArmSide, Rect)> = state
            .selection
            .iter()
            .filter_map(|k| track.keys.get(*k).map(|_| *k))
            .flat_map(|k| {
                [ArmSide::In, ArmSide::Out].into_iter().filter_map(move |side| {
                    arm_live(track, k, side)
                        .then(|| arm_handle(scope, vpu, track, k, side, s))
                        .flatten()
                        .map(|h| (k, side, Rect::from_center_size(h, Vec2::splat(hit_r * 1.6))))
                })
            })
            .collect();
        for (k, side, b) in arms {
            let arm_id = ui.alloc_id(("curve_arm", ti, k, side == ArmSide::Out));
            let resp = ui.interact(arm_id, b);
            if resp.pressed {
                state.begin_arm_drag(ti, k, side);
                return None;
            }
        }
    }

    let mut hovered_key: Option<usize> = None;
    let mut press: Option<usize> = None;
    for (k, b) in &hit {
        let id = ui.alloc_id(("curve_key", ti, *k));
        let resp = ui.interact(id, *b);
        if resp.hovered {
            hovered_key = Some(*k);
        }
        if resp.pressed {
            press = Some(*k);
        }
    }
    if let Some(k) = press {
        if shift {
            // Shift extends without starting a drag: a modifier that both
            // selected and moved would make an extend impossible to do
            // without nudging.
            state.toggle_select(k);
        } else if let Some(p) = pointer {
            state.begin_drag(ti, k, to_curve(scope, vpu, p));
        }
        return None;
    }

    if let Some(k) = hovered_key {
        let key = track.keys[k];
        let text = format!(
            "t {:.3}s  v {:.3}  \u{2014}  {} \u{b7} {}",
            key.t,
            key.value,
            key.interp.label(),
            key.tangent.label()
        );
        let anchor = Rect::from_center_size(to_screen(scope, vpu, key.t, key.value), Vec2::splat(hit_r * 2.0));
        show_tooltip_for(ui, anchor, &text);
    }

    // Right-click on a key opens the key menu — the blind cycle is retired.
    // `CanvasScope` has already decided this release was a click and not a pan.
    let mut menu_at = None;
    if let Some(p) = scope.right_clicked() {
        if let Some((k, _)) = hit.iter().find(|(_, b)| b.contains(p)) {
            if !state.is_selected(*k) {
                state.select_only(*k);
            }
            state.menu_anchor = Some((p.x, p.y));
            menu_at = Some(p);
        } else {
            state.menu_anchor = None;
        }
    }

    // Double-click on empty plot adds a key; a plain press starts a box
    // select, which is also how an empty sweep deselects.
    let bg = ui.alloc_id("curve_plot_bg");
    let resp = ui.interact(bg, r);
    if resp.double_clicked(ui) && hovered_key.is_none() {
        if let Some(p) = pointer {
            let (t, v) = to_curve(scope, vpu, p);
            let (t, v) = (
                if snap.time { super::curve_editor::snap_to(t, SNAP_TIME) } else { t },
                if snap.value { super::curve_editor::snap_to(v, SNAP_VALUE) } else { v },
            );
            state.add_key(ti, t, v);
        }
        return menu_at;
    }
    if resp.pressed && hovered_key.is_none() {
        if let Some(p) = pointer {
            // Clicking a dimmed curve selects its track — the editor-wide
            // "click the thing to select it" grammar, so the sidebar is not the
            // only way in. Anything else arms a sweep.
            match track_under(scope, s, state, p) {
                Some(i) => {
                    state.selected_track = i;
                    state.clear_selection();
                }
                None => {
                    let start = to_curve(scope, vpu, p);
                    state.marquee = Some(super::curve_editor::Marquee {
                        start,
                        cur: start,
                        add: shift,
                        armed: false,
                    });
                }
            }
        }
    }
    menu_at
}

/// The topmost *visible, unselected* track whose curve passes within a grab
/// radius of `p`. Vertical distance at the cursor's own time — the curve is a
/// function of t, so that is the whole test.
fn track_under(scope: &CanvasScope, s: f32, state: &CurveEditorState, p: Pos2) -> Option<usize> {
    let vpu = state.value_px;
    let (t, _) = to_curve(scope, vpu, p);
    let grab = KEY_HIT * s;
    state
        .doc
        .tracks
        .iter()
        .enumerate()
        .filter(|(i, tr)| {
            *i != state.selected_track && !tr.keys.is_empty() && !state.is_hidden(*i)
        })
        .map(|(i, tr)| (i, (to_screen(scope, vpu, t, tr.sample(t)).y - p.y).abs()))
        .filter(|(_, d)| *d <= grab)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

// ── key context menu ────────────────────────────────────────────────────────

/// The current-mode marker: the mockup's filled/hollow radio dot, in text so
/// it costs nothing and lines up in any font.
fn radio(on: bool) -> &'static str {
    if on {
        "\u{25CF}"
    } else {
        "\u{25CB}"
    }
}

/// Right-click menu for the selected keys: interpolation, tangent mode, and
/// the two actions. Every row applies to the whole selection as one entry, and
/// every row carries the number key that does the same thing.
fn key_menu(ui: &mut Ui, state: &mut CurveEditorState, open_at: Option<Pos2>) {
    if state.menu_anchor.is_none() {
        return;
    }
    let f = state.footer();
    if f.count == 0 {
        state.menu_anchor = None;
        return;
    }
    let mut interp: Option<Interp> = None;
    let mut tangent: Option<TangentMode> = None;
    let mut flatten = false;
    let mut straighten = false;
    crusty_gui::widgets::context_menu_at(ui, "curve_key_menu", open_at, |ui| {
        ui.menu_group_header("Interpolation");
        for (n, mode) in Interp::ALL.iter().enumerate() {
            let cur = f.interp == Some(*mode);
            if ui.menu_item_shortcut(
                format!("{} {}", radio(cur), mode.label()),
                format!("{}", n + 1),
                true,
            ) {
                interp = Some(*mode);
            }
        }
        ui.menu_group_header("Tangent");
        // Auto / User / Break / Flat carry 4–7; Linear is reachable through
        // Straighten (9) and the footer's fifth segment.
        for (n, mode) in TangentMode::ALL.iter().take(4).enumerate() {
            let cur = f.tangent == Some(*mode);
            if ui.menu_item_shortcut(
                format!("{} {}", radio(cur), mode.label()),
                format!("{}", n + 4),
                f.tangent_enabled,
            ) {
                tangent = Some(*mode);
            }
        }
        ui.separator();
        if ui.menu_item_shortcut("Flatten", "8", f.tangent_enabled) {
            flatten = true;
        }
        if ui.menu_item_shortcut("Straighten", "9", f.tangent_enabled) {
            straighten = true;
        }
    });
    if let Some(i) = interp {
        state.set_selection_interp(i);
        state.menu_anchor = None;
    }
    if let Some(m) = tangent {
        state.set_selection_tangent(m);
        state.menu_anchor = None;
    }
    if flatten {
        state.flatten_selection();
        state.menu_anchor = None;
    }
    if straighten {
        state.straighten_selection();
        state.menu_anchor = None;
    }
}

/// What to do when there is nothing to look at. A bare grid is
/// indistinguishable from a broken panel, and double-click-to-add is the one
/// gesture here that nothing else announces — so the empty states are where
/// it gets said.
fn plot_hint(ui: &mut Ui, rect: Rect, state: &CurveEditorState, st: &Style) {
    let (head, sub) = if state.doc.tracks.is_empty() {
        ("No tracks yet".to_string(), "Add one in the sidebar to start a curve".to_string())
    } else {
        match state.doc.tracks.get(state.selected_track) {
            Some(t) if t.keys.is_empty() => (
                format!("No keys on \u{201c}{}\u{201d}", t.label),
                "Double-click the plot to add one".to_string(),
            ),
            _ => return,
        }
    };
    let mut p = ui.painter();
    let w = p.measure_text(&head, st.fonts.body, None).x;
    p.text(
        Pos2::new(rect.center().x - w * 0.5, rect.center().y - st.fonts.body * 1.4),
        &head,
        st.fonts.body,
        st.palette.text_secondary,
        None,
    );
    let w = p.measure_text(&sub, st.fonts.small, None).x;
    p.text(
        Pos2::new(rect.center().x - w * 0.5, rect.center().y),
        &sub,
        st.fonts.small,
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

// ── keyboard ────────────────────────────────────────────────────────────────

/// The key menu's numeric shortcut for `key`, if it has one.
///
/// Pure so the gating is testable: digits mean nothing without a selection,
/// and the tangent digits mean nothing on a key no cubic segment touches —
/// exactly the rows the menu greys.
pub fn numeric_shortcut(key: char, f: &FooterState) -> Option<CurveShortcut> {
    if f.count == 0 {
        return None;
    }
    let tangent = |m: TangentMode| {
        f.tangent_enabled.then_some(CurveShortcut::Tangent(m))
    };
    match key {
        '1' => Some(CurveShortcut::Interp(Interp::Constant)),
        '2' => Some(CurveShortcut::Interp(Interp::Linear)),
        '3' => Some(CurveShortcut::Interp(Interp::Cubic)),
        '4' => tangent(TangentMode::Auto),
        '5' => tangent(TangentMode::User),
        '6' => tangent(TangentMode::Break),
        '7' => tangent(TangentMode::Flat),
        '8' => f.tangent_enabled.then_some(CurveShortcut::Flatten),
        '9' => f.tangent_enabled.then_some(CurveShortcut::Straighten),
        _ => None,
    }
}

/// What a numeric shortcut does — the menu's rows, addressable from the
/// keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CurveShortcut {
    Interp(Interp),
    Tangent(TangentMode),
    Flatten,
    Straighten,
}

fn handle_panel_keys(
    ui: &Ui,
    state: &mut CurveEditorState,
    handle_shortcuts: bool,
    out: &mut CurveEditorOutput,
) {
    let input = &ui.ctx().input;
    if ui.ctx().text_focused() || ui.ctx().modal_any_open() || state.rename.is_some() {
        // A focused text field owns the keyboard: Del must delete a character.
        return;
    }
    // The key menu's digits are the panel's in both hosting modes: nothing in
    // the editor keymap binds a bare digit, and they only do anything with
    // keys selected — the same condition that puts the menu on screen.
    if input.modifiers.is_empty() {
        let f = state.footer();
        for d in ['1', '2', '3', '4', '5', '6', '7', '8', '9'] {
            if input.key_pressed(UiKey::Char(d)) {
                match numeric_shortcut(d, &f) {
                    Some(CurveShortcut::Interp(i)) => state.set_selection_interp(i),
                    Some(CurveShortcut::Tangent(m)) => state.set_selection_tangent(m),
                    Some(CurveShortcut::Flatten) => state.flatten_selection(),
                    Some(CurveShortcut::Straighten) => state.straighten_selection(),
                    None => {}
                }
            }
        }
    }
    if !handle_shortcuts {
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
        state.delete_selection();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::editor::curve_editor::CurveEditorState;
    use curve_asset::{CurveDoc, Key, Tangent, Track};

    fn doc() -> CurveDoc {
        let mut d = CurveDoc::default();
        let mut t = Track::new("h", "H");
        t.keys = vec![
            Key { t: 0.0, value: -1.0, interp: Interp::Linear, tangent: Tangent::Auto },
            Key { t: 2.0, value: 3.0, interp: Interp::Linear, tangent: Tangent::Auto },
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
        let (view, vpu) = fit_view(&st, size);
        let scope_min = view.pan;
        let scope_max = view.pan + size / view.zoom;
        for k in st.doc.tracks[0].keys.iter() {
            let (x, y) = (k.t * PX_PER_SECOND, -k.value * vpu);
            assert!(x >= scope_min.x && x <= scope_max.x, "t {} off screen", k.t);
            assert!(y >= scope_min.y && y <= scope_max.y, "v {} off screen", k.value);
        }
        assert!(view.zoom <= 4.0 + 1e-4);

        // An empty document still gets a usable scale rather than NaN.
        let empty = CurveEditorState::from_doc(CurveDoc::default(), "e.curve");
        let (v, vpu) = fit_view(&empty, size);
        assert!(v.zoom.is_finite() && v.zoom > 0.0);
        assert!(v.pan.x.is_finite() && v.pan.y.is_finite());
        assert!((VALUE_PX_MIN..=VALUE_PX_MAX).contains(&vpu), "value scale stays sane: {vpu}");
    }

    /// Fit track answers about one track, not the document: a second track
    /// living an order of magnitude away must not flatten the one being
    /// edited.
    #[test]
    fn fit_track_frames_only_the_selected_track() {
        let mut st = CurveEditorState::from_doc(doc(), "t.curve");
        st.add_track("Big").expect("added");
        st.doc.tracks[1].keys = vec![
            Key { t: 0.0, value: 0.0, interp: Interp::Linear, tangent: Tangent::Auto },
            Key { t: 40.0, value: 900.0, interp: Interp::Linear, tangent: Tangent::Auto },
        ];
        st.selected_track = 0;
        st.frame_track();
        let all = {
            let mut s2 = CurveEditorState::from_doc(st.doc.clone(), "t.curve");
            s2.frame_all();
            fit_view(&s2, Vec2::new(800.0, 600.0))
        };
        let one = fit_view(&st, Vec2::new(800.0, 600.0));
        assert!(
            one.0.zoom > all.0.zoom,
            "one track fills more of the plot horizontally: {:?} vs {:?}",
            one.0,
            all.0
        );
        assert!(
            one.1 > all.1,
            "…and vertically: the value scale is per fit, not a constant ({} vs {})",
            one.1,
            all.1
        );
    }

    /// Track colours come off the ramp, start on the Float pin hue (a track
    /// *is* a float pin) and adjacent tracks are distinct — the sidebar bar,
    /// the curve and the playhead chip are the same lookup, so a collision
    /// would make two tracks unreadable in three places at once.
    #[test]
    fn track_colors_are_ramp_entries_and_neighbours_differ() {
        let ramp = ramp();
        assert_eq!(track_color(0), ramp[FLOAT_HUE].bright, "track 0 is the Float hue");
        let mut seen = Vec::new();
        for i in 0..24 {
            let c = track_color(i);
            assert!(ramp.iter().any(|h| h.bright == c), "track {i} is not a ramp hue");
            assert_ne!(c, track_color(i + 1), "tracks {i} and {} collide", i + 1);
            if i < 12 {
                assert!(!seen.contains(&c), "hue repeats before all twelve are spent");
                seen.push(c);
            }
        }
    }

    /// The numeric shortcuts are exactly the menu's rows, and they are gated
    /// the same way the menu greys them.
    #[test]
    fn numeric_shortcuts_match_the_menu_and_its_gating() {
        let mut f = FooterState::empty();
        assert_eq!(numeric_shortcut('1', &f), None, "no selection, no shortcut");

        f.count = 1;
        f.tangent_enabled = false;
        assert_eq!(numeric_shortcut('1', &f), Some(CurveShortcut::Interp(Interp::Constant)));
        assert_eq!(numeric_shortcut('3', &f), Some(CurveShortcut::Interp(Interp::Cubic)));
        for d in ['4', '5', '6', '7', '8', '9'] {
            assert_eq!(numeric_shortcut(d, &f), None, "{d} needs a cubic segment");
        }

        f.tangent_enabled = true;
        assert_eq!(numeric_shortcut('4', &f), Some(CurveShortcut::Tangent(TangentMode::Auto)));
        assert_eq!(numeric_shortcut('5', &f), Some(CurveShortcut::Tangent(TangentMode::User)));
        assert_eq!(numeric_shortcut('6', &f), Some(CurveShortcut::Tangent(TangentMode::Break)));
        assert_eq!(numeric_shortcut('7', &f), Some(CurveShortcut::Tangent(TangentMode::Flat)));
        assert_eq!(numeric_shortcut('8', &f), Some(CurveShortcut::Flatten));
        assert_eq!(numeric_shortcut('9', &f), Some(CurveShortcut::Straighten));
        assert_eq!(numeric_shortcut('0', &f), None);
    }

    /// The footer's number formatting is the design's: mono three decimals, or
    /// the em dash when the selection disagrees.
    #[test]
    fn footer_fields_show_an_em_dash_when_mixed() {
        assert_eq!(footer_field_text(Some(1.5)), "1.500");
        assert_eq!(footer_field_text(None), "\u{2014}");
    }
}
