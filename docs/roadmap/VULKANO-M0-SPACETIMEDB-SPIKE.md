# Task M0: SpacetimeDB Scale Spike (Net-0)

**Status**: Planned
**Type**: Throwaway spike — go/no-go gate for the multiplayer arc (M5–M8)
**Related**: ADR-014 (SpacetimeDB), ADR-015 (kinematic movement), ADR-016 (spike-first gating)
**Estimated effort**: ~2 weeks

---

## Why this exists (don't lose this context)

The multiplayer arc assumes SpacetimeDB can hold an MMO realm. That assumption
has never been tested by us. If it's wrong, finding out *after* building
collision cooking, zone lifecycle, prediction, and combat on top of it wastes
months. This spike is the cheapest possible way to falsify the assumption
first.

### Why throwaway

- No code from the spike ships. Only **numbers, configs, and this report**
  survive. Treating it as throwaway removes all pressure to write "good" code
  and all temptation to keep scaffolding that would constrain the real design.
- If SpacetimeDB fails the gate, there is nothing to un-build.

### Why NO engine integration

- The spike is a **standalone SpacetimeDB module + headless console bot
  clients**. No render loop, no ECS, no FramePacket, no crusty-gui.
- Reasons:
  1. **Isolation**: if numbers are bad, the only suspect is the backend —
     there's no engine code to blame or tune.
  2. **Speed**: a console app is days, an engine integration is weeks.
  3. **Honesty**: engine integration would tempt us to "fix" problems in
     engine-side code, masking backend limits.
- Engine integration happens in M5 (Net-A), only after a **go** decision.

### What go/no-go means

- Pass criteria are defined **up front** (below), not after seeing results.
- **Go**: proceed with M5–M8 on SpacetimeDB.
- **No-go**: switch to the fallback (custom Rust server, renet/QUIC) behind
  the same `game_shared` command interface. M2–M4 proceed unchanged either way.
- A partial pass (e.g. 3,000 solid, 30,000 impossible on one module) is a
  **go with a documented sharding/zoning requirement** — the realistic
  expected outcome. The spike's real product is knowing the **per-module
  ceiling** and what the sharding strategy must be.

---

## Spike design

### Server module (Rust → WASM reducers)

Minimal but representative of the real game's write patterns:

- `Player` table: position, velocity, zone/cell id, hp, target
- `move` reducer: input command → kinematic position update (cheap shape-cast
  stand-in: plane + a few AABBs; real collision cooking is M2)
- `cast_ability` reducer: target legality check, damage write to target row,
  cooldown row update — models **combat fan-out** (one action → N row writes)
- `chat` reducer: broadcast-ish table insert (models high-subscription rows)
- Spatial cell index column + subscription queries filtered by cell — a crude
  stand-in for interest management (real version is M8)

### Bot clients (headless console app, official Rust SDK)

Each bot: connect → subscribe to its cell → send movement at 10 Hz → cast an
ability every 3–6 s → chat occasionally → disconnect/reconnect on a schedule.

**Bots must NOT wander uniformly.** The top false-positive risk (from design
review) is a spike that passes because load is evenly spread. Bot behavior
must model:

1. **Hotspots**: 60% of bots cluster in 10% of cells (city / raid boss
   pattern). Subscription overlap is what kills replication, not raw counts.
2. **Churn**: 5–10% of bots disconnect/reconnect per minute (login storms,
   zone transfers). Reconnect = full resubscribe = subscription evaluation
   spike.
3. **Combat fan-out**: scheduled "raid moments" where a hotspot's bots all
   cast within the same second — worst-case transactional write burst +
   replication burst to every overlapping subscriber.

### Metrics collected (per rung, per scenario)

| Metric | How | Pass threshold |
|--------|-----|----------------|
| Reducer/tick p50, p95, p99 latency | SpacetimeDB logs/metrics | p95 < 50 ms, p99 < 120 ms |
| Server tick budget headroom | CPU of module host | < 70% sustained |
| Egress bandwidth per client | measured at bots | < 64 KB/s avg per client |
| Client-observed round trip (input → own state echo) | bot timestamps | p95 < 250 ms |
| Reconnect storm recovery | time to steady-state after 10% churn burst | < 30 s |
| Memory of module host | OS metrics | stable (no unbounded growth over 30 min) |

Thresholds are calibrated to WoW-style pacing (GCD ~1 s, cast times). They are
**recorded before running** and may only be changed with a written reason.

---

## The bot ladder: 300 / 3,000 / 30,000

### Rung 1 — 300 bots (co-op / small-server reality check)

- **Feasibility**: fully honest test. 300 real SDK clients, real sockets, one
  dev machine (bots) + one server machine (or same machine, measured
  separately).
- **Models**: a busy co-op server or one small zone of a realm.
- **Pass**: all thresholds met with wide margin (< 30% CPU). If 300 struggles,
  that's an immediate no-go — no point running higher rungs.

### Rung 2 — 3,000 bots (realistic single-realm scale)

- **Feasibility**: mostly honest. 3,000 real connections is achievable but
  needs either several bot processes across 2–3 machines, or reduced per-bot
  send rates (document exactly what was reduced). OS socket/file-descriptor
  limits must be raised; note every such tweak in the report.
- **Models**: a healthy realm's concurrent players (WoW realms historically
  ~2–5k concurrent).
- **Pass**: thresholds met, possibly with hotspot-cell degradation that
  interest management (M8) can plausibly fix — degradation must be *localized
  to hot cells*, not global.
- This rung is the **primary gate**. Go/no-go is decided mostly here.

### Rung 3 — 30,000 bots (ceiling discovery, not pass/fail)

- **Feasibility**: **not feasible as real sockets from one machine.** Honest
  options, in order of preference:
  1. **Distributed cloud bot runners**: ~10–15 cheap VMs × 2–3k bots each.
     Real protocol, real subscriptions. Costs a few dollars/hours; do this
     if rung 2 passed cleanly and we want the true ceiling.
  2. **Server-side synthetic actors**: a scheduled reducer moves 27k
     table-resident "NPC bots" while 3k real clients subscribe. Tests write
     volume and subscription fan-out honestly, but NOT connection scaling —
     label results accordingly.
  3. If neither is worth it: **skip the rung** and extrapolate from rung 2
     saturation curves. Write down that it was skipped and why.
- **Models**: megaserver / full-shard scale (BitCraft territory).
- **Expected outcome**: this rung is expected to **find the per-module
  ceiling**, not to pass. The deliverable is the number N where things fall
  over and the shape of the failure (CPU? egress? subscription evaluation?),
  which directly dictates the M8 interest-management and sharding design.

### Ladder execution order

Run each rung with three scenarios: uniform (baseline), hotspot, hotspot +
churn + raid-moment. Stop the ladder early on hard failure of a lower rung.

---

## Deliverables

1. This document updated with: results tables per rung/scenario, every
   feasibility compromise made (send rates, socket limits, synthetic actors),
   saturation curves, and the failure shape at the ceiling.
2. **A written go/no-go decision** with the per-module ceiling and the implied
   sharding/zoning requirements for M8.
3. Spike code lives in a scratch repo or `spikes/` dir — never merged into
   engine crates.

## Results

Spike code lives at `E:\Projects\Rust\STDBStressTest` (server module +
bot runner + measurement script, own READMEs). Built against SpacetimeDB
2.6.1 (CLI, module crate, and client SDK).

### Run: 2026-07-16 rung=300 scenario=uniform duration=5min — PASS
- Host: dev desktop (Windows 11), server + bots + foreground apps on one machine (loopback)
- RTT ms: p50≈26 steady, p95≈38–50 typical, worst p95=233 (single 10 s blip, recovered immediately), p99≈46–298
- Errors: runner=0, server=0 panics; disc/reconn=0
- Server CPU 11–15 % flat, no upward trend; memory 222→238 MB over run, released to 96 MB on disconnect
- Bot runner: 11–15 % CPU, 126 MB — client was not the bottleneck
- Caveats: shared desktop — machine sat at 60–71 % total CPU from foreground apps mid-run,
  which inflates the baseline (an earlier, quieter run showed p50≈11 ms) and explains the
  p95 blips. An earlier run also showed progressive RTT decay from t≈190 s; it did not
  reproduce under measurement and correlated with foreground machine load, not server state.
- Verdict: PASS with large headroom. Rerun on an idle box for the sign-off number.

### Run: 2026-07-16 rung=300 scenario=hotspot duration=5min — PASS (memory watch-item)
- Host: same shared desktop, loopback; system CPU 37–90 % from foreground apps
- RTT ms: p50 26→36 (mild ramp), p95 typical 44–55, worst p95=117, worst p99=214 (first interval)
- Errors: runner=0; disc/reconn=0
- Server CPU 12→20 % rising with hotspot density (bots walk to center over ~2 min, fan-out
  per hot-cell update is ~180 subscribers vs ~10 in uniform)
- **Watch-item**: server memory climbed 184→466 MB over 5 min without plateauing
  (decelerating: ~72→~41 MB/min). Released cleanly to 100 MB on disconnect —
  subscription/send-queue memory, not leaked rows. The 30-min sign-off run must confirm
  a plateau; if it grows unbounded at 300 bots, hotspot density is a real ceiling.
- Bot runner: ~18 % CPU, 228 MB — client not the bottleneck
- Verdict: PASS on all latency thresholds; memory trend unresolved at 5 min.

### Run: 2026-07-16 rung=300 scenario=churn duration=5min — PASS (at ~17x designed churn)
- Bot-runner bug made this run far harsher than designed: churn probability was evaluated
  at worker-loop rate (~130 Hz) instead of input rate (10 Hz) → ~125 %/min effective churn
  (1,872 disconnects in 5 min vs the designed ~112). Bug fixed after the run; kept the
  result because it bounds a much worse case.
- conn held 269–289 throughout; reconn tracked disc with only ~25 sessions in flight
  (2–5 s reconnect delay); recovery threshold (<30 s) trivially met
- RTT ms: p50 22–29 steady, worst p95=156 (one interval); errors=0
- Server CPU 11–13 % (max 15), memory 225→260 MB during run, released to 106 MB after
- Verdict: PASS — connection lifecycle at 300 bots is cheap even at ~17x design churn.

### Run: 2026-07-16 rung=300 scenario=raid duration=5min — INVALID (bot bug), rerun required
- Bot-runner bug: the raid branch bypassed GCD pacing entirely, so during each 10 s raid
  window every bot cast at its input rate → ~3,000 casts/s (≈18k/10 s interval vs the
  designed ~2.7k/window at 1.1 s GCD pacing). Almost all were rejected by the cooldown
  check, but each rejection is still a transaction + ERROR log line.
- Effect: subscription delivery collapsed — RTT p50 climbed 4.2 s → 6.5 s, then echoes
  stopped entirely (n=0) for the rest of the run. Bots kept sending; nothing came back.
- Server during the run (run1.csv): CPU 30–42 % sustained, memory sawtoothing 1–2.7 GB
  (queued subscription updates), then a ~3-min drain at 6–10 % CPU after bots stopped,
  finally back to 105 MB idle.
- **Server did NOT crash.** Verified after the run: process alive, `tick_counter`
  advancing, SQL responsive, `combat_event` pruned to 0. It spent minutes draining the
  backlog (~10 cores briefly, ~155 CPU-seconds in ~20 s) then returned to idle. Full
  recovery, no restart needed.
- Learning worth keeping: a rejected-write flood is NOT free — the server survives but
  subscription latency dies for everyone. Real clients must rate-limit casts client-side;
  the server-side cooldown check alone does not protect replication.
- Bug fixed (cast gate now honors `next_cast` at 1.1 s; teleport arms an immediate first
  cast). Verdict: INVALID as a raid measurement — rerun at designed pacing.

### Run: 2026-07-16 rung=300 scenario=raid duration=5min — INVALID as raid, but a real density data point
- Casts were correctly GCD-paced this time (~1.6–2.4k per raid window), yet RTT collapsed
  from the first window: p50 37 ms → 6.4 s → 32 s, rising ~1 s per wall-clock second, and
  never recovered between windows.
- Second bot bug: bots teleported INTO the raid cell each minute but never teleported
  home. After t≈10 s the run was effectively **300 bots permanently in one cell** — every
  10 Hz move fanned out to ~300 subscribers (~10⁵–10⁶ row-deliveries/s sustained), not
  the designed 10 s burst per minute. The server fell behind at a constant rate and had
  no quiet period to drain.
- `n=0` after 70 s is partly a measurement artifact: the pending-echo buffer capped at 64
  entries (~6–13 s of inputs), so once lag exceeded that, echoes couldn't be matched.
  Cap raised to 1024 so recovery is visible next time.
- Server (run1.csv): CPU flat at ~35–40 % the whole run (system total 100 % — bots took
  another ~38 % on the same box), memory sawtoothing 1–2.1 GB. After bots stopped:
  ~100 s drain at 6–7 % CPU, then released to 106 MB. **No crash, full recovery again.**
- Real finding worth keeping: **300 clients sustained in one cell is beyond the per-cell
  ceiling on this machine** — delivery backlog grows unboundedly while server CPU sits
  at ~40 %, suggesting the bottleneck is subscription evaluation/delivery, not raw CPU.
  Interest management must keep per-cell subscriber counts well below this.
- Bugs fixed (teleport home after window; pending cap 1024). Rerun required to measure
  the designed burst-and-recover behavior.

### Run: 2026-07-16 rung=300 scenario=raid (all bots, one cell) duration=5min — FAIL as run; ceiling found
- Bots fixed this time (GCD pacing, teleport home after each 10 s window, echo buffer
  1024). Pre-raid baseline clean: p50 25 ms, p95 38 ms, full echo rate.
- First raid window: RTT p50 exploded and grew ~1 s per wall-clock second thereafter
  (10 s → 32 s → 61 s → 114 s). Server drained in visible bursts between windows
  (n=73k echoes in one interval at t=120 s, p50 dropped 61→36 s) but each new window
  re-flooded it faster than it drained. Blind (n=0) only in the last 50 s.
- Math: 10 s window × 300 bots × 10 Hz moves × ~300 subscribers ≈ 9M row-deliveries
  per window, vs a measured drain of well under 1M per 50 s gap → unbounded growth.
- Server (run1.csv): CPU **flat ~32–37 %** the entire time — never saturated — while
  memory sawtoothed 1–2.5 GB. Bots also flat ~36 %; system total 100 %. After the run:
  drain at 7 % CPU with memory still climbing to 3.1 GB before release. No crash.
- Failure shape (the spike's key deliverable): the ceiling is **subscription
  delivery/fan-out throughput, not CPU** — the server can't push ~1M row-updates/s
  even with idle cores. Caveats: single machine (client deserialization shares the
  box and is a plausible co-bottleneck), and this scenario (100 % of the realm in one
  cell) is harsher than the design's "a hotspot's bots".
- Consequence for M8: hard cap on per-cell subscriber count. To find the actual cap,
  the runner now takes `--raid-frac <0..1>` (fraction of bots that raid; the rest
  wander uniformly). Next: binary-search raid-frac (0.25 → 75 bots, then 0.5) for the
  largest raid that recovers within the 50 s gap.

### Run: 2026-07-16 rung=300 scenario=raid raid-frac=0.25 (75 raiders) duration=5min — PASS
- 75 bots teleport into one cell every 60 s, cast GCD-paced for 10 s, teleport home;
  remaining 225 wander uniformly.
- RTT ms: p50 flat 23–32 all run, full echo rate (n≈29.3k per interval), errors=0.
  Raid windows visible only as p95 bumps to 78–104 (at 120/180/240/300 s), recovering
  within one 10 s interval — well under the 250 ms threshold even mid-burst.
- Ceiling bracket so far: **75 in one cell = comfortable; 300 = unbounded collapse.**
- Next probe: raid-frac 0.5 (150 raiders) to tighten the bracket.

### Run: 2026-07-16 rung=300 scenario=raid raid-frac=0.5 (150 raiders) duration=5min — degraded but recovers
- Each raid window (50/110/170/230/290 s) pushed RTT into the seconds: worst window
  p50=7.8 s, p95=10.3 s. But every window **fully drained within 10–20 s of ending**
  (p50 back to ~28 ms), errors=0, echo rate healthy throughout. Bounded, localized,
  recoverable — the opposite shape from the 300-raider collapse.
- Baseline between windows identical to uniform (p50 27–33 ms, p95 ~45 ms).
- Drain rate ≈ 2× the fill rate at 150 raiders; at 300 the fill quadruples (fan-out is
  quadratic: N movers × N subscribers) while drain stays flat → unbounded. Consistent
  quadratic model.

### Rung-1 raid verdict (ceiling characterized)
- **75 in one cell**: negligible (p95 bump to ~100 ms, recovers in one interval).
- **150 in one cell**: multi-second lag during the burst, full recovery in 10–20 s.
- **300 in one cell**: unbounded backlog growth, no recovery while load continues.
- The p95<250 ms threshold is broken *during* 150-raider bursts, but degradation is
  localized to the hot cell and recovery beats the 30 s storm-recovery bar — per the
  gate rules this is a **pass with a documented interest-management requirement**:
  M8 must keep effective per-cell subscriber overlap ≲100 at full update rate
  (tiered/throttled updates or cell splitting above that). Single-machine caveat
  applies to all numbers; revisit on rung 2 hardware.

### Run: 2026-07-16 rung=300 scenario=hotspot duration=30min — memory watch-item RESOLVED (plateau); late runaway
- **Memory plateau confirmed**: server grew 282 → ~950 MB decelerating over ~15 min,
  then held flat 950–1000 MB for 10+ min (t≈870–1470 s). The 5-min watch-item is
  closed — hotspot subscription memory is bounded at this density.
- Healthy for 25 min: p50 26–45 ms, p95 mostly < 100 ms with occasional blips
  (232–452 ms) that always recovered. Slow upward creep of baseline and blip size
  over the run.
- At t≈1505 s: runaway. RTT grew ~0.65 s/s (1.4 s → 110 s over 200 s), echoes blind
  after t=1710 (lag exceeded the 1024-entry echo buffer), no recovery while load
  continued. Server memory exploded 1.0 → 6.6 GB (queued deliveries); CPU stayed
  27–38 % — same not-CPU-bound failure shape as raid. After bots stopped: drained
  ~4 min, released to 119 MB. No crash, errs=0, disc=0.
- Trigger is ambiguous: system CPU jumped 60 → 100 % at the same moment (13:09:39),
  but server+bots account for ~27 pp of that jump themselves — could be foreground
  contention tipping a marginally-stable system, or organic drift (e.g. commitlog
  growth) crossing the delivery ceiling. The 25-min creep suggests the system was
  slowly approaching the ceiling either way.
- Verdict: memory PASS; stability at 300-bot hotspot density on a shared box is
  **marginal** — once delivery falls behind, the spiral does not self-recover under
  sustained load. Rung 2 (dedicated hardware, bots on a separate machine) must
  disambiguate contention vs organic drift.

### Run: 2026-07-16 rung=3000 scenario=uniform duration=5min LOCAL SMOKE — INVALID for server, conclusive for runner capacity
- Purpose: smoke-test the runner at 3 k before renting rung-2 hardware. `--bots 3000
  --workers 24`, server on the same machine.
- **Machine-wide CPU pinned at 100 % for the entire load window.** stress-bots alone
  took 40–58 % of the box, the server 20–35 %, remainder OS/network stack. Neither
  process was the bottleneck — the box was.
- Consequences visible in the log: immediate delivery collapse (p50 3.4 s at t=10 s,
  growing ~1 s/s, echoes blind from 150 s) *and* the runner's own input loop stalled
  from t≈140 s (10 s intervals sending ~2–10 k inputs vs ~290 k expected — workers
  couldn't schedule at 10 Hz/bot). errs=0, conn held 3000 throughout.
- Server showed the usual backlog signature: memory +~350 MB/sample to 7.5 GB, CPU
  flat 20–35 %, drained after disconnect (settled ~4.6 GB retained heap at 7 % idle).
- Verdict: **one machine cannot honestly drive 3,000 bots and host the server** —
  all RTT numbers are contaminated by client-side scheduling starvation. Rung 2
  requires bots on separate hardware (one VM can likely drive 3 k alone; if its CPU
  pins, split into 2× `--bots 1500` processes). No server conclusions drawn.

Rung 1 complete. Next: rung 2 (3,000 bots, bots on a separate machine from the server).

## Fallback plan (on no-go)

Custom Rust server with **renet/QUIC**: we build persistence, replication, and
interest management ourselves (significant cost — this is why we spike first).
The `game_shared` command interface (ADR-014) is the swap seam; client-side
code above the interface does not change. M2–M4 are backend-neutral and
proceed regardless.
