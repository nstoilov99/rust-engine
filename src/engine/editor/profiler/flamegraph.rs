//! Flamegraph view rendering
//!
//! Displays profiling data as a flamegraph with thread rows and nested scopes.
//! Uses a single-painter architecture for simpler layout and reliable background fill.

use egui::{Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Ui, Vec2};

use super::data::{ProfileScope, ProfileThread, ProfilerState};
use super::scope_colors::{darken, dim_color, lighten, scope_color};

/// Height of the time axis at the top
const TIME_AXIS_HEIGHT: f32 = 24.0;
/// Height of the breadcrumb bar when visible
const BREADCRUMB_HEIGHT: f32 = 22.0;
/// Height of each thread header row
const THREAD_HEADER_HEIGHT: f32 = 22.0;
/// Height of each scope row
const ROW_HEIGHT: f32 = 20.0;
/// Vertical gap between scope rows
const SCOPE_VERTICAL_GAP: f32 = 2.0;
/// Minimum scope width to render
const MIN_SCOPE_WIDTH: f32 = 2.0;
/// Padding between thread sections
const THREAD_PADDING: f32 = 8.0;
/// Left margin for content
const LEFT_MARGIN: f32 = 8.0;
/// Maximum zoom level (pixels per millisecond) - allows viewing sub-microsecond scopes
const MAX_ZOOM: f64 = 10_000_000.0;

/// Info about a scope for zoom-to-scope and hover features
#[derive(Clone)]
struct ScopeInfo {
    name: String,
    start_ms: f64,
    duration_ms: f64,
}

/// Result from drawing scopes - tracks both hovered and clicked scope
struct DrawScopesResult<'a> {
    hovered: Option<&'a ProfileScope>,
    hovered_rect: Option<Rect>,
    clicked: Option<ScopeInfo>,
}

/// Render the flamegraph view
pub fn render(ui: &mut Ui, state: &mut ProfilerState) {
    crate::profile_scope!("profiler_flamegraph");

    let Some(frame) = state.selected_frame().cloned() else {
        ui.centered_and_justified(|ui| {
            ui.label(RichText::new("No frame selected").weak());
        });
        return;
    };

    if frame.threads.is_empty() {
        ui.centered_and_justified(|ui| {
            ui.label(RichText::new("No profiling data in this frame").weak());
        });
        return;
    }

    // Calculate the time range across all threads
    let (min_time_ns, max_time_ns) = calculate_time_range(&frame.threads);
    let time_range_ns = (max_time_ns - min_time_ns).max(1) as f64;
    let time_range_ms = time_range_ns / 1_000_000.0;

    // Check if breadcrumbs should be shown
    let show_breadcrumbs = !state.breadcrumb_path.is_empty();

    // Render breadcrumb bar first (before allocating the painter rect)
    let breadcrumb_offset = if show_breadcrumbs {
        draw_breadcrumb_bar(ui, state);
        BREADCRUMB_HEIGHT
    } else {
        0.0
    };

    // Get remaining available rect after breadcrumbs
    let full_rect = ui.available_rect_before_wrap();
    let content_width = full_rect.width();

    // Calculate auto-fit zoom (pixels per ms) to fit frame in view
    let auto_zoom = content_width as f64 / time_range_ms;
    let effective_zoom = if state.user_zoomed {
        state.flamegraph_zoom
    } else {
        auto_zoom
    };

    // Create painter for entire area
    let painter = ui.painter_at(full_rect);

    // Fill entire background with dark color
    painter.rect_filled(full_rect, 0.0, Color32::from_gray(25));

    // Allocate the entire rect for interactions
    let response = ui.allocate_rect(full_rect, Sense::click_and_drag());

    // Define regions
    let time_axis_rect = Rect::from_min_size(
        full_rect.min,
        Vec2::new(full_rect.width(), TIME_AXIS_HEIGHT),
    );
    let content_rect = Rect::from_min_max(
        Pos2::new(full_rect.left(), full_rect.top() + TIME_AXIS_HEIGHT),
        full_rect.max,
    );

    // Store content_width for later use in breadcrumb navigation
    let _ = breadcrumb_offset; // Suppress unused warning

    // Calculate visible time range based on pan and zoom
    let visible_start_ms = state.flamegraph_pan_ms;
    let visible_range_ms = content_width as f64 / effective_zoom;

    // Draw time axis
    draw_time_axis(&painter, time_axis_rect, visible_start_ms, visible_range_ms);

    // Draw grid lines across the content area
    draw_grid_lines(
        &painter,
        content_rect,
        visible_start_ms,
        visible_range_ms,
        state.settings.sub_grid_divisions,
    );

    // Calculate total content height for scrolling
    let total_content_height = calculate_total_content_height(&frame.threads, state);
    let max_scroll = (total_content_height - content_rect.height()).max(0.0);
    state.scroll_offset_y = state.scroll_offset_y.clamp(0.0, max_scroll);

    // Draw threads
    let mut y = content_rect.top() - state.scroll_offset_y;
    let mut clicked_thread: Option<String> = None;
    let mut clicked_scope: Option<ScopeInfo> = None;
    let mut hovered_scope_rect: Option<Rect> = None;

    // Check for click position (only on actual click, not drag)
    let click_pos = if response.clicked() {
        response.interact_pointer_pos()
    } else {
        None
    };

    // Pre-compute lowercase filter once to avoid per-scope allocation
    let filter_lower: Option<String> = if state.filter_text.is_empty() {
        None
    } else {
        Some(state.filter_text.to_lowercase())
    };
    let filter_lower_ref = filter_lower.as_deref();

    for thread in &frame.threads {
        let is_collapsed = state.collapsed_threads.contains(&thread.name);

        // Draw thread header
        let header_rect = draw_thread_header(
            &painter,
            content_rect,
            Pos2::new(content_rect.left(), y),
            thread,
            is_collapsed,
        );

        // Check if header was clicked
        if let Some(pointer_pos) = click_pos {
            if header_rect.contains(pointer_pos) {
                clicked_thread = Some(thread.name.clone());
            }
        }

        y += THREAD_HEADER_HEIGHT;

        // Draw scopes if not collapsed
        if !is_collapsed {
            let thread_height = (thread.max_depth + 1) as f32 * ROW_HEIGHT;

            // Only draw if visible
            if y + thread_height > content_rect.top() && y < content_rect.bottom() {
                let result = draw_thread_scopes(
                    &painter,
                    ui,
                    content_rect,
                    thread,
                    y,
                    min_time_ns,
                    visible_start_ms,
                    effective_zoom,
                    state,
                    response.hover_pos(),
                    click_pos,
                    filter_lower_ref,
                );

                // Track clicked scope (first one wins)
                if clicked_scope.is_none() {
                    clicked_scope = result.clicked;
                }
                // Track hovered scope rect (last one wins - most specific)
                if result.hovered_rect.is_some() {
                    hovered_scope_rect = result.hovered_rect;
                }
            }

            y += thread_height + THREAD_PADDING;
        } else {
            y += 4.0; // Small gap when collapsed
        }
    }

    // Draw cursor line with time display
    if let Some(hover) = response.hover_pos() {
        // Only show cursor line when hovering within the content area
        if content_rect.contains(hover) || time_axis_rect.contains(hover) {
            let mut line_x = hover.x;
            let mut snapped_to_scope = false;

            // Check if Shift is held for scope edge snapping
            let shift_held = ui.input(|i| i.modifiers.shift);

            if shift_held {
                if let Some(scope_rect) = hovered_scope_rect {
                    // Snap to nearest edge of hovered scope
                    let dist_to_left = (hover.x - scope_rect.left()).abs();
                    let dist_to_right = (hover.x - scope_rect.right()).abs();

                    line_x = if dist_to_left < dist_to_right {
                        scope_rect.left()
                    } else {
                        scope_rect.right()
                    };
                    snapped_to_scope = true;
                }
            }

            // Convert screen X to time in milliseconds
            let time_ms = visible_start_ms + (line_x - content_rect.left()) as f64 / effective_zoom;

            // Choose line color based on snap state
            let line_color = if snapped_to_scope {
                Color32::from_rgb(100, 200, 255) // Cyan when snapped
            } else {
                Color32::from_rgba_unmultiplied(255, 255, 255, 180) // Semi-transparent white
            };

            // Draw vertical line across content area
            painter.line_segment(
                [
                    Pos2::new(line_x, time_axis_rect.bottom()),
                    Pos2::new(line_x, content_rect.bottom().min(full_rect.bottom())),
                ],
                egui::Stroke::new(1.0, line_color),
            );

            // Draw time label at top of line (in time axis area)
            let time_text = format!("{:.3}ms", time_ms);

            // Measure text size for background
            let text_galley = painter.layout_no_wrap(
                time_text.clone(),
                FontId::proportional(11.0),
                Color32::WHITE,
            );

            // Position text centered on line, but clamp to stay within bounds
            let text_width = text_galley.size().x;
            let text_x = line_x.clamp(
                content_rect.left() + text_width / 2.0 + 4.0,
                content_rect.right() - text_width / 2.0 - 4.0,
            );

            // Draw background rect for readability
            let bg_rect = Rect::from_min_size(
                Pos2::new(text_x - text_width / 2.0 - 3.0, time_axis_rect.top() + 3.0),
                Vec2::new(text_width + 6.0, text_galley.size().y + 2.0),
            );
            painter.rect_filled(bg_rect, 3.0, Color32::from_rgba_unmultiplied(30, 30, 30, 230));

            // Draw text centered
            painter.text(
                Pos2::new(text_x, time_axis_rect.top() + 4.0),
                Align2::CENTER_TOP,
                time_text,
                FontId::proportional(11.0),
                line_color,
            );
        }
    }

    // Handle click-to-zoom on scope (only if thread header wasn't clicked)
    if clicked_thread.is_none() {
        if let Some(scope_info) = clicked_scope {
            // Push to breadcrumb for navigation history
            state.push_breadcrumb(
                scope_info.name.clone(),
                scope_info.start_ms,
                scope_info.duration_ms,
            );

            // Zoom to fit the clicked scope (scope fills ~60% of view width)
            let target_ratio = 0.6;
            let new_visible_range = scope_info.duration_ms / target_ratio;
            let new_zoom = (content_width as f64 / new_visible_range).clamp(1.0, MAX_ZOOM);
            state.flamegraph_zoom = new_zoom;
            state.user_zoomed = true;

            // Center on the scope
            let scope_center = scope_info.start_ms + scope_info.duration_ms / 2.0;
            state.flamegraph_pan_ms = scope_center - new_visible_range / 2.0;
        }
    }

    // Handle thread collapse toggle
    if let Some(thread_name) = clicked_thread {
        if state.collapsed_threads.contains(&thread_name) {
            state.collapsed_threads.remove(&thread_name);
        } else {
            state.collapsed_threads.insert(thread_name);
        }
    }

    // Handle interactions
    handle_interactions(
        &response,
        ui,
        state,
        time_range_ms,
        content_width,
        content_rect.height(),
        max_scroll,
    );
}

/// Calculate the overall time range across all threads
fn calculate_time_range(threads: &[ProfileThread]) -> (i64, i64) {
    let mut min_time = i64::MAX;
    let mut max_time = i64::MIN;

    for thread in threads {
        if let Some((start, end)) = thread.time_range_ns() {
            min_time = min_time.min(start);
            max_time = max_time.max(end);
        }
    }

    if min_time == i64::MAX {
        (0, 1)
    } else {
        (min_time, max_time)
    }
}

/// Calculate total content height for all threads
fn calculate_total_content_height(threads: &[ProfileThread], state: &ProfilerState) -> f32 {
    let mut height = 0.0;
    for thread in threads {
        height += THREAD_HEADER_HEIGHT;
        if !state.collapsed_threads.contains(&thread.name) {
            height += (thread.max_depth + 1) as f32 * ROW_HEIGHT + THREAD_PADDING;
        } else {
            height += 4.0;
        }
    }
    height
}

/// Calculate nice grid spacing based on visible range
fn calculate_grid_spacing(visible_range_ms: f64) -> f64 {
    if visible_range_ms <= 0.0 {
        return 1.0;
    }

    // Target ~4-5 grid lines visible (reduced from 6 for less clutter)
    let target_lines = 4.0;
    let raw_spacing = visible_range_ms / target_lines;

    // Round to nice values: 0.1, 0.2, 0.5, 1, 2, 5, 10, 20, 50, etc.
    let magnitude = 10_f64.powf(raw_spacing.log10().floor());
    let normalized = raw_spacing / magnitude;

    let nice = if normalized <= 1.5 {
        1.0
    } else if normalized <= 3.5 {
        2.0
    } else if normalized <= 7.5 {
        5.0
    } else {
        10.0
    };

    (nice * magnitude).max(0.001)
}

/// Draw the time axis at the top
fn draw_time_axis(
    painter: &egui::Painter,
    rect: Rect,
    visible_start_ms: f64,
    visible_range_ms: f64,
) {
    // Background
    painter.rect_filled(rect, 0.0, Color32::from_gray(35));

    // Draw separator line at bottom of time axis
    painter.line_segment(
        [Pos2::new(rect.left(), rect.bottom()), Pos2::new(rect.right(), rect.bottom())],
        egui::Stroke::new(1.0, Color32::from_gray(50)),
    );

    let grid_spacing_ms = calculate_grid_spacing(visible_range_ms);

    // Draw "0ms" label at the start if it's visible
    if visible_start_ms <= 0.0 {
        let x = rect.left() + ((-visible_start_ms) / visible_range_ms * rect.width() as f64) as f32;
        if x >= rect.left() && x <= rect.right() - 30.0 {
            painter.line_segment(
                [Pos2::new(x, rect.bottom() - 6.0), Pos2::new(x, rect.bottom())],
                egui::Stroke::new(1.0, Color32::from_gray(100)),
            );
            painter.text(
                Pos2::new(x + 3.0, rect.center().y),
                Align2::LEFT_CENTER,
                "0ms",
                FontId::proportional(11.0),
                Color32::from_gray(180),
            );
        }
    }

    // Calculate first grid line position (skip 0 and negative values)
    let first_grid = (visible_start_ms / grid_spacing_ms).ceil() * grid_spacing_ms;
    // Ensure we start at a positive value, skip 0 (handled separately) and negatives
    let mut current_ms = first_grid.max(grid_spacing_ms);
    let end_ms = visible_start_ms + visible_range_ms;

    while current_ms <= end_ms {
        let x = rect.left()
            + ((current_ms - visible_start_ms) / visible_range_ms * rect.width() as f64) as f32;

        // Draw tick mark
        painter.line_segment(
            [Pos2::new(x, rect.bottom() - 6.0), Pos2::new(x, rect.bottom())],
            egui::Stroke::new(1.0, Color32::from_gray(100)),
        );

        // Draw label
        let label = format_time_label(current_ms, grid_spacing_ms);
        painter.text(
            Pos2::new(x + 3.0, rect.center().y),
            Align2::LEFT_CENTER,
            label,
            FontId::proportional(11.0),
            Color32::from_gray(180),
        );

        current_ms += grid_spacing_ms;
    }
}

/// Format time label based on grid spacing - smart precision
fn format_time_label(time_ms: f64, spacing_ms: f64) -> String {
    if time_ms >= 1000.0 {
        format!("{:.1}s", time_ms / 1000.0)
    } else if spacing_ms >= 1.0 {
        // Whole milliseconds: 0ms, 1ms, 2ms, 3ms
        format!("{:.0}ms", time_ms)
    } else if spacing_ms >= 0.1 {
        // One decimal: 1.1ms, 1.2ms, 1.3ms
        format!("{:.1}ms", time_ms)
    } else {
        // Two decimals when needed: 1.01ms, 1.02ms
        format!("{:.2}ms", time_ms)
    }
}

/// Draw grid lines in the content area
fn draw_grid_lines(
    painter: &egui::Painter,
    rect: Rect,
    visible_start_ms: f64,
    visible_range_ms: f64,
    sub_divisions: u8,
) {
    let grid_spacing_ms = calculate_grid_spacing(visible_range_ms);
    let end_ms = visible_start_ms + visible_range_ms;

    // Draw sub-grid lines first (behind major grid)
    if sub_divisions > 0 {
        let sub_spacing_ms = grid_spacing_ms / sub_divisions as f64;
        let first_sub = (visible_start_ms / sub_spacing_ms).ceil() * sub_spacing_ms;
        let mut current_ms = first_sub.max(0.0);

        while current_ms <= end_ms {
            // Skip if this is a major grid line (within small tolerance)
            let ratio = current_ms / grid_spacing_ms;
            let is_major = (ratio - ratio.round()).abs() < 0.001;

            if !is_major {
                let x = rect.left()
                    + ((current_ms - visible_start_ms) / visible_range_ms * rect.width() as f64)
                        as f32;

                painter.line_segment(
                    [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
                    egui::Stroke::new(1.0, Color32::from_gray(32)), // Lighter than major
                );
            }
            current_ms += sub_spacing_ms;
        }
    }

    // Draw major grid lines
    let first_grid = (visible_start_ms / grid_spacing_ms).ceil() * grid_spacing_ms;
    let mut current_ms = first_grid.max(0.0); // Start at 0 or first positive grid line

    while current_ms <= end_ms {
        let x = rect.left()
            + ((current_ms - visible_start_ms) / visible_range_ms * rect.width() as f64) as f32;

        painter.line_segment(
            [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
            egui::Stroke::new(1.0, Color32::from_gray(40)),
        );

        current_ms += grid_spacing_ms;
    }
}

/// Draw the breadcrumb navigation bar
fn draw_breadcrumb_bar(ui: &mut Ui, state: &mut ProfilerState) {
    let content_width = ui.available_width();

    ui.horizontal(|ui| {
        ui.set_height(BREADCRUMB_HEIGHT);

        // Style the bar with a subtle background
        let rect = ui.available_rect_before_wrap();
        ui.painter()
            .rect_filled(rect, 0.0, Color32::from_gray(35));

        ui.add_space(LEFT_MARGIN);

        // Frame/Home button to reset view
        if ui
            .add(
                egui::Button::new(RichText::new("Frame").color(Color32::from_gray(180)))
                    .fill(Color32::from_gray(45))
                    .corner_radius(2.0),
            )
            .on_hover_text("Reset to full frame view")
            .clicked()
        {
            state.reset_flamegraph_view();
        }

        // Breadcrumb items - iterate by index to avoid clone
        let breadcrumb_count = state.breadcrumb_path.len();
        let mut clicked_index: Option<usize> = None;

        for i in 0..breadcrumb_count {
            let item = &state.breadcrumb_path[i];
            ui.label(RichText::new(">").color(Color32::from_gray(100)));

            // Truncate long names
            let display_name = if item.name.len() > 20 {
                format!("{}...", &item.name[..17])
            } else {
                item.name.clone()
            };

            let hover_text = format!("{} ({:.2}ms)", item.name, item.duration_ms);

            if ui
                .add(
                    egui::Button::new(
                        RichText::new(&display_name).color(Color32::from_gray(200)),
                    )
                    .fill(Color32::from_gray(45))
                    .corner_radius(2.0),
                )
                .on_hover_text(hover_text)
                .clicked()
            {
                clicked_index = Some(i);
            }
        }

        // Handle click after iteration (avoids borrow conflict)
        if let Some(i) = clicked_index {
            state.navigate_to_breadcrumb(i, content_width);
        }
    });
}

/// Draw a thread header row
fn draw_thread_header(
    painter: &egui::Painter,
    clip_rect: Rect,
    pos: Pos2,
    thread: &ProfileThread,
    is_collapsed: bool,
) -> Rect {
    let header_rect = Rect::from_min_size(pos, Vec2::new(clip_rect.width(), THREAD_HEADER_HEIGHT));

    // Only draw if visible
    if header_rect.bottom() < clip_rect.top() || header_rect.top() > clip_rect.bottom() {
        return header_rect;
    }

    // Subtle background for header
    painter.rect_filled(header_rect, 0.0, Color32::from_gray(30));

    // Draw collapse arrow (using simple ASCII that renders reliably)
    let arrow = if is_collapsed { ">" } else { "v" };
    let text = format!("{} {} ({} scopes)", arrow, thread.name, thread.scope_count());

    painter.text(
        Pos2::new(header_rect.left() + LEFT_MARGIN, header_rect.center().y),
        Align2::LEFT_CENTER,
        text,
        FontId::proportional(12.0),
        Color32::from_gray(220),
    );

    header_rect
}

/// Draw all scopes for a thread
fn draw_thread_scopes<'a>(
    painter: &egui::Painter,
    ui: &Ui,
    clip_rect: Rect,
    thread: &'a ProfileThread,
    base_y: f32,
    frame_start_ns: i64,
    visible_start_ms: f64,
    zoom: f64, // pixels per ms
    state: &ProfilerState,
    hover_pos: Option<Pos2>,
    click_pos: Option<Pos2>,
    filter_lower: Option<&str>, // Pre-computed lowercase filter
) -> DrawScopesResult<'a> {
    let mut result = DrawScopesResult {
        hovered: None,
        hovered_rect: None,
        clicked: None,
    };

    for scope in &thread.scopes {
        let scope_result = draw_scope(
            painter,
            clip_rect,
            scope,
            base_y,
            frame_start_ns,
            visible_start_ms,
            zoom,
            state,
            hover_pos,
            click_pos,
            filter_lower,
        );

        if scope_result.hovered.is_some() {
            result.hovered = scope_result.hovered;
            result.hovered_rect = scope_result.hovered_rect;
        }
        if scope_result.clicked.is_some() && result.clicked.is_none() {
            result.clicked = scope_result.clicked;
        }
    }

    // Show tooltip for hovered scope
    if let Some(scope) = result.hovered {
        egui::show_tooltip_at_pointer(
            ui.ctx(),
            ui.layer_id(),
            egui::Id::new("scope_tooltip"),
            |ui| {
                ui.label(RichText::new(scope.name.as_ref()).strong());
                ui.label(RichText::new(scope.location.as_ref()).weak().small());
                ui.separator();
                ui.label(format!("Duration: {:.3} ms", scope.duration_ms()));
                if !scope.children.is_empty() {
                    ui.label(format!("Children: {}", scope.children.len()));
                }
                ui.add_space(4.0);
                ui.label(RichText::new("Click to zoom | R: reset | +/-: zoom | Arrows: pan").weak().small());
            },
        );
    }

    result
}

/// Draw a single scope and its children
fn draw_scope<'a>(
    painter: &egui::Painter,
    clip_rect: Rect,
    scope: &'a ProfileScope,
    base_y: f32,
    frame_start_ns: i64,
    visible_start_ms: f64,
    zoom: f64,
    state: &ProfilerState,
    hover_pos: Option<Pos2>,
    click_pos: Option<Pos2>,
    filter_lower: Option<&str>, // Pre-computed lowercase filter to avoid per-scope allocation
) -> DrawScopesResult<'a> {
    let start_ms = (scope.start_ns - frame_start_ns) as f64 / 1_000_000.0;
    let duration_ms = scope.duration_ms();

    // Calculate position
    let x = clip_rect.left() + ((start_ms - visible_start_ms) * zoom) as f32;
    let width = (duration_ms * zoom) as f32;
    let y = base_y + scope.depth as f32 * ROW_HEIGHT;

    // Helper to collect results from children
    let collect_children_results = |painter: &egui::Painter| {
        let mut result = DrawScopesResult {
            hovered: None,
            hovered_rect: None,
            clicked: None,
        };
        for child in &scope.children {
            let child_result = draw_scope(
                painter,
                clip_rect,
                child,
                base_y,
                frame_start_ns,
                visible_start_ms,
                zoom,
                state,
                hover_pos,
                click_pos,
                filter_lower,
            );
            if child_result.hovered.is_some() {
                result.hovered = child_result.hovered;
                result.hovered_rect = child_result.hovered_rect;
            }
            if child_result.clicked.is_some() && result.clicked.is_none() {
                result.clicked = child_result.clicked;
            }
        }
        result
    };

    // Skip if off-screen or too small
    if x + width < clip_rect.left() || x > clip_rect.right() {
        return collect_children_results(painter);
    }

    // Skip if too small to be meaningful
    if width < MIN_SCOPE_WIDTH {
        return collect_children_results(painter);
    }

    // Skip if outside vertical clip rect
    if y + ROW_HEIGHT < clip_rect.top() || y > clip_rect.bottom() {
        return collect_children_results(painter);
    }

    let scope_rect = Rect::from_min_size(
        Pos2::new(x, y),
        Vec2::new(width, ROW_HEIGHT - SCOPE_VERTICAL_GAP),
    );

    // Clip to visible area
    let visible_rect = scope_rect.intersect(clip_rect);
    if visible_rect.width() <= 0.0 || visible_rect.height() <= 0.0 {
        return collect_children_results(painter);
    }

    // Check if hovered
    let is_hovered = hover_pos.map_or(false, |pos| scope_rect.contains(pos));

    // Check if clicked
    let is_clicked = click_pos.map_or(false, |pos| scope_rect.contains(pos));

    // Get color based on duration
    let mut color = scope_color(duration_ms, &state.settings);

    // Apply filter dimming if filter is active and scope doesn't match
    if let Some(filter) = filter_lower {
        if !scope.matches_filter_recursive(filter) {
            color = dim_color(color, 0.7); // Dim by 70%
        }
    }

    if is_hovered {
        color = lighten(color, 0.2);
    }

    // Draw scope rectangle
    painter.rect_filled(visible_rect, 3.0, color);

    // Draw border
    painter.rect_stroke(
        visible_rect,
        3.0,
        egui::Stroke::new(1.0, darken(color, 0.3)),
        egui::StrokeKind::Inside,
    );

    // Draw text if wide enough
    if width > 40.0 {
        let name = scope.name.as_ref();
        let text: String = if width > 150.0 {
            format!("{} ({:.2}ms)", name, duration_ms)
        } else if width > 80.0 {
            name.to_string()
        } else {
            // Truncate
            let max_chars = ((width - 10.0) / 6.0) as usize;
            if name.len() > max_chars && max_chars > 3 {
                format!("{}...", &name[..max_chars.min(name.len()) - 3])
            } else {
                name.to_string()
            }
        };

        // Use dark text on light backgrounds (calculate perceived brightness)
        // Using relative luminance formula: 0.299*R + 0.587*G + 0.114*B
        let brightness = 0.299 * color.r() as f32 + 0.587 * color.g() as f32 + 0.114 * color.b() as f32;
        let text_color = if brightness > 140.0 {
            Color32::from_gray(30)
        } else {
            Color32::from_gray(250)
        };

        // Clip text to visible rect
        painter.with_clip_rect(visible_rect).text(
            Pos2::new(visible_rect.left() + 4.0, visible_rect.center().y),
            Align2::LEFT_CENTER,
            text,
            FontId::proportional(11.0),
            text_color,
        );
    }

    // Draw children and collect their results
    let mut child_result = DrawScopesResult {
        hovered: None,
        hovered_rect: None,
        clicked: None,
    };
    for child in &scope.children {
        let cr = draw_scope(
            painter,
            clip_rect,
            child,
            base_y,
            frame_start_ns,
            visible_start_ms,
            zoom,
            state,
            hover_pos,
            click_pos,
            filter_lower,
        );
        if cr.hovered.is_some() {
            child_result.hovered = cr.hovered;
            child_result.hovered_rect = cr.hovered_rect;
        }
        if cr.clicked.is_some() && child_result.clicked.is_none() {
            child_result.clicked = cr.clicked;
        }
    }

    // Build result - children take priority (more specific/nested scopes)
    DrawScopesResult {
        hovered: if child_result.hovered.is_some() {
            child_result.hovered
        } else if is_hovered {
            Some(scope)
        } else {
            None
        },
        hovered_rect: if child_result.hovered_rect.is_some() {
            child_result.hovered_rect
        } else if is_hovered {
            Some(scope_rect)
        } else {
            None
        },
        clicked: if child_result.clicked.is_some() {
            child_result.clicked
        } else if is_clicked {
            Some(ScopeInfo {
                name: scope.name.to_string(),
                start_ms,
                duration_ms,
            })
        } else {
            None
        },
    }
}

/// Handle pan/zoom/scroll interactions
fn handle_interactions(
    response: &egui::Response,
    ui: &Ui,
    state: &mut ProfilerState,
    time_range_ms: f64,
    content_width: f32,
    content_height: f32,
    max_scroll: f32,
) {
    // Pan with drag
    if response.dragged() {
        let delta = response.drag_delta();

        // Horizontal pan (time) - allow panning into negative region
        let visible_range_ms = if state.user_zoomed {
            content_width as f64 / state.flamegraph_zoom
        } else {
            time_range_ms
        };
        let ms_per_pixel = visible_range_ms / content_width as f64;
        state.flamegraph_pan_ms -= delta.x as f64 * ms_per_pixel;
        // No clamping - allow negative pan

        // Vertical scroll
        state.scroll_offset_y -= delta.y;
        state.scroll_offset_y = state.scroll_offset_y.clamp(0.0, max_scroll);
    }

    // Zoom with Ctrl + scroll
    if response.hovered() {
        let scroll_delta = ui.input(|i| i.raw_scroll_delta);

        if ui.input(|i| i.modifiers.ctrl) && scroll_delta.y != 0.0 {
            // Zoom
            let zoom_factor = if scroll_delta.y > 0.0 { 1.15 } else { 0.87 };

            // Get current effective zoom
            let current_zoom = if state.user_zoomed {
                state.flamegraph_zoom
            } else {
                content_width as f64 / time_range_ms
            };

            // Apply zoom (increased max from 10000 to 100000 for deep zoom into small scopes)
            let new_zoom = (current_zoom * zoom_factor).clamp(1.0, MAX_ZOOM);
            state.flamegraph_zoom = new_zoom;
            state.user_zoomed = true;

            // Symmetric zoom - expand/contract equally from center
            let old_visible_range = content_width as f64 / current_zoom;
            let new_visible_range = content_width as f64 / new_zoom;
            let delta_range = old_visible_range - new_visible_range;
            // Shift pan by half the delta to keep center fixed
            state.flamegraph_pan_ms += delta_range / 2.0;
            // No clamping - allow negative pan
        } else if scroll_delta.y != 0.0 {
            // Vertical scroll (no modifier)
            state.scroll_offset_y -= scroll_delta.y;
            state.scroll_offset_y = state.scroll_offset_y.clamp(0.0, max_scroll);
        }
    }

    // Double-click to reset view
    if response.double_clicked() {
        state.reset_flamegraph_view();
    }

    // Keyboard shortcuts (only when flamegraph is hovered/focused)
    if response.hovered() {
        // R - Reset view
        if ui.input(|i| i.key_pressed(egui::Key::R)) {
            state.reset_flamegraph_view();
        }

        // +/= - Zoom in
        if ui.input(|i| i.key_pressed(egui::Key::Equals) || i.key_pressed(egui::Key::Plus)) {
            let current_zoom = if state.user_zoomed {
                state.flamegraph_zoom
            } else {
                content_width as f64 / time_range_ms
            };
            let new_zoom = (current_zoom * 1.25).clamp(1.0, MAX_ZOOM);
            state.flamegraph_zoom = new_zoom;
            state.user_zoomed = true;
            // Symmetric zoom from center
            let old_visible_range = content_width as f64 / current_zoom;
            let new_visible_range = content_width as f64 / new_zoom;
            state.flamegraph_pan_ms += (old_visible_range - new_visible_range) / 2.0;
        }

        // - - Zoom out
        if ui.input(|i| i.key_pressed(egui::Key::Minus)) {
            let current_zoom = if state.user_zoomed {
                state.flamegraph_zoom
            } else {
                content_width as f64 / time_range_ms
            };
            let new_zoom = (current_zoom * 0.8).clamp(1.0, MAX_ZOOM);
            state.flamegraph_zoom = new_zoom;
            state.user_zoomed = true;
            // Symmetric zoom from center
            let old_visible_range = content_width as f64 / current_zoom;
            let new_visible_range = content_width as f64 / new_zoom;
            state.flamegraph_pan_ms += (old_visible_range - new_visible_range) / 2.0;
        }

        // Left/Right arrow keys - pan horizontally
        let visible_range_ms = if state.user_zoomed {
            content_width as f64 / state.flamegraph_zoom
        } else {
            time_range_ms
        };
        let pan_step = visible_range_ms * 0.1; // Pan 10% of visible range

        if ui.input(|i| i.key_pressed(egui::Key::ArrowLeft)) {
            state.flamegraph_pan_ms -= pan_step;
        }
        if ui.input(|i| i.key_pressed(egui::Key::ArrowRight)) {
            state.flamegraph_pan_ms += pan_step;
        }

        // Up/Down arrow keys - scroll vertically
        let scroll_step = 50.0;
        if ui.input(|i| i.key_pressed(egui::Key::ArrowUp)) {
            state.scroll_offset_y = (state.scroll_offset_y - scroll_step).max(0.0);
        }
        if ui.input(|i| i.key_pressed(egui::Key::ArrowDown)) {
            state.scroll_offset_y = (state.scroll_offset_y + scroll_step).min(max_scroll);
        }

        // Space to toggle pause
        if ui.input(|i| i.key_pressed(egui::Key::Space)) {
            state.toggle_pause();
        }
    }
}
