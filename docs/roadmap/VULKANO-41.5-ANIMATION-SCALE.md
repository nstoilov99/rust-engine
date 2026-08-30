# Task 41.5 — Animation at Scale + IK

**Status:** 📋 Planned (drafted 2026-08-30, Claude + Codex audit)
**Duration:** ~2 weeks
**Prerequisites:** Task 41 (animation graph — complete, `task-41-animation-graph` branch)
**Goal:** Hundreds of animated characters on a client at frame rate, and a
first IK layer (foot placement, look-at) on top of the Task 41 graph.

The graph runtime itself is sound for this goal: plans compile once and are
shared (`Arc<AnimGraphPlan>`), per-entity state is small, `evaluate_pose` is
allocation-free at steady state. Everything that blocks scale is *around*
the evaluation. IK is a pose post-process and slots in without changing the
graph's design.

---

## 1. What blocks scale today (verified)

| # | Problem | Where |
|---|---------|-------|
| 1 | New 16 KB UBO + descriptor set allocated **per skinned entity per frame**; `SkeletonInstance.dirty` never read | `render_loop.rs:131`, `skinning.rs:88` |
| 2 | `compute_palette()` allocates a `Vec<Mat4>` per entity per frame; model-space matrices discarded | `components.rs:107` |
| 3 | `AnimGraphSystem` fully serial; lifecycle scans collect temp `Vec`s each frame; no rayon in workspace | `runner.rs:515` |
| 4 | No update-rate throttling or anim culling; camera pass frustum-culls but shadow pass doesn't — off-screen characters still evaluate, upload, and draw shadows | `render_loop.rs:150+` |
| 5 | One `draw_indexed(.., 1, ..)` per submesh, no instancing — but crowds share 2–3 meshes | `deferred_renderer.rs:1100,1193` |
| 6 | Clip keys are three separately-allocated `Vec<(f32, T)>` per bone; every sample = three binary searches; poor cache behaviour at crowd counts | `model_loader.rs:35`, `sampling.rs:15` |

---

## 2. Design — scale

### S-D1. Palette ring buffer (the `LargeSsbo` backend `skinning.rs` anticipates)

One large per-frame **SSBO** holding all palettes back-to-back, sized to the
sum of actual bone counts. **Three** independently-written frame regions —
matching the renderer's 3-slot fence ring (`render_thread.rs:109`), a region
reused only after its fence is reclaimed — each aligned to
`minStorageBufferOffsetAlignment`.

**Addressing ruling** (Codex): per-draw palette *base index*, flat-indexed
in the shader — **not** a push-constant append and **not** dynamic offsets.
The push-constant block is already two `mat4`s = 128 B, Vulkan's portable
minimum (`gbuffer.vert:18`); dynamic offsets re-bind per draw anyway. So:
`view_projection` moves out of push constants into a per-pass UBO (it is
constant per pass), freeing 64 B; push constants become `model` + `palette_base`.
Shader palette decl changes from `uniform BonePalette { mat4 bones[256]; }`
to `readonly buffer` with runtime array; `MAX_PALETTE_BONES` cap goes away.

**Migration is all-or-nothing per layout:** the shadow pipeline
(`shadow_vs.glsl:12`) and the editor thumbnail / mesh-preview paths bind the
same reflected set-0 layout and migrate in the same commit. Realistic result
is one palette bind *per pass*, not per frame — still O(1) instead of O(n).
CPU side writes each skeleton's palette once (honouring `dirty`), no
descriptor allocation in the loop.

### S-D2. Two-phase FK, no allocation, retained model space

`compute_palette` splits: (1) locals → `model_space: Vec<Mat4>` **kept on
`SkeletonInstance`** (scratch reused, no per-frame alloc); (2) `palette[i] =
model_space[i] * inverse_bind`. Retained model space is the IK substrate
(I-D1) and gives bone sockets (weapon attach) for free. Note: this space is
the mesh's local **Y-up render space** — same space `debug_draw.rs:48`
reconstructs today by inverting the inverse-bind.

### S-D3. Parallel pose evaluation

rayon (new engine dep) + hecs `query_mut().into_iter_batched(n)` over
`(AnimGraphRuntime, SkeletonInstance)` — **only step 3** of the system's
run. Arming, structural insert/remove, and cache mutation stay serial
(schedule hands systems `&mut World`; the `Task 58` note on `runner.rs:387`
stands). Copy `dt`, take the `AnimClipCache` borrow immutably, per-thread
`PoseScratch`. Also: reuse the two lifecycle `Vec`s across frames.

### S-D4. Update-rate optimisation — without breaking semantics

Codex flagged the trap: events are collected against the sampled spans
(`runner.rs:599`) and a short play-once could start, fire, and end invisibly
if whole ticks were skipped. Ruling:

- **Machine tick, slot tick, and event collection run every frame** (cheap —
  no sampling). Only **pose evaluation + palette upload** are throttled.
- Rate from a significance bucket (screen-space size / distance, visible in
  camera *or* shadow frustum), with hysteresis, and entity-id staggering so
  buckets don't beat.
- **Forced evaluation** overrides the bucket: active crossfade/transition,
  active play-once, an event fired this tick, IK lock/unlock edge, first
  visible frame. Held pose otherwise (no interpolation in v1; ABA-style
  measured global ms budget and pose lerp are the v2 lever).

### S-D5. Instanced skinned draws

Batch by mesh+material; per-instance metadata SSBO (`model`, `palette_base`)
addressed by `gl_InstanceIndex`; push constants drop to nothing per draw.
Builds directly on S-D1's buffer plumbing. Crowds of shared meshes go from
hundreds of draws to a handful.

### S-D6. Clip data layout (gated on P0 profiling)

Only if sampling shows hot after S1–S4: immutable SoA streams resampled to a
fixed rate at import (index = `t * rate`, no binary search), contiguous
per-bone blocks, per-instance cursors. Must preserve sparse-channel /
rest-pose semantics. Quantisation deferred.

---

## 3. Design — IK

### I-D1. Pipeline position and spaces

Frame order becomes: machine → blend trees → play-once overlay → **IK** →
palette. IK reads S-D2's retained **pre-inverse-bind model-space** matrices
(never palette matrices), writes corrected *locals* for chain bones
(`local = parent_model⁻¹ * solved_model`), then re-runs FK phase 2.

**Space ruling** (Codex-verified): solvers run in the mesh's Y-up model
space. Gameplay targets are Z-up world; conversion is one matrix per
effector: `target_model = entity_render⁻¹ * zup_to_yup(target_world)`, with
`entity_render = TransformCache::get_render` (previous frame — one-frame
latency accepted, same as today's render path).

### I-D2. Solvers (`animation/ik.rs`, pure functions, unit-tested on the acceptance skeleton)

- **Two-bone analytic** with mandatory pole vector (BoneData has no joint
  limits or preferred bend axes — pole is the disambiguator in v1).
- **Look-at / aim** with angle clamp.
- FABRIK/CCD for N-bone chains: deferred.

### I-D3. Chains and targets

- `PlanIkChain { name, bone names, solver, weight_param }` compiled from the
  `.animgraph` (not inside `PlanTree` — that region stays single-typed
  Pose). Bone names → indices at arm time (`runner.rs:~560`). `weight_param`
  is a declared Float, so states fade IK through the existing parameter
  contract.
- `IkTargets` component: world-space effector + pole per chain, written by
  gameplay/foot placement.

### I-D4. Foot placement

`FootPlacementSystem` (before `AnimGraphSystem`): ray down from each foot's
last model-space position (via S-D2) transformed to world; pelvis offset =
lowest foot delta; foot lock holds the contact point until released.

Engine gaps to close (Codex): `PhysicsWorld::raycast` (`world.rs:237`)
returns no **surface normal** and no **filter** — extend with Rapier's
`cast_ray_and_get_normal` + a filter excluding the character's own collider.
`AnimEventFire` is name+weight only (`machine.rs:1054`) — v1 uses name
conventions (`foot_l_down` / `foot_l_up`) to drive per-foot lock state; lock
edges force pose evaluation (S-D4).

**Non-goal:** root motion. Pelvis adjust is cosmetic bone offset only; the
entity/collider never moves from animation. Root-motion extraction is its
own future task.

### I-D5. Scale interaction

IK cost is the raycasts, not the solvers. Foot IK runs only in the top
significance bucket; look-at fades by distance. IK weight → 0 skips the
solve entirely.

---

## 4. Work packages (each = one reviewable commit)

- **P0 — stress scene**: `--stress-anim N` spawns N characters on
  `content/graphs/character.animgraph`; capture baseline numbers (frame ms,
  anim system ms, palette-upload count). Every later package quotes
  before/after from this scene.
- **P1 — palette ring buffer** (S-D1): SSBO ring ×3, VP → per-pass UBO,
  `palette_base` push constant, gbuffer + shadow + thumbnail/preview shaders
  and layouts in one commit; honour `dirty`.
- **P2 — two-phase FK** (S-D2): retained model space, zero-alloc, socket
  accessor; `debug_draw` switches to reading it instead of inverting binds.
- **P3 — parallel evaluation** (S-D3): rayon dep, batched loop, per-thread
  scratch; lifecycle Vec reuse.
- **P4 — update-rate optimisation** (S-D4): significance buckets +
  hysteresis + stagger + forced-eval list; correctness tests (play-once
  fires while throttled, crossfade forces eval, event ordering unchanged).
- **P5 — IK core** (I-D1..3): solvers + tests, `PlanIkChain` compile/arm,
  `IkTargets`, IK stage wired into `AnimGraphSystem`, editor debug draw of
  effectors/poles.
- **P6 — foot placement** (I-D4): raycast normal+filter extension, per-foot
  lock via event conventions, pelvis adjust; demo on stairs/slope.
- **P7 — instanced skinned draws** (S-D5).
- **P8 — clip layout** (S-D6) — only if P0 profiling after P4 shows
  sampling hot; otherwise record and skip.
- **P9 — close-out**: docs (ARCHITECTURE ▸ Animation, KNOWLEDGE gotchas),
  ROADMAP ledger, review pass.

## 5. Acceptance

- Stress scene: 300 characters ≥ 60 fps on the dev machine; animation
  system + palette upload within a stated ms budget (numbers from P0).
- Zero steady-state allocations in the anim path (checked by profile).
- Semantics hold under throttling: anim events fire exactly as at full
  rate; play-once always visibly plays; crossfades smooth.
- IK demo: character walks stairs/slope with planted feet and pelvis
  adjust; head look-at tracks a target; IK fades via graph parameter.
- Editor unaffected: viewport preview, blend-space preview, thumbnails.
- Existing gates: tests, clippy, both builds (`editor` + standalone).

## 6. Deferred ledger

- Pose interpolation under URO + measured global ms budget (ABA-style).
- Leader-pose sharing (identical graph+params share one evaluation).
- FABRIK/CCD chains; joint limits / preferred bend axes in `BoneData`.
- Root-motion extraction and entity/collider reconciliation.
- Skeleton/mesh LOD tiers; GPU crowd path (compute skinning / baked
  animation textures) for thousands.
- Clip quantisation.

## 7. Risks

- **P1 touches every skinned pipeline at once** (gbuffer, shadow,
  thumbnail, mesh preview). Mitigation: reflected layouts keep them in one
  place; P0 scene + editor smoke after.
- **Parallel eval data race via interior mutability** — none known in the
  tick path (plan/clips immutable `Arc`); guarded by a debug assert that
  arming never runs inside the parallel section.
- **URO correctness drift**: P4's tests encode the forced-eval list; any
  new graph feature must add itself to that list or tick every frame.
