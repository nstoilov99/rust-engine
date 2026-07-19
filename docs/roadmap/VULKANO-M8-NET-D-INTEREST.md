# M8 — Net-D: Interest Management & Load

**Status:** 📝 Draft (2026-07-20) — gpt-5.6-Sol/Codex review reconciled
(9 findings folded in); open questions ruled 2026-07-20 (3×3/7×7 rings,
2 m/1 s coarse cadence, no far labels, 50 bots/process).

Replaces M5's crude quadrant-zone visibility with cell-indexed interest
management: subscriptions follow a hysteresis-stabilized anchor cell, with a
layered scope (near = full state, far = coarse position rows, out = nothing),
plus server write-rate hygiene so replication cost tracks what observers can
see. Closes with load tests that re-run the M0 scenario patterns through the
real `game_client_net` stack and compare against the M0 ceilings.

## Current state (verified 2026-07-20)

- **Interest today is 4 static quadrant zones.**
  `zone_id_from_position(x, y)` (game_shared/src/world_grid.rs:46) maps sign
  bits to zones SW/SE/NW/NE; doc comment says "Frozen for M5; manifest-driven
  server zones are future work". The cell grid M8 needs already exists:
  `CHUNK_SIZE = 64.0` with `chunk_coord` floor semantics
  (world_grid.rs:11-20), and the file header says "M8 reuses this grid".
- **Server** (`server/game_module/src/lib.rs`): `Player`, `Npc`,
  `Projectile`, `ActiveCast` all carry `zone_id: u32 #[index(btree)]`.
  `move_tick` (50 ms) rewrites the Player row only when state changed and
  recomputes `zone_id` per write; `dev_teleport` also recomputes it. `tick`
  (100 ms) rewrites every live NPC row — but NPCs *wander* every tick, so
  those writes are activity-bound, not waste; a changed-only guard is free
  insurance for future idle AI, not a load fix. **Projectiles never
  recompute `zone_id` after spawn** (fixed-at-spawn) — tolerable at
  quadrant scale, wrong at 64 m cells: a firebolt crosses a cell in ~2 s.
- `zone_id` is written at more sites than movement: player creation
  (lib.rs:468) and respawn (lib.rs:754), NPC seed/respawn (lib.rs:1053),
  projectile creation (lib.rs:976), cast start. The cell swap must cover
  all of them.
- **Client** (`game_client_net/src/client.rs:463-532`): one full-detail
  replication subscription per zone (`player`/`npc`/`projectile`/
  `active_cast WHERE zone_id = {zone}` + own `ability_cooldown`).
  `pump_zone_swap` swaps when the own row's zone changes — **no hysteresis**
  (standing on x=0 wiggling ±1 cm re-subscribes every crossing), one swap in
  flight, apply-new-then-drop-old overlap so shared rows never leave the
  cache. This overlap-swap machinery is exactly what M8 generalizes.
- Base subscription (client.rs:433-461) is identity-scoped and permanent:
  config, tombstones, own account/player/ping rows. Unaffected by M8.
- Replication client-side: cache-diff drives spawn/despawn; `InterpBuffer`
  per remote entity; generation-replace on live rows is unit-tested.
  Targeting (Tab) picks from replicated combat-capable entities.
- **M0 bot harness** lives in `E:\Projects\Rust\STDBStressTest` (spike repo,
  official SDK, not the engine stack). Bots: connect → subscribe own cell →
  10 Hz movement → cast every 3-6 s.

### Binding constraints (M0 report + ADRs)

- **Per-cell subscriber overlap ≲ 150 at full update rate** is the binding
  ceiling from the M0 GO decision (75 in one cell = negligible; 150 =
  recovers in 10-20 s; 300 = unbounded collapse). Density, not realm
  population, is the limit — interest layering exists to keep *full-rate*
  overlap under this number.
- Per-module uniform capacity ≈ 500 clients; delivery ceiling ~110-135 Mbps
  / ~100 k row-deliveries per second, software-bound (no WS batching #2784,
  full-scan subscription evaluation #5317, serialized per-connection send
  #2891). Consequences: (a) row *writes* are the cost unit — suppress
  no-change writes; (b) every subscription query in every client's set is
  evaluated per transaction — query-set size is a server-CPU cost, and the
  load test must measure it.
- Schema migration policy unchanged: schema-touching packages require
  `./publish.ps1 -Wipe`; timer inserts stay idempotent.
- Zones remain a *client streaming* concept (M4 WorldStreamer) — M8 removes
  them from the *net* path only.
- Multi-module sharding is **out of scope**: M0 ruled density the limit, so
  the slice ships one module + cell interest; the sharding report section is
  carried, not implemented.

## Design

### D1. Cell membership + hysteresis (`game_shared::world_grid`)

The 64 m collision grid becomes the interest grid — one grid, one set of
floor semantics.

- `cell_key(coord: IVec2) -> u64`: packed `(cx as u32 as u64) << 32 |
  (cy as u32 as u64)` — a single indexable equality column, negative coords
  round-trip via the u32 cast. Inverse `cell_coord(key)` for tooling.
- Hysteresis is an **anchor cell**, not a per-crossing debounce:
  `re_anchor(anchor: IVec2, pos: Vec2) -> Option<IVec2>` returns a new
  anchor only when `pos` is more than `INTEREST_HYSTERESIS_M = 8.0` outside
  the anchor cell's AABB (distance to the box, so corners behave). Walking
  a border oscillates position by centimeters → anchor never moves;
  committing to a neighbor cell by > 8 m re-anchors once. Pure functions,
  unit-tested (border wiggle produces zero re-anchors; diagonal corner
  crossing produces exactly one).
- Interest rings derived from the anchor: `near_cells(anchor)` = 3×3
  (worst case ≥ 64 m of full detail ahead of an edge-standing player —
  covers the 25 m max ability range with margin), `far_cells(anchor)` =
  7×7 minus the 3×3 ring (40 cells, coarse awareness out to ~224 m).

**Schema swap (server):** `zone_id: u32` → `cell_id: u64 #[index(btree)]`
on `Player`, `Npc`, `Projectile`, `ActiveCast`; written at **every** site
that writes `zone_id` today (`move_tick`, `dev_teleport`, NPC tick, cast
start, player create/respawn, NPC seed/respawn, projectile create) —
**plus** projectile stepping, which must now recompute `cell_id` per step
(new behavior; today projectiles keep their spawn zone forever). Requires
wipe-publish. Client `EntityState` carries `cell_id` through unchanged
plumbing.

### D2. Layered subscriptions (client)

`pump_zone_swap` generalizes to `pump_interest`: anchored at a cell, one
swap in flight, same applied-then-drop overlap.

- **Near (full)**: per near cell, `player`/`npc`/`projectile`/`active_cast
  WHERE cell_id = {key}` — 9 × 4 = 36 equality queries, plus own
  `ability_cooldown` (37).
- **Far (coarse)**: `entity_coarse WHERE cell_id = {key}` for the 40 ring
  cells (D3). Total set ≈ 77 equality queries, each hitting a btree column
  — a deliberately large set (7×7 ruled over 5×5 for awareness range), so
  the D5 query-set-cost measurement and the range-predicate fallback below
  are load-bearing, not hypothetical. If the harness shows the set binds,
  the fallback (or dropping far to 5×5) is the recorded package-5 decision.
- **Out**: nothing — rows leave the cache on swap exactly like today's zone
  drops (out-of-scope removal, no tombstone).
- Swap trigger is the **predicted own position** fed through `re_anchor`
  each poll (the own row is always subscribed, but prediction leads it —
  anchoring on prediction avoids a subscribe lag spike right as you sprint
  into a new cell). Plumbing this requires a trait change: prediction lives
  in `game_client` and `NetClient::poll` takes only the event vector — add
  `set_interest_hint(pos: [f32; 2])` to `game_shared::net::traits::
  NetClient` (no-op default), called each frame from `game_client/src/
  net.rs` with the predicted position, falling back to the own-row position
  until prediction seeds. Package 2 includes the trait, the STDB impl, and
  test-double updates.
- **Swaps are not atomic** (and never were): the current promote path
  activates the new handle on `on_applied`, then calls the *asynchronous*
  `unsubscribe` on the old — old-set-only rows linger until the
  unsubscription applies. At quadrant scale this was invisible; with
  near/coarse tiers overlapping, an entity can transiently exist in both
  tiers' tables. The client contract is therefore **precedence, not
  exclusion**: full-tier state always wins while present, coarse fills in
  otherwise (D3), and transient dual membership is a supported state — not
  a race to eliminate.
- Query-set size is the acknowledged risk (#5317 full-scan eval): the D5
  harness measures server CPU vs. set size. Fallback if it binds: split
  `cell_x: i32` / `cell_y: i32` columns with compound range predicates
  (`WHERE cell_x >= a AND cell_x <= b AND cell_y >= c AND cell_y <= d` —
  the STDB SQL reference supports compound `>=`/`<=` conjunctions;
  `BETWEEN` is **not** in the grammar), one query per table per tier.
  Verify index use against the running instance before adopting; the
  packed-key equality form is the guaranteed-indexed baseline.

### D3. Coarse tier (`entity_coarse` table)

STDB replicates whole rows, so "position/name only" means a second, smaller
table — not a projection.

- Public table `EntityCoarse { entity_id: u64 PK, kind: u32,
  generation: u32, cell_id: u64 #[index(btree)], x: f32, y: f32, z: f32,
  updated_micros: i64 }`. Generation included so tombstone/replace semantics
  match the full tier when an entity hands off between rings. No
  projectiles (short-lived, near-only by nature), no combat fields.
- **Server update policy** (the write-rate contract): upsert from
  `move_tick`/`tick` only when the entity changed cell **or** moved ≥ 2 m
  since its last coarse write; hard cap one coarse write per entity per
  second. Idle entities cost zero coarse deliveries. Movement is not the
  only writer: **death deletes the coarse row, respawn re-upserts it with
  the bumped generation** (far observers see corpses vanish — acceptable
  for the slice; without this, a movement-only policy leaves a dead entity
  standing at full health in the far ring indefinitely). Row also deleted
  with the entity itself.
- **Client**: this is new plumbing, not a reuse — `WorldSnapshot` is built
  from the player/NPC tables only and the replication replace path triggers
  on generation mismatch, so coarse rows are invisible to it today.
  Package 3 extends the snapshot with a coarse entity list and teaches the
  cache-diff tier precedence: full row present → full proxy (coarse row
  ignored); coarse only → light proxy (existing dummy/capsule mesh, no
  `InterpBuffer` — lerp between the last two coarse samples over their
  timestamp gap, which reads as slow drift at distance); neither → despawn.
  Tier transitions reuse the entity's GUID (same entity_id + generation) so
  the rendered entity persists across the hand-off. Coarse proxies are
  **not targetable** and draw no HUD frames.

### D4. Server write hygiene

- NPC `tick`: add the changed-only guard `move_tick` already has — but as
  cheap insurance, not a load fix: today's NPCs wander every tick, so their
  writes are activity-bound by design. NPC wander load is part of the
  measured baseline, not something hygiene can remove. The idle-delivery
  acceptance target applies to *players and the coarse tier* (an idle
  player population must produce zero replication deliveries).
- Audit remaining unconditional writers (`Projectile.last_update_micros`
  every step is fine — projectiles move by definition; anything else that
  rewrites unchanged rows gets the guard).
- Counters for the load test: per-tick rows-written by table, in a **new
  private-by-default `metrics` table that only the harness subscribes to**
  — not `config`, which is in every client's permanent base subscription
  and would broadcast a counter write to the whole population each tick,
  defeating the idle-delivery goal it measures.

### D5. Load harness through the engine stack

M0 measured the *SDK*; M8 must measure *our* client path — connection,
subscription churn, cache-diff, prediction input stream.

- New bin crate `tools/net_bots`: N headless bots per process, each a
  `game_client_net::Client` polled in a loop (the exploit battery already
  proves the crate runs headless). Bots: connect → wander at 20 Hz input
  (through the real epoch/seq rules) → cast every 3-6 s → report RTT via
  the existing `ping_result` path plus client-side row-delivery and
  swap counts.
- **Scenarios** (M0 patterns + one new):
  1. *Uniform 300* — pass: p50/p95 RTT within 1.5× of M0 rung 1
     (26 ms / ≤ 50 ms), server steady.
  2. *Hotspot* — 75 then 150 bots converging on one cell; pass mirrors M0
     (75 negligible, 150 recovers ≤ 20 s after dispersal). 300-in-cell is
     expected to fail; run it once to confirm the ceiling didn't silently
     move.
  3. *Churn* — connect/disconnect cycling; pass: no leak in module memory,
     tombstone GC keeps up.
  4. *Border thrash* (new, validates D1) — bots pacing across a cell
     border; pass: re-anchor rate per bot ≈ walking cadence, not oscillation
     rate; zero subscription-error disconnects.
- Results written to `docs/roadmap/M8-LOAD-REPORT.md` against the M0
  numbers, including the query-set-size CPU measurement (D2 risk) and the
  coarse-tier delivery ratio (far ring should cost ≪ full-tier
  deliveries/entity). This report is the "sharding strategy as far as the
  slice needs" artifact: it either confirms one module suffices for the
  slice or states the observed limit.

## Packages (one commit each)

1. **Shared cell membership** — `cell_key`/`cell_coord`/`re_anchor`/ring
   helpers + unit tests; schema swap `zone_id` → `cell_id` on all four
   tables at every write site (including projectile per-step recompute,
   spawn/respawn/seed paths); wipe-publish. Client compiles by treating
   `cell_id` of the own row as the (single) subscribed key — behavior
   parity with today at cell granularity.
2. **Near tier + hysteresis** — `NetClient::set_interest_hint` trait
   plumbing (prediction → anchor), `pump_interest` with 3×3 full-detail
   set, swap-overlap and dual-membership precedence contract;
   border-wiggle integration test (two clients, one pacing a border: no
   thrash, no entity pops).
3. **Coarse tier** — `entity_coarse` table + server update policy
   (movement, death-delete, respawn-upsert) + far ring subscription +
   snapshot/cache-diff extension with tier precedence + client coarse
   proxy path; hand-off test (walk a remote entity far → near → far, no
   duplicate or orphaned proxy; kill it in the far ring, corpse vanishes).
4. **Write hygiene + counters** — NPC changed-only guard, writer audit,
   harness-only `metrics` table; assert idle *player* population ≈ zero
   deliveries.
5. **Load harness + report** — `tools/net_bots`, four scenarios,
   `M8-LOAD-REPORT.md` vs M0 baselines; D2 fallback decision recorded.
6. **Close-out** — roadmap/CLAUDE.md status, doc marked complete.

## Ruled decisions (2026-07-20)

1. Ring sizes: **near 3×3 / far 7×7** — awareness range preferred; the
   77-query set cost is measured in package 5, with range-predicate
   fallback or 5×5 retreat as the recorded mitigation if it binds.
2. Coarse cadence: **2 m moved or cell change, ≤ 1 write/s per entity**.
3. Far-tier proxies: **no name labels** — kind-colored proxy mesh only;
   nameplates are future HUD work with no other consumer yet.
4. `tools/net_bots`: **50 bots per process** (SDK panics on dead WS
   senders → bounded blast radius, mirroring M0's catch_unwind
   experience); scenario scripts spawn multiple processes.
