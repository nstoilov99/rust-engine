//! User-local editor preferences (M10 P7) — persisted to `editor_prefs.ron`
//! in the working directory (gitignored). Changes apply live and autosave
//! with a short debounce; there is no OK/Apply button.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use super::play_settings::PlaySettings;
use super::theme::EditorTheme;

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
    pub const ALL: [ThemePreset; 4] = [
        ThemePreset::Steel,
        ThemePreset::Tidepool,
        ThemePreset::Graphite,
        ThemePreset::Rusty,
    ];

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
            Ok(s) => match ron::from_str(&s) {
                Ok(p) => (p, true),
                Err(_) => (Self::default(), false),
            },
            Err(_) => (Self::default(), false),
        }
    }

    pub fn save(&self) -> Result<(), Box<dyn std::error::Error>> {
        let ron_str = ron::ser::to_string_pretty(self, Default::default())?;
        std::fs::write(Self::path(), ron_str)?;
        Ok(())
    }
}
