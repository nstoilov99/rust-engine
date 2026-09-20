# Task 41.7 — Animation Pipeline Root (constrained pose graph)

**Status:** draft v1 (2026-09-21) — v0 challenged by Codex/Astra (`.scratch/pipeline/astra1.out`), revised; awaiting round 2 + user grilling.
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

After this task the demo graph reads:

```
[State Machine] ─pose─▶ [Layered Blend Per Bone] ─▶ [Play Once: cast] ─▶ [IK foot_l] ─▶ [IK foot_r] ─▶ [Output Pose]
[Clip: upper idle] ──▲ Layer   (mask: mixamorig:Spine + descendants, weight `aim`)
```

### Non-goals (deferred ledger)

Additive layers, mesh-space (component-space) blending, per-bone weight
falloff curves, multiple overlay channels, cached-pose nodes, look-at aim
offsets, motion matching node, arbitrary DAGs with fan-out, undo for
pipeline property edits beyond what the panel has, forbidding nested
documents' lifted slots/IK (kept for compat, see D3.7).

## 2. What exists (verified 2026-09-21; corrections from round 1 applied)

- **Document** (`crates/node_graph_types`, v3): one flat `nodes` list +
  `edges`; `regions: BTreeMap<owner node id, GraphRegion{nodes, edges}>`
  — regions **cannot nest**. Regions are *optional*: a leaf state has
  none; states with blend trees and transitions with rules own one.
  Migrations are additive stamps (`io.rs::migrate_container`); the
  animation side additionally runs `upgrade_any_state` on load at three
  sites: compiler (`plan.rs` ~742), runtime loader (`runner.rs` ~579/664)
  and editor (`graph_editor.rs` ~2600).
- **Top-level canvas** = machine nodes (`anim_entry`, `anim_state`,
  `anim_state_alias`, `anim_transition`) **plus** pin-less `anim_play_once`
  and `anim_ik_chain` nodes. `compile_doc` requires ≥ 1 state and exactly
  one ENTRY (`plan.rs` ~843). Slots: host by node id, then nested in state
  order (`~1028`); IK: host + nested merged, then **globally** sorted by
  node id (`~1188`).
- **Plan**: `AnimGraphPlan { states, transitions, entry, parameters,
  slots, ik_chains }`; `PoseSource::{Tree(PlanTree), Machine{plan}}`;
  `PlanClip { clip, clip_name }` — no speed, no clock (a state supplies
  both). Compile errors are `Result<_, String>`; there are no warnings.
- **Runtime order** (`runner.rs::tick_entity` ~1100): `machine.tick` →
  `slot.tick` → `collect_anim_events` (clears `out`, scales every base
  event by `1 − slot.weight`, appends the slot's own events) → eval gate →
  `evaluate_pose` (restarts `PoseScratch` at level 0) → `slot.apply`
  (also level 0) → `compute_model_space` → `apply_ik` → palette. Nested
  machines are `AnimMachine.subs[state]` with reset-on-enter semantics
  (`machine.rs` ~274). Parallel over entities, throttled; serial steps
  2.5/2.6 write `resolved` / `lock_model`; workers never touch
  `Resources`.
- **`rt.machine` consumers outside the runner**: `foot_placement.rs`
  (`lock_state` = inline machine's current state), `anim_preview.rs`,
  `app.rs` ~8955 (`Mirror { machine: rt.machine.clone() }` for the
  preview panel), acceptance tests. `anim_graph_preview.rs` evaluates its
  **own** machine (or the mirror), not the entity runtime.
- **Editor**: one `GraphEditorState` per document = the machine canvas.
  `is_animation()` gates 9 sites in `graph_editor.rs` and 16 in
  `graph_editor_crusty.rs` (e.g. connection ghost geometry at ~3944 treats
  every animation document as a machine). `RuleScope` projects a region
  into a child editor with drain-back. Paste refuses a fragment carrying
  foreign types **whole** (~5241). `anchor_anim_refusal` has no IK arm.
  Dock layout profiles are keyed by document kind (`dock_crusty.rs` ~149).
- **Fixtures**: `character.animgraph` (bench + net rig; no slots/IK),
  `locomotion_demo.animgraph` (two IK chains), `defeated.animgraph`. None
  nests or has slots — parity fixtures for those must be synthetic.

## 3. Design

### D1 — Pipeline nodes live in the same flat document, partitioned by type

No container change beyond a version stamp (v4). The top-level `nodes`
list holds two families:

| Family | Types | Canvas |
|---|---|---|
| machine | `anim_entry`, `anim_state`, `anim_state_alias`, `anim_transition` | Machine scope (today's canvas) |
| pipeline | `anim_pipe_machine`, `anim_clip` (as a source), `anim_pipe_layer`, `anim_play_once`, `anim_ik_chain`, `anim_pipe_output` | Pipeline scope (the new root) |

Edges partition with their endpoints (`anim_flow` = machine, `anim_pose`
= pipeline); an edge whose endpoints straddle families is a compile
refusal and cannot be authored (pin domains differ). Regions keep their
owner-id keying. Comments and groups gain a `family` tag derived on
upgrade from what they enclose (default: machine); new ones tag by the
scope they are created in.

### D2 — Pipeline node set (registry `anim_pipeline_registry`)

All pipeline nodes carry `anim_pose` pins (rose, keyed domain).

- **State Machine** `anim_pipe_machine` — out: Pose. Prop `graph: Asset`
  (empty = *this document's* inline machine; a path = a nested
  `.animgraph`'s machine, evaluated as its own `AnimMachine` instance).
  At most one inline node. Double-click (inline) enters the machine scope;
  (nested) opens that file in a tab.
- **Clip** `anim_clip` — out: Pose. A static pose source at the root:
  loops, `speed` prop, own clock and event track (**D4: root clips get a
  `RootClipClock`** — the plan carries `PlanRootClip { clip: PlanClip,
  speed, node_id }`, the runtime a per-node clock; events fire like a
  state's). Blend1D/2D/blend-space nodes are not admitted at the root in
  v1.
- **Layered Blend Per Bone** `anim_pipe_layer` — in: Base, Layer; out:
  Pose. Props: `bones: Str` (mask roots, comma list), `weight_param: Str`
  (declared Float), `include_root: Bool` (default true). Evaluation order
  is **Base then Layer** (fixed, published), which is also the arbitration
  order for slots on sibling branches.
- **Play Once** `anim_play_once` — in: Pose; out: Pose. Existing props +
  `bones: Str` (optional mask; empty = whole body). One overlay channel
  in v1; **wire order replaces node-id order** for "first triggered wins"
  (the depth-first Base-before-Layer walk from Output defines the order;
  the header chip shows it). A node applies the channel's clip only if
  *it* owns the playing slot; otherwise it passes the pose through.
- **IK Chain** `anim_ik_chain` — in: Pose; out: Pose. Model-space stage;
  wire order replaces node-id order. Foot config unchanged.
- **Output Pose** `anim_pipe_output` — in: Pose. Exactly one.

### D3 — Compile rules

The compiler grows a **warnings channel**: `compile_anim_graph_with`
returns `Result<Compiled { plan, warnings: Vec<Anchored> }, String>`
(the editor renders warnings with the existing anchored style; the
runtime prints them once at arm). Machine compilation and pipeline
compilation are separate functions; a document with zero machine nodes
skips the machine compiler entirely (pure clip/layer documents are legal
— the "≥ 1 state + ENTRY" rule applies only when machine nodes exist).

Refusals (anchored on the named node):
1. Exactly one Output Pose, input wired.
2. Every node reachable from Output has all pose inputs wired; a pose
   cycle refuses (`"the pipeline loops through 'X'"`).
3. Stage order on every root→Output path: `[sources] → [Layer | Play Once]*
   → [IK Chain]* → Output`; a local-space node downstream of an IK Chain
   refuses (`"'X' must come before the first IK Chain — IK works in model
   space"`).
4. Fan-out refuses; fan-in only through a Layer's two inputs; pin
   cardinality and domain checked (no cross-family edges).
5. If the document has machine nodes, an inline State Machine node must
   exist **and be reachable** from Output; at most one inline node.
6. Layer/Play Once `bones` roots and `weight_param` validated like IK
   (param declared Float at compile; bones resolve at arm, refusal anchored
   on the node). Empty `bones` on a Layer refuses (a Layer with no mask is
   a bug, not a whole-body blend); empty on Play Once = whole body.
7. **Nested documents** (`anim_pipe_machine.graph`, or a state's `graph`
   prop): only their *machine* is consumed; their pipeline is **not
   compiled or validated** (an ignored child pipeline must not fail the
   host). Their slots / IK chains are lifted into the host exactly as
   today, deduplicated, with provenance, and a **warning** naming them.
   Ordering: host wired slots/chains first, then lifted ones in the
   legacy order (host-first then nested in state order for slots; node-id
   order for lifted chains). Newly authored nested references should not
   rely on lifting (documented; forbidding is deferred).

Warnings: unreachable pipeline node; lifted nested slots/chains; a
Layer whose mask covers no bone on the armed skeleton (arm-time,
runtime-printed).

### D4 — Plan and runtime

```rust
pub struct PlanMachineRef { pub node_id: u64, pub source: MachineSource }
pub enum MachineSource { Inline, Nested { graph: String, plan: Arc<AnimGraphPlan> } }
pub enum PlanPose {
    Machine(usize),                                   // index into plan.machines
    Clip(usize),                                      // index into plan.root_clips
    Layer { base: Box<PlanPose>, layer: Box<PlanPose>, mask: PlanMask, weight_param: String, node_id: u64 },
    Overlay { input: Box<PlanPose>, slot: usize, mask: Option<PlanMask>, node_id: u64 }, // one slot per node
}
pub struct PlanMask { pub roots: Vec<String>, pub include_root: bool }      // symbolic; resolved at arm
pub struct PlanPipeline { pub root: PlanPose, pub slot_order: Vec<usize>, pub ik_order: Vec<usize> }
```

`AnimGraphPlan` gains `machines: Vec<PlanMachineRef>`,
`inline_machine: Option<usize>`, `root_clips: Vec<PlanRootClip>`,
`pipeline: PlanPipeline`. `states/transitions/entry` stay the inline
machine's (empty when there is none). `slots` and `ik_chains` keep their
Vecs; `slot_order` / `ik_order` are the wire orders (lifted entries
appended). Masks stay symbolic in the immutable plan; the runtime holds
`masks: BTreeMap<node_id, Vec<f32>>` resolved per skeleton at arm (serial,
with `arm_ik_chains`).

Runtime: `AnimGraphRuntime.machine` **stays** as the inline machine (no
accessor churn: foot lock `lock_state`, the preview mirror, editor viz
and the acceptance tests keep compiling); a new `extra_machines:
Vec<AnimMachine>` holds nested pipeline machines index-aligned with
`plan.machines` minus the inline one. `AnimMachine.subs` (state-nested
machines) are untouched. `root_clocks: Vec<RootClipClock>`.

`tick_entity`: tick inline + extra machines + root clocks, then the slot
(consumption order: inline machine's transitions first, then extra
machines in `plan.machines` order, then the slot — published); forced-eval
sources aggregate across all machines; **events**: `collect_anim_events`
is refactored to append (no clear) per source, and the slot's own events
are emitted once; the `1 − slot.weight` suppression of base events
becomes **per-bone-mask-aware**: a masked overlay suppresses base events
only by `weight × (mask covers the event's owner)`. v1 rule: base events
carry no bone, so a masked overlay (`bones` non-empty) does **not**
suppress base events at all; a whole-body overlay suppresses as today.
This keeps footstep / foot-lock events alive under an upper-body cast
(the riskiest item, per round 1 — pinned by an acceptance test: locked
foot survives a masked cast, releases under a whole-body one).

`evaluate_pipeline(root, level)` recurses with an explicit `PoseScratch`
level; `evaluate_pose` and `PlayOnceSlot::apply` gain a `level`
parameter (today both hard-code 0) so nested evaluation never aliases a
buffer. Layer: evaluate base into `level`, layer into `level+1`, blend in
place with `LocalBoneTransform::blend` weighted by `mask[b] × w` (w
clamped to `[0,1]`, non-finite → 0). Eval gate, `compute_model_space`,
`apply_ik` over `ik_order`, palette, throttling, parallelism, serial
resolution: unchanged. Skipped (throttled) entities keep transforms,
palette and revision exactly as today.

Parity: a migrated v3 document evaluates **bit-identically**: golden
tests on the demo graph (two chains) and on synthetic fixtures with
slots and a nested machine carrying a chain (legacy ordering preserved by
D3.7 + D5).

### D5 — Migration (document v3 → v4)

`migrate_container` 3→4 = stamp. The animation-side upgrade
(`upgrade_pipeline_root`, run beside `upgrade_any_state` at all three
load sites) triggers only on a document that has machine nodes and **no**
`anim_pipe_output` **and** `version < 4` (so a v4 document whose author
deleted Output is a refusal, not a silent re-upgrade):

1. Add `anim_pipe_machine` (inline) at the left, `anim_pipe_output` at
   the right.
2. Chain existing `anim_play_once` nodes in node-id order, then
   `anim_ik_chain` nodes in node-id order, then Output. Their canvas
   positions are re-laid out on a row (old positions were machine-canvas
   coordinates, meaningless here); machine node positions, regions,
   variables, ids untouched. Comments/groups tagged `machine`.
3. Editor marks the document dirty with a console line ("upgraded to
   pipeline root"); runtime loaders upgrade in memory only.

Nested-effect ordering after migration = D3.7's legacy order, so the
golden tests hold for nested fixtures too.

### D6 — Editor

- **One `GraphEditorState`, two scopes** via `CanvasScope::{Pipeline,
  Machine}` on animation documents. Centralised: a single
  `visible(node) -> bool` predicate + `visible_nodes()` /
  `visible_edges()` queries used by draw, hit-test, box select, paste
  target, auto-layout, frame-all; the existing `is_animation()` gates
  become `is_machine_scope()` where they mean "machine geometry"
  (connection ghost, state-border wires) and stay `is_animation()` where
  they mean "animation domain" (layout profile, panels). Each scope keeps
  its own pan/zoom; selection clears on switch; rule scopes are settled
  (`close_rule_scope`) and gestures cancelled before switching.
- **Root = Pipeline.** Breadcrumb `‹graph›`; double-click / Enter on the
  inline State Machine node → `‹graph› › State Machine`; PageUp /
  breadcrumb click returns. A nested State Machine node opens the file
  (existing open-request path). `RuleScope` works from the Machine scope
  only.
- **Palette**: Pipeline scope = `anim_pipeline_registry` (+ `anim_clip`);
  Machine scope = the machine subset of `anim_node_registry` (slot / IK
  descriptors move to the pipeline registry). **Paste**: a fragment is
  split by family; the foreign part is dropped with a console line (the
  current refuse-whole rule is relaxed for this one case so copying a
  mixed selection across scopes is not a dead end).
- **Header chips**: Play Once / IK Chain show their applied order; Layer
  shows its root count.
- **Details**: Layer (Bones text, Weight dropdown from `doc.variables`,
  Include Root), State Machine (Graph picker; "inline" when empty; the
  ENTRY preview-mesh property is mirrored here as a second access point —
  a pure-clip document with no ENTRY stores it on the Output node
  instead), Output (preview mesh when no machine), Play Once (+ Bones).
- **Diagnostics**: `anchor_anim_refusal` gains arms for every pipeline
  type (incl. the missing IK arm); anchored *warnings* render dimmed; F8
  / error click switches scope to the anchored node's family.
- **Preview**: `anim_graph_preview` builds its own machine from the plan
  — it must build the **pipeline** (extra machines, root clocks, masks)
  via the same `evaluate_pipeline`, factored so the panel and the runtime
  share one evaluator; the `Mirror` gains `extra_machines` +
  `root_clocks`. Blend-space preview constructs plan literals
  (`blend_space_preview.rs` ~325) — updated to the new fields.
  Thumbnails: unaffected (mesh-based).
- **New-document template**: `SM → Output` pre-placed; ENTRY + one state
  in the machine scope as today.
- **Serialization**: new props round-trip through the existing `PropValue`
  path (tests for defaults + custom values on every new node type).

### D7 — Layered blend semantics (v1)

Local-space per-bone: translation/scale lerp, rotation slerp, weight =
`mask[b] × clamp(weight_param, 0, 1)` (non-finite → 0). Mask = 1 on
listed roots (unless `include_root: false`) and every descendant, 0
elsewhere; overlapping roots union; a root missing on the armed skeleton
refuses at arm. A layer clip missing channels for a masked bone keeps
the base for that bone (the sampler leaves unsampled bones at their
pre-sample value, which is the base pose buffer copy — the same
agreement every blend in `machine.rs` keeps). Known limitation: local-space
layering of the spine inherits the base's pelvis/spine orientation;
mesh-space is deferred until real casts show it is needed (Astra R4).

## 4. Work packages (one commit each, serial; each must compile and pass alone)

| P | Scope | Files |
|---|---|---|
| P0 | This plan; Astra rounds; user grilling; rulings into `.scratch/pipeline/spec.md` | docs |
| P1 | **Schema + compiler**: node type ids/descriptors, `anim_pipeline_registry`, warnings channel, `PlanPipeline`/`PlanMask`/`PlanMachineRef`/`PlanRootClip`, separate machine vs pipeline compile, D3 refusals/warnings, `upgrade_pipeline_root` at the three load sites + v4 stamp, prop round-trip tests, fixture tests (three shipped graphs + synthetic slot/nested fixtures upgrade and compile; every refusal) | `plan.rs`, new `pipeline.rs`, `library.rs`, `node_graph_types` stamp |
| P2 | **Compatible runtime**: `extra_machines`, `root_clocks`, masks armed serially, `evaluate_pipeline` with explicit scratch levels (+ `level` on `evaluate_pose` / `slot.apply`), `slot_order`/`ik_order`, event collection refactor (append, slot once, mask-aware suppression), forced-eval aggregation; **golden parity tests** (demo graph + synthetic slot/nested fixtures, 120 ticks, palette equality); crowd bench re-run on the migrated `character.animgraph` | `runner.rs`, `machine.rs`, `acceptance.rs` |
| P3 | **Layering + events acceptance**: Layer/masked Play Once evaluation tests (masked bones follow the layer, others the base; masked overlay keeps base events + foot lock; whole-body overlay suppresses), root-clip clock/events, preview evaluator factored to share `evaluate_pipeline` (`anim_graph_preview`, `Mirror`, blend-space preview literals) | `runner.rs`, `anim_graph_preview.rs`, `app.rs`, `blend_space_preview.rs` |
| P4 | **Editor scopes + navigation**: `CanvasScope`, centralised visibility predicate, geometry gates, palette gating, breadcrumb + double-click + PageUp, dirty-on-upgrade line, split-paste | `graph_editor.rs`, `graph_editor_crusty.rs` |
| P5 | **Editor authoring**: Details rows, header chips, refusal/warning anchoring for all pipeline types + IK, F8 scope switch, new-document template, comment/group family tags | same + `anim_node_registry`, `dialogs` |
| P6 | **Demo**: `locomotion_demo.animgraph` gains a Layer (upper-body clip from the user, or `Idle_1` masked to `mixamorig:Spine` as a stand-in) on a new `aim` Float + a masked Play Once cast (if a clip is provided); `character.animgraph` migrates to `SM → Output` | content |
| P7 | **Close-out**: ARCHITECTURE ▸ Animation Pipeline (replaces "IK chains are standalone nodes"), KNOWLEDGE gotchas, ROADMAP ledger, CLAUDE.md, Opus review of P1–P5, user live verification | docs |

Riskiest item (round 1's verdict, accepted): **masked-event semantics
coupled to foot locking** — pinned first by P3's acceptance tests before
any editor work.

## 5. Acceptance

- Opening any shipped animgraph shows `State Machine → … → Output Pose`;
  double-click enters the machine, breadcrumb returns; saving writes v4;
  reopening is stable (no re-upgrade, no diff churn).
- Golden parity: migrated documents animate bit-identically (demo +
  synthetic slot/nested fixtures); the demo character looks the same live.
- A Layer with `Idle_1` masked to the spine at `aim = 1` holds the
  layered upper body while the legs walk; `aim = 0` restores the base. A
  masked Play Once plays on the arms while walking, and the walk's foot
  locks keep working under it.
- Every D3 refusal anchors on its node in the pipeline scope; IK refusals
  anchor (new); warnings render.
- `--stress-anim 300` within 5 % of the 41.5 numbers on the migrated
  bench graph.
- Engine + client tests green; both hosts launch with a clean schedule.

## 6. Resolved questions (round 1)

- **R1** scope-as-filter, with the visibility predicate centralised (not
  30 independent filters); settle rule scopes before switching.
- **R2** keep lifting with provenance, dedup, explicit legacy order;
  documented as compat; forbidding deferred.
- **R3** chained Play Once nodes, one slot per compiled Overlay node,
  published Base-before-Layer arbitration order.
- **R4** local-space only in v1.
- **R5** one inline machine; nested machines are files; parameter/trigger
  consumption order published (inline → extra → slot).
- **R6** root clips only, with clock/speed/events/arming.
- **R7** re-lay-out migrated pipeline nodes; machine positions,
  comments/groups preserved and tagged.

## 7. Open for round 2 / the user

- **Q1** The masked-overlay event rule (masked overlay never suppresses
  base events; whole-body does): acceptable v1 semantics, or should base
  events carry an owning bone so masks can suppress precisely?
- **Q2** `anim_clip` at the root owns a clock + events: should it also
  expose `loop: Bool` (default true) now?
- **Q3** Pure-clip documents (no machine): worth supporting in v1 at all,
  or refuse "an animation graph needs a State Machine" and drop the
  `inline_machine: Option` complexity?
