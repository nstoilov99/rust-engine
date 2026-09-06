# P0 — Stress scene + baseline capture

**Status:** done
**Plan:** §4 P0.

CLI flags on `game_client` (standalone, no editor feature needed):
- `--stress-anim N` — after world load, spawn N characters in a grid: each =
  skinned character mesh + `GraphRunner`/`AnimGraphRuntime` on
  `content/graphs/character.animgraph` (same components the scene's existing
  character entity has — copy its component recipe, don't invent one).
- `--bench-secs S` — run S seconds after first frame, then write
  `.scratch/anim-scale/baseline-N.txt` and exit(0). Never requires input.

Metrics captured per frame, reported as avg / p95 over the bench window:
- frame ms (main-thread tick), anim system ms (AnimGraphSystem run),
  palette-upload count + ms, skinned draw count.
- Use existing `profile_*` infra if it can be read programmatically; else a
  small `BenchStats` struct in game_client updated from explicit `Instant`
  timings around the two spots. Keep it behind the flags — zero cost
  otherwise.

## Checklist
- [x] flags parse; standalone build runs without them exactly as before
- [x] spawned characters animate (share the compiled plan Arc) — recipe cloned
      from the scene's animated character (Transform + MeshRenderer +
      `AnimGraphRunner`; `AnimGraphSystem` arms lazily, inserts
      `SkeletonInstance` from mesh bones, plans shared via `AnimGraphPlanCache`)
- [x] bench writes file and exits cleanly (event-loop exit → code 0)
- [x] `cargo check` editor + standalone; commit
- [x] print the exact command for the user to capture baselines at N=1/100/300

## Implementation notes (for P1+)

- `game_client/src/bench.rs`: render-loop hooks are relaxed atomics armed only
  by `BenchRun::new`; `TimedAnimGraph` wraps `AnimGraphSystem` only when
  `--bench-secs` is present. Standalone-only pieces are
  `#[cfg(not(feature = "editor"))]`.
- Palette-upload count/ms is measured around `skinning.create_palette_set`
  in `render_loop::prepare_mesh_data` — i.e. per-entity UBO + descriptor-set
  allocation, exactly the loop P1's ring buffer replaces. After P1 the
  timing hook must move to wherever the ring write happens or the metric
  reads ~0.
- Skinned draw count = per-submesh entries pushed (shadow list + camera
  list). Defeated.mesh has 2 submeshes, so expect ≈ N×2 shadow + N×2 camera
  when all are on screen.
- `--bench-secs` forces `SwapchainPresentModePreference::Immediate` so frame
  ms isn't vsync-flattened; the baseline file records build profile
  (debug/release) and measured window. First frame (plan compile) excluded.
- Metrics: frame ms = `GameLoop::delta_ms` (main-thread frame-to-frame);
  bench window starts after the first frame; avg + p95 per metric.
- Run from repo root (file path `.scratch/anim-scale/baseline-N.txt` and
  content dir are cwd-relative). Use `--release` for acceptance numbers
  (dev profile carries debug assertions; release is opt 3 + LTO).
