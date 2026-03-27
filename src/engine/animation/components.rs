//! ECS components for skeletal animation.

use glam::{Mat4, Quat, Vec3};

use crate::engine::assets::model_loader::{BoneData, RawAnimationClip};

/// Runtime skeleton instance attached to an entity.
///
/// Stores the bone hierarchy, current local-space transforms, and
/// the computed world-space bone palette ready for GPU upload.
pub struct SkeletonInstance {
    /// Per-bone metadata (name, parent, inverse-bind).
    pub bones: Vec<BoneData>,
    /// Per-bone local-space transforms (SQT), indexed by bone index.
    pub local_transforms: Vec<LocalBoneTransform>,
    /// World-space bone palette: `inverse_bind * world_transform` for each bone.
    /// This is what gets uploaded to the GPU bone palette UBO.
    pub palette: Vec<Mat4>,
    /// Whether the palette needs re-uploading this frame.
    pub dirty: bool,
    /// Whether to show debug bone visualization in the viewport.
    pub debug_draw_visible: bool,
}

/// Local-space bone transform in SQT (Scale-Quaternion-Translation) form.
#[derive(Debug, Clone, Copy)]
pub struct LocalBoneTransform {
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

impl Default for LocalBoneTransform {
    fn default() -> Self {
        Self {
            translation: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        }
    }
}

impl LocalBoneTransform {
    /// Convert to a 4x4 affine matrix (T * R * S).
    pub fn to_matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }

    /// Linearly blend between two SQT transforms.
    /// `t = 0.0` returns `self`, `t = 1.0` returns `other`.
    pub fn blend(&self, other: &Self, t: f32) -> Self {
        Self {
            translation: self.translation.lerp(other.translation, t),
            rotation: self.rotation.slerp(other.rotation, t),
            scale: self.scale.lerp(other.scale, t),
        }
    }
}

impl SkeletonInstance {
    /// Create from a bone hierarchy. Initializes all local transforms to
    /// the rest pose (derived from inverse-bind matrices).
    pub fn from_bones(bones: Vec<BoneData>) -> Self {
        let bone_count = bones.len();

        // Derive rest-pose local transforms from inverse bind matrices.
        // The inverse bind matrix transforms from model space → bone-local space,
        // so the bind matrix (inverse of inverse-bind) gives model-space pose.
        let local_transforms: Vec<LocalBoneTransform> = (0..bone_count)
            .map(|i| {
                let bind_matrix = bones[i].inverse_bind_matrix.inverse();
                let parent_bind = bones[i]
                    .parent_index
                    .map(|p| bones[p].inverse_bind_matrix.inverse())
                    .unwrap_or(Mat4::IDENTITY);
                // Local = parent_bind^-1 * bind_matrix
                let local = parent_bind.inverse() * bind_matrix;
                let (scale, rotation, translation) = local.to_scale_rotation_translation();
                LocalBoneTransform {
                    translation,
                    rotation,
                    scale,
                }
            })
            .collect();

        let palette = vec![Mat4::IDENTITY; bone_count];

        let mut instance = Self {
            bones,
            local_transforms,
            palette,
            dirty: true,
            debug_draw_visible: false,
        };
        instance.compute_palette();
        instance
    }

    /// Recompute world-space bone palette via forward kinematics.
    ///
    /// Traverses bones in index order (parent-before-child guarantee
    /// from the importer). Each bone's world transform is:
    ///   `world[i] = world[parent[i]] * local[i]`
    /// Then the palette entry is:
    ///   `palette[i] = world[i] * inverse_bind[i]`
    pub fn compute_palette(&mut self) {
        let bone_count = self.bones.len();
        // Temp storage for world-space transforms
        let mut world_transforms = vec![Mat4::IDENTITY; bone_count];

        for i in 0..bone_count {
            let local_mat = self.local_transforms[i].to_matrix();
            let parent_world = self.bones[i]
                .parent_index
                .map(|p| world_transforms[p])
                .unwrap_or(Mat4::IDENTITY);
            world_transforms[i] = parent_world * local_mat;
            self.palette[i] = world_transforms[i] * self.bones[i].inverse_bind_matrix;
        }

        self.dirty = true;
    }
}

/// Playback state of an animation player.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PlaybackState {
    /// Not playing — holds at current time.
    Stopped,
    /// Actively advancing time.
    Playing,
}

impl Default for PlaybackState {
    fn default() -> Self {
        Self::Stopped
    }
}

/// Active crossfade state: blends from a snapshot of previous transforms
/// into the current clip over a duration.
pub struct CrossfadeState {
    /// Snapshot of bone local transforms at the time of crossfade start.
    pub from_transforms: Vec<LocalBoneTransform>,
    /// Total crossfade duration in seconds.
    pub duration: f32,
    /// Elapsed time since crossfade started.
    pub elapsed: f32,
}

impl CrossfadeState {
    /// Blend weight for the new clip (0→1 over duration).
    pub fn blend_weight(&self) -> f32 {
        if self.duration <= 0.0 {
            1.0
        } else {
            (self.elapsed / self.duration).clamp(0.0, 1.0)
        }
    }

    /// Whether the crossfade is complete.
    pub fn is_done(&self) -> bool {
        self.elapsed >= self.duration
    }
}

/// Animation player component — drives playback of a `RawAnimationClip`.
///
/// Attach to the same entity as `SkeletonInstance`. The
/// `AnimationUpdateSystem` reads the player, samples keyframes, and
/// writes into the skeleton's local transforms.
pub struct AnimationPlayer {
    /// The animation clip being played.
    pub clip: RawAnimationClip,
    /// Current playback time in seconds.
    pub time: f32,
    /// Playback speed multiplier (1.0 = normal).
    pub speed: f32,
    /// Whether to loop when reaching the end.
    pub looping: bool,
    /// Current playback state.
    pub state: PlaybackState,
    /// Active crossfade from a previous pose (if any).
    pub crossfade: Option<CrossfadeState>,
}

impl AnimationPlayer {
    /// Create a new player for the given clip (starts stopped at t=0).
    pub fn new(clip: RawAnimationClip) -> Self {
        Self {
            clip,
            time: 0.0,
            speed: 1.0,
            looping: true,
            state: PlaybackState::Stopped,
            crossfade: None,
        }
    }

    /// Transition to a new clip with a crossfade from the current pose.
    ///
    /// Snapshots the skeleton's current local transforms and blends
    /// from them into the new clip over `duration` seconds.
    pub fn crossfade_to(
        &mut self,
        new_clip: RawAnimationClip,
        duration: f32,
        current_transforms: &[LocalBoneTransform],
    ) {
        self.crossfade = Some(CrossfadeState {
            from_transforms: current_transforms.to_vec(),
            duration,
            elapsed: 0.0,
        });
        self.clip = new_clip;
        self.time = 0.0;
        self.state = PlaybackState::Playing;
    }

    /// Start or resume playback.
    pub fn play(&mut self) {
        self.state = PlaybackState::Playing;
    }

    /// Stop playback (holds current pose).
    pub fn stop(&mut self) {
        self.state = PlaybackState::Stopped;
    }

    /// Reset to the beginning.
    pub fn reset(&mut self) {
        self.time = 0.0;
    }

    /// Advance playback time by `dt` seconds. Handles looping.
    pub fn advance(&mut self, dt: f32) {
        if self.state != PlaybackState::Playing {
            return;
        }
        self.time += dt * self.speed;
        let duration = self.clip.duration_seconds;
        if duration > 0.0 {
            if self.looping {
                self.time = self.time.rem_euclid(duration);
            } else if self.time >= duration {
                self.time = duration;
                self.state = PlaybackState::Stopped;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blend_halfway() {
        let a = LocalBoneTransform {
            translation: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        };
        let b = LocalBoneTransform {
            translation: Vec3::new(10.0, 0.0, 0.0),
            rotation: Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
            scale: Vec3::splat(2.0),
        };
        let mid = a.blend(&b, 0.5);
        assert!((mid.translation.x - 5.0).abs() < 1e-4);
        assert!((mid.scale.x - 1.5).abs() < 1e-4);
    }

    #[test]
    fn blend_at_zero_returns_self() {
        let a = LocalBoneTransform {
            translation: Vec3::new(1.0, 2.0, 3.0),
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        };
        let b = LocalBoneTransform::default();
        let result = a.blend(&b, 0.0);
        assert!((result.translation - a.translation).length() < 1e-6);
    }

    #[test]
    fn crossfade_weight_progression() {
        let fade = CrossfadeState {
            from_transforms: vec![],
            duration: 0.5,
            elapsed: 0.25,
        };
        assert!((fade.blend_weight() - 0.5).abs() < 1e-6);
        assert!(!fade.is_done());
    }

    #[test]
    fn crossfade_done_when_elapsed_exceeds_duration() {
        let fade = CrossfadeState {
            from_transforms: vec![],
            duration: 0.5,
            elapsed: 0.6,
        };
        assert!((fade.blend_weight() - 1.0).abs() < 1e-6);
        assert!(fade.is_done());
    }
}
