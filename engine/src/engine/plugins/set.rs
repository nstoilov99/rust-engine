//! [`PluginSet`] — owns the compiled-in plugins, resolves activation and
//! dependency order, and commits each plugin's stage.
//!
//! The binary constructs the set (plugin *inclusion* is Rust code + Cargo
//! features); the per-project manifest decides *activation*.

use std::collections::{BTreeMap, HashSet};

use crate::engine::ecs::resources::Resources;
use crate::engine::ecs::schedule::Schedule;
use crate::engine::node_graph::NodeRegistry;

use super::context::{PluginContext, PluginCounts, WorldLoadedFn};
use super::{
    EnginePlugin, PluginEntry, PluginError, PluginFailure, PluginManifest, RegistrationPhase,
};

/// The live registries a plugin's stage is committed into.
///
/// All three exist before any plugin builds (P2's registries-then-content
/// order), so none of them is optional.
pub struct PluginTargets<'a> {
    pub schedule: &'a mut Schedule,
    pub resources: &'a mut Resources,
    pub node_registry: &'a mut NodeRegistry,
}

/// A plugin that came up, with what it registered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginRecord {
    pub manifest: PluginManifest,
    pub counts: PluginCounts,
    /// Non-fatal collisions — the "enabled with warnings" state.
    pub warnings: Vec<String>,
}

/// Owns the compiled-in plugins and the outcome of building them.
///
/// Lives beside `GameWorld` in `App`/`StandaloneApp`, because it also owns the
/// `on_world_loaded` callbacks that run at every content moment.
#[derive(Default)]
pub struct PluginSet {
    plugins: Vec<Box<dyn EnginePlugin>>,
    /// Old id -> new id, applied when reading the manifest. Without one, a
    /// rename orphans the old entry and the new id defaults to enabled —
    /// silently reversing a user's disable.
    aliases: BTreeMap<String, String>,
    records: Vec<PluginRecord>,
    failures: Vec<PluginFailure>,
    /// Ids switched off by the manifest (never built).
    disabled: Vec<String>,
    /// Manifest ids matching no compiled-in plugin. Not an error: a project
    /// moved between builds must not lose its intent.
    orphans: Vec<String>,
    world_loaded: Vec<(String, WorldLoadedFn)>,
    debug_draws: Vec<(String, super::context::DebugDrawFn)>,
    /// Live panels contributed by built plugins (D6). A disabled or failed
    /// plugin contributes none — which is exactly what makes a restored
    /// layout degrade to a missing-panel placeholder instead of silently
    /// dropping the tab.
    #[cfg(feature = "editor")]
    panels: Vec<super::panel::PluginPanelEntry>,
    #[cfg(feature = "editor")]
    settings_pages: Vec<super::panel::PluginSettingsEntry>,
}

impl PluginSet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a compiled-in plugin. Insertion order is the "manifest order"
    /// tie-break for dependency sorting.
    pub fn add(&mut self, plugin: impl EnginePlugin + 'static) -> &mut Self {
        self.plugins.push(Box::new(plugin));
        self
    }

    /// Record a plugin id rename, so a manifest entry written against the old
    /// id keeps applying to the renamed plugin.
    pub fn with_alias(&mut self, old_id: impl Into<String>, new_id: impl Into<String>) -> &mut Self {
        self.aliases.insert(old_id.into(), new_id.into());
        self
    }

    fn resolve<'a>(&'a self, id: &'a str) -> &'a str {
        self.aliases.get(id).map(String::as_str).unwrap_or(id)
    }

    /// Build every enabled plugin, in dependency order.
    ///
    /// `filter` is the `project.ron` manifest; `None` skips activation
    /// filtering entirely (exports resolve activation at build time — a
    /// shipped game has no runtime manifest and needs none).
    ///
    /// Never panics and never returns `Err`: per-plugin outcomes land in
    /// [`records`](Self::records) and [`failures`](Self::failures), so the
    /// editor can boot and show what broke. Standalone callers should treat a
    /// non-empty `failures()` as fatal.
    pub fn build_all(&mut self, mut targets: PluginTargets<'_>, filter: Option<&[PluginEntry]>) {
        self.records.clear();
        self.failures.clear();
        self.disabled.clear();
        self.orphans.clear();
        self.world_loaded.clear();
        self.debug_draws.clear();
        #[cfg(feature = "editor")]
        {
            self.panels.clear();
            self.settings_pages.clear();
        }

        // Take ownership so per-plugin commits can borrow `self` mutably.
        let plugins = std::mem::take(&mut self.plugins);
        let manifests: Vec<PluginManifest> = plugins.iter().map(|p| p.manifest()).collect();

        // --- activation ---
        let mut enabled: Vec<usize> = Vec::new();
        for (idx, manifest) in manifests.iter().enumerate() {
            if self.is_enabled(&manifest.id, manifest.internal, filter) {
                enabled.push(idx);
            } else {
                self.disabled.push(manifest.id.clone());
            }
        }
        if let Some(entries) = filter {
            let known: HashSet<&str> = manifests.iter().map(|m| m.id.as_str()).collect();
            for entry in entries {
                if !known.contains(self.resolve(&entry.id)) {
                    self.orphans.push(entry.id.clone());
                }
            }
        }

        // --- dependency order ---
        let (order, cycle) = Self::topological_order(&manifests, &enabled);
        let cycle_members: Vec<String> = cycle.iter().map(|&i| manifests[i].id.clone()).collect();
        for &idx in &cycle {
            self.failures.push(PluginFailure {
                id: manifests[idx].id.clone(),
                error: PluginError::DependencyCycle {
                    members: cycle_members.clone(),
                },
                phase: RegistrationPhase::Dependency,
            });
        }

        // --- build ---
        let mut succeeded: HashSet<String> = HashSet::new();
        for idx in order {
            let manifest = &manifests[idx];

            if let Some(missing) = manifest
                .depends_on
                .iter()
                .find(|dep| !succeeded.contains(dep.as_str()))
            {
                self.failures.push(PluginFailure {
                    id: manifest.id.clone(),
                    error: PluginError::MissingDependency {
                        plugin: manifest.id.clone(),
                        missing: missing.clone(),
                    },
                    phase: RegistrationPhase::Dependency,
                });
                continue;
            }

            let mut ctx = PluginContext::new();
            let outcome = match plugins[idx].build(&mut ctx) {
                Ok(()) => self.commit(manifest, ctx, &mut targets),
                Err(error) => Err(error),
            };

            match outcome {
                Ok(record) => {
                    succeeded.insert(manifest.id.clone());
                    self.records.push(record);
                }
                // The stage is dropped with `ctx` — nothing is half-registered.
                Err(error) => self.failures.push(PluginFailure {
                    id: manifest.id.clone(),
                    phase: error.phase(),
                    error,
                }),
            }
        }

        self.plugins = plugins;
    }

    /// Preflight everything, then commit. Nothing after the first mutation can
    /// fail, so an `Err` here leaves the live registries untouched.
    fn commit(
        &mut self,
        manifest: &PluginManifest,
        mut ctx: PluginContext,
        targets: &mut PluginTargets<'_>,
    ) -> Result<PluginRecord, PluginError> {
        let mut counts = ctx.staged_counts();

        // Preflight: resource collisions (core, or an earlier plugin).
        for (type_id, type_name) in &ctx.resource_names {
            if targets.resources.contains_id(*type_id) {
                return Err(PluginError::at(
                    RegistrationPhase::Resources,
                    format!(
                        "resource '{type_name}' is already provided by the engine or an \
                         earlier plugin; a plugin may not replace it"
                    ),
                ));
            }
        }

        // Preflight: panel / settings-page id collisions. Unlike a duplicate
        // node type — where skipping protects a document that references it —
        // nothing user-authored is at stake here, and two panels fighting over
        // one dock tab id is a plain bug. So it fails the plugin.
        #[cfg(feature = "editor")]
        {
            for (id, _, _) in &ctx.panels {
                if self.panels.iter().any(|p| &p.id == id) {
                    return Err(PluginError::at(
                        RegistrationPhase::Panels,
                        format!("panel id '{id}' is already registered by another plugin"),
                    ));
                }
            }
            for (id, _, _) in &ctx.settings_pages {
                if self.settings_pages.iter().any(|p| &p.id == id) {
                    return Err(PluginError::at(
                        RegistrationPhase::Panels,
                        format!("settings page id '{id}' is already registered by another plugin"),
                    ));
                }
            }
        }

        // Node registry: preflights and commits atomically inside merge_staged.
        let mut warnings = Vec::new();
        if ctx.touches_node_registry() {
            let staged = std::mem::take(&mut ctx.registry);
            let report = targets
                .node_registry
                .merge_staged(&manifest.id, staged)
                .map_err(|e| PluginError::at(RegistrationPhase::NodeRegistry, e.to_string()))?;
            counts.node_types = report.nodes_registered;
            counts.domain_pins = report.domain_pins_registered;
            counts.migrations = report.migrations_registered;
            warnings = report.warnings;
        }

        // Commit.
        targets.schedule.append_from(ctx.schedule);
        targets.resources.append_from(ctx.resources);
        for callback in ctx.world_loaded {
            self.world_loaded.push((manifest.id.clone(), callback));
        }
        for callback in ctx.debug_draws {
            self.debug_draws.push((manifest.id.clone(), callback));
        }
        #[cfg(feature = "editor")]
        {
            for (id, title, factory) in ctx.panels {
                let panel = factory();
                self.panels.push(super::panel::PluginPanelEntry {
                    plugin_id: manifest.id.clone(),
                    id,
                    title,
                    panel,
                });
            }
            for (id, title, factory) in ctx.settings_pages {
                let page = factory();
                self.settings_pages.push(super::panel::PluginSettingsEntry {
                    plugin_id: manifest.id.clone(),
                    id,
                    title,
                    page,
                });
            }
        }

        Ok(PluginRecord {
            manifest: manifest.clone(),
            counts,
            warnings,
        })
    }

    /// Manifest lookup. Absent id = enabled (batteries included: a fresh
    /// project gets everything without editing anything). Internal plugins are
    /// never manifest-toggleable.
    fn is_enabled(&self, id: &str, internal: bool, filter: Option<&[PluginEntry]>) -> bool {
        if internal {
            return true;
        }
        match filter {
            None => true,
            Some(entries) => entries
                .iter()
                .find(|e| self.resolve(&e.id) == id)
                .map(|e| e.enabled)
                .unwrap_or(true),
        }
    }

    /// Kahn's algorithm over `depends_on`, restricted to the enabled set.
    /// Returns `(build order, unresolved indices)`. Ties break by manifest
    /// order (which is unique here, so the id tie-break never fires).
    ///
    /// Dependencies outside the enabled set are ignored for *ordering* — they
    /// surface as `MissingDependency` at build time, which also covers a
    /// dependency that built and failed.
    fn topological_order(
        manifests: &[PluginManifest],
        enabled: &[usize],
    ) -> (Vec<usize>, Vec<usize>) {
        let position: BTreeMap<&str, usize> = enabled
            .iter()
            .map(|&i| (manifests[i].id.as_str(), i))
            .collect();

        let mut remaining: Vec<usize> = enabled.to_vec();
        let mut order: Vec<usize> = Vec::with_capacity(remaining.len());
        let mut done: HashSet<&str> = HashSet::new();

        loop {
            // First remaining plugin (manifest order) whose in-set deps are done.
            let next = remaining.iter().position(|&i| {
                manifests[i].depends_on.iter().all(|dep| {
                    !position.contains_key(dep.as_str()) || done.contains(dep.as_str())
                })
            });
            match next {
                Some(pos) => {
                    let idx = remaining.remove(pos);
                    done.insert(manifests[idx].id.as_str());
                    order.push(idx);
                }
                None => break,
            }
        }

        (order, remaining)
    }

    // === Outcome, for the Plugin Manager ===

    pub fn records(&self) -> &[PluginRecord] {
        &self.records
    }

    pub fn failures(&self) -> &[PluginFailure] {
        &self.failures
    }

    /// Ids switched off by the manifest.
    pub fn disabled(&self) -> &[String] {
        &self.disabled
    }

    /// Manifest ids present in `project.ron` but not in this build.
    pub fn orphans(&self) -> &[String] {
        &self.orphans
    }

    /// Every compiled-in plugin, enabled or not.
    pub fn manifests(&self) -> Vec<PluginManifest> {
        self.plugins.iter().map(|p| p.manifest()).collect()
    }

    // === Editor extension points (D6) ===

    /// `(tab id, title)` for every live plugin panel — feeds View ▸ Panels and
    /// the dock tab strip.
    #[cfg(feature = "editor")]
    pub fn panel_menu_entries(&self) -> Vec<(String, String)> {
        self.panels
            .iter()
            .map(|p| (p.tab_id(), p.title.clone()))
            .collect()
    }

    /// Look up a live panel by its panel id (the part after `plugin:`).
    /// `None` means "no plugin registered this id in this session" — the
    /// dispatch sites turn that into a missing-panel placeholder.
    #[cfg(feature = "editor")]
    pub fn panel_mut(&mut self, id: &str) -> Option<&mut super::panel::PluginPanelEntry> {
        self.panels.iter_mut().find(|p| p.id == id)
    }

    #[cfg(feature = "editor")]
    pub fn settings_pages_mut(&mut self) -> &mut [super::panel::PluginSettingsEntry] {
        &mut self.settings_pages
    }

    #[cfg(feature = "editor")]
    pub fn settings_pages(&self) -> &[super::panel::PluginSettingsEntry] {
        &self.settings_pages
    }

    // === Lifecycle ===

    /// Run every built plugin's `on_world_loaded` callback. Called after any
    /// world population; see `GameWorld::world_and_resources_mut`.
    ///
    /// One callback failing does not stop the others — the caller decides what
    /// a failure means (editor: surface and continue; standalone: fatal).
    ///
    /// Failures are also recorded in [`failures`](Self::failures), so the
    /// Plugin Manager stops showing a plugin as Enabled after its content hook
    /// blew up. Every callback re-runs at every content moment, so the previous
    /// moment's world-loaded failures are replaced rather than accumulated —
    /// a hook that succeeds this time clears its own row.
    #[must_use]
    pub fn run_world_loaded(
        &mut self,
        world: &mut hecs::World,
        resources: &mut Resources,
    ) -> Vec<PluginFailure> {
        self.failures
            .retain(|f| f.phase != RegistrationPhase::WorldLoaded);
        let mut failures = Vec::new();
        for (id, callback) in &mut self.world_loaded {
            if let Err(error) = callback(world, resources) {
                failures.push(PluginFailure {
                    id: id.clone(),
                    phase: RegistrationPhase::WorldLoaded,
                    error,
                });
            }
        }
        self.failures.extend(failures.iter().cloned());
        failures
    }

    /// How many world-loaded callbacks are registered (test/diagnostic hook).
    pub fn world_loaded_count(&self) -> usize {
        self.world_loaded.len()
    }

    /// Run every built plugin's debug-overlay contribution.
    pub fn run_debug_draw(
        &mut self,
        world: &hecs::World,
        buffer: &mut crate::engine::debug_draw::DebugDrawBuffer,
    ) {
        for (_id, callback) in &mut self.debug_draws {
            callback(world, buffer);
        }
    }

    /// Is this plugin live *right now*?
    ///
    /// True only if it built successfully this session. Deliberately reads the
    /// active runtime set, never the pending manifest: between a manifest edit
    /// and the restart, what is running has not changed (ruling §5.8).
    pub fn is_active(&self, id: &str) -> bool {
        self.records.iter().any(|r| r.manifest.id == id)
    }
}
