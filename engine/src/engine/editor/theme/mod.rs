//! Editor theme system — design tokens, typography, density.
//!
//! [`tokens`] is the single source of color and metrics (landed from
//! `docs/mockup/theme.rs`); everything else here is the engine-side wiring.
//! The canonical theme instance lives in `EditorServices`. `gui/crusty.rs`
//! reads the tokens directly to build a crusty-gui [`Style`] each frame.
//!
//! **Scale model.** `metrics.ui_scale` is the one master knob (DESIGN.md ▸
//! Metrics & density: "every metric is a base value × `ui_scale`, resolved at
//! draw time"). `typography` and `spacing` stay at their Comfortable *base*
//! values here and are multiplied at the seam (`style_from_theme`), so a
//! metric is never scaled twice. Density presets are just named `ui_scale`
//! values — Compact 0.85 / Comfortable 1.0 / Spacious 1.15.
//!
//! Flat-design rule (DESIGN.md core rule 1): there are no shadow tokens. The
//! old `ShadowTokens` had no consumers and was removed with the reconcile.

pub mod density;
pub mod tokens;
pub mod typography;

pub use density::{Density, SpacingTokens};
pub use tokens::{
    asset_color, asset_deep_color, category_color, category_tag_color, contrast_ratio,
    domain_ramp_index, grid_major, grid_minor, neutral, pin_color, ramp, wire_color, Accents, Axis,
    ContrastIssue, Hue, Metrics, Motion, Palette, Palettes, Selection, Status, Surfaces,
    TextColors, GRID_MAJOR_STEP, GRID_MINOR_MIN_ZOOM, GRID_MINOR_STEP, PALETTES,
};
pub use typography::Typography;

/// State color kinds for semantic UI indicators.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StateKind {
    Disabled,
    Mixed,
    Overridden,
    Error,
    Warning,
    Success,
    Info,
}

/// Central theme struct — holds all design tokens.
#[derive(Clone, Debug)]
pub struct EditorTheme {
    pub palette: Palette,
    pub motion: Motion,
    /// Base metrics + the `ui_scale` master knob.
    pub metrics: Metrics,
    /// Base type scale (unscaled — see the module note).
    pub typography: Typography,
    /// Base spacing (unscaled — see the module note).
    pub spacing: SpacingTokens,
    pub density: Density,
}

impl EditorTheme {
    /// Steel — the design system's default preset.
    pub fn steel() -> Self {
        Self::with_palette(Palette::steel())
    }

    pub fn tidepool() -> Self {
        Self::with_palette(Palette::tidepool())
    }

    pub fn graphite() -> Self {
        Self::with_palette(Palette::graphite())
    }

    /// Rusty — brand demo only (splash/about/marketing). Deliberately absent
    /// from the Editor Preferences picker; the constructor stays so brand
    /// surfaces can ask for it explicitly.
    pub fn rusty() -> Self {
        Self::with_palette(Palette::rusty())
    }

    fn with_palette(palette: Palette) -> Self {
        Self {
            palette,
            motion: Motion::tokens(),
            metrics: Metrics::tokens(),
            typography: Typography::comfortable(),
            spacing: SpacingTokens::comfortable(),
            density: Density::Comfortable,
        }
    }

    /// Default theme (Steel). Kept under the old name for existing callers.
    pub fn dark_default() -> Self {
        Self::steel()
    }

    /// Returns a copy at the given global UI scale. Clamped to the range the
    /// Editor Preferences slider offers.
    pub fn with_ui_scale(&self, ui_scale: f32) -> Self {
        let mut out = self.clone();
        out.metrics.ui_scale = ui_scale.clamp(UI_SCALE_MIN, UI_SCALE_MAX);
        out
    }

    /// Returns a copy at the density preset's `ui_scale`.
    pub fn with_density(&self, density: Density) -> Self {
        let mut out = self.with_ui_scale(density.ui_scale());
        out.density = density;
        out
    }

    /// Minimal fallback theme for pre-Step-1 compatibility.
    /// Used by `UiExt::theme()` when the real theme hasn't been installed yet.
    pub fn fallback() -> Self {
        Self::dark_default()
    }

    /// Verify WCAG AA compliance. Delegates to `Palette::verify_wcag_aa()`.
    pub fn verify_wcag_aa(&self) -> Vec<ContrastIssue> {
        self.palette.verify_wcag_aa()
    }

    /// Map a `StateKind` to its corresponding semantic color.
    pub fn state_color(&self, kind: StateKind) -> crusty_gui::math::Color {
        match kind {
            StateKind::Disabled => self.palette.text.disabled,
            StateKind::Mixed => self.palette.status.mixed,
            StateKind::Overridden => self.palette.status.overridden,
            StateKind::Error => self.palette.status.error,
            StateKind::Warning => self.palette.status.warning,
            StateKind::Success => self.palette.status.success,
            StateKind::Info => self.palette.status.info,
        }
    }
}

/// Bounds of the Editor Preferences ▸ Appearance "UI scale" slider.
pub const UI_SCALE_MIN: f32 = 0.75;
pub const UI_SCALE_MAX: f32 = 1.5;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_default_passes_wcag() {
        let theme = EditorTheme::dark_default();
        let issues = theme.verify_wcag_aa();
        assert!(
            issues.is_empty(),
            "WCAG AA failures:\n{}",
            issues.iter().map(|i| format!("  {i}")).collect::<Vec<_>>().join("\n")
        );
    }

    #[test]
    fn density_presets_map_onto_ui_scale() {
        let base = EditorTheme::dark_default();
        assert_eq!(base.metrics.ui_scale, 1.0);
        assert_eq!(base.with_density(Density::Compact).metrics.ui_scale, 0.85);
        assert_eq!(base.with_density(Density::Comfortable).metrics.ui_scale, 1.0);
        assert_eq!(base.with_density(Density::Spacious).metrics.ui_scale, 1.15);
        // Base tokens are untouched — scaling happens at the crusty seam.
        let compact = base.with_density(Density::Compact);
        assert_eq!(compact.spacing.item_spacing_x, base.spacing.item_spacing_x);
        assert_eq!(compact.typography.body, base.typography.body);
        assert_eq!(compact.metrics.scaled(compact.typography.body), 12.0 * 0.85);
    }

    #[test]
    fn ui_scale_is_clamped() {
        let base = EditorTheme::dark_default();
        assert_eq!(base.with_ui_scale(0.1).metrics.ui_scale, UI_SCALE_MIN);
        assert_eq!(base.with_ui_scale(9.0).metrics.ui_scale, UI_SCALE_MAX);
    }

    #[test]
    fn state_color_returns_semantic() {
        let theme = EditorTheme::dark_default();
        assert_eq!(theme.state_color(StateKind::Error), theme.palette.status.error);
        assert_eq!(theme.state_color(StateKind::Success), theme.palette.status.success);
    }
}
