//! Shadow tokens for the editor theme.
//!
//! Defines elevation levels for floating surfaces. Step 12 adds the
//! 9-slice shadow rendering; this module just holds the token values.

use egui::{Color32, Vec2};

/// Shadow specification for a given elevation level.
#[derive(Clone, Debug)]
pub struct ShadowSpec {
    /// Offset from the element origin
    pub offset: Vec2,
    /// Spread radius (conceptual — used by the 9-slice renderer in Step 12)
    pub spread: f32,
    /// Shadow color (typically semi-transparent black)
    pub color: Color32,
}

/// Shadow tokens for two elevation levels.
#[derive(Clone, Debug)]
pub struct ShadowTokens {
    /// Low elevation — cards, small popups
    pub low: ShadowSpec,
    /// High elevation — modals, large dialogs
    pub high: ShadowSpec,
}

impl ShadowTokens {
    /// Default shadow values for the dark theme.
    pub fn dark_default() -> Self {
        Self {
            low: ShadowSpec {
                offset: Vec2::new(0.0, 2.0),
                spread: 8.0,
                color: Color32::from_rgba_unmultiplied(0, 0, 0, 80),
            },
            high: ShadowSpec {
                offset: Vec2::new(0.0, 4.0),
                spread: 16.0,
                color: Color32::from_rgba_unmultiplied(0, 0, 0, 120),
            },
        }
    }
}
