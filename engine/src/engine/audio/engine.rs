//! AudioEngine resource — wraps Kira's AudioManager with named sub-tracks.

use kira::sound::static_sound::{StaticSoundData, StaticSoundHandle};
use kira::track::{SpatialTrackHandle, TrackBuilder, TrackHandle};
use kira::{AudioManager, AudioManagerSettings, DefaultBackend, Tween};
use std::time::Duration;

use super::components::AudioBus;

/// Resource wrapping the Kira audio backend.
///
/// Created once at startup. If creation fails the resource is simply not inserted
/// and `AudioSystem` early-returns.
pub struct AudioEngine {
    manager: AudioManager<DefaultBackend>,
    music_track: TrackHandle,
    sfx_track: TrackHandle,
    ambient_track: TrackHandle,
    preview_track: TrackHandle,
    /// Currently playing preview handle (editor only)
    preview_handle: Option<StaticSoundHandle>,
    /// Kira listener handle (Phase 2)
    listener: Option<kira::listener::ListenerHandle>,
}

// Safety: Kira's AudioManager is Send but not Sync by default.
// We only ever access AudioEngine from the main thread through the System trait.
unsafe impl Sync for AudioEngine {}

impl AudioEngine {
    /// Try to create the audio engine. Returns `None` on failure (no-audio fallback).
    pub fn new() -> Option<Self> {
        let mut manager = match AudioManager::<DefaultBackend>::new(AudioManagerSettings::default())
        {
            Ok(m) => m,
            Err(e) => {
                log::warn!("Failed to create audio engine: {e}. Running without audio.");
                return None;
            }
        };

        let music_track = match manager.add_sub_track(TrackBuilder::default()) {
            Ok(t) => t,
            Err(e) => {
                log::warn!("Failed to create music track: {e}");
                return None;
            }
        };
        let sfx_track = match manager.add_sub_track(TrackBuilder::default()) {
            Ok(t) => t,
            Err(e) => {
                log::warn!("Failed to create sfx track: {e}");
                return None;
            }
        };
        let ambient_track = match manager.add_sub_track(TrackBuilder::default()) {
            Ok(t) => t,
            Err(e) => {
                log::warn!("Failed to create ambient track: {e}");
                return None;
            }
        };
        let preview_track = match manager.add_sub_track(TrackBuilder::default()) {
            Ok(t) => t,
            Err(e) => {
                log::warn!("Failed to create preview track: {e}");
                return None;
            }
        };

        log::info!("Audio engine initialized (Kira backend)");
        Some(Self {
            manager,
            music_track,
            sfx_track,
            ambient_track,
            preview_track,
            preview_handle: None,
            listener: None,
        })
    }

    /// Play a sound on the appropriate bus track. Returns the playback handle.
    pub fn play(
        &mut self,
        data: StaticSoundData,
        bus: AudioBus,
    ) -> Result<StaticSoundHandle, Box<dyn std::error::Error>> {
        let handle = match bus {
            AudioBus::Music => self.music_track.play(data)?,
            AudioBus::SFX => self.sfx_track.play(data)?,
            AudioBus::Ambient => self.ambient_track.play(data)?,
        };
        Ok(handle)
    }

    /// Play a spatial sound — creates a `SpatialTrackHandle` positioned in 3D.
    /// Requires a listener to be active.
    /// Returns both the sound handle and the spatial track handle (must be kept alive).
    pub fn play_spatial(
        &mut self,
        data: StaticSoundData,
        position: [f32; 3],
        max_distance: f32,
    ) -> Result<(StaticSoundHandle, SpatialTrackHandle), Box<dyn std::error::Error>> {
        let listener = self
            .listener
            .as_ref()
            .ok_or("No audio listener active")?;

        let pos = mint::Vector3 {
            x: position[0],
            y: position[1],
            z: position[2],
        };

        let mut spatial_track = self.manager.add_spatial_sub_track(
            listener,
            pos,
            kira::track::SpatialTrackBuilder::new().distances(
                kira::track::SpatialTrackDistances {
                    min_distance: 1.0,
                    max_distance,
                },
            ),
        )?;

        let handle = spatial_track.play(data)?;
        Ok((handle, spatial_track))
    }

    // ── Listener management (Phase 2) ──

    /// Create or update the Kira listener from entity position/orientation.
    pub fn update_listener(&mut self, position: [f32; 3], forward: [f32; 3], up: [f32; 3]) {
        let pos = mint::Vector3 {
            x: position[0],
            y: position[1],
            z: position[2],
        };

        // Build orientation quaternion from forward+up using glam
        let fwd = glam::Vec3::new(forward[0], forward[1], forward[2]).normalize_or_zero();
        let u = glam::Vec3::new(up[0], up[1], up[2]).normalize_or_zero();
        // Kira default: faces -Z with +X right, +Y up
        let orientation = if fwd.length_squared() > 0.0 && u.length_squared() > 0.0 {
            glam::Quat::from_mat3(&glam::Mat3::from_cols(
                fwd.cross(u).normalize(),
                u,
                -fwd,
            ))
        } else {
            glam::Quat::IDENTITY
        };

        let quat = mint::Quaternion {
            v: mint::Vector3 {
                x: orientation.x,
                y: orientation.y,
                z: orientation.z,
            },
            s: orientation.w,
        };

        let tween = Tween {
            duration: Duration::ZERO,
            ..Default::default()
        };

        match &mut self.listener {
            Some(handle) => {
                handle.set_position(pos, tween);
                handle.set_orientation(quat, tween);
            }
            None => {
                match self.manager.add_listener(pos, quat) {
                    Ok(handle) => {
                        self.listener = Some(handle);
                    }
                    Err(e) => {
                        log::warn!("Failed to create audio listener: {e}");
                    }
                }
            }
        }
    }

    /// Whether a listener currently exists.
    pub fn has_listener(&self) -> bool {
        self.listener.is_some()
    }

    // ── Bus volume controls ──

    /// Set the volume of a bus (in dB, instant).
    pub fn set_bus_volume(&mut self, bus: AudioBus, volume_db: f32) {
        let tween = Tween {
            duration: Duration::ZERO,
            ..Default::default()
        };
        let vol = kira::Decibels(volume_db);
        match bus {
            AudioBus::Music => self.music_track.set_volume(vol, tween),
            AudioBus::SFX => self.sfx_track.set_volume(vol, tween),
            AudioBus::Ambient => self.ambient_track.set_volume(vol, tween),
        }
    }

    /// Fade a bus to a target volume over a duration.
    pub fn fade_bus(&mut self, bus: AudioBus, target_db: f32, duration: Duration) {
        let tween = Tween {
            duration,
            ..Default::default()
        };
        let vol = kira::Decibels(target_db);
        match bus {
            AudioBus::Music => self.music_track.set_volume(vol, tween),
            AudioBus::SFX => self.sfx_track.set_volume(vol, tween),
            AudioBus::Ambient => self.ambient_track.set_volume(vol, tween),
        }
    }

    // ── Editor preview ──

    /// Play an audio clip on the dedicated preview track (editor).
    pub fn play_preview(
        &mut self,
        data: StaticSoundData,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Stop any existing preview
        self.stop_preview();
        let handle = self.preview_track.play(data)?;
        self.preview_handle = Some(handle);
        Ok(())
    }

    /// Stop the current preview.
    pub fn stop_preview(&mut self) {
        if let Some(ref mut handle) = self.preview_handle {
            handle.stop(Tween {
                duration: Duration::from_millis(50),
                ..Default::default()
            });
        }
        self.preview_handle = None;
    }

    /// Whether a preview is currently playing.
    pub fn is_preview_playing(&self) -> bool {
        self.preview_handle.as_ref().is_some_and(|h| {
            matches!(
                h.state(),
                kira::sound::PlaybackState::Playing | kira::sound::PlaybackState::Pausing
            )
        })
    }

    /// Mutable access to the underlying Kira manager (for advanced use).
    pub fn manager_mut(&mut self) -> &mut AudioManager<DefaultBackend> {
        &mut self.manager
    }
}
