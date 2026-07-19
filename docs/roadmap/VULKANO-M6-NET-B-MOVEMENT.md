# M6 — Net-B: Server-Authoritative Movement

**Status:** 📝 Planned (approved 2026-07-19; Codex/gpt-5.6-Sol plan review reconciled, 9 findings folded in)

Replaces M5's trust-the-client `submit_input` with a shared kinematic character
controller that runs identically (within tolerance) on the client (prediction)
and inside the server WASM module (authority), colliding against the M2 cooked
collision chunks. Governed by ADR-015: kinematic, not dynamic; server authority
means **no bit-exact determinism requirement** — float divergence surfaces as
tiny reconciliation corrections.

## Current state (verified 2026-07-19)

- `game_shared::collision` is the ready-made shared query backend: no-I/O
  `ChunkStore` (`insert_chunk(bytes)`, `raycast`, `cast_shape`, `contacts`),
  parry3d `=0.20.2` + parry3d-f64 twin pin (f64 per-triangle casts for
  1 mm / 0.1° tolerances), deterministic candidate ordering, seam ties broken
  by stable triangle ID. Already compiles to WASM (module depends on
  game_shared). `TriangleFlags` currently exposes only `FLAGS_MATERIAL_MASK`;
  walkable/blocking bit *positions* are reserved, the constants and semantics
  are added in M6 (package 2).
- `battery.rs` golden battery covers collision *queries* on identical bytes
  across targets; it is the foundation the M6 *motion* trace suite builds on,
  not a motion parity suite itself (`POSITION_TOLERANCE = 1e-3`,
  `NORMAL_TOLERANCE_DEG = 0.1`).
- Greybox collision content: **81 chunks** (coords −4..4 squared),
  4,278,780 bytes of `.ccol` + 8 KB manifest. Trimesh-only v1 format.
- Server (`server/game_module/src/lib.rs`): 100 ms scheduled `tick` (clock +
  NPC wander); `submit_input(epoch, seq, x, y, z, yaw)` accepts the client's
  integrated position after finite/step-cap checks. Player row carries
  `x y z vx vy vz yaw epoch last_input_seq last_update_micros`.
- Client: `game_client_net` coalesces `ClientInput` to the latest sample and
  sends at `INPUT_SEND_HZ = 20`, **stamping epoch/seq itself at send time**.
  Replication of the own row's `(epoch, last_input_seq)` IS the ack
  (`NetEvent::InputAck { epoch, seq }` — carries no state).
  `drive_local_player` in `game_client/src/replication.rs` integrates position
  locally. `InterpBuffer`/`NetClock` handle remote proxies only.
- Client collision: the engine's `CollisionWorld` (scene-stem loading, editor
  streaming) is *not* available in standalone — prediction cannot piggyback on
  it (see D4: prediction owns its own `ChunkStore`).
- Constants in `game_shared/src/net/schema.rs`: `INPUT_SEND_HZ = 20`,
  `PLAYER_SPEED_MPS = 4.0`, `SPRINT_MULTIPLIER = 1.6` (`MAX_INPUT_STEP_M`
  retires with trust-the-client).

### Binding constraints

- M0 spike: one delivery flush per transaction; practical ceilings ~500
  uniform clients, 150–200 per crowd, serial reducer ceiling ~1,100–1,200/s.
  → batch all players' movement in **one** scheduled transaction per tick.
- ADR-015: never write our own collision math; never put Rapier in the server
  module (client Rapier stays cosmetic-only); grounding seam
  (`GroundRef { entity_id, generation, local_anchor, inherited_velocity }`)
  designed day one, moving platforms deferred to M6.5.
- Parity is tolerance-based. Native/WASM lowering, transcendentals, and
  threshold branches can diverge; mitigate with fixed `dt`, stable candidate
  ordering, conservative epsilons — not with bit-exactness claims.
- Schema migration policy: M6 adds tables and Player columns. The dev database
  is disposable — **every M6 package that touches the schema requires
  `./publish.ps1 -Wipe`** (same as M5 practice). No incremental-migration
  path is designed; additionally `init` timer inserts are made idempotent
  (insert only if the scheduled row is absent) so re-publishes never
  double-schedule.

## Design

### D1. Collision data into the server WASM (decided: build-time embedding)

A generated registry of `include_bytes!` references — not blob tables (needs
uploader + atomic activation + restart-safe BVH rebuild; overkill for M6), not
cooker-generated Rust arrays (duplicates `.ccol`, inflates compile time).

- `server/game_module/build.rs` resolves
  `{CARGO_MANIFEST_DIR}/../../content/collision/greybox/`, canonicalizes it,
  and writes `$OUT_DIR/collision_registry.rs`: a static slice of
  `(chunk_coord, &'static [u8])` via `include_bytes!`. Generated path literals
  use forward slashes (or raw strings) so Windows backslashes never produce
  escape sequences. Emits `cargo:rerun-if-changed` **per chunk file and for
  the manifest**, not just the directory. Also embeds the manifest bytes and
  a `COLLISION_MANIFEST_HASH: u64` (xxhash of the manifest file).
- Module holds a lazily initialized `OnceLock<Option<ChunkStore>>`: first
  movement tick pays the BVH build (measured + logged). Malformed embedded
  data yields `None` — the movement tick then early-outs with a throttled
  error log instead of panicking the scheduled transaction every 50 ms.
  Never build BVHs per-transaction.
- `publish.ps1` gains a printed module-size report and fails loudly if publish
  rejects the ~4.3 MB payload (SpacetimeDB documents no hard ceiling; the
  smoke test is the guarantee).
- Content-mismatch guard: `COLLISION_MANIFEST_HASH` is exposed in the existing
  version/config row; the client compares it against the hash of the manifest
  *it* loaded (D4) at connect and surfaces a named error, like the
  protocol-version gate. Mismatch = disconnect, not silent divergence.

### D2. Shared kinematic controller (`game_shared::motion`)

Pure functions over `&ChunkStore` — no I/O, no ECS, wasm-safe.

- `MotionConfig`: capsule radius / half-height, `walk_speed = PLAYER_SPEED_MPS`,
  `sprint_mult = SPRINT_MULTIPLIER`, `gravity`, `jump_speed`, terminal fall
  speed, `max_slope_deg`, `step_height`, `snap_dist`, `skin` (contact offset,
  0.02 m), fixed `MOVE_DT = 1.0 / 20.0`.
- `MotionState`: `pos: [f32;3]`, `vel: [f32;3]`, `grounded: bool`,
  `ground_ref: Option<GroundRef>` (reserved, always `None` in M6; serialized so
  M6.5 platforms are additive).
- `MoveIntent`: `move_dir: [f32;2]` (world-space XY, matching M5), `yaw`,
  `sprint`, `jump`.
- `step(cfg, state, intent, &ChunkStore) -> MotionState` — **normative order**
  (identical on both sides; every threshold lives in `MotionConfig`):
  1. Depenetrate: `contacts()` at current pos; push out along contact normals,
     at most `2 × skin` total per step.
  2. Horizontal wish velocity from `move_dir` (clamped to unit length) ×
     walk/sprint speed — instantaneous, no acceleration in M6. `yaw` is
     pass-through state, normalized to (−π, π].
  3. Jump: if `grounded && intent.jump` → `vel.z = jump_speed`,
     `grounded = false`; ground snap is skipped this step.
  4. Gravity: `vel.z −= gravity × dt`, clamped to terminal fall speed.
  5. Horizontal collide-and-slide: ≤ 3 `cast_shape` iterations, stop distance
     backed off by `skin`; casts shorter than 1e-5 m are skipped; blocked-and-
     grounded triggers one step-up attempt (re-cast from `+step_height`, then
     cast down; accept only if the landing normal is walkable).
  6. Vertical pass: cast along `vel.z × dt`; a downward contact with walkable
     normal sets `grounded = true` and zeroes `vel.z`.
  7. Ground snap: if previously grounded, not jumping, `vel.z ≤ 0`, and step 6
     found no ground → cast down `snap_dist`; snap and stay grounded on
     walkable hit, otherwise become airborne (no hysteresis beyond
     `snap_dist`).
- Walkability: normal-vs-`max_slope_deg` test; where `TriangleFlags`
  walkable/blocking bits (new constants, package 2) are set they override the
  slope test. Greybox v1 has no authored flags → slope fallback everywhere.
- Steep slopes are slid along (treated as walls), not walked.

### D3. Server: per-input queue + 20 Hz movement tick

`submit_input` stops moving anything. New signature carries intent:
`submit_input(epoch, seq, move_x, move_y, yaw, sprint, jump)`.

- **Two sequence counters** (Codex finding: one counter can't serve both):
  - `last_input_seq` on the Player row keeps its M5 meaning — highest
    *received* seq, updated at acceptance (`accept_input` unchanged: epoch
    match + strictly increasing). Guards against reorder/replay.
  - `last_applied_seq` (new column) — highest seq *consumed by the movement
    tick*; this is what prediction reconciles against.
- Accepted inputs are **queued**, not latched: private (non-public) table
  `pending_input { entity_id (btree), seq, move_x, move_y, yaw, sprint,
  jump }`. A latest-intent mailbox would collapse several predicted steps
  into one server step and lose `jump` edges; the queue keeps client
  prediction steps and server authoritative steps 1:1 (both run `MOVE_DT`
  per seq).
- New scheduled table `move_timer, scheduled(move_tick)` at 50 ms (inserted
  idempotently in `init`). One transaction per tick iterates players with a
  live session:
  - Pop pending inputs in seq order, at most `MAX_STEPS_PER_TICK = 4`
    (catch-up bound); each popped input = one `motion::step` with `MOVE_DT`;
    set `last_applied_seq = seq` after each. Queue depth beyond 8 → drop
    oldest (reconciliation snaps the client).
  - Empty queue → repeat the last consumed *move* intent (not jump) for a
    250 ms grace (5 ticks), then zero `move_dir`; gravity always integrates
    (a silent client still falls). These no-input steps advance no seq.
  - Write `x y z vx vy vz yaw grounded last_applied_seq last_update_micros`
    once per player per tick.
- Player row gains `grounded: bool` and `last_applied_seq: u32`, so **one row
  update atomically carries the complete ack**
  `(epoch, last_applied_seq, pos, vel, yaw, grounded)`. Replication of the
  own row remains the ack transport — no new ack table or reducer.
- The 100 ms `tick` keeps clock + NPC wander (NPCs remain non-colliding).
- `dev_teleport` clears the pending queue, resets `grounded = false`, and
  leaves seq counters intact.
- Disconnect teardown deletes the player's `pending_input` rows.

### D4. Client: prediction owns the sequence

`drive_local_player` is replaced by `PredictionState` in `game_client`.

- **Prediction owns its collision data**: it builds a `game_shared`
  `ChunkStore` directly from `content/collision/greybox/` bytes via the asset
  source (works in standalone and editor; independent of the engine's
  `CollisionWorld`/streamer). All 81 chunks load at connect (4.3 MB — no
  streaming); prediction and input sending are gated until the store is built
  and the manifest hash matches the server's (D1). The cosmetic Rapier path
  is untouched.
- Fixed-step accumulator at 20 Hz, aligned with the send rate: each step
  samples current input → assigns `seq` (**seq ownership moves from
  `game_client_net` to the game side**) → runs `motion::step` → records
  `(seq, MoveIntent, MotionState)` in a bounded deque (40 entries ≈ 2 s;
  overflow drops oldest and forces a snap on next ack) → sends the input.
  Accumulator catch-up is capped at 5 steps per frame; excess time is
  discarded.
- Backend changes (`game_shared::net` + `game_client_net`):
  - `ClientInput` becomes intent (`move_dir`, `yaw`, `sprint`, `jump`, caller-
    stamped `seq`); the backend forwards without re-stamping or coalescing
    (one input per prediction step, same 20 Hz wire rate). Epoch stays
    backend-stamped from the own row; epoch restarts surface as a `NetEvent`
    so the game side resets seq + prediction buffer.
  - `NetEvent::InputAck` is replaced by
    `NetEvent::OwnStateAck { epoch, seq, pos, vel, yaw, grounded }`, emitted
    on own-row updates (the current code ignores own-row state — this event
    is the new single channel for authoritative own state; `EntityState`
    stays unchanged for proxies).
- Ack handling: drop records `≤ ack.seq`; compare the recorded predicted
  state at `ack.seq` with the server state; within epsilon (1 cm pos /
  0.1 rad yaw) do nothing, else reset simulation to server state and replay
  the remaining records through `motion::step`. Because the server applies
  exactly one step per seq (D3), predicted-at-seq vs server-at-seq is
  well-defined; server no-input gravity steps only occur during packet loss
  and are absorbed as ordinary corrections. The positional jump goes into a
  **visual-only error offset** decayed over ~200 ms — rendering adds it,
  collision and future predictions never see it.
- `InterpBuffer` / `NetClock` untouched (remote-proxy-only).

### D5. Parity harness (tolerance-based, actual WASM)

- Trace format (RON in `game_shared/src/motion/traces/`): spawn state + a list
  of per-step `MoveIntent`s + expected per-step positions, recorded from a
  native run. Cases: flat walk, slope up/down at/over limit, step-up, jump
  arc, fall + land, wall slide, **chunk-seam crossing** (named failure mode),
  one 60 s long-run.
- Native side: unit tests replay traces against the checked-in greybox chunks,
  asserting per-step position within 1 mm and long-run terminal drift within
  a bounded envelope (no state quantization — decided; server authority makes
  drift self-correcting).
- WASM side (actual, not cross-compiled-native): dev reducer
  `run_parity_trace(trace_id) -> final state + per-step hash` in the module;
  an `#[ignore]`d test (same infra as the M5 acceptance suite) invokes it and
  compares against the native replay of the same trace.
- The M2 golden battery keeps running unchanged underneath (query-level
  parity on the same bytes).

### D6. Combat groundwork (M7 unblock)

- `game_shared::motion::broadphase`: cell-indexed position grid (reuse
  `world_grid` cell math) over the player/NPC rows the server already holds;
  `aoe_candidates(center, radius)` returns entity ids by cell overlap.
- `hitscan(origin, dir, max_dist)`: `ChunkStore::raycast` for static world +
  capsule tests against broadphase candidates; returns first hit
  (world or entity).
- Projectile: fixed-step segment sweep (per movement tick, same 50 ms) using
  the same raycast + candidate tests. Server-side only; no replication of
  projectiles in M6 (M7 decides representation).

### Non-goals (deferred)

- Moving platforms / ground-ref attachment (M6.5 — seam is reserved, unused).
- NPC collision with the world (NPCs keep scripted orbits).
- Interest-managed subscriptions (M8), combat content (M7), client-side
  Rapier removal or changes (cosmetic path untouched).
- Blob-table collision streaming / live content updates.
- Incremental schema migration (dev DB is wiped per schema change).

## Work packages (each = one reviewable commit)

1. **Embedded collision registry** — build.rs registry + manifest hash,
   lazy `ChunkStore` in module, hash in config row + client gate, publish.ps1
   size report + publish smoke test. WASM trace-harness skeleton
   (`run_parity_trace` plumbing, empty case list).
2. **Shared controller** — `game_shared::motion` (config/state/intent/step per
   D2's normative order), walkable/blocking `TriangleFlags` constants,
   walk/slope/step/jump/gravity/wall-slide unit tests on synthetic meshes +
   first recorded greybox traces.
3. **Server authority + client bridge** — `pending_input` queue, two-counter
   scheme, 50 ms `move_tick`, grounded/last_applied_seq columns, held-intent
   grace, bindings regen, **and the minimal `game_client_net` adaptation to
   the new reducer signature** (backend still stamps seq; `drive_local_player`
   temporarily sends intent instead of position). The workspace never has a
   non-building state and the M5 acceptance suite stays green (its bots use
   `dev_teleport`, not movement inputs). Requires `-Wipe` publish.
4. **Client prediction** — seq ownership move, prediction-owned `ChunkStore`
   load + hash gate, `PredictionState` replacing `drive_local_player`,
   `OwnStateAck` event, replay-on-ack, visual error offset, epoch-restart
   event. Two-instance manual test: proxy stays interpolated, local stays
   crisp under artificial latency.
5. **Parity suite** — full trace battery incl. chunk-seam + long-run, actual
   WASM comparison via `run_parity_trace`, randomized-intent fuzz (native
   only, tolerance envelope), perf: movement-tick p95/p99 logged, cold BVH
   load measured.
6. **Combat groundwork** — cell broadphase, `aoe_candidates`, hitscan,
   fixed-step projectile sweep; unit tests on synthetic layouts.

## Risks & mitigations

| Risk | Mitigation |
|---|---|
| Module size/memory (4.3 MB embed + BVH) | publish smoke test + size report (pkg 1); measure cold load + resident memory |
| Native/WASM float divergence at seams | actual-WASM traces, stable triangle-ID ordering, tolerance envelope; divergence beyond envelope = named test failure |
| Movement-tick transaction overruns | p95/p99 instrumentation (pkg 5); `MAX_STEPS_PER_TICK` bounds work per player; M0 ceilings respected; cap active population before sharding |
| Prediction buffer loss/overflow | bounded deque + `MAX_STEPS_PER_TICK` catch-up bound, snap-to-server fallback |
| Content mismatch client vs server | manifest hash gate at connect (pkg 1/4) |
| Schema changes on a live dev DB | `-Wipe` publish required per schema package; idempotent timer inserts |
| Reducer signature change breaks client build mid-milestone | pkg 3 bundles the client bridge; no non-building intermediate state |

## Decisions (ruled 2026-07-19)

- Build-time `include_bytes!` embedding for M6 — **approved**.
- 20 Hz scheduled movement authority (not the 100 ms tick) — **approved**.
- No long-run state quantization; reconciliation absorbs drift — **approved**.
- Package 6 (combat groundwork) stays in M6 — **approved**.
- 81 chunks (−4..4²) confirmed as the intended greybox coverage.
- Post-review (Codex findings): per-input queue instead of latest-intent
  mailbox (1:1 seq↔step, preserves jump edges); split
  `last_input_seq`/`last_applied_seq`; `OwnStateAck` event carries full state;
  prediction owns its own `ChunkStore`; schema changes = `-Wipe` publish.
