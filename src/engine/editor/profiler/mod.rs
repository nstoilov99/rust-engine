//! Profiler panel module
//!
//! Provides an in-engine profiler UI built on puffin for data collection
//! and egui for visualization.

mod collector;
mod data;
mod flamegraph;
mod frame_history;
mod scope_colors;
mod table_view;
mod toolbar;
pub mod tracy;

pub use data::{ProfileFrame, ProfileScope, ProfileThread, ProfilerSettings, ProfilerState, ProfilerView};

use egui::{RichText, Ui};
use std::sync::mpsc::Receiver;
use std::sync::Arc;

/// Main profiler panel
pub struct ProfilerPanel {
    /// Profiler state
    pub state: ProfilerState,
    /// Receiver for frame data from puffin
    frame_rx: Option<Receiver<Arc<ProfileFrame>>>,
    /// Puffin sink ID (for cleanup)
    sink_id: Option<puffin::FrameSinkId>,
}

impl Default for ProfilerPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl ProfilerPanel {
    /// Create a new profiler panel
    pub fn new() -> Self {
        Self {
            state: ProfilerState::default(),
            frame_rx: None,
            sink_id: None,
        }
    }

    /// Register the puffin frame sink
    /// Call this during app initialization
    pub fn register_sink(&mut self) {
        if self.sink_id.is_some() {
            // Already registered
            return;
        }

        let (rx, sink_id) = collector::create_profiler_channel();
        self.frame_rx = Some(rx);
        self.sink_id = Some(sink_id);
    }

    /// Poll for new frames from puffin
    /// Call this at the beginning of each UI frame
    pub fn update(&mut self) {
        let Some(ref rx) = self.frame_rx else {
            return;
        };

        // Drain all available frames
        while let Ok(frame) = rx.try_recv() {
            self.state.push_frame(frame);
        }
    }

    /// Render the profiler panel contents
    /// This is the main entry point called from EditorTabViewer
    pub fn show_contents(&mut self, ui: &mut Ui) {
        crate::profile_scope!("profiler_ui");

        // Poll for new frames
        self.update();

        // Check if profiling is enabled
        if !puffin::are_scopes_on() {
            ui.centered_and_justified(|ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(20.0);
                    ui.label(RichText::new("Profiling is disabled").size(16.0));
                    ui.add_space(10.0);
                    ui.label("Press F12 to enable profiling");
                    ui.add_space(10.0);
                    if ui.button("Enable Profiling").clicked() {
                        puffin::set_scopes_on(true);
                    }
                });
            });
            return;
        }

        // Render toolbar
        toolbar::render(ui, &mut self.state);
        ui.separator();

        // Render frame history bar chart
        frame_history::render(ui, &mut self.state);
        ui.separator();

        // View toggle
        ui.horizontal(|ui| {
            ui.label("View:");
            ui.selectable_value(
                &mut self.state.current_view,
                ProfilerView::Flamegraph,
                "Flamegraph",
            );
            ui.selectable_value(&mut self.state.current_view, ProfilerView::Table, "Table");

            // Filter (shared between views)
            ui.separator();
            ui.label("Filter:");
            ui.add(egui::TextEdit::singleline(&mut self.state.filter_text).desired_width(150.0));
            if ui.small_button("x").clicked() {
                self.state.filter_text.clear();
            }
        });

        ui.separator();

        // Render main view
        match self.state.current_view {
            ProfilerView::Flamegraph => flamegraph::render(ui, &mut self.state),
            ProfilerView::Table => table_view::render(ui, &mut self.state),
        }

        // Render popups
        toolbar::render_settings_popup(ui, &mut self.state);
        toolbar::render_info_popup(ui, &mut self.state);
        toolbar::render_tracy_popup(ui, &mut self.state);
    }
}

impl Drop for ProfilerPanel {
    fn drop(&mut self) {
        // Remove the frame sink when the panel is dropped
        if let Some(sink_id) = self.sink_id.take() {
            puffin::GlobalProfiler::lock().remove_sink(sink_id);
        }
    }
}
