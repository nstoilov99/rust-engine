//! IK solvers (Task 41.5 P5, I-D2). Pure functions over model-space
//! matrices — no ECS, no assets, no `Resources`.
//!
//! **Space:** everything here is the mesh's local **Y-up model space** — the
//! retained `SkeletonInstance::model_space` (FK phase 1 output,
//! pre-inverse-bind). Callers convert world Z-up targets before calling
//! (I-D1: `target_model = entity_render⁻¹ * zup_to_yup(target_world)`).
//!
//! **Scale:** solvers operate on rotations and derived joint positions only.
//! Each output matrix is recomposed from the input's own decomposed scale, a
//! new rotation and a new translation, so bone scale rides through untouched
//! (the same TRS-decomposability assumption the SQT pose pipeline already
//! makes).

use glam::{Mat4, Quat, Vec3};

use crate::engine::animation::components::LocalBoneTransform;

const EPS: f32 = 1e-6;

/// `v` with its component along `axis` removed, normalized — `None` when the
/// rejection is too small to give a direction (v parallel to axis, or zero).
fn reject(v: Vec3, axis: Vec3) -> Option<Vec3> {
    let r = v - axis * v.dot(axis);
    (r.length_squared() > 1e-8).then(|| r.normalize())
}

/// Two-bone analytic IK with a **mandatory pole vector**.
///
/// `root`, `mid`, `tip` are the chain's current model-space matrices
/// (root→tip down one hierarchy path); `target` is where the tip should
/// land and `pole` disambiguates the bend plane (`BoneData` carries no
/// joint limits or preferred bend axes, so the pole is the v1
/// disambiguator). Returns the corrected `(root, mid)` matrices — the tip
/// (and anything below it) follows when the caller re-walks descendants
/// from the unchanged locals ([`rewalk_descendants`]).
///
/// Edge cases:
/// - **Target beyond reach** (`> l1+l2`): the chain straightens toward the
///   target, tip clamped to full extension on the target ray.
/// - **Target too close** (`< |l1−l2|`): maximum fold — the tip lands at
///   `|l1−l2|` along the target ray.
/// - **Target on the root**: no aim direction exists; the chain keeps its
///   current direction and folds along it.
/// - **Degenerate pole** (parallel to the root→target axis): falls back to
///   the chain's *current* bend plane (the animated mid projected off the
///   axis); a perfectly straight chain falls back to an arbitrary but
///   deterministic perpendicular (`Vec3::any_orthonormal_vector`).
/// - **Zero-length bone**: returned unchanged (nothing to solve).
pub fn solve_two_bone(
    root: Mat4,
    mid: Mat4,
    tip: Mat4,
    target: Vec3,
    pole: Vec3,
) -> (Mat4, Mat4) {
    let (root_s, root_r, a) = root.to_scale_rotation_translation();
    let (mid_s, mid_r, b) = mid.to_scale_rotation_translation();
    let c = tip.to_scale_rotation_translation().2;

    let l1 = (b - a).length();
    let l2 = (c - b).length();
    if l1 <= EPS || l2 <= EPS {
        return (root, mid);
    }

    // Aim axis: toward the target; a target sitting on the root keeps the
    // chain's current direction (there is nothing to aim at).
    let to_target = target - a;
    let n = to_target
        .try_normalize()
        .or_else(|| (c - a).try_normalize())
        .unwrap_or(Vec3::X);
    // Reach clamp (see the doc comment's edge cases).
    let d = to_target
        .length()
        .clamp((l1 - l2).abs(), l1 + l2)
        .max(EPS);
    // Bend plane: pole rejected off the aim axis, with the documented
    // fallbacks for a degenerate pole.
    let bend = reject(pole - a, n)
        .or_else(|| reject(b - a, n))
        .unwrap_or_else(|| n.any_orthonormal_vector());

    // Law of cosines at the root places the mid joint; the tip lands on the
    // target ray at the clamped distance.
    let cos_root = ((l1 * l1 + d * d - l2 * l2) / (2.0 * l1 * d)).clamp(-1.0, 1.0);
    let sin_root = (1.0 - cos_root * cos_root).max(0.0).sqrt();
    let b2 = a + (n * cos_root + bend * sin_root) * l1;
    let c2 = a + n * d;

    // Delta rotations: r1 swings the whole chain so the mid lands on b2;
    // r2 then swings the lower bone so the tip lands on c2. Composing the
    // deltas onto the animated rotations keeps roll and skinning behavior
    // as close to the animation as the reach allows.
    let r1 = Quat::from_rotation_arc((b - a) / l1, ((b2 - a) / l1).normalize());
    let r2 = Quat::from_rotation_arc(
        (r1 * ((c - b) / l2)).normalize(),
        ((c2 - b2) / l2).normalize(),
    );

    (
        Mat4::from_scale_rotation_translation(root_s, (r1 * root_r).normalize(), a),
        Mat4::from_scale_rotation_translation(mid_s, (r2 * r1 * mid_r).normalize(), b2),
    )
}

/// Look-at / aim with an angle clamp.
///
/// Rotates `bone` (model space) so that its local `forward_axis` points at
/// `target`, rotating no more than `max_angle` **radians** away from the
/// animated orientation (the shortest arc, clamped). Translation and scale
/// are preserved. A zero axis, a target on the bone, or an already-aligned
/// bone return the input unchanged.
pub fn solve_look_at(bone: Mat4, target: Vec3, forward_axis: Vec3, max_angle: f32) -> Mat4 {
    let (s, r, t) = bone.to_scale_rotation_translation();
    let Some(axis_local) = forward_axis.try_normalize() else {
        return bone;
    };
    let Some(to) = (target - t).try_normalize() else {
        return bone;
    };
    let full = Quat::from_rotation_arc((r * axis_local).normalize(), to);
    let (axis, angle) = full.to_axis_angle();
    if angle <= EPS {
        return bone;
    }
    let delta = Quat::from_axis_angle(axis, angle.min(max_angle.max(0.0)));
    Mat4::from_scale_rotation_translation(s, (delta * r).normalize(), t)
}

/// Weight-blend a solved model-space matrix against the animated one
/// (the P5 ruling): slerp on the decomposed rotation, lerp on the
/// translation, and the **animated scale kept** — IK never scales bones.
/// `w = 0` returns `from`, `w = 1` returns `to` (with `from`'s scale).
pub fn blend_model(from: &Mat4, to: &Mat4, w: f32) -> Mat4 {
    let (s, fr, ft) = from.to_scale_rotation_translation();
    let (_, tr, tt) = to.to_scale_rotation_translation();
    Mat4::from_scale_rotation_translation(s, fr.slerp(tr, w).normalize(), ft.lerp(tt, w))
}

/// Re-walk the strict descendants of the `edited` bones (P2 caveat: FK
/// phase 2 never auto-updates them): every bone below an edited one gets
/// `model_space[i] = model_space[parent] * local[i]` from the *unchanged*
/// animated locals; the edited bones themselves keep their solved matrices.
///
/// Preconditions: bones are index-ordered parent-before-child (the importer
/// guarantee `compute_model_space` already relies on). `touched` is caller
/// scratch, cleared and resized here — hand back the same Vec every frame
/// and the walk is allocation-free at steady state.
pub fn rewalk_descendants(
    model_space: &mut [Mat4],
    locals: &[LocalBoneTransform],
    parent_of: impl Fn(usize) -> Option<usize>,
    edited: &[usize],
    touched: &mut Vec<bool>,
) {
    touched.clear();
    touched.resize(model_space.len(), false);
    let mut start = model_space.len();
    for &e in edited {
        if e < touched.len() {
            touched[e] = true;
            start = start.min(e);
        }
    }
    for i in (start + 1)..model_space.len() {
        if touched[i] {
            continue; // an edited bone keeps its solved matrix
        }
        let Some(p) = parent_of(i) else { continue };
        if touched[p] {
            model_space[i] = model_space[p] * locals[i].to_matrix();
            touched[i] = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The acceptance chain, in Y-up mesh space: root at the origin, mid one
    // unit up, tip two units up — a straight l1 = l2 = 1 arm.
    fn chain() -> (Mat4, Mat4, Mat4) {
        (
            Mat4::IDENTITY,
            Mat4::from_translation(Vec3::new(0.0, 1.0, 0.0)),
            Mat4::from_translation(Vec3::new(0.0, 2.0, 0.0)),
        )
    }

    /// Where the tip lands after a solve — derived exactly as the runtime
    /// derives it: the tip's (unchanged) local re-walked under the new mid.
    fn tip_after(mid_before: Mat4, tip_before: Mat4, mid_after: Mat4) -> Vec3 {
        (mid_after * (mid_before.inverse() * tip_before))
            .to_scale_rotation_translation()
            .2
    }

    fn pos(m: Mat4) -> Vec3 {
        m.to_scale_rotation_translation().2
    }

    #[test]
    fn a_reachable_target_is_reached_exactly() {
        let (root, mid, tip) = chain();
        let target = Vec3::new(1.0, 1.0, 0.0); // |target| = √2 < 2
        let (root2, mid2) = solve_two_bone(root, mid, tip, target, Vec3::new(1.0, 0.0, 0.0));
        let t = tip_after(mid, tip, mid2);
        assert!((t - target).length() < 1e-4, "tip {t} vs target {target}");
        // Bone lengths hold.
        assert!(((pos(mid2) - pos(root2)).length() - 1.0).abs() < 1e-4);
        assert!(((t - pos(mid2)).length() - 1.0).abs() < 1e-4);
    }

    #[test]
    fn a_target_beyond_reach_straightens_toward_it_clamped() {
        let (root, mid, tip) = chain();
        let target = Vec3::new(3.0, 4.0, 0.0); // 5 units away, reach is 2
        let (_, mid2) = solve_two_bone(root, mid, tip, target, Vec3::new(1.0, 0.0, 0.0));
        let t = tip_after(mid, tip, mid2);
        let expected = target.normalize() * 2.0; // full extension on the ray
        assert!((t - expected).length() < 1e-4, "tip {t} vs {expected}");
    }

    #[test]
    fn a_target_too_close_holds_the_maximum_fold() {
        // l1 = 1, l2 = 0.5: the tip can never get closer than 0.5 to the root.
        let root = Mat4::IDENTITY;
        let mid = Mat4::from_translation(Vec3::new(0.0, 1.0, 0.0));
        let tip = Mat4::from_translation(Vec3::new(0.0, 1.5, 0.0));
        let target = Vec3::new(0.0, 0.1, 0.0);
        let (_, mid2) = solve_two_bone(root, mid, tip, target, Vec3::new(1.0, 0.0, 0.0));
        let t = tip_after(mid, tip, mid2);
        assert!((t - Vec3::new(0.0, 0.5, 0.0)).length() < 1e-4, "tip {t}");
    }

    #[test]
    fn the_pole_flips_the_bend_plane() {
        let (root, mid, tip) = chain();
        let target = Vec3::new(0.0, 1.8, 0.0);
        let (_, mid_px) = solve_two_bone(root, mid, tip, target, Vec3::new(5.0, 0.0, 0.0));
        let (_, mid_nx) = solve_two_bone(root, mid, tip, target, Vec3::new(-5.0, 0.0, 0.0));
        assert!(pos(mid_px).x > 0.1, "+X pole bends toward +X: {}", pos(mid_px));
        assert!(pos(mid_nx).x < -0.1, "-X pole bends toward -X: {}", pos(mid_nx));
        // Both still reach.
        assert!((tip_after(mid, tip, mid_px) - target).length() < 1e-4);
        assert!((tip_after(mid, tip, mid_nx) - target).length() < 1e-4);
    }

    #[test]
    fn a_degenerate_pole_falls_back_to_the_current_bend_plane() {
        // Chain pre-bent toward +X; the pole sits ON the root→target axis
        // (degenerate), so the solver must keep bending toward +X.
        let root = Mat4::IDENTITY;
        let mid = Mat4::from_translation(Vec3::new(0.3, 1.0, 0.0));
        let tip = Mat4::from_translation(Vec3::new(0.0, 2.0, 0.0));
        let target = Vec3::new(0.0, 1.8, 0.0);
        let (_, mid2) = solve_two_bone(root, mid, tip, target, Vec3::new(0.0, 7.0, 0.0));
        assert!(pos(mid2).x > 0.05, "kept the animated bend side: {}", pos(mid2));
        assert!((tip_after(mid, tip, mid2) - target).length() < 1e-3);
    }

    #[test]
    fn a_straight_chain_with_a_degenerate_pole_still_solves_finitely() {
        let (root, mid, tip) = chain();
        // Pole straight up the chain: doubly degenerate.
        let (root2, mid2) =
            solve_two_bone(root, mid, tip, Vec3::new(0.0, 1.5, 0.0), Vec3::new(0.0, 9.0, 0.0));
        let t = tip_after(mid, tip, mid2);
        assert!(t.is_finite() && pos(root2).is_finite());
        assert!((t - Vec3::new(0.0, 1.5, 0.0)).length() < 1e-3, "tip {t}");
    }

    #[test]
    fn bone_scale_survives_the_solve() {
        let root = Mat4::from_scale_rotation_translation(
            Vec3::splat(2.0),
            Quat::IDENTITY,
            Vec3::ZERO,
        );
        let mid = Mat4::from_translation(Vec3::new(0.0, 1.0, 0.0));
        let tip = Mat4::from_translation(Vec3::new(0.0, 2.0, 0.0));
        let (root2, mid2) =
            solve_two_bone(root, mid, tip, Vec3::new(1.0, 1.0, 0.0), Vec3::X);
        assert!((root2.to_scale_rotation_translation().0 - Vec3::splat(2.0)).length() < 1e-4);
        assert!((mid2.to_scale_rotation_translation().0 - Vec3::ONE).length() < 1e-4);
    }

    #[test]
    fn look_at_aims_the_forward_axis_when_within_the_clamp() {
        let bone = Mat4::IDENTITY;
        let solved = solve_look_at(
            bone,
            Vec3::new(1.0, 0.0, 1.0),
            Vec3::Z,
            std::f32::consts::FRAC_PI_2,
        );
        let fwd = solved.to_scale_rotation_translation().1 * Vec3::Z;
        let want = Vec3::new(1.0, 0.0, 1.0).normalize();
        assert!((fwd - want).length() < 1e-4, "forward {fwd} vs {want}");
    }

    #[test]
    fn look_at_clamps_at_max_angle() {
        let bone = Mat4::IDENTITY;
        let max = 1.0; // radians; the target sits a full π away
        let solved = solve_look_at(bone, Vec3::new(0.0, 0.0, -3.0), Vec3::Z, max);
        let fwd = solved.to_scale_rotation_translation().1 * Vec3::Z;
        let turned = fwd.dot(Vec3::Z).clamp(-1.0, 1.0).acos();
        assert!((turned - max).abs() < 1e-3, "turned {turned} rad, clamp {max}");
    }

    #[test]
    fn blend_model_holds_the_endpoints_and_keeps_the_animated_scale() {
        let from = Mat4::from_scale_rotation_translation(
            Vec3::splat(2.0),
            Quat::IDENTITY,
            Vec3::ZERO,
        );
        let to = Mat4::from_scale_rotation_translation(
            Vec3::ONE,
            Quat::from_rotation_z(1.0),
            Vec3::new(2.0, 0.0, 0.0),
        );
        let (s0, r0, t0) = blend_model(&from, &to, 0.0).to_scale_rotation_translation();
        assert!((s0 - Vec3::splat(2.0)).length() < 1e-4);
        assert!(r0.angle_between(Quat::IDENTITY) < 1e-4 && t0.length() < 1e-4);
        let (s1, r1, t1) = blend_model(&from, &to, 1.0).to_scale_rotation_translation();
        assert!((s1 - Vec3::splat(2.0)).length() < 1e-4, "animated scale kept at w=1");
        assert!(r1.angle_between(Quat::from_rotation_z(1.0)) < 1e-4);
        assert!((t1 - Vec3::new(2.0, 0.0, 0.0)).length() < 1e-4);
        let t_half = blend_model(&from, &to, 0.5).to_scale_rotation_translation().2;
        assert!((t_half - Vec3::new(1.0, 0.0, 0.0)).length() < 1e-4);
    }

    #[test]
    fn rewalk_updates_descendants_of_edited_bones_only() {
        // 0 → 1 → 2 → 3, one unit of +Y per local; bone 1 gets rotated 90°
        // about Z: 2 and 3 must follow, 0 must not move.
        let locals: Vec<LocalBoneTransform> = (0..4)
            .map(|i| LocalBoneTransform {
                translation: if i == 0 { Vec3::ZERO } else { Vec3::new(0.0, 1.0, 0.0) },
                ..Default::default()
            })
            .collect();
        let parents = [None, Some(0), Some(1), Some(2)];
        let mut model: Vec<Mat4> = vec![
            Mat4::IDENTITY,
            Mat4::from_translation(Vec3::new(0.0, 1.0, 0.0)),
            Mat4::from_translation(Vec3::new(0.0, 2.0, 0.0)),
            Mat4::from_translation(Vec3::new(0.0, 3.0, 0.0)),
        ];
        model[1] = Mat4::from_rotation_translation(
            Quat::from_rotation_z(-std::f32::consts::FRAC_PI_2),
            Vec3::new(0.0, 1.0, 0.0),
        );
        let mut touched = Vec::new();
        rewalk_descendants(&mut model, &locals, |i| parents[i], &[1], &mut touched);
        assert_eq!(pos(model[0]), Vec3::ZERO, "the un-edited root never moves");
        // +Y in bone 1's frame now points along +X.
        assert!((pos(model[2]) - Vec3::new(1.0, 1.0, 0.0)).length() < 1e-4, "{}", pos(model[2]));
        assert!((pos(model[3]) - Vec3::new(2.0, 1.0, 0.0)).length() < 1e-4, "{}", pos(model[3]));
    }
}
