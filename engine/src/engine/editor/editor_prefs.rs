//! User-local editor preferences (M10 P7) — persisted to `editor_prefs.ron`
//! in the working directory (gitignored). Changes apply live and autosave
//! with a short debounce; there is no OK/Apply button.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use super::play_settings::PlaySettings;
use super::theme::{EditorTheme, UI_SCALE_MAX, UI_SCALE_MIN};

pub const PREFS_FILE: &str = "editor_prefs.ron";

/// Crusty Design System palette presets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ThemePreset {
    #[default]
    Steel,
    Tidepool,
    Graphite,
    Rusty,
}

impl ThemePreset {
    /// Presets offered in the Editor Preferences picker. Rusty is
    /// deliberately absent (DESIGN.md ▸ Presets): it is a brand-demo preset
    /// for splash/about/marketing shots — orange survives in exactly one
    /// place, the brand mark. The constructor stays for those surfaces.
    pub const USER_SELECTABLE: [ThemePreset; 3] = [
        ThemePreset::Steel,
        ThemePreset::Tidepool,
        ThemePreset::Graphite,
    ];

    /// True for presets a user may pick in the preferences window.
    pub fn user_selectable(self) -> bool {
        Self::USER_SELECTABLE.contains(&self)
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Steel => "Steel",
            Self::Tidepool => "Tidepool",
            Self::Graphite => "Graphite",
            Self::Rusty => "Rusty",
        }
    }

    pub fn theme(&self) -> EditorTheme {
        match self {
            Self::Steel => EditorTheme::steel(),
            Self::Tidepool => EditorTheme::tidepool(),
            Self::Graphite => EditorTheme::graphite(),
            Self::Rusty => EditorTheme::rusty(),
        }
    }
}

/// All user-local editor preferences. `#[serde(default)]` keeps older files
/// parsing as fields are added.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EditorPrefs {
    // Appearance
    pub theme_preset: ThemePreset,
    pub popover_translucent: bool,
    /// Global UI scale (DESIGN.md's `ui_scale`): every editor metric is a
    /// base value × this. Density presets are named values of it.
    pub ui_scale: f32,

    // Viewport — camera
    pub camera_speed: f32,
    pub camera_speed_scalar: f32,
    pub mouse_sensitivity: f32,
    pub invert_y: bool,
    pub fov_deg: f32,

    // Viewport — grid & gizmos
    pub grid_visible: bool,
    pub gizmo_size: f32,

    // Snapping
    pub grid_snap_enabled: bool,
    pub rotation_snap_enabled: bool,
    pub scale_snap_enabled: bool,
    pub snap_translate: f32,
    pub snap_rotate: f32,
    pub snap_scale: f32,

    // Editing
    pub undo_limit: usize,

    // Graph editor (Task 40) — canvas zoom limits.
    pub graph_zoom_min: f32,
    pub graph_zoom_max: f32,

    // Asset browser
    pub thumbnail_size: f32,

    // Console
    pub console_max_lines: usize,
    pub console_show_info: bool,
    pub console_show_warning: bool,
    pub console_show_error: bool,

    // Play (migrated from `CrustyDockLayout.play_settings`)
    pub play: PlaySettings,
}

impl Default for EditorPrefs {
    fn default() -> Self {
        Self {
            theme_preset: ThemePreset::Steel,
            popover_translucent: true,
            ui_scale: 1.0,
            camera_speed: 1.0,
            camera_speed_scalar: 1.0,
            mouse_sensitivity: 0.003,
            invert_y: false,
            fov_deg: 45.0,
            grid_visible: true,
            gizmo_size: 75.0,
            grid_snap_enabled: false,
            rotation_snap_enabled: false,
            scale_snap_enabled: false,
            snap_translate: 1.0,
            snap_rotate: 15.0,
            snap_scale: 0.1,
            undo_limit: 100,
            graph_zoom_min: 0.25,
            graph_zoom_max: 2.5,
            thumbnail_size: 96.0,
            console_max_lines: 2000,
            console_show_info: true,
            console_show_warning: true,
            console_show_error: true,
            play: PlaySettings::default(),
        }
    }
}

impl EditorPrefs {
    pub fn path() -> PathBuf {
        PathBuf::from(PREFS_FILE)
    }

    /// Load, returning `(prefs, file_existed)`. A missing or unparsable file
    /// yields defaults so the caller can seed migrated values.
    pub fn load() -> (Self, bool) {
        match std::fs::read_to_string(Self::path()) {
            Ok(s) => match ron::from_str::<Self>(&s) {
                Ok(mut p) => {
                    p.normalize();
                    (p, true)
                }
                Err(_) => (Self::default(), false),
            },
            Err(_) => (Self::default(), false),
        }
    }

    /// Repair values a stored file may hold that the current build no longer
    /// offers. Never fails — a pref file is not worth a crash.
    fn normalize(&mut self) {
        if !self.theme_preset.user_selectable() {
            // Rusty was pickable before it became brand-demo-only.
            println!(
                "editor prefs: theme preset '{}' is no longer user-selectable \
                 (brand-demo only) — falling back to Steel",
                self.theme_preset.label()
            );
            self.theme_preset = ThemePreset::Steel;
        }
        if !self.ui_scale.is_finite() {
            self.ui_scale = 1.0;
        }
        self.ui_scale = self.ui_scale.clamp(UI_SCALE_MIN, UI_SCALE_MAX);
    }

    pub fn save(&self) -> Result<(), Box<dyn std::error::Error>> {
        let ron_str = ron::ser::to_string_pretty(self, Default::default())?;
        std::fs::write(Self::path(), ron_str)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rusty_is_not_user_selectable() {
        assert!(!ThemePreset::Rusty.user_selectable());
        assert_eq!(ThemePreset::USER_SELECTABLE.len(), 3);
        for t in ThemePreset::USER_SELECTABLE {
            assert!(t.user_selectable());
        }
        // The constructor stays for brand surfaces.
        let _ = ThemePreset::Rusty.theme();
    }

    #[test]
    fn stored_rusty_falls_back_to_steel() {
        let mut p = EditorPrefs {
            theme_preset: ThemePreset::Rusty,
            ui_scale: 12.0,
            ..EditorPrefs::default()
        };
        p.normalize();
        assert_eq!(p.theme_preset, ThemePreset::Steel);
        assert_eq!(p.ui_scale, UI_SCALE_MAX);
    }
}
