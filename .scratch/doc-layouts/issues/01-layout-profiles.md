# 01 — Layout profiles: model, persistence, swap on focused document

**What to build:** Per the spec: `LayoutProfile` + `profile_of(tab, domain)`; `CrustyDockLayout` v2
(`profiles`, active profile, documents-leaf marker) with the v1 → v2 migration and the default
trees (new panels' tab ids may already appear in defaults — they render an empty-state body until
tickets 02–04 land; add the `EditorTab` variants + tab ids now); `swap_profile` (write-back,
gather, activate) in `dock_crusty.rs` with tests (round-trip, gather rule, v1 migration, marker
never rendered); host hook in `app.rs` after the dock frame: if `focused_tab` is a document with a
different profile, swap. Layout Save/Reset apply to the active profile; "Reset" offers all
profiles. Persist on exit as today.

**Why:** the mechanism everything else in this spec hangs on.

**Blocked by:** —

**Status:** done

- [ ] Clicking Graph — character swaps to the AnimGraph layout; clicking Main Scene swaps back; each remembers its own splits/panels across a restart — **user to eyeball** (the editor was not launched; mechanism covered by unit tests)
- [x] A v1 `editor_layout_crusty.ron` loads as the Scene profile unchanged (`v1_file_loads_as_the_scene_profile_verbatim`)
- [x] Tests green (sanctioned only); `cargo check -p game_client --features editor`

## Close-out

**What to eyeball.** Load your existing `editor_layout_crusty.ron` (a v1 file): it must come up
exactly as before. Click *Graph — character* in the viewport strip: Hierarchy/Inspector go away,
Variables (left) | strip | Preview over Details (right), Assets + Console below appear, with both
the viewport tab and the graph tab in the strip (graph in front). The three new panels show
"No graph focused" until tickets 02–03. Click *Main Scene*: the old Scene layout returns with
whatever you did to it. Move a splitter in each, restart, check both remembered. Also check
View ▸ Reset Layout (resets only the layout you are in) and View ▸ Reset All Layouts. Opening a
graph/curve/blend space from Assets now drops its tab into the document strip (next to the
viewports) instead of the least-crowded leaf.

**Swap rule (`CrustyDockLayout::swap_profile(to, focus)`).**
1. *Document strip* of the live tree = the leaf holding the most document tabs (ties → first in
   traversal order). Document tabs = `viewport:*`, `graph:*`, `curve:*`, `blendspace:*`, `mesh:*`,
   `ia:*`, `mc:*` (`dock_crusty::is_document`). Everything else is a side panel.
2. Write-back: push the marker `documents` into the strip leaf, then `close_tab` every document
   tab in the whole tree (strays docked elsewhere included; leaves that empty collapse). Store as
   `profiles[outgoing] = { tree, state }`.
3. Take `profiles.remove(to)` (or `default_tree(to)`; also if the stored tree lost its marker),
   splice all document tabs — strip order first, then strays in traversal order — where the marker
   was, `focus` becomes the leaf's active tab and `state.focused_tab`. `profiles` never contains
   the active profile.
4. Host (`App::sync_layout_profile`, after the dock frame + tab closes): `focused_document` =
   `DockState::focused_tab` when that is a document tab still in the tree; otherwise the previous
   value if still in the tree; otherwise the strip's front document. Profile =
   `profile_of(tab, loaded graph domain)` (graph unloaded → `ScriptGraph`, per the rulings); swap
   when it differs from `active`. Clicking a side panel never swaps and never clears
   `focused_document`.
5. Edit routing (`App::edit_target_document`, feeding `active_graph_key` / `active_curve_key` /
   `active_blend_space_key` and the Edit-menu overrides): the focused document — unless a
   scene-side panel (Hierarchy, Inspector, Assets, Console, …) holds the dock focus, which keeps
   today's scene routing (Delete after clicking Hierarchy still deletes the entity). The graph
   panels (Details, Variables, Preview) defer to the focused graph, so edits made in them undo on
   the graph's stack. Reconciles the ruling with the gpt-5.6-Sol review of this ticket.

**File format (`editor_layout_crusty.ron`, v2).**
```
(
    version: 2,
    tree: ...,            // the ACTIVE profile's live tree (real document tabs in the strip)
    state: (focused_tab: Some("graph:graphs/character.animgraph")),
    play_settings: (...), // unchanged
    active: AnimGraph,    // Scene | AnimGraph | ScriptGraph | BlendSpace | Curve | Mesh
    profiles: {           // inactive profiles only; strip = the marker tab "documents"
        Scene: (tree: ..., state: (focused_tab: None)),
    },
)
```
v1 files (no `version`/`active`/`profiles`) load as `active: Scene` with `tree`/`state`/
`play_settings` verbatim and are rewritten as v2 on the next save. A `documents` marker that leaks
into the live tree is stripped on load; the marker is never drawn.

**Not done here.** Panel bodies (02–03); tuning the default proportions against the Unreal
reference and docs (04). `EditorTab::to_window_kind` returns `None` for the three new panels
(function has no callers).
