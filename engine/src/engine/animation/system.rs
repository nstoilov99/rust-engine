//! Animation update system — advances playback, samples keyframes,
//! computes forward kinematics, and marks skeletons dirty for GPU upload.

use crate::engine::animation::components::{AnimationPlayer, PlaybackState, SkeletonInstance};
use crate::engine::animation::sampling;
use crate::engine::ecs::resources::Resources;
use crate::engine::ecs::schedule::System;

/// ECS system that drives skeletal animation each frame.
///
/// For each entity with both `AnimationPlayer` and `SkeletonInstance`:
/// 1. Advances playback time by `Time::scaled_delta()`
/// 2. Samples keyframes into local bone transforms
/// 3. Applies crossfade blending if active
/// 4. Runs forward kinematics to recompute the bone palette
///
/// Register in `Stage::PreUpdate` (before transform propagation).
pub struct AnimationUpdateSystem;

impl System for AnimationUpdateSystem {
    fn run(&mut self, world: &mut hecs::World, resources: &mut Resources) {
        crate::profile_function!();

        let dt = resources
            .get::<crate::engine::ecs::resources::Time>()
            .map(|t| t.scaled_delta())
            .unwrap_or(0.0);

        for (_entity, (player, skeleton)) in
            world.query_mut::<(&mut AnimationPlayer, &mut SkeletonInstance)>()
        {
            if player.state != PlaybackState::Playing {
                continue;
            }

            // 1. Advance time
            player.advance(dt);

            // 2. Sample keyframes into local transforms
            sampling::sample_channels(
                &player.clip.channels,
                player.time,
                &mut skeleton.local_transforms,
            );

            // 3. Apply crossfade blending if active
            if let Some(ref mut fade) = player.crossfade {
                fade.elapsed += dt;
                let t = fade.blend_weight();

                // Blend from snapshot → newly sampled transforms
                let bone_count = skeleton.local_transforms.len().min(fade.from_transforms.len());
                for i in 0..bone_count {
                    skeleton.local_transforms[i] =
                        fade.from_transforms[i].blend(&skeleton.local_transforms[i], t);
                }

                if fade.is_done() {
                    player.crossfade = None;
                }
            }

            // 4. Forward kinematics → palette
            skeleton.compute_palette();
        }
    }

    fn name(&self) -> &str {
        "AnimationUpdateSystem"
    }
}
