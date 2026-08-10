//! Plugin system (Task 39.8) — the engine/game/extension boundary.
//!
//! Tier 1: a plugin is Rust code compiled into the binary; whether it *runs*
//! is decided at startup by the per-project manifest (`project.ron`). There is
//! no dynamic loading, and nothing here assumes static linking either — plugin
//! identity is a string id, registration goes through runtime registry calls,
//! and the [`PluginContext`] surface stays narrow so a future binary-plugin
//! tier can wrap it.
//!
//! Registration is *staged*: [`EnginePlugin::build`] writes into a per-plugin
//! scratch [`PluginContext`], and [`PluginSet`] commits that stage only when
//! `build()` returned `Ok`. A plugin that fails half-way registers nothing.
//!
//! Plan: `docs/roadmap/VULKANO-39.8-PLUGIN-SYSTEM.md`.

mod context;
/// Editor panel / settings-page extension points (D6). Editor-only: the
/// traits mention crusty-gui types.
#[cfg(feature = "editor")]
pub mod panel;
/// Rapier physics stepping as a plugin (D7). Module-first by decision;
/// promoting it to a workspace crate later is a move, not a redesign.
pub mod rapier;
mod set;

/// Fixture node types (D5) — the first plugin, and the doc example.
/// Compiled in only under the `dev_nodes` feature.
#[cfg(feature = "dev_nodes")]
pub mod dev_nodes;
/// Export feature resolution (D9): Cargo features as the packaging tool.
pub mod export;

#[cfg(test)]
mod tests;

pub use context::{DebugDrawFn, PluginContext, PluginCounts, WorldLoadedFn};
pub use export::{export_features, exported_plugins, ExportedPlugin, EXPORT_BASE_FEATURES};
pub use rapier::{RapierPhysicsPlugin, PHYSICS_RAPIER_ID};
#[cfg(feature = "dev_nodes")]
pub use dev_nodes::DevNodesPlugin;
#[cfg(feature = "editor")]
pub use panel::{
    PluginPanel, PluginPanelCtx, PluginPanelEntry, PluginSettingsCtx, PluginSettingsEntry,
    PluginSettingsPage,
};
pub use set::{PluginRecord, PluginSet, PluginTargets};

use serde::{Deserialize, Serialize};

/// Where a plugin comes from — drives Plugin Manager grouping (Unreal's
/// Engine/Project split, O3DE's Gems catalog).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PluginOrigin {
    /// Ships with the engine (physics, dev nodes, …).
    Engine,
    /// Belongs to this project.
    Project,
}

/// Whether a plugin ships in an exported game.
///
/// `EditorOnly` is the analog of Unreal's Editor module type: tooling,
/// importers and debug panels are excluded from game exports (D9) without the
/// user having to think about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PluginKind {
    /// Ships with the exported game when enabled.
    Runtime,
    /// Never enters an export feature set, regardless of enabled state.
    EditorOnly,
}

/// Static metadata describing a plugin.
///
/// `id` is a permanent, stable slug — the same append-only convention as node
/// type slugs. Renaming one orphans the manifest entry (silently re-enabling
/// something the user disabled), so renames require an alias registered with
/// [`PluginSet::with_alias`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    /// Plugin ids this plugin needs. A hole (absent, disabled or failed)
    /// fails this plugin with [`PluginError::MissingDependency`].
    pub depends_on: Vec<String>,
    pub origin: PluginOrigin,
    pub kind: PluginKind,
    /// Internal plugins (the project's own game module) run through
    /// [`PluginSet`] for uniform mechanics but are hidden from the Plugin
    /// Manager and are not manifest-toggleable — no engine lists the game's
    /// own code as a plugin.
    pub internal: bool,
    /// Where the code lives, shown in the manager's Manifest facts (D8 v1
    /// delta 1). Tier 1 has no `plugin.ron` and no DLL entry point; the
    /// honest answer to "where is this from" is the module path.
    pub module_path: String,
    /// The Cargo feature that decides whether this plugin is *compiled in*
    /// at all. `None` = always compiled in. Inclusion is the feature;
    /// activation is the manifest.
    pub cargo_feature: Option<String>,
}

impl PluginManifest {
    /// Minimal manifest: engine-origin, runtime, not internal, no deps.
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            version: "0.1.0".to_string(),
            description: String::new(),
            author: String::new(),
            depends_on: Vec::new(),
            origin: PluginOrigin::Engine,
            kind: PluginKind::Runtime,
            internal: false,
            module_path: String::new(),
            cargo_feature: None,
        }
    }

    /// Where this plugin's code lives, e.g.
    /// `engine/src/engine/plugins/rapier`.
    pub fn with_module_path(mut self, path: impl Into<String>) -> Self {
        self.module_path = path.into();
        self
    }

    /// The Cargo feature gating its inclusion in the build.
    pub fn with_cargo_feature(mut self, feature: impl Into<String>) -> Self {
        self.cargo_feature = Some(feature.into());
        self
    }

    pub fn with_version(mut self, v: impl Into<String>) -> Self {
        self.version = v.into();
        self
    }

    pub fn with_description(mut self, d: impl Into<String>) -> Self {
        self.description = d.into();
        self
    }

    pub fn with_author(mut self, a: impl Into<String>) -> Self {
        self.author = a.into();
        self
    }

    pub fn depends_on<I, S>(mut self, ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.depends_on = ids.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_origin(mut self, origin: PluginOrigin) -> Self {
        self.origin = origin;
        self
    }

    pub fn with_kind(mut self, kind: PluginKind) -> Self {
        self.kind = kind;
        self
    }

    /// Mark as internal: runs, but never appears in the Plugin Manager.
    pub fn internal(mut self) -> Self {
        self.internal = true;
        self
    }
}

/// Which part of registration a failure happened in — carried on the error and
/// on [`PluginFailure`] so the Plugin Manager can say *where* it broke.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistrationPhase {
    /// Dependency resolution, before `build()` was ever called.
    Dependency,
    /// Inside the plugin's own `build()`.
    Build,
    /// Committing staged systems into the live schedule.
    Systems,
    /// Committing staged resources.
    Resources,
    /// Committing staged node types / domain pins / migrations.
    NodeRegistry,
    /// Committing staged editor panels / settings pages.
    Panels,
    /// Running an `on_world_loaded` callback at a content moment.
    WorldLoaded,
}

impl RegistrationPhase {
    pub fn label(self) -> &'static str {
        match self {
            RegistrationPhase::Dependency => "dependency resolution",
            RegistrationPhase::Build => "build",
            RegistrationPhase::Systems => "system registration",
            RegistrationPhase::Resources => "resource registration",
            RegistrationPhase::NodeRegistry => "node registration",
            RegistrationPhase::Panels => "panel registration",
            RegistrationPhase::WorldLoaded => "world-loaded callback",
        }
    }
}

/// Why a plugin did not come up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginError {
    /// A declared dependency is absent from this build, disabled in the
    /// manifest, or itself failed.
    MissingDependency { plugin: String, missing: String },
    /// This plugin sits in (or behind) a `depends_on` cycle.
    DependencyCycle { members: Vec<String> },
    /// `build()` failed, or committing its stage did. Carries the phase.
    Build {
        phase: RegistrationPhase,
        message: String,
    },
}

impl PluginError {
    /// A failure raised by a plugin's own `build()`.
    pub fn build(message: impl Into<String>) -> Self {
        PluginError::Build {
            phase: RegistrationPhase::Build,
            message: message.into(),
        }
    }

    /// A failure raised while committing a specific part of the stage.
    pub fn at(phase: RegistrationPhase, message: impl Into<String>) -> Self {
        PluginError::Build {
            phase,
            message: message.into(),
        }
    }

    pub fn phase(&self) -> RegistrationPhase {
        match self {
            PluginError::MissingDependency { .. } | PluginError::DependencyCycle { .. } => {
                RegistrationPhase::Dependency
            }
            PluginError::Build { phase, .. } => *phase,
        }
    }
}

impl std::fmt::Display for PluginError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PluginError::MissingDependency { plugin, missing } => write!(
                f,
                "plugin '{plugin}' requires '{missing}', which is not available"
            ),
            PluginError::DependencyCycle { members } => {
                write!(f, "plugin dependency cycle: {}", members.join(" -> "))
            }
            PluginError::Build { phase, message } => {
                write!(f, "{} failed: {message}", phase.label())
            }
        }
    }
}

impl std::error::Error for PluginError {}

/// A plugin that did not come up, retained for the Plugin Manager.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginFailure {
    pub id: String,
    pub error: PluginError,
    pub phase: RegistrationPhase,
}

/// One manifest row in `project.ron`.
///
/// Lives here rather than in `editor::project_config` so the non-editor build
/// can still talk about plugin activation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginEntry {
    pub id: String,
    pub enabled: bool,
}

impl PluginEntry {
    pub fn new(id: impl Into<String>, enabled: bool) -> Self {
        Self {
            id: id.into(),
            enabled,
        }
    }
}

/// A unit of engine functionality that registers itself at startup.
///
/// `build()` must be side-effect free outside `ctx`: everything it registers is
/// staged and only committed if it returns `Ok`.
pub trait EnginePlugin: Send + Sync {
    /// Static metadata. Called before `build()` (for dependency ordering) and
    /// again for display, so keep it cheap and constant.
    fn manifest(&self) -> PluginManifest;

    /// The single registration entry point.
    fn build(&self, ctx: &mut PluginContext) -> Result<(), PluginError>;
}
