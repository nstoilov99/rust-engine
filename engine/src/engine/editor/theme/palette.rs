//! Color palette tokens for the editor theme.
//!
//! All colors are designed for WCAG AA contrast compliance against
//! their expected background surfaces.

use crusty_gui::math::Color;

/// sRGB 8-bit RGB → linear crusty [`Color`]. Shorthand for the many
/// `Color::from_srgb_u8(r, g, b, 255)` palette literals below.
#[inline]
fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::from_srgb_u8(r, g, b, 255)
}

/// Core color palette for the dark editor theme.
#[derive(Clone, Debug)]
pub struct Palette {
    /// Primary brand/accent color (used sparingly for focus, selection highlights)
    pub primary: Color,
    /// Secondary accent color
    pub accent: Color,
    /// Surface colors from darkest (0) to lightest (4).
    /// surface[0] = main background, surface[1] = panel bg, surface[2] = elevated,
    /// surface[3] = hover, surface[4] = active/pressed.
    pub surface: [Color; 5],
    /// Background color for editable fields (TextEdit, DragValue, Slider track).
    /// Darker than panel fill to create clear visual separation.
    pub field_bg: Color,
    /// Component section header background — slightly raised from panel fill.
    pub header_bg: Color,
    /// Primary text color — used for body text, headings
    pub text_primary: Color,
    /// Secondary text color — used for labels, captions, less important info
    pub text_secondary: Color,
    /// Disabled text color
    pub text_disabled: Color,
    /// Semantic colors for status indicators
    pub semantic: SemanticColors,
    /// Focus ring color (keyboard navigation)
    pub focus_ring: Color,
    /// Default stroke/border color
    pub stroke: Color,
}

/// Semantic colors for status/state indicators.
#[derive(Clone, Debug)]
pub struct SemanticColors {
    pub error: Color,
    pub warning: Color,
    pub success: Color,
    pub info: Color,
    /// Mixed value across multi-selection
    pub mixed: Color,
    /// Prefab/asset override indicator
    pub overridden: Color,
}

impl Palette {
    /// Default dark theme palette, tuned for WCAG AA contrast.
    pub fn dark_default() -> Self {
        Self {
            // Rusty orange (crusty-gui's editor_dark accent) — warm selection
            // highlight reminiscent of Unreal's.
            primary: rgb(245, 143, 46),
            accent: rgb(245, 143, 46),

            // Surface stack: darkest to lightest
            // Darker overall — clear separation between each level.
            surface: [
                rgb(18, 18, 21),    // [0] main/window background (darkest)
                rgb(24, 24, 28),    // [1] panel background
                rgb(34, 35, 39),    // [2] elevated (cards, popups, menus)
                rgb(48, 49, 54),    // [3] hover
                rgb(60, 62, 68),    // [4] active/pressed
            ],

            // Editable field background — darker than panel fill (surface[1])
            // to create a visible "inset" like Godot/Unity input fields.
            field_bg: rgb(14, 14, 17),
            // Component section header background — slightly raised from panel fill.
            header_bg: rgb(30, 31, 35),

            // Text — AA contrast ratios against surface[0..2]:
            // text_primary (#E8EAED) on surface[0] (#19191C) = ~14.5:1
            // text_primary (#E8EAED) on surface[2] (#292A2E) = ~10.7:1
            // text_secondary (#AAB0B6) on surface[0] (#19191C) = ~7.5:1
            // text_disabled (#747A80) on surface[2] (#292A2E) = ~3.2:1 (meets 3:1 for UI)
            text_primary: rgb(232, 234, 237),
            text_secondary: rgb(170, 176, 182),
            text_disabled: rgb(116, 122, 128),

            semantic: SemanticColors {
                error: rgb(242, 83, 75),
                warning: rgb(251, 188, 4),
                success: rgb(52, 168, 83),
                info: rgb(66, 133, 244),
                mixed: rgb(171, 130, 255),
                overridden: rgb(255, 167, 38),
            },

            focus_ring: rgb(255, 178, 102),
            stroke: rgb(55, 57, 63),
        }
    }
}

/// Contrast ratio between two colors (WCAG relative luminance formula).
/// Returns a value >= 1.0; higher = more contrast.
pub fn contrast_ratio(fg: Color, bg: Color) -> f32 {
    let lum_fg = relative_luminance(fg);
    let lum_bg = relative_luminance(bg);
    let (lighter, darker) = if lum_fg > lum_bg {
        (lum_fg, lum_bg)
    } else {
        (lum_bg, lum_fg)
    };
    (lighter + 0.05) / (darker + 0.05)
}

/// Relative luminance from a linear-space [`Color`] (no additional sRGB
/// conversion — the color is already linear).
fn relative_luminance(c: Color) -> f32 {
    0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b
}

/// A failing WCAG contrast pair.
#[derive(Debug)]
pub struct ContrastIssue {
    pub fg_name: &'static str,
    pub bg_name: &'static str,
    pub ratio: f32,
    pub required: f32,
}

impl std::fmt::Display for ContrastIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} on {} = {:.1}:1 (need {:.1}:1)",
            self.fg_name, self.bg_name, self.ratio, self.required
        )
    }
}

impl Palette {
    /// Verify WCAG AA compliance for all text/background pairings.
    /// Returns an empty vec if everything passes.
    ///
    /// Rules:
    /// - Body text (text_primary, text_secondary): >= 4.5:1 against surfaces 0-2
    /// - UI text (text_disabled): >= 3.0:1 against surfaces 0-2
    pub fn verify_wcag_aa(&self) -> Vec<ContrastIssue> {
        let mut issues = Vec::new();

        let bg_surfaces: [(&str, Color); 4] = [
            ("surface[0]", self.surface[0]),
            ("surface[1]", self.surface[1]),
            ("surface[2]", self.surface[2]),
            ("field_bg", self.field_bg),
        ];

        // Body text pairs — need 4.5:1
        let body_text: [(&str, Color); 2] = [
            ("text_primary", self.text_primary),
            ("text_secondary", self.text_secondary),
        ];
        for (fg_name, fg) in &body_text {
            for (bg_name, bg) in &bg_surfaces {
                let ratio = contrast_ratio(*fg, *bg);
                if ratio < 4.5 {
                    issues.push(ContrastIssue {
                        fg_name,
                        bg_name,
                        ratio,
                        required: 4.5,
                    });
                }
            }
        }

        // UI / large text — need 3.0:1
        let ui_text: [(&str, Color); 1] = [("text_disabled", self.text_disabled)];
        for (fg_name, fg) in &ui_text {
            for (bg_name, bg) in &bg_surfaces {
                let ratio = contrast_ratio(*fg, *bg);
                if ratio < 3.0 {
                    issues.push(ContrastIssue {
                        fg_name,
                        bg_name,
                        ratio,
                        required: 3.0,
                    });
                }
            }
        }

        issues
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_default_passes_wcag_aa() {
        let palette = Palette::dark_default();
        let issues = palette.verify_wcag_aa();
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
    fn contrast_ratio_white_on_black() {
        let ratio = contrast_ratio(Color::WHITE, Color::BLACK);
        assert!((ratio - 21.0).abs() < 0.5, "expected ~21:1, got {ratio}");
    }
}
