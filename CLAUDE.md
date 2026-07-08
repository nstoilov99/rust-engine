# Rust Game Engine - AI Context

## Quick Reference

| Aspect | Value |
|--------|-------|
| **Coordinate System** | Z-up (X=Forward/Red, Y=Right/Green, Z=Up/Blue) |
| **Renderer** | Vulkano 0.35, Deferred Pipeline |
| **ECS** | Custom (wrapping hecs 0.10) |
| **GUI** | egui 0.33, migrating to in-house `crusty-gui` (Phase 16, feature `crusty`) |
| **Physics** | Rapier 3D 0.25 |
| **Serialization** | RON (Rusty Object Notation) |

## Critical: Coordinate System

```
Game World (Z-up)          Vulkan Render (Y-up)
      Z (Up/Blue)                Y (Up)
      |                          |
      |                          |
      +------ Y (Right/Green)    +------ X (Right)
     /                          /
    X (Forward/Red)           -Z (Forward)
```

**Conversion**: All game logic uses Z-up. The `render_adapter` module converts to Y-up at render time.

- Use `local_matrix_zup()` for hierarchy composition
- Use `model_matrix()` or `world_matrix_to_render()` for rendering
- Gizmo colors: X=Red, Y=Green, Z=Blue

## Before Making Changes

1. **Read relevant docs** in `docs/` folder for the subsystem you're modifying
2. **Check existing patterns** in similar files
3. **Maintain Z-up convention** - never mix coordinate systems
4. **Run `cargo check`** before considering work complete

## Key Documentation

| File | Contents |
|------|----------|
| `docs/ARCHITECTURE.md` | System architecture and module relationships |
| `docs/KNOWLEDGE.md` | Conventions, patterns, and gotchas |
| `docs/DECISIONS.md` | Architectural decision records |
| `docs/TUTORIAL-ROADMAP.md` | Development roadmap and progress |

## Code Rules

### Do
- Use `Result<T, E>` for error handling
- Use `profile_function!()` / `profile_scope!()` for performance-critical code
- Components are plain data structs (no behavior)
- Systems are stateless functions
- Use builder patterns for complex struct construction

### Don't
- Use `unwrap()` in production code (use `?` or handle errors)
- Use `Box<dyn Component>` in ECS (type-safe storage only)
- Mix coordinate systems (Z-up for logic, Y-up only for Vulkan)
- Add behavior to components (put logic in systems)

## Module Overview

```
src/engine/
├── core/        # Vulkan context, device, swapchain
├── rendering/   # 2D/3D pipelines, deferred renderer
├── ecs/         # Entity-Component-System (hecs wrapper)
├── editor/      # Editor panels, viewport, gizmos
├── physics/     # Rapier 3D integration
├── assets/      # Asset loading and management
├── gui/         # egui-Vulkano integration
├── adapters/    # Coordinate conversion (Z-up ↔ Y-up)
└── camera/      # Camera systems
```

## Features and run commands

```bash
cargo run -p game_client --features editor          # editor (egui)
cargo run -p game_client --features editor,crusty   # editor + crusty-gui overlay (Phase 16)
cargo run -p game_client                            # standalone game
```

Feature `crusty` implies `editor` and pulls `crusty-gui` as a path dep from
`../crusty-gui`.

## Threading model (rendering)

Main thread does game logic, ECS and UI layout (CPU-only); the render thread
records command buffers and presents. The only data crossing is `FramePacket`
(bounded(2) crossbeam channel) — all fields owned, no shared mutable state,
except the crusty `SharedTextRenderer` mutex (layout shapes text on main,
renderer flushes glyph uploads on record).

## GUI migration (Phase 16)

The editor UI migrates from egui to `../crusty-gui` panel by panel (roadmap in
`../crusty-gui/ROADMAP.md`). Seam: `engine/src/engine/gui/crusty.rs`
(`CrustyGui` main thread / `CrustyRenderer` render thread). egui stays as
fallback until all panels are ported. Don't fork panel logic between the two
UIs — both call the same state structs and systems.

Note: no logger is installed in `game_client` — `log::` output is invisible;
use `println!` for temporary diagnostics (and remove before commit).

## Current Development Focus

- Phase 16: egui → crusty-gui editor migration (in progress, Console panel next)
- Task 25: Build Pipeline and Export (Planned)

## Patched Dependencies

These crates are forked/patched in `crates/`:
- `emath` - Fix for DragValue crash (egui issue #7747)
- `transform-gizmo` - Modified for Z-up coordinate system

## PR Guidelines
- Do not include HTML comments (e.g. <!-- generated-by-cyrus -->)
- Do not include the "Tip" block about @mentioning cyrusagent
- Do not append a "Linear issue:" link at the bottom