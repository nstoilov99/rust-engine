# Rust Engine

A modern 3D game engine built in Rust with Vulkan rendering, physically-based simulation, and data-driven architecture.

## Features

### Core Architecture
- **ECS (Entity Component System)**: Using `hecs` for high-performance entity management
- **Deferred Rendering Pipeline**: Two-pass rendering with G-Buffer (position, normal, albedo, material properties)
- **Physics Simulation**: Rapier3D integration with collision detection, rigid body dynamics, and debug visualization
- **Hot-Reload System**: Asset watching with automatic reload during development
- **Custom egui Integration**: Immediate-mode GUI system integrated with Vulkano 0.34

### Rendering
- **Vulkano 0.34**: Type-safe Vulkan wrapper
- **Deferred Shading**: Multi-target G-Buffer with separate geometry and lighting passes
- **GLTF Model Loading**: Support for complex 3D models with materials
- **Debug Visualization**: Wireframe rendering for physics colliders (cuboids, spheres)
- **Multiple View Modes**: Toggle between G-Buffer attachments for debugging (position, normal, albedo, depth)

### Physics
- **Rigid Body Dynamics**: Static and dynamic objects with proper mass/inertia calculation
- **Collision Detection**: Cuboid and sphere colliders with collision filtering
- **Collision Groups**: Bitmask-based filtering (Player, Enemy, Projectile, Environment, Sensor)
- **Event System**: Thread-safe collision event handling (Started/Stopped)
- **Fixed Timestep**: 60Hz physics updates with proper synchronization

### Scene Management
- **RON-based Scenes**: Human-readable scene format with hot-reload support
- **Component Serialization**: Transform, MeshRenderer, Camera, Lights serializable
- **Multiple Examples**: Demo scenes for ECS, serialization, and rendering features

## ⚠️ CRITICAL: Coordinate System Convention

**This project uses Z-up, X-forward coordinates (NOT Y-up!)**

```
X-axis = FORWARD (NOT Z!)
Y-axis = RIGHT
Z-axis = UP (NOT Y!)
```

### Why This Matters

Most game engines use Y-up (Unity, Unreal), but this engine uses Z-up (like Blender, CAD software). This affects:

**Correct (Z-up):**
- ✅ Height/elevation: `position.z = 10.0` (10 units up)
- ✅ Forward direction: **positive X-axis**
- ✅ Up vector: `Vec3::new(0.0, 0.0, 1.0)`
- ✅ Gravity: `Vec3::new(0.0, 0.0, -9.81)`
- ✅ Camera look direction: along X-axis

**Incorrect (Y-up - DO NOT USE):**
- ❌ Using `Vec3::Y` for up direction
- ❌ Assuming Y is vertical
- ❌ Gravity as `Vec3::new(0.0, -9.81, 0.0)`
- ❌ Forward as positive Z-axis

### Internal Y-up Conversion

Rapier3D physics engine uses Y-up internally. Conversions happen automatically:
- **Gameplay code**: Always use Z-up coordinates
- **Boundary conversions**: `convert_position_zup_to_yup()` handles render/physics boundaries
- **Utilities**: See [src/engine/coords.rs](src/engine/coords.rs) for conversion functions

## Getting Started

### Prerequisites

- **Rust**: 1.70 or later
- **Vulkan SDK**: Required for graphics (install from [vulkan.lunarg.com](https://vulkan.lunarg.com/))
- **GPU**: Vulkan 1.2+ capable graphics card

### Building

```bash
# Clone the repository
git clone <repository-url>
cd rust-engine

# Run the main demo
cargo run

# Run with optimizations (much faster)
cargo run --release

# Run specific examples
cargo run --example ecs_demo
cargo run --example test_gbuffer
```

### Controls

**Camera (3D Orbit)**:
- **Right Mouse**: Rotate camera around target
- **Middle Mouse**: Pan camera
- **Scroll**: Zoom in/out

**Debug Visualization**:
- **0-5 Keys**: Toggle G-Buffer debug views
  - 0: Final shaded output
  - 1: Position buffer
  - 2: Normal buffer
  - 3: Albedo buffer
  - 4: Material properties
  - 5: Depth buffer

**GUI**:
- **F1**: Toggle egui debug panel (camera info, entity inspector, physics controls)

## Project Structure

```
rust-engine/
├── src/
│   ├── engine/           # Core engine systems
│   │   ├── renderer/     # Vulkan rendering (deferred pipeline, shaders)
│   │   ├── physics/      # Rapier3D integration (debug_render, events)
│   │   ├── ecs/          # Entity-Component-System (components, physics)
│   │   ├── camera/       # Camera systems (camera_3d, input)
│   │   ├── assets/       # Asset loading & hot-reload (gltf, scenes)
│   │   ├── coords.rs     # Z-up ↔ Y-up coordinate conversions
│   │   └── ui/           # egui integration for Vulkano 0.34
│   ├── game/             # Game-specific code
│   │   └── main.rs       # Entry point, game loop, input handling
│   └── lib.rs            # Engine library exports
├── assets/
│   ├── models/           # GLTF 3D models
│   ├── textures/         # Image assets
│   └── scenes/           # RON scene definitions
├── shaders/              # GLSL shaders (geometry.vert, lighting.frag, etc.)
└── examples/             # Example programs demonstrating features
```

## Architecture

### ECS (Entity-Component-System)

Using `hecs` for high-performance data-oriented design:

**Components** ([src/engine/ecs/components.rs](src/engine/ecs/components.rs)):
- `Transform`: Position, rotation, scale (Z-up coordinates)
- `MeshRenderer`: References to mesh and material data
- `Camera`: FOV, near/far planes, active flag
- `DirectionalLight`: Direction, color, intensity

**Physics Components** ([src/engine/ecs/physics.rs](src/engine/ecs/physics.rs)):
- `RigidBody`: Dynamic/Static with mass, velocity, forces
- `Collider`: Shape (Cuboid/Ball), collision groups, offset

**Systems**:
- Physics simulation (60Hz fixed timestep)
- Render system (deferred pipeline)
- Input handling
- Asset hot-reload

### Rendering Pipeline

**Deferred Rendering** ([src/engine/renderer/deferred_renderer.rs](src/engine/renderer/deferred_renderer.rs)):

1. **Geometry Pass**: Render scene to G-Buffer (4 color attachments + depth)
   - Attachment 0: World-space positions (RGB)
   - Attachment 1: World-space normals (RGB)
   - Attachment 2: Albedo colors (RGB)
   - Attachment 3: Material properties (metallic, roughness, etc.)

2. **Lighting Pass**: Screen-space lighting using G-Buffer data
   - Directional lights
   - Point lights (future)
   - PBR material evaluation

**Advantages**:
- Decouples geometry from lighting (fewer draw calls)
- Supports many lights efficiently
- Easy to add post-processing effects

### Physics System

**Rapier3D Integration** ([src/engine/physics/mod.rs](src/engine/physics/mod.rs)):

- **Two-way ECS sync**: hecs World ↔ Rapier physics world
- **Fixed timestep**: 60Hz (16.67ms) for deterministic simulation
- **Collision filtering**: 5 predefined groups with bitmask filtering
- **Event handling**: Thread-safe collision callbacks (Mutex-based)
- **Debug rendering**: Wireframe visualization for all collider shapes

**Coordinate Handling**:
```rust
// Gameplay code uses Z-up
let position = Vec3::new(x, y, z);  // z = height

// Convert at physics boundary (internal)
let rapier_pos = convert_position_zup_to_yup(position);
```

### Scene Format

**RON (Rusty Object Notation)** - Human-readable, hot-reloadable:

```ron
(
    version: "1.0",
    name: "Main Scene",
    entities: [
        (
            name: "Main Camera",
            components: [
                (
                    type: "Transform",
                    position: (0.0, 5.0, 10.0),  // X, Y, Z (Z is height!)
                    rotation: (0.0, 0.0, 0.0, 1.0),
                    scale: (1.0, 1.0, 1.0),
                ),
                (
                    type: "Camera",
                    fov: 60.0,
                    near: 0.1,
                    far: 1000.0,
                    active: true,
                ),
            ],
        ),
    ],
)
```

## Development

### Hot-Reload Workflow

The engine supports hot-reloading for rapid iteration:

1. **Edit assets** (scenes, models, textures) while the game is running
2. **Save changes** - the engine detects modifications
3. **Assets reload automatically** without restarting

**Watched directories**:
- `assets/scenes/` - Scene definitions
- `assets/models/` - GLTF models
- `assets/textures/` - Image files

### Git Workflow

This project uses **vibe-kanban** for task management with git integration:

**Commit message format** (Conventional Commits):
```
feat: Add new physics collision groups
fix: Correct Z-up coordinate conversion in camera
refactor: Simplify deferred renderer pipeline
docs: Update README with architecture details
```

**Clean commits**:
- Git hooks automatically remove "Generated with Claude Code" footers
- Use `.githooks/squash-and-commit.sh` to squash multiple work-in-progress commits

### Building for Release

```bash
# Optimized build (3-5x faster than debug)
cargo build --release

# Run optimized binary
./target/release/game
```

**Release profile** (`Cargo.toml`):
- `opt-level = 3`: Maximum optimizations
- `lto = true`: Link-Time Optimization (slower compile, faster runtime)

### Debugging

**Physics Debug Rendering**:
```rust
// Toggle in egui panel or set directly
physics_debug.enabled = true;
physics_debug.draw_colliders = true;
```

**G-Buffer Visualization**:
Press number keys 0-5 to inspect rendering stages

**Logging**:
```bash
# Set Rust log level
RUST_LOG=debug cargo run
```

## Performance

**Target**: 60 FPS (16.67ms frame time)

**Typical frame breakdown**:
- Physics: ~2ms (fixed 60Hz)
- Rendering: ~8-10ms (deferred pipeline)
- ECS queries: <1ms
- Asset loading: Async (background threads)

**Optimization tips**:
- Use `cargo run --release` for accurate performance testing
- Profile with `cargo flamegraph` to identify bottlenecks
- Batch similar meshes to reduce draw calls

## Dependencies

**Core**:
- `vulkano` 0.34 - Safe Vulkan bindings
- `hecs` 0.10 - ECS framework
- `rapier3d` 0.25 - Physics engine
- `glam` 0.29 - Fast math library

**Windowing & Input**:
- `winit` 0.28 - Cross-platform window creation
- `egui` 0.33 - Immediate-mode GUI

**Assets**:
- `gltf` 1.4 - 3D model loading
- `image` 0.25 - Texture loading
- `ron` 0.8 - Scene serialization

**Utilities**:
- `notify` 6.1 - File watching (hot-reload)
- `tokio` 1.35 - Async runtime

## Contributing

When contributing, remember:

1. **Always use Z-up coordinates** in gameplay code
2. Follow Conventional Commits format
3. Run `cargo fmt` and `cargo clippy` before committing
4. Test physics changes with debug rendering enabled
5. Update documentation for new features

## License

[Add your license here]

## Acknowledgments

Built with:
- [Vulkano](https://vulkano.rs/) - Safe Vulkan wrapper
- [Rapier](https://rapier.rs/) - Physics engine
- [hecs](https://github.com/Ralith/hecs) - ECS framework
