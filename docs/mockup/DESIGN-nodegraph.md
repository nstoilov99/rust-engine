# Crusty Editor — DESIGN-nodegraph.md · v1.2

One graph editor for every graph type (script, material, animation) — the node library filters by
graph type, the chrome is learned once. Reference: **Crusty Node Graph.dc.html**.
Core rules, ramp + palette maps, tokens: **DESIGN.md**.

**Status legend — spec vs build.** This document has outrun the implementation; trust requires
saying which is which. ✅ = built (in `graph_editor_crusty.rs` and/or proven in the reference
prototype's live router) · 📐 = specified only, not built. As of v1.2.2:
✅ node anatomy/edges/tags, pins + strict typing, wire colors, selection, groups/comments
(rendering), validation model, realms, break gestures, inspector, router geometry (all three
modes, continuous degradation, node-aware backward lane, Manhattan stagger — prototype-proven
against the acceptance tests).
📐 bundling proper, crossings, flow bubbles, zoom LOD ladder, copy/paste, auto-layout, per-node
previews, error navigation, find-in-graph, pin hover docs, culling budget, diff-friendly
serialization, the Editor Preferences ▸ Graph surface, debug sidecar persistence.
A 📐 item moves to ✅ only when its acceptance criteria are exercised against the Rust
implementation — the next real information comes from there, not from another spec pass.

- **Canvas** is `input` (darkest — it *is* an editing surface). Grid: minor 16px @ 5% white, major
  128px @ 9%; minor grid drops below 40% zoom. Grid never uses accent.
- **Node** = `header` fill, 1px `stroke`, radius 6, 26px header on `elevated`, 22px pin rows, 6px
  bottom pad. Min width 128, max 320 (titles middle-truncate). Category identity is a **2px top
  edge** in the category's ramp slot (deep tone) plus a 9px mono tag in the same slot's bright
  tone — never a filled title bar (that's the one thing both Blender and Unreal do that does not
  survive this system's density).
- **Categories** = the `categories` map (deep tone for the edge and GroupBox tints, bright for
  the tag): Event=ember · Flow=gold · Math=lime · Logic=green · Data=teal · Audio=cyan ·
  Physics=azure · Spatial=blue · Render=violet · Gameplay=magenta · Interface=rose ·
  **Dev=neutral** · anything unreserved hashes `% 12` (`category_color()` — keep the hash, spend
  its output on the 2px edge). The hash never lands on neutral, so gray always means
  unknown/fixture. Tags are derived, never stored: PURE from `descriptor.pure`, SUB from
  `node.subgraph.is_some()`, EVENT from exec-out-and-no-exec-in.
  `Dev` is the `dev_nodes.rs` fixture category — test scaffolding, not a shipping taxonomy entry,
  and it must never be offered as a category for new work; on neutral, fixture nodes visually
  announce themselves as not-a-real-category. A fixture node MAY appear in a demo or doc graph,
  but only disclosed: the inspector prints its source (`dev_nodes.rs`) where a non-registry node
  prints `proposed descriptor`. Registry facts (type_id, category, realm) must be reproduced
  verbatim or not at all — a plausible-looking invented descriptor is worse than an admitted
  proposal. This binds docs too: a category card with no registered member gets a placeholder
  name marked SHAPE ONLY, and only `test_event` / `test_damage` / `test_add` /
  `test_editor_note` may ever be shown as real. Tags are derived per node and need not match a
  card's edge color — color answers "which category", the tag answers "what kind of node".
  Per-node color override picks from the 12 deep-tone swatches, replaces the edge, keeps the
  tag — needs `NodeInst.tint: Option<u8>` (a ramp index, not a hex), `serde(default)`.
- **Pins** mirror `PinType` exactly — the `pins` map, bright tone: Exec=white@92% · Float=green ·
  Vec2/3/4=blue · Color=violet · Bool=ember · Enum=rose · Texture=amber · Mesh=magenta ·
  Entity=azure · `Domain(k)`=its registry key, else `hash(k) % 12` · **unregistered=neutral**
  (never a guessed color). Mesh/magenta is shared with the geometry *asset* on purpose — same
  concept, same row in an asset reference field; they move together. There is no Int, String,
  Quat, Transform, Struct or Wildcard — the UI must not imply one.
  Shape: circle (scalar/handle), triangle (Exec), diamond (Vec2/3/4, arity in the label
  `Position ·3`), rounded square (Color). Filled = connected, hollow = open. **Shape is
  deliberate redundancy for color-vision deficiency** — twelve ramp hues exceed comfortable
  deuteranopia separation (green/lime, ember/rose merge for some users), and shape + the label
  carry the difference. The diamond is reserved, in writing, for a future cardinality/container
  concept should `PinType` ever grow one; do not spend it on anything else. No multi-input shape
  exists — `InputMultiplyConnected` is an error, so an input takes one edge and a second drop
  *replaces* it.
  Pins sit fully inside the node border (Unreal-style), 6px in; the wire terminates at the border
  at row-center. Never straddle the border: it forces overflow:visible nodes and clips against
  rounded corners. Unconnected inputs stay Blender-style — hollow socket + inline value widget.
  Draw r=4.5 / 9px across (the exec triangle 11px), hit-test r=9 / 18px across.
- **Typing is strict.** `validate_doc` compares `PinType` by equality; no implicit conversion exists.
  A refused drop names both types and offers a registry converter node if one exists — never a buzz.
- **Inline widgets** — one per `PropValue` variant, all reusing existing system controls: Float mono
  field · Vec2/3/4 axis-edge fields (the only row allowed to grow to two lines, L0 only) · Color
  swatch+hex · Bool checkbox · Enum dropdown · Asset = the asset reference field · `Raw` = a
  warning-dashed `preserved` chip, never an empty field (it is forward-compat data, not a blank).
  Entity pins are connection-only and show no widget.
- **Edges are keyed by pin slug**, so pin *index* must never surface in tooltips or error text, and
  reordering a descriptor's pins is a safe non-migrating change.
- **Wires** take their pin's color at 80%, 1.9px; exec wires are white at 92%, 2.4px (thickness is
  a theme metric — never a routing preference). Resolve through `wire_color()`. Hover →
  `focus_ring` + midpoint handle; a *selected* wire takes `selection.outline`. In-flight = dashed
  in the source color. Muted branch = grey fine dots. Broken = `status.error` with an × at
  midpoint. Executing = **flow bubbles** (v1.2 — replaces marching white dashes, decided; dashes
  stay spent on in-flight and muted only). Spline tangent
  `clamp(|dx|·(0.35+curve·0.55), 34, 190)`, grown when the target is left of the source; routing
  modes below.
- **Wire routing (v1.2, router corrected v1.2.1).** Three per-user modes in `WirePrefs`
  (`editor/graph_prefs.rs` — NOT `Theme`: no color, author taste, must not swap with presets;
  every field `serde(default)`; serialized as the `graph.wires` section of `editor_prefs.ron` —
  one settings file, one save path, never a second `graph_prefs.ron`):
  **Spline** (default, unchanged — do not retune the tangent), **Manhattan** (H/V, 90° corners
  rounded by `corner_radius`), **Subway** (horizontal runs + 45° diagonals, chamfered — the
  preferred orthogonal mode). The old 3-segment H/V/H angled mode is removed. Reference
  behaviour: Electronic Nodes (Unreal) — reimplement from this spec, never port the plugin's
  code (paid product; orthogonal routing itself is generic PCB/transit technique). Upsides
  beyond looks: polyline + pre-baked corner arcs tessellate cheaper than beziers, and
  hit-test/intersection become exact.
  - **The turn is anchored to a node, not to the span.** All wires sharing the anchored node
    turn at the same x regardless of span — under the Target default, every wire *arriving* at
    a node; under Source, every wire *leaving* one. This is what keeps bundles parallel and
    makes the graph read as a circuit board. The v1.2 `align` lerp (midpoint turn) is removed:
    it made the turn a function of span length, so diagonals floated marooned mid-canvas and
    parallel wires fanned apart.
  - **Geometry, `turn_anchor = Target` (default — the reference's "Right"; supersedes the
    correction brief's ship-Source note, decided against the reference screenshots).** One long
    horizontal run at the source row, then a single turn — 45° diagonal (Subway) or vertical
    (Manhattan) covering the full `dy` — ending exactly `horizontal_offset` (16, fixed,
    span-independent — replaces `stub` and `align` outright) before the target pin, measured
    from the node *border* (pins sit 6px inside). `Source` is the mirror (turn 16px after the
    source) — Target suits many-converge-on-one, Source one-fans-to-many.
  - **Near-horizontal shortcut** requires being near-horizontal: `|dy| < 2r` **and**
    `dx ≥ 6·|dy|` (~10° cap). Gating on dy alone let two side-by-side, slightly offset nodes —
    an extremely common arrangement — draw an arbitrary-angle line (dx=25, dy=19 → 37°); below
    the cap the normal path handles it on-grid (ho=3, [0°, 45°, 0°]).
  - **Space + degradation (v1.2.2 — measured, see router test report).** Subway's full-comfort
    requirement is `|dx| ≥ 16 + |dy| + 16` — one 22px row = **54px**; Manhattan needs 32px.
    When tighter, **Subway degrades continuously inside its own style**: offsets compress
    16 → 0 — at zero the route is a pure pin-to-pin 45°, still on-grid — and when `|dy| > |dx|`
    (no 45° exists) it routes **vertical-then-45°** rather than an arbitrary-angle line. The
    handoff must be continuous: the v1.2.1 compress-to-4-then-Line rule popped from a
    three-segment 45° to a 38° line mid-drag. **`min_dist` and `min_dist_style` are deleted (v1.2.3)** — dead config: continuous
    degradation plus vertical-then-45° covers every `dx ≥ 0`, the backward lane owns
    `dx ≤ −backward_lane_threshold`, and the leftward band between them routes
    vertical-then-45° (Subway) or vertical-then-horizontal (Manhattan). No input can reach a
    floor, and a documented preference nothing can trigger must not ship. The one residual —
    `|dx| < 24` **and** `|dy| < 20`, a stub between nearly-overlapping pins — draws straight
    (worst case 22° over ≤18px), which is the right answer at that size. The only mode swap is
    the L2 zoom collapse. `|dy| < 2×corner_radius` → straight horizontal. No transition may
    flicker while a node drags through a threshold. `corner_radius` 10 (reference default; was
    8), clamped per corner to half the shorter adjacent segment.
  - **Turn priority** `None` (default) | `Node` | `Pin`. None turns at exactly the offset —
    cheapest, and the only one that preserves span-independence; Node nudges to a lane clearing
    both bounding boxes (broadphase query); Pin aligns to pin positions. Node/Pin reintroduce
    per-wire variation and are opt-in.
  - **Exec override** (`exec_overwrite: Option<ExecWirePrefs>`, default off): when enabled, exec
    wires route with their own style/anchor/priority (suggested Manhattan · Target · Node) —
    control flow separated from data flow by *shape*, matching exec's existing color/width/pin
    distinctions.
  - **Smaller reference behaviours:** `disable_pin_offset` (hard turn at the border) · the turn
    offset is computed in **graph space and then scaled**, never applied in screen space — a
    screen-space 16px changes the route's shape as the user zooms and reads as a bug.
  - **Backward route**: 5 segments via a clear lane computed from **both nodes' bounding
    rects** — `min(tops) − 24` above / `max(bottoms) + 24` below, never from the pin (a pin-
    relative lane lands inside a tall node's own body and the wire runs under its own node);
    side = fewer intersected node rects in the corridor, tie → nearer the source row. This is
    the one place the router genuinely needs the broadphase. The lane engages below
    `backward_lane_threshold` (**−24**, a named `WirePrefs` field, not a magic number) — the
    band `−24 < dx < 0` instead routes vertical-then-45°/vertical-then-horizontal, on-grid.
  - **Bundling — mandatory for Manhattan, not optional polish.** Target-anchored Manhattan
    verticals coincide *exactly* at `x₂ − horizontal_offset` (six wires into one node render as
    a single thick bus; sources at different x change nothing, and Subway escapes only because
    its diagonal staggers by dy — a property of the diagonal, not the anchoring). Bundling
    ships with Manhattan or Manhattan does not ship. Shared-lane **mid-segments** offset perpendicular by `bundle_offset` 4px
    (`bundle_merge_offset` 20 where ribbons join), ordered by target Y so a bundle never
    self-crosses. Fan-out rule unchanged — both ends stay exactly on their pins. `Force
    Outside` option; above `bundle_max` 8, draw coincident. Bundling only yields parallel
    ribbons once turns are span-independent — re-check it after the router correction lands.
  - **Subway rendering details.** Bundle offsets are perpendicular to the segment (4px on a
    diagonal = 2.83px in x and y). Crossing symbols rotate to the segment angle. Below L2,
    Subway collapses to Manhattan/straight — 1px diagonals alias.
  - **Crossings.** Lower-priority wire interrupted: None (default) · Gap ~6px · Arc r=4 · Circle.
    Exec passes over data; ties → lower edge id under (stable across frames/reloads).
    Uniform-grid broadphase, hard cap ~2,000 visible segments, disabled below L2.
  - **Flow bubbles** (the one motion signal — dashes retired from execution): 4px, exec bubbles
    larger than data, 150 px/s, 40px spacing, adjustable; exec wires only by default; **only
    during an active debug session** (never on an idle graph); only above 50% zoom (L0–L1);
    optional selected-nodes filter.
  - **Acceptance tests** (each maps to a visible failure of the v1.2 router): 1 parallelism —
    two wires arriving at adjacent pins of one node, from sources at different distances, turn
    at the same x · 2 long span = one long run + a short angle at its destination, no
    mid-canvas diagonal · 3 one row apart at 60px = clean
    45° with both offsets compressed · 4 `|dy| > |dx|`, including the leftward band `−24 < dx < 0` =
    on-grid vertical-then-45°, never an arbitrary-angle kink (exhaustive sweep: every segment
    on a 45° multiple across dx ∈ [−40, 400] × dy ∈ [−200, 200]) · 5 degradation is continuous through every threshold —
    no shape pop at any dx during a drag · 6 near-horizontal = straight, no micro-kink · 7 backward =
    lane route, never through the source · 8 zoom invariance — same shape at 40% and 200% · 9
    six wires to a column of six = evenly spaced parallel ribbon (re-test once real bundling
    exists — in the un-bundled router the spacing falls out of the diagonal geometry) · **10**
    six Manhattan wires into six adjacent pins of one node = six *distinguishable* verticals
    spaced by `bundle_offset`, never one coincident bar. Tests 1, 4, 8 and 10 are the ones
    most likely to be missed.
  - **Re-verify against polylines, same release as the router:** midpoint handle and broken-× at
    the **arc-length midpoint** (not endpoint midpoint — off-wire on an L); hover = per-segment
    distance @ 9px each side (companion stroke 18 — a wire must never be harder to grab than a
    pin, whose hit radius is also 9), with a test for a wire below `min_dist`; stroke width is
    zoom-invariant (`non-scaling-stroke`) while geometry scales — this is what keeps test 8
    passing; `⌘`-drag slash-cut = exact
    segment–segment intersection, and the red-dash preview tests every segment, not bounding
    boxes; drop-on-wire splice snaps to the nearest polyline point and inherits the lane; a
    chain of three reroutes stays visually straight (each hop is its own route under the same
    rules).
  - **Settings surface:** Editor Preferences ▸ Graph (see DESIGN-panels.md — sections, disable
    rules, live preview strip, toolbar quick switch).
- **Selection is the documented exception** to `selection.fill`: a node keeps its fill and swaps
  its border to **`selection.outline`** (its own preset-invariant token — never `focus_ring`,
  which is a different JOB) — last-clicked 100%, rest of the set 55%, 2px outline offset. Accent
  on canvas is spent only on the compile chip, the marquee, and drag-time alignment guides.
- **Zoom LOD** — L0 90–220% everything · L1 60–90% inline widgets → plain values · L2 35–60% pin
  labels drop · L3 15–35% header block + type edge only · L4 <15% 4px type bars + 1px wires.
  Below 35% no glyphs are submitted at all — **except annotation titles** (group titles and
  comment NOTE bars, v1.2), which render down to L4 at a floor pixel size: on a graph too big to
  read, annotations are the only wayfinding left.
- **Badges never stack.** One glyph in the header's left gutter by precedence: breakpoint-hit →
  error → warning → breakpoint → hidden-pins. Status color reaches the *border* on a node (no row
  to tint); `status.danger` stays for the destructive action (cut wires, Delete).
- **Two annotations, not one — both tintable (v1.2).** Structure distinguishes them, so color no
  longer has to. `GroupBox` = titled *container*: tint paints the **6% body wash + 45% border**;
  20px title bar; auto-fits its nodes with a 12px margin; collapsible; below nodes.
  `CommentBox` = free-floating *note*: tint paints the **NOTE-bar fill + a 1px left edge only** —
  the body stays opaque `elevated`, never washed; 1px `stroke_strong`; 18px bar; above groups,
  below nodes; never encloses. A tinted group is a translucent region *containing* things; a
  tinted comment is an opaque card with a colored label *next to* things — distinguishable at any
  zoom, and both can now carry color.
  Tint = `Option<u8>` ramp index (12 deep-tone swatches + none), matching `NodeInst.tint`; never a
  free picker — a hand-picked color that reads on Steel vanishes on Graphite, breaks the no-hex
  lint and re-skinning; a genuine future need lands as an explicit `Custom(Color)` variant with a
  documented exception, not a loosened default. Tinted-bar label = the slot's **bright** tone —
  the `category_tag_color()` pairing, already measured AA (worst 4.89:1).
  Fields, all `serde(default)`, no migration (older files render pixel-identical):
  `CommentBox { rect, text, tint: Option<u8>, font_scale: f32 (1.0, range 0.75–3.0 — scales bar +
  body, bar height grows), anchor: Option<NodeId>, collapsed: bool }` ·
  `GroupBox { rect, title, tint: Option<u8>, collapsed: bool }`.
  Behaviour: an **anchored** note moves with the node it explains and is deleted with it
  (free-floating stays default — anchoring is what stops notes drifting stale). Corner/edge
  resize; auto-grow height to wrapped text, never auto-shrink below content; width author-set
  only. Collapse folds a comment to its NOTE bar — same `▾`, same gesture as groups.
  `#node-slug` in body text renders as a chip; click selects and frames that node (the breadcrumb
  path) — body otherwise **plain text, stored verbatim** (the `Raw` philosophy: never reformat an
  author's text on load; no markdown, no rich text, no images).
- **Validation errors** are a closed set of ten (`GraphError`), so the error UI is complete, not
  best-effort: anchor each one to the thing that is wrong (node border + gutter badge, pin ring,
  or the wire — `TypeMismatch` is the *only* error that colors a wire) and demote the corner overlay
  to a count. `DanglingEdgeNode` and `SubgraphCycle` are document-level: compiler row only, the cycle
  rendered as a clickable mono breadcrumb. `UnknownPin`/`SubgraphPinUnknown` add a dashed *ghost row*
  so the wire has somewhere to land. An unregistered `Domain` pin draws neutral — never a guessed
  color. Doc errors and reference errors stay visually separate; reference errors draw on the
  subgraph node that pulled them in. Validation runs on every edit; "valid" and "compiled" are two
  chips, never one traffic light.
- **Realm / purity / determinism** are metadata, not identity: graph realm = mono chip in the graph
  toolbar; node realm = right side of the 10px subtitle row and a palette column, with `Shared`
  printing nothing; `RealmViolation` = error border + a chip naming both sides (`ServerSafe in
  Client`); non-admitted palette entries stay listed at 45% rather than hidden. `deterministic:
  false` is marked only where it matters (Server/replay), as a `~` glyph. Changing a graph's realm
  preflights the count of nodes it will invalidate.
- **Breaking connections — three paths.** Click a wire and `Del` (discoverable, no modifier; ⇧-click
  extends). `⌥`-click a pin, or a node header, to break its links (Unreal's gesture, extended to the
  whole node). `⌘`-drag a slash to cut in bulk — crossed wires go red-dashed *during* the drag so the
  cut is previewed and Esc-abortable. Each reports what it did ("Broke 3 links") and is one undo
  transaction; each is also in the context menu.
- **Node inspector** — four blocks in touch-frequency order: editable **Title** (double-click / `F2`,
  ⏎ or blur commits, Esc reverts; on a subgraph node this renames the *call site*, and the mono
  `node <id>` line below never changes) · **Actions** (Rename, Breakpoint, Break All *n*, Delete in
  `status.danger` text) · **Descriptor** (read-only type_id, category, realm, pure, deterministic,
  version, position; non-Shared realm and unregistered type_ids in `status.warning`; nothing
  editable ever appears in this block) · **Defaults**
  (one row per input with a descriptor default — a connected input still lists, greyed and disabled,
  so the overridden value stays visible) · **Pins** (every pin with dot, type, and an ✕ that breaks
  its links).
- **No minimap.** At the zoom LOD ladder's L3/L4 the graph itself already reads as a map, so a
  minimap duplicates it in a worse form and costs a corner of canvas.
- **Organization** ships day one: groups (`C`, `⇧C` for a note), reroutes (one in, many out,
  insertable on a wire), named reroutes (declaration/usage pair, listed first in the palette),
  collapse to subgraph (`⌘G`), align & distribute for 3+ nodes (`⌥A`), view bookmarks (`⌘B`),
  purge-unused (`⌥⌘K`, confirms with a count).
- **Add-node palette** is the asset-picker shell at E3: `Tab` or double-click for unfiltered; drag
  off a pin and release on empty canvas for a type-filtered list (incompatible results stay at 45%
  with their type tag, never hidden); release on a node body auto-connects with no palette. New node
  lands with its wired pin at the drop point, nudging 8px until clear.
- **Debugging** follows Unreal: octagon breakpoint in the header gutter (hollow grey = disabled,
  warning + `!` = invalid), paused node in `status.warning` with a mono PAUSED chip and the taken
  path tinted, watch chips (`input` @94%, mono, 10px right of the pin, dashed when not yet executed),
  newest-first execution trace, flow bubbles on active wires (see Wire routing). Debug-object picker lives in the graph
  toolbar, not the debug panel.
- **Copy / paste / duplicate (v1.2.2 — the largest functional gap).** Clipboard = a RON subset
  (nodes + edges + annotations), never an internal pointer set, so paste survives across graph
  tabs and editor sessions. Internal edges remap **by pin slug** (the existing edge-identity
  rule); boundary edges drop silently but report a count — "Pasted 6 nodes, 2 links dropped",
  the "Broke 3 links" convention. Paste lands at the cursor preserving relative layout, nudging
  8px until clear (the palette's drop rule). Cross-graph paste validates realm + registry;
  unresolvable nodes paste as `preserved` placeholders rather than vanishing (the `Raw`
  philosophy). `⌘D` duplicate = copy+paste at +16,+16, one transaction.
- **Auto-layout** (`⌥L`): layered Sugiyama-style — columns by depth, in-column order minimizes
  crossings — over the whole graph or the selection, one undo transaction; group boxes are fixed
  containers; pinned nodes never move. Never incremental or continuous — that fights the author.
  It compounds with the router: orthogonal routing pays off on ranked left-to-right structure.
- **Per-node previews**: registry opt-in `descriptor.preview: Option<PreviewKind>` — render
  target for material nodes, curve strip for animation, none for script (the common case pays
  nothing). 64×64 at L0 only, dropped from L1 down; per-frame render budget with round-robin
  refresh, never all at once.
- **Error navigation**: `F8` / `⇧F8` cycles validation errors newest-first, framing and
  selecting each anchor — the bookmark framing path reused.
- **Find in graph**: `⌘F` on canvas — filter field, non-matches dim to 45%, `↵` cycles matches
  and frames each; the settings-window search idiom (DESIGN-panels.md), not a new pattern.
- **Pin hover docs**: tooltip on the pin itself — type name + one descriptor doc line, after the
  standard 400ms delay — removes inspector round-trips exactly when wiring.
- **Culling + budget**: the LOD ladder governs detail per node, not node count — add viewport
  culling on node rects and wire segments, with one stated budget ("60fps at 2,000 nodes, 5,000
  edges") from which the crossing broadphase's ~2,000-segment cap is *derived*, not chosen
  separately.
- **Diff-friendly serialization**: nodes serialize in stable id order (never insertion/hash
  order) and positions round to whole pixels — cheap now, painful to retrofit once graphs live
  in version control.
- **Persistence**: node positions already live in the asset by design (`NodeInst.position`), along
  with groups, comments, subgraph refs and tints. Debug state wants the opposite: breakpoints,
  watches, bookmarks and the last view transform in a user-local sidecar keyed by asset path, so two
  people editing one graph don't fight over it.
- **Code delta** (`editor/graph_editor_crusty.rs`) in payoff order: header tint → 2px edge · wire
  color from `pin_color(from_ty)` (ramp bright tone) · two-level grid · anchored errors instead of
  `error_overlay()`'s first-three-lines · `ROW_H` 18→22 and auto width 128–320 (was fixed 168) ·
  pin shapes · in-row `PropValue` widgets (keep `edit_popup()` for L1 and below) · search-first,
  pin-type-filtered `create_menu()` · group/comment divergence · zoom LOD thresholds ·
  `category_color()` retargeted to `hash % 12` over the ramp (deep tone) · v1.2: the orthogonal
  router (stubs / corner arcs / backward lane / min-dist fallback) + `WirePrefs` in
  `editor/graph_prefs.rs` · bubbles replace exec dashes · annotation `tint` / `font_scale` /
  `anchor` / `collapsed` fields · registry: `domain_registration(slug) -> Option<Option<u8>>` so
  `theme::domain_ramp_index()` can resolve keyed / unkeyed / unregistered domains.
