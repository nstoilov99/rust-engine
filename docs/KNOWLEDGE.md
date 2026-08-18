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
