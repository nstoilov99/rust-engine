# Rust Game Engine - AI Context

## Quick Reference

| Aspect | Value |
|--------|-------|
| **Coordinate System** | Z-up (X=Forward/Red, Y=Right/Green, Z=Up/Blue) |
| **Renderer** | Vulkano 0.35, Deferred Pipeline |
| **ECS** | Custom (wrapping hecs 0.10) |
| **GUI** | In-house `crusty-gui` (path dep `../crusty-gui`) — egui fully removed |
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
├── gui/         # crusty-gui integration (main-thread layout / render-thread seam)
├── adapters/    # Coordinate conversion (Z-up ↔ Y-up)
└── camera/      # Camera systems
```

## Features and run commands

```bash
cargo run -p game_client --features editor   # editor (crusty-gui)
cargo run -p game_client                     # standalone game
```

`editor` pulls `crusty-gui` as a path dep from `../crusty-gui`. `crusty` is a
deprecated alias for `editor` (kept for muscle memory).

## Threading model (rendering)

Main thread does game logic, ECS and UI layout (CPU-only); the render thread
records command buffers and presents. The only data crossing is `FramePacket`
(bounded(2) crossbeam channel) — all fields owned, no shared mutable state,
except the crusty `SharedTextRenderer` mutex (layout shapes text on main,
renderer flushes glyph uploads on record).

## GUI (crusty-gui)

The editor UI is `../crusty-gui` (Phase 16 migration complete — egui is fully
removed). Seam: `engine/src/engine/gui/crusty.rs` (`CrustyGui` main thread /
`CrustyRenderer` render thread, `SharedTextRenderer` mutex between them; paint
list crosses in `FramePacket`). Panels are thin drawing layers — logic stays in
state structs and systems. Dock layout persists to `editor_layout_crusty.ron`.

Note: no logger is installed in `game_client` — `log::` output is invisible;
use `println!` for temporary diagnostics (and remove before commit).

## Current Development Focus

- Phase 16 (egui → crusty-gui migration): **complete** — egui removed
- Task 25 (Build Pipeline and Export, Windows standalone): **complete**
- M0 SpacetimeDB scale spike: **complete — GO** (see `docs/roadmap/VULKANO-M0-SPACETIMEDB-SPIKE.md`)
- M2 Collision Pipeline v1 (cooked chunks): **complete** (see `docs/roadmap/VULKANO-M2-COLLISION-PIPELINE.md`)
- M3 Greybox World v1 (`tools/greybox_gen`, deterministic; generated content checked in): **complete** (see `docs/roadmap/VULKANO-M3-GREYBOX-WORLD.md`)
- M4 Zone & Chunk Lifecycle (WorldStreamer: budgeted cell/chunk streaming, GUID-stable reload): **complete** (see `docs/roadmap/VULKANO-M4-ZONE-CHUNK-LIFECYCLE.md`)
- M5 Net-A (SpacetimeDB connection, identity, replication, zone-scoped subscriptions): **complete** (see `docs/roadmap/VULKANO-M5-NET-A-CONNECTION-IDENTITY.md`)
- M6 Net-B (server-authoritative movement: shared controller, client prediction, WASM parity suite, combat groundwork): **complete** (see `docs/roadmap/VULKANO-M6-NET-B-MOVEMENT.md`)
- M7 Net-C (combat vertical slice: ability roster, cast pipeline, death/respawn, projectiles, standalone crusty HUD behind `hud` feature): **complete** (see `docs/roadmap/VULKANO-M7-NET-C-COMBAT.md`)
- M8 Net-D (interest management: cell anchor + hysteresis, near 3×3 full / far 7×7 coarse tiers, write hygiene, `tools/net_bots` load harness): **complete** (see `docs/roadmap/VULKANO-M8-NET-D-INTEREST.md`; load results in `docs/roadmap/M8-LOAD-REPORT.md` — sim CPU ceiling ~150 active movers/module is the next scaling constraint)
- M9 Multiplayer Packaging (net_config.ron, export/build-dialog targets standalone/mp-client/mp-server, build-id stamp + protocol v5, `scripts/host_local.ps1` play-test loop): **complete** (see `docs/roadmap/VULKANO-M9-MP-PACKAGING.md`)
- M9.5 Packaged Co-op Verification (smoke script, load-sanity rerun, Maincloud publish + WAN two-client check, soak monitor + runbook): **complete** (see `docs/roadmap/VULKANO-M9.5-COOP-VERIFICATION.md`) — the two-machine hour soak was skipped by decision (2026-07)
- M9.6 Editor Net Play (server-announced `Config.world_scene` + protocol v6, deferred standalone world load with offline fallback, editor PlaySettings dropdown: Play As Client / Listen Server launcher / Number of Players): **complete** (see `docs/roadmap/VULKANO-M9.6-EDITOR-NET-PLAY.md`)
- M10 Editor UX & Design System v1 (semantic theme tokens + 4 presets, widget state ladder + focus, settings windows, panel restyle, Edit menu with entity clipboard Cut/Copy/Paste + GUID remap, verb/object undo labels): **complete** (see `docs/roadmap/VULKANO-M10-EDITOR-UX.md`)
- 🎯 Networked Co-op Slice milestone: **achieved** (M0–M10; hour soak waived)
- Task 40 (Node Graph Framework & Custom Node SDK): **complete** — `engine/src/engine/node_graph/` (docs/registry/validation/migration/resolver), `crates/node_graph_macros`, graph editor on crusty-gui's `Canvas` primitive (see `docs/roadmap/VULKANO-40-NODE-GRAPH-FRAMEWORK.md`)
- Next: Task 39.8 (Plugin System & Module Registry — implements against Task 40's registry contract), per `docs/roadmap/ROADMAP.md`

## Patched Dependencies

These crates are forked/patched in `crates/`:
- `transform-gizmo` - Modified for Z-up coordinate system
- `emath` - only remains as a dependency of the `transform-gizmo` fork (egui itself is gone)

## Code style
- Always strive for concise, simple solutions.
- If a problem can be solved in a simpler way, propose it.

## General preferences
- If asked to do too much work at once, stop and state that clearly.
- If computer use is helpful for completing or verifying work (screenshots,
  visual comparison, driving a running app), shell out to gpt-5.6-Sol via Codex
  (capture with `../crusty-gui/.claude/skills/port-panel/screenshot.ps1`,
  compare with `codex exec -i`). Codex is configured via `.codex/config.toml`
  (model gpt-5.6-sol, workspace-write sandbox).

## Picking the right model for workflows and subagents
Rankings, higher = better. Cost reflects what the user actually pays (OpenAI is
near-free due to a deal), not list price. Intelligence is how hard a problem
you can hand the model unsupervised. Taste covers UI/UX, code quality, API
design and copy.

| model       | cost |  intelligence   |  taste |
|-------------|------|-----------------|--------|
| gpt-5.6-Sol | 9    |  8              | 6      |
| sonnet-5    | 5    |  5              | 7      |
| opus-4.8    | 4    |  7              | 8      |
| fable-5     | 2    |  9              | 9      |

How to apply:
- These are defaults, not limits. You have standing permission to override them: if a cheaper model's output doesn't meet the bar, rerun or redo the work with a smarter model without asking. Judge the output, not the price tag. Escalating costs less than shipping mediocre work.
- Cost is a tie-breaker only; when axes conflict for anything that ships, intelligence > taste > cost.
- Bulk/mechanical work (clear-spec implementation, data analysis, migrations): gpt-5.6-Sol it's effectively free.
- Reviews of plans/implementation: fable-5 or opus-4.8, optionally gpt-5.6-Sol as an extra independent perspective.
- Never use Haiku.

## Visual Changes and Computer Use
- When implementing/fixing visual change shell out gpt-5.6-Sol to take a screenshot of the change and have fable-5 and gpt-5.6-Sol discuss it.
- Have fable-5 review the result for UI/UX quality, visual hierarchy, consistency, and overall taste.
- Reconcile the findings and fix material issues before considering the task complete.

## Following Visual References
- When following a visual reference, aim for approximately 90–95% visual similarity unless pixel-perfect reproduction is explicitly requested.
- Prioritize:
   - Correct functionality and usability
   - Consistency with the product's existing visual language
   - Good UX and accessibility
   - The important composition, hierarchy, spacing, typography, and visual cues from the reference
   - Minor decorative similarity
- Do not reproduce defects or poor UX from a reference merely for visual fidelity.
- When intentionally deviating from the reference, preserve its core intent and briefly explain any material UX or consistency improvements.

## Conventions
- Commit messages: short, imperative. Never add Co-Authored-By.
- `TargetRenderer` sync contract: its command buffers must execute serially
  (glyph atlas and blur targets are shared across frames). `CrustyRenderer`
  upholds this on the render thread.
- Keep panel code readable; extract helpers when nesting hurts.
- Changes that need new crusty-gui `backend`/`shell` API are made in
  `../crusty-gui` first; verify both repos build before commit.

## PR Guidelines
- Do not include HTML comments (e.g. <!-- generated-by-cyrus -->)
- Do not include the "Tip" block about @mentioning cyrusagent
- Do not append a "Linear issue:" link at the bottom