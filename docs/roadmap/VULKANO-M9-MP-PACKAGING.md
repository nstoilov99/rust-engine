# M9 — Multiplayer Packaging: Client & Server Build Targets

**Status:** 📋 Ready to implement (2026-07-20) — gpt-5.6-Sol/Codex review
reconciled (8 findings); all open questions ruled 2026-07-20. Editor Net
Mode play settings split off as follow-up task M9.6.

Extends Task 25's Windows export (✅ `game.exe` + `game.pak`) with build
*targets* so the Co-op Slice can ship artifacts instead of dev checkouts.
SpacetimeDB shape note: there is no server executable to compile — the
"server build" is the WASM module plus a publish step. M9 is scripts, a
connection config file, and version stamping; it deliberately writes very
little engine code. Verification of the artifacts is M9.5, not here.

## Current state (verified 2026-07-20)

- **Export pipeline (Task 25).** Two *independent* implementations of the
  same packaging: `scripts/export_windows.ps1` / `.sh` (`-OutputDir`,
  `-Profile release|shipping`) and the editor `File > Build Game` dialog
  (`engine/src/engine/editor/build_dialog.rs`). Both build `game.exe`,
  cook collision for every scene, pack the whole `content/` tree into
  `game.pak`, and drop exe + DLLs + pak into `build/export/` — but the
  scripts fall back to loose `content/` when `pak_tool` fails, while the
  dialog errors out. Root configs (`window_config.ron` etc.) are not
  bundled.
- **Standalone startup is offline by default.** `game_client/src/main.rs`
  (`init_asset_source`) prefers `game.pak` next to the exe; networking only
  exists if `--connect` is on the command line.
- **Connection selection is CLI-only.** `NetSession::from_args`
  (game_client/src/net.rs:70) parses `--connect [host [module]]`;
  `DEFAULT_HOST = "http://127.0.0.1:3000"`, `DEFAULT_MODULE =
  "rust-engine-dev"` (net.rs:21-22). No env var, no config file. Both
  editor (`app.rs:658`) and standalone (`standalone.rs`) go through it.
- **Server module** is the workspace-excluded crate `server/game_module`.
  `server/publish.ps1` does `spacetime publish --module-path
  server/game_module rust-engine-dev` (`-Wipe` → `--delete-data=always`),
  prints the wasm size. Host and module name are hardcoded to the local
  dev instance; publish is the only build path (no build-only script).
- **Versioning today.** `game_shared/src/net/schema.rs:7` has a manually
  bumped `PROTOCOL_VERSION = 4`, stored by the module and **exact-match
  checked** by clients at connect (`version_compatible` — refuses on
  mismatch). The Config handshake happens in
  `game_client_net/src/client.rs:837`, *not* in `game_client`. The
  client's `game_client/build.rs` already stamps `GIT_HASH` and
  `BUILD_PROFILE` (full hash, no dirty detection), but only benchmark
  metadata reads them. The module has no build id — and note it already
  has a substantial `build.rs` (embedded collision registry, M6 D1), so
  D4 extends it rather than adding one. The `Config` row is written only
  by the `init` reducer (lib.rs:411), which does **not** run on
  incremental publishes.

## Design

### D1. `net_config.ron` — client connection config

A RON file next to the exe, editable post-build (the roadmap requirement:
repoint a shipped client at another server with a text editor).

```ron
NetConfig(
    host: "http://127.0.0.1:3000",
    module: "rust-engine-dev",
    auto_connect: false,
)
```

- Loaded from `<exe dir>/net_config.ron` (same discovery rule as
  `game.pak`); absent file = today's behavior, no error.
- Precedence is **field-wise**: CLI positional > config value > compiled
  default, per field. `--connect example.com` with a config present takes
  host from CLI and module from the config; bare `--connect` takes both
  from the config.
- `auto_connect: true` makes the standalone client connect at startup with
  no CLI args — double-click-the-exe multiplayer. The check lives in the
  standalone startup path (`standalone.rs`, already
  `cfg(not(feature = "editor"))`), *not* inside `from_args` — the editor
  never reads `auto_connect` because the code that would act on it isn't
  compiled in. (The editor still honors host/module from a config beside
  the dev exe when `--connect` is passed; that's a feature, not a leak.)
- Structure for testability: `NetConfig::parse(&str)` +
  `resolve(cli_positionals, config) -> ModuleAddr` as pure functions in
  `game_client/src/net.rs`; only the `<exe dir>` lookup touches the
  filesystem. RON via the serde already in the client. Unit tests:
  field-wise precedence, missing file, malformed file (warn + ignore,
  don't crash a shipped client).

### D2. Export targets (`-Target standalone|mp-client`)

`scripts/export_windows.ps1` / `.sh` grow a `-Target` parameter:

- **`standalone`** (default): today's pipeline — plus it now **deletes**
  any `net_config.ron` in the output dir, so re-exporting standalone over
  a previous mp-client export can't silently ship an auto-connecting
  build. Targets own their marker files.
- **`mp-client`**: standalone bundle + writes `net_config.ron` into the
  output dir (`-ServerUri` / `-Module` script params fill it; defaults
  point at localhost dev). Written via validated inputs — module name must
  match SpacetimeDB's `^[a-z0-9]+(-[a-z0-9]+)*$`, host must parse as a
  URI — so string templating can't produce broken RON. Nothing else
  differs: the exe is the same binary; the target is configuration, not
  compilation.
- The editor Build dialog mirrors the targets in D6/P5; the scripts stay
  the packaging authority (the dialog reproduces the same steps, it does
  not gain extra behavior).
- Pre-existing Task 25 wart, explicitly *not* fixed here: a failed pak
  step falls back to loose `content/`, and a later successful pak leaves
  the stale loose tree behind (runtime prefers the pak). Noted for M9.5's
  smoke script to assert on; cleaning it up is not an M9 deliverable.

### D3. `mp-server` target (`server/publish.ps1` generalization)

- `publish.ps1` gains `-Server <uri>` (default: local standalone) and
  `-Module <name>` (default `rust-engine-dev`) → `spacetime publish
  -s <uri> --module-path ... <name>`. `-Wipe` unchanged. Local dev and
  the rented host become the same command with different `-Server`.
- No new script: "mp-server build target" *is* this parameterized publish,
  per the SpacetimeDB shape note.

### D4. Version stamping

`PROTOCOL_VERSION` already hard-gates schema compatibility. M9 adds a
**soft** build id for drift the protocol number can't see (motion-config
parity, content changes, forgotten bumps):

- **Stamping**: the module's *existing* `build.rs` (collision registry)
  is extended to emit `GIT_HASH` — `git rev-parse --short HEAD`, suffixed
  `-dirty` when `git status --porcelain` is non-empty, `"unknown"` when
  git is unavailable. `game_client/build.rs` gets the same dirty suffix
  (today it stamps the bare hash) and correct `rerun-if-changed` on the
  workspace `.git/HEAD` + `.git/index` (the current package-relative
  paths don't exist).
- **Storage + refresh**: new `build_id: String` column on the existing
  `Config` row. `init` writes it — but `init` only runs on first publish,
  so a plain `set_build_id` reducer (no args) copies the embedded
  constant into `Config`, and `publish.ps1` calls it after every publish
  (`spacetime call <module> set_build_id`). The value always reflects the
  wasm actually deployed; the script only triggers the write. Guard:
  `init` records the owner identity in `Config`; `set_build_id` rejects
  other senders.
- **Protocol impact**: a `Config` column is a schema/wire change ⇒
  `PROTOCOL_VERSION` 4 → 5, regenerate + commit
  `game_client_net/src/module_bindings`, wipe-publish, update affected
  tests — same drill as every M6/M7 schema package.
- **Client check**: the Config handshake is in `game_client_net`
  (client.rs:837), which knows nothing of `game_client`'s `GIT_HASH`.
  API: the connect call takes an `expected_build_id: Option<String>`;
  on Config arrival a mismatch emits a new
  `NetEvent::BuildMismatch { server: String, client: String }`.
  `game_client` passes `option_env!("GIT_HASH")` and surfaces the event
  as a console warning + HUD status note. Mismatch **warns**, never
  refuses — protocol mismatch refuses, build mismatch informs.

### D5. Host-locally convenience (`scripts/host_local.ps1`)

The Unreal listen-server analogue, against *packaged* artifacts. Params:
`-ExportDir` (default `build/export`), `-Module`, `-Wipe`.

1. If nothing listens on :3000, `Start-Process spacetime start` in its own
   window (user-owned; the script never kills it — repeated runs reuse a
   running instance).
2. Readiness: retry `server/publish.ps1` (which forwards `-Module`/
   `-Wipe`) for up to ~30 s — publish succeeding *is* the readiness
   check; no separate port probe.
3. Launch `<ExportDir>/game.exe --connect http://127.0.0.1:3000 <module>`
   — explicit positionals, so a stale exported `net_config.ron` can't
   redirect the local loop.

Fails loudly if `<ExportDir>/game.exe` is missing (run the export first —
the script does not build). This is the packaged play-test loop M9.5's
smoke script will automate assertions around.

### D6. Build dialog targets (UE-style)

`File > Build Game` grows a **Target** dropdown next to platform/profile:

- **Standalone**: today's dialog behavior + the same `net_config.ron`
  cleanup as D2.
- **MP Client**: adds Server URI + Module text fields (same validation
  rules as D2, defaults from `net.rs` constants); writes `net_config.ron`
  into the output dir after packing.
- **MP Server**: no exe build at all — runs `server/publish.ps1
  -Server ... -Module ...` (D3) as a child process, streaming its output
  into the dialog's existing build log. Requires the `spacetime` CLI on
  PATH; a missing CLI is a normal build-log error, not a crash.

The dialog performs the same steps as the scripts, in the same order;
any future divergence is a bug in the dialog (scripts are authority).

## Packages (one commit each)

1. **P1 — client net config**: D1 (`NetConfig` load + precedence + tests).
2. **P2 — export targets**: D2 (`-Target`/`-ServerUri`/`-Module` in both
   export scripts, template writing).
3. **P3 — server target + version stamp**: D3 + D4 (publish params,
   `build.rs` extensions, `Config.build_id` + `set_build_id` reducer,
   `PROTOCOL_VERSION` 4 → 5 + bindings regen, `NetEvent::BuildMismatch`
   + client warn; wipe-publish).
4. **P4 — host-local loop**: D5.
5. **P5 — build dialog targets**: D6 + doc close-out (mark M9 complete,
   roadmap + CLAUDE.md).

## Open questions — all ruled 2026-07-20

1. Build-id mismatch: **warn-only everywhere**; `PROTOCOL_VERSION` stays
   the sole refusal gate.
2. `mp-client` exports default **`auto_connect: true`** (an mp-client
   bundle that starts offline is surprising; the standalone target has
   no config file at all).
3. The Build dialog **does** get targets — D6/P5, UE-style.

## Follow-up: M9.6 — Editor Net Play Modes (separate task)

UE-style **Net Mode** in the editor play settings, deliberately *not* an
M9 package because it's engine code (play/edit state + `NetSession`
lifecycle), while M9 is packaging. Depends on M9's D1/D3/D5 pieces.
Scope sketch (full plan doc when M9 ships):

- **Play Standalone**: today's play mode, untouched.
- **Play As Client**: entering play mode creates a `NetSession` against
  the configured host/module; exiting tears it down. The hard part:
  `NetSession` is currently constructed once at startup from CLI args
  (`app.rs:658`), and play-mode snapshot/restore must handle net-spawned
  proxies. This is play/edit-state work — main-model territory.
- **Play As Listen Server**: Play-As-Client + the D5 auto-start logic
  (spawn `spacetime start` if :3000 is quiet, publish, connect). Note
  SpacetimeDB has no true listen server — the sim always runs in the
  SpacetimeDB process; this is a launcher, which is also why it works.
- **Number of Players**: spawn N−1 extra standalone client processes
  (packaged exe if exported, else `cargo run`) with `--connect`.
- Out of scope for M9.6: shipping a listen-server *launcher inside the
  packaged game UI* (bundling `spacetime` binaries — check SpacetimeDB's
  redistribution license first).

## Acceptance

- Three commands from a clean checkout produce: a standalone bundle, an
  mp-client bundle whose `net_config.ron` repoints without recompiling,
  and a published module on a chosen host.
- `host_local.ps1` gives the one-command packaged play-test loop.
- A client whose git hash differs from the module's logs a visible warning
  at connect; protocol mismatch still refuses.
