//! Frame history bar chart widget
//!
//! Displays a horizontal bar chart of recent frame times.
//! Supports Live mode (chronological) and Slowest mode (sorted by duration).

use egui::{Color32, Pos2, Rect, Sense, StrokeKind, Ui, Vec2};
use std::sync::Arc;

use super::data::{FrameHistoryMode, ProfileFrame, ProfilerState};
use super::scope_colors::frame_bar_color_fps;

/// Height of the frame history panel
const FRAME_HISTORY_HEIGHT: f32 = 60.0;
/// Padding between bars
const BAR_PADDING: f32 = 1.0;
/// Minimum bar width
const MIN_BAR_WIDTH: f32 = 2.0;
/// Height of the bottom control bar (buttons + scrollbar)
const CONTROL_BAR_HEIGHT: f32 = 18.0;

/// Render the frame history bar chart
pub fn render(ui: &mut Ui, state: &mut ProfilerState) {
    crate::profile_scope!("profiler_frame_history");

    let available_width = ui.available_width();
    let frame_count = state.frame_history.len();

    if frame_count == 0 {
        ui.allocate_space(Vec2::new(
            available_width,
            FRAME_HISTORY_HEIGHT + CONTROL_BAR_HEIGHT,
        ));
        ui.centered_and_justified(|ui| {
            ui.label(egui::RichText::new("No frames captured").weak());
        });
        return;
    }

    // Calculate bar dimensions
    // User-specified limit (if > 0) or auto-fit to width
    let max_bars = if state.settings.max_display_bars > 0 {
        state.settings.max_display_bars.min(frame_count)
    } else {
        ((available_width / (MIN_BAR_WIDTH + BAR_PADDING)) as usize).min(frame_count)
    };
    let bar_width = (available_width - BAR_PADDING * max_bars as f32) / max_bars as f32;

    // In Slowest mode, we show max_bars sorted by duration
    // In Live mode, we might need scrollbar if frame_count > max_bars
    let is_slowest_mode = state.frame_history_mode == FrameHistoryMode::Slowest;
    let show_scrollbar = !is_slowest_mode && frame_count > max_bars;
    let scroll_range = frame_count.saturating_sub(max_bars);

    // Auto-scroll to newest when not paused (Live mode only)
    if !state.is_paused && !is_slowest_mode {
        state.frame_history_scroll = 0;
    }

    // Clamp scroll to valid range
    state.frame_history_scroll = state.frame_history_scroll.min(scroll_range);

    // Get frames to display based on mode
    let display_frames: Vec<(usize, Arc<ProfileFrame>)> = if is_slowest_mode {
        // Sort by duration (slowest first) - collect indices with frames
        let mut indexed: Vec<(usize, Arc<ProfileFrame>)> = state
            .frame_history
            .iter()
            .enumerate()
            .map(|(i, f)| (i, f.clone()))
            .collect();
        indexed.sort_by(|a, b| b.1.duration_ns.cmp(&a.1.duration_ns));
        indexed.into_iter().take(max_bars).collect()
    } else {
        // Chronological (Live mode) - calculate start index based on scroll
        let start_index = scroll_range.saturating_sub(state.frame_history_scroll);
        state
            .frame_history
            .iter()
            .enumerate()
            .skip(start_index)
            .take(max_bars)
            .map(|(i, f)| (i, f.clone()))
            .collect()
    };

    // Fixed scale - always show up to ~20 FPS (50ms) to keep bars consistent
    // This ensures the 60 FPS (16.67ms) and 30 FPS (33.33ms) lines are always visible
    const FIXED_MAX_DURATION_MS: f64 = 50.0;
    let max_duration_ms = FIXED_MAX_DURATION_MS;

    // Allocate space and get painter for frame bars
    let (response, painter) = ui.allocate_painter(
        Vec2::new(available_width, FRAME_HISTORY_HEIGHT),
        Sense::click(),
    );
    let rect = response.rect;

    // Draw background
    painter.rect_filled(rect, 0.0, Color32::from_gray(30));

    // Draw 60 FPS reference line (16.67ms)
    let fps_60_y = rect.bottom() - (16.67 / max_duration_ms * rect.height() as f64) as f32;
    if fps_60_y > rect.top() {
        let line_color = Color32::from_rgba_unmultiplied(100, 200, 100, 100);
        painter.line_segment(
            [
                Pos2::new(rect.left(), fps_60_y),
                Pos2::new(rect.right() - 20.0, fps_60_y),
            ],
            egui::Stroke::new(1.0, line_color),
        );
        // Draw "60" label on right side
        painter.text(
            Pos2::new(rect.right() - 2.0, fps_60_y),
            egui::Align2::RIGHT_CENTER,
            "60",
            egui::FontId::proportional(10.0),
            Color32::from_rgb(100, 200, 100),
        );
    }

    // Draw 30 FPS reference line (33.33ms)
    let fps_30_y = rect.bottom() - (33.33 / max_duration_ms * rect.height() as f64) as f32;
    if fps_30_y > rect.top() {
        let line_color = Color32::from_rgba_unmultiplied(220, 180, 60, 100);
        painter.line_segment(
            [
                Pos2::new(rect.left(), fps_30_y),
                Pos2::new(rect.right() - 20.0, fps_30_y),
            ],
            egui::Stroke::new(1.0, line_color),
        );
        // Draw "30" label on right side
        painter.text(
            Pos2::new(rect.right() - 2.0, fps_30_y),
            egui::Align2::RIGHT_CENTER,
            "30",
            egui::FontId::proportional(10.0),
            Color32::from_rgb(220, 180, 60),
        );
    }

    // Track which bar is hovered and clicked
    let mut hovered_index: Option<usize> = None;
    let mut clicked_index: Option<usize> = None;

    let pointer_pos = ui.ctx().pointer_hover_pos();

    // Draw bars
    for (bar_i, (original_index, frame)) in display_frames.iter().enumerate() {
        let x = rect.left() + (bar_i as f32) * (bar_width + BAR_PADDING);
        let duration_ms = frame.duration_ms();
        let height = ((duration_ms / max_duration_ms) * rect.height() as f64) as f32;
        let height = height.min(rect.height()).max(2.0);

        let bar_rect = Rect::from_min_size(
            Pos2::new(x, rect.bottom() - height),
            Vec2::new(bar_width, height),
        );

        // Check if hovered
        let is_hovered = pointer_pos.is_some_and(|pos| bar_rect.contains(pos));
        if is_hovered {
            hovered_index = Some(*original_index);
        }

        // Check if this is the selected frame
        let is_selected = state.selected_frame_index == Some(*original_index);

        // Get bar color based on FPS
        let mut color = frame_bar_color_fps(duration_ms);

        // Highlight if hovered or selected
        if is_selected {
            color = super::scope_colors::lighten(color, 0.3);
        } else if is_hovered {
            color = super::scope_colors::lighten(color, 0.15);
        }

        // Draw the bar
        painter.rect_filled(bar_rect, 1.0, color);

        // Draw selection border
        if is_selected {
            painter.rect_stroke(
                bar_rect.expand(1.0),
                1.0,
                egui::Stroke::new(2.0, Color32::WHITE),
                StrokeKind::Outside,
            );
        }
    }

    // Handle click on bars
    if response.clicked() {
        if let Some(idx) = hovered_index {
            clicked_index = Some(idx);
        }
    }

    // Handle frame selection
    if let Some(idx) = clicked_index {
        state.selected_frame_index = Some(idx);
        // Auto-pause when selecting a specific frame
        if !state.is_paused {
            state.is_paused = true;
        }
    }

    // Show tooltip for hovered frame
    if let Some(idx) = hovered_index {
        if let Some(frame) = state.frame_history.get(idx) {
            egui::containers::Tooltip::always_open(
                ui.ctx().clone(),
                ui.layer_id(),
                egui::Id::new("frame_tooltip"),
                egui::containers::PopupAnchor::Pointer,
            )
            .show(|ui| {
                ui.label(format!("Frame #{}", frame.frame_number));
                ui.label(format!(
                    "{:.2} ms ({:.0} FPS)",
                    frame.duration_ms(),
                    1000.0 / frame.duration_ms()
                ));
                ui.label(format!(
                    "{} threads, {} scopes",
                    frame.thread_count(),
                    frame.total_scopes
                ));
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new("Click to select and pause")
                        .weak()
                        .small(),
                );
            });
        }
    }

    // Handle mouse wheel scrolling over the frame bars (Live mode only)
    if response.hovered() && show_scrollbar {
        let scroll_delta = ui.input(|i| i.raw_scroll_delta.x + i.raw_scroll_delta.y);
        if scroll_delta != 0.0 {
            // Positive scroll = newer frames, negative = older frames
            let scroll_change = (scroll_delta / 30.0 * scroll_range as f32) as isize;
            let new_scroll = (state.frame_history_scroll as isize - scroll_change)
                .clamp(0, scroll_range as isize) as usize;
            state.frame_history_scroll = new_scroll;
        }
    }

    // Render bottom control bar (mode buttons + scrollbar/info)
    render_control_bar(
        ui,
        state,
        available_width,
        frame_count,
        max_bars,
        scroll_range,
        show_scrollbar,
    );
}

/// Render the bottom control bar with mode toggle and scrollbar
fn render_control_bar(
    ui: &mut Ui,
    state: &mut ProfilerState,
    available_width: f32,
    frame_count: usize,
    max_bars: usize,
    scroll_range: usize,
    show_scrollbar: bool,
) {
    let is_slowest_mode = state.frame_history_mode == FrameHistoryMode::Slowest;

    ui.horizontal(|ui| {
        ui.set_height(CONTROL_BAR_HEIGHT);
        ui.set_min_width(available_width);

        // Mode toggle buttons
        let live_selected = state.frame_history_mode == FrameHistoryMode::Live;

        // Live button
        let live_response = ui.add(egui::Button::new("Live").selected(live_selected));
        if live_response.clicked() && !live_selected {
            state.frame_history_mode = FrameHistoryMode::Live;
        }
        live_response.on_hover_text("Show frames chronologically (newest on right)");

        // Slowest button
        let slowest_response = ui.add(egui::Button::new("Slowest").selected(!live_selected));
        if slowest_response.clicked() && live_selected {
            state.frame_history_mode = FrameHistoryMode::Slowest;
            // Auto-pause when switching to Slowest mode
            if !state.is_paused {
                state.is_paused = true;
            }
        }
        slowest_response.on_hover_text("Show slowest frames first (sorted by duration)");

        ui.separator();

        // Scrollbar or info text depending on mode
        if is_slowest_mode {
            // In Slowest mode, show info about displayed frames
            let displayed = max_bars.min(frame_count);
            ui.label(
                egui::RichText::new(format!(
                    "Showing {} slowest of {} frames",
                    displayed, frame_count
                ))
                .color(Color32::from_gray(160))
                .small(),
            );
        } else if show_scrollbar {
            // In Live mode with overflow, show inline scrollbar
            render_inline_scrollbar(ui, state, scroll_range, frame_count, max_bars);
        } else {
            // In Live mode without overflow, show frame count
            ui.label(
                egui::RichText::new(format!("{} frames", frame_count))
                    .color(Color32::from_gray(160))
                    .small(),
            );
        }
    });
}

/// Render an inline scrollbar within the horizontal layout
fn render_inline_scrollbar(
    ui: &mut Ui,
    state: &mut ProfilerState,
    scroll_range: usize,
    frame_count: usize,
    max_bars: usize,
) {
    // Calculate scrollbar dimensions
    let scrollbar_width = (ui.available_width() - 100.0).max(100.0); // Leave space for range text

    let (scroll_response, scroll_painter) = ui.allocate_painter(
        Vec2::new(scrollbar_width, CONTROL_BAR_HEIGHT - 4.0),
        Sense::click_and_drag(),
    );
    let scroll_rect = scroll_response.rect;

    // Draw track background
    scroll_painter.rect_filled(scroll_rect, 3.0, Color32::from_gray(40));

    // Calculate thumb dimensions
    let thumb_ratio = max_bars as f32 / frame_count as f32;
    let thumb_width = (scrollbar_width * thumb_ratio).max(20.0);
    let scroll_area = scrollbar_width - thumb_width;

    // Position: scroll=0 means newest (thumb at right), scroll=max means oldest (thumb at left)
    let scroll_normalized = if scroll_range > 0 {
        state.frame_history_scroll as f32 / scroll_range as f32
    } else {
        0.0
    };
    let thumb_x = scroll_rect.right() - thumb_width - scroll_area * scroll_normalized;

    let thumb_rect = Rect::from_min_size(
        Pos2::new(thumb_x, scroll_rect.top() + 1.0),
        Vec2::new(thumb_width, scroll_rect.height() - 2.0),
    );

    // Thumb color based on interaction state
    let thumb_color = if scroll_response.dragged() {
        Color32::from_gray(180)
    } else if thumb_rect.contains(scroll_response.hover_pos().unwrap_or_default()) {
        Color32::from_gray(140)
    } else {
        Color32::from_gray(100)
    };
    scroll_painter.rect_filled(thumb_rect, 3.0, thumb_color);

    // Handle drag
    if scroll_response.dragged() {
        let delta = scroll_response.drag_delta().x;
        if scroll_area > 0.0 {
            // Dragging left (negative delta) = scroll to older frames (increase scroll)
            let scroll_delta = (-delta / scroll_area * scroll_range as f32) as isize;
            let new_scroll = (state.frame_history_scroll as isize + scroll_delta)
                .clamp(0, scroll_range as isize) as usize;
            state.frame_history_scroll = new_scroll;
        }
    }

    // Handle click on track (jump to position)
    if scroll_response.clicked() {
        if let Some(pos) = scroll_response.interact_pointer_pos() {
            if scroll_area > 0.0 {
                // Normalize: 0 = left (oldest), 1 = right (newest)
                let click_normalized =
                    (scroll_rect.right() - thumb_width / 2.0 - pos.x) / scroll_area;
                let click_normalized = click_normalized.clamp(0.0, 1.0);
                state.frame_history_scroll = (click_normalized * scroll_range as f32) as usize;
            }
        }
    }

    // Draw frame range indicator text after scrollbar
    let visible_start = scroll_range.saturating_sub(state.frame_history_scroll);
    let visible_end = visible_start + max_bars;
    ui.label(
        egui::RichText::new(format!(
            "{}-{} of {}",
            visible_start + 1,
            visible_end,
            frame_count
        ))
        .color(Color32::from_gray(160))
        .small(),
    );
}
