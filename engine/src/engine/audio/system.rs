//! AudioSystem — ECS system that drives Kira playback from AudioEmitter components.

use std::collections::{HashMap, HashSet};

use kira::sound::static_sound::StaticSoundHandle;
use kira::sound::PlaybackState;
use kira::track::SpatialTrackHandle;
use kira::Tween;
use nalgebra_glm as glm;
use std::time::Duration;

use super::components::{AudioBus, AudioEmitter, AudioListener};
use super::engine::AudioEngine;
use crate::engine::ecs::components::{Camera, Transform};
use crate::engine::ecs::resources::{EditorState, PlayMode, Resources};
use crate::engine::ecs::schedule::System;

/// Per-entity runtime playback state (not serialized, not an ECS component).
struct EmitterPlaybackState {
    handle: StaticSoundHandle,
    /// Snapshot of the clip path that was loaded — used to detect config changes.
    loaded_clip: String,
    /// Snapshot of the bus that was used.
    loaded_bus: AudioBus,
    /// Spatial track handle — must be kept alive for spatial audio to play.
    spatial_track: Option<SpatialTrackHandle>,
}

/// Resource: queue of content-relative paths that were hot-reloaded.
/// `app.rs` pushes paths here; `AudioSystem` drains them each frame.
pub struct AudioReloadQueue(pub Vec<String>);

impl AudioReloadQueue {
    pub fn new() -> Self {
        Self(Vec::new())
    }
}

impl Default for AudioReloadQueue {
    fn default() -> Self {
        Self::new()
    }
}

/// ECS system that syncs `AudioEmitter` components with Kira playback.
pub struct AudioSystem {
    /// Runtime state keyed by hecs Entity id.
    playback: HashMap<u64, EmitterPlaybackState>,
    /// Tracks entities that have already been auto-played (prevents restart after finish).
    auto_played: HashSet<u64>,
}

impl Default for AudioSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioSystem {
    pub fn new() -> Self {
        Self {
            playback: HashMap::new(),
            auto_played: HashSet::new(),
        }
    }

    /// Extract listener position/forward/up from a Transform.
    fn extract_listener_transform(
        transform: &Transform,
        pos: &mut [f32; 3],
        fwd: &mut [f32; 3],
        up: &mut [f32; 3],
    ) {
        *pos = [transform.position.x, transform.position.y, transform.position.z];
        let rot_mat = glm::quat_to_mat3(&transform.rotation);
        // In Z-up: forward = +Y, up = +Z (engine convention)
        let f = rot_mat * glm::vec3(0.0, 1.0, 0.0);
        let u = rot_mat * glm::vec3(0.0, 0.0, 1.0);
        *fwd = [f.x, f.y, f.z];
        *up = [u.x, u.y, u.z];
    }

    /// Stop and remove playback state for a given entity key.
    fn stop_entity(&mut self, key: u64) {
        if let Some(mut state) = self.playback.remove(&key) {
            state.handle.stop(Tween {
                duration: Duration::from_millis(30),
                ..Default::default()
            });
        }
    }
}

impl System for AudioSystem {
    fn run(&mut self, world: &mut hecs::World, resources: &mut Resources) {
        crate::profile_function!();

        // Early-return if no audio engine.
        if !resources.contains::<AudioEngine>() {
            return;
        }

        // Check play mode — auto-play only works during play mode.
        // When not playing, stop all emitters.
        let is_playing = resources
            .get::<EditorState>()
            .map(|es| es.play_mode == PlayMode::Playing)
            .unwrap_or(true); // If no EditorState, assume runtime (always playing)

        if !is_playing {
            if !self.playback.is_empty() {
                let keys: Vec<u64> = self.playback.keys().copied().collect();
                for key in keys {
                    self.stop_entity(key);
                }
            }
            self.auto_played.clear();
            return;
        }

        // ── Phase 2: Update listener ──
        // Priority: explicit AudioListener > active Camera (like Unreal)
        {
            let mut listener_pos = [0.0f32; 3];
            let mut listener_fwd = [0.0, 0.0, -1.0f32];
            let mut listener_up = [0.0, 0.0, 1.0f32];
            let mut found = false;

            // 1. Check for explicit AudioListener components
            let mut listener_count = 0u32;
            for (_entity, (transform, listener)) in
                world.query::<(&Transform, &AudioListener)>().iter()
            {
                if !listener.active {
                    continue;
                }
                listener_count += 1;
                if listener_count > 1 {
                    log::warn!("Multiple active AudioListeners found — using the first one");
                    break;
                }
                Self::extract_listener_transform(transform, &mut listener_pos, &mut listener_fwd, &mut listener_up);
                found = true;
            }

            // 2. Fallback: use the active Camera (like Unreal's default behavior)
            if !found {
                for (_entity, (transform, camera)) in
                    world.query::<(&Transform, &Camera)>().iter()
                {
                    if !camera.active {
                        continue;
                    }
                    Self::extract_listener_transform(transform, &mut listener_pos, &mut listener_fwd, &mut listener_up);
                    found = true;
                    break;
                }
            }

            if found {
                if let Some(engine) = resources.get_mut::<AudioEngine>() {
                    engine.update_listener(listener_pos, listener_fwd, listener_up);
                }
            }
        }

        // ── Handle hot-reload queue ──
        let reloaded_paths: Vec<String> = resources
            .get_mut::<AudioReloadQueue>()
            .map(|q| q.0.drain(..).collect())
            .unwrap_or_default();

        if !reloaded_paths.is_empty() {
            // Reload the asset cache entries
            if let Some(asset_mgr) = resources.get::<std::sync::Arc<crate::engine::assets::AssetManager>>() {
                for path in &reloaded_paths {
                    if let Err(e) = asset_mgr.audio.reload(path) {
                        log::warn!("Failed to reload audio asset '{path}': {e}");
                    }
                }
            }

            // Stop emitters using reloaded clips so they re-init next frame
            let keys_to_stop: Vec<u64> = self
                .playback
                .iter()
                .filter(|(_, state)| reloaded_paths.contains(&state.loaded_clip))
                .map(|(k, _)| *k)
                .collect();
            for key in keys_to_stop {
                self.stop_entity(key);
            }
        }

        // ── Collect live entity keys ──
        let mut live_keys: Vec<u64> = Vec::new();

        // ── Iterate emitters ──
        // We need to collect emitter data first, then act on it (to avoid
        // holding world borrows while accessing resources).
        struct EmitterSnapshot {
            key: u64,
            clip_path: String,
            bus: AudioBus,
            volume_db: f32,
            pitch: f32,
            looping: bool,
            auto_play: bool,
            spatial: bool,
            max_distance: f32,
            position: [f32; 3],
        }

        let mut snapshots: Vec<EmitterSnapshot> = Vec::new();
        for (entity, emitter) in world.query::<&AudioEmitter>().iter() {
            let key = entity.to_bits().get();
            live_keys.push(key);

            let position = world
                .get::<&Transform>(entity)
                .map(|t| [t.position.x, t.position.y, t.position.z])
                .unwrap_or([0.0, 0.0, 0.0]);

            snapshots.push(EmitterSnapshot {
                key,
                clip_path: emitter.clip_path.clone(),
                bus: emitter.bus,
                volume_db: emitter.volume_db,
                pitch: emitter.pitch,
                looping: emitter.looping,
                auto_play: emitter.auto_play,
                spatial: emitter.spatial,
                max_distance: emitter.max_distance,
                position,
            });
        }

        // ── Cleanup despawned entities ──
        let stale_keys: Vec<u64> = self
            .playback
            .keys()
            .filter(|k| !live_keys.contains(k))
            .copied()
            .collect();
        for key in stale_keys {
            self.stop_entity(key);
            self.auto_played.remove(&key);
        }
        self.auto_played.retain(|k| live_keys.contains(k));

        // ── Process each emitter ──
        for snap in &snapshots {
            if snap.clip_path.is_empty() {
                // No clip assigned — stop if playing
                if self.playback.contains_key(&snap.key) {
                    self.stop_entity(snap.key);
                }
                continue;
            }

            // Check if config changed (clip path or bus)
            let needs_restart = match self.playback.get(&snap.key) {
                Some(state) => state.loaded_clip != snap.clip_path || state.loaded_bus != snap.bus,
                None => snap.auto_play && !self.auto_played.contains(&snap.key),
            };

            if needs_restart {
                // Stop old playback
                self.stop_entity(snap.key);

                // Load audio data
                let sound_data = {
                    let asset_mgr = resources.get::<std::sync::Arc<crate::engine::assets::AssetManager>>();
                    let Some(asset_mgr) = asset_mgr else {
                        continue;
                    };
                    match asset_mgr.audio.load(&snap.clip_path) {
                        Ok(handle) => {
                            let data = handle.get().clone();
                            // Apply settings
                            let mut data = data
                                .volume(kira::Decibels(snap.volume_db))
                                .playback_rate(kira::PlaybackRate(snap.pitch as f64));
                            if snap.looping {
                                data = data.loop_region(..);
                            }
                            data
                        }
                        Err(e) => {
                            log::warn!("Failed to load audio '{}': {e}", snap.clip_path);
                            continue;
                        }
                    }
                };

                // Play
                let engine = resources.get_mut::<AudioEngine>();
                let Some(engine) = engine else { continue };

                let play_result: Result<(StaticSoundHandle, Option<SpatialTrackHandle>), Box<dyn std::error::Error>> = if snap.spatial {
                    engine.play_spatial(sound_data, snap.position, snap.max_distance)
                        .map(|(h, st)| (h, Some(st)))
                } else {
                    engine.play(sound_data, snap.bus)
                        .map(|h| (h, None))
                };

                match play_result {
                    Ok((handle, spatial_track)) => {
                        self.playback.insert(
                            snap.key,
                            EmitterPlaybackState {
                                handle,
                                loaded_clip: snap.clip_path.clone(),
                                loaded_bus: snap.bus,
                                spatial_track,
                            },
                        );
                        if snap.auto_play {
                            self.auto_played.insert(snap.key);
                        }
                    }
                    Err(e) => {
                        log::warn!("Failed to play audio '{}': {e}", snap.clip_path);
                    }
                }
            } else if let Some(state) = self.playback.get_mut(&snap.key) {
                // Sync config changes (volume, pitch) without restarting
                let tween = Tween {
                    duration: Duration::from_millis(30),
                    ..Default::default()
                };
                state
                    .handle
                    .set_volume(kira::Decibels(snap.volume_db), tween);
                state
                    .handle
                    .set_playback_rate(kira::PlaybackRate(snap.pitch as f64), tween);

                // Update spatial track position
                if let Some(ref mut spatial) = state.spatial_track {
                    spatial.set_position(
                        mint::Vector3 {
                            x: snap.position[0],
                            y: snap.position[1],
                            z: snap.position[2],
                        },
                        tween,
                    );
                }

                // Update loop region if changed
                if snap.looping {
                    state.handle.set_loop_region(..);
                } else {
                    state.handle.set_loop_region(None);
                }

                // If stopped/finished and was playing, clean up
                if matches!(state.handle.state(), PlaybackState::Stopped) {
                    self.playback.remove(&snap.key);
                }
            }
        }
    }

    fn name(&self) -> &str {
        "AudioSystem"
    }
}
