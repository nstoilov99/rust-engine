//! The Anim Preview dock panel's CPU half (per-document layouts ticket 03):
//! the blend space tab's embedded preview ([`super::blend_space_preview`])
//! generalised to a *whole* animation graph.
//!
//! The focused `.animgraph` document compiles through the real compiler and
//! the disk loader (nested graphs and blend spaces resolve exactly as the
//! runtime's do), and runs through the same [`AnimMachine`] +
//! [`evaluate_pose`] an entity would — on a skeleton of the panel's own, so
//! the world is never touched. The panel owns the blackboard: the graph
//! tab's preview strip drives it when nothing in the world is bound. When
//! a world entity *is* bound the strip keeps driving that runtime and the
//! panel [`Mirror`]s it read-only (spec ruling on parameter ownership).
//!
//! The host owns the GPU side (a [`MeshPreviewState`] render target it fills
//! in `gpu`, the per-frame palette upload) and the compile cache key: it
//! passes the document's `revision`, and the plan rebuilds only when that
//! moved — a parameter write never restarts the machine.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use node_graph_types::GraphDoc;

use super::anim_preview::{preview_from_machine, AnimParamEdit, AnimPreview, PANEL_INSTANCE_ID};
use super::blend_space_preview::{bones_cover, stem};
use super::mesh_editor::MeshPreviewState;
use crate::engine::animation::components::SkeletonInstance;
use crate::engine::animation::graph::plan::preview_mesh_of;
use crate::engine::animation::graph::{
    compile_anim_graph_with, evaluate_pose, AnimAssetLoader, AnimGraphLoader, AnimGraphPlan,
    AnimMachine, AnimParamType, AnimParams, ClipSet, DiskAnimAssets, PlanClip, PoseScratch,
    PoseSource,
};

/// What the strip's chip calls the panel's own machine.
pub const PANEL_INSTANCE_NAME: &str = "Preview panel";

/// A bound world runtime, copied for one frame: the pane poses the preview
/// mesh with exactly what the entity shows, and ticks nothing of its own.
pub struct Mirror {
    /// The entity's display name, for the overlay.
    pub name: String,
    pub plan: Arc<AnimGraphPlan>,
    pub machine: AnimMachine,
    pub params: AnimParams,
}

pub struct AnimGraphPreview {
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
    /// Set by the host each frame a world entity is bound to this graph's
    /// strip; cleared when not. See [`Mirror`].
    pub mirror: Option<Mirror>,
    /// The pane drew this frame. The host consumes it: a preview nobody
    /// looks at keeps its clock but gives up its render target.
    pub shown: bool,
    /// Clip containers by normalized path; `None` = failed to load.
    clips: HashMap<String, Option<ClipSet>>,
    /// Bone names per `.mesh` (auto-pick cache; empty = no skeleton).
    mesh_bones: HashMap<String, Vec<String>>,
    plan: Arc<AnimGraphPlan>,
    /// The compiler's refusal for the current revision, if any.
    compile_error: Option<String>,
    /// The document named a mesh (as opposed to auto-pick) — for the message.
    mesh_chosen: bool,
    machine: AnimMachine,
    params: AnimParams,
    scratch: PoseScratch,
    compiled_revision: Option<u64>,
    /// The mirror plan `status` was last diagnosed against.
    diagnosed_mirror: Option<Arc<AnimGraphPlan>>,
    last_frame: Option<Instant>,
}

impl Default for AnimGraphPreview {
    fn default() -> Self {
        let plan = Arc::new(AnimGraphPlan::default());
        Self {
            playing: true,
            mesh: None,
            status: None,
            skeleton: None,
            gpu: None,
            gpu_mesh: None,
            mirror: None,
            shown: false,
            clips: HashMap::new(),
            mesh_bones: HashMap::new(),
            machine: AnimMachine::new(&plan),
            plan,
            compile_error: None,
            mesh_chosen: false,
            params: AnimParams::default(),
            scratch: PoseScratch::new(),
            compiled_revision: None,
            diagnosed_mirror: None,
            last_frame: None,
        }
    }
}

impl AnimGraphPreview {
    /// Seconds into the active state's clip (the mirrored runtime's while
    /// mirroring).
    pub fn time(&self) -> f32 {
        match &self.mirror {
            Some(m) => m.machine.time(),
            None => self.machine.time(),
        }
    }

    /// The compiled plan (empty while the document refuses).
    pub fn plan(&self) -> &Arc<AnimGraphPlan> {
        &self.plan
    }

    /// Forget loaded clips (an `.anim` changed on disk) and force a rebuild
    /// on the next tick.
    pub fn forget_clips(&mut self) {
        self.clips.clear();
        self.compiled_revision = None;
    }

    /// The strip's parameter edit, on the panel's own blackboard. `false` =
    /// refused (undeclared or mistyped against the current plan).
    pub fn apply(&mut self, edit: &AnimParamEdit) -> bool {
        edit.apply(&mut self.params)
    }

    /// The strip's read surface over the panel's machine. A document that
    /// does not compile hands back a *refused* preview carrying the reason:
    /// the chip says so and the controls hide, like a refused entity.
    pub fn snapshot(&self) -> AnimPreview {
        preview_from_machine(
            &self.plan,
            &self.machine,
            &self.params,
            self.compile_error.clone(),
            PANEL_INSTANCE_NAME.to_string(),
            PANEL_INSTANCE_ID,
        )
    }

    /// The overlay's state readout: the active state's name, `from → to`
    /// while a crossfade runs, and the nested machine's state after a slash
    /// when the active state is a sub-machine. `None` with nothing compiled.
    pub fn state_label(&self) -> Option<String> {
        let (plan, machine) = match &self.mirror {
            Some(m) => (&*m.plan, &m.machine),
            None => (&*self.plan, &self.machine),
        };
        state_label(plan, machine)
    }

    /// Per-frame entry: recompile when the document `revision` moved, then
    /// advance by wall-clock time (capped so a stall never jumps the pose).
    pub fn tick(&mut self, doc: &GraphDoc, path: &str, revision: u64, mesh_assets: &[String]) {
        let now = Instant::now();
        let dt = self
            .last_frame
            .map(|t| now.duration_since(t).as_secs_f32().min(0.1))
            .unwrap_or(0.0);
        self.last_frame = Some(now);
        let loader = DiskAnimAssets { content_root: "content".into() };
        self.tick_with(doc, path, revision, mesh_assets, &loader, &loader, dt);
    }

    /// [`Self::tick`] with explicit loaders and step (tests).
    #[allow(clippy::too_many_arguments)]
    pub fn tick_with(
        &mut self,
        doc: &GraphDoc,
        path: &str,
        revision: u64,
        mesh_assets: &[String],
        assets: &dyn AnimAssetLoader,
        graphs: &dyn AnimGraphLoader,
        dt: f32,
    ) {
        let rebuilt = self.compiled_revision != Some(revision);
        if rebuilt {
            self.rebuild(doc, path, mesh_assets, assets, graphs);
            self.compiled_revision = Some(revision);
        }
        // The diagnosis walks the plan's clip references; it only changes
        // with a rebuild or when the mirrored runtime (whose plan may sample
        // clips the document's does not — unsaved edits on either side)
        // comes, goes or swaps plans.
        let mirror_plan = self.mirror.as_ref().map(|m| m.plan.clone());
        let mirror_changed = match (&mirror_plan, &self.diagnosed_mirror) {
            (Some(a), Some(b)) => !Arc::ptr_eq(a, b),
            (None, None) => false,
            _ => true,
        };
        if rebuilt || mirror_changed {
            if let Some(plan) = &mirror_plan {
                self.ensure_clips(plan, assets);
            }
            self.status = self.diagnose();
            self.diagnosed_mirror = mirror_plan;
        }
        self.advance(dt);
    }

    /// Re-resolve the plan, the clips, the mesh and the skeleton against the
    /// document. An unchanged plan keeps the running machine and blackboard;
    /// a changed one restarts at ENTRY, carrying parameter values over by
    /// name so a toggled Bool survives the edit that added a transition. A
    /// refusal keeps everything from the last good compile and only records
    /// the message: a document is broken for a few edits at a time (a state
    /// dropped before it is wired), and the blackboard must not reset on
    /// each of them.
    pub fn rebuild(
        &mut self,
        doc: &GraphDoc,
        path: &str,
        mesh_assets: &[String],
        assets: &dyn AnimAssetLoader,
        graphs: &dyn AnimGraphLoader,
    ) {
        let plan = match compile_anim_graph_with(doc, path, graphs) {
            Ok(p) => Arc::new(p),
            Err(e) => {
                self.compile_error = Some(e);
                return;
            }
        };
        self.compile_error = None;
        self.ensure_clips(&plan, assets);
        let chosen = preview_mesh_of(doc);
        self.mesh_chosen = !chosen.is_empty();
        let mesh = if chosen.is_empty() {
            self.auto_pick(&plan, mesh_assets, assets)
        } else {
            Some(chosen)
        };
        if mesh != self.mesh {
            self.mesh = mesh.clone();
            self.skeleton = mesh
                .as_deref()
                .and_then(|m| assets.load_skeleton(m))
                .filter(|b| !b.is_empty())
                .map(SkeletonInstance::from_bones);
        }
        if *plan != *self.plan {
            let mut params = AnimParams::from_decls(&plan.parameters);
            for decl in &self.plan.parameters {
                match decl.ty {
                    AnimParamType::Float => {
                        if let Some(v) = self.params.get_float(&decl.slug) {
                            params.set_float(&decl.slug, v);
                        }
                    }
                    AnimParamType::Bool => {
                        if let Some(v) = self.params.get_bool(&decl.slug) {
                            params.set_bool(&decl.slug, v);
                        }
                    }
                    // A buffered one-shot does not outlive the machine it
                    // was fired at.
                    AnimParamType::Trigger => {}
                }
            }
            self.machine = AnimMachine::new(&plan);
            self.params = params;
            self.plan = plan;
        }
    }

    /// Load (once) every clip container the plan samples.
    fn ensure_clips(&mut self, plan: &AnimGraphPlan, assets: &dyn AnimAssetLoader) {
        for path in plan.clip_refs() {
            if !self.clips.contains_key(path) {
                let set = assets.load_clips(path);
                self.clips.insert(path.to_string(), set);
            }
        }
    }

    /// The first `.mesh` whose bones cover every bone the plan's loadable
    /// clips name.
    fn auto_pick(
        &mut self,
        plan: &AnimGraphPlan,
        mesh_assets: &[String],
        assets: &dyn AnimAssetLoader,
    ) -> Option<String> {
        let mut clip_bones: Vec<String> = plan
            .clip_refs()
            .into_iter()
            .filter_map(|p| self.clips.get(p)?.as_ref())
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
                    assets
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

    /// Why the pane cannot draw, in the order a user would fix things. A
    /// mirrored runtime runs its own (already compiled) plan, so the
    /// document's refusal does not stop the pane showing what the entity
    /// does; the mesh still has to fit.
    fn diagnose(&self) -> Option<String> {
        let plan: &AnimGraphPlan = match &self.mirror {
            Some(m) => &m.plan,
            None => {
                if let Some(e) = &self.compile_error {
                    return Some(e.clone());
                }
                &self.plan
            }
        };
        let Some(mesh) = &self.mesh else {
            return Some(if self.mesh_chosen {
                "Set Preview Mesh".into()
            } else {
                "No skinned mesh matches this graph's clips \u{2014} set Preview Mesh".into()
            });
        };
        let Some(skel) = &self.skeleton else {
            return Some(format!("{} has no skeleton \u{2014} set Preview Mesh", stem(mesh)));
        };
        if plan.states.is_empty() {
            return Some("Graph has no states".into());
        }
        let mesh_bones: Vec<String> = skel.bones.iter().map(|b| b.name.clone()).collect();
        let refs = plan
            .states
            .iter()
            .flat_map(|s| s.source.clips())
            .chain(plan.slots.iter().map(|s| &s.clip));
        for c in refs {
            let Some(set) = self.clips.get(&c.clip).and_then(Option::as_ref) else {
                return Some(format!("clip '{}' could not be loaded", c.clip));
            };
            if set.select(c.clip_name.as_deref()).is_none() {
                return Some(format!(
                    "clip '{}' not in {}",
                    c.clip_name.clone().unwrap_or_default(),
                    c.clip
                ));
            }
            if !bones_cover(&mesh_bones, &set.bone_names) {
                return Some(format!(
                    "{} bones don't match {}'s skeleton \u{2014} set Preview Mesh",
                    stem(&c.clip),
                    stem(mesh)
                ));
            }
        }
        None
    }

    /// Evaluate one frame: the panel's machine ticks by `dt` while playing
    /// (paused ticks with `dt = 0`, the runtime's own frozen frame), or the
    /// mirror poses as-is; then the pose and palette refresh.
    pub fn advance(&mut self, dt: f32) {
        let Some(skel) = self.skeleton.as_mut() else { return };
        if self.status.is_some() {
            return;
        }
        let clips = &self.clips;
        let clip_for =
            |c: &PlanClip| clips.get(&c.clip)?.as_ref()?.select(c.clip_name.as_deref());
        match &self.mirror {
            Some(m) => evaluate_pose(
                &m.machine,
                &m.plan,
                &m.params,
                clip_for,
                &mut skel.local_transforms,
                &mut self.scratch,
            ),
            None => {
                self.machine.tick(
                    &self.plan,
                    &mut self.params,
                    if self.playing { dt } else { 0.0 },
                );
                evaluate_pose(
                    &self.machine,
                    &self.plan,
                    &self.params,
                    clip_for,
                    &mut skel.local_transforms,
                    &mut self.scratch,
                );
            }
        }
        skel.compute_palette();
    }
}

/// `Idle`, `Idle → Walk` mid-fade, `Locomotion / Run` inside a nested state.
fn state_label(plan: &AnimGraphPlan, machine: &AnimMachine) -> Option<String> {
    let cur = machine.current_state();
    let name = |i: usize| plan.states.get(i).map(|s| s.name.clone());
    let mut label = match machine.crossfade() {
        Some(f) => format!("{} \u{2192} {}", name(f.from)?, name(cur)?),
        None => name(cur)?,
    };
    if let (Some(PoseSource::Machine { plan: child, .. }), Some(sub)) =
        (plan.states.get(cur).map(|s| &s.source), machine.sub(cur))
    {
        if let Some(inner) = state_label(child, sub) {
            label.push_str(" / ");
            label.push_str(&inner);
        }
    }
    Some(label)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::animation::graph::plan::{
        ANIM_ENTRY_TYPE_ID, ANIM_STATE_TYPE_ID, ANIM_TRANSITION_TYPE_ID, CLIP_PROP,
        DURATION_PROP, PREVIEW_MESH_PROP, PRIORITY_PROP, RULE_RESULT_PIN, STATE_IN_PIN,
        STATE_OUT_PIN, TRANSITION_FROM_PIN, TRANSITION_TO_PIN,
    };
    use node_graph_types::PropValue;
    use crate::engine::animation::graph::{anim_node_registry, ANIM_STATE_ALIAS_TYPE_ID};
    use crate::engine::assets::model_loader::{AnimationChannel, BoneData, RawAnimationClip};
    use crate::engine::editor::graph_editor::{GraphDomain, GraphEditorState};
    use glam::{Mat4, Vec3};
    use node_graph_types::{
        Edge, GraphDoc, GraphRealm, GraphRegion, NodeInst, PinType, VarDecl, VAR_GET_TYPE_ID,
        VAR_PROP, VAR_VALUE_PIN,
    };

    fn node(id: u64, type_id: &str, title: Option<&str>) -> NodeInst {
        NodeInst {
            id,
            type_id: type_id.into(),
            type_version: 1,
            position: [id as f32 * 200.0, 0.0],
            properties: Default::default(),
            subgraph: None,
            tint: None,
            title: title.map(str::to_string),
        }
    }

    fn with(id: u64, type_id: &str, title: Option<&str>, props: &[(&str, PropValue)]) -> NodeInst {
        let mut n = node(id, type_id, title);
        for (k, v) in props {
            n.properties.insert(k.to_string(), v.clone());
        }
        n
    }

    fn edge(from: u64, fp: &str, to: u64, tp: &str) -> Edge {
        Edge { from_node: from, from_pin: fp.into(), to_node: to, to_pin: tp.into() }
    }

    /// ENTRY → Idle; Idle → Walk on the Bool `walk`, 0.5 s crossfade.
    fn two_state_doc() -> GraphDoc {
        let mut doc = GraphDoc { realm: GraphRealm::Client, ..GraphDoc::default() };
        doc.variables = vec![VarDecl {
            slug: "walk".into(),
            label: "Walk".into(),
            ty: PinType::Bool,
            default: Some(PropValue::Bool(false)),
            group: None,
        }];
        doc.nodes = vec![
            node(1, ANIM_ENTRY_TYPE_ID, None),
            with(2, ANIM_STATE_TYPE_ID, Some("Idle"), &[(CLIP_PROP, PropValue::Asset("idle.anim".into()))]),
            with(3, ANIM_STATE_TYPE_ID, Some("Walk"), &[(CLIP_PROP, PropValue::Asset("walk.anim".into()))]),
            with(
                4,
                ANIM_TRANSITION_TYPE_ID,
                None,
                &[(DURATION_PROP, PropValue::Float(0.5)), (PRIORITY_PROP, PropValue::Int(0))],
            ),
        ];
        doc.edges = vec![
            edge(1, STATE_OUT_PIN, 2, STATE_IN_PIN),
            edge(2, STATE_OUT_PIN, 4, TRANSITION_FROM_PIN),
            edge(4, TRANSITION_TO_PIN, 3, STATE_IN_PIN),
        ];
        doc.regions.insert(
            4,
            GraphRegion {
                nodes: vec![
                    with(0, VAR_GET_TYPE_ID, None, &[(VAR_PROP, PropValue::Str("walk".into()))]),
                    node(1, crate::engine::animation::graph::plan::ANIM_RULE_RESULT_TYPE_ID, None),
                ],
                edges: vec![edge(0, VAR_VALUE_PIN, 1, RULE_RESULT_PIN)],
            },
        );
        doc
    }

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

    /// Bone 0 holds x = `x` for the whole 1 s cycle.
    fn clip(name: &str, x: f32) -> RawAnimationClip {
        RawAnimationClip {
            name: name.into(),
            duration_seconds: 1.0,
            channels: vec![AnimationChannel {
                bone_index: 0,
                position_keys: vec![(0.0, Vec3::new(x, 0.0, 0.0)), (1.0, Vec3::new(x, 0.0, 0.0))],
                rotation_keys: vec![],
                scale_keys: vec![],
            }],
            events: vec![],
        }
    }

    struct Mem;

    impl AnimAssetLoader for Mem {
        fn load_graph(&self, _: &str) -> Option<GraphDoc> {
            None
        }
        fn load_clips(&self, rel: &str) -> Option<ClipSet> {
            let (name, x) = match rel {
                "idle.anim" => ("Idle", 2.0),
                "walk.anim" => ("Walk", 10.0),
                _ => return None,
            };
            Some(ClipSet { bone_names: vec!["root".into(), "child".into()], clips: vec![clip(name, x)] })
        }
        fn load_skeleton(&self, rel: &str) -> Option<Vec<BoneData>> {
            match rel {
                "a.mesh" => Some(bones(&["root"])),
                "b.mesh" => Some(bones(&["root", "child"])),
                "c.mesh" => Some(vec![]),
                _ => None,
            }
        }
    }

    impl AnimGraphLoader for Mem {
        fn graph(&self, _: &str) -> Option<GraphDoc> {
            None
        }
    }

    fn meshes() -> Vec<String> {
        ["a.mesh", "b.mesh", "c.mesh"].iter().map(|s| s.to_string()).collect()
    }

    fn tick(p: &mut AnimGraphPreview, doc: &GraphDoc, rev: u64, dt: f32) {
        p.tick_with(doc, "graphs/t.animgraph", rev, &meshes(), &Mem, &Mem, dt);
    }

    fn bone0_x(p: &AnimGraphPreview) -> f32 {
        p.skeleton.as_ref().expect("skeleton").local_transforms[0].translation.x
    }

    #[test]
    fn runs_the_entry_state_on_an_auto_picked_mesh_and_follows_the_strip() {
        let doc = two_state_doc();
        let mut p = AnimGraphPreview::default();
        tick(&mut p, &doc, 1, 0.1);
        assert_eq!(p.status, None);
        assert_eq!(p.mesh.as_deref(), Some("b.mesh"), "a.mesh lacks 'child', c.mesh has no bones");
        assert_eq!(p.state_label().as_deref(), Some("Idle"));
        assert!((bone0_x(&p) - 2.0).abs() < 1e-4, "entry state pose");
        let snap = p.snapshot();
        assert_eq!(snap.instance_id, PANEL_INSTANCE_ID);
        assert_eq!(snap.disabled, None);
        assert_eq!(snap.active_state, Some(2), "document id of the entry state");

        // The strip's Bool toggle lands on the panel's blackboard and the
        // transition runs: mid-fade the label names both ends.
        assert!(p.apply(&AnimParamEdit::SetBool("walk".into(), true)));
        tick(&mut p, &doc, 1, 0.1);
        assert_eq!(p.state_label().as_deref(), Some("Idle \u{2192} Walk"));
        assert_eq!(p.snapshot().fade.map(|(from, _)| from), Some(2));
        tick(&mut p, &doc, 1, 0.6);
        assert_eq!(p.state_label().as_deref(), Some("Walk"));
        assert!((bone0_x(&p) - 10.0).abs() < 1e-4, "settled on Walk's pose");

        // Pause holds the frame; the clock stops.
        let t = p.time();
        p.playing = false;
        tick(&mut p, &doc, 1, 0.3);
        assert_eq!(p.time(), t);
    }

    #[test]
    fn a_compile_error_is_the_status_and_refuses_the_strip() {
        let mut doc = two_state_doc();
        doc.nodes.retain(|n| n.type_id != ANIM_ENTRY_TYPE_ID);
        let mut p = AnimGraphPreview::default();
        tick(&mut p, &doc, 1, 0.1);
        assert!(p.status.as_deref().is_some_and(|s| s.contains("ENTRY")), "{:?}", p.status);
        assert!(p.snapshot().disabled.as_deref().is_some_and(|s| s.contains("ENTRY")));
        assert_eq!(p.state_label(), None);
    }

    #[test]
    fn a_transient_refusal_keeps_the_last_good_machine_and_blackboard() {
        let good = two_state_doc();
        let mut p = AnimGraphPreview::default();
        tick(&mut p, &good, 1, 0.1);
        p.apply(&AnimParamEdit::SetBool("walk".into(), true));
        tick(&mut p, &good, 1, 0.1);
        tick(&mut p, &good, 1, 0.6);
        assert_eq!(p.state_label().as_deref(), Some("Walk"));
        // Mid-edit the document refuses: the pane says so, the strip is
        // refused, and the pose holds.
        let mut broken = good.clone();
        broken.nodes.retain(|n| n.type_id != ANIM_ENTRY_TYPE_ID);
        tick(&mut p, &broken, 2, 0.1);
        assert!(p.status.as_deref().is_some_and(|s| s.contains("ENTRY")), "{:?}", p.status);
        assert!(p.snapshot().disabled.is_some());
        assert!((bone0_x(&p) - 10.0).abs() < 1e-4, "the last pose holds");
        // The next edit fixes it: same plan, so the machine is still in Walk
        // with the Bool still set — nothing reset.
        tick(&mut p, &good, 3, 0.1);
        assert_eq!(p.status, None);
        assert_eq!(p.state_label().as_deref(), Some("Walk"));
        assert_eq!(p.snapshot().disabled, None);
        assert!(p.snapshot().params.iter().any(|q| q.slug == "walk"
            && q.value == crate::engine::animation::graph::ParamValue::Bool(true)));
    }

    #[test]
    fn the_entry_nodes_preview_mesh_wins_and_a_mismatch_is_explained() {
        let mut doc = two_state_doc();
        doc.nodes[0].properties.insert(PREVIEW_MESH_PROP.into(), PropValue::Str("a.mesh".into()));
        let mut p = AnimGraphPreview::default();
        tick(&mut p, &doc, 1, 0.1);
        assert_eq!(p.mesh.as_deref(), Some("a.mesh"));
        assert!(p.status.as_deref().is_some_and(|s| s.contains("don't match")), "{:?}", p.status);
        doc.nodes[0].properties.insert(PREVIEW_MESH_PROP.into(), PropValue::Str("c.mesh".into()));
        tick(&mut p, &doc, 2, 0.1);
        assert!(p.status.as_deref().is_some_and(|s| s.contains("no skeleton")), "{:?}", p.status);
        doc.nodes[0].properties.remove(PREVIEW_MESH_PROP);
        doc.nodes[1].properties.insert(CLIP_PROP.into(), PropValue::Asset("missing.anim".into()));
        tick(&mut p, &doc, 3, 0.1);
        assert!(p.status.as_deref().is_some_and(|s| s.contains("could not be loaded")), "{:?}", p.status);
    }

    #[test]
    fn recompiles_on_revision_only_and_carries_params_across_a_changed_plan() {
        let mut doc = two_state_doc();
        let mut p = AnimGraphPreview::default();
        tick(&mut p, &doc, 1, 0.1);
        p.apply(&AnimParamEdit::SetBool("walk".into(), true));
        tick(&mut p, &doc, 1, 0.1);
        tick(&mut p, &doc, 1, 0.6);
        assert_eq!(p.state_label().as_deref(), Some("Walk"));
        let t = p.time();
        // Same document, new revision (a node move, say): the plan is equal,
        // so the machine and its clock survive.
        tick(&mut p, &doc, 2, 0.1);
        assert_eq!(p.state_label().as_deref(), Some("Walk"));
        assert!(p.time() > t);
        // A real change restarts at ENTRY with the Bool still set: the
        // transition fires again on the first tick.
        doc.nodes[3].properties.insert(DURATION_PROP.into(), PropValue::Float(0.2));
        tick(&mut p, &doc, 3, 0.05);
        assert_eq!(p.state_label().as_deref(), Some("Idle \u{2192} Walk"));
        assert_eq!(p.snapshot().params.iter().find(|q| q.slug == "walk").map(|q| q.value.clone()),
            Some(crate::engine::animation::graph::ParamValue::Bool(true)));
    }

    #[test]
    fn a_mirror_poses_the_runtimes_machine_read_only() {
        let doc = two_state_doc();
        let mut p = AnimGraphPreview::default();
        tick(&mut p, &doc, 1, 0.1);
        // A runtime that already walked: its plan, machine and blackboard,
        // copied the way the host does each frame.
        let plan = p.plan().clone();
        let mut params = AnimParams::from_decls(&plan.parameters);
        params.set_bool("walk", true);
        let mut machine = AnimMachine::new(&plan);
        // The transition fires on the first tick; the fade runs on the next.
        machine.tick(&plan, &mut params, 0.1);
        machine.tick(&plan, &mut params, 1.0);
        p.mirror = Some(Mirror { name: "Hero".into(), plan, machine, params });
        tick(&mut p, &doc, 1, 0.1);
        assert_eq!(p.state_label().as_deref(), Some("Walk"));
        assert!((bone0_x(&p) - 10.0).abs() < 1e-4, "the entity's pose, not the panel's");
        // The panel's own machine did not move: unmirrored, it is still Idle.
        p.mirror = None;
        tick(&mut p, &doc, 1, 0.0);
        assert_eq!(p.state_label().as_deref(), Some("Idle"));
    }

    #[test]
    fn set_preview_mesh_is_one_undoable_property_on_the_entry_node() {
        let reg = anim_node_registry();
        let mut st = GraphEditorState::from_doc(
            "graphs/t.animgraph".into(),
            two_state_doc(),
            GraphDomain::Animation,
            &reg,
        );
        let rev = st.revision;
        assert_eq!(st.preview_mesh(), "");
        st.set_preview_mesh("b.mesh".into(), &reg);
        assert_eq!(st.preview_mesh(), "b.mesh");
        assert_eq!(preview_mesh_of(&st.doc), "b.mesh");
        assert!(st.dirty);
        assert!(st.revision > rev, "an edit moves the revision");
        assert_eq!(st.stack.undo_description().as_deref(), Some("Set preview_mesh"));
        st.undo(&reg);
        assert_eq!(st.preview_mesh(), "");
        st.redo(&reg);
        st.set_preview_mesh(String::new(), &reg);
        assert!(
            !st.doc.nodes[0].properties.contains_key(PREVIEW_MESH_PROP),
            "auto removes the property rather than storing an empty string"
        );
        // Not an ENTRY property on any other node.
        assert!(!st.doc.nodes.iter().any(|n| {
            n.type_id == ANIM_STATE_ALIAS_TYPE_ID && n.properties.contains_key(PREVIEW_MESH_PROP)
        }));
    }
}
