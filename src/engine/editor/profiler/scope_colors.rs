//! Scope color mapping based on duration
//!
//! Maps scope durations to colors for visual representation in the profiler.

use egui::Color32;

use super::data::ProfilerSettings;

/// Get the color for a scope based on its duration
/// Uses the pink→salmon→orange→red→dark-red palette from Figma:
/// - #FFC4D3 - Fast (light pink)
/// - #FFA1DC - Normal (pink)
/// - #FF8C80 - Warning (salmon)
/// - #FF8252 - Slow (orange)
/// - #FF5252 - Critical (red)
/// - #700000 - Blocking/clogging engine (very dark red)
pub fn scope_color(duration_ms: f64, settings: &ProfilerSettings) -> Color32 {
    if duration_ms < settings.fast_threshold_ms as f64 {
        // Fast - light pink
        Color32::from_rgb(0xFF, 0xC4, 0xD3) // #FFC4D3
    } else if duration_ms < settings.warning_threshold_ms as f64 {
        // Normal to Warning - lerp light pink → salmon
        let t = ((duration_ms - settings.fast_threshold_ms as f64)
            / (settings.warning_threshold_ms - settings.fast_threshold_ms) as f64)
            .clamp(0.0, 1.0) as f32;
        lerp_color(
            Color32::from_rgb(0xFF, 0xA1, 0xDC), // #FFA1DC pink
            Color32::from_rgb(0xFF, 0x8C, 0x80), // #FF8C80 salmon
            t,
        )
    } else if duration_ms < settings.slow_threshold_ms as f64 {
        // Slow - salmon to orange
        let t = ((duration_ms - settings.warning_threshold_ms as f64)
            / (settings.slow_threshold_ms - settings.warning_threshold_ms) as f64)
            .clamp(0.0, 1.0) as f32;
        lerp_color(
            Color32::from_rgb(0xFF, 0x8C, 0x80), // #FF8C80 salmon
            Color32::from_rgb(0xFF, 0x82, 0x52), // #FF8252 orange
            t,
        )
    } else if duration_ms < settings.slow_threshold_ms as f64 * 2.0 {
        // Critical - orange to red
        let t = ((duration_ms - settings.slow_threshold_ms as f64)
            / settings.slow_threshold_ms as f64)
            .clamp(0.0, 1.0) as f32;
        lerp_color(
            Color32::from_rgb(0xFF, 0x82, 0x52), // #FF8252 orange
            Color32::from_rgb(0xFF, 0x52, 0x52), // #FF5252 red
            t,
        )
    } else {
        // Blocking/clogging - very dark red (engine is stalling)
        Color32::from_rgb(0x70, 0x00, 0x00) // #700000
    }
}

/// Get color for frame bar based on duration (deprecated, use frame_bar_color_fps)
#[allow(dead_code)]
pub fn frame_bar_color(duration_ms: f64, settings: &ProfilerSettings) -> Color32 {
    scope_color(duration_ms, settings)
}

/// Get color for frame bar based on FPS (derived from duration)
/// Uses solid colors (no gradient):
/// - Green: >60 FPS (<16.67ms) - good performance
/// - Yellow: 30-60 FPS (16.67-33.33ms) - acceptable
/// - Red: <30 FPS (>33.33ms) - poor performance
#[allow(dead_code)]
pub fn frame_bar_color_fps(duration_ms: f64) -> Color32 {
    let fps = 1000.0 / duration_ms;
    if fps > 60.0 {
        // Green - good performance (>60 FPS)
        Color32::from_rgb(80, 200, 80)
    } else if fps >= 30.0 {
        // Yellow - acceptable (30-60 FPS)
        Color32::from_rgb(220, 180, 60)
    } else {
        // Red - poor performance (<30 FPS)
        Color32::from_rgb(220, 80, 80)
    }
}

/// Get color for frame bar based on duration using settings thresholds
/// Uses solid colors (no gradient) - unified with scope colors:
/// - Green: fast (< fast_threshold_ms)
/// - Yellow: warning (< warning_threshold_ms)
/// - Orange: slow (< slow_threshold_ms)
/// - Red: critical (>= slow_threshold_ms)
#[allow(dead_code)]
pub fn frame_bar_color_settings(duration_ms: f64, settings: &ProfilerSettings) -> Color32 {
    if duration_ms < settings.fast_threshold_ms as f64 {
        // Green - excellent performance
        Color32::from_rgb(80, 200, 80)
    } else if duration_ms < settings.warning_threshold_ms as f64 {
        // Yellow - acceptable
        Color32::from_rgb(220, 180, 60)
    } else if duration_ms < settings.slow_threshold_ms as f64 {
        // Orange - slow
        Color32::from_rgb(220, 120, 60)
    } else {
        // Red - critical
        Color32::from_rgb(220, 80, 80)
    }
}

/// Lerp between two colors
fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    Color32::from_rgb(
        (a.r() as f32 + (b.r() as f32 - a.r() as f32) * t) as u8,
        (a.g() as f32 + (b.g() as f32 - a.g() as f32) * t) as u8,
        (a.b() as f32 + (b.b() as f32 - a.b() as f32) * t) as u8,
    )
}

/// Get a slightly darker version of a color (for hover effect)
pub fn darken(color: Color32, amount: f32) -> Color32 {
    let factor = 1.0 - amount.clamp(0.0, 1.0);
    Color32::from_rgb(
        (color.r() as f32 * factor) as u8,
        (color.g() as f32 * factor) as u8,
        (color.b() as f32 * factor) as u8,
    )
}

/// Get a slightly lighter version of a color (for selection effect)
pub fn lighten(color: Color32, amount: f32) -> Color32 {
    let factor = amount.clamp(0.0, 1.0);
    Color32::from_rgb(
        (color.r() as f32 + (255.0 - color.r() as f32) * factor) as u8,
        (color.g() as f32 + (255.0 - color.g() as f32) * factor) as u8,
        (color.b() as f32 + (255.0 - color.b() as f32) * factor) as u8,
    )
}

/// Dim a color toward gray (for non-matching filter)
pub fn dim_color(color: Color32, amount: f32) -> Color32 {
    let gray = 40.0; // Target dim gray
    let factor = amount.clamp(0.0, 1.0);
    Color32::from_rgb(
        (color.r() as f32 * (1.0 - factor) + gray * factor) as u8,
        (color.g() as f32 * (1.0 - factor) + gray * factor) as u8,
        (color.b() as f32 * (1.0 - factor) + gray * factor) as u8,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scope_colors() {
        let settings = ProfilerSettings::default();

        // Fast should be green-ish
        let fast = scope_color(0.5, &settings);
        assert!(fast.g() > fast.r()); // More green than red

        // Critical should be red-ish
        let critical = scope_color(20.0, &settings);
        assert!(critical.r() > critical.g()); // More red than green
    }
}
