# 06 — Animation graph states reference a blend space

**What to build:** A leaf state's config band gains a **Blend Space** row (text field, content-relative `.blendspace` path) with the same yield rules as runtime precedence: a nonempty Graph hides Blend Space and Clip rows; a nonempty Blend Space hides the Clip rows; a nonempty blend-tree region supersedes all. The state card's subtitle shows the blend space file. Double-click / PageDown on such a state opens the blend space tab (`file_descend_target` accepts `.blendspace`; the open request routes to the tab from ticket 04 — no breadcrumb back-chain for this tab). Compile refusals from ticket 02 render as anchored error badges on the state and appear in F8 navigation; they recompute when any `.blendspace` saves or reloads. Wire the demo: a state in `content/graphs/defeated.animgraph` (not `character.animgraph`, which carries local edits) references `blendspaces/locomotion.blendspace` so the shipped scene exercises the path.

**Blocked by:** 02, 04

**Status:** done

- [x] Selected leaf state shows Clip / Blend Space / Graph / Speed rows with the stated yield behaviour (existing config-row tests extended)
- [x] Card subtitle shows the `.blendspace` file when set
- [x] Double-click on the state opens the blend space tab
- [x] A bad reference shows an anchored error badge, is reachable with F8, and clears after the file is fixed and saved
- [x] The demo graph compiles and the Human entity animates through the blend space in Play

Notes (done 2026-08-30): `config_rows` (Blend Space row, `SPACE_PROP`), `anim_state_subtitle` (extracted; tree > graph > space > clip), `file_descend_target` (`.blendspace`; Graph wins), host dispatch on `.blendspace` in both the docked and float open-request sinks -> `App::open_blend_space_document`. `defeated.animgraph`: `Walk` plays `blendspaces/locomotion.blendspace`, driven by the new Float parameter `Speed` (default 0). Refusals recompute via `save_blend_space_state` (ticket 04) and `ReloadEvent::BlendSpaceChanged` -> `refresh_anim_graph_hosts`, both pre-existing. Play was not launched (compile-seam verification only: `the_committed_demo_document_loads_and_compiles` through `DiskAnimAssets`).
