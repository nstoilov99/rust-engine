use crate::engine::rendering::{RenderCounters, ResourceCounters};
use std::collections::HashMap;

/// Configuration for a benchmark run.
#[derive(Debug, Clone)]
pub struct BenchmarkConfig {
    pub warmup_frames: u32,
    pub sample_frames: u32,
    pub seed: u64,
    pub resolution: [u32; 2],
    pub entity_count: u32,
    pub uncapped: bool,
}

impl Default for BenchmarkConfig {
    fn default() -> Self {
        Self {
            warmup_frames: 100,
            sample_frames: 500,
            seed: 42,
            resolution: [1920, 1080],
            entity_count: 500,
            uncapped: false,
        }
    }
}

/// Hardware and build metadata captured at benchmark start.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BenchmarkMetadata {
    pub git_hash: String,
    pub build_profile: String,
    pub features: Vec<String>,
    pub cpu_name: String,
    pub gpu_name: String,
    pub present_mode: String,
    pub resolution: [u32; 2],
    pub seed: u64,
}

/// Snapshot of render counters averaged across sampled frames.
#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
pub struct RenderCounterSnapshot {
    pub draw_calls: u32,
    pub triangles: u32,
    pub material_changes: u32,
    pub visible_entities: u32,
}

impl RenderCounterSnapshot {
    pub fn from_average(counters: &RenderCounterTotals, sample_frames: u32) -> Self {
        if sample_frames == 0 {
            return Self::default();
        }

        let divisor = sample_frames as u64;
        Self {
            draw_calls: (counters.draw_calls / divisor) as u32,
            triangles: (counters.triangles / divisor) as u32,
            material_changes: (counters.material_changes / divisor) as u32,
            visible_entities: (counters.visible_entities / divisor) as u32,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct RenderCounterTotals {
    pub draw_calls: u64,
    pub triangles: u64,
    pub material_changes: u64,
    pub visible_entities: u64,
}

impl RenderCounterTotals {
    pub fn accumulate(&mut self, counters: &RenderCounters) {
        self.draw_calls += counters.draw_calls as u64;
        self.triangles += counters.triangles as u64;
        self.material_changes += counters.material_changes as u64;
        self.visible_entities += counters.visible_entities as u64;
    }
}

/// Complete benchmark results written to the report file.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BenchmarkResults {
    pub metadata: BenchmarkMetadata,
    pub frame_times_ms: Vec<f64>,
    pub avg_frame_ms: f64,
    pub min_frame_ms: f64,
    pub max_frame_ms: f64,
    pub p1_frame_ms: f64,
    pub p50_frame_ms: f64,
    pub p99_frame_ms: f64,
    pub stddev_frame_ms: f64,
    pub render_counters: RenderCounterSnapshot,
    pub resource_counters: ResourceCounters,
    pub category_averages: HashMap<String, f64>,
}

impl BenchmarkResults {
    pub fn compute(
        metadata: BenchmarkMetadata,
        frame_times_ms: Vec<f64>,
        render_counters: RenderCounterSnapshot,
        resource_counters: ResourceCounters,
        category_averages: HashMap<String, f64>,
    ) -> Self {
        if frame_times_ms.is_empty() {
            return Self {
                metadata,
                frame_times_ms,
                avg_frame_ms: 0.0,
                min_frame_ms: 0.0,
                max_frame_ms: 0.0,
                p1_frame_ms: 0.0,
                p50_frame_ms: 0.0,
                p99_frame_ms: 0.0,
                stddev_frame_ms: 0.0,
                render_counters,
                resource_counters,
                category_averages,
            };
        }

        let mut sorted = frame_times_ms.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        let len = sorted.len();
        let avg = frame_times_ms.iter().sum::<f64>() / frame_times_ms.len() as f64;
        let variance = frame_times_ms
            .iter()
            .map(|time| (time - avg).powi(2))
            .sum::<f64>()
            / frame_times_ms.len() as f64;

        Self {
            metadata,
            frame_times_ms,
            avg_frame_ms: avg,
            min_frame_ms: sorted[0],
            max_frame_ms: sorted[len - 1],
            p1_frame_ms: percentile(&sorted, 0.01),
            p50_frame_ms: percentile(&sorted, 0.50),
            p99_frame_ms: percentile(&sorted, 0.99),
            stddev_frame_ms: variance.sqrt(),
            render_counters,
            resource_counters,
            category_averages,
        }
    }
}

fn percentile(sorted: &[f64], percentile: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }

    let max_index = sorted.len().saturating_sub(1);
    let index = ((max_index as f64) * percentile).floor() as usize;
    sorted[index.min(max_index)]
}
