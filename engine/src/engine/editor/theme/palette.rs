//! Color tokens for the editor theme — the Crusty Design System's semantic
//! palette. Presets swap SURFACES + ACCENTS only; selection, status, axis
//! and type colors are invariant across presets. Components must never
//! hard-code a color that exists here.

use crusty_gui::math::Color;

/// sRGB 8-bit RGB → linear crusty [`Color`].
#[inline]
fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::from_srgb_u8(r, g, b, 255)
}

/// Opaque surface stack. Depth = lighter fill + 1px border, never shadows.
#[derive(Clone, Debug)]
pub struct Surfaces {
    /// Editable fields (TextEdit, DragValue, spinbox) — darkest, reads inset.
    pub input: Color,
    /// Main window background, tab strips, status bar.
    pub window: Color,
    /// Panel bodies.
    pub panel: Color,
    /// Section headers, spinbox caps.
    pub header: Color,
    /// Popovers, dialogs, toasts, dropdown lists (E3/E4).
    pub elevated: Color,
    /// Pointer-over fill for rows, menu items, ghost buttons.
    pub hover: Color,
    /// Pressed/active neutral fill (also "on" filter chips).
    pub active: Color,
    /// 1px default border.
    pub stroke: Color,
    /// 1px border for hovered controls + popover/dialog outlines.
    pub stroke_strong: Color,
}

/// Accent tokens — split by JOB. One job per token, never shared.
#[derive(Clone, Debug)]
pub struct Accents {
    /// Active tab underline, toggled tool buttons, live profiler controls,
    /// checked checkbox/toggle, slider + progress fill.
    pub accent_active: Color,
    /// Focused inputs & keyboard nav. 1px border swap, no glow.
    pub focus_ring: Color,
    /// `accent_active` at 14-16% alpha: fill behind toggled tool buttons,
    /// active filter chips, segmented controls.
    pub accent_soft: Color,
    /// Text/icons on top of `accent_active` fills (primary buttons).
    pub on_accent: Color,
}

/// Preset-invariant selection tokens (UE-outliner-style calm neutral).
#[derive(Clone, Debug)]
pub struct Selection {
    /// Hierarchy rows, asset tiles, dropdown current value, palette row.
    /// Opaque desaturated blue-gray — identical in every preset.
    pub fill: Color, // #3C4654
    /// Text/icons on top of `fill`.
    pub text: Color, // #FFFFFF
}

/// Preset-invariant status + danger colors.
#[derive(Clone, Debug)]
pub struct Status {
    pub error: Color,   // #E25B54 — messages, chips, log tags
    pub warning: Color, // #E6C04F
    pub success: Color, // #5FC875
    pub info: Color,    // #6EA8E8
    /// Destructive ACTIONS (Remove/Delete buttons). Not for messages.
    pub danger: Color, // #B3382F
    pub mixed: Color,      // #AB82FF — mixed multi-selection values
    pub overridden: Color, // #FFA726 — prefab/asset override marker
}

/// Preset-invariant axis colors for transform fields + gizmos.
#[derive(Clone, Debug)]
pub struct Axis {
    pub x: Color, // #E05252
    pub y: Color, // #71C24E
    pub z: Color, // #4A9EE8
}

/// Preset-invariant per-asset-type colors (tile edges, typed icons).
/// Mirrors IconPalette's category colors.
#[derive(Clone, Debug)]
pub struct TypeColors {
    pub geometry: Color,  // #A8B0BA
    pub lights: Color,    // #FFC857
    pub cameras: Color,   // #5B9BD5
    pub vfx: Color,       // #E07856
    pub audio: Color,     // #62C370
    pub animation: Color, // #E66BB8
    pub materials: Color, // #A47AE8
    pub scripting: Color, // #4FC1B6
    pub physics: Color,   // #9DCC4D
    pub ui: Color,        // #F08C7E
}

#[derive(Clone, Debug)]
pub struct TextColors {
    pub primary: Color,   // #E8EAED
    pub secondary: Color, // #AAB0B6
    pub disabled: Color,  // #747A80
    pub mono: Color,      // #C6CBD3 — numeric values, paths, logs
}

/// Full color palette: preset surfaces/accents + the invariant groups.
#[derive(Clone, Debug)]
pub struct Palette {
    pub surfaces: Surfaces,
    pub accents: Accents,
    pub selection: Selection,
    pub status: Status,
    pub axis: Axis,
    pub type_colors: TypeColors,
    pub text: TextColors,
    /// Alpha for transient surfaces (menus, dropdowns, tooltips) only.
    /// 1.0 disables translucency globally. Simple alpha — no blur.
    pub popover_alpha: f32,
    /// Scrim behind modals.
    pub scrim_alpha: f32,
}

impl Palette {
    /// Steel — default preset. Cool neutral surfaces, steel-blue accent.
    pub fn steel() -> Self {
        Self {
            surfaces: Surfaces {
                input: rgb(14, 14, 17),
                window: rgb(18, 18, 21),
                panel: rgb(24, 24, 28),
                header: rgb(30, 31, 35),
                elevated: rgb(34, 35, 39),
                hover: rgb(48, 49, 54),
                active: rgb(60, 62, 68),
                stroke: rgb(55, 57, 63),
                stroke_strong: rgb(74, 77, 85),
            },
            accents: Accents {
                accent_active: rgb(79, 163, 232),
                focus_ring: rgb(122, 187, 240),
                accent_soft: rgb(79, 163, 232).with_alpha(0.15),
                on_accent: rgb(14, 21, 32),
            },
            ..Self::invariants()
        }
    }

    /// Tidepool — teal accent, surfaces pulled slightly green.
    pub fn tidepool() -> Self {
        Self {
            surfaces: Surfaces {
                input: rgb(13, 16, 16),
                window: rgb(17, 20, 20),
                panel: rgb(23, 27, 26),
                header: rgb(29, 34, 32),
                elevated: rgb(33, 39, 38),
                hover: rgb(45, 53, 51),
                active: rgb(57, 67, 65),
                stroke: rgb(54, 62, 60),
                stroke_strong: rgb(73, 84, 81),
            },
            accents: Accents {
                accent_active: rgb(63, 193, 176),
                focus_ring: rgb(106, 212, 197),
                accent_soft: rgb(63, 193, 176).with_alpha(0.15),
                on_accent: rgb(14, 21, 32),
            },
            ..Self::invariants()
        }
    }

    /// Graphite — near-neutral. Active state reads through brightness plus
    /// a thin bright border; the "accent" is a bright gray.
    pub fn graphite() -> Self {
        Self {
            surfaces: Surfaces {
                input: rgb(15, 15, 15),
                window: rgb(19, 19, 19),
                panel: rgb(25, 25, 25),
                header: rgb(31, 31, 32),
                elevated: rgb(35, 35, 36),
                hover: rgb(47, 48, 50),
                active: rgb(59, 60, 62),
                stroke: rgb(60, 60, 63),
                stroke_strong: rgb(78, 78, 82),
            },
            accents: Accents {
                accent_active: rgb(196, 203, 212),
                focus_ring: rgb(154, 163, 174),
                accent_soft: rgb(196, 203, 212).with_alpha(0.13),
                on_accent: rgb(14, 21, 32),
            },
            ..Self::invariants()
        }
    }

    /// Rusty — brand preset matching the logo. Warm surfaces, rust-orange
    /// accent. Selection stays the same neutral blue-gray as every preset.
    pub fn rusty() -> Self {
        Self {
            surfaces: Surfaces {
                input: rgb(16, 14, 12),
                window: rgb(21, 18, 16),
                panel: rgb(27, 24, 21),
                header: rgb(34, 30, 26),
                elevated: rgb(38, 34, 29),
                hover: rgb(53, 48, 42),
                active: rgb(66, 59, 51),
                stroke: rgb(62, 56, 49),
                stroke_strong: rgb(87, 78, 68),
            },
            accents: Accents {
                accent_active: rgb(204, 107, 51),
                focus_ring: rgb(224, 150, 104),
                accent_soft: rgb(204, 107, 51).with_alpha(0.15),
                on_accent: rgb(14, 21, 32),
            },
            ..Self::invariants()
        }
    }

    /// Preset-invariant token groups, usable from panel code without a live
    /// theme reference (the design system locks these across presets).
    pub fn invariant_axis() -> Axis {
        Self::invariants().axis
    }
    pub fn invariant_status() -> Status {
        Self::invariants().status
    }
    pub fn invariant_type_colors() -> TypeColors {
        Self::invariants().type_colors
    }

    /// Tokens shared by every preset. (Surfaces/accents here are placeholders
    /// immediately overridden by the preset constructors.)
    fn invariants() -> Self {
        Self {
            surfaces: Surfaces {
                input: Color::BLACK,
                window: Color::BLACK,
                panel: Color::BLACK,
                header: Color::BLACK,
                elevated: Color::BLACK,
                hover: Color::BLACK,
                active: Color::BLACK,
                stroke: Color::BLACK,
                stroke_strong: Color::BLACK,
            },
            accents: Accents {
                accent_active: Color::WHITE,
                focus_ring: Color::WHITE,
                accent_soft: Color::WHITE,
                on_accent: Color::BLACK,
            },
            selection: Selection {
                fill: rgb(60, 70, 84),
                text: Color::WHITE,
            },
            status: Status {
                error: rgb(226, 91, 84),
                warning: rgb(230, 192, 79),
                success: rgb(95, 200, 117),
                info: rgb(110, 168, 232),
                danger: rgb(179, 56, 47),
                mixed: rgb(171, 130, 255),
                overridden: rgb(255, 167, 38),
            },
            axis: Axis {
                x: rgb(224, 82, 82),
                y: rgb(113, 194, 78),
                z: rgb(74, 158, 232),
            },
            type_colors: TypeColors {
                geometry: rgb(168, 176, 186),
                lights: rgb(255, 200, 87),
                cameras: rgb(91, 155, 213),
                vfx: rgb(224, 120, 86),
                audio: rgb(98, 195, 112),
                animation: rgb(230, 107, 184),
                materials: rgb(164, 122, 232),
                scripting: rgb(79, 193, 182),
                physics: rgb(157, 204, 77),
                ui: rgb(240, 140, 126),
            },
            text: TextColors {
                primary: rgb(232, 234, 237),
                secondary: rgb(170, 176, 182),
                disabled: rgb(116, 122, 128),
                mono: rgb(198, 203, 211),
            },
            popover_alpha: 0.96,
            scrim_alpha: 0.45,
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
    /// - Body text (text.primary, text.secondary): >= 4.5:1 against
    ///   window/panel/elevated/input surfaces
    /// - UI text (text.disabled): >= 3.0:1 against the same surfaces
    pub fn verify_wcag_aa(&self) -> Vec<ContrastIssue> {
        let mut issues = Vec::new();

        let bg_surfaces: [(&str, Color); 4] = [
            ("window", self.surfaces.window),
            ("panel", self.surfaces.panel),
            ("elevated", self.surfaces.elevated),
            ("input", self.surfaces.input),
        ];

        // Body text pairs — need 4.5:1
        let body_text: [(&str, Color); 2] = [
            ("text.primary", self.text.primary),
            ("text.secondary", self.text.secondary),
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
        let ui_text: [(&str, Color); 1] = [("text.disabled", self.text.disabled)];
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
    fn all_presets_pass_wcag_aa() {
        for (name, palette) in [
            ("steel", Palette::steel()),
            ("tidepool", Palette::tidepool()),
            ("graphite", Palette::graphite()),
            ("rusty", Palette::rusty()),
        ] {
            let issues = palette.verify_wcag_aa();
            assert!(
                issues.is_empty(),
                "WCAG AA failures in {name}:\n{}",
                issues
                    .iter()
                    .map(|i| format!("  {i}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            );
        }
    }

    #[test]
    fn contrast_ratio_white_on_black() {
        let ratio = contrast_ratio(Color::WHITE, Color::BLACK);
        assert!((ratio - 21.0).abs() < 0.5, "expected ~21:1, got {ratio}");
    }
}
