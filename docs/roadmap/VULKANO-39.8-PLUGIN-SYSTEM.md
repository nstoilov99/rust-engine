# Task 39.8 — Plugin System & Module Registry

**Status:** Plan — awaiting review
**Duration:** ~1.5–2 weeks
**Prerequisites:** Task 40 complete (registry contract), Refactor Checkpoint #6 complete (render graph owns pass execution — the shape future render-pass plugins will use)
**Plan owner's note:** supersedes the roadmap section's "flip = recompile" model. The
architecture decision (2026-08-10 discussion) is a **two-tier model**: toggling a
compiled-in plugin is **restart-only, never a rebuild** — a shipped editor must be
usable by non-programmers (artists). Cargo features are demoted from user-facing
toggle to packaging detail.

---

## 1. The two-tier model

**Tier 1 (this task): compiled-in plugins, activation-toggled.**
A plugin is a Rust crate compiled into the editor/game binary. Whether it *runs* is
decided at startup by the per-project manifest (`project.ron`). Toggling in the
Plugin Manager edits the manifest and prompts a restart. No toolchain on the user's
machine, ever. Disabled = never built into the running app: its `build()` is not
called, so nothing registers — no systems, no panels, no node types. Dormant code
in the binary is the accepted cost (editor binary size; shipped *game* binaries can
still strip via features at export, §8).

**Tier 2 (explicitly out of scope, shape-preserved): binary plugins.**
Third-party plugins not compiled into our build — Godot-GDExtension-style C-ABI
seam or WASM (Zellij/Extism precedent) at *narrow* extension points (node types,
importers). Nothing in this task may preclude it; nothing in this task implements
it. Concretely preserved: registration goes through runtime registry calls (never
compile-time inventory as the only path), plugin identity is a string id (not a
TypeId), and the `PluginContext` surface stays object-safe.

**Why not Unreal's DLL model for tier 1:** Rust has no stable ABI; generic APIs
monomorphize into the caller and can't cross a DLL boundary; the Task 40 registry
API is generic-adjacent and trait-heavy. Same-compiler dylib tricks
(hot-lib-reloader) are dev-only tools whose caveats (TypeId instability breaks the
ECS, vtables in long-lived state, Windows file locking) rule them out here. This is
the ecosystem consensus (Bevy's model), not a solo-dev shortcut.

---

## 2. Current state (from the 2026-08-10 seam audit)

What exists and is load-bearing for this task:

- **`GamePlugin` trait** (`engine/src/engine/plugin.rs:14`):
  `fn build(&self, schedule: &mut Schedule, resources: &mut Resources)`. Called
  once, for exactly one plugin, in both binaries (`app.rs:488`,
  `standalone.rs:242`), mid-schedule-build. This is the seed of the new trait; the
  call sites are already correctly placed relative to the Schedule.
- **`Schedule`** is a real builder (stages First..Last, `add_system_described_with_criteria`,
  access-declaration validation) — plugins can add systems today with no new
  infrastructure.
- **`NodeRegistry`** was built for this task: runtime `register()` is the primary
  API, reserved slugs enforced, duplicate-id errors typed, and Task 40's D3 rule
  ("a disabled plugin must not eat graphs" — unknown `type_id` degrades to a
  placeholder, never data loss) is already implemented and tested.
- **`ProjectConfig`/`project.ron`** exists (per-project, VCS-checked-in,
  Ctrl+S-dirty semantics) — the natural manifest home.
- **Init-order defect (editor only):** `app.rs` loads the scene (L372) *before*
  the Schedule (L460) and NodeRegistry (L693) exist. Standalone is clean
  (registries ready before deferred `load_world`). Plugins that register
  components/systems consumed by scene content need the editor fixed to
  registries-then-content.
- **Closed enums at two seams:** `EditorTab` (+ 4 hardcoded match sites across
  `dock_crusty.rs`/`app.rs`) and `AssetType`/loader dispatch. Neither has a
  registry today.
- **Export never passes `--features`:** `BuildTarget` only affects post-build
  copy/publish. Feature selection for lean exports is new work (§8).
- **Physics coupling surface** (the extraction candidate): `PhysicsWorld`
  resource, `PhysicsStepSystem`, components (`RigidBody`/`Collider`/…) used by
  scene serialization + inspector UI + play-mode snapshot/restore + debug draw.

---

## 3. Decisions

### D1 — `EnginePlugin` trait and `PluginContext`

One trait, replacing (absorbing) `GamePlugin`:

- `manifest(&self) -> PluginManifest` — static metadata: `id` (stable string slug,
  same identity rules as node type ids), `name`, `version` (semver string),
  `description`, `author`, `depends_on: Vec<String>` (plugin ids).
- `build(&self, ctx: &mut PluginContext) -> Result<(), PluginError>` — the single
  registration entry point.

`PluginContext` is a borrow-bundle over the seams a plugin may touch, **constructed
once at the registries-ready point** (D4): `schedule`, `resources`,
`node_registry`, plus the editor-only extension points behind
`#[cfg(feature = "editor")]`: panel factories (D6), settings pages (D6). It is a
plain struct of `&mut` references — no `Any`-map, no god-object; adding a seam
later is adding a field. Methods stay object-safe where the seam allows (tier-2
shape preservation); `Schedule::add_system` is generic and that is fine — tier 2
will wrap it, not call it.

**Staged commit (review finding):** `build()` must not mutate live engine state
directly — a partial registration followed by an error could not be "skipped".
`PluginContext` *stages* per-plugin (systems, resource inserts, node descriptors,
panel/settings factories, lifecycle callbacks into scratch collections);
`PluginSet` commits a plugin's stage only when its `build()` returns `Ok`. This
also yields the Plugin Manager's registration counts (D8) for free.

`build()` returns `Result`: a failing plugin is recorded (id + error + phase),
its stage discarded, and the editor boots and shows it in the Plugin Manager —
never crashes. (Standalone/game: failure is fatal with a clear message; a shipped
game missing a plugin it needs should not limp.)

**Lifecycle callbacks (review finding):** registration alone is not enough —
some plugin work runs at *content* moments, not startup. v1 adds exactly one:
`ctx.on_world_loaded(callback)`, invoked by the engine after any world
population (editor initial scene, standalone `load_world`, benchmark scene
loads, play-mode scene reset). Motivating case: Rapier's
`register_physics_entities` scans loaded entities to create bodies — under the
registries-then-content order (D4) it must re-run per load, not once.

### D2 — Registration set, v1

A plugin can register, in v1:
1. **ECS systems** (any stage, with run criteria and access declarations) — exists.
2. **Resources** (insert into `Resources`) — exists.
3. **Node types + domain pins + migrations** (`NodeRegistry`) — exists.
4. **Editor panels** (D6) — new seam.
5. **Settings pages** (D6) — new seam, same factory mechanism as panels.

**Not registrable in v1 (documented debts, not oversights):**
- **Component types in scenes.** Scene serialization and the inspector are
  hand-rolled per component; plugin components crossing them needs a
  name-keyed component registry with serialize/deserialize/inspect fns — a
  reflection-lite arc that would double this task. Rapier's components therefore
  *stay in engine core* (D7). Revisit when a second physics backend or a real
  third-party component need exists.
- **Asset types/importers.** `AssetType` stays closed; no candidate plugin needs
  it (Rapier no, Steam no, dev_nodes no). Noted as the first tier-2 extension
  point to design when needed.
- **Render passes.** Checkpoint #6's `add_pass_with` is the right shape, but pass
  registration from plugins waits for a real customer (Phase 12/13 tasks).

### D3 — Manifest: `plugins` in `project.ron`

`ProjectConfig` gains `plugins: Vec<PluginEntry>` — `{ id: String, enabled: bool }`.
Rules:
- Compiled-in plugins **absent from the manifest default to enabled** (batteries
  included: a fresh project gets physics without editing anything). The Plugin
  Manager materializes entries on first toggle.
- Manifest entries whose id matches **no compiled-in plugin** are *not* errors:
  shown in the Plugin Manager as "not present in this build" (grayed, with the
  manifest id). This is the tier-2/handoff case — a project moved between machines
  or engine builds must not lose its intent. They are preserved on save.
- Dependency closure is enforced at *toggle time* in the UI (disabling a plugin
  that others depend on warns and offers to disable the closure) and re-checked at
  startup (a dependency hole = the dependent plugin fails with
  `PluginError::MissingDependency`, boots disabled, surfaces in the manager).
- **Plugin ids are permanent** (same append-only convention as node type slugs).
  A rename would make the old id an orphan while the new id defaults to enabled —
  silently reversing a user's disable (review finding). Renames therefore require
  an alias entry at `PluginSet` construction (old id → new id, applied when
  reading the manifest); no alias, no rename.

### D4 — Startup: registries-then-content, dependency-ordered build

- **`PluginSet`** owns `Vec<Box<dyn EnginePlugin>>`. The binary (game_client)
  constructs it — plugin *inclusion* is still Rust code + Cargo features (that's
  tier 1); *activation* is the manifest filter.
- Build order: topological sort by `depends_on` (deterministic tie-break: manifest
  order, then id). Cycle = all plugins in the cycle fail with
  `PluginError::DependencyCycle`.
- **Unify the editor/standalone plugin position (review finding):** today the
  plugin call sits *before* transform propagation in the editor (`app.rs:488`)
  but *after* it in standalone (`standalone.rs` — after L230's additions). These
  genuinely execute differently. D4 picks the editor's order as canonical
  (gameplay systems that move entities should run before transform propagation);
  standalone moves to match. Deliberate, small behavior change — verified by
  diffing `print_access_report()` on both binaries before/after.
- **Editor init-order fix:** restructure `App::new()` so construction order is:
  core (window/renderer/assets) → ECS world (empty) → Schedule shell →
  NodeRegistry + editor registries → **`PluginSet::build_all(ctx)`** →
  engine-core systems that must bracket plugin systems (unchanged relative
  order: input/animation/physics-step before, transform-propagation/audio after —
  preserved by keeping today's pre/post split around the plugin call) →
  `schedule.validate()` → scene load → render thread. Scene load moves after
  registration; the audit found nothing in `load_or_create_scene` that the moved
  segments depend on, but this is the riskiest refactor in the task and gets its
  own package (P2) with editor + standalone + net-play smoke gates.
- Standalone calls the same `PluginSet::build_all` at its existing (already
  correct) hook point.

### D5 — dev_nodes becomes the first plugin (proof by conversion)

`dev_nodes` is already exactly a plugin: feature-gated, one `register_dev_nodes()`
call at startup. Convert it to `DevNodesPlugin` (id `"dev_nodes"`). Zero new
functionality; proves trait + manifest + node-registration + Plugin Manager end to
end before the hard extraction starts. Also becomes the doc example.

### D6 — Editor extension points: plugin panels and settings pages

Minimal open seam, not a full panel-system rewrite (Task 45.5 restructures editor
code later; don't pre-empt it):
- `EditorTab` gains **one** variant: `Plugin(String /* panel id */)`.
  `tab_id`/`parse_tab` map it as `"plugin:<id>"`. Unknown panel id on layout
  restore = the existing missing-document placeholder pattern (hydration lesson
  from Task 40: restored tabs must degrade visibly, not silently).
- `PluginContext::register_panel(id, title, factory)` where factory produces a
  `Box<dyn PluginPanel>`: `fn draw(&mut self, ui: …, world: …)` — the exact
  signature is fixed in P4 against what the two `app.rs` match sites actually pass
  panels today (audit: docked at ~L3543, floating at ~L4816). Both sites get one
  new arm dispatching through the panel registry. Panels appear in the existing
  Window/panels menu.
- Settings pages: same factory pattern into `SettingsState`'s page list — this is
  how a plugin ships its own preferences (and how the Plugin Manager itself is
  built, eating our own dogfood).

### D7 — Rapier extraction: prove the seam on the stepping lifecycle; the world stays core

The review pass killed the "no `PhysicsWorld` resource when disabled" version:
the resource is load-bearing far outside the plugin boundary — profiler counters
`expect()` it (`app.rs:2334`), play-mode enter/restore recreates it
(`app.rs:5909`, `:6059`), the benchmark loader removes/reinserts it
(`app.rs:5746`), standalone `load_world` rebuilds it (`standalone.rs:368`).
Hardening every site for an optional resource buys nothing — an unstepped
`PhysicsWorld` is already inert (edit mode proves this daily).

Revised split — `RapierPhysicsPlugin` (id `"physics_rapier"`) owns what
*advances* physics:
- `PhysicsStepSystem` registration (stage + run-criteria unchanged).
- Body/collider registration at content moments via `on_world_loaded` (D1) —
  today's `register_physics_entities`.
- Debug-draw submission (collider overlay) via a small engine-owned hook.

Engine core keeps: the `PhysicsWorld` resource itself (inserted always, inert
when unstepped), `rebuild_physics` plumbing around play mode (harmless when
nothing steps), component definitions, scene serialization, inspector UI. With
the plugin disabled: scenes with physics components load/save/display losslessly,
nothing moves, no handles are created. The inspector shows a passive note on
physics components ("physics_rapier disabled — components inert").

**Cascade (review finding — critical):** the game's own systems hard-depend on
physics: `PlayerInputSystem` declares `.after(PHYSICS_STEP)`
(`player_input.rs:108`) and schedule validation rejects ordering edges to absent
systems (`schedule.rs:390`); `CharacterMovementSystem` writes `PhysicsWorld` and
consumes Rapier handles. So `ClientGamePlugin` declares
`depends_on: ["physics_rapier"]`, and disabling physics disables the gameplay
systems with it (the manager shows the cascade per D3). "Physics off" is a
scene-editing/tooling configuration, not a playable one — which is the honest
truth of this codebase.

**Honesty clause:** this is a stepping-lifecycle extraction (system + content
hooks + editor overlay), not a full componentized decoupling. The roadmap's
"extracting Rapier proves the seam on the hardest case" is amended: it proves
registration, dependency cascade, lifecycle callbacks, and the manifest/restart
loop on the most-coupled subsystem. Full component extraction is the
reflection-lite arc, deliberately out (D2).

### D8 — Plugin Manager UI

Settings-window page (design brief already sent to the design pass; states below
are the contract, matching D1/D3):
- Enabled / Disabled / **Pending restart** (manifest changed this session) /
  **Failed** (build error, with phase + message) / **Blocked: missing dependency**
  (names it) / **Not in this build** (manifest orphan) / Enabled-with-warnings
  (e.g. duplicate node id — `RegistryError` surfaced, not fatal).
- Toggle writes `ProjectConfig` (dirty → Ctrl+S semantics like every project
  setting), banner offers **Restart Now** — which is scoped to *relaunch the
  editor process* (spawn self + exit; the M9.6 listen-server launcher already
  spawns sibling processes, reuse that mechanism). Unsaved-changes check runs
  first (normal close path).
- Registration counts per plugin (N systems, N node types, N panels) — collected
  by `PluginContext` during `build()`, zero bookkeeping for plugin authors.

### D9 — Export integration: features become the packaging tool

Extend `run_cargo_build` (build_dialog.rs) and `scripts/export_windows.ps1` to
pass an explicit feature set. Two corrections from review:
- `game_client`'s `default` features include non-plugin `hud` — naive
  `--features` addition can't strip a default-enabled plugin, and
  `--no-default-features` alone would strip the HUD. Policy: export builds pass
  `--no-default-features --features <base + enabled plugin features>`, where
  *base* is a declared constant list of the non-plugin defaults (today: `hud`).
  The plugin id ↔ feature mapping lives at `PluginSet` construction.
- Exports don't ship `project.ron` (only `content/` → `game.pak` +
  `net_config.ron`), so there is no runtime manifest in a shipped game —
  and none is needed: activation is resolved *at build time*. Every plugin
  compiled into an export is enabled; disabled plugins aren't in the binary.
  Standalone's `PluginSet` therefore skips the manifest filter entirely.

Disabled plugin ⇒ compiled out of the *shipped game* — the zero-cost story
survives exactly where it matters. The editor binary itself stays
batteries-included. `MpServer` (WASM publish) is unaffected.

### D10 — Non-goals

- No dynamic loading of any kind (DLL, WASM) — tier 2, future task.
- No hot reload / no toggling without restart.
- No marketplace, install, download, versioned plugin distribution.
- No plugin-provided components in scenes, asset types, or render passes (D2).
- No per-plugin sandboxing or capability model — compiled-in code is trusted.
- No migration of engine-core systems (input/animation/transform/audio) into
  plugins — core stays core; only the three roadmap candidates are plugin-shaped,
  and only Rapier + dev_nodes are in scope (Steam SDK and the Ability System
  remain listed as follow-on candidates, unblocked by this task).

---

## 4. Packages

**P1 — Trait + PluginSet + manifest.** `EnginePlugin`, `PluginManifest`,
`PluginError`, `PluginContext` as a *staging* context (systems/resources/
node-registry + `on_world_loaded`), `PluginSet` with topo-ordered `build_all`,
commit-on-Ok, failure capture. `plugins` field in `ProjectConfig` (serde-default:
empty = all enabled) with orphan preservation + id-alias table. `GamePlugin`
absorbed: `ClientGamePlugin` ported (now `depends_on: ["physics_rapier"]` per
D7), old trait deleted; standalone's plugin position unified with the editor's
(D4). *Accept:* unit tests — ordering, cycle error, missing dep, partial-failure
discards the stage (no half-registered plugin), manifest round-trip incl.
orphans + alias; both binaries build all combos; access-report diff shows only
the intended standalone reorder.

**P2 — Editor init-order refactor.** Registries-then-content in `App::new()` per
D4. *Accept:* editor boots, scene loads, play mode works, net play (M9.6
dropdown) works, standalone unchanged; `schedule.validate()` clean; no
double-registration on scene reload.

**P3 — dev_nodes conversion (D5).** *Accept:* `dev_nodes` feature builds produce
identical registry contents via the plugin path (golden: same create-menu
inventory); toggling it off in the manifest → demo graphs open with placeholder
nodes, no data loss on save (Task 40 D3 behavior, now exercised for real).

**P4 — Editor extension points (D6).** `EditorTab::Plugin`, panel/settings
factories, dispatch arms, layout persistence + missing-panel placeholder.
*Accept:* a test plugin registers a panel + settings page; dock/undock/float/
restore-across-restart; disable plugin → placeholder tab, layout not corrupted.

**P5 — Rapier extraction (D7).** Plugin crate/module layout decided here
(recommendation: `engine/src/engine/plugins/rapier/` module first, workspace
crate only if the dependency graph forces it — avoid pre-emptive crate
proliferation). `on_world_loaded` wiring at all four content moments (editor
initial, standalone `load_world`, benchmark loads, play-mode reset), debug-draw
hook. *Accept:* physics on = identical behavior (existing physics tests +
play-mode smoke + access-report diff); physics off = gameplay-systems cascade
disables cleanly (validation passes — the `PlayerInputSystem.after(PHYSICS_STEP)`
edge must not dangle), editor boots and edits, scenes with physics components
load/save losslessly, inspector shows inert note, `PhysicsWorld` exists but
never steps and creates no handles; toggle both ways via manifest + restart.

**P6 — Plugin Manager UI (D8) + restart flow.** Built as a settings page via P4's
own seam. Reconcile with the external design pass (screenshots → review loop)
before polish. *Accept:* every D8 state reachable and visually distinct; toggle →
save → restart → new state active; failed-plugin path exercised by a
deliberately-failing test plugin.

**P7 — Export features (D9) + close-out.** Feature passthrough in build dialog +
script; docs (`docs/ARCHITECTURE.md` plugin section, roadmap close-out, plugin
author how-to with dev_nodes as the example). *Accept:* export with physics
disabled produces a binary without Rapier symbols (verify via binary size drop +
`strings`/symbol check); with enabled, plays the M9 co-op smoke.

Suggested order: P1 → P2 → P3 (early end-to-end proof) → P4 → P6-skeleton (needs
P4) → P5 → P6-polish → P7. Commit-per-package with the standard gates (test
suite, all feature combos, clippy-no-new, design lint, editor+standalone smoke).

---

## 5. Risks / open questions

1. **P2 is the risk concentration.** Moving scene load after registration touches
   the most-trafficked constructor in the codebase. Mitigation: it's an isolated
   package with the widest gate set, done before any new features depend on it.
2. **Panel factory signature** depends on what the two match sites actually need
   (`&mut` into CoreApp state). If a clean object-safe signature isn't extractable
   without threading half of `App` through, fallback: panels get a narrow
   `PanelCtx` struct built per-frame at the dispatch site (same pattern as
   Checkpoint #6's `PassContext`).
3. **Relaunch mechanism on Windows** (spawn-self + exit while holding file locks
   on layout/prefs): write-then-spawn-then-exit ordering matters; reuse and
   harden the M9.6 launcher path.
4. **Open for review:** should `ClientGamePlugin`'s systems (player input,
   character movement) become a *manifest-visible* plugin ("game" — toggleable,
   weird) or stay a hardwired `PluginSet` entry not shown in the manager
   (recommended: hardwired, `internal: true` flag on the manifest struct)?
5. **Open for review:** module vs workspace crate for `RapierPhysicsPlugin` (P5
   recommends module-first; a crate is more honest tier-1 "plugin shape" but adds
   workspace churn — cheap to revisit at P5 kickoff).
