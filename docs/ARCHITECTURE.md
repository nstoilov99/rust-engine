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

```
1. Begin Frame
   └── Acquire swapchain image

2. Shadow Pass (per shadow-casting light)
   └── Render depth-only to shadow map

3. G-Buffer Pass
   ├── Bind G-buffer framebuffer
   ├── For each entity with MeshRenderer:
   │   ├── Get world transform (hierarchy-aware)
   │   ├── Convert to render space (Z-up → Y-up)
   │   └── Draw to G-buffer
   └── Output: albedo, normal, position, material textures

4. Lighting Pass
   ├── Read G-buffer textures
   ├── Apply directional lights with shadows
   ├── Apply point lights
   ├── Apply ambient light
   └── Output: HDR color buffer

5. Compose Pass
   ├── Tone mapping (Reinhard)
   ├── Gamma correction
   └── Output to swapchain

6. GUI Pass
   ├── egui rendering
   └── Overlay on top of scene

7. End Frame
   └── Present swapchain image
```

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

```
┌─────────────────────────────────────────────────────────────────┐
│                         Menu Bar                                 │
├─────────────┬───────────────────────────────┬───────────────────┤
│             │                               │                   │
│  Hierarchy  │         Viewport              │    Inspector      │
│    Panel    │                               │      Panel        │
│             │   ┌───────────────────────┐   │                   │
│  - Entity   │   │                       │   │  - Transform      │
│    Tree     │   │     3D Scene View     │   │  - Components     │
│             │   │                       │   │  - Materials      │
│             │   │     + Gizmos          │   │                   │
│             │   │     + Grid            │   │                   │
│             │   └───────────────────────┘   │                   │
│             │                               │                   │
├─────────────┴───────────────────────────────┴───────────────────┤
│                         Console                                  │
│  - Log messages                                                  │
│  - Commands                                                      │
├─────────────────────────────────────────────────────────────────┤
│                      Asset Browser                               │
│  - File tree                                                     │
│  - Asset preview                                                 │
└─────────────────────────────────────────────────────────────────┘
```

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
