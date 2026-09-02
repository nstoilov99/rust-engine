# P4 — Update-rate optimisation (S-D4)

**Status:** done
**Plan:** §2 S-D4, §4 P4, risk §7.3. Binding ruling: machine tick, slot
tick, event collection every frame; only pose evaluation + palette upload
throttled. Significance buckets from screen-space size/distance (visible in
camera OR shadow frustum), hysteresis, entity-id stagger. Forced-eval
overrides: active crossfade/transition, active play-once, event fired this
tick, IK lock/unlock edge, first visible frame. Held pose otherwise (no
interpolation in v1).

Correctness tests are the deliverable as much as the feature: play-once
fires + visibly plays while throttled; crossfade forces eval; event ordering
identical to full rate.

## Checklist
- [x] bucket assignment + hysteresis + stagger
- [x] forced-eval list implemented exactly as ruled
- [x] tests: throttled play-once, crossfade, event ordering (+ hysteresis,
      first-visible-frame)
- [x] `cargo check` both + engine tests; commit

## How it works (implementation notes)

### Significance pre-pass (`runner.rs`, step 2.5 of `AnimGraphSystem::run`)
- New resource `AnimViewInfo { camera_pos, frustum }` (**Y-up render
  space**), inserted each frame by both hosts right before `run_schedule`
  (`standalone.rs::update`, `app.rs::update`) from `renderer.camera_3d` —
  the previous frame's camera, the accepted render-path latency. **Absent ⇒
  full rate** (tests, tools, editor previews unaffected).
- **Significance inputs shipped (deviation, documented in the code):**
  camera distance + camera-frustum visibility only. The shadow VP is
  computed in `prepare_light_data` *after* this system runs, and the shadow
  pass draws all casters unculled — there is no shadow frustum available
  main-thread pre-system. So off-frustum entities are never frozen: they
  clamp to the slowest interval (`OFFSCREEN_INTERVAL = 8`) so their shadows
  keep moving.
- Buckets/constants in one place at the top of the throttling section in
  `runner.rs`: `BUCKET_INTERVALS = [1, 2, 4, 8]`, boundaries
  `BUCKET_MAX_DISTANCE = [15, 35, 70]` m, `BUCKET_HYSTERESIS = 1.15`
  (outward needs `> bound×H`, inward `< bound÷H`), `VIS_RADIUS = 2` m pad
  on `contains_sphere`.
- Stagger: due when `(frame + entity.to_bits()) % interval == 0`.
- Positions: `TransformCache::get_render` when the host has one
  (hierarchy-correct), else the entity's `Transform` converted Z-up→Y-up
  (what the test harness uses).
- Per-entity state: `AnimGraphRuntime.throttle: ThrottleState { bucket,
  eval_this_frame, was_visible, pending_first_eval, force_eval_external }`.
  The pre-pass consumes `pending_first_eval` (first frame after arming) and
  `force_eval_external` (the P5/P6 IK hook — see below), detects the
  first-visible edge via `was_visible`, and writes `eval_this_frame`.

### The gate inside `tick_entity` (parallel section)
Machine tick + slot tick + `collect_anim_events` run **every frame** — only
`evaluate_pose` + `slot.apply` + `compute_palette` sit behind the gate.
Tick-local forces (computed on `rt` after the ticks, overriding a skip
locally, no shared state): crossfade active **before** the tick (so the
completion frame still evaluates), `AnimMachine::transition_activity()`
(new: fade active or transition fired this tick, recursing into the sampled
sub-machine chain only — a frozen sub must not pin full rate), active
play-once (`slot.playing()`), any event fired this tick. Skipped frames
hold the last pose (no interpolation). `AnimGraphSystem.skipped` counts
holds per run (bench read surface `evals_skipped_last_run()`).

### Upload gate (what shipped — the "stable bases" scheme, not the fallback)
- `SkeletonInstance.revision: u64` bumps in
  `refresh_palette_from_model_space` (hence every `compute_palette`) — the
  "did this frame evaluate" observable, also used by the tests.
- `SkinningBackend::write_palette(key, revision, palette)` (key = entity
  bits from `prepare_mesh_data`): per ring region a `RegionResidency`
  records `(key, len, revision)` per write **in frame write order**. The
  cursor **always advances** (space reserved whether or not the copy
  happens), so bases repeat exactly when the frame's write-sequence prefix
  repeats. Each visit compares writes against the region's recorded
  sequence; on the first divergence (entity appeared/vanished/reordered,
  bone count changed) the rest of the visit copies and re-records. A
  prefix-matching entry with the same revision skips the memcpy — the
  palette is still *present* in the region (R2 holds: every visible
  skeleton occupies its span in every frame's region). `grow()` keeps the
  current region's carried-over prefix and forgets the rest. No hash map,
  no allocator, O(1) per write, self-heals on any churn.

## Deviations
- Shadow frustum not a significance input (see above) — distance +
  camera-frustum, off-screen clamps to slowest instead of freezing.
- "Screen-space size" reduced to camera distance for v1 (no per-entity
  bounds available pre-render; buckets are distance bands).

## Notes for P5/P6 (IK)
- **Forced-eval hook:** set
  `AnimGraphRuntime.throttle.force_eval_external = true` on a foot
  lock/unlock edge (or any external pose-affecting event). The significance
  pre-pass consumes it and forces that frame's evaluation. Set it from any
  serial system that runs before `AnimGraphSystem` (it is plain component
  state — never touch it from inside the parallel section).
- **Buckets × IK (I-D5):** read `rt.throttle.bucket` — run foot IK
  (raycasts) only when `bucket == 0`; fade look-at by the same distance the
  bucket derives from. When IK weight > 0 changes the pose on a frame the
  bucket would skip, either set the hook or make the IK stage part of the
  gated block (it re-runs FK phase 2, which bumps `revision` via
  `refresh_palette_from_model_space` — the upload gate then copies
  naturally).
- Tests live at the end of `acceptance.rs` ("Update-rate throttling"
  section); any new forced-eval source must be added to the list in
  `tick_entity` **and** pinned there (plan risk §7.3).
