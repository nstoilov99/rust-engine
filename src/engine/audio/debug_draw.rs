//! Audio debug draw — orange cross + wireframe sphere for spatial emitters.

use crate::engine::audio::components::AudioEmitter;
use crate::engine::debug_draw::DebugDrawBuffer;
use crate::engine::ecs::components::Transform;

/// Orange color for audio debug visualization.
const AUDIO_COLOR: [f32; 4] = [1.0, 0.55, 0.2, 1.0];

/// Submit debug draw primitives for all spatial AudioEmitters.
///
/// Draws an orange cross at the emitter position and a wireframe sphere
/// showing the `max_distance` attenuation radius.
pub fn submit_audio_debug_draws(world: &hecs::World, buffer: &mut DebugDrawBuffer, is_playing: bool) {
    for (_entity, (transform, emitter)) in world.query::<(&Transform, &AudioEmitter)>().iter() {
        if !emitter.spatial || emitter.clip_path.is_empty() {
            continue;
        }
        // Hide in play mode when toggle is on (default)
        if is_playing && emitter.hide_range_in_game {
            continue;
        }

        let pos = [
            transform.position.x,
            transform.position.y,
            transform.position.z,
        ];

        // Orange cross at emitter position
        let s = 0.3;
        buffer.line(
            [pos[0] - s, pos[1], pos[2]],
            [pos[0] + s, pos[1], pos[2]],
            AUDIO_COLOR,
        );
        buffer.line(
            [pos[0], pos[1] - s, pos[2]],
            [pos[0], pos[1] + s, pos[2]],
            AUDIO_COLOR,
        );
        buffer.line(
            [pos[0], pos[1], pos[2] - s],
            [pos[0], pos[1], pos[2] + s],
            AUDIO_COLOR,
        );

        // Wireframe sphere showing max_distance
        buffer.sphere_wireframe(pos, emitter.max_distance, AUDIO_COLOR);
    }
}
