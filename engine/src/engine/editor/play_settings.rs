//! Editor net-play settings (M9.6): how the Play button launches the game —
//! offline, joining a server, or hosting a local listen server.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum NetPlayMode {
    #[default]
    Standalone,
    Client,
    ListenServer,
}

impl NetPlayMode {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Standalone => "Standalone",
            Self::Client => "Play As Client",
            Self::ListenServer => "Listen Server",
        }
    }
}

/// Persisted inside the editor layout file (`#[serde(default)]`, so old
/// layouts still parse). Defaults mirror `net_config.ron` defaults, so
/// Play As Client against `scripts/host_local.ps1` works with zero config.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlaySettings {
    pub mode: NetPlayMode,
    pub host: String,
    pub module: String,
    /// Total players (1..=4); players beyond the first launch as child
    /// client processes (M9.6 P5).
    pub player_count: u8,
}

impl Default for PlaySettings {
    fn default() -> Self {
        Self {
            mode: NetPlayMode::Standalone,
            host: "http://127.0.0.1:3000".to_string(),
            module: "rust-engine-dev".to_string(),
            player_count: 1,
        }
    }
}
