//! Window configuration persistence
//!
//! Saves and restores window size, position, and fullscreen state between sessions.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Configuration file name stored in the current directory
const CONFIG_FILE: &str = "window_config.ron";

/// Window configuration that persists between sessions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowConfig {
    /// Window width in pixels
    pub width: u32,
    /// Window height in pixels
    pub height: u32,
    /// Window X position
    pub x: i32,
    /// Window Y position
    pub y: i32,
    /// Whether window was maximized
    pub maximized: bool,
    /// Whether window was in fullscreen mode
    pub fullscreen: bool,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            width: 1280,
            height: 720,
            x: 100,
            y: 100,
            maximized: false,
            fullscreen: false,
        }
    }
}

impl WindowConfig {
    /// Get the default config file path
    pub fn default_path() -> PathBuf {
        PathBuf::from(CONFIG_FILE)
    }

    /// Save config to file
    pub fn save(&self, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        let ron_str = ron::ser::to_string_pretty(self, Default::default())?;
        fs::write(path, ron_str)?;
        Ok(())
    }

    /// Save config to the default file path
    pub fn save_to_default(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.save(&Self::default_path())
    }

    /// Load config from file, returning None if file doesn't exist or is invalid
    pub fn load(path: &Path) -> Option<Self> {
        let content = fs::read_to_string(path).ok()?;
        ron::from_str(&content).ok()
    }

    /// Load config from the default file path, or return default config
    pub fn load_or_default() -> Self {
        Self::load(&Self::default_path())
            .filter(|config| config.is_valid())
            .unwrap_or_default()
    }

    /// Check if the config has valid values
    pub fn is_valid(&self) -> bool {
        if self.width == 0 || self.height == 0 {
            return false;
        }
        if self.x < -10000 || self.y < -10000 {
            return false;
        }
        true
    }
}
