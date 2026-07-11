//! Profiler panel rendered with crusty-gui (Phase 16 port).
//!
//! Reads/writes the same `ProfilerState` as the egui version. The toolbar,
//! frame history and flamegraph follow the egui reference; the Table and
//! Budget views are redesigned (user-approved deviation). Settings/Info/
//! Tracy popups are real floating windows, fixing the egui z-order bug
//! where they rendered below the viewport toolbar.

use crusty_gui::context::{Align, Direction, Ui, UiOptions};
use crusty_gui::id::Id;
use crusty_gui::input::{Key, Modifiers};
use crusty_gui::math::{Color, Pos2, Rect, Rounding, Vec2};
use crusty_gui::paint::{PaintCmd, Painter, RectShape};
use crusty_gui::widgets::{
    show_tooltip_for, Button, ComboBox, DragValue, Label, ScrollArea, SelectableValue, TextEdit,
    Window,
};

use super::profiler::data::{
    FrameBarScale, FrameHistoryMode, ProfileScope, ProfilerSettings, ProfilerState, ProfilerView,
    SortColumn, SortDirection,
};
use super::profiler::scope_colors::{darken, dim_color, frame_bar_color_fps, lighten, scope_color};
use super::profiler::{budget, ProfilerPanel};

// ── color helpers ───────────────────────────────────────────────────────────

fn c32(c: egui::Color32) -> Color {
    Color::from_srgb_u8(c.r(), c.g(), c.b(), c.a())
}

fn gray(v: u8) -> Color {
    Color::from_srgb_u8(v, v, v, 255)
}

fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::from_srgb_u8(r, g, b, 255)
}

fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color {
    Color::from_srgb_u8(r, g, b, a)
}

/// Solid frame color by FPS thresholds (green >60, yellow 30–60, red <30).
fn fps_color(duration_ms: f64) -> Color {
    c32(frame_bar_color_fps(duration_ms))
}

/// egui `widgets.inactive.bg_fill` — plain toolbar buttons (crusty's default
/// surface is much darker than the egui reference).
fn btn_fill() -> Color {
    gray(60)
}

/// Threshold color for table time cells (settings-driven).
fn time_color(ms: f64, settings: &ProfilerSettings) -> Color {
    if ms < settings.fast_threshold_ms as f64 {
        rgb(120, 200, 120)
    } else if ms < settings.warning_threshold_ms as f64 {
        rgb(220, 200, 80)
    } else if ms < settings.slow_threshold_ms as f64 {
        rgb(220, 140, 80)
    } else {
        rgb(220, 80, 80)
    }
}

/// Top-left y so text of `font` px sits vertically centered in `rect`.
fn text_y(rect: Rect, font: f32) -> f32 {
    rect.min.y + (rect.height() - font * 1.25) * 0.5
}

/// A label vertically centered against button-height rows. A plain `Label`
/// allocates only its text height and so rides high next to padded buttons.
fn row_label(ui: &mut Ui, text: &str, font: f32, color: Color) {
    let style = ui.style();
    let row_h = style.fonts.body * 1.25 + style.spacing.button_padding.y * 2.0;
    let w = ui.text_mut().measure(text, font, None).x;
    let rect = ui.allocate(Vec2::new(w, row_h));
    ui.painter()
        .text(Pos2::new(rect.min.x, text_y(rect, font)), text, font, color, None);
}

fn fmt_thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

// ── entry point ─────────────────────────────────────────────────────────────

struct ToolbarAnchors {
    info: Pos2,
    settings: Pos2,
    tracy: Pos2,
}

/// Draw the profiler into the dock tab's content rect. `tab_rect` is in
/// egui points; `ppp` maps it into crusty's physical-pixel space.
pub fn profiler_panel(ui: &mut Ui, tab_rect: egui::Rect, ppp: f32, panel: &mut ProfilerPanel) {
    panel.update();
    let s = ppp;
    let rect = Rect::from_min_max(
        Pos2::new(tab_rect.min.x * s, tab_rect.min.y * s),
        Pos2::new(tab_rect.max.x * s, tab_rect.max.y * s),
    );
    let style = ui.style();
    let opts = UiOptions {
        padding: Vec2::new(4.0 * s, 2.0 * s),
        spacing: style.spacing.item,
    };
    ui.run_at(rect, Direction::TopDown, Id::new("engine_profiler_panel"), opts, |ui| {
        if !puffin::are_scopes_on() {
            disabled_screen(ui, s);
            return;
        }
        let state = &mut panel.state;

        let anchors = toolbar(ui, s, state);
        ui.separator();
        frame_history(ui, s, state);
        ui.separator();
        view_row(ui, s, state);
        ui.separator();
        match state.current_view {
            ProfilerView::Flamegraph => flamegraph(ui, s, state),
            ProfilerView::Table => table_view(ui, s, state),
            ProfilerView::Budget => budget_view(ui, s, state),
        }
        settings_popup(ui, s, state, anchors.settings);
        info_popup(ui, s, state, anchors.info);
        tracy_popup(ui, s, state, anchors.tracy);
    });
}

fn disabled_screen(ui: &mut Ui, s: f32) {
    let style = ui.style();
    ui.add_space((ui.available_size().y * 0.35).max(20.0 * s));
    ui.horizontal_aligned(Align::Center, |ui| {
        Label::new("Profiling is disabled").size(16.0 * s).show(ui);
    });
    ui.add_space(10.0 * s);
    ui.horizontal_aligned(Align::Center, |ui| {
        Label::new("Press F12 to enable profiling")
            .color(style.palette.text_dim)
            .show(ui);
    });
    ui.add_space(10.0 * s);
    ui.horizontal_aligned(Align::Center, |ui| {
        if Button::new("Enable Profiling").show(ui).clicked {
            puffin::set_scopes_on(true);
        }
    });
}

// ── toolbar ─────────────────────────────────────────────────────────────────

fn toolbar(ui: &mut Ui, s: f32, state: &mut ProfilerState) -> ToolbarAnchors {
    let style = ui.style();
    let row_top = ui.cursor();
    let right_edge = ui.available().max.x;
    let font = style.fonts.body;
    let pad = style.spacing.button_padding;
    let spacing = style.spacing.item;
    let btn_h = font + pad.y * 2.0;
    let info_w = ui.text_mut().measure("Info", font, None).x + pad.x * 2.0;
    let settings_w = ui.text_mut().measure("Settings", font, None).x + pad.x * 2.0;

    let mut tracy_anchor = Pos2::new(row_top.x, row_top.y + btn_h + 4.0 * s);

    // Left cluster: capture controls + selected-frame stats.
    ui.horizontal(|ui| {
        let (label, fill) = if state.is_paused {
            ("Resume", rgb(80, 180, 80))
        } else {
            ("Pause", rgb(180, 140, 60))
        };
        let r = Button::new(label).fill(fill).text_color(Color::WHITE).show(ui);
        if r.clicked {
            state.toggle_pause();
        }
        if r.hovered {
            show_tooltip_for(ui, r.rect, "Toggle capture (Space)");
        }

        let r = Button::new("Clear").fill(btn_fill()).show(ui);
        if r.clicked {
            state.clear();
        }
        if r.hovered {
            show_tooltip_for(ui, r.rect, "Clear captured frame history");
        }

        ui.add_space(4.0 * s);
        ui.separator();
        ui.add_space(4.0 * s);

        let (tlabel, tfill, ttext) = if state.tracy_state.is_connected() {
            ("Tracy \u{25CF}", rgb(50, 100, 50), gray(220))
        } else if state.tracy_state.is_enabled() {
            ("Tracy \u{25CB}", rgb(100, 80, 30), gray(220))
        } else {
            // Disabled look (egui uses a non-interactive dimmed button).
            ("Tracy", gray(30), gray(110))
        };
        let r = Button::new(tlabel).fill(tfill).text_color(ttext).show(ui);
        if r.clicked {
            state.show_tracy_popup = !state.show_tracy_popup;
        }
        tracy_anchor = Pos2::new(r.rect.min.x, r.rect.max.y + 4.0 * s);

        if state.is_paused {
            ui.add_space(4.0 * s);
            row_label(ui, "PAUSED", font, rgb(255, 220, 80));
            ui.add_space(8.0 * s);
        }

        ui.separator();
        ui.add_space(4.0 * s);

        if let Some(frame) = state.selected_frame() {
            let dur = frame.duration_ms();
            let color = fps_color(dur);
            row_label(ui, &format!("Frame #{}", frame.frame_number), font, gray(200));
            ui.separator();
            row_label(ui, &format!("{dur:.2} ms"), font, color);
            ui.separator();
            row_label(ui, &format!("{:.0} FPS", 1000.0 / dur), font, color);
            ui.separator();
            row_label(ui, &format!("{} threads", frame.thread_count()), font, gray(180));
            ui.separator();
            row_label(ui, &format!("{} scopes", frame.total_scopes), font, gray(180));
            ui.separator();
            row_label(
                ui,
                &format!("{:.1} KB", frame.data_size_bytes as f64 / 1024.0),
                font,
                gray(160),
            );
        } else {
            row_label(ui, "No frame selected", font, style.palette.text_dim);
        }
    });

    // Right cluster: Info + Settings, right-anchored deterministically
    // (egui hid these at narrow widths; here they are always visible).
    let total = info_w + settings_w + spacing;
    let rrect = Rect::from_min_size(
        Pos2::new(right_edge - total, row_top.y),
        Vec2::new(total, btn_h),
    );
    let ropts = UiOptions { padding: Vec2::ZERO, spacing };
    let ((info_anchor, settings_anchor), _) = ui.run_at(
        rrect,
        Direction::LeftToRight,
        Id::new("profiler_toolbar_right"),
        ropts,
        |ui| {
            let ri = Button::new("Info").fill(btn_fill()).show(ui);
            if ri.clicked {
                state.show_info = !state.show_info;
            }
            let rs = Button::new("Settings").fill(btn_fill()).show(ui);
            if rs.clicked {
                state.show_settings = !state.show_settings;
            }
            (
                Pos2::new(ri.rect.min.x, ri.rect.max.y + 4.0 * s),
                Pos2::new(rs.rect.max.x, rs.rect.max.y + 4.0 * s),
            )
        },
    );

    // Second row: aggregate statistics over the whole history.
    let stats = state.frame_statistics();
    if stats.frame_count > 0 {
        ui.horizontal(|ui| {
            ui.add_space(8.0 * s);
            let avg_color = if stats.avg_fps >= 60.0 {
                rgb(80, 200, 80)
            } else if stats.avg_fps >= 30.0 {
                rgb(220, 180, 60)
            } else {
                rgb(220, 80, 80)
            };
            Label::new("Avg:").color(gray(140)).show(ui);
            Label::new(format!("{:.2} ms", stats.avg_duration_ms))
                .color(avg_color)
                .show(ui);
            Label::new(format!("({:.0} FPS)", stats.avg_fps)).color(avg_color).show(ui);
            ui.separator();
            Label::new("Min:").color(gray(140)).show(ui);
            Label::new(format!("{:.2} ms", stats.min_duration_ms))
                .color(gray(180))
                .show(ui);
            ui.separator();
            Label::new("Max:").color(gray(140)).show(ui);
            Label::new(format!("{:.2} ms", stats.max_duration_ms))
                .color(gray(180))
                .show(ui);
            ui.separator();
            Label::new(format!("({} frames)", stats.frame_count))
                .color(gray(120))
                .show(ui);
        });
    }

    ToolbarAnchors {
        info: info_anchor,
        settings: settings_anchor,
        tracy: tracy_anchor,
    }
}

// ── frame history ───────────────────────────────────────────────────────────

fn frame_history(ui: &mut Ui, s: f32, state: &mut ProfilerState) {
    let style = ui.style();
    let chart_h = 60.0 * s;
    let avail_w = ui.available_size().x;
    let chart = ui.allocate(Vec2::new(avail_w, chart_h));

    if state.frame_history.is_empty() {
        let mut p = ui.painter();
        p.rect_filled(chart, 0.0, gray(30));
        let text = "No frames captured";
        let size = p.measure_text(text, style.fonts.body, None);
        p.text(
            Pos2::new(chart.center().x - size.x * 0.5, chart.center().y - size.y * 0.5),
            text,
            style.fonts.body,
            gray(120),
            None,
        );
        return;
    }

    let total = state.frame_history.len();
    let max_bars = if state.settings.max_display_bars == 0 {
        ((chart.width() / (3.0 * s)) as usize).max(1)
    } else {
        state.settings.max_display_bars
    };
    let n = max_bars.min(total).max(1);
    let overflow = total.saturating_sub(n);

    if !state.is_paused {
        state.frame_history_scroll = 0;
    }
    state.frame_history_scroll = state.frame_history_scroll.min(overflow);

    // (history index, duration) of the frames on screen, left → right.
    let display: Vec<(usize, f64)> = match state.frame_history_mode {
        FrameHistoryMode::Live => {
            let start = overflow - state.frame_history_scroll;
            (start..start + n)
                .map(|i| (i, state.frame_history[i].duration_ms()))
                .collect()
        }
        FrameHistoryMode::Slowest => {
            let mut all: Vec<(usize, f64)> = state
                .frame_history
                .iter()
                .enumerate()
                .map(|(i, f)| (i, f.duration_ms()))
                .collect();
            all.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            all.truncate(n);
            all
        }
    };

    let max_ms = match state.settings.frame_bar_scale {
        FrameBarScale::Fixed => 50.0,
        FrameBarScale::Auto => display
            .iter()
            .map(|(_, d)| *d)
            .fold(1.0f64, f64::max),
    };

    let resp = ui.interact(Id::new("profiler_frame_history"), chart);
    let pointer = ui.ctx().input.pointer_pos;

    let bar_pad = 1.0 * s;
    let bar_w = ((chart.width() - bar_pad * n as f32) / n as f32).max(2.0 * s);
    let mut hovered_bar: Option<(usize, Rect)> = None; // display idx + bar rect

    {
        let mut p = ui.painter();
        p.rect_filled(chart, 0.0, gray(30));

        // FPS threshold guide lines (60 / 30 FPS).
        for (ms, line, label) in [
            (16.67f64, rgba(100, 200, 100, 100), "60"),
            (33.33, rgba(220, 180, 60, 100), "30"),
        ] {
            if ms < max_ms {
                let y = chart.max.y - (ms / max_ms) as f32 * chart_h;
                p.line_segment(
                    Pos2::new(chart.min.x, y),
                    Pos2::new(chart.max.x - 20.0 * s, y),
                    1.0,
                    line,
                );
                let sz = p.measure_text(label, 10.0 * s, None);
                let lc = line.to_srgb_u8();
                p.text(
                    Pos2::new(chart.max.x - 18.0 * s, y - sz.y * 0.5),
                    label,
                    10.0 * s,
                    Color::from_srgb_u8(lc[0], lc[1], lc[2], 255),
                    None,
                );
            }
        }

        for (i, (orig, dur)) in display.iter().enumerate() {
            let x = chart.min.x + i as f32 * (bar_w + bar_pad);
            let h = ((dur / max_ms) as f32 * chart_h).clamp(2.0 * s, chart_h);
            let bar = Rect::from_min_max(
                Pos2::new(x, chart.max.y - h),
                Pos2::new(x + bar_w, chart.max.y),
            );
            let hover = resp.hovered
                && pointer.map_or(false, |pt| {
                    pt.x >= x && pt.x < x + bar_w + bar_pad && chart.contains(pt)
                });
            if hover {
                hovered_bar = Some((i, bar));
            }
            let base = frame_bar_color_fps(*dur);
            let selected = state.selected_frame_index == Some(*orig);
            let color = if selected {
                c32(lighten(base, 0.3))
            } else if hover {
                c32(lighten(base, 0.15))
            } else {
                c32(base)
            };
            p.rect_filled(bar, Rounding::same(1.0 * s), color);
            if selected {
                p.rect_stroke(bar.expand(1.0 * s), Rounding::same(1.0 * s), 2.0, Color::WHITE);
            }
        }
    }

    // Bar selection + hover tooltip.
    if let Some((i, bar)) = hovered_bar {
        let (orig, dur) = display[i];
        let frame = &state.frame_history[orig];
        let text = format!(
            "Frame #{}\n{:.2} ms ({:.0} FPS)\n{} threads, {} scopes\nClick to select and pause",
            frame.frame_number,
            dur,
            1000.0 / dur,
            frame.thread_count(),
            frame.total_scopes,
        );
        show_tooltip_for(ui, bar, &text);
        if resp.clicked {
            state.selected_frame_index = Some(orig);
            state.is_paused = true;
        }
    }

    // Wheel over the chart scrolls back through history (Live + overflow).
    if resp.hovered && state.frame_history_mode == FrameHistoryMode::Live && overflow > 0 {
        let d = ui.ctx().input.scroll_delta.y;
        if d != 0.0 {
            let step = (d / 20.0 * n as f32 / 10.0).round() as i64;
            if step != 0 {
                let cur = state.frame_history_scroll as i64;
                state.frame_history_scroll = (cur + step).clamp(0, overflow as i64) as usize;
                state.is_paused = true;
            }
        }
    }

    // Control bar: mode toggle + range info / inline scrollbar.
    ui.horizontal(|ui| {
        toggle_chip(ui, &mut state.frame_history_mode, FrameHistoryMode::Live, "Live");
        if toggle_chip(ui, &mut state.frame_history_mode, FrameHistoryMode::Slowest, "Slowest") {
            state.is_paused = true;
        }
        ui.separator();

        match state.frame_history_mode {
            FrameHistoryMode::Slowest => {
                row_label(
                    ui,
                    &format!("Showing {n} slowest of {total} frames"),
                    style.fonts.small,
                    gray(160),
                );
            }
            FrameHistoryMode::Live if overflow > 0 => {
                // Inline scrollbar: thumb at the right = newest frames.
                let label = {
                    let start = overflow - state.frame_history_scroll;
                    format!("{}-{} of {}", start + 1, start + n, total)
                };
                let label_w = ui.text_mut().measure(&label, style.fonts.small, None).x;
                let track_w = (ui.available_size().x - label_w - 16.0 * s).max(60.0 * s);
                let row_h = style.fonts.body + style.spacing.button_padding.y * 2.0;
                let slot = ui.allocate(Vec2::new(track_w, row_h));
                let track = Rect::from_min_max(
                    Pos2::new(slot.min.x, slot.center().y - 3.0 * s),
                    Pos2::new(slot.max.x, slot.center().y + 3.0 * s),
                );
                let sr = ui.interact(Id::new("profiler_fh_scrollbar"), slot);
                let ratio = n as f32 / total as f32;
                let thumb_w = (track.width() * ratio).max(20.0 * s);
                if sr.pressed {
                    if let Some(pt) = ui.ctx().input.pointer_pos {
                        let f = ((pt.x - track.min.x - thumb_w * 0.5)
                            / (track.width() - thumb_w).max(1.0))
                        .clamp(0.0, 1.0);
                        state.frame_history_scroll =
                            ((1.0 - f) * overflow as f32).round() as usize;
                        state.is_paused = true;
                    }
                }
                let frac = 1.0 - state.frame_history_scroll as f32 / overflow as f32;
                let thumb_x = track.min.x + (track.width() - thumb_w) * frac;
                let thumb = Rect::from_min_max(
                    Pos2::new(thumb_x, track.min.y),
                    Pos2::new(thumb_x + thumb_w, track.max.y),
                );
                let thumb_color = if sr.pressed {
                    gray(180)
                } else if sr.hovered {
                    gray(140)
                } else {
                    gray(100)
                };
                let mut p = ui.painter();
                p.rect_filled(track, Rounding::same(3.0 * s), gray(40));
                p.rect_filled(thumb, Rounding::same(3.0 * s), thumb_color);
                drop(p);
                row_label(ui, &label, style.fonts.small, gray(160));
            }
            _ => {
                row_label(ui, &format!("{total} frames"), style.fonts.small, gray(160));
            }
        }
    });
}

// ── view row ────────────────────────────────────────────────────────────────

/// Selection fill: accent over the backdrop, strong enough to read as a
/// solid chip (egui's 0.55 tint was barely visible on our darker theme).
fn selection_fill(accent: Color) -> Color {
    Color::rgba(accent.r, accent.g, accent.b, 0.8)
}

/// egui `Button::selected` equivalent: a regular button that switches to the
/// accent selection fill when active. Returns true on click.
fn toggle_chip<T: PartialEq + Copy>(ui: &mut Ui, value: &mut T, option: T, label: &str) -> bool {
    let clicked = if *value == option {
        let a = ui.style().palette.accent;
        Button::new(label)
            .fill(selection_fill(a))
            .text_color(gray(255))
            .show(ui)
            .clicked
    } else {
        Button::new(label).fill(btn_fill()).show(ui).clicked
    };
    if clicked {
        *value = option;
    }
    clicked
}

/// egui `selectable_value` equivalent: frameless text chip. Crusty's
/// `SelectableValue` fills the row width (it is a popup-list row) and
/// `Button` always draws a border, so this paints directly.
fn select_chip<T: PartialEq + Copy>(ui: &mut Ui, value: &mut T, option: T, label: &str) -> bool {
    let style = ui.style();
    let font = style.fonts.body;
    let pad = style.spacing.button_padding;
    let text_size = ui.text_mut().measure(label, font, None);
    let id = ui.alloc_id(label);
    let rect = ui.allocate(Vec2::new(text_size.x + pad.x * 2.0, text_size.y + pad.y * 2.0));
    let resp = ui.interact(id, rect);
    let selected = *value == option;
    if selected || resp.hovered {
        let fill = if selected {
            selection_fill(style.palette.accent)
        } else {
            style.palette.surface_hover
        };
        ui.painter().rect_filled(rect, style.rounding.widget, fill);
    }
    // White on the accent fill — accent-on-accent text was unreadable.
    let text_color = if selected { gray(255) } else { style.palette.text };
    ui.painter().text(
        Pos2::new(rect.min.x + pad.x, rect.min.y + pad.y),
        label,
        font,
        text_color,
        None,
    );
    if resp.clicked {
        *value = option;
    }
    resp.clicked
}

fn view_row(ui: &mut Ui, s: f32, state: &mut ProfilerState) {
    ui.horizontal(|ui| {
        let (font, text) = { let st = ui.style(); (st.fonts.body, st.palette.text) };
        row_label(ui, "View:", font, text);
        select_chip(ui, &mut state.current_view, ProfilerView::Flamegraph, "Flamegraph");
        select_chip(ui, &mut state.current_view, ProfilerView::Table, "Table");
        select_chip(ui, &mut state.current_view, ProfilerView::Budget, "Budget");
        ui.separator();
        row_label(ui, "Filter:", font, text);
        TextEdit::new(&mut state.filter_text)
            .width(150.0 * s)
            .fill(rgb(14, 14, 17))
            .show_full(ui);
        if Button::new("x").fill(btn_fill()).show(ui).clicked {
            state.filter_text.clear();
        }
    });
}

// ── flamegraph ──────────────────────────────────────────────────────────────

struct FgCtx {
    origin_x: f32,
    zoom: f64,
    start_ms: f64,
    t0_ns: i64,
    content: Rect,
    settings: ProfilerSettings,
    filter: String,
    pointer: Option<Pos2>,
    s: f32,
}

struct HoveredScope {
    name: String,
    location: String,
    start_ms: f64,
    duration_ms: f64,
    children: usize,
    rect: Rect,
}

#[derive(Clone, Copy)]
struct FgDrag {
    pos: Pos2,
    total: f32,
}

fn effective_view(state: &ProfilerState, width: f32, time_range: f64) -> (f64, f64) {
    if state.user_zoomed {
        (state.flamegraph_zoom.max(1e-9), state.flamegraph_pan_ms)
    } else {
        (width as f64 / time_range, 0.0)
    }
}

fn zoom_about(state: &mut ProfilerState, factor: f64, center_ms: f64, width: f32, time_range: f64) {
    let (zoom, start) = effective_view(state, width, time_range);
    let auto = width as f64 / time_range;
    let new_zoom = (zoom * factor).clamp(auto * 0.1, 10_000_000.0);
    let px = (center_ms - start) * zoom;
    state.flamegraph_pan_ms = center_ms - px / new_zoom;
    state.flamegraph_zoom = new_zoom;
    state.user_zoomed = true;
}

/// Major grid spacing: 1-2-5 series targeting ~90 px between majors.
fn nice_spacing(zoom: f64) -> f64 {
    let raw = (90.0 / zoom).max(1e-6);
    let mag = 10f64.powf(raw.log10().floor());
    let n = raw / mag;
    let nice = if n <= 1.0 {
        1.0
    } else if n <= 2.0 {
        2.0
    } else if n <= 5.0 {
        5.0
    } else {
        10.0
    };
    (nice * mag).max(0.001)
}

fn fmt_axis(ms: f64, spacing: f64) -> String {
    if ms.abs() < 1e-9 {
        "0ms".into()
    } else if ms >= 1000.0 {
        format!("{:.1}s", ms / 1000.0)
    } else if spacing >= 1.0 {
        format!("{ms:.0}ms")
    } else if spacing >= 0.1 {
        format!("{ms:.1}ms")
    } else {
        format!("{ms:.2}ms")
    }
}

fn centered_message(ui: &mut Ui, text: &str) {
    let style = ui.style();
    let avail = Rect::from_min_max(ui.cursor(), ui.available().max);
    ui.allocate(avail.size());
    let mut p = ui.painter();
    let size = p.measure_text(text, style.fonts.body, None);
    p.text(
        Pos2::new(avail.center().x - size.x * 0.5, avail.center().y - size.y * 0.5),
        text,
        style.fonts.body,
        gray(120),
        None,
    );
}

fn flamegraph(ui: &mut Ui, s: f32, state: &mut ProfilerState) {
    let Some(frame) = state.selected_frame().cloned() else {
        centered_message(ui, "No frame selected");
        return;
    };
    if frame.threads.iter().all(|t| t.scopes.is_empty()) {
        centered_message(ui, "No profiling data in this frame");
        return;
    }

    let left_margin = 8.0 * s;
    let row_h = 20.0 * s;
    let header_h = 22.0 * s;

    // Breadcrumb bar.
    if !state.breadcrumb_path.is_empty() {
        let bc_h = 22.0 * s;
        let avail_w = ui.available_size().x;
        let bc_rect = ui.allocate(Vec2::new(avail_w, bc_h));
        {
            let mut p = ui.painter();
            p.rect_filled(bc_rect, 0.0, gray(35));
        }
        let content_w = avail_w - left_margin;
        let mut nav: Option<usize> = None;
        let mut reset = false;
        let bopts = UiOptions {
            padding: Vec2::new(4.0 * s, 1.0 * s),
            spacing: 4.0 * s,
        };
        ui.run_at(bc_rect, Direction::LeftToRight, Id::new("fg_breadcrumb"), bopts, |ui| {
            if Button::new("Frame").fill(gray(45)).text_color(gray(180)).show(ui).clicked {
                reset = true;
            }
            for (i, item) in state.breadcrumb_path.iter().enumerate() {
                // Painted by hand: a Label top-aligns in the row while the
                // buttons around it are taller, so ">" would sit high.
                let font = ui.style().fonts.body;
                let sep_w = ui.text_mut().measure(">", font, None).x;
                let sep = ui.allocate(Vec2::new(sep_w, bc_h - 2.0 * s));
                ui.painter()
                    .text(Pos2::new(sep.min.x, text_y(sep, font)), ">", font, gray(100), None);
                let name = if item.name.chars().count() > 20 {
                    let cut: String = item.name.chars().take(17).collect();
                    format!("{cut}...")
                } else {
                    item.name.clone()
                };
                let r = Button::new(name).fill(gray(45)).text_color(gray(180)).show(ui);
                if r.hovered {
                    show_tooltip_for(
                        ui,
                        r.rect,
                        &format!("{} ({:.2}ms)", item.name, item.duration_ms),
                    );
                }
                if r.clicked {
                    nav = Some(i);
                }
            }
        });
        if reset {
            state.reset_flamegraph_view();
        }
        if let Some(i) = nav {
            state.navigate_to_breadcrumb(i, content_w);
        }
    }

    // From the cursor down — `available()` alone spans the whole panel and
    // would paint over the toolbar rows above.
    let remaining = Rect::from_min_max(ui.cursor(), ui.available().max);
    let axis_h = 24.0 * s;
    let axis_rect = Rect::from_min_size(remaining.min, Vec2::new(remaining.width(), axis_h));
    let content = Rect::from_min_max(Pos2::new(remaining.min.x, axis_rect.max.y), remaining.max);
    if content.height() < row_h {
        return;
    }
    ui.allocate(remaining.size());

    // Content height → vertical scroll clamp.
    let mut total_h = 0.0f32;
    for t in &frame.threads {
        total_h += header_h;
        total_h += if state.collapsed_threads.contains(&t.name) {
            4.0 * s
        } else {
            (t.max_depth + 1) as f32 * row_h + 8.0 * s
        };
    }
    let max_scroll = (total_h - content.height()).max(0.0);
    state.scroll_offset_y = state.scroll_offset_y.clamp(0.0, max_scroll);

    let width = (content.width() - left_margin).max(1.0);
    // Scope timestamps are absolute; normalize to the earliest scope start
    // across threads and auto-fit to the actual span (egui parity).
    let (t0_ns, t1_ns) = frame
        .threads
        .iter()
        .filter_map(|t| t.time_range_ns())
        .fold((i64::MAX, i64::MIN), |(lo, hi), (s, e)| (lo.min(s), hi.max(e)));
    let time_range = ((t1_ns - t0_ns).max(1)) as f64 / 1e6;
    let (zoom, start_ms) = effective_view(state, width, time_range);

    let resp = ui.interact(Id::new("fg_content"), content);
    let pointer = ui.ctx().input.pointer_pos;
    let shift_down = ui.ctx().input.modifiers.contains(Modifiers::SHIFT);

    let fg = FgCtx {
        origin_x: content.min.x + left_margin,
        zoom,
        start_ms,
        t0_ns,
        content,
        settings: state.settings.clone(),
        filter: state.filter_text.to_lowercase(),
        pointer: if resp.hovered { pointer } else { None },
        s,
    };

    let mut hovered: Option<HoveredScope> = None;
    let mut headers: Vec<(Rect, String)> = Vec::new();

    // ── content pass
    ui.ctx_mut().paint.push(PaintCmd::PushClip(content));
    {
        let mut p = ui.painter();
        p.rect_filled(content, 0.0, gray(25));

        // Grid lines.
        let spacing = nice_spacing(zoom);
        let sub = state.settings.sub_grid_divisions;
        let first = (start_ms / spacing).floor() * spacing;
        let visible_range = width as f64 / zoom;
        let mut t = first;
        while t < start_ms + visible_range + spacing {
            let x = fg.origin_x + ((t - start_ms) * zoom) as f32;
            if x >= content.min.x && x <= content.max.x {
                p.line_segment(
                    Pos2::new(x, content.min.y),
                    Pos2::new(x, content.max.y),
                    1.0,
                    gray(40),
                );
            }
            if sub > 1 {
                let step = spacing / sub as f64;
                for k in 1..sub {
                    let ts = t + k as f64 * step;
                    let xs = fg.origin_x + ((ts - start_ms) * zoom) as f32;
                    if xs >= content.min.x && xs <= content.max.x {
                        p.line_segment(
                            Pos2::new(xs, content.min.y),
                            Pos2::new(xs, content.max.y),
                            1.0,
                            gray(32),
                        );
                    }
                }
            }
            t += spacing;
        }

        // Threads + scopes.
        let mut y = content.min.y - state.scroll_offset_y;
        for thread in &frame.threads {
            let header = Rect::from_min_size(
                Pos2::new(content.min.x, y),
                Vec2::new(content.width(), header_h),
            );
            let collapsed = state.collapsed_threads.contains(&thread.name);
            if header.max.y >= content.min.y && header.min.y <= content.max.y {
                p.rect_filled(header, 0.0, gray(30));
                let arrow = if collapsed { ">" } else { "v" };
                let label =
                    format!("{arrow} {} ({} scopes)", thread.name, thread.scope_count());
                p.text(
                    Pos2::new(header.min.x + 8.0 * s, text_y(header, 12.0 * s)),
                    &label,
                    12.0 * s,
                    gray(220),
                    None,
                );
            }
            headers.push((header, thread.name.clone()));
            y += header_h;
            if collapsed {
                y += 4.0 * s;
                continue;
            }
            for scope in &thread.scopes {
                draw_scope(&mut p, &fg, scope, 0, y, false, &mut hovered);
            }
            y += (thread.max_depth + 1) as f32 * row_h + 8.0 * s;
        }
    }
    ui.ctx_mut().paint.push(PaintCmd::PopClip);

    // ── axis pass + cursor line
    {
        let mut p = ui.painter();
        p.rect_filled(axis_rect, 0.0, gray(35));
        p.line_segment(
            Pos2::new(axis_rect.min.x, axis_rect.max.y),
            Pos2::new(axis_rect.max.x, axis_rect.max.y),
            1.0,
            gray(50),
        );
        let spacing = nice_spacing(zoom);
        let first = (start_ms / spacing).floor() * spacing;
        let visible_range = width as f64 / zoom;
        let mut t = first;
        while t < start_ms + visible_range + spacing {
            let x = fg.origin_x + ((t - start_ms) * zoom) as f32;
            if x >= axis_rect.min.x && x <= axis_rect.max.x {
                p.line_segment(
                    Pos2::new(x, axis_rect.max.y - 6.0 * s),
                    Pos2::new(x, axis_rect.max.y),
                    1.0,
                    gray(100),
                );
                let label = fmt_axis(t, spacing);
                p.text(
                    Pos2::new(x + 3.0 * s, text_y(axis_rect, 11.0 * s)),
                    &label,
                    11.0 * s,
                    gray(180),
                    None,
                );
            }
            t += spacing;
        }

        // Cursor line + time readout.
        if let Some(pt) = pointer {
            if content.contains(pt) || axis_rect.contains(pt) {
                let (line_x, line_color) = if shift_down {
                    if let Some(h) = &hovered {
                        let snap = if (pt.x - h.rect.min.x).abs() < (pt.x - h.rect.max.x).abs() {
                            h.rect.min.x
                        } else {
                            h.rect.max.x
                        };
                        (snap, rgba(100, 200, 255, 180))
                    } else {
                        (pt.x, rgba(255, 255, 255, 180))
                    }
                } else {
                    (pt.x, rgba(255, 255, 255, 180))
                };
                p.line_segment(
                    Pos2::new(line_x, axis_rect.min.y),
                    Pos2::new(line_x, content.max.y),
                    1.0,
                    line_color,
                );
                let t_ms = start_ms + (line_x - fg.origin_x) as f64 / zoom;
                let label = format!("{t_ms:.3}ms");
                let sz = p.measure_text(&label, 11.0 * s, None);
                let lx = (line_x - sz.x * 0.5)
                    .clamp(axis_rect.min.x, axis_rect.max.x - sz.x - 4.0 * s);
                let bg = Rect::from_min_size(
                    Pos2::new(lx - 3.0 * s, axis_rect.min.y + 2.0 * s),
                    Vec2::new(sz.x + 6.0 * s, sz.y + 2.0 * s),
                );
                p.rect_filled(bg, Rounding::same(3.0 * s), rgba(30, 30, 30, 230));
                p.text(
                    Pos2::new(lx, axis_rect.min.y + 3.0 * s),
                    &label,
                    11.0 * s,
                    gray(230),
                    None,
                );
            }
        }
    }

    // ── input handling (applies next frame — standard IM pattern)
    let drag_id = Id::new("fg_drag_state");
    if resp.pressed {
        if let Some(pt) = pointer {
            let prev = ui.ctx().memory.data_get::<FgDrag>(drag_id).copied();
            match prev {
                Some(mut d) => {
                    let dx = pt.x - d.pos.x;
                    let dy = pt.y - d.pos.y;
                    d.total += dx.abs() + dy.abs();
                    d.pos = pt;
                    ui.ctx_mut().memory.data_insert(drag_id, d);
                    if dx != 0.0 {
                        state.flamegraph_pan_ms = start_ms - dx as f64 / zoom;
                        state.flamegraph_zoom = zoom;
                        state.user_zoomed = true;
                    }
                    if dy != 0.0 {
                        state.scroll_offset_y =
                            (state.scroll_offset_y - dy).clamp(0.0, max_scroll);
                    }
                }
                None => ui
                    .ctx_mut()
                    .memory
                    .data_insert(drag_id, FgDrag { pos: pt, total: 0.0 }),
            }
        }
    }
    let drag_total = ui
        .ctx()
        .memory
        .data_get::<FgDrag>(drag_id)
        .map(|d| d.total)
        .unwrap_or(0.0);
    let is_click = resp.clicked && drag_total < 4.0 * s;
    if !ui.ctx().input.pointer_down && !resp.pressed {
        ui.ctx_mut().memory.data_remove::<FgDrag>(drag_id);
    }

    // Scroll: Ctrl = zoom about the pointer, plain = vertical scroll.
    let sd = ui.ctx().input.scroll_delta;
    let ctrl = ui.ctx().input.modifiers.contains(Modifiers::CTRL);
    if resp.hovered && sd.y != 0.0 {
        if ctrl {
            let center_ms = pointer
                .map(|pt| start_ms + (pt.x - fg.origin_x) as f64 / zoom)
                .unwrap_or(start_ms + width as f64 / zoom * 0.5);
            let factor = if sd.y > 0.0 { 1.15 } else { 0.87 };
            zoom_about(state, factor, center_ms, width, time_range);
        } else {
            state.scroll_offset_y = (state.scroll_offset_y - sd.y).clamp(0.0, max_scroll);
        }
    }

    // Keyboard (only when nothing has text focus).
    if resp.hovered && ui.ctx().memory.focused_widget.is_none() {
        let center_ms = start_ms + width as f64 / zoom * 0.5;
        let visible = width as f64 / zoom;
        let inp = &ui.ctx().input;
        let reset = inp.key_pressed(Key::Char('r')) || inp.key_pressed(Key::Char('R'));
        let zoom_in = inp.key_pressed(Key::Char('+')) || inp.key_pressed(Key::Char('='));
        let zoom_out = inp.key_pressed(Key::Char('-'));
        let left = inp.key_pressed(Key::ArrowLeft);
        let right = inp.key_pressed(Key::ArrowRight);
        let up = inp.key_pressed(Key::ArrowUp);
        let down = inp.key_pressed(Key::ArrowDown);
        let space = inp.key_pressed(Key::Space);
        if reset {
            state.reset_flamegraph_view();
        }
        if zoom_in {
            zoom_about(state, 1.25, center_ms, width, time_range);
        }
        if zoom_out {
            zoom_about(state, 0.8, center_ms, width, time_range);
        }
        if left || right {
            let delta = visible * 0.1 * if left { -1.0 } else { 1.0 };
            state.flamegraph_pan_ms = start_ms + delta;
            state.flamegraph_zoom = zoom;
            state.user_zoomed = true;
        }
        if up || down {
            let delta = 50.0 * s * if up { -1.0 } else { 1.0 };
            state.scroll_offset_y = (state.scroll_offset_y + delta).clamp(0.0, max_scroll);
        }
        if space {
            state.toggle_pause();
        }
    }

    // Click: thread header toggle, or zoom-to-scope. Double-click resets.
    if resp.double_clicked(ui) {
        state.reset_flamegraph_view();
    } else if is_click {
        if let Some(pt) = pointer {
            if let Some((_, name)) = headers.iter().find(|(r, _)| r.contains(pt)) {
                if !state.collapsed_threads.remove(name) {
                    state.collapsed_threads.insert(name.clone());
                }
            } else if let Some(h) = &hovered {
                state.push_breadcrumb(h.name.clone(), h.start_ms, h.duration_ms);
                let visible = h.duration_ms / 0.6;
                state.flamegraph_zoom = (width as f64 / visible).clamp(1.0, 10_000_000.0);
                state.flamegraph_pan_ms =
                    h.start_ms + h.duration_ms * 0.5 - visible * 0.5;
                state.user_zoomed = true;
            }
        }
    }

    // Hover tooltip.
    if let Some(h) = &hovered {
        let text = format!(
            "{}\n{}\nDuration: {:.3} ms\nChildren: {}\nClick to zoom | R: reset | +/-: zoom | Arrows: pan",
            h.name, h.location, h.duration_ms, h.children,
        );
        show_tooltip_for(ui, h.rect, &text);
    }
}

fn draw_scope(
    p: &mut Painter,
    fg: &FgCtx,
    scope: &ProfileScope,
    depth: usize,
    base_y: f32,
    ancestor_match: bool,
    hovered: &mut Option<HoveredScope>,
) {
    let rel_start_ms = (scope.start_ns - fg.t0_ns) as f64 / 1e6;
    let rel_end_ms = rel_start_ms + scope.duration_ms();
    let x0 = fg.origin_x + ((rel_start_ms - fg.start_ms) * fg.zoom) as f32;
    let x1 = fg.origin_x + ((rel_end_ms - fg.start_ms) * fg.zoom) as f32;
    if x1 < fg.content.min.x || x0 > fg.content.max.x {
        return;
    }
    let row_h = 20.0 * fg.s;
    let w = x1 - x0;
    let y = base_y + depth as f32 * row_h;
    let rect = Rect::from_min_max(Pos2::new(x0, y), Pos2::new(x1, y + row_h - 2.0 * fg.s));
    let visible_v = rect.max.y >= fg.content.min.y && rect.min.y <= fg.content.max.y;

    if w >= 2.0 * fg.s && visible_v {
        let mut c = scope_color(scope.duration_ms(), &fg.settings);
        // A match keeps its whole subtree bright (ancestor_match), not just
        // the path of ancestors above it — dimming children of a match reads
        // as a broken vertical stripe.
        if !fg.filter.is_empty()
            && !ancestor_match
            && !scope.matches_filter_recursive(&fg.filter)
        {
            c = dim_color(c, 0.7);
        }
        let is_hover = fg
            .pointer
            .map_or(false, |pt| rect.contains(pt) && fg.content.contains(pt));
        if is_hover {
            c = lighten(c, 0.2);
            *hovered = Some(HoveredScope {
                name: scope.name.to_string(),
                location: scope.location.to_string(),
                start_ms: rel_start_ms,
                duration_ms: scope.duration_ms(),
                children: scope.children.len(),
                rect,
            });
        }
        p.rect_filled(rect, Rounding::same(3.0 * fg.s), c32(c));
        p.rect_stroke(rect, Rounding::same(3.0 * fg.s), 1.0, c32(darken(c, 0.3)));

        if w > 40.0 * fg.s {
            let label = if w > 150.0 * fg.s {
                format!("{} ({:.2}ms)", scope.name, scope.duration_ms())
            } else if w > 80.0 * fg.s {
                scope.name.to_string()
            } else {
                let max_chars = ((w - 10.0 * fg.s) / (6.0 * fg.s)) as usize;
                let n = scope.name.chars().count();
                if n > max_chars && max_chars > 3 {
                    let cut: String = scope.name.chars().take(max_chars - 3).collect();
                    format!("{cut}...")
                } else {
                    scope.name.to_string()
                }
            };
            let lum = 0.299 * c.r() as f32 + 0.587 * c.g() as f32 + 0.114 * c.b() as f32;
            let tc = if lum > 140.0 { gray(30) } else { gray(250) };
            let clip = rect.intersect(fg.content);
            p.paint_mut().push(PaintCmd::PushClip(clip));
            let tx = rect.min.x.max(fg.content.min.x) + 4.0 * fg.s;
            p.text(Pos2::new(tx, text_y(rect, 11.0 * fg.s)), &label, 11.0 * fg.s, tc, None);
            p.paint_mut().push(PaintCmd::PopClip);
        }
    }

    let child_ancestor_match =
        ancestor_match || (!fg.filter.is_empty() && scope.matches_filter(&fg.filter));
    for child in &scope.children {
        draw_scope(p, fg, child, depth + 1, base_y, child_ancestor_match, hovered);
    }
}

// ── table view (redesigned) ─────────────────────────────────────────────────

fn table_view(ui: &mut Ui, s: f32, state: &mut ProfilerState) {
    let style = ui.style();
    let frame_ms = state.selected_frame().map(|f| f.duration_ms()).unwrap_or(0.0);

    // (label, fraction of width, right-aligned, sort key)
    let columns: [(&str, f32, bool, SortColumn); 6] = [
        ("Name", 0.28, false, SortColumn::Name),
        ("Location", 0.26, false, SortColumn::Location),
        ("Calls", 0.08, true, SortColumn::CallCount),
        ("Total", 0.14, true, SortColumn::TotalTime),
        ("Mean", 0.12, true, SortColumn::MeanTime),
        ("Max", 0.12, true, SortColumn::MaxTime),
    ];

    let avail_w = ui.available_size().x;
    let header_h = 22.0 * s;
    let header = ui.allocate(Vec2::new(avail_w, header_h));
    let pointer = ui.ctx().input.pointer_pos;
    let clicked = ui.interact(Id::new("profiler_table_header"), header).clicked;

    // Header cells: flat labels, hover highlight, accent + arrow when sorted.
    {
        let mut x = header.min.x;
        for (label, frac, right, key) in columns {
            let w = frac * avail_w;
            let cell = Rect::from_min_size(Pos2::new(x, header.min.y), Vec2::new(w, header_h));
            let hover = pointer.map_or(false, |pt| cell.contains(pt));
            let sorted = state.sort_column == key;
            if clicked && hover {
                if sorted {
                    state.sort_direction = match state.sort_direction {
                        SortDirection::Descending => SortDirection::Ascending,
                        SortDirection::Ascending => SortDirection::Descending,
                    };
                } else {
                    state.sort_column = key;
                    state.sort_direction = SortDirection::Descending;
                }
                state.invalidate_stats_cache();
            }
            let sorted = state.sort_column == key;
            let text = if sorted {
                let arrow = match state.sort_direction {
                    SortDirection::Descending => "v",
                    SortDirection::Ascending => "^",
                };
                format!("{label} {arrow}")
            } else {
                label.to_string()
            };
            let mut p = ui.painter();
            if hover {
                p.rect_filled(cell, Rounding::same(2.0 * s), rgba(255, 255, 255, 10));
            }
            let color = if sorted { style.palette.accent } else { gray(150) };
            let tw = p.measure_text(&text, style.fonts.small, None).x;
            let tx = if right {
                cell.max.x - tw - 6.0 * s
            } else {
                cell.min.x + 6.0 * s
            };
            p.text(
                Pos2::new(tx, text_y(cell, style.fonts.small)),
                &text,
                style.fonts.small,
                color,
                None,
            );
            x += w;
        }
        let mut p = ui.painter();
        p.line_segment(
            Pos2::new(header.min.x, header.max.y),
            Pos2::new(header.max.x, header.max.y),
            1.0,
            style.palette.stroke,
        );
    }

    // Body.
    let settings = state.settings.clone();
    let filter_empty = state.filter_text.is_empty();
    let list_h = ui.available_size().y;
    let stats = state.get_scope_stats();
    if stats.is_empty() {
        let msg = if filter_empty {
            "No scope data available"
        } else {
            "No scopes match filter"
        };
        centered_message(ui, msg);
        return;
    }
    let row_h = 22.0 * s;
    ScrollArea::new(list_h).auto_shrink(false).inset(0.0).show(ui, |ui| {
        for (i, st) in stats.iter().enumerate() {
            let row = ui.allocate(Vec2::new(ui.available_size().x.max(avail_w), row_h));
            let hover = ui.contains_pointer(row);
            let mut p = ui.painter();
            if hover {
                p.rect_filled(row, 0.0, rgba(255, 255, 255, 12));
            } else if i % 2 == 1 {
                p.rect_filled(row, 0.0, rgba(255, 255, 255, 5));
            }
            let mut x = row.min.x;
            for (ci, (_, frac, right, _)) in columns.iter().enumerate() {
                let w = frac * avail_w;
                let cell = Rect::from_min_size(Pos2::new(x, row.min.y), Vec2::new(w, row_h));
                let (text, color, font) = match ci {
                    0 => (st.name.to_string(), style.palette.text, style.fonts.body),
                    1 => (st.location.to_string(), gray(130), style.fonts.small),
                    2 => (st.call_count.to_string(), gray(180), style.fonts.body),
                    3 => (
                        format!("{:.3} ms", st.total_time_ms()),
                        time_color(st.total_time_ms(), &settings),
                        style.fonts.body,
                    ),
                    4 => (
                        format!("{:.3} ms", st.mean_time_ms()),
                        time_color(st.mean_time_ms(), &settings),
                        style.fonts.body,
                    ),
                    _ => (
                        format!("{:.3} ms", st.max_time_ms()),
                        time_color(st.max_time_ms(), &settings),
                        style.fonts.body,
                    ),
                };
                // Heat bar behind the Total cell: share of the frame.
                if ci == 3 && frame_ms > 0.0 {
                    let f = (st.total_time_ms() / frame_ms).clamp(0.0, 1.0) as f32;
                    if f > 0.01 {
                        let bar = Rect::from_min_max(
                            Pos2::new(cell.max.x - (w - 8.0 * s) * f, cell.min.y + 3.0 * s),
                            Pos2::new(cell.max.x - 2.0 * s, cell.max.y - 3.0 * s),
                        );
                        let mut bc = time_color(st.total_time_ms(), &settings);
                        bc.a = 0.13;
                        p.rect_filled(bar, Rounding::same(2.0 * s), bc);
                    }
                }
                let tw = p.measure_text(&text, font, None).x;
                let tx = if *right {
                    cell.max.x - tw - 6.0 * s
                } else {
                    cell.min.x + 6.0 * s
                };
                p.paint_mut().push(PaintCmd::PushClip(cell));
                p.text(Pos2::new(tx, text_y(cell, font)), &text, font, color, None);
                p.paint_mut().push(PaintCmd::PopClip);
                x += w;
            }
        }
    });
}

// ── budget view (redesigned) ────────────────────────────────────────────────

/// A budget bar with a marker at 100%: the bar spans 0..2× budget.
fn budget_bar(p: &mut Painter, rect: Rect, actual: f64, budget_ms: f64, color: Color, s: f32) {
    let round = Rounding::same(rect.height() * 0.3);
    p.rect_filled(rect, round, gray(38));
    let f = ((actual / (budget_ms * 2.0)).clamp(0.0, 1.0)) as f32;
    if f > 0.005 {
        let fill = Rect::from_min_size(rect.min, Vec2::new(rect.width() * f, rect.height()));
        p.rect_filled(fill, round, color);
    }
    // 100% marker.
    let mx = rect.min.x + rect.width() * 0.5;
    p.line_segment(
        Pos2::new(mx, rect.min.y - 1.0 * s),
        Pos2::new(mx, rect.max.y + 1.0 * s),
        1.0,
        rgba(230, 230, 230, 160),
    );
}

fn budget_view(ui: &mut Ui, s: f32, state: &mut ProfilerState) {
    // Scrolled + clipped: on short panels the counter cards used to paint
    // straight over the status bar below.
    let h = ui.available_size().y;
    ScrollArea::new(h)
        .auto_shrink(false)
        .inset(0.0)
        .show(ui, |ui| budget_body(ui, s, state));
}

fn budget_body(ui: &mut Ui, s: f32, state: &mut ProfilerState) {
    let style = ui.style();
    let Some(frame) = state.selected_frame().cloned() else {
        centered_message(ui, "Select a captured frame to inspect budgets");
        return;
    };
    let fb = &budget::FRAME_BUDGET;
    let dur = frame.duration_ms();
    let frame_color = if dur <= fb.total_budget_ms {
        rgb(80, 200, 80)
    } else if dur <= 33.33 {
        rgb(220, 180, 60)
    } else {
        rgb(220, 80, 80)
    };

    ui.add_space(4.0 * s);

    // Header row: title left, value right.
    let avail_w = ui.available_size().x;
    let hrow = ui.allocate(Vec2::new(avail_w, style.fonts.body + 6.0 * s));
    {
        let mut p = ui.painter();
        p.text(
            Pos2::new(hrow.min.x + 2.0 * s, text_y(hrow, style.fonts.body)),
            "Frame Budget",
            style.fonts.body,
            style.palette.text,
            None,
        );
        let value = format!("{:.2} / {:.2} ms  ({:.0} FPS)", dur, fb.total_budget_ms, 1000.0 / dur);
        let vw = p.measure_text(&value, style.fonts.body, None).x;
        p.text(
            Pos2::new(hrow.max.x - vw - 2.0 * s, text_y(hrow, style.fonts.body)),
            &value,
            style.fonts.body,
            frame_color,
            None,
        );
    }
    let bar = ui.allocate(Vec2::new(avail_w, 14.0 * s));
    {
        let mut p = ui.painter();
        budget_bar(&mut p, bar, dur, fb.total_budget_ms, frame_color, s);
    }

    ui.add_space(10.0 * s);
    Label::new("Category Budgets").color(gray(150)).size(style.fonts.small).show(ui);
    ui.add_space(4.0 * s);

    let label_w = 110.0 * s;
    let value_w = 118.0 * s;
    for category in fb.categories {
        let actual = budget::scope_total_ms(&frame, category.scopes);
        let color = if actual <= category.budget_ms {
            rgb(80, 200, 80)
        } else if actual <= category.budget_ms * 1.5 {
            rgb(220, 180, 60)
        } else {
            rgb(220, 80, 80)
        };
        let row = ui.allocate(Vec2::new(avail_w, 18.0 * s));
        let mut p = ui.painter();
        p.text(
            Pos2::new(row.min.x + 2.0 * s, text_y(row, style.fonts.body)),
            category.name,
            style.fonts.body,
            style.palette.text,
            None,
        );
        let bar = Rect::from_min_max(
            Pos2::new(row.min.x + label_w, row.min.y + 4.0 * s),
            Pos2::new(row.max.x - value_w, row.max.y - 4.0 * s),
        );
        budget_bar(&mut p, bar, actual, category.budget_ms, color, s);
        let value = format!("{:.2} / {:.2} ms", actual, category.budget_ms);
        let vw = p.measure_text(&value, style.fonts.small, None).x;
        p.text(
            Pos2::new(row.max.x - vw - 2.0 * s, text_y(row, style.fonts.small)),
            &value,
            style.fonts.small,
            color,
            None,
        );
        drop(p);
        ui.add_space(2.0 * s);
    }

    ui.add_space(10.0 * s);
    Label::new("Runtime Counters").color(gray(150)).size(style.fonts.small).show(ui);
    ui.add_space(4.0 * s);

    // 4-across stat cards.
    let rc = state.latest_render_counters.clone();
    let xc = state.latest_resource_counters.clone();
    let counters: [(&str, u64); 8] = [
        ("Draw Calls", rc.draw_calls as u64),
        ("Triangles", rc.triangles as u64),
        ("Material Changes", rc.material_changes as u64),
        ("Visible Entities", rc.visible_entities as u64),
        ("Entities", xc.entity_count as u64),
        ("Meshes", xc.mesh_count as u64),
        ("Textures", xc.texture_count as u64),
        ("Rigid Bodies", xc.rigid_body_count as u64),
    ];
    let gap = 6.0 * s;
    let card_w = (avail_w - gap * 3.0) / 4.0;
    let card_h = 36.0 * s;
    for chunk in counters.chunks(4) {
        let row = ui.allocate(Vec2::new(avail_w, card_h));
        let mut p = ui.painter();
        for (i, (label, value)) in chunk.iter().enumerate() {
            let x = row.min.x + i as f32 * (card_w + gap);
            let card = Rect::from_min_size(Pos2::new(x, row.min.y), Vec2::new(card_w, card_h));
            p.rect_filled(card, style.rounding.small, style.palette.surface);
            p.text(
                Pos2::new(card.min.x + 8.0 * s, card.min.y + 4.0 * s),
                label,
                style.fonts.small,
                gray(140),
                None,
            );
            p.text(
                Pos2::new(card.min.x + 8.0 * s, card.min.y + 5.0 * s + style.fonts.small * 1.2),
                &fmt_thousands(*value),
                style.fonts.body,
                style.palette.text,
                None,
            );
        }
        drop(p);
        ui.add_space(gap);
    }
}

// ── popups ──────────────────────────────────────────────────────────────────

/// Framed group inside a popup: content on a `surface`-colored rounded card.
fn section<R>(ui: &mut Ui, s: f32, id: &str, f: impl FnOnce(&mut Ui) -> R) -> R {
    let style = ui.style();
    let top = ui.cursor();
    let avail = ui.available();
    let insert_at = ui.ctx().paint.len();
    let pad = 8.0 * s;
    let inner = Rect::from_min_max(top, Pos2::new(avail.max.x, avail.max.y));
    let opts = UiOptions {
        padding: Vec2::new(pad, pad),
        spacing: style.spacing.item,
    };
    let (r, used) = ui.run_at(inner, Direction::TopDown, Id::new(("profiler_section", id)), opts, f);
    let bg = Rect::from_min_max(top, Pos2::new(avail.max.x, used.max.y + pad));
    ui.ctx_mut().paint.insert(
        insert_at,
        PaintCmd::Rrect(RectShape::fill(bg, Rounding::same(4.0 * s), gray(35))),
    );
    ui.allocate(Vec2::new(bg.width(), bg.height()));
    r
}

fn popup_heading(ui: &mut Ui, s: f32, text: &str) {
    let style = ui.style();
    Label::new(text).size(style.fonts.body + 1.0 * s).show(ui);
    ui.add_space(2.0 * s);
}

fn settings_popup(ui: &mut Ui, s: f32, state: &mut ProfilerState, anchor: Pos2) {
    if !state.show_settings {
        return;
    }
    let style = ui.style();
    let mut close = false;
    let mut resize_to: Option<usize> = None;
    let width = 330.0 * s;
    let pos = Pos2::new(anchor.x - width, anchor.y);
    let settings = &mut state.settings;
    let show = &mut state.show_settings;
    Window::new("Profiler Settings")
        .open(show)
        .default_pos(pos)
        .default_size(Vec2::new(width, 470.0 * s))
        .resizable(false)
        .collapsible(false)
        .show(ui, |ui| {
            popup_heading(ui, s, "Frame History");
            section(ui, s, "history", |ui| {
                ui.horizontal(|ui| {
                    { let (f, c) = { let st = ui.style(); (st.fonts.body, st.palette.text) }; row_label(ui, "Capacity:", f, c); }
                    let mut cap = settings.history_capacity as f32;
                    DragValue::new(&mut cap)
                        .speed(50.0)
                        .range(100.0..=10000.0)
                        .decimals(0)
                        .suffix(" frames")
                        .width(110.0 * s)
                        .show(ui);
                    if cap as usize != settings.history_capacity {
                        resize_to = Some(cap as usize);
                    }
                });
                ui.horizontal(|ui| {
                    { let (f, c) = { let st = ui.style(); (st.fonts.body, st.palette.text) }; row_label(ui, "Max bars:", f, c); }
                    let mut mb = settings.max_display_bars as f32;
                    DragValue::new(&mut mb)
                        .speed(10.0)
                        .range(50.0..=500.0)
                        .decimals(0)
                        .width(80.0 * s)
                        .show(ui);
                    settings.max_display_bars = mb as usize;
                });
            });
            ui.add_space(8.0 * s);

            popup_heading(ui, s, "Performance Thresholds");
            Label::new("Affects bar colors + flamegraph")
                .size(style.fonts.small)
                .color(gray(140))
                .show(ui);
            ui.add_space(2.0 * s);
            section(ui, s, "thresholds", |ui| {
                ui.horizontal(|ui| {
                    { let (f, c) = { let st = ui.style(); (st.fonts.body, st.palette.text) }; row_label(ui, "Presets:", f, c); }
                    if Button::new("60 FPS").show(ui).clicked {
                        settings.fast_threshold_ms = 1.0;
                        settings.warning_threshold_ms = 5.0;
                        settings.slow_threshold_ms = 16.67;
                    }
                    if Button::new("30 FPS").show(ui).clicked {
                        settings.fast_threshold_ms = 5.0;
                        settings.warning_threshold_ms = 16.0;
                        settings.slow_threshold_ms = 33.33;
                    }
                    if Button::new("144 FPS").show(ui).clicked {
                        settings.fast_threshold_ms = 0.5;
                        settings.warning_threshold_ms = 2.0;
                        settings.slow_threshold_ms = 6.9;
                    }
                });
                ui.add_space(4.0 * s);
                let warn = settings.warning_threshold_ms;
                threshold_row(
                    ui,
                    s,
                    rgb(80, 200, 80),
                    "Fast <",
                    &mut settings.fast_threshold_ms,
                    0.1,
                    0.1..=(warn - 0.1).max(0.2),
                );
                let fast = settings.fast_threshold_ms;
                let slow = settings.slow_threshold_ms;
                threshold_row(
                    ui,
                    s,
                    rgb(220, 180, 60),
                    "Warning <",
                    &mut settings.warning_threshold_ms,
                    0.5,
                    (fast + 0.1)..=(slow - 0.1).max(fast + 0.2),
                );
                let warn = settings.warning_threshold_ms;
                threshold_row(
                    ui,
                    s,
                    rgb(220, 80, 80),
                    "Slow >=",
                    &mut settings.slow_threshold_ms,
                    1.0,
                    (warn + 0.1)..=100.0,
                );
            });
            ui.add_space(8.0 * s);

            popup_heading(ui, s, "Flamegraph Grid");
            section(ui, s, "grid", |ui| {
                ui.horizontal(|ui| {
                    { let (f, c) = { let st = ui.style(); (st.fonts.body, st.palette.text) }; row_label(ui, "Major grid:", f, c); }
                    ComboBox::new("profiler_grid_major")
                        .selected_text(format!("{} ms", settings.grid_spacing_ms))
                        .width(90.0 * s)
                        .show_ui(ui, |ui| {
                            for v in [0.1f32, 0.5, 1.0, 2.0, 5.0, 10.0] {
                                SelectableValue::new(
                                    &mut settings.grid_spacing_ms,
                                    v,
                                    format!("{v} ms"),
                                )
                                .show(ui);
                            }
                        });
                });
                ui.horizontal(|ui| {
                    { let (f, c) = { let st = ui.style(); (st.fonts.body, st.palette.text) }; row_label(ui, "Sub-div:", f, c); }
                    let cur = if settings.sub_grid_divisions == 0 {
                        "None".to_string()
                    } else {
                        settings.sub_grid_divisions.to_string()
                    };
                    ComboBox::new("profiler_grid_sub")
                        .selected_text(cur)
                        .width(70.0 * s)
                        .show_ui(ui, |ui| {
                            for (v, label) in
                                [(0u8, "None"), (2, "2"), (4, "4"), (5, "5"), (10, "10")]
                            {
                                SelectableValue::new(&mut settings.sub_grid_divisions, v, label)
                                    .show(ui);
                            }
                        });
                });
            });
            ui.add_space(10.0 * s);

            ui.horizontal(|ui| {
                if Button::new("Reset to Defaults").show(ui).clicked {
                    *settings = ProfilerSettings::default();
                    resize_to = Some(settings.history_capacity);
                }
                if Button::new("Close").show(ui).clicked {
                    close = true;
                }
            });
        });
    if close {
        state.show_settings = false;
    }
    if let Some(cap) = resize_to {
        state.resize_history(cap);
    }
}

fn threshold_row(
    ui: &mut Ui,
    s: f32,
    color: Color,
    label: &str,
    value: &mut f32,
    speed: f64,
    range: std::ops::RangeInclusive<f32>,
) {
    ui.horizontal(|ui| {
        let (font, tc, row_h) = {
            let st = ui.style();
            (
                st.fonts.body,
                st.palette.text,
                st.fonts.body * 1.25 + st.spacing.button_padding.y * 2.0,
            )
        };
        // Swatch centered in the row (a bare 12px allocation would top-align).
        let slot = ui.allocate(Vec2::new(12.0 * s, row_h));
        let cy = (slot.min.y + slot.max.y) * 0.5;
        let sw = Rect::from_min_max(
            Pos2::new(slot.min.x, cy - 6.0 * s),
            Pos2::new(slot.max.x, cy + 6.0 * s),
        );
        let mut p = ui.painter();
        p.rect_filled(sw, Rounding::same(2.0 * s), color);
        drop(p);
        row_label(ui, label, font, tc);
        DragValue::new(value)
            .speed(speed)
            .range(range)
            .suffix(" ms")
            .width(80.0 * s)
            .show(ui);
    });
}

fn info_popup(ui: &mut Ui, s: f32, state: &mut ProfilerState, anchor: Pos2) {
    if !state.show_info {
        return;
    }
    let style = ui.style();
    let mut close = false;
    let width = 300.0 * s;
    let pos = Pos2::new(anchor.x - width * 0.5, anchor.y);
    let dim = gray(170);
    let show = &mut state.show_info;
    Window::new("Profiler Help")
        .open(show)
        .default_pos(pos)
        .default_size(Vec2::new(width, 360.0 * s))
        .resizable(false)
        .collapsible(false)
        .show(ui, |ui| {
            popup_heading(ui, s, "Keyboard Shortcuts");
            section(ui, s, "keys", |ui| {
                Label::new("Space - Pause/Resume capture").color(dim).show(ui);
                Label::new("F12 - Toggle profiler visibility").color(dim).show(ui);
            });
            ui.add_space(6.0 * s);
            popup_heading(ui, s, "Flamegraph Controls");
            section(ui, s, "flame", |ui| {
                Label::new("Drag - Pan the view").color(dim).show(ui);
                Label::new("Ctrl+Scroll - Zoom at cursor").color(dim).show(ui);
                Label::new("Click scope - Zoom to fit").color(dim).show(ui);
                Label::new("Double-click - Reset view").color(dim).show(ui);
            });
            ui.add_space(6.0 * s);
            popup_heading(ui, s, "Frame History");
            section(ui, s, "hist", |ui| {
                Label::new("Click bar - Select frame and pause").color(dim).show(ui);
            });
            ui.add_space(6.0 * s);
            popup_heading(ui, s, "Table View");
            section(ui, s, "table", |ui| {
                Label::new("Click header - Sort by column").color(dim).show(ui);
                Label::new("Type in filter - Search scopes").color(dim).show(ui);
            });
            ui.add_space(8.0 * s);
            if Button::new("Close").show(ui).clicked {
                close = true;
            }
            let _ = style;
        });
    if close {
        state.show_info = false;
    }
}

fn tracy_popup(ui: &mut Ui, s: f32, state: &mut ProfilerState, anchor: Pos2) {
    if !state.show_tracy_popup {
        return;
    }
    let style = ui.style();
    let mut close = false;
    let width = 330.0 * s;
    let pos = Pos2::new(anchor.x, anchor.y);
    let connected = state.tracy_state.is_connected();
    let dim = gray(170);
    let show = &mut state.show_tracy_popup;
    Window::new("Tracy Profiler")
        .open(show)
        .default_pos(pos)
        .default_size(Vec2::new(width, 330.0 * s))
        .resizable(false)
        .collapsible(false)
        .show(ui, |ui| {
            popup_heading(ui, s, "Status");
            section(ui, s, "status", |ui| {
                if connected {
                    Label::new("\u{25CF} Tracy Active").color(rgb(80, 200, 80)).show(ui);
                } else {
                    Label::new("Tracy not compiled in").color(gray(140)).show(ui);
                    Label::new("Build with: cargo build --features tracy")
                        .size(style.fonts.small)
                        .color(gray(100))
                        .show(ui);
                }
            });
            ui.add_space(6.0 * s);
            popup_heading(ui, s, "About Tracy");
            section(ui, s, "about", |ui| {
                Label::new("- Nanosecond-precision timing").color(dim).show(ui);
                Label::new("- Memory allocation tracking").color(dim).show(ui);
                Label::new("- GPU profiling support").color(dim).show(ui);
                Label::new("- Lock contention analysis").color(dim).show(ui);
                ui.add_space(4.0 * s);
                Label::new("Puffin:").color(gray(180)).show(ui);
                Label::new("Quick in-app overview")
                    .size(style.fonts.small)
                    .color(gray(140))
                    .show(ui);
                Label::new("Tracy:").color(gray(180)).show(ui);
                Label::new("Deep investigation tool")
                    .size(style.fonts.small)
                    .color(gray(140))
                    .show(ui);
            });
            ui.add_space(8.0 * s);
            if Button::new("Close").show(ui).clicked {
                close = true;
            }
        });
    if close {
        state.show_tracy_popup = false;
    }
}
