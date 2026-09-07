//! Task 41.6 D9: footfall events by tool, not by hand. Samples a clip's foot
//! bones through the real FK path, finds the plants (local minima of the
//! model-space foot height) and writes `<chain>_down` / `<chain>_up` markers
//! back to the `.anim` — the events `FootPlacementSystem` locks on
//! (`foot_placement.rs`). Deterministic and re-runnable: the `#[ignore]`
//! test `author_footfall_events` is the tool; the Anim Events dialog can
//! still adjust the result.

use std::path::Path;

use crate::engine::animation::components::SkeletonInstance;
use crate::engine::animation::graph::ClipSet;
use crate::engine::animation::sampling::sample_channels;
use crate::engine::assets::mesh_import::{load_anim_binary, load_mesh_binary, write_anim_binary};
use crate::engine::assets::model_loader::AnimEventMarker;

/// Sampling rate of the height tracks.
pub const SAMPLE_RATE: f32 = 120.0;
/// Two plants closer than this are one (the earlier one — contact start — wins).
pub const MIN_GAP: f32 = 0.15;
/// `_up` sits this far along the interval from a plant to the next.
pub const UP_FRACTION: f32 = 0.4;
/// A minimum above this fraction of the track's height range is a swing
/// wobble, not a plant.
const CONTACT_BAND: f32 = 0.3;

/// The demo's chains and the Mixamo bones they end in.
pub const MIXAMO_FEET: [(&str, &str); 2] = [
    ("foot_l", "mixamorig:LeftFoot"),
    ("foot_r", "mixamorig:RightFoot"),
];

/// Plant times of a looping height track sampled at `rate` Hz: circular
/// local minima inside the contact band, thinned so no two lie closer than
/// `min_gap` (across the loop seam too).
pub fn contact_times(heights: &[f32], rate: f32, min_gap: f32) -> Vec<f32> {
    let n = heights.len();
    if n < 3 {
        return Vec::new();
    }
    let lo = heights.iter().copied().fold(f32::INFINITY, f32::min);
    let hi = heights.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let band = lo + (hi - lo) * CONTACT_BAND;
    let mut kept: Vec<f32> = Vec::new();
    for i in 0..n {
        let (prev, next) = (heights[(i + n - 1) % n], heights[(i + 1) % n]);
        let h = heights[i];
        if !(h < prev && h <= next && h <= band) {
            continue;
        }
        let t = i as f32 / rate;
        if kept.last().is_some_and(|&k| t - k < min_gap) {
            continue;
        }
        kept.push(t);
    }
    let duration = n as f32 / rate;
    if kept.len() > 1 && duration - kept[kept.len() - 1] + kept[0] < min_gap {
        kept.pop();
    }
    kept
}

/// `<chain>_down` at each plant, `<chain>_up` at the plant plus
/// [`UP_FRACTION`] of the (circular) interval to the next plant.
pub fn footfall_markers(chain: &str, plants: &[f32], duration: f32) -> Vec<AnimEventMarker> {
    let mut out = Vec::with_capacity(plants.len() * 2);
    for (i, &t) in plants.iter().enumerate() {
        let next = plants.get(i + 1).copied().unwrap_or(plants[0] + duration);
        out.push(AnimEventMarker {
            time_seconds: t,
            name: format!("{chain}_down"),
        });
        out.push(AnimEventMarker {
            time_seconds: (t + (next - t) * UP_FRACTION) % duration,
            name: format!("{chain}_up"),
        });
    }
    out
}

/// Author `foot_*` markers on every clip of `anim_path` against the
/// skeleton of `mesh_path`: sample each clip at [`SAMPLE_RATE`] through
/// `SkeletonInstance` FK (clips armed by name, D7), take the foot bones'
/// model-space height along the skeleton's up axis, replace any existing
/// `foot_*` markers and write the file back. Returns one report line per
/// clip and chain.
pub fn author_footfall_events(
    anim_path: &Path,
    mesh_path: &Path,
    feet: &[(&str, &str)],
) -> Result<Vec<String>, String> {
    let (bone_names, mut clips) =
        load_anim_binary(anim_path).map_err(|e| format!("{}: {e}", anim_path.display()))?;
    let bones = load_mesh_binary(mesh_path)
        .map_err(|e| format!("{}: {e}", mesh_path.display()))?
        .bones;
    let index_of = |name: &str| bones.iter().position(|b| b.name == name);
    let feet: Vec<(&str, usize)> = feet
        .iter()
        .map(|(chain, bone)| {
            index_of(bone)
                .map(|i| (*chain, i))
                .ok_or_else(|| format!("{}: no bone '{bone}'", mesh_path.display()))
        })
        .collect::<Result<_, _>>()?;

    // The clips as the runner would sample them against this skeleton.
    let set = ClipSet {
        bone_names: bone_names.clone(),
        clips: clips.clone(),
    };
    let armed = match set.armed_for(&bones) {
        Some((armed, dropped)) => {
            if !dropped.is_empty() {
                eprintln!("footfall: dropped channels for missing bones {}", dropped.join(", "));
            }
            armed.clips
        }
        None => set.clips,
    };

    let spine = (index_of("mixamorig:Hips"), index_of("mixamorig:Head"));
    let mut skel = SkeletonInstance::from_bones(bones);
    // The skeleton's up axis (mesh-local Y-up by convention — verified from
    // the bind pose: the axis the spine runs along).
    let up = match spine {
        (Some(hips), Some(head)) => {
            let d = (skel.model_space[head].w_axis - skel.model_space[hips].w_axis).abs();
            if d.y >= d.x && d.y >= d.z {
                1
            } else if d.z >= d.x {
                2
            } else {
                0
            }
        }
        _ => 1,
    };

    let mut report = vec![format!(
        "{}: skeleton up axis {} ({} bones)",
        anim_path.display(),
        ["X", "Y", "Z"][up],
        skel.bones.len()
    )];
    for (clip, armed) in clips.iter_mut().zip(&armed) {
        let samples = ((clip.duration_seconds * SAMPLE_RATE).floor() as usize).max(1);
        let mut heights: Vec<Vec<f32>> = vec![Vec::with_capacity(samples); feet.len()];
        for i in 0..samples {
            sample_channels(&armed.channels, i as f32 / SAMPLE_RATE, &mut skel.local_transforms);
            skel.compute_model_space();
            for (k, (_, bone)) in feet.iter().enumerate() {
                heights[k].push(skel.model_space[*bone].w_axis[up]);
            }
        }
        clip.events.retain(|e| !e.name.starts_with("foot_"));
        for (k, (chain, _)) in feet.iter().enumerate() {
            let plants = contact_times(&heights[k], SAMPLE_RATE, MIN_GAP);
            let markers = footfall_markers(chain, &plants, clip.duration_seconds);
            report.push(format!(
                "  '{}' {chain}: {}",
                clip.name,
                if markers.is_empty() {
                    "no plants found".to_string()
                } else {
                    markers
                        .iter()
                        .map(|m| format!("{} @ {:.3}", m.name, m.time_seconds))
                        .collect::<Vec<_>>()
                        .join(", ")
                }
            ));
            clip.events.extend(markers);
        }
        clip.events
            .sort_by(|a, b| a.time_seconds.total_cmp(&b.time_seconds));
    }
    write_anim_binary(anim_path, &clips, &bone_names)
        .map_err(|e| format!("{}: {e}", anim_path.display()))?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(f: impl Fn(f32) -> f32, duration: f32) -> Vec<f32> {
        (0..(duration * SAMPLE_RATE) as usize)
            .map(|i| f(i as f32 / SAMPLE_RATE))
            .collect()
    }

    /// Two strides per second: plants at the cosine minima, `_up` 40 % of
    /// the way to the next plant, the loop seam counted as an interval.
    #[test]
    fn plants_are_the_low_minima_and_up_sits_at_forty_percent() {
        let h = track(|t| 1.0 + (t * std::f32::consts::TAU * 2.0).cos(), 1.0);
        let plants = contact_times(&h, SAMPLE_RATE, MIN_GAP);
        assert_eq!(plants, vec![0.25, 0.75]);
        let m = footfall_markers("foot_l", &plants, 1.0);
        let got: Vec<(&str, f32)> = m.iter().map(|e| (e.name.as_str(), e.time_seconds)).collect();
        assert_eq!(
            got,
            vec![("foot_l_down", 0.25), ("foot_l_up", 0.45), ("foot_l_down", 0.75), ("foot_l_up", 0.95)]
        );
    }

    /// A flat plant (plateau) is one contact at its start; a shallow dip in
    /// the swing is not a plant; two dips within the gap are one.
    #[test]
    fn plateaus_swing_wobble_and_near_duplicates_do_not_multiply_plants() {
        let mut h = track(|t| 1.0 + (t * std::f32::consts::TAU).cos(), 1.0);
        for (i, v) in h.iter_mut().enumerate() {
            let t = i as f32 / SAMPLE_RATE;
            if (0.45..0.52).contains(&t) {
                *v = 0.0; // plateau on the ground
            }
            if (0.05..0.1).contains(&t) {
                *v = 1.9; // a wobble near the top of the swing
            }
            if (0.55..0.58).contains(&t) {
                *v = 0.05; // a second dip within the gap of the plant's start
            }
        }
        assert_eq!(contact_times(&h, SAMPLE_RATE, MIN_GAP), vec![0.45]);
    }

    #[test]
    fn short_or_flat_tracks_have_no_plants() {
        assert!(contact_times(&[1.0, 1.0], SAMPLE_RATE, MIN_GAP).is_empty());
        assert!(contact_times(&[1.0; 240], SAMPLE_RATE, MIN_GAP).is_empty());
    }

    /// The tool (D9). Rewrites `content/anims/{Walking,Running}.anim` against
    /// `content/Defeated.mesh`; missing clips are skipped with a message.
    /// Run once with `--ignored --nocapture` after importing the clips.
    #[test]
    #[ignore = "authoring tool: rewrites content/anims/*.anim"]
    fn author_footfall_events() {
        let content = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("content");
        let mesh = content.join("Defeated.mesh");
        for name in ["Walking", "Running"] {
            let anim = content.join(format!("anims/{name}.anim"));
            if !anim.exists() {
                println!("footfall: skipping {} (not imported yet)", anim.display());
                continue;
            }
            match super::author_footfall_events(&anim, &mesh, &MIXAMO_FEET) {
                Ok(lines) => lines.iter().for_each(|l| println!("footfall: {l}")),
                Err(e) => panic!("{e}"),
            }
        }
    }
}
