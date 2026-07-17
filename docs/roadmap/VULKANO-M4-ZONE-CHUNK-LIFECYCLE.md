# M4 — Zone & Chunk Lifecycle

**Status:** ✅ Complete (2026-07-17). Packages 0–5 landed (`67f0946`,
`62942aa`, `ffe5d0d`, `133ec27`, `8b8cc7c`, +acceptance). Acceptance
(`engine/tests/streaming_acceptance.rs`): streamed-vs-full collision raycast
parity, GUID stability across load/unload/reload, and the D4 flythrough
harness — 9 boundary crossings at 3× run speed, worst streaming main-thread
frame ~0.95 ms (< 2 ms bound), p99 within 1 ms of the standing-still
baseline. Headless harness measures the streaming update itself; renderer
frame time is not included.
**Duration:** ~1–2 weeks
**Prerequisites:** M3 (complete)
**Roadmap:** `ROADMAP.md` Task M4 — absorbs the zone-lifecycle slice of old Task 43; general scene transitions/loading screens stay deferred.

Goal: stream world cells (render mesh + collision chunks + cell entities) in and
out around a moving center, with stable `EntityGuid` identity across
unload/reload and a per-frame loading budget so cell boundaries cause **no
frame hitches**. This is the substrate M5 (replication scoped by zone), M6
(server movement vs the same chunks) and M8 (interest management with
hysteresis) build on.

---

## Current state (verified 2026-07-17)

| Piece | State | Pointer |
|---|---|---|
| `WorldManifest` (cells/zones/spawn points, deterministic root GUIDs) | Exists, written by greybox_gen, **unread at runtime** | `game_shared/src/world_manifest.rs` |
| Collision streaming primitives | Ready: `insert_chunk`/`remove_chunk`/`contains` by `IVec2` | `game_shared/src/collision/store.rs:136` |
| `CollisionWorld::load_for_scene` | Monolithic sync load of all cooked chunks | `engine/src/engine/collision/world.rs:81` |
| Scene load | Monolithic sync `load_scene` → clear world, spawn everything | `engine/src/engine/scene/scene_serializer.rs:400` |
| `MeshManager` | Append-only `Vec<GpuMesh>` + `path_index`; **no unload** | `engine/src/engine/rendering/3d/mesh_manager.rs:77` |
| Async IO | `AsyncAssetLoader` (tokio, 2 threads) does CPU-side load; GPU upload on main thread after `poll_results()` | `engine/src/engine/assets/async_loader.rs:30` |
| `EntityGuid` | On every entity, persisted in `.scene`; **no guid→entity map** (linear scans) | `engine/src/engine/ecs/components.rs:648`, ADR-013 (Proposed) |
| Frame budget | None anywhere | — |
| Known bug | `.scene` serializes runtime `mesh_index`; editor saves drift generated content (`--check` failed on it 2026-07-17) | `scene_serializer.rs:58` |

World contract from M3: 8×8 cells of 64 m (coords −4..3), 4 quadrant zones,
one mesh + one collision chunk per cell, plus 17 cooked border-sliver chunks
on the +X/+Y edges that are **not** listed in the world manifest (manifest ⊆
cooked invariant).

---

## Design

### D1. `WorldStreamer` — desired-set logic (pure, testable)

New engine module `engine/src/engine/world/` owning a `WorldStreamer` resource:

- Loads `WorldManifest` via `asset_source` at scene load (when
  `content/world/<stem>/manifest.ron` exists; scenes without a manifest keep
  today's monolithic path untouched). Validates on load: manifest `scene`
  field matches the requested scene, `version` supported, and every cell's
  `collision_chunk` exists in the cooked collision manifest (M2 failure
  policy: warn + degrade, never crash).
- Each frame, given a **streaming center** (Z-up world position) it computes
  the desired cell set on the cell grid using **Chebyshev distance with
  hysteresis**: load cells with `dist ≤ R_load`, unload only when
  `dist > R_unload` (`R_load < R_unload`, defaults `R_load = 2`,
  `R_unload = 3` → up to 5×5 resident, 7×7 lingering). Hysteresis kills
  load/unload thrash when the center oscillates on a boundary — the same
  pattern M8 needs for subscriptions.
- Diffs desired vs resident set into an ordered op queue: loads
  nearest-first, unloads farthest-first, unloads always run after pending
  loads for the same cell are cancelled.
- Tracks the center's **zone id** (from the manifest cell→zone map) and
  emits a `ZoneChanged { previous, current }` ECS event on transitions —
  M5's subscription scoping consumes this; M4 itself only logs it.

Streaming center selection:
- **Play mode / standalone**: the player entity's position (fallback: active
  camera).
- **Editor edit mode**: streaming is **off by default** — the editor loads
  the full world exactly as today. A debug toggle ("Stream around camera")
  exercises the system in-editor with the viewport camera as center.

Desired-set computation, hysteresis, and op ordering are pure functions on
`(center_cell, resident_set, manifest)` — unit-tested without any IO.

### D2. Cell lifecycle — what load/unload actually does

Per-cell **load** (staged, budget-gated; see D4):
1. **IO + parse (dedicated worker thread)**: read mesh file bytes and
   `.ccol` chunk bytes; parse mesh to a CPU-side model. This is a small
   streamer-owned worker (std thread + crossbeam channels), **not** the
   existing `AsyncAssetLoader` — its results carry only `AssetId` + success
   (no payload) and its detached `spawn_blocking` jobs can't be cancelled
   (`async_loader.rs:15,48`). Each request carries a **generation token**;
   results whose token doesn't match the cell's current generation (cell was
   unloaded or re-requested meanwhile) are dropped on receipt — no
   late-result zombie uploads. The worker is **not a FIFO dump**: the
   streamer keeps the pending list itself and dispatches at most N (default
   2) in-flight requests, picking the nearest still-desired cell at each
   dispatch — so a burst of stale wants can never queue-block newly needed
   cells, and "cancellation" is simply never dispatching + the token check.
2. **GPU upload (main thread, budgeted)**: upload mesh via `MeshManager`
   (D3), verify `.ccol` content hash against the collision manifest entry,
   `ChunkStore::insert_chunk`.
3. **Spawn (main thread, cheap)**: spawn the cell root entity with
   `EntityGuid` = the manifest's `root_entity_guid`, **identity
   `Transform`** — greybox cell meshes bake world placement into their
   vertices (`greybox_gen/src/terrain.rs`), so any non-identity root
   transform double-translates; the manifest `coord` is metadata, not an
   offset — `MeshRenderer` resolved **by path**, and a new marker component
   `StreamedCell { coord: IVec2 }`.

Per-cell **unload** (main thread, cheap — never split across frames):
despawn entities carrying `StreamedCell(coord)`, `ChunkStore::remove_chunk`,
`MeshManager::release_path` (D3). Bump `CollisionWorld::generation` on every
chunk insert/remove so the debug-wireframe GPU cache invalidates (mechanism
already in place).

**Collision slivers**: the desired *chunk* set is derived from the cooked
collision manifest, not the world manifest — every cooked chunk whose coord
is within `R_load` (Chebyshev) of the center cell loads. This picks up the
17 border-sliver chunks automatically; a cell-driven set would silently drop
edge collision. Chunk residency uses the same hysteresis as cells.

**Identity contract (the M4 slice of ADR-013)**: a cell's root entity always
respawns with the manifest GUID, so guid-keyed references survive
unload/reload. The streamer keeps a `guid → hecs::Entity` map **for streamed
cells only** — a *global* `GuidRegistry` can't be kept correct in M4 because
scene loading and many editor paths spawn directly into the raw `hecs::World`
(`scene_serializer.rs:350`); the engine-wide registry is deferred to M5's
World Identity Contract, where those spawn paths get audited anyway.

**Physics invariant**: streamed cells carry no Rapier bodies in M4 (cooked
chunks are the collision story; `PhysicsWorld` handles are not unregistered
by raw despawn, `game_world.rs:192`). Debug-assert at spawn that streamed
subtrees contain no physics components — lifting this is an explicit M6+
change.

Failure policy mirrors M2: a cell whose mesh or chunk fails to load logs a
warning and stays non-resident (retry only on re-entering the load ring);
never panic, never block the frame.

### D3. `MeshManager` unload (the one real engine refactor)

Append-only `Vec<GpuMesh>` can't stream. Change:

- `meshes: Vec<Option<GpuMesh>>` + free-list; slots are reused.
- Per-path **refcount, scoped to streamed paths**: `acquire_path` (load or
  ref-bump) / `release_path` (unref; on zero, take the slot's `GpuMesh` and
  push the index to the free-list). Paths loaded through the existing
  resolution flow (`app.rs:1161` — load-once, shared by any number of
  entities with no per-entity ownership) are **pinned**: `release_path` on a
  path that was never `acquire`d is a no-op + debug assert. This keeps the
  eviction surface exactly = streamed cells; auditing full ownership
  semantics for every MeshRenderer/prefab/preview path is out of scope until
  a task needs shared streamed assets. Debug-assert if a non-streamed entity
  resolves a streamed path (would let eviction pull a mesh out from under
  it).
- **Pin/streamed interaction is defined, not just asserted**: *pin wins*.
  `acquire_path` on an already-pinned path returns it but the entry stays
  non-evictable; if the normal resolver touches a currently-streamed path,
  the entry is **promoted to pinned** (debug-warn — in greybox this never
  fires, but promotion makes the race benign instead of a
  mesh-vanishes-under-entity bug).
- GPU safety: dropping a `GpuMesh` only drops main-thread `Arc`s; frames in
  flight keep their own `Arc<Subbuffer>` clones inside `FramePacket`, so
  vulkano frees the memory after the last frame using it completes. No
  fence-waiting needed — document this invariant on `release_path`.
- **Index stability**: resident meshes never move (slot reuse, no
  compaction), so a `mesh_index` cached in a component stays valid for the
  lifetime of that path's residency. Stale-index misuse is closed by D5
  (indices are never persisted, always resolved from path at spawn). Note:
  per-frame render data is built from a path's **whole submesh list**
  (`render_loop.rs:100`), so slot reuse must preserve `path_index` grouping,
  not just single indices.
- **Hot reload is NOT unchanged**: reload replaces the entire `MeshManager`
  (`asset_manager.rs:72`), destroying slots and refcounts — and today that
  replacement is not ordered with the app's event handling. M4 makes the
  order explicit and **main-thread only**: on receiving the reload event,
  (1) flush the streamer (despawn cells, remove chunks, drop refcount
  state), (2) rebuild the manager, (3) let the desired-set diff re-stream
  through the new manager. The watcher thread only ever sends events; it
  must not touch the manager.

### D4. Frame budget

A single `StreamingBudget` (default **1.0 ms** per frame, configurable)
gates main-thread streaming work:

- Each frame the streamer drains its op queue while
  `elapsed < budget`; at minimum one op per frame makes progress even under
  budget pressure (a 33×33 cell mesh upload is far below 1 ms; the budget
  exists for future heavier content).
- Async IO/parse is not budgeted (off-thread); only main-thread effects
  (GPU upload, chunk insert, spawn/despawn) count.
- Instrumented with `profile_scope!` and surfaced in a small debug overlay:
  resident cells / queue depth / worst streaming ms over last 300 frames.

**Acceptance metric**: scripted straight-line flythrough crossing ≥ 8 cell
boundaries at 3× run speed; p99 frame time within 1 ms of the standing-still
baseline, zero frames where streaming main-thread work exceeds 2 ms.

### D5. Content & tooling changes (greybox_gen v2)

Streaming and the monolithic `.scene` currently overlap: the scene contains
all 64 cell entities. Change of ownership — **the manifest owns cells; the
scene owns everything else**:

- `greybox_gen` v2: emit `greybox.scene` **without** the 64 cell entities
  (keeps sun, camera, traversal-gym entities); cells exist only as manifest
  entries + mesh/chunk files. Version bump, regenerate, re-cook, drift guard
  stays authoritative.
- **Cooker must learn the manifest** (hard dependency, not optional): the
  collision cooker discovers inputs solely from scene entities with
  `StaticCollision` + `MeshRenderer` (`collision/output.rs:66`) — with cells
  gone from the scene, a re-cook would silently produce a gym-only world.
  `cook_scene` v2 cooks **scene entities ∪ world-manifest cells** (manifest
  cells cook their mesh file at identity transform), and
  `scene_content_hash` must fold the manifest bytes + manifest-referenced
  mesh bytes so staleness detection keeps covering cells.
- Editor with streaming off ("full world") gets cells by asking the streamer
  to load *all* manifest cells at startup — one code path, no radius.
- **Editor save must not serialize streamed cells**: `serialize_scene`
  skips entities with `StreamedCell`. This also fixes the observed
  save-churn class of drift.
- Play-mode snapshot/restore: snapshots exclude `StreamedCell` entities;
  on restore the streamer re-streams from current center (cells are static
  in M4, so no state is lost by design).
- **Bug fix (do first, standalone commit)**: stop persisting `mesh_index`
  in `.scene`. The component field is already `#[serde(skip)]`
  (`components.rs:86`) — the leak is in the persistence layer:
  `ComponentData` still carries and writes the runtime value
  (`scene_format.rs:737`, `scene_serializer.rs:53`), and `prefab.rs:107`
  copies the stale index too. Fix: `ComponentData` stops **writing** the
  field and ignores it on read (`serde(default)` keeps old files loading);
  spawn leaves `mesh_index` at 0 and the existing path-based resolution
  pass (`app.rs:1161`, `render_loop.rs:100`) remains the sole authority.
  Removes the nondeterministic-save trap for all scenes, streamed or not.

### D6. Explicit non-goals

- No terrain LOD / distance-based mesh simplification (Task 46).
- No trigger-volume or scripted streaming, no loading screens (Task 43/44 leftovers, deferred).
- No dynamic-entity ↔ cell assignment (M5 decides ownership; M4 only streams manifest cells).
- No server-side streaming — this is client/editor lifecycle only; the server consumes chunks its own way in M6.
- No prefetch heuristics beyond the load ring (velocity-based prediction can bolt onto the same desired-set function later).

---

## Work packages (each = one reviewable commit)

| # | Package | Contents |
|---|---|---|
| 0 | `fix(scene)`: mesh_index de-persisted | `ComponentData` stops writing/reading it (serde default), prefab path fixed, round-trip test; regen greybox scene; drift guard green |
| 1 | `feat(render)`: MeshManager slots + scoped refcount | `Vec<Option<GpuMesh>>`, free-list, `acquire_path`/`release_path` (pinned non-streamed paths), unit tests incl. slot reuse, `path_index` grouping, pin semantics |
| 2 | `feat(world)`: manifest loader + WorldStreamer core | runtime `WorldManifest` load/validate, desired-set + hysteresis + op ordering (pure fns), `ZoneChanged` event, unit tests |
| 3 | `feat(world)`: cell lifecycle execution | streamer IO worker + generation tokens, budgeted main-thread stage, spawn/despawn with GUIDs (streamed-cell map), chunk insert/remove + sliver handling, hot-reload flush, failure policy |
| 4 | `feat(content)`: greybox_gen v2 + cooker v2 + editor integration | cell-less scene, **manifest-aware cook + staleness hash**, streamer-driven full-world editor load, save/snapshot exclusion, debug toggle + overlay |
| 5 | `test/docs`: acceptance | flythrough hitch harness, streamed-vs-full collision parity spot-check (raycast battery), load/unload/reload GUID-stability integration test, ROADMAP/plan status updates |

Suggested order is as numbered; 1 and 2 are independent and can interleave.

---

## Risks & mitigations

1. **GPU buffer freed while render thread still recording** — mitigated by
   Arc lifetimes through `FramePacket` (same guarantee the debug-draw cache
   relies on); assert in debug builds that `release_path` never drops a
   mesh uploaded in the same frame.
2. **Editor systems referencing despawned cell entities** (selection,
   undo/redo): streaming stays off by default in edit mode; the debug
   toggle clears selection on unload of a selected entity. Undo entries
   referencing streamed cells are the sharpest edge — the toggle is
   explicitly debug-only in M4, and cell entities are not editable content
   (they're generated), so the practical exposure is nil.
3. **Desired-set flapping at diagonal boundaries** — Chebyshev + hysteresis
   handles it; the unit tests include an oscillation scenario on a cell
   corner.
4. **Manifest/scene/cook drift** (three artifacts now share truth): the
   existing staleness hash covers scene↔cook; add a manifest↔collision
   validation at load (every manifest `collision_chunk` must exist in the
   cooked collision manifest) with the M2 failure policy (warn + disable
   that cell's collision, not a crash).
5. **Play-mode restore ordering** (restore snapshot vs re-stream): restore
   first, then let the streamer converge on the next frames; cells are
   static so late arrival is cosmetic only.
6. **Scene tabs**: dormant tabs park whole `GameWorld`s while `MeshManager`
   stays global (`app.rs:3596`), and `park_active_scene` has no teardown
   hook today. Policy: streaming state belongs to the **active** scene only,
   and since editor "full world" cells are streamer-owned too (D5), parking
   **always** flushes the streamer synchronously *before* the world swap —
   not just when the debug toggle is on. Reactivating a tab re-streams from
   its manifest (full set or radius, per mode). This needs a small explicit
   teardown call added to the park path.

## Open questions (decide at review)

1. Budget default 1.0 ms and rings `R_load=2` / `R_unload=3` — fine for
   greybox, or set from a config so M8 load tests can sweep them?
2. Should the standalone client *without* a player entity (current
   `main.scene`) stream around the camera, or is streaming simply inert for
   scenes without a world manifest? (Plan assumes: inert without manifest.)
3. `GuidRegistry` now vs deferring to M5 — plan includes it now because
   spawn/despawn churn makes linear guid scans immediately worse.
