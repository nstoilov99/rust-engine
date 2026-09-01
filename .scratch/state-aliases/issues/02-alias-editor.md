# 02 — Alias in the graph editor

**What to build:** `graph_editor_crusty.rs` / `graph_editor.rs` / `graph_anim_edge.rs`:
`AnimCardKind::Any` → `Alias` (pill with title, `ALIAS` tag, subtitle "Global" or names, per spec);
config rows for the selected alias (Global checkbox; when not global, one Bool row per state in doc
order that toggles the id in `states`, each an undoable property edit); `is_flow_source` and
`transition_shortcut` accept the alias; deleting a state strips its id from aliases (same undo
step); `upgrade_any_state` runs when a document opens in the editor (the doc becomes dirty only if
something was rewritten — surface it as a toast "Upgraded N Any State node(s) to aliases"); the
create palette lists "State Alias" and no longer "Any State". Anchored refusal badges from ticket 01
render like other domain errors (verify with a test through the existing error-index path). Update
the editor tests at `graph_editor.rs` ~8123 and `graph_editor_crusty.rs` ~4413. Do **not** edit
`content/graphs/character.animgraph` (it carries the user's uncommitted edits); the committed demo
migrates on open.

**Why:** the user wants Unreal's alias workflow on the canvas.

**Blocked by:** 01

**Status:** done

- [x] Opening `character.animgraph` shows the former Any State as a Global alias pill; selecting it shows the Global checkbox and state list; unchecking Global and ticking states updates the subtitle — *code + unit tests; not eyeballed live (see close-out)*
- [x] Dragging from the alias to a state makes a transition — `transition_shortcut` / `is_flow_source` accept the alias (test `a_state_to_state_wire_becomes_a_transition`)
- [x] `cargo test -p rust_engine --features editor` (sanctioned only), `cargo check -p game_client --features editor`

## Close-out (2026-09-01)

**Where things landed**

- `graph_editor.rs` — `GraphEditorState.migrated` (save-point flag; `after_edit` sets
  `dirty = stack.is_dirty() || migrated`, `save` clears it); `open()` runs `upgrade_any_state`
  for `.animgraph` and toasts "Upgraded N Any State node(s) to aliases"; `anchor_anim_refusal`
  gained the `alias '<name>'` arm (compiler name = title else "Alias"); `delete_selection` wraps
  `RemoveNodes` + one `SetProperty(states)` per touched alias in a `Composite` labelled like the
  plain delete; helpers `alias_name` / `alias_is_global` / `alias_states` / `alias_states_value` /
  `alias_strips`; `transition_shortcut` no longer knows the legacy id.
- `graph_editor_crusty.rs` — `AnimCardKind::Alias`; the pill is the ENTRY family (plain header
  fill, solid `stroke_strong` border, no category band) with the title in body text, `ALIAS` tag
  and a mono subtitle ("Global" / "Idle, Jump" / "Idle, Jump +2" / "No states"); a selected alias
  unfolds to the standard card via the same path as a state (pins stripped, hidden `out` anchor
  only); `config_rows` alias arm = Global Bool row + one Bool row per state in document order
  (keys `states.<id>`); `config_write_back` folds a state-row tick into the single `states`
  array so the edit and its undo are one `SetProperty`; `rule_state_name` fallback "Alias".
- `graph_anim_chip.rs` / `graph_anim_edge.rs` / `library.rs` — legacy `ANY` arms retired.

**What the user must eyeball (no editor was launched)**

1. Open the committed `character.animgraph`: the toast fires once, the tab is dirty, the old
   Any State draws as a pill titled "Any State" with `ALIAS` and subtitle "Global".
2. Select it: Global checkbox; untick → one row per state; tick two → subtitle "A, B", three →
   "A, B +1"; Ctrl+Z steps back one tick at a time.
3. Delete a listed state → its row and its name vanish from the alias; one Ctrl+Z restores both.
4. Pill width/height vs. a state card at rest (alias sizes to its text like ENTRY, two lines
   tall) — judge whether it wants the state's `min_w` instead.
5. Error badge: an alias with Global off and nothing ticked shows the red gutter dot and F8
   lands on it.

**Review (gpt-5.6-Sol, read-only, on the diff)** — three findings, all fixed before commit:
the unfolded (selected) alias could not start a border-drag wire (`graph_editor_crusty.rs`
border-rim gate now accepts the alias); alias refusals anchored on the first namesake
(`anchor_anim_refusal` now prefers the alias that actually offends — empty list / lists the
missing node — then the first by name; test covers both); `anim_state_name` now spells a blank
title exactly as the compiler does.

**Not done / deferred**

- The unfolded (selected) alias header uses `DocDescriptors::display_name` like every generic
  card, so an *untitled* alias reads "State Alias" there and "Alias" on its pill / in refusals.
  Cosmetic; title it and both agree.
