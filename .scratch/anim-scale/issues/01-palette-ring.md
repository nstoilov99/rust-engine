# P1 — Palette ring buffer (S-D1)

**Status:** done
**Plan:** §2 S-D1, §4 P1, risk §7.1. All rulings in the plan are binding:
one SSBO ring (fence-ring aligned, `minStorageBufferOffsetAlignment`),
flat `palette_base` indexing (no dynamic offsets, no push-constant append),
`view_projection` moves to a per-pass UBO, push constants = `model` +
`palette_base`, runtime-array `readonly buffer` palette decl, drop
`MAX_PALETTE_BONES`. Gbuffer + shadow + thumbnail + mesh-preview pipelines
migrate in this one commit. Honour `SkeletonInstance.dirty`; no descriptor
allocation in the per-entity loop.

## Checklist
- [x] SSBO ring in skinning backend; region reuse gated on reclaimed fence
- [x] shaders + reflected layouts migrated together (gbuffer, shadow, thumbnail, preview)
- [x] per-pass UBO for VP; push-constant block = model + palette_base
- [x] dirty respected; palette written once per skeleton per frame (see ruling R2 —
      the ring copy itself cannot be skipped; `dirty` gates evaluation from P4)
- [x] editor smoke paths compile: viewport, blend-space preview, anim preview, thumbnails
- [x] `cargo check` both + engine tests; commit

## How it works (implementation notes for P2/P7)

### Ring (`engine/src/engine/rendering/3d/skinning.rs`)
- One `Subbuffer<[[f32;16]]>` (STORAGE_BUFFER, PREFER_DEVICE +
  HOST_SEQUENTIAL_WRITE), split into **4** regions (`PALETTE_RING_REGIONS`);
  region capacity in mat4s is rounded so region byte offsets are multiples of
  `minStorageBufferOffsetAlignment`. Element 0 of every region is the
  identity → static meshes (and failed writes) use `palette_base = 0`.
- **Why 4 regions against the 3-slot fence ring** (ruling R1): the render
  thread reclaims fence slot `N % 3` *lazily* — at the start of processing
  frame N's packet it takes frame N-3's fence. With 3 regions the main thread
  would need that reclaim before it could build/send frame N → deadlock at
  frame 3. One region of slack matches the real reclaim point: frame N gates
  on frame N-4, published while the render thread processes frame N-1
  (already sent). All indices derive from `frame_number` (regions `% 4`,
  fence slots `% 3`) — no second counter, no drift.
- Main-thread API (used by `render_loop::prepare_mesh_data`):
  `begin_frame(frame_number)` (blocks until frame N-4 marked done; 2 s
  timeout + eprintln escape hatch) → `write_palette(&[Mat4]) -> u32`
  (region-relative base; one call per visible skeleton per frame; slice-write
  so vulkano's range tracking only locks this region) → `end_frame() ->
  SkinnedPaletteFrame { region, slot }` (goes into `FramePacket.palette`).
- **Handshake** `PaletteRingSync` (3 markers, one per fence slot, value =
  `seq+1` of the newest finished frame on that slot): the render thread calls
  `mark_done(slot, seq)` when it reclaims a fence (fence_slots entries now
  carry the frame_number); `PaletteSlotGuard` (render_thread.rs) covers every
  packet consumed *without* a stored fence (no swapchain, recreate, acquire
  fail, render error) — and does a `wait_idle` first in the rare
  flushed-but-no-fence case (present failure). `benchmark_runner` (single
  threaded, own fence ring) marks done inline after its fence reclaim, which
  now runs *before* `prepare_mesh_data`.
- **Growth rule**: if a frame needs more matrices than a region holds,
  allocate a new buffer on the spot (`max(needed, 2×current)` rounded to
  alignment), copy the current frame's already-written span, and reset the
  epoch — the first 4 frames on a new buffer skip the ring wait (fresh
  regions, never GPU-visible). Old buffers stay alive via the Arcs held by
  in-flight descriptor sets/command buffers; no fence wait on growth.

### Renderer side (`deferred_renderer.rs`)
- `PassSkinBind` per pass (geometry, shadow): 4 tiny VP UBOs (one per
  region slot, rewritten each frame — safe because the region's previous
  fence was reclaimed before `render()` runs) + 4 cached descriptor sets
  binding `{ binding 0: region slice, binding 1: vp ubo }`. Sets rebuild only
  when the region identity changes (growth). Geometry VP = `packet.view_proj`,
  shadow VP = `light_data.light_vp`.
- `render()` takes `Option<&SkinnedPaletteFrame>`; `None` binds a 1-mat
  identity region (tests/tools) with an internal rotating slot.
- One set-0 bind per pass; per-draw data is push constants
  `{ mat4 model; uint palette_base }` (68 B — `PushConstantData` must stay
  exactly this shape). `MeshRenderData` lost `bone_palette_set`.
- Editor-mode meshes now use `packet.view_proj` (viewport camera) instead of
  `renderer.camera_3d`'s projection — consistent with grid/debug lines
  (fixes a latent near/far mismatch; culling still uses camera_3d).

### Editor preview / thumbnail paths
- `thumbnail_vs.glsl` (shared by `ThumbnailRenderer` and
  `MeshPreviewRenderer` → mesh tabs, blend-space preview, Anim Preview) uses
  the same set-0 shape. These paths create a **fresh one-off** SSBO + VP UBO +
  set per recorded preview via `SkinningBackend::create_preview_set` — no
  reuse, so no ring discipline needed. `MeshPreviewRenderer::render` now
  takes `Option<&[Mat4]>` instead of a descriptor set.

### Bench hook (P0 comparison stays meaningful)
- Still in `prepare_mesh_data`, now wrapping `skinning.write_palette`:
  count = skeletons written into the ring this frame, ms = ring write time.
  `begin_frame`'s wait is *not* counted (sync overhead, not upload).
  Baseline-file footnote updated to say so.

### P7 handoff (instanced draws)
- Reuse this ring discipline for the instance-metadata SSBO: a region per
  `frame_number % PALETTE_RING_REGIONS`, gated on the *same*
  `PaletteRingSync` markers (frames are done atomically — one handshake
  serves any per-frame GPU-read buffer). Either add a second cursor/buffer to
  `SkinningBackend` or lift the region/epoch/growth logic into a generic ring
  allocator. Renderer side: `PassSkinBind` shows the per-slot set-cache
  pattern; instance metadata can join set 0 as another binding (rebuild rule:
  region identity change).
- `palette_base` is already per-draw data ready to move into the
  per-instance struct (`model`, `palette_base`) addressed by
  `gl_InstanceIndex`; push constants then drop to nothing.

### P2 handoff (two-phase FK)
- Keep exactly one `write_palette` per skeleton per frame; retained
  model-space matrices don't change the upload path. If P2 makes
  `compute_palette` cheaper for clean skeletons, the ring copy still happens
  every frame (R2).

## Deviations from the plan text
- Ring is ×4 regions, not ×3 (ruling R1 above; the ×3 intent — fence-matched
  reuse, bounded memory — is preserved; ×3 deadlocks against the renderer's
  lazy reclaim point).
- `SkeletonInstance.dirty` is read nowhere new (ruling R2): with rotating
  regions every visible skeleton must be copied into every frame's region, so
  dirty cannot gate the upload; it will gate pose *evaluation* under P4 URO.
