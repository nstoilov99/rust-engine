# Task 45-A — Graph Execution Core (Visual Scripting v1)

**Status:** ✅ **Complete** (2026-08-11, P1–P9 — commit map and deferred
ledger in [`ROADMAP.md`](ROADMAP.md) ▸ Task 45-A). Drafted 2026-07-27;
audited pre-kickoff 2026-08-11 (see the addendum at the end, whose rulings
amend the sections they name).
**Duration:** ~2–2.5 weeks
**Prerequisites:** Task 40 (complete). Pulls the core of roadmap Task 45
forward, per decision 2026-07-27; the rest of Task 45 (full gameplay node
library, polish) remains where it is.
**Decisions locked at planning:** gameplay scripting is the first executing
domain; client-first execution with an M6-style portable core (SpacetimeDB
server execution later via a Task 39.8 plugin); full latent support in v1
(Delay + Timeline with curves).

## Goal

Graphs run. A `.graph` asset attached to an entity executes Blueprint-style:
event entry points, exec-flow control (Branch, loops, Sequence, Gate…),
lazily-pulled pure data chains, per-graph variables, latent nodes that
suspend across frames, and a starter gameplay node set — with execution
visible in the graph editor. The interpreter core is engine-independent and
deterministic-capable so the same code can later compile into the
SpacetimeDB module unchanged.

## Reference semantics (adopted from Blueprint, adapted)

- Exec wires carry control; an impure node fires, pulls its data inputs
  backward through pure chains, performs its effect, then names the exec
  output(s) to continue on. Pure nodes are re-evaluated per impure-node
  firing (no cross-statement caching in v1 — correctness first).
- Control flow = impure nodes with multiple exec outputs choosing/repeating
  continuations (Branch, Sequence, ForLoop, ForEach, WhileLoop, Gate,
  DoOnce, FlipFlop). No Blender-style zones — wrong idiom for scripting
  (zones stay a candidate for the material/geometry domains later).
- Iteration budget per tick (default ~100k node firings) — a cycle in exec
  wires or a runaway loop stops the instance with a reported error, never
  hangs the engine.
- Latent nodes suspend an execution thread; the runner resumes it in a later
  tick. Latent state lives in the instance, is serializable, and is keyed by
  node id.

## Design

### D1. Portable interpreter core — a real crate, real threads, real
memoization

**Location: a new workspace crate `crates/node_graph_exec`**, depended on by
the engine — *not* a module inside it. That's how M6 actually achieves
native/WASM parity (`game_shared` is an independent crate both sides
depend on); a cfg test inside the engine crate proves nothing about the
server module's ability to consume it. The crate depends only on the doc
types (either re-exported plain data or a small shared types crate — decide
at P1; no ECS, no Vulkan, no wall-clock).

- `NodeImpl` contract: pure — `eval(inputs) -> outputs`; impure —
  `fire(ctx) -> FireResult` (`ctx`: pulled inputs, variable get/set, effect
  sink, injected time/RNG, latent scheduling). `FireResult`: continue on
  exec pin(s), loop protocol (interpreter-owned frames, impls stateless),
  suspend, stop.
- **Execution threads, not a single stack**: each event activation spawns a
  thread — continuation point, call/loop frame stack, locals, and a unique
  activation id. Latent state is keyed by **activation id** (a Delay inside
  a ForLoop, or two concurrent activations of the same Delay node, each
  suspend independently). `GraphInstance` = variables + live threads +
  queued events + RNG state — all plain serializable data.
- **Pure evaluation semantics** (spelled out so nobody guesses):
  memoization is *statement-scoped* — during one impure-node firing, each
  pure node evaluates at most once (memo keyed by node id, cleared at the
  next firing). Pure evaluations count against the tick budget. **Data-edge
  cycles are rejected at validation** (new `validate_doc` rule: the
  data-pin subgraph must be a DAG; exec edges may loop). Nodes that read
  instance RNG (Random*) carry `deterministic: false` and are **exempt from
  memoization** (volatile) while still being replay-deterministic given the
  seed.
- `EffectSink` trait: every world-touching action is an emitted effect
  (`SpawnPrefab`, `SetTransform`, `EmitEvent`, `Log`, …); the core never
  mutates anything else. Spawn effects return an **instance-local alias id**
  the graph can hold in variables; the runner resolves aliases to real
  entities after application (real ids never enter the portable core).
- `WorldRead` trait for snapshot reads; injected `dt`/time.
- Compilation at instance spawn: doc → flattened plan (subgraphs inlined —
  see D3's interface-binding nodes, which make that possible — unreachable
  nodes pruned, per-input source table). Cached per (asset, version); this
  is the compile utility later consumer tasks reuse.

### D2. Type-system additions (framework, with migrations where needed)

- `PinType::Int` + `PropValue::Int(i64)`; `PinType::String` +
  `PropValue::Str`. Loop indices, names, logging.
- `PinType::Array(Box<PinType>)` + `PropValue::Array(Vec<PropValue>)` —
  ForEach works; array *literal editing* in the UI is deferred (arrays flow
  through wires and variables in v1).
- Enum pins gain declared variants on `PinDescriptor` (needed by the
  property editor, D6).

### D3. Variables, events, and document-dependent node types

- `variables: Vec<VarDecl { slug, label, ty, default }>` — a **container
  v1→v2 bump with a real migration** implemented in `parse_graph`'s
  envelope pass (which is currently only a placeholder seam — this is the
  first real container migration, and it retires that placeholder with a
  golden fixture).
- **`DocDescriptors` — a descriptor resolver layered over the registry.**
  Today `validate_doc` special-cases exactly one document-dependent type
  (`subgraph`); this task adds three more (variable get/set, interface
  binding, events with payloads), so the special-casing generalizes: one
  resolver answers "what are this node instance's pins" from
  (registry | doc context), and **validation, the editor, compilation, and
  migration all consume it** instead of raw registry lookups. This lands
  first (P1) because everything else sits on it.
- **Subgraph interface binding**: `IfacePin` only declares the *external*
  interface — nothing in the schema connects those pins to the subgraph's
  internals, so inlining is currently impossible. Fix: reserved
  `graph_input` / `graph_output` nodes *inside* subgraph docs (Blender's
  Group Input/Output pattern) whose pins mirror the declared interface;
  compilation splices host edges through them. Validation: interface pins
  unbound by any `graph_input`/`graph_output` node get a warning-level
  error; the editor auto-inserts the pair when creating a `.subgraph`.
- Event entry nodes: `event_begin_play`, `event_tick` (dt output),
  `event_custom { name, payload pins }`, `event_input_action { action }`
  (Task 33 bridge). Impure, exec-outputs-only; indexed as entry points at
  compile time. **Delivery semantics (fixed, not implementation-defined):**
  per-instance FIFO queue; emitting during a tick queues for the *next*
  tick (no reentrancy); within one tick the drain order is BeginPlay
  (exactly once per instance lifetime) → due latents (by due time, then
  activation id) → input-action events (input order) → custom events
  (FIFO) → Tick. Multiple entry nodes for the same event all fire, in doc
  order. Custom events are same-entity scoped in v1.

### D4. Runtime binding (client)

- **Split component**: `GraphRunner` (serialized config: asset path,
  enabled) vs. runtime-only state (`GraphInstance` + compiled-plan handle)
  in a separate non-serialized component attached at play time. Scene
  persistence is the closed `ComponentData` enum with explicit
  serializer/deserializer arms — `GraphRunner` gets its arms added there
  (there is no "like any component" free ride), runtime state is never
  serialized.
- **Effect application** (the seam the plan must not hand-wave): the runner
  system collects each instance's effect stream, then applies —
  non-structural effects (SetTransform etc.) directly, since the system
  declares `writes::<Transform>()` via `SystemDescriptor`; structural
  effects (spawn/destroy) through `world.commands()` closures
  (`EcsCommand::Custom` — the buffer is closure-based, not command-enum-
  based, and is applied by the scheduler between stages, which also fixes
  ordering). Spawn alias ids (D1) resolve when the spawn closure runs and
  are written back into the instance's alias map before its next tick.
- **Enter-play lifecycle** (Task 24 transition hooks): on play enter, all
  runtime-state components are (re)created fresh and BeginPlay is armed;
  on stop, the play-mode world teardown drops them with the world — restart
  therefore re-fires BeginPlay exactly once per entry, including for
  entities spawned *during* play (armed on instance creation, not on
  play-enter only).
- Realm gate at instance creation: client refuses graphs whose realm the
  client side doesn't admit (console error, instance disabled).
- Runners execute only in play mode (RunIfPlaying).

### D5. Starter node library (framework-owned, `std_nodes` module)

Control: Branch, Sequence(2–4), ForLoop, ForEachElement, WhileLoop, Gate,
DoOnce, FlipFlop, Select. Latent: Delay(seconds), Timeline (see D7).
Logic/math: And/Or/Not, comparisons (Int/Float), Add/Sub/Mul/Div (Int and
Float variants — no polymorphic pins in v1), Lerp, Clamp, RandomFloat/Int
(seeded from instance RNG). Data: Get/Set variable (synthesized),
MakeVec3/BreakVec3, IntToFloat/FloatToInt/ToString. Effects: PrintLog
(console + on-screen), EmitEvent(name), SpawnPrefab(path, transform),
DestroyEntity, GetEntityTransform/SetEntityTransform, GetSelf. That's the
demonstrably-useful floor; the broader gameplay API stays Task 45's tail.

### D6. Editor: make authoring actually possible

- **Inline property editing** (closes the known Task 40 gap): per-type field
  widgets on unconnected inputs (DragValue for Float/Int, checkbox, text,
  enum dropdown from D2 variants, asset field), `SetProperty` undo entries.
- **Variables panel**: side strip on the graph tab — add/rename/retype/
  default-edit variables (undoable), drag onto canvas → Get/Set choice.
- **Wire-drag create menu**: release a dragged wire on empty canvas → create
  menu filtered to compatible pins, auto-connect on pick.
- **Execution visualization**: the runner records fired wires + last values
  per instance (ring buffer, editor builds only); with a running instance
  selected, wires pulse and pin hover shows last value. Print output goes to
  the console panel tagged with the graph path.

### D7. Timeline node & curve asset (the "full latent" decision)

- `Timeline`: play/reverse/stop exec inputs, per-track float outputs, update
  + finished exec outputs; duration + looping flags; tracks reference a
  curve asset.
- `.curve` asset (new `AssetType::Curve`): named float tracks of keyframes
  with interpolation (constant/linear/cubic) — RON, same single-segment
  scheme. Minimal curve editor panel (keyframe add/drag/delete on a small
  plot) — deliberately basic; the anim task will grow it.

### D8. Portability & determinism discipline (the MP insurance)

- Portability is structural, not aspirational: `node_graph_exec` is its own
  workspace crate (D1) with no engine dependency — CI builds it standalone
  (and `--target wasm32-unknown-unknown` as a smoke check), the same way
  `game_shared` proves the M6 controller.
- All nondeterminism injected: time comes from the runner; RNG is a seeded
  per-instance PRNG; iteration order over instances is stable.
- A determinism test: same graph + same seed + same scripted inputs, two
  runs → identical effect streams. This is the future WASM parity suite's
  seed.
- Explicit non-goals now: variable replication, server event sources,
  prediction/rollback, running in the SpacetimeDB module (that lands as a
  39.8 plugin exercise later).

### D9. Non-goals

Polymorphic/wildcard pins (Int and Float ops are separate nodes in v1);
bytecode compilation; graph-to-Rust codegen; cross-graph function libraries
beyond subgraphs; array literal editors; debugging breakpoints (visualization
only in v1); Blender-style zones; hot-swapping a graph asset under running
instances (edit during play = instances restart on re-enter).

## Work packages (each = one reviewable commit)

- **P1 — contracts first** (D2, D3): pin/prop additions + enum variants;
  `DocDescriptors` resolver replacing the subgraph special case across
  validation/editor/migration; `graph_input`/`graph_output` binding nodes;
  VarDecl + the **real container v1→v2 migration** (retiring the envelope
  placeholder) with golden fixtures; data-cycle validation rule; event
  descriptors + delivery-order spec as doc'd constants.
- **P2 — interpreter walking skeleton** (D1): the `node_graph_exec` crate;
  NodeImpl contract + compile/flatten (incl. subgraph splicing) + thread
  model + budget + EffectSink/WorldRead — proven end-to-end with ~6 real
  nodes (Branch, ForLoop, Add, Print, variable get/set) **and a thin
  engine-side integration spike applying effects to a real world**, so the
  contract is validated against both consumers before it freezes.
  Determinism test lands here.
- **P3 — std node library** (D5 minus latent): the remaining ~25 nodes on
  the now-proven contract; per-node semantic tests.
- **P4 — latent machinery + Delay** (D1 latent half): suspension/resume on
  activation ids, Delay node, tests (suspend across ticks, Delay-inside-
  ForLoop, two concurrent activations, instance serialization mid-wait).
- **P5 — runtime binding** (D4): GraphRunner component + `ComponentData`
  serializer arms, runtime-state component, runner system with the
  documented effect-application split, alias resolution, realm gate,
  play-mode transition hooks (BeginPlay exactly-once incl. play-spawned
  entities); scene round-trip test; demo content update.
- **P6 — editor authoring** (D6 first half): inline property editing +
  variables panel (+ enum dropdowns).
- **P7 — editor flow + viz** (D6 second half): wire-drag create menu,
  execution pulse + value hover, console-tagged Print.
- **P8 — Timeline + curves** (D7): curve asset + minimal editor + Timeline
  node.
- **P9 — close-out**: docs (ARCHITECTURE/KNOWLEDGE), roadmap, review pass,
  demo graph showcasing control flow + latent + variables.

## Acceptance

- Headless: a fixture graph using Branch + ForLoop + variables + subgraph
  produces the expected effect stream; determinism test green; budget test
  kills an infinite WhileLoop with a reported error.
- In engine: demo graph on an entity — BeginPlay spawns prefabs in a ForLoop,
  Tick moves one via Timeline, Delay chains fire — all visible via execution
  pulse; play → stop → play restarts cleanly; editor can author all of it
  (typed constants, variables, wire-drag create) without touching RON.
- Realm gate: a `Server`-realm graph on a client runner errors visibly and
  doesn't run.
- All Task 40 gates hold (tests, clippy, color-literal grep, both builds).

## Risks & mitigations

- **Latent + loops interacting** (Delay inside ForLoop body): the loop stack
  must serialize with the suspension. Mitigated: it's one data structure
  (the thread state) by design in D1; P4 tests exactly this case.
- **NodeImpl contract churn once real nodes exist**: P3 immediately stress-
  tests P2's contract with ~30 nodes before any UI depends on it.
- **Editor scope creep** (curve editor, variables panel): both specced
  minimal; P8 is the designated cut/slip package (Delay alone satisfies
  "latent works"; Timeline can trail).
- **Determinism erosion by future nodes**: the effect-stream determinism
  test runs in CI from P2 on; nodes that break it fail loudly.

## Open questions — RESOLVED (user decision 2026-08-11, per recommendation)

1. Effect granularity: **fine-grained** (SetTransform etc.), as planned.
2. `Int` width: **i32** — SpacetimeDB-friendlier, and graphs don't need
   64-bit loop counters. `PropValue::Int(i32)`; amends D2's i64.
3. `event_custom` scope: **same-entity in v1**, explicit target pin later.
4. Timeline curves: **shared `.curve` asset from day one** — the animation
   task (41) grows the same asset/editor rather than forking one.

## Audit addendum (2026-08-11, Claude + Codex, pre-kickoff)

The plan predates Task 39.8, the node-graph design-system arc, the input
model, and Refactor Checkpoint #6. Audited against the current tree; twelve
findings, all folded here. These rulings amend the sections they name.

**Corrections (discrepancies/stale):**
1. **D3 — `DocDescriptors` scope grows:** validation now special-cases *two*
   doc-dependent types (subgraph at `validate.rs:213` AND dynamically-typed
   reroutes at `validate.rs:236`), plus the cross-asset resolver
   (`resolver.rs`). The resolver must absorb all three, not just subgraph.
2. **D4 — rewrite the lifecycle around what exists now:** enter-play
   snapshots, flips mode, conditionally rebuilds physics (gated on the
   Rapier plugin), and emits a `PlayModeChanged` event; stop restores the
   snapshot (physics-free signature) then runs the world-population hooks.
   "Task 24 transition hooks" as written no longer describes reality.
3. **D4 — BeginPlay arming must NOT ride `on_world_loaded`:** play-enter is
   deliberately not a population moment, and population hooks also run
   while editing. Ruling: **lazy runtime-state creation by the runner
   system itself** (`RunIfPlaying`): any `GraphRunner` entity without a
   `GraphInstance` gets one created + BeginPlay armed on first playing
   tick — covers initial entities and play-spawned ones uniformly, and
   snapshot restore deletes non-serialized state so re-entry re-arms
   naturally. No lifecycle hook needed at all.
4. **P6/P7 — already partially shipped by the design-system arc:** inline
   property editing (Float/Bool/Color/Enum, coalesced `SetProperty`) and
   wire-drop-on-empty-canvas → filtered palette + auto-connect both exist
   (`graph_editor_crusty.rs:4931`, `:2050`). Retarget P6 to: Int/String
   field widgets + the variables panel; P7 to execution viz only.
5. **Roadmap Task 45 contradicts this plan** (`ScriptComponent`,
   world-mutating `NodeContext`): 45-A is authoritative; Task 45 becomes
   the node-library/gameplay-API/debugging tail on 45-A's runtime.
   (Roadmap section annotated.)

**Additions (gaps the landed systems create):**
6. **Graph execution ships as an `EnginePlugin`** — `"graph_scripting"`,
   `PluginOrigin::Engine`, `PluginKind::Runtime`, staging `std_nodes`
   registrations + the runner system atomically (Rapier pattern). Unlike
   Rapier it gets a **Cargo feature (default-on)**: `node_graph_exec` is a
   leaf crate, so the gate is clean and a disabled plugin genuinely strips
   from exports — the first plugin to exercise the full D9 export policy.
7. **P1 — new pin types cross the design system:** `pin_color`/`wire_color`
   in `theme/tokens.rs:224` map the current families exhaustively — add
   deliberate Int/String/Array entries + resolver tests + design-lint
   coverage, and the exhaustive `PropValue` presentation matches in
   `graph_editor_crusty.rs` (~:464).
8. **P6/P7/P8 — keymap registration:** new editor actions (variables panel,
   curve editor) register in `keymap.rs`'s action table with contexts —
   wire-drag-create is a pointer gesture, no action needed.
9. **P5 — persistence is three seams, not one:** `ComponentData` arms
   (`scene_format.rs:728`) plus scene_serializer extraction/insertion
   (`scene_serializer.rs:37`/`:447`) plus prefab loading (`prefab.rs:82`).
10. **P8 — `.curve` touches every closed `AssetType` table**
    (`asset_type.rs`: variant, `from_extension`, `display_name`,
    `extensions`, `all`).

**Ownership split vs Task 45.5 (double-booking resolved):**
11. 45-A P7 ships the *trace data* + basic pulse/value hover; 45.5 owns
    flow-bubble rendering, watches, and persisted breakpoints. 45-A P1
    ships `graph_input`/`graph_output` + compiler splicing (and the
    `NodeInst.title` schema field, cheap while the container migrates);
    45.5 owns collapse-to-subgraph and rename UI.
12. Verified still true (no change needed): the io.rs envelope placeholder,
    closure-based command buffer flushed between stages,
    `SystemDescriptor::writes::<T>()`, `ComponentData` closed enum.
