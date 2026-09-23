# Rust Game Engine - Knowledge Base

This document contains conventions, patterns, common gotchas, and tribal knowledge for working with the engine.

## Coordinate System (Critical)

### The Golden Rule

**All game logic uses Z-up. Never mix coordinate systems.**

```
Game World (Z-up)              Vulkan Render (Y-up)
      Z (Up/Blue)                    Y (Up)
      |                              |
      |                              |
      +------ Y (Right/Green)        +------ X (Right)
     /                              /
    X (Forward/Red)               -Z (Forward)
```

### When to Use Each Matrix Function

| Function | Use Case |
|----------|----------|
| `local_matrix_zup()` | Hierarchy composition, physics, game logic |
| `model_matrix()` | Simple entities without parents (rendering) |
| `world_matrix_to_render()` | Convert final world matrix for rendering |

### Common Mistakes

```rust
// WRONG: Mixing coordinate systems
let world = parent.model_matrix() * child.local_matrix_zup();

// CORRECT: Compose in Z-up, convert at the end
let world_zup = parent.local_matrix_zup() * child.local_matrix_zup();
let render_matrix = world_matrix_to_render(&world_zup);
```

## ECS Patterns

### Components Are Data Only

```rust
// GOOD: Plain data struct
pub struct Health {
    pub current: f32,
    pub max: f32,
}

// BAD: Logic in component
impl Health {
    pub fn take_damage(&mut self, amount: f32) { ... }  // Don't do this
}
```

### Systems Are Stateless Functions

```rust
// GOOD: System function
pub fn damage_system(world: &mut World, delta: f32) {
    for (id, (health, damage)) in world.query::<(&mut Health, &Damage)>().iter() {
        health.current -= damage.amount * delta;
    }
}

// ACCEPTABLE: System struct for configuration
pub struct DamageSystem {
    pub damage_multiplier: f32,
}

impl System for DamageSystem {
    fn update(&mut self, world: &mut World, delta: f32) {
        // Use self.damage_multiplier
    }
}
```

### Querying Patterns

```rust
// Single component
for (id, transform) in world.query::<&Transform>().iter() { }

// Multiple components
for (id, (transform, mesh)) in world.query::<(&Transform, &MeshRenderer)>().iter() { }

// Optional component
for (id, (transform, mesh_opt)) in world.query::<(&Transform, Option<&MeshRenderer>)>().iter() {
    if let Some(mesh) = mesh_opt { }
}

// Mutable access
for (id, transform) in world.query_mut::<&mut Transform>() { }

// Exclude component
for (id, transform) in world.query::<&Transform>()
    .without::<Static>()
    .iter() { }
```

### Hierarchy Traversal

```rust
// Get world transform (handles parent chain)
let world_transform = hierarchy::get_world_transform(world, entity);

// Iterate children
if let Ok(children) = world.get::<&Children>(parent) {
    for child in &children.0 {
        // Process child
    }
}

// Set parent
hierarchy::set_parent(world, child, Some(parent));
```

## Rendering Patterns

### Mesh Management

```rust
// Load mesh once, reuse index
let mesh_index = mesh_manager.add_mesh(gpu_mesh);

// Reference in component
entity.insert(MeshRenderer {
    mesh_index,
    material_index: 0,
});
```

### Material Setup

```rust
// Materials are indexed, not stored in components
let material = PbrMaterial {
    albedo: [1.0, 0.0, 0.0, 1.0],  // Red
    metallic: 0.0,
    roughness: 0.5,
    ..Default::default()
};
let material_index = material_manager.add(material);
```

### Light Direction Convention

```rust
// Direction points FROM light TO scene (like sun rays)
let sun = DirectionalLight {
    direction: glm::vec3(0.5, -0.5, -1.0).normalize(),  // Z-up space
    color: glm::vec3(1.0, 0.98, 0.95),
    intensity: 2.0,
};
```

## Editor Patterns

### Panel State

```rust
pub struct MyPanel {
    // Persistent UI state
    selected_index: usize,
    scroll_offset: f32,

    // NOT scene data - that goes in ECS
}

impl MyPanel {
    pub fn show(&mut self, ui: &mut egui::Ui, world: &mut World) {
        // Read/write ECS, update UI state
    }
}
```

### Selection System

```rust
// Selection is stored in EditorState, not components
if let Some(selected) = editor_state.selected_entity {
    if let Ok(transform) = world.get::<&Transform>(selected) {
        // Show inspector for selected entity
    }
}
```

### Viewport Input Priority

1. Gizmo interaction (highest)
2. Camera controls
3. Entity selection
4. Panel interaction (lowest when cursor in viewport)

### Layout profiles (per-document layouts)

The dock has one tree per *document kind* (`dock_crusty::LayoutProfile`: `Scene`, `AnimGraph`,
`ScriptGraph`, `BlendSpace`, `Curve`, `Mesh`); the focused document picks which one is live.
Gotchas:

- **The `documents` marker is never rendered.** A stored (inactive) profile tree holds the tab id
  `documents` where its document strip goes; `swap_profile` splices the real document tabs in
  there. Every default tree has exactly one marker (test-pinned); a stored tree that lost its
  marker falls back to `default_tree`. A marker that leaks into the live tree is stripped on load.
- **Gather rule.** Document tabs (`viewport:*`, `graph:*`, `curve:*`, `blendspace:*`, `mesh:*`,
  `ia:*`, `mc:*` — `is_document`) live in one leaf. Docking one elsewhere works until the next
  swap, which gathers every document back into the incoming profile's strip (strip order, then
  strays in traversal order). Side panels stay in whichever profile tree they were docked in.
- **`focused_document` is not the dock focus.** `DockState::focused_tab` is whatever was clicked
  last, side panels included. `App::focused_document` remembers the last focused *document* and
  only changes when another document takes focus or the current one closes. Profile swaps and the
  graph side panels (`focused_graph_key`) follow `focused_document`, so clicking Assets or Console
  never swaps the layout or blanks Details/Variables/Preview. Edit-action routing
  (`edit_target_document` → `active_graph_key`) is stricter: a scene-side panel holding the dock
  focus keeps scene routing (Delete after clicking Hierarchy still deletes the entity).
- **Layout file v1 → v2.** `editor_layout_crusty.ron` gained `version`, `active` and `profiles`
  (inactive profiles only — `profiles` never contains `active`). `tree`/`state` stay the active
  profile's live copy. A v1 file (no `version`) loads as the `Scene` profile verbatim and is
  rewritten as v2 on the next save. View ▸ Reset Layout resets the active profile only; Reset All
  Layouts resets every profile. A *stored* profile never re-reads `default_tree`; a profile with no
  stored tree yet does on its first activation (every non-Scene profile after a v1 migration, all
  of them after Reset All, one whose stored tree lost its marker).
- **The document strip is a heuristic, not an id.** `documents_leaf_index` picks the leaf holding
  the most document tabs (ties → first in traversal order); `active_document`, `reset` and the
  swap write-back all use it.
- **Profile gate vs preview gate.** `profile_of` uses `GraphDomain::is_animation_family` (machine
  *and* embedded rule graphs get the AnimGraph layout); `anim_preview_body` needs `is_animation`
  (the machine), so a focused rule graph shows the Preview panel with "Not an animation graph".
- **One preview target at a time.** A graph's preview strip drives exactly one target: a bound
  ECS `AnimGraphRuntime` when one exists (the Anim Preview panel then *mirrors* it read-only,
  chip `LIVE · name`, no Play/Pause), otherwise the panel's own `AnimMachine`
  (`PANEL_INSTANCE_ID`). `fold_anim_panel_previews` in `app.rs` decides per frame. Panel machines
  live in `scene.anim_previews` and are pruned when nobody drew them last frame — the next draw
  restarts at ENTRY, so hiding the panel resets the preview by design. `build_anim_preview_cbs`
  records one command buffer per frame under the fixed `ANIM_PREVIEW_TAB` key and stops at the
  first entry that records; a second Anim Preview surface needs per-graph target keys first.

## Physics Patterns

### Body Types

```rust
// Dynamic: Affected by forces, collisions
RigidBodyType::Dynamic

// Kinematic: Moved by code, affects dynamic bodies
RigidBodyType::KinematicPositionBased

// Static: Never moves, infinite mass
RigidBodyType::Static
```

### Sync Timing

```rust
// Physics runs AFTER ECS systems, BEFORE rendering
loop {
    input.update();
    systems.update(world, delta);  // Game logic first
    physics.step(world, delta);    // Then physics
    render(world);                 // Then render
}
```

## Collision Pipeline (M2, cooked chunks)

See `docs/roadmap/VULKANO-M2-COLLISION-PIPELINE.md` for the full design.

### Conventions

- **Queries run in Z-up world space** — `game_shared::collision::ChunkStore`
  is the one query layer for client and (M5+) server WASM. Never query Rapier
  for static world geometry.
- **World grid**: `game_shared::world_grid`, `CHUNK_SIZE = 64.0`, `IVec2`
  chunk coords. Shared with M8 interest cells.
- **`StaticCollision` is opt-in** — unmarked meshes don't cook. Falling
  through the floor usually means the mesh isn't marked.
- Border triangles are **duplicated** into both chunks with the same stable
  triangle id; queries dedup by id (earliest TOI, tie-break lowest id).

### Cooking workflow

```bash
cargo run --bin collision_cooker -- "scenes/<name>.scene" [--force]
```

- Also available as an editor menu action. Export scripts cook every scene
  before packing.
- Output: `content/collision/<stem>/manifest.ron` + `<x>_<y>.ccol`.
- **Staleness**: manifest stores `scene_hash` (fnv1a of scene bytes),
  `format_version`, `cooker_hash`; the cooker skips when all match. The hash
  does not cover referenced mesh assets — after editing a mesh, use `--force`.
- Bump `COOKER_VERSION_HASH` (`engine/src/engine/collision/cook.rs`) whenever
  cook output changes for identical input.

### Precision: shape-casts run in f64

parry's f32 GJK terminates at ~1e-3 relative error — too coarse for the
1 mm / 0.1° battery tolerances and for stable face-vs-edge contact ordering
at triangle seams. Per-triangle shape-casts therefore widen to `parry3d-f64`
(same `=0.20.2` pin). f64 add/mul/sqrt is IEEE-deterministic on both x86-64
and wasm32, so client/server parity holds. Cooked chunks and raycasts stay
f32. Never "simplify" the cast path back to f32.

### Golden battery

- Cases: `game_shared/tests/data/collision/battery.ron`, run against
  checked-in canonical `.ccol` chunks in the same directory. M6 reruns the
  identical files in server WASM.
- After changing the test geometry or format:
  `cargo test -p game_shared --test golden_battery regenerate -- --ignored`
  (a drift-guard test fails until you do).
- `game_shared` must keep compiling for `wasm32-unknown-unknown`
  (`cargo check -p game_shared --target wasm32-unknown-unknown`).

## Serialization Patterns

### Custom Serde for nalgebra-glm

```rust
// nalgebra types need custom serialization
#[derive(Serialize, Deserialize)]
pub struct Transform {
    #[serde(with = "vec3_serde")]
    pub position: glm::Vec3,

    #[serde(with = "quat_serde")]
    pub rotation: glm::Quat,
}
```

### Entity References in Saved Data

```rust
// DON'T save hecs::Entity directly (unstable IDs)
// DO save stable identifiers
pub struct EntityRef {
    pub name: String,  // Or UUID
}
```

## Node Graph Patterns (Task 40)

- **Stable slugs everywhere**: a node type's `id` and its pin `slug`s are
  serialized identity — never rename them without registering a migration
  step (`registry.register_migration(type_id, from_version, |ctx| ...)` and
  bumping `NodeDescriptor::version`). Display `name`/`label` change freely.
- **Pin renames are migrations**: use `MigrationCtx::rename_pin` — it moves
  the stored constant *and* rewrites incident edges. A props-only edit will
  silently orphan edges.
- **Consumers must not evaluate graphs by mutating `&mut World` directly**:
  when Tasks 41/45 build evaluators, node execution goes through
  `CommandBuffer` and the executing system declares access via
  `SystemDescriptor`, per the Task 32 contract (the Task 40 close-out
  records this as a standing constraint — no executor exists in Task 40).
- **Adding a node type**: prefer `#[derive(ScriptNode)]` (or a domain derive)
  over hand-writing `NodeDescriptor`; add `auto_register` only if the node
  should always exist — plugin-owned nodes register manually in
  `Plugin::build`.
- **Graph fixtures**: golden files live in `node_graph/fixtures/`;
  `UPDATE_GRAPH_FIXTURES=1 cargo test -p rust_engine write_fixture` (and the
  migration golden) regenerate them. Never hand-edit the `_expected` files.
- **Editor keys are content-relative forward-slash paths** — same key shape
  for tabs (`graph:{key}`), the resolver, and hot-reload matching.

## Graph Execution Gotchas (Task 45-A)

- **Timeline is a per-node ticker, not a wait**: `update` fires *once per
  tick* while a run is under way — the Play tick samples `t = 0` in the
  caller's activation, every later tick is an interpreter-spawned drive
  activation independent of all exec flow (`GraphInstance::tickers`, Task 41
  ticket 10). Play is therefore fire-and-forget, and a `Delay` — in the
  Update chain or after Play — parks only its own activation, never the run.
  A run of duration `d` lands its last sample on the tick that crosses `d` —
  do not assert "N ticks = N×dt seconds of curve" without accounting for the
  `t = 0` sample. `finished` fires exactly once, one tick after the clamped
  end sample, and never for a looping Timeline.
- **A pause holds the whole instance, and only the bound one** (GS-4). When
  any activation parks on a breakpoint, no other activation of that instance
  advances, no due latent wakes, no queued event drains and instance time does
  not move — one graph is one timeline of effects, and a half-frozen one is a
  state no unpaused run could reach. Conversely the `BreakSet` lives on
  `GraphRuntime`, not on the shared `Plan`: only the instance a graph tab is
  bound to ever pauses, so debugging one Duck does not stop the other three.
  `Paused` is its own `ThreadState` (no due time, resumes only on command) and
  parks *before* the node pulls its inputs, so the effect you stopped at has
  provably not happened. Step is one firing for the whole instance; Stop ends
  the session with no `halted` error, and the runtime component is rebuilt on
  the next play, so everything re-arms by itself.
- **The debugger's keys are F11 / F10, not the mockup's F5 / F10.** `App`
  handles F5 (Play/Stop) and F6 (Pause/Resume play) as raw winit key events,
  before any keymap context is consulted and without looking at modifiers —
  so `Keymap::conflicts()` cannot see them and no `GraphTab` binding on F5 (or
  Shift+F5, or Ctrl+F5) would survive. Resume ships on F11; the banner labels
  its buttons from `Keymap::chord_label`, so a rebind re-labels them.
- **A suspension captures the whole continuation**, loop frames included.
  That is why `Delay` inside `ForLoop` works, and also why an activation holds
  **at most one** suspension — a second latent on the same activation is a
  contract violation, not a queue. Concurrent *activations* of one latent node
  wait independently (node state is shared, frames are not).
- **Exec inputs fan in; data inputs do not.** Many exec wires may converge on
  one exec input — that is how a Branch's two sides rejoin a shared tail, and
  it is unauthorable otherwise. Two *values* arriving at one data input has no
  meaning and is `InputMultiplyConnected`. Both rules live in `validate_doc`
  (`d.pin_type(..) == Exec` is the gate); do not re-derive them at a call site.
  The mirror rule: an exec **output** takes at most one wire
  (`ExecOutputFanOut`), because `PlanNode::exec` holds one target per output
  pin — a second wire would not fan out, it would silently replace the first.
  If you need two continuations, that is what `Sequence` is for. The editor
  enforces it as a gesture: dragging a second wire off an occupied exec output
  *replaces* the existing one, the same way a data input's second wire does.
- **Config rows shift the pin band**: a node's per-instance configuration
  (the variable a `var_get` names, a Timeline's curve) occupies rows
  `0..config_n`, so pin row `i` is `config_n + i`. Everything measured off a
  node — pin centres, wire anchors, the band separator, the node's height —
  goes through `band_y`/`node_h` for that reason. Computing a pin's `y`
  directly is how a node ends up disagreeing with its own wires by one row.
- **Cubic curve tangents are time-scaled Catmull-Rom, deliberately**:
  keyframes are not uniformly spaced in time, and the uniform form overshoots
  badly when they are not. The finite-difference form scaled by each segment's
  duration is C1, degenerates to a straight line on collinear keys, and is
  what an editor's "auto" tangent gives — so the plot draws what the
  interpreter samples. Never reimplement sampling; call
  `curve_asset::Track::sample`.
- **Every author-side write invalidates the whole plan cache.** Saving a
  `.graph`, a `.subgraph` or a `.curve` calls `GraphPlanCache::invalidate`,
  which is `invalidate_all` — a subgraph inlines into its hosts and a curve
  track is a Timeline pin, and the cache does not track either reference tree,
  so dropping one key restarts the hosts *onto their stale plans*, which is
  worse than not invalidating at all. The editor's own resolver needs nothing:
  it is rebuilt from open tabs + disk every frame, open tabs winning.
- **A save is what makes a plan stale, so the save path invalidates** —
  not the file watcher, whose echo guard returns early precisely because the
  write was ours. Both `save_graph_editor` and `save_curve_state` follow the
  same shape: write, then invalidate. Adding a third asset kind means adding
  the second half too.
- **A graph-spawned entity is a full citizen**: it gets an `EntityGuid`, and
  if the prefab carries `RigidBody` + `Collider` it is registered with Rapier
  on the spawn tick (`physics::register_entity`) and removed on despawn
  (`physics::deregister_entity`, walking the subtree *before*
  `despawn_recursive` dissolves it). Registration is skipped, not failed, when
  no `PhysicsWorld` resource exists.
- **Demo/fixture assets are generated, not hand-written**:
  `UPDATE_GRAPH_FIXTURES=1 cargo test -p rust_engine --lib write_runner_demo`
  regenerates `content/graphs/runner_demo.graph` *and*
  `content/prefabs/graph_cube.prefab`, and refuses to write either unless the
  graph compiles and runs.

## Animation at Scale Gotchas (Task 41.5)

- **Any new pose-affecting feature must join the forced-eval list — or tick
  every frame.** Under update-rate throttling only pose evaluation is gated;
  a feature that changes the pose on a frame the bucket skips will simply
  not show (plan risk §7.3). Tick-local sources (crossfade, play-once, event
  fired) live in `tick_entity`; external sources set
  `throttle.force_eval_external` from a *serial* system before
  `AnimGraphSystem` (foot-lock edges do this). Every source is pinned by a
  test in the "Update-rate throttling" section of `acceptance.rs` — add
  yours there too, or the next refactor silently drops it.
- **Never touch `Resources` or arming from the parallel section.**
  `AnimGraphSystem.evaluating` is set around the rayon region and `arm()`
  debug-asserts against it. The parallel closure gets an entity's two
  components and an immutable clip-cache borrow, nothing else — anything
  needing camera data, `TransformCache`, or structural changes goes in a
  serial pre-pass that lands results as component state (that is exactly
  what the significance pre-pass and IK target resolution do).
- **`model_space` (and everything IK) is mesh-local Y-up render space** —
  pre-inverse-bind, not game Z-up. Converting a world Z-up target:
  `target_model = entity_render⁻¹ * zup_to_yup(target_world)` with
  `entity_render = TransformCache::get_render` — which is the *previous*
  frame's transform (accepted one-frame latency, same as the render path).
  Going the other way (sockets → world): entity render matrix then
  yup→zup, as `debug_draw.rs::joint_positions` does.
- **Editing a bone's model-space matrix does not move its children.**
  Phase-1 FK ran already; descendants keep their stale model matrices until
  you call `ik::rewalk_descendants` over the edited set (recomputing them
  from the unchanged animated locals). Forgetting this is how a solved knee
  leaves the foot behind.
- **Two-bone IK reach is exact only under uniform bone scale.** The solver
  recomposes via TRS decomposition; a non-uniformly scaled mid bone
  displaces the tip from the target (no NaN, just imprecision) — the SQT
  pipeline's existing decomposability assumption (ruling R7).
- **Every visible skeleton is present in every ring region — `dirty`/
  revision gate evaluation and the memcpy, never presence** (rulings
  R2/R6). Regions rotate, so "skip the upload for clean skeletons" is
  wrong by construction; the cursor always advances and only the redundant
  bytes are skipped (prefix + revision match). Do not "optimize" a skipped
  skeleton out of `write_palette` — its base index must exist in the frame's
  region or its draws read someone else's palette.
- **`Some`/`None` palette frames must never interleave** (debug-assert in
  `prepare_skinning_binds`, commit 5d8682e). The palette-less fallback
  rotates its own slot independently of the fence ring, so a `None` frame
  after palette-bearing frames could rewrite a VP UBO still referenced by
  an in-flight frame. Shipping hosts always send `Some`; the `None` path is
  for tests/tools that never mix.
- **Ring growth waits like any other frame** (ruling R14). P1's
  "fresh regions skip the wait" epoch shortcut was removed in P7: with two
  ring buffers (palettes + instance metadata) behind one `PaletteRingSync`
  handshake, skipping is only sound if *both* buffers were just replaced.
  The wait is free in steady state — do not reintroduce the shortcut when
  adding a third per-frame GPU-read buffer; join the existing handshake.
- **Foot IK is configured on the IK Chain node, not a component** (ruling
  R11): `foot`/`ankle_offset`/`pelvis` props on `anim_ik_chain`; the foot
  bone is the chain's tip, and the lock event names derive from the *chain
  name* (`<chain>_down`/`<chain>_up` clip events). Rename a chain and the
  clip's event markers must follow. `IkTargets` is inserted by
  `FootPlacementSystem` automatically — there is nothing to attach by hand.

## Locomotion & Foot IK Gotchas (Task 41.6)

- **Rapier's default friction combine rule is Average.** A friction-0
  capsule against a 0.8 floor still gets μ = 0.4 — enough to pin the
  trailing hemisphere on a stair edge while the snap pushes it down.
  `register_entity` gives colliders declared with friction ≤ 0 the `Min`
  rule so the zero wins; everything else stays Average. Zero friction is a
  stated intent, not a default.
- **A capsule wider than a tread is always an edge balance.** Radius 0.4
  on 0.3 m treads means the body never rests flat on one step; any snap or
  slope follow applied while standing presses it onto a tilted edge
  contact and shoves it forward every step — a residual slide that keeps
  the speed up, keeps friction off and rides the whole staircase down.
  Hence **never snap while standing**: `standing = grounded && !has_input`
  (intent, not speed) keeps `vz`, and grip friction plus the contact solver
  hold the body.
- **`set_timestep` must set `integration_parameters.dt`.** It only changed
  the accumulator interval before P6, so Rapier integrated at 1/60 whatever
  `fixed_timestep_hz` said. Per-fixed-step corrections (`snap_vz`,
  `step_lift_vz`) use `PhysicsWorld::fixed_dt()`, never render dt —
  overshoot into the ground otherwise.
- **`Transform` of a dynamic body is the interpolated presentation pose**
  (`PhysicsWorld::present`, ≤ 1 fixed step behind). Probe and measure from
  `PhysicsWorld::body_position(handle)`; a ground ray cast from the
  presentation position reports last step's ground.
- **The schedule validator needs a direct edge per conflicting pair** in a
  stage (`schedule.rs` checks pairs, not transitive reachability), edges
  to feature-gated systems must be `#[cfg]`-gated (dangling names are
  rejected globally), and **only a launch builds the runtime schedule** —
  or the `plugin.rs` test `gameplay_systems_validate_against_the_host_
  schedule`, which stubs the hosts' descriptors; extend it (and keep the
  FOOT_PLACEMENT descriptor identical in `app.rs`, `standalone.rs` and
  the stub) rather than waiting for a launch panic.
- **Mixamo rigs face −X after import.** The controller faces +X, so the
  rig child's `Transform` carries a 180° yaw (`rotation: (0, 0, 1, 0)`); a
  character that runs backwards or sideways is a rig-yaw bug, not a
  controller bug.
- **`.anim` bone indices assume the sibling mesh's bone order.** A clip
  imported from a separate file (`--import-anim`, "Animation only") is
  remapped by bone *name* when armed (`ClipSet::armed_for`, memoised in
  `AnimClipCache::armed`); bones the skeleton lacks are dropped with one
  `eprintln!` per clip set. Identical tables skip the remap.
- **Import scale 0.01 for Mixamo.** An animation-only asset records no
  scale anywhere; position keys must be written at the target mesh's scale
  (`Defeated.mesh.ron`: 0.01) — `--import-scale 0.01` or the dialog's Scale
  field. A mismatch is a rig that slides or floats, not an error.
- **`log::` is invisible in `game_client`** (no logger installed). Use
  `println!` / `eprintln!` for anything that must be seen (the dropped-bone
  report, schedule validation errors), and keep it terse.
- **A foot lock belongs to the planting state.** `FootState.lock_state`
  records `machine.current_state()` at `_down`; `place_feet` releases when
  the state changes, because a stop into Idle (or a jump) never fires the
  clip's `_up` and the held point would sit at the reach limit forever.
- **Foot placement writes terrain deltas, not points.** `IkGoal::Offset` is
  applied on the *pre-pelvis* animated tip; pinning the foot to the ray
  contact every frame erases swing clearance ("sticky feet") and drives the
  two-bone solver to full extension ("knee break"). Never write a `Point`
  for an unlocked foot; leave `pole: None` for foot chains so the knee is
  taken from the current pose.

## Animation Pipeline Gotchas (Task 41.7)

- **Regions cannot nest — scope over the flat document, never a container.**
  `regions` is one level deep (a state's blend tree, a transition's rule),
  so the pipeline is a *type partition* of the top-level `nodes` list
  (`is_pipeline_node`) shown through `CanvasScope`, not a region around the
  machine. Anything that walks `doc.nodes` on an animation document must
  go through `visible_nodes()` / `node_visible` or it will draw, hit or
  lay out the other canvas's nodes.
- **`parse_graph` stamps the version before domain upgrades run** — a "v3
  document" is never observable by version once parsed. Animation upgrade
  triggers are structural (`needs_pipeline_root`: machine nodes and no
  pipeline-*only* type; loose slots / chains are legal v3 content). Do not
  add a version-gated animation upgrade; and a document with pipeline nodes
  but no Output is an authored deletion (refusal), not a re-upgrade.
- **`anim_node_registry()` is the union of both families; the subsets are
  for palettes only.** The editor opens, validates, migrates and themes the
  flat document against the union; placing, validating or committing
  against a subset flags the other family as unknown types (R25). Use
  `anim_machine_registry()` / `anim_pipeline_registry()` for listing.
- **An empty `slot_order` / `ik_order` means two different things.** On a
  compiled root plan it means "nothing reachable — run nothing"; on an
  uncompiled plan (hand-built tests, the blend-space preview, nested plans)
  it falls back to index order (R13). `PlanPipeline::compiled` decides;
  always read orders through `slot_index` / `slot_count` / `ik_index` /
  `ik_count`, never `if order.is_empty()` — that shortcut solved
  unreachable IK chains (review F1).
- **Masks are keyed by host node id only.** Nested pipelines are never
  compiled, so `rt.masks: BTreeMap<node_id, _>` has no cross-document
  collision, and lifted slots carry no mask. Masks are symbolic in the plan
  and resolved per skeleton at arm (`arm_masks`); the preview arms against
  its *own* skeleton and never copies the entity's.
- **Masked overlays never silence base events or touch foot locks; only
  whole-body ones do** (U1). Markers carry no owning bone, so a masked Play
  Once suppresses nothing and a lock survives it; a whole-body Play Once
  suppresses base events by `1 − weight` and releases every lock on start
  (`slot.started()` read by `place_feet` next frame). A full-body action
  that must cut footsteps needs an empty `bones`.
- **A partial-channel layer leaks into the next frame's base.** Unkeyed
  channels keep their pre-sample value (the agreement every blend in
  `machine.rs` relies on): once a Layer whose clip keys rotation / scale
  has blended into a bone whose base clip keys only translation, the next
  frame's *base* pose already carries that rotation / scale. A test that
  reads a base pose must take it before the layer ever blends (P3's SQT
  test reads it off the arming tick).
- **The preview arms clips on any bone overlap, like the runtime.** The
  Mixamo X Bot clips name seven fingertip bones differently from
  `Defeated.mesh`; requiring full coverage (`bones_cover`) left them
  unarmed and the preview posed nothing. Only a zero-overlap set stays
  unarmed so "bones don't match" can still be diagnosed (`bones_overlap`,
  review F2) — a mismatch fixture must be *disjoint* (`["other"]`), not a
  subset.
- **Parity is proven by `assert_legacy_frame`, not by the v3-vs-v4 file
  halves.** The compiler upgrades a v3 document on a private `Cow` copy
  (R2), so compile(v3) ≡ compile(upgraded) by construction and comparing
  the two proves nothing. The golden tests compare every tick against the
  surviving legacy evaluator (`evaluate_pose` + `slot.apply` +
  `collect_anim_events`); keep that path alive as long as the tests cite it
  (review F3).
- **Runtime `disabled` strings carry a `"{graph}: "` prefix the anchor arms
  do not strip** (review F7). Only compile-time messages are anchored
  today; the preview's `status` carries the bare `layer #<id>: …` /
  `IK chain 'X': …` shape and anchors. Strip the prefix before feeding a
  runtime `disabled` string to `anchor_anim_refusal`.
- **`-p rust_engine` tests need `--features editor` — run them.** Two
  preview tests (`a_chosen_mesh_wins_and_a_mismatch_is_explained`,
  `the_entry_nodes_preview_mesh_wins_and_a_mismatch_is_explained`) had been
  failing on main since `149efed`: the by-name remap made
  `arm_clips_to_skeleton` overwrite a clip's bone table with the
  skeleton's, hiding the mismatch diagnosis. Nobody noticed until P1's gate
  (fixed in `f06b713`, relaxed to overlap by F2). The editor-feature suite
  is part of every package gate for a reason.

- **Every world the schedule runs on needs the animation caches.** The
  editor builds a fresh `GameWorld` per scene tab (`fresh_scene_world`);
  until 2026-09-23 it lacked `AnimGraphPlanCache` / `AnimClipCache` /
  `BlendSpaceCache`, so graphs armed silently and never evaluated — every
  character in a tab-opened scene sat in a T-pose with nothing in the
  Console. `insert_anim_caches` now serves both worlds, arming refuses
  loudly without a clip cache, and `anim.status` in the Console prints each
  runtime's state (armed / refused, machine time, revision, palette bones
  off bind pose). Plugin-staged resources still reach only the startup
  world — see the 41.7 deferred ledger.

- **Animation plays only in Play.** Both animation systems take their delta
  from `Time::playing_delta`, which is zero while the editor is in Edit mode
  (and while paused), so a character in the level holds the entry state's
  first frame — a posed preview, not a T-pose — and starts moving on F5.
  Standalone has no `EditorState` and always runs. The graph and single-clip
  systems ran unconditionally until 2026-09-23.

## Performance Gotchas

### Profile Before Optimizing

```rust
crate::profile_function!();
crate::profile_scope!("expensive_operation");
```

### Avoid Per-Frame Allocations

```rust
// BAD: Allocates every frame
fn update(&mut self, world: &mut World) {
    let entities: Vec<Entity> = world.query::<&Transform>()
        .iter()
        .map(|(e, _)| e)
        .collect();
}

// GOOD: Reuse buffer
struct MySystem {
    entity_buffer: Vec<Entity>,
}

fn update(&mut self, world: &mut World) {
    self.entity_buffer.clear();
    self.entity_buffer.extend(
        world.query::<&Transform>().iter().map(|(e, _)| e)
    );
}
```

### Batch Rendering

```rust
// BAD: Draw call per entity
for entity in entities {
    draw(entity);
}

// GOOD: Sort by material, batch
entities.sort_by_key(|e| e.material_index);
for (material, group) in entities.group_by(|e| e.material_index) {
    bind_material(material);
    draw_batch(group);
}
```

## Common Errors and Fixes

### "Entity does not exist"

```rust
// Entity was despawned but reference kept
// FIX: Check existence before access
if world.contains(entity) {
    world.get::<&Transform>(entity)?;
}
```

### Transform Scale is Zero

```rust
// Scale components clamped to prevent matrix singularity
transform.scale.x = transform.scale.x.max(0.001);
```

### Gizmo in Wrong Position

```rust
// Probably using local transform instead of world transform
// FIX: Use hierarchy::get_world_transform()
let world_pos = hierarchy::get_world_transform(world, entity);
```

### Mesh Renders at Origin

```rust
// Model matrix not applied
// FIX: Check push constants include model matrix
push_constants.model = world_matrix_to_render(&world_transform);
```

### Physics Body Doesn't Move

```rust
// Check body type - Static bodies never move
// FIX: Use Dynamic for movable bodies
RigidBodyType::Dynamic
```

## Testing Patterns

### Unit Tests for Systems

```rust
#[test]
fn test_damage_system() {
    let mut world = World::new();
    let entity = world.spawn((Health { current: 100.0, max: 100.0 }, Damage { amount: 10.0 }));

    damage_system(&mut world, 1.0);

    let health = world.get::<&Health>(entity).unwrap();
    assert_eq!(health.current, 90.0);
}
```

### Integration Tests

```rust
// Test coordinate conversion round-trip
#[test]
fn test_coordinate_conversion() {
    let pos_zup = glm::vec3(1.0, 2.0, 3.0);
    let pos_yup = position_to_render(&pos_zup);
    // Verify mapping: X→-Z, Y→X, Z→Y
    assert_eq!(pos_yup, glm::vec3(2.0, 3.0, -1.0));
}
```

## Debugging Tips

### Visual Debugging

```rust
// Draw debug lines (add to debug render pass)
debug_draw.line(start, end, color);
debug_draw.sphere(center, radius, color);
debug_draw.aabb(min, max, color);
```

### Console Commands

```
stat fps          # Show FPS overlay
entity.count      # Count entities in world
help              # List all commands
```

### Profiler Shortcuts

- **puffin**: Built-in, shows flame graph
- **Tracy**: External, more detailed timeline

## Code Style

### Error Handling

```rust
// Use Result for recoverable errors
pub fn load_asset(path: &Path) -> Result<Asset, AssetError> {
    let file = std::fs::read(path)?;
    // ...
}

// Use expect() only for programmer errors
let value = map.get(&key).expect("Key should exist after insert");
```

### Naming Conventions

| Type | Convention | Example |
|------|------------|---------|
| Components | PascalCase noun | `Transform`, `MeshRenderer` |
| Systems | snake_case verb | `update_transforms`, `apply_damage` |
| Resources | PascalCase noun | `Time`, `EditorState` |
| Events | PascalCase past tense | `EntitySpawned`, `CollisionOccurred` |

### Module Organization

```rust
// mod.rs exports public API
pub mod components;
pub mod systems;

pub use components::*;
pub use systems::{TransformSystem, PhysicsSystem};
```

## Patched Dependencies

These crates are forked in `crates/` directory:

| Crate | Reason | Issue |
|-------|--------|-------|
| `emath` | DragValue crash fix | egui #7747 |
| `transform-gizmo` | Z-up coordinate system | Custom |
| `transform-gizmo-egui` | Z-up coordinate system | Custom |

When updating egui, check if patches are still needed.
