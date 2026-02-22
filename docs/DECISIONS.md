# Rust Game Engine - Architectural Decision Records

This document records significant architectural decisions made during engine development, including context, alternatives considered, and rationale.

---

## ADR-001: Z-Up Coordinate System

**Date**: 2024

**Status**: Accepted

### Context

Game engines use different coordinate systems. Common conventions:
- Y-up (Unity, Unreal, DirectX default)
- Z-up (Blender, 3ds Max, engineering/CAD)

Vulkan uses a Y-up, right-handed coordinate system with -Z forward.

### Decision

Use **Z-up** for all game logic, converting to Y-up only at render time.

- X = Forward (Red)
- Y = Right (Green)
- Z = Up (Blue)

### Rationale

1. **Blender compatibility**: Primary modeling tool uses Z-up
2. **Intuitive physics**: Gravity naturally points -Z
3. **Level design**: Top-down views show X-Y plane (ground)
4. **Industry standard**: Many simulation/engineering tools use Z-up

### Consequences

- Requires coordinate conversion at render boundary
- All transform math must be Z-up aware
- Gizmo library needed patching for Z-up support
- Team must understand both coordinate systems

### Implementation

Conversion handled in `render_adapter.rs` using basis change matrices:
```rust
pub fn world_matrix_to_render(world_zup: &Mat4) -> Mat4 {
    C * world_zup * C_inv
}
```

---

## ADR-002: Custom ECS Architecture (wrapping hecs)

**Date**: 2024

**Status**: Accepted

### Context

Multiple ECS libraries available for Rust:
- **bevy_ecs**: Full-featured, part of Bevy engine
- **specs**: Parallel-focused, complex
- **legion**: Fast, good ergonomics
- **hecs**: Minimal, lightweight

### Decision

Build a **custom ECS architecture** using hecs for low-level entity/component storage, with custom systems for Resources, Events, Stages, and Change Detection.

### Rationale

1. **Control**: Full control over ECS features without external constraints
2. **Learning**: Deep understanding of ECS internals
3. **Tailored**: Features specific to our engine needs (editor support, play mode)
4. **hecs as foundation**: Proven archetype storage, we add the rest
5. **Incremental**: Can evolve without breaking changes

### Architecture

```
Custom ECS Layer
├── Resources (global state)
├── Events (double-buffered)
├── Stages (First → PreUpdate → Update → PostUpdate → Last)
├── Schedule (system ordering, run criteria)
├── Change Detection (Added, Changed filters)
└── Transform Hierarchy (Parent/Children, world matrix caching)

hecs Layer (wrapped)
├── Entity storage
├── Component archetype storage
└── Query iteration
```

### Consequences

- More development effort than using bevy_ecs
- Full ownership of ECS behavior
- Can optimize for specific use cases
- Migration path to parallel execution planned

### Alternatives Considered

| Library | Pros | Cons |
|---------|------|------|
| bevy_ecs | Full-featured | Heavy, tightly coupled to Bevy |
| specs | Parallel | Complex setup, older design |
| legion | Fast | API changed frequently |
| Pure hecs | Simple | Missing Resources, Events, Stages |

---

## ADR-003: Vulkano over ash

**Date**: 2024

**Status**: Accepted

### Context

Two primary Vulkan binding options in Rust:
- **ash**: Thin, raw bindings (1:1 with C API)
- **Vulkano**: Safe, high-level wrapper

### Decision

Use **Vulkano** for Vulkan rendering.

### Rationale

1. **Safety**: Compile-time validation of Vulkan usage
2. **Productivity**: Less boilerplate than raw Vulkan
3. **Shader compilation**: Built-in GLSL compilation
4. **Memory management**: Automatic buffer/image allocation
5. **Learning**: Focuses on graphics concepts, not API details

### Consequences

- Some Vulkan features harder to access
- Version updates can have breaking changes
- Performance overhead (minimal in practice)
- Less control over memory allocation strategies

### Trade-offs Accepted

Raw Vulkan control sacrificed for development velocity. Can migrate to ash later if needed.

---

## ADR-004: Deferred Rendering Pipeline

**Date**: 2024

**Status**: Accepted

### Context

Rendering architecture choices:
- **Forward**: Simple, one pass per object
- **Forward+**: Tiled light culling
- **Deferred**: G-buffer, decoupled lighting

### Decision

Use **deferred rendering** with G-buffer.

### Rationale

1. **Many lights**: Lighting cost independent of geometry
2. **Post-processing**: G-buffer enables effects (SSAO, SSR)
3. **Editor tools**: G-buffer useful for selection, debugging
4. **Learning**: Common AAA technique worth implementing

### Consequences

- Higher memory bandwidth (multiple G-buffer textures)
- Transparency requires separate forward pass
- MSAA more complex (needs resolve or deferred MSAA)
- More complex pipeline setup

### G-Buffer Layout

| Attachment | Format | Contents |
|------------|--------|----------|
| RT0 | RGBA8 | Albedo RGB, Metallic A |
| RT1 | RGBA16F | World Normal XYZ, Roughness A |
| RT2 | RGBA16F | World Position XYZ, AO A |
| Depth | D32F | Depth buffer |

---

## ADR-005: egui for Editor UI

**Date**: 2024

**Status**: Accepted

### Context

UI framework options:
- **imgui-rs**: Mature, C++ bindings
- **egui**: Pure Rust, immediate mode
- **iced**: Elm-style, retained mode
- **Custom**: Build from scratch

### Decision

Use **egui** for all editor UI.

### Rationale

1. **Pure Rust**: No C++ dependencies
2. **Immediate mode**: Simple, stateless UI code
3. **Active development**: Frequent updates, good community
4. **Customizable**: Styling, custom widgets possible
5. **Integration**: Vulkano backend available

### Consequences

- Custom Vulkano integration needed (egui-vulkano crate)
- Some layout limitations vs retained mode
- Text rendering can be blurry at small sizes
- Needed to patch emath for DragValue crash

---

## ADR-006: RON for Scene Serialization

**Date**: 2024

**Status**: Accepted

### Context

Serialization format options:
- **JSON**: Universal, verbose
- **TOML**: Config-focused, limited nesting
- **YAML**: Human-readable, complex spec
- **RON**: Rust-native, concise
- **Binary**: Fast, not human-readable

### Decision

Use **RON** (Rusty Object Notation) for scene files.

### Rationale

1. **Rust alignment**: Syntax mirrors Rust structs
2. **Type information**: Can include struct names
3. **Concise**: Less verbose than JSON
4. **Human-readable**: Easy to hand-edit
5. **serde support**: First-class integration

### Example

```ron
(
    name: "Player",
    transform: (
        position: (x: 0.0, y: 0.0, z: 1.0),
        rotation: (x: 0.0, y: 0.0, z: 0.0, w: 1.0),
        scale: (x: 1.0, y: 1.0, z: 1.0),
    ),
)
```

### Consequences

- Less universal than JSON
- Tooling support not as widespread
- Need custom serde for nalgebra types

---

## ADR-007: Rapier for Physics

**Date**: 2024

**Status**: Accepted

### Context

Physics engine options:
- **Rapier**: Pure Rust, active development
- **nphysics**: Older Rust engine (deprecated)
- **PhysX bindings**: Industry standard, C++ dependency
- **Custom**: Educational but time-consuming

### Decision

Use **Rapier 3D** for physics simulation.

### Rationale

1. **Pure Rust**: No C dependencies, easy integration
2. **Modern design**: Good API, active maintenance
3. **Feature complete**: Collision, rigid bodies, joints
4. **Performance**: Comparable to PhysX for game use cases
5. **WASM support**: Future web export possible

### Consequences

- ECS sync needed (Rapier has own world)
- Some advanced features missing vs PhysX
- Documentation less extensive than PhysX

---

## ADR-008: Transform Gizmo Library

**Date**: 2024

**Status**: Accepted (with patches)

### Context

Need 3D transform manipulation gizmos for editor. Options:
- **transform-gizmo-egui**: Rust crate, egui integration
- **Custom implementation**: Full control
- **Port from other engine**: Complex

### Decision

Use **transform-gizmo-egui** with custom patches for Z-up support.

### Rationale

1. **Existing solution**: Saves months of development
2. **egui integration**: Works with our UI framework
3. **Feature complete**: Translate, rotate, scale, snap

### Patches Required

The library assumes Y-up. Forked to `crates/transform-gizmo`:
- Modified axis rendering for Z-up
- Adjusted rotation calculations
- Fixed snapping in Z-up space

### Consequences

- Must maintain fork
- Upstream updates require merge effort
- Alternative: Submit PR upstream (if accepted)

---

## ADR-009: Profiler Integration

**Date**: 2024

**Status**: Accepted

### Context

Profiling options:
- **puffin**: Pure Rust, simple
- **Tracy**: Powerful, external viewer
- **superluminal**: Windows-only, commercial
- **perf/VTune**: OS-level, complex setup

### Decision

Integrate both **puffin** and **Tracy**.

### Rationale

1. **puffin**: Quick iteration, built-in viewer
2. **Tracy**: Deep analysis, timeline view
3. **Complementary**: Use puffin for dev, Tracy for optimization

### Implementation

```rust
// Single macro expands to both
crate::profile_function!();
crate::profile_scope!("name");
```

### Consequences

- Compile-time feature flags for each profiler
- Small overhead when disabled (should be zero)
- Tracy requires external viewer installation

---

## ADR-010: Single-Threaded Architecture (Initial)

**Date**: 2024

**Status**: Accepted (temporary)

### Context

Threading model options:
- **Single-threaded**: Simple, deterministic
- **Task-based**: Job system, complex
- **Multi-threaded ECS**: Parallel queries

### Decision

Start **single-threaded**, plan for parallelization later.

### Rationale

1. **Simplicity**: Correct code first
2. **Debugging**: Easier without race conditions
3. **Learning**: Understand sequential flow first
4. **Performance**: Not yet bottlenecked

### Future Plan

See ADR-011 for planned advanced ECS with parallel systems.

### Consequences

- CPU-bound on complex scenes
- Simpler synchronization
- Will need refactoring for parallel

---

## ADR-011: Advanced ECS Architecture (Planned)

**Date**: 2025

**Status**: Proposed

### Context

Current hecs-based ECS lacks:
- Resources (global state)
- Events (entity communication)
- System stages (execution order)
- Change detection

### Decision

Implement custom ECS extensions over hecs. See [VULKANO-23.5-ADVANCED-ECS-ARCHITECTURE.md](roadmap/VULKANO-23.5-ADVANCED-ECS-ARCHITECTURE.md).

### Planned Features

1. **Resources**: Type-safe global state
2. **Events**: Double-buffered event queues
3. **Stages**: First → PreUpdate → Update → PostUpdate → Last
4. **Run Criteria**: Conditional system execution
5. **Change Detection**: Track component modifications

### Rationale

Need more ECS features for complex gameplay without switching to bevy_ecs.

---

## ADR-012: Play Mode Snapshot via In-Memory RON Serialization

**Date**: 2025

**Status**: Proposed

### Context

Play Mode requires saving scene state before simulation and restoring it on stop. Two approaches:

1. **In-memory RON serialization**: Reuse existing `save_scene`/`load_scene` code path, serialize to a String instead of a file
2. **In-memory world clone**: Clone hecs::World and all components directly

### Decision

Use **in-memory RON serialization** for Play Mode snapshot/restore.

### Rationale

1. **Proven code path**: Reuses existing, tested serialization logic
2. **No new code**: Minimal additional surface area
3. **Correctness**: Scene files already handle hierarchy, all component types
4. **Physics safety**: Rapier handles (`handle: None`) naturally reset on deserialize
5. **Clone approach is fragile**: Would require all components to impl Clone, doesn't handle Rapier handles

### Consequences

- Slightly slower than raw memory clone (serialize + deserialize)
- Acceptable for editor use (< 100ms for typical scenes)
- Can optimize later with binary format if needed
- Entity IDs change on restore (requires GUID mapping for selection)

---

## ADR-013: EntityGuid for Persistent Entity Identity

**Date**: 2025

**Status**: Proposed

### Context

`hecs::Entity` IDs are volatile — they change on every save/load cycle. Current entity identification relies on `Name` strings, which aren't guaranteed unique.

Play Mode restore, future networking, prefab instances, and robust undo/redo all need persistent entity identity.

### Decision

Add an **`EntityGuid(uuid::Uuid)`** component to every entity.

### Rationale

1. **Unique**: UUID v4 guarantees uniqueness
2. **Persistent**: Survives save/load/snapshot/restore
3. **Cross-cutting**: Benefits play mode, networking, prefabs, undo/redo
4. **Backward compatible**: Old scene files load fine (`serde(default)`)
5. **Industry standard**: Unity, Unreal, Godot all use persistent entity IDs

### Consequences

- Every entity spawn path must assign a GUID
- Scene format gains an optional `guid` field
- Small memory overhead per entity (16 bytes)
- Parent references still use names (GUID-based parents are a future migration)

---

## Decision Template

```markdown
## ADR-XXX: Title

**Date**: YYYY-MM

**Status**: Proposed | Accepted | Deprecated | Superseded

### Context

What is the issue that we're seeing that is motivating this decision?

### Decision

What is the change that we're proposing and/or doing?

### Rationale

Why is this change being made? What alternatives were considered?

### Consequences

What becomes easier or harder as a result of this change?
```
