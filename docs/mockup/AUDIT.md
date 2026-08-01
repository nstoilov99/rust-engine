# AUDIT.md — Crusty node graph: spec vs implementation

## CLOSE-OUT (2026-08-01) — implemented, Phases 1–7

All seven phases shipped, one-or-more reviewable commits each:
Phase 1 tokens/lint `6a2eee2` · Phase 2 node/pins/LOD `d9311ff` · Phase 3
router `a30ef3a` · Phase 4 settings `b53d5bd` · Phase 5 annotations/errors/
organization `74e5832`+`8fbe6e3`+`a5f8c87` · Phase 6 clipboard/breaking
`987a718` · Phase 7 palette/nav/layout/etc `09ea082`+`432b695`. 456 engine
tests green (the one pre-existing render_thread failure excepted), design
lint in CI, spec docs amended per the rulings (error-set wording, min_dist
removal, status legend refreshed in DESIGN-nodegraph.md).

**Deferred ledger** (📐 remaining, with blockers): crossings rendering (prefs
+ derived `CROSSING_SEGMENT_CAP` land; broadphase unwritten — pure remaining
work) · flow bubbles + debug visuals (blocked on Task 45-A's evaluator; inert
breakpoint marks could land earlier) · bundling proper (ruling #5 — v0
stagger shipped) · `preserved` distinct visual (unknown types render
missing-red with intact data) · Vec2/3/4 two-line axis fields · L1 value edit
popup (needs text→PropValue parse) · on-canvas color picker · collapse-to-
subgraph inner wiring (45-A `graph_input`/`graph_output`) · pinned nodes
(no pinning concept) · full asset-reference field on canvas (panels scope).
Post-ship note: edges are not sort-canonicalized (nodes are) — next
diff-noise source if it bites. Visual review remains user-side (agent
sessions cannot capture the Vulkan swapchain).

The requirements tables below are the *pre-implementation* audit snapshot,
kept for traceability; the status legend in DESIGN-nodegraph.md is now the
live record.

Phase 0 gap audit, 2026-08-01. Sources: `DESIGN.md` v1.2 · `DESIGN-nodegraph.md`
v1.2 (status legend v1.2.2) · `DESIGN-panels.md` v1.1 · `theme.rs` v1.1(+v1.2
notes) · `theme.json` · `Crusty Node Graph.dc.html` (prose lines ~22–978, live
router JS ~979–1616) · `Crusty Design System.dc.html`. Implementation:
`engine/src/engine/editor/graph_editor_crusty.rs` (1291 L),
`editor/graph_editor.rs` (1188 L), `engine/src/engine/node_graph/*`,
`editor/theme/*`, `crusty-gui/src/widgets/canvas.rs`, host wiring in
`game_client/src/app.rs`.

Statuses: `implemented` · `partial` · `missing` · `contradicts` (code exists and
disagrees) · `unclear` (two reasonable readings differ). Line refs are current.

**Headline counts:** ~140 discrete requirements audited → 9 implemented ·
17 partial · 78 missing · 31 contradicts · 8 unclear. The spec's own ✅ legend
overstates Rust status: several ✅ items (wire colors, selection.outline, break
gestures, inspector, pin shapes) are prototype-proven but **absent from the
Rust build** — statuses below are Rust-only; the router is ✅ *as a reference
implementation to port*, not as shipped code.

---

## 0-R. Rulings (user, 2026-08-01) — all eight items decided

1. **Design system implements now, as specced.** `PinType` stays exactly as-is
   (no Int/String/Array). Task 45-A runs *after* this work and reconciles its
   type additions against the spec then (new ramp slots/shapes to be specced
   as part of 45-A, which also absorbs missed/improvable UI/UX).
2. **Windows keys**: ⌘→Ctrl, ⌥→Alt everywhere (⌘-drag cut = Ctrl-drag,
   ⌥-click = Alt-click, ⌥⌘K = Ctrl+Alt+K, etc.).
3. **GraphError stays 11**; spec wording amends to "a closed set (currently 11)".
   UnknownPin/SubgraphPinUnknown both get the ghost-row treatment.
4. **Zoom**: continuous 0.15–2.2 (no 12-step quantization), prefs override
   kept, `fit()`'s odd [0.3,1.6] clamp not ported (frame_view's ≤1.0 cap
   stays). Additionally: **F (frame/focus) applies to all canvas widgets** —
   F frames the selected nodes *or* the selected comment/group, not nodes only.
5. **Bundling**: port the prototype's pin-index stagger as shipped bundling-v0
   (cap `max(4, dx/2)` documented, test 10 scoped to spans wide enough);
   perpendicular-offset "bundling proper" (+merge 20, max 8, Force Outside)
   stays a later 📐 item.
6. **One `corner_radius` pref** feeds both the 2R straightness threshold and
   the rounding radius.
7. **Wire hover hit-test is screen-space**, 9px each side at every zoom.
8. **`hOf` +4px prototype bug is not ported** — Rust uses exact node heights.

## 0. Cross-cutting decisions needed before Phase 1 (blocking)

1. **`PinType` freeze vs Task 45-A.** Spec: "There is no Int, String, Quat,
   Transform, Struct or Wildcard — the UI must not imply one" and the diamond
   shape is reserved in writing for a future container concept.
   `VULKANO-45A-GRAPH-EXECUTION-CORE.md` P1 (drafted earlier) adds `Int`,
   `String`, `Array`. These cannot both hold. Options: (a) 45-A P1 amends this
   spec first (new ramp slots + shapes for Int/String, Array takes the reserved
   diamond); (b) 45-A retargets to Float-only v1. Needs your call; everything
   else in this audit is independent of it.
2. **`⌘`/`⌥` on Windows.** Spec uses Mac glyphs throughout (⌘-drag cut, ⌥-click
   break, ⌘G/⌘B/⌘D/⌘F/⌥A/⌥L/⌥⌘K). Prototype accepts `ctrlKey || metaKey` for
   the cut. Assume ⌘→Ctrl, ⌥→Alt everywhere unless told otherwise; note ⌥⌘K →
   Ctrl+Alt+K collides with nothing current.
3. **`GraphError` "closed set of ten" vs eleven in code** (`validate.rs:14–37`:
   DuplicateNodeId · UnknownNodeType · DanglingEdgeNode · UnknownPin ·
   TypeMismatch · InputMultiplyConnected · RealmViolation · UnknownDomainPin ·
   MissingSubgraph · SubgraphPinUnknown · SubgraphCycle). The spec's error-UI
   table doesn't enumerate its ten. Either the spec under-counts or one variant
   is meant to merge (UnknownPin/SubgraphPinUnknown are the natural pair — both
   get the ghost-row treatment). Proposal: keep 11, amend spec wording to "a
   closed set (currently 11)".
4. **Zoom range/stepping.** Prototype wheel: continuous ×`exp`-style over
   `[0.15, 2.2]` (JS 1599) and `fit()` clamps to a *different* `[0.3, 1.6]`
   (JS 1252). Prose says "15%–220%, 12 steps" (~line 811). Current Rust:
   continuous 0.25–2.5 from prefs. LOD bands (L4 <15%…L0 ≤220%) fit the
   0.15–2.2 range. Proposal: continuous 0.15–2.2 (prose's 12 steps read as
   stale), keep prefs override, drop the odd fit() clamp in favor of
   `frame_view`'s existing ≤1.0 cap.
5. **Manhattan stagger identity + acceptance-test tension.** Prototype staggers
   the vertical by **target pin row index** (`ho = min(16 + bIdx·4, max(4, dx/2))`,
   JS 1081–1084); prose says the offset is "fixed and span-independent" (stale
   per its own v1.2.2 correction) and the Bundling section describes a
   *different* mechanism (perpendicular mid-segment offsets ordered by target
   Y, merge 20, max 8). Also: the `max(4, dx/2)` cap silently collapses the
   stagger for `dx ≤ 32 + 8·bi` — acceptance test 10's "never one coincident
   bar" fails against the prototype's own code at short spans. Proposal: port
   the prototype stagger as shipped bundling-v0 (it's what test 10 can verify),
   spec the cap behavior explicitly, and treat perpendicular-offset bundling +
   merge + max-8 as the later "bundling proper" (already 📐).
6. **Corner radius: one pref or two.** `polyPts` uses `R=10` only as the
   `2R` straightness threshold; `roundedPath` has an independent local `r=10`
   for actual rounding. Spec has a single `corner_radius: 10`. Proposal: one
   pref feeding both uses (they were tuned equal).
7. **Wire hit-test space.** Prototype: 18px **screen** stroke
   (`non-scaling-stroke`, JS 1318) = 9 screen px/side at every zoom; spec prose
   says 9px each side without naming the space, and separately mandates
   zoom-invariant stroke *width*. Proposal: screen-space 9px/side (matches
   prototype + the "never harder to grab than a pin" rule at far zoom).
8. **Prototype `hOf` bug — do not port.** `hOf = 28 + rows·22 + 6` over-reports
   the rendered box by ~4–6px (the live template has no trailing 6px spacer);
   it feeds the backward-lane broadphase. The Rust port computes exact node
   heights already; note the deviation in the router tests.

**Known spec defects (from the brief) — confirmed + resolution recorded:**
- `min_dist` / `min_dist_style`: confirmed unreachable; nodegraph v1.2.3
  already deletes them, but **DESIGN-panels.md ▸ Graph still lists "Minimum
  distance (100px)" and "Below minimum, draw" rows, and the preview strip's
  third sample says "a span just under min_dist"** — panels doc needs the
  v1.2.3 edit. Resolution: delete both fields; third preview sample becomes the
  residual-stub case (`|dx|<24 ∧ |dy|<20`).
- `−24` magic number: confirmed prototype-only (JS comment 1086 names it);
  ships as `WirePrefs.backward_lane_threshold`.
- Lint rules: confirmed broken as published (hex false-positives, `rg` exit
  inversion, scope). Fix per Phase 1 brief; the spec's own lint block already
  contains the corrected inversion pattern — the *hex regex* still needs the
  word-boundary/3-6-8-only fix and comment-line skip; scope list
  `gui/ editor/ graph/ plugins/` doesn't match this repo's layout
  (`engine/src/engine/...`, `crates/`, `game_client/`, `../crusty-gui/src`) —
  scan list must be rewritten for the real tree.

---

## 1. Requirements table

### Tokens & palette (DESIGN.md · theme.rs)

| Requirement | Spec source | Status | Notes |
|---|---|---|---|
| `theme.rs` landed verbatim as single color/metric source | DESIGN.md Core 2; theme.rs | missing | Engine has its own `theme/palette.rs` (M10): 10-field `TypeColors`, no ramp, no resolvers |
| 12-hue ramp, two tones, indices-not-colors | DESIGN.md Palette arch | missing | No ramp type anywhere in `theme/` (inventory §3.4) |
| Three maps (assets/pins/categories) store ramp indices | same | missing | `type_colors` = flat 10 named colors; graph code indexes an ad-hoc local array |
| `neutral` outside ramp = unknown only; hash never lands on it | same | missing | No neutral concept; unknowns get guessed colors (see Pins/Categories rows) |
| Resolvers `asset_color/pin_color/wire_color/category_color/category_tag_color/domain_ramp_index` | DESIGN.md; theme.rs | missing | Local `pin_color`/`category_color` exist in graph_editor_crusty.rs:176–212 with wrong mappings; no others |
| `domain_registration(slug) -> Option<Option<u8>>` registry hook | theme.rs:176; code delta | missing | Registry has only `domain_pin_registered() -> bool` (registry.rs:188) — no keyed index |
| `PinType::Enum(_)` payload assumed by theme.rs | theme.rs:199 | contradicts | Current `Enum` has no payload (doc.rs:55–68); theme.rs comment says "adjust patterns" — decide: adjust theme.rs, don't grow PinType here |
| Presets swap surfaces+accents only; selection/status/axis/ramp pinned | DESIGN.md Core 3 | partial | Engine invariants match in spirit; values differ from theme.rs (e.g. selection.fill rgb(60,70,84)=#3C4654 ✓ matches) |
| Graphite `selection.outline` #B8BCC0 carve-out | DESIGN.md Presets; theme.rs:433 | missing | Engine `Selection` has no `outline` field at all |
| `selection.outline` #A9C0D8 token exists | theme.rs:271 | missing | Graph selection uses `selection_fill` as a border color instead |
| Rusty excluded from user-facing picker (`user_selectable()`) | DESIGN.md Presets; theme.rs:471 | contradicts | `ThemePreset::ALL` (editor_prefs.rs) includes Rusty in the preferences dropdown |
| Motion tokens 80/140ms + ease-out; pressed snaps | DESIGN.md Motion | missing | No motion tokens in engine theme; graph editor has zero animations (§1.12) |
| `popover_alpha` 0.96 / `scrim_alpha` 0.45 | theme.rs Metrics | implemented | palette.rs:117–121 matches |
| Status/axis/text token values | theme.rs | implemented | Engine invariants byte-match theme.rs (error #E25B54 etc.) |
| `on_accent` engine token has no theme.rs counterpart | — | unclear | Engine extra; theme.rs has none — keep (harmless) or add to theme.rs |
| Engine `ShadowTokens` vs "no shadows" rule | DESIGN.md Core 1 | contradicts | `EditorTheme` carries ShadowTokens; spec system is flat — retire during Phase 1 reconcile |

### Metrics & density

| Requirement | Spec source | Status | Notes |
|---|---|---|---|
| Every metric = base × `ui_scale`, resolved at draw | DESIGN.md Metrics | missing | No ui_scale exists anywhere; pixels_per_point pinned 1.0 (gui/crusty.rs:49) |
| Density presets 0.85/1.0/1.15 as ui_scale | same | contradicts | Engine `Density` = Compact/Comfortable only, scales fonts+spacing only, not geometry |
| Base set: radius 2/3/6 · border 1 · edge 2 · row 22 · control 24 · spacing 2‑24 · indent 18 | theme.rs Metrics | missing | Graph editor: 30+ bare literals (inventory §1.14); crusty `Style` metrics not theme-fed |
| 2px reserved (tab edge + type edge), lint-enforced | DESIGN.md Do/Don't | missing | No lint; graph uses 2.0 for selected borders + wire width (would need allow-listing or retokening) |

### Contrast & lint

| Requirement | Spec source | Status | Notes |
|---|---|---|---|
| 28-cell contrast table recomputed + asserted in a test | DESIGN.md Contrast; Phase 1 brief | missing | Engine has `verify_wcag_aa()` for *its* palette; nothing asserts the spec table against theme.rs values |
| Hex lint CI job (fixed regex, inverted exit, full scan, fixture pair) | DESIGN.md Lint; Phase 1 brief | missing | No such job. M10's "grep gate" was manual. Scope must be rewritten for this repo's layout (see §0) |
| 2px-width lint with allow-list | same | missing | — |

### Canvas

| Requirement | Spec source | Status | Notes |
|---|---|---|---|
| Canvas surface = `input` | nodegraph Canvas | implemented | :392 |
| Grid minor 16px @5% white, major 128px @9%; minor drops <40% | same | contradicts | Single 40px grid @ `stroke`, no LOD (:393–:412); prototype's coarse-only alpha is .07 — prose 5/9% wins, prototype stale |
| Grid never accent | same | implemented | Uses `stroke` |
| Grid snapping 8px default-on + 1px accent guide while dragging | prose ~521 | missing | No snapping of any kind |
| Alignment guides (edge/center) during drag | prose ~524 | missing | — |
| Marquee = 1px accent + 8% accent fill; ⇧ add, ⌥ subtract; ⌘ lasso | prose ~528/808 | contradicts | Current: `selection_fill`@0.18 + 1px selection_fill border, replace-only, nodes only (:1098–:1147); prototype has **no** marquee (LMB pans) — prose is the target |
| Zoom about cursor, wheel | prototype 1599–1601 | implemented | canvas.rs Ctrl+scroll; plain-scroll = pan (prototype: plain wheel zooms — minor divergence, current behavior is fine per crusty conventions, flag) |
| F/A frame shortcuts | (current extra, not in spec) | implemented | Keep; spec's `Home`=fit (prototype 1044) maps to A. **Bug**: F/A ignore modifiers, so Ctrl+A/Ctrl+F over canvas also frame (:590) — fix in Phase 2 |

### Node

| Requirement | Spec source | Status | Notes |
|---|---|---|---|
| Header 26px on `elevated`; body `header` fill | nodegraph Node | contradicts | Current: HEADER_H 22, header filled with **category color** (:507–:511), body `elevated` (:521) — inverted fills + the exact "filled title bar" the spec forbids |
| 22px pin rows | same | contradicts | ROW_H = 18 (:33) |
| 6px bottom pad | same | implemented | BODY_PAD 6 (:34) |
| Radius 6 | same | contradicts | 4.0·zoom (:506) |
| Min width 128 / max 320, auto-fit, middle-truncate titles | same | contradicts | Fixed NODE_W 168 (:31); labels overflow, no truncation |
| Category = 2px top edge (deep) + 9px mono tag (bright) | same | missing | No edge, no tag; category paints the whole header instead |
| 1px `stroke` border | same | partial | Border exists but doubles as selection (see Selection) |
| `NodeInst.tint: Option<u8>` per-node override, 12 deep swatches | same | missing | No field (doc.rs:92–110) |
| Height model header+rows·22+pad | prototype hOf | partial | Same shape, different constants; don't port hOf's +4px bug (§0.8) |

### Categories

| Requirement | Spec source | Status | Notes |
|---|---|---|---|
| Reserved table Event/Flow/Math/Logic/Data/Audio/Physics/Spatial/Render/Gameplay/Interface → fixed slots | DESIGN.md; theme.rs PALETTES | missing | No reserved table; everything hashes |
| Dev (and Debug) → neutral, announces fixture status | same; theme.rs:135 | contradicts | "Dev" hashes into `scripting` teal (inventory §2.1) — reads as a real category |
| Unreserved → hash%12, never neutral | same | contradicts | Hash%10 over an ad-hoc array incl. gray-ish `geometry` — gray can be produced by hash |
| Hash fn `h·31+byte`, `%12`, tag = first 5 upper | prototype 1140/1151 | partial | Same hash (:208–:210) but %10 and no tag |
| Tags derived: SUB > PURE > EVENT > cat tag | nodegraph Categories; prototype tagOf 1151 | missing | No tags anywhere; note precedence: prototype puts SUB first |
| Registry facts verbatim-or-not-at-all; only the 4 dev nodes shown as real | nodegraph Categories | implemented | Current dev set is exactly test_event/test_damage/test_add/test_editor_note |

### Pins

| Requirement | Spec source | Status | Notes |
|---|---|---|---|
| Colors: Exec white@92 · Float green · Vec blue · Color violet · Bool ember · Enum rose · Texture amber · Mesh magenta · Entity azure | DESIGN.md pins map | contradicts | Entirely different mapping (Float→physics lime, Vec→geometry gray-ish, etc., :176–:190). Gray-ish Vec ≈ neutral — violates "gray means unknown" |
| Domain(k): keyed → index · registered unkeyed → hash%12 · unregistered → neutral | same | contradicts | All domains → `ui` salmon regardless of registration (:188) |
| Shapes: circle / triangle(Exec 11px) / diamond(Vec, `·3` arity label) / rounded-square(Color) | nodegraph Pins | missing | All pins are filled circles (:539) |
| Filled = connected, hollow = open (exec unconnected = filled grey per prototype) | same; prototype dot() 1492 | missing | Always filled |
| Pins 6px inside border; wire terminates at border at row-center | same | contradicts | Pin centers sit exactly ON the border (inset 0, :156/:163); wires start at pin center |
| Draw r=4.5/9px; hit r=9/18px | same | partial | Draw ✓ 4.5 (:35); hit = 13.5px world square (:660) — smaller than spec at zoom<1.33, shrinks with zoom (spec/prototype: screen-space) |
| Diamond reserved for future container concept | same | unclear | Held pending decision §0.1 |
| Second drop on occupied input **replaces** the edge | nodegraph Pins | contradicts | `validate_connection` refuses occupied inputs outright (:241–:248) and no way to free them (see Breaking) |
| No Int/String/Quat/Transform/Struct/Wildcard implied | same | implemented | True today; §0.1 decision governs future |

### Inline widgets

| Requirement | Spec source | Status | Notes |
|---|---|---|---|
| Per-PropValue widgets: Float mono field / Vec axis fields (2-line, L0 only) / Color swatch+hex / Bool checkbox / Enum dropdown / Asset ref field | nodegraph Inline | missing | Values render as read-only text `label: value` (:558–:565); no editing on canvas at all |
| `Raw` = warning-dashed `preserved` chip | same | missing | Renders via `prop_display` as plain text |
| Entity: connection-only, no widget | same | implemented | No PropValue::Entity exists |
| Keep `edit_popup` for L1-and-below editing | code delta | partial | edit_popup exists but only for annotation text (:1016) |

### Edges (identity)

| Requirement | Spec source | Status | Notes |
|---|---|---|---|
| Edges keyed by pin slug; index never surfaces; reorder = non-migrating | nodegraph Edges | implemented | Edge struct + all error text use slugs |

### Wires (appearance)

| Requirement | Spec source | Status | Notes |
|---|---|---|---|
| Wire = pin color @80%, 1.9px; exec white@92%, 2.4px; via `wire_color()` | nodegraph Wires | contradicts | All wires `accent_active` @ 2.0·zoom (:491, :1153) — accent-budget violation and no type info |
| Selected wire ×2.6/3.0, `selection.outline`; hover `focus_ring` + midpoint handle | same; prototype 1309 | missing | Wires have no hit-test, no states |
| In-flight = dashed source color | same | contradicts | Live drag wire is solid success/error/accent (:794–:799); no dash support needed check in crusty painter (none exists — needs library dash or manual segments) |
| Muted branch grey fine dots; broken = error + × | same | missing | No mute/broken concepts |
| Spline tangent `clamp(|dx|·(0.35+curve·0.55),34,190)` + backward `max(t, 70+0.35|dx|)` | same; prototype path 1124 | contradicts | Current: horizontal tangent `max(|dx|·0.5, |dy|·0.4, 24·zoom)` (:1150) — also **zoom-dependent** (spec: graph-space then scale) |
| Stroke width zoom-invariant, geometry scales | nodegraph routing | contradicts | Width scales with zoom (:1153) — 0.5px wires at 0.25× |
| Wire color from **source** pin type | prototype 1302 | missing | n/a until typed colors land |

### Wire routing (the router)

| Requirement | Spec source | Status | Notes |
|---|---|---|---|
| `WireStyle` Spline/Manhattan/Subway + `WirePrefs` struct in `editor/graph_prefs.rs`, serialized as `graph.wires` in editor_prefs.ron | nodegraph routing; brief | missing | File doesn't exist; prefs has only zoom min/max. Note: current prefs serialize flat — a nested `graph.wires` section needs a struct-in-struct (fine with serde) |
| Branch order: near-horizontal (`|dy|<2r ∧ dx≥6|dy|`, ≈9.46°) → subway forward (compress `min(16,(dx−|dy|)/2)` → vertical-then-45°) → manhattan forward (`dx≥8`, stagger `min(16+bi·4, max(4,dx/2))`) → band (`−24<dx`, straight if `|dy|<20`) → backward lane (6 pts, both-rect lane ±24, fallback ±34, corridor `[x2−16, x1+16]`, tie→target side) | prototype polyPts 1069–1104 (verbatim in fact sheet) | missing | Port structurally. Note prototype threshold `dx≥8` for Manhattan is undocumented in prose — keep, document |
| `roundedPath`: dedup ε1e-6 then per-corner clamp `min(r, l1/2, l2/2)`, quadratic corners | prototype 1106–1123 | missing | Rust: emit arcs or quadratics into polyline for crusty painter (no path primitive — tessellate manually) |
| Routing math in graph space, then scaled | nodegraph routing | missing | — |
| Turn anchor Target default / Source mirror; `turn_priority` None/Node/Pin; `disable_pin_offset` | same | missing | Prototype hardcodes Target/None — prefs are spec-only; implement per spec |
| Backward-lane threshold −24 as named field | brief; prototype comment 1086 | missing | Resolution recorded §0 |
| Bundling v0 = prototype pin-index stagger; bundling proper (perpendicular 4px, merge 20, max 8, Force Outside) later | §0.5 decision | missing | Two-stage plan proposed |
| Crossings None/Gap/Arc/Circle, exec-over-data, ~2k segment cap, off below L2 | nodegraph routing | missing | Absent from prototype too — spec-only, derive cap from budget |
| Exec override prefs | same | missing | Spec-only |
| Flow bubbles: 4px, 150px/s, 40px spacing, debug-only, L0–L1 only | same | missing | Prototype: 3 bubbles r=3, duration = L1-distance/150, no zoom gate, no spacing model — **prose wins** (40px spacing), note divergence |
| Acceptance tests 1–10 + exhaustive sweep 11 (dx∈[−40,400]×dy∈[−200,200], 0.5° of 45° multiples, two exceptions) | brief; nodegraph acceptance | missing | Write test 11 first per brief. Exceptions as implemented: ≤9.46° shortcut; residual stub `|dx|<24∧|dy|<20` |
| Midpoint handle + broken-× at arc-length midpoint | nodegraph re-verify | missing | NOT in prototype (fact-sheet item 10) — implement from prose |
| Hover hit 9px/side screen-space; slash-cut exact seg-seg with preview; splice at nearest polyline point | same; prototype segHit/cutTest | partial-reference | Cut + hit exist in prototype (port); splice does NOT (prose-only). Sampling: 4 subdivisions/segment, spline 17 pts |

### Selection (canvas)

| Requirement | Spec source | Status | Notes |
|---|---|---|---|
| Node keeps fill; border → `selection.outline`, last-clicked 100% / rest 55%, 2px outline offset | nodegraph Selection | contradicts | Border swaps to `selection_fill` (wrong token), no offset, no primary/rest distinction (:512–:524) |
| Accent on canvas: compile chip + marquee + alignment guides only | same | contradicts | Accent spent on all wires + edit-popup border today |
| ⇧-click toggle | prose | implemented | :696–:702 |
| Wire selection (click, ⇧ extend) | Breaking §1 | missing | No wire hit-test |

### Zoom LOD

| Requirement | Spec source | Status | Notes |
|---|---|---|---|
| Explicit 5-level enum: L0 90–220 · L1 60–90 (widgets→values) · L2 35–60 (pin labels drop) · L3 15–35 (header+edge only) · L4 <15 (4px bars + 1px wires); below 35% zero glyphs; not scattered ifs | nodegraph LOD; Phase 2 brief | missing | Current: crusty `label_size` 7-step font quantizer + 7px floor — all labels vanish together at ≈<0.63 zoom; no per-element ladder. Replace for the graph (canvas `label_size` stays for other users) |
| Annotation titles exempt down to L4 at floor size | same | missing | — |
| Subway collapses to Manhattan/straight below L2 | routing | missing | — |
| Prototype has no LOD (grid-only k>0.4) | prototype §3.1 | — | Prototype models current build; ladder is spec-only — flagged unverified |

### Badges

| Requirement | Spec source | Status | Notes |
|---|---|---|---|
| One glyph, left gutter, precedence breakpoint-hit → error → warning → breakpoint → hidden-pins | nodegraph Badges | missing | No badges; errors shown only in corner overlay |
| Status color reaches node border | same | partial | Error border exists for missing-type nodes only (:515) |

### Annotations

| Requirement | Spec source | Status | Notes |
|---|---|---|---|
| GroupBox: tint = 6% body wash + 45% border; 20px title bar; auto-fit nodes +12px margin; collapsible; below nodes | nodegraph Annotations | partial | Wash 12% `panel`, border stroke_strong, 20px bar ✓, no tint, no auto-fit, no collapse (:417–:443) |
| CommentBox: NOTE bar tint + 1px left edge; body **opaque `elevated`**, never washed; 18px bar; above groups below nodes | same | contradicts | Body is `elevated`@0.35 translucent (:452) — exactly the forbidden wash; header bar carries no NOTE label |
| Fields: `tint: Option<u8>`, `font_scale` 0.75–3.0, `anchor: Option<NodeId>`, `collapsed`, all serde(default) | same | missing | Neither struct has any (doc.rs:133–143) |
| Tint = 12 deep swatches, never free picker | same | missing | — |
| Anchored note follows/dies with node | same | missing | — |
| Corner/edge resize; auto-grow height; width author-set | same | missing | Move-only via header (inventory §5.4) |
| Collapse comment to NOTE bar | same | missing | — |
| `#node-slug` chips in body; click selects+frames | same | missing | — |
| Body plain text verbatim | same | implemented | Stored verbatim, single-line edit popup mangles multi-line though (:1049 — flag) |
| Prototype comment colors hardcoded purple | prototype §3.4 | — | rgba(98,83,185)= ramp slot 9 deep — becomes `tint: Some(9)` in port; not a new color |

### Validation errors

| Requirement | Spec source | Status | Notes |
|---|---|---|---|
| Anchored per-error: node border+badge, pin ring, or wire; overlay demoted to count | nodegraph Validation | missing | Corner overlay, first 3 lines + count (:1246–:1291); not clickable |
| TypeMismatch = only wire-coloring error | same | missing | n/a until wire states land; note prototype can't even create a mismatched wire (refused at drop) — the state arises from *edits after* wiring (descriptor change), which Rust supports |
| Doc-level rows for DanglingEdgeNode/SubgraphCycle; cycle = clickable mono breadcrumb | same | missing | — |
| Ghost rows for UnknownPin/SubgraphPinUnknown | same | missing | Edges to unknown pins currently just have no endpoint (wires silently vanish for missing-type nodes — worse: inventory §2.3) |
| Reference errors draw on the subgraph node that pulled them in | same | partial | `ref_errors` exist separately but render only in the shared overlay |
| "valid" and "compiled" two chips | same | missing | No compile concept (45-A) |
| Closed set of ten | same | unclear | §0.3 |

### Realm / purity

| Requirement | Spec source | Status | Notes |
|---|---|---|---|
| Graph realm mono chip in toolbar | nodegraph Realm | missing | No toolbar at all (inventory §7.3) |
| Node realm on 10px subtitle row; Shared prints nothing | same | missing | No subtitle row |
| RealmViolation: error border + chip naming both sides | same | partial | Error text names both (validate.rs Display); no visual |
| Non-admitted palette entries listed at 45% | same | missing | Palette doesn't know realms |
| `deterministic: false` = `~` glyph where relevant | same | missing | — |
| Realm change preflights invalidation count | same | missing | Realm isn't editable in UI at all |

### Breaking connections

| Requirement | Spec source | Status | Notes |
|---|---|---|---|
| Click wire + Del (⇧ extends) | nodegraph Breaking | missing | **No interaction removes an edge today** — `GraphEdit::Disconnect` exists, never constructed by UI (inventory §5.3). Biggest single functional hole with copy/paste-shape fixed |
| ⌥-click pin / node header breaks links | same | missing | Prototype has both (breakPin/breakNode) — port semantics |
| ⌘-drag slash-cut, red-dash preview, Esc abort | same | missing | Prototype has cut (40-pt path cap, exact seg-seg) — port |
| Each reports "Broke N links", one undo transaction, also in context menu | same | missing | No toast system on canvas; prototype flash() = 1800ms |

### Node inspector

| Requirement | Spec source | Status | Notes |
|---|---|---|---|
| Four blocks: Title (F2/double-click rename call-site) · Actions · Descriptor (read-only, warnings tinted) · Defaults (connected inputs listed greyed) · Pins (✕ break per pin) | nodegraph Inspector | missing | No graph-node inspector exists; engine inspector panel is entity-only. Node double-click does nothing (subgraph nodes open the asset) |

### Organization

| Requirement | Spec source | Status | Notes |
|---|---|---|---|
| Groups `C` / note `⇧C` | nodegraph Organization | partial | Create via context menu only; no keys |
| Reroutes (insertable on wire), one-in-many-out | same | missing | — |
| Named reroutes (decl/usage pair, listed first in palette) | same | missing | — |
| Collapse to subgraph `⌘G` | same | missing | — |
| Align & distribute 3+ `⌥A` | same | missing | — |
| View bookmarks `⌘B` | same | missing | — |
| Purge unused `⌥⌘K` with count confirm | same | missing | — |

### Add-node palette

| Requirement | Spec source | Status | Notes |
|---|---|---|---|
| Asset-picker shell at E3; `Tab` or double-click opens unfiltered | nodegraph Palette | partial | Right-click-only context menu, generic styling; no Tab, no double-click-canvas |
| Drag off pin → release empty → type-filtered list; incompatible at 45% w/ type tag, never hidden | same | missing | Release on empty cancels today; prototype hides incompatible + caps 6 — **prose wins** (45% dimming, full ranked list) |
| Release on node body → auto-connect best compatible pin | same | missing | Prototype opens palette instead — prose wins |
| Search ranking: exact-prefix → whole-word → fuzzy, five tiers | prose ~677 | partial | Plain `contains` on name/id (:1188); no ranking, no keyboard nav |
| New node lands wired-pin-at-drop, nudge 8px until clear | same | missing | Lands at right-click point, no nudge, no auto-wire |

### Debugging

| Requirement | Spec source | Status | Notes |
|---|---|---|---|
| Breakpoint octagon (hollow/disabled/invalid states), paused = warning + PAUSED chip + tinted taken path | nodegraph Debugging | missing | No debug anything; blocked on 45-A evaluator for live parts; breakpoint *marks* could land earlier as inert data |
| Watch chips (`input`@94%, mono, dashed-until-executed) | same | missing | Prototype chip hardcodes Steel hexes — retoken on port |
| Execution trace newest-first; debug-object picker in toolbar | same | missing | — |
| Flow bubbles (see routing) | same | missing | — |
| Debug state in user-local sidecar keyed by asset path | Persistence | missing | — |

### Copy / paste / duplicate

| Requirement | Spec source | Status | Notes |
|---|---|---|---|
| Clipboard = RON subset incl. annotations; survives tabs **and sessions** | nodegraph C/P/D | contradicts | In-memory `GraphFragment{nodes,edges}` only; no annotations; dies with app (inventory §5.1). Cross-tab works |
| Remap by pin slug; boundary edges drop with count report "Pasted N, M links dropped" | same | partial | Remap ✓; boundary edges drop **silently**, no report |
| Paste at cursor, preserve layout, nudge 8px until clear | same | contradicts | Fixed +30,+30 from source; not pointer-relative; no clear-nudge |
| Cross-graph paste validates realm+registry; unresolvable → `preserved` placeholders | same | missing | Pastes blind; unknown types then render missing-red (accidental near-match, but no `preserved` semantics) |
| `⌘D` duplicate = +16,+16, one transaction | same | contradicts | Ctrl+D exists ✓ but +30,+30 |

### Auto-layout · Previews · Error nav · Find · Hover docs

| Requirement | Spec source | Status | Notes |
|---|---|---|---|
| `⌥L` Sugiyama layered, whole/selection, one undo, groups fixed, pinned nodes never move | nodegraph Auto-layout | missing | Also: no "pinned node" concept exists (needs a field or session state — spec unclear where pinning lives) |
| `descriptor.preview: Option<PreviewKind>`, 64×64 L0-only, round-robin budget | Per-node previews | missing | Descriptor has no field |
| `F8`/`⇧F8` cycle errors, frame+select anchors | Error navigation | missing | — |
| `⌘F` find-in-graph, dim non-matches 45%, ↵ cycles | Find in graph | missing | — |
| Pin tooltip: type + descriptor doc line, 400ms | Pin hover docs | missing | `PinDescriptor` has no doc field (registry.rs:18) — needs schema addition |

### Culling + budget

| Requirement | Spec source | Status | Notes |
|---|---|---|---|
| Viewport culling on node rects **and wire segments** | Culling | partial | Nodes/annotations culled; wires and all interaction rects are not; per-pin `interact` allocs every frame (inventory §5.5) |
| Stated budget 60fps @ 2k nodes / 5k edges; crossing cap derived | same | missing | No budget, no measurement; `build_geoms` re-allocs strings per frame — needs a pass before the budget is honest |

### Diff-friendly serialization

| Requirement | Spec source | Status | Notes |
|---|---|---|---|
| Nodes serialize in stable id order | Serialization | contradicts | Vec insertion order (paste/delete reorder it) |
| Positions round to whole pixels | same | contradicts | Raw f32s serialized |

### Persistence

| Requirement | Spec source | Status | Notes |
|---|---|---|---|
| Positions/groups/comments/refs/tints in asset | Persistence | implemented | (tints pending — no field yet) |
| Breakpoints/watches/bookmarks/**last view transform** in user-local sidecar keyed by asset path | same | missing | View resets every open (inventory §6.3) |

### Settings surface (Editor Preferences ▸ Graph)

| Requirement | Spec source | Status | Notes |
|---|---|---|---|
| Graph sidebar category with Wires / Execution wires / Bundling / Crossings / Flow bubbles sections | panels ▸ Graph | missing | Current "Graph Editor" category = 2 zoom rows only (settings_crusty.rs:833) — keep as the "canvas prefs natural neighbours" the spec anticipates |
| Rows disable by style, never hide; nesting rule | same | missing | — |
| Live preview strip ~300×120 through the **real** router; 3 samples | same | missing | Third sample must change per min_dist deletion (§0) |
| Toolbar 3-way segmented control, same pref, nothing else | same | missing | There is no graph toolbar at all — this creates it |
| `WirePrefs` all serde(default), one prefs file, `graph.wires` section | nodegraph routing | missing | — |
| No RESTART chips | same | n/a | Trivially satisfied |

### Out-of-scope note (DESIGN-panels.md non-graph sections)

Editor tabs (pinned viewport, Hide Tabs, overflow chip), asset tiles, asset
reference fields: not covered by the phase plan in the brief; current M10
implementations differ in several details. Not audited row-by-row here —
say if you want a follow-up audit; nothing in the graph phases depends on them
except the palette's "asset-picker shell" styling reference.

---

## 2. Hardcoded values that should be tokens

Full literal inventory: `graph_editor_crusty.rs` — 30+ bare numerics (grid 40,
rounds 4.0, insets 6/8/12, alphas 0.12/0.18/0.35/0.6, wire tangent 0.5/0.4/24,
widths 1.0/2.0, popup 200×26/196/170, overlay 6/3/1.35/8) listed exhaustively
in the Phase-0 inventory §1.14; plus `graph_editor.rs` paste +30/+30, comment
default 220×130, group pad 24/est 168×100, frame PAD 0.9. None reads
`st.spacing`/`st.metrics`/density. All colors already resolve through the
(old) palette — zero hex literals in graph code ✓ — but several *token choices*
violate the new spec (accent wires, selection_fill borders, panel/elevated
washes) per the table above.

---

## 3. Proposed implementation order & effort

Follows the brief's phases; effort assumes the current velocity (agent-driven
packages with review gates).

| Phase | Contents | Effort | Notes |
|---|---|---|---|
| 1 | theme.rs landed + reconcile (delete type_colors, retarget 8 consumer files incl. 5 non-graph panels), resolvers + `domain_registration`, ui_scale plumbing (new — crusty Style feed), presets/user_selectable fix, contrast test, both lint jobs + fixtures | 3–4 d | ui_scale is the sleeper: touches the crusty style seam. Blocked on §0.1 only for the Enum payload pattern |
| 2 | Node geometry (26/22/6/r6/128–320/truncation), 2px edge + 9px tag + derived tags, pin shapes/fill states/inset-6, inline PropValue widgets, LOD enum ladder + annotation-title exemption, marquee retoken + ⇧/⌥ modes, F/A modifier fix | 4–5 d | Inline widgets are the biggest chunk (new per-type field widgets on canvas) |
| 3 | Router port (branch structure per fact sheet), WirePrefs + graph_prefs.rs, spline retune, wire states/colors, sweep test 11 first + tests 1–10, midpoint/hit/cut/splice on polylines, zoom-invariant stroke | 5–7 d | Highest risk; fact sheet has the verbatim reference. Bundling-v0 per §0.5 |
| 4 | Settings ▸ Graph (5 sections, disable rules, live preview via real router), toolbar (new) + segmented control + realm chip | 2 d | Toolbar creation is new scope the spec assumes |
| 5 | Annotation divergence + new fields (tint/font_scale/anchor/collapsed) + resize/auto-grow/collapse + #node-slug chips, reroutes + named reroutes, collapse-to-subgraph, align/distribute, bookmarks, purge-unused, anchored error rendering + ghost rows + count overlay | 5–6 d | Ten GraphError anchors depend on Phase 2/3 visuals |
| 6 | Clipboard → RON (annotations incl.), cursor-anchored paste + 8px nudge, boundary-drop report, preserved placeholders, break gestures (wire+Del, ⌥-click, slash-cut) + toasts | 3–4 d | Break gestures grouped here since they share wire hit-testing from Phase 3 |
| 7 | Palette rebuild (Tab, drag-off-pin filtered, 45% dimming, ranking, auto-connect, drop-nudge), find-in-graph, error navigation F8, pin hover docs (+descriptor doc field), auto-layout, per-node previews (schema + budget), culling/budget pass, diff-friendly serialization, view/debug sidecar | 6–8 d | Debug visuals beyond inert breakpoint marks blocked on 45-A |

Total ≈ 28–36 working days of agent time. Independent early wins if you want
them pulled forward: diff-friendly serialization (hours, stops asset churn now)
and the F/A modifier bug (minutes).

**Stopping here per the brief — no feature code until you've reviewed this.**
