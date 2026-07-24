//! Editor theme system — design tokens, palette, typography, density, shadows.
//!
//! The canonical theme instance lives in `EditorServices`. `gui/crusty.rs`
//! reads the tokens directly to build a crusty-gui [`Style`] each frame.

pub mod density;
pub mod palette;
pub mod shadows;
pub mod typography;

pub use density::{Density, SpacingTokens};
pub use palette::{
    Accents, Axis, ContrastIssue, Palette, Selection, Status, Surfaces, TextColors, TypeColors,
};
pub use shadows::{ShadowSpec, ShadowTokens};
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
    pub typography: Typography,
    pub spacing: SpacingTokens,
    pub shadows: ShadowTokens,
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

    pub fn rusty() -> Self {
        Self::with_palette(Palette::rusty())
    }

    fn with_palette(palette: Palette) -> Self {
        Self {
            palette,
            typography: Typography::comfortable(),
            spacing: SpacingTokens::comfortable(),
            shadows: ShadowTokens::dark_default(),
            density: Density::Comfortable,
        }
    }

    /// Default theme (Steel). Kept under the old name for existing callers.
    pub fn dark_default() -> Self {
        Self::steel()
    }

    /// Returns a new theme with the specified density applied.
    /// Typography and spacing are scaled relative to the Comfortable baseline.
    pub fn with_density(&self, density: Density) -> Self {
        Self {
            palette: self.palette.clone(),
            typography: Typography::comfortable().scaled(density.font_factor()),
            spacing: SpacingTokens::comfortable().scaled(density.spacing_factor()),
            shadows: self.shadows.clone(),
            density,
        }
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
            issues
                .iter()
                .map(|i| format!("  {i}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }

    #[test]
    fn density_toggle_changes_spacing() {
        let base = EditorTheme::dark_default();
        let compact = base.with_density(Density::Compact);
        assert!(compact.spacing.item_spacing_x < base.spacing.item_spacing_x);
        assert!(compact.typography.body < base.typography.body);
    }

    #[test]
    fn state_color_returns_semantic() {
        let theme = EditorTheme::dark_default();
        assert_eq!(
            theme.state_color(StateKind::Error),
            theme.palette.status.error
        );
        assert_eq!(
            theme.state_color(StateKind::Success),
            theme.palette.status.success
        );
    }
}
