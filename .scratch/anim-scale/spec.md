# Task 41.5 — Animation at Scale + IK (working spec)

The binding spec is `docs/roadmap/VULKANO-41.5-ANIMATION-SCALE.md` (Claude +
Codex audited 2026-08-30). Read it in full before any ticket. This file only
adds process rules and records rulings made during implementation.

Branch: `task-41.5-animation-scale` (off main @ 57c3b11).
Tickets: `issues/NN-*.md`, one per work package P0–P9. One reviewable commit
per ticket. Serial execution (single-build machine).

## Process rules (binding)

- One cargo build at a time, `-j 2`. Never run two builds concurrently.
- Never launch the editor or any window that steals focus; never send
  synthetic OS input. Benchmarks must be runnable by the user via a single
  printed command, auto-exit, and write results to a file.
- Do not touch the user's stray working-tree files:
  `content/graphs/character.animgraph`, `editor_layout_crusty.ron`,
  deleted `docs/mockup/DESIGN-nodegraph.md`, anything in `.scratch/` you
  didn't create.
- Repo is not rustfmt-clean (`graph_editor_crusty.rs`, `graph_anim_edge.rs`,
  `graph_editor.rs` especially). Never rustfmt whole files; hand-format.
- Commit messages: short imperative, no Co-Authored-By.
- `log::` is invisible in game_client (no logger) — bench output uses
  `println!` / file writes.
- Gates per ticket: `cargo check -j 2` for both `-p game_client --features editor`
  and `-p game_client`, plus `cargo test -j 2 -p engine` for touched areas.
- Crusty-gui API changes (if any) go in `../crusty-gui` first; both repos
  must build before commit.

## Rulings made during implementation

(append here as they happen)

- **R1 (P1): palette ring is 4 regions gated by the 3-slot fence ring.**
  The plan said "×3 fence-matched regions", but the render thread reclaims a
  fence slot lazily — frame N-3's fence is taken at the start of processing
  frame N's packet — so with 3 regions the main thread would wait on a
  reclaim that requires the very packet it is building (deadlock at frame 3).
  One region of slack matches the actual reclaim point (frame N gates on
  N-4, published while the renderer processes N-1). Region index =
  `frame_number % 4`, fence slot = `frame_number % 3` — both derived from
  the one packet counter; no second ring counter exists.
- **R2 (P1): `dirty` cannot gate the ring copy.** Regions rotate, so every
  visible skeleton's palette must be present in every frame's region — the
  per-frame memcpy is unconditional (~64 B/bone). `SkeletonInstance.dirty`
  stays untouched in P1 and becomes the pose-evaluation gate under P4's URO
  (which throttles evaluation, not the upload).
- **R3 (P1): VP UBO lives at set 0 binding 1 next to the palette SSBO**, one
  UBO per pass per ring slot, owned and rewritten by the renderer (camera VP
  from `packet.view_proj`, shadow VP from `light_data.light_vp`). Descriptor
  sets are cached per slot and rebuilt only on ring growth. Editor preview /
  thumbnail pipelines share the shader shape but build fresh one-off
  buffers + set per recorded preview (nothing reused → no ring discipline).
- **R4 (P1): editor-mode mesh VP source changed** from
  `renderer.camera_3d`'s matrices to `packet.view_proj` (the viewport
  camera) — now consistent with grid/debug lines; camera_3d never received
  the viewport near/far, so this fixes a latent mismatch.
- **Process note (P1):** `engine::rendering::render_thread::tests::
  test_render_thread_ready_handshake` fails on this machine at HEAD too
  (validation layer enforces VUID-VkAttachmentDescription2-finalLayout —
  PresentSrc without `khr_swapchain` on the extension-less test device).
  Pre-existing; not a P1 gate failure. 880/881 engine tests pass.
