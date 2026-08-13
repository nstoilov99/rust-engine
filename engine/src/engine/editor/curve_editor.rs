//! Curve editor per-document state (Task 45-A P8b).
//!
//! Holds one open `.curve` document, its canvas view, selection and a
//! doc-local undo/redo stack — the same shape `graph_editor` gives a `.graph`,
//! down to the saved-cursor dirty rule. The drawing/interaction layer is
//! `curve_editor_crusty`.
//!
//! Deliberately basic, per the plan: named float tracks, keyframes you can
//! add, drag, delete and re-interpolate. The animation task grows it; what it
//! must not have to grow is the *plumbing* — tab, undo, save and the pin
//! refresh a Timeline depends on all live here from day one.
//!
//! Sampling is never reimplemented: everything drawn comes out of
//! [`curve_asset::Track::sample`], the function the interpreter calls.

use std::path::Path;
use std::time::Instant;

use crusty_gui::widgets::CanvasView;
use curve_asset::{parse_curve, serialize_curve, CurveDoc, Interp, Key, Track};

/// A keyframe address. Indices, not ids: a `.curve` has no identity beyond
/// position, and every edit below either preserves the index (drags clamp
/// between neighbours) or restores it exactly (add/remove).
pub type KeyRef = (usize, usize);

// ---------------------------------------------------------------------------
// Edit stack: reversible ops + saved-cursor dirty (mirrors `GraphEditStack`).
// ---------------------------------------------------------------------------

/// A reversible document edit. Each variant stores enough to both apply and
/// revert on its own.
#[derive(Debug, Clone, PartialEq)]
pub enum CurveEdit {
    AddKey { track: usize, index: usize, key: Key },
    RemoveKey { track: usize, index: usize, key: Key },
    /// One drag gesture, coalesced: press-time key on the left, release-time
    /// key on the right. A drag that moves nothing is never recorded.
    MoveKey { track: usize, index: usize, from: Key, to: Key },
    SetInterp { track: usize, index: usize, from: Interp, to: Interp },
    AddTrack { index: usize, track: Track },
    RemoveTrack { index: usize, track: Track },
    RenameTrack { index: usize, from: String, to: String },
}

impl CurveEdit {
    /// (Re-)apply against `doc`. Out-of-range indices are ignored rather than
    /// panicking: the stack is the only writer, so that can only happen if a
    /// future edit type breaks the invariant, and losing one redo beats
    /// taking the editor down with it.
    pub fn apply(&self, doc: &mut CurveDoc) {
        match self {
            CurveEdit::AddKey { track, index, key } => {
                if let Some(t) = doc.tracks.get_mut(*track) {
                    if *index <= t.keys.len() {
                        t.keys.insert(*index, *key);
                    }
                }
            }
            CurveEdit::RemoveKey { track, index, .. } => {
                if let Some(t) = doc.tracks.get_mut(*track) {
                    if *index < t.keys.len() {
                        t.keys.remove(*index);
                    }
                }
            }
            CurveEdit::MoveKey { track, index, to, .. } => set_key(doc, *track, *index, *to),
            CurveEdit::SetInterp { track, index, to, .. } => {
                if let Some(k) = doc.tracks.get_mut(*track).and_then(|t| t.keys.get_mut(*index)) {
                    k.interp = *to;
                }
            }
            CurveEdit::AddTrack { index, track } => {
                if *index <= doc.tracks.len() {
                    doc.tracks.insert(*index, track.clone());
                }
            }
            CurveEdit::RemoveTrack { index, .. } => {
                if *index < doc.tracks.len() {
                    doc.tracks.remove(*index);
                }
            }
            CurveEdit::RenameTrack { index, to, .. } => {
                if let Some(t) = doc.tracks.get_mut(*index) {
                    t.label = to.clone();
                }
            }
        }
    }

    fn revert(&self, doc: &mut CurveDoc) {
        match self {
            CurveEdit::AddKey { track, index, key } => {
                CurveEdit::RemoveKey { track: *track, index: *index, key: *key }.apply(doc)
            }
            CurveEdit::RemoveKey { track, index, key } => {
                CurveEdit::AddKey { track: *track, index: *index, key: *key }.apply(doc)
            }
            CurveEdit::MoveKey { track, index, from, .. } => set_key(doc, *track, *index, *from),
            CurveEdit::SetInterp { track, index, from, to } => {
                CurveEdit::SetInterp { track: *track, index: *index, from: *to, to: *from }
                    .apply(doc)
            }
            CurveEdit::AddTrack { index, track } => {
                CurveEdit::RemoveTrack { index: *index, track: track.clone() }.apply(doc)
            }
            CurveEdit::RemoveTrack { index, track } => {
                CurveEdit::AddTrack { index: *index, track: track.clone() }.apply(doc)
            }
            CurveEdit::RenameTrack { index, from, to } => {
                CurveEdit::RenameTrack { index: *index, from: to.clone(), to: from.clone() }
                    .apply(doc)
            }
        }
    }

    /// Verb-object, matching the Edit menu's undo labels (M10).
    pub fn description(&self) -> String {
        match self {
            CurveEdit::AddKey { .. } => "Add Key".into(),
            CurveEdit::RemoveKey { .. } => "Delete Key".into(),
            CurveEdit::MoveKey { .. } => "Move Key".into(),
            CurveEdit::SetInterp { to, .. } => format!("Set Interp {}", to.label()),
            CurveEdit::AddTrack { track, .. } => format!("Add Track {}", track.label),
            CurveEdit::RemoveTrack { track, .. } => format!("Delete Track {}", track.label),
            CurveEdit::RenameTrack { .. } => "Rename Track".into(),
        }
    }
}

fn set_key(doc: &mut CurveDoc, track: usize, index: usize, key: Key) {
    if let Some(k) = doc.tracks.get_mut(track).and_then(|t| t.keys.get_mut(index)) {
        *k = key;
    }
}

/// Doc-local undo/redo with saved-cursor dirty tracking. Same contract as
/// `GraphEditStack`: dirty is the distance from the save point, not a sticky
/// flag, and a post-undo edit that truncates the branch holding the save point
/// loses it.
#[derive(Default)]
pub struct CurveEditStack {
    undo: Vec<CurveEdit>,
    redo: Vec<CurveEdit>,
    saved: Option<usize>,
}

impl CurveEditStack {
    /// A stack for a freshly loaded (clean) document.
    pub fn new() -> Self {
        Self { undo: Vec::new(), redo: Vec::new(), saved: Some(0) }
    }

    /// Record an edit that has *already* been applied to the doc.
    pub fn record(&mut self, edit: CurveEdit) {
        if let Some(s) = self.saved {
            if s > self.undo.len() {
                self.saved = None;
            }
        }
        self.undo.push(edit);
        self.redo.clear();
    }

    pub fn undo(&mut self, doc: &mut CurveDoc) -> Option<String> {
        let edit = self.undo.pop()?;
        edit.revert(doc);
        let desc = edit.description();
        self.redo.push(edit);
        Some(desc)
    }

    pub fn redo(&mut self, doc: &mut CurveDoc) -> Option<String> {
        let edit = self.redo.pop()?;
        edit.apply(doc);
        let desc = edit.description();
        self.undo.push(edit);
        Some(desc)
    }

    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo_description(&self) -> Option<String> {
        self.undo.last().map(CurveEdit::description)
    }

    pub fn redo_description(&self) -> Option<String> {
        self.redo.last().map(CurveEdit::description)
    }

    pub fn mark_saved(&mut self) {
        self.saved = Some(self.undo.len());
    }

    pub fn is_dirty(&self) -> bool {
        self.saved != Some(self.undo.len())
    }
}

// ---------------------------------------------------------------------------
// Open document state
// ---------------------------------------------------------------------------

/// A key drag in flight. The gesture is one undo entry, so the pre-drag key is
/// held here until the release decides whether anything actually changed.
#[derive(Debug, Clone, Copy)]
pub struct KeyDrag {
    pub track: usize,
    pub index: usize,
    pub from: Key,
}

/// Which sidebar row is being renamed, and the text so far.
#[derive(Debug, Clone)]
pub struct TrackRename {
    pub index: usize,
    pub text: String,
    /// Focus is requested on the first frame only. Requesting it every frame
    /// would pin the caret to the end and make the field impossible to leave
    /// — the same first-frame flag the graph's variable draft carries.
    pub first_frame: bool,
}

/// One open `.curve` document.
pub struct CurveEditorState {
    /// Content-relative path — the tab key and the resolver key.
    pub path: String,
    pub doc: CurveDoc,
    /// Plot pan/zoom. World units are pixels at zoom 1: x = seconds ×
    /// [`PX_PER_SECOND`], y = −value × [`PX_PER_UNIT`] (screen y grows down,
    /// values grow up).
    pub view: CanvasView,
    /// The track the plot edits. Others draw dimmed and are not hit-tested,
    /// so a dense curve does not become a minefield.
    pub selected_track: usize,
    pub selected_key: Option<KeyRef>,
    pub drag: Option<KeyDrag>,
    pub rename: Option<TrackRename>,
    /// "Add track" field contents.
    pub new_track: String,
    /// A delete that needs confirming because the track has keys.
    pub confirm_delete: Option<usize>,
    pub dirty: bool,
    pub stack: CurveEditStack,
    /// When this editor last wrote the file — the hot-reload echo guard, same
    /// rule as `GraphEditorState`.
    pub last_saved_at: Option<Instant>,
    /// Transient status line (save failures, refused gestures).
    pub toast: Option<(String, Instant)>,
    /// Ask the panel to re-fit the view. Set on open and by the Frame button;
    /// the panel is the only place the viewport size is known.
    pub frame_pending: bool,
}

/// World pixels per second of curve time at zoom 1.
pub const PX_PER_SECOND: f32 = 220.0;
/// World pixels per unit of value at zoom 1.
pub const PX_PER_UNIT: f32 = 60.0;
/// Smallest gap a drag leaves between a key and its neighbour in time, so two
/// keys never stack by accident (stacking is a deliberate authoring act, and
/// this editor has no gesture for it yet).
pub const MIN_KEY_GAP: f32 = 1.0e-3;

impl CurveEditorState {
    /// Load a `.curve` from disk. `content_rel` is the tab/resolver key.
    pub fn open(abs_path: &Path, content_rel: &str) -> Result<Self, String> {
        let text = std::fs::read_to_string(abs_path)
            .map_err(|e| format!("{}: {e}", abs_path.display()))?;
        let doc = parse_curve(&text).map_err(|e| e.to_string())?;
        Ok(Self::from_doc(doc, content_rel))
    }

    pub fn from_doc(doc: CurveDoc, content_rel: &str) -> Self {
        Self {
            path: content_rel.to_string(),
            doc,
            view: CanvasView::default(),
            selected_track: 0,
            selected_key: None,
            drag: None,
            rename: None,
            new_track: String::new(),
            confirm_delete: None,
            dirty: false,
            stack: CurveEditStack::new(),
            last_saved_at: None,
            toast: None,
            frame_pending: true,
        }
    }

    /// Write the doc back to disk, canonical, clearing dirty.
    ///
    /// Canonicalizes **in memory first**, for the reason `GraphEditorState::
    /// save` snaps positions: a save writes a canonicalized clone, so leaving
    /// a different order in memory would make "clean" describe content the
    /// file does not have.
    pub fn save(&mut self, abs_path: &Path) -> Result<(), String> {
        // Settle first, always: a save cursor has to describe committed
        // content, and making that the *function's* job rather than every
        // caller's is what keeps a future third caller correct. Finalizing
        // (not reverting) keeps the edit the user was making — the rule the
        // graph editor already follows for an interrupted gesture.
        self.end_drag();
        self.rename = None;
        self.doc.canonicalize();
        let text = serialize_curve(&self.doc).map_err(|e| e.to_string())?;
        super::atomic_file::atomic_write(abs_path, &text)
            .map_err(|e| format!("{}: {e}", abs_path.display()))?;
        self.stack.mark_saved();
        self.dirty = false;
        self.last_saved_at = Some(Instant::now());
        Ok(())
    }

    /// Record an already-applied edit and refresh dirty.
    pub fn commit(&mut self, edit: CurveEdit) {
        self.stack.record(edit);
        self.dirty = self.stack.is_dirty();
    }

    pub fn undo(&mut self) {
        self.cancel_gestures();
        if let Some(d) = self.stack.undo(&mut self.doc) {
            self.toast(format!("Undo {d}"));
        }
        self.clamp_selection();
        self.dirty = self.stack.is_dirty();
    }

    pub fn redo(&mut self) {
        self.cancel_gestures();
        if let Some(d) = self.stack.redo(&mut self.doc) {
            self.toast(format!("Redo {d}"));
        }
        self.clamp_selection();
        self.dirty = self.stack.is_dirty();
    }

    /// A drag or a rename in flight. Save and undo settle these first, so a
    /// save cursor never describes content the file does not have.
    pub fn gesture_in_flight(&self) -> bool {
        self.drag.is_some() || self.rename.is_some()
    }

    /// Abandon anything half-finished, reverting the live drag's key.
    pub fn cancel_gestures(&mut self) {
        if let Some(d) = self.drag.take() {
            set_key(&mut self.doc, d.track, d.index, d.from);
        }
        self.rename = None;
        self.confirm_delete = None;
    }

    pub fn toast(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), Instant::now()));
    }

    /// Selection can outlive the thing it names (undo of an add, a track
    /// removal); collapse it rather than letting the plot index off the end.
    pub fn clamp_selection(&mut self) {
        if self.selected_track >= self.doc.tracks.len() {
            self.selected_track = self.doc.tracks.len().saturating_sub(1);
        }
        let ok = self
            .selected_key
            .is_some_and(|(t, k)| self.doc.tracks.get(t).is_some_and(|tr| k < tr.keys.len()));
        if !ok {
            self.selected_key = None;
        }
    }

    // -- editing ------------------------------------------------------------

    /// Insert a key at `(t, value)` on `track`, keeping the key list sorted.
    /// Returns the new key's index.
    pub fn add_key(&mut self, track: usize, t: f32, value: f32) -> Option<usize> {
        let tr = self.doc.tracks.get(track)?;
        let t = t.max(0.0);
        // Sorted insert: the position of the first key strictly later than
        // `t`, so a key dropped onto an existing time lands after it (the
        // stacked-key step convention `Track::sample` documents).
        let index = tr.keys.partition_point(|k| k.t <= t);
        // Inherit the mode of the segment being split — adding a key to a
        // cubic curve must not silently straighten it.
        let interp = tr
            .keys
            .get(index.saturating_sub(1))
            .or_else(|| tr.keys.first())
            .map(|k| k.interp)
            .unwrap_or_default();
        // A new key inherits its neighbour's interpolation and starts on the
        // default tangent: nobody has shaped it yet, and Auto is the mode that
        // says so (v2).
        let key = Key { t, value, interp, tangent: Default::default() };
        let edit = CurveEdit::AddKey { track, index, key };
        edit.apply(&mut self.doc);
        self.commit(edit);
        self.selected_key = Some((track, index));
        Some(index)
    }

    pub fn remove_key(&mut self, track: usize, index: usize) {
        let Some(key) = self.doc.tracks.get(track).and_then(|t| t.keys.get(index)).copied() else {
            return;
        };
        let edit = CurveEdit::RemoveKey { track, index, key };
        edit.apply(&mut self.doc);
        self.commit(edit);
        self.selected_key = None;
    }

    /// Right-click cycle: Constant → Linear → Cubic → Constant.
    pub fn cycle_interp(&mut self, track: usize, index: usize) {
        let Some(k) = self.doc.tracks.get(track).and_then(|t| t.keys.get(index)).copied() else {
            return;
        };
        let edit =
            CurveEdit::SetInterp { track, index, from: k.interp, to: k.interp.next() };
        edit.apply(&mut self.doc);
        let label = k.interp.next().label();
        self.commit(edit);
        self.toast(format!("Interp: {label}"));
    }

    /// Begin a drag on an existing key.
    pub fn begin_drag(&mut self, track: usize, index: usize) {
        let Some(from) = self.doc.tracks.get(track).and_then(|t| t.keys.get(index)).copied()
        else {
            return;
        };
        self.drag = Some(KeyDrag { track, index, from });
        self.selected_key = Some((track, index));
    }

    /// Live drag update — writes straight into the doc (no undo entry yet);
    /// `t` is clamped between the neighbours so the key order, and therefore
    /// every index the stack holds, survives the gesture.
    pub fn drag_to(&mut self, t: f32, value: f32) {
        let Some(d) = self.drag else { return };
        let Some(tr) = self.doc.tracks.get(d.track) else { return };
        let lo = if d.index == 0 {
            0.0
        } else {
            tr.keys[d.index - 1].t + MIN_KEY_GAP
        };
        let hi = tr
            .keys
            .get(d.index + 1)
            .map(|k| k.t - MIN_KEY_GAP)
            .unwrap_or(f32::MAX);
        // A neighbour closer than the gap on both sides leaves an empty range;
        // holding position is the only honest answer.
        let t = if lo <= hi { t.clamp(lo, hi) } else { d.from.t };
        // A move changes where the key is, never what it is: interpolation and
        // the hand-shaped tangent both ride along untouched.
        set_key(
            &mut self.doc,
            d.track,
            d.index,
            Key { t, value, interp: d.from.interp, tangent: d.from.tangent },
        );
    }

    /// Finish the drag: one `MoveKey` entry, or nothing at all if the key
    /// ended where it started (a click that happened to jitter is not an edit).
    pub fn end_drag(&mut self) {
        let Some(d) = self.drag.take() else { return };
        let Some(to) = self.doc.tracks.get(d.track).and_then(|t| t.keys.get(d.index)).copied()
        else {
            return;
        };
        if to == d.from {
            return;
        }
        self.commit(CurveEdit::MoveKey { track: d.track, index: d.index, from: d.from, to });
    }

    /// Append a track named `label`. Slug is derived and de-duplicated by
    /// `CurveDoc::free_slug`, so the Timeline pin it becomes is an identifier.
    pub fn add_track(&mut self, label: &str) -> Option<usize> {
        let label = label.trim();
        if label.is_empty() {
            self.toast("Track needs a name");
            return None;
        }
        let slug = self.doc.free_slug(label);
        let index = self.doc.tracks.len();
        let edit = CurveEdit::AddTrack { index, track: Track::new(&slug, label) };
        edit.apply(&mut self.doc);
        self.commit(edit);
        self.selected_track = index;
        self.selected_key = None;
        Some(index)
    }

    pub fn remove_track(&mut self, index: usize) {
        let Some(track) = self.doc.tracks.get(index).cloned() else {
            return;
        };
        let edit = CurveEdit::RemoveTrack { index, track };
        edit.apply(&mut self.doc);
        self.commit(edit);
        self.clamp_selection();
        self.selected_key = None;
    }

    /// Rename the *label* only. The slug is the Timeline's pin and stays put —
    /// that separation is the whole reason `Track` carries both.
    pub fn rename_track(&mut self, index: usize, to: &str) {
        let to = to.trim();
        let Some(from) = self.doc.tracks.get(index).map(|t| t.label.clone()) else {
            return;
        };
        if to.is_empty() || to == from {
            return;
        }
        let edit = CurveEdit::RenameTrack { index, from, to: to.to_string() };
        edit.apply(&mut self.doc);
        self.commit(edit);
    }

    // -- view ---------------------------------------------------------------

    /// Ask for a re-fit on the next draw. The fit itself lives in the panel,
    /// which is the only place that knows how big the plot is.
    pub fn frame_all(&mut self) {
        self.frame_pending = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc() -> CurveDoc {
        let mut d = CurveDoc::default();
        let mut t = Track::new("height", "Height");
        t.keys = vec![
            Key { t: 0.0, value: 0.0, interp: Interp::Linear, tangent: Default::default() },
            Key { t: 1.0, value: 10.0, interp: Interp::Linear, tangent: Default::default() },
            Key { t: 2.0, value: 0.0, interp: Interp::Linear, tangent: Default::default() },
        ];
        d.tracks = vec![t];
        d
    }

    fn state() -> CurveEditorState {
        CurveEditorState::from_doc(doc(), "curves/test.curve")
    }

    #[test]
    fn add_key_lands_sorted_and_inherits_the_split_segment() {
        let mut s = state();
        s.doc.tracks[0].keys[0].interp = Interp::Cubic;
        let i = s.add_key(0, 0.5, 5.0).expect("added");
        assert_eq!(i, 1);
        let ts: Vec<f32> = s.doc.tracks[0].keys.iter().map(|k| k.t).collect();
        assert_eq!(ts, vec![0.0, 0.5, 1.0, 2.0]);
        assert_eq!(s.doc.tracks[0].keys[1].interp, Interp::Cubic, "splitting cubic stays cubic");
        assert!(s.dirty);
    }

    #[test]
    fn a_drag_is_one_undo_entry_and_cannot_reorder_keys() {
        let mut s = state();
        s.begin_drag(0, 1);
        // Way past both neighbours, in both directions, then settle.
        s.drag_to(99.0, 3.0);
        assert!(s.doc.tracks[0].keys[1].t < s.doc.tracks[0].keys[2].t);
        s.drag_to(-99.0, 3.0);
        assert!(s.doc.tracks[0].keys[1].t > s.doc.tracks[0].keys[0].t);
        s.drag_to(1.5, 3.0);
        s.end_drag();
        assert_eq!(s.stack.undo_len(), 1, "one gesture, one entry");
        assert_eq!(s.doc.tracks[0].keys[1], Key { t: 1.5, value: 3.0, interp: Interp::Linear, tangent: Default::default() });

        s.undo();
        assert_eq!(s.doc.tracks[0].keys[1].t, 1.0);
        assert_eq!(s.doc.tracks[0].keys[1].value, 10.0);
        assert!(!s.dirty, "back at the save point");
    }

    #[test]
    fn a_drag_that_moves_nothing_records_nothing() {
        let mut s = state();
        s.begin_drag(0, 1);
        s.drag_to(1.0, 10.0);
        s.end_drag();
        assert_eq!(s.stack.undo_len(), 0);
        assert!(!s.dirty);
    }

    #[test]
    fn interp_cycles_and_reverts() {
        let mut s = state();
        s.cycle_interp(0, 0);
        assert_eq!(s.doc.tracks[0].keys[0].interp, Interp::Cubic);
        s.cycle_interp(0, 0);
        assert_eq!(s.doc.tracks[0].keys[0].interp, Interp::Constant);
        s.undo();
        s.undo();
        assert_eq!(s.doc.tracks[0].keys[0].interp, Interp::Linear);
    }

    #[test]
    fn tracks_add_rename_remove_round_trip_through_undo() {
        let mut s = state();
        s.add_track("Duck Height").expect("added");
        assert_eq!(s.doc.tracks[1].slug, "duck_height");
        s.rename_track(1, "Hop");
        assert_eq!(s.doc.tracks[1].label, "Hop");
        assert_eq!(s.doc.tracks[1].slug, "duck_height", "renaming a label never moves the pin");
        s.remove_track(1);
        assert_eq!(s.doc.tracks.len(), 1);

        s.undo();
        assert_eq!(s.doc.tracks[1].label, "Hop");
        s.undo();
        assert_eq!(s.doc.tracks[1].label, "Duck Height");
        s.undo();
        assert_eq!(s.doc.tracks.len(), 1);
        assert!(!s.dirty);
    }

    /// The whole acceptance gesture chain, undone back to the save point.
    #[test]
    fn undo_walks_the_whole_session_back_to_clean() {
        let mut s = state();
        let before = s.doc.clone();
        s.add_key(0, 0.5, 5.0);
        s.begin_drag(0, 1);
        s.drag_to(0.6, 6.0);
        s.end_drag();
        s.cycle_interp(0, 1);
        s.add_track("Lean");
        assert!(s.dirty);
        for _ in 0..4 {
            s.undo();
        }
        assert_eq!(s.doc, before);
        assert!(!s.dirty);
        assert!(!s.stack.can_undo());

        // …and forward again.
        for _ in 0..4 {
            s.redo();
        }
        assert!(s.dirty);
        assert_eq!(s.doc.tracks.len(), 2);
    }

    #[test]
    fn removing_a_track_clears_a_selection_that_named_it() {
        let mut s = state();
        s.add_track("Lean");
        s.selected_key = Some((1, 0));
        s.remove_track(1);
        assert_eq!(s.selected_track, 0);
        assert_eq!(s.selected_key, None);
    }

    #[test]
    fn save_writes_canonical_text_and_clears_dirty() {
        let dir = std::env::temp_dir().join(format!("curve_editor_{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("tmp dir");
        let path = dir.join("t.curve");

        let mut s = state();
        // Deliberately out of order in memory, the way a drag could leave it
        // if clamping were ever relaxed.
        s.doc.tracks[0].keys.swap(0, 2);
        s.add_key(0, 0.5, 5.0);
        s.save(&path).expect("save");
        assert!(!s.dirty);

        let text = std::fs::read_to_string(&path).expect("read");
        let back = parse_curve(&text).expect("parse");
        assert_eq!(back, s.doc, "in-memory doc matches what was written");
        let ts: Vec<f32> = back.tracks[0].keys.iter().map(|k| k.t).collect();
        let mut sorted = ts.clone();
        sorted.sort_by(f32::total_cmp);
        assert_eq!(ts, sorted, "saved keys are canonical");

        // Reopening is a fixed point.
        let reopened = CurveEditorState::open(&path, "t.curve").expect("open");
        assert_eq!(reopened.doc, back);
        let _ = std::fs::remove_file(&path);
    }

    /// The package's acceptance gesture chain, on the shipped demo asset:
    /// open, add a key, drag it, cycle its interp, add a track, undo all of
    /// it, save — and the file that comes back is byte-identical to the
    /// canonical form of what went in. Skipped where the content tree is
    /// absent (a packaged build), because the claim is about the asset.
    #[test]
    fn duck_hop_survives_the_whole_acceptance_chain() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../content/curves/duck_hop.curve");
        if !src.exists() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("p8b_duck_{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("tmp dir");
        let work = dir.join("duck_hop.curve");
        std::fs::copy(&src, &work).expect("copy");

        let mut s = CurveEditorState::open(&work, "curves/duck_hop.curve").expect("open");
        let before = s.doc.clone();
        assert_eq!(s.doc.slugs(), vec!["height", "lean"]);
        assert!(!s.dirty);

        s.add_key(0, 0.35, 1.8).expect("add");
        let (t, k) = s.selected_key.expect("the new key is selected");
        s.begin_drag(t, k);
        s.drag_to(0.4, 2.0);
        s.end_drag();
        s.cycle_interp(t, k);
        s.add_track("Squash").expect("add track");
        assert!(s.dirty);
        assert_eq!(s.stack.undo_len(), 4);

        // Saving *now* is what a Timeline sees: the new track is a new pin.
        s.save(&work).expect("save");
        assert!(!s.dirty);
        let mid = parse_curve(&std::fs::read_to_string(&work).expect("read")).expect("parse");
        assert_eq!(mid.slugs(), vec!["height", "lean", "squash"]);

        // …and undoing back to the top leaves the original document, saved
        // canonically. `dirty` is true because the save point is now behind us.
        for _ in 0..4 {
            s.undo();
        }
        assert_eq!(s.doc, before);
        assert!(s.dirty, "we are four edits away from the save cursor");
        s.save(&work).expect("save");

        let reopened = CurveEditorState::open(&work, "curves/duck_hop.curve").expect("reopen");
        assert_eq!(reopened.doc, before, "round trip is a fixed point");
        assert_eq!(
            std::fs::read_to_string(&work).expect("read"),
            serialize_curve(&before).expect("ser"),
            "what is on disk is the canonical form"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cancelling_a_drag_puts_the_key_back() {
        let mut s = state();
        s.begin_drag(0, 1);
        s.drag_to(1.4, 44.0);
        s.cancel_gestures();
        assert_eq!(s.doc.tracks[0].keys[1].value, 10.0);
        assert_eq!(s.stack.undo_len(), 0);
    }
}
