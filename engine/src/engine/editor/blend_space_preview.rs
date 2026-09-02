//! The blend space tab's embedded 3D preview — the CPU half (Task 41.5
//! ticket 08).
//!
//! Evaluates the open document on a skeleton of its own, ECS-free: a
//! one-state plan (ENTRY → the space) runs through the same [`AnimMachine`]
//! + [`evaluate_pose`] the runtime uses, so sync groups, rate scales and
//! input smoothing behave exactly as in play. This module owns the
//! skeleton, the clip cache, the clock and the pose; the host owns the GPU
//! side (a [`MeshPreviewState`] render target it fills in `gpu`, and the
//! per-frame palette upload).
//!
//! Bone mapping follows the runtime's rule: clip channels index the sibling
//! mesh's bones directly, so a mesh "matches" a clip when it has every bone
//! the clip names ([`bones_cover`]).

use std::collections::HashMap;
use std::sync::Arc;

use super::mesh_editor::MeshPreviewState;
use crate::engine::animation::blend_space::{BlendSpace, BlendSpaceDoc};
use crate::engine::animation::components::SkeletonInstance;
use crate::engine::animation::graph::{
    evaluate_pose, AnimAssetLoader, AnimGraphPlan, AnimMachine, AnimParamType, AnimParams,
    ClipSet, ParamDecl, ParamValue, PlanClip, PlanSpace, PlanState, PlanTree, PoseScratch,
    PoseSource,
};

/// Default share of the right-hand column the preview pane takes.
pub const DEFAULT_SPLIT: f32 = 0.55;
/// Minimum height of either pane at UI scale 1.0.
pub const MIN_PANE: f32 = 160.0;

/// Clamp the preview-pane fraction so both panes keep `min_px` of
/// `total_px`; halfway when the column cannot fit both minimums.
pub fn clamp_split(frac: f32, total_px: f32, min_px: f32) -> f32 {
    if total_px <= 2.0 * min_px {
        return 0.5;
    }
    let lo = min_px / total_px;
    frac.clamp(lo, 1.0 - lo)
}

/// `true` when the mesh has every bone the clip names (and the clip names
/// any at all).
pub fn bones_cover(mesh_bones: &[String], clip_bones: &[String]) -> bool {
    !clip_bones.is_empty() && clip_bones.iter().all(|b| mesh_bones.contains(b))
}

/// The input the preview evaluates at: the preview point, else the axis
/// minimums.
pub fn preview_input(doc: &BlendSpaceDoc, point: Option<[f32; 2]>) -> [f32; 2] {
    point.unwrap_or_else(|| {
        let mut v = [0.0; 2];
        for (k, a) in doc.active_axes().iter().enumerate() {
            v[k] = a.min.min(a.max);
        }
        v
    })
}

/// File stem of a content-relative path, for labels.
pub fn stem(path: &str) -> String {
    std::path::Path::new(path)
        .file_stem()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string())
}

pub struct BlendSpacePreview {
    /// Preview pane share of the right-hand column (session state).
    pub split: f32,
    /// A dragged splitter in flight.
    pub split_drag: bool,
    pub playing: bool,
    /// The mesh being previewed — the document's choice or the auto-pick;
    /// `None` = nothing matched.
    pub mesh: Option<String>,
    /// Why the pane shows a message instead of the mesh.
    pub status: Option<String>,
    pub skeleton: Option<SkeletonInstance>,
    /// Host-owned render target for `gpu_mesh`; recreated when the resolved
    /// mesh changes.
    pub gpu: Option<MeshPreviewState>,
    pub gpu_mesh: Option<String>,
    /// Clip containers by path; `None` = failed to load.
    clips: HashMap<String, Option<ClipSet>>,
    /// Bone names per `.mesh` (auto-pick cache; empty = no skeleton).
    mesh_bones: HashMap<String, Vec<String>>,
    plan: Arc<AnimGraphPlan>,
    machine: AnimMachine,
    params: AnimParams,
    scratch: PoseScratch,
}

impl Default for BlendSpacePreview {
    fn default() -> Self {
        let plan = Arc::new(AnimGraphPlan::default());
        Self {
            split: DEFAULT_SPLIT,
            split_drag: false,
            playing: true,
            mesh: None,
            status: None,
            skeleton: None,
            gpu: None,
            gpu_mesh: None,
            clips: HashMap::new(),
            mesh_bones: HashMap::new(),
            machine: AnimMachine::new(&plan),
            plan,
            params: AnimParams::default(),
            scratch: PoseScratch::new(),
        }
    }
}

impl BlendSpacePreview {
    /// Seconds into the preview's clock.
    pub fn time(&self) -> f32 {
        self.machine.time()
    }

    /// Forget loaded clips (an `.anim` changed on disk); the next rebuild
    /// reloads them.
    pub fn forget_clips(&mut self) {
        self.clips.clear();
    }

    /// Re-resolve the mesh, the skeleton and the plan against the current
    /// document. Called whenever the document (or its compiled space)
    /// changed; the machine's clock and smoothing memory survive.
    pub fn rebuild(
        &mut self,
        doc: &BlendSpaceDoc,
        compiled: &Result<BlendSpace, String>,
        mesh_assets: &[String],
        loader: &dyn AnimAssetLoader,
    ) {
        for s in &doc.samples {
            // Cached under both spellings so the plan's normalized path and
            // the document's own resolve to one load.
            let norm = crate::engine::scripting::normalize_graph_path(&s.clip);
            if !s.clip.is_empty() && !self.clips.contains_key(&norm) {
                let set = loader.load_clips(&norm);
                self.clips.insert(norm.clone(), set.clone());
                self.clips.insert(s.clip.clone(), set);
            }
        }
        let mesh = if doc.preview_mesh.is_empty() {
            self.auto_pick(doc, mesh_assets, loader)
        } else {
            Some(doc.preview_mesh.clone())
        };
        if mesh != self.mesh {
            self.mesh = mesh.clone();
            self.skeleton = mesh
                .as_deref()
                .and_then(|m| loader.load_skeleton(m))
                .filter(|b| !b.is_empty())
                .map(SkeletonInstance::from_bones);
        }
        let had_states = self.plan.states.len();
        self.plan = Arc::new(match compiled {
            Ok(space) => one_state_plan(doc, space),
            Err(_) => AnimGraphPlan::default(),
        });
        // The machine sizes its per-state memory from the plan it was built
        // against: a plan that gained or lost its state needs a fresh one.
        // Otherwise the running clock and smoothing carry across the edit.
        if self.plan.states.len() != had_states {
            self.machine = AnimMachine::new(&self.plan);
        }
        self.params = AnimParams::from_decls(&self.plan.parameters);
        self.status = self.diagnose(doc, compiled);
    }

    /// The first `.mesh` whose bones cover every bone the loadable sample
    /// clips name.
    fn auto_pick(
        &mut self,
        doc: &BlendSpaceDoc,
        mesh_assets: &[String],
        loader: &dyn AnimAssetLoader,
    ) -> Option<String> {
        let mut clip_bones: Vec<String> = doc
            .samples
            .iter()
            .filter_map(|s| self.clips.get(&s.clip)?.as_ref())
            .flat_map(|c| c.bone_names.iter().cloned())
            .collect();
        clip_bones.sort();
        clip_bones.dedup();
        if clip_bones.is_empty() {
            return None;
        }
        mesh_assets
            .iter()
            .find(|m| {
                let bones = self.mesh_bones.entry((*m).clone()).or_insert_with(|| {
                    loader
                        .load_skeleton(m)
                        .unwrap_or_default()
                        .into_iter()
                        .map(|b| b.name)
                        .collect()
                });
                bones_cover(bones, &clip_bones)
            })
            .cloned()
    }

    /// Why the pane cannot draw, in the order a user would fix things.
    fn diagnose(&self, doc: &BlendSpaceDoc, compiled: &Result<BlendSpace, String>) -> Option<String> {
        let Some(mesh) = &self.mesh else {
            return Some(if doc.preview_mesh.is_empty() {
                "No skinned mesh matches these clips \u{2014} set Preview Mesh".into()
            } else {
                "Set Preview Mesh".into()
            });
        };
        let Some(skel) = &self.skeleton else {
            return Some(format!("{} has no skeleton \u{2014} set Preview Mesh", stem(mesh)));
        };
        if let Err(e) = compiled {
            return Some(e.clone());
        }
        let mesh_bones: Vec<String> = skel.bones.iter().map(|b| b.name.clone()).collect();
        for (i, s) in doc.samples.iter().enumerate() {
            let Some(set) = self.clips.get(&s.clip).and_then(Option::as_ref) else {
                return Some(format!("sample {i}: clip '{}' could not be loaded", s.clip));
            };
            if set.select(s.clip_name.as_deref()).is_none() {
                return Some(format!("sample {i}: clip '{}' not in {}", s.clip_name.clone().unwrap_or_default(), s.clip));
            }
            if !bones_cover(&mesh_bones, &set.bone_names) {
                return Some(format!(
                    "{} bones don't match {}'s skeleton \u{2014} set Preview Mesh",
                    stem(&s.clip),
                    stem(mesh)
                ));
            }
        }
        None
    }

    /// Evaluate one frame at `input` (axis units): the clock advances by
    /// `dt` while playing; paused ticks with `dt = 0` — the runtime's own
    /// frozen frame, so the clip clock and any input smoothing hold while an
    /// unsmoothed point still snaps. Then the pose and palette refresh.
    pub fn advance(&mut self, dt: f32, input: [f32; 2]) {
        let Some(skel) = self.skeleton.as_mut() else { return };
        if self.status.is_some() {
            return;
        }
        for (k, decl) in self.plan.parameters.iter().enumerate() {
            self.params.set_float(&decl.slug, input[k.min(1)]);
        }
        self.machine
            .tick(&self.plan, &mut self.params, if self.playing { dt } else { 0.0 });
        let clips = &self.clips;
        let clip_for =
            |c: &PlanClip| clips.get(&c.clip)?.as_ref()?.select(c.clip_name.as_deref());
        evaluate_pose(
            &self.machine,
            &self.plan,
            &self.params,
            clip_for,
            &mut skel.local_transforms,
            &mut self.scratch,
        );
        skel.compute_palette();
    }
}

/// ENTRY → one state whose source is the compiled space: what the graph
/// compiler produces for a state naming this `.blendspace`, minus the
/// document.
fn one_state_plan(doc: &BlendSpaceDoc, space: &BlendSpace) -> AnimGraphPlan {
    let params: Vec<String> =
        doc.active_axes().iter().map(|a| a.param_name().to_string()).collect();
    let samples = space
        .samples()
        .iter()
        .map(|s| {
            // The graph compiler's normalization: forward slashes, an empty
            // clip name means "the first".
            let clip = crate::engine::scripting::normalize_graph_path(&s.clip);
            let clip_name = s.clip_name.clone().filter(|n| !n.trim().is_empty());
            (PlanClip { clip, clip_name }, s.rate_scale)
        })
        .collect();
    AnimGraphPlan {
        parameters: params
            .iter()
            .map(|slug| ParamDecl {
                slug: slug.clone(),
                ty: AnimParamType::Float,
                default: ParamValue::Float(0.0),
            })
            .collect(),
        states: vec![PlanState {
            node_id: 0,
            name: "Preview".into(),
            source: PoseSource::Tree(PlanTree::Space(PlanSpace {
                params,
                space: Arc::new(space.clone()),
                samples,
                smoothing: doc.input_smoothing.max(0.0),
            })),
            speed: 1.0,
        }],
        transitions: Vec::new(),
        entry: 0,
        slots: Vec::new(),
        ik_chains: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::animation::blend_space::{BlendAxis, BlendSample};
    use crate::engine::assets::model_loader::{AnimationChannel, BoneData, RawAnimationClip};
    use glam::{Mat4, Vec3};
    use node_graph_types::GraphDoc;

    fn bones(names: &[&str]) -> Vec<BoneData> {
        names
            .iter()
            .enumerate()
            .map(|(i, n)| BoneData {
                name: (*n).into(),
                parent_index: (i > 0).then_some(0),
                inverse_bind_matrix: Mat4::IDENTITY,
            })
            .collect()
    }

    /// Bone 0 slides from x=`from` at t=0 to x=`to` at t=1 (1 s cycle).
    fn clip(name: &str, from: f32, to: f32) -> RawAnimationClip {
        RawAnimationClip {
            name: name.into(),
            duration_seconds: 1.0,
            channels: vec![AnimationChannel {
                bone_index: 0,
                position_keys: vec![(0.0, Vec3::new(from, 0.0, 0.0)), (1.0, Vec3::new(to, 0.0, 0.0))],
                rotation_keys: vec![],
                scale_keys: vec![],
            }],
            events: vec![],
        }
    }

    struct Mem {
        meshes: Vec<(&'static str, Vec<&'static str>)>,
        clips: Vec<(&'static str, Vec<&'static str>, RawAnimationClip)>,
    }

    impl AnimAssetLoader for Mem {
        fn load_graph(&self, _: &str) -> Option<GraphDoc> {
            None
        }
        fn load_clips(&self, rel: &str) -> Option<ClipSet> {
            self.clips.iter().find(|(p, _, _)| *p == rel).map(|(_, b, c)| ClipSet {
                bone_names: b.iter().map(|s| s.to_string()).collect(),
                clips: vec![c.clone()],
            })
        }
        fn load_skeleton(&self, rel: &str) -> Option<Vec<BoneData>> {
            self.meshes.iter().find(|(p, _)| *p == rel).map(|(_, b)| bones(b))
        }
    }

    fn assets() -> Mem {
        Mem {
            meshes: vec![("a.mesh", vec!["root"]), ("b.mesh", vec!["root", "child"]), ("c.mesh", vec![])],
            clips: vec![
                ("walk.anim", vec!["root", "child"], clip("Walk", 2.0, 2.0)),
                ("run.anim", vec!["root", "child"], clip("Run", 10.0, 10.0)),
                ("slide.anim", vec!["root", "child"], clip("Slide", 0.0, 1.0)),
            ],
        }
    }

    fn mesh_list() -> Vec<String> {
        ["a.mesh", "b.mesh", "c.mesh"].iter().map(|s| s.to_string()).collect()
    }

    fn walk_run() -> BlendSpaceDoc {
        let mut doc = BlendSpaceDoc::default();
        doc.axes[0] = BlendAxis::new("Speed", 0.0, 6.0);
        doc.samples = vec![BlendSample::new(0.0, 0.0, "walk.anim"), BlendSample::new(6.0, 0.0, "run.anim")];
        doc
    }

    fn built(doc: &BlendSpaceDoc) -> BlendSpacePreview {
        let mut p = BlendSpacePreview::default();
        p.rebuild(doc, &BlendSpace::compile(doc), &mesh_list(), &assets());
        p
    }

    fn bone0_x(p: &BlendSpacePreview) -> f32 {
        p.skeleton.as_ref().expect("skeleton").local_transforms[0].translation.x
    }

    #[test]
    fn split_keeps_both_pane_minimums() {
        let near = |a: f32, b: f32| (a - b).abs() < 1e-5;
        assert!(near(clamp_split(0.55, 1000.0, 160.0), 0.55));
        assert!(near(clamp_split(0.01, 1000.0, 160.0), 0.16));
        assert!(near(clamp_split(0.99, 1000.0, 160.0), 0.84));
        assert!(near(clamp_split(0.9, 300.0, 160.0), 0.5), "too short for both: halfway");
    }

    #[test]
    fn auto_pick_takes_the_first_mesh_covering_the_clip_bones() {
        let p = built(&walk_run());
        assert_eq!(p.mesh.as_deref(), Some("b.mesh"), "a.mesh lacks 'child', c.mesh has no bones");
        assert_eq!(p.status, None);
        assert!(bones_cover(&["root".into(), "child".into()], &["child".into()]));
        assert!(!bones_cover(&["root".into()], &[]), "a clip naming no bones matches nothing");
    }

    #[test]
    fn a_chosen_mesh_wins_and_a_mismatch_is_explained() {
        let mut doc = walk_run();
        doc.preview_mesh = "a.mesh".into();
        let p = built(&doc);
        assert_eq!(p.mesh.as_deref(), Some("a.mesh"));
        assert!(p.status.as_deref().is_some_and(|s| s.contains("don't match")), "{:?}", p.status);
        doc.preview_mesh = "c.mesh".into();
        let p = built(&doc);
        assert!(p.status.as_deref().is_some_and(|s| s.contains("no skeleton")), "{:?}", p.status);
        doc.preview_mesh.clear();
        doc.samples[0].clip = "missing.anim".into();
        let p = built(&doc);
        assert!(p.status.as_deref().is_some_and(|s| s.contains("could not be loaded")), "{:?}", p.status);
    }

    #[test]
    fn the_pose_follows_the_preview_input() {
        let doc = walk_run();
        let mut p = built(&doc);
        for (point, expected) in [(None, 2.0), (Some([6.0, 0.0]), 10.0), (Some([3.0, 0.0]), 6.0), (Some([1.5, 0.0]), 4.0)] {
            p.advance(0.1, preview_input(&doc, point));
            let x = bone0_x(&p);
            assert!((x - expected).abs() < 1e-4, "{point:?}: pose {x} expected {expected}");
        }
    }

    #[test]
    fn rate_scale_and_pause_drive_the_clock() {
        let mut doc = walk_run();
        doc.samples = vec![BlendSample { rate_scale: 2.0, ..BlendSample::new(0.0, 0.0, "slide.anim") }];
        let mut p = built(&doc);
        p.advance(0.25, [0.0, 0.0]);
        assert!((bone0_x(&p) - 0.5).abs() < 1e-4, "rate 2: 0.25 s of clock is half the cycle");
        assert!((p.time() - 0.25).abs() < 1e-6);
        p.playing = false;
        p.advance(0.25, [0.0, 0.0]);
        assert!((bone0_x(&p) - 0.5).abs() < 1e-4, "paused: the frame holds");
        p.playing = true;
        // A doc edit rebuilds the plan but keeps the running clock.
        doc.samples[0].rate_scale = 1.0;
        p.rebuild(&doc, &BlendSpace::compile(&doc), &mesh_list(), &assets());
        p.advance(0.25, [0.0, 0.0]);
        assert!((bone0_x(&p) - 0.5).abs() < 1e-4, "0.5 s at rate 1");
    }

    #[test]
    fn preview_mesh_round_trips_and_defaults_empty() {
        use crate::engine::animation::blend_space::{parse_blend_space, serialize_blend_space};
        let mut doc = walk_run();
        doc.preview_mesh = "chars/hero.mesh".into();
        let text = serialize_blend_space(&doc).expect("serializes");
        assert_eq!(parse_blend_space(&text).expect("parses"), doc);
        let old = parse_blend_space("(version: 1, samples: [(x: 0.0, clip: \"a.anim\")])").expect("parses");
        assert_eq!(old.preview_mesh, "");
    }
}
