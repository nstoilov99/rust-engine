//! Scope color mapping based on duration.
//!
//! Duration → colour for the flamegraph and frame bars. The arithmetic stays
//! in sRGB u8 component space (lerp / darken / lighten / dim on `[u8; 4]`) so
//! the user-approved bar colours don't shift: linear-space lerps between the
//! same sRGB endpoints would produce visibly different midpoint hues. Only
//! the *edges* of this module convert to the linear-space crusty [`Color`]
//! that the paint layer wants.

use crusty_gui::math::Color;

use super::data::ProfilerSettings;

/// Internal representation: sRGB u8 components. All arithmetic below is
/// component-wise on these tuples; conversion to [`Color`] happens only at
/// the `pub fn` boundary.
type Srgba = [u8; 4];

const fn srgb(r: u8, g: u8, b: u8) -> Srgba {
    [r, g, b, 255]
}

/// Convert an sRGB-component tuple to the linear-space runtime [`Color`].
#[inline]
fn to_color(c: Srgba) -> Color {
    Color::from_srgb_u8(c[0], c[1], c[2], c[3])
}

/// Get the color for a scope based on its duration
/// Uses the pink→salmon→orange→red→dark-red palette from Figma:
/// - #FFC4D3 - Fast (light pink)
/// - #FFA1DC - Normal (pink)
/// - #FF8C80 - Warning (salmon)
/// - #FF8252 - Slow (orange)
/// - #FF5252 - Critical (red)
/// - #700000 - Blocking/clogging engine (very dark red)
pub fn scope_color(duration_ms: f64, settings: &ProfilerSettings) -> Color {
    to_color(scope_color_srgb(duration_ms, settings))
}

fn scope_color_srgb(duration_ms: f64, settings: &ProfilerSettings) -> Srgba {
    if duration_ms < settings.fast_threshold_ms as f64 {
        // Fast - light pink
        srgb(0xFF, 0xC4, 0xD3)
    } else if duration_ms < settings.warning_threshold_ms as f64 {
        // Normal to Warning - lerp light pink → salmon
        let t = ((duration_ms - settings.fast_threshold_ms as f64)
            / (settings.warning_threshold_ms - settings.fast_threshold_ms) as f64)
            .clamp(0.0, 1.0) as f32;
        lerp_srgb(
            srgb(0xFF, 0xA1, 0xDC), // pink
            srgb(0xFF, 0x8C, 0x80), // salmon
            t,
        )
    } else if duration_ms < settings.slow_threshold_ms as f64 {
        // Slow - salmon to orange
        let t = ((duration_ms - settings.warning_threshold_ms as f64)
            / (settings.slow_threshold_ms - settings.warning_threshold_ms) as f64)
            .clamp(0.0, 1.0) as f32;
        lerp_srgb(
            srgb(0xFF, 0x8C, 0x80), // salmon
            srgb(0xFF, 0x82, 0x52), // orange
            t,
        )
    } else if duration_ms < settings.slow_threshold_ms as f64 * 2.0 {
        // Critical - orange to red
        let t = ((duration_ms - settings.slow_threshold_ms as f64)
            / settings.slow_threshold_ms as f64)
            .clamp(0.0, 1.0) as f32;
        lerp_srgb(
            srgb(0xFF, 0x82, 0x52), // orange
            srgb(0xFF, 0x52, 0x52), // red
            t,
        )
    } else {
        // Blocking/clogging - very dark red (engine is stalling)
        srgb(0x70, 0x00, 0x00)
    }
}

/// Get color for frame bar based on duration (deprecated, use frame_bar_color_fps)
#[allow(dead_code)]
pub fn frame_bar_color(duration_ms: f64, settings: &ProfilerSettings) -> Color {
    scope_color(duration_ms, settings)
}

/// Get color for frame bar based on FPS (derived from duration)
/// Uses solid colors (no gradient):
/// - Green: >60 FPS (<16.67ms) - good performance
/// - Yellow: 30-60 FPS (16.67-33.33ms) - acceptable
/// - Red: <30 FPS (>33.33ms) - poor performance
pub fn frame_bar_color_fps(duration_ms: f64) -> Color {
    let fps = 1000.0 / duration_ms;
    let c = if fps > 60.0 {
        srgb(80, 200, 80)
    } else if fps >= 30.0 {
        srgb(220, 180, 60)
    } else {
        srgb(220, 80, 80)
    };
    to_color(c)
}

/// Get color for frame bar based on duration using settings thresholds
#[allow(dead_code)]
pub fn frame_bar_color_settings(duration_ms: f64, settings: &ProfilerSettings) -> Color {
    let c = if duration_ms < settings.fast_threshold_ms as f64 {
        srgb(80, 200, 80)
    } else if duration_ms < settings.warning_threshold_ms as f64 {
        srgb(220, 180, 60)
    } else if duration_ms < settings.slow_threshold_ms as f64 {
        srgb(220, 120, 60)
    } else {
        srgb(220, 80, 80)
    };
    to_color(c)
}

/// Lerp two sRGB u8 tuples component-wise. Deliberately NOT a linear-space
/// lerp — user-approved bar colours require the historical sRGB behaviour.
fn lerp_srgb(a: Srgba, b: Srgba, t: f32) -> Srgba {
    let t = t.clamp(0.0, 1.0);
    [
        (a[0] as f32 + (b[0] as f32 - a[0] as f32) * t) as u8,
        (a[1] as f32 + (b[1] as f32 - a[1] as f32) * t) as u8,
        (a[2] as f32 + (b[2] as f32 - a[2] as f32) * t) as u8,
        255,
    ]
}

/// Slightly darker version of a colour (hover effect).
///
/// Takes a linear-space [`Color`], performs the darken multiplication in sRGB
/// u8 component space (`round(c * (1 - amount))`) to match the historical
/// user-approved appearance, then converts back.
pub fn darken(color: Color, amount: f32) -> Color {
    let [r, g, b, a] = color.to_srgb_u8();
    let factor = 1.0 - amount.clamp(0.0, 1.0);
    Color::from_srgb_u8(
        (r as f32 * factor) as u8,
        (g as f32 * factor) as u8,
        (b as f32 * factor) as u8,
        a,
    )
}

/// Slightly lighter version (selection effect); sRGB u8 arithmetic — see
/// [`darken`] for rationale.
pub fn lighten(color: Color, amount: f32) -> Color {
    let [r, g, b, a] = color.to_srgb_u8();
    let factor = amount.clamp(0.0, 1.0);
    Color::from_srgb_u8(
        (r as f32 + (255.0 - r as f32) * factor) as u8,
        (g as f32 + (255.0 - g as f32) * factor) as u8,
        (b as f32 + (255.0 - b as f32) * factor) as u8,
        a,
    )
}

/// Dim toward gray (for non-matching filter); sRGB u8 arithmetic — see
/// [`darken`] for rationale.
pub fn dim_color(color: Color, amount: f32) -> Color {
    let [r, g, b, a] = color.to_srgb_u8();
    let gray = 40.0; // Target dim gray
    let factor = amount.clamp(0.0, 1.0);
    Color::from_srgb_u8(
        (r as f32 * (1.0 - factor) + gray * factor) as u8,
        (g as f32 * (1.0 - factor) + gray * factor) as u8,
        (b as f32 * (1.0 - factor) + gray * factor) as u8,
        a,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scope_colors() {
        let settings = ProfilerSettings::default();

        // Fast should be pink-ish (r > g)
        let fast = scope_color_srgb(0.5, &settings);
        assert!(fast[0] > fast[1]); // More red than green

        // Critical should be very dark red
        let critical = scope_color_srgb(20.0, &settings);
        assert!(critical[0] > critical[1]);
    }
}
