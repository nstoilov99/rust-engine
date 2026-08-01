//! Density presets for the editor theme.
//!
//! DESIGN.md ▸ Metrics & density: density is not its own set of factors — it
//! is a named value of the one master knob, `Metrics::ui_scale`. Compact
//! 0.85 / Comfortable 1.0 / Spacious 1.15; OS display scaling multiplies on
//! top. Everything geometric (spacing, fonts, row heights, radii) derives
//! from it at draw time, so a preset can never scale fonts and spacing by
//! two different amounts again.

/// Density mode — a named `ui_scale`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Density {
    /// ui_scale 0.85 — tighter, for small screens / power users.
    Compact,
    /// ui_scale 1.00 — the design system baseline.
    #[default]
    Comfortable,
    /// ui_scale 1.15 — roomier, for large or high-DPI displays.
    Spacious,
}

impl Density {
    /// The single scale factor this preset stands for.
    pub fn ui_scale(self) -> f32 {
        match self {
            Density::Compact => 0.85,
            Density::Comfortable => 1.0,
            Density::Spacious => 1.15,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Density::Compact => "Compact",
            Density::Comfortable => "Comfortable",
            Density::Spacious => "Spacious",
        }
    }

    pub const ALL: [Density; 3] = [Density::Compact, Density::Comfortable, Density::Spacious];

    /// The preset whose `ui_scale` a raw slider value corresponds to, if any.
    pub fn from_ui_scale(scale: f32) -> Option<Density> {
        Self::ALL
            .into_iter()
            .find(|d| (d.ui_scale() - scale).abs() < 1e-4)
    }
}

/// Base spacing tokens (Comfortable). Scaled by `ui_scale` at the crusty
/// seam — see `theme::mod`'s scale-model note.
#[derive(Clone, Debug)]
pub struct SpacingTokens {
    /// Horizontal gap between adjacent widgets
    pub item_spacing_x: f32,
    /// Vertical gap between adjacent widgets
    pub item_spacing_y: f32,
    /// Padding inside buttons
    pub button_padding_x: f32,
    pub button_padding_y: f32,
    /// Minimum interactive size (height for buttons, etc.)
    pub interact_size_y: f32,
    /// Panel/window inner margin
    pub window_margin: f32,
    /// Indent per hierarchy depth level
    pub indent: f32,
}

impl SpacingTokens {
    /// Comfortable baseline values.
    pub fn comfortable() -> Self {
        Self {
            item_spacing_x: 8.0,
            item_spacing_y: 4.0,
            button_padding_x: 8.0,
            button_padding_y: 3.0,
            interact_size_y: 22.0,
            window_margin: 8.0,
            indent: 18.0,
        }
    }

    /// Returns a copy scaled by the given factor.
    pub fn scaled(&self, factor: f32) -> Self {
        Self {
            item_spacing_x: self.item_spacing_x * factor,
            item_spacing_y: self.item_spacing_y * factor,
            button_padding_x: self.button_padding_x * factor,
            button_padding_y: self.button_padding_y * factor,
            interact_size_y: self.interact_size_y * factor,
            window_margin: self.window_margin * factor,
            indent: self.indent * factor,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn density_presets_are_the_documented_scales() {
        assert_eq!(Density::Compact.ui_scale(), 0.85);
        assert_eq!(Density::Comfortable.ui_scale(), 1.0);
        assert_eq!(Density::Spacious.ui_scale(), 1.15);
        assert_eq!(Density::default(), Density::Comfortable);
    }

    #[test]
    fn round_trips_through_ui_scale() {
        for d in Density::ALL {
            assert_eq!(Density::from_ui_scale(d.ui_scale()), Some(d));
        }
        assert_eq!(Density::from_ui_scale(1.07), None);
    }

    #[test]
    fn spacing_scales_correctly() {
        let base = SpacingTokens::comfortable();
        let compact = base.scaled(Density::Compact.ui_scale());
        assert!((compact.item_spacing_x - 6.8).abs() < 0.01);
        assert!((compact.interact_size_y - 18.7).abs() < 0.01);
    }
}
