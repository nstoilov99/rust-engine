# Task M2: Collision Pipeline v1 (cooked chunks)

**Status**: ✅ Complete (implemented 2026-07-17; all 11 work items landed — see "Implementation notes" at the end)
**Duration**: ~3 weeks
**Prerequisites**: M0 go decision (✅ 2026-07-16)
**Related**: ADR-015 (kinematic movement), M3 (greybox world consumes chunks), M4 (zone lifecycle streams them), M6 (movement parity tests), M8 (interest cells share the grid)

---

## Goal

Offline cooking of a scene's static geometry into compact, versioned binary
**collision chunks** aligned to a new world grid, loaded and queried
**identically** by the client and (in M5+) the server WASM module. This is the
foundation ADR-015 stands on: one shared query layer, one data format, no
client/server divergence by construction.

**Non-goals (v1):** dynamic/moving colliders (grounding seam is M6/M6.5),
convex decomposition, heightfield section (format reserves it; M3 greybox is
trimesh), runtime re-cooking, LODs, cooked-BVH caching, server integration
itself (M5 — but the crate must compile for `wasm32-unknown-unknown` now).

---

## Architecture decisions (the load-bearing ones)

### 1. parry3d is the shared query layer — pinned to Rapier's exact version

- The shared collision code depends on **the exact parry3d version the
  workspace lockfile resolves for rapier3d 0.25.1 — currently `=0.20.2`**
  (verify against `Cargo.lock` at implementation time) — same types, zero
  duplicate-crate risk. The server WASM module (M5) depends on parry alone,
  never rapier (no solver/bodies — viable on `wasm32-unknown-unknown`,
  SpacetimeDB's target).
- Minimal features: no `parallel`, no SIMD, no serde. Whenever Rapier is
  upgraded, the parry pin moves in lockstep (chunks stay valid — raw geometry
  is version-agnostic; see format).
- API reality checks for the implementer: `TriMesh::new(vertices, indices)`
  returns a `Result` and builds its BVH internally; shape-casts go through
  parry's query API with `ShapeCastOptions`; **`ShapeCastHit` does NOT carry a
  triangle/part id** — see decision 6.

### 2. Collision lives in `game_shared::collision` — and queries run in Z-up

- Per ADR-015 ("one loader crate in game_shared"): a new `collision` module in
  `game_shared`, adding the parry3d dependency. `game_shared` stays
  engine-free; parry is not the engine.
- **Cooked data and all queries are Z-up world space.** The kinematic
  controller (M6) queries this layer directly — it never touches the Rapier
  world. This kills an entire divergence class: today's Z-up→Y-up conversion
  (`engine/src/engine/physics/world.rs`) stays render/cosmetics-only. Rapier
  keeps ragdolls/debris; gameplay collision moves to the shared layer.

### 3. Raw geometry is the durable format; BVH is rebuilt at load

- Chunks store chunk-local `f32` vertices + `u32` indices + per-triangle
  flags — **not** serialized parry `SharedShape`/QBVH (private layouts, not a
  stable asset ABI across crate versions). Loader builds `TriMesh` (and its
  QBVH) on load. If load time ever matters, an opaque BVH-cache section can be
  added behind the reserved section mechanism — raw geometry remains the
  source of truth.

### 4. World grid is born here, in `game_shared::world_grid`

- Constants: `CHUNK_SIZE = 64.0` m (matches the M0 spike's interest-cell size;
  M8 reuses this grid), chunk coordinate = `IVec2` (XY plane, Z-up world;
  vertical is unbounded within a chunk v1).
- **Chunk-local coordinates**: a chunk stores geometry relative to its origin
  (`chunk_coord * CHUNK_SIZE`); world position = integer chunk coord + local
  f32. Keeps float precision flat across the world.

### 5. Everything cooks to trimesh in v1

- Static primitives (Cuboid/Ball/Capsule from `physics/components.rs`) are
  tessellated into the chunk trimesh at cook time (sensor colliders excluded);
  mesh entities contribute their RMSH geometry. One section type, one query
  path. The format reserves section IDs for `Heightfield` and `Primitives` if
  profiling ever justifies them.
- **RMSH axis caveat (critical)**: `.mesh` vertices are stored in Y-up render
  space (`mesh_import.rs` applies axis conversion at import). The cooker must
  NOT read raw RMSH and apply `local_matrix_zup()` — that double-converts.
  Work item: a **canonical collision-geometry accessor** in the engine's asset
  layer that returns Z-up-local positions, reproducing exactly the import-time
  axis/scale handling the renderer trusts. Cooker and any future consumer go
  through it.

### 6. Triangle attribution needs a custom query path

Parry's stock `cast_shapes` against a whole `TriMesh` returns no triangle id,
but M6 needs per-hit flags (walkable/material) and seam dedup needs identity.
The chunk query is therefore implemented as: BVH traversal for candidate
triangles under the swept AABB → per-triangle shape-cast → keep earliest TOI,
carrying our own triangle index → flags/id lookup. Deterministic tie-breaking
on equal TOI: lowest stable triangle id wins. This is a self-contained,
testable unit — budgeted explicitly in the work breakdown.

---

## Chunk file format (`.ccol`, one file per chunk)

Little-endian, explicitly serialized field-by-field (never struct memcpy),
16-byte-aligned sections, every count/offset validated at load.

**Header:** magic `RCOL`, format version (u32), cooker version hash (u32 —
parry pin + cooker code; client/server refuse to mix), flags (u32), chunk
coord (i32×2), local AABB (f32×6), section count + table (id, offset, size).

**Section `TRIMESH` (v1's only section):** vertex count, **triangle count**,
vertices (f32×3, chunk-local), indices (u32×3), per-triangle **stable id**
(u64: hash of source-entity GUID + source triangle index — powers seam dedup
and battery references), per-triangle flags (u32: material id low 16 bits,
walkable/blocking bits reserved for M6, rest zero).

**Validation at load**: counts/offsets in bounds, indices < vertex count, all
floats finite, sane maximums; unknown section ids are skipped (forward
compat), unknown *format version* is rejected. **Version policy**: format
version mismatch = refuse; cooker hash is informational (raw geometry outlives
parry/cooker upgrades) — client/server *chunk-set* compatibility is enforced
via manifest content hashes, which is what ADR-015's "refuse to mix versions"
actually needs.

**Manifest** (`content/collision/<scene>/manifest.ron`): scene name, format +
cooker versions, grid constants, list of cooked chunk coords with content
hashes. Loader uses it for existence checks and version validation; M4 streams
from it. Ships inside the existing RPAK like any other content.

---

## Cooking flow

The cook logic is a **library module in `engine`** (e.g.
`engine::collision::cook`) so the editor menu action calls it directly —
no editor→tool-crate dependency cycle. `tools/collision_cooker` is a thin
CLI wrapper over that module (follows the `pak_tool` precedent) for headless
cooks and CI.

**`StaticCollision` marker — full serialization surface** (this is real work,
not just a struct): new component in `physics/components.rs`, a
`ComponentData` enum variant in `scene_format.rs`, save/load arms in
`scene_serializer.rs`, prefab component mapping, and an inspector
add-component entry. Opt-in per entity.

1. Load scene (existing `.scene` RON path).
2. Gather static collision sources: entities with a fixed `RigidBody` +
   `Collider` (primitives; **sensor colliders excluded**), and entities with
   `MeshRenderer` + `StaticCollision`. Skinned meshes rejected with a warning.
3. Mesh-local positions come from the **canonical collision-geometry
   accessor** (decision 5, Z-up local); compose to world space via
   `local_matrix_zup()` hierarchy (same path the renderer trusts). Negative
   determinants (mirror scales) flip winding; zero-area/degenerate triangles
   and non-finite vertices are dropped with a warning.
4. **Weld** vertices (epsilon 1e-4 m) *before* chunk assignment, so chunk
   borders come from identical source vertices — seams are crack-free by
   construction (duplicates are bit-identical welded geometry).
5. Assign triangles to chunks: a triangle belongs to **every chunk its AABB
   overlaps** (duplication at borders, no clipping/splitting). Oversized
   triangles (AABB spanning > 2×2 chunks — e.g. one giant ground quad) are
   deterministically midpoint-subdivided pre-weld to bound duplication;
   cooker warns so authors can fix the source instead.
6. Emit `.ccol` per non-empty chunk + manifest. Deterministic output: stable
   triangle ordering (sorted by source entity GUID, then source index) so
   re-cooks of an unchanged scene are byte-identical (hash-friendly).

**Pak/export integration**: cooked chunks + manifest live under
`content/collision/<scene>/` and ship in the RPAK like any content. The
export script runs the cooker before packing; manifest content hashes make
staleness detection cheap (re-cook only when the scene changed).

## Loading & query API (`game_shared::collision`)

- `ChunkStore`: owns loaded chunks (`HashMap<IVec2, LoadedChunk>`), explicit
  `insert_chunk(bytes)` / `remove_chunk(coord)` — **no I/O in the shared
  crate**; client hands it file/pak bytes, server (M5) hands it bytes from its
  own storage. M4's streaming drives insert/remove.
- Queries (all Z-up, all resolving against every chunk overlapped by the
  swept AABB + skin, earliest-TOI wins with stable tie-breaking on equal TOI):
  - shape-cast (capsule/any parry shape): pose + translation delta →
    `{toi, normal, witness point, triangle flags}`
  - contact/overlap probe (initial depenetration for the M6 controller)
  - raycast (targeting, ground probes)
- This surface is exactly what a move-and-slide controller needs (sweep,
  slide, step up/forward/down casts, ground snap, slope from `normal·+Z`) —
  the controller itself is M6.

## Client integration (this task proves the loop end-to-end)

- Engine loads chunks for the current scene at scene-load (all chunks v1 —
  streaming is M4) through the existing filesystem/pak asset source.
- Debug draw: cooked-chunk wireframe + chunk-grid overlay via the existing
  `physics/debug_render.rs` path, toggle in the editor.
- A temporary test: editor fly-camera raycast against the `ChunkStore`
  (verifies transform correctness visually against the rendered scene).
- **Lifecycle**: a `CollisionWorld` engine resource owns the `ChunkStore`.
  Populated at scene load, cleared on scene switch/reload; play-mode reads the
  same store (static geometry — no play/edit divergence). Manifest missing or
  version-mismatched = scene loads with collision disabled + visible warning;
  an individual corrupt chunk = logged error, chunk skipped (editor iteration
  stays unblocked; the cooker is the fix, not the loader).

---

## Parity & correctness tests

1. **Golden battery** (the M2 deliverable the roadmap names): a RON-defined
   battery of shape-casts/raycasts (slopes, steps, edges, seam-crossing
   sweeps, degenerate near-parallel hits) against a checked-in cooked test
   chunk set. Assertions within tolerance: position/TOI ≤ 1 mm, normal ≤
   0.1°. Runs in CI as a normal `cargo test`.
2. **wasm32 build gate**: `game_shared` (with collision) compiles for
   `wasm32-unknown-unknown` in CI — catches accidental std/engine deps now,
   not in M5.
3. **Seam fuzz**: randomized sweeps crossing chunk borders on the test set;
   assert single consistent hit (no double-hit from duplicated border
   triangles, no gap). Duplicated-triangle dedup by stable triangle ID.
4. **Determinism check**: cook twice, byte-compare.
5. Full client-vs-server-WASM battery execution stays in M6 (per roadmap) —
   but the battery format is defined here so M6 reruns it, not reinvents it.

---

## Work breakdown (~3 weeks)

| # | Work | Est |
|---|------|-----|
| 1 | `world_grid` constants + `collision` module skeleton in `game_shared`, parry3d pin, wasm32 CI gate | 0.5 d |
| 2 | `.ccol` format: writer + validating reader + manifest | 2 d |
| 3 | Canonical collision-geometry accessor (Z-up mesh positions in asset layer) | 1 d |
| 4 | `StaticCollision` component: ComponentData variant, scene serializer, prefab mapping, inspector UI | 1 d |
| 5 | Cook library in `engine`: scene walk, weld, chunk assignment + subdivision policy, primitive tessellation, determinism | 3 d |
| 6 | `tools/collision_cooker` CLI + editor cook menu action | 1 d |
| 7 | `ChunkStore` + query API incl. triangle-attribution path (BVH traverse → per-tri cast → tie-break) | 3 d |
| 8 | Client integration: `CollisionWorld` lifecycle, debug draw, raycast smoke test | 1.5 d |
| 9 | Pak/export integration (cook-before-pack, manifest-hash staleness) | 0.5 d |
| 10 | Test suite: golden battery, seam fuzz, determinism byte-compare | 2 d |
| 11 | Docs (KNOWLEDGE.md conventions, this doc → final) | 0.5 d |

Total: 16 d ≈ 3 weeks with slack.

## Resolved review questions (decided 2026-07-17)

1. **`CHUNK_SIZE = 64 m`, shared with interest cells** — one grid aligns M8
   interest management and collision streaming; M0 validated 64 m density.
   Single constant, cheap to change later if profiling demands it.
2. **Duplication at borders, not clipping** — clipping creates new border
   vertices (T-junction/crack risk) and a hairier cooker; few % file size is
   acceptable.
3. **`StaticCollision` is opt-in** — explicit marking is trivial during M3
   greybox authoring; opt-out would silently cook every decorative mesh.
   Opt-in fails loud (fall through floor → obvious cause).
4. **Editor cook action ships in v1** — the library entry point exists
   anyway; M3's edit→cook→walk iteration loop needs it immediately.

---

## Implementation notes (final, 2026-07-17)

Landed as planned, with two deviations worth recording:

1. **Shape-casts widen to f64** (`parry3d-f64`, same `=0.20.2` pin). parry's
   f32 GJK terminates at ~1e-3 *relative* error (`gjk::eps_tol`), which blew
   the 1 mm/0.1° battery tolerances and — worse — mis-ordered face-vs-edge
   contacts at triangle seams (a flat-ground cast could return a tilted edge
   normal from the adjacent triangle). Per-triangle casts now run in f64 via
   `widen_shape`/`cast_triangle_f64` in `game_shared::collision::store`;
   unmapped shape types fall back to f32. f64 arithmetic is
   IEEE-deterministic on x86-64 and wasm32, so parity is preserved. Cooked
   chunks and raycasts (per-triangle analytic, precise in f32) are unchanged.
2. **Staleness uses a `scene_hash` in the manifest** (fnv1a of the source
   `.scene` bytes) checked cook-side only, alongside `format_version` and
   `COOKER_VERSION_HASH`. It doesn't cover referenced mesh assets;
   `--force` re-cooks.

Test artifacts: `game_shared/tests/data/collision/` holds the canonical
`.ccol` chunk set + `battery.ron` (12 analytic cases); a drift-guard test
byte-compares them against the generator, and a `regenerate` `#[ignore]` test
rewrites them after intentional changes. Seam fuzz runs 400 randomized casts
across the x=64 border. M6 reruns these exact files in server WASM.
