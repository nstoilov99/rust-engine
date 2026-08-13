//! Curve editor per-document state (Task 45-A P8b; restructured in GS-5b).
//!
//! Holds one open `.curve` document, its canvas view, selection and a
//! doc-local undo/redo stack — the same shape `graph_editor` gives a `.graph`,
//! down to the saved-cursor dirty rule. The drawing/interaction layer is
//! `curve_editor_crusty`.
//!
//! GS-5b turns the P8b baseline into the surface the design package asks for:
//! a **multi-key selection** (box select, Shift-extend, group drag, group
//! delete), per-key **tangent modes** on top of the v2 schema (GS-5a), a
//! **footer detail bar** whose state is derived here rather than in the
//! painter, **snapping** with a temporary Ctrl inversion, and a **playhead**
//! whose readouts come from `curve_asset`'s own evaluation.
//!
//! The rule that keeps the two layers honest: everything that can be decided
//! without pixels is decided here and tested here. The panel draws what these
//! functions say.
//!
//! Sampling is never reimplemented: everything drawn comes out of
//! [`curve_asset::Track::sample`], the function the interpreter calls.

use std::collections::HashSet;
use std::path::Path;
use std::time::Instant;

use crusty_gui::widgets::CanvasView;
use curve_asset::{parse_curve, serialize_curve, CurveDoc, Interp, Key, Tangent, Track};

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
    /// A key's tangent mode (v2). One entry per gesture, whatever the arm
    /// drag passed through on the way.
    SetTangent { track: usize, index: usize, from: Tangent, to: Tangent },
    AddTrack { index: usize, track: Track },
    RemoveTrack { index: usize, track: Track },
    RenameTrack { index: usize, from: String, to: String },
    /// Several edits that were **one gesture**: a group drag, a group delete,
    /// a mode applied to a whole selection. Applied in order and reverted in
    /// reverse, so a batch of removals (built high index first) reinserts low
    /// index first and lands every key back where it was.
    Batch { label: String, edits: Vec<CurveEdit> },
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
            CurveEdit::SetTangent { track, index, to, .. } => {
                if let Some(k) = doc.tracks.get_mut(*track).and_then(|t| t.keys.get_mut(*index)) {
                    k.tangent = *to;
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
            CurveEdit::Batch { edits, .. } => {
                for e in edits {
                    e.apply(doc);
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
            CurveEdit::SetTangent { track, index, from, to } => {
                CurveEdit::SetTangent { track: *track, index: *index, from: *to, to: *from }
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
            CurveEdit::Batch { edits, .. } => {
                for e in edits.iter().rev() {
                    e.revert(doc);
                }
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
            CurveEdit::SetTangent { to, .. } => format!("Set Tangent {}", to.label()),
            CurveEdit::AddTrack { track, .. } => format!("Add Track {}", track.label),
            CurveEdit::RemoveTrack { track, .. } => format!("Delete Track {}", track.label),
            CurveEdit::RenameTrack { .. } => "Rename Track".into(),
            CurveEdit::Batch { label, .. } => label.clone(),
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
// Gestures in flight
// ---------------------------------------------------------------------------

/// A key drag in flight. The gesture is one undo entry, so every dragged key's
/// pre-drag state is held here until the release decides what actually moved.
#[derive(Debug, Clone)]
pub struct KeyDrag {
    pub track: usize,
    /// The key under the pointer when the drag started. Snapping resolves
    /// against *this* key and the rest of the selection follows — a body that
    /// snapped per key would shear.
    pub anchor: usize,
    /// `(index, pre-drag key)` for every key in the gesture, ascending.
    pub from: Vec<(usize, Key)>,
    /// Where the pointer grabbed, in curve units.
    pub grab: (f32, f32),
}

/// Which side of a key an arm drag is shaping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArmSide {
    In,
    Out,
}

/// A tangent-arm drag in flight — one gesture, one "Edit Tangent" entry.
#[derive(Debug, Clone, Copy)]
pub struct ArmDrag {
    pub track: usize,
    pub index: usize,
    pub side: ArmSide,
    /// The mode the key had before the gesture, including before any
    /// auto-promotion — so Esc and Undo both land back on (say) `Auto`.
    pub from: Tangent,
}

/// A box select in flight, in curve units so a mid-gesture zoom cannot warp it.
#[derive(Debug, Clone, Copy)]
pub struct Marquee {
    pub start: (f32, f32),
    pub cur: (f32, f32),
    /// Shift was held at press: the sweep adds rather than replaces. Captured
    /// at press, like the graph canvas' marquee — releasing the modifier
    /// mid-drag must not change what the gesture means.
    pub add: bool,
    /// Past the movement threshold. Below it the gesture is still a click.
    pub armed: bool,
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

// ---------------------------------------------------------------------------
// Tangent modes as the UI names them
// ---------------------------------------------------------------------------

/// The five states the footer's Tangent control shows and the key menu sets.
///
/// [`Tangent`] carries *slopes*; this carries only the **mode**, which is what
/// a segmented control compares and what a menu row names. `Linear` is a full
/// segment here (not just the Straighten action's output): a control that
/// shows state has to be able to show every state it can be in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TangentMode {
    Auto,
    User,
    Break,
    Flat,
    Linear,
}

impl TangentMode {
    /// Footer order — the mockup's, and the one the menu's numeric shortcuts
    /// follow for its first four.
    pub const ALL: [TangentMode; 5] = [
        TangentMode::Auto,
        TangentMode::User,
        TangentMode::Break,
        TangentMode::Flat,
        TangentMode::Linear,
    ];

    pub fn label(self) -> &'static str {
        match self {
            TangentMode::Auto => "Auto",
            TangentMode::User => "User",
            TangentMode::Break => "Break",
            TangentMode::Flat => "Flat",
            TangentMode::Linear => "Linear",
        }
    }

    pub fn of(t: &Tangent) -> TangentMode {
        match t {
            Tangent::Auto => TangentMode::Auto,
            Tangent::Flat => TangentMode::Flat,
            Tangent::Linear => TangentMode::Linear,
            Tangent::User { .. } => TangentMode::User,
            Tangent::Break { .. } => TangentMode::Break,
        }
    }

    /// Does a key in this mode draw *solid* arms? The explicit modes do; the
    /// derived ones (Auto/Flat/Linear) show a ghost arm when selected, which
    /// is the affordance that makes promotion discoverable.
    pub fn has_arms(self) -> bool {
        matches!(self, TangentMode::User | TangentMode::Break)
    }
}

/// The tangent a key takes when `mode` is applied to it **now**.
///
/// The explicit modes are seeded from the slope the key already has, which is
/// what makes "switch to User" a no-op you can then drag from — GS-5a made the
/// units (value per second) line up so this is true by construction rather
/// than by a fudge factor.
pub fn tangent_for_mode(track: &Track, index: usize, mode: TangentMode) -> Tangent {
    match mode {
        TangentMode::Auto => Tangent::Auto,
        TangentMode::Flat => Tangent::Flat,
        TangentMode::Linear => Tangent::Linear,
        TangentMode::User => Tangent::User { tangent: track.out_tangent(index) },
        TangentMode::Break => Tangent::Break {
            in_tan: track.in_tangent(index),
            out_tan: track.out_tangent(index),
        },
    }
}

/// Key shape — mode is the glyph, colour stays the track's, selection is the
/// ring. Derived here so the painter has no policy in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyGlyph {
    /// Auto, and the two explicit modes (their arms carry the distinction).
    Circle,
    /// Flat — a level hold.
    Square,
    /// Linear — a mechanical ramp.
    Diamond,
}

pub fn key_glyph(t: &Tangent) -> KeyGlyph {
    match TangentMode::of(t) {
        TangentMode::Flat => KeyGlyph::Square,
        TangentMode::Linear => KeyGlyph::Diamond,
        _ => KeyGlyph::Circle,
    }
}

// ---------------------------------------------------------------------------
// Footer detail bar state
// ---------------------------------------------------------------------------

/// What the footer shows for the current selection. `None` is the design's
/// "—": the selection disagrees, and typing into the field sets them all.
#[derive(Debug, Clone, PartialEq)]
pub struct FooterState {
    pub count: usize,
    pub t: Option<f32>,
    pub value: Option<f32>,
    pub interp: Option<Interp>,
    pub tangent: Option<TangentMode>,
    /// Tangents govern cubic segments only, so the control greys out when no
    /// selected key touches one — the schema's rule, said in the UI.
    pub tangent_enabled: bool,
}

impl FooterState {
    pub fn empty() -> Self {
        Self {
            count: 0,
            t: None,
            value: None,
            interp: None,
            tangent: None,
            tangent_enabled: false,
        }
    }

    /// Mixed *and* non-empty — the state that earns the warning tag.
    pub fn tangent_mixed(&self) -> bool {
        self.count > 0 && self.tangent.is_none()
    }

    pub fn interp_mixed(&self) -> bool {
        self.count > 0 && self.interp.is_none()
    }
}

/// Which footer field the keyboard is in. Held across frames so the panel
/// knows when it may overwrite a buffer with the selection's own numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FooterField {
    Time,
    Value,
}

/// Does key `index` touch a cubic segment? The honest predicate for "do
/// tangents do anything here": the key's own `interp` governs the segment
/// leaving it, the previous key's the one arriving.
pub fn touches_cubic(track: &Track, index: usize) -> bool {
    let out = track.keys.get(index).is_some_and(|k| k.interp == Interp::Cubic);
    let inc = index
        .checked_sub(1)
        .and_then(|p| track.keys.get(p))
        .is_some_and(|k| k.interp == Interp::Cubic);
    out || inc
}

// ---------------------------------------------------------------------------
// Snapping
// ---------------------------------------------------------------------------

/// Snap increments. Toolbar toggles own the *resting* state; Ctrl inverts it
/// for the duration of a gesture.
pub const SNAP_TIME: f32 = 0.1;
pub const SNAP_VALUE: f32 = 0.05;

/// The resting toggles, live for every gesture until they are changed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SnapSettings {
    pub time: bool,
    pub value: bool,
}

impl SnapSettings {
    /// What the gesture actually does: Ctrl **inverts** each toggle rather
    /// than forcing snapping on, so the modifier means the same thing in both
    /// resting states (contract rule 4 — spatial/bulk modifier).
    pub fn effective(self, ctrl: bool) -> SnapSettings {
        SnapSettings { time: self.time != ctrl, value: self.value != ctrl }
    }
}

pub fn snap_to(v: f32, step: f32) -> f32 {
    if step <= 0.0 {
        return v;
    }
    (v / step).round() * step
}

// ---------------------------------------------------------------------------
// Open document state
// ---------------------------------------------------------------------------

/// One open `.curve` document.
pub struct CurveEditorState {
    /// Content-relative path — the tab key and the resolver key.
    pub path: String,
    pub doc: CurveDoc,
    /// Plot pan/zoom. World units are pixels at zoom 1: x = seconds ×
    /// [`PX_PER_SECOND`], y = −value × [`PX_PER_UNIT`] (screen y grows down,
    /// values grow up).
    pub view: CanvasView,
    /// World pixels per unit of **value**, at zoom 1 — the plot's vertical
    /// scale, chosen per document by a fit rather than fixed.
    ///
    /// A canvas has one zoom for both axes, and curve data does not: six
    /// seconds against a value range of three would leave a fit using a
    /// quarter of the plot's height. Framing therefore picks a value scale as
    /// well as a zoom, which is also what makes "Fit track" mean something on
    /// a document whose tracks live at different magnitudes.
    pub value_px: f32,
    /// The track the plot edits. Others draw dimmed and are not hit-tested,
    /// so a dense curve does not become a minefield.
    pub selected_track: usize,
    /// Selected key indices **on `selected_track`**, ascending and unique.
    /// Selection is single-track by construction: only the selected track is
    /// hit-tested, which is the baseline vocabulary GS-5b keeps.
    pub selection: Vec<usize>,
    pub drag: Option<KeyDrag>,
    pub arm: Option<ArmDrag>,
    pub marquee: Option<Marquee>,
    pub rename: Option<TrackRename>,
    /// "Add track" field contents, and whether the field is showing at all —
    /// the sidebar foot is a `+ Track` action until it is asked for a name,
    /// so a list of three rows is not outweighed by the way to make a fourth.
    pub new_track: String,
    pub adding_track: bool,
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
    /// Fit the *selected track* rather than the whole document.
    pub frame_track_only: bool,
    pub snap: SnapSettings,
    /// Footer field buffers. The panel refreshes them from the selection on
    /// every frame the field is *not* being typed into — which is what
    /// `field_focus` (last frame's answer) is for.
    pub field_t: String,
    pub field_v: String,
    pub field_focus: Option<FooterField>,
    /// Where the key context menu was opened, in screen pixels. `Some` for as
    /// long as it is open, so it survives the frames after the click.
    pub menu_anchor: Option<(f32, f32)>,
    /// Preview cursor, in seconds. Never played back — GS-5b ships no
    /// transport; this is the "what does the runtime say at this time" probe.
    pub playhead: f32,
    pub playhead_drag: bool,
    /// View state, by slug so it survives reordering, and deliberately **not**
    /// serialized: hiding a track is how you read a plot, not what the asset
    /// says.
    hidden: HashSet<String>,
    locked: HashSet<String>,
}

/// World pixels per second of curve time at zoom 1.
pub const PX_PER_SECOND: f32 = 220.0;
/// The value scale a document starts at, before anything has been framed.
pub const PX_PER_UNIT: f32 = 60.0;
/// Bounds on the fitted value scale, so a degenerate document (one key, or a
/// range of 1e-9) cannot produce a mapping that overflows the plot maths.
pub const VALUE_PX_MIN: f32 = 4.0;
pub const VALUE_PX_MAX: f32 = 4000.0;
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
            value_px: PX_PER_UNIT,
            selected_track: 0,
            selection: Vec::new(),
            drag: None,
            arm: None,
            marquee: None,
            rename: None,
            new_track: String::new(),
            adding_track: false,
            confirm_delete: None,
            dirty: false,
            stack: CurveEditStack::new(),
            last_saved_at: None,
            toast: None,
            frame_pending: true,
            frame_track_only: false,
            snap: SnapSettings::default(),
            field_t: String::new(),
            field_v: String::new(),
            field_focus: None,
            menu_anchor: None,
            playhead: 0.0,
            playhead_drag: false,
            hidden: HashSet::new(),
            locked: HashSet::new(),
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
        self.end_arm_drag();
        self.marquee = None;
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

    /// Record a group of already-applied edits as **one** undo entry. An empty
    /// group is not a gesture and records nothing.
    ///
    /// A single-edit group is still wrapped: the *label* is the gesture's, and
    /// "Flatten" must not turn into "Set Tangent Flat" in the Edit menu just
    /// because one key was selected.
    fn commit_batch(&mut self, label: impl Into<String>, edits: Vec<CurveEdit>) {
        if edits.is_empty() {
            return;
        }
        self.commit(CurveEdit::Batch { label: label.into(), edits });
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

    /// A gesture in flight. Save and undo settle these first, so a save cursor
    /// never describes content the file does not have.
    pub fn gesture_in_flight(&self) -> bool {
        self.drag.is_some()
            || self.arm.is_some()
            || self.marquee.is_some()
            || self.rename.is_some()
    }

    /// Abandon anything half-finished, reverting the live drag's keys and the
    /// live arm's mode.
    pub fn cancel_gestures(&mut self) {
        if let Some(d) = self.drag.take() {
            for (i, k) in d.from {
                set_key(&mut self.doc, d.track, i, k);
            }
        }
        if let Some(a) = self.arm.take() {
            self.write_tangent(a.track, a.index, a.from);
        }
        self.marquee = None;
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
        let n = self
            .doc
            .tracks
            .get(self.selected_track)
            .map(|t| t.keys.len())
            .unwrap_or(0);
        self.selection.retain(|i| *i < n);
    }

    // -- selection ----------------------------------------------------------

    pub fn has_selection(&self) -> bool {
        !self.selection.is_empty()
    }

    /// The first selected key, as a `(track, index)` address — what a single
    /// -key caller (tooltip, tests, the host's Edit menu) asks for.
    pub fn selected_key(&self) -> Option<KeyRef> {
        self.selection.first().map(|i| (self.selected_track, *i))
    }

    pub fn is_selected(&self, index: usize) -> bool {
        self.selection.contains(&index)
    }

    pub fn clear_selection(&mut self) {
        self.selection.clear();
    }

    pub fn select_only(&mut self, index: usize) {
        self.selection.clear();
        self.selection.push(index);
    }

    /// Shift-click: extend, or drop the key if it was already in.
    pub fn toggle_select(&mut self, index: usize) {
        match self.selection.iter().position(|i| *i == index) {
            Some(p) => {
                self.selection.remove(p);
            }
            None => {
                self.selection.push(index);
                self.selection.sort_unstable();
            }
        }
    }

    fn set_selection(&mut self, mut keys: Vec<usize>) {
        keys.sort_unstable();
        keys.dedup();
        self.selection = keys;
    }

    /// Commit a box select. `add` extends the existing selection, otherwise
    /// the sweep replaces it — including with nothing, which is how an empty
    /// sweep deselects.
    pub fn select_in_box(&mut self, t: (f32, f32), value: (f32, f32), add: bool) {
        let (t0, t1) = (t.0.min(t.1), t.0.max(t.1));
        let (v0, v1) = (value.0.min(value.1), value.0.max(value.1));
        let Some(track) = self.doc.tracks.get(self.selected_track) else {
            return;
        };
        let hits: Vec<usize> = track
            .keys
            .iter()
            .enumerate()
            .filter(|(_, k)| k.t >= t0 && k.t <= t1 && k.value >= v0 && k.value <= v1)
            .map(|(i, _)| i)
            .collect();
        let mut next = if add { self.selection.clone() } else { Vec::new() };
        next.extend(hits);
        self.set_selection(next);
    }

    // -- track view state ---------------------------------------------------

    fn slug(&self, index: usize) -> Option<&str> {
        self.doc.tracks.get(index).map(|t| t.slug.as_str())
    }

    pub fn is_hidden(&self, index: usize) -> bool {
        self.slug(index).is_some_and(|s| self.hidden.contains(s))
    }

    pub fn is_locked(&self, index: usize) -> bool {
        self.slug(index).is_some_and(|s| self.locked.contains(s))
    }

    pub fn toggle_hidden(&mut self, index: usize) {
        if let Some(s) = self.slug(index).map(str::to_string) {
            if !self.hidden.remove(&s) {
                self.hidden.insert(s);
            }
        }
    }

    pub fn toggle_locked(&mut self, index: usize) {
        if let Some(s) = self.slug(index).map(str::to_string) {
            if !self.locked.remove(&s) {
                self.locked.insert(s);
            }
        }
    }

    /// Can this track be edited at all? A locked track still draws and still
    /// samples under the playhead — it just refuses every write, once, with a
    /// reason.
    pub fn editable(&mut self, index: usize) -> bool {
        if self.is_locked(index) {
            let label = self
                .doc
                .tracks
                .get(index)
                .map(|t| t.label.clone())
                .unwrap_or_default();
            self.toast(format!("'{label}' is locked"));
            return false;
        }
        true
    }

    // -- editing ------------------------------------------------------------

    /// Insert a key at `(t, value)` on `track`, keeping the key list sorted.
    /// Returns the new key's index.
    pub fn add_key(&mut self, track: usize, t: f32, value: f32) -> Option<usize> {
        if !self.editable(track) {
            return None;
        }
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
        let key = Key { t, value, interp, tangent: Tangent::Auto };
        let edit = CurveEdit::AddKey { track, index, key };
        edit.apply(&mut self.doc);
        self.commit(edit);
        self.selected_track = track;
        self.select_only(index);
        Some(index)
    }

    /// Delete every selected key — one undo entry, whatever the count.
    ///
    /// Removals are built **high index first** so each one is valid as it is
    /// applied; reverting a batch walks backwards, which reinserts low index
    /// first and lands every key exactly where it was.
    pub fn delete_selection(&mut self) {
        if self.selection.is_empty() {
            return;
        }
        let track = self.selected_track;
        if !self.editable(track) {
            return;
        }
        let mut indices = self.selection.clone();
        indices.sort_unstable_by(|a, b| b.cmp(a));
        let mut edits = Vec::new();
        for index in indices {
            let Some(key) = self.doc.tracks.get(track).and_then(|t| t.keys.get(index)).copied()
            else {
                continue;
            };
            let edit = CurveEdit::RemoveKey { track, index, key };
            edit.apply(&mut self.doc);
            edits.push(edit);
        }
        let label = if edits.len() == 1 { "Delete Key" } else { "Delete Keys" };
        self.commit_batch(label, edits);
        self.selection.clear();
    }

    /// Apply an interpolation mode to the whole selection — one entry.
    pub fn set_selection_interp(&mut self, to: Interp) {
        let track = self.selected_track;
        if self.selection.is_empty() || !self.editable(track) {
            return;
        }
        let mut edits = Vec::new();
        for index in self.selection.clone() {
            let Some(from) = self.doc.tracks.get(track).and_then(|t| t.keys.get(index)).map(|k| k.interp)
            else {
                continue;
            };
            if from == to {
                continue;
            }
            let edit = CurveEdit::SetInterp { track, index, from, to };
            edit.apply(&mut self.doc);
            edits.push(edit);
        }
        let n = edits.len();
        self.commit_batch(format!("Set Interp {}", to.label()), edits);
        if n > 0 {
            self.toast(format!("Interp: {}", to.label()));
        }
    }

    /// Apply a tangent mode to the whole selection — one entry, and the
    /// explicit modes are seeded from each key's current slope so the curve
    /// does not jump when you switch to a mode you are about to shape.
    pub fn set_selection_tangent(&mut self, mode: TangentMode) {
        self.apply_tangent_mode(mode, mode.label().to_string());
    }

    /// Level the selection: the "Flatten" action, which is the Flat mode.
    pub fn flatten_selection(&mut self) {
        self.apply_tangent_mode(TangentMode::Flat, "Flatten".to_string());
    }

    /// Straighten the selection: the Linear mode, so a cubic segment between
    /// two straightened keys is genuinely a straight line (GS-5a's reason for
    /// making this a tangent rather than an `interp` change).
    pub fn straighten_selection(&mut self) {
        self.apply_tangent_mode(TangentMode::Linear, "Straighten".to_string());
    }

    fn apply_tangent_mode(&mut self, mode: TangentMode, label: String) {
        let track = self.selected_track;
        if self.selection.is_empty() || !self.editable(track) {
            return;
        }
        let mut edits = Vec::new();
        for index in self.selection.clone() {
            let Some(tr) = self.doc.tracks.get(track) else { continue };
            let Some(from) = tr.keys.get(index).map(|k| k.tangent) else { continue };
            let to = tangent_for_mode(tr, index, mode);
            if from == to {
                continue;
            }
            let edit = CurveEdit::SetTangent { track, index, from, to };
            edit.apply(&mut self.doc);
            edits.push(edit);
        }
        let n = edits.len();
        self.commit_batch(label.clone(), edits);
        if n > 0 {
            self.toast(format!("Tangent: {label}"));
        }
    }

    /// Type a time into the footer: every selected key lands on it, clamped
    /// against the keys that are *not* moving. One entry.
    pub fn set_selection_time(&mut self, t: f32) {
        self.write_selection("Set Time", Some(t.max(0.0)), None);
    }

    /// Type a value into the footer. Values have no neighbour constraint —
    /// two keys may share one, and often should.
    pub fn set_selection_value(&mut self, value: f32) {
        self.write_selection("Set Value", None, Some(value));
    }

    /// Shared body for the footer's two fields: write `t` and/or `value` into
    /// every selected key, clamping time against unselected neighbours only.
    fn write_selection(&mut self, label: &str, t: Option<f32>, value: Option<f32>) {
        let track = self.selected_track;
        if self.selection.is_empty() || !self.editable(track) {
            return;
        }
        let selected: HashSet<usize> = self.selection.iter().copied().collect();
        let mut edits = Vec::new();
        for index in self.selection.clone() {
            let Some(tr) = self.doc.tracks.get(track) else { continue };
            let Some(from) = tr.keys.get(index).copied() else { continue };
            let (lo, hi) = free_span(tr, index, &selected);
            let mut to = from;
            if let Some(t) = t {
                to.t = if lo <= hi { t.clamp(lo, hi) } else { from.t };
            }
            if let Some(v) = value {
                to.value = v;
            }
            if to == from {
                continue;
            }
            let edit = CurveEdit::MoveKey { track, index, from, to };
            edit.apply(&mut self.doc);
            edits.push(edit);
        }
        self.commit_batch(label, edits);
    }

    // -- key drag -----------------------------------------------------------

    /// Begin a drag on `index`, carrying the whole selection. `grab` is the
    /// pointer position in curve units, so the gesture moves by a delta rather
    /// than teleporting the key to the cursor.
    pub fn begin_drag(&mut self, track: usize, index: usize, grab: (f32, f32)) {
        if !self.editable(track) {
            return;
        }
        let len = match self.doc.tracks.get(track) {
            Some(t) => t.keys.len(),
            None => return,
        };
        if index >= len {
            return;
        }
        if track != self.selected_track {
            self.selected_track = track;
            self.selection.clear();
        }
        if !self.is_selected(index) {
            self.select_only(index);
        }
        let Some(tr) = self.doc.tracks.get(track) else { return };
        let from: Vec<(usize, Key)> = self
            .selection
            .iter()
            .filter_map(|i| tr.keys.get(*i).map(|k| (*i, *k)))
            .collect();
        self.drag = Some(KeyDrag { track, anchor: index, from, grab });
    }

    /// Live drag update — writes straight into the doc (no undo entry yet).
    ///
    /// The pointer moves the **anchor** key; every other selected key follows
    /// by the same delta, so the selection travels as a body and can never
    /// re-order itself. Time is clamped once, against the tightest bound any
    /// selected key has against an *unselected* neighbour: a body that clipped
    /// per key would shear apart at the first obstacle.
    pub fn drag_to(&mut self, t: f32, value: f32, snap: SnapSettings) {
        let Some(d) = self.drag.clone() else { return };
        let Some(tr) = self.doc.tracks.get(d.track) else { return };
        let Some((_, anchor_from)) = d.from.iter().find(|(i, _)| *i == d.anchor).copied() else {
            return;
        };

        // Where the anchor wants to be, snapped there rather than after the
        // delta — so a snapped drag lands the key you are holding on the grid,
        // which is the one the eye is on.
        let mut want_t = anchor_from.t + (t - d.grab.0);
        let mut want_v = anchor_from.value + (value - d.grab.1);
        if snap.time {
            want_t = snap_to(want_t, SNAP_TIME);
        }
        if snap.value {
            want_v = snap_to(want_v, SNAP_VALUE);
        }
        let dv = want_v - anchor_from.value;
        let mut dt = want_t - anchor_from.t;

        let selected: HashSet<usize> = d.from.iter().map(|(i, _)| *i).collect();
        let (mut dt_min, mut dt_max) = (f32::NEG_INFINITY, f32::INFINITY);
        for (i, from) in &d.from {
            let (lo, hi) = free_span(tr, *i, &selected);
            if lo > hi {
                // Boxed in on both sides: this key cannot move, so neither can
                // the body it belongs to.
                dt_min = 0.0;
                dt_max = 0.0;
                break;
            }
            dt_min = dt_min.max(lo - from.t);
            dt_max = dt_max.min(hi - from.t);
        }
        dt = if dt_min <= dt_max { dt.clamp(dt_min, dt_max) } else { 0.0 };

        for (i, from) in &d.from {
            // A move changes where a key is, never what it is: interpolation
            // and the hand-shaped tangent both ride along untouched.
            set_key(
                &mut self.doc,
                d.track,
                *i,
                Key { t: from.t + dt, value: from.value + dv, ..*from },
            );
        }
    }

    /// Finish the drag: one entry for the gesture, or nothing at all if every
    /// key ended where it started (a click that happened to jitter is not an
    /// edit).
    pub fn end_drag(&mut self) {
        let Some(d) = self.drag.take() else { return };
        let mut edits = Vec::new();
        for (index, from) in d.from {
            let Some(to) = self.doc.tracks.get(d.track).and_then(|t| t.keys.get(index)).copied()
            else {
                continue;
            };
            if to == from {
                continue;
            }
            edits.push(CurveEdit::MoveKey { track: d.track, index, from, to });
        }
        let label = if edits.len() == 1 { "Move Key" } else { "Move Keys" };
        self.commit_batch(label, edits);
    }

    // -- tangent arms -------------------------------------------------------

    /// Grab one of a key's tangent arms.
    ///
    /// A key that has no explicit tangent is **promoted to User, seeded with
    /// the slope it already has** — which GS-5a's value-per-second units make
    /// a no-op by construction, so the curve does not twitch at the moment you
    /// take hold of it. `from` remembers the mode before the promotion, so Esc
    /// and Undo both restore `Auto`.
    pub fn begin_arm_drag(&mut self, track: usize, index: usize, side: ArmSide) {
        if !self.editable(track) {
            return;
        }
        let Some(tr) = self.doc.tracks.get(track) else { return };
        let Some(from) = tr.keys.get(index).map(|k| k.tangent) else { return };
        if !TangentMode::of(&from).has_arms() {
            let seeded = tangent_for_mode(tr, index, TangentMode::User);
            self.write_tangent(track, index, seeded);
        }
        self.arm = Some(ArmDrag { track, index, side, from });
    }

    /// Live arm update: `slope` is value-per-second, the unit the schema
    /// stores. User moves both sides together; Break moves only the side that
    /// was grabbed — that is the whole difference between the two modes.
    pub fn arm_to(&mut self, slope: f32) {
        let Some(a) = self.arm else { return };
        let Some(cur) = self
            .doc
            .tracks
            .get(a.track)
            .and_then(|t| t.keys.get(a.index))
            .map(|k| k.tangent)
        else {
            return;
        };
        let next = match cur {
            Tangent::Break { in_tan, out_tan } => match a.side {
                ArmSide::In => Tangent::Break { in_tan: slope, out_tan },
                ArmSide::Out => Tangent::Break { in_tan, out_tan: slope },
            },
            _ => Tangent::User { tangent: slope },
        };
        self.write_tangent(a.track, a.index, next);
    }

    /// Finish the arm gesture: one "Edit Tangent" entry, or nothing if the arm
    /// came back to where it started (including the promotion, which is undone
    /// with it).
    pub fn end_arm_drag(&mut self) {
        let Some(a) = self.arm.take() else { return };
        let Some(to) = self
            .doc
            .tracks
            .get(a.track)
            .and_then(|t| t.keys.get(a.index))
            .map(|k| k.tangent)
        else {
            return;
        };
        if to == a.from {
            return;
        }
        let edit = CurveEdit::SetTangent { track: a.track, index: a.index, from: a.from, to };
        // Already written live; record the finished shape only.
        self.commit(edit);
    }

    fn write_tangent(&mut self, track: usize, index: usize, tangent: Tangent) {
        if let Some(k) = self.doc.tracks.get_mut(track).and_then(|t| t.keys.get_mut(index)) {
            k.tangent = tangent;
        }
    }

    // -- footer / readouts --------------------------------------------------

    /// What the footer detail bar shows. Pure derivation, so the panel has no
    /// say in what "mixed" means.
    pub fn footer(&self) -> FooterState {
        let Some(track) = self.doc.tracks.get(self.selected_track) else {
            return FooterState::empty();
        };
        let keys: Vec<&Key> = self
            .selection
            .iter()
            .filter_map(|i| track.keys.get(*i))
            .collect();
        if keys.is_empty() {
            return FooterState::empty();
        }
        let all = |f: &dyn Fn(&Key) -> f32| -> Option<f32> {
            let first = f(keys[0]);
            keys.iter().all(|k| f(k) == first).then_some(first)
        };
        let interp = {
            let first = keys[0].interp;
            keys.iter().all(|k| k.interp == first).then_some(first)
        };
        let tangent = {
            let first = TangentMode::of(&keys[0].tangent);
            keys.iter()
                .all(|k| TangentMode::of(&k.tangent) == first)
                .then_some(first)
        };
        FooterState {
            count: keys.len(),
            t: all(&|k| k.t),
            value: all(&|k| k.value),
            interp,
            tangent,
            // Any selected key touching a cubic segment makes the control
            // useful; requiring all of them would disable it on exactly the
            // mixed selection someone is trying to unify.
            tangent_enabled: self
                .selection
                .iter()
                .any(|i| touches_cubic(track, *i)),
        }
    }

    /// The playhead's sampled value for every **visible** track, in document
    /// order. `Track::sample` is the interpreter's function: a Timeline node
    /// at this time outputs exactly these numbers.
    pub fn playhead_readouts(&self) -> Vec<(usize, f32)> {
        self.doc
            .tracks
            .iter()
            .enumerate()
            .filter(|(i, _)| !self.is_hidden(*i))
            .map(|(i, t)| (i, t.sample(self.playhead)))
            .collect()
    }

    /// Move the preview cursor. Never negative, and snapping applies — the
    /// playhead is read against the same grid as the keys.
    pub fn set_playhead(&mut self, t: f32, snap: SnapSettings) {
        let t = if snap.time { snap_to(t, SNAP_TIME) } else { t };
        self.playhead = t.max(0.0);
    }

    // -- tracks -------------------------------------------------------------

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
        self.selection.clear();
        Some(index)
    }

    pub fn remove_track(&mut self, index: usize) {
        let Some(track) = self.doc.tracks.get(index).cloned() else {
            return;
        };
        let edit = CurveEdit::RemoveTrack { index, track };
        edit.apply(&mut self.doc);
        self.commit(edit);
        self.selection.clear();
        self.clamp_selection();
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
        self.frame_track_only = false;
    }

    /// Fit the selected track alone — the toolbar's second framing verb, for
    /// a document whose tracks live at wildly different scales.
    pub fn frame_track(&mut self) {
        self.frame_pending = true;
        self.frame_track_only = true;
    }
}

/// The time range key `index` may occupy without colliding with a key that is
/// **not** moving. Selected keys are skipped: they travel with it, so treating
/// them as walls would freeze every group drag at the first neighbour.
fn free_span(track: &Track, index: usize, moving: &HashSet<usize>) -> (f32, f32) {
    let lo = (0..index)
        .rev()
        .find(|i| !moving.contains(i))
        .and_then(|i| track.keys.get(i))
        .map(|k| k.t + MIN_KEY_GAP)
        .unwrap_or(0.0)
        .max(0.0);
    let hi = (index + 1..track.keys.len())
        .find(|i| !moving.contains(i))
        .and_then(|i| track.keys.get(i))
        .map(|k| k.t - MIN_KEY_GAP)
        .unwrap_or(f32::MAX);
    (lo, hi)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc() -> CurveDoc {
        let mut d = CurveDoc::default();
        let mut t = Track::new("height", "Height");
        t.keys = vec![
            Key { t: 0.0, value: 0.0, interp: Interp::Linear, tangent: Tangent::Auto },
            Key { t: 1.0, value: 10.0, interp: Interp::Linear, tangent: Tangent::Auto },
            Key { t: 2.0, value: 0.0, interp: Interp::Linear, tangent: Tangent::Auto },
        ];
        d.tracks = vec![t];
        d
    }

    fn state() -> CurveEditorState {
        CurveEditorState::from_doc(doc(), "curves/test.curve")
    }

    /// A four-key cubic track, for the tangent tests.
    fn cubic_state() -> CurveEditorState {
        let mut d = CurveDoc::default();
        let mut t = Track::new("height", "Height");
        t.keys = vec![
            Key { t: 0.0, value: 0.0, interp: Interp::Cubic, tangent: Tangent::Auto },
            Key { t: 1.0, value: 2.0, interp: Interp::Cubic, tangent: Tangent::Auto },
            Key { t: 3.0, value: 1.0, interp: Interp::Cubic, tangent: Tangent::Auto },
            Key { t: 4.0, value: 0.0, interp: Interp::Cubic, tangent: Tangent::Auto },
        ];
        d.tracks = vec![t];
        CurveEditorState::from_doc(d, "curves/cubic.curve")
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
        s.begin_drag(0, 1, (1.0, 10.0));
        // Way past both neighbours, in both directions, then settle.
        s.drag_to(99.0, 3.0, SnapSettings::default());
        assert!(s.doc.tracks[0].keys[1].t < s.doc.tracks[0].keys[2].t);
        s.drag_to(-99.0, 3.0, SnapSettings::default());
        assert!(s.doc.tracks[0].keys[1].t > s.doc.tracks[0].keys[0].t);
        s.drag_to(1.5, 3.0, SnapSettings::default());
        s.end_drag();
        assert_eq!(s.stack.undo_len(), 1, "one gesture, one entry");
        assert_eq!(
            s.doc.tracks[0].keys[1],
            Key { t: 1.5, value: 3.0, interp: Interp::Linear, tangent: Tangent::Auto }
        );

        s.undo();
        assert_eq!(s.doc.tracks[0].keys[1].t, 1.0);
        assert_eq!(s.doc.tracks[0].keys[1].value, 10.0);
        assert!(!s.dirty, "back at the save point");
    }

    #[test]
    fn a_drag_that_moves_nothing_records_nothing() {
        let mut s = state();
        s.begin_drag(0, 1, (1.0, 10.0));
        s.drag_to(1.0, 10.0, SnapSettings::default());
        s.end_drag();
        assert_eq!(s.stack.undo_len(), 0);
        assert!(!s.dirty);
    }

    #[test]
    fn setting_interp_on_a_selection_is_one_entry_and_reverts() {
        let mut s = state();
        s.set_selection([0, 1].to_vec());
        s.set_selection_interp(Interp::Cubic);
        assert_eq!(s.doc.tracks[0].keys[0].interp, Interp::Cubic);
        assert_eq!(s.doc.tracks[0].keys[1].interp, Interp::Cubic);
        assert_eq!(s.stack.undo_len(), 1, "a selection edit is one gesture");
        s.undo();
        assert_eq!(s.doc.tracks[0].keys[0].interp, Interp::Linear);
        assert_eq!(s.doc.tracks[0].keys[1].interp, Interp::Linear);
        assert!(!s.dirty);
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
        s.begin_drag(0, 1, (0.5, 5.0));
        s.drag_to(0.6, 6.0, SnapSettings::default());
        s.end_drag();
        s.set_selection_interp(Interp::Cubic);
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
        s.selection = vec![0];
        s.remove_track(1);
        assert_eq!(s.selected_track, 0);
        assert!(!s.has_selection());
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
    /// open, add a key, drag it, set its interp, add a track, undo all of
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
        let (t, k) = s.selected_key().expect("the new key is selected");
        s.begin_drag(t, k, (0.35, 1.8));
        s.drag_to(0.4, 2.0, SnapSettings::default());
        s.end_drag();
        // Constant, not Cubic: the new key inherited the segment it split, so
        // "set it to what it already is" would record nothing and the count
        // below would be measuring the wrong thing.
        s.set_selection_interp(Interp::Constant);
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
        s.begin_drag(0, 1, (1.0, 10.0));
        s.drag_to(1.4, 44.0, SnapSettings::default());
        s.cancel_gestures();
        assert_eq!(s.doc.tracks[0].keys[1].value, 10.0);
        assert_eq!(s.stack.undo_len(), 0);
    }

    // -- GS-5b: selection ---------------------------------------------------

    /// A box partitions the track: keys inside come in, keys outside stay out,
    /// and `add` is what makes a second sweep extend rather than replace.
    #[test]
    fn box_select_partitions_on_the_selected_track() {
        let mut s = state();
        s.select_in_box((-0.5, 1.5), (-1.0, 20.0), false);
        assert_eq!(s.selection, vec![0, 1], "keys 0 and 1 are inside the box");

        // A replace-sweep over nothing is how you deselect.
        s.select_in_box((5.0, 6.0), (0.0, 1.0), false);
        assert!(s.selection.is_empty());

        // Value bounds count too: key 1 sits at 10, above this box.
        s.select_in_box((-1.0, 9.0), (-1.0, 5.0), false);
        assert_eq!(s.selection, vec![0, 2]);
        // …and Shift extends.
        s.select_in_box((0.9, 1.1), (9.0, 11.0), true);
        assert_eq!(s.selection, vec![0, 1, 2]);
    }

    #[test]
    fn shift_click_extends_and_un_extends() {
        let mut s = state();
        s.select_only(1);
        s.toggle_select(0);
        assert_eq!(s.selection, vec![0, 1], "kept ascending");
        s.toggle_select(1);
        assert_eq!(s.selection, vec![0]);
    }

    /// Group drag: the selection moves as a body, clamped by the tightest
    /// bound against an **unselected** neighbour — never against each other.
    #[test]
    fn group_drag_clamps_against_unselected_neighbours_only() {
        let mut s = state();
        // Keys 0 and 1 selected, key 2 (t = 2.0) is the wall.
        s.set_selection(vec![0, 1]);
        s.begin_drag(0, 1, (1.0, 10.0));
        s.drag_to(99.0, 10.0, SnapSettings::default());
        let ts: Vec<f32> = s.doc.tracks[0].keys.iter().map(|k| k.t).collect();
        assert!((ts[1] - (2.0 - MIN_KEY_GAP)).abs() < 1e-4, "anchor stops at the wall: {ts:?}");
        assert!(
            (ts[1] - ts[0] - 1.0).abs() < 1e-4,
            "the body keeps its shape (selected keys are not walls): {ts:?}"
        );
        assert!(ts[0] < ts[1] && ts[1] < ts[2], "order survives: {ts:?}");

        // …and back the other way: key 0 hits the t = 0 floor, which stops the
        // whole body rather than letting key 1 slide through it.
        s.drag_to(-99.0, 10.0, SnapSettings::default());
        let ts: Vec<f32> = s.doc.tracks[0].keys.iter().map(|k| k.t).collect();
        assert!((ts[0] - 0.0).abs() < 1e-4, "{ts:?}");
        assert!((ts[1] - 1.0).abs() < 1e-4, "{ts:?}");

        // Settle somewhere new, so the release has something to record.
        s.drag_to(1.4, 10.0, SnapSettings::default());
        s.end_drag();
        assert_eq!(s.stack.undo_len(), 1, "one gesture, one entry, two keys");
        s.undo();
        let ts: Vec<f32> = s.doc.tracks[0].keys.iter().map(|k| k.t).collect();
        assert_eq!(ts, vec![0.0, 1.0, 2.0], "undo restores every key in the batch");
    }

    #[test]
    fn deleting_a_selection_is_one_entry_and_undo_restores_order() {
        let mut s = state();
        s.set_selection(vec![0, 2]);
        s.delete_selection();
        assert_eq!(s.doc.tracks[0].keys.len(), 1);
        assert_eq!(s.doc.tracks[0].keys[0].t, 1.0);
        assert_eq!(s.stack.undo_len(), 1);
        s.undo();
        let ts: Vec<f32> = s.doc.tracks[0].keys.iter().map(|k| k.t).collect();
        assert_eq!(ts, vec![0.0, 1.0, 2.0], "keys come back where they were");
        assert!(!s.has_selection());
    }

    // -- GS-5b: snapping ----------------------------------------------------

    #[test]
    fn ctrl_inverts_the_resting_snap_toggles() {
        let off = SnapSettings { time: false, value: false };
        assert_eq!(off.effective(false), off);
        assert_eq!(off.effective(true), SnapSettings { time: true, value: true });
        let on = SnapSettings { time: true, value: true };
        assert_eq!(on.effective(false), on);
        assert_eq!(on.effective(true), SnapSettings { time: false, value: false });
    }

    #[test]
    fn a_snapped_drag_lands_the_anchor_on_the_grid() {
        let mut s = state();
        s.begin_drag(0, 1, (1.0, 10.0));
        s.drag_to(1.34, 10.13, SnapSettings { time: true, value: true });
        let k = s.doc.tracks[0].keys[1];
        assert!((k.t - 1.3).abs() < 1e-4, "t snapped to 0.1: {}", k.t);
        assert!((k.value - 10.15).abs() < 1e-4, "value snapped to 0.05: {}", k.value);

        // Unsnapped, the same pointer keeps its fractional position.
        s.drag_to(1.34, 10.13, SnapSettings::default());
        let k = s.doc.tracks[0].keys[1];
        assert!((k.t - 1.34).abs() < 1e-4);
        assert!((k.value - 10.13).abs() < 1e-4);
    }

    // -- GS-5b: tangents ----------------------------------------------------

    /// Promotion is a no-op *by construction*: taking hold of an Auto key's
    /// arm seeds User with the slope it already had, so the sampled curve does
    /// not move — and undo puts the mode back.
    #[test]
    fn an_arm_drag_promotes_auto_to_user_without_moving_the_curve() {
        let mut s = cubic_state();
        let before: Vec<f32> = (0..=40).map(|i| s.doc.tracks[0].sample(i as f32 * 0.1)).collect();
        let auto_slope = s.doc.tracks[0].out_tangent(1);

        s.begin_arm_drag(0, 1, ArmSide::Out);
        assert_eq!(
            s.doc.tracks[0].keys[1].tangent,
            Tangent::User { tangent: auto_slope },
            "seeded with the slope it already had"
        );
        let after: Vec<f32> = (0..=40).map(|i| s.doc.tracks[0].sample(i as f32 * 0.1)).collect();
        assert_eq!(before, after, "grabbing an arm must not move the curve");

        s.arm_to(auto_slope * 2.0);
        s.end_arm_drag();
        assert_eq!(s.stack.undo_len(), 1, "one arm gesture, one entry");
        assert_eq!(s.stack.undo_description().as_deref(), Some("Set Tangent User"));
        assert_eq!(s.doc.tracks[0].keys[1].tangent, Tangent::User { tangent: auto_slope * 2.0 });

        s.undo();
        assert_eq!(s.doc.tracks[0].keys[1].tangent, Tangent::Auto, "undo restores Auto");
    }

    /// The whole point of Break: the two arms are independent, and a User key
    /// moves both at once.
    #[test]
    fn user_arms_move_together_and_break_arms_do_not() {
        let mut s = cubic_state();
        s.doc.tracks[0].keys[1].tangent = Tangent::User { tangent: 1.0 };
        s.begin_arm_drag(0, 1, ArmSide::In);
        s.arm_to(-3.0);
        assert_eq!(s.doc.tracks[0].keys[1].tangent, Tangent::User { tangent: -3.0 });
        s.end_arm_drag();

        s.doc.tracks[0].keys[1].tangent = Tangent::Break { in_tan: 1.0, out_tan: 2.0 };
        s.begin_arm_drag(0, 1, ArmSide::Out);
        s.arm_to(-9.0);
        assert_eq!(
            s.doc.tracks[0].keys[1].tangent,
            Tangent::Break { in_tan: 1.0, out_tan: -9.0 },
            "the side you did not grab is untouched"
        );
        s.end_arm_drag();
    }

    /// An arm gesture abandoned with Esc leaves the document byte-identical —
    /// including undoing the promotion it did on the way in.
    #[test]
    fn escaping_an_arm_drag_restores_the_original_mode() {
        let mut s = cubic_state();
        let before = s.doc.clone();
        s.begin_arm_drag(0, 1, ArmSide::Out);
        s.arm_to(42.0);
        s.cancel_gestures();
        assert_eq!(s.doc, before, "Esc reverts the promotion too");
        assert_eq!(s.stack.undo_len(), 0);
        assert!(!s.dirty);
    }

    /// Applying a mode to a mixed selection is one entry, and the explicit
    /// modes are seeded so the sampled curve does not jump.
    #[test]
    fn applying_a_tangent_mode_to_a_selection_is_one_seeded_entry() {
        let mut s = cubic_state();
        s.set_selection(vec![1, 2]);
        let before: Vec<f32> = (0..=40).map(|i| s.doc.tracks[0].sample(i as f32 * 0.1)).collect();
        s.set_selection_tangent(TangentMode::Break);
        assert_eq!(s.stack.undo_len(), 1);
        let after: Vec<f32> = (0..=40).map(|i| s.doc.tracks[0].sample(i as f32 * 0.1)).collect();
        assert_eq!(before, after, "Break seeded from the current slopes is a no-op");
        assert!(matches!(s.doc.tracks[0].keys[1].tangent, Tangent::Break { .. }));
        assert!(matches!(s.doc.tracks[0].keys[2].tangent, Tangent::Break { .. }));

        s.undo();
        assert_eq!(s.doc.tracks[0].keys[1].tangent, Tangent::Auto);
        assert_eq!(s.doc.tracks[0].keys[2].tangent, Tangent::Auto);
    }

    #[test]
    fn flatten_and_straighten_are_the_flat_and_linear_modes() {
        let mut s = cubic_state();
        s.set_selection(vec![1]);
        s.flatten_selection();
        assert_eq!(s.doc.tracks[0].keys[1].tangent, Tangent::Flat);
        assert_eq!(s.stack.undo_description().as_deref(), Some("Flatten"));
        s.straighten_selection();
        assert_eq!(s.doc.tracks[0].keys[1].tangent, Tangent::Linear);
        assert_eq!(s.stack.undo_description().as_deref(), Some("Straighten"));
    }

    /// Glyph is mode; the two arm modes share the circle because their arms
    /// carry the distinction.
    #[test]
    fn glyphs_encode_the_mode() {
        assert_eq!(key_glyph(&Tangent::Auto), KeyGlyph::Circle);
        assert_eq!(key_glyph(&Tangent::User { tangent: 1.0 }), KeyGlyph::Circle);
        assert_eq!(key_glyph(&Tangent::Break { in_tan: 0.0, out_tan: 0.0 }), KeyGlyph::Circle);
        assert_eq!(key_glyph(&Tangent::Flat), KeyGlyph::Square);
        assert_eq!(key_glyph(&Tangent::Linear), KeyGlyph::Diamond);
        assert!(TangentMode::User.has_arms() && TangentMode::Break.has_arms());
        assert!(!TangentMode::Auto.has_arms());
    }

    // -- GS-5b: footer ------------------------------------------------------

    #[test]
    fn the_footer_reports_agreement_and_mixedness() {
        let mut s = cubic_state();
        assert_eq!(s.footer(), FooterState::empty(), "no selection, nothing to show");

        s.select_only(1);
        let f = s.footer();
        assert_eq!(f.count, 1);
        assert_eq!(f.t, Some(1.0));
        assert_eq!(f.value, Some(2.0));
        assert_eq!(f.interp, Some(Interp::Cubic));
        assert_eq!(f.tangent, Some(TangentMode::Auto));
        assert!(f.tangent_enabled, "a cubic key can carry a tangent");

        // Two keys that disagree about everything measurable.
        s.set_selection(vec![1, 2]);
        s.doc.tracks[0].keys[2].tangent = Tangent::Flat;
        let f = s.footer();
        assert_eq!(f.count, 2);
        assert_eq!(f.t, None, "mixed time is the em dash");
        assert_eq!(f.value, None);
        assert_eq!(f.tangent, None);
        assert!(f.tangent_mixed());
        assert!(!f.interp_mixed(), "both are Cubic");

        // Tangents are a cubic concern: a track with no cubic segment greys
        // the control.
        let mut lin = state();
        lin.select_only(1);
        let f = lin.footer();
        assert!(!f.tangent_enabled);
        assert_eq!(f.tangent, Some(TangentMode::Auto), "the mode is still readable");
    }

    #[test]
    fn typing_into_the_footer_sets_the_whole_selection_in_one_entry() {
        let mut s = state();
        s.set_selection(vec![0, 1]);
        s.set_selection_value(4.0);
        assert_eq!(s.doc.tracks[0].keys[0].value, 4.0);
        assert_eq!(s.doc.tracks[0].keys[1].value, 4.0);
        assert_eq!(s.stack.undo_len(), 1);
        assert_eq!(s.footer().value, Some(4.0), "no longer mixed");

        // A typed time still respects the keys that are not moving.
        s.set_selection(vec![0]);
        s.set_selection_time(9.0);
        assert!(
            (s.doc.tracks[0].keys[0].t - (1.0 - MIN_KEY_GAP)).abs() < 1e-4,
            "clamped below key 1: {}",
            s.doc.tracks[0].keys[0].t
        );
        s.undo();
        assert_eq!(s.doc.tracks[0].keys[0].t, 0.0);
    }

    // -- GS-5b: tracks + playhead -------------------------------------------

    #[test]
    fn hide_and_lock_are_view_state_keyed_by_slug() {
        let mut s = state();
        s.add_track("Lean").expect("added");
        s.toggle_hidden(0);
        s.toggle_locked(1);
        assert!(s.is_hidden(0) && !s.is_hidden(1));
        assert!(s.is_locked(1) && !s.is_locked(0));

        // Removing the *first* track shifts indices; the flags follow their
        // slugs rather than the positions they happened to have.
        s.remove_track(0);
        assert!(s.is_locked(0), "'lean' is still locked at its new index");

        // A locked track refuses writes, with a reason.
        s.selected_track = 0;
        assert!(s.add_key(0, 1.0, 1.0).is_none());
        assert!(s.toast.as_ref().is_some_and(|(m, _)| m.contains("locked")));
    }

    /// The playhead's readouts are `Track::sample` — the interpreter's own
    /// function — and hidden tracks are not sampled.
    #[test]
    fn playhead_readouts_match_the_runtime_evaluation() {
        let mut s = state();
        s.add_track("Lean").expect("added");
        s.doc.tracks[1].keys = vec![
            Key { t: 0.0, value: 1.0, interp: Interp::Linear, tangent: Tangent::Auto },
            Key { t: 2.0, value: 3.0, interp: Interp::Linear, tangent: Tangent::Auto },
        ];
        s.set_playhead(0.5, SnapSettings::default());
        let readouts = s.playhead_readouts();
        assert_eq!(readouts.len(), 2);
        for (i, v) in readouts {
            assert_eq!(v, s.doc.tracks[i].sample(0.5), "track {i} readout is the sampler");
        }

        s.toggle_hidden(0);
        assert_eq!(s.playhead_readouts().len(), 1, "a hidden track is not read out");

        // Snapping applies to the playhead too, and it never goes negative.
        s.set_playhead(0.34, SnapSettings { time: true, value: false });
        assert!((s.playhead - 0.3).abs() < 1e-4);
        s.set_playhead(-5.0, SnapSettings::default());
        assert_eq!(s.playhead, 0.0);
    }
}
