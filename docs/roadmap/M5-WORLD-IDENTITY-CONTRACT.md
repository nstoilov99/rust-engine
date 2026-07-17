# M5 World Identity & Session Contract

**Status:** 🚧 Proposed (Package 0 of `VULKANO-M5-NET-A-CONNECTION-IDENTITY.md`)
**Rule:** no M5 networking code lands until this contract is agreed. Changes
after agreement require editing this doc first.

This contract defines *identity*: how a thing on the server and a thing on a
client are known to be the same thing, across frames, subscriptions, zone
changes, disconnects, respawns, and (later) module shards. Everything here is
normative; "MUST/NEVER" is meant literally.

---

## 1. Entity identity

### 1.1 The identity tuple

Every replicated entity is identified by:

```
(realm_id: u32, entity_id: u64, generation: u32)
```

- **`realm_id`** — identifies the module instance (shard). Constant `0` in
  M5. It exists now so that when M8+ shards zones into modules, IDs from
  different modules can never collide in client-side maps, derived GUIDs, or
  logs. Clients MUST carry `realm_id` in every identity-keyed structure even
  while it is always `0`.
- **`entity_id`** — allocated by a single, module-wide, monotonically
  increasing `u64` counter. One namespace for **all** replicated kinds
  (players, NPCs, later projectiles/drops). IDs are NEVER reused; the
  counter never resets (persisted in a singleton row, survives module
  updates). At 1M allocations/s a u64 lasts >500k years — wrap handling is
  explicitly out of contract.
- **`generation`** — increments each time the *same logical entity*
  respawns. A player keeps their `entity_id` for the life of the character;
  death + respawn bumps `generation`. Purpose: a state sample or event
  stamped with an old generation can NEVER attach to the successor
  incarnation. Starts at 1.

Rule of thumb: `entity_id` answers "which entity", `generation` answers
"which life of it".

### 1.2 Allocation

- The allocator is a singleton counter row; `alloc_entity_id()` is a helper
  inside the module, called only from reducers (transactional, so no races).
- Nothing client-side ever allocates or proposes an `entity_id`.

### 1.3 `character_id` — the persistence key

- Each account owns one `character_id: u64` (M5: exactly one character),
  allocated from the same counter style but a separate sequence.
- `character_id` is the ONLY key that persistence, social features, or
  cross-module references may use. `entity_id` is a *runtime* handle:
  module transfer (M8+) re-issues `entity_id` under the new `realm_id`, but
  `character_id` travels unchanged.
- Corollary: nothing durable may be keyed by `entity_id`. In M5 the player
  row carries both; the `entity_id` is regenerated if the module is ever
  wiped and re-seeded from persistent character data.

## 2. Client-side identity

### 2.1 Proxies and `NetIndex`

- Replicated entities exist client-side as **proxy entities** in the hecs
  world, created and destroyed ONLY by the replication system. They are
  never created by scene load and NEVER serialized into `.scene` files
  (guard: proxies carry a `NetProxy` component; the scene serializer skips
  entities that have it).
- A `NetIndex` resource maps `(entity_id, generation) → hecs::Entity` and
  back (`realm_id` omitted from the map key while single-realm; the field
  exists in `NetProxy` so adding it to the key later is mechanical).
- Any incoming sample or event whose `(entity_id, generation)` is not
  currently live in `NetIndex` is dropped silently. This single rule kills
  the apply-after-despawn and stale-generation bug classes.
- **Local-player exclusion**: the own player row (identified by
  `owner_identity == local Identity`) is NEVER proxied. The cache-diff
  skips it; instead it is *bound* to the pre-existing local player entity
  (which input drives directly in M5). The binding is registered in
  `NetIndex` like any other entry so generation/epoch rules apply
  uniformly. On snapshot apply after reconnect, and on a generation bump
  (respawn), the local entity snaps to the row's state. Without this
  exclusion the naïve diff would spawn a second "me" — that is a contract
  violation, not an edge case.

### 2.2 Derived `EntityGuid`

- World/static entities keep their manifest-derived `EntityGuid` (M4,
  ADR-013) — unchanged by this contract.
- Proxies get `EntityGuid = uuid_v5(NET_PROXY_NS, name)` where
  `NET_PROXY_NS = 7d9c4b62-3f1a-4e08-9b5d-2c8a61f0e4d3` (constant in
  `game_shared::net`) and `name` is exactly 16 bytes:
  `realm_id as u32 LE (4) ‖ entity_id as u64 LE (8) ‖ generation as u32
  LE (4)`. Fully deterministic and identical across clients;
  collision-*resistant* against manifest GUIDs (distinct namespace) — so
  debug overlays and editor tooling treat proxies uniformly.
- The derived GUID is NEVER authoritative and never sent over the wire; the
  tuple is the identity, the GUID is a projection of it.

## 3. Lifecycle: spawn, despawn, tombstones

### 3.1 Server-side

- **Spawn (new logical entity)**: a reducer inserts the entity row with a
  fresh `entity_id`, `generation = 1`.
- **Respawn (same logical entity — players)**: an **update** of the
  existing row in one transaction: `generation += 1`, position reset to
  spawn point. The row never vanishes and NO tombstone is written — the
  generation bump on a live row is itself the "previous incarnation ended"
  signal (see 3.2). `entity_id` (the PK) never changes.
- **Despawn (destruction — NPCs and future kinds)**: the reducer MUST, in
  one transaction, upsert the tombstone (see below) and delete the entity
  row. Both or neither. A destroyed logical entity never returns; a
  "respawned NPC" is a new logical entity with a fresh `entity_id`.
- **Tombstone table**: `tombstone { entity_id (PK), generation,
  despawned_at }`, **upserted** on despawn — repeated deaths of the same
  `entity_id` before GC keep the highest generation and latest timestamp.
- **Player disconnect is not despawn**: `client_disconnected` clears the
  session field; the player row persists (SpacetimeDB tables are the save
  system). The player entity simply stops moving. (Despawn-on-logout
  policies are gameplay decisions deferred past M5.)
- **Tombstone GC**: a scheduled reducer deletes tombstones older than
  `TOMBSTONE_TTL` (contract value: **5 minutes**). Because `entity_id`s are
  never reused, an expired tombstone can cause no ambiguity — a client that
  was gone longer than the TTL treats every unknown missing row identically
  (see 3.2), which is correct behavior after that long an absence.

### 3.2 Client-side interpretation

Tombstones live in the **permanent** subscription, and the client MUST
retain **local tombstone evidence**: every tombstone row insert/update
observed is recorded in a client-side map `entity_id → (generation,
seen_at)`, pruned by `TOMBSTONE_TTL` locally. This map — not the cache
alone — feeds classification, so a pump that delivers destruction *and*
GC-deletion (or any callback-ordering quirk) before the next diff cannot
erase the evidence.

When the cache-diff (plan D3) finds a proxy whose row has vanished, with
proxy generation `g`:

- evidence exists for its `entity_id` with `generation >= g` →
  **destroyed**: despawn the proxy, destruction effects allowed.
- otherwise → **out of scope** (left my zone subscription): despawn the
  proxy, no destruction effects. (An older-generation tombstone does NOT
  classify a newer incarnation as destroyed.)

When the diff finds a *live* row whose `generation` differs from the
proxy's: treat as despawn (no effects) + fresh spawn — reset the proxy
in place (new `NetIndex` entry, cleared interpolation buffer, snap).

Both vanish paths remove the proxy from `NetIndex`. In M5 the visible
difference is only debug-overlay tagging; the distinction exists because
M7 (death effects) and M8 (scope churn at scale) need it already correct.

## 4. Credentials, accounts, sessions

### 4.1 Token persistence

- On first successful connect, the SDK-issued auth token is persisted to
  `<user data dir>/rust-engine/net/credentials.ron` (plaintext file, dev
  policy; production credential storage is explicitly out of M5 scope).
- Every reconnect presents the stored token (`with_token`), so the
  SpacetimeDB `Identity` — and therefore the `account` row, `character_id`,
  and owned player row — survive client restarts. **This is the reconnect
  guarantee**; without it "no duplicate entities" is unachievable.
- Invalid/expired/rejected token: delete the stored token, connect fresh
  (new Identity ⇒ new account), log a prominent warning. Account recovery
  is out of scope.
- "Log out / reset identity" (dev tooling): delete the credentials file.

### 4.2 Accounts

- `account { identity (PK), character_id, name, created_at }` is created by
  `client_connected` on first sight of an Identity. `name` defaults to
  `"Player-" + short hash of identity`; `enter_world` MAY carry an optional
  name claim (no uniqueness enforcement in M5; polish deferred to M9).

### 4.3 Sessions — logical revocation, not "kick"

SpacetimeDB has no reducer API to forcibly terminate another
`ConnectionId`, and identity-only auth would let a stale connection of the
same Identity keep acting. Sessions are therefore **logically revoked**:

- **One active session per Identity.** The player row stores
  `session: Option<ConnectionId>`.
- `client_connected` (or `enter_world`, see 4.5) for an Identity whose
  player row already holds a live session **overwrites** `session` with the
  new `ConnectionId` (last-wins). The old connection is not closed by the
  server; it is *revoked*: every session-scoped reducer rejects it (4.4).
- **Session-replaced signal**: the revoked client observes its own player
  row's `session` change to a `ConnectionId` that is not its own (the row
  is in its permanent subscription). On seeing this it MUST stop sending,
  surface "session replaced on another connection", and disconnect itself.
- `client_disconnected` clears `session` ONLY if the disconnecting
  `ConnectionId` matches the stored one — a late disconnect callback from a
  revoked stale connection MUST NOT knock the new session offline.

### 4.4 Authorization: identity AND session

Every client-callable, player-affecting reducer (`enter_world`,
`submit_input`, future actions) MUST verify **both**:

1. `ctx.sender == player.owner_identity` (ownership), and
2. `ctx.connection_id == player.session` (active session)

and silently reject otherwise (exception: `enter_world` may *install* the
session per 4.5). Identity alone is insufficient — a revoked connection
shares the Identity. NPCs have no owner; only scheduled reducers mutate
them.

### 4.5 `enter_world` lifecycle guards

- Table constraints make duplicates structurally impossible:
  `player.owner_identity` and `player.character_id` are `#[unique]`
  columns — a second player row for the same account cannot exist.
- First call for an account: creates the player row (spawn point,
  `generation = 1`, `epoch = 1`), installs `session = ctx.connection_id`.
- Subsequent calls: if the row exists, `enter_world` installs the caller
  as the active session (revoking any previous, per 4.3) and bumps
  `epoch`. **Idempotency**: if `session` already equals
  `ctx.connection_id`, the call is a no-op (no epoch bump, no state
  change) — a retried/duplicated `enter_world` from the live connection
  cannot desync the epoch.
- A revoked (non-session) connection calling `enter_world` re-takes the
  session (that is the reconnect path — last-wins by design). Any other
  reducer from a revoked connection is rejected per 4.4.

## 5. Input identity: epoch + sequence

- The player row carries `epoch: u32` and `last_input_seq: u32`.
- A non-idempotent `enter_world` (4.5) increments `epoch` and resets
  `last_input_seq` to 0. The client learns its current `epoch` from its
  own player row after the snapshot applies, and MUST NOT send input
  before then.
- Client stamps each `ClientInput` with `(epoch, seq)`, `seq` starting at 1
  and strictly increasing within the session. The server accepts iff
  `epoch` matches the row AND `seq > last_input_seq`; accepted input sets
  `last_input_seq = seq`. Everything else (stale, duplicate, wrong-epoch)
  is dropped silently — never an error, never a disconnect.
- Gaps are legal (client coalesces). `u32` cannot wrap at ≤20 Hz within any
  plausible session; wrap is out of contract.
- Replication of `(epoch, last_input_seq)` on the own-player row IS the
  ack. M6's prediction buffer keys on it; M5 only displays it.
- A failed reducer call client-side is dropped, not retried; the next
  input supersedes it.

## 6. Zone and module transfer

- **Zone change (M5)**: subscription scope changes; identity does not.
  `entity_id`, `generation`, `epoch` all survive; the proxy set on other
  clients changes purely through subscription diffs.
- **Module transfer (M8+, contract only)**: the destination module
  allocates a fresh `entity_id` under its own `realm_id`; `character_id`,
  account, and name travel; `generation` restarts at 1; `epoch` restarts.
  The invariant this contract binds now: **no system may assume
  `entity_id` is stable across realms, and no durable state may key on
  it** (§1.3). Systems written in M5–M7 that honor §1.3 need no rewrite
  for sharding.

## 7. Recorded layout decisions (plan Open Questions)

1. **Client net code**: separate crate `game_client_net/` wrapping
   `spacetimedb-sdk` behind the `game_shared` `NetClient` trait.
   `game_client` depends on it unconditionally for now (compile-time cost
   only); feature-gating out of editor builds is deferred until it hurts.
2. **WASM sharing**: verified 2026-07-18 — `game_shared` compiles cleanly
   to `wasm32-unknown-unknown`. The server module depends on `game_shared`
   for constants (`PROTOCOL_VERSION`, `INPUT_SEND_HZ`, `TOMBSTONE_TTL`,
   grid math) and pure logic (epoch/seq acceptance, version check). Table
   row types remain module-local (they must derive SpacetimeDB traits and
   `game_shared` must stay backend-free); conversion happens inside
   reducers. Protocol structs use plain scalar/array fields so mirroring
   is field-for-field.
3. **Input send rate**: fixed constant `INPUT_SEND_HZ: u32 = 20` in
   `game_shared::net::schema`, independent of the server tick rate.
4. **Names**: auto-generated with optional claim in `enter_world` (§4.2);
   account polish deferred to M9.
5. **Tables**: `player` and `npc` stay separate (player-only columns:
   session, epoch, input seq, owner). The shared allocator (§1.2) already
   guarantees one ID namespace across both.

## 8. Invariants summary (test targets)

1. An `entity_id` is never reused (allocator monotonic, persisted).
2. A sample stamped `(id, gen)` never mutates an entity whose live
   generation differs.
3. Proxies never enter `.scene` files, and the own player row never
   produces a proxy (local-player exclusion, §2.1).
4. Destroyed vs out-of-scope is always distinguishable within
   `TOMBSTONE_TTL`, generation-aware, even if destruction and tombstone-GC
   arrive in one pump (local evidence map, §3.2).
5. A client restart with a valid token resumes the same account, player
   row, and position — zero duplicate entities after reconnect; `#[unique]`
   constraints (§4.5) make a duplicate player row unrepresentable.
6. A revoked stale connection can neither act (every session-scoped
   reducer rejects it, §4.4) nor take the live session offline with its
   late disconnect (§4.3).
7. Input with wrong epoch or non-increasing seq is provably ignored;
   repeated `enter_world` from the live connection does not bump the epoch
   (§4.5).
8. Nothing durable is keyed by `entity_id` (grep-auditable: persistence
   code touches `character_id` only).
