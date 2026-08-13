//! The editor keymap — bindings as **data**, not as `match` arms.
//!
//! Every keyboard shortcut in the graph editor is an [`Action`] with a stable
//! id, and a [`Keymap`] maps chords onto those ids. The default map is
//! compiled in (a [`Preset`]); a user's `keymap.ron` sits beside
//! `editor_prefs.ron` and *overlays* it, so an absent file means defaults, a
//! partial file means "these actions differ, leave the rest alone", and no
//! user ever inherits a stale copy of a binding we later change.
//!
//! # Scope: keyboard only
//!
//! v1 binds **keyboard chords and nothing else**. Mouse gestures — right-drag
//! to pan, Alt+click to break a link, drag thresholds, wheel semantics — are
//! the input model's domain (Rules 1–3), not remappable data. They are
//! structural: which button starts a marquee is not a preference, it is what
//! makes the three-phase gesture model coherent. The one dial we do expose is
//! [`MouseProfile`], a whole-profile switch the input code reads, rather than
//! fifty individually rebindable mouse verbs.
//!
//! # Contexts and shadowing
//!
//! An action belongs to exactly one [`Context`], and contexts nest:
//! `Canvas` ⊂ `GraphTab` ⊂ `Global`. Lookup walks that chain outward from the
//! most specific active context, so a `Canvas` binding legitimately
//! **shadows** a `Global` one on the same chord — that is the mechanism by
//! which `C` groups nodes on the canvas without breaking `Ctrl+C` everywhere
//! else. `TextEntry` is deliberately *not* in the graph chain: it is a mode
//! that suppresses single-key shortcuts while the user is typing.
//!
//! Two bindings sharing a chord **in the same context** is a genuine conflict
//! and is reported at load, naming both actions. A conflict is never fatal:
//! the editor falls back to the compiled-in defaults so it still opens.

use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

use crusty_gui::context::InputState;
use crusty_gui::input::{Key, Modifiers};
use serde::{Deserialize, Serialize};

pub const KEYMAP_FILE: &str = "keymap.ron";

// ─────────────────────────────────────────────────────────────────────────────
// Contexts
// ─────────────────────────────────────────────────────────────────────────────

/// Where a binding is live. Ordered by specificity for display; the nesting
/// relation itself is [`Context::chain`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Context {
    /// Anywhere in the editor.
    Global,
    /// A graph document tab has focus, pointer anywhere within it.
    GraphTab,
    /// The pointer is over the graph canvas itself.
    Canvas,
    /// A text field has keyboard focus. Single-key shortcuts must not fire.
    TextEntry,
}

impl Context {
    pub const ALL: [Context; 4] =
        [Context::Canvas, Context::GraphTab, Context::Global, Context::TextEntry];

    pub fn label(self) -> &'static str {
        match self {
            Context::Global => "Global",
            Context::GraphTab => "Graph Tab",
            Context::Canvas => "Graph Canvas",
            Context::TextEntry => "Text Entry",
        }
    }

    /// Contexts that are simultaneously active when `self` is, most specific
    /// first. Lookup walks this, so the first hit wins and outer contexts are
    /// shadowed rather than contended.
    pub fn chain(self) -> &'static [Context] {
        match self {
            Context::Canvas => &[Context::Canvas, Context::GraphTab, Context::Global],
            Context::GraphTab => &[Context::GraphTab, Context::Global],
            // Typing suppresses the graph chain entirely — that is the point.
            Context::TextEntry => &[Context::TextEntry, Context::Global],
            Context::Global => &[Context::Global],
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Chords
// ─────────────────────────────────────────────────────────────────────────────

/// A keyboard chord: one key plus an exact modifier set.
///
/// Its RON representation is the same string the UI shows (`"Ctrl+Shift+F9"`),
/// so the file stays hand-editable and there is exactly one spelling of a
/// chord in the codebase — [`Chord::label`] and [`Chord::parse`] are inverses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Chord {
    pub key: Key,
    pub mods: Modifiers,
}

/// Why a chord string could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChordError {
    Empty,
    UnknownKey(String),
    UnknownModifier(String),
}

impl fmt::Display for ChordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChordError::Empty => write!(f, "empty chord"),
            ChordError::UnknownKey(k) => write!(f, "unknown key `{k}`"),
            ChordError::UnknownModifier(m) => write!(f, "unknown modifier `{m}`"),
        }
    }
}

impl Chord {
    pub fn new(key: Key, mods: Modifiers) -> Self {
        Self { key, mods }
    }

    /// A chord with no modifiers.
    pub fn plain(key: Key) -> Self {
        Self::new(key, Modifiers::empty())
    }

    pub fn ctrl(key: Key) -> Self {
        Self::new(key, Modifiers::CTRL)
    }

    pub fn shift(key: Key) -> Self {
        Self::new(key, Modifiers::SHIFT)
    }

    pub fn alt(key: Key) -> Self {
        Self::new(key, Modifiers::ALT)
    }

    /// `"Ctrl+Shift+F9"`. Modifier order is fixed (Ctrl, Alt, Shift, Win) so
    /// two spellings of one chord can never render differently.
    pub fn label(&self) -> String {
        let mut s = String::new();
        for (m, name) in [
            (Modifiers::CTRL, "Ctrl"),
            (Modifiers::ALT, "Alt"),
            (Modifiers::SHIFT, "Shift"),
            (Modifiers::META, "Win"),
        ] {
            if self.mods.contains(m) {
                s.push_str(name);
                s.push('+');
            }
        }
        s.push_str(&key_label(self.key));
        s
    }

    pub fn parse(s: &str) -> Result<Chord, ChordError> {
        let s = s.trim();
        if s.is_empty() {
            return Err(ChordError::Empty);
        }
        let mut mods = Modifiers::empty();
        // Split on '+' but keep a trailing literal '+' as the key itself.
        let parts: Vec<&str> = split_chord(s);
        let (last, rest) = parts.split_last().ok_or(ChordError::Empty)?;
        for p in rest {
            mods |= match p.trim().to_ascii_lowercase().as_str() {
                "ctrl" | "control" => Modifiers::CTRL,
                "alt" | "option" => Modifiers::ALT,
                "shift" => Modifiers::SHIFT,
                "win" | "cmd" | "meta" | "super" => Modifiers::META,
                other => return Err(ChordError::UnknownModifier(other.to_string())),
            };
        }
        Ok(Chord { key: parse_key(last.trim())?, mods })
    }
}

impl fmt::Display for Chord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.label())
    }
}

impl Serialize for Chord {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.label())
    }
}

impl<'de> Deserialize<'de> for Chord {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Chord::parse(&s).map_err(serde::de::Error::custom)
    }
}

/// Split a chord on `+`, treating a trailing `+` as the key (so `"Ctrl++"`
/// binds the plus key rather than parsing as an empty key name).
fn split_chord(s: &str) -> Vec<&str> {
    if let Some(head) = s.strip_suffix("++") {
        let mut v: Vec<&str> = head.split('+').collect();
        v.push(&s[s.len() - 1..]);
        return v;
    }
    if s == "+" {
        return vec!["+"];
    }
    s.split('+').collect()
}

fn key_label(k: Key) -> String {
    match k {
        Key::Escape => "Esc".into(),
        Key::Tab => "Tab".into(),
        Key::Backspace => "Backspace".into(),
        Key::Enter => "Enter".into(),
        Key::Space => "Space".into(),
        Key::ArrowLeft => "Left".into(),
        Key::ArrowRight => "Right".into(),
        Key::ArrowUp => "Up".into(),
        Key::ArrowDown => "Down".into(),
        Key::Home => "Home".into(),
        Key::End => "End".into(),
        Key::PageUp => "PageUp".into(),
        Key::PageDown => "PageDown".into(),
        Key::Delete => "Del".into(),
        Key::Insert => "Insert".into(),
        Key::F(n) => format!("F{n}"),
        Key::Char(c) => c.to_ascii_uppercase().to_string(),
        Key::Unknown => "?".into(),
    }
}

fn parse_key(s: &str) -> Result<Key, ChordError> {
    let lower = s.to_ascii_lowercase();
    // Function keys first, so "F5" never reads as the character F.
    if let Some(n) = lower.strip_prefix('f') {
        if let Ok(n) = n.parse::<u8>() {
            if (1..=24).contains(&n) {
                return Ok(Key::F(n));
            }
        }
    }
    Ok(match lower.as_str() {
        "esc" | "escape" => Key::Escape,
        "tab" => Key::Tab,
        "backspace" => Key::Backspace,
        "enter" | "return" => Key::Enter,
        "space" => Key::Space,
        "left" | "arrowleft" => Key::ArrowLeft,
        "right" | "arrowright" => Key::ArrowRight,
        "up" | "arrowup" => Key::ArrowUp,
        "down" | "arrowdown" => Key::ArrowDown,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" | "pgup" => Key::PageUp,
        "pagedown" | "pgdn" => Key::PageDown,
        "del" | "delete" => Key::Delete,
        "insert" | "ins" => Key::Insert,
        _ => {
            let mut it = s.chars();
            match (it.next(), it.next()) {
                // Stored lowercase: crusty delivers `Char` lowercased, and a
                // chord's Shift lives in `mods`, never in the character.
                (Some(c), None) => Key::Char(c.to_ascii_lowercase()),
                _ => return Err(ChordError::UnknownKey(s.to_string())),
            }
        }
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Actions
// ─────────────────────────────────────────────────────────────────────────────

/// How far along an action is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionStatus {
    /// Dispatches to a live handler.
    Live,
    /// Bound and listed, but the handler lands in Pass C. Dispatch no-ops and
    /// consumers (menus, cheat sheet, preferences) dim the row rather than
    /// hiding it — a user who reads the table should find the key listed.
    Unimplemented,
    /// Enforced structurally by the input model, not by keymap dispatch.
    /// Listed so the cheat sheet is complete; not remappable, because its
    /// precedence *is* the interaction model (Rules 1 and 3).
    Fixed,
}

impl ActionStatus {
    pub fn dispatchable(self) -> bool {
        matches!(self, ActionStatus::Live)
    }
}

struct ActionDef {
    id: &'static str,
    name: &'static str,
    group: &'static str,
    context: Context,
    status: ActionStatus,
}

/// A bindable editor command, referenced by a stable string id in
/// `keymap.ron`. A newtype over an index into the compiled table, so it is
/// `Copy`, cheap to compare, and carries its metadata without allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Action(u16);

macro_rules! actions {
    ($($konst:ident => $id:literal, $name:literal, $group:literal, $ctx:expr, $status:expr;)*) => {
        static ACTIONS: &[ActionDef] = &[
            $(ActionDef { id: $id, name: $name, group: $group, context: $ctx, status: $status },)*
        ];
        impl Action {
            actions!(@consts 0u16; $($konst,)*);
        }
    };
    (@consts $i:expr; ) => {};
    (@consts $i:expr; $head:ident, $($tail:ident,)*) => {
        pub const $head: Action = Action($i);
        actions!(@consts $i + 1u16; $($tail,)*);
    };
}

use ActionStatus::{Fixed, Live, Unimplemented};
use Context::{Canvas, GraphTab, Global};

actions! {
    // ── Creation and deletion
    ADD_NODE_PALETTE => "graph.add_node_palette", "Add Node…", "Create", Canvas, Live;
    DELETE_SELECTION => "graph.delete_selection", "Delete Selection", "Create", Canvas, Live;
    DELETE_AND_HEAL  => "graph.delete_and_heal", "Delete and Reconnect", "Create", Canvas, Unimplemented;
    QUICK_PLACE      => "graph.quick_place", "Quick Place", "Create", Canvas, Unimplemented;

    // ── Clipboard
    CUT       => "edit.cut", "Cut", "Clipboard", GraphTab, Live;
    COPY      => "edit.copy", "Copy", "Clipboard", GraphTab, Live;
    PASTE     => "edit.paste", "Paste", "Clipboard", GraphTab, Live;
    DUPLICATE => "edit.duplicate", "Duplicate", "Clipboard", GraphTab, Live;

    // ── History
    UNDO => "edit.undo", "Undo", "History", Global, Live;
    REDO => "edit.redo", "Redo", "History", Global, Live;

    // ── Wires
    STRAIGHTEN => "graph.straighten", "Straighten Connections", "Wires", Canvas, Unimplemented;

    // ── Movement
    NUDGE_UP         => "graph.nudge_up", "Nudge Up", "Movement", Canvas, Live;
    NUDGE_DOWN       => "graph.nudge_down", "Nudge Down", "Movement", Canvas, Live;
    NUDGE_LEFT       => "graph.nudge_left", "Nudge Left", "Movement", Canvas, Live;
    NUDGE_RIGHT      => "graph.nudge_right", "Nudge Right", "Movement", Canvas, Live;
    NUDGE_UP_FINE    => "graph.nudge_up_fine", "Nudge Up (Fine)", "Movement", Canvas, Live;
    NUDGE_DOWN_FINE  => "graph.nudge_down_fine", "Nudge Down (Fine)", "Movement", Canvas, Live;
    NUDGE_LEFT_FINE  => "graph.nudge_left_fine", "Nudge Left (Fine)", "Movement", Canvas, Live;
    NUDGE_RIGHT_FINE => "graph.nudge_right_fine", "Nudge Right (Fine)", "Movement", Canvas, Live;

    // ── Organization
    GROUP    => "graph.group", "Group Selection", "Organization", Canvas, Live;
    COMMENT  => "graph.comment", "Add Comment", "Organization", Canvas, Live;
    COLLAPSE => "graph.collapse_to_subgraph", "Collapse to Subgraph", "Organization", Canvas, Live;
    RENAME   => "graph.rename", "Rename", "Organization", Canvas, Live;

    // ── Alignment
    ALIGN_STRIP    => "graph.align_strip", "Align & Distribute…", "Alignment", Canvas, Live;
    ALIGN_TOP      => "graph.align_top", "Align Top", "Alignment", Canvas, Live;
    ALIGN_LEFT     => "graph.align_left", "Align Left", "Alignment", Canvas, Live;
    ALIGN_BOTTOM   => "graph.align_bottom", "Align Bottom", "Alignment", Canvas, Live;
    ALIGN_RIGHT    => "graph.align_right", "Align Right", "Alignment", Canvas, Live;
    ALIGN_CENTER_H => "graph.align_center_h", "Align Centers Horizontally", "Alignment", Canvas, Live;
    ALIGN_CENTER_V => "graph.align_center_v", "Align Centers Vertically", "Alignment", Canvas, Live;
    AUTO_LAYOUT    => "graph.auto_layout", "Auto-Layout", "Alignment", Canvas, Live;

    // ── View
    FRAME_SELECTION => "graph.frame_selection", "Frame Selection", "View", Canvas, Live;
    FIT_GRAPH       => "graph.fit_graph", "Fit Graph", "View", Canvas, Live;
    PARENT_GRAPH    => "graph.parent_graph", "Go to Parent Graph", "View", GraphTab, Live;
    CHILD_GRAPH     => "graph.child_graph", "Go to Child Graph", "View", GraphTab, Live;
    TOGGLE_VARIABLES => "graph.toggle_variables", "Variables Panel", "View", GraphTab, Live;

    // ── Bookmarks
    BOOKMARK_STORE    => "graph.bookmark_store", "Store Bookmark", "Bookmarks", Canvas, Live;
    BOOKMARK_RECALL_1 => "graph.bookmark_recall_1", "Recall Bookmark 1", "Bookmarks", Canvas, Live;
    BOOKMARK_RECALL_2 => "graph.bookmark_recall_2", "Recall Bookmark 2", "Bookmarks", Canvas, Live;
    BOOKMARK_RECALL_3 => "graph.bookmark_recall_3", "Recall Bookmark 3", "Bookmarks", Canvas, Live;
    BOOKMARK_RECALL_4 => "graph.bookmark_recall_4", "Recall Bookmark 4", "Bookmarks", Canvas, Live;
    BOOKMARK_RECALL_5 => "graph.bookmark_recall_5", "Recall Bookmark 5", "Bookmarks", Canvas, Live;

    // ── Search and validation
    FIND         => "graph.find", "Find in Graph", "Search", GraphTab, Live;
    NEXT_ERROR   => "graph.next_error", "Next Validation Error", "Search", GraphTab, Live;
    PREV_ERROR   => "graph.prev_error", "Previous Validation Error", "Search", GraphTab, Live;
    COMPILE      => "graph.compile", "Compile", "Search", GraphTab, Live;
    PURGE_UNUSED => "graph.purge_unused", "Purge Unused Nodes", "Search", GraphTab, Live;

    // ── Debugging
    TOGGLE_BREAKPOINT => "graph.toggle_breakpoint", "Toggle Breakpoint", "Debugging", Canvas, Live;
    CLEAR_BREAKPOINTS => "graph.clear_breakpoints", "Clear All Breakpoints", "Debugging", GraphTab, Live;
    DEBUG_RESUME      => "graph.debug_resume", "Resume Execution", "Debugging", GraphTab, Live;
    DEBUG_STEP        => "graph.debug_step", "Step One Node", "Debugging", GraphTab, Live;
    DEBUG_STOP        => "graph.debug_stop", "Stop Session", "Debugging", GraphTab, Live;

    // ── Input model (structural, listed for the cheat sheet)
    CANCEL => "editor.cancel", "Dismiss / Abort / Revert", "Input Model", Global, Fixed;
}

impl Action {
    pub fn all() -> impl Iterator<Item = Action> {
        (0..ACTIONS.len() as u16).map(Action)
    }

    pub fn from_id(id: &str) -> Option<Action> {
        ACTIONS.iter().position(|a| a.id == id).map(|i| Action(i as u16))
    }

    fn def(self) -> &'static ActionDef {
        &ACTIONS[self.0 as usize]
    }

    /// Stable id used in `keymap.ron`. Never rename one that has shipped.
    pub fn id(self) -> &'static str {
        self.def().id
    }

    /// Human name, as shown in preferences, menus and the cheat sheet.
    pub fn name(self) -> &'static str {
        self.def().name
    }

    /// Heading this action sits under in the preferences list.
    pub fn group(self) -> &'static str {
        self.def().group
    }

    pub fn context(self) -> Context {
        self.def().context
    }

    pub fn status(self) -> ActionStatus {
        self.def().status
    }
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Presets
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Preset {
    #[default]
    Crusty,
    Unreal,
}

impl Preset {
    pub const ALL: [Preset; 2] = [Preset::Crusty, Preset::Unreal];

    pub fn label(self) -> &'static str {
        match self {
            Preset::Crusty => "Crusty",
            Preset::Unreal => "Unreal Engine",
        }
    }
}

/// Which mouse *dialect* the input code speaks.
///
/// Mouse gestures are not individually rebindable (see the module header), but
/// the two dialects differ enough — Unreal toggles selection with Ctrl+click
/// where Crusty uses Shift, and breaks links with Alt+click on a pin where
/// Crusty also offers the slash-cut — that a whole-profile switch is worth
/// having. Pass C is where the input code reads this.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum MouseProfile {
    #[default]
    Crusty,
    Unreal,
}

impl MouseProfile {
    pub const ALL: [MouseProfile; 2] = [MouseProfile::Crusty, MouseProfile::Unreal];

    pub fn label(self) -> &'static str {
        match self {
            MouseProfile::Crusty => "Crusty",
            MouseProfile::Unreal => "Unreal Engine",
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Bindings and the map
// ─────────────────────────────────────────────────────────────────────────────

/// One chord bound to one action. `context` is derived from the action rather
/// than stored per binding: an action means one thing in one place, and
/// letting the two disagree would only create unresolvable states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Binding {
    pub action: Action,
    pub chord: Chord,
}

impl Binding {
    pub fn context(&self) -> Context {
        self.action.context()
    }
}

/// Two actions claiming one chord in one context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub chord: Chord,
    pub context: Context,
    pub first: Action,
    pub second: Action,
}

impl fmt::Display for Conflict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} is bound to both \"{}\" and \"{}\" in {}",
            self.chord.label(),
            self.first.name(),
            self.second.name(),
            self.context.label(),
        )
    }
}

/// The live keymap: every action's chords, plus the preset they came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keymap {
    pub preset: Preset,
    pub mouse_profile: MouseProfile,
    /// Action → its chords, primary first. An empty list means "unbound",
    /// which a user overlay may legitimately ask for.
    chords: BTreeMap<Action, Vec<Chord>>,
}

impl Default for Keymap {
    fn default() -> Self {
        Self::from_preset(Preset::Crusty)
    }
}

impl Keymap {
    pub fn from_preset(preset: Preset) -> Self {
        let mut km = Self {
            preset,
            mouse_profile: match preset {
                Preset::Crusty => MouseProfile::Crusty,
                Preset::Unreal => MouseProfile::Unreal,
            },
            chords: BTreeMap::new(),
        };
        for (action, chords) in preset_bindings(preset) {
            km.chords.insert(action, chords);
        }
        km
    }

    /// Every binding, flattened. Ordered by action, primary chord first.
    pub fn bindings(&self) -> impl Iterator<Item = Binding> + '_ {
        self.chords.iter().flat_map(|(a, cs)| {
            cs.iter().map(move |c| Binding { action: *a, chord: *c })
        })
    }

    pub fn chords_for(&self, action: Action) -> &[Chord] {
        self.chords.get(&action).map_or(&[], |v| v.as_slice())
    }

    /// The chord to show in a menu row or tooltip — an action's primary.
    pub fn chord_label(&self, action: Action) -> Option<String> {
        self.chords_for(action).first().map(|c| c.label())
    }

    /// Which action `chord` triggers with `ctx` active, walking the context
    /// chain outward so a specific binding shadows a `Global` one.
    pub fn resolve(&self, chord: Chord, ctx: Context) -> Option<Action> {
        for scope in ctx.chain() {
            if let Some(a) = self
                .bindings()
                .find(|b| b.chord == chord && b.context() == *scope)
            {
                return Some(a.action);
            }
        }
        None
    }

    /// Every action triggered by this frame's key presses, in `ctx`.
    ///
    /// Only [`ActionStatus::Live`] actions come back: an `Unimplemented`
    /// binding is listed everywhere a user might look for it, but pressing it
    /// does nothing until Pass C fills the handler in — quietly, because a
    /// "not yet" toast on every arrow key would be worse than silence. `Fixed`
    /// actions are handled by the input model before dispatch ever runs.
    pub fn dispatch(&self, input: &InputState, ctx: Context) -> Vec<Action> {
        let mut out = Vec::new();
        for kp in &input.key_presses {
            if let Some(a) = self.resolve(Chord::new(kp.key, kp.modifiers), ctx) {
                if a.status().dispatchable() && !out.contains(&a) {
                    out.push(a);
                }
            }
        }
        out
    }

    /// Rows the Preferences ▸ Keyboard Shortcuts page renders, in display
    /// order: by context (most specific first), then by group, then by name.
    ///
    /// The page builds *only* from this, which is what makes "every action is
    /// reachable in preferences" a property of the data rather than a promise
    /// somebody has to keep by hand.
    pub fn rows(&self) -> Vec<PrefsRow> {
        let mut rows: Vec<PrefsRow> = Action::all()
            .map(|action| PrefsRow {
                action,
                chords: self.chords_for(action).to_vec(),
            })
            .collect();
        rows.sort_by_key(|r| {
            let ctx = Context::ALL
                .iter()
                .position(|c| *c == r.action.context())
                .unwrap_or(usize::MAX);
            (ctx, r.action.group(), r.action.name())
        });
        rows
    }

    /// [`rows`](Self::rows) filtered by a search over name, group and chord —
    /// so typing `F9` finds the breakpoint rows and `align` finds the strip.
    pub fn rows_matching(&self, query: &str) -> Vec<PrefsRow> {
        let q = query.trim().to_ascii_lowercase();
        if q.is_empty() {
            return self.rows();
        }
        self.rows()
            .into_iter()
            .filter(|r| {
                r.action.name().to_ascii_lowercase().contains(&q)
                    || r.action.group().to_ascii_lowercase().contains(&q)
                    || r.chords.iter().any(|c| c.label().to_ascii_lowercase().contains(&q))
            })
            .collect()
    }

    pub fn set_chords(&mut self, action: Action, chords: Vec<Chord>) {
        self.chords.insert(action, chords);
    }

    /// Restore one action to its preset default.
    pub fn reset(&mut self, action: Action) {
        let d = Keymap::from_preset(self.preset);
        self.chords.insert(action, d.chords_for(action).to_vec());
    }

    /// Same chord, same context, two actions. Cross-context collisions are
    /// *not* conflicts — that is shadowing, and it is how the map is meant to
    /// work.
    pub fn conflicts(&self) -> Vec<Conflict> {
        let mut seen: BTreeMap<(Context, Chord), Action> = BTreeMap::new();
        let mut out = Vec::new();
        for b in self.bindings() {
            let k = (b.context(), b.chord);
            match seen.get(&k) {
                Some(first) => out.push(Conflict {
                    chord: b.chord,
                    context: b.context(),
                    first: *first,
                    second: b.action,
                }),
                None => {
                    seen.insert(k, b.action);
                }
            }
        }
        out
    }

    pub fn path() -> PathBuf {
        PathBuf::from(KEYMAP_FILE)
    }

    /// Read `keymap.ron` and overlay it on the preset's defaults.
    ///
    /// Returns the map plus any problems worth telling the user about. A
    /// broken or conflicting file is never fatal: the caller gets working
    /// defaults and a message, because an editor that will not open is a worse
    /// outcome than an editor with the stock bindings.
    pub fn load() -> (Keymap, Vec<String>) {
        let Ok(text) = std::fs::read_to_string(Self::path()) else {
            // No file is the normal case, not a problem.
            return (Keymap::default(), Vec::new());
        };
        match ron::from_str::<KeymapFile>(&text) {
            Ok(file) => Self::from_file(&file),
            Err(e) => (
                Keymap::default(),
                vec![format!("{KEYMAP_FILE} could not be read ({e}); using defaults")],
            ),
        }
    }

    /// Build from a parsed overlay, reporting unknown ids, bad chords and
    /// conflicts. Falls back to the untouched preset if the result conflicts.
    pub fn from_file(file: &KeymapFile) -> (Keymap, Vec<String>) {
        let base = Keymap::from_preset(file.preset);
        let mut km = base.clone();
        if let Some(mp) = file.mouse_profile {
            km.mouse_profile = mp;
        }
        let mut problems = Vec::new();

        for (id, chords) in &file.bindings {
            let Some(action) = Action::from_id(id) else {
                problems.push(format!("{KEYMAP_FILE}: unknown action `{id}` ignored"));
                continue;
            };
            km.chords.insert(action, chords.clone());
        }

        let conflicts = km.conflicts();
        if !conflicts.is_empty() {
            for c in &conflicts {
                problems.push(format!("{KEYMAP_FILE}: {c}"));
            }
            problems.push("using the default keymap instead".to_string());
            return (base, problems);
        }
        (km, problems)
    }

    /// Does this keymap differ from its own preset? The question the
    /// settings sidebar's modified dot asks — rebuilding the preset is
    /// cheaper than tracking a dirty flag through every rebind path, and it
    /// cannot drift out of sync with one.
    pub fn is_customized(&self) -> bool {
        let base = Keymap::from_preset(self.preset);
        self.chords != base.chords || self.mouse_profile != base.mouse_profile
    }

    pub fn to_file(&self) -> KeymapFile {
        KeymapFile {
            preset: self.preset,
            mouse_profile: Some(self.mouse_profile),
            bindings: self
                .chords
                .iter()
                .map(|(a, cs)| (a.id().to_string(), cs.clone()))
                .collect(),
        }
    }

    pub fn save(&self) -> Result<(), Box<dyn std::error::Error>> {
        let s = ron::ser::to_string_pretty(&self.to_file(), Default::default())?;
        std::fs::write(Self::path(), s)?;
        Ok(())
    }
}

/// One row of the Preferences ▸ Keyboard Shortcuts list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrefsRow {
    pub action: Action,
    /// Bound chords, primary first. Empty means unbound.
    pub chords: Vec<Chord>,
}

/// On-disk shape of `keymap.ron`. Only the actions a user changed need to
/// appear; the rest come from `preset`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct KeymapFile {
    pub preset: Preset,
    /// `None` follows the preset — a user who switched preset and never
    /// touched the mouse dialect should get that preset's dialect, not a
    /// stale default frozen into their file.
    pub mouse_profile: Option<MouseProfile>,
    /// Action id → its chords, primary first. An empty list unbinds it.
    pub bindings: BTreeMap<String, Vec<Chord>>,
}

// ─────────────────────────────────────────────────────────────────────────────
// The compiled-in presets
// ─────────────────────────────────────────────────────────────────────────────

fn c(s: &str) -> Vec<Chord> {
    vec![Chord::parse(s).expect("compiled-in preset chord must parse")]
}

fn cs(list: &[&str]) -> Vec<Chord> {
    list.iter()
        .map(|s| Chord::parse(s).expect("compiled-in preset chord must parse"))
        .collect()
}

fn preset_bindings(preset: Preset) -> Vec<(Action, Vec<Chord>)> {
    let mut v = crusty_bindings();
    if preset == Preset::Unreal {
        for (action, chords) in unreal_overrides() {
            match v.iter_mut().find(|(a, _)| *a == action) {
                Some(slot) => slot.1 = chords,
                None => v.push((action, chords)),
            }
        }
    }
    v
}

/// The Crusty preset — the canvas-focus keyboard table.
fn crusty_bindings() -> Vec<(Action, Vec<Chord>)> {
    vec![
        (Action::ADD_NODE_PALETTE, c("Tab")),
        (Action::DELETE_SELECTION, c("Del")),
        (Action::DELETE_AND_HEAL, c("Shift+Del")),
        // Quick-place is `<key>+click`, and which key comes from the node
        // descriptor's `quick_key`, not from here — it has no fixed chord.
        (Action::QUICK_PLACE, vec![]),
        (Action::CUT, c("Ctrl+X")),
        (Action::COPY, c("Ctrl+C")),
        (Action::PASTE, c("Ctrl+V")),
        (Action::DUPLICATE, c("Ctrl+D")),
        (Action::UNDO, c("Ctrl+Z")),
        // Two chords, one action: Windows' Ctrl+Y and the editor-flavoured
        // Ctrl+Shift+Z both redo, and both should show up in the cheat sheet.
        (Action::REDO, cs(&["Ctrl+Y", "Ctrl+Shift+Z"])),
        (Action::STRAIGHTEN, c("Q")),
        (Action::NUDGE_UP, c("Up")),
        (Action::NUDGE_DOWN, c("Down")),
        (Action::NUDGE_LEFT, c("Left")),
        (Action::NUDGE_RIGHT, c("Right")),
        (Action::NUDGE_UP_FINE, c("Shift+Up")),
        (Action::NUDGE_DOWN_FINE, c("Shift+Down")),
        (Action::NUDGE_LEFT_FINE, c("Shift+Left")),
        (Action::NUDGE_RIGHT_FINE, c("Shift+Right")),
        (Action::GROUP, c("C")),
        (Action::COMMENT, c("Shift+C")),
        (Action::COLLAPSE, c("Ctrl+G")),
        (Action::RENAME, c("F2")),
        (Action::ALIGN_STRIP, c("Alt+A")),
        (Action::ALIGN_TOP, c("Shift+W")),
        (Action::ALIGN_LEFT, c("Shift+A")),
        (Action::ALIGN_BOTTOM, c("Shift+S")),
        (Action::ALIGN_RIGHT, c("Shift+D")),
        (Action::ALIGN_CENTER_H, c("Alt+Shift+W")),
        (Action::ALIGN_CENTER_V, c("Alt+Shift+S")),
        (Action::AUTO_LAYOUT, c("Alt+L")),
        (Action::FRAME_SELECTION, c("F")),
        (Action::FIT_GRAPH, c("Home")),
        (Action::PARENT_GRAPH, c("PageUp")),
        (Action::CHILD_GRAPH, c("PageDown")),
        // Alt+V joins the Alt+A / Alt+L graph-tool family. **Not Ctrl+B**:
        // that is `graph.bookmark_store` in `Canvas`, which shadows `GraphTab`
        // whenever the canvas has the pointer — the binding would be dead
        // exactly where it is needed rather than conflicting visibly.
        (Action::TOGGLE_VARIABLES, c("Alt+V")),
        (Action::BOOKMARK_STORE, c("Ctrl+B")),
        (Action::BOOKMARK_RECALL_1, c("Shift+1")),
        (Action::BOOKMARK_RECALL_2, c("Shift+2")),
        (Action::BOOKMARK_RECALL_3, c("Shift+3")),
        (Action::BOOKMARK_RECALL_4, c("Shift+4")),
        (Action::BOOKMARK_RECALL_5, c("Shift+5")),
        (Action::FIND, c("Ctrl+F")),
        (Action::NEXT_ERROR, c("F8")),
        (Action::PREV_ERROR, c("Shift+F8")),
        (Action::COMPILE, c("F7")),
        (Action::PURGE_UNUSED, c("Ctrl+Alt+K")),
        (Action::TOGGLE_BREAKPOINT, c("F9")),
        (Action::CLEAR_BREAKPOINTS, c("Ctrl+Shift+F9")),
        // **Not F5**, which the mockup asks for and this editor cannot give:
        // `App` handles F5 as Play/Stop at the winit level, before any keymap
        // context is consulted and without looking at modifiers — so F5,
        // Shift+F5 and Ctrl+F5 all toggle play, and a GraphTab binding on any
        // of them would resume the debugger *and* end the session that owns
        // it. F6 is Pause/Resume play for the same reason. F11 is free, keeps
        // the debugger in the function-key family the other two Debugging
        // actions already live in, and is one key from Step.
        //
        // Step keeps the mockup's F10 exactly: nothing binds it, and no
        // Canvas-context binding shadows it (the P6c lesson — a Canvas chord
        // wins over a GraphTab one whenever the pointer is over the canvas,
        // which is where a person pressing Step has their pointer).
        //
        // Stop ships unbound, as the mockup draws it: a session-ending verb
        // with a hair-trigger function key is a bad trade, and the row is in
        // Preferences for anyone who wants one.
        (Action::DEBUG_RESUME, c("F11")),
        (Action::DEBUG_STEP, c("F10")),
        (Action::DEBUG_STOP, vec![]),
        (Action::CANCEL, c("Esc")),
    ]
}

/// The Unreal preset — keyboard differences only.
///
/// The substantive Unreal divergences are *mouse* semantics (Ctrl+click to
/// toggle selection, Ctrl+drag to move a wire), which travel in
/// [`MouseProfile`] and are consumed by the input code in Pass C. On the
/// keyboard the two are close, because the Crusty table is already
/// Unreal-influenced — Q, F7, F9 and the clipboard set are UE's. What differs
/// is the handful below.
fn unreal_overrides() -> Vec<(Action, Vec<Chord>)> {
    vec![
        // UE has one annotation, the comment box, on a bare C.
        (Action::COMMENT, c("C")),
        (Action::GROUP, vec![]),
        // UE ships alignment on the right-click menu with no default keys.
        (Action::ALIGN_TOP, vec![]),
        (Action::ALIGN_LEFT, vec![]),
        (Action::ALIGN_BOTTOM, vec![]),
        (Action::ALIGN_RIGHT, vec![]),
        (Action::ALIGN_CENTER_H, vec![]),
        (Action::ALIGN_CENTER_V, vec![]),
        // UE frames the selection with F and has no fit-graph key.
        (Action::FIT_GRAPH, vec![]),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Chord round-trip and formatting

    #[test]
    fn chord_label_matches_the_documented_spelling() {
        let c = Chord::new(Key::F(9), Modifiers::CTRL | Modifiers::SHIFT);
        assert_eq!(c.label(), "Ctrl+Shift+F9");
        assert_eq!(Chord::plain(Key::Delete).label(), "Del");
        assert_eq!(Chord::shift(Key::Char('c')).label(), "Shift+C");
        assert_eq!(Chord::plain(Key::ArrowUp).label(), "Up");
        assert_eq!(
            Chord::new(Key::Char('w'), Modifiers::ALT | Modifiers::SHIFT).label(),
            "Alt+Shift+W",
        );
    }

    #[test]
    fn modifier_order_is_fixed_so_one_chord_has_one_spelling() {
        let a = Chord::parse("Shift+Ctrl+F9").unwrap();
        let b = Chord::parse("Ctrl+Shift+F9").unwrap();
        assert_eq!(a, b, "order on the way in does not matter");
        assert_eq!(a.label(), b.label());
        assert_eq!(a.label(), "Ctrl+Shift+F9", "and on the way out it is fixed");
    }

    #[test]
    fn every_chord_round_trips_through_its_label() {
        for (_, chords) in preset_bindings(Preset::Crusty) {
            for c in chords {
                let back = Chord::parse(&c.label()).expect("label must re-parse");
                assert_eq!(back, c, "{} did not round-trip", c.label());
            }
        }
    }

    #[test]
    fn chords_round_trip_through_ron() {
        let km = Keymap::default();
        let text = ron::ser::to_string_pretty(&km.to_file(), Default::default()).unwrap();
        assert!(text.contains("\"Ctrl+Shift+F9\""), "chords store as labels:\n{text}");
        let back: KeymapFile = ron::from_str(&text).unwrap();
        let (rebuilt, problems) = Keymap::from_file(&back);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(rebuilt, km);
    }

    #[test]
    fn chord_parsing_rejects_nonsense_without_panicking() {
        assert!(Chord::parse("").is_err());
        assert!(Chord::parse("Hyper+A").is_err());
        assert!(Chord::parse("Ctrl+NotAKey").is_err());
        // Function keys never read as the letter F.
        assert_eq!(Chord::parse("F5").unwrap().key, Key::F(5));
        assert_eq!(Chord::parse("F").unwrap().key, Key::Char('f'));
        // Case and aliases are forgiving on input.
        assert_eq!(Chord::parse("ctrl+delete").unwrap(), Chord::ctrl(Key::Delete));
        assert_eq!(Chord::parse("CTRL+Del").unwrap(), Chord::ctrl(Key::Delete));
    }

    // ── Action inventory

    #[test]
    fn action_ids_are_unique_and_resolvable() {
        let mut ids: Vec<&str> = Action::all().map(|a| a.id()).collect();
        let n = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), n, "duplicate action id");
        for a in Action::all() {
            assert_eq!(Action::from_id(a.id()), Some(a));
            assert!(!a.name().is_empty());
            assert!(!a.group().is_empty());
        }
        assert!(n >= 45, "the inventory should cover the table; got {n}");
    }

    #[test]
    fn the_crusty_preset_binds_every_action_it_can() {
        let km = Keymap::from_preset(Preset::Crusty);
        for a in Action::all() {
            // Quick-place's key comes from the node descriptor, not the keymap.
            // Stop-session ships unbound on purpose (GS-4): the mockup draws it
            // as a button with no chord, and ending a session on a stray
            // function key is a bad trade.
            if a == Action::QUICK_PLACE || a == Action::DEBUG_STOP {
                continue;
            }
            assert!(
                !km.chords_for(a).is_empty(),
                "{} has no default chord",
                a.id()
            );
        }
    }

    #[test]
    fn unimplemented_actions_are_bound_but_not_dispatchable() {
        let km = Keymap::from_preset(Preset::Crusty);
        // The table lists them, so the user can find the key; dispatch no-ops
        // until Pass C fills the handler in.
        // (QUICK_PLACE is deliberately absent: its key comes from the node
        // descriptor, so it is the one unimplemented action with no chord.)
        for a in [Action::STRAIGHTEN, Action::DELETE_AND_HEAL] {
            assert_eq!(a.status(), ActionStatus::Unimplemented, "{}", a.id());
            assert!(!a.status().dispatchable());
            assert!(!km.chords_for(a).is_empty(), "{} is still listed", a.id());
        }
        assert_eq!(Action::CANCEL.status(), ActionStatus::Fixed);
        assert!(Action::DELETE_SELECTION.status().dispatchable());
    }

    // ── Conflicts and shadowing

    #[test]
    fn both_shipped_presets_are_conflict_free() {
        for p in Preset::ALL {
            let km = Keymap::from_preset(p);
            assert!(km.conflicts().is_empty(), "{:?}: {:?}", p, km.conflicts());
        }
    }

    #[test]
    fn a_duplicate_in_one_context_is_a_conflict_and_names_both() {
        let mut km = Keymap::default();
        // Both live in Canvas.
        km.set_chords(Action::GROUP, c("Ctrl+G"));
        let conflicts = km.conflicts();
        assert_eq!(conflicts.len(), 1);
        let msg = conflicts[0].to_string();
        assert!(msg.contains("Group Selection"), "{msg}");
        assert!(msg.contains("Collapse to Subgraph"), "{msg}");
        assert!(msg.contains("Ctrl+G"), "{msg}");
        assert!(msg.contains("Graph Canvas"), "{msg}");
    }

    #[test]
    fn the_same_chord_in_different_contexts_is_shadowing_not_conflict() {
        let mut km = Keymap::default();
        // FIND is GraphTab; give a Canvas action the same chord.
        km.set_chords(Action::GROUP, c("Ctrl+F"));
        assert!(
            km.conflicts().is_empty(),
            "a nested context may legitimately shadow an outer one"
        );
        let chord = Chord::ctrl(Key::Char('f'));
        assert_eq!(
            km.resolve(chord, Context::Canvas),
            Some(Action::GROUP),
            "over the canvas the specific binding wins"
        );
        assert_eq!(
            km.resolve(chord, Context::GraphTab),
            Some(Action::FIND),
            "elsewhere in the tab the outer one is still reachable"
        );
    }

    #[test]
    fn resolution_walks_outward_to_global() {
        let km = Keymap::default();
        let undo = Chord::ctrl(Key::Char('z'));
        assert_eq!(km.resolve(undo, Context::Canvas), Some(Action::UNDO));
        assert_eq!(km.resolve(undo, Context::GraphTab), Some(Action::UNDO));
        assert_eq!(km.resolve(undo, Context::Global), Some(Action::UNDO));
        // A canvas binding is not reachable from outside the canvas.
        let group = Chord::plain(Key::Char('c'));
        assert_eq!(km.resolve(group, Context::Canvas), Some(Action::GROUP));
        assert_eq!(km.resolve(group, Context::GraphTab), None);
    }

    #[test]
    fn typing_suppresses_the_graph_chain_but_not_global_shortcuts() {
        let km = Keymap::default();
        assert_eq!(
            km.resolve(Chord::plain(Key::Char('c')), Context::TextEntry),
            None,
            "a bare C must type a letter, not group the selection"
        );
        assert_eq!(
            km.resolve(Chord::ctrl(Key::Char('z')), Context::TextEntry),
            Some(Action::UNDO),
            "but Ctrl+Z still reaches Global",
        );
    }

    // ── Overlay merge

    #[test]
    fn a_user_overlay_merges_by_action_id_and_leaves_the_rest_alone() {
        let file = KeymapFile {
            preset: Preset::Crusty,
            mouse_profile: None,
            bindings: [("graph.find".to_string(), vec![Chord::parse("Ctrl+Shift+F").unwrap()])]
                .into_iter()
                .collect(),
        };
        let (km, problems) = Keymap::from_file(&file);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(km.chord_label(Action::FIND).as_deref(), Some("Ctrl+Shift+F"));
        assert_eq!(
            km.chord_label(Action::DELETE_SELECTION).as_deref(),
            Some("Del"),
            "an untouched action keeps its preset chord"
        );
        assert_eq!(km.resolve(Chord::ctrl(Key::Char('f')), Context::GraphTab), None);
    }

    #[test]
    fn an_overlay_may_unbind_an_action() {
        let file = KeymapFile {
            bindings: [("graph.group".to_string(), vec![])].into_iter().collect(),
            ..Default::default()
        };
        let (km, problems) = Keymap::from_file(&file);
        assert!(problems.is_empty(), "{problems:?}");
        assert!(km.chords_for(Action::GROUP).is_empty());
        assert_eq!(km.chord_label(Action::GROUP), None);
    }

    #[test]
    fn an_unknown_action_id_is_reported_and_skipped_not_fatal() {
        let file = KeymapFile {
            bindings: [("graph.no_such_thing".to_string(), c("Ctrl+J"))]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let (km, problems) = Keymap::from_file(&file);
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("graph.no_such_thing"), "{problems:?}");
        assert_eq!(km.chord_label(Action::FIND).as_deref(), Some("Ctrl+F"));
    }

    #[test]
    fn a_conflicting_overlay_falls_back_to_defaults_and_says_why() {
        let file = KeymapFile {
            bindings: [("graph.group".to_string(), c("Ctrl+G"))].into_iter().collect(),
            ..Default::default()
        };
        let (km, problems) = Keymap::from_file(&file);
        assert!(
            problems.iter().any(|p| p.contains("Group Selection")
                && p.contains("Collapse to Subgraph")),
            "the message must name both actions: {problems:?}"
        );
        assert!(problems.iter().any(|p| p.contains("using the default keymap")));
        assert_eq!(
            km,
            Keymap::from_preset(Preset::Crusty),
            "the editor still opens, on stock bindings"
        );
    }

    #[test]
    fn an_overlay_can_switch_preset_and_the_unreal_one_differs() {
        let file = KeymapFile { preset: Preset::Unreal, ..Default::default() };
        let (km, problems) = Keymap::from_file(&file);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(km.chord_label(Action::COMMENT).as_deref(), Some("C"));
        assert!(km.chords_for(Action::GROUP).is_empty());
        assert!(km.chords_for(Action::ALIGN_TOP).is_empty());
        // The mouse dialect travels with the preset.
        assert_eq!(km.mouse_profile, MouseProfile::Unreal);
        // Shared ground stays shared.
        assert_eq!(km.chord_label(Action::UNDO).as_deref(), Some("Ctrl+Z"));
    }

    #[test]
    fn resetting_one_action_restores_only_that_one() {
        let mut km = Keymap::default();
        km.set_chords(Action::FIND, c("Ctrl+Shift+P"));
        km.set_chords(Action::AUTO_LAYOUT, c("Alt+Y"));
        km.reset(Action::FIND);
        assert_eq!(km.chord_label(Action::FIND).as_deref(), Some("Ctrl+F"));
        assert_eq!(km.chord_label(Action::AUTO_LAYOUT).as_deref(), Some("Alt+Y"));
    }


    // ── Acceptance 14: every action is reachable in preferences

    #[test]
    fn acceptance_14_every_action_appears_in_the_preferences_page() {
        let km = Keymap::default();
        let rows = km.rows();
        assert_eq!(
            rows.len(),
            Action::all().count(),
            "the page builds from Action::all(), so the counts cannot drift"
        );
        let mut listed: Vec<&str> = rows.iter().map(|r| r.action.id()).collect();
        listed.sort_unstable();
        let mut every: Vec<&str> = Action::all().map(|a| a.id()).collect();
        every.sort_unstable();
        assert_eq!(listed, every);
        // Unimplemented actions are listed too — dimmed, not hidden, so a user
        // reading the table finds the key rather than wondering.
        assert!(rows
            .iter()
            .any(|r| r.action.status() == ActionStatus::Unimplemented));
    }

    #[test]
    fn preferences_rows_group_by_context_then_group_then_name() {
        let rows = Keymap::default().rows();
        let ctx_order: Vec<usize> = rows
            .iter()
            .map(|r| Context::ALL.iter().position(|c| *c == r.action.context()).unwrap())
            .collect();
        assert!(
            ctx_order.windows(2).all(|w| w[0] <= w[1]),
            "contexts must come out already grouped"
        );
        // Canvas is the most specific and leads the list.
        assert_eq!(rows[0].action.context(), Context::Canvas);
    }

    #[test]
    fn preferences_search_covers_name_group_and_chord() {
        let km = Keymap::default();
        assert!(km
            .rows_matching("F9")
            .iter()
            .any(|r| r.action == Action::TOGGLE_BREAKPOINT), "chord search");
        assert!(km
            .rows_matching("align")
            .iter()
            .any(|r| r.action == Action::ALIGN_STRIP), "name search");
        assert!(km
            .rows_matching("bookmarks")
            .iter()
            .any(|r| r.action == Action::BOOKMARK_STORE), "group search");
        assert_eq!(km.rows_matching("").len(), Action::all().count(), "empty = all");
        assert!(km.rows_matching("zzzznope").is_empty());
    }

    // ── Acceptance 15: a bound row shows its chord

    #[test]
    fn acceptance_15_every_menu_row_with_a_binding_can_display_it() {
        let km = Keymap::default();
        // The exact lookup the context menus use (`menu_row_for`).
        for action in [
            Action::DELETE_SELECTION,
            Action::AUTO_LAYOUT,
            Action::ALIGN_TOP,
            Action::ALIGN_LEFT,
            Action::ALIGN_BOTTOM,
            Action::ALIGN_RIGHT,
        ] {
            let label = km.chord_label(action);
            assert!(label.is_some(), "{} is bound, so its row must show it", action.id());
            assert!(!label.unwrap().is_empty());
        }
        assert_eq!(km.chord_label(Action::DELETE_SELECTION).as_deref(), Some("Del"));
        assert_eq!(km.chord_label(Action::ALIGN_TOP).as_deref(), Some("Shift+W"));
    }

    #[test]
    fn an_unbound_action_offers_no_chord_so_its_row_shows_none() {
        let mut km = Keymap::default();
        km.set_chords(Action::AUTO_LAYOUT, vec![]);
        assert_eq!(km.chord_label(Action::AUTO_LAYOUT), None);
    }

    // ── Dispatch

    #[test]
    fn dispatch_returns_live_actions_and_swallows_unimplemented_ones() {
        use crusty_gui::context::KeyPress;
        let km = Keymap::default();
        let press = |key, mods| {
            let mut i = InputState::default();
            i.key_presses = vec![KeyPress { key, modifiers: mods, repeat: false }];
            i
        };

        assert_eq!(
            km.dispatch(&press(Key::Char('c'), Modifiers::empty()), Context::Canvas),
            vec![Action::GROUP],
        );
        // Bound, listed, but its handler lands in Pass C: nothing fires, and
        // nothing complains.
        assert!(
            km.dispatch(&press(Key::Char('q'), Modifiers::empty()), Context::Canvas).is_empty(),
            "an unimplemented binding dispatches to nothing, quietly"
        );
        // Canvas-only bindings are unreachable from the tab context.
        assert!(km
            .dispatch(&press(Key::Char('c'), Modifiers::empty()), Context::GraphTab)
            .is_empty());
        // ...but Global ones reach both.
        assert_eq!(
            km.dispatch(&press(Key::Char('z'), Modifiers::CTRL), Context::GraphTab),
            vec![Action::UNDO],
        );
    }

    #[test]
    fn bare_a_no_longer_fits_the_graph_and_home_does() {
        let km = Keymap::default();
        use crusty_gui::context::KeyPress;
        let press = |key| {
            let mut i = InputState::default();
            i.key_presses =
                vec![KeyPress { key, modifiers: Modifiers::empty(), repeat: false }];
            i
        };
        assert!(
            km.dispatch(&press(Key::Char('a')), Context::Canvas).is_empty(),
            "bare A is freed for the Shift+W/A/S/D align family"
        );
        assert_eq!(km.dispatch(&press(Key::Home), Context::Canvas), vec![Action::FIT_GRAPH]);
        assert_eq!(
            km.dispatch(&press(Key::Char('f')), Context::Canvas),
            vec![Action::FRAME_SELECTION],
            "F keeps frame-selection",
        );
    }


    // ── Rebind persistence and preset rebasing (B2b)

    #[test]
    fn a_rebind_writes_the_overlay_and_survives_a_reload() {
        let mut km = Keymap::default();
        km.set_chords(Action::FIND, c("Ctrl+Shift+P"));

        // What the debounced autosave would write.
        let text = ron::ser::to_string_pretty(&km.to_file(), Default::default()).unwrap();
        // ...and what the next launch reads back.
        let file: KeymapFile = ron::from_str(&text).unwrap();
        let (reloaded, problems) = Keymap::from_file(&file);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(reloaded.chord_label(Action::FIND).as_deref(), Some("Ctrl+Shift+P"));
        assert_eq!(reloaded, km, "a round-trip through disk changes nothing");
        assert_eq!(
            reloaded.resolve(Chord::ctrl(Key::Char('f')), Context::GraphTab),
            None,
            "and the old chord really is gone"
        );
    }

    #[test]
    fn an_unbind_survives_a_reload_rather_than_reverting_to_the_default() {
        let mut km = Keymap::default();
        km.set_chords(Action::AUTO_LAYOUT, vec![]);
        let text = ron::ser::to_string_pretty(&km.to_file(), Default::default()).unwrap();
        let file: KeymapFile = ron::from_str(&text).unwrap();
        let (reloaded, _) = Keymap::from_file(&file);
        assert!(
            reloaded.chords_for(Action::AUTO_LAYOUT).is_empty(),
            "an empty list is a decision, not an absence"
        );
    }

    #[test]
    fn switching_preset_rebases_and_drops_the_old_overlay() {
        let mut km = Keymap::default();
        km.set_chords(Action::FIND, c("Ctrl+Shift+P"));
        km.set_chords(Action::COMMENT, c("Alt+M"));

        // What the preset dropdown does.
        km = Keymap::from_preset(Preset::Unreal);

        assert_eq!(km.preset, Preset::Unreal);
        assert_eq!(
            km.chord_label(Action::FIND).as_deref(),
            Some("Ctrl+F"),
            "the old rebinding is gone — switching preset means \"give me that preset\""
        );
        assert_eq!(
            km.chord_label(Action::COMMENT).as_deref(),
            Some("C"),
            "and the new preset's own value applies"
        );
        assert_eq!(km.mouse_profile, MouseProfile::Unreal, "the dialect follows");
        assert_eq!(km, Keymap::from_preset(Preset::Unreal), "nothing lingers");
    }

    /// **GS-4's keymap evidence.** The mockup asks for Resume F5 / Step F10.
    /// F10 ships as drawn; F5 cannot, and the reason is not a conflict this
    /// map can see: `App` handles F5 (Play/Stop) and F6 (Pause/Resume play)
    /// as raw winit key events, before any context is consulted and ignoring
    /// modifiers, so nothing here would ever hear them. Resume therefore ships
    /// on F11 — and the banner draws whatever the map says, so a rebind
    /// re-labels the button rather than lying about it.
    #[test]
    fn the_debug_actions_are_conflict_free_and_unshadowed() {
        for p in Preset::ALL {
            let km = Keymap::from_preset(p);
            assert!(km.conflicts().is_empty(), "{p:?}: {:?}", km.conflicts());

            for (action, want) in
                [(Action::DEBUG_RESUME, "F11"), (Action::DEBUG_STEP, "F10")]
            {
                let chord = Chord::parse(want).unwrap();
                assert_eq!(km.chord_label(action).as_deref(), Some(want));
                // The P6c lesson: a Canvas binding shadows a GraphTab one
                // whenever the pointer is over the canvas — which is exactly
                // where the pointer is when someone presses Step. Resolving
                // from the innermost context is the only honest check.
                assert_eq!(
                    km.resolve(chord, Context::Canvas),
                    Some(action),
                    "{want} must survive the Canvas context, not just GraphTab"
                );
            }

            // Nothing claims the play-transport keys, in any context.
            for taken in ["F5", "F6", "Shift+F5", "Ctrl+F5"] {
                let chord = Chord::parse(taken).unwrap();
                assert_eq!(
                    km.resolve(chord, Context::Canvas),
                    None,
                    "{taken} belongs to the play transport, which the keymap never sees"
                );
            }

            // Stop ships unbound, as the mockup draws it — listed, rebindable,
            // and not on a hair trigger.
            assert!(km.chords_for(Action::DEBUG_STOP).is_empty());
            assert!(
                km.rows().iter().any(|r| r.action == Action::DEBUG_STOP),
                "an unbound action is still a row in Preferences"
            );
        }
    }

    #[test]
    fn a_rebind_onto_a_taken_chord_is_detectable_before_it_is_applied() {
        // The page probes a copy rather than applying and undoing, so an
        // accidental collision never transiently steals a live binding.
        let km = Keymap::default();
        let mut probe = km.clone();
        probe.set_chords(Action::GROUP, c("Ctrl+G"));
        let clash = probe
            .conflicts()
            .into_iter()
            .find(|c| c.first == Action::GROUP || c.second == Action::GROUP)
            .expect("the collision must be visible on the probe");
        // Order within the pair is the map's, not the user's, so the page
        // reads "the other one" rather than assuming a side.
        let other = if clash.first == Action::GROUP { clash.second } else { clash.first };
        assert_eq!(other, Action::COLLAPSE, "and it names who already holds it");
        assert_eq!(
            km.chord_label(Action::GROUP).as_deref(),
            Some("C"),
            "while the live map is untouched until the user says rebind anyway"
        );
    }

    #[test]
    fn reset_all_returns_every_action_to_the_current_preset() {
        let mut km = Keymap::from_preset(Preset::Unreal);
        km.set_chords(Action::FIND, c("Ctrl+Shift+P"));
        km.set_chords(Action::UNDO, vec![]);
        km = Keymap::from_preset(km.preset);
        assert_eq!(km, Keymap::from_preset(Preset::Unreal));
    }

    #[test]
    fn a_hand_written_partial_file_parses_the_way_a_user_would_write_one() {
        // Exactly what the docs will show: two remapped actions, nothing else.
        let text = r#"(
            preset: Crusty,
            bindings: {
                "graph.find": ["Ctrl+Shift+F"],
                "graph.auto_layout": ["Alt+L", "Ctrl+Shift+L"],
            },
        )"#;
        let file: KeymapFile = ron::from_str(text).expect("hand-written file must parse");
        let (km, problems) = Keymap::from_file(&file);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(km.chord_label(Action::FIND).as_deref(), Some("Ctrl+Shift+F"));
        assert_eq!(km.chords_for(Action::AUTO_LAYOUT).len(), 2);
        assert_eq!(km.chord_label(Action::GROUP).as_deref(), Some("C"), "untouched");
        assert_eq!(km.mouse_profile, MouseProfile::Crusty, "absent field follows the preset");
    }

    #[test]
    fn an_action_may_carry_several_chords_and_the_first_is_the_label() {
        let km = Keymap::default();
        assert_eq!(km.chords_for(Action::REDO).len(), 2);
        assert_eq!(km.chord_label(Action::REDO).as_deref(), Some("Ctrl+Y"));
        for chord in ["Ctrl+Y", "Ctrl+Shift+Z"] {
            assert_eq!(
                km.resolve(Chord::parse(chord).unwrap(), Context::Canvas),
                Some(Action::REDO),
                "{chord} must redo"
            );
        }
    }
}
