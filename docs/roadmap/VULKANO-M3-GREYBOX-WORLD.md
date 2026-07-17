# Task M3: Greybox World v1

**Status**: ✅ Complete (2026-07-17). Implemented as planned with defaults for the open
questions (45°/0.4 m/3 m controller constants, 512×512 m world, silent auto-cook, no
per-zone tint). Load baseline (release): 81 chunks in ~31 ms
(`engine/tests/collision_load.rs`). Staleness hash now folds referenced mesh-file bytes;
Build dialog cooks stale scenes before packing; editor auto-cooks on save.
**Duration**: ~1.5–2 weeks
**Prerequisites**: M2 (✅ 2026-07-17)
**Related**: M4 (streams the cells this task defines), M5 (replicates entities in it), M6 (kinematic controller walks it), M8 (interest cells share the grid), Task 46 (real terrain replaces the proxy), Task 52 (prefab links — deliberately NOT used here, see decisions)

---

## Goal

A partitioned test world, **ugly on purpose**: grid-partitioned heightfield
greybox terrain plus a traversal gym, big enough to span multiple zones and
cells so M4/M5/M8 have something real to stream, replicate, and subscribe to.
Produced entirely offline by a deterministic generator; consumed by the
existing, unmodified render/scene/collision pipelines.

**Non-goals:** real terrain (Task 46), streaming (M4 — but the manifest this
task emits is what M4 streams), any new runtime system, prefab links for
generated content (Task 52; M3 content is disposable by design), heightfield
collision section (chunks stay trimesh per the M2 format decision).

---

## Architecture decisions

### 1. Offline generation — the generator is a content compiler

`tools/greybox_gen` (CLI, engine dependency like `collision_cooker`) emits
ordinary content: RMSH `.mesh` assets (via `mesh_import::write_mesh_binary`),
one `.scene`, one world manifest. Then the existing collision cook runs.
Runtime knows nothing about "greybox" — no procedural terrain system, no
special loaders, nothing for M5/M6 determinism to worry about.

- Deterministic: fixed seed + generator version + parameters baked into the
  tool; identical inputs → byte-identical outputs.
- `--check` mode: regenerate in memory, byte-compare against the checked-in
  files, non-zero exit on drift (CI + export precondition; mirrors the M2
  golden-battery drift guard).

### 2. World layout: 8×8 cells centered on the origin

Cells `(-4..4) × (-4..4)` on the existing 64 m grid → 512×512 m. Centering on
the origin exercises negative-coordinate floor semantics in `world_grid` and
the cooker — an off-by-one class M8 interest math will hit eventually; better
to live on it from day one.

Four zones of 4×4 cells (the origin quadrants). Zones exist only as manifest
metadata in M3 — enough for M4 transfer tests; M8 operates on individual
cells.

### 3. Terrain sampling: integer global grid, bit-identical seams

Height function is sampled at **integer global grid coordinates** (2 m
spacing), never per-cell floating-point offsets — shared border vertices of
adjacent cells are computed from the same integers and are bit-identical by
construction. This is the same seam philosophy as M2's border duplication:
correctness by construction, not by epsilon.

- Analytic height function (sum of a few sines at different frequencies +
  seeded per-region amplitude), gentle overall (< ~20° slopes) so ordinary
  terrain never fights the controller.
- Flat plateaus (height forced to a constant over rectangular regions) where
  the gym and spawn areas sit, and along zone borders where M4/M5 test routes
  cross.
- One 33×33-vertex mesh per cell (2048 tris; ~131 k world). 2 m resolution is
  fine for landscape and streaming; it is deliberately NOT the precision
  instrument — the gym is.

### 4. Per-cell terrain entities, generated assets checked in

One entity per cell: `MeshRenderer` (`models/greybox/cell_<x>_<y>.mesh`) +
`StaticCollision` + identity transform (heights are baked into the mesh —
world-space cell placement is baked in too, so transforms stay identity and
cooked chunks are exactly cell-aligned). The generator marks `StaticCollision`
itself — generated content marks itself; nobody hand-clicks 64 cells.

Meshes are written in the importer's Y-up render-local convention (the
generator applies the Z-up→Y-up conversion once, exactly like the importer;
heights are Z in game space). The M2 accessor then round-trips them correctly.

All generated output is **checked in**: `content/models/greybox/*.mesh`, the
scene, the manifest, and the cooked `.ccol` chunks. Export/CI never
regenerates content — packaging must not mutate `content/`. CI runs
`greybox_gen --check` + the cook staleness check instead.

Entity GUIDs are derived deterministically from (seed, cell coord) /
(seed, station id) — regeneration preserves identity, which M4/M5 need for
stable spawn/despawn and replication.

### 5. World manifest: separate file, minimal shape

`content/world/greybox.world.ron` — NOT inside the scene (the scene format
stays `{version, name, entities}`; cell ownership is orchestration metadata,
and M4 must be able to read it without loading a scene):

- `version`, `scene` (relative path), `cell_size`
- `cells`: coord, `zone_id`, `root_entity_guid`, `mesh` (relative path),
  `collision_chunk` (relative path)
- `zones`: id, member cell coords
- `spawn_points`: name + position (gym stations, zone-border routes,
  default player spawn)

Bounds are derived from coord × cell_size — not stored. Mesh/chunk paths are
technically derivable from naming conventions, but explicit dependencies beat
convention-knowledge for M4's loader; they stay in.

### 6. Traversal gym: analytically precise, threshold pairs

The gym sits on flat plateaus and is built from scaled/rotated
`__primitive__/Cube` entities with `StaticCollision` (full-transform cook path
— supports scale, unlike physics-collider primitives). All dimensions dyadic
where possible (M2 battery discipline: exact in f32).

The controller's limits are not fixed until M6; M3 builds against
**provisional constants, written in the manifest as metadata** so M6 reads
them instead of re-inventing: slope limit 45°, step height 0.4 m, jump gap
reference 3 m. Every threshold gets a **pair straddling the limit**:

- Slope array: 30°, 40°, **44°, 46°**, 50°, 60° ramps (walk vs slide)
- Step arrays: ascending AND descending runs, 0.2 / 0.3 / **0.35, 0.45** /
  0.6 m risers
- Gap array: 1 / 2 / **2.75, 3.25** / 4 m, flat and with landing slopes
- Ramp tower: 3 stories, ramps + platforms, long-fall drop edge (gravity,
  fall damage later), low-ceiling passage (head clearance), narrow ledge
- Corner block: inside and outside corners, a long wall for slide-along
- Seam routes: at least one station straddling a cell border and one
  straddling a zone border (collision + streaming + replication all cross
  seams on the same geometry M2's seam fuzz validated)

Every station gets a named spawn point next to it (manifest `spawn_points`) —
M6 test harness teleports between stations instead of walking minutes of
terrain.

### 7. Pipeline hardening (small, load-bearing, found in review)

- **Staleness hash gap**: the M2 `scene_hash` covers scene bytes only. The
  generator's common iteration (tweak height function → meshes change, scene
  byte-identical) would silently skip re-cooking. Fix in M3: cook staleness
  additionally hashes the **referenced mesh file bytes** (fold each
  `StaticCollision` entity's mesh file into a `content_hash` in the
  manifest). `--force` stays as the escape hatch.
- **Editor Build dialog doesn't cook**: `build_dialog.rs` packs `content/`
  raw. The export *scripts* cook (M2 item 9); the in-editor build must too —
  run the same cook-before-pack step, fail the build on cook errors.
- **Auto-cook on scene save** (editor): staleness makes it nearly free; kills
  the manual File → Cook step for the M3 edit→cook→walk loop.
- Noted, not fixed: collision output namespaces by scene file stem only
  (`scenes/a/x.scene` and `scenes/b/x.scene` would collide). All scenes live
  flat in `scenes/` today; revisit when that changes.

### 8. Recorded decision: no prefab links for M3 content

Scenes bake prefab instances flat (no `prefab_source` serialized — verified).
M3 greybox content therefore cannot be retro-linked when Task 52 lands. This
is accepted: the content is throwaway by design (Task 46 replaces terrain,
gym is regenerated at will). Do NOT add prefab-link schema now; Task 52
defines its own link shape. This paragraph exists so it's a decision, not a
surprise.

---

## Performance baseline (for M4, not a target for M3)

M3 loads everything at once: 64 mesh loads, 64 chunk BVH builds at scene
load. That is M4's "before" measurement. M3 adds a single log line at scene
load — cell count, chunk count, total load ms — so M4 has a number to beat.
Unique-mesh-per-cell (no instancing) is an intentional streaming stress case.

---

## Tests

1. **Generator determinism**: `--check` (regenerate, byte-compare all
   outputs) — also run twice in CI.
2. **Seam continuity**: unit test — adjacent cells' shared border vertices
   bit-identical (assert on generated vertex data, both axes + corners).
3. **Cook integration**: generated scene cooks with zero warnings; every
   cell yields its expected chunk; chunk set matches the manifest.
4. **Manifest validity**: cells ↔ scene GUIDs one-to-one, zones partition
   the cell set, spawn points on solid ground (raycast down hits within
   1 m).
5. **Walkthrough** (manual): editor open, collision debug draw on, raycast
   probe on each gym station, fly a seam route.

---

## Work breakdown (~9 d)

| # | Work | Est |
|---|------|-----|
| 1 | Height sampling (integer-grid, seeded) + per-cell RMSH mesh emission (Y-up conversion, plateaus) | 2 d |
| 2 | Scene + manifest emission: cell entities, deterministic GUIDs, zones, spawn points | 1 d |
| 3 | Traversal gym stations (threshold pairs, tower, corners, seam routes) | 2 d |
| 4 | `greybox_gen` CLI: seed/params/version, `--check`, determinism + seam tests | 1 d |
| 5 | Pipeline hardening: mesh-content staleness hash, Build-dialog cook, auto-cook on save | 1.5 d |
| 6 | Generate + cook the world, check in, load-time baseline log, walkthrough, docs | 1.5 d |

---

## Open questions (for user review)

1. Provisional controller constants (45° / 0.4 m / 3 m) — fine as the values
   the gym is built around, or do you want different targets before geometry
   is generated around them?
2. World size 512×512 m / 4 zones — enough for M4/M8 testing, or go bigger
   now (cost is linear: cells × 2048 tris)?
3. Auto-cook on save: silent (console line only) or a status-bar indicator?
4. Any visual niceties worth the time (per-zone tint material so zone borders
   are visible in-editor), or keep it fully ugly?
