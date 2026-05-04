//! Icon classification system for the editor.
//!
//! Three rendering modes for editor icons:
//! - **Chrome:** monochrome UI glyphs, tinted per interaction state
//! - **Typed:** category-colored icons (Mesh = grey, Light = amber, etc.)
//! - **Severity:** semantic state icons (error red, warning yellow, etc.)
//!
//! All icons render as white source pixels tinted at draw time. The tint color
//! is resolved from the `IconPalette` based on the icon's class.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use egui::Color32;
use serde::{Deserialize, Serialize};

/// Filename used when saving the palette to the workspace root.
const PALETTE_FILE: &str = "editor_icon_palette.ron";

/// Three rendering modes for editor icons.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, serde::Serialize, serde::Deserialize)]
pub enum IconClass {
    /// Monochrome glyph, tinted at draw time per interaction state.
    /// Source pixels are pure white + alpha; tint multiplies cleanly.
    Chrome,
    /// Category-colored icon. Source is white; tinted with category color.
    Typed,
    /// Semantic state icon. Source is white; tinted with severity color.
    Severity,
}

/// Semantic categories for Typed icons.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, serde::Serialize, serde::Deserialize)]
pub enum TypeCategory {
    Geometry,
    Lights,
    Cameras,
    Vfx,
    Audio,
    Animation,
    Materials,
    Scripting,
    Physics,
    Ui,
}

impl TypeCategory {
    pub const ALL: &'static [TypeCategory] = &[
        Self::Geometry,
        Self::Lights,
        Self::Cameras,
        Self::Vfx,
        Self::Audio,
        Self::Animation,
        Self::Materials,
        Self::Scripting,
        Self::Physics,
        Self::Ui,
    ];

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Geometry => "Geometry",
            Self::Lights => "Lights",
            Self::Cameras => "Cameras",
            Self::Vfx => "VFX",
            Self::Audio => "Audio",
            Self::Animation => "Animation",
            Self::Materials => "Materials",
            Self::Scripting => "Scripting",
            Self::Physics => "Physics",
            Self::Ui => "UI",
        }
    }
}

/// Severity levels for status indicators.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, serde::Serialize, serde::Deserialize)]
pub enum Severity {
    Error,
    Warning,
    Success,
    Info,
}

impl Severity {
    pub const ALL: &'static [Severity] = &[Self::Error, Self::Warning, Self::Success, Self::Info];

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Error => "Error",
            Self::Warning => "Warning",
            Self::Success => "Success",
            Self::Info => "Info",
        }
    }
}

/// Interaction state for Chrome icons.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, serde::Serialize, serde::Deserialize)]
pub enum ChromeState {
    Default,
    Hovered,
    Active,
    Disabled,
    Accent,
}

impl ChromeState {
    pub const ALL: &'static [ChromeState] = &[
        Self::Default,
        Self::Hovered,
        Self::Active,
        Self::Disabled,
        Self::Accent,
    ];

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Default => "Default",
            Self::Hovered => "Hovered",
            Self::Active => "Active",
            Self::Disabled => "Disabled",
            Self::Accent => "Accent",
        }
    }
}

/// How an icon's pixels reach the screen.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, serde::Serialize, serde::Deserialize)]
pub enum TintMode {
    /// White-source atlas pixels; final color = looked-up tint.
    Tinted,
    /// Multi-color authored art; drawn as-is, no tint applied.
    Authored,
}

/// Color palette for the icon system.
///
/// Class-default colors are looked up by category/severity/chrome-state.
/// Per-icon overrides take precedence over class defaults for `Tinted` icons.
#[derive(Clone, Debug)]
pub struct IconPalette {
    /// Category colors for Typed icons.
    pub category: HashMap<TypeCategory, Color32>,
    /// Severity colors for Severity icons.
    pub severity: HashMap<Severity, Color32>,
    /// Chrome state colors for Chrome icons.
    pub chrome: HashMap<ChromeState, Color32>,
    /// Per-icon, per-state tint overrides. Only applies to `TintMode::Tinted`
    /// icons. The lookup chain in `IconRegistry::tint` is:
    /// `(kind, state)` → `(kind, Default)` → class default.
    pub overrides: HashMap<(super::widgets::IconKind, ChromeState), Color32>,
    /// Per-icon tint mode overrides.
    pub tint_modes: HashMap<super::widgets::IconKind, TintMode>,
}

impl IconPalette {
    /// Default dark theme icon palette.
    pub fn default_dark() -> Self {
        let mut category = HashMap::new();
        category.insert(TypeCategory::Geometry, Color32::from_rgb(0xA8, 0xB0, 0xBA));
        category.insert(TypeCategory::Lights, Color32::from_rgb(0xFF, 0xC8, 0x57));
        category.insert(TypeCategory::Cameras, Color32::from_rgb(0x5B, 0x9B, 0xD5));
        category.insert(TypeCategory::Vfx, Color32::from_rgb(0xE0, 0x78, 0x56));
        category.insert(TypeCategory::Audio, Color32::from_rgb(0x62, 0xC3, 0x70));
        category.insert(TypeCategory::Animation, Color32::from_rgb(0xE6, 0x6B, 0xB8));
        category.insert(TypeCategory::Materials, Color32::from_rgb(0xA4, 0x7A, 0xE8));
        category.insert(TypeCategory::Scripting, Color32::from_rgb(0x4F, 0xC1, 0xB6));
        category.insert(TypeCategory::Physics, Color32::from_rgb(0x9D, 0xCC, 0x4D));
        category.insert(TypeCategory::Ui, Color32::from_rgb(0xF0, 0x8C, 0x7E));

        let mut severity = HashMap::new();
        severity.insert(Severity::Error, Color32::from_rgb(0xE5, 0x6B, 0x6B));
        severity.insert(Severity::Warning, Color32::from_rgb(0xE6, 0xC0, 0x4F));
        severity.insert(Severity::Success, Color32::from_rgb(0x6E, 0xCB, 0x7C));
        severity.insert(Severity::Info, Color32::from_rgb(0x6E, 0xA8, 0xE8));

        let mut chrome = HashMap::new();
        chrome.insert(ChromeState::Default, Color32::from_rgb(0xC0, 0xC4, 0xCC));
        chrome.insert(ChromeState::Hovered, Color32::from_rgb(0xE6, 0xE8, 0xEC));
        chrome.insert(ChromeState::Active, Color32::WHITE);
        chrome.insert(ChromeState::Disabled, Color32::from_rgb(0x60, 0x64, 0x6C));
        chrome.insert(ChromeState::Accent, Color32::from_rgb(0x4F, 0xA3, 0xE8));

        Self {
            category,
            severity,
            chrome,
            overrides: HashMap::new(),
            tint_modes: HashMap::new(),
        }
    }

    /// Reset all overrides and tint modes back to defaults.
    pub fn reset(&mut self) {
        *self = Self::default_dark();
    }

    /// Path used by `save_to_default` / `load_from_default`.
    pub fn default_path() -> PathBuf {
        PathBuf::from(PALETTE_FILE)
    }

    /// Serialize the palette to a RON file.
    pub fn save(&self, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        let file = PaletteFile::from(self);
        let ron_str = ron::ser::to_string_pretty(&file, ron::ser::PrettyConfig::default())?;
        std::fs::write(path, ron_str)?;
        Ok(())
    }

    /// Save to the default workspace-root path (`editor_icon_palette.ron`).
    pub fn save_to_default(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.save(&Self::default_path())
    }

    /// Try loading a palette from a RON file. Returns `None` if the file is
    /// missing or unparseable.
    pub fn load(path: &Path) -> Option<Self> {
        let content = std::fs::read_to_string(path).ok()?;
        let file: PaletteFile = ron::from_str(&content).ok()?;
        Some(file.into())
    }

    /// Load from the default workspace-root path; returns `None` if absent.
    pub fn load_from_default() -> Option<Self> {
        Self::load(&Self::default_path())
    }
}

// ─── On-disk representation ──────────────────────────────────────────────────
//
// `Color32` and the rest of egui aren't pulled in with the `serde` feature, so
// we map to a serde-friendly shape with `[u8; 4]` colors and convert back and
// forth. Keeps the on-disk file readable and version-tolerant.

#[derive(Serialize, Deserialize)]
struct PaletteFile {
    category: Vec<(TypeCategory, [u8; 4])>,
    severity: Vec<(Severity, [u8; 4])>,
    chrome: Vec<(ChromeState, [u8; 4])>,
    /// `(IconKind, ChromeState, RGBA)` so the on-disk shape stays a flat list
    /// of tuples — easy to read, easy to diff, no nested maps.
    overrides: Vec<(super::widgets::IconKind, ChromeState, [u8; 4])>,
    tint_modes: Vec<(super::widgets::IconKind, TintMode)>,
}

fn color_to_rgba(c: Color32) -> [u8; 4] {
    [c.r(), c.g(), c.b(), c.a()]
}

fn rgba_to_color([r, g, b, a]: [u8; 4]) -> Color32 {
    Color32::from_rgba_unmultiplied(r, g, b, a)
}

impl From<&IconPalette> for PaletteFile {
    fn from(p: &IconPalette) -> Self {
        let mut category: Vec<_> = p.category.iter().map(|(k, v)| (*k, color_to_rgba(*v))).collect();
        let mut severity: Vec<_> = p.severity.iter().map(|(k, v)| (*k, color_to_rgba(*v))).collect();
        let mut chrome: Vec<_> = p.chrome.iter().map(|(k, v)| (*k, color_to_rgba(*v))).collect();
        let mut overrides: Vec<_> = p
            .overrides
            .iter()
            .map(|((kind, state), v)| (*kind, *state, color_to_rgba(*v)))
            .collect();
        let mut tint_modes: Vec<_> = p.tint_modes.iter().map(|(k, v)| (*k, *v)).collect();

        // Stable order so diffs stay readable.
        category.sort_by_key(|(k, _)| format!("{k:?}"));
        severity.sort_by_key(|(k, _)| format!("{k:?}"));
        chrome.sort_by_key(|(k, _)| format!("{k:?}"));
        overrides.sort_by_key(|(kind, state, _)| format!("{kind:?}/{state:?}"));
        tint_modes.sort_by_key(|(k, _)| format!("{k:?}"));

        Self { category, severity, chrome, overrides, tint_modes }
    }
}

impl From<PaletteFile> for IconPalette {
    fn from(f: PaletteFile) -> Self {
        // Start from defaults so any newly-added entries are populated even
        // when loading an older file.
        let mut palette = IconPalette::default_dark();
        // Saved overrides replace defaults rather than merge with them — the
        // file is the source of truth for the keys it does contain.
        palette.overrides.clear();
        palette.tint_modes.clear();

        for (k, v) in f.category {
            palette.category.insert(k, rgba_to_color(v));
        }
        for (k, v) in f.severity {
            palette.severity.insert(k, rgba_to_color(v));
        }
        for (k, v) in f.chrome {
            palette.chrome.insert(k, rgba_to_color(v));
        }
        for (kind, state, v) in f.overrides {
            palette.overrides.insert((kind, state), rgba_to_color(v));
        }
        for (k, v) in f.tint_modes {
            palette.tint_modes.insert(k, v);
        }
        palette
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_palette_has_all_categories() {
        let palette = IconPalette::default_dark();
        for cat in TypeCategory::ALL {
            assert!(
                palette.category.contains_key(cat),
                "Missing category color: {cat:?}"
            );
        }
    }

    #[test]
    fn default_palette_has_all_severities() {
        let palette = IconPalette::default_dark();
        for sev in Severity::ALL {
            assert!(
                palette.severity.contains_key(sev),
                "Missing severity color: {sev:?}"
            );
        }
    }

    #[test]
    fn default_palette_has_all_chrome_states() {
        let palette = IconPalette::default_dark();
        for state in ChromeState::ALL {
            assert!(
                palette.chrome.contains_key(state),
                "Missing chrome state color: {state:?}"
            );
        }
    }

    #[test]
    fn reset_clears_overrides() {
        let mut palette = IconPalette::default_dark();
        palette.overrides.insert(
            (
                crate::engine::editor::widgets::IconKind::Folder,
                ChromeState::Default,
            ),
            Color32::RED,
        );
        palette
            .tint_modes
            .insert(crate::engine::editor::widgets::IconKind::Folder, TintMode::Authored);
        palette.reset();
        assert!(palette.overrides.is_empty());
        assert!(palette.tint_modes.is_empty());
    }
}
