//! Density scaling for the editor theme.
//!
//! Two density modes: Compact (tighter for small screens / power users)
//! and Comfortable (default, more spacious).

/// Density mode controlling spacing and font scale.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Density {
    /// Tighter spacing: spacing × 0.75, font × 0.93
    Compact,
    /// Default spacing: spacing × 1.00, font × 1.00
    #[default]
    Comfortable,
}

impl Density {
    /// Spacing multiplier relative to the Comfortable baseline.
    pub fn spacing_factor(self) -> f32 {
        match self {
            Density::Compact => 0.75,
            Density::Comfortable => 1.0,
        }
    }

    /// Font size multiplier relative to the Comfortable baseline.
    pub fn font_factor(self) -> f32 {
        match self {
            Density::Compact => 0.93,
            Density::Comfortable => 1.0,
        }
    }
}

/// Spacing tokens scaled by density.
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
    fn compact_factors() {
        assert_eq!(Density::Compact.spacing_factor(), 0.75);
        assert_eq!(Density::Compact.font_factor(), 0.93);
    }

    #[test]
    fn comfortable_factors() {
        assert_eq!(Density::Comfortable.spacing_factor(), 1.0);
        assert_eq!(Density::Comfortable.font_factor(), 1.0);
    }

    #[test]
    fn spacing_scales_correctly() {
        let base = SpacingTokens::comfortable();
        let compact = base.scaled(Density::Compact.spacing_factor());
        assert!((compact.item_spacing_x - 6.0).abs() < 0.01);
        assert!((compact.interact_size_y - 16.5).abs() < 0.01);
    }
}
