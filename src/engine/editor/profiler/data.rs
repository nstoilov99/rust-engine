//! Profiler data model types
//!
//! Defines the structures for storing and representing profiling data
//! extracted from puffin.

use std::collections::{HashSet, VecDeque};
use std::sync::Arc;

use super::tracy::TracyState;
use crate::engine::rendering::{RenderCounters, ResourceCounters};

/// A single profiling scope within a frame
#[derive(Clone, Debug)]
pub struct ProfileScope {
    /// Unique identifier for this scope type
    pub id: String,
    /// Display name of the scope (Arc<str> for cheap cloning from cache)
    pub name: Arc<str>,
    /// Source location (file:line) (Arc<str> for cheap cloning from cache)
    pub location: Arc<str>,
    /// Start time in nanoseconds relative to frame start
    pub start_ns: i64,
    /// Duration in nanoseconds
    pub duration_ns: i64,
    /// Nesting depth (0 = root)
    pub depth: usize,
    /// Child scopes
    pub children: Vec<ProfileScope>,
}

impl ProfileScope {
    /// Get duration in milliseconds
    pub fn duration_ms(&self) -> f64 {
        self.duration_ns as f64 / 1_000_000.0
    }

    /// Get start time in milliseconds
    pub fn start_ms(&self) -> f64 {
        self.start_ns as f64 / 1_000_000.0
    }

    /// Get end time in milliseconds
    pub fn end_ms(&self) -> f64 {
        (self.start_ns + self.duration_ns) as f64 / 1_000_000.0
    }

    /// Count total scopes including children recursively
    pub fn total_scope_count(&self) -> usize {
        1 + self
            .children
            .iter()
            .map(|c| c.total_scope_count())
            .sum::<usize>()
    }

    /// Check if this scope matches a filter string (case-insensitive)
    pub fn matches_filter(&self, filter_lower: &str) -> bool {
        self.name.as_ref().to_lowercase().contains(filter_lower)
            || self.location.as_ref().to_lowercase().contains(filter_lower)
    }

    /// Check if this scope or any of its children match the filter
    pub fn matches_filter_recursive(&self, filter_lower: &str) -> bool {
        if self.matches_filter(filter_lower) {
            return true;
        }
        self.children
            .iter()
            .any(|c| c.matches_filter_recursive(filter_lower))
    }
}

/// Thread profiling data for a single frame
#[derive(Clone, Debug)]
pub struct ProfileThread {
    /// Thread name
    pub name: String,
    /// Root scopes (depth 0)
    pub scopes: Vec<ProfileScope>,
    /// Maximum nesting depth
    pub max_depth: usize,
}

impl ProfileThread {
    /// Count total scopes in this thread
    pub fn scope_count(&self) -> usize {
        self.scopes.iter().map(|s| s.total_scope_count()).sum()
    }

    /// Get the time range covered by all scopes
    pub fn time_range_ns(&self) -> Option<(i64, i64)> {
        if self.scopes.is_empty() {
            return None;
        }
        let start = self.scopes.iter().map(|s| s.start_ns).min().unwrap_or(0);
        let end = self
            .scopes
            .iter()
            .map(|s| s.start_ns + s.duration_ns)
            .max()
            .unwrap_or(0);
        Some((start, end))
    }
}

/// A complete frame's profiling data
#[derive(Clone, Debug)]
pub struct ProfileFrame {
    /// Frame number
    pub frame_number: u64,
    /// Frame duration in nanoseconds
    pub duration_ns: i64,
    /// Threads with profiling data
    pub threads: Vec<ProfileThread>,
    /// Total scope count across all threads
    pub total_scopes: usize,
    /// Approximate memory size of this frame's data
    pub data_size_bytes: usize,
}

impl ProfileFrame {
    /// Get frame duration in milliseconds
    pub fn duration_ms(&self) -> f64 {
        self.duration_ns as f64 / 1_000_000.0
    }

    /// Get thread count
    pub fn thread_count(&self) -> usize {
        self.threads.len()
    }
}

/// Aggregated scope statistics for table view
#[derive(Clone, Debug)]
pub struct ScopeStats {
    /// Scope identifier
    pub id: String,
    /// Display name (Arc<str> for cheap cloning from ProfileScope)
    pub name: Arc<str>,
    /// Source location (Arc<str> for cheap cloning from ProfileScope)
    pub location: Arc<str>,
    /// Number of calls
    pub call_count: usize,
    /// Total time across all calls (ns)
    pub total_time_ns: i64,
    /// Mean time per call (ns)
    pub mean_time_ns: i64,
    /// Maximum time for any single call (ns)
    pub max_time_ns: i64,
}

impl ScopeStats {
    pub fn total_time_ms(&self) -> f64 {
        self.total_time_ns as f64 / 1_000_000.0
    }

    pub fn mean_time_ms(&self) -> f64 {
        self.mean_time_ns as f64 / 1_000_000.0
    }

    pub fn max_time_ms(&self) -> f64 {
        self.max_time_ns as f64 / 1_000_000.0
    }
}

/// Which view is currently active
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ProfilerView {
    #[default]
    Flamegraph,
    Table,
    Budget,
}

/// Frame history display mode
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FrameHistoryMode {
    #[default]
    Live, // Chronological order, newest on right
    Slowest, // Sorted by duration, slowest first
}

/// Frame bar scale mode for frame history chart
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FrameBarScale {
    #[default]
    Fixed, // Fixed 50ms scale (good for comparing across time)
    Auto, // Auto-scale to max visible frame (good for seeing detail)
}

/// Profiler settings
#[derive(Clone, Debug)]
pub struct ProfilerSettings {
    /// Grid line spacing in milliseconds
    pub grid_spacing_ms: f32,
    /// Duration threshold for "fast" scopes (green)
    pub fast_threshold_ms: f32,
    /// Duration threshold for "warning" scopes (yellow)
    pub warning_threshold_ms: f32,
    /// Duration threshold for "slow" scopes (orange/red)
    pub slow_threshold_ms: f32,
    /// Number of sub-divisions between major grid lines (0 = no sub-grid)
    pub sub_grid_divisions: u8,
    /// Maximum frames to keep in history (default: 300)
    pub history_capacity: usize,
    /// Maximum bars to show in frame history chart (0 = auto based on width)
    pub max_display_bars: usize,
    /// Frame bar scale mode for frame history chart
    pub frame_bar_scale: FrameBarScale,
}

impl Default for ProfilerSettings {
    fn default() -> Self {
        Self {
            grid_spacing_ms: 1.0,
            fast_threshold_ms: 1.0,
            warning_threshold_ms: 5.0,
            slow_threshold_ms: 16.67, // 60 FPS budget
            sub_grid_divisions: 4,    // 4 sub-divisions by default
            history_capacity: 300,
            max_display_bars: 300, // Show up to 300 bars at a time
            frame_bar_scale: FrameBarScale::default(),
        }
    }
}

/// Breadcrumb item for navigation in the flamegraph
#[derive(Clone, Debug)]
pub struct BreadcrumbItem {
    /// Display name of the scope
    pub name: String,
    /// Start time in milliseconds
    pub start_ms: f64,
    /// Duration in milliseconds
    pub duration_ms: f64,
}

/// Aggregate statistics for frame timing (avg/min/max FPS and ms)
#[derive(Clone, Debug, Default)]
pub struct FrameStatistics {
    /// Average frame duration in milliseconds
    pub avg_duration_ms: f64,
    /// Average FPS (frames per second)
    pub avg_fps: f64,
    /// Minimum frame duration in milliseconds
    pub min_duration_ms: f64,
    /// Maximum frame duration in milliseconds
    pub max_duration_ms: f64,
    /// Number of frames in the sample
    pub frame_count: usize,
}

impl FrameStatistics {
    /// Calculate frame statistics from a collection of frames
    pub fn calculate(frames: &VecDeque<Arc<ProfileFrame>>) -> Self {
        if frames.is_empty() {
            return Self::default();
        }

        let mut sum = 0.0;
        let mut min = f64::MAX;
        let mut max = f64::MIN;

        for frame in frames.iter() {
            let ms = frame.duration_ms();
            sum += ms;
            min = min.min(ms);
            max = max.max(ms);
        }

        let count = frames.len();
        let avg = sum / count as f64;

        Self {
            avg_duration_ms: avg,
            avg_fps: 1000.0 / avg,
            min_duration_ms: min,
            max_duration_ms: max,
            frame_count: count,
        }
    }
}

/// Column for table sorting
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SortColumn {
    Location,
    Name,
    #[default]
    TotalTime,
    CallCount,
    MeanTime,
    MaxTime,
}

/// Sort direction
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SortDirection {
    #[default]
    Descending,
    Ascending,
}

/// Main profiler state
#[derive(Debug)]
pub struct ProfilerState {
    /// Whether profiling capture is paused
    pub is_paused: bool,
    /// Ring buffer of recent frames
    pub frame_history: VecDeque<Arc<ProfileFrame>>,
    /// Currently selected frame index (into frame_history)
    pub selected_frame_index: Option<usize>,
    /// Current view mode
    pub current_view: ProfilerView,
    /// Scope filter text
    pub filter_text: String,
    /// Set of collapsed thread names
    pub collapsed_threads: HashSet<String>,
    /// Profiler settings
    pub settings: ProfilerSettings,
    /// Flamegraph pan offset (in ms)
    pub flamegraph_pan_ms: f64,
    /// Flamegraph zoom level (pixels per ms)
    pub flamegraph_zoom: f64,
    /// Whether user has manually zoomed (disables auto-fit)
    pub user_zoomed: bool,
    /// Vertical scroll offset for flamegraph content
    pub scroll_offset_y: f32,
    /// Table sort column
    pub sort_column: SortColumn,
    /// Table sort direction
    pub sort_direction: SortDirection,
    /// Show settings popup
    pub show_settings: bool,
    /// Show info/help popup
    pub show_info: bool,
    /// Breadcrumb navigation path for zoom history
    pub breadcrumb_path: Vec<BreadcrumbItem>,
    /// Cached scope stats for table view (avoids recalculating every frame)
    cached_stats: Option<Vec<ScopeStats>>,
    /// Frame number when stats were last calculated
    stats_frame_number: Option<u64>,
    /// Filter text when stats were last calculated
    stats_filter_text: String,
    /// Sort column when stats were last calculated
    stats_sort_column: SortColumn,
    /// Sort direction when stats were last calculated
    stats_sort_direction: SortDirection,
    /// Cached frame statistics (avg/min/max)
    cached_frame_stats: Option<FrameStatistics>,
    /// Frame number of newest frame when stats were last calculated
    frame_stats_last_frame_number: Option<u64>,
    /// Scroll offset for frame history (0 = show newest frames)
    pub frame_history_scroll: usize,
    /// Frame history display mode (Live = chronological, Slowest = sorted by duration)
    pub frame_history_mode: FrameHistoryMode,
    /// Tracy profiler state
    pub tracy_state: TracyState,
    /// Show Tracy popup window
    pub show_tracy_popup: bool,
    /// Latest frame-level render counters supplied by the runtime.
    pub latest_render_counters: RenderCounters,
    /// Latest scene-level resource counters supplied by the runtime.
    pub latest_resource_counters: ResourceCounters,
}

impl Default for ProfilerState {
    fn default() -> Self {
        let settings = ProfilerSettings::default();
        Self {
            is_paused: false,
            frame_history: VecDeque::with_capacity(settings.history_capacity),
            selected_frame_index: None,
            current_view: ProfilerView::default(),
            filter_text: String::new(),
            collapsed_threads: HashSet::new(),
            settings,
            flamegraph_pan_ms: 0.0,
            flamegraph_zoom: 1.0,
            user_zoomed: false,
            scroll_offset_y: 0.0,
            sort_column: SortColumn::default(),
            sort_direction: SortDirection::default(),
            show_settings: false,
            show_info: false,
            breadcrumb_path: Vec::new(),
            cached_stats: None,
            stats_frame_number: None,
            stats_filter_text: String::new(),
            stats_sort_column: SortColumn::default(),
            stats_sort_direction: SortDirection::default(),
            cached_frame_stats: None,
            frame_stats_last_frame_number: None,
            frame_history_scroll: 0,
            frame_history_mode: FrameHistoryMode::default(),
            tracy_state: TracyState::new(),
            show_tracy_popup: false,
            latest_render_counters: RenderCounters::default(),
            latest_resource_counters: ResourceCounters::default(),
        }
    }
}

impl ProfilerState {
    /// Add a new frame to the history
    pub fn push_frame(&mut self, frame: Arc<ProfileFrame>) {
        if self.is_paused {
            return;
        }

        self.frame_history.push_back(frame);

        // Trim to capacity
        while self.frame_history.len() > self.settings.history_capacity {
            self.frame_history.pop_front();
        }

        // Auto-select latest frame when not paused
        if self.selected_frame_index.is_none() {
            self.selected_frame_index = Some(self.frame_history.len().saturating_sub(1));
        } else if let Some(idx) = self.selected_frame_index {
            // Keep selection at the latest frame when running
            if idx == self.frame_history.len().saturating_sub(2) {
                self.selected_frame_index = Some(self.frame_history.len().saturating_sub(1));
            }
        }
    }

    /// Get the currently selected frame
    pub fn selected_frame(&self) -> Option<&Arc<ProfileFrame>> {
        self.selected_frame_index
            .and_then(|idx| self.frame_history.get(idx))
    }

    /// Clear all frame history
    pub fn clear(&mut self) {
        self.frame_history.clear();
        self.selected_frame_index = None;
    }

    /// Resize the history capacity
    /// Trims oldest frames if new capacity is smaller than current history size
    pub fn resize_history(&mut self, new_capacity: usize) {
        self.settings.history_capacity = new_capacity;
        // Trim if current history exceeds new capacity
        while self.frame_history.len() > new_capacity {
            self.frame_history.pop_front();
        }
        // Invalidate stats cache since history changed
        self.cached_frame_stats = None;
        // Adjust selected frame index if it's now out of bounds
        if let Some(idx) = self.selected_frame_index {
            if idx >= self.frame_history.len() {
                self.selected_frame_index = self.frame_history.len().checked_sub(1);
            }
        }
    }

    /// Calculate aggregate frame statistics (avg/min/max ms and FPS)
    /// Uses caching to avoid recalculating on every frame
    pub fn frame_statistics(&mut self) -> FrameStatistics {
        // Get the newest frame number to detect when new frames are added
        let newest_frame_number = self.frame_history.back().map(|f| f.frame_number);

        // Recalculate if no cache OR if newest frame changed (even if len is same)
        if self.cached_frame_stats.is_none()
            || self.frame_stats_last_frame_number != newest_frame_number
        {
            self.cached_frame_stats = Some(FrameStatistics::calculate(&self.frame_history));
            self.frame_stats_last_frame_number = newest_frame_number;
        }
        self.cached_frame_stats.clone().unwrap_or_default()
    }

    /// Toggle pause state
    pub fn toggle_pause(&mut self) {
        self.is_paused = !self.is_paused;
    }

    /// Reset flamegraph view to fit all content
    pub fn reset_flamegraph_view(&mut self) {
        self.flamegraph_pan_ms = 0.0;
        self.flamegraph_zoom = 1.0;
        self.user_zoomed = false;
        self.scroll_offset_y = 0.0;
        self.breadcrumb_path.clear();
    }

    /// Push a scope to the breadcrumb path when zooming
    pub fn push_breadcrumb(&mut self, name: String, start_ms: f64, duration_ms: f64) {
        self.breadcrumb_path.push(BreadcrumbItem {
            name,
            start_ms,
            duration_ms,
        });
    }

    /// Navigate to a breadcrumb by index, removing deeper items
    pub fn navigate_to_breadcrumb(&mut self, index: usize, content_width: f32) {
        if index < self.breadcrumb_path.len() {
            let item = self.breadcrumb_path[index].clone();
            let target_ratio = 0.6;
            let new_visible_range = item.duration_ms / target_ratio;
            // Max zoom allows viewing sub-microsecond scopes (10M px/ms)
            self.flamegraph_zoom =
                (content_width as f64 / new_visible_range).clamp(1.0, 10_000_000.0);
            self.flamegraph_pan_ms =
                item.start_ms + item.duration_ms / 2.0 - new_visible_range / 2.0;
            self.user_zoomed = true;
            // Truncate to keep only items up to and including this one
            self.breadcrumb_path.truncate(index + 1);
        }
    }

    /// Calculate scope statistics for the selected frame
    pub fn calculate_scope_stats(&self) -> Vec<ScopeStats> {
        let Some(frame) = self.selected_frame() else {
            return Vec::new();
        };

        use std::collections::HashMap;
        let mut stats_map: HashMap<String, ScopeStats> = HashMap::new();

        fn collect_stats(scope: &ProfileScope, stats_map: &mut HashMap<String, ScopeStats>) {
            let key = format!("{}:{}", scope.location, scope.name);
            let entry = stats_map.entry(key).or_insert_with(|| ScopeStats {
                id: scope.id.clone(),
                name: scope.name.clone(),
                location: scope.location.clone(),
                call_count: 0,
                total_time_ns: 0,
                mean_time_ns: 0,
                max_time_ns: 0,
            });

            entry.call_count += 1;
            entry.total_time_ns += scope.duration_ns;
            entry.max_time_ns = entry.max_time_ns.max(scope.duration_ns);

            for child in &scope.children {
                collect_stats(child, stats_map);
            }
        }

        for thread in &frame.threads {
            for scope in &thread.scopes {
                collect_stats(scope, &mut stats_map);
            }
        }

        // Calculate means
        let mut stats: Vec<ScopeStats> = stats_map
            .into_values()
            .map(|mut s| {
                s.mean_time_ns = s.total_time_ns / s.call_count as i64;
                s
            })
            .collect();

        // Apply filter
        if !self.filter_text.is_empty() {
            let filter_lower = self.filter_text.to_lowercase();
            stats.retain(|s| {
                s.name.as_ref().to_lowercase().contains(&filter_lower)
                    || s.location.as_ref().to_lowercase().contains(&filter_lower)
            });
        }

        // Sort
        match self.sort_column {
            SortColumn::Location => stats.sort_by(|a, b| a.location.cmp(&b.location)),
            SortColumn::Name => stats.sort_by(|a, b| a.name.cmp(&b.name)),
            SortColumn::TotalTime => stats.sort_by(|a, b| a.total_time_ns.cmp(&b.total_time_ns)),
            SortColumn::CallCount => stats.sort_by(|a, b| a.call_count.cmp(&b.call_count)),
            SortColumn::MeanTime => stats.sort_by(|a, b| a.mean_time_ns.cmp(&b.mean_time_ns)),
            SortColumn::MaxTime => stats.sort_by(|a, b| a.max_time_ns.cmp(&b.max_time_ns)),
        }

        if self.sort_direction == SortDirection::Descending {
            stats.reverse();
        }

        stats
    }

    /// Get cached scope stats, recalculating only when needed
    /// This avoids the expensive HashMap + sort operations every frame
    /// Returns a slice reference to avoid cloning the entire Vec
    pub fn get_scope_stats(&mut self) -> &[ScopeStats] {
        // Check if we need to recalculate
        let needs_recalc = match self.selected_frame() {
            Some(frame) => {
                self.cached_stats.is_none()
                    || self.stats_frame_number != Some(frame.frame_number)
                    || self.stats_filter_text != self.filter_text
                    || self.stats_sort_column != self.sort_column
                    || self.stats_sort_direction != self.sort_direction
            }
            None => false, // No frame selected, return empty
        };

        if needs_recalc {
            let stats = self.calculate_scope_stats();
            if let Some(frame) = self.selected_frame() {
                self.stats_frame_number = Some(frame.frame_number);
            }
            self.stats_filter_text = self.filter_text.clone();
            self.stats_sort_column = self.sort_column;
            self.stats_sort_direction = self.sort_direction;
            self.cached_stats = Some(stats);
        }

        self.cached_stats.as_deref().unwrap_or(&[])
    }

    /// Invalidate the cached stats (call when sort/filter changes)
    pub fn invalidate_stats_cache(&mut self) {
        self.cached_stats = None;
    }

    pub fn set_runtime_counters(
        &mut self,
        render_counters: RenderCounters,
        resource_counters: ResourceCounters,
    ) {
        self.latest_render_counters = render_counters;
        self.latest_resource_counters = resource_counters;
    }
}
