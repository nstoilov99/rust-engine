# 02 — Graph Details and Variables panels

**What to build:** `EditorTab::GraphDetails` / `GraphVariables` bodies for the focused graph
document (resolve via the existing `active_graph_key()` pattern in `app.rs`): Details renders the
selection's config rows through the same helpers the inline bands use (edits go through the same
`GraphEdit` path, so undo/dirty/save behave identically; multi-selection → "N nodes selected";
nothing selected → the graph's own summary: domain, variables count, errors); Variables lifts the
strip (list, add, rename, type, default, the selected variable's footer inspector) into the panel
sharing the same state. Both show "No graph focused" when no graph tab is focused.

**Blocked by:** 01

**Status:** done

- [x] Selecting a state/transition/alias in the canvas shows its rows in Details; editing there updates the canvas and undoes as one step
- [x] Variables panel and the in-tab strip stay in sync
- [x] Tests green (sanctioned only); `cargo check -p game_client --features editor`

## Close-out (2026-09-01)

**Where it lives.** `engine/src/engine/editor/graph_dock_panels_crusty.rs` — `graph_details_panel` /
`GraphDetailsPanelCtx` and `graph_variables_panel` / `GraphVariablesPanelCtx`. Both are thin: Details
draws `config_rows` through `inline_widget` (the canvas band's own pair, now `pub(super)`), Variables
is `variables_panel` drawn `docked` (no collapse caret / rail / strip edge). The host builds the ctx
in both `app.rs` body matches (main dock and float windows) from the new `focused_graph_key()` —
`focused_document` resolved directly, *not* through the edit-routing gate, so clicking Assets or
Console never blanks the panels (spec ruling; dragging a clip from Assets into a Details row depends
on it). `active_graph_key()` still owns Edit routing.

**Twin-surface rules (strip + dock panel of one document drawn in the same frame).**
- Every variables-strip widget id is salted by a caller `root` (`Id`), path-included:
  `("graph_vars_strip", path)`, `("graph_vars_rule_strip", path)`, `("graph_vars_dock", path)`.
  Before this the strip's ids were global — two live twins would each apply the other's drag.
- The footer's rename field commits on blur only for the surface that *held* focus
  (`VarPanel::rename_owner`); the unfocused twin used to commit the other's half-typed name every
  keystroke (one undo entry per letter). Found by the Codex review.
- The New Group entry requests focus once (the frame its `+` menu item fired), not every frame.
- `inline_widget` takes its `Id` from the caller (`graph_inline` on the canvas,
  `graph_details_inline` under the path-salted Details root).
- The Details Name draft commits when the selection moves to another node (a selection change is
  a blur), on Enter, or on blur-after-focus; Esc drops it.

**Review.** Codex first pass (gpt-5.6-Sol, read-only) found the rename-twin bug, the New Group focus
fight, unsalted dock ids and the edit-gate misuse — all fixed above. The second pass was cut off by
a Codex usage limit; the rename-owner / name-draft / `GraphEdit` exhaustiveness checks were done by
hand (exhaustiveness is compiler-enforced: `SetNodeTitle` has apply / revert / description arms).

**Details content.** One node: header (state name / alias name / "A → B" for a transition, type
tag), a **Name** row for states and aliases (new `GraphEdit::SetNodeTitle` + `set_node_title`, "Rename
Node" undo label, commit on Enter or blur-after-focus via `GraphEditorState::details_rename`), a
**Rule** row for transitions (node count only — see deferred), then the config rows. No node selected
but a variable selected in the strip: the declaration read-only (slug, type, default, group, uses).
Nothing selected: file, domain, nodes, variables, errors. Multi: "N nodes selected".

**Locate from the dock panel.** It has no canvas, so `GraphEditorState::locate_request` parks the
node id and the tab frames it on its next draw.

**What the user must eyeball (not screenshotted — no editor launched).**
1. Open `content/graphs/character.animgraph`; the AnimGraph profile should show Variables (left) and
   Details (right-bottom). Click a state: Details shows Name / Clip / Blend Space / Graph / Speed; drag
   Speed — the node's band follows and one Ctrl+Z reverts it. Type a Name, press Enter — the card
   retitles; Undo label reads "Undo Rename Node".
2. Click a transition: header "Idle → Walk", Rule row, Duration / Priority.
3. With both the strip and the Variables panel visible: select a variable, rename it in *one* of the
   footers — the other must not fight (no per-keystroke commits; check the Undo label once).
4. Click Assets or Console: Details / Variables keep showing the graph.
5. Click a variable's usage count in the dock panel: the tab frames the node.

**Deferred / not done.**
- Transition **rule summary** as an expression ("Speed > 0.1"): no rule pretty-printer exists; the
  row shows the rule's node count. Needs a small compiler-side formatter — its own ticket.
- Script-graph nodes get header + config rows only (no inline pin constants in Details; the band has
  them). Same scope as the ticket text.
- The variables confirm dialog (retype/delete) is drawn by the tab, so a request raised in the dock
  panel shows its dialog over the tab — fine while the graph is visible, invisible if the tab is
  hidden behind another document in the same leaf.
