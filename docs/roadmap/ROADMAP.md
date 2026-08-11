# Vulkano Task Roadmap

This document outlines the planned Task sequence for the Rust Game Engine project.

## Philosophy: Engine-First with 3D Priority

The Tasks follow an **engine-first approach** with **early 3D implementation** to maintain motivation and learn core concepts quickly. Systems (ECS, asset management, editor) are introduced when they solve real pain points, not theoretically.

---

## Completed Tasks

- ✅ **Task 1**: Vulkan Setup and Triangle
- ✅ **Task 2**: Rendering Pipeline
- ✅ **Task 3**: Textures and Performance
- ✅ **Task 4**: Sprites and Transforms
- ✅ **Task 5**: Camera and Input
- ✅ **Task 6**: Sprite Batching and Scene Management
- ✅ **Task 7**: Animation and Sprite Sheets
- ✅ **Task 8**: Pixel-Perfect Coordinates
- ✅ **Task 9**: 3D Rendering Basics
- ✅ **Task 10**: GLTF Models and 3D Asset Loading
- ✅ **Task 11**: Basic Lighting and Materials
- ✅ **Task 12**: PBR and Advanced Materials
- ✅ **Task 13**: Shadow Mapping
- ✅ **Task 14**: Asset Management System
- ✅ **Task 15**: Entity Component System (ECS)
- ✅ **Task 16**: Scene Serialization and Prefabs
- ✅ **Task 17**: Deferred Rendering Pipeline
- ✅ **Task 18**: Immediate Mode GUI (egui) Integration *(egui since replaced by in-house crusty-gui, 2026-07)*
- ✅ **Task 18.5**: egui Input Handling *(ditto)*
- ✅ **Tasks 19–39**: complete through the Visual Quality phase (incl. material-instance runtime wiring)

**Current focus:** Refactor Checkpoint #5 (slim) → **Phase M: Multiplayer Foundation (M0–M8)** → Networked Co-op Slice milestone.

---

## Phase 1: Core 3D Foundation (Tasks 9-11)

### Task 9: 3D Rendering Basics
**Status:** ✅ Complete
**Duration:** ~1 week
**Prerequisites:** Tasks 1-8

**What you'll learn:**
- 3D mesh representation (vertices, indices, normals)
- Perspective camera vs orthographic
- Depth buffer and Z-testing
- Model-View-Projection (MVP) matrices
- 3D vertex shaders
- Basic 3D primitive rendering (cubes, planes)

**Why now:**
- You already have rendering pipeline (Vulkan works)
- Camera system exists (just needs perspective mode)
- Transforms work for 3D (4x4 matrices)
- Get to 3D quickly (main goal)

**Key concepts:**
- Converting 2D renderer to support 3D
- Depth testing configuration
- 3D coordinate spaces (local, world, view, clip)

---

### Task 10: GLTF Models and 3D Asset Loading
**Status:** ✅ Complete
**Duration:** ~1 week
**Prerequisites:** Task 9

**What you'll learn:**
- GLTF file format structure
- Loading meshes with `gltf` crate
- Texture mapping in 3D (UV coordinates)
- Mesh instancing (batching for 3D)
- Managing multiple 3D models

**Why now:**
- Cubes are boring - need real models
- Understand asset complexity before building asset system
- Foundation for asset management (Task 14)

**Key concepts:**
- Vertex buffer layouts for complex meshes
- Material data from GLTF
- Transform hierarchies in models

---

### Task 11: Basic Lighting and Materials
**Status:** ✅ Complete
**Duration:** ~1 week
**Prerequisites:** Task 10

**What you'll learn:**
- Directional lights (sun)
- Diffuse lighting (Lambert)
- Specular highlights (Blinn-Phong)
- Normal mapping basics
- Light uniforms vs push constants
- Material system (metallic, roughness)

**Why now:**
- Make 3D look good (not flat)
- Foundation for PBR rendering
- Learn lighting concepts incrementally

**Key concepts:**
- Light calculations in fragment shader
- Normal transformation
- Ambient + diffuse + specular formula
- Lighting uniform buffers

---

## 🔄 Refactor Checkpoint #1

**After Task 11** - Reorganize codebase now that 2D and 3D coexist

**See:** [VULKANO-11.5-REFACTOR-CHECKPOINT-01.md](VULKANO-11.5-REFACTOR-CHECKPOINT-01.md)

### Goals:
- Separate 2D and 3D rendering code
- Clean folder structure
- Delete unused/obsolete code
- Consolidate common rendering logic

### Folder Structure:
```
src/
├── engine/
│   ├── core/                 # Vulkan, device, swapchain, context
│   ├── rendering/
│   │   ├── 2d/              # Sprite batch, 2D-specific pipelines
│   │   ├── 3d/              # Mesh batch, 3D-specific pipelines
│   │   ├── common/          # Shared renderer, pipeline utilities
│   │   └── shaders/         # All shader modules organized
│   ├── scene/               # Scene, entity, components
│   ├── camera/              # Camera2D and Camera3D
│   ├── input/               # Input manager
│   └── mod.rs
├── game/                     # Game-specific code (separate from engine)
│   └── main.rs
└── assets/                   # Textures, models, data files
```

### Code Cleanup:
- ❌ Delete `render()` - old single-sprite renderer
- ❌ Delete `render_sprite()` - non-batched version
- ❌ Delete test/demo code in main.rs
- ❌ Remove unused shaders (vs, fs, textured_vs if fully replaced)
- ✅ Keep: `render_sprite_batch()`, `render_mesh_batch()`

### Update All Tasks:
- Update Tasks 1-11 with new folder paths
- Fix import statements in examples
- Add migration guide for existing code

**Duration:** 4-6 hours

---

## Phase 2: Engine Systems (Tasks 12-16)

### Task 12: PBR and Advanced Materials
**Status:** ✅ Complete
**Duration:** ~2 weeks
**Prerequisites:** Task 11, Refactor #1

**What you'll learn:**
- Physically Based Rendering (PBR) theory
- Cook-Torrance BRDF implementation
- Tangent-space normal mapping
- Metallic-roughness workflow
- Multiple texture maps (albedo, normal, metallic-roughness, AO)
- GLTF material extraction

**Why now:**
- Industry standard rendering
- Make materials look realistic
- Foundation for advanced rendering

**Key concepts:**
- Energy conservation and Fresnel effect
- GGX distribution, Smith geometry, Schlick Fresnel
- Tangent vector calculation
- TBN matrix for normal mapping

---

### Task 13: Shadow Mapping
**Status:** ✅ Complete
**Duration:** ~1.5 weeks
**Prerequisites:** Task 12

**What you'll learn:**
- Shadow mapping fundamentals
- Directional light shadows (shadow maps)
- Point light shadows (cubemap shadows)
- Shadow map framebuffers and depth textures
- Percentage-closer filtering (PCF)
- Shadow bias and peter-panning
- Cascaded shadow maps (CSM) basics

**Why now:**
- Natural progression from lighting
- Shadows make scenes look realistic
- Required for believable 3D environments

**Key concepts:**
- Rendering from light's perspective
- Depth comparison in shaders
- Multi-pass rendering (shadow pass + main pass)
- Shadow acne and solutions

---

### Task 14: Asset Management System
**Status:** ✅ Complete
**Duration:** ~1.5 weeks
**Prerequisites:** Task 13

**What you'll learn:**
- Centralized resource loading (textures, models, audio)
- Resource caching and reuse (prevent duplicate loads)
- Asset handles and lifetimes
- Hot-reloading assets without restart
- Asset metadata and dependency tracking

**Why now:**
- You've manually loaded textures/models - felt the pain
- 3D models make caching critical (large files)
- Foundation for editor asset browser

**Key concepts:**
- `AssetManager<T>` generic system
- Handle-based resource access
- Reference counting vs ownership
- File watching for hot-reload

**Technologies:**
- `notify` crate for file watching
- `parking_lot` for thread-safe caching
- Custom `Handle<T>` type

---

### Task 15: Entity Component System (ECS)
**Status:** ✅ Complete
**Duration:** ~2 weeks
**Prerequisites:** Task 14

**What you'll learn:**
- ECS architecture (Entity, Component, System)
- Component storage strategies (SoA vs AoA)
- System scheduling and dependencies
- Entity queries and iteration
- Archetype-based storage

**Why now:**
- Current scene system is ad-hoc and limited
- 3D scenes get complex fast (many entities)
- Industry standard architecture
- Foundation for editor and serialization

**Key concepts:**
- Using `hecs` or `bevy_ecs` (don't roll your own)
- Migration from current `Scene` system
- Query patterns and filters
- System execution order

**Refactoring required:**
- Replace `Scene` with `World`
- Convert components to ECS components
- Rewrite rendering to use queries

---

### Task 16: Scene Serialization and Prefabs
**Status:** ✅ Complete
**Duration:** ~1 week
**Prerequisites:** Task 15

**What you'll learn:**
- Serialize/deserialize ECS world to RON/JSON
- Component reflection and registration
- Scene file format design
- Prefab system (reusable entity templates)
- Scene loading pipeline

**Why now:**
- Can't have editor without save/load
- Data-driven content starts here
- No more hardcoding entities in main.rs

**Key concepts:**
- `serde` for serialization
- Component registration macro
- Entity references in serialized data
- Prefab instantiation

**Technologies:**
- RON (Rusty Object Notation) for human-readable scenes
- `serde` and `typetag` for polymorphic components

---

## Phase 3: Advanced Rendering (Tasks 17-18)

### Task 17: Deferred Rendering Pipeline
**Status:** ✅ Complete
**Duration:** ~1.5 weeks
**Prerequisites:** Task 16, Refactor #2

**What you'll learn:**
- Deferred vs forward rendering
- G-buffer setup (position, normal, albedo, etc.)
- Light accumulation pass
- Handling many lights efficiently
- SSAO (Screen-Space Ambient Occlusion)

**Why now:**
- Forward rendering struggles with many lights
- Foundation for advanced effects
- Industry standard for complex scenes

**Key concepts:**
- Multiple render targets (MRT)
- G-buffer layout optimization
- Light volume rendering
- Transparency handling (forward + deferred hybrid)

---

### Task 18: Immediate Mode GUI (egui) Integration
**Status:** ✅ Complete
**Duration:** ~1 week
**Prerequisites:** Task 17

**What you'll learn:**
- Integrate egui into engine
- Render egui alongside 3D scene
- Basic windows, panels, buttons
- Custom Vulkan renderer for egui
- Texture upload with partial updates

**Why now:**
- Foundation for all editor UI
- Debug tools immediately useful
- Immediate mode is perfect for tools

**Key concepts:**
- Separate render pass for UI
- Command buffer chaining
- Modern egui API (not deprecated methods)

**Technologies:**
- `egui` crate
- `egui-winit` integration
- Custom Vulkan renderer for Vulkano 0.34

---

### Task 18.5: egui Input Handling
**Status:** ✅ Complete
**Duration:** ~1 day
**Prerequisites:** Task 18

**What you'll learn:**
- Full input handling for egui
- Mouse, keyboard, and scroll events
- Input consumption flags
- Dual-tracking with InputManager
- Modifier key handling

**Why now:**
- Make GUI interactive (not display-only)
- Prevent input conflicts between GUI and game
- Essential for editor functionality

**Key concepts:**
- Event collection vs processing
- `wants_keyboard` and `wants_pointer` flags
- Winit to egui event conversion
- Clean separation of GUI and game input

---

## Phase 4: Editor Development (Tasks 19-22)

### Task 19: Physics System Integration
**Status:** ✅ Complete (Rapier 3D 0.25 integrated)
**Duration:** ~1.5 weeks
**Prerequisites:** Task 18.5

**What you'll learn:**
- Integrate Rapier 3D physics engine
- Rigidbodies and colliders as ECS components
- Physics simulation step
- Raycasting and shape queries
- Physics debug visualization
- Collision events and callbacks

**Why now:**
- Most 3D games need physics
- Don't reinvent the wheel (use Rapier)
- Needed for gameplay before editor

**Key concepts:**
- Syncing physics world with ECS world
- Transform synchronization
- Collision layers and filtering
- Performance considerations

**Technologies:**
- `rapier3d` physics engine
- ECS integration patterns

---

## 🔄 Refactor Checkpoint #2

**After Task 19** - Performance and architecture review

### Goals:
- Profile and optimize hot paths
- Clean up deprecated systems
- Consolidate duplicate code
- Review ECS + Asset architecture

### Performance Pass:
```bash
cargo install cargo-flamegraph
cargo flamegraph
```

**Actions:**
- Identify CPU bottlenecks (flamegraph)
- GPU profiling with RenderDoc
- Optimize hot rendering paths
- Memory allocation analysis

### Code Quality:
- Remove any remaining deprecated code
- Extract common patterns into utilities
- Add inline documentation
- Run `cargo clippy` and fix warnings

### Architecture Review:
- Is ECS working well? Pain points?
- Asset manager efficient? Memory leaks?
- Scene graph vs flat ECS trade-offs

**Duration:** 8-12 hours

---

### Task 20: Scene Hierarchy Editor
**Status:** ✅ Complete
**Duration:** ~1 week
**Prerequisites:** Task 19

**What you'll learn:**
- Entity list view (tree structure)
- Add/remove entities at runtime
- Entity selection and highlighting
- Parent-child relationships (scene graph)
- Entity search and filtering

**Why now:**
- Core editor feature
- Navigate and manage scene contents
- Visual feedback for ECS data

**Key concepts:**
- Tree UI with egui
- Entity selection state
- Visual highlighting in viewport
- Drag-and-drop hierarchy reordering

---

### Task 21: Inspector and Property Editing
**Status:** ✅ Complete
**Duration:** ~1.5 weeks
**Prerequisites:** Task 20

**What you'll learn:**
- Component reflection to UI
- Edit transforms, meshes, materials live
- Undo/redo system
- Property validation and constraints
- Custom component editors

**Why now:**
- The "meat" of an editor
- Tweak anything without code
- Immediate visual feedback

**Key concepts:**
- Component trait for editor UI
- Command pattern for undo/redo
- Property change detection
- Type-safe property editing

---

### Task 22: Viewport and Gizmos
**Status:** ✅ Complete (transform-gizmo fork, Z-up)
**Duration:** ~1.5 weeks
**Prerequisites:** Task 21

**What you'll learn:**
- Separate editor camera from game camera
- Translate/rotate/scale gizmos (Unity-style)
- Grid rendering and snapping
- Multi-viewport support
- Object picking (mouse selection in 3D)

**Why now:**
- Visual editing is crucial
- Drag entities instead of typing coordinates
- Professional editor feel

**Key concepts:**
- Ray casting for object picking
- Gizmo rendering and interaction
- Camera controls (orbit, pan, zoom)
- Snapping and grid alignment

---

## Phase 5: Editor Polish (Tasks 23-25)

### Task 23: Asset Browser and Management UI
**Status:** ✅ Complete (asset_browser panel + thumbnails)
**Duration:** ~1.5 weeks
**Prerequisites:** Task 22

**What you'll learn:**
- Visual asset browser (thumbnails)
- Drag-and-drop assets into scene
- Asset import pipeline UI
- Metadata editing
- Asset tagging and search

**Why now:**
- Professional workflow
- See your assets visually
- Easier than typing file paths

**Key concepts:**
- Thumbnail generation
- File system watching UI
- Drag-and-drop from browser to viewport
- Asset preview rendering

---

### Task 24: Play Mode vs Edit Mode
**Status:** ✅ Complete
**Duration:** ~1.5-2 weeks
**Prerequisites:** Task 23.5 (Advanced ECS Architecture)

**See:** [VULKANO-24-PLAY-MODE.md](VULKANO-24-PLAY-MODE.md)

**What you'll learn:**
- Play/Pause/Stop state machine with snapshot/restore
- EntityGuid for persistent entity identity across save/load
- In-memory scene snapshot using existing RON serialization
- GUID-based selection restore after play mode exit
- Run criteria gating (RunIfPlaying, RunIfEditing, RunIfNotPaused)
- Physics world rebuild on snapshot restore
- Transition hooks (clear events/commands/ticks across mode changes)

**Why now:**
- Test without leaving editor
- Critical for iteration speed
- Industry standard workflow (Unity/Unreal Play-in-Editor)
- EntityGuid needed for future networking, prefab instances, undo/redo

**Key concepts:**
- Scene state snapshot to in-memory RON string
- State machine: Edit ↔ Playing ↔ Paused
- EntityGuid component (uuid::Uuid) for persistent identity
- GUID → Entity mapping for selection/hierarchy restore
- PlayModeManager orchestrates all transitions
- Clean transition: flush commands, clear events, rebuild physics

---

### Task 25: Build Pipeline and Export (Windows)
**Status:** ✅ Complete
**Duration:** ~1-1.5 weeks
**Prerequisites:** Task 24

**See:** [VULKANO-25-BUILD-PIPELINE.md](VULKANO-25-BUILD-PIPELINE.md)

**What you'll learn:**
- Feature-flag the editor so it compiles out entirely (`#[cfg(feature = "editor")]`)
- Standalone game binary that loads a scene and runs without editor UI
- Standalone render path (deferred pipeline direct to swapchain, no viewport texture)
- Content path resolution (relative to executable, not CWD)
- Export scripts for Windows (PowerShell + Bash)
- Cargo shipping profile (strip, LTO, codegen-units=1, panic=abort)
- Game camera entity vs editor fly camera

**Why now:**
- Ship your game as a standalone executable
- Editor code shouldn't exist in shipped builds
- Validates clean engine/editor separation
- Final step for complete engine workflow

**Key concepts:**
- Cargo feature flags (`editor` feature gates egui, panels, gizmos)
- Conditional compilation (`#[cfg(feature = "editor")]` on struct fields and modules)
- Two render paths: editor (viewport texture + egui) vs standalone (direct to swapchain)
- Content bundling: loose files next to exe (no asset packing yet)
- Build profiles: dev, release, shipping

---

## 🔄 Refactor Checkpoint #3: Performance & Polish
**Status:** ✅ Complete
**Duration:** ~1-2 weeks

**See:** [VULKANO-25.5-REFACTOR-CHECKPOINT-3.md](VULKANO-25.5-REFACTOR-CHECKPOINT-3.md)

**Focus areas:**

### Critical CPU Performance:
- Remove synchronous GPU texture upload blocking (gui/renderer.rs `.wait(None)`)
- Cache world transforms (transform propagation system instead of per-frame hierarchy traversal)
- Engine is CPU-bound: Ryzen 9950x + 1050 Ti gets 5x more FPS than RTX 5080 + 3900x

### Editor UI Optimization:
- Hierarchy panel virtualization (currently O(n) every frame for all entities)
- Console message count caching + cap unbounded growth
- Inspector component existence caching
- String allocation reduction in hot paths (~150+ format!/clone() in per-frame code)

### Macro-Driven Code Reduction:
- `component_inspector!` macro (~750 lines removed from inspector panel)
- Pipeline creation macro (~400 lines across 8+ pipeline files)
- Console command registration macro
- GLM serde helpers

### Code Quality:
- `unwrap()` audit in production code
- Profile macro coverage on all render/system/asset functions
- System independence audit (prep for Task 27 multi-threading)
- Feature flag cleanliness verification

### Documentation:
- Update ARCHITECTURE.md with CoreApp/EditorApp, RenderTarget
- Add performance/profiling section to KNOWLEDGE.md
- Update roadmap status

---

## Phase 6: Measure, Fix, Prove (Tasks 26-29)

### Task 26: Performance Profiling & Baselines
**Status:** ✅ Complete
**Duration:** ~1 week
**Prerequisites:** Refactor Checkpoint #3

**What you'll build:**
- Repeatable CPU/GPU benchmarking pipeline
- Standardized stress-test scene (500+ entities, multiple lights, shadows)
- Frame-time budgets and memory targets
- Baseline measurements that all future tasks are compared against

**What you'll learn:**
- GPU profiling with RenderDoc (capture, analyze draw calls, identify overdraw)
- CPU profiling with flamegraphs (identify hot functions, allocation pressure)
- Tracy integration for frame-level timeline analysis
- Memory profiling and leak detection patterns
- How to build reproducible benchmark scenes

**Why now:**
- Measure before optimizing — you can't improve what you can't measure
- Establish baselines before threading and new systems add complexity
- Identify whether the engine is CPU-bound or GPU-bound (Refactor #3 suggests CPU-bound)
- Learn profiling tools that will be used throughout the rest of the roadmap

**Key concepts:**
- RenderDoc GPU capture and draw call analysis
- `cargo-flamegraph` for CPU hot-path identification
- Tracy frame timeline and zone profiling
- Performance regression detection
- Budget allocation: how much time each system gets per frame

**Technologies:**
- `cargo-flamegraph` for CPU profiling
- RenderDoc for GPU profiling
- `tracy-client` (already integrated) for timeline profiling
- `puffin` (already integrated) for in-engine profiling UI

---

### Task 27: Performance Optimization & Frustum Culling
**Status:** ✅ Complete (frustum culling in render path)
**Duration:** ~1.5 weeks
**Prerequisites:** Task 26

**What you'll build:**
- Transform propagation system (cached world matrices, dirty flags)
- CPU-side frustum culling (your `math/frustum.rs` already exists — wire it up)
- Async GPU texture uploads (remove blocking `.wait(None)` in gui/renderer.rs)
- String allocation reduction in hot paths (~150+ format!/clone() identified in Refactor #3)
- Draw call batching improvements

**What you'll learn:**
- Transform hierarchy caching strategies (dirty flag propagation vs full BFS)
- Frustum extraction from view-projection matrix
- AABB-frustum intersection testing
- How to identify and eliminate per-frame allocations
- Batch sorting strategies (by material, by mesh, by distance)

**Why now:**
- Act on concrete findings from Task 26 profiling
- Transform caching eliminates redundant per-frame hierarchy traversal
- Frustum culling prevents submitting invisible geometry for every subsequent task
- These are foundational optimizations that benefit all future rendering work

**Key concepts:**
- Frustum planes extraction from MVP matrix
- AABB vs frustum culling (cheap, effective for most cases)
- Cache-friendly iteration patterns
- Avoiding allocations in hot loops (reuse buffers, pre-allocate)

---

### Task 28: Automated Testing & CI Pipeline
**Status:** ✅ Complete (`.github/workflows/ci.yml`)
**Duration:** ~1 week
**Prerequisites:** Task 27

**What you'll build:**
- ECS unit test suite: spawn, despawn, query, hierarchy operations, component add/remove
- Serialization round-trip tests: save scene to RON → load back → compare equality
- Physics synchronization tests: ECS transform ↔ Rapier body consistency
- Play-mode snapshot/restore verification: enter play → modify scene → stop → verify original state restored
- Scene smoke test: create a scene programmatically, render one frame, assert no crash
- CI pipeline: `cargo build --features editor && cargo build && cargo test` on every push
- `cargo clippy` enforcement in CI

**What you'll learn:**
- Testing strategies for game engines (deterministic vs non-deterministic systems)
- Round-trip serialization testing patterns
- Headless rendering for CI (or skipping GPU tests gracefully)
- GitHub Actions or similar CI configuration for Rust projects
- How to test ECS systems in isolation

**Why now:**
- Safety net before threading (Task 32), animation (Task 30), and audio (Task 31) add complexity
- Serialization bugs are brutal to diagnose without round-trip tests
- CI catches broken builds before they accumulate — every task after this benefits
- Play-mode snapshot/restore is fragile and needs automated verification

**Key concepts:**
- `#[cfg(test)]` module organization
- Mocking strategies for systems that need Vulkan context
- Deterministic test scenes (fixed seed, fixed entity layout)
- CI pipeline design for Rust projects

---

### Task 29: Debug Draw System
**Status:** ✅ Complete (`engine/src/engine/debug_draw/`)
**Duration:** ~3-4 days
**Prerequisites:** Task 27

**What you'll build:**
- Immediate-mode debug drawing API: `debug_draw.line()`, `debug_draw.sphere()`, `debug_draw.box()`, `debug_draw.arrow()`, `debug_draw.text_3d()`
- Depth-tested mode (occluded by scene geometry) and overlay mode (always visible)
- Automatic per-frame clearing
- Color and lifetime parameters (persistent lines for multi-frame visualization)
- Works in both editor and runtime debug builds (`#[cfg(debug_assertions)]`)
- Efficient batched line rendering (single draw call for all debug lines)

**What you'll learn:**
- Line rendering pipeline (line list primitive topology)
- Sphere/box wireframe generation from primitives
- Billboard text rendering in 3D space
- Conditional compilation for debug-only features
- How to design an API that's pleasant to use in every subsystem

**Why now:**
- Every subsequent task benefits: frustum bounds (27), physics shapes (existing), animation bones (30), audio ranges (31), culling results (39), particle emitters (38), terrain chunks (46), navmesh (56), network entity states (59)
- Small investment (~3 days), permanent payoff
- Debugging 3D systems without visualization is painfully slow

**Key concepts:**
- Immediate-mode API design (no retained state, no handles)
- Batched line rendering with a single vertex buffer
- Debug rendering as a separate render pass (after main scene, before UI)

---

### Task 29.5: Multi-Format Model Import (FBX + OBJ)
**Status:** ✅ Complete (ufbx 0.10, tobj 4.0)
**Duration:** ~3-4 days
**Prerequisites:** Task 29

**What you'll build:**
- FBX import via `ufbx` crate (single C file compiled via `cc`, no CMake/system libs)
- OBJ import via `tobj` crate (pure Rust, lightweight)
- Per-format adapter layer: ufbx scene / tobj output → existing `LoadedMesh` pipeline
- FBX bone and animation data extraction (skeleton hierarchy, keyframes, bind poses) for Task 30
- Import-time coordinate system handling (ufbx handles FBX axis conversion automatically)
- Asset browser integration: preview and import for FBX and OBJ files
- glTF/GLB continues using the existing `gltf` crate directly (no change to current path)

**What you'll learn:**
- ufbx scene graph (meshes, materials, bones, skin deformers, animation stacks)
- FBX coordinate system and unit scale conventions (handled by ufbx `target_axes`)
- OBJ/MTL parsing and material mapping
- Bridging format-specific mesh/material data into an existing asset pipeline
- Splitting a monolithic loader into a format-dispatch architecture

**Why now:**
- FBX is the de facto exchange format for game characters (Mixamo, Unreal exports, Maya, 3ds Max)
- OBJ is ubiquitous for simple static meshes and kitbash assets
- Task 30 (Skeletal Animation) benefits immediately — animated FBX characters work from day one
- The asset browser already lists `.fbx` and `.obj` as recognized types but loading silently fails
- FBX + OBJ + existing glTF covers ~95% of game assets in practice
- Minimal C footprint — ufbx is a single 27K SLoC C file, not a C++ build system

**Key concepts:**
- Format-specific adapters (not a universal importer): each format has a thin conversion layer into `LoadedMesh`
- ufbx handles FBX complexity (pre-rotation, pivot points, axis conversion, unit scaling)
- glTF fast path preserved (existing `gltf` crate, no overhead added)
- Coordinate normalization at import time, not runtime

**Technologies:**
- `ufbx` crate (single C file FBX loader, compiled via `cc` — used by Bevy)
- `tobj` crate (pure Rust OBJ/MTL parser)
- Existing `model_loader.rs` pipeline split into format-specific adapters
- Existing `asset_type.rs` already recognizes FBX/OBJ extensions

---

### Task 29.5c: 3D Rendered Asset Thumbnails
**Status:** ✅ Complete (`asset_browser/thumbnail_renderer.rs`)
**Duration:** ~1-2 days
**Prerequisites:** Task 29.5a

**What you'll build:**
- Offscreen Vulkan render pass for generating asset thumbnails (independent of swapchain)
- Camera auto-framing using LoadedMesh bounding sphere (fits any model automatically)
- Simplified forward render (albedo + basic lighting — no full deferred pipeline needed)
- Pixel readback from GPU framebuffer to CPU image for egui thumbnail display
- Replaces current texture-extraction hack with actual 3D renders (like Unreal/Unity/Godot)
- Works for all model formats equally (glTF, FBX, OBJ)
- Foundation for skeleton wireframe and animation pose previews (after Task 30)

**What you'll learn:**
- Offscreen Vulkan rendering (framebuffer creation without swapchain)
- GPU → CPU pixel readback patterns
- Camera auto-framing from bounding geometry
- Simplified forward shading pass

**Why now:**
- Current thumbnail system extracts first embedded texture (glTF-only, not representative)
- 3D rendered thumbnails are the industry standard (Unreal, Unity, Godot, Blender)
- All model formats benefit equally — no per-format thumbnail logic needed
- LoadedMesh bounding sphere data is already available from the loader pipeline
- Sets up skeleton/animation preview thumbnails for Task 30

**Key concepts:**
- Small offscreen framebuffer (128x128 or 256x256)
- Fixed camera angle with auto-framing (orbit position based on bounding sphere radius)
- Forward pass with basic directional light (not full deferred — thumbnails don't need GBuffer)
- Async-friendly: thumbnail render can happen on background thread with dedicated Vulkan resources

**Technologies:**
- Vulkano offscreen framebuffer + render pass
- Existing Vertex3D / PBR vertex shader (simplified fragment shader for thumbnails)
- Existing bounding sphere computation from LoadedMesh

---

## Phase 7: Core Gameplay Systems (Tasks 30-31)

### Task 30: Skeletal Animation & Multi-Format Playback
**Status:** ✅ Complete (skeletal animation + GPU skinning)
**Duration:** ~2 weeks
**Prerequisites:** Task 29, Task 29.5

**What you'll build:**
- Bone/joint hierarchy loading from glTF and FBX files (via Task 29.5's format adapters)
- GPU skinning in the vertex shader (bone matrices as uniform buffer)
- Animation clip playback with timeline scrubbing in the editor
- Basic crossfade blending between two clips (walk → run transition)
- Bone debug visualization using Task 29's debug draw (wireframe skeleton overlay)
- Animation clip asset type integrated with asset browser
- FBX animation import: Mixamo characters and animation packs load directly

**What you'll learn:**
- Skeletal animation fundamentals (bind pose, inverse bind matrices, joint transforms)
- GPU skinning: vertex shader bone matrix palette, per-vertex bone weights/indices
- glTF and FBX animation data structures (channels, samplers, interpolation modes)
- Animation blending math (quaternion slerp, position lerp, weight blending)
- Skinned mesh rendering pipeline modifications
- Handling format differences between glTF and FBX bone hierarchies

**Why now:**
- Biggest missing feature for making actual games — characters can't move without this
- Stress-tests the asset pipeline (animations are complex multi-part assets)
- Foundation for animation state machines (Task 41)
- glTF already loaded by the engine — animation data is there, just not used
- FBX support from Task 29.5 means Mixamo and game-store assets work out of the box

**Key concepts:**
- Bone matrix palette uploaded as uniform buffer
- Vertex attributes: bone indices (ivec4) + bone weights (vec4)
- Animation sampling: keyframe interpolation (step, linear, cubic spline)
- Skeleton hierarchy traversal for computing world-space bone matrices
- Skinning equation: `finalPos = sum(weight[i] * boneMatrix[i] * vertexPos)`
- Format-agnostic skeleton: glTF and FBX bones normalized to same internal representation

**Technologies:**
- `gltf` crate (already used — animation accessor support)
- `ufbx` (FBX animation/bone data via Task 29.5's format adapter)
- Modified PBR vertex shader for skinning
- Uniform buffer for bone matrix palette

---

### Task 31: Audio System (Kira)
**Status:** ✅ Complete (kira 0.12)
**Duration:** ~1.5 weeks
**Prerequisites:** Task 30

**What you'll build:**
- Kira audio engine integration
- 3D spatial audio with listener (camera) and emitter (entity) ECS components
- `AudioListener` component (attached to camera entity)
- `AudioEmitter` component (position, attenuation curve, max distance)
- Music track playback with crossfade between tracks
- Sound effect playback (one-shot and looping)
- Volume buses: master, music, SFX, ambient (hierarchical mixing)
- Fade in/out and crossfade transitions
- Asset pipeline integration: audio files (.wav, .ogg, .mp3) in asset browser with preview playback
- Audio debug visualization: emitter range spheres using debug draw

**What you'll learn:**
- Real-time audio engine architecture
- 3D spatial audio math (distance attenuation, panning)
- Audio bus/mixer architecture (hierarchical volume control)
- Streaming vs pre-loaded audio trade-offs
- Audio asset management (format support, caching, memory)

**Why now:**
- Audio is the single most impactful "game feel" feature after animation
- Self-contained system with clean ECS integration
- Kira is pure Rust, actively maintained, and designed for games
- A scene with spatial audio feels alive; without it, nothing does

**Key concepts:**
- Distance-based attenuation models (linear, inverse, exponential)
- Listener-relative positioning for stereo panning
- Audio bus hierarchy (SFX bus → master bus)
- Streaming large audio files vs loading small SFX into memory
- Audio component lifecycle (play on spawn, stop on despawn)

**Technologies:**
- `kira` crate for audio engine
- `.wav`, `.ogg`, `.mp3` format support
- ECS integration via `AudioEmitter` and `AudioListener` components

---

## 🔄 Refactor Checkpoint #4: System Independence Audit
**Status:** ✅ Complete
**Duration:** ~3-5 days

**Goals:**
- Audit all systems for shared mutable state — identify any system that directly mutates another system's data
- Verify command buffers are the sole mutation path for structural ECS changes
- Ensure no system holds references across frame boundaries
- Map read/write access patterns for every system (preparation for Task 32)
- Full test suite pass (Task 28)
- Profile against Task 26 baselines to measure cumulative gains from Tasks 27-31

**Why now:**
- Task 32 (System Access Declarations) formalizes what this audit discovers
- Animation and audio just added new systems with their own access patterns
- Threading readiness starts here — hidden shared state becomes race conditions later

---

## Phase 8: Architecture & Boundaries (Tasks 32-34)

### Task 32: System Access Declarations & Scheduler Skeleton
**Status:** ✅ Complete (`ecs/access.rs`, sequential scheduler)
**Duration:** ~1 week
**Prerequisites:** Refactor Checkpoint #4

**What you'll build:**
- System access declaration API: every system declares what components/resources it reads and writes
- Sequential scheduler that validates declarations at startup — errors loudly on conflicts
- Compile-time or init-time verification that no two systems in the same stage write to the same component without synchronization
- No actual threading — this is the contract, not the execution
- Migration of all existing systems to declare their access patterns

**What you'll learn:**
- System access modeling (read sets, write sets, exclusive access)
- Scheduling algorithms (topological sort based on dependencies)
- How Bevy-style SystemParam declarations work conceptually
- Conflict detection strategies
- How to design an API that's easy to use but impossible to misuse

**Why now:**
- Every system written after this point is forced to declare its access patterns
- This is the single most important architectural decision for threading — getting it wrong means a massive retrofit at Task 58
- Refactor #4 just mapped all the access patterns — formalize them now while the knowledge is fresh
- The scripting API (Task 34) and node graph execution (Task 45) depend on this contract

**Key concepts:**
- Read/write access sets per system
- Stage-based scheduling (existing: First → PreUpdate → Update → PostUpdate → Last)
- Conflict detection: two systems writing the same component type in the same stage = error
- Exclusive access: some systems need sole access to the world (scene loading, snapshot/restore)

**Architecture:**
```rust
// Every system declares its access
schedule.add_system(
    FunctionSystem::new("movement", movement_system)
        .reads::<Transform>()
        .reads::<Velocity>()
        .writes::<Transform>(),
    Stage::Update,
);

// Scheduler validates at startup:
// "movement" and "physics_sync" both write Transform
// → ERROR if in same stage without ordering constraint
```

---

### Task 33: Input Action System & Gamepad
**Status:** ✅ Complete (input actions + gilrs 0.11)
**Duration:** ~1.5 weeks
**Prerequisites:** Task 32

**What you'll build:**
- Action-based input mapping: logical actions ("Jump", "Move", "Attack") mapped to physical inputs
- Input contexts that stack: gameplay, UI, vehicle, swimming — only the top context receives input
- Gamepad support via `gilrs` crate (Xbox, PlayStation, generic controllers)
- Rebindable controls with serialization (save/load input mappings to RON)
- Dead zones, sensitivity curves, and input smoothing for analog sticks
- Editor panel for authoring input maps (visual binding editor)
- Action types: button (pressed/released), axis (analog -1..1), axis2D (stick vector)
- Modifier support (Ctrl+Click, Shift+Move)

**What you'll learn:**
- Input abstraction layers (physical → logical mapping)
- Context-based input routing (modal input handling)
- Gamepad API and platform differences
- Input serialization for user preferences
- Dead zone and response curve math

**Why now:**
- Scripting (Task 34), animation state machines (Task 41), and AI (Task 56) all need to consume input
- Building them against raw `is_key_pressed(Key::W)` means rewriting when actions arrive
- Gamepad support requires action abstraction — raw key checks can't represent analog input
- Written against Task 32's access declarations from day one

**Key concepts:**
- `InputAction` enum: Button, Axis, Axis2D
- `InputContext` with priority stacking (UI context blocks gameplay context)
- `InputMapping`: serializable binding from action to physical input(s)
- Multiple bindings per action (keyboard + gamepad simultaneously)
- Input consumption: once an action is consumed by a context, lower contexts don't see it

**Technologies:**
- `gilrs` crate for gamepad input
- Existing `winit` for keyboard/mouse (already integrated)
- RON serialization for input map persistence

---

### Task 34: Game Logic Architecture & Crate Boundaries
**Status:** ✅ Complete (engine / game_shared / game_client split)
**Duration:** ~1-1.5 weeks
**Prerequisites:** Task 33

**What you'll build:**
- `game_shared` crate: shared types, commands, validation rules, data definitions (used by both client and future server)
- `game_client` crate: client-side gameplay systems, presentation logic, engine integration
- Clean trait-based API boundary between engine and gameplay (`GamePlugin` trait or similar)
- Command/event interface that all authoring systems (visual scripting, node graphs) use to interact with the engine
- Crate dependency graph optimized for incremental compilation
- Optional: dynamic library hot-reload for `game_client` during development (dylib + `libloading`)

**What you'll learn:**
- Rust workspace and crate boundary design
- Trait-based plugin architectures
- Command pattern as a universal execution interface
- Incremental compilation optimization through crate splitting
- Hot-reload strategies on Windows (dylib challenges: file locking, PDB)

**Why now:**
- Every system after this builds against the crate boundary — visual scripting issues commands through this interface, SpacetimeDB consumes `game_shared` types
- Clean separation prevents gameplay code from taking hidden dependencies on engine internals
- Incremental compile times improve immediately (game logic changes don't recompile the renderer)
- The command/event interface defined here is what makes networking transparent to gameplay code later

**Key concepts:**
- `game_shared`: pure data, no engine dependencies, compiles independently
- `game_client`: depends on engine + game_shared, contains presentation and client-side prediction
- Future `game_server_stdb`: depends on game_shared + SpacetimeDB, no engine dependency
- All gameplay mutations go through commands — never raw world access from gameplay code

**Architecture:**
```
engine/          → Rendering, ECS, physics, audio, editor (library)
game_shared/     → Types, commands, rules, validation (pure Rust, no engine deps)
game_client/     → Client gameplay, presentation, engine integration
game_server_stdb/ → (Future Task 59) SpacetimeDB reducers, authority
```

---

## Phase 9: Rendering Architecture (Tasks 35-36)

### Task 35: Render Graph / Frame Graph
**Status:** ✅ Complete (7/7 tests pass)
**Duration:** ~1.5-2 weeks
**Prerequisites:** Task 34

**What you'll build:**
- Declarative render pass graph: passes declare input textures (read) and output textures (write)
- Automatic pass ordering based on data dependencies
- Automatic transient resource lifetime management (textures created and destroyed within a frame)
- Current hardcoded pipeline (shadow → G-buffer → lighting → compose → GUI) expressed as graph nodes
- API for adding/removing/reordering passes at runtime
- Pass culling: unused passes automatically skipped
- Resource aliasing: transient textures can share memory if lifetimes don't overlap

**What you'll learn:**
- Frame graph architecture (Frostbite-style render graph)
- Dependency-driven pass scheduling
- GPU resource lifetime management
- How modern engines decouple render features from the main loop
- Vulkan render pass and subpass dependencies

**Why now:**
- Post-processing (Task 37) needs to plug in as graph nodes, not hardcoded passes
- Compute passes for particles (Task 38) need explicit resource dependencies
- Render thread separation (Task 36) is cleaner when pass dependencies are explicit
- Every advanced rendering feature (SSAO, SSR, bloom) becomes a pluggable node instead of spaghetti

**Key concepts:**
- Pass = function + declared inputs + declared outputs
- Resource = texture or buffer with format, size, usage flags
- Graph compilation: topological sort passes, allocate transient resources, barrier insertion
- Imported resources: swapchain image, G-buffer textures (persistent across frames)
- Transient resources: bloom mip chain, SSAO noise (allocated per-frame, automatically freed)

---

### Task 36: Render Thread Separation
**Status:** ✅ Complete (FramePacket over bounded(2) channel)
**Duration:** ~1.5 weeks
**Prerequisites:** Task 35

**What you'll build:**
- Dedicated render thread that processes GPU command submission
- Double-buffered render data: game thread writes frame N+1 while render thread submits frame N
- Vulkan fence/semaphore pipelining for frame overlap
- Frame data snapshot: extract all render-relevant data (transforms, meshes, materials, lights) into a self-contained frame packet
- Thread-safe communication via channel (frame packets sent from game thread to render thread)

**What you'll learn:**
- Two-thread producer/consumer architecture
- Vulkan synchronization primitives (fences, semaphores, timeline semaphores)
- Frame-in-flight pattern (2-3 frames in pipeline simultaneously)
- Data snapshotting strategies (copy vs reference counting)
- Thread-safe resource management

**Why now:**
- Engine is CPU-bound (identified in Task 26, confirmed by Refactor #3 observation: 9950x + 1050Ti >> 5080 + 3900x)
- Render graph (Task 35) makes pass dependencies explicit, so the render thread knows exactly what to execute
- Game logic and render submission can overlap — immediate throughput gain
- Foundation for future parallel work (particles, animation evaluation)

**Conditional:** If Task 26-27 profiling shows the engine is GPU-bound, defer this task and focus on GPU optimization instead. Render thread only helps if CPU render submission is the bottleneck.

**Key concepts:**
- Frame packet: self-contained snapshot of everything needed to render one frame
- `mpsc::channel` or `crossbeam::channel` for frame packet delivery
- Vulkan fence per frame-in-flight (CPU waits for GPU to finish frame N-2 before reusing its resources)
- No shared mutable state between threads — frame packet is the only communication

**Technologies:**
- `std::thread` for render thread
- `crossbeam` channels for frame packet transfer
- Vulkan fences and semaphores for GPU synchronization

---

## Phase 10: Visual Quality (Tasks 37-39)

### Task 37: PBR Deferred Lighting, Shadows & Post-Processing
**Status:** ✅ Complete (deferred PBR, PCF shadows, bloom/SSAO/tonemapping)
**Duration:** ~2.5-3 weeks
**Prerequisites:** Task 35

**What you'll build:**

*Phase 1 — PBR Deferred Lighting (biggest visual impact, smallest effort):*
- Port Cook-Torrance BRDF from existing `pbr_fs.glsl` into deferred `lighting.frag`
- GGX normal distribution, Smith geometry, Schlick Fresnel — all already implemented in forward pipeline
- Use metallic, roughness, and AO already stored in G-buffer material texture
- Tone mapping (Reinhard + ACES filmic, selectable per-scene)
- Gamma correction (linear → sRGB)
- The G-buffer already contains everything needed — this is primarily a shader port

*Phase 2 — Directional Shadow Mapping:*
- Wire existing shadow infrastructure (`shadow.rs`, depth render pass) into the render graph as a shadow pass node
- Shadow map generation: render depth from directional light's perspective
- Add shadow map sampler (binding) to deferred lighting shader
- PCF soft shadows (3x3 kernel — already implemented in `pbr_fs.glsl`)
- Shadow bias and distance fade
- Single directional light shadow map (cascaded shadows in Task 48)

*Phase 3 — Post-Processing Stack:*
- HDR rendering pipeline (render to R16G16B16A16_SFLOAT, tone map to LDR at the end)
- Bloom: brightness threshold → progressive downsample → progressive upsample → composite
- SSAO from existing G-buffer data (position + normal textures already available)
- Exposure control: auto-exposure (luminance histogram) and manual override
- Vignette effect
- Each effect implemented as a render graph node (pluggable, toggleable, reorderable)
- Per-scene post-processing settings (override exposure, bloom intensity, etc.)

**What you'll learn:**
- Deferred PBR shading (adapting forward PBR math to G-buffer reads)
- Shadow map integration in a render graph (shadow pass → lighting pass dependency)
- HDR rendering workflow (linear-space rendering, tone mapping to display range)
- Bloom algorithm (dual Kawase or progressive Gaussian)
- SSAO implementation (hemisphere sampling, noise, blur)
- Luminance histogram for auto-exposure
- How post-processing chains compose in a render graph

**Why now:**
- Phase 1 alone transforms flat lighting into realistic material response — huge bang-for-buck
- Shadow mapping makes objects feel grounded in the scene (the "duck floating" problem)
- Post-processing (bloom + SSAO + tone mapping) is the final piece for professional visual quality
- All three phases build on existing code: PBR math in `pbr_fs.glsl`, shadow infra in `shadow.rs`, G-buffer data
- Render graph (Task 35) makes each effect a pluggable node — clean architecture

**Key concepts:**
- Cook-Torrance BRDF: D(GGX) * G(Smith) * F(Schlick) / (4 * NdotV * NdotL)
- Shadow mapping: render depth from light POV, sample in lighting pass with PCF
- Linear HDR → tone map → gamma correct → display
- Bloom: threshold → 6-level mip chain downsample → upsample with additive blending
- SSAO: sample hemisphere in tangent space, compare depth, blur result
- Auto-exposure: compute average luminance, smoothly adapt exposure over time

---

### Task 38: Particles & VFX (Inspector-Based)
**Status:** ✅ Complete (GPU compute particles)
**Duration:** ~1.5-2 weeks
**Prerequisites:** Task 37

**What you'll build:**
- Compute shader particle simulation (position, velocity, lifetime, size, color per particle)
- `ParticleEmitter` ECS component (emission rate, burst count, emission shape: point/sphere/cone/box)
- Particle forces: gravity, wind, turbulence noise, point attractors
- Billboard rendering with soft particles (depth-fade at intersections with scene geometry)
- Particle sorting for correct alpha blending (back-to-front relative to camera)
- Built-in presets: fire, smoke, sparks, dust
- Inspector-based authoring in the editor (all emitter properties editable as component fields)
- Particle debug visualization using debug draw (emission shape, force fields)

**What you'll learn:**
- Vulkan compute shaders for simulation
- GPU particle buffer management (ring buffer or dead/alive lists)
- Billboard rendering (camera-facing quads)
- Soft particle depth fade technique
- Alpha blending and sorting challenges

**Why now:**
- Particles make physics interactions, weapons, and environments feel polished
- Compute shaders learned here are reused for VFX graph (Task 51)
- Inspector-based authoring is sufficient for now — full node-based VFX graph comes at Task 51

**Note:** This task delivers inspector-based particle authoring. The full node-based VFX graph (Niagara-style) comes at Task 51 once the Node Graph Framework (Task 40) exists.

**Key concepts:**
- Compute shader dispatch: one thread per particle
- Dead/alive particle pool (recycle expired particles)
- Emission shapes: random point within sphere, cone direction, box volume
- Soft particles: compare particle depth with scene depth, fade when close

**Technologies:**
- Vulkano compute pipeline
- GLSL compute shaders
- Render graph integration (compute pass before render pass)

---

### Task 39: Shader Hot-Reload & Material Instancing
**Status:** ⚠️ Mostly complete — shader hot-reload works; material-instance types/serialization exist but `material_manager.rs` is not wired into the runtime render path (finish in Refactor Checkpoint #5)
**Duration:** ~1-1.5 weeks
**Prerequisites:** Task 38

**What you'll build:**
- File watcher on `.glsl` shader files — detect changes, recompile affected pipelines without restarting
- Graceful pipeline recreation (keep old pipeline active until new one compiles successfully, log errors on failure)
- Material instances: create variations of a base material by overriding specific PBR parameters (albedo color, roughness, metallic) without duplicating the entire material
- Material preview sphere in the inspector panel
- Material library: save/load material definitions as assets
- Material asset type integrated with asset browser

**What you'll learn:**
- Pipeline recreation in Vulkan (create new pipeline, swap, destroy old)
- File watching patterns for development tools
- Material instancing patterns (base material + override map)
- How to handle shader compilation errors gracefully (don't crash the editor)
- Asset hot-reload patterns

**Why now:**
- Shader iteration is currently: edit GLSL → recompile engine → restart → reload scene — painfully slow
- Material instancing reduces memory (100 red cubes share one material instance, not 100 material copies)
- Foundation for Visual Material Editor (Task 50) — needs material instance infrastructure
- Improves both engine development and content authoring workflow

**Key concepts:**
- `notify` watcher on shader directories (already used for asset hot-reload)
- Pipeline cache and recreation strategy
- Material instance = base material handle + HashMap<parameter_name, override_value>
- Shader compilation error recovery (keep old pipeline, show error in console)

---

### Task 39.4: Editor UI Theme & Widget Library
**Status:** ⛔ Superseded by the crusty-gui migration (2026-07). egui is fully removed; crusty-gui delivered the command palette, status bar, toasts, modals, dirty-state tabs, color picker, and layout persistence. The remaining goals (theme tokens, style guide, keyboard focus) move to **Task M1: Editor UX & Design System v1**; a11y/empty-states/full-undo move to UX v2 (deferred). Original egui-based plan kept below for reference.
**Duration:** ~7-9 weeks
**Prerequisites:** Task 39

Lift the editor from "egui tools" toward "real engine editor." Coherent visual language, polished interaction states, reusable UI primitives, modern-editor UX features (command palette, status bar, toasts, modals, dirty-state tracking), and live asset previews. Stays on egui — egui delivers this look once properly themed with a real custom widget library and supporting systems; no engine swap.

The task organizes into five conceptual blocks:
1. **Foundation** (Steps 1-2) — theme, typography, iconography, widget library, design-system patterns
2. **Panel rebuilds** (Steps 3-5) — hierarchy, inspector, asset browser brought up to the new standard
3. **Editor-wide UX** (Steps 6-11) — status bar, command palette, toasts, modal/dialog API, dirty-state model, layout save/restore
4. **Visual polish** (Steps 12-13) — drop shadows, color picker
5. **Preview Surfaces** (Step 14) — live render-to-texture for material/texture/material-instance previews

Steps 1-11 are pure egui painting + plumbing (no shader work). Steps 12-14 add a small amount of shader / render-to-texture work for high-ROI polish.

**Why now:**
- Task 39's inspector UI for material instances is brand new — it should ship with the new theme, not get retro-themed later
- Task 39.5 (asset editor windows) builds many new windows and benefits enormously from a widget library being already in place
- Hierarchy and inspector are the most-touched surfaces in the editor; their visual quality drives the whole "feel" of working in the engine

**What you'll build:**

**Step 1 — Theme system, typography, iconography (~3-4 days)**
- `EditorTheme` struct with palette tokens: primary, accent, 5 surface elevation levels, text-primary/secondary/disabled, semantic colors (error / warning / success / info)
- Apply theme to `egui::Visuals` and `egui::Style` — full pass over widget colors, rounding, shadows, focus rings, hover/active/inactive states
- Single dark theme for v1 (light deferred — every modern engine ships dark-first)
- Bundle `Inter` (UI font) and `JetBrains Mono` (code/path display); type scale: heading-large / heading / body / caption / mono with consistent line-heights
- Pre-rasterized PNG icon atlas (~40 icons sourced from Lucide); `IconRegistry` resource with `ui.icon(IconKind::Folder)` API
- **Accessibility / readability targets:** every text-on-background pair in the palette meets WCAG AA contrast (≥ 4.5:1 for body text, ≥ 3:1 for large text and UI components); minimum interactive row height 22px (compact) / 28px (comfortable); minimum body font size 12px; verified with a contrast-test harness that runs in the dev-only UI showcase
- **Panel density setting:** `EditorTheme.density: Density { Compact, Comfortable }` scales spacing tokens (~75% / 100%) and font tokens (~93% / 100%). Stored in user preferences; switchable at runtime via a command (registered in Step 7). Widgets in Step 2 read these tokens — they don't hardcode any spacing values.

**Step 2 — Custom widget library + design-system patterns (~5-6 days)**
New `engine/src/engine/editor/widgets/` module. All widgets are pure egui painting — no shaders, no new passes.

*Core widgets:*
- `themed_button` (primary / secondary / danger variants, optional icon)
- `toggle_switch` (replaces plain checkbox; animated thumb via `animate_bool`)
- `slider_with_input` (combined slider + numeric input, UE-style)
- `field_row` (label + value with consistent label width + alignment)
- `panel_header` (collapsible section: chevron animation, icon, color stripe)
- `asset_slot` (asset picker showing thumbnail; replaces text-path field)
- `tab_bar` (pillbox tabs for secondary windows)
- `tree_row` (hierarchy/asset row with hover, indent guide, drag handle, prefab override dot)
- `search_field` (with clear-button + fuzzy-match underline)

*Cross-cutting patterns (defined here, applied throughout the task):*
- **`SearchPopup` primitive** — one search-popup implementation that powers asset picker, component picker, command palette (Step 7), and future node picker (Task 40). Three popups must not become three implementations.
- **`EmptyState` widget + tokens** — unified pattern for "no asset selected" / "preview loading" / "failed to load" / "no results" / "missing texture." Used by inspector, asset browser, preview surfaces, search results. Avoids each panel reinventing its empty/error look.
- **State color tokens** — disabled / mixed / overridden / error / warning / success — applied uniformly across all widgets so a "disabled component" reads the same as a "disabled property" reads the same as a "disabled menu item."
- **Keyboard navigation rules** — codified once in widget code, consistent across every panel:
  - `Tab` / `Shift+Tab` cycles focus through interactive widgets in visual order (use `ui.memory_mut().request_focus(id)` and explicit focus chains where needed)
  - `Escape` closes any active popup, dropdown, modal, or rename in flight; cascades inward (innermost popup first)
  - `Enter` confirms the active control (button click, dialog accept, search-popup pick); `Ctrl+Enter` triggers the secondary action where applicable
  - `Arrow keys` navigate vertical lists (search results, hierarchy rows, asset browser tiles); `Home` / `End` jump to extremes; `Page Up` / `Page Down` step by visible-row count
  - `F2` enters rename on the focused hierarchy / asset row
  - Every widget that receives focus shows the focus ring (theme-defined, not egui's default)

*Dev-only UI showcase window:*
- Hidden behind a debug flag (`--features editor-debug` or runtime toggle); not in the user-facing menu
- One scrollable page demoing every widget × every state (normal / hover / active / focused / disabled / selected / error / warning / mixed)
- Built incrementally as widgets are written — every new widget adds a row
- Serves as design-system reference for the team and a regression check during the apply-pass steps (3-5)

**Step 3 — Hierarchy panel rebuild (~3-4 days)**
- Indent guides (vertical strokes through depth)
- Per-entity icon: pick dominant component (mesh / light / camera / UI / empty) and show icon left of name
- Visibility-toggle column (eye icon, always-visible)
- Drag-drop reorder + reparent with insertion-line feedback (`dnd_source` / `dnd_drop_zone` + custom paint of the insertion bar)
- Inline rename via double-click or F2 (swap `TextEdit` over the label on activation)
- Multi-select (Ctrl+click, Shift+click); selected rows render with accent fill
- Search bar at top (fuzzy match against names; collapse non-matching subtrees)
- Filter chips (Meshes / Lights / Cameras / Empty) — toggle buttons that filter the visible set
- Smooth foldout animation (chevron rotation + height interpolation)
- Right-click context menu: Create / Duplicate / Delete / Copy / Paste
- Prefab-override indicator (dot left of name when entity is a prefab variant with overrides — display only; editing UI is Task 52)
- Hover row highlight + clear selection contrast

**Step 4 — Inspector panel rebuild (~3-4 days)**
- Component foldout sections using `panel_header` widget — icon + 2px color stripe per category (rendering blue, physics green, animation orange, audio purple, gameplay grey)
- Drag-handle on the header gripper (`dnd_source`) to reorder components — **inspector display order only.** The reorder is persisted as inspector display state per entity (or per archetype, TBD during implementation); it does not affect runtime component storage, system iteration order, or any ECS semantics. Component order in hecs has no runtime meaning by design
- Per-property reset-to-default button (small revert arrow at row right edge, only visible when value differs from default)
- Property tooltips via `on_hover_text` on every label (descriptions filled in pass)
- "Add Component" search dropdown (custom popup with fuzzy match) replacing the current button-list
- Per-component context menu on right-click: Copy Values / Paste Values / Reset / Remove
- Multi-edit display: when multiple entities are selected and they share a component, show fields with mixed values greyed; click reveals "(mixed)" indicator (coordinated *commit* across the selection is deferred)
- Locked / disabled component visualization (greyed out section with lock icon)

**Step 5 — Asset browser polish (~1-2 days)**
- Hover-highlight tile + selection accent
- Tile / list view toggle
- Real icons per asset type (already in scope from Step 1's icon pass)
- Drag-drop into hierarchy / inspector slots
- Search with fuzzy match (uses `SearchPopup` primitive from Step 2)
- Filter chips by asset type
- `EmptyState` widget shown for "no folder selected" / "empty folder" / "no results"

**Step 6 — Status bar (~1-2 days)**
Persistent bottom strip across the main window. Composes existing data sources, no new state added — just surfaces what's already tracked.
- Left section: play/edit mode indicator, current scene name, save state ("●" dirty / "✓" saved — driven by Step 10's dirty-state model)
- Center section: notification ticker (latest non-error toast — slides in/out)
- Right section: FPS, frame time, selected entity count, GPU memory budget, shader-compile state, asset hot-reload state
- Click any segment to open the relevant panel (FPS → profiler, save state → save dialog, etc.)
- **Data-unavailable fallback rules** — every segment must render gracefully when its source isn't implemented or returns no data:
  - Numeric values (FPS, frame time, entity count): show "—" instead of zero or stale value
  - State indicators (compile, hot-reload): hide entirely when no event has occurred this session, rather than showing a "ready" steady state
  - GPU memory budget: hidden until `ResourceCounters` exposes it (currently partial); wire-up is a TODO comment, not a fake number
  - Scene name: shows "Untitled" when no scene is loaded
  - The three sections (left / center / right) are reserved slots that stay anchored to their edges. Individual segments inside a section collapse out when their data isn't available, but the section boundaries themselves don't move — so a missing FPS counter doesn't reflow the entire bar

**Step 7 — Command palette (~2-3 days)**
JetBrains-style "Find Action" — Cmd/Ctrl+Shift+P opens a search popup over registered editor commands.
- Reuses the `SearchPopup` primitive from Step 2 (consistency with asset/component pickers)
- `CommandRegistry` resource — commands register with a stable ID, display name, optional icon, optional keyboard shortcut, and category
- Fuzzy match on display name + category; recent commands surfaced first
- **In v1: commands only.** Asset/panel/setting search via the same popup deferred — different backends, want to ship the UX first
- Default keybinding registered through the existing input-action system

**v1 command list (bounded to prevent scope creep — anything outside this list is registered in a later task that owns the underlying feature):**
- *File:* New Scene, Open Scene, Save Scene, Save Scene As, Quit
- *Edit:* Undo, Redo, Cut, Copy, Paste, Duplicate, Delete
- *View:* Toggle Hierarchy / Inspector / Asset Browser / Console / Profiler, Reset Layout to Default, Switch Density (Compact/Comfortable), Toggle Dev Showcase
- *Selection:* Select All, Deselect All, Focus Selection (frame in viewport), Find Entity by Name
- *Play:* Toggle Play Mode, Step Frame, Restart from Edit State
- *Render:* Switch Debug View (Position / Normal / Albedo / Material / Depth / None), Toggle Wireframe, Toggle Grid, Toggle Gizmos
- *Asset:* Reimport Selected, Show in Explorer, Reveal in Asset Browser
- *Engine:* Reload All Shaders (Task 39 dev menu), Open Settings

Commands added by later tasks (graph editor commands from Task 40, animation commands from Task 41, etc.) register through the same `CommandRegistry` API but are not in 39.4's scope. Total v1 surface ≈ 30 commands.

**Step 8 — Notification / toast system (~1-2 days)**
Replaces console-only logging for transient user-visible events.
- `Toasts` resource — push API (`toasts.info("Saved: scene/main.scene")`, `toasts.error(...)`)
- Stacked rendering at bottom-right of the main window, max 4 visible at once
- Auto-dismiss after 4s for info, 6s for warning, 10s for error (clickable → expand to full text)
- Levels: info / warning / error / success — each maps to a state color token from Step 2
- Sources wired up: scene save, asset hot-reload (Task 39), shader hot-reload (Task 39), build complete, asset import done, validation errors
- Mirror to console (existing log) so the persistent record stays — toasts are *additive*, not a replacement

**Step 9 — Modal / dialog API (~2 days)**
Standardized blocking dialogs for actions that genuinely require user attention — paired with toasts (Step 8) which are non-blocking. Most editor flows should use toasts; modals are for decisions that affect data integrity.
- `ConfirmDialog` API: `dialogs.confirm(title, body, buttons).on_response(handler)`. Buttons are typed (`OkCancel`, `YesNoCancel`, `SaveDiscardCancel`, `Custom`); handler receives the chosen variant
- Modal stack — at most one modal visible at a time; subsequent dialogs queue and replay in order
- Backdrop dim + escape-to-cancel + enter-to-default-action (defined per dialog type)
- Built on the same `EmptyState` / state-token primitives from Step 2 so visual style matches everything else
- **Use cases wired up:** save-before-close (consumes Step 10's dirty-state), delete-N-entities confirmation, asset import warnings ("This will overwrite N references"), validation errors that block an action, "Apply Layout Reset?" for the Reset-to-Default command
- **Not** used for routine confirmations that have toasts equivalents — "Asset reimported" is a toast, not a modal

**Step 10 — Global dirty-state model + unsaved-change indicators (~1-2 days)**
Engine-side resource that tracks "what's modified since last save"; UI consumes it for indicators and save-before-close prompts.
- `DirtyState` resource with three scopes:
  - **Scene dirty** — set when entities/components mutate, cleared on save
  - **Asset dirty** — per-asset-path dirty flag (set when an asset is edited in its editor window, cleared when written to disk)
  - **Layout dirty** — set when dock state changes, cleared on session-end auto-save
- Indicators rendered uniformly across surfaces:
  - Title bar shows "●" prefix when scene is dirty (e.g., `● MyScene — Editor`)
  - Tab labels in dock show "●" suffix per dirty asset/scene (e.g., `MyScene ●`)
  - Asset browser tiles show a small dot overlay on dirty assets
  - Status bar (Step 6) summarizes scene+asset dirty count: "2 unsaved changes"
- Wired into the modal API (Step 9): closing a window with unsaved changes triggers a `SaveDiscardCancel` dialog before close completes
- **Mark-dirty discipline:** dirty marking is centralized through the editor's command/action layer, not sprinkled across systems. Every user-authored mutation already flows through an editor command (entity edits via inspector commands, asset edits via asset-editor commands, layout changes via dock commands); those command handlers call `dirty_state.mark_*()` once at the dispatch point. Random ECS systems do not call `mark_*` — keeping the responsibility centralized prevents "I forgot to mark this dirty" bugs as new systems land
- One source of truth — UI only reads `DirtyState`, never writes it; only command handlers write. Save operations clear the relevant scope on successful write

**Step 11 — Layout save / restore (~2-3 days)**
Save/restore custom dock layouts. **No named workflow presets in this task** — premature given workflow tasks (Task 41 Animation, Task 45 Scripting, Task 50 Material Graph) aren't yet shipped; presets come organically as those workflows mature.
- Persist current layout (window positions, sizes, dock state, open tabs) to `~/.config/<engine>/layout.ron` (or platform equivalent) on shutdown
- Restore on startup; fall back to default layout if file missing or invalid
- "Reset to Default" command (registered with the command palette) reverts to the built-in default layout — gated through the modal API (Step 9) since unsaved layout changes would be discarded
- **Layout file versioning + migration:**
  - `layout.ron` carries a `version: u32` field; loader matches it against the current expected version
  - When tab kinds, window kinds, or dock-tree structure changes (Tasks 41/45/50 will add new `SecondaryWindowKind` variants and likely rename or split existing ones), bump the version and register a migration function `fn migrate_v{N}_to_v{N+1}(old: &Value) -> Value`
  - Unknown tab/window kinds in a loaded layout are dropped with a warning logged (not a hard error — the user just sees an empty slot, and the layout still loads)
  - "Reset to Default" command is the always-available escape hatch when migrations fail catastrophically
- Layout file is the same format already serialized by `dock_layout.rs` — this step formalizes the load/save lifecycle, adds versioning, and registers the reset command

**Step 12 — Real drop shadows (~3-4 days)**
Step 1 sets up baseline shadow tokens (`egui::Style.window_shadow`, default popup shadow) — those are functional placeholders using egui's stacked-offset-rect rendering. This step **upgrades the shadows on elevated / floating surfaces specifically** to a soft, properly-falling-off look. Docked panels keep the baseline (subtle / none); only floating surfaces (popups, dropdowns, dialogs, drag previews) get the upgrade.
- **Primary path (no shader):** pre-baked 9-slice shadow texture (single PNG asset, blurred in an image editor); sample via egui's existing painter using UV math + `epaint::Mesh`. Zero new render passes. Handles 95% of cases.
- **Optional shader path:** a small Gaussian-blur fragment shader for *animated* shadows — e.g., panels lifting when dragged, dropdown shadow expanding on open. Only worth it if you want shadow size to animate on hover/active states. Skip if not.
- Apply to: floating panels, popups, modal dialogs, dropdown menus, drag-preview rendering. Not applied to docked panels (they don't need elevation).
- Theme tokens for shadow elevation levels (e.g., `shadow_low` for cards, `shadow_high` for modals).

**Step 13 — Custom color picker with shader-painted color spaces (~2-3 days)**
egui's default `color_edit_button_rgba` is functional but visually weak. This step adds a proper color picker as a custom widget for material/light authoring.
- Small fragment shader that takes UV + a mode flag (`hue_strip` / `hue_wheel` / `saturation_value_square` / `alpha_strip`) and outputs the appropriate color-space pixel
- Render the gradient surfaces to small offscreen textures on first display (or on-demand if hue rotates in HSV mode); embed via `egui::Image`
- Custom widget chrome around the gradients: numeric R/G/B/H/S/V/Hex inputs that round-trip with the gradient picker
- Saved swatches row (recently-used colors persist per-session; cleared on editor restart unless user pins)
- Used everywhere the editor edits color today: lights, materials, fog, post-processing settings, debug-draw colors

**Step 14 — Preview Surfaces (live render-to-texture for asset editors) (~4-5 days)**
The editor already renders mesh thumbnails / mesh-editor previews via `MeshPreviewRenderer` (offscreen render-to-texture, embedded in egui). This step generalizes that infrastructure for the *other* asset types so editor windows show real previews instead of placeholders. Architecturally distinct from Steps 1-11 (which are theme + widgets + UX); this is render infrastructure with editor consumers — bundled into 39.4 because previews are inseparable from "modern editor feel."
- Generalize `MeshPreviewRenderer` into an `AssetPreviewRenderer` with pluggable scenes:
  - **Material preview** — unit sphere with the selected material applied + 3-light studio rig + simple cube-map environment for IBL-flavor lighting (real IBL is Task 49 territory; here we use a baked low-res cubemap)
  - **Material instance preview** — same as material preview but using the `MaterialInstance`
  - **Mesh preview** — already exists; conform to the trait
  - **Texture preview** — draw the texture on a unit quad with checkered backdrop for transparency (helpful for albedo/normal/MR/AO inspection); no 3D scene needed, just a fullscreen quad pass
- Re-render on dirty: each preview tracks an invalidation flag; updated when asset changes (hot-reload), parameters change (slider drag), or selection changes
- Output as `Arc<ImageView>` registered as an egui texture, embedded in the relevant inspector panels and Task 39.5 editor windows
- Each preview runs at small fixed resolution (256x256 or 512x512), budgeted explicitly and re-rendered only when its dirty flag is set (asset reload, parameter change, selection change). Idle preview surfaces cost nothing — the previously-rendered texture is sampled directly. Per-preview cost stays bounded by resolution × number of *dirty* surfaces per frame; in practice that's a handful at most
- **Not in scope:** animation playback in clip preview (still images only at this step), audio waveform rendering (separate render path, will land in Task 39.5's Audio editor)

**Out of scope:**
- Light theme (deferred; dark-first like every modern engine)
- Per-user theme customization
- Custom OS window chrome / title-bar replacement
- Frosted-glass / backdrop blur (would need a separate render pass capturing the framebuffer behind the panel — net-negative for text contrast, deliberately skipped)
- Custom property drawers per component type (writing a custom inspector for `Transform`, etc., is a bigger system — separate task)
- Coordinated multi-edit commit across selections
- Hierarchy presentation modes beyond tree (Unreal-style "World Outliner" view)
- Animated ripple / shimmer / glow widget effects (wrong paradigm for desktop tooling)
- True linear/radial/conic gradient fills on widgets (modern flat design avoids gradients deliberately)

Each step leaves the editor in a usable shipped state — landing only Steps 1+2 already lifts the visual quality noticeably; Steps 3-5 deliver the most user-visible panel upgrades; Steps 6-11 add the editor-wide UX features (status bar, command palette, toasts, modals, dirty-state, layout persistence) that turn it from "tool" into "editor"; Steps 12-14 are the screenshot-quality polish (shadows, color picker, live previews) that make the engine read as "modern."

**Acceptance deliverables (committed to repo at task close):**
- **`docs/editor-ui-style.md` — UI style guide.** Single source of truth for everything Tasks 40/41/45/50 graph editors will need to align with: color tokens (palette, surface elevation, state colors), spacing tokens (compact vs comfortable scale), type scale, icon usage rules, widget catalogue with do/don't examples, modal vs toast decision rules, keyboard navigation contract, density behavior, dirty-state indicator conventions, layout versioning notes. Living doc — updated as new patterns land in later tasks. Cross-referenced from ARCHITECTURE.md.
- Before/after screenshots for each rebuilt panel: hierarchy, inspector, asset browser, command palette, material preview window, color picker, status bar. Used as design-system reference and regression baseline.
- Dev-only UI showcase window (built up across Step 2 onward) covering every widget × every state, runnable via debug flag — serves as the live design-system reference paired with the written style guide.
- Updated screenshots in ARCHITECTURE.md / KNOWLEDGE.md where editor diagrams currently exist.

---

### Task 39.5: Asset / Systems Editor Windows
**Status:** ⚠️ Partially complete — secondary-window infrastructure, dock/undock, and `MeshEditor` shipped (now on crusty-gui). Remaining asset-editor window types land opportunistically; not a blocker for the Multiplayer Foundation phase.
**Duration:** ~1-1.5 weeks
**Prerequisites:** Task 39, Task 39.4

Unreal-style: double-clicking an asset opens it as an OS-native window with its own winit `Window`, Vulkan `Surface`, swapchain, and egui `Gui`. Windows can be redocked into the main editor as tabs; tabs can be undocked back into their own OS windows. **Asset editor windows use the theme + widget library from Task 39.4 from day one.**

**The infrastructure already exists:** [`SecondaryWindow`](engine/src/engine/editor/secondary_window.rs), `SecondaryWindowKind`, `PendingWindowRequest`, dock/undock plumbing in `dock_layout.rs`. The `MeshEditor` already ships as a secondary window. This task **extends** that system — it does not build it from scratch.

**What you'll build:**
- New `SecondaryWindowKind` variants for the asset-editor types listed below
- Asset-browser double-click routing → emit `PendingWindowRequest` with the right kind for the file's extension
- v1 ships **basic / stub layouts** per type — full editing systems (animation graph, material graph, etc.) land in their own future tasks
- `EditorTab` ↔ `SecondaryWindowKind` round-tripping for every new variant so dock/undock works the same as for `MeshEditor`
- Open-windows session persistence (window position/size, current tabs, per-window dock state) — reopen on next launch
- Task 39.5 ships against the **current** asset extension scheme (`.material.ron`, `.matinst.ron`, `.mappingcontext.ron`). The single-segment migration to `<name>.<type>` happens in Refactor Checkpoint #5 *after* 39.5 lands; the migration pass at that time updates 39.5's references along with everything else. Avoids the ordering conflict where 39.5 would otherwise depend on a migration that hasn't run yet.

**New `SecondaryWindowKind` variants:**
- **Material** (`.material.ron`) — factors panel + texture slot pickers + **live preview sphere** (consumes `AssetPreviewRenderer` from Task 39.4 Step 14)
- **MaterialInstance** (`.matinst.ron`) — base picker + per-override checkbox/editor grid (elevates Task 39's inspector UI) + **live preview sphere** showing the resolved material with overrides applied
- **Texture** (`.png` / `.jpg` / `.ktx2` / etc.) — preview + import settings stub (sRGB, mipmaps, compression — fields wired but apply-on-save deferred)
- **AnimationClip** — timeline scrubber + bone-curve list (stub; full editing in Task 41)
- **AnimationGraph** (future, format TBD by Task 41) — placeholder window with "Editor lands in Task 41" notice + read-only graph dump
- **MaterialGraph** (future, format TBD by Task 50) — placeholder window with "Editor lands in Task 50" notice
- **Audio** (`.wav` / `.ogg` / `.mp3` / `.flac`) — waveform thumbnail + play/stop buttons + import settings (loop, gain, 3D)
- **MappingContext** (`.mappingcontext.ron`) — context layer + action list editor (elevates from inspector if not already done in `InputContext`)
- **Prefab** (future, format TBD) — mini-viewport with read-only entity tree (full prefab editing deferred)

**Existing variants reused as-is:** `Mesh`, `Hierarchy`, `Inspector`, `AssetBrowser`, `Console`, `Profiler`, `InputSettings`, `InputAction`, `InputContext`.

**Why now:**
- Pre-Task-40 surface area for graph editor stubs (Animation Graph / Material Graph placeholders) means later graph tasks plug into existing routing rather than inventing UI integration
- `MeshEditor` is the only secondary-window editor today; extending the registry surfaces any architectural rough edges before `Task 40-50` add ten more

**Out of scope:**
- Scene editing (already in the main viewport)
- Cross-window tab drag-drop — works today via dock-then-undock; native drag-between-windows is polish, deferred
- Apply-on-save for texture import settings — fields wired, persistence deferred
- Real animation timeline / curve editing — stub only; Task 41 fills in

---

## 🔄 Refactor Checkpoint #5: Production Readiness Review
**Status:** ⚠️ Core items done (material-instance wiring, asset-extension migration — commit 1b919e9); perf/threading/code-quality review passes remain optional before M0
**Duration:** ~1 week
**Prerequisites:** none (Tasks 26–39 complete; crusty-gui migration complete)

**Revised 2026-07:** this checkpoint now gates the **Multiplayer Foundation phase (M0–M8)**, not Task 40 — node graphs are deferred until after the multiplayer arc. egui-era items were dropped (migration already done). Scope is deliberately slim: roadmap status sync (done), the asset-extension migration below, and material-instance runtime wiring.

### Material Instance Runtime Wiring (finish Task 39) — ✅ Done (commit 1b919e9)
- `MaterialManager` wired into `resolve_material_sets`: bases registered from `.material` defs, `.matinst` instances resolve to descriptor sets; hot-reload evicts stale instances
- Exit met: `content/models/Duck_red.matinst` renders the Duck with a red tint (verified in editor 2026-07-15)

### Performance:
- Profile against Task 26 baselines — measure cumulative gains from all optimizations
- Memory leak audit (asset loading/unloading cycles, play mode enter/exit cycles)
- Identify any performance regressions introduced by new systems (animation, audio, particles)

### Threading Readiness:
- Verify ALL systems declare access patterns (Task 32 contract)
- Eliminate any remaining undeclared shared mutable state
- Verify asset loading is structured as async-friendly work
- Confirm scripting/node APIs route through commands, not raw world access

### Code Quality:
- API cleanup for public interfaces that accumulated debt
- Full test suite pass (Task 28 tests + any added since)
- `cargo clippy --all-targets --all-features` clean
- `cargo fmt --check` clean
- Documentation update (ARCHITECTURE.md, KNOWLEDGE.md)

### Asset Extension Scheme Normalization — ✅ Done (commit 1b919e9)
Implemented: classifier/savers/scanners/watcher accept both schemes (legacy warns); `tools/migrate_asset_extensions` (dry-run default, `--apply`, reference rewriting) migrated `content/` + `assets/`.

**Decision:** drop the `.ron` suffix on engine-owned RON assets. Files move from `<name>.<type>.ron` to `<name>.<type>`. The **final dot-segment** is the asset type discriminator (so `foo.test.material` still classifies as a material — the leading dots are part of the basename, not type tags). The serialization format (RON) stops being part of the filename.

**Migration map:**
| Before | After |
|--------|-------|
| `foo.scene.ron` | `foo.scene` |
| `foo.material.ron` | `foo.material` |
| `foo.matinst.ron` | `foo.matinst` |
| `foo.inputaction.ron` | `foo.inputaction` |
| `foo.mappingcontext.ron` | `foo.mappingcontext` |

`foo.mesh.ron` import sidecars are **not** renamed — stripping `.ron` would collide with the binary `foo.mesh` file. They stay hidden metadata (classified `Unknown` in the asset browser); references inside them are still rewritten by the migration tool.

Future asset types introduced after this checkpoint (Task 40's `.graph` / `.subgraph`, Task 41's `.animgraph`, Task 45's `.vscript`, Task 50's `.matgraph`, Task 47's `.prefab`, etc.) follow the same `<name>.<type>` form from day one.

**What ships:**
- **Migration script in `tools/`** with `--dry-run` mode (default) and `--apply` mode
  - Walks the content dir, renames `*.<type>.ron` → `*.<type>` on disk
  - **Updates serialized references inside the assets** themselves: scenes carry `mesh_path` / `material_paths` / etc.; prefabs carry component paths; mesh sidecars carry material slot paths. The script rewrites these strings, not just the filenames on disk. Runs as part of the same pass.
  - Reports a per-file summary (files renamed, references rewritten per file, any mismatches)
- **Tests** for the migration logic: golden-file fixtures with both old and new schemes, asserting the rewrite is byte-identical to a hand-authored "after" file
- `AssetType::extensions()` map updated to the single-segment list (`["material"]`, `["matinst"]`, `["mesh"]`, etc.)
- Asset browser classifier + watcher dispatcher audited — both currently key on the multi-segment form; switch to the final-segment rule
- **Old-asset deprecation policy:** loaders **emit a warning** when they encounter `*.<type>.ron` (with a pointer to the migration script) and load successfully via the legacy path. After two minor versions, the loader fallback is removed and old files fail hard. Decision rationale: zero-disruption upgrade for users who haven't run the migration yet, with an explicit deprecation horizon.
- Texture/audio extensions (`.png`, `.wav`, etc.) untouched — they're third-party formats, not ours
- Editor file associations documented for VSCode/RustRover so the new extensions get RON syntax highlighting (`*.material → ron`, etc.)
- Cross-doc update pass: this roadmap, `VULKANO-39-*`, `TASK-39-*`, ARCHITECTURE.md, KNOWLEDGE.md

### Task 40 Readiness Subsection
**Deferred with Task 40** — execute this subsection when node graphs are scheduled, after the Multiplayer Foundation phase.

The graph framework lands in Task 40. This subsection verifies the foundations it needs are in place:
- **Graph asset save/load path decided** — RON via the new `<name>.<type>` scheme; serializer registered alongside scene/material; load path goes through `AssetManager`
- **Asset references support graph/subgraph dependencies** — `Vec<String>` path lists generalize cleanly to "graph references graph" without schema changes; verified via a dummy graph asset that references another dummy graph
- **Secondary window host can open graph-like editors as stubs** — the `AnimationGraph` / `MaterialGraph` placeholder windows from Task 39.5 confirm the routing works; Task 40 swaps the placeholder body for the real editor
- **Command-only mutation path verified** — every system that will be exposed to scripting/node execution mutates world state via `Commands` (not raw `&mut World`); audit every system added in Tasks 31–39 against Task 32's access-declaration contract
- **Serialization/versioning test harness ready** — graph schema migrations require a per-node-type version number + migration function; harness validates that an "old" graph asset round-trips through the migration path. No real migrations exist yet (no graph types yet) — this delivers an empty-but-functional harness for Task 40 to plug into

### Exit Criteria
The checkpoint is complete when **all** of the following are green:
- ✅ `cargo test --workspace --all-features` passes
- ✅ `cargo clippy --workspace --all-targets --all-features -- -D warnings` clean
- ✅ `cargo fmt --check` clean
- ✅ Editor build (`--features editor`) and standalone/runtime build (`--no-default-features` if applicable, otherwise default) both pass and run without panics on startup
- ✅ Benchmark report produced against Task 26 baselines, no unexplained regressions
- ✅ Access-declaration audit: zero systems without declared access patterns
- ✅ Asset migration script tested on the engine's sample content directory; dry-run on real user content directory completes without errors
- ✅ Loaders emit deprecation warnings for `*.<type>.ron` files; legacy load path still functional
- ✅ ARCHITECTURE.md and KNOWLEDGE.md reflect the new asset naming scheme and the secondary-window editor architecture
- ✅ Material instances render at runtime (Task 39 closed out)
- ~~Task 40 readiness items~~ — deferred with Task 40

---

## Phase M: Multiplayer Foundation (Tasks M0–M8)

**Added 2026-07.** Goal: a **working multiplayer engine** for an open-world MMO (WoW-like pacing) that also serves co-op sessions. The proof point is a **networked co-op slice**, replacing the old single-player vertical slice milestone.

**Numbering:** M-prefix task IDs avoid renumbering Tasks 40–60 and breaking existing cross-references. This phase runs **before** Phase 11 — node graphs and most of Phases 11–15 are deferred until after it (see per-task annotations below).

**Design anchors** (full rationale in `docs/DECISIONS.md`):
- **ADR-014** — SpacetimeDB as backend; backend-neutral command interface in `game_shared`; renet/QUIC fallback
- **ADR-015** — server-authoritative *kinematic* movement; cosmetic-only client Rapier; no determinism requirement (prediction + reconciliation)
- **ADR-016** — spike-first go/no-go gating (M0) before any engine networking

**Top risks** (from design review with gpt-5.6-Sol):
1. False-positive spike — bots must model hotspots, churn, and combat fan-out, not uniform wandering
2. Client/server movement parity across collision-chunk seams (M2/M6 parity tests)
3. Identity/lifecycle coupling — ghost state from entity respawn/zone transfer (M5 World Identity Contract)

**crusty-gui:** upgrade tasks are inserted into this phase when a milestone needs them (per its own ROADMAP.md post-parity phases), rather than pre-scheduled.

---

### Task M0: SpacetimeDB Scale Spike (Net-0)
**Status:** ✅ Complete — **GO** (per-module ceiling ~500 uniform / ~150–200 per crowd / ~110–135 Mbps delivery; sharding + interest-management requirements recorded in the spike doc)
**Duration:** ~2 weeks
**Prerequisites:** Refactor Checkpoint #5

**Go/no-go gate for M5–M8.** Throwaway spike, **no engine integration**: standalone SpacetimeDB module (move/cast/chat reducers, cell-filtered subscriptions) + headless console bot clients, run at a **300 / 3,000 / 30,000 bot ladder** with hotspot, churn, and raid-moment scenarios. Pass criteria fixed up front; deliverable is the per-module ceiling and the implied sharding/zoning strategy for M8. No-go triggers the renet/QUIC fallback behind the same `game_shared` command interface.

**Full plan:** [VULKANO-M0-SPACETIMEDB-SPIKE.md](VULKANO-M0-SPACETIMEDB-SPIKE.md)

---

### Task M1: Editor UX & Design System v1
**Status:** 🔀 Superseded by **Task M10** (mockup-driven scope: full design-system
restyle + settings windows — see `VULKANO-M10-EDITOR-UX.md`)
**Duration:** ~2 weeks (hard time-box)
**Prerequisites:** none (can overlap M0)

Inherits the surviving goals of superseded Task 39.4, now on crusty-gui:
- **Theme tokens**: single source of palette/spacing/typography tokens in crusty-gui's style system; kill ad-hoc colors in panels
- **Style guide**: short written guide (docs/) — when to use which widget, spacing rules, semantic colors
- **Keyboard focus**: consistent focus order and visible focus rings across editor panels

**Out of scope (→ UX v2, deferred):** accessibility audit, empty-state designs, full undo coverage of every editor action. The time-box is deliberate: editor polish must not delay the multiplayer arc.

---

### Task M2: Collision Pipeline v1
**Status:** ✅ Complete (2026-07-17 — see `VULKANO-M2-COLLISION-PIPELINE.md`)
**Duration:** ~2 weeks
**Prerequisites:** M0 go decision

The foundation of server-authoritative movement (ADR-015):
- **Cooked static-collision chunks**: offline cooking of scene static geometry into a compact, versioned binary format (heightfield/trimesh + AABB tree), partitioned into chunks aligned with the world grid (M3)
- **Version stamp** in every chunk; client and server refuse to mix versions
- Consumed **identically** by client Rapier queries and server WASM shape-casts — one loader crate in `game_shared`
- Parity test: identical shape-cast batteries run against the same chunk on both loaders; results must match within tolerance

---

### Task M3: Greybox World v1
**Status:** ✅ Complete (2026-07-17) — see `VULKANO-M3-GREYBOX-WORLD.md`
**Duration:** ~1-2 weeks
**Prerequisites:** M2

A partitioned test world, ugly on purpose (real terrain is deferred Task 46):
- Grid-partitioned **terrain proxy** (heightfield greybox) + M2 collision chunks per cell
- Traversal content: slopes, steps, gaps, a vertical structure — exercises the kinematic controller's full move set
- Big enough to span multiple zones/cells so M4/M5/M8 have something real to stream and subscribe to

---

### Task M4: Zone & Chunk Lifecycle
**Status:** ✅ Complete (2026-07-17) — see `VULKANO-M4-ZONE-CHUNK-LIFECYCLE.md`
**Duration:** ~1-2 weeks
**Prerequisites:** M3

Scene management **scoped to what multiplayer needs** (absorbs old Task 43; general scene transitions/loading screens stay deferred):
- Load/unload world cells (render assets + collision chunks) around the local player
- Entity spawn/despawn tied to cell lifecycle, with stable identity across unload/reload (EntityGuid, ADR-013)
- Async loading budget — no frame hitches on cell boundaries

**Full plan:** [VULKANO-M4-ZONE-CHUNK-LIFECYCLE.md](VULKANO-M4-ZONE-CHUNK-LIFECYCLE.md)

---

### Task M5: Net-A — Connection, Identity & Replication
**Status:** ✅ Complete (2026-07-19 — see `VULKANO-M5-NET-A-CONNECTION-IDENTITY.md`)
**Duration:** ~3 weeks
**Prerequisites:** M0 go, M4

First engine↔SpacetimeDB integration, through the `game_shared` command interface only:
- Auth/session + entity **ownership** model
- **Schema versioning** from day one (client/server schema mismatch = clean refusal, not corruption)
- Input **sequence numbers** and server ack; **clock sync** estimate
- **Reconnect** flow: resubscribe, state snapshot, no duplicate entities
- **World Identity Contract** (named subtask): one written contract covering EntityGuid ↔ server row id mapping, **spawn generations + tombstones** (kills ghost-state class of bugs), and identity across zone transfer — reviewed before code
- Transform sync + interpolation for remote entities; basic zone-scoped visibility (crude, replaced by M8)
- **Server persistence schema** (absorbs old Task 42 for networked state — SpacetimeDB tables *are* the save system)

---

### Task M6: Net-B — Server-Authoritative Movement
**Status:** ✅ Complete (2026-07-19 — see `VULKANO-M6-NET-B-MOVEMENT.md`)
**Duration:** ~3 weeks
**Prerequisites:** M5

ADR-015 made real:
- **Shared kinematic character controller** in `game_shared`, compiled into client and server WASM; shape-casts vs M2 chunks (walk, slope, step, jump, gravity)
- **Client prediction + server reconciliation** (input seq numbers from M5); smooth correction, no rubber-banding at WoW-pace movement
- **Parity test suite**: recorded input traces replayed on both sides; divergence across chunk seams is the named failure mode to hunt
- **Grounding seam designed day one** (implementation of moving platforms deferred to M6.5): controller state carries ground entity id + generation, local anchor point, inherited platform velocity, loss-of-ground rules — so platforms/elevators later require no rewrite
- Server-side **AoE spatial broadphase** (cell-indexed queries) and **hitscan + simple projectile** paths — the combat primitives M7 needs

**M6.5 (deferred):** moving platforms/elevators using the grounding seam.

---

### Task M7: Net-C — Authoritative Combat & Thin HUD
**Status:** ✅ Complete (2026-07-20 — see `VULKANO-M7-NET-C-COMBAT.md`)
**Duration:** ~3 weeks
**Prerequisites:** M6

- Server-authoritative **ability system**: GCD + per-ability cooldowns, resource costs (mana/energy), range + line-of-sight + **target legality** checks, cast times with **interruption** rules
- Combat resolution as reducer transactions (damage, death, respawn via spawn generations)
- **Exploit tests**: replayed/forged/reordered inputs, cooldown bypass attempts, out-of-range casts — server must reject all
- **Thin HUD in crusty-gui** (absorbs the minimal slice of old Task 55; the general in-game UI framework stays deferred): action bar with cooldown sweeps, cast bar, target frame, connection-state indicator

---

### Task M8: Net-D — Interest Management & Load
**Status:** ✅ Complete (2026-07-20 — see `VULKANO-M8-NET-D-INTEREST.md`; load results in `M8-LOAD-REPORT.md`)
**Duration:** ~2-3 weeks
**Prerequisites:** M7

- **Indexed zone/cell membership** with **hysteresis** (no subscription thrash at cell borders)
- **Layered subscriptions**: near = full state, far = coarse (position/name only), out = none
- Load tests against M0's bot harness patterns, now through the real engine client; compare against the M0 ceiling and the thresholds it set
- Sharding/zoning strategy from M0's report implemented as far as the slice needs

---

### Task M9: Multiplayer Packaging — Client & Server Build Targets
**Status:** ✅ Complete (see `VULKANO-M9-MP-PACKAGING.md`)
**Duration:** ~1 week
**Prerequisites:** M5 (can run any time after; the Co-op Slice milestone consumes it)

Extends Task 25's export (✅ standalone Windows builds) with **build targets**.
Note the SpacetimeDB shape: there is no dedicated server executable to compile —
the "server build" is the WASM module plus a publish step.

- **`standalone` target**: the existing Task 25 pipeline, unchanged
- **`mp-client` target**: Task 25 bundle + a connection config (server URI, db
  name) as a RON file next to the exe — editable post-build, no recompile to
  point at a different server
- **`mp-server` target**: `spacetime build` the module + publish script; two
  destinations: local standalone instance (dev) or a rented host (prod)
- **Host-locally convenience** (Unreal listen-server analogue): one script
  spawns `spacetime start`, publishes, launches the packaged client against
  localhost — the packaged play-test loop
- **Version stamping**: client and module carry a matching build id; client
  warns on mismatch at connect (schema drift is otherwise a silent desync)

---

### Task M9.5: Packaged Co-op Verification
**Status:** ✅ Complete (two-machine hour soak skipped by decision, 2026-07 — runbook remains at `M9.5-COOP-RUNBOOK.md` if ever needed; see `VULKANO-M9.5-COOP-VERIFICATION.md`)
**Duration:** ~2-3 days
**Prerequisites:** M8, M9

Prove the *packaged artifacts* — not the dev environment — deliver the co-op
slice. No cargo, no editor, no source checkout on the test machines.

- **Smoke script (CI-able)**: publish module to a local standalone instance,
  launch the packaged client, assert connect + own-entity spawn within N s,
  exit clean
- **Two-machine internet test**: the Co-op Slice milestone acceptance run
  entirely on M9 builds (one publishes to the rented host, both clients join)
- **Load sanity**: point the M0 bot harness at the packaged-published module;
  numbers must match the M0/M8 baselines (catches shipping-profile and
  cooked-content regressions)

---

### Task M9.6: Editor Net Play Modes & Server-Announced World
**Status:** ✅ Complete (full plan + close-out: `VULKANO-M9.6-EDITOR-NET-PLAY.md`)
**Duration:** ~4-6 days
**Prerequisites:** M9 (consumes its net config, publish params, host-local logic), M9.5

UE-style **Net Mode** in the editor play settings, plus the **Server
Default Map** analogue folded in:

- **Server-announced world**: module `config` table gains `world_scene`
  (protocol v6); clients load the announced scene instead of hardcoding
  `greybox.scene` — kills the connected-but-wrong-scene bug class
- **Play Standalone**: today's play mode, unchanged
- **Play As Client**: `NetSession` created on play-enter / torn down on
  play-exit against the configured server (today it only exists via
  startup CLI args) — includes play-mode snapshot/restore handling of
  net-spawned proxies
- **Play As Listen Server**: Play-As-Client + auto-start local
  SpacetimeDB and publish if needed (there is no true listen server in
  SpacetimeDB — the sim always runs in the SpacetimeDB process; this is
  the launcher analogue)
- **Number of Players**: spawn N−1 extra standalone client processes
  with `--connect`

---

### Task M10: Editor UX & Design System v1 (Crusty Theme Tokens)
**Status:** ✅ Complete — see `VULKANO-M10-EDITOR-UX.md`
**Duration:** ~3–4 weeks
**Prerequisites:** M9.6 (absorbs superseded Task M1)

Implements the Crusty Design System mockup (`docs/mockup/`, local) at ≥95%
fidelity:

- **Semantic token system**: 9 surfaces + 3 accents + invariant
  selection/status/axis/type colors; four presets (Steel default, Tidepool,
  Graphite, Rusty), live-switchable; zero hard-coded colors in panels
- **Fonts**: bundled IBM Plex Sans + JetBrains Mono, multi-family text runs
- **Widget state ladder + keyboard focus**: five states everywhere, focus
  rings, Tab traversal; new primitives (toggle, chips, mixed checkbox,
  structured tooltip, spinbox caps)
- **Full Edit menu** (History / clipboard / Configuration) with entity
  Cut/Copy/Paste
- **Settings windows**: Editor Preferences (`editor_prefs.ron`, live-apply;
  PlaySettings migrates here) + Project Settings (`project.ron`, checked in
  — the file Task 39.8's plugin manifest later extends)
- **Panel restyle**: chrome, hierarchy, inspector axis/asset-reference
  fields, asset browser, console, profiler, status-bar command palette

---

### 🎯 Milestone: Networked Co-op Slice
**Status:** ✅ Achieved (M0–M10 complete; hour soak waived, 2026-07)
**Replaces the old "Single-Player Vertical Slice" as the engine's first proof point.**

A friend can join over the internet: both players traverse the greybox world (M3), see each other move smoothly (M6), fight mobs and each other with abilities (M7), disconnect and reconnect without ghosts (M5), while the server holds authority throughout — **running the packaged M9 builds, verified per M9.5**. Runs for an hour without desync, leak, or crash.

---

### Deferred until after the Multiplayer Foundation phase
Node graphs (40, 41, 45, 50, 51, 53, 57), visual terrain (46), sky/atmosphere (47), CSM (48), IBL/GI (49), full asset cooking & streaming (44), general in-game UI framework (55 remainder), parallel ECS (58), Editor UX v2 (a11y, empty states, full undo).

---

### 🔄 Refactor Checkpoint #6: Rendering API Cleanup (slim)
**Status:** ✅ Complete (2026-08-10, commits `1fe455b` items 1-2, `bc64b1f` item 3, `b382e44` items 4-5, `6af6ccc` adversarial-review fixes — of note: item 3's declared `ssao_blurred` read revealed that SSAO was previously culled out entirely, i.e. "SSAO enabled" rendered nothing before this checkpoint; it now actually runs when enabled). All exit criteria verified: legacy `render_*` deleted + all feature combos build; resize exercised editor + standalone (programmatic SetWindowPos passes) with zero validation errors; adding a pass touches 3 sites (pass file, constructor + `passes_mut()` registration, one `add_pass_with` block). Net ~-1000 lines. Scope note: SSAO sample count NOT surfaced — baked into `ssao.frag` (fixed 64-tap kernel loop); exposing it is a shader change, not a refactor.
**Duration:** ~3-4 days (items 1-3 are each self-contained, afternoon-to-day sized)
**Prerequisites:** none

From the 2026-08 rendering API audit (Claude + Codex pass over `engine/src/engine/rendering/`). Verdict: the internals are sound — `FramePacket` thread boundary, render-graph core (sorted/culled/tested), `ArcSwap` hot-reload registry — but the *authoring* surface accumulated debt. In priority order:

1. **Retire the legacy `common/renderer.rs` facade.** The five `render_*` methods (~400 lines: sprite/mesh paths) are dead code presenting a misleading API — per-mesh acquire/present, blocking `wait(None)`, hardcoded texture load in `new()`. Keep only the bootstrap (device/swapchain/allocators, `submit_and_present`), folded into the existing `GpuContext`/`SwapchainState` split.
2. **Per-pass resize/rebind trait.** `DeferredRenderer::resize` is a hand-maintained chain (gbuffer → SSAO → bloom → luminance → composite → plankton); one missed recreation = stale-descriptor crash on window resize, and resize errors are only logged on the render thread. A `DeferredPass` trait (`resize` + `rebind(inputs)`) turns the chain into a loop; propagate the errors.
3. **Execution closures on the render graph.** Dispatch is a stringly-typed `match pass_name` (`deferred_renderer.rs:903`) — an unmatched name silently does nothing, and declared reads/writes aren't checked against what passes actually touch. Attach execution at `add_pass` (closure or enum dispatch) so the graph owns execution, not just ordering. Prerequisite if 39.8 plugins or Phase 12/13 tasks ever inject passes.
4. **`PassPipelineBuilder` helper** for the shared `GraphicsPipelineCreateInfo` recipe — bloom alone builds three near-identical pipelines (~145 lines); SSAO/luminance/composite repeat the same fullscreen-pass boilerplate. Also makes registering all passes for hot reload (today: only Geometry/Lighting) nearly free.
5. **Small debts:** typed `RenderError` at the module boundary (replace `Box<dyn Error>`); builder constructors for `PbrMaterial`/`MaterialInstance` (8-12 positional args); camera near/far through `FramePacket` (plankton hardcodes 0.1/1000); once-per-path warning when a mesh/material path fails to resolve (`render_loop.rs` silently skips/defaults today); bloom mip count + SSAO sample count surfaced through `PostProcessingSettings`.

**Explicitly not in scope:** full retained-mode render graph (automatic barriers, transient aliasing) — with nine fixed passes the payoff isn't there. Revisit only when plugin-injected passes become real.

**Exit criteria:** legacy `render_*` methods deleted, all feature combos build; resize exercised in editor + standalone with zero stale-descriptor validation errors; adding a pass touches ≤3 sites (pass file, constructor, graph registration).

---

### Task 39.8: Plugin System & Module Registry
**Status:** ✅ **Complete** (2026-08-10) — plan + binding rulings in `docs/roadmap/VULKANO-39.8-PLUGIN-SYSTEM.md`; architecture in `docs/ARCHITECTURE.md` ▸ Plugin System; author guide in `docs/PLUGINS.md`
**Prerequisites:** Task 40 (registry contract), Refactor Checkpoint #6

**Commit map** (one commit per package, each gated independently):

| Package | Commit | What landed |
|---|---|---|
| P1 — trait + PluginSet + manifest | `cc5b010` | `EnginePlugin`, `PluginManifest`, `PluginError`, staging `PluginContext`, topo-ordered `build_all` with commit-on-Ok, `plugins` in `project.ron`, `ClientGamePlugin` ported, `GamePlugin` deleted, standalone plugin position unified |
| P2 — editor init-order refactor | `46b1d7e` | registries-then-content in `App::new()`, one world-population helper for all five content moments, `world_and_resources_mut`, fallible `on_world_loaded` |
| P3 — dev_nodes conversion | `c89ea3e` | `DevNodesPlugin` (first real plugin), registry-parity golden, doc-level unregistered-type save round-trip |
| P4 — editor extension points | `0bd9473` | `EditorTab::Plugin`, `PluginPanel`/`PluginSettingsPage` + `PluginPanelCtx`, View ▸ Panels, layout persistence + missing-panel placeholder |
| P5 — Rapier extraction | `b6fffc2` | `RapierPhysicsPlugin` (step system, body registration via `on_world_loaded`, collider overlay hook), `depends_on` cascade, play-enter gating, inspector inert note, gameplay-disabled hint |
| P6a — relaunch mechanics | `61fb30f` | `ReplaceFileW` atomic config writes, `--wait-parent` + `OpenProcess`/`WaitForSingleObject` handle wait, manifest facts |
| P6b — Plugin Manager UI | `67ed6d6` | two-pane manager as a Project Settings page, full D8 state ladder, cascade warning, Relaunch Now |
| P7 — export features + close-out | `d37bd82` | `--no-default-features --features <base + enabled runtime>`, `EditorOnly` never ships, build-dialog visibility, docs |

**Verified at close-out:** export with base features only is 17,317,376 bytes
and contains no `dev_nodes` fixture strings; forcing `dev_nodes` in adds
12,800 bytes and the fixture content appears — i.e. the `EditorOnly` rule
strips something real. HUD survives `--no-default-features`. Packaged bundle
runs from its own output directory.

**Deferred, honestly:**

1. **`physics_rapier` cannot be stripped from exports.** It has no Cargo
   feature, and adding one would not help: D7 deliberately keeps
   `PhysicsWorld`, the components, scene serialization and the inspector in
   engine core, and `rapier3d` is a non-optional dependency used by 16 files
   (collision cooking, prefabs, counters, character movement, …). A
   `plugin-rapier` feature would gate ~100 lines while the crate stayed
   linked — a near-zero saving and a false promise in the manager. Real
   stripping requires the reflection-lite component-registry arc that D2 put
   out of scope. Disabling physics remains a *runtime* configuration.
2. **Plugin Manager: no "Copy error" button** on the failure detail (the
   mockup has one; not part of D8's state contract).
3. **Filter-segment counts** render inside the segment label rather than as a
   separately-colored span — the shared `segmented_control` widget takes
   `&[&str]`.
4. **Orphan rows show no kind chip**: a plugin that is not in this build has
   no known `kind`, so the manager declines to assert one.
5. **Floating plugin panels** take their OS window title from the plugin set,
   but `float_window_attrs` still falls back to the panel id for anything it
   cannot resolve.
6. **Component types, asset types and render passes** remain non-registrable
   (D2). Steam SDK and the Gameplay Ability System remain follow-on plugin
   candidates, unblocked by this task.
7. **`streaming_acceptance::flythrough_streaming_stays_within_budget`** fails
   on this machine both before and after the whole task — a pre-existing
   machine-speed budget test, unrelated to plugins.

<details>
<summary>Original plan bullets (superseded where they differ)</summary>


Engine functionality as discoverable, toggleable units (Unreal-plugin analogue,
Rust-shaped). **Two-tier model** (revised 2026-08-10 — the original "flip =
recompile" toggle was rejected: a shipped editor must be usable by
non-programmers): tier 1 = compiled-in plugin crates whose *activation* is a
per-project manifest read at startup — **toggle = restart only, never a
rebuild**; Cargo features remain the packaging tool (game export strips disabled
plugins; editor ships batteries-included). Tier 2 (future, shape-preserved only)
= binary plugins via a C-ABI/WASM seam at narrow extension points
(GDExtension/Zellij precedent).

- `EnginePlugin` trait: one entry point (`fn build(&self, ctx)`) staging systems,
  resources, node types, editor panels/settings pages; commit-on-Ok so a failing
  plugin is skipped cleanly and surfaced in the Plugin Manager.
- **Project manifest** (`plugins` in `project.ron`): per-project enabled set,
  VCS-checked-in; unknown-in-this-build entries preserved, dependency cascade
  enforced; Plugin Manager settings page + restart flow.
- Registry unification with Task 40's node/component registry (that design
  already reserves runtime registration "for future plugins").
- **First candidates, in extraction order:**
  1. **Physics (Rapier)** — currently hard-wired; extracting it proves the
     seam on the hardest case (2D physics becomes a sibling plugin later)
  2. **Steam SDK** (steamworks wrapper: achievements, overlay, rich presence)
  3. **Gameplay Ability System** — refactor M7's server-authoritative ability
     code into a reusable plugin
- **Discipline starting now** (costs nothing): new subsystems are built as
  self-contained modules with a single registration entry point, so extraction
  into plugins is mechanical, not surgery.

</details>

---

## Phase 11: Node Graph Foundation & Game Architecture (Tasks 40-45)

**Deferred:** this phase and Phases 12–15 now run **after** the Multiplayer Foundation phase (M0–M8). Per-task annotations below mark what was absorbed into M-tasks.

### Task 40: Node Graph Framework & Custom Node SDK
**Status:** ✅ Complete — plan + close-out: `VULKANO-40-NODE-GRAPH-FRAMEWORK.md` (all packages P0–P9 shipped; minimap cut after hand-testing in favor of F/A frame shortcuts; crusty-gui gained the Phase 22 `Canvas` pan/zoom primitive)
**Duration:** ~2-2.5 weeks
**Prerequisites:** Refactor Checkpoint #5

This is the highest-risk, highest-value infrastructure task in the roadmap. The node graph framework is used by six subsequent tasks (animation, scripting, materials, VFX, audio, AI). Getting it right matters more than getting it fast.

**Phase A — MVP (~1.5 weeks):**

*Graph Data Model:*
- Graph, node, pin, edge as plain data structs (no behavior, pure data)
- **Stable IDs** for node types and pins — string slugs, not display names (`id = "set_world_position"` is stable, `name = "Set World Position"` is a display label that can change freely without breaking saved graphs)
- Pin type system: base types (float, vec2, vec3, vec4, color, bool, enum, texture, mesh, entity, execution flow), extensible per domain
- **Realm metadata** on node types: `editor`, `client`, `shared`, `server_safe` — validated at graph compile time, prevents authority violations before networking even exists
- **Pure/impure classification**: pure nodes compute values (cacheable, no side effects), impure nodes emit commands (require exec flow pins, ordered execution)
- **Deterministic flag**: same inputs always produce same outputs (matters for networking, replay)

*Node Registry:*
- `NodeRegistry` with manual `register()` API — the core registration mechanism
- Nodes registered by stable ID, looked up by ID, displayed by name/category
- Registry supports runtime registration (for future plugins/hot-reload), not just static linking

*Graph Editor UI:*
- Canvas viewport with pan (middle-mouse drag) and zoom (scroll)
- Node rendering: header with name/color, typed input/output pins, body content area
- Connection drawing with type validation (float→float OK, float→texture ERROR, exec→exec OK)
- Node search/create menu (right-click → type to search → filtered by category)
- Selection, multi-select, box select, delete, copy/paste, duplicate
- Node grouping and comment boxes (organize complex graphs visually)
- Minimap for navigation in large graphs

*Subgraphs:*
- **Subgraph / reusable graph functions** — a graph that can be used as a single node inside another graph
- Defined inputs/outputs become the subgraph node's pins
- Prevents spaghetti: users create a "DamageCalculation" subgraph and reuse it everywhere
- Subgraphs are assets (saved/loaded independently, referenced by other graphs)

*Serialization & Versioning:*
- Graph asset serialization (save/load as RON assets, keyed by stable IDs)
- **Graph versioning with schema migration**: when a node type changes (pin added, removed, renamed, type changed), saved graphs that use that node type are migrated automatically
- Version number per node type definition, migration functions registered per version

**Phase B — Macro Layer (~3-4 days):**

*Proc Macro Crate (`node_graph_macros`):*
- Shared derive logic: parse struct fields, extract pin attributes, generate `NodeDescriptor` implementation
- Type mapping: Rust types → pin types (f32 → Float, glm::Vec3 → Vec3, Entity → Entity, ExecPin → Exec)
- Metadata extraction: id, name, category, pure/impure, realm, deterministic
- `inventory`-based auto-registration as one backend feeding into `NodeRegistry`
- Manual registration remains available (for runtime plugins, dynamic loading, hot-reload)
- First domain macro: `#[derive(AnimationNode)]` (immediately used by Task 41)

*Custom Node Declaration:*
```rust
#[derive(ScriptNode)]
#[node(
    id = "damage_zone",           // Stable ID for serialization/migration
    name = "Damage Zone",         // Display label (can change freely)
    category = "Gameplay",        // Search menu organization
    pure = false,                 // Emits commands (impure)
    realm = "shared",             // Valid in client + server graphs
    deterministic = true,         // Same inputs → same outputs
)]
pub struct DamageZone {
    #[input(pin = "exec")]
    exec_in: ExecPin,
    #[input(label = "DPS")]
    dps: f32,
    #[input(label = "Radius")]
    radius: f32,
    #[output(pin = "exec")]
    exec_out: ExecPin,
    #[output(label = "Entities Hit")]
    hit_count: i32,
}
```

**Non-goals for Task 40:**
- Domain-specific node libraries (each consumer task builds its own)
- Domain-specific compilers/evaluators (each consumer implements its own backend)
- Any game-specific logic

**What you'll learn:**
- Graph data structure design and traversal algorithms
- Proc macro development in Rust (derive macros, attribute macros)
- egui custom widget development (canvas, nodes, connections)
- Asset versioning and schema migration patterns
- Type system design for visual programming

---

### Task 41: Animation Graph (Node-Based)
**Status:** 📋 Planned
**Duration:** ~2 weeks
**Prerequisites:** Task 40

First consumer of the Node Graph Framework.

**What you'll build:**
- `#[derive(AnimationNode)]` proc macro (domain-specific macro built on Task 40 infrastructure)
- Animation graph runtime: evaluates a graph of animation nodes to produce a final pose each frame
- State machine nodes: states as nodes, transitions as directed edges with conditions (parameter thresholds, triggers)
- Blend tree nodes: 1D blend (walk→run by speed parameter), 2D directional blend (8-way movement), additive blending, layered blending
- Parameter nodes: float, bool, trigger — drive transitions and blend weights
- Animation clip nodes: play clip, sample at time, loop/clamp modes
- Animation events: fire callbacks at specific keyframes (e.g., trigger footstep SFX via Task 31 audio)
- Preview in viewport: play animation graph result on selected entity
- Animation graph asset type: save/load graphs, reference from `Animator` ECS component
- Inspector-based authoring remains available for simple cases (single clip playback, basic crossfade)

**What you'll learn:**
- Animation state machine design
- Blend tree evaluation (recursive tree evaluation producing weighted pose)
- Graph compilation for animation (optimization, constant folding)
- Layer-based animation (upper body attack + lower body locomotion)
- How Unreal Animation Blueprints work conceptually

**Key concepts:**
- Pose = array of bone transforms (one per joint in the skeleton)
- Blend = weighted interpolation between two poses (slerp for rotations, lerp for positions)
- State machine: current state + transition rules + parameter evaluation
- Layers: evaluate multiple state machines independently, composite results with masks

```rust
#[derive(AnimationNode)]
#[node(id = "speed_blend", name = "Speed Blend", category = "Locomotion")]
pub struct SpeedBlend {
    #[input(label = "Speed")]
    speed: f32,
    #[input(label = "Walk Clip")]
    walk: PoseInput,
    #[input(label = "Run Clip")]
    run: PoseInput,
    #[output(label = "Result")]
    result: PoseOutput,
}
```

---

### Task 42: Save/Load & Runtime Persistence
**Status:** 🔀 Absorbed into Task M5 for networked state (SpacetimeDB tables are the save system). A local single-player save format remains deferred here.
**Duration:** ~1-1.5 weeks
**Prerequisites:** Task 41

**What you'll build:**
- Separation of editor scene format from game save format
- Editor scenes: full-fidelity RON with EntityGuid, editor metadata, all components (existing format, unchanged)
- Game save format: runtime state only — player position, inventory, quest flags, world mutations (enemies killed, doors opened, items collected)
- Save data defined using `game_shared` types from Task 34
- Checkpoint/quicksave system: capture current game state to a save slot
- Save slot management: multiple save files, metadata (timestamp, screenshot thumbnail, play time)
- Save/load UI hooks for in-game UI (Task 55)

**What you'll learn:**
- Separating authored content from runtime state
- Save game architecture (what to persist, what to reconstruct)
- Delta-based saves (only save what changed from the base scene)
- Save compatibility and versioning

**Why now:**
- Play mode snapshot/restore (Task 24) already serializes scenes — but game saves are different
- Editor scene format contains editor-only data (EntityGuid, metadata) games don't need
- Games need data (player progress, world mutations) that scenes don't contain
- Animation graphs and audio settings need save/restore support

---

### Task 43: Scene Management & Transitions
**Status:** 🔀 Absorbed into Task M4 (zone/chunk lifecycle). General transitions/loading screens stay deferred here.
**Duration:** ~1-1.5 weeks
**Prerequisites:** Task 42

**What you'll build:**
- Multiple scene support with additive loading (load a UI scene on top of a game scene)
- Scene transitions: fade to black, crossfade, custom transition effects
- Per-scene settings: ambient light color/intensity, fog parameters, post-processing overrides, sky configuration
- Scene registry: list of all scenes in the project, metadata per scene
- Scene asset type integrated with asset browser
- Main menu → loading screen → game level workflow
- Scene unloading with proper cleanup (despawn entities, unload scene-specific assets)

**What you'll learn:**
- Scene lifecycle management (load, initialize, update, unload)
- Additive scene loading patterns
- Transition effect implementation (render both scenes during transition)
- Resource management across scene boundaries

**Why now:**
- Real games have multiple levels, menus, loading screens
- Current single-scene architecture can't support a shipping game
- Validates the build pipeline (Task 25) for multi-scene games

---

### Task 44: Asset Cooking & Level Streaming
**Status:** 📋 Planned
**Duration:** ~1.5-2 weeks
**Prerequisites:** Task 43

**⚠️ High-risk scope** — may need splitting into "Asset Cooking Pipeline" and "Level Streaming" during execution.

**What you'll build:**
- Cooked asset format: convert raw editor assets (glTF, PNG, RON) into optimized runtime format (pre-processed meshes, compressed textures, binary scene data)
- Import rules: per-asset-type processing settings (texture compression format, mesh LOD generation, audio quality)
- Dependency tracking: know which assets depend on which (material references texture, scene references mesh)
- Cache invalidation: re-cook only assets whose source files or dependencies changed
- Asset versioning: cooked assets include version number, outdated assets trigger re-cook
- Trigger-volume-based level streaming: `StreamingVolume` component, async load/unload scene chunks when player enters/exits
- Background loading with progress reporting (loading screen integration)
- Upgrade `.pak` format (Task 25) to support cooked assets
- Replace load-whole-pak-into-RAM with mmap/ranged reads (pak size must stop costing RAM before content grows)
- Multiple paks mounted in priority order: base + patch paks (ship updates as small overlay paks) — same mechanism serves DLC
- Chunk paks by locality (per-zone / per-asset-type) so streaming reads are contiguous and platform install-chunks map cleanly

**What you'll learn:**
- Asset pipeline architecture (import → process → cook → package)
- Texture compression formats (BC1-BC7 for GPU-friendly runtime loading)
- Dependency graph construction and traversal
- Incremental build systems (cook only what changed)
- Async loading patterns (load in background, integrate when ready)
- Streaming architecture (what to load, when to load, when to unload)

**Why now:**
- Raw assets are slow to load at runtime (parse glTF, decode PNG every launch)
- Level streaming enables large worlds that don't fit in memory
- Dependency tracking prevents "I changed a texture and broke 3 scenes"
- Foundation for terrain (Task 46) and large outdoor scenes

---

### Task 45-A: Graph Execution Core (Visual Scripting v1)
**Status:** ✅ **Complete** (2026-08-11). Plan + rulings:
[`VULKANO-45A-GRAPH-EXECUTION-CORE.md`](VULKANO-45A-GRAPH-EXECUTION-CORE.md).

Graphs run. A `.graph` on an entity executes Blueprint-style — events, exec
control flow, lazily-pulled pure data chains, per-graph variables, latents
that suspend across frames — with execution visible in the editor. The
interpreter is engine-independent and deterministic, so the same code can
later compile into the SpacetimeDB module (D8, the M6 insurance policy
applied to scripting).

**Commit map**

| Pkg | Commit | What landed |
|---|---|---|
| P1 | `45b4871` | Int(i32)/String/Array pin types, `DocDescriptors` resolver, `graph_input`/`graph_output`, `VarDecl` + container v1→v2 migration, data-cycle validation, `NodeInst.title` |
| P2 | `dbc667f` | `node_graph_types` extraction (wasm-clean), `node_graph_exec` interpreter, 5-stage compile, execution threads + budget + phased event queue, determinism test |
| P3 | `72b05d0` | Exec fan-in ruling, 46-node std library, entered-pin tracking, per-node state |
| P4 | `f120d40` | Latent machinery: suspend-time continuation, `EventPhase::Latent` drain, `Delay`, mid-wait serialization |
| P5 | `902431d` | `GraphScriptingPlugin` (feature-gated, strips from exports), `GraphRunner` + 4 persistence seams, `GraphPlanCache`, effect application + spawn-alias binding |
| P6a | `f1169bc` | Int/String inline field widgets, Array read-only chip, reserved config-row gap |
| P6b | `a235dcb` | Variables model: 5 undoable edits, no-coercion retype, per-gesture coalescing |
| P6c | `e4a16dc` | Variables panel, config-row band via `band_y`/`node_h`, Alt+V |
| P7 | `7b34a0d` | `TraceSink` generic (zero-cost `NoTrace`), `GraphTrace` ring, plan→doc mapping, wire pulse + node rings + value hover |
| P8a | `1991247` | `curve_asset` crate (time-scaled CR Hermite), `AssetType::Curve`, Timeline node, `CurveCache` shared by compiler + runtime |
| P8b | `5a452c9` | Curve editor tab (`Canvas` plot, `CurveEditStack`, atomic save), `DocResolvers` curve wiring, `CurveChanged` hot-reload |
| P9 | _(this commit)_ | Showcase demo verbatim, acceptance sweep, docs, close-out |

**Acceptance** — every plan line, with what proves it:

| Plan line | Evidence |
|---|---|
| Headless fixture: Branch + ForLoop + variables + subgraph → expected effect stream | `walking_skeleton::the_acceptance_fixture_runs_branch_loop_variables_and_a_subgraph` (P9) |
| Determinism test green | `walking_skeleton::determinism_holds_across_runs` (P2) |
| Budget kills an infinite WhileLoop with a reported error | `budget_kills_a_runaway_loop` + `budget_kills_a_runaway_and_names_the_node` (P2) |
| Demo graph: BeginPlay spawns prefabs in a ForLoop, Tick moves one via Timeline, Delay chains fire | `acceptance::the_committed_demo_shows_all_three_behaviours` over the committed `runner_demo.graph` + `duck_hop.curve` + `graph_cube.prefab` (P9) |
| …all visible via execution pulse | `trace.rs` suite + `graph_editor_crusty::a_pulse_resolves_through_reroutes` (P7) |
| play → stop → play restarts cleanly | `acceptance::stopping_and_replaying_refires_begin_play_exactly_once` (P5) |
| Editor authors all of it without touching RON | typed constants `graph_editor_crusty::int_and_string_are_editable_arrays_are_not` (P6a); variables `graph_variables_tests` (9 tests, P6b); wire-drag create `graph_palette::auto_connect_picks_the_type_then_the_closest_name` (P7) |
| Realm gate: a `Server` graph on a client errors visibly and does not run | `acceptance::the_realm_gate_refuses_a_server_graph_on_a_client` (P5) |
| Portability (D8) | CI checks `node_graph_types` / `node_graph_exec` / `curve_asset` standalone **and** for `wasm32-unknown-unknown` (P9) |
| All Task 40 gates hold | tests, clippy (no new warnings), `lint_design.sh`, both builds — every package |

**Deferred ledger.** Everything the arc deliberately left, in one place.
Items marked **45.5** are already in that task's backlog; the rest need a
home when the owning task is scheduled.

*Owned by Task 45.5 (Node Graph Editor v2)*
- Flow bubbles, watch chips, PAUSED state and tinted taken-path rendering —
  45-A P7 ships the trace *data* and basic pulse/value hover only (addendum
  ruling 11).
- Persisted breakpoints in the graph sidecar.
- Per-instance subgraph inspection: an inlined subgraph lights its *host*
  node whole; stepping into one instance of it is 45.5.
- Collapse-to-subgraph inner wiring UI, and node rename via `NodeInst.title`
  + F2 — 45-A ships the schema field and the compiler splicing, not the UI.

*Owned by Task 45 (Visual Scripting tail)*
- Broader gameplay node API beyond the 46-node starter library.
- `event_custom` cross-entity targeting — v1 is same-entity only (resolved
  question 3); an explicit target pin is the follow-up.

*Owned by Task 41 (Animation)*
- **Timeline/Delay coupling**: both are latents on one activation, so a
  Timeline cannot run *while* a Delay on the same activation waits. Blueprint
  decouples them with per-node timelines driven independently of exec flow;
  that decoupling is Task 41's, together with growing the `.curve` editor
  (which 45-A shipped deliberately basic).

*Unowned — schedule with whichever task next touches the area*
- **Exec output fan-out is unvalidated**: two wires off one exec output are
  not rejected, and `PlanNode::exec` keeps one target per pin, so one wire
  silently wins. Wants a `validate_doc` rule (and the editor replacing rather
  than adding on connect).
- **Suspend-edge trace gap**: the exec edge that *enters* a latent is traced,
  but the resume is a new activation step with no incoming edge to light, so
  a `Delay` resuming reads as a graph that started by itself.
- **Array literal editing**: `ForEach` runs and arrays flow through pins, but
  editing an array *constant* on the canvas is not authorable — the inline
  cell is a read-only chip (plan D6 note).
- **Docked-undo focus quirk**: per-file editors claim Undo/Redo only while
  their tab is the focused tab of the *main* dock; a float window owns its
  own keyboard. Consistent between graph and curve editors, but it means the
  Edit menu's labels follow dock focus rather than last-edited document.
- **Config-band label alignment across nodes**: config rows are aligned
  within a node (`band_y`), not across neighbouring nodes, so two adjacent
  nodes with different config counts have visually unrelated first pin rows.
- **Severity badge rendering**: P1 added `ErrorSeverity` and the anchor
  taxonomy; warnings and errors still render with the same badge treatment.
- **Curve editor visual verification (45-A P8b)**: the three review
  screenshots (curve editor with `duck_hop.curve`, a key selected, a Timeline
  node showing its track pins) could not be captured — the workstation locked
  mid-session and GDI capture needs an unlocked desktop. Substituted by two
  independent code reviews with fixes applied; a manual pass is queued.

---

### Task 45: Visual Scripting (Node-Based)
**Status:** 📋 Planned — **superseded in part by Task 45-A** (`docs/roadmap/VULKANO-45A-GRAPH-EXECUTION-CORE.md`, audited 2026-08-11): the execution runtime, `GraphRunner` component (not `ScriptComponent`), events, control flow, latents, and the starter node library land in 45-A. Task 45 becomes the tail on that runtime: broader gameplay node API, debugging beyond basic viz, polish. Bullets below predate 45-A where they conflict.
**Duration:** ~2-2.5 weeks (reduced by 45-A)
**Prerequisites:** Task 45-A

Second consumer of the Node Graph Framework.

**What you'll build:**
- `#[derive(ScriptNode)]` proc macro (domain-specific macro)
- Visual scripting runtime: graph evaluator that processes execution flow and data flow
- Script graph asset type: save/load, reference from `ScriptComponent` ECS component

*Node Classification:*
- **Pure nodes** (Add, GetPosition, Compare, Distance): compute values with no side effects. No exec pins required. Can be evaluated on hover in the editor for debugging. Cacheable. `realm = "shared"` by default.
- **Impure nodes** (SpawnEntity, PlaySound, ApplyDamage, SetTransform): emit commands/events through the command buffer. Require exec flow pins. Execution order matters. `NodeContext` provides command buffer access for impure nodes, read-only world queries for pure nodes. Realm must be explicitly declared.

*Built-in Node Library:*
- Event nodes: on_start, on_update, on_collision, on_trigger, on_input_action
- Flow control: branch, for-each, sequence, gate, delay, do-once
- Data nodes: get/set component, get resource, math (add/sub/mul/div/mod), comparison, variable get/set
- Action nodes: spawn entity, despawn entity, play sound, apply force, set transform, send event
- Entity reference nodes: self, find by name, find by tag
- Debug nodes: print, draw debug line/sphere

*Debugging:*
- Active execution path highlighting during play mode (nodes glow as they execute)
- Breakpoints on nodes (pause execution, inspect pin values)
- Watch window: monitor pin values in real time

*Expression nodes:*
- Optional inline expression evaluation for complex math (avoids building 10 nodes for `health * 0.5 + armor`)

**What you'll learn:**
- Visual programming language design
- Execution flow vs data flow graph evaluation
- Graph compilation and optimization
- Debugging tools for visual scripts
- Command pattern integration with visual authoring

```rust
#[derive(ScriptNode)]
#[node(
    id = "damage_zone",
    name = "Damage Zone",
    category = "Gameplay",
    pure = false,
    realm = "shared",
    deterministic = true,
)]
pub struct DamageZone {
    #[input(pin = "exec")]
    exec_in: ExecPin,
    #[input(label = "DPS")]
    dps: f32,
    #[input(label = "Radius")]
    radius: f32,
    #[output(pin = "exec")]
    exec_out: ExecPin,
    #[output(label = "Entities Hit")]
    hit_count: i32,
}

impl DamageZone {
    #[node_execute]
    fn execute(&self, ctx: &mut NodeContext) -> NodeResult {
        let dps = ctx.read_input::<f32>("dps")?;
        let radius = ctx.read_input::<f32>("radius")?;
        let dt = ctx.resource::<Time>()?.delta();

        let mut count = 0;
        for (entity, (transform, health)) in ctx.query::<(&Transform, &mut Health)>() {
            if transform.position.distance(ctx.owner_position()) < radius {
                health.current -= dps * dt;
                count += 1;
            }
        }

        ctx.write_output("hit_count", count);
        ctx.trigger_exec("exec_out")
    }
}
```

---

### Task 45.5: Node Graph Editor v2 — Refactor & Polish
**Status:** 📋 Planned (user-requested 2026-08-02: "refactor it again and improve")
**Duration:** ~1.5–2 weeks
**Prerequisites:** Task 45-A (graph execution core) and ideally Task 45 — most
of the deferred ledger unblocks only once the evaluator and the `NodeInst`
schema additions exist.

One consolidated pass over everything the Task 40 + design-system +
input-model arcs deliberately deferred. The authoritative backlog is the
deferred ledger in `docs/mockup/AUDIT.md` (close-out + Pass C sections);
snapshot of it as of scheduling:

- **Unblocked by 45-A** (do first): node rename via `NodeInst.title`
  override + F2 · breakpoint persistence in the sidecar + debug visuals
  (watch chips, execution trace, PAUSED, tinted taken path) · flow bubbles ·
  collapse-to-subgraph inner wiring (`graph_input`/`graph_output`).
- **Input-model residue**: Q straighten (needs geometry access from the
  state layer — small architecture decision) · Shift+Del exec-chain heal ·
  quick-place (`descriptor.quick_key`, three crates) · Unreal mouse profile
  consumption + Ctrl+drag-from-pin move-all-connections (multi-edge
  ConnectDrag) · Alt+click group-title boundary break (title-region split).
- **Design-system residue**: crossings rendering (Gap/Arc/Circle over the
  existing broadphase cap) · bundling proper (perpendicular offsets, merge,
  max-8) · Vec2/3/4 two-line axis fields · L1 value edit popup ·
  on-canvas color picker · `preserved` distinct visual · full
  asset-reference field on canvas · pinned nodes (auto-layout exemption).
- **Structural refactors earned by three arcs of accretion**:
  `graph_editor_crusty.rs` has grown past 4k lines — split into
  draw/interact/menu modules · edges sort-canonicalization (the recorded
  next diff-noise source) · crusty Phase 17 (retained tessellation /
  culling) when the 2k-node budget is actually approached · revisit the
  glass-vs-alpha unsure list and the palette 0.96 ruling.

Re-audit AUDIT.md at kickoff — the ledger is live and later tasks may have
closed or added items.

---

## Phase 12: World Building (Tasks 46-49)

### Task 46: Terrain System
**Status:** 📋 Planned
**Duration:** ~2 weeks
**Prerequisites:** Task 44

**What you'll build:**
- Heightmap-based terrain (16-bit PNG import for fine elevation control)
- Chunked mesh with distance-based LOD (near chunks high detail, far chunks simplified)
- 4-layer texture splatting with blend map (paint different textures: grass, dirt, rock, sand)
- Terrain brush tools in editor: raise, lower, smooth, flatten, paint texture, set height
- Rapier heightfield collider for physics (characters and objects interact with terrain)
- Terrain normals computed from heightmap for correct lighting
- Terrain material with PBR support (normal mapping, roughness variation per texture layer)
- Terrain chunk streaming integration with Task 44 (load/unload terrain chunks by distance)

**What you'll learn:**
- Heightmap-based terrain generation
- Terrain LOD algorithms (CDLOD, quadtree-based)
- Texture splatting in fragment shader (blend 4 textures by splat map weights)
- Terrain editing tools (raycasting for brush placement, heightmap modification)
- Heightfield collision shape generation

**Why now:**
- Can't make outdoor games without terrain
- Foundational world-building tool that pairs with sky (Task 47)
- Exercises renderer with large meshes and editor with custom tool modes
- Level streaming (Task 44) enables large terrain that doesn't fit in memory

**Key concepts:**
- Heightmap: 2D grid of elevation values, typically 16-bit for precision
- Chunk: square section of terrain (e.g., 64x64 vertices), independently renderable and LOD-switchable
- Splat map: RGBA texture where each channel stores blend weight for one terrain texture
- Terrain brush: raycast to terrain surface, modify heightmap/splat map in a radius

---

### Task 47: Environment, Sky & Atmosphere
**Status:** 📋 Planned
**Duration:** ~1.5 weeks
**Prerequisites:** Task 46

**What you'll build:**
- Procedural sky rendering (Preetham or Hosek-Wilkie atmospheric scattering model) or HDR cubemap skybox fallback
- Time-of-day system: sun entity angle drives directional light direction, color, and intensity automatically
- Day/night cycle with configurable speed (or manual time control in editor)
- Distance fog (exponential, exponential squared) with configurable density and color
- Height fog (fog density increases below a configurable altitude)
- Baked environment probes: cubemap capture at specified positions for metallic surface reflections
- Skybox rendering integrated into render graph (drawn after opaque geometry, before transparent)
- Editor controls: time-of-day slider, fog parameters, sky model selection

**What you'll learn:**
- Atmospheric scattering theory (Rayleigh for blue sky, Mie for sun glow)
- Cubemap rendering and sampling
- Fog integration in the lighting pass
- Environment probe capture and usage (sample cubemap for metallic reflections)
- Time-of-day system design (parameterize everything by sun angle)

**Why now:**
- Terrain without sky looks wrong — together they create believable outdoor scenes
- Time-of-day provides a dramatic demo scene (sunrise, sunset, night)
- Environment probes significantly improve metallic material quality
- Fog adds depth and atmosphere to large scenes

---

### Task 48: Cascaded Shadow Maps & Shadow Quality
**Status:** 📋 Planned
**Duration:** ~1.5-2 weeks
**Prerequisites:** Task 47

**What you'll build:**
- Cascaded Shadow Maps (CSM) for directional lights: 3-4 cascades covering near to far range
- Stable cascade splits (logarithmic + uniform hybrid, or manual per-cascade distances)
- Shadow map atlas: all cascades packed into a single large texture (e.g., 4096x4096 atlas, 4x 2048x2048 cascades)
- Per-cascade frustum fitting: tight bounding for each cascade to maximize shadow texel density
- Cascade blending: smooth transition between cascades (avoid visible seams)
- Percentage-Closer Soft Shadows (PCSS) or configurable PCF kernel sizes
- Shadow distance fade (shadows fade out beyond a configurable distance)
- Debug visualization: cascade boundaries drawn using debug draw (color-coded)

**What you'll learn:**
- CSM theory and implementation (split view frustum into depth slices)
- Shadow map atlas packing and UV coordinate calculation
- Cascade stability techniques (texel snapping to prevent shimmer)
- Soft shadow algorithms (PCSS penumbra estimation)
- Shadow rendering performance tuning

**Why now:**
- Current single shadow map breaks at terrain scale (not enough resolution)
- CSM is the industry standard for directional light shadows in open worlds
- Focused task — just shadows, done properly
- Required for believable outdoor scenes with terrain

---

### Task 49: Advanced Lighting, IBL & GI
**Status:** 📋 Planned
**Duration:** ~2.5 weeks
**Prerequisites:** Task 48

**⚠️ High-risk scope** — may need scoping down to "point shadows + spot lights + IBL + emissive" with full GI deferred to a later task.

**What you'll build:**

*Multi-Light Support:*
- Point light support in the deferred pipeline with light volumes (sphere proxy geometry)
- Point light cubemap shadows: render 6 faces of a cubemap depth texture per shadow-casting point light
- Spot light type: cone-shaped light with configurable angle, falloff, and optional shadow map
- Light culling optimization: only evaluate lights that affect visible pixels (tile-based or clustered deferred)
- Emissive materials feeding into bloom (Task 37 integration): emissive surface color contributes to bloom pass

*Image-Based Lighting (IBL):*
- Environment cubemap sampling for indirect specular reflections (metallic surfaces reflect the environment)
- Pre-filtered environment map (mip chain for roughness-based filtering)
- Irradiance map for indirect diffuse lighting (replaces flat ambient with environment-derived ambient)
- BRDF integration lookup texture (split-sum approximation)
- Per-scene environment map assignment (editor-configurable)
- Fallback: use sky/atmosphere from Task 47 as the environment source when no explicit cubemap is set

*Global Illumination:*
- Screen-space GI or irradiance probes for indirect illumination (ambient light that bounces off surfaces)
- Light probe baking workflow: place probes in editor, bake indirect lighting, sample at runtime

**What you'll learn:**
- Cubemap shadow rendering (6-pass per light, or geometry shader single-pass)
- Spot light math (cone angle, falloff, spotlight matrix for shadows)
- IBL theory: split-sum approximation, pre-filtered environment maps, irradiance convolution
- Spherical harmonics for efficient irradiance storage
- Global illumination techniques (screen-space, probe-based, or hybrid)
- Clustered or tiled deferred lighting for many lights

**Why now:**
- IBL is what makes idle scenes look alive — ambient comes from the environment, not a flat constant
- Point light shadows transform indoor scenes (currently point lights cast no shadows)
- Spot lights enable flashlights, lamps, headlights — essential light type
- Emissive materials with bloom make neon, fire, magical effects glow realistically
- GI (even approximate) eliminates the flat look of purely direct lighting

---

## Phase 13: Visual Authoring Expansion (Tasks 50-54)

### Task 50: Visual Material Editor (Node-Based)
**Status:** 📋 Planned
**Duration:** ~2 weeks
**Prerequisites:** Task 40, Task 39

Third consumer of the Node Graph Framework.

**What you'll build:**
- `#[derive(MaterialNode)]` proc macro
- Material graph runtime: evaluates a graph of material nodes to produce shader parameters or generate GLSL code
- **Compiles to GLSL**: the graph generates shader source code that is compiled into a Vulkan pipeline
- Live preview on a sphere/plane (re-renders preview whenever graph changes)
- Material permutation management (transparency, double-sided, masked — each generates a shader variant)

*Built-in Node Library:*
- Texture nodes: texture sample (albedo, normal, etc.), texture coordinate, sampler settings
- Math nodes: add, subtract, multiply, divide, lerp, clamp, power, abs, min, max, fract, step, smoothstep
- Vector nodes: combine, split, dot product, cross product, normalize, transform
- UV nodes: tiling/offset, panner (animated UV scroll), rotator, triplanar mapping
- Constant nodes: color, float, vec2, vec3, vec4
- World data nodes: world position, world normal, camera direction, time, screen UV
- Utility nodes: fresnel, noise (perlin, simplex), distance
- PBR output node: connects to albedo, normal, metallic, roughness, emissive, opacity, AO slots

- Inspector-based material editing remains for simple PBR parameter tweaks
- Material graph assets saved/loaded independently, referenced by material instances
- If materials become a workflow bottleneck during world building (Tasks 46-49), this task can be pulled earlier

**What you'll learn:**
- Shader code generation from a visual graph
- GLSL code composition and function generation
- Material permutation and shader variant management
- Live preview rendering (render-to-texture for preview sphere)

```rust
#[derive(MaterialNode)]
#[node(id = "triplanar_mapping", name = "Triplanar Mapping", category = "UV")]
pub struct TriplanarMapping {
    #[input(label = "Texture")]
    texture: TextureHandle,
    #[input(label = "Sharpness", default = 1.0)]
    sharpness: f32,
    #[output(label = "Color")]
    color: Vec4,
}
```

---

### Task 51: VFX Graph (Node-Based)
**Status:** 📋 Planned
**Duration:** ~2 weeks
**Prerequisites:** Task 40, Task 38

Fourth consumer of the Node Graph Framework. Upgrades Task 38's inspector-based particles to full node-based authoring (similar to Unreal Niagara).

**What you'll build:**
- `#[derive(VfxNode)]` proc macro
- VFX graph runtime: evaluates particle behavior as a graph (each particle's lifecycle is a graph)
- **Compiles to compute shader** dispatch or CPU simulation depending on complexity
- Live preview in editor (particles play in the viewport as you edit the graph)

*Built-in Node Library:*
- Emitter nodes: spawn rate, burst, emission shape (point, sphere, cone, box, mesh surface)
- Initialize nodes: set initial position, velocity, size, color, lifetime
- Update nodes: apply force, noise, drag, orbit, attract/repel, size over life, color over life
- Render nodes: billboard, mesh particle, ribbon/trail, lit particle
- Condition nodes: kill if below ground, kill if too old, collide with scene
- Math/utility nodes: random, noise, remap, curve sample

- Inspector-based authoring (Task 38) remains for simple emitters
- VFX graph assets saved/loaded independently
- Subgraphs for reusable VFX behaviors (e.g., "flame flicker" subgraph used in torch, campfire, explosion)

**What you'll learn:**
- Particle system architecture at scale (Niagara-style)
- Compute shader code generation from visual graph
- GPU simulation pipeline design
- How to design a VFX authoring tool

```rust
#[derive(VfxNode)]
#[node(id = "spiral_force", name = "Spiral Force", category = "Forces")]
pub struct SpiralForce {
    #[input(label = "Strength")]
    strength: f32,
    #[input(label = "Radius")]
    radius: f32,
    #[input(label = "Angular Speed")]
    angular_speed: f32,
    #[output(label = "Force")]
    force: Vec3,
}
```

---

### Task 52: Prefab Variants & Nested Prefabs
**Status:** 📋 Planned
**Duration:** ~1.5 weeks
**Prerequisites:** Task 45

**What you'll build:**
- Nested prefabs: prefab instances inside other prefabs (a "building" prefab contains "furniture" prefabs)
- Property overrides per-instance: change a value on an instance without breaking the prefab link (one red door in a building of blue doors)
- "Apply" workflow: push instance changes back to the base prefab (apply override to all instances)
- "Revert" workflow: discard instance overrides, return to base prefab values
- Visual indicator in hierarchy panel: distinguishing prefab instances from regular entities (icon, color)
- Drag prefab from asset browser into scene (creates instance)
- Override visualization in inspector: modified properties highlighted, revert button per-property
- Prefab asset type with proper dependency tracking

**What you'll learn:**
- Prefab override architecture (base values + override layer)
- Nested instantiation (recursive prefab resolution)
- Diff-based serialization (only save what differs from base)
- Unity/Unreal prefab/blueprint variant patterns

**Why now:**
- Current prefab system is basic (instantiate, no link back to source)
- Proper prefab workflow is what makes large scenes manageable
- EntityGuid system (Task 24) provides the identity infrastructure needed for prefab tracking

---

### Task 53: Audio Graph (Node-Based)
**Status:** 📋 Planned
**Duration:** ~1.5-2 weeks
**Prerequisites:** Task 40, Task 31

Fifth consumer of the Node Graph Framework. Upgrades Task 31's basic audio with visual authoring for complex soundscapes and adaptive audio (similar to Unreal MetaSounds).

**What you'll build:**
- `#[derive(AudioNode)]` proc macro
- Audio graph runtime: evaluates audio processing chain in real-time
- Audio graph asset type: save/load, reference from `AudioSource` ECS component

*Built-in Node Library:*
- Source nodes: audio clip playback, random clip selector, sequence player, loop/one-shot
- Mixer nodes: volume, pan (stereo placement), crossfade between sources
- DSP effect nodes: reverb, delay/echo, low-pass filter, high-pass filter, distortion, chorus
- Spatial nodes: 3D positioning, distance attenuation curve, occlusion (ray-based), direction-based filtering
- Logic nodes: parameter-driven switching (play "combat" music when enemies nearby), random selection, blend by parameter
- Envelope nodes: fade in/out, ADSR envelope
- Output node: final output to audio bus

- Inspector-based audio setup remains for simple one-shot SFX and music tracks
- Audio graph enables adaptive audio: music that responds to gameplay, environmental soundscapes that change with time of day

**What you'll learn:**
- Real-time audio graph processing
- DSP fundamentals (filters, reverb algorithms, delay lines)
- Adaptive audio design (parameter-driven music systems)
- Audio graph evaluation scheduling (must run at audio sample rate, not game frame rate)

---

### Task 54: Editor Workflow Polish
**Status:** 📋 Planned
**Duration:** ~1.5 weeks
**Prerequisites:** Task 52

**What you'll build:**
- Full undo/redo coverage for ALL editor operations:
  - Hierarchy changes (reparent, reorder)
  - Entity creation and deletion
  - Component add/remove
  - Prefab override changes
  - Graph node operations (add, delete, connect, disconnect)
  - Transform gizmo operations
  - (Inspector property editing already has undo — extend to everything else)
- Multi-entity selection with box select in viewport (drag rectangle to select multiple entities)
- Batch operations on selection: move, delete, duplicate, group, hide/show
- Editor preferences panel: key bindings, grid settings, snap values, gizmo sizes, theme/colors, auto-save interval
- Project settings panel: physics gravity, default scene, build configuration, rendering quality presets
- Search and filtering improvements: global search across hierarchy, assets, and components
- Recent files / recent scenes quick access
- Editor layout save/load (persist panel arrangement across sessions — already partially implemented with egui_dock)

**What you'll learn:**
- Command pattern for comprehensive undo/redo
- Box selection with frustum intersection testing
- Batch operation patterns (operate on selection set)
- Editor settings persistence and migration

**Why now:**
- These quality-of-life improvements individually are small but collectively transform editor usability
- Full undo/redo is the #1 frustration in any editor — accidentally deleting something with no undo kills productivity
- Multi-select is essential for productive scene editing at scale

---

## Phase 14: Production Features (Tasks 55-57)

### Task 55: In-Game UI Framework
**Status:** 🔀 Thin-HUD slice absorbed into Task M7 (crusty-gui action bar / cast bar / target frame). The general framework stays deferred here.
**Duration:** ~2 weeks
**Prerequisites:** Task 54

**What you'll build:**
- Retained-mode UI system separate from egui (egui is for editor, this is for in-game UI)
- Anchored layout system: position elements relative to screen edges/center (top-left health bar, centered title text, bottom-right minimap)
- Layout containers: horizontal/vertical stack, grid, absolute positioning
- Basic widgets: text (with font support), image, button, slider, progress bar, panel/container, toggle, text input
- UI event system: on_click, on_hover, on_value_changed with data binding to ECS components
- Resolution-independent scaling: reference resolution (e.g., 1920x1080) with automatic scaling to actual resolution
- DPI awareness for sharp rendering on high-DPI displays
- UI animation: fade, slide, scale, rotate tweens with easing functions
- UI asset type: save/load UI layouts as assets

**What you'll learn:**
- Retained-mode UI architecture vs immediate-mode (egui)
- Layout algorithms (flexbox-style constraint solving)
- UI rendering pipeline (separate from 3D scene rendering)
- Resolution independence and DPI scaling
- UI animation and tween systems

**Why now:**
- egui is great for editor but wrong for game UI (immediate mode redraws everything, limited styling, looks like debug UI)
- HUD, health bars, menus, dialogue boxes, inventory screens all need a proper UI framework
- Players expect polished UI — egui can't deliver that for runtime game UI

---

### Task 56: Navigation & AI Foundation
**Status:** 📋 Planned
**Duration:** ~1.5-2 weeks
**Prerequisites:** Task 45

**What you'll build:**
- Navmesh generation from scene geometry (walkable surface extraction, obstacle carving)
- A* pathfinding on navmesh (find path from point A to point B avoiding obstacles)
- `NavAgent` ECS component: speed, radius, height, avoidance priority
- Steering behaviors: seek, flee, follow path, wander, obstacle avoidance, separation (crowd behavior)
- Path smoothing: string-pulling algorithm (funnel algorithm) for natural-looking paths
- Debug visualization using debug draw: navmesh wireframe overlay, path lines, agent radius circles
- Basic behavior tree runtime: node types (selector, sequence, parallel, decorator, leaf/action), blackboard for shared data
- Inspector-based behavior tree authoring (tree structure editable in inspector)
- Navmesh rebaking in editor (rebake when geometry changes)

**What you'll learn:**
- Navigation mesh theory and generation algorithms
- A* pathfinding on polygon meshes
- Steering behavior composition
- Behavior tree design patterns
- Agent-based AI architecture

**Why now:**
- Every game with NPCs needs pathfinding
- Navmesh + behavior trees are the industry standard AI foundation
- Debug draw (Task 29) makes navmesh development dramatically easier
- Foundation for node-based behavior tree editor (Task 57)

**Technologies:**
- Consider `recast-rs` or custom navmesh generation
- Custom A* implementation on navmesh polygon graph

---

### Task 57: AI Behavior Tree Editor (Node-Based)
**Status:** 📋 Planned
**Duration:** ~1.5 weeks
**Prerequisites:** Task 40, Task 56

Sixth consumer of the Node Graph Framework.

**What you'll build:**
- `#[derive(BehaviorNode)]` proc macro
- Behavior tree graph editor using Task 40's framework (tree structure rendered as a graph — parent nodes at top, children below)
- Runtime debug visualization: active branch highlighted, each node shows its current state (success/failure/running) in real time during play mode

*Built-in Node Library:*
- Composite nodes: selector (try children until one succeeds), sequence (run children in order, stop on failure), parallel (run all children simultaneously)
- Decorator nodes: repeat (loop N times or forever), invert (flip success/failure), cooldown (prevent re-entry for N seconds), blackboard condition (check a value before proceeding)
- Leaf/action nodes: move to position, attack target, play animation, wait duration, patrol waypoints, look at target, send event
- Blackboard variable nodes: get/set/compare blackboard values (shared data between nodes)

- Inspector-based behavior tree editing remains for simple AI (single patrol, basic chase)
- Users create custom behavior nodes for game-specific AI behaviors

**What you'll learn:**
- Behavior tree traversal and evaluation algorithms
- Tree-structured graph rendering (different from DAG — strict hierarchy)
- Real-time debug overlay for tree execution state
- Blackboard pattern for AI data sharing
- How to map tree semantics to the graph framework

```rust
#[derive(BehaviorNode)]
#[node(id = "patrol_route", name = "Patrol Route", category = "Movement")]
pub struct PatrolRoute {
    #[input(label = "Waypoints")]
    waypoints: Vec<Vec3>,
    #[input(label = "Wait Time", default = 2.0)]
    wait_time: f32,
    #[output(label = "Status")]
    status: BehaviorStatus,
}
```

---

## ✅ Validation Milestone: Single-Player Vertical Slice
**Status:** 🔀 Replaced by the **Networked Co-op Slice** milestone at the end of the Multiplayer Foundation phase (see Phase M).
**Duration:** ~2-3 days (focused effort)

**What you'll build:**
A small but fully playable scene that exercises all major systems together. Not a shipping game — a functional proof that everything composes correctly.

**Must exercise:**
- Scene loading (Task 43)
- Physics (existing)
- Skeletal animation with animation graph (Tasks 30, 41)
- Audio with spatial sound (Task 31)
- Particles (Task 38)
- Visual scripting — at least one scripted interaction (Task 45)
- Materials (existing PBR + Task 39 instances)
- In-game UI — health bar, simple menu (Task 55)
- Save/load — save game state, reload, verify consistency (Task 42)
- Navigation with behavior trees — one NPC that patrols and reacts (Tasks 56, 57)

**Why now:**
- Proves the single-player runtime is solid before adding multiplayer complexity
- Exposes integration gaps between systems that unit tests don't catch
- Validates the engine can ship a real experience, not just individual tech demos
- Fix every gap this vertical slice exposes before moving to threading and networking

---

## Phase 15: Threading, Networking & Validation (Tasks 58-60)

### Task 58: Enable Parallel ECS Execution
**Status:** 📋 Deferred — not needed for the Multiplayer Foundation phase (server holds the load; client stays sequential until profiling says otherwise). No longer a prerequisite for networking.
**Duration:** ~1.5 weeks
**Prerequisites:** Task 32 (access declarations in place since this task)

**What you'll build:**
- Rayon-backed parallel scheduler (replaces the sequential scheduler from Task 32)
- Automatic parallel scheduling of non-conflicting systems (systems that don't write to the same components run simultaneously)
- Parallel query support for data-heavy work (`par_bridge()` for transform propagation, animation evaluation, particle updates)
- Worker thread pool with work stealing
- Performance comparison: profile parallel vs sequential, identify which systems benefit most
- Thread pool size tuning (default: logical core count - 2, one for game thread, one for render thread)
- **ECS debugger panel** (inspired by Epic's Mass Debugger, Unreal Fest Chicago 2026): system dependency/conflict graph rendered from the Task 32 access declarations (`Schedule::validate()` already computes it — today it only reaches `print_access_report()`), parallel-group visualization once the rayon scheduler lands, and a query-driven entity browser (run an ad-hoc component query, inspect/edit matching entities' data live — complements the single-entity inspector). Traditional data breakpoints are useless in archetype ECS (storage relocates on composition change), so this panel is the debugging story.

**What you'll learn:**
- Rayon integration with custom ECS scheduling
- Work stealing thread pool architecture
- Parallel iteration patterns for ECS queries
- Performance tuning for multi-threaded systems
- How to measure parallel speedup and identify Amdahl's law bottlenecks

**Why now:**
- Every system has declared its access patterns since Task 32 — this is a scheduler backend swap, not a codebase rewrite
- By now there are enough systems (animation, audio, particles, physics, scripting, AI, navigation) that parallelism has real payoff
- Render thread (Task 36) already handles GPU submission — this task parallelizes the CPU game logic side

**Key concepts:**
- Access declarations → build dependency graph → topological sort → identify parallelizable groups
- `rayon::scope` for parallel system execution within a stage
- `par_bridge()` for data-parallel queries (update 1000 particle emitters across 8 cores)
- Barrier between stages (all systems in a stage complete before next stage begins)

**Technologies:**
- `rayon` for data parallelism and thread pool
- Existing access declaration system (Task 32) for conflict detection

---

### Task 58.5: Multi-Window Viewport & Editor Tabs v2
**Status:** 📋 Planned (after Task 58 — the extra swapchain + per-window record work wants the threading dust settled first)
**Duration:** ~1-1.5 weeks
**Prerequisites:** Task 58; M10 + the 2026-07-26 "Editor tabs" mockup pass (hide-tabs shipped there)

**What you'll build:**
- **Viewport floats to its own OS window**: render the scene into a second
  surface/swapchain so a viewport tab can leave the main window like any
  panel (today the scene presents only into the main swapchain and the
  drop is forced back — `app.rs` "Viewports render only in the main
  window"). This is the render-thread work item; everything below is UI.
- Remaining "Editor tabs" mockup items:
  - Overflow "▾ N" chip when tabs exceed the strip, with MRU-ordered
    popup menu (dirty dots survive into the menu, ⌘-number hints)
  - ⌘1–9 direct tab switching + Ctrl+Tab MRU cycling
  - F11 focus-viewport (collapse every strip + docked panels; floating
    "Esc to exit" chip, fades after 2s)
  - Tab right-click menu: Hide Tabs / Close / Close Left-Right-Others /
    Unpin / Split Right / Move to New Window
  - Per-tab type icons (camera for viewport, type-tinted editor icons)
  - Pinned viewport tab (first, no ×; reopen via Window ▸ Viewport / ⌘1)
  - Single-tab dock slots draw a plain 24px header instead of a strip

**Why after threading:**
- A second presentation surface touches the render-thread contract
  (`FramePacket`, serial `TargetRenderer` command buffers); do it once the
  parallel scheduler has stabilized what crosses threads, not while it's
  in flux.
- The tab UX items are pure crusty-gui/editor work and can be cherry-picked
  earlier if the editor needs them sooner.

---

### Task 59: Networking Foundation (SpacetimeDB)
**Status:** ⛔ Superseded by the Multiplayer Foundation phase (Tasks M0–M8), which expands this single task into a full arc with a spike gate. Kept below for reference.
**Duration:** ~2-3 weeks
**Prerequisites:** Task 34 (crate boundaries), ~~Task 58~~

**What you'll build:**
- `game_server_stdb` crate: SpacetimeDB module using `game_shared` types from Task 34
- SpacetimeDB reducers for authoritative game actions (move, attack, interact, spawn, despawn)
- Generated Rust client bindings from server module schema
- Client-side prediction using shared validation rules from `game_shared` (movement feels instant, server corrects if wrong)
- Entity replication using EntityGuid: server spawns entity → client receives and spawns matching entity with same GUID
- Transform synchronization with interpolation and snapshot buffers (smooth remote entity movement)
- Connection management: connect, disconnect, reconnect, timeout handling
- Simple test scene: two players moving in a shared world, seeing each other in real time
- Node realm metadata validation: `server_safe` and `shared` nodes validated at graph compile time — `client`-only nodes flagged if used in authoritative graphs

**What you'll learn:**
- Client-server game architecture (authoritative server model)
- SpacetimeDB module development (Rust WASM modules for server logic)
- Entity replication patterns (what to replicate, what to predict, what to interpolate)
- Client-side prediction and server reconciliation (rollback on misprediction)
- Network serialization and bandwidth management
- Latency compensation techniques

**Why now:**
- `game_shared` crate (Task 34) provides shared types and validation rules — server and client speak the same language
- EntityGuid (Task 24) provides persistent entity identity for replication
- Visual scripting issues commands through the command interface (Task 34) — networking is transparent to graphs
- Hardest feature in the roadmap — benefits from maximum engine stability and maturity
- Placed last among features so all systems are proven before adding network complexity

**Architecture:**
```
engine/              → Rendering, ECS, physics, audio, editor
game_shared/         → Types, commands, rules, validation (used by both)
game_client/         → Client gameplay, prediction, presentation
game_server_stdb/    → SpacetimeDB module: reducers, tables, subscriptions
```

**Technologies:**
- SpacetimeDB Rust SDK (server module + client bindings)
- `game_shared` crate for cross-boundary types
- Existing EntityGuid system for entity identity

---

### Task 60: Production Hardening & Ship a Game
**Status:** 📋 Planned
**Duration:** ~2-3 weeks
**Prerequisites:** All previous tasks

The capstone task. This is where the engine proves it works — not as a collection of tech demos, but as a tool that ships a product.

**What you'll build:**
- Architecture review and API cleanup: identify and fix inconsistencies, remove deprecated code paths, unify naming conventions
- Crash handling with diagnostic logging: panic handler that captures stack trace, last N log lines, engine state, writes crash report file
- Error recovery and graceful degradation: asset loading failures show fallback, shader compilation errors keep old pipeline, network disconnects don't crash
- CI pipeline hardening: automated build (editor + standalone), test suite, clippy, formatting check
- Documentation update: ARCHITECTURE.md, KNOWLEDGE.md, DECISIONS.md reflect final engine state
- **Ship a small but complete game** (5-15 minute experience) that exercises:
  - Scene loading and transitions (Task 43)
  - Physics interactions (existing)
  - Skeletal animation with animation graph (Tasks 30, 41)
  - Audio with spatial sound and audio graph (Tasks 31, 53)
  - Particles with VFX graph (Tasks 38, 51)
  - Visual scripting for gameplay logic (Task 45)
  - Materials from visual material editor (Task 50)
  - In-game UI for HUD and menus (Task 55)
  - Save/load for player progress (Task 42)
  - Navigation and AI with behavior trees (Tasks 56, 57)
  - At least one scripted gameplay mechanic
- Fix every gap the game exposes
- Package as standalone executable using build pipeline (Task 25) with cooked assets (Task 44)

**Why now:**
- This is the only real proof the engine works
- Building a game exposes integration issues that individual tasks and tech demos never reveal
- Forces you to use your own tools the way a game developer would
- Validates the complete workflow: author in editor → test in play mode → build standalone → ship

---

## Optional Tasks (Unnumbered, Added When Needed)

### Scripting Layer (Lua via mlua)
Added only if iteration pain justifies it before Task 45 (Visual Scripting) is complete. Thin command layer for:
- Triggers and quest logic
- Dialogue scripting
- Console commands
- Cutscene sequencing
- Modding API for players

Non-authoritative by design — scripts issue commands, Rust validates and executes. Choose Lua over Rhai for ecosystem maturity, async support, and industry precedent (30+ years of game scripting).

### UI Graph (Node-Based)
Seventh potential consumer of the Node Graph Framework. Node-based UI layout and animation authoring. Only if in-game UI complexity justifies visual authoring over code-based UI construction.

---

## Summary Table

| # | Task | Phase | Type | Node Graph |
|---|------|-------|------|------------|
| 26 | ✅ Performance Profiling & Baselines | Measure/Fix/Prove | Infrastructure | |
| 27 | ✅ Performance Optimization & Frustum Culling | Measure/Fix/Prove | Infrastructure | |
| 28 | ✅ Automated Testing & CI Pipeline | Measure/Fix/Prove | Infrastructure | |
| 29 | ✅ Debug Draw System | Measure/Fix/Prove | Tool | |
| 30 | ✅ Skeletal Animation & glTF Playback | Core Gameplay | Feature | |
| 31 | ✅ Audio System (Kira) | Core Gameplay | Feature | |
| -- | ✅ *Refactor #4: System Independence Audit* | -- | -- | |
| 32 | ✅ System Access Declarations & Scheduler | Architecture | Infrastructure | |
| 33 | ✅ Input Action System & Gamepad | Architecture | Feature | |
| 34 | ✅ Game Logic Architecture & Crate Boundaries | Architecture | Infrastructure | |
| 35 | ✅ Render Graph / Frame Graph | Rendering Architecture | Infrastructure | |
| 36 | ✅ Render Thread Separation | Rendering Architecture | Performance | |
| 37 | ✅ PBR Deferred Lighting, Shadows & Post-Processing | Visual Quality | Feature | |
| 38 | ✅ Particles & VFX (Inspector-Based) | Visual Quality | Feature | |
| 39 | ⚠️ Shader Hot-Reload & Material Instancing (instance wiring → RC#5) | Visual Quality | Feature | |
| -- | ⛔ *Task 39.4: Editor UI Theme (superseded by crusty-gui migration → M1)* | -- | -- | |
| -- | 🔜 *Refactor #5: Production Readiness Review (slim)* | -- | -- | |
| **M0** | ✅ **SpacetimeDB Scale Spike — GO decision recorded** | **Multiplayer Foundation** | **Spike** | |
| M1 | Editor UX & Design System v1 (time-boxed) | Multiplayer Foundation | UX | |
| **M2** | ✅ **Collision Pipeline v1 (cooked chunks)** | **Multiplayer Foundation** | **Infrastructure** | |
| **M3** | ✅ **Greybox World v1** | **Multiplayer Foundation** | **Content** | |
| **M4** | ✅ **Zone & Chunk Lifecycle** | **Multiplayer Foundation** | **Feature** | |
| M5 | Net-A: Connection, Identity & Replication | Multiplayer Foundation | Feature | |
| M6 | Net-B: Server-Authoritative Movement | Multiplayer Foundation | Feature | |
| M7 | Net-C: Authoritative Combat & Thin HUD | Multiplayer Foundation | Feature | |
| M8 | Net-D: Interest Management & Load | Multiplayer Foundation | Feature | |
| M9 | Multiplayer Packaging (client & server build targets) | Multiplayer Foundation | Infrastructure | |
| M9.5 | Packaged Co-op Verification | Multiplayer Foundation | Testing | |
| M9.6 | Editor Net Play Modes (Play as Client / Listen Server) | Multiplayer Foundation | Feature | |
| -- | 🎯 *Milestone: Networked Co-op Slice (on packaged builds)* | -- | -- | |
| -- | 📋 *Refactor #6: Rendering API Cleanup (slim — pass trait, graph dispatch, dead facade)* | -- | -- | |
| 39.8 | Plugin System & Module Registry (physics/Steam/GAS as first plugins) | Game Architecture | Infrastructure | ✅ Complete |
| **40** | **Node Graph Framework & Custom Node SDK** | **Node Graph Foundation** | **Infrastructure** | **Framework** |
| **41** | **Animation Graph** | Game Architecture | Feature | **1st consumer** |
| 42 | 🔀 Save/Load & Runtime Persistence (networked part → M5) | Game Architecture | Feature | |
| 43 | 🔀 Scene Management & Transitions (zone lifecycle → M4) | Game Architecture | Feature | |
| 44 | Asset Cooking & Level Streaming | Game Architecture | Infrastructure | |
| **45** | **Visual Scripting** | Game Architecture | Feature | **2nd consumer** |
| 46 | Terrain System | World Building | Feature | |
| 47 | Environment, Sky & Atmosphere | World Building | Feature | |
| 48 | Cascaded Shadow Maps & Shadow Quality | World Building | Rendering | |
| 49 | Advanced Lighting, IBL & GI | World Building | Rendering | |
| **50** | **Visual Material Editor** | Visual Authoring | Feature | **3rd consumer** |
| **51** | **VFX Graph** | Visual Authoring | Feature | **4th consumer** |
| 52 | Prefab Variants & Nested Prefabs | Visual Authoring | Feature | |
| **53** | **Audio Graph** | Visual Authoring | Feature | **5th consumer** |
| 54 | Editor Workflow Polish | Visual Authoring | UX | |
| 55 | 🔀 In-Game UI Framework (thin HUD → M7) | Production | Feature | |
| 56 | Navigation & AI Foundation | Production | Feature | |
| **57** | **AI Behavior Tree Editor** | Production | Feature | **6th consumer** |
| -- | 🔀 *Validation Milestone: Single-Player Vertical Slice (→ Networked Co-op Slice)* | -- | -- | |
| 58 | 📋 Enable Parallel ECS Execution (deferred, no longer gates networking) | Threading | Performance | |
| 58.5 | 📋 Multi-Window Viewport & Editor Tabs v2 | Editor | Feature | |
| 59 | ⛔ Networking Foundation (superseded by M0–M8) | Multiplayer | Feature | |
| 60 | Production Hardening & Ship a Game | Validation | Capstone | |

---

## Architectural Decisions

1. **Threading contract early, parallelism late** — Task 32 establishes access declarations, Task 58 enables parallel execution
2. **Crate boundaries before features** — Task 34 creates game_shared/game_client, everything builds against it
3. **Input actions before consumers** — Task 33 before animation graphs, visual scripting, AI
4. **Node graph framework built once, used six times** — Task 40 serves Tasks 41, 45, 50, 51, 53, 57
5. **Custom Node SDK via proc macros** — built on a manual `NodeRegistry` API, with `inventory` as one auto-registration backend
6. **Stable IDs for node types and pins** — display names can change freely, graph serialization uses stable string slugs
7. **Node metadata: pure/impure, realm, deterministic** — validated at graph compile time, enforced before networking exists
8. **Inspector authoring always coexists with graph authoring** — simple cases stay simple, graphs are for complex behaviors
9. **Graph versioning and subgraphs from day one** — schema migration for asset evolution, reusable graph functions to prevent spaghetti
10. **Scripting is optional, not scheduled** — Lua via mlua added only when iteration pain justifies it
11. **SpacetimeDB integration is Rust-native** — Tasks M0–M8 (superseding Task 59) use game_shared from Task 34 behind a backend-neutral command interface (ADR-014); graphs issue commands transparently
12. **Spike before networking** — Net-0 (M0) go/no-go gate validates the backend before the engine arc; the networked co-op slice replaces the single-player vertical slice as the proof point (ADR-016)
13. **High-risk tasks flagged** — Tasks 44 and 49 may need splitting during execution

---

## Design Principles

### 1. **Measure Before Optimizing**
- Profile before every optimization decision
- Establish baselines, compare after changes
- Don't guess where the bottleneck is

### 2. **Establish Contracts Before Systems**
- Threading access declarations exist before systems that need them
- Crate boundaries exist before gameplay code
- Input action maps exist before systems that consume input
- Node graph framework exists before domain-specific graphs

### 3. **Inspector-First, Graphs When Needed**
- Every system works with inspector-based property editing
- Node graphs add power for complex behaviors
- Simple cases should never require opening a graph editor

### 4. **One Framework, Many Backends**
- Shared graph editor UI (canvas, nodes, pins, connections)
- Domain-specific compilers (GLSL, compute shader, graph IR, DSP chain, behavior tree)
- Domain-specific node libraries (each task builds its own)
- Shared proc macro infrastructure for custom node creation

### 5. **Rust Owns Authority**
- Engine core, networking, and shared game rules are Rust
- Visual scripting and optional text scripting issue commands above that
- SpacetimeDB integration is Rust-native (game_shared crate)
- Scripts never bypass the command/event interface

### 6. **Ship Something Real**
- Validation milestone: prove single-player works before networking
- Final task: build an actual game, fix what breaks
- The engine is done when it ships a product, not when all tasks are checked off

---

## Notes

- **Flexibility**: Order can be adjusted based on needs — particularly Task 50 (Material Editor) can be pulled earlier if materials are a bottleneck
- **High-risk tasks**: Tasks 44 (Asset Cooking & Level Streaming) and 49 (Advanced Lighting, IBL & GI) are flagged for potential splitting
- **Dependencies**: Later tasks build on earlier ones — the summary table shows the dependency chain
- **Node graph consumers**: Each domain task (41, 45, 50, 51, 53, 57) stress-tests and hardens the framework progressively
- **Optional tasks**: Lua scripting and UI graph are genuinely optional — added only when justified by real needs

---

**Last Updated:** 2026-07-15
**Current Progress:** Tasks 26–39 complete; crusty-gui migration complete (egui removed); RC#5 material-instance wiring + asset-extension migration done (commit 1b919e9). Next: remaining RC#5 review items as needed, then Task M0 (SpacetimeDB scale spike) opening the Multiplayer Foundation phase (M0–M8) toward the Networked Co-op Slice milestone.
