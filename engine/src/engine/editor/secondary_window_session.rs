//! Session persistence for secondary OS windows.
//!
//! Saves and restores which secondary windows were open (and their positions/sizes)
//! between editor sessions. Follows the same RON file pattern as `WindowConfig`
//! and `EditorDockState`.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use super::secondary_window::SecondaryWindowKind;

/// Session file stored alongside `editor_layout.ron` and `window_config.ron`.
const SESSION_FILE: &str = "secondary_windows.ron";

/// Version tag for forward-compatible migration. Bump when the schema changes.
const SESSION_VERSION: u32 = 1;

/// Root structure persisted to disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecondaryWindowSession {
    /// Schema version for future migration.
    #[serde(default = "default_version")]
    pub version: u32,
    /// One entry per open secondary window at shutdown.
    pub windows: Vec<SecondaryWindowState>,
}

fn default_version() -> u32 {
    SESSION_VERSION
}

/// Serializable snapshot of a single secondary window.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecondaryWindowState {
    /// The window kind as a string tag (e.g. "Material", "Texture").
    pub kind: String,
    /// Content-relative asset path or editor key.
    pub editor_key: String,
    /// Window position in screen coordinates.
    pub x: i32,
    pub y: i32,
    /// Window size in physical pixels.
    pub width: u32,
    pub height: u32,
    /// Whether the window was maximized.
    pub maximized: bool,
}

impl SecondaryWindowSession {
    /// Default (empty) session.
    pub fn empty() -> Self {
        Self {
            version: SESSION_VERSION,
            windows: Vec::new(),
        }
    }

    /// Default file path.
    pub fn default_path() -> PathBuf {
        PathBuf::from(SESSION_FILE)
    }

    /// Save to a RON file.
    pub fn save(&self, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        let ron_str = ron::ser::to_string_pretty(self, Default::default())?;
        fs::write(path, ron_str)?;
        Ok(())
    }

    /// Save to the default path.
    pub fn save_to_default(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.save(&Self::default_path())
    }

    /// Load from a RON file. Returns `None` if the file is missing or invalid.
    pub fn load(path: &Path) -> Option<Self> {
        let content = fs::read_to_string(path).ok()?;
        let session: Self = ron::from_str(&content).ok()?;
        // Version gate: silently discard incompatible future schemas.
        if session.version > SESSION_VERSION {
            log::warn!(
                "secondary_windows.ron version {} > expected {}; ignoring",
                session.version,
                SESSION_VERSION,
            );
            return None;
        }
        Some(session)
    }

    /// Load from the default path, or return an empty session.
    pub fn load_or_default() -> Self {
        Self::load(&Self::default_path()).unwrap_or_else(Self::empty)
    }
}

// ---------------------------------------------------------------------------
// SecondaryWindowKind ↔ session string conversion
// ---------------------------------------------------------------------------

impl SecondaryWindowKind {
    /// Stable string tag for session persistence.
    pub fn as_session_string(&self) -> &'static str {
        match self {
            Self::Mesh => "Mesh",
            Self::Hierarchy => "Hierarchy",
            Self::Inspector => "Inspector",
            Self::AssetBrowser => "AssetBrowser",
            Self::Console => "Console",
            Self::Profiler => "Profiler",
            Self::InputSettings => "InputSettings",
            Self::InputAction => "InputAction",
            Self::InputContext => "InputContext",
            Self::Material => "Material",
            Self::MaterialInstance => "MaterialInstance",
            Self::Texture => "Texture",
            Self::AnimationClip => "AnimationClip",
            Self::AnimationGraph => "AnimationGraph",
            Self::MaterialGraph => "MaterialGraph",
            Self::Audio => "Audio",
            Self::Prefab => "Prefab",
            #[cfg(feature = "editor-debug")]
            Self::IconInspector => "IconInspector",
        }
    }

    /// Parse a session string back to a `SecondaryWindowKind`.
    /// Returns `None` for unrecognised tags (e.g. from a newer version).
    pub fn from_session_string(s: &str) -> Option<Self> {
        match s {
            "Mesh" => Some(Self::Mesh),
            "Hierarchy" => Some(Self::Hierarchy),
            "Inspector" => Some(Self::Inspector),
            "AssetBrowser" => Some(Self::AssetBrowser),
            "Console" => Some(Self::Console),
            "Profiler" => Some(Self::Profiler),
            "InputSettings" => Some(Self::InputSettings),
            "InputAction" => Some(Self::InputAction),
            "InputContext" => Some(Self::InputContext),
            "Material" => Some(Self::Material),
            "MaterialInstance" => Some(Self::MaterialInstance),
            "Texture" => Some(Self::Texture),
            "AnimationClip" => Some(Self::AnimationClip),
            "AnimationGraph" => Some(Self::AnimationGraph),
            "MaterialGraph" => Some(Self::MaterialGraph),
            "Audio" => Some(Self::Audio),
            "Prefab" => Some(Self::Prefab),
            #[cfg(feature = "editor-debug")]
            "IconInspector" => Some(Self::IconInspector),
            _ => None,
        }
    }
}
