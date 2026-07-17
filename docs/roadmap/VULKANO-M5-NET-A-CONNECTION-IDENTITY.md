# M5 — Net-A: Connection, Identity & Replication

**Status:** 🚧 In Progress
**Duration:** ~3 weeks
**Prerequisites:** M0 SpacetimeDB spike (✅ GO), M4 Zone & Chunk Lifecycle (✅)
**Roadmap:** `ROADMAP.md` Task M5 — first slice of the Multiplayer Foundation
arc (Phase M). ADR-013 (EntityGuid), ADR-014 (SpacetimeDB + backend-neutral
seam), ADR-015 (server-authoritative movement), ADR-016 (spike-first gating).

Goal: a client can connect to a SpacetimeDB module, authenticate with a
persistent identity, spawn a player entity with stable identity, see other
players/NPCs replicated into the local ECS with interpolation, disconnect
and reconnect without duplicate entities — with schema versioning, input
sequence numbers and a clock-sync estimate in place from day one. Movement
in M5 is **trust-the-client** (server echoes positions); M6 makes it
server-authoritative.

---

## Current state (verified 2026-07-18)

| Piece | State | Pointer |
|---|---|---|
| Networking code in engine/game_client | **None** — no session, auth, or socket code anywhere | — |
| `GameCommand` / `GameCommandBuffer` | Local frame-command buffer only; **not** the ADR-014 net seam (that does not exist yet) | `game_shared/src/commands.rs` |
| M0 spike code | External, explicitly **non-reusable** (throwaway); reference only | `E:\Projects\Rust\STDBStressTest` (`stress-server`, `stress-bots`) |
| SpacetimeDB versions proven in spike | `spacetimedb` 2.6, `spacetimedb-sdk` 2.6 (resolved 2.6.1) | spike `Cargo.toml`s |
| `EntityGuid` | On every entity, persisted in `.scene`; **no guid→entity map** (linear scans) | `engine/src/engine/ecs/components.rs:648`, ADR-013 |
| Zone tracking | `WorldStreamer` tracks current zone, emits `ZoneChanged` events | `engine/src/engine/world/streamer.rs` |
| Local player control | Input → movement systems, single-player only | `game_client/src/systems/player_input.rs`, `systems/character_movement.rs`, `plugin.rs` |
| Async pattern to copy | `AsyncAssetLoader`: worker threads + main-thread `poll_results()` drain | `engine/src/engine/assets/async_loader.rs` |
| World contract | `WorldManifest` cells/zones/spawn points, deterministic root GUIDs; 64 m cells, Z-up | `game_shared/src/world_manifest.rs` |

### Binding constraints from the M0 spike (Go/no-go verdict)

1. Interest management (M8) is mandatory at scale; until then keep test
   populations small and zone-scoped subscriptions crude but real.
2. Realm scale requires module sharding later — design the `game_shared`
   seam so a zone can map to a module without client rewrites (the seam
   must carry **module addressing**, not assume one endpoint).
3. **Client-side rate limiting of casts/inputs is required** — server-side
   rejection still burns the delivery budget. This lands in M5, not later.
4. Egress is the scarce resource; failure mode is a latency spiral with RAM
   backlog. Keep per-row payloads small from the first schema.

Spike-observed risks that shape this plan: delivery is
serialization-bound on one thread; no outbound backpressure in the host;
WebSocket/TCP head-of-line blocking; **SDK reducer calls can panic after the
socket dies** (must guard every call site behind the connection wrapper).

### SpacetimeDB SDK facts the design must respect

- Table reads come from the **client cache**, which is populated only by
  applied subscriptions — nothing (including `config`) is readable before a
  subscription's `on_applied` fires.
- Initial subscription apply and unsubscribe both fire row insert/delete
  callbacks; a row callback therefore does **not** by itself mean gameplay
  spawn/despawn.
- An anonymous connection without the previously issued token gets a
  **new `Identity`**; resuming an account requires persisting and
  re-presenting the token (`with_token`).
- One `Identity` can hold multiple simultaneous connections
  (`ConnectionId`s); `client_disconnected` fires per connection.
- The SDK supports main-thread message pumping (`frame_tick`-style) as an
  alternative to a background callback thread.

---

## Design

### D0. World Identity & Session Contract (written + reviewed before any code)

A short contract doc (`docs/roadmap/M5-WORLD-IDENTITY-CONTRACT.md`) agreed
before Package 1 starts. It must pin down, precisely:

**Entity identity**
- Every replicated entity has a server-issued `entity_id: u64` from a
  **single monotonic allocator** shared by all replicated kinds (players,
  NPCs, later drops/projectiles) — one namespace, IDs never reused. A
  `spawn_generation: u32` increments each time the *same logical entity*
  respawns (player death/respawn keeps `entity_id`, bumps generation), so a
  late sample can never attach to a successor incarnation.
- The network identity is `(realm_id, entity_id, generation)`. `realm_id`
  identifies the module instance; it exists in the contract now (constant
  `0` in M5) so sharded modules later cannot collide, per spike
  requirement 2.
- **Client-side**: a `NetIndex` resource maps
  `(entity_id, generation) ↔ hecs::Entity`. Replicated entities are
  **proxies** spawned/despawned only by the replication system — never by
  scene load, never persisted into `.scene` files.
- **EntityGuid relationship**: world/static entities keep their
  manifest-derived `EntityGuid` (M4). Proxies get a GUID derived
  deterministically from `(realm_id, entity_id, generation)` for uniform
  editor/debug tooling; the GUID is *derived*, never authoritative.
- **Tombstones**: every gameplay despawn writes a `tombstone` row
  (`entity_id` PK, `generation`, `despawned_at`) *before* the entity row is
  deleted, retained for a fixed window (minutes) and GC'd by a scheduled
  reducer. Tombstones are in every client's **permanent** subscription
  (small table), so a client can always distinguish "destroyed"
  (tombstone present) from "left my subscription scope" (no tombstone).
  Since `entity_id`s are never reused, tombstones are unambiguous.
- **Identity across zone transfer**: within one module, `entity_id` is
  stable across zones (subscription scope changes, identity does not).
  Module transfer (M8+) re-issues `entity_id` under a new `realm_id` but
  carries a persistent `character_id` (account-scoped); persistence and
  social features key on `character_id`, never `entity_id`.

**Credentials & sessions**
- On first connect the server-issued token is persisted locally (dev: file
  under the user data dir; production hardening deferred). Reconnects
  present it via `with_token` so `Identity` — and therefore the account and
  owned player row — survive restarts. Invalid/expired token ⇒ clear it,
  connect fresh, log a warning (dev policy; account recovery is out of
  scope).
- **Session policy**: one active session per `Identity`. `client_connected`
  for an identity that already has a live session kicks the old
  `ConnectionId` (last-wins). The player row tracks
  `session: Option<ConnectionId>` rather than a bare `online` bool;
  `client_disconnected` only marks offline if the disconnecting
  `ConnectionId` matches.
- **Ownership**: player rows carry `owner_identity`; every input reducer
  validates the caller is the owner.

The contract doc also records the crate-layout decision (D3) and the
shared-types-in-WASM decision (D2) — see Open questions.

### D1. Backend-neutral protocol seam in `game_shared`

New module `game_shared/src/net/` (pure data, no SpacetimeDB deps):

- `protocol.rs` — versioned message/row shapes as plain Rust types:
  `ClientInput { epoch: u32, seq: u32, movement, look, actions }`,
  `EntityState { entity_id, generation, pos, vel, yaw, kind,
  server_time_us }` (the server timestamp is what interpolation buffers key
  on), `SpawnInfo`, `DespawnInfo { destroyed: bool }`, `ClockSample`.
  All positions Z-up, meters, matching `world_grid`.
- `schema.rs` — `PROTOCOL_VERSION: u32` (single source of truth) plus a
  compatibility check helper. Version bumps are manual and deliberate.
- `traits.rs` — the ADR-014 seam: a `NetClient` trait
  (`connect(ModuleAddr)`, `send_input`, `poll`, `connection_state`) and a
  `NetEvent` enum (`Connected`, `Disconnected{reason}`, `SpawnEntity`,
  `DespawnEntity`, `StateUpdate`, `InputAck{epoch, seq}`, `ClockSample`,
  `VersionMismatch{server, client}`). `ModuleAddr` (host + module name) is
  part of the seam so zone→module routing later is a lookup, not a client
  rewrite (spike requirement 2). A renet/QUIC fallback backend would
  implement the same trait.

`GameCommandBuffer` stays what it is (local frame commands); it is not
extended for networking in M5.

### D2. Server module crate (SpacetimeDB, WASM)

New crate `server/game_module/` (workspace-**excluded**; own target dir,
built with the spacetime CLI toolchain like the spike). Depends on
`game_shared` for shared math/protocol types where WASM-compatible;
otherwise mirrors types with conversion at the boundary (decide in
Package 0/1 — spike kept them separate).

Tables:

- `config` — singleton: `protocol_version`, `realm_id`. In every client's
  permanent subscription; the version handshake reads it at `on_applied`.
- `entity_allocator` — singleton `auto_inc` counter (or equivalent):
  the single `entity_id` source (D0).
- `account` — `identity` (PK), `character_id`, `name`, `created_at`.
- `player` — `entity_id` (PK), `generation`, `owner_identity`,
  `character_id`, `session: Option<ConnectionId>`, `pos`, `vel`, `yaw`,
  `zone_id`, `epoch`, `last_input_seq`, `last_update_time_us`.
- `npc` — `entity_id` (PK), `generation`, `pos`, `yaw`, `zone_id`, `kind`,
  `last_update_time_us`.
- `tombstone` — `entity_id` (PK), `generation`, `despawned_at` (D0).
- `clock` — singleton, server tick counter + wall time, updated by the
  scheduled `tick` reducer (coarse time base).
- `ping_result` — `identity` (PK), `nonce`, `server_time_us`: written by
  the `ping` reducer, subscribed by owner only (clock sync, D5).

Reducers: `init`, `client_connected` (session kick policy per D0),
`client_disconnected` (clears `session` if matching; row is **never
deleted** — persistence), `enter_world` (spawn or resume at spawn point /
last pos; bumps `epoch`, resets `last_input_seq` to 0, bumps `generation`
on respawn-after-death), `submit_input(ClientInput)` (validates ownership,
epoch, seq; applies movement trust-the-client; stamps
`last_update_time_us`), `despawn_npc` (writes tombstone then deletes —
exists so Package 5 can test destruction vs scope-loss), `ping(nonce)`,
`tick` (scheduled: NPC wander with timestamps, clock row, tombstone GC).

**The SpacetimeDB tables are the save system** — no separate persistence
path.

### D3. Client net crate: main-thread pump + subscriptions

New crate `game_client_net/` (or module in `game_client` — decide in
Package 0): wraps `spacetimedb-sdk` behind the `NetClient` trait.

- **Main-thread pumping**: the connection is pumped once per frame on the
  main thread (SDK `frame_tick`-style), so callbacks and cache reads all
  happen on the game thread — no cross-thread channels, no locks around
  game state. Fallback if the pump blocks or the SDK version fights this:
  background thread + two queues (reliable control queue that is never
  dropped, plus a latest-wins per-entity state map; overflow of the
  control queue forces a full resnapshot). The fallback design is written
  down so switching is mechanical, but main-thread pump is Plan A.
- **Every** reducer call goes through a connection wrapper that checks
  liveness first (spike: SDK panics on dead-socket calls) and surfaces
  `Disconnected` instead of panicking. No raw SDK calls outside it.
- Connection state machine: `Offline → Connecting → AwaitBaseSub →
  VersionCheck → InWorld → (Disconnected → Reconnecting → …)`.
- **Two subscription sets**:
  - *Permanent* (applied immediately after connect): `config`, own
    `account` + `player` row, `tombstone`, own `ping_result`. Version
    check runs when this set's `on_applied` fires; mismatch ⇒ clean
    refusal with a user-visible message, disconnect, no retry loop.
  - *Zone* (replaceable): `player`/`npc` where `zone_id == current_zone`.
    On `ZoneChanged` (from `WorldStreamer`): apply the new zone
    subscription, then drop the old one after the new `on_applied` (no
    visibility gap). Crude by design; M8 replaces it.
- **Spawn/despawn is cache-diff, not callback-driven** (row callbacks fire
  on subscription apply/unapply, so they can't mean gameplay events).
  Callbacks only set a dirty flag and feed state samples; each dirty
  frame, the replication system diffs "entity rows currently in cache"
  against `NetIndex`: new row ⇒ spawn proxy; row gone ⇒ despawn proxy
  (consult `tombstone` to tag destroyed vs out-of-scope); present in both
  ⇒ update. This single mechanism handles initial snapshot, live
  spawn/despawn, zone-swap churn, **and reconnect reconciliation** — no
  special cases, no duplicate entities possible.

### D4. Input pipeline: epoch + sequence, ack, rate limiting

- Sequencing is **session-scoped**: `enter_world` bumps the server-side
  `epoch` and resets `last_input_seq` to 0; the client stamps every
  `ClientInput` with the current `epoch` (learned from its own player row
  at snapshot) and a `seq` starting at 1. A restarted client therefore
  never fights a stale persisted counter. The server accepts inputs with
  matching `epoch` and `seq > last_input_seq` (gaps fine — client may
  coalesce), silently drops stale/duplicate/wrong-epoch inputs. `u32` seq
  at ≤20 Hz cannot wrap within a session; wrap handling is explicitly not
  needed.
- The client sends nothing until its permanent subscription is applied and
  its player row (with `epoch`) is visible.
- Send rate: fixed (target 20 Hz), coalescing per-frame input — never
  per-frame sends. A failed/rejected reducer call is dropped, not retried;
  the next input supersedes it.
- Replication of `(epoch, last_input_seq)` *is* the ack. Client tracks
  `acked_seq` for the M6 prediction buffer (M5 only shows it in the debug
  overlay).
- **Client-side rate limiter** (spike requirement 3): hard cap on
  `submit_input` and any action reducer per second, enforced inside the
  `NetClient` impl so no game system can bypass it.

### D5. Clock sync

Two-part protocol (the scheduled `clock` row alone has no RTT correction):

- **Ping**: client calls `ping(nonce)` recording local send time; server
  writes `(nonce, server_time_us)` to the caller's `ping_result` row; the
  row update's local arrival time completes an NTP-style sample:
  `offset ≈ server_time − (send + recv)/2`, `rtt = recv − send`. Samples
  with outlier RTT are discarded; the rest feed an EWMA. Pings run at a
  slow steady rate (~1/2 s).
- Result exposed as a `NetClock` resource:
  `estimated_server_time_us()`. Accuracy target: tens of ms — enough to
  timestamp interpolation buffers, not lockstep.
- Every replicated state row carries `last_update_time_us` stamped by the
  server (D2), so interpolation has an authoritative time base per sample.

### D6. Replication → ECS proxies + interpolation

- The cache-diff system (D3) spawns proxies: `Transform` + `MeshRenderer`
  (placeholder capsule) + `Name` + `NetProxy { entity_id, generation }`,
  registered in `NetIndex`. Samples whose `(entity_id, generation)` is not
  live in `NetIndex` are dropped (kills apply-after-despawn races).
- Remote transforms are **interpolated**: per-proxy ring buffer of
  `(server_time_us, pos, yaw)` samples; render at
  `NetClock::estimated_server_time_us() − interp_delay` (~100–150 ms).
  Snap on spawn and on gaps larger than the buffer.
- The local player's entity is **not** a proxy: input drives it directly
  (trust-the-client in M5); the server row exists so *others* see us. M6
  swaps this for prediction + reconciliation against acked state. Both the
  server-side movement application and the client-side direct drive are
  isolated in single functions marked for M6 replacement.

### D7. Reconnect flow

Reconnect presents the persisted token (same `Identity`), re-runs the
state machine, calls `enter_world` (resumes at persisted position, bumps
`epoch`), and the cache-diff reconciliation (D3) absorbs the fresh
snapshot against surviving `NetIndex` entries — update in place, spawn
missing, despawn vanished. **No duplicate entities, no ghosts**, verified
by acceptance test, and no reconnect-specific code path beyond the state
machine.

### D8. Dev/test topology

- Server runs on a local `spacetime` standalone instance (as in the
  spike); a script target publishes the module and wipes dev data.
- A minimal headless bot binary (few clients, not a stress test) exercises
  connect/spawn/move/disconnect for acceptance tests. Lives in `server/`
  next to the module; explicitly small — M0's stress harness is not
  resurrected.

### Non-goals (deferred)

- Client prediction + server reconciliation of movement, server-side
  collision — **M6** (ADR-015 lands there; M5 movement is
  trust-the-client).
- Combat, abilities, damage — **M7**.
- Real interest management, hysteresis subscriptions, module sharding —
  **M8** (M5's zone scoping is a placeholder; the `ModuleAddr` seam and
  `realm_id` are the only sharding provisions).
- Packaged builds, mp-client/mp-server targets, deployment — **M9/M9.5**.
- Editor multiplayer tooling, chat, social features, account recovery,
  production credential storage.

---

## Work packages (each = one reviewable commit)

**Package 0 — World Identity & Session Contract.**
Write `M5-WORLD-IDENTITY-CONTRACT.md` (D0: entity identity, allocator,
tombstones, credential/session policy). Reviewed and agreed **before any
code**. Records the crate-layout and shared-types-in-WASM decisions.

**Package 1 — Protocol seam + server module skeleton.**
`game_shared/src/net/` (D1, incl. `ModuleAddr`) with unit tests for the
schema-version check and epoch/seq acceptance rules (pure functions).
`server/game_module/` with all tables, `init`,
`client_connected/disconnected` (session policy), version + realm
stamping; publishes to local standalone; verified with `spacetime` CLI.

**Package 2 — Client connection, auth persistence, version handshake.**
`NetClient` SpacetimeDB impl: main-thread pump, permanent subscription,
version check at `on_applied` with clean mismatch refusal, token
persistence + `with_token` reconnect (identity stable across restart),
liveness-guarded reducer wrapper, rate limiter shell, session-kick
behavior, connection-state debug overlay. Acceptance: connect; mismatch
refusal (bump server version); kill server mid-session without panic;
restart client and confirm same `Identity`; second connection kicks the
first.

**Package 3 — Snapshot replication: cache-diff proxies.**
`enter_world`, NPC `tick` wander with server timestamps, `NetIndex` +
cache-diff replication (D3/D6 spawn/despawn/update, raw transforms — no
interpolation yet), tombstone-aware despawn tagging, derived-GUID proxies.
Acceptance: proxies appear/disappear with server state; client restart or
reconnect produces zero duplicates (D7 falls out here, early on purpose).

**Package 4 — Input, ack, clock sync, interpolation.**
Epoch+seq input pipeline end-to-end with trust-the-client movement,
ack tracking in overlay, hard client-side rate cap, ping-based `NetClock`
(D5), timestamped interpolation buffers replacing raw transform writes
(D6). Acceptance: two clients + bot see each other move smoothly at
~150 ms delay; input rate provably capped; ack advances.

**Package 5 — Zone scoping + acceptance suite.**
Zone-subscription swap driven by `ZoneChanged` (apply-new-then-drop-old),
`despawn_npc` + tombstone GC, bot-driven acceptance tests: reconnect with
no duplicates, position persistence across disconnect, cross-zone
visibility change without gaps, generation bump prevents stale-sample
attach, destroyed vs out-of-scope distinguished via tombstones.

---

## Risks & mitigations

| Risk | Mitigation |
|---|---|
| SDK panics on reducer call after socket death (spike-observed) | Single liveness-guarded call wrapper (D3); no raw SDK calls outside it |
| Main-thread pump (`frame_tick`) blocks or is unsupported in SDK 2.6 | Verified in Package 2 first; written fallback: background thread + control queue / latest-wins state map with resnapshot-on-overflow (D3) |
| Schema drift between module and client | One `PROTOCOL_VERSION` in `game_shared`; refusal handshake tested in Package 2 |
| Row callbacks misread as gameplay events (sub apply/unapply churn) | Cache-diff replication is the only spawn/despawn authority; callbacks just mark dirty (D3) |
| Zone unsub/resub churn on boundary oscillation | Apply-new-before-drop-old avoids gaps; churn accepted for M5 (zone granularity makes it rare); M8 adds hysteresis |
| Trust-the-client movement invites divergence from M6 design | Movement application isolated in one server reducer + one client function, both marked for M6 replacement |
| WASM module can't reuse `game_shared` types | Decide in Package 0/1; fallback is mirrored types + boundary conversion (spike pattern) |
| Cache-diff cost per frame | O(rows in scope) only on dirty frames; trivial at M5 populations; M8 owns scaling |

## Open questions (decide at review)

1. `game_client_net` as separate crate vs module in `game_client`?
   (Separate crate keeps SDK deps out of the editor build.)
2. Does `game_shared` compile to `wasm32-unknown-unknown` cleanly enough
   for the module to depend on it, or do we mirror protocol types?
3. Input send rate: fixed 20 Hz, or derived from a server tick-rate
   constant in the protocol?
4. Auth naming: anonymous identity + auto name is fine for M5 — claim
   names in `enter_world` now, or defer accounts polish to M9?
5. Merge `player`/`npc` into one `entity` table with a kind enum? The
   shared allocator already gives one ID namespace either way; separate
   tables keep player-only columns (session, input seq) out of NPC rows.
