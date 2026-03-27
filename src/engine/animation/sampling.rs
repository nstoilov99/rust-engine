//! Keyframe sampling with binary search + interpolation.

use glam::{Quat, Vec3};

use crate::engine::animation::components::LocalBoneTransform;
use crate::engine::assets::model_loader::AnimationChannel;

/// Sample all channels of an animation at the given time, writing
/// results into the provided local transforms slice.
pub fn sample_channels(
    channels: &[AnimationChannel],
    time: f32,
    local_transforms: &mut [LocalBoneTransform],
) {
    for channel in channels {
        let bone_idx = channel.bone_index;
        if bone_idx >= local_transforms.len() {
            continue;
        }

        if !channel.position_keys.is_empty() {
            local_transforms[bone_idx].translation =
                sample_vec3_keys(&channel.position_keys, time);
        }
        if !channel.rotation_keys.is_empty() {
            local_transforms[bone_idx].rotation =
                sample_quat_keys(&channel.rotation_keys, time);
        }
        if !channel.scale_keys.is_empty() {
            local_transforms[bone_idx].scale = sample_vec3_keys(&channel.scale_keys, time);
        }
    }
}

/// Sample a Vec3 keyframe track at the given time using binary search + lerp.
pub fn sample_vec3_keys(keys: &[(f32, Vec3)], time: f32) -> Vec3 {
    debug_assert!(!keys.is_empty());

    if keys.len() == 1 || time <= keys[0].0 {
        return keys[0].1;
    }
    if time >= keys[keys.len() - 1].0 {
        return keys[keys.len() - 1].1;
    }

    // Binary search for the interval containing `time`
    let idx = match keys.binary_search_by(|k| k.0.partial_cmp(&time).unwrap()) {
        Ok(i) => return keys[i].1, // Exact match
        Err(i) => i,               // time is between keys[i-1] and keys[i]
    };

    let (t0, v0) = keys[idx - 1];
    let (t1, v1) = keys[idx];
    let dt = t1 - t0;
    if dt <= 0.0 {
        return v0;
    }
    let alpha = (time - t0) / dt;
    v0.lerp(v1, alpha)
}

/// Sample a Quat keyframe track at the given time using binary search + slerp.
pub fn sample_quat_keys(keys: &[(f32, Quat)], time: f32) -> Quat {
    debug_assert!(!keys.is_empty());

    if keys.len() == 1 || time <= keys[0].0 {
        return keys[0].1;
    }
    if time >= keys[keys.len() - 1].0 {
        return keys[keys.len() - 1].1;
    }

    let idx = match keys.binary_search_by(|k| k.0.partial_cmp(&time).unwrap()) {
        Ok(i) => return keys[i].1,
        Err(i) => i,
    };

    let (t0, q0) = keys[idx - 1];
    let (t1, q1) = keys[idx];
    let dt = t1 - t0;
    if dt <= 0.0 {
        return q0;
    }
    let alpha = (time - t0) / dt;
    q0.slerp(q1, alpha)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_key_returns_value() {
        let keys = vec![(0.0, Vec3::new(1.0, 2.0, 3.0))];
        assert_eq!(sample_vec3_keys(&keys, 0.0), Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(sample_vec3_keys(&keys, 5.0), Vec3::new(1.0, 2.0, 3.0));
    }

    #[test]
    fn lerp_midpoint() {
        let keys = vec![
            (0.0, Vec3::ZERO),
            (1.0, Vec3::new(10.0, 0.0, 0.0)),
        ];
        let v = sample_vec3_keys(&keys, 0.5);
        assert!((v.x - 5.0).abs() < 1e-5);
    }

    #[test]
    fn clamp_before_first_key() {
        let keys = vec![
            (1.0, Vec3::new(1.0, 0.0, 0.0)),
            (2.0, Vec3::new(2.0, 0.0, 0.0)),
        ];
        assert_eq!(sample_vec3_keys(&keys, 0.0), Vec3::new(1.0, 0.0, 0.0));
    }

    #[test]
    fn clamp_after_last_key() {
        let keys = vec![
            (0.0, Vec3::new(1.0, 0.0, 0.0)),
            (1.0, Vec3::new(2.0, 0.0, 0.0)),
        ];
        assert_eq!(sample_vec3_keys(&keys, 5.0), Vec3::new(2.0, 0.0, 0.0));
    }

    #[test]
    fn slerp_identity_to_90() {
        let q0 = Quat::IDENTITY;
        let q1 = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        let keys = vec![(0.0, q0), (1.0, q1)];
        let q = sample_quat_keys(&keys, 0.5);
        let expected = q0.slerp(q1, 0.5);
        assert!((q.x - expected.x).abs() < 1e-5);
        assert!((q.y - expected.y).abs() < 1e-5);
        assert!((q.z - expected.z).abs() < 1e-5);
        assert!((q.w - expected.w).abs() < 1e-5);
    }

    #[test]
    fn sample_channels_updates_transforms() {
        use crate::engine::assets::model_loader::AnimationChannel;

        let channels = vec![AnimationChannel {
            bone_index: 0,
            position_keys: vec![
                (0.0, Vec3::ZERO),
                (1.0, Vec3::new(10.0, 0.0, 0.0)),
            ],
            rotation_keys: vec![],
            scale_keys: vec![],
        }];

        let mut transforms = vec![LocalBoneTransform::default(); 2];
        sample_channels(&channels, 0.5, &mut transforms);
        assert!((transforms[0].translation.x - 5.0).abs() < 1e-5);
        // Bone 1 should be untouched
        assert_eq!(transforms[1].translation, Vec3::ZERO);
    }
}
