//! Blend space editor per-document state (Task 41.5 ticket 04).
//!
//! Holds one open `.blendspace` document, its doc-local undo/redo stack
//! (saved-cursor dirty rule, like the curve and graph editors), the compiled
//! [`BlendSpace`] the canvas draws from, and the session state the panel
//! needs (selection, in-flight field edits). The drawing layer is
//! `blend_space_editor_crusty`.
//!
//! Every edit is one undo entry with a verb-object label; a field that
//! commits the value it already had records nothing.

use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;


use super::blend_space_preview::{preview_input, BlendSpacePreview};
use super::edit_stack::{EditStack, ReversibleEdit};
use crate::engine::animation::graph::{AnimAssetLoader, DiskAnimAssets};
use crate::engine::animation::blend_space::{
    parse_blend_space, serialize_blend_space, BlendAxis, BlendSample, BlendSpace, BlendSpaceDoc,
};

/// A reversible document edit. Whole-struct before/after for axes and
/// samples: the structs are tiny and one variant then covers every field.
#[derive(Debug, Clone, PartialEq)]
pub enum BlendSpaceEdit {
    SetAxisCount { from: u32, to: u32 },
    SetAxis { axis: usize, from: BlendAxis, to: BlendAxis, label: String },
    SetSmoothing { from: f32, to: f32 },
    AddSample { index: usize, sample: BlendSample },
    RemoveSample { index: usize, sample: BlendSample },
    SetSample { index: usize, from: BlendSample, to: BlendSample, label: String },
    SetPreviewMesh { from: String, to: String },
}

impl ReversibleEdit for BlendSpaceEdit {
    type Doc = BlendSpaceDoc;

    fn apply(&self, doc: &mut BlendSpaceDoc) {
        match self {
            Self::SetAxisCount { to, .. } => doc.axis_count = *to,
            Self::SetPreviewMesh { to, .. } => doc.preview_mesh = to.clone(),
            Self::SetAxis { axis, to, .. } => {
                if let Some(a) = doc.axes.get_mut(*axis) {
                    *a = to.clone();
                }
            }
            Self::SetSmoothing { to, .. } => doc.input_smoothing = *to,
            Self::AddSample { index, sample } => {
                if *index <= doc.samples.len() {
                    doc.samples.insert(*index, sample.clone());
                }
            }
            Self::RemoveSample { index, .. } => {
                if *index < doc.samples.len() {
                    doc.samples.remove(*index);
                }
            }
            Self::SetSample { index, to, .. } => {
                if let Some(s) = doc.samples.get_mut(*index) {
                    *s = to.clone();
                }
            }
        }
    }

    fn revert(&self, doc: &mut BlendSpaceDoc) {
        match self {
            Self::SetAxisCount { from, to } => Self::SetAxisCount { from: *to, to: *from }.apply(doc),
            Self::SetAxis { axis, from, to, label } => Self::SetAxis {
                axis: *axis,
                from: to.clone(),
                to: from.clone(),
                label: label.clone(),
            }
            .apply(doc),
            Self::SetSmoothing { from, to } => Self::SetSmoothing { from: *to, to: *from }.apply(doc),
            Self::SetPreviewMesh { from, .. } => doc.preview_mesh = from.clone(),
            Self::AddSample { index, sample } => {
                Self::RemoveSample { index: *index, sample: sample.clone() }.apply(doc)
            }
            Self::RemoveSample { index, sample } => {
                Self::AddSample { index: *index, sample: sample.clone() }.apply(doc)
            }
            Self::SetSample { index, from, to, label } => Self::SetSample {
                index: *index,
                from: to.clone(),
                to: from.clone(),
                label: label.clone(),
            }
            .apply(doc),
        }
    }

    fn description(&self) -> String {
        match self {
            Self::SetAxisCount { to, .. } => format!("Set Axes {}", if *to >= 2 { "2D" } else { "1D" }),
            Self::SetAxis { label, .. } | Self::SetSample { label, .. } => label.clone(),
            Self::SetSmoothing { .. } => "Set Smoothing".into(),
            Self::SetPreviewMesh { .. } => "Set Preview Mesh".into(),
            Self::AddSample { .. } => "Add Sample".into(),
            Self::RemoveSample { .. } => "Delete Sample".into(),
        }
    }
}

pub type BlendSpaceEditStack = EditStack<BlendSpaceEdit>;

/// An editable field in the details column. Keys the in-flight text buffer
/// and drag start so a field commits once, at the end of the gesture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Field {
    AxisName(usize),
    AxisParam(usize),
    AxisMin(usize),
    AxisMax(usize),
    AxisGrid(usize),
    SampleX(usize),
    SampleY(usize),
    SampleRate(usize),
    Smoothing,
}

/// A canvas right-click the panel turns into a context menu.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CanvasMenu {
    /// Screen anchor on the frame it opened (`None` afterwards — the menu
    /// widget remembers its own placement).
    pub open_at: Option<[f32; 2]>,
    /// Where on the axes the click landed.
    pub at: [f32; 2],
    /// The sample under the click, if any.
    pub sample: Option<usize>,
    /// Grid snapping as the modifiers had it when the menu opened (Shift
    /// bypasses), so the choice is not read off a later frame's keyboard.
    pub snap: bool,
}

/// What a numeric field did this frame (see [`BlendSpaceEditorState::numeric_event`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FieldEvent {
    None,
    /// Mid-scrub: show the value, do not record it.
    Live(f32),
    /// The gesture ended (release, Enter, click-away): one undo entry.
    Commit { from: f32, to: f32 },
}

pub struct BlendSpaceEditorState {
    /// Content-relative path — the tab key and the cache key.
    pub path: String,
    pub doc: BlendSpaceDoc,
    pub dirty: bool,
    pub stack: BlendSpaceEditStack,
    /// The current document compiled, or why it cannot be. Refreshed after
    /// every doc change; the canvas draws triangles from `Ok`.
    pub compiled: Result<BlendSpace, String>,
    /// Selected sample index (session state; ticket 05 drives it from the canvas).
    pub selection: Option<usize>,
    /// Preview input point in axis units (session state; ticket 05).
    pub preview_point: Option<[f32; 2]>,
    /// A canvas sample drag in flight: the index and its pre-drag value
    /// (ticket 05). The doc updates live; one entry lands on release.
    pub drag: Option<(usize, BlendSample)>,
    /// A Ctrl-drag of the preview point in flight.
    pub preview_drag: bool,
    /// Sample under the pointer this frame (the hover label).
    pub hovered: Option<usize>,
    /// Right-click menu pending/open: screen anchor, doc coords, the sample
    /// under the click if any.
    pub menu: Option<CanvasMenu>,
    /// The panel drew this tab this frame; the host takes it to decide which
    /// tabs' preview points may drive an entity.
    pub shown: bool,
    /// Display name of the entity the host is driving from `preview_point`
    /// (`None` = nothing bound). Written by the host after the UI.
    pub preview_bound: Option<String>,
    /// The text field being typed into and its buffer — committed on
    /// Enter/click-away, never per keystroke.
    pub field_text: Option<(Field, String)>,
    /// A numeric scrub in flight: the field and its pre-drag value.
    pub field_drag: Option<(Field, f32)>,
    /// Clip names per `.anim` path, loaded lazily for the Clip-name dropdown.
    /// A container that fails to load is cached as empty (no dropdown).
    clip_names: HashMap<String, Vec<String>>,
    /// When this editor last wrote the file — the hot-reload echo guard.
    pub last_saved_at: Option<Instant>,
    /// Transient status line (save failures, undo/redo labels).
    pub toast: Option<(String, Instant)>,
    /// The embedded 3D preview (ticket 08): skeleton, clock, pose, and the
    /// host-filled render target.
    pub preview: BlendSpacePreview,
    /// The document changed since the preview last rebuilt its plan.
    preview_stale: bool,
    /// When the preview last advanced — the frame clock.
    preview_last_frame: Option<Instant>,
}

impl BlendSpaceEditorState {
    /// Load a `.blendspace` from disk. `content_rel` is the tab/cache key.
    pub fn open(abs_path: &Path, content_rel: &str) -> Result<Self, String> {
        let text = std::fs::read_to_string(abs_path)
            .map_err(|e| format!("{}: {e}", abs_path.display()))?;
        Ok(Self::from_doc(parse_blend_space(&text)?, content_rel))
    }

    pub fn from_doc(doc: BlendSpaceDoc, content_rel: &str) -> Self {
        let compiled = BlendSpace::compile(&doc);
        Self {
            path: content_rel.to_string(),
            doc,
            dirty: false,
            stack: BlendSpaceEditStack::new(),
            compiled,
            selection: None,
            preview_point: None,
            drag: None,
            preview_drag: false,
            hovered: None,
            menu: None,
            shown: false,
            preview_bound: None,
            field_text: None,
            field_drag: None,
            clip_names: HashMap::new(),
            last_saved_at: None,
            toast: None,
            preview: BlendSpacePreview::default(),
            preview_stale: true,
            preview_last_frame: None,
        }
    }

    // ── embedded preview (ticket 08) ─────────────────────────────────────

    /// One frame of the preview: rebuild its plan if the document changed,
    /// then advance by the wall-clock frame time (capped so a stall does
    /// not leap). `mesh_assets` feeds the auto-pick.
    pub fn tick_preview(&mut self, mesh_assets: &[String]) {
        let now = Instant::now();
        let dt = self
            .preview_last_frame
            .map(|t| now.duration_since(t).as_secs_f32().min(0.1))
            .unwrap_or(0.0);
        self.preview_last_frame = Some(now);
        let loader = DiskAnimAssets { content_root: "content".into() };
        self.tick_preview_with(&loader, mesh_assets, dt);
    }

    /// [`Self::tick_preview`] with an explicit loader and step (tests).
    pub fn tick_preview_with(&mut self, loader: &dyn AnimAssetLoader, mesh_assets: &[String], dt: f32) {
        if self.preview_stale {
            self.preview.rebuild(&self.doc, &self.compiled, mesh_assets, loader);
            self.preview_stale = false;
        }
        let input = preview_input(&self.doc, self.preview_point);
        self.preview.advance(dt, input);
    }

    /// The input the preview plays at: the preview point, else the axis minimums.
    pub fn preview_input(&self) -> [f32; 2] {
        preview_input(&self.doc, self.preview_point)
    }

    /// Choose the preview mesh (empty = auto); one "Set Preview Mesh" entry.
    pub fn set_preview_mesh(&mut self, to: String) {
        let from = std::mem::replace(&mut self.doc.preview_mesh, to.clone());
        if from != to {
            self.commit(BlendSpaceEdit::SetPreviewMesh { from, to });
        }
    }

    fn recompile(&mut self) {
        self.compiled = BlendSpace::compile(&self.doc);
        self.preview_stale = true;
    }

    /// Write the doc back to disk, clearing dirty. Cache invalidation is the
    /// host's job (it owns the resources).
    pub fn save(&mut self, abs_path: &Path) -> Result<(), String> {
        self.field_text = None;
        self.field_drag = None;
        let text = serialize_blend_space(&self.doc)?;
        super::atomic_file::atomic_write(abs_path, &text)
            .map_err(|e| format!("{}: {e}", abs_path.display()))?;
        self.stack.mark_saved();
        self.dirty = false;
        self.last_saved_at = Some(Instant::now());
        Ok(())
    }

    /// Record an already-applied edit and refresh dirty + the compiled space.
    pub fn commit(&mut self, edit: BlendSpaceEdit) {
        self.stack.record(edit);
        self.after_change();
    }

    pub fn undo(&mut self) {
        self.field_text = None;
        self.field_drag = None;
        if let Some(d) = self.stack.undo(&mut self.doc) {
            self.toast(format!("Undo {d}"));
        }
        self.after_change();
    }

    pub fn redo(&mut self) {
        self.field_text = None;
        self.field_drag = None;
        if let Some(d) = self.stack.redo(&mut self.doc) {
            self.toast(format!("Redo {d}"));
        }
        self.after_change();
    }

    fn after_change(&mut self) {
        self.dirty = self.stack.is_dirty();
        self.recompile();
        let n = self.doc.samples.len();
        if self.selection.is_some_and(|i| i >= n) {
            self.selection = None;
        }
        if self.drag.as_ref().is_some_and(|(i, _)| *i >= n) {
            self.drag = None;
        }
    }

    // ── canvas gestures (ticket 05) ──────────────────────────────────────

    /// Clamp `p` to the axis ranges and, when `snap`, round it to the grid
    /// (step = axis range / `grid_divisions`). One-axis spaces leave `y`
    /// alone.
    pub fn snap_point(&self, p: [f32; 2], snap: bool) -> [f32; 2] {
        let mut out = p;
        for (k, a) in self.doc.active_axes().iter().enumerate() {
            let (lo, hi) = (a.min.min(a.max), a.min.max(a.max));
            let mut v = p[k].clamp(lo, hi);
            if snap && a.grid_divisions > 0 && hi > lo {
                let step = (hi - lo) / a.grid_divisions as f32;
                v = (lo + ((v - lo) / step).round() * step).clamp(lo, hi);
            }
            out[k] = v;
        }
        out
    }

    /// Start moving sample `index` (selects it).
    pub fn begin_drag(&mut self, index: usize) {
        let Some(from) = self.doc.samples.get(index).cloned() else { return };
        self.selection = Some(index);
        self.drag = Some((index, from));
    }

    /// Move the dragged sample to `p` (snapped unless bypassed). Live: the
    /// document changes and recompiles, nothing is recorded yet.
    pub fn drag_to(&mut self, p: [f32; 2], snap: bool) {
        let Some((index, _)) = self.drag else { return };
        let p = self.snap_point(p, snap);
        let two_d = self.doc.is_2d();
        let Some(sm) = self.doc.samples.get_mut(index) else { return };
        if sm.x == p[0] && (!two_d || sm.y == p[1]) {
            return;
        }
        sm.x = p[0];
        if two_d {
            sm.y = p[1];
        }
        self.recompile();
    }

    /// Release: one "Move Sample" entry from the pre-drag value (nothing
    /// when it never moved).
    pub fn end_drag(&mut self) {
        if let Some((index, from)) = self.drag.take() {
            self.record_sample(index, from, "Move Sample");
        }
    }

    /// Escape: put the sample back where the drag found it.
    pub fn cancel_drag(&mut self) {
        if let Some((index, from)) = self.drag.take() {
            if let Some(sm) = self.doc.samples.get_mut(index) {
                *sm = from;
            }
            self.recompile();
        }
    }

    /// Append a sample at `p` (snapped unless bypassed) and select it; the
    /// clip is picked afterwards in the details column.
    pub fn add_sample_at(&mut self, p: [f32; 2], snap: bool) -> usize {
        let p = self.snap_point(p, snap);
        let y = if self.doc.is_2d() { p[1] } else { (self.doc.axes[1].min + self.doc.axes[1].max) * 0.5 };
        let sample = BlendSample::new(p[0], y, "");
        let index = self.doc.samples.len();
        self.doc.samples.push(sample.clone());
        self.commit(BlendSpaceEdit::AddSample { index, sample });
        self.selection = Some(index);
        index
    }

    /// Place the preview point, clamped to the axis ranges.
    pub fn set_preview(&mut self, p: [f32; 2]) {
        self.preview_point = Some(self.snap_point(p, false));
    }

    pub fn clear_preview(&mut self) {
        self.preview_point = None;
        self.preview_drag = false;
    }

    /// The samples the preview point blends and their weights (document
    /// indices), empty without a point or a compiled space.
    pub fn preview_weights(&self) -> Vec<(usize, f32)> {
        match (&self.compiled, self.preview_point) {
            (Ok(space), Some(p)) => space.weights(p).as_slice().to_vec(),
            _ => Vec::new(),
        }
    }

    /// `true` while a canvas gesture owns the pointer.
    pub fn gesture_in_flight(&self) -> bool {
        self.drag.is_some() || self.preview_drag
    }

    pub fn toast(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), Instant::now()));
    }

    // ── document edits (apply + one undo entry each) ─────────────────────

    /// Flip 1D ↔ 2D. Only `axis_count` changes: `y` stays on every sample.
    pub fn set_axis_count(&mut self, to: u32) {
        let to = to.clamp(1, 2);
        let from = self.doc.axis_count;
        if from == to {
            return;
        }
        self.doc.axis_count = to;
        self.commit(BlendSpaceEdit::SetAxisCount { from, to });
    }

    /// Replace axis `axis` with `to` under `label` ("Set Axis Name", ...).
    pub fn set_axis(&mut self, axis: usize, to: BlendAxis, label: &str) {
        let Some(from) = self.doc.axes.get(axis).cloned() else { return };
        if from == to {
            return;
        }
        self.doc.axes[axis] = to;
        self.record_axis(axis, from, label);
    }

    /// The doc already holds the new axis; record the entry against `from`.
    pub fn record_axis(&mut self, axis: usize, from: BlendAxis, label: &str) {
        let Some(to) = self.doc.axes.get(axis).cloned() else { return };
        if from == to {
            return;
        }
        self.commit(BlendSpaceEdit::SetAxis { axis, from, to, label: label.into() });
    }

    pub fn set_smoothing(&mut self, to: f32) {
        let from = self.doc.input_smoothing;
        self.doc.input_smoothing = to;
        self.record_smoothing(from);
    }

    pub fn record_smoothing(&mut self, from: f32) {
        let to = self.doc.input_smoothing;
        if from != to {
            self.commit(BlendSpaceEdit::SetSmoothing { from, to });
        }
    }

    /// Append a sample at the axis midpoint (both axes, so a later 2D flip
    /// finds it centred). Returns its index.
    pub fn add_sample(&mut self) -> usize {
        let mid = |a: &BlendAxis| (a.min + a.max) * 0.5;
        let sample = BlendSample::new(mid(&self.doc.axes[0]), mid(&self.doc.axes[1]), "");
        let index = self.doc.samples.len();
        self.doc.samples.push(sample.clone());
        self.commit(BlendSpaceEdit::AddSample { index, sample });
        self.selection = Some(index);
        index
    }

    pub fn remove_sample(&mut self, index: usize) {
        if index >= self.doc.samples.len() {
            return;
        }
        let sample = self.doc.samples.remove(index);
        self.commit(BlendSpaceEdit::RemoveSample { index, sample });
    }

    pub fn delete_selection(&mut self) {
        if let Some(i) = self.selection.take() {
            self.remove_sample(i);
        }
    }

    pub fn set_sample(&mut self, index: usize, to: BlendSample, label: &str) {
        let Some(from) = self.doc.samples.get(index).cloned() else { return };
        if from == to {
            return;
        }
        self.doc.samples[index] = to;
        self.record_sample(index, from, label);
    }

    /// The doc already holds the new sample; record the entry against `from`.
    pub fn record_sample(&mut self, index: usize, from: BlendSample, label: &str) {
        let Some(to) = self.doc.samples.get(index).cloned() else { return };
        if from == to {
            return;
        }
        self.commit(BlendSpaceEdit::SetSample { index, from, to, label: label.into() });
    }

    // ── field gestures (panel helpers, pixel-free) ───────────────────────

    /// Fold one frame of a numeric widget into a gesture. `before`/`after`
    /// are the value going in and coming out of the widget, `pressed` whether
    /// the widget is being scrubbed. A scrub reports `Live` until release,
    /// then one `Commit` from the pre-drag value; a typed or stepped value
    /// commits at once.
    pub fn numeric_event(&mut self, field: Field, before: f32, after: f32, pressed: bool) -> FieldEvent {
        let active = self.field_drag.filter(|(f, _)| *f == field).map(|(_, v)| v);
        if pressed {
            if after != before && active.is_none() {
                self.field_drag = Some((field, before));
            }
            return if after != before { FieldEvent::Live(after) } else { FieldEvent::None };
        }
        if let Some(from) = active {
            self.field_drag = None;
            return if from != after { FieldEvent::Commit { from, to: after } } else { FieldEvent::None };
        }
        if after != before {
            FieldEvent::Commit { from: before, to: after }
        } else {
            FieldEvent::None
        }
    }

    /// The buffer a text field should show this frame: the live one while it
    /// is being typed into, else the document value.
    pub fn text_buffer(&self, field: Field, current: &str) -> String {
        match &self.field_text {
            Some((f, b)) if *f == field => b.clone(),
            _ => current.to_string(),
        }
    }

    /// Fold one frame of a text widget into a gesture. Holds the buffer while
    /// focused; returns the new value once on Enter or click-away (Escape
    /// drops it), and nothing when it equals `current`.
    pub fn text_event(
        &mut self,
        field: Field,
        current: &str,
        buf: String,
        focused: bool,
        submitted: bool,
        cancelled: bool,
    ) -> Option<String> {
        let was_active = self.field_text.as_ref().is_some_and(|(f, _)| *f == field);
        if focused && !submitted && !cancelled {
            self.field_text = Some((field, buf));
            return None;
        }
        if was_active {
            self.field_text = None;
        }
        if (was_active || submitted) && !cancelled && buf != current {
            Some(buf)
        } else {
            None
        }
    }

    /// Clip names inside the container at content-relative `clip`, loaded on
    /// first ask. Empty when the file is missing or unreadable.
    pub fn clip_names(&mut self, clip: &str) -> &[String] {
        if !self.clip_names.contains_key(clip) {
            let names = crate::engine::assets::mesh_import::load_anim_binary(
                &Path::new("content").join(clip),
            )
            .map(|(_, clips)| clips.into_iter().map(|c| c.name).collect())
            .unwrap_or_default();
            self.clip_names.insert(clip.to_string(), names);
        }
        self.clip_names.get(clip).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Forget cached clip names (an `.anim` changed on disk).
    pub fn forget_clip_names(&mut self) {
        self.clip_names.clear();
        self.preview.forget_clips();
        self.preview_stale = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> BlendSpaceEditorState {
        let mut doc = BlendSpaceDoc::default();
        doc.samples.push(BlendSample::new(0.0, 0.0, "anims/idle.anim"));
        doc.samples.push(BlendSample::new(1.0, 0.5, "anims/run.anim"));
        BlendSpaceEditorState::from_doc(doc, "blendspaces/t.blendspace")
    }

    #[test]
    fn edits_mark_dirty_and_undo_redo_restore_exactly() {
        let mut s = state();
        let clean = s.doc.clone();
        assert!(!s.dirty);

        let mut axis = s.doc.axes[0].clone();
        axis.name = "Velocity".into();
        s.set_axis(0, axis, "Set Axis Name");
        s.set_smoothing(0.25);
        let i = s.add_sample();
        assert_eq!(s.doc.samples[i].x, 0.5, "appended at the axis midpoint");
        let mut moved = s.doc.samples[0].clone();
        moved.x = 0.3;
        s.set_sample(0, moved, "Move Sample");
        s.remove_sample(1);
        assert!(s.dirty);
        assert_eq!(s.stack.undo_len(), 5);
        assert_eq!(s.stack.undo_description().as_deref(), Some("Delete Sample"));
        let edited = s.doc.clone();

        for _ in 0..5 {
            s.undo();
        }
        assert_eq!(s.doc, clean);
        assert!(!s.dirty, "walked back to the save point");
        for _ in 0..5 {
            s.redo();
        }
        assert_eq!(s.doc, edited);
        assert!(s.dirty);
    }

    #[test]
    fn same_value_records_nothing() {
        let mut s = state();
        let axis = s.doc.axes[0].clone();
        s.set_axis(0, axis, "Set Axis Name");
        s.set_smoothing(0.0);
        let same = s.doc.samples[0].clone();
        s.set_sample(0, same, "Move Sample");
        assert!(!s.stack.can_undo());
        assert!(!s.dirty);
    }

    #[test]
    fn axis_toggle_preserves_y() {
        let mut s = state();
        s.set_axis_count(2);
        assert!(s.doc.is_2d());
        assert_eq!(s.doc.samples[1].y, 0.5);
        s.set_axis_count(1);
        assert!(!s.doc.is_2d());
        assert_eq!(s.doc.samples[1].y, 0.5, "hidden, not lost");
        assert_eq!(s.stack.undo_description().as_deref(), Some("Set Axes 1D"));
        s.set_axis_count(1);
        assert_eq!(s.stack.undo_len(), 2, "a no-op flip records nothing");
    }

    #[test]
    fn compiled_tracks_the_document() {
        let mut s = state();
        assert!(s.compiled.is_ok());
        s.remove_sample(1);
        s.remove_sample(0);
        assert_eq!(s.compiled.as_ref().err().map(String::as_str), Some("no samples"));
        s.undo();
        assert!(s.compiled.is_ok());
    }

    #[test]
    fn save_round_trips_and_clears_dirty() {
        let dir = std::env::temp_dir().join(format!("blend_space_editor_{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("tmp dir");
        let path = dir.join("t.blendspace");

        let mut s = state();
        s.set_axis_count(2);
        s.add_sample();
        assert!(s.dirty);
        s.save(&path).expect("save");
        assert!(!s.dirty);
        assert!(s.last_saved_at.is_some());

        let reopened = BlendSpaceEditorState::open(&path, "t.blendspace").expect("open");
        assert_eq!(reopened.doc, s.doc);

        // Undo past the save point is dirty again; redo back to it is clean.
        s.undo();
        assert!(s.dirty);
        s.redo();
        assert!(!s.dirty);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn numeric_gesture_is_one_commit_from_the_pre_drag_value() {
        let mut s = state();
        let f = Field::SampleX(0);
        assert_eq!(s.numeric_event(f, 0.0, 0.0, true), FieldEvent::None);
        assert_eq!(s.numeric_event(f, 0.0, 0.1, true), FieldEvent::Live(0.1));
        assert_eq!(s.numeric_event(f, 0.1, 0.4, true), FieldEvent::Live(0.4));
        assert_eq!(s.numeric_event(f, 0.4, 0.4, false), FieldEvent::Commit { from: 0.0, to: 0.4 });
        // A typed value commits at once.
        assert_eq!(s.numeric_event(f, 0.4, 2.0, false), FieldEvent::Commit { from: 0.4, to: 2.0 });
        // A scrub that returns to where it started records nothing.
        assert_eq!(s.numeric_event(f, 2.0, 2.5, true), FieldEvent::Live(2.5));
        assert_eq!(s.numeric_event(f, 2.5, 2.0, true), FieldEvent::Live(2.0));
        assert_eq!(s.numeric_event(f, 2.0, 2.0, false), FieldEvent::None);
    }

    #[test]
    fn text_gesture_commits_on_end_edit_only() {
        let mut s = state();
        let f = Field::AxisName(0);
        assert_eq!(s.text_event(f, "Speed", "Sp".into(), true, false, false), None);
        assert_eq!(s.text_buffer(f, "Speed"), "Sp");
        assert_eq!(s.text_event(f, "Speed", "Spe".into(), true, false, false), None);
        // Click-away commits.
        assert_eq!(s.text_event(f, "Speed", "Spe".into(), false, false, false), Some("Spe".into()));
        assert_eq!(s.text_buffer(f, "Spe"), "Spe");
        // Escape drops.
        assert_eq!(s.text_event(f, "Spe", "Sx".into(), true, false, false), None);
        assert_eq!(s.text_event(f, "Spe", "Sx".into(), false, false, true), None);
        // Enter commits; unchanged text is not an edit.
        assert_eq!(s.text_event(f, "Spe", "Spe".into(), true, true, false), None);
        assert_eq!(s.text_event(f, "Spe", "Run".into(), true, true, false), Some("Run".into()));
    }
}

/// Which entity a blend space tab previews on: the first selected entity
/// that has an animation runtime, else the first runtime whose plan
/// references this blend space. `runtimes` is `(entity bits, references
/// this space)` in world order; `selected` is the scene selection.
pub fn resolve_blend_space_bind(selected: &[u64], runtimes: &[(u64, bool)]) -> Option<u64> {
    selected
        .iter()
        .copied()
        .find(|s| runtimes.iter().any(|(id, _)| id == s))
        .or_else(|| runtimes.iter().find(|(_, refs)| *refs).map(|(id, _)| *id))
}

#[cfg(test)]
mod canvas_tests {
    use super::*;

    fn state() -> BlendSpaceEditorState {
        let mut doc = BlendSpaceDoc::default();
        doc.axis_count = 2;
        doc.axes[0] = BlendAxis::new("Speed", 0.0, 6.0);
        doc.axes[0].grid_divisions = 6;
        doc.axes[1] = BlendAxis::new("Direction", -1.0, 1.0);
        doc.axes[1].grid_divisions = 4;
        doc.samples.push(BlendSample::new(0.0, 0.0, "a.anim"));
        doc.samples.push(BlendSample::new(6.0, -1.0, "b.anim"));
        doc.samples.push(BlendSample::new(6.0, 1.0, "c.anim"));
        BlendSpaceEditorState::from_doc(doc, "blendspaces/t.blendspace")
    }

    #[test]
    fn drag_commits_one_snapped_move_on_release() {
        let mut s = state();
        s.begin_drag(0);
        assert_eq!(s.selection, Some(0));
        s.drag_to([1.3, 0.2], true);
        s.drag_to([2.6, 0.4], true);
        assert_eq!((s.doc.samples[0].x, s.doc.samples[0].y), (3.0, 0.5), "snapped live");
        assert!(!s.stack.can_undo(), "nothing recorded mid-drag");
        s.end_drag();
        assert_eq!(s.stack.undo_len(), 1);
        assert_eq!(s.stack.undo_description().as_deref(), Some("Move Sample"));
        s.undo();
        assert_eq!((s.doc.samples[0].x, s.doc.samples[0].y), (0.0, 0.0));

        // Shift bypasses the grid but still clamps to the axes; Escape restores.
        s.begin_drag(1);
        s.drag_to([7.0, -0.37], false);
        assert_eq!((s.doc.samples[1].x, s.doc.samples[1].y), (6.0, -0.37));
        s.cancel_drag();
        assert_eq!((s.doc.samples[1].x, s.doc.samples[1].y), (6.0, -1.0));
        assert_eq!(s.stack.undo_len(), 0);
    }

    #[test]
    fn one_axis_drag_keeps_y() {
        let mut s = state();
        s.doc.axis_count = 1;
        s.begin_drag(2);
        s.drag_to([2.0, 0.0], true);
        assert_eq!((s.doc.samples[2].x, s.doc.samples[2].y), (2.0, 1.0));
    }

    #[test]
    fn add_sample_here_lands_on_the_click_selected() {
        let mut s = state();
        let i = s.add_sample_at([2.6, 0.3], true);
        assert_eq!(i, 3);
        assert_eq!(s.selection, Some(3));
        assert_eq!((s.doc.samples[3].x, s.doc.samples[3].y), (3.0, 0.5));
        assert_eq!(s.stack.undo_description().as_deref(), Some("Add Sample"));
        let j = s.add_sample_at([2.6, 0.3], false);
        assert_eq!((s.doc.samples[j].x, s.doc.samples[j].y), (2.6, 0.3));
        s.undo();
        s.undo();
        assert_eq!(s.doc.samples.len(), 3);
    }

    #[test]
    fn preview_point_clamps_and_reports_the_compiled_weights() {
        let mut s = state();
        s.set_preview([9.0, -3.0]);
        assert_eq!(s.preview_point, Some([6.0, -1.0]));
        s.set_preview([4.0, 0.0]);
        let ws = s.preview_weights();
        let expect = s.compiled.as_ref().expect("compiled").weights([4.0, 0.0]);
        assert_eq!(ws, expect.as_slice().to_vec());
        assert!((ws.iter().map(|w| w.1).sum::<f32>() - 1.0).abs() < 1e-5);
        s.clear_preview();
        assert!(s.preview_weights().is_empty());
    }

    #[test]
    fn binding_prefers_a_selected_runtime_then_the_first_referencing_plan() {
        let rts = [(1, false), (2, true), (3, true)];
        assert_eq!(resolve_blend_space_bind(&[1], &rts), Some(1), "selected runtime wins even without a reference");
        assert_eq!(resolve_blend_space_bind(&[9, 3], &rts), Some(3));
        assert_eq!(resolve_blend_space_bind(&[9], &rts), Some(2), "first plan that references the space");
        assert_eq!(resolve_blend_space_bind(&[], &[(1, false)]), None);
    }
}
