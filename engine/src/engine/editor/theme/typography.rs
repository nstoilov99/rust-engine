//! Typography configuration for the editor theme.
//!
//! Defines the type scale (point sizes) consumed by the crusty seam
//! (`gui/crusty.rs`) via `Style::fonts`. The old font-plumbing helpers
//! (`text_styles`, `font_definitions`) were removed as part of the UI
//! teardown.

/// Typography configuration — font sizes for each semantic text role.
#[derive(Clone, Debug)]
pub struct Typography {
    pub heading_large: f32,
    pub heading: f32,
    pub body: f32,
    pub caption: f32,
    pub mono: f32,
}

impl Typography {
    /// Comfortable density baseline.
    pub fn comfortable() -> Self {
        Self {
            heading_large: 18.0,
            heading: 14.0,
            body: 12.0,
            caption: 10.5,
            mono: 12.0,
        }
    }

    /// Returns a scaled copy by the given factor.
    pub fn scaled(&self, factor: f32) -> Self {
        Self {
            heading_large: self.heading_large * factor,
            heading: self.heading * factor,
            body: self.body * factor,
            caption: self.caption * factor,
            mono: self.mono * factor,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comfortable_baseline_sizes() {
        let typo = Typography::comfortable();
        assert_eq!(typo.heading_large, 18.0);
        assert_eq!(typo.body, 12.0);
        assert_eq!(typo.caption, 10.5);
    }

    #[test]
    fn scaling_applies_factor() {
        let base = Typography::comfortable();
        let scaled = base.scaled(0.93);
        assert!((scaled.body - 12.0 * 0.93).abs() < 0.01);
    }
}
