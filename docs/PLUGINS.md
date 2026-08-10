# Writing a Plugin

A plugin is a Rust type that registers engine functionality at startup. This
guide walks through the whole surface using `DevNodesPlugin` — the fixture
plugin at `engine/src/engine/plugins/dev_nodes.rs` — as the worked example.

Architecture context: [ARCHITECTURE.md ▸ Plugin System](ARCHITECTURE.md).
Design rationale: `roadmap/VULKANO-39.8-PLUGIN-SYSTEM.md`.

## The shape of a plugin

```rust
use crate::engine::plugins::{
    EnginePlugin, PluginContext, PluginError, PluginKind, PluginManifest, PluginOrigin,
};

pub struct DevNodesPlugin;

pub const DEV_NODES_ID: &str = "dev_nodes";

impl EnginePlugin for DevNodesPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new(DEV_NODES_ID, "Dev Nodes")
            .with_description("Fixture node types for exercising the node graph editor.")
            .with_author("rust-engine")
            .with_origin(PluginOrigin::Engine)
            .with_kind(PluginKind::EditorOnly)
            .with_module_path("engine/src/engine/plugins/dev_nodes.rs")
            .with_cargo_feature("dev_nodes")
    }

    fn build(&self, ctx: &mut PluginContext) -> Result<(), PluginError> {
        for descriptor in dev_node_descriptors() {
            ctx.register_node(descriptor);
        }
        Ok(())
    }
}
```

That is the whole contract: metadata, and one registration entry point.

## Two things decide whether your plugin runs

| | Mechanism | Changing it costs |
|---|---|---|
| **Inclusion** — is it compiled in? | Rust code + a Cargo feature | a rebuild |
| **Activation** — does it run? | `plugins` in `project.ron` | a restart |

Keep them separate. Feature-gate a plugin when you want the option of *not
shipping its code*; use the manifest for everything a user should be able to
turn off without a toolchain.

## Manifest fields that carry real behaviour

- **`id`** — permanent. It is written into `project.ron`, so renaming it
  orphans every existing entry and the new id defaults to enabled, silently
  reversing a user's disable. If you must rename, register an alias at
  `PluginSet` construction: `set.with_alias("old_id", "new_id")`.
- **`kind`** — `Runtime` ships with exported games; **`EditorOnly` never
  enters an export feature set, regardless of enabled state.** Fixtures,
  tooling and debug panels are `EditorOnly`. Getting this wrong is how test
  content reaches a shipped game.
- **`depends_on`** — plugin ids. A hole (absent, disabled, or itself failed)
  fails your plugin with `MissingDependency`. This cascades: `ClientGamePlugin`
  depends on `physics_rapier`, so disabling physics disables gameplay too.
- **`internal`** — the project's own game module. Runs through `PluginSet` for
  uniform mechanics, hidden from the Plugin Manager, not manifest-toggleable.
- **`origin`** — groups the manager's list (Engine plugins, then Project).
- **`module_path` / `cargo_feature`** — shown in the manager's Manifest facts.

## What you can register

### Systems

```rust
ctx.add_system(system, Stage::Update, SystemDescriptor::new("MySystem"));
ctx.add_system_with_criteria(system, Stage::PreUpdate, descriptor, RunIfPlaying);
```

Descriptors declare access so the schedule can validate conflicts. Ordering
constraints (`.after("OtherSystem")`) may reference core systems — but if you
`.after()` a system owned by *another plugin*, `depends_on` that plugin too,
or validation will reject the dangling edge when it is disabled.

### Resources

```rust
ctx.insert_resource(MyResource::new());
```

Colliding with a core or earlier-plugin resource is an **error**: a plugin
never silently replaces something else's state.

### Node types

```rust
ctx.register_node(descriptor);
ctx.register_domain_pin("shader");
ctx.register_migration("my_node", 1, |mig_ctx| { /* … */ });
```

A node id already taken is skipped with a warning (your plugin ends "enabled
with warnings") — a duplicate must not cost the user a whole plugin. A
duplicate migration key *is* an error: silently picking one would change how
documents upgrade.

### Content hooks

```rust
ctx.on_world_loaded(|world, resources| {
    // Runs after EVERY world population.
    Ok(())
});
```

Registration happens once at startup, when the world is empty. Anything that
must inspect loaded entities belongs here. `RapierPhysicsPlugin` uses it to
create rigid bodies. Failure is surfaced-and-continue in the editor, fatal in
a shipped game.

### Debug overlay

```rust
ctx.register_debug_draw(|world, buffer| {
    submit_collider_debug_draws(world, buffer);
});
```

### Editor panels and settings pages (`#[cfg(feature = "editor")]`)

```rust
ctx.register_panel("my_panel", "My Panel", || Box::new(MyPanel::default()));
ctx.register_settings_page("my_page", "My Plugin", || Box::new(MyPage));
```

A panel implements `PluginPanel::draw(&mut self, ui, rect, ctx)` and receives a
`PluginPanelCtx { world, resources, play_mode }`. The implementor *is* the
panel's state. Panel ids are permanent for the same reason plugin ids are —
they are written into `editor_layout_crusty.ron`.

## Registration is staged

`build()` must not touch live engine state, and it doesn't: everything above
writes into scratch storage. `PluginSet` commits the whole stage only if you
return `Ok`. Return `Err(PluginError::build("…"))` at any point and *nothing*
you registered lands — no half-registered plugin, no cleanup code for you to
write.

The editor surfaces failures in the console and the Plugin Manager and boots
anyway. A shipped game treats them as fatal.

## Registering your plugin

Plugins are added to the set the binary constructs
(`game_client/src/plugin.rs`):

```rust
pub fn client_plugin_set() -> PluginSet {
    let mut set = PluginSet::new();
    set.add(RapierPhysicsPlugin);
    #[cfg(feature = "dev_nodes")]
    set.add(DevNodesPlugin);
    set.add(ClientGamePlugin);
    set
}
```

Insertion order is the tie-break for dependency sorting; engine plugins go in
before the project's own module.

## Testing a plugin

Build it against scratch registries — no editor, no window:

```rust
let mut set = PluginSet::new();
set.add(MyPlugin);
set.build_all(
    PluginTargets {
        schedule: &mut schedule,
        resources: &mut resources,
        node_registry: &mut registry,
    },
    None, // or Some(&[PluginEntry::new("my_plugin", false)])
);
assert!(set.failures().is_empty());
```

`dev_nodes.rs` and `rapier/mod.rs` both carry tests worth copying: a
registry-parity golden, a disabled-registers-nothing check, and a cascade test
proving no ordering edge dangles when a dependency is switched off.

## Checklist

- [ ] `id` is permanent, or you added an alias.
- [ ] `kind` is `EditorOnly` unless it genuinely belongs in a shipped game.
- [ ] `depends_on` names every plugin you `.after()` or whose data you consume.
- [ ] `build()` returns `Err` rather than panicking on bad input.
- [ ] Content-dependent work is in `on_world_loaded`, not `build()`.
- [ ] A disabled-plugin test asserts nothing is registered.
