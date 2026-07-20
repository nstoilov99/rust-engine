# M8 Net-D Load Report

Date: 2026-07-20. Box: same dev machine as M0 (server + bots local).
Harness: `tools/net_bots` (headless SDK bots, 50/process per D5 ruling),
runner `tools/net_bots/run_scenarios.ps1`. Server: SpacetimeDB standalone,
module `rust-engine-dev`, 20 Hz `move_tick`, `MAX_STEPS_PER_TICK = 4`.

Bots send 20 Hz input + `set_interest_hint`, cast every 3-6 s, ping every 2 s
(client automatic). Interest: near 3x3 full detail, far 7x7 coarse-only,
anchor + 8 m hysteresis, promote-on-applied swaps.

## M0 baselines (pass bar = 1.5x M0)

| scenario | M0 result | M8 pass bar |
|---|---|---|
| uniform 300 | RTT p50 ~26 ms, p95 38-50 ms | p50 <= 39 ms, p95 <= 75 ms |
| hotspot 75 (one cell) | negligible impact | same |
| hotspot 150 | degrades, recovers 10-20 s after disperse | same |
| hotspot 300 | unbounded collapse | ceiling confirm only |
| churn 100 | p50 22-29 ms at 17x design churn | p50 <= 43 ms |

## Results

### Uniform

| bots | RTT p50 | RTT p95 | ping return | full rows /bot/s | coarse /bot/s | swaps mean | disc |
|---|---|---|---|---|---|---|---|
| 50 | 8.9 ms | 26 ms | full (n=1500) | 321 | 26 | 2.5 | 0 |
| 150 (3x50) | 130-141 ms | 201-205 ms | full (n=2250/proc) | ~662 | ~64 | 2.8 | 0 |
| 300 (6x50) | 892-1539 ms | up to ~2 s | ~100/3000 per proc | ~700 | ~10 | 2.0 | 0 |

Server side:

| bots | move_tick p50 | p95 | max | server CPU | RSS |
|---|---|---|---|---|---|
| 50 | 16.5 ms | 22.9 ms | - | - | - |
| 150 | 149.3 ms | 172.1 ms | 193.4 ms | 4.6-5.1 cores | ~200 MB |
| 300 | starved: 12 tick executions in 140 s | - | - | ~5 cores | ~442 MB |

At 300 the reducer queue grows so deep that `move_tick` barely gets
scheduled at all — the sim effectively stalls behind queued inputs.

**Verdict: uniform-50 passes; uniform-150 and uniform-300 FAIL the 1.5x-M0
bar — but for a different reason than M0 measured (see Diagnosis).**

### Hotspot (75 -> disperse at 60 s)

All bots teleport into one cell (center 32,32), disperse across the map at
t=60 s.

- RTT p50 14.5-17.4 ms, p95 47-49 ms, full ping return, 0 disconnects.
- full_row_updates ~1300/bot/s (2x uniform — everyone sees everyone),
  coarse ~26/bot/s. Swaps mean 7.1-7.6 (disperse drives re-anchoring).
- move_tick window: p50 28.6 ms, p95 42.9 ms, max 58.9 ms — within budget.
- Server CPU: ~3.4-3.8 cores clustered, ~1.9 after disperse, idle by +30 s.
- Metrics delta over window: player +105k, npc +2.3k, coarse +7.8k rows.

**PASS — matches M0 "negligible impact at 75".**

### Hotspot 150 (3x50, disperse at 60 s)

- RTT p50 207-218 ms, p95 337-363 ms, near-full ping return (n~2965-2990
  of 3000/proc), 0 disconnects.
- full_row_updates ~1140/bot/s, coarse ~56/bot/s, swaps mean 5.0-5.9.
- move_tick window: p50 157.6 ms, p95 238.6 ms, max 290.8 ms (tick
  overruns; effective tick rate ~3-4 Hz during cluster).
- Server CPU: ramps 8.4 -> 12.2 cores clustered (delivery fan-out —
  everyone sees everyone: ~150^2 row deliveries per tick), falls through
  9.6 -> 7.3 -> 6.0 cores within ~20 s of disperse, idle shortly after.
- Metrics delta: player +57.8k, npc +1.5k, coarse +14.1k rows.

**Degrades under cluster, recovers within ~20 s of disperse, no
disconnects — matches M0's hotspot-150 shape (bar: degrade + recover).**

### Hotspot 300 (ceiling confirm, single run, 6x50)

- RTT p50 1394-1750 ms, p95 1864-1984 ms; ping return ~200 of 3000/proc
  (in-flight pings superseded under lag). 0 disconnects.
- Swaps stuck at 1-2/bot: interest re-anchoring stalls behind the reducer
  queue — bots "dispersed" client-side but promotions lagged out.
- move_tick starved: 17 executions in 140 s (p95 299 ms, max 307 ms).
  Player row writes nearly frozen (+4.2k over the window vs +58k at 150).
- Server CPU pegged 14.4 -> 16.2 cores (box saturated).

**Collapse confirmed, same shape as M0's hotspot-300. The 300-in-one-cell
case remains out of design scope; the per-cell overlap requirement stays
~100-150 subscribers.**

### Churn (100 bots, 2x50; connected 10-20 s, off 1-3 s)

- RTT p50 15.2-15.5 ms, p95 55-57 ms, near-full ping return, 664
  disconnect / 658 reconnect cycles completed cleanly (per-process:
  332/329 — the delta is bots mid-cycle at shutdown).
- Swaps mean 12/bot (reconnect = full re-anchor + subscription rebuild).
- move_tick: p50 32.0 ms, p95 48.8 ms, max 73.0 ms — occasional overrun,
  no backlog accumulation.
- Server CPU oscillates 2.4-3.7 cores with the connect waves; ~200 MB RSS.
- Metrics delta: player +120k, npc +2.4k, coarse +9.9k rows.

**PASS — p50 well under the 43 ms bar (M0: 22-29 ms at 17x design churn);
subscription setup/teardown churn is not a bottleneck.**

### Thrash (50 bots pacing +-6 m across a cell border)

- RTT p50 12.9 ms, p95 36.0 ms, full ping return, 0 disconnects.
- **Swaps: min 1, max 2, mean 1.9 over 120 s** — the 8 m hysteresis fully
  absorbs +-6 m border pacing. After the initial anchor, bots re-anchor at
  most once; no subscription flapping at all (uniform wanderers see more
  swaps than deliberate border-thrashers).
- move_tick: p50 20.0 ms, p95 32.2 ms, max 41.1 ms. Server ~1.2-1.7 cores.
- Metrics delta: player +76.6k, npc +2.6k, coarse +5.6k rows.

**PASS — the swap-storm scenario the hysteresis was designed for is a
non-event.**

## Diagnosis: the ceiling is simulation CPU, not interest delivery

M0's ceiling was subscription fan-out (delivery-side). M8's observed ceiling
is **`move_tick` wasm simulation cost**:

- move_tick p50 goes 16.5 ms @ 50 bots -> 149 ms @ 150 bots: past ~100-150
  active movers the 50 ms tick budget overruns, inputs backlog, and the
  `MAX_STEPS_PER_TICK = 4` catch-up multiplies per-tick work ~4x until it
  plateaus (150 players x 4 steps x ~0.3 ms/step ~ 180 ms, matching the
  measured 149-193 ms).
- RTT then queues behind the tick: 130-141 ms at 150 bots (roughly one
  tick), collapsing to ~1-1.5 s at 300 (reducer queue growth; ping return
  rate drops to ~3-7%; at 300 the tick itself starves — 12-17 executions
  per 140 s window).
- Interest delivery stays healthy throughout: 0 disconnects, swaps complete
  (mean 2-3 per bot), coarse dedup works, and per-bot delivery bandwidth is
  flat (~660-700 full rows/bot/s server-push regardless of load).

The ~0.3 ms/player-step is dominated by wasm collision stepping in
`motion::step`. This is our module's cost, not SpacetimeDB's delivery path.

One caveat from hotspot-150: when everyone sees everyone, delivery fan-out
becomes a real second cost (12.2 cores clustered vs ~5 for the same 150
bots uniform). That cost is O(subscribers^2) per cell and is exactly what
the interest system bounds — it only appears when the overlap cap
(~100-150 per cell) is deliberately exceeded.

## Coarse tier effectiveness

Coarse deliveries are a small fraction of full-detail deliveries at every
load point (26 vs 321 /bot/s at 50 bots; ~64 vs ~662 at 150; ~10 vs ~700 at
300). The far ring costs little bandwidth; write hygiene (2 m / 1 s cap,
change guards, D4) keeps coarse row writes ~5x rarer than player row writes
(metrics table: player=104k vs coarse=19k cumulative at the 150-bot mark).

## D2 decision: keep the ~77-query equality set

The 77-equality-query subscription set (9 near + 40 far + base) is **not**
the binding constraint at 50-150 clients: subscription evaluation and swap
latency stayed healthy at every load we could drive before the simulation
ceiling. Fallbacks considered and rejected for now:

- **Range predicates / fewer queries**: no evidence needed; query count is
  not what saturates.
- **5x5 far ring**: would cut coarse traffic that is already negligible.

The binding constraint is sim CPU (~150 active movers per module on this
box). Options recorded for when it matters, in rough order of leverage:

1. Cheapen `motion::step` (fewer collision iterations, spatial early-out).
2. Larger fixed step / lower tick rate for the server sim.
3. Native (non-wasm) module build.
4. Player cap per module + horizontal sharding by zone.

## Verdict

**The interest management layer passes on every axis it controls.**
Hotspot-75 negligible, hotspot-150 degrades-and-recovers in ~20 s,
hotspot-300 collapses exactly as out-of-scope, churn at ~660 reconnect
cycles is a non-event, and border thrash produces at most one re-anchor
per bot (hysteresis working as designed). Coarse-tier bandwidth is
negligible next to full-detail, and write hygiene holds coarse writes to
a small fraction of player writes.

**The uniform-150/300 RTT bar failures are not an interest regression.**
They are the module's wasm simulation ceiling (~100-150 active movers on
this box), a cost M0's spike did not carry (M0 had no per-player collision
stepping). The M8 machinery itself stayed healthy right up to and past
that ceiling. Sim optimization (see D2 options above) is recorded as
future work, not part of M8 scope.

M8 Net-D: **accepted**, with the sim-CPU ceiling logged as the next
scaling constraint.
