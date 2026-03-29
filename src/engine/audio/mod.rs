//! Audio system — Kira 0.12 integration with ECS-driven playback.

pub mod audio_asset_manager;
pub mod components;
pub mod debug_draw;
pub mod engine;
pub mod system;

pub use audio_asset_manager::AudioAssetManager;
pub use components::{AudioBus, AudioEmitter, AudioListener};
pub use engine::AudioEngine;
pub use system::{AudioReloadQueue, AudioSystem};
