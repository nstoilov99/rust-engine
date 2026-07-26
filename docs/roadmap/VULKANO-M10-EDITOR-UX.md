# M10 — Editor UX & Design System v1 (Crusty Theme Tokens)

**Status:** ✅ Complete (P0–P9 delivered)
**Duration:** ~3–4 weeks (supersedes Task M1's 2-week box — scope grew from
"tokens + style guide + focus" to a full mockup-fidelity restyle plus the
settings windows; flagged and accepted)
**Prerequisites:** M9.6 complete; mockup package present locally in
`docs/mockup/` (gitignored)

## Goal

Implement the Crusty Design System mockup at **≥95% visual fidelity**:

1. **Semantic token system** — one `Theme` (surfaces / accents / invariant
   selection / status / axis / type colors / metrics / typography), four
   presets (Steel default, Tidepool, Graphite, Rusty), live-switchable.
   "If you type a hex value in widget code, it's a bug."
2. **Widget state ladder + keyboard focus** — every interactive widget gets
   default/hover/pressed/focused/disabled per the ladder; visible
   `focus_ring`, Tab traversal.
3. **Full Edit menu** (History / Edit / Configuration groups) and the two
   **settings windows**: Editor Preferences (`editor_prefs.ron`, user-local,
   live-apply) and Project Settings (`project.ron`, checked in, dirty+save).
4. **Panel restyle** to the mockup: chrome, hierarchy, inspector (axis
   fields, asset reference fields), asset browser, console, profiler,
   status bar with command field.

Source of truth: `docs/mockup/Crusty Design System.dc.html` (visual) +
`docs/mockup/theme.json` / `theme.rs` (values) + `docs/mockup/DESIGN.md`
(rules). DESIGN.md moves to `../crusty-gui/docs/DESIGN.md` as the living
style guide (M1's deliverable).

## Current seams (research)

- **crusty-gui `Style`** (`src/style/mod.rs`): 12-field `Palette`
  (`surface/surface_hover/surface_active/accent/accent_glow/text/text_dim/
  stroke/stroke_hover/success/panel/tab_bar`) — no `input`, `elevated`,
  `header`, `stroke_strong`, `focus_ring`, `accent_soft`, `selection`.
  Widgets already consume `ui.style()` exclusively (Phase 5 done), so a
  palette swap propagates mechanically. Presets: `editor_dark()` +
  `glassmorphism()` (to delete). Applied once at `crusty.rs:137` via
  `style_from_theme()`.
- **Engine theme** (`engine/src/engine/gui/…`): `EditorTheme` has
  `focus_ring` (`palette.rs:41`) and `ShadowTokens` but neither is routed
  into `Style`. `theme.rs` in the mockup package is already written against
  `crusty_gui::math::Color` — it becomes the new `EditorTheme` core.
- **Fonts**: `TextRenderer` (cosmic-text) supports `load_font()` +
  `set_default_family()` but **one family per renderer**; no font is
  bundled (OS-default fallback), no mono routing. Mockup needs IBM Plex
  Sans (UI) + JetBrains Mono (values/paths/shortcuts) — both SIL OFL,
  bundling is fine.
- **Focus**: `Memory` has `focused_widget` + `focusable` +
  `focus_next` (`memory.rs:120-194`); only TextEdit uses it. No focus-ring
  color, no traversal in panels.
- **Popover shadows**: `with_shadow(20, rgba(0,0,0,0.45))` hard-coded in
  `combo_box.rs:208`, `context_menu.rs:88`, `menu.rs:314`, `popup.rs:91`,
  `color_picker.rs:305/701`. The design system is **flat** — E3 = `elevated`
  × `popover_alpha` 0.96 + 1px `stroke_strong`, no shadows, no blur.
- **Hard-coded colors in engine panels** (`Color::rgb*` counts):
  hierarchy 16, inspector 14, menu_bar 12, asset_browser 12, mesh_editor 7,
  console 5, profiler 5, dialogs 3, command_palette 1.
- **Edit menu today** (`menu_bar_crusty.rs:118-136`): Undo/Redo only.
- **Persistence**: `CrustyDockLayout` → `editor_layout_crusty.ron`
  (`dock_crusty.rs:158`), including M9.6's `PlaySettings`. No
  `editor_prefs.ron` / `project.ron` exists. `editor_icon_palette.ron` is
  live (`icon_classes.rs`).
- **Modals**: `Window::modal(true)` + `DialogStack` work today — settings
  windows build on this, not on OS windows.

## Packages

Convention throughout: crusty-gui work lands in `../crusty-gui` first, both
repos build before each commit.

### P0 — Fonts (crusty-gui)

- Bundle IBM Plex Sans (Regular/Medium) + JetBrains Mono (Regular) TTFs +
  OFL licenses in crusty-gui; load at `TextRenderer` init.
- **Multi-family text runs**: extend the text pipeline so a run can select
  the mono family (cosmic-text `Attrs::family` per span — no second
  renderer). API: `ui.mono_label(…)` / a `FontChoice` on text params.
- Map the type scale 18/14/12/10.5 + mono 12 onto `Style::Fonts`
  (`Typography::comfortable()` already matches).
- **Degradation path** (this is the deepest-risk change and the whole
  restyle sits on it): if per-span family routing fights back (shaping,
  measurement, or glyph/layout caching), ship P0 as IBM-Plex-Sans-only —
  `mono_label` falls back to the UI family — and land mono routing
  mid-milestone. The token system doesn't depend on it; nothing else
  queues behind a text-pipeline debugging session.

### P1 — Token system (crusty-gui `Style` rebuild + engine theme)

- Replace crusty-gui `Palette` with the semantic set: 9 surfaces
  (`input/window/panel/header/elevated/hover/active/stroke/stroke_strong`),
  3 accents (`accent_active/focus_ring/accent_soft`), `selection`
  (fill/text), text (primary/secondary/disabled/mono), plus
  `popover_alpha`/`scrim_alpha` and the metric tokens (radii 2/3/6,
  border 1, edge 2, row 22, control 24, spacing scale, indent 18).
  Delete `glassmorphism()`; `editor_dark()` becomes a thin Steel alias
  until P8 removes it.
- Adopt mockup `theme.rs` as the engine's `EditorTheme` replacement
  (surfaces+accents per preset; selection/status/axis/type invariant);
  rewrite `style_from_theme()` as a total mapping — every `Style` field
  fed from a token, nothing invented.
- **Flat pass**: remove `with_shadow` from popover-family widgets; E3 =
  elevated @ 0.96 + `stroke_strong`; modal scrim 0.45 (E4 opaque).
- Status/axis/type colors stay engine-side (`EditorTheme`), exposed to
  panels via `EditorServices` — widgets in crusty-gui never see them,
  except checkbox `mixed` which takes an explicit color parameter.

### P2 — Widget state ladder + new primitives (crusty-gui)

- Apply the ladder uniformly: hover = fill one step lighter +
  `stroke_strong`; pressed = one step darker; focused = border →
  `focus_ring` (nothing else); disabled = `header` fill, 50% stroke,
  disabled text. Buttons (standard/primary/danger/ghost/toggled-tool),
  inputs (24px, `input` fill), checkbox/radio 14px (+ **mixed state**),
  **new toggle switch** 28×15, slider (4px track, 12→14px thumb),
  dropdown (22px rows, selection = `selection.fill`, hover = neutral —
  *selected ≠ hovered, never combined*), scrollbar (10px gutter, 6px
  thumb, no accent), progress (5px, + **indeterminate** 25%-segment),
  spinbox caps (22px, `header` fill) on DragValue.
- **Chips**: count chips (status tint 13% + border + dot when active;
  neutral outline + gray dot muted) and filter chips — new small widget,
  replaces console's ad-hoc versions.
- **Structured tooltip** (name / type · size / mono path), 400 ms delay,
  single instance.
- Selection tokens in `SelectableValue`/tree/table rows; multi-select
  last-clicked = 1px `focus_ring` inset; drag-insertion = 2px accent line.

### P3 — Keyboard focus & traversal (crusty-gui + panels)

- Register all interactive widgets as focusable; render the focus state
  from the ladder (border swap only, no glow).
- Tab/Shift-Tab traversal within a panel/window scope (extend
  `focus_next` with scoping so Tab in a settings window cycles that
  window only); Enter activates, Space toggles, Escape closes
  popover/modal (already partial).
- Arrow-key navigation in dropdown lists and settings sidebar.

### P4 — Editor chrome (engine panels; visual only)

- Menu bar 30px; **full Edit menu** (250px, 24px rows): History — Undo /
  Redo / Undo History…; Edit — Cut/Copy/Paste/Duplicate/Delete;
  Configuration — Editor Preferences…, Keyboard Shortcuts…, Project
  Settings…, Plugins (disabled until 39.8). Unavailable items disable,
  never hide — and in P4 that rule does the heavy lifting: **Cut / Copy /
  Paste and Undo History… ship disabled** (functionality is P9, which may
  slip past the visual milestone); Duplicate / Delete wire to existing
  ops; Undo/Redo keep their current plain labels until P9 adds metadata.
- Scene tabs (30px, 2px accent top edge on active, 5px warning dirty dot),
  toolbar 36px (26×24 tool buttons, toggled = accent_soft fill + accent
  border/icon), status bar 28px with the **command field** — the command
  palette moves here (status-bar `>` field, ≤420px results popover); the
  old floating palette entry point is removed per DESIGN.md.
- Dialogs (360px, 36px title, ghost Cancel / filled danger), toasts (3px
  status rail, sticky errors), progress variants.
- Kill every hard-coded color in `menu_bar_crusty.rs`,
  `dialogs_crusty.rs`, `command_palette_crusty.rs` (grep gate begins).

### P5 — Panels restyle (engine)

- **Hierarchy**: 22px rows, neutral hover vs `selection.fill` selected,
  hidden-row treatment, type icons via `IconPalette`, drag-insertion line.
- **Inspector**: 26px E2 section headers; transform = 22px XYZ fields with
  **3px axis-colored inset edge + tinted axis letter** (axis color never
  on the border), per-row R reset 20×22; Mesh Renderer per mockup.
- **Asset browser**: 96px cards, 70px thumbnail, 2px bottom type edge +
  mono type label, selection wraps (never tints) the thumbnail; 150px
  folder rail; toolbar 34px.
- **Console**: filter count chips (from P2), 11.5px mono logs, 8% status
  tint on error rows, 28px command field.
- **Profiler**: 32px toolbar, live/pause as accent-soft toggles,
  flamegraph blocks keep thread-family colors (join `type_colors`).
- Grep gate complete: **zero `Color::rgb*` literals in
  `*_crusty.rs` panels** (icon tints via `IconPalette`, status via theme).

### P6 — Asset reference field + picker (engine composite)

One layout for every asset-typed property (mesh, material, clip, audio):
44px thumb (`window` fill, 1px stroke, 2px type edge; live preview via
existing thumbnail renderer), name dropdown (missing ref = `status.error`
+ dot), 18px utility strip (use-selected / locate / reset), read-only slot
chip. Picker popover: auto-focused search, type-locked chip, selection
row, New/Edit/Copy/Paste/Clear group, mono-count footer. (HTML renders the
thumb at 52px vs 44px in the DESIGN.md copy — measure the HTML during
implementation; the rendered page wins per its own source-of-truth rule.)

### P7 — Settings framework + the two windows (engine)

- **Shell** (one implementation, two documents): modal-family window,
  36px title bar (title + scope chip + search), 158px flat sidebar
  (selection.fill current, modified-dot accumulation), 26px E2 section
  bars, 28px rows (170px secondary-text label column), footer (mono file
  path + save state). Row model: value widget, modified-from-default →
  `status.overridden` dot + mono "default …" hint + R reset,
  RESTART chip where needed, search filters rows across categories.
- **Editor Preferences** → `editor_prefs.ron` (user-local, gitignored,
  live-apply + autosave, no OK button; **writes debounced ~500 ms** — a
  slider drag must not write the file per frame). Wired v1: Appearance
  (theme preset ×4, popover translucency toggle — **no density row**:
  Comfortable is the only density this milestone and a one-option
  dropdown is dead UI; Compact + the row return together later), Viewport (camera fly
  speed/boost/sensitivity/invert-Y/FOV, grid size/visibility, gizmo
  size), Snapping (move/rotate/scale increments + defaults), Editing
  (undo limit, autosave toggle+interval if cheap — else stub), Asset
  Browser (thumbnail size), Console (max lines, default chips), **Play**
  (PlaySettings migrates here from `CrustyDockLayout` — net mode / host /
  module / players; `#[serde(default)]` keeps old layout files parsing),
  Performance (editor FPS cap / vsync if already surfaced — else stub).
- **Project Settings** → `project.ron` (checked in; dirties, saves with
  Ctrl+S; same file 39.8's plugin manifest will extend). Wired v1:
  Project (name/version), Maps & Modes (game default scene, editor
  startup scene, **server world scene read-only** — it's a compile-time
  module constant; display + "republish to change" hint), Networking
  (edits `net_config.ron` values in place — the packaged runtime keeps
  reading that file), Physics (gravity, fixed timestep), Streaming
  (budgets), Build (default target, output dir). Stubbed v1: Input
  (read-only action list), Rendering, Audio.
- Asset-typed settings rows reuse P6's reference field unchanged.
- **Slip valve**: P7 is the largest and least-coupled package — nothing in
  P0–P6/P8 depends on it except preset switching, which can temporarily
  live in the View menu. If the milestone box leaks, P7 is what slips (or
  splits: framework + Editor Preferences first, Project Settings after).

### P8 — Presets, docs, verification

- Appearance preset switching live (rebuild `Style` from `EditorTheme`,
  no restart), persisted in `editor_prefs.ron`; remove `editor_dark()`.
- `DESIGN.md` lands as `../crusty-gui/docs/DESIGN.md`; short pointer from
  engine docs. Delete superseded style notes.
- **Fidelity pass**: screenshot each restyled surface (screenshot.ps1)
  and compare against the mockup HTML rendered in a browser
  (`codex exec -i`); fable-5 reviews for taste; fix material deltas.
  ≥95% is a taste gate, not a measurement — **bounded at two review
  rounds**; after round two, remaining nits become backlog notes, not
  merge blockers (the grep gate is the objective check).

### P9 — Edit menu functionality (may slip past P8) — ✅ delivered

Deliberately split out of P4: this is systems work, not restyle, and the
visual milestone must not block on it.

Delivered: `serialize_subtree`/`spawn_subtree` in the scene serializer
(fresh-GUID paste with intra-subtree Parent remap via live `set_parent`
against an old-GUID→index map — no serialized rewriting needed; unit-tested),
`DeleteSubtreeCommand` + `PasteCommand` (GUID-tracked, respawn-safe),
Cut = clipboard write + one delete command labeled "Cut X" (single undo
action, no compound machinery), verb/object undo labels ("Undo Paste Duck"),
Delete now undoable in edit mode, shortcuts gated on `wants_keyboard`.

- **Entity clipboard**: RON-serialize the selected entity subtree via the
  scene serializer to an internal buffer; Paste instantiates as sibling
  with **fresh GUIDs, remapping intra-subtree references** (a child
  pointing at a copied sibling must point at the *new* GUID — this
  remap-table pass is the real work and needs a test); Cut = Copy +
  Delete with undo recorded as one compound action.
- **Undo labels**: extend undo-stack entries with verb/object metadata so
  the menu reads "Undo Move Entity"; if the stack's entry type resists a
  cheap extension, keep plain "Undo" — the label is polish, not contract.
- **Undo History… window** stays disabled even here — no package builds
  it this milestone (explicitly: it is *not* a week-3 quick-add). It gets
  a line in UX v2.

## Acceptance

- [x] Four presets switch live from Editor Preferences → Appearance and
      persist across restarts; Steel is the default
- [x] `grep -rn "Color::rgb" game_client/src/panels engine/src/engine/editor`
      (crusty panels) returns zero widget-color literals
      (`game_client/src/panels` doesn't exist; the 4 editor hits are
      computed color math — picker value, hover blend, status tint, gizmo
      premultiply — not widget literals)
- [x] Every interactive widget shows the five ladder states; Tab traverses
      the settings windows and inspector; focus ring visible
- [x] IBM Plex Sans renders UI text; JetBrains Mono renders values, paths,
      shortcuts, logs
- [x] Edit menu complete per mockup; Duplicate/Delete functional;
      Cut/Copy/Paste functional (P9 landed in-box)
- [x] Editor Preferences autosaves `editor_prefs.ron` live; Project
      Settings saves `project.ron`; both windows searchable with
      modified-dots and resets
- [x] PlaySettings live in Editor Preferences → Play; M9.6 play flows
      (Client / Listen Server / N players) still pass
- [x] Side-by-side screenshots vs mockup judged ≥95% on: chrome, hierarchy,
      inspector, asset browser, console, settings windows

## Out of scope

- Plugins window (menu entry ships disabled — Task 39.8)
- Input remapping UI (read-only list only)
- Accessibility audit, empty states, full undo coverage (UX v2)
- Undo History window (menu item ships disabled — UX v2)
- Compact density (Comfortable-only this milestone; no density row in
  Appearance until Compact exists)
- Server `world_scene` write-path (read-only display)
- Terrain/Modeling modes shown in the mode-selector mockup (visual spec
  only; selector ships with implemented modes)

## Open questions (answered by recommendation unless overridden)

1. **Command palette moves to the status bar?** → yes, per mockup; the
   menu loses its search field, palette keyboard shortcut stays.
2. **Drop popover shadows?** → yes. The system is flat; E3 alpha + strong
   stroke replaces them (one commit, easy to revert if it feels wrong).
3. **Entity clipboard scope** → internal buffer (not OS clipboard) v1;
   cross-instance paste deferred.
4. **PlaySettings migration out of the layout file** → yes; prefs is the
   correct home and old layouts still parse via `serde(default)`.
5. **`net_config.ron` vs `project.ron` for networking defaults** → keep
   `net_config.ron` as the runtime file; Project Settings is its editor.
   Merging into `project.ron` would force the packaged client to parse
   editor config — wrong direction.

## P8 fidelity backlog (round-2 close-out, 2026-07-24)

Reviewed vs mockup (codex round 1+2). Fixed: Reset All → new crusty
`ButtonVariant::DangerOutline` (filled danger stays in confirm dialogs).
Remaining nits — deliberate deviations or UX-v2 items, not blockers:

- Centered "Main Scene — Crusty" chrome identity: superseded by scene
  tabs (P4 decision; docking reality).
- Toolbar mode control is icon-only, not the mockup's wide
  icon+label dropdown (P4 decision; toolbar density).
- Settings shell keeps a fixed 780×560 window (stable across category
  switches) instead of content-hugging height.
- Settings header is two rows (crusty Window title bar + scope strip);
  mockup folds chip+search into the 36px title bar — needs custom
  title-bar content API in crusty Window. UX v2.
- Appearance lacks accent / UI scale / font-size rows — features don't
  exist engine-side yet; add rows when they do.
- ~~Undo/Redo verb+object labels → P9.~~ Landed in P9.
