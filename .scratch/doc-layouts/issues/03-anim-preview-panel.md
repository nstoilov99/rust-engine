# 03 — Anim Preview panel

**What to build:** `EditorTab::AnimPreview`: generalise `blend_space_preview.rs` into a machine
preview that compiles the focused anim graph's document (`compile_anim_graph_with` + the disk
loader), runs `AnimMachine::tick` + `evaluate_pose` with parameters driven by the graph's preview
strip (and the strip's Trigger buttons), on the document's `preview_mesh` property (new, undoable,
same dropdown as the blend space) or auto-pick by bone coverage; `MeshPreviewRenderer` path, per-tab
texture, docked + float CBs exactly as the blend space pane does; orbit, Play/Pause, current state
+ fade overlay. Recompiles on save/undo (plan cache keyed by doc revision). Compile errors show as
a message in the pane.

**Blocked by:** 01

**Status:** done

- [ ] With Graph — character focused, the Preview shows the Defeated character in the entry state; toggling `Died` in the preview strip plays the transition — *not screenshotted (no editor launch from the agent); user verifies live*
- [x] Tests green (sanctioned only); `cargo check -p game_client --features editor`

## Built

- `engine/src/engine/editor/anim_graph_preview.rs` — `AnimGraphPreview`: compiles the focused
  document through `compile_anim_graph_with` + `DiskAnimAssets` (nested graphs and blend spaces
  resolve as the runtime's do), loads `plan.clip_refs()`, resolves the mesh (ENTRY node's
  `preview_mesh` or auto-pick by bone coverage), runs the real `AnimMachine` + `evaluate_pose` on
  its own `SkeletonInstance`. Plan cache keyed on `GraphEditorState::revision` (new counter: bumps
  on every edit/undo/redo, save, and host refresh after a nested file changed). An unchanged plan
  keeps the running machine; a changed one restarts at ENTRY with Float/Bool values carried over
  by name. `snapshot()` hands the strip an `AnimPreview` under `PANEL_INSTANCE_ID` (a compile
  error becomes a *refused* chip carrying the message); `mirror` copies a bound world runtime's
  plan/machine/params for read-only posing. `state_label()`: `Idle`, `Idle → Walk` mid-fade,
  `Locomotion / Run` inside a nested state.
- `engine/src/engine/editor/anim_preview_crusty.rs` — `skinned_preview_pane` (render target,
  orbit, chip, play/pause, clock, centred reason) shared by the blend space tab and the panel;
  `anim_preview_panel` (mesh + state chip, `LIVE · name` line and no play/pause while mirroring).
  `blend_space_editor_crusty::preview_pane` is now a thin wrapper over it.
- `PREVIEW_MESH_PROP` + `preview_mesh_of` in `animation/graph/plan.rs`;
  `GraphEditorState::{preview_mesh, set_preview_mesh}` — one undoable `SetProperty` on the ENTRY
  node (auto removes the property). Details panel: ENTRY node shows a "Preview Mesh" dropdown
  (`(auto) name` + every `.mesh`, warning when the chosen file is missing).
- Host (`app.rs`/`main.rs`): `scene.anim_previews` map (one entry per graph the panel drew last
  frame — `prune_anim_previews` drops the rest, targets included); `build_anim_preview_cbs` and
  the blend space builder share `record_skinned_preview`; `anim_preview` native registration
  (docked) / `ensure_mesh_texture` (float); `fold_anim_panel_previews` implements the ownership
  ruling (bound entity → strip drives it, panel mirrors; else the strip drives the panel's
  machine via `apply_anim_param_edits`).

## Close-out — what to eyeball

1. Open `Graph — character` with the AnimGraph profile: the Preview panel shows the mesh in the
   entry state, chip reads `<mesh> · <state>`; orbit / play-pause / clock work; the strip's chip
   reads `PREVIEW · Preview panel` with the parameter controls live (canvas highlight follows).
2. Toggle `Died` in the strip: chip reads `Idle → Defeated` during the fade, then `Defeated`.
3. Details ▸ select the ENTRY node ▸ Preview Mesh dropdown: pick a mesh (undo entry
   "Set preview_mesh"), `(auto)` restores auto-pick.
4. Enter play with a character selected: the strip binds the entity, the panel shows
   `LIVE · <name>` and mirrors it (no play/pause); deselect → back to the panel's own machine.
5. Break the graph (delete ENTRY): the pane shows the compiler's message, the strip chip reads
   `✕ REFUSED · Preview panel`; undo restores.
6. Float the Preview panel into its own window, then re-dock; close the graph tab (panel reads
   "No graph focused").
