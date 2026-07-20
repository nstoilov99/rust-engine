# M9.5 — Packaged Co-op Verification

**Status:** ✅ Complete (automatable scope, 2026-07-20) — hour soak pending (user-executed)

- **P1** (`efeea59`): `scripts/smoke_packaged.ps1` — SMOKE PASS (spawn 2 s,
  kill-cleanup 1 s); missing-export failure path exits 1. Found + fixed a PS
  5.1 trap: function return unrolls 1-element arrays to a scalar (call sites
  wrap in `@()`). All repo .ps1 made ASCII-only (PS 5.1 mangles UTF-8-no-BOM
  non-ASCII).
- **P2** (`152a3eb`): load subset rerun vs a packaged-published module —
  parity with M8 fresh-DB baselines (see `M8-LOAD-REPORT.md` addendum);
  `run_scenarios.ps1` gained `-TargetHost`/`-Module` passthrough.
- **P3** (this commit): published `nstoilov-rust-engine` to Maincloud —
  SQL-verified protocol v5 + `build_id` stamp matching HEAD. mp-client
  bundle auto-connected over TLS; two clients (distinct `--net-id` slots)
  mutually visible over real WAN, no mismatch warnings; hard-kill cleared
  the session server-side in <8 s; soak monitor exercised live against both
  processes. **Bug found + fixed**: PS 5.1 `Set-Content -Encoding utf8`
  wrote a BOM into `net_config.ron`, RON rejected it at 1:1 and the client
  silently fell back to localhost defaults — export script now writes ascii,
  and `NetConfig::parse` strips a leading BOM (Notepad hand-edits) with a
  regression test.
- **Remaining**: the two-machine one-hour soak per `M9.5-COOP-RUNBOOK.md`
  (user + friend); record the outcome here.

**Duration:** ~2–3 days
**Prerequisites:** M8 (interest + load harness), M9 (packaging targets, publish script, host-local loop, build-id stamp)

## Goal

Prove the **packaged artifacts** — not the dev environment — deliver the co-op
slice. No cargo, no editor, no source checkout on the test machines. Three
proofs, in increasing reach:

1. a scripted local smoke test with a deterministic exit code (CI-able),
2. load sanity against the module published via the M9 flow,
3. the internet path: publish to a hosted instance, packaged clients join
   over WAN — enabling the milestone's two-machine, one-hour co-op soak.

The Networked Co-op Slice milestone bar: *"running the packaged M9 builds,
verified per M9.5 … runs for an hour without desync, leak, or crash."* The
hour soak itself needs two humans on two machines; M9.5 delivers everything
up to that run (scripts, hosted deployment, runbook, monitoring), and
verifies the full path with two packaged clients from one machine.

## What M9 already gives us (inputs)

- `scripts/export_windows.ps1` — targets `standalone` / `mp-client`
  (mp-client writes `net_config.ron` with `auto_connect: true` next to exe)
- `server/publish.ps1 [-Wipe] [-Server <uri|nickname>] [-Module <name>]` —
  builds + publishes the WASM module, then stamps `build_id` via the
  owner-gated `set_build_id` reducer (warn-only on stamp failure, `exit 0`)
- `scripts/host_local.ps1` — reuse-or-start local SpacetimeDB, publish with
  retry gate, launch packaged client
- Version gates: `PROTOCOL_VERSION = 5` exact-match (hard refusal) +
  `build_id` soft stamp (client prints
  `net: WARNING: build mismatch (server …, client …)` and proceeds)
- Client stdout markers (console subsystem; Rust's stdout is line-buffered
  via `LineWriter` even when redirected, so lines land promptly in a log
  file): `net: connecting to {uri} / {module}`, `net: in world as
  {identity}`, `net: local player bound to entity {id}`,
  `net: disconnected: {reason:?}`
- `--net-id <name>` (M5): distinct identity slot per name — required for two
  clients on one machine, otherwise the second launch replaces the first
  session instead of adding a player

## Package 1 — Packaged smoke script (CI-able)

`scripts/smoke_packaged.ps1` — end-to-end assertion on the packaged client
against a locally published module. Deterministic pass/fail exit code, no
interaction, no reliance on pre-existing state.

Flow:

1. **Preflight**: `<ExportDir>/game.exe` exists (default `build/export`;
   `-Export` switch optionally runs `export_windows.ps1 -Target standalone`
   first). Reuse-or-start local SpacetimeDB exactly like `host_local.ps1`
   (never stop a running instance).
2. **Isolated module**: wipe-publish to `rust-engine-smoke` (own module name
   — never touches `rust-engine-dev` data). Publish uses the same
   `server/publish.ps1`, i.e. the M9 mp-server artifact path.
3. **Launch**: start `game.exe` with
   `--connect http://127.0.0.1:3000 rust-engine-smoke`, stdout/stderr
   redirected to `build/smoke/client.log`.
4. **Assert connect + spawn** (budget: 30 s). Primary signal is SQL (server
   truth); the client log is corroboration:
   - `spacetime sql rust-engine-smoke "select entity_id, session from
     player"` shows exactly one row with a live session
   - client log contains `net: in world as` **and**
     `net: local player bound to entity`
   - client log does **not** contain `WARNING: build mismatch` (exporter and
     publisher ran from the same tree, so the stamps must agree — a mismatch
     here means the stamping pipeline regressed)
5. **Assert crash-disconnect hygiene** (budget: 15 s, tune to the server's
   transport timeout if it proves tight): kill the client process (socket
   drop, the harshest disconnect), then poll SQL until the player row's
   session clears — the M5 server-side cleanup path. Graceful disconnect
   (`net: disconnected:` on clean close) is asserted separately in the P3
   two-client checklist, not here — a scripted kill can't produce it.
6. **Report**: print PASS/FAIL per assertion; exit 0 only if all pass. Log
   and SQL snapshots stay in `build/smoke/` for post-mortem.

Notes:
- Polling loops count only sleep time toward budgets (host_local convention).
- The script must leave a running dev SpacetimeDB untouched; the only state
  it owns is the `rust-engine-smoke` module and `build/smoke/`.
- "CI-able" means deterministic and non-interactive; actually wiring CI is
  out of scope (no CI infra in this repo yet).

## Package 2 — Load sanity on the packaged-published module

Point the M8 bot harness at a module published via the M9 flow and compare
against the M8 baselines (catches shipping-profile and cooked-content
regressions in the publish path).

1. **Tooling tweak**: `tools/net_bots/run_scenarios.ps1` currently hardcodes
   host/module — add `-TargetHost` / `-Module` passthrough to the underlying
   `--host` / `--module` flags (defaults unchanged).
2. **Run**: fresh wipe-publish via `server/publish.ps1 -Wipe` (M8 learned:
   stale DBs inflate tick times), then the fast M8 subset —
   `uniform 50`, `churn 100`, `thrash 50`, 120 s each.
3. **Compare** (fresh-DB M8 observations; pass bar 1.5× M0 as in M8):
   | scenario | M8 `move_tick` p50/p95 (ms) | RTT limit |
   |---|---|---|
   | uniform 50 | 16.5 / 22.9 | p50 ≤ 39, p95 ≤ 75 |
   | churn 100 | 32.0 / 48.8 | p50 ≤ 43 |
   | thrash 50 | 20.0 / 32.2 | — |
4. **Record** results as an addendum section in `M8-LOAD-REPORT.md`
   ("M9.5 packaged-publish rerun") rather than a new report file.

Expected outcome: parity — the publish path was already release-profile WASM
in M8. This package is a cheap regression tripwire, not new ground.

## Package 3 — Internet path: hosted publish + WAN clients

Get the module onto a host reachable over the internet and verify packaged
clients join it. **Recommendation: SpacetimeDB Maincloud free tier** —
zero ops, 2,500 TeV/month energy credit (≈3M reducer calls; a 2-player
1-hour soak at 20 Hz tick ≈ 75k calls — comfortable). Fallback if Maincloud
disappoints (latency, version skew vs CLI 2.6.1): rented VPS running
`spacetime start` — documented as a runbook variant, not implemented.

1. **Publish**: `spacetime login` (verify with `spacetime login show`), then
   `server/publish.ps1 -Server maincloud -Module <name>` (the `-Server`
   plumbing from M9 P3 passes CLI nicknames through). Maincloud module
   names are globally scoped — use an account-unique name (e.g.
   `nstoilov-rust-engine`). The `set_build_id` stamp succeeding against
   Maincloud is an acceptance gate here, not warn-and-shrug: the owner row
   is whoever ran init, so a publish from any other identity would stamp-
   fail silently (publish.ps1 deliberately exits 0 on stamp failure).
2. **Client bundle**: `export_windows.ps1 -Target mp-client` with the
   Maincloud URI + module in `net_config.ron`; verify the packaged client
   connects over TLS (`https://maincloud.spacetimedb.com`) — first real
   exercise of a non-localhost, https URI end to end.
3. **Two clients, one machine, real WAN**: launch two packaged clients
   with distinct `--net-id` slots (same host/module share a credential
   store — without it the second launch replaces the first session);
   verify mutual visibility, movement replication, combat, graceful
   disconnect (`net: disconnected:` on clean close) and reconnect without
   ghosts. This proves the entire internet path minus the second physical
   machine.
4. **Soak monitor**: `scripts/soak_monitor.ps1` — samples the client
   process's working set + handle count every 30 s to a CSV; the leak
   evidence for the hour soak (flat-ish memory = pass, monotonic growth =
   investigate).
5. **Runbook**: `docs/roadmap/M9.5-COOP-RUNBOOK.md` — step-by-step for the
   two-machine hour soak: host publishes (step 1), zips + sends the
   mp-client bundle, both players run it alongside `soak_monitor.ps1`;
   checklist of the milestone behaviors (traverse M3 world, see each other
   move (M6), abilities/combat (M7), disconnect + reconnect cleanly (M5));
   what to capture on failure (client logs, monitor CSVs, `spacetime logs
   --server maincloud <module>`, SQL snapshots — admin commands need the
   explicit `--server maincloud` since this machine's default is `local`).
   Separate machines have separate credential stores, so `--net-id` is not
   needed there.

## Acceptance

- [x] `scripts/smoke_packaged.ps1` passes from a clean shell; exit 1 with a
      clear message when the export is missing or an assertion fails
- [x] Load rerun numbers recorded; within 1.5× M0 bars (parity with M8
      fresh-DB expected)
- [x] Module published to Maincloud with correct protocol v5 and the
      build-id stamp **verified via SQL** (not just publish exit code); two
      packaged clients complete the co-op checklist over WAN from this
      machine
- [x] Runbook + soak monitor committed — the two-machine hour soak is
      hand-off-able to "user + friend" without me in the loop
- [ ] The hour soak itself: **user-executed**, results recorded back into
      this doc's status line when done

## Out of scope

- CI wiring (no CI infra yet; script is CI-shaped for later)
- VPS self-hosting implementation (runbook variant only)
- Any editor work (that's M9.6)
- New gameplay/net features — verification only; bugs found here get fixed
  under their owning milestone's banner

## Open questions (answered by recommendation unless overridden)

1. **Maincloud vs rented VPS** → Maincloud free tier; VPS stays a
   documented fallback.
2. **Smoke module name** → fixed `rust-engine-smoke`, wipe-published every
   run; keeps dev data untouchable.
3. **Load rerun scope** → fast subset (uniform-50 / churn-100 / thrash-50)
   rather than the full M8 matrix; the full matrix reruns only if the subset
   deviates.
