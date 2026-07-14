//! Profiler panel module
//!
//! Provides an in-engine profiler UI built on puffin for data collection.
//! The egui rendering fns (`show_contents`, plus the `flamegraph`, `toolbar`,
//! `frame_history`, `table_view` submodules) were removed as part of the egui
//! teardown; the crusty analog lives in `profiler_crusty`. The `budget` and
//! `scope_colors` submodules stay because the crusty views read them.

pub(crate) mod budget;
mod collector;
pub(crate) mod data;
pub(crate) mod scope_colors;
pub mod tracy;

use crate::engine::rendering::{RenderCounters, ResourceCounters};
pub use data::{
    ProfileFrame, ProfileScope, ProfileThread, ProfilerSettings, ProfilerState, ProfilerView,
};

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

    pub fn set_runtime_counters(
        &mut self,
        render_counters: RenderCounters,
        resource_counters: ResourceCounters,
    ) {
        self.state
            .set_runtime_counters(render_counters, resource_counters);
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
