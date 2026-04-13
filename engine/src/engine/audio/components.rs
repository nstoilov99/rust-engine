//! Audio ECS components — pure config, no Kira types.

use serde::{Deserialize, Serialize};

/// Which mixer bus an emitter routes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AudioBus {
    Music,
    SFX,
    Ambient,
}

impl Default for AudioBus {
    fn default() -> Self {
        Self::SFX
    }
}

impl AudioBus {
    /// All bus variants for UI dropdowns.
    pub const ALL: &'static [AudioBus] = &[AudioBus::Music, AudioBus::SFX, AudioBus::Ambient];

    pub fn display_name(&self) -> &'static str {
        match self {
            AudioBus::Music => "Music",
            AudioBus::SFX => "SFX",
            AudioBus::Ambient => "Ambient",
        }
    }
}

/// Audio emitter component (pure config — no Kira handles).
///
/// Runtime playback state is tracked in `AudioSystem`'s internal map.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioEmitter {
    /// Content-relative path to audio clip (e.g. "audio/music.ogg")
    #[serde(default)]
    pub clip_path: String,
    /// Which mixer bus to route through
    #[serde(default)]
    pub bus: AudioBus,
    /// Volume in dB (0.0 = unity, negative = quieter)
    #[serde(default)]
    pub volume_db: f32,
    /// Playback rate multiplier (1.0 = normal)
    #[serde(default = "default_pitch")]
    pub pitch: f32,
    /// Whether the clip loops
    #[serde(default)]
    pub looping: bool,
    /// Auto-play when the emitter is spawned / enters play mode
    #[serde(default)]
    pub auto_play: bool,
    /// Spatial (3D) audio — position comes from the entity's Transform
    #[serde(default)]
    pub spatial: bool,
    /// Max audible distance for spatial emitters (metres)
    #[serde(default = "default_max_distance")]
    pub max_distance: f32,
    /// Hide range wireframe during play mode (visible in edit mode regardless)
    #[serde(default = "default_true")]
    pub hide_range_in_game: bool,
}

impl Default for AudioEmitter {
    fn default() -> Self {
        Self {
            clip_path: String::new(),
            bus: AudioBus::default(),
            volume_db: 0.0,
            pitch: 1.0,
            looping: false,
            auto_play: false,
            spatial: false,
            max_distance: 50.0,
            hide_range_in_game: true,
        }
    }
}

/// Audio listener component — marks the entity whose Transform drives the
/// spatial audio listener position/orientation.
///
/// Only the first active listener found is used; extras log a warning.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct AudioListener {
    /// Whether this listener is active
    #[serde(default = "default_true")]
    pub active: bool,
}

impl Default for AudioListener {
    fn default() -> Self {
        Self { active: true }
    }
}

fn default_pitch() -> f32 {
    1.0
}

fn default_max_distance() -> f32 {
    50.0
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_emitter_defaults() {
        let e = AudioEmitter::default();
        assert!(e.clip_path.is_empty());
        assert_eq!(e.bus, AudioBus::SFX);
        assert_eq!(e.volume_db, 0.0);
        assert_eq!(e.pitch, 1.0);
        assert!(!e.looping);
        assert!(!e.auto_play);
        assert!(!e.spatial);
        assert_eq!(e.max_distance, 50.0);
    }

    #[test]
    fn audio_listener_defaults() {
        let l = AudioListener::default();
        assert!(l.active);
    }

    #[test]
    fn audio_emitter_serde_roundtrip() {
        let emitter = AudioEmitter {
            clip_path: "audio/test.ogg".to_string(),
            bus: AudioBus::Music,
            volume_db: -6.0,
            pitch: 1.5,
            looping: true,
            auto_play: true,
            spatial: true,
            max_distance: 100.0,
            hide_range_in_game: true,
        };
        let ron_str = ron::to_string(&emitter).expect("serialize");
        let decoded: AudioEmitter = ron::from_str(&ron_str).expect("deserialize");
        assert_eq!(decoded.clip_path, "audio/test.ogg");
        assert_eq!(decoded.bus, AudioBus::Music);
        assert_eq!(decoded.volume_db, -6.0);
        assert_eq!(decoded.pitch, 1.5);
        assert!(decoded.looping);
        assert!(decoded.auto_play);
        assert!(decoded.spatial);
        assert_eq!(decoded.max_distance, 100.0);
    }

    #[test]
    fn audio_listener_serde_roundtrip() {
        let listener = AudioListener { active: false };
        let ron_str = ron::to_string(&listener).expect("serialize");
        let decoded: AudioListener = ron::from_str(&ron_str).expect("deserialize");
        assert!(!decoded.active);
    }

    #[test]
    fn audio_bus_display_names() {
        assert_eq!(AudioBus::Music.display_name(), "Music");
        assert_eq!(AudioBus::SFX.display_name(), "SFX");
        assert_eq!(AudioBus::Ambient.display_name(), "Ambient");
    }

    #[test]
    fn audio_bus_all_contains_all_variants() {
        assert_eq!(AudioBus::ALL.len(), 3);
        assert!(AudioBus::ALL.contains(&AudioBus::Music));
        assert!(AudioBus::ALL.contains(&AudioBus::SFX));
        assert!(AudioBus::ALL.contains(&AudioBus::Ambient));
    }

    #[test]
    fn audio_emitter_deserialize_with_defaults() {
        // Minimal RON — all optional fields should use defaults
        let ron_str = "(clip_path: \"audio/bg.ogg\")";
        let decoded: AudioEmitter = ron::from_str(ron_str).expect("deserialize minimal");
        assert_eq!(decoded.clip_path, "audio/bg.ogg");
        assert_eq!(decoded.bus, AudioBus::SFX); // default
        assert_eq!(decoded.volume_db, 0.0);
        assert_eq!(decoded.pitch, 1.0);
        assert!(!decoded.looping);
        assert!(!decoded.auto_play);
        assert!(!decoded.spatial);
        assert_eq!(decoded.max_distance, 50.0);
    }

    #[test]
    fn audio_emitter_clone_preserves_all_fields() {
        let original = AudioEmitter {
            clip_path: "music.ogg".to_string(),
            bus: AudioBus::Ambient,
            volume_db: -12.0,
            pitch: 0.5,
            looping: true,
            auto_play: true,
            spatial: true,
            max_distance: 200.0,
            hide_range_in_game: false,
        };
        let cloned = original.clone();
        assert_eq!(cloned.clip_path, original.clip_path);
        assert_eq!(cloned.bus, original.bus);
        assert_eq!(cloned.volume_db, original.volume_db);
        assert_eq!(cloned.pitch, original.pitch);
        assert_eq!(cloned.looping, original.looping);
        assert_eq!(cloned.auto_play, original.auto_play);
        assert_eq!(cloned.spatial, original.spatial);
        assert_eq!(cloned.max_distance, original.max_distance);
    }
}
