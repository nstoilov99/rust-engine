# Task 41.7 — Animation Pipeline Root (constrained pose graph)

**Status:** draft v0 (2026-09-21) — for Codex/Astra challenge, then user grilling.
**Depends on:** Task 41 (machine + regions), 41.5 (IK, throttling, parallel eval), 41.6 (foot IK v2, demo graph = migration fixture).
**Branch:** `task-41.7-anim-pipeline-root` off `main` @ `24b8545`.
**Decision record:** 2026-09-06 debate (`.scratch/anim-flow/round{1,2}.txt` + Opus): the animgraph root becomes a constrained pose graph in the Unreal AnimGraph shape; the user wants per-bone layering (combat casts over locomotion).

## 1. Goal

Separate *what the character does* (the state machine) from *how the final
pose is assembled* (the pipeline). Opening an `.animgraph` lands on the
**pipeline canvas**: pose sources → blends/overlays → IK tail → Output
Pose. The state machine is a node on that canvas; double-clicking it enters
today's machine canvas (breadcrumb `graph › State Machine`), exactly as a
transition's rule opens today. Slots and IK chains stop being loose nodes
beside the states.

Concretely, after this task the demo graph reads:

```
[State Machine] ─pose─▶ [Layered Blend Per Bone] ─▶ [Overlay: cast] ─▶ [IK foot_l] ─▶ [IK foot_r] ─▶ [Output Pose]
[Clip: upper idle] ──▲ (layer, mask: mixamorig:Spine + descendants, weight `aim`)
```

### Non-goals (deferred ledger)

Additive layers, mesh-space (component-space) blending, per-bone weight
falloff curves, multiple overlay channels, cached-pose / pose-snapshot
nodes, look-at aim offsets, motion matching node, arbitrary DAGs with
fan-out, undo for pipeline property edits beyond what the panel has.

## 2. What exists (verified 2026-09-21)

- **Document** (`crates/node_graph_types`, v3): one flat `nodes` list +
  `edges`; `regions: BTreeMap<owner node id, GraphRegion{nodes, edges}>`
  — regions **cannot nest**, and every state (pose tree) and transition
  (rule) already owns one. Migrations are additive stamps
  (`io.rs::migrate_container`); the animation loader additionally runs
  `upgrade_any_state` (domain-specific rewrite on load).
- **Top-level canvas** = machine nodes (`anim_entry`, `anim_state`,
  `anim_state_alias`, `anim_transition`) **plus** pin-less `anim_play_once`
  slot nodes and `anim_ik_chain` nodes. The compiler collects slots and IK
  chains from the flat list, sorts by node id, and merges nested graphs'
  in (`plan.rs` ~1000–1210).
- **Plan**: `AnimGraphPlan { states, transitions, entry, parameters,
  slots, ik_chains }`; `PoseSource::{Tree(PlanTree), Machine{plan}}`.
- **Runtime order** (`runner.rs::tick_entity`): `machine.tick` →
  `slot.tick` → events → eval gate → `evaluate_pose` (machine → blend
  trees, `PoseScratch` levels) → `slot.apply` (single channel overlay,
  local space) → `compute_model_space` → `apply_ik` (pelvis pre-pass, then
  chains in plan order, model space) → palette. Parallel over entities
  (rayon), throttled by significance bucket; serial steps 2.5/2.6 write
  `resolved`/`lock_model`.
- **Editor**: one `GraphEditorState` per document = the machine canvas.
  A transition's rule opens as `RuleScope` (a projection editor over the
  region, edits drained back as `GraphEdit::InRegion`). Palette placement
  is gated by registry (`anim_node_registry` vs `anim_rule_registry`).
  Error anchoring (`anchor_anim_refusal`) has arms for state/transition/
  slot/alias/ENTRY, none for IK.

## 3. Design

### D1 — Pipeline nodes live in the same flat document, partitioned by type

No container change beyond a version stamp. The top-level `nodes` list
holds two families:

| Family | Types | Canvas |
|---|---|---|
| machine | `anim_entry`, `anim_state`, `anim_state_alias`, `anim_transition` | Machine scope (today's canvas) |
| pipeline | `anim_pipe_machine`, `anim_clip` (as a source), `anim_pipe_layer`, `anim_play_once`, `anim_ik_chain`, `anim_pipe_output` | Pipeline scope (the new root) |

Edges partition with their endpoints (`anim_flow` edges are machine;
`anim_pose` edges are pipeline). Regions keep their owner-id keying —
nothing moves. This is the "scope over the flat document" ruling: the
alternative (wrapping the machine into a region) is impossible because
regions cannot nest and the states already own regions.

### D2 — Pipeline node set (registry `anim_pipeline_registry`)

All pipeline nodes carry `anim_pose` pins (rose, keyed domain).

- **State Machine** `anim_pipe_machine` — out: Pose. Prop `graph: Asset`
  (empty = *this document's* inline machine; a path = a nested
  `.animgraph`'s machine, evaluated as its own `AnimMachine` instance).
  Exactly one node may be inline. Double-click (inline) enters the machine
  scope; (nested) opens that file.
- **Clip** `anim_clip` — out: Pose. The existing pose-tree clip node,
  admitted at the root as a static pose source (an upper-body idle to
  layer, a held aim pose). Loops; `speed` prop honoured. (Blend1D/2D and
  blend space nodes are **not** admitted at the root in v1 — a state does
  that job.)
- **Layered Blend Per Bone** `anim_pipe_layer` — in: Base, Layer; out:
  Pose. Props: `bones: Str` (comma list of mask roots; each root and all
  its descendants get weight 1, everything else 0), `weight_param: Str`
  (declared Float, 0..1), `include_root: Bool` (default true; false =
  descendants only, for "arms but not the shoulder joint"). Local-space
  per-bone blend: `out[b] = lerp(base[b], layer[b], mask[b] · weight)`
  using the existing `LocalBoneTransform::blend`. Mask resolved to a
  `Vec<f32>` per skeleton at arm time (by-name; a missing root bone
  refuses at arm, anchored on the node — same rule as IK bones).
- **Play Once** `anim_play_once` — in: Pose; out: Pose. Existing props
  (trigger, clip, fades) plus new `bones: Str` (optional mask, same
  semantics as the layer node; empty = whole body). The single overlay
  channel is unchanged in v1; **wire order replaces node-id order** for
  the "first triggered wins" priority (upstream wins). The node applies
  the channel's clip only if *it* is the one playing; otherwise it passes
  the pose through — an honest chain.
- **IK Chain** `anim_ik_chain` — in: Pose; out: Pose. Existing props.
  Model-space stage; **wire order replaces node-id order** for the
  application order. Foot config unchanged.
- **Output Pose** `anim_pipe_output` — in: Pose. Exactly one.

### D3 — Compile rules (anchored refusals)

The pipeline compiles to a tree by walking back from Output Pose:

1. Exactly one Output Pose; its input wired. (`"the pipeline has no
   Output Pose"` / `"two Output Pose nodes"` / `"Output Pose is unwired"`.)
2. Every node reachable from Output has all pose inputs wired; unreachable
   pipeline nodes are a **warning** (drawn dimmed), not a refusal.
3. Stage order on every path root→Output: `[sources] → [Layer | Play Once]*
   → [IK Chain]* → Output`. A local-space node downstream of an IK Chain
   refuses: `"'X' must come before the first IK Chain — IK works in model
   space"`. (No component-space conversion node in v1.)
4. Fan-out is refused (`"'X' feeds two nodes — pose wires are one-to-one
   in v1"`); fan-in only via the Layer node's two inputs.
5. Exactly one inline State Machine node when the document has machine
   nodes; a document with machine nodes but no inline State Machine node
   refuses (`"the state machine is not in the pipeline — add a State
   Machine node"`); zero machine nodes + no inline SM node is legal (a
   pure clip/layer document).
6. Mask roots and weight params validated like IK: param must be a
   declared Float; bones resolve at arm time.
7. Nested documents (`anim_pipe_machine.graph` or a state's `graph`
   prop): **only their machine is consumed**; their own pipeline nodes are
   ignored, and any slots / IK chains they carry are **lifted into the
   host** exactly as today (compat), with a compile *warning* naming them
   (`"nested 'x.animgraph' carries 2 IK chains — applied by the host's
   pipeline tail"`). Lifted chains/slots order **after** the host's wired
   ones. (Astra R2's "state references consume machine-only assets" —
   v1 keeps lifting so existing nesting keeps working; a future version
   may forbid it.)

### D4 — Plan and runtime

`AnimGraphPlan` gains `pipeline: PlanPipeline`:

```rust
pub enum PlanPose {
    Machine { source: usize },           // index into plan.machines
    Clip(PlanClip),
    Layer { base: Box<PlanPose>, layer: Box<PlanPose>, mask: PlanMask, weight_param: String, node_id: u64 },
    Overlay { input: Box<PlanPose>, slots: Vec<usize> /* indices into plan.slots, wire order */, mask: Option<PlanMask> },
}
pub struct PlanPipeline { pub root: PlanPose, pub ik_order: Vec<usize> /* into plan.ik_chains */ }
pub struct PlanMachineRef { node_id: u64, inline: bool, plan: Option<Arc<AnimGraphPlan>> /* nested */ }
```

`plan.machines[0]` is the inline machine (states/transitions/entry as
today); nested refs hold their compiled plan. `plan.slots` keeps its
Vec (arm-time clip lookup unchanged) but the **channel arbitration order
is `pipeline` wire order**; `ik_chains` likewise ordered by `ik_order`.

Runtime (`AnimGraphRuntime`): `machine: AnimMachine` becomes
`machines: Vec<AnimMachine>` index-aligned with `plan.machines`
(`machine()` accessor keeps every existing call site on index 0 — the
inline machine — so the editor viz, state-alias logic and foot-lock
`lock_state` are untouched). `tick_entity`: tick every machine, then the
slot, collect events from all, then `evaluate_pipeline(&plan.pipeline,
…)` recursively into `PoseScratch` levels (one buffer per tree depth:
Layer takes two, blends into the base buffer). Masks: `PlanMask` is a
`Vec<f32>` resolved per skeleton at arm time (stored on the runtime beside
`ik`, keyed by node id). Overlay applies `slot.apply` with the mask.
`compute_model_space` → `apply_ik` over `ik_order` → palette: unchanged.
**Throttling, parallel eval, foot placement, the crowd path: untouched**
— the pipeline evaluates inside the same eval gate on the same worker.

Parity: a migrated v3 document (machine + N slots + M chains) must
evaluate **bit-identically** to today's runtime. Golden test: the demo
graph before/after migration, 120 ticks, palette equality.

### D5 — Migration (document v3 → v4)

`migrate_container` 3→4 is the additive stamp. The animation loader's
domain upgrade (beside `upgrade_any_state`) runs on any animgraph whose
top-level has machine nodes but no `anim_pipe_output`:

1. Add `anim_pipe_machine` (inline) at the left, `anim_pipe_output` at
   the right.
2. Existing `anim_play_once` nodes: chain them in **node-id order** (the
   order they had), then existing `anim_ik_chain` nodes likewise, then
   Output. Wire `SM → slots… → chains… → Output`. Positions: a row laid
   out left-to-right at y = 0 with 260 px spacing; their old machine-canvas
   positions are discarded (they were never meaningful there).
3. Nothing else changes: states, transitions, regions, variables, ids.

Runs at load (like the alias upgrade) so old files keep working
unmodified until saved; the editor marks the document dirty on upgrade
so the author sees a save prompt ("upgraded to pipeline root"). Test
fixtures: `character.animgraph` (no slots/IK → `SM → Output`),
`locomotion_demo.animgraph` (two chains), `defeated.animgraph`.

### D6 — Editor

- **One `GraphEditorState`, two scopes.** `CanvasScope::{Pipeline,
  Machine}` on the animation-domain editor state. The scope filters which
  nodes/edges are drawn, hit-tested, box-selected, pasted into and
  auto-laid-out; each scope keeps its own pan/zoom; selection clears on
  switch. No projection/drain machinery: both families are the same
  document with shared ids, so edits record as today. (This is cheaper
  than a `RuleScope`-style child editor and avoids the id-remap and
  drain-back cost the region path pays.)
- **Root = Pipeline.** Opening an animgraph shows the pipeline; the
  breadcrumb reads `‹graph›`. Double-click the inline State Machine node
  (or press Enter on it) → Machine scope, breadcrumb `‹graph› › State
  Machine`; PageUp / breadcrumb click returns. A nested State Machine
  node opens the referenced file in a new tab (existing open-request
  path). Rule peeks (`RuleScope`) work from the Machine scope only.
- **Palette gating by scope**: Pipeline scope lists
  `anim_pipeline_registry` (+ `anim_clip`), Machine scope lists the machine
  subset of `anim_node_registry` (slots and IK chains removed from it).
  Paste across scopes drops the foreign family with a console line.
- **Header chips**: Play Once and IK Chain nodes show their applied order
  (`1`, `2`, …, derived from the compiled wire order) in the header; the
  Layer node shows the mask root count.
- **Details**: existing config rows for slot/IK; new rows for Layer
  (Bones text, Weight param dropdown from `doc.variables`, Include Root
  checkbox) and State Machine (Graph asset picker, read-only "inline"
  when empty); Output has none.
- **Diagnostics**: `anchor_anim_refusal` gains arms for every pipeline
  type (incl. the missing IK arm); F8 / clicking an error switches to
  the scope that owns the anchored node. Compile *warnings* (unreachable
  node, lifted nested chains) render as the existing anchored warning
  style.
- **Preview**: the Anim Preview panel drives the full pipeline (it calls
  the runtime; nothing to change beyond the machine accessor). The
  preview mesh property stays on ENTRY (machine scope); the pipeline
  scope's Details shows it on the inline State Machine node too (same
  property, two access points).
- **Selection/undo/copy-paste**: unchanged mechanics; the family
  partition is a draw/hit filter. `GraphEdit` records are unaffected.

### D7 — Layered blend semantics (v1)

Local-space per-bone lerp of translation/scale and slerp of rotation,
weight = `mask[b] × weight_param`. Mask = 1 on the listed roots (unless
`include_root: false`) and all descendants, 0 elsewhere. Root motion is
out of scope. Known limitation, documented: local-space layering of the
spine means the layered upper body inherits the base's pelvis/spine
orientation — the standard "mesh space vs local space" trade; Unreal's
default is also local. Mesh-space is deferred.

## 4. Work packages (one commit each, serial)

| P | Scope | Files |
|---|---|---|
| P0 | This plan; Astra + user review; rulings into `.scratch/pipeline/spec.md` | docs |
| P1 | **Document + compiler**: node type ids/descriptors, `anim_pipeline_registry`, `PlanPipeline` + `PlanMask` + `plan.machines`, compile walk with the D3 refusals/warnings, v4 migration + domain upgrade, fixture tests (three shipped graphs migrate; each refusal; nested lifting) | `plan.rs`, `library.rs`, `node_graph_types` (version stamp), new `pipeline.rs` |
| P2 | **Runtime**: `machines: Vec<AnimMachine>`, `evaluate_pipeline`, mask arming, overlay/IK wire order, events from all machines; golden parity test on the demo graph; acceptance tests for layer blend (masked bones follow the layer, others the base) and masked play-once | `runner.rs`, `machine.rs`, `acceptance.rs` |
| P3 | **Editor**: `CanvasScope` filter (draw/hit/select/paste/layout), root = pipeline, State Machine double-click + breadcrumb, palette gating, header order chips, Details rows, refusal anchoring for all pipeline types incl. IK, F8 scope switch, dirty-on-upgrade prompt | `graph_editor.rs`, `graph_editor_crusty.rs`, `anim_node_registry`, `dock`/breadcrumb |
| P4 | **Demo**: `locomotion_demo.animgraph` gains a Layered Blend (upper-body clip from the user — a Mixamo "Standing Arguing"/"Waving"/aim pose, or `Idle_1` masked to `mixamorig:Spine` as a stand-in) driven by a new `aim` Float, plus a masked Play Once cast (if a clip is provided). Inspector/Variables tuning only. Net player's `character.animgraph` migrates but stays `SM → Output`. | content |
| P5 | **Close-out**: ARCHITECTURE ▸ Animation Pipeline (replaces the "IK chains are standalone nodes" text), KNOWLEDGE gotchas, ROADMAP ledger, CLAUDE.md, Opus review of P1–P3, user live verification | docs |

P1 and P2 are engine-only and testable headlessly; P3 is the largest
and the only one with editor risk; P4 needs one clip from the user.

## 5. Acceptance

- Opening any shipped animgraph shows a pipeline canvas with `State
  Machine → … → Output Pose`; double-click enters the machine, breadcrumb
  returns; saving writes v4; reopening is stable (no re-upgrade, no diff
  churn).
- The demo character animates identically to 41.6 before any authoring
  (golden parity test + live check).
- Adding a Layered Blend with `Idle_1` masked to the spine and `aim = 1`
  shows the upper body holding the layered pose while the legs walk; `aim
  = 0` restores the base. A masked Play Once plays a cast on the arms
  while walking.
- Every D3 refusal anchors on its node in the pipeline scope; an IK
  refusal anchors (new).
- Crowd bench (`--stress-anim 300`) within 5 % of the 41.5 numbers.
- Engine + client tests green; both hosts launch with a clean schedule.

## 6. Risks / open questions for the challenge round

- **R1 Scope-as-filter vs child editor.** The filter approach touches
  many draw/hit/paste sites (Opus counted ~30 `is_animation*` branches).
  Is a `RuleScope`-style projection actually less invasive despite the
  id/drain machinery? (My call: filter; ids stay shared, undo untouched.)
- **R2 Nested documents keep lifting slots/IK** (compat) vs forbid now.
- **R3 Overlay as chained per-slot nodes** (each Play Once node has pose
  pins) vs one Overlay node + slot rows. Chained nodes make the mask
  per-slot natural and reuse the existing node; is the "only the playing
  one modifies" pass-through honest enough?
- **R4 Local-space layering only** — is a mesh-space option needed for
  the user's casts to read well, or is that a later task?
- **R5 Multiple inline machines** (upper/lower body machines in one file)
  are excluded; a second machine must be a nested file. Acceptable?
- **R6 `anim_clip` at the root** — enough, or admit blend1d/2d too?
- **R7 Migration laid-out positions** — any reason to preserve the old
  standalone positions?
