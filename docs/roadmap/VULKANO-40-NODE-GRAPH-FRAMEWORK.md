# Task 40 — Node Graph Framework & Custom Node SDK

**Status:** 📋 Plan (drafted 2026-07-27)
**Duration:** ~2–2.5 weeks (Phase A ~1.5 wk, Phase B ~3–4 days)
**Prerequisites:** Networked Co-op Slice milestone (done). Absorbs the deferred
"Task 40 Readiness Subsection" of Refactor Checkpoint #5 (ROADMAP.md:1669).
**Consumers:** Tasks 41 (anim), 45 (scripting), 50 (materials), 51 (VFX),
53 (audio), 57 (AI). Task 39.8 (plugins) implements against D2's registry
contract — the registry is designed here, deliberately.

## Goal

A domain-agnostic node graph framework: graph data model with stable IDs,
`NodeRegistry`, RON graph assets with schema migration, a pan/zoom graph
editor in crusty-gui, subgraphs-as-assets, and a derive-macro layer for
declaring node types. No domain node libraries, no evaluators — those belong
to consumer tasks. "Getting it right matters more than getting it fast."

## Current state (verified 2026-07-27)

Three research passes over the engine and crusty-gui. Corrections to what the
roadmap assumes, first — these change the plan:

- **The Task 39.5 `AnimationGraph`/`MaterialGraph` placeholder windows do not
  exist.** The old secondary-OS-window system was deleted; `SecondaryWindowKind`
  (`editor/secondary_kind.rs`) survives only as a dirty/save-state key. Editor
  surfaces are `EditorTab` variants (`editor/dock_layout.rs:16`) dispatched by
  a match in `game_client/src/app.rs` (~3352 docked, ~3940 float via
  `CrustyFloatWindow`). Graph editor integration is greenfield on the
  `MeshEditor(String)` pattern, not "swap a placeholder body".
- **No shared `SearchPopup` primitive exists.** Command palette, asset-ref
  picker (`inspector_crusty.rs:893`), and material picker are three
  independent hand-rolled popovers over `crusty_gui::widgets::Popup`.
- **The serialization/versioning migration harness was never built** (deferred
  Checkpoint #5 item). No schema-migration code exists anywhere; it is
  designed from scratch here (D4).
- **crusty-gui has no pan/zoom primitive.** Its own roadmap lists "Phase 22 —
  node canvas" as unstarted: no transform/affine type, all paint commands are
  absolute screen-space, `Ui::interact` is primary-button-only, hit-testing is
  screen-space. Closest prior art is the profiler flamegraph
  (`editor/profiler_crusty.rs`): hand-rolled 1-D pan + zoom-about-pointer over
  raw `Painter` + typed `Memory` state.

What we build on (all confirmed present):

- **Asset pipeline**: single-segment extension scheme landed (`<name>.graph`
  from day one, per ROADMAP.md:1654). New type touch points:
  `assets/asset_type.rs` (variant + `from_extension`/`extensions()`/
  `display_name()`/`all()`), own module for the RON schema (per-type modules
  are the norm — no shared RonAsset trait), `assets/hot_reload.rs`
  (`ReloadEvent` variant + watcher arm; consumer arm in
  `game_client/src/app.rs::process_hot_reload`), optional
  `asset_browser_crusty.rs::type_color`/`type_icon_stem` arms. Cross-asset
  references are **content-relative path strings** (`String`/`Vec<String>`),
  never GUIDs — graph→subgraph references follow suit. (`AssetDependencies`
  in `assets/dependencies.rs` exists but is dead code; do not wire to it.)
- **Editor shell**: `CommandRegistry::register` (`editor/command_palette/`)
  + flat `EditorAction` enum with central `dispatch_action`. Undo via
  `editor/commands.rs` (`Command` trait `execute/undo/description` over
  `&mut hecs::World`, `CommandHistory`, `verb_object_label`). Preferences:
  `EditorPrefs` (`editor_prefs.ron`, `#[serde(default)]`, live-apply) +
  settings rows in `settings_crusty.rs`. Theme: zero hard-coded colors in
  `*_crusty.rs` panels (M10 grep gate — `ui.style().palette.*` only;
  invariant groups via `Palette::invariant_*()`).
- **crusty-gui painting**: `Painter::bezier_cubic` (adaptive flattening, the
  wire primitive), per-corner rounded rects with fill/stroke/shadow/glow,
  circles, convex polygons, `mesh()` escape hatch, clip stack, layer orders +
  overlay buffer, `context_menu_at(ui, id, Some(pos), body)` (opens at an
  explicit position — the node-create menu maps onto it directly), `Popup`,
  typed per-Id `Memory` (`data_get/insert::<T>` — not serialized; fine, view
  state persists in the asset/app), `Ui::run_at` (independent child Ui inside
  an explicit rect — the canvas host mechanism), `InputState::middle_down`
  (documented for pan drags), `scroll_delta`, modifiers, double-click.

## Design

### D1. Graph data model & serialization

Plain data structs in a new `engine/src/engine/node_graph/` module (engine
crate, not editor-gated — consumers evaluate graphs at runtime later).

- `GraphDoc { version: u32, nodes, edges, comments, groups, inputs, outputs }`
  — `inputs`/`outputs` are the subgraph interface declarations (empty for
  top-level graphs; presence is what makes a `.subgraph` usable as a node).
- Node instance: local integer id (unique within the doc), `type_id: String`
  (stable slug, e.g. `"set_world_position"`), `type_version: u32` (the
  descriptor version it was saved against — the migration hook), `position`
  (canvas world-space; node positions live in the asset, not GUI memory),
  and a property map for unconnected input constants.
- Property values: `PropValue` enum mirroring the pin types — `Float(f32)`,
  `Vec2/3/4`, `Color`, `Bool`, `Enum(String)` (variant slug), `Asset(String)`
  (content-relative path) — plus `Raw(String)` carrying any value whose type
  isn't recognized, so unknown data survives a load/save cycle instead of
  being dropped. `Entity` pins are connection-only (no serialized constant
  form). Properties are a `BTreeMap<String, PropValue>` (slug-keyed, ordered)
  so saves are byte-stable.
- Edge: `(from_node, from_pin_slug) → (to_node, to_pin_slug)`. Pin slugs, not
  indices — pins can be reordered/added without breaking saved edges.
- Pin types: base enum `Float, Vec2, Vec3, Vec4, Color, Bool, Enum, Texture,
  Mesh, Entity, Exec` plus `Domain(String)` for consumer-registered types
  (registered with the registry so validation and pin coloring work for e.g.
  Task 50's `Shader` type without touching this enum).
- Serialization: RON, `<name>.graph` / `<name>.subgraph`. One `AssetType::Graph`
  variant with `extensions() => &["graph", "subgraph"]` (Material's
  `["material", "matinst"]` precedent). Loader in the node_graph module
  (`load_graph(path) -> Result<GraphDoc>` / `save_graph`), plain
  `ron::from_str` like `load_material_ron`. No `AssetManager` cache in v1 —
  the editor holds open docs; consumer tasks add `Handle`-based caching when
  runtime evaluation needs it (mirroring `model_manager.rs` then).
- Subgraph references: content-relative path string on the subgraph node
  instance, per the engine-wide convention.

### D2. NodeRegistry & NodeDescriptor — the contract 39.8 consumes

- `NodeDescriptor`: `id` (stable slug — the serialized identity), `name`
  (display, free to change), `category`, `version: u32`, input/output
  `PinDescriptor`s (`slug`, `label`, `PinType`, optional default), `pure:
  bool`, `realm: Realm { Editor, Client, Shared, ServerSafe }`,
  `deterministic: bool`, migration hooks (D4).
- `NodeRegistry`: plain struct, `register(NodeDescriptor)` (upsert by id,
  error on id collision from a different source), `get(&str)`,
  `iter_by_category()`, plus domain pin-type registration. **Runtime
  registration is the primary API** — no static-only assumptions, no global
  singleton baked in at compile time. This is deliberate: Task 39.8's
  `Plugin::build(app)` later calls `registry.register(...)` like any other
  caller, and Phase B's inventory auto-registration is just one backend
  feeding the same method. A unit test registers a node type at runtime and
  asserts it round-trips through search + graph load.
- Ownership: registry lives in engine (constructed at app init, editor and
  runtime both see it), not in editor-gated code.

### D3. Graph validation

Two layers, both pure functions run on load, on edit (errors shown inline),
and in tests:

- `validate_doc(doc, registry) -> Vec<GraphError>` — doc-local, no I/O:
  - Edge type check: pin types must match (no implicit conversions in v1;
    conversion nodes are a consumer-domain concern).
  - Exec rules: exec pins connect only to exec pins; impure nodes require
    exec flow; pure nodes may not have exec pins.
  - Realm check: a graph declares its realm; a node whose realm doesn't admit
    the graph's realm is an error (`Shared` admits client+server, etc.).
    Enforced now, before any networking consumer exists — cheap here,
    authority-bug prevention later.
  - Unknown `type_id` → error carrying the slug (graph still loads and
    renders the node as a themed "missing node" so users can fix or delete
    it — a disabled plugin in 39.8 must not eat graphs).
  - Dangling edges (removed pins after migration), duplicate node ids.
- `validate_refs(doc, resolver) -> Vec<GraphError>` — cross-asset, through a
  `GraphResolver` trait (`resolve(content_rel_path) -> Option<&GraphDoc>`):
  subgraph interface match and reference-cycle detection (D8). Missing
  reference is its own error. The editor's resolver prefers open (possibly
  unsaved) docs over disk; tests use a plain map. Paths are canonical
  content-relative (forward slashes) — the one true key form used by editor
  tab keys, hot-reload matching, and references alike. The resolver also
  maintains the reverse index (which hosts reference a given subgraph) that
  hot-reload uses to refresh open host graphs (D6).

### D4. Versioning & migration harness

The Checkpoint #5 deferred item, built here because the first graph types
exist here.

- Per node **type** version in the descriptor; each node instance stores the
  version it was saved with. On load, for each instance with
  `saved < current`, run the type's registered migration chain before
  validation. A migration step receives a context, not just the property
  map: `migrate(step, ctx)` where `ctx` exposes the node's properties *and*
  `rename_pin(old_slug, new_slug)` (rewrites the node's incident edges) —
  pin renames are the most common migration and edges store pin slugs
  outside the node, so a props-only signature could not express them.
- `GraphDoc.version` covers container-level changes (rare). It is read via a
  raw envelope pass (peek the version field from the RON value) *before*
  strict deserialization into the current schema, so container migrations
  can rewrite old documents that no longer parse as today's `GraphDoc`.
- Harness: test module + golden-file fixtures (`tests/fixtures/graphs/`) —
  an "old" RON file checked in verbatim, loaded through the migration path,
  asserted equal to the expected migrated doc. First real fixture ships with
  a deliberate v1→v2 rename on a test node so the harness is proven
  non-empty. Consumer tasks add fixtures as their node libraries evolve.

### D5. Canvas primitive (crusty-gui — this is the Phase 22 work)

Per convention, new backend/shell API lands in `../crusty-gui` first; both
repos must build before commit. Scope is a minimal reusable primitive, not a
node widget — nodes stay engine-side:

- `CanvasTransform { pan: Vec2, zoom: f32 }` with `world_to_screen`/
  `screen_to_world`, zoom-about-pointer (generalizing the flamegraph's
  `zoom_about` math to 2-D), clamped zoom range.
- `Canvas` widget: allocates a rect and runs a child scope via the `run_at`
  mechanism handing the body `(&mut Ui, &CanvasTransform)`. `run_at` inherits
  the parent clip — the Canvas must push its own clip (widget rect ∩ parent
  clip) explicitly. Pan input: middle-drag via raw `middle_down`
  (`Ui::interact` is primary-only, and `middle_down` is held-state only — the
  canvas stores the previous pointer position and drag ownership itself),
  plus Ctrl+scroll zoom and scroll pan.
- State ownership: the caller owns the canonical `CanvasTransform` and passes
  `&mut` in; typed GUI `Memory` holds only transient drag state. GUI memory
  is per-context, so app-owned state is what survives tear-off to a float
  window (separate `CrustyGui` context) and restarts — for the graph editor
  the transform lives in `GraphEditorState`.
- Hit-testing helper: `canvas.interact(id, world_rect)` converting through the
  transform before testing — bypassing screen-space `Ui::contains_pointer`.
- **Text under zoom**: no scaled text exists; text "zooms" only by re-shaping
  at a new `size_px`, and each distinct size is a shape-cache entry. Policy:
  **quantize label font size to a small LOD ladder** (e.g. 4–5 discrete sizes
  across the zoom range; hide pin labels below a threshold) so smooth zoom
  doesn't churn the shape cache/glyph atlas. Geometry zooms continuously;
  only text snaps.
- Not in the primitive: minimap, box-select, culling (engine-side, D6).

### D6. Graph editor panel (engine)

New `editor/graph_editor.rs` (state) + `editor/graph_editor_crusty.rs`
(drawing), following the mesh-editor split. Integration points, exhaustively:

- `EditorTab::GraphEditor(String)` (key = content-relative path) in
  `dock_layout.rs` + `title_string`/`id_string`/`to_window_kind` arms;
  `tab_id`/`parse_tab` arms in `dock_crusty.rs` (`"graph:{key}"`);
  `SecondaryWindowKind::Graph` for dirty/save keying; dispatch arms at both
  `app.rs` call sites with per-key state in
  `HashMap<String, GraphEditorState>` (mirroring `mesh_editors`);
  `float_window_attrs` arm so tear-off works; save dispatch arms
  (`app.rs:1021/1322` pattern) + `EditorAction::SaveAndCloseEditor` handling.
  Asset browser double-click on `.graph`/`.subgraph` opens the tab.
  Two known generalizations (today's code special-cases `"mesh:"` keys
  only): `handle_crusty_tab_close` (`app.rs:3807`) and the float-window
  close path (`app.rs:~4143`) need graph arms routing through the
  save/discard veto, and `dock_crusty::tab_titles`'s dirty-indicator
  logic (scene tabs only today) must ask per-tab dirty state so graph tabs
  get the dot and the close veto.
- Hot-reload: watcher events carry normalized absolute paths — map to the
  canonical content-relative key before matching open editors or subgraph
  references, and suppress the echo event from our own just-completed save
  (per-key last-save timestamp guard). External change while a tab is
  dirty: keep the in-editor doc and surface a warning; clean docs reload
  silently (hosts re-derive subgraph pins via the resolver's reverse
  index, D3).
- Rendering: rounded-rect node bodies (header tinted by category via theme
  type colors), circle pins colored by pin type, `bezier_cubic` wires with
  horizontal tangents, selection ring via `palette.selection`. Pin/realm/
  error colors come from a new **invariant palette group** (graph pin colors)
  added to `theme/palette.rs::invariants()` — zero `Color::rgb*` literals in
  the panel (M10 grep gate extends to the new files).
- Interaction: drag nodes (undo-coalesced per drag), drag from pin to pin to
  connect (live type-validity feedback: wire tinted ok/error, invalid drop
  rejected), click/shift-click/box-select (marquee drawn with
  `rect_stroke`+translucent fill, manual rect-intersection), Delete,
  Ctrl+C/V/D (clipboard = doc fragment with remapped local ids, offset paste),
  double-click subgraph node → opens that asset's tab.
- Node-create menu: right-click → `context_menu_at` at pointer with search
  field + category rows, following the `asset_ref_picker` pattern (there is
  no shared SearchPopup to reuse; extracting one is **not** in scope — third
  duplication is noted as future cleanup). Created node lands at the
  right-click's world position.
- Large-graph hygiene: cull node draw/interact calls outside
  `screen_to_world(clip_rect)` by hand (flamegraph/Table precedent — no
  library culling primitive exists).
- Minimap: fixed-size overlay in a corner, second cheap paint pass over node
  rects at a fitted transform, click-to-jump. Small, but explicitly the first
  candidate to cut if the package overruns (ROADMAP lists it; nothing
  downstream depends on it).

### D7. Undo/redo

`editor/commands.rs`'s `Command` trait is bound to `&mut hecs::World`; graph
edits mutate a `GraphDoc`, not the world. Rather than widening the global
trait, the graph editor carries a **doc-local undo stack**:
`GraphEditStack { undo: Vec<GraphEdit>, redo: Vec<GraphEdit> }` where
`GraphEdit` is a reversible op (AddNode, RemoveNodes{subgraph-of-doc},
Connect, Disconnect, MoveNodes{ids, delta}, SetProperty{old,new}, Paste)
with M10-style verb/object descriptions ("Undo Connect", "Undo Move 3
Nodes"). This mirrors how the mesh editor owns per-document state and keeps
world-undo and doc-undo from interleaving incoherently.

- Dirty is a **saved-cursor**, not a sticky flag: the stack records the
  position at last save; dirty ⇔ current position ≠ saved position, so
  undoing back to the save point clears the dot, and a post-save edit or
  redo re-dirties. If an edit truncates the redo branch containing the save
  point, the doc stays dirty until the next save. (`DirtyState` discipline
  unchanged: state changes only from edit/dispatch call sites.)
- **Edit-action focus routing** (the real integration cost): today the Edit
  menu receives the scene `CommandHistory` plus scene selection/clipboard
  flags (`menu_bar_crusty.rs:50`), and all Undo/Cut/Copy/Paste/Delete
  actions operate on scene state (`app.rs:~1096`). `MenuBarCtx` gains an
  *active edit target* — resolved from the focused tab, reported by both the
  main dock and float windows — so Ctrl+Z/Y and clipboard actions route to
  the focused graph tab's stack (labels read from it) and fall back to
  scene state otherwise. This lands with P5, not at close-out.

### D8. Subgraphs

- A `.subgraph` declares `inputs`/`outputs` (slug, label, pin type) in its
  doc; a subgraph node instance in a host graph references it by path and
  derives its pins from those declarations at load/registry time.
- Interface change handling: if the referenced subgraph's interface no longer
  matches saved edges, affected edges become validation errors (D3), not
  silent drops.
- Cycle detection lives in `validate_refs` (D3, needs the resolver): the
  subgraph reference graph must be a DAG (direct or transitive
  self-reference is an error).
- Editing a subgraph is just opening it as a tab; host graphs re-derive pins
  on reload (hot-reload arm from D1 makes an open host refresh).

### D9. Phase B — macro layer

New workspace crate `node_graph_macros`:

- Derive macro parsing a struct with `#[node(...)]` + `#[input]`/`#[output]`
  field attributes → generates the `NodeDescriptor` (the roadmap's
  `DamageZone` sketch is the target syntax). Type mapping f32→Float,
  Vec3→Vec3, Entity→Entity, ExecPin→Exec, etc.
- `inventory`-based auto-registration as **one backend**: a collect-and-
  register-all call feeding `NodeRegistry::register` — manual registration
  stays first-class (39.8 plugins, hot-reload).
- First domain macro `#[derive(AnimationNode)]` sharing the parse/generate
  core — proves the domain-macro layering for Task 41 without shipping any
  animation nodes.
- Registry/descriptor API must not change to accommodate the macro; if it
  has to, that's a D2 design bug to fix in D2's terms.

### D10. Non-goals

Domain node libraries; evaluators/compilers (no graph *runs* in Task 40);
game-specific nodes; shared SearchPopup extraction; crusty-gui retained
tessellation/culling (its Phase 17); graph-diff/merge tooling; comments on
wires; any 39.8 plugin work beyond keeping D2 runtime-registerable.

## Work packages (each = one reviewable commit)

- **P0 — crusty-gui canvas primitive** (D5): transform + Canvas widget +
  hit-test helper + text LOD policy; exercised by a small gallery/demo screen
  in crusty-gui. Both repos build.
- **P1 — data model + assets** (D1): node_graph module, GraphDoc RON
  save/load **with the complete v1 schema — `type_version`, container
  version envelope, `PropValue`, and subgraph `inputs`/`outputs` fields all
  present from the first save, so P3/P6 add behavior, never schema** —
  `AssetType::Graph`, hot-reload arms, browser color/icon arms; round-trip
  test + dummy graph-references-subgraph fixture (closes two readiness
  bullets).
- **P2 — registry + doc-local validation** (D2, `validate_doc` of D3):
  NodeRegistry, NodeDescriptor, built-in *test-only* node set (a few
  math/exec dummies behind a `dev_nodes` feature), doc-local validation
  suite incl. realm + the runtime-registration test. (`validate_refs` +
  cycles need the resolver and land in P6.)
- **P3 — migration harness** (D4): version fields, migration chain, golden
  fixtures with a real v1→v2 test migration (closes the harness readiness
  bullet).
- **P4 — editor shell integration** (D6 integration half): tab/window/save/
  dirty wiring end-to-end with a placeholder body — including the graph arms
  in `handle_crusty_tab_close` + float-window close and the `tab_titles`
  per-tab dirty generalization; asset-browser open; tear-off works.
- **P5 — canvas editor core** (D6 + D7): nodes, wires, connect with
  validation feedback, selection/box-select, delete, move, node-create search
  menu, doc-local undo/redo with saved-cursor dirty, copy/paste/duplicate,
  and the Edit-menu/shortcut focus routing (`MenuBarCtx` active-edit-target
  change).
- **P6 — subgraphs** (D8): `GraphResolver` + `validate_refs` (interface
  match, cycle detection), subgraph node with derived pins, open-in-tab
  navigation, interface-drift validation, host refresh on subgraph reload.
- **P7 — comments/groups + minimap** (D6 tail): comment boxes, group frames,
  minimap. The cut-line package.
- **P8 — macro layer** (D9): `node_graph_macros`, inventory backend,
  `#[derive(AnimationNode)]`, macro-generated descriptor equality test
  against a hand-written one.
- **P9 — close-out**: CommandRegistry entries + `EditorAction` arms
  (open-graph, toggle-minimap, etc.), `EditorPrefs` fields (zoom limits,
  grid snap) + settings rows, ARCHITECTURE/KNOWLEDGE docs, roadmap status,
  command-only-mutation audit note for consumer tasks (readiness bullet —
  recorded as a constraint on Tasks 41/45, since no executor exists here).

## Acceptance

- `.graph`/`.subgraph` round-trip byte-stable through save/load; dummy
  subgraph-reference fixture passes.
- Golden-fixture migration test: a checked-in old-version graph loads,
  migrates, validates clean.
- Runtime registration test: a node type registered at runtime appears in
  the create menu and loads from a saved graph.
- Validation: type-mismatched edge, realm violation, unknown type_id, and
  subgraph cycle each produce the specific error; unknown-type node renders
  as "missing node" without data loss on re-save.
- Editor: open from asset browser, edit (all P5 ops), undo/redo each op,
  save, close, reopen — doc identical; works docked and torn-off.
- Zero `Color::rgb*` literals in new `*_crusty.rs` files (grep gate).
- `cargo test --workspace --all-features`, clippy `-D warnings`, fmt clean;
  engine + crusty-gui both build; editor runs without panic.

## Risks & mitigations

- **Canvas is genuinely novel for crusty-gui** (no transform infrastructure
  at all). Mitigation: P0 is first and isolated; the flamegraph's math is the
  proven template; scope is one transform + one widget, not a scene graph.
- **Text-under-zoom cache churn** (each font size is a shape-cache/atlas
  entry). Mitigation: LOD ladder policy in D5; acceptance includes soak-
  zooming a 100-node graph without atlas blowup.
- **Registry shape wrong for 39.8** — the whole reason Task 40 goes first.
  Mitigation: runtime-registration test in P2 simulates the plugin call
  pattern; no global static registration assumed anywhere.
- **Per-frame rebuild cost on large graphs** (no retained tessellation).
  Mitigation: manual culling; target = smooth at ~200 nodes; beyond that is
  crusty-gui Phase 17 territory, explicitly out of scope.
- **Scope creep via editor polish** (grouping, minimap, alignment tools…).
  Mitigation: P7 is the designated cut package; anything not listed is out.

## Open questions (decide at review)

1. `AssetType::Graph` with two extensions vs. separate `Graph`/`Subgraph`
   variants — plan assumes one variant (Material precedent); split only if
   the browser needs distinct filtering.
2. Doc-local undo (D7) vs. widening the global `Command` trait to a
   world-or-doc target — plan assumes doc-local; global unification is a
   possible later refactor when more document editors exist.
3. Test-node exposure: `#[cfg(test)]`-only vs. a `dev_nodes` feature visible
   in the editor for manual testing — plan leans `dev_nodes` feature, off by
   default, so the editor is manually testable before Task 41 ships real
   nodes.
4. Zoom range and text LOD steps (proposed 0.25×–2.5×, 4 label sizes) —
   tune in P0's demo.
5. Does P0 land in crusty-gui as "Phase 22 partial" with their roadmap
   updated accordingly? (Assumed yes.)
