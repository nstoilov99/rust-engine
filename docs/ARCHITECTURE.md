# Rust Game Engine - Architecture

## Overview

This is a 3D game engine built with Rust using Vulkano (Vulkan bindings), hecs ECS, egui for the editor UI, and Rapier 3D for physics. The engine follows a modular architecture with clear separation between subsystems.

## High-Level Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│                          Application Layer                           │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐ │
│  │   Editor    │  │    Scene    │  │   Assets    │  │   Input     │ │
│  │   Panels    │  │  Management │  │   Browser   │  │   Manager   │ │
│  └──────┬──────┘  └──────┬──────┘  └──────┬──────┘  └──────┬──────┘ │
└─────────┼────────────────┼────────────────┼────────────────┼────────┘
          │                │                │                │
┌─────────┼────────────────┼────────────────┼────────────────┼────────┐
│         ▼                ▼                ▼                ▼         │
│                        Engine Core                                   │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐ │
│  │     ECS     │  │  Rendering  │  │   Physics   │  │    GUI      │ │
│  │   (hecs)    │  │  (Vulkano)  │  │  (Rapier)   │  │   (egui)    │ │
│  └──────┬──────┘  └──────┬──────┘  └──────┬──────┘  └──────┬──────┘ │
└─────────┼────────────────┼────────────────┼────────────────┼────────┘
          │                │                │                │
┌─────────┼────────────────┼────────────────┼────────────────┼────────┐
│         ▼                ▼                ▼                ▼         │
│                      Platform Layer                                  │
│  ┌─────────────────────────────────────────────────────────────────┐ │
│  │                 Vulkan Context (vulkano)                        │ │
│  │          Device, Swapchain, Command Buffers, Sync               │ │
│  └─────────────────────────────────────────────────────────────────┘ │
│  ┌─────────────────────────────────────────────────────────────────┐ │
│  │                    Window (winit)                               │ │
│  └─────────────────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────────────┘
```

## Module Structure

```
src/engine/
├── core/           # Vulkan initialization and management
│   ├── context.rs      # VulkanContext - instance, surface, device selection
│   ├── device.rs       # LogicalDeviceContext - queues, command pools
│   └── swapchain.rs    # Swapchain management and recreation
│
├── rendering/      # All rendering pipelines
│   ├── common.rs       # Shared rendering utilities, Renderer trait
│   ├── graph/          # Render graph (frame graph) system
│   │   ├── render_graph.rs   # RenderGraph: topological sort, culling, execution
│   │   ├── pass_node.rs      # PassNode, PassBuilder, PassContext
│   │   ├── resource.rs       # ResourceId, ResourceDesc, ResourceTable
│   │   └── resource_pool.rs  # TransientResourcePool (texture reuse across frames)
│   ├── 2d/             # 2D sprite rendering
│   │   ├── pipeline_2d.rs
│   │   └── sprite_batch.rs
│   └── 3d/             # 3D deferred rendering
│       ├── deferred.rs     # G-buffer and deferred pipeline
│       ├── light.rs        # Light types and calculations
│       ├── material.rs     # PBR material system
│       ├── mesh.rs         # Mesh primitives (cube, plane)
│       ├── mesh_manager.rs # GPU mesh management
│       ├── pipeline_3d.rs  # Forward pipeline (fallback)
│       └── shadow.rs       # Shadow mapping
│
├── ecs/            # Entity Component System
│   ├── components.rs   # Core components (Transform, MeshRenderer, etc.)
│   ├── hierarchy.rs    # Parent-child relationships, world transforms
│   ├── systems.rs      # System trait and scheduler
│   └── world.rs        # World wrapper and utilities
│
├── editor/         # Editor UI and tools
│   ├── panels/         # Editor panels (hierarchy, inspector, console)
│   ├── viewport/       # 3D viewport rendering and interaction
│   ├── gizmos/         # Transform gizmos
│   └── asset_browser/  # Asset management UI
│
├── physics/        # Rapier 3D integration
│   ├── mod.rs          # PhysicsWorld, sync with ECS
│   └── components.rs   # RigidBody, Collider, Velocity
│
├── assets/         # Asset loading and management
│   ├── mod.rs          # Asset loading functions
│   ├── texture.rs      # Texture loading
│   └── gltf.rs         # GLTF/GLB model loading
│
├── gui/            # egui-Vulkano integration
│   ├── mod.rs          # EguiVulkanoIntegration
│   └── renderer.rs     # egui render pass
│
├── adapters/       # Coordinate system conversion
│   └── render_adapter.rs   # Z-up ↔ Y-up conversion
│
├── camera/         # Camera systems
│   ├── camera_2d.rs
│   └── camera_3d.rs
│
├── input/          # Input handling
│   └── mod.rs          # InputManager, key/mouse state
│
├── math/           # Math utilities
│   └── frustum.rs      # Frustum culling
│
└── utils/          # General utilities
    ├── coords.rs       # Coordinate conversion helpers
    └── game_loop.rs    # Fixed timestep game loop
```

## Rendering Pipeline

### Deferred Rendering

The engine uses a deferred rendering pipeline for efficient multi-light scenes:

```
┌──────────────┐     ┌──────────────┐     ┌──────────────┐
│  G-Buffer    │     │   Lighting   │     │   Compose    │
│    Pass      │────▶│    Pass      │────▶│    Pass      │
└──────────────┘     └──────────────┘     └──────────────┘
      │                    │                    │
      ▼                    ▼                    ▼
┌──────────────┐     ┌──────────────┐     ┌──────────────┐
│ - Albedo     │     │ - Directional│     │ - Tone map   │
│ - Normal     │     │ - Point      │     │ - Gamma      │
│ - Position   │     │ - Ambient    │     │ - FXAA       │
│ - Material   │     │ - Shadows    │     │              │
└──────────────┘     └──────────────┘     └──────────────┘
```

### Render Frame Flow

The deferred renderer uses a **render graph** (`engine/src/engine/rendering/graph/`) to determine pass execution order. Each frame, a local `RenderGraph` is built:

1. Passes declare resource dependencies (read/write/modify)
2. The graph topologically sorts passes based on these dependencies
3. Dead passes (writing resources nobody reads, not marked as output) are culled
4. `compiled_order()` drives command recording in the correct order

```
1. Begin Frame
   └── Acquire swapchain image

2. Build Render Graph
   ├── Declare virtual resources (gbuffer textures, target)
   ├── Register passes with dependencies:
   │   ├── geometry: writes gbuffer (position, normal, albedo, material, depth)
   │   ├── lighting: reads gbuffer, writes target
   │   ├── grid (optional): reads depth, modifies target
   │   └── debug_draw (optional): reads depth, modifies target
   ├── Mark target as output
   └── Compile (topological sort + culling)

3. Execute Passes (in graph-determined order)
   ├── Geometry Pass → G-Buffer
   ├── Lighting Pass → Target image
   ├── Grid Pass (if visible) → Overlay on target
   └── Debug Draw Pass (if lines exist) → Overlay on target

4. GUI Pass (outside graph)
   ├── egui rendering
   └── Overlay on top of scene

5. End Frame
   └── Present swapchain image
```

The graph supports transient resources (pooled textures reused across frames) for future passes that need temporary render targets.

## ECS Architecture

### Custom ECS (wrapping hecs)

The engine uses a **custom ECS architecture** that wraps hecs for entity/component storage while adding Resources, Events, Commands, ChangeTicks, and Staged Scheduling on top.

Entity IDs use `hecs::Entity` directly (no custom allocator). Component storage is entirely hecs. Our custom layers sit alongside.

```
┌─────────────────────────────────────────────────────────────┐
│                    Custom ECS Layer (GameWorld)              │
├─────────────────────────────────────────────────────────────┤
│  Resources      │  Events          │  Schedule              │
│  ├── Time       │  ├── EntitySpawned    ├── First          │
│  └── EditorState│  ├── EntityDeleted    ├── PreUpdate      │
│                 │  ├── SelectionChanged ├── Update         │
│  ChangeTicks    │  └── PlayModeChanged  ├── PostUpdate     │
│  ├── added map  │                       └── Last           │
│  └── changed map│  CommandBuffer (in GameWorld only)       │
│                 │  ├── Spawn / Despawn                     │
│                 │  └── Insert / Remove                     │
├─────────────────────────────────────────────────────────────┤
│                    hecs Layer (wrapped)                      │
│  ├── Entity archetype storage (hecs::Entity IDs)            │
│  ├── Component queries                                       │
│  └── Iteration                                               │
└─────────────────────────────────────────────────────────────┘
```

```rust
// Components are plain data structs
pub struct Transform {
    pub position: glm::Vec3,
    pub rotation: glm::Quat,
    pub scale: glm::Vec3,
}

// Systems use Resources for global state
fn movement_system(world: &mut hecs::World, resources: &mut Resources) {
    let delta = resources.get::<Time>().map(|t| t.scaled_delta()).unwrap_or(0.0);

    for (id, (transform, velocity)) in world.query_mut::<(&mut Transform, &Velocity)>() {
        transform.position += velocity.0 * delta;
    }
}

// Systems registered to stages with run criteria
schedule.add_system_with_criteria(
    FunctionSystem::new("movement", movement_system),
    Stage::Update,
    RunIfPlaying,  // Only runs during play mode
);

// Deferred ops via Commands (no borrow conflicts during iteration)
fn spawner_system(world: &mut hecs::World, resources: &mut Resources) {
    let commands = resources.get_mut::<CommandBuffer>().unwrap();
    commands.spawn((Transform::default(), Name::new("New Entity")));
    // Applied between stages by Schedule
}
```

### Core Components

| Component | Purpose |
|-----------|---------|
| `Transform` | Position, rotation, scale in Z-up space |
| `Parent` | Entity parent reference |
| `Children` | List of child entities |
| `Name` | Human-readable entity name |
| `MeshRenderer` | Mesh and material indices |
| `Camera` | Camera parameters (FOV, near, far) |
| `DirectionalLight` | Sun-like light |
| `PointLight` | Local light source |
| `RigidBody` | Physics body |
| `Collider` | Physics collision shape |

### Hierarchy System

Parent-child relationships with cached world transforms:

```
┌─────────────┐
│   Parent    │  Transform: (5, 0, 0)
│   Entity    │  WorldTransform: (5, 0, 0)
└──────┬──────┘
       │
       ▼
┌─────────────┐
│   Child     │  Transform: (2, 0, 0)  ← Local offset
│   Entity    │  WorldTransform: (7, 0, 0)  ← Parent + Local
└─────────────┘
```

## Coordinate System

### Game World (Z-up)
- **X** = Forward (Red)
- **Y** = Right (Green)
- **Z** = Up (Blue)

### Vulkan Render (Y-up)
- **X** = Right
- **Y** = Up
- **-Z** = Forward (into screen)

### Conversion

All game logic uses Z-up. The `render_adapter` module converts to Y-up at render time using basis change matrices:

```rust
// In render_adapter.rs
pub fn world_matrix_to_render(world_matrix_zup: &glm::Mat4) -> glm::Mat4 {
    let c = get_basis_change_matrix();
    let c_inv = glm::transpose(&c);  // C is orthogonal
    c * world_matrix_zup * c_inv
}
```

## Editor Architecture

### Panel System

Panels are tabs in a crusty-gui dock tree (`DockNode` splits/leaves, persisted as
`editor_layout_crusty.ron` by `dock_crusty::CrustyDockLayout`). Documents (scene viewports,
graphs, curves, blend spaces, meshes, input assets) share one **document strip** leaf; everything
else is a side panel. The dock keeps one tree per **layout profile** (`LayoutProfile`: `Scene`,
`AnimGraph`, `ScriptGraph`, `BlendSpace`, `Curve`, `Mesh`) and swaps the surrounding panels when
the focused document's kind changes — Unreal-style asset editors inside the main window. Each
profile remembers its own splits and panels; gotchas in `docs/KNOWLEDGE.md` ▸ Layout profiles.

```
Scene (default)                          AnimGraph (default)
┌──────────┬───────────────┬──────────┐  ┌─────────┬────────────────┬───────────┐
│Hierarchy │  documents    │Inspector │  │Variables│   documents    │  Preview  │
│   20%    │  (viewports…) │   20%    │  │   18%   │   (graphs…)    │           │
│          ├───────────────┤          │  │         ├────────────────┼───────────┤
│          │Console|Profil.│          │  │         │Assets | Console│  Details  │
└──────────┴───────────────┴──────────┘  └─────────┴────────────────┴───────────┘
```

`ScriptGraph` is Variables 18% | documents 62% | Details 20% (Details alone on the right, at the
Inspector's width); `BlendSpace`, `Curve` and `Mesh` are documents over Assets | Console (those
tabs embed their own details/preview).

### Graph side panels

Three dock panels are keyed to the **focused graph document** (`App::focused_document`, not the
dock focus, so clicking another panel never loses the graph):

- **Details** (`graph_dock_panels_crusty::graph_details_panel`) — the selection's properties,
  drawn from the same `config_rows` / inline-widget helpers as the canvas bands, so edits go
  through the same `GraphEdit::SetProperty` path and undo identically.
- **Variables** (`graph_variables_panel`) — the in-tab variables strip drawn docked; strip and
  panel share one state (widget ids are salted per surface).
- **Anim Preview** (`anim_preview_crusty::anim_preview_panel` over
  `anim_graph_preview::AnimGraphPreview`) — compiles the focused anim graph and runs the real
  `AnimMachine` + `evaluate_pose` on its own skeleton instance, parameters driven by the graph's
  preview strip; mirrors a bound ECS runtime read-only when one exists.

The Anim Preview panel and the blend space tab share one **skinned preview pane**
(`anim_preview_crusty::skinned_preview_pane`: `MeshPreviewRenderer` target, orbit, Play/Pause,
state chip); the host records both through `record_skinned_preview` in `app.rs`.

### Viewport Interaction

1. **Camera Controls** (Unreal-style)
   - RMB + WASD: Fly camera
   - Alt + LMB: Orbit around focus
   - Alt + MMB: Pan
   - Scroll: Zoom / adjust speed

2. **Selection**
   - LMB click: Select entity
   - Ctrl + click: Multi-select

3. **Transform Gizmos**
   - W: Translate mode
   - E: Rotate mode
   - R: Scale mode
   - Q: Toggle local/world space

## Physics Integration

Rapier 3D integration with automatic ECS synchronization:

```
┌──────────────┐     ┌──────────────┐     ┌──────────────┐
│     ECS      │────▶│   Physics    │────▶│     ECS      │
│  Transform   │     │   Simulate   │     │  Transform   │
│   (input)    │     │              │     │  (updated)   │
└──────────────┘     └──────────────┘     └──────────────┘
```

### Sync Flow

1. **Pre-physics**: Copy ECS transforms to Rapier bodies
2. **Simulate**: Rapier steps the physics world
3. **Post-physics**: Copy Rapier positions back to ECS transforms

## Asset Pipeline

```
┌─────────────┐     ┌─────────────┐     ┌─────────────┐
│  Raw Asset  │────▶│   Loader    │────▶│  GPU Asset  │
│  (.gltf,    │     │             │     │  (buffers,  │
│   .png)     │     │             │     │   textures) │
└─────────────┘     └─────────────┘     └─────────────┘
```

### Supported Formats

| Type | Formats | Loader |
|------|---------|--------|
| Models | GLTF, GLB | `load_gltf()` |
| Textures | PNG, JPG, HDR | `load_texture()` |
| Scenes | RON | `serde` + custom |

## Serialization

Scenes are saved as RON (Rusty Object Notation):

```ron
(
    entities: [
        (
            name: "Cube",
            transform: (
                position: (x: 0.0, y: 0.0, z: 0.0),
                rotation: (x: 0.0, y: 0.0, z: 0.0, w: 1.0),
                scale: (x: 1.0, y: 1.0, z: 1.0),
            ),
            mesh_renderer: Some((
                mesh_index: 0,
                material_index: 0,
            )),
        ),
    ],
    meshes: ["cube"],
    materials: ["default"],
)
```

## Node Graph Framework (Task 40)

`engine/src/engine/node_graph/` is the domain-agnostic graph layer used by
future animation/scripting/material/VFX/audio/AI tasks. No domain logic or
evaluators live here — consumers bring their own node libraries and backends.

- **Documents** (`doc.rs`): `GraphDoc` — plain data. Node *types* and *pins*
  are stable string slugs, node *instances* are doc-local integer ids,
  cross-asset references (subgraphs) are content-relative path strings.
  Node positions live in the asset, not GUI memory. Serialized as RON
  (`<name>.graph` / `<name>.subgraph`), byte-stable saves.
- **IO** (`io.rs`): version-envelope parse — the container version is probed
  before strict deserialization so container migrations can rewrite old
  documents.
- **Registry** (`registry.rs`): `NodeRegistry` — runtime `register()` is the
  primary API (Task 39.8 plugins call it from `Plugin::build`; the derive
  macros are just another caller). Descriptor-level invariants (pure vs exec
  pins, unique slugs) are enforced at registration.
- **Validation**: `validate_doc` (doc-local: edge types, exec rules, realm,
  fan-in) + `validate_refs` (cross-asset via `GraphResolver`: subgraph
  interface match, reference cycles). Unknown node types are errors but
  never data loss — docs load, render as "missing node", and re-save intact.
- **Migration** (`migrate.rs`): per-node-type version chains;
  `MigrationCtx::rename_pin` rewrites stored constants *and* incident edges.
  Golden fixtures under `node_graph/fixtures/` (`UPDATE_GRAPH_FIXTURES=1` to
  regenerate).
- **Macros** (`crates/node_graph_macros`): `#[derive(ScriptNode)]` /
  `#[derive(AnimationNode)]` generate `NodeDescriptor`s; optional
  `inventory`-based auto-registration backend (`register_inventory_nodes`).
- **Editor**: `editor/graph_editor.rs` (state, doc-local `GraphEditStack`
  undo with saved-cursor dirty) + `graph_editor_crusty.rs` (canvas UI on
  crusty-gui's `Canvas` pan/zoom primitive). Graph tabs mirror the
  MeshEditor pattern (`EditorTab::GraphEditor(key)`).

## Graph Execution (Task 45-A)

Graphs *run*. Plan and rulings:
[VULKANO-45A-GRAPH-EXECUTION-CORE.md](roadmap/VULKANO-45A-GRAPH-EXECUTION-CORE.md).

### Crate layout, and why

Three workspace crates outside the engine, because the interpreter has to be
able to leave:

| Crate | Owns | Depends on |
|---|---|---|
| `crates/node_graph_types` | documents, registry, descriptors, validation, migration, resolvers | serde, ron, `curve_asset` |
| `crates/node_graph_exec` | compiler (`Plan`), interpreter, node impls, `Effect` | `node_graph_types` |
| `crates/curve_asset` | `.curve` data + evaluation | serde, ron |

None of them can see Vulkan, `hecs`, or the editor. That is the M6 insurance
policy applied to scripting (D8): client-first execution today, the *same*
interpreter compiled into a SpacetimeDB module later. CI enforces it — the
three crates are checked standalone and for `wasm32-unknown-unknown`
alongside `game_shared`.

The engine's `engine/src/engine/node_graph/` is a re-export shim plus the two
pieces that genuinely belong to the engine build (derive-macro markers,
`inventory` auto-registration).

### Compile pipeline

`GraphDoc` → **validate** → **flatten** → `Plan` (`node_graph_exec::plan`):

1. `validate_doc_with` + `validate_refs` over the whole reference closure.
   Errors refuse compilation; warnings do not.
2. Subgraphs inline through their interface, reroutes splice away,
   unreachable nodes are pruned — none of the three exist at run time.
3. Data inputs resolve to an `InputSource` (constant, another node's output,
   a variable slot) so the interpreter never searches edges.
4. A Timeline's outputs are its `.curve`'s track names, so compilation needs
   the asset: `CurveCache` prefetches every `curve_refs()` before compiling,
   and the same copy is what the interpreter samples. One source, so the
   descriptor the editor draws, the plan the compiler builds and the values
   the runtime samples cannot drift.

`GraphPlanCache` caches by content-relative path; **failures are cached too**
(a broken graph must not recompile sixty times a second to fail identically).
Invalidation bumps a generation counter, so live instances restart.

### Execution

`GraphInstance` is plain serializable data: variables, an activation list,
per-node state, a seeded PRNG. A tick drains queued events in phase order and
runs each activation until it blocks or finishes.

- **Exec vs data**: an impure node fires, *pulls* its data inputs backward
  through pure chains, performs its effect, then names the exec output to
  continue on. Pure nodes are re-evaluated per firing (no cross-statement
  caching in v1).
- **Latents** (`Delay`) suspend an activation and resume it on a later tick,
  loop stack and all — one data structure, so a `Delay` inside a `ForLoop`
  serializes correctly by construction. A `Timeline` is not a latent: it is a
  per-node ticker driven once per tick in a fresh activation, independent of
  exec flow (Blueprint-style), so a waiting `Delay` never stalls it.
- **Budget**: every activation runs under a step budget; exceeding it halts
  the instance with an error naming the node, which is how an infinite
  `WhileLoop` is a reported bug rather than a hang.

### Effect seam

The interpreter never touches the world. It emits `Effect`s
(`SetPosition`, `SpawnPrefab`, `Log`, …) into a buffer; `GraphScriptRunnerSystem`
(`engine/src/engine/scripting/runner.rs`) applies them, non-structural first,
then spawns/despawns. `SpawnPrefab` hands back an **alias** immediately and the
runner binds it to a real entity before the owner's next tick, so a graph can
spawn something and act on the handle.

Registered as the `graph_scripting` plugin (Task 39.8) behind a default-on
`graph-scripting` Cargo feature — the first plugin to exercise the full export
policy, since disabling it genuinely strips the interpreter from a build.

### Trace / viz

Tracing is a generic parameter (`TraceSink`), so `NoTrace` costs nothing and a
standalone build contains zero trace symbols. Editor builds keep a bounded
`GraphTrace` ring per instance; `graph_exec_viz.rs` maps plan-space hits back
to document space (an inlined subgraph lights its host node) for wire pulses,
node rings and value hover.

### Determinism discipline

All nondeterminism is injected: time comes from the runner, RNG is a seeded
per-instance PRNG, iteration order over instances and events is stable. A
determinism test runs the same graph + seed + inputs twice and compares the
effect streams; it has been in CI since P2, so a node that breaks it fails
loudly rather than quietly.

## Plugin System (Task 39.8)

`engine/src/engine/plugins/` is the engine/game/extension boundary. Author
guide: [PLUGINS.md](PLUGINS.md). Plan and rulings:
`roadmap/VULKANO-39.8-PLUGIN-SYSTEM.md`.

### Two-tier model

**Tier 1 (shipped).** A plugin is Rust code compiled into the binary. Whether
it *runs* is decided at startup by `project.ron`. Toggling in the Plugin
Manager edits the manifest and prompts a restart — **restart-only, never a
rebuild**, because a shipped editor must be usable by people without a
toolchain. Dormant code in the editor binary is the accepted cost; shipped
*game* binaries strip via Cargo features at export.

**Tier 2 (out of scope, shape preserved).** Binary plugins over a C-ABI or
WASM seam at narrow extension points. Nothing here precludes it: registration
goes through runtime registry calls (never compile-time inventory as the only
path), plugin identity is a string id (never a `TypeId`), and the
`PluginContext` surface stays object-safe.

Rust has no stable ABI and the registry APIs are generic-adjacent, so a DLL
model for tier 1 is not available — this is the ecosystem consensus (Bevy's
model), not a shortcut.

### Trait and staged commit

```rust
pub trait EnginePlugin: Send + Sync {
    fn manifest(&self) -> PluginManifest;
    fn build(&self, ctx: &mut PluginContext) -> Result<(), PluginError>;
}
```

`build()` never touches live engine state. `PluginContext` is a **staging**
context: a scratch `Schedule`, a scratch `Resources`, a `StagedRegistry` of
node types/domain pins/migrations, and vectors of lifecycle callbacks, panels
and settings pages. `PluginSet` commits a plugin's stage only when `build()`
returns `Ok` — a plugin that fails half-way registers *nothing*.

Commit order is preflight-everything-then-mutate:

1. resource `TypeId` collisions (with core or an earlier plugin) → error;
2. panel / settings-page id collisions → error;
3. `NodeRegistry::merge_staged` — itself preflight-then-apply;
4. `Schedule::append_from` (preserves stage, run criteria, enabled flag and
   access descriptor; reassigns insertion order), `Resources::append_from`,
   then callbacks/panels.

Collision policy differs by what is at stake: a **node id** collision skips
that descriptor and records a warning (a user's graph document references it,
so the plugin ends "enabled with warnings" rather than failing), while
**migration keys**, **resources** and **panel ids** are hard errors.

### Manifest and activation

`PluginManifest` carries `id` (permanent slug), `name`, `version`,
`description`, `author`, `depends_on`, `origin` (`Engine`/`Project`), `kind`
(`Runtime`/`EditorOnly`), `internal`, `module_path` and `cargo_feature`.

`ProjectConfig.plugins: Vec<PluginEntry { id, enabled }>` is the manifest.
Absent id = enabled (batteries included). Entries naming nothing in this build
are **preserved, not errors** — a project moved between machines must not lose
its intent. Renames need an alias (`PluginSet::with_alias`); without one the
old id orphans and the new id defaults to enabled, silently reversing a user's
disable. Build order is a topological sort over `depends_on`; a dependency
hole (absent, disabled, or itself failed) fails the dependent with
`MissingDependency`.

### Lifecycle callbacks

Registration alone is not enough — some plugin work happens at *content*
moments. `ctx.on_world_loaded(cb)` runs after **every** world population, and
all five moments funnel through one helper
(`game_client/src/world_population.rs`): editor startup, editor scene open,
standalone `load_world`, benchmark load, play-mode stop. Play-mode *enter* is
deliberately not one — nothing is loaded there; it resyncs Rapier with
edit-mode transform edits.

A callback failure is recorded in `PluginSet::failures()` as well as returned,
so the Plugin Manager stops calling that plugin Enabled; the record stays
(its committed systems really are registered). Each content moment re-runs
every callback, so the previous moment's world-loaded failures are *replaced*
— a hook that succeeds again clears its own row. Editor policy is
surface-and-continue (console + manager), shipped-game policy is fatal.

`ctx.register_debug_draw(cb)` contributes to the debug overlay: the engine owns
*when* it runs, the plugin owns *what* it draws.

### Restart flow

The manager's **Relaunch now** saves `project.ron` + layout, spawns a
replacement process (which parks on the parent's process *handle*) and exits.
Because it is a process exit, it refuses outright — with a console error, the
banner left pending — while play mode is active (the edit-world snapshot would
be lost), while any scene is dirty (scenes are never auto-saved for you), or
while a build is running.

### Editor extension points

`ctx.register_panel(id, title, factory)` adds a dock panel
(`EditorTab::Plugin(id)` ↔ `"plugin:<id>"`), and
`ctx.register_settings_page(...)` adds a Project Settings page — the seam the
Plugin Manager itself is built on. Panels receive a per-frame `PluginPanelCtx`
(world, resources, play mode) rather than `&mut App`: both dispatch sites run
inside closures where `App` is already split into disjoint field borrows.

A `plugin:<id>` tab **always parses**, even when nothing registers that id, so
a restored layout degrades to a visible missing-panel placeholder instead of
silently reshaping itself.

### Export rule

Exports resolve activation at *build time* — a shipped game has no runtime
manifest and needs none. Builds pass
`--no-default-features --features <base + enabled runtime plugin features>`;
`--no-default-features` is mandatory because `default` contains the non-plugin
`hud`, which the base list carries back in
(`plugins::EXPORT_BASE_FEATURES`).

The feature list is recomputed each frame from the *edited* project config, so
a build is refused while `project.ron` is dirty ("Save project settings
(Ctrl+S) before exporting") — an export must never consume a toggle the user
never committed. Module publishes (`MpServer`) read no features and are not
blocked.

**`EditorOnly` plugins never contribute a feature, regardless of enabled
state** — Unreal's Editor-module-type behaviour. Enabled-but-unused runtime
plugins *do* ship (registration references the code, so no LTO can drop it);
the mitigation is visibility — the build dialog lists what is going in — not
magic.

### What is deliberately not plugin-registrable

Components in scenes (needs a name-keyed registry with
serialize/deserialize/inspect — a reflection-lite arc), asset types/importers,
and render passes. See `VULKANO-39.8-PLUGIN-SYSTEM.md` §D2.

## Animation at Scale (Task 41.5)

How hundreds of skinned characters run at frame rate, and where IK sits in
the frame. Plan and rulings:
[`VULKANO-41.5-ANIMATION-SCALE.md`](roadmap/VULKANO-41.5-ANIMATION-SCALE.md).
Runtime: `engine/src/engine/animation/` (components, `graph/runner.rs`,
`ik.rs`, `foot_placement.rs`); GPU side:
`rendering/3d/skinning.rs` + `deferred/deferred_renderer.rs`.

### Palette SSBO ring

All bone palettes for a frame live back-to-back in one large SSBO
(`SkinningBackend`), split into **4** regions; a draw addresses its palette
by a flat `palette_base` index. Four regions against the renderer's 3-slot
fence ring, deliberately: the render thread reclaims fences *lazily* — frame
N-3's fence is taken at the start of processing frame N's packet — so with 3
regions the main thread would wait on a reclaim that requires the very packet
it is building (deadlock at frame 3). One region of slack matches the real
reclaim point: frame N gates on frame N-4. Both indices derive from the one
packet counter (region `% 4`, fence slot `% 3`); no second counter exists.

- **Handshake**: `PaletteRingSync` — the render thread calls
  `mark_done(slot, seq)` when it reclaims a fence; `begin_frame(seq)` on the
  main thread blocks until frame N-4 is marked. A `PaletteSlotGuard` covers
  every packet consumed *without* a stored fence (no swapchain, recreate,
  acquire fail, render error), so the ring never stalls on an error path.
- **Growth**: a frame that outgrows a region allocates a new buffer on the
  spot (`max(needed, 2×current)`), copies the already-written span; old
  buffers stay alive via the Arcs in in-flight command buffers. Growth does
  **not** skip the ring wait (ruling R14): with two ring buffers behind one
  handshake, skipping is only sound if both were just replaced — and the
  wait is free in steady state anyway.
- **Upload gate** ("stable bases", ruling R6): the cursor always advances —
  every visible skeleton occupies its span in every frame's region (ruling
  R2: regions rotate, so presence is unconditional; `dirty` gates
  *evaluation*, never presence). Each region records `(entity, len,
  revision)` per write in frame order; a prefix-matching write with an
  unchanged `SkeletonInstance.revision` skips the memcpy. First divergence in
  a visit → the rest copies and re-records. O(1) per write, no hash map,
  self-heals on churn.

### Instanced draws

Skinned and unskinned meshes share the gbuffer/shadow vertex shaders, so
both ride one instanced path. A second SSBO ring (parallel cursor in
`SkinningBackend`, same 4 regions, same handshake) carries per-instance
`InstanceData { model, palette_base }` (80 B std430) addressed by
`gl_InstanceIndex`; unskinned meshes use `palette_base = 0` (identity at
element 0 of every palette region).

- **Batching** (`render_loop::prepare_mesh_data`): per (entity, submesh)
  records, stable-sorted and grouped by `(material_index, submesh index,
  material-set Arc ptr)` — one `draw_indexed(.., instance_count, ..,
  first_instance)` per batch. 300 characters × 2 submeshes collapse from
  ~600 draws per pass to single digits.
- **Per-pass ranges via visible prefix** (ruling R15): camera-visible
  instances sort first within a batch, so one contiguous metadata span
  serves both passes — camera draws `(first_instance, visible)`, shadow
  draws `(first_instance, total)` (shadow stays unculled, as before).
- **No push constants** in these passes: `view_projection` lives in a
  per-pass UBO (set 0 binding 1, one per ring slot, rewritten by the
  renderer — camera VP from `packet.view_proj`, shadow VP from the light).
  Set 0 is `{palette region, VP UBO, instance region}`, bound once per pass;
  descriptor sets are cached per slot and rebuilt only on ring growth. The
  editor thumbnail/preview paths (`thumbnail_vs.glsl`) keep their own
  push-constant shader with fresh one-off buffers — single-mesh draws,
  nothing to batch, no ring discipline needed.

### Two-phase FK, retained model space

`compute_palette` is two phases on `SkeletonInstance`: (1)
`compute_model_space` walks locals parent-before-child into a retained
`model_space: Vec<Mat4>` (allocated once, reused — zero steady-state
alloc); (2) `refresh_palette_from_model_space` does `palette[i] =
model_space[i] * inverse_bind[i]` and bumps `revision`. Retained model
space is the IK substrate and gives bone sockets
(`SkeletonInstance::socket(name)`) for free. The space is the mesh's local
**Y-up render space** — pre-inverse-bind, the same space `debug_draw.rs`
now reads directly instead of inverting binds. Phase 2 alone is the
re-entry point after IK edits model space.

### Parallel pose evaluation

`AnimGraphSystem::run` parallelizes only step 3 — the per-entity tick —
via rayon over `query_mut::<(&mut AnimGraphRuntime, &mut
SkeletonInstance)>().into_iter_batched(64).par_bridge()`. Everything
structural stays serial: stale-runtime removal, arming (plan compile, clip
prefetch — the only `AnimClipCache` mutation — component inserts), the
significance pre-pass, and IK target resolution. The parallel closure
touches only that entity's two components plus an immutable clip-cache
borrow; per-thread `PoseScratch` lives in a `thread_local`, reused across
frames. An `evaluating: AtomicBool` brackets the parallel region and
`arm()` debug-asserts it is clear — any new serial-side work must stay
outside that window, and nothing inside it may touch `Resources`. Anim
events stay per-entity (`rt.events`, refilled in place), so ordering is
deterministic regardless of rayon scheduling.

### Update-rate throttling

Machine tick, slot tick and event collection run **every frame** (cheap, no
sampling); only pose evaluation + palette recompute sit behind the gate, so
a short play-once can never start, fire and end invisibly. Significance
comes from a serial pre-pass reading `AnimViewInfo` (camera position +
frustum, Y-up, previous frame's camera; absent ⇒ full rate — tests, tools
and previews are unaffected): distance buckets with intervals `[1, 2, 4,
8]` frames, hysteresis, and entity-id stagger so buckets don't beat.
Off-frustum entities clamp to the slowest interval rather than freezing
(their shadows keep moving — there is no shadow frustum available
main-thread pre-system). Forced evaluation overrides the bucket: active
crossfade/transition, active play-once, an event fired this tick, first
visible frame, first frame after arming, and the external hook
(`throttle.force_eval_external`, set by serial systems — foot-lock edges
use it). Skipped frames hold the last pose; no interpolation in v1.

### IK pipeline

Frame order per entity, inside the eval gate: machine → blend trees →
play-once overlay → `compute_model_space()` → `apply_ik` →
`refresh_palette_from_model_space()` — one palette pass, one revision bump.

- **Solvers** (`animation/ik.rs`, pure, no ECS): two-bone analytic with a
  mandatory pole vector (degenerate pole falls back to the chain's current
  bend plane), and look-at with an angle clamp. Solvers replace rotation +
  translation only — animated scale is never touched. Weight blends in
  model space per edited bone (`blend_model`: slerp/lerp, scale kept).
- **Chains from the graph**: `PlanIkChain { name, bones, solver,
  weight_param, foot }` compiles from `anim_ik_chain` nodes on the machine
  canvas (pin-less, like play-once slots); bone names resolve to indices at
  arm time, refusals disable the runtime with an anchored message.
  `weight_param` is a declared Float, so states fade IK through the normal
  parameter contract; weight 0 skips the solve and all writes.
- **Targets**: `IkTargets` (world Z-up, keyed by chain name) is resolved to
  mesh Y-up model space in a serial pre-pass (`entity_render⁻¹ *
  zup_to_yup(target)`, `entity_render` from `TransformCache::get_render` —
  previous frame, the accepted render-path latency). That pre-pass is the
  only place IK touches `Resources`.
- **`apply_ik` order**: record each two-bone chain's *animated* tip, apply
  the pelvis offset (bone matrix + descendant re-walk) **before** the leg
  chains solve — so both feet can still reach — then per chain: solve,
  blend by weight, write model-space matrices, re-walk descendants from the
  unchanged animated locals. IK writes model space, never locals (the next
  evaluation overwrites locals anyway).
- **Foot placement** (`FootPlacementSystem`, serial, before
  `AnimGraphSystem`): configured on the IK Chain node itself (`foot`,
  `ankle_offset`, `pelvis` props — ruling R11); the system inserts
  `IkTargets` itself. Rays down from the recorded *pre-IK* foot position
  (a locked foot must not pin its own ray) via
  `PhysicsWorld::raycast_filtered` (normal + own-collider exclusion);
  effector = contact + normal × ankle_offset. Foot lock latches the contact
  on a `<chain>_down` anim event and releases on `<chain>_up`; both edges
  set `force_eval_external`. Pelvis drop measures against the entity's
  ground plane (not the oscillating animated foot height — ruling R12),
  clamped and smoothed; cosmetic only, the entity/collider never move.
  Raycasts run only in bucket 0; leaving the bucket removes the targets,
  releases locks and forces one eval so the pose snaps back to animated.

The `--stress-anim N --bench-secs S` flags on the standalone client spawn a
character crowd and write baseline metrics to
`.scratch/anim-scale/baseline-N.txt`; the acceptance numbers (300 @ 60 fps)
are pending the user's baseline capture, as is the P8 clip-layout decision
gated on them.

## Performance Profiling

The engine integrates puffin and Tracy for profiling:

```rust
crate::profile_function!();  // Profile entire function
crate::profile_scope!("render_meshes");  // Profile specific scope
```

View profiles with:
- **puffin_viewer**: Built-in Rust profiler UI
- **Tracy**: External profiler with detailed timeline

## Threading Model

Currently single-threaded with planned parallelization:

```
Main Thread:
├── Input polling
├── ECS systems update
├── Physics step
├── Render command recording
├── GUI update
└── Frame presentation
```

Future: System parallelization using rayon or custom scheduler.

## Memory Management

- **GPU Memory**: Managed by Vulkano allocators
- **Asset Caching**: Reference-counted with manual unload
- **ECS Storage**: Dense component arrays (hecs internals)
- **Frame Resources**: Ring buffer for per-frame allocations

## Error Handling

The engine uses `Result<T, E>` throughout:

```rust
// Recoverable errors use Result
pub fn load_texture(path: &Path) -> Result<Arc<ImageView>, AssetError>

// Critical errors (Vulkan init) may panic with context
let device = create_device(&instance)
    .expect("Failed to create Vulkan device");
```

## Play Mode Architecture (Planned)

See [VULKANO-24-PLAY-MODE.md](roadmap/VULKANO-24-PLAY-MODE.md) for the full spec.

### State Machine

```
Edit ──(Play)──> Playing ──(Pause)──> Paused
  ^                 │                    │
  └────(Stop)───────┴────(Stop)──────────┘
```

### Snapshot/Restore

- **Enter Play**: Serialize scene to in-memory RON string (reuses `save_scene` path)
- **Stop**: Clear world, deserialize from snapshot, rebuild physics
- **EntityGuid**: `uuid::Uuid` component on every entity for identity across restore
- **Selection**: Stored as GUID, remapped to new Entity handle after restore

### Run Criteria Integration

```
RunIfPlaying    → physics, gameplay systems
RunIfEditing    → editor-only systems
RunIfNotPaused  → systems that stop on pause
Always          → input, profiling, rendering
```

## Future Architecture Plans

- Parallel system execution (rayon, read/write access declarations)
- SparseSet storage and query caching
- Node-graph visual scripting (outputs Commands)
- EntityGuid-based parent references (replace name-based)
- Networking entity replication (via GUID)
