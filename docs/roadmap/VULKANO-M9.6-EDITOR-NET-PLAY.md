# M9.6 — Editor Net Play Modes & Server-Announced World

**Status:** ✅ Complete
**Duration:** ~4-6 days
**Prerequisites:** M9 (net_config, publish/host-local scripts, build-id stamp), M9.5 (packaged verification)

## Goal

Two things that share one mechanism:

1. **Server Default Map** (UE analogue): the module announces which scene it
   simulates via its `config` table; clients load *that* instead of
   hardcoding `scenes/greybox.scene`. Kills the "connected but wrong local
   scene" bug class for good (see `7026a33` — the auto-connect bundle loaded
   the offline demo scene because scene choice re-derived the connect
   decision).
2. **UE-style Net Mode in the editor play settings**: Play Standalone /
   Play As Client / Play As Listen Server + Number of Players — the
   iterate-on-multiplayer-without-leaving-the-editor loop.

## Current seams (research)

- `Config` singleton (`server/game_module/src/lib.rs:108`): `id`,
  `protocol_version`, `realm_id`, `collision_manifest_hash`, `build_id`.
  Written by `init` (first publish only) + owner-gated `set_build_id`.
  No scene field.
- Client handshake (`game_client_net/src/client.rs:793`):
  `Offline → Connecting → AwaitBaseSub → VersionCheck → EnteringWorld →
  InWorld`. Base subscription includes `SELECT * FROM config`; after
  `on_applied`, `poll` reads the cached row (`client.rs:841`) and gates
  protocol (hard), collision hash (hard), build id (warn).
- Standalone loads the scene at startup, *before* any polling
  (`standalone.rs:116` vs `:410`) — so today scene choice cannot see the
  config row.
- Editor: `NetSession::from_args` once at startup (`app.rs:654`),
  `net.update()` every frame (`app.rs:1127`) regardless of play mode.
  Play enter/exit = full RON snapshot / `world.clear()` + restore
  (`play_mode.rs:14/67`) — anything spawned during play (including net
  proxies) is destroyed on exit. No play-settings UI exists;
  `PlayModeState` (`app.rs:206`) holds snapshot/camera/build-dialog only.

## Package 1 — Server-announced world scene

The module owns the map name; clients stop guessing.

1. **Module**: add `world_scene: String` to `Config`; `init` sets it from a
   compile-time constant next to `REALM_ID` (`"scenes/greybox.scene"`).
   Schema change ⇒ **bump `PROTOCOL_VERSION` to 6** (the row shape changes
   under existing bindings; the exact-match gate makes a bump the honest
   move) and wipe-publish. Regenerate `module_bindings`, rebuild
   `net_bots` (same v5→v6 lesson as M9.5). Caveat (review): stale v5
   clients may fail the base subscription on the unknown column *before*
   reaching the clean `VersionMismatch` — they still refuse, just with a
   subscription/schema error; acceptable, documented here.
2. **game_client_net**: after the config checks pass, surface the value —
   `world_scene()` getter valid from `VersionCheck` onward, plus a
   one-shot `NetEvent::WorldScene(String)` so consumers don't poll.
3. **Standalone consumption** (`standalone.rs`): when a `NetSession`
   exists, don't load a scene at startup. Run the loop sceneless
   ("Connecting…" via the existing HUD/title status; camera/HUD/streaming
   all tolerate an empty world — verified) until the event arrives, then
   run a **deferred world-init helper**: `load_or_create_scene` +
   `register_physics_entities` + transform-cache propagation +
   `WorldStreamer::load_for_scene`. Scene load alone is not enough — those
   three follow-up steps currently run once at startup right after the
   load (`standalone.rs:126-152`), and skipping them would mean no scene
   colliders and stale transforms. Failure handling:
   - announced scene missing from pak → hard disconnect with a clear
     message (content/server mismatch — same family as the collision gate);
   - **new: connect/config deadline** — the state machine today can sit in
     `Connecting` forever (no elapsed-time check in
     `client.rs:793` `poll`); add a timeout (~15 s) that surfaces as a
     disconnect event → standalone falls back to the offline default scene
     with a console line, so a dead server doesn't brick the packaged exe.
4. **Offline default** (Game Default Map analogue): `scenes/main.scene`
   stays the compiled-in default; a `default_scene` override in
   `net_config.ron`'s sibling `game_config.ron` is *not* worth a file yet —
   defer until something else needs packaged game settings (YAGNI).

## Package 2 — Play settings: Net Mode UI + state

- `NetPlayMode { Standalone, Client, ListenServer }` + `host`, `module`,
  `player_count: u8 (1..=4)` in a `PlaySettings` struct on
  `PlayModeState`; persisted by extending the layout file's persisted
  struct (`dock_crusty.rs:62` — currently `tree` + `state` only) with a
  `#[serde(default)]` play-settings field, so old layout files still
  parse and no new file appears.
- UI: dropdown attached to the Play cluster in the menu bar
  (`menu_bar_crusty.rs:374`) — UE's play-options chevron. Fields: mode
  radio, host/module text inputs, player count. Any missing crusty-gui
  widget work happens in `../crusty-gui` first per convention.
- Defaults mirror `NetConfig::default()` (`127.0.0.1:3000` /
  `rust-engine-dev`) so Play As Client against `host_local.ps1` works
  zeroth-config.

## Package 3 — Play As Client

The play/edit-state heart of the milestone.

- Today `--connect` in the editor creates a session at startup and
  `net.update()` runs unconditionally — net proxies spawn **in edit
  mode** (`app.rs:658` / `app.rs:1127`), a latent oddity. M9.6 changes
  the contract: in the editor, `--connect` **pre-fills PlaySettings**
  (mode = Client + host/module) instead of creating a startup session;
  sessions exist only between play-enter and play-exit.
- **Enter play** (mode == Client): after `enter_play_mode()` snapshots,
  create `NetSession::connect(host, module)` — same pipeline as
  standalone, including the P1 world-scene event. Editor semantics for
  the scene: **PIE keeps the open scene** (that's the point of editing);
  if it differs from the server's `world_scene`, warn in the console
  (build-mismatch style), don't swap the user's content out from under
  them. Prediction collision may diverge on non-matching scenes — the
  warning says so; server stays authoritative regardless.
- **During play**: `net.update()` gates on
  `PlayMode::Playing | PlayMode::Paused` — excluding only `Edit`. Pause
  is a distinct mode (`app.rs:4816`), and freezing the event pump while
  paused would time out the connection; net proxies keep updating under
  pause (matches UE PIE behavior).
- **Exit play**: explicit `teardown()` (clean disconnect —
  `client.rs:433`, currently the only clean-close path and not wired to
  drop) then drop the `NetSession` *before* `restore_snapshot()`.
  Dropping is sufficient to stop world mutation (replication maps,
  projectile handles and prediction state are owned fields, `net.rs:81`),
  and `world.clear()` in restore deletes the proxies. Server-side cleanup
  is the proven M5 path. Re-entering play reconnects fresh — same
  identity slot, exactly the rejoin path M9.5 verified.

## Package 4 — Play As Listen Server

Launcher semantics (SpacetimeDB has no true listen server — the sim always
runs in the SpacetimeDB process):

- On play-enter: if the configured host is local and quiet, spawn
  `spacetime start` (child process, readiness-polled) and wipe-free
  publish via the `server/publish.ps1` steps driven from Rust
  (`std::process::Command`); surface progress in the console/status bar.
  Then proceed exactly as Play As Client.
- The spawned `spacetime start` **outlives play-exit** (matches
  `host_local.ps1`'s reuse-don't-stop rule; restarting per-play would be
  slow and destroys dev data for no reason). Publish reuses M9's retry
  gate.
- Non-local host + ListenServer = config error, refused at play-enter.

## Package 5 — Number of Players

- N−1 extra clients as child processes on play-enter (Client or
  ListenServer, N ≥ 2): prefer `build/export/game.exe` if present, else
  `cargo run --release -p game_client --`, each with
  `--connect <host> <module> --net-id editor_p<i>` (distinct slots — the
  M9.5 same-machine lesson).
- Children are killed on play-exit (tracked PIDs; kill is the harshest
  disconnect and M9.5 proved server cleanup handles it in seconds).

## Acceptance

- [x] Module announces `world_scene`; packaged mp-client with **no
      client-side scene knowledge beyond the pak** loads the right world
      (delete the hardcoded greybox path from `standalone.rs`)
- [x] Announced-scene-missing-from-pak refuses with a clear message;
      dead-server standalone falls back offline
- [x] Protocol v6 wipe-published; bindings + net_bots regenerated; smoke
      script (`smoke_packaged.ps1`) still passes
- [x] Play As Client: enter play → connected proxies visible in the open
      scene; exit play → clean disconnect, no proxies in edit mode, undo
      history sane; re-enter → rejoin without ghosts
- [x] Play As Listen Server from a cold machine (no spacetime running):
      one button → server up, module published, editor in world
- [x] Number of Players = 2 spawns a second client that appears in-world;
      both torn down on stop
- [x] Settings persist across editor restarts

## Close-out (2026-07-22)

Landed as five packages: P1 `98015e3` (server-announced world, protocol
v6, deferred standalone load + offline fallback, 15 s handshake timeout),
P2 `2391686` (PlaySettings persisted in `editor_layout_crusty.ron`,
play-cluster dropdown; crusty-gui popup outside-click fix `fd261b5`),
P3 `92c4138` (Play As Client: session per play run, `--connect` prefills
settings, one-shot scene-mismatch warning), P4 `f0e3637` (listen-server
launcher: `spacetime start` + publish off-thread, server outlives play,
non-local host refused), P5 `1b27b4b` (N−1 child clients on
`--net-id editor_p<i>` slots, killed on play-exit).

Verified live: cold-machine listen server (kill spacetime → one F5 →
published + in world), reuse path, remote-host refusal, 2-player child
spawn/kill, `smoke_packaged.ps1` PASS against the v6 export. Deliberate
deviation from the plan: PlaySettings lives solely on `CrustyDockLayout`
(single source of truth, auto-persisted) instead of duplicating on
`PlayModeState`. Known wart: PIE plays on the open scene by design — the
console warns once when it differs from the server's world scene.

## Out of scope

- Bundling `spacetime` binaries in the packaged game (redistribution
  license unchecked) — listen-server stays editor-only
- Multi-scene server content / scene rotation — one `world_scene` per
  module
- Editor auto-loading the server's scene on Play As Client (warn-only
  this milestone; revisit if the warning proves annoying)
- Dedicated-server orchestration UI (Maincloud dashboard covers it)

## Open questions (answered by recommendation unless overridden)

1. **Protocol bump for the Config column** → yes, v6; cheap here, and
   silent binding skew is exactly what version gates are for.
2. **PIE scene on mismatch: warn or swap?** → warn. Editing is the
   editor's job; swapping scenes under the user is hostile.
3. **Extra players: processes or in-proc?** → processes. In-proc
   multi-client means multi-window/multi-world in one process — a
   renderer-architecture project, not a play-settings feature.
