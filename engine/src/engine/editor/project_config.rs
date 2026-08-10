//! Project-level settings (M10 P7) — persisted to `project.ron` in the
//! project root and checked into VCS. Edits dirty the project and save with
//! Ctrl+S (never silently), unlike the user-local `editor_prefs.ron`.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use super::build_dialog::BuildTarget;
use crate::engine::plugins::PluginEntry;

pub const PROJECT_FILE: &str = "project.ron";

/// Mirrors `WORLD_SCENE` in `server/game_module/src/lib.rs` — a compile-time
/// module constant, shown read-only (republish the module to change it).
pub const SERVER_WORLD_SCENE: &str = "scenes/greybox.scene";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProjectConfig {
    // Project
    pub name: String,
    pub version: String,

    // Maps & Modes
    pub default_scene: String,
    /// Scene the editor opens at startup; empty = `default_scene`.
    pub editor_startup_scene: String,

    // Physics (Z-up world gravity; negative = down)
    pub gravity_z: f32,
    pub fixed_timestep_hz: f32,

    // Networking (written through to `net_config.ron` on save)
    pub net_host: String,
    pub net_module: String,
    pub net_auto_connect: bool,

    // Streaming (WorldStreamer budgets)
    pub stream_r_load: i32,
    pub stream_r_unload: i32,
    pub stream_budget_ms: f32,
    pub stream_max_in_flight: usize,

    // Build defaults (seed the Build Game dialog)
    pub build_target: BuildTarget,
    pub build_output_dir: String,

    /// Plugin activation (Task 39.8). Empty = every compiled-in plugin is
    /// enabled; an absent id is enabled. Entries naming a plugin that is not
    /// in this build are kept verbatim so moving a project between builds
    /// doesn't lose the user's intent.
    pub plugins: Vec<PluginEntry>,
}

impl Default for ProjectConfig {
    fn default() -> Self {
        Self {
            name: "rust-engine".to_string(),
            version: "0.1.0".to_string(),
            default_scene: "scenes/main.scene".to_string(),
            editor_startup_scene: String::new(),
            gravity_z: -9.81,
            fixed_timestep_hz: 60.0,
            net_host: "http://127.0.0.1:3000".to_string(),
            net_module: "rust-engine-dev".to_string(),
            net_auto_connect: true,
            stream_r_load: 2,
            stream_r_unload: 3,
            stream_budget_ms: 1.0,
            stream_max_in_flight: 2,
            build_target: BuildTarget::Standalone,
            build_output_dir: "build/export".to_string(),
            plugins: Vec::new(),
        }
    }
}

/// Matches `NetConfig` in `game_client/src/net.rs`.
#[derive(Serialize)]
struct NetConfigOut<'a> {
    host: &'a str,
    module: &'a str,
    auto_connect: bool,
}

impl ProjectConfig {
    pub fn path() -> PathBuf {
        PathBuf::from(PROJECT_FILE)
    }

    pub fn load_or_default() -> Self {
        std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|s| ron::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Write `project.ron`, and refresh any exported `net_config.ron` in
    /// place so packaged runtimes keep reading current values.
    pub fn save(&self) -> Result<(), Box<dyn std::error::Error>> {
        let ron_str = ron::ser::to_string_pretty(self, Default::default())?;
        // Atomic (39.8 §5.6): the relaunch flow saves and then exits, so this
        // file is read by the next process moments later.
        super::atomic_file::atomic_write(&Self::path(), &ron_str)?;

        let net = NetConfigOut {
            host: &self.net_host,
            module: &self.net_module,
            auto_connect: self.net_auto_connect,
        };
        for dir in ["build/export-mp", "build/export"] {
            let p = PathBuf::from(dir).join("net_config.ron");
            if p.exists() {
                let s = ron::ser::to_string_pretty(&net, Default::default())?;
                std::fs::write(p, s)?;
            }
        }
        Ok(())
    }

    /// Scene the editor should open at startup.
    pub fn startup_scene(&self) -> &str {
        if self.editor_startup_scene.is_empty() {
            &self.default_scene
        } else {
            &self.editor_startup_scene
        }
    }
}
