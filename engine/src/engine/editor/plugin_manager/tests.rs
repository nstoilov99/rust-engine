//! Model tests — every D8 state, the filters, and the cascade closure,
//! asserted without touching the UI.

use super::*;
use crate::engine::ecs::resources::Resources;
use crate::engine::ecs::schedule::Schedule;
use crate::engine::node_graph::NodeRegistry;
use crate::engine::plugins::{
    EnginePlugin, PluginContext, PluginError, PluginManifest, PluginTargets,
};

struct TestPlugin {
    manifest: PluginManifest,
    build: Box<dyn Fn(&mut PluginContext) -> Result<(), PluginError> + Send + Sync>,
}

impl TestPlugin {
    fn ok(manifest: PluginManifest) -> Self {
        Self {
            manifest,
            build: Box::new(|_| Ok(())),
        }
    }
    fn failing(manifest: PluginManifest) -> Self {
        Self {
            manifest,
            build: Box::new(|_| Err(PluginError::build("deliberate test failure"))),
        }
    }
    fn warning(manifest: PluginManifest) -> Self {
        Self {
            manifest,
            build: Box::new(|ctx| {
                // A node id the harness pre-registers ⇒ skip + warning.
                ctx.register_node(node("clash"));
                Ok(())
            }),
        }
    }
}

impl EnginePlugin for TestPlugin {
    fn manifest(&self) -> PluginManifest {
        self.manifest.clone()
    }
    fn build(&self, ctx: &mut PluginContext) -> Result<(), PluginError> {
        (self.build)(ctx)
    }
}

fn node(id: &str) -> crate::engine::node_graph::NodeDescriptor {
    use crate::engine::node_graph::doc::{NodeRealm, PinType};
    use crate::engine::node_graph::{NodeDescriptor, PinDescriptor};
    NodeDescriptor {
        id: id.into(),
        name: id.into(),
        category: "Test".into(),
        version: 1,
        inputs: vec![PinDescriptor::new("a", "A", PinType::Float)],
        outputs: vec![PinDescriptor::new("o", "O", PinType::Float)],
        pure: true,
        realm: NodeRealm::Shared,
        deterministic: true,
        doc: None,
        preview: None,
    }
}

fn manifest(id: &str) -> PluginManifest {
    PluginManifest::new(id, id).with_module_path(format!("engine/src/{id}"))
}

/// Build a set + project pair and return the model.
fn model_of(
    plugins: Vec<TestPlugin>,
    entries: Vec<PluginEntry>,
    seed_clash: bool,
) -> (PluginManagerModel, PluginSet, ProjectConfig) {
    let mut set = PluginSet::new();
    for p in plugins {
        set.add(p);
    }
    let mut schedule = Schedule::new();
    let mut resources = Resources::new();
    let mut registry = NodeRegistry::new();
    if seed_clash {
        registry.register(node("clash")).expect("seed");
    }
    let mut project = ProjectConfig::default();
    project.plugins = entries;
    set.build_all(
        PluginTargets {
            schedule: &mut schedule,
            resources: &mut resources,
            node_registry: &mut registry,
        },
        Some(&project.plugins),
    );
    let model = PluginManagerModel::build(&set, &project);
    (model, set, project)
}

#[test]
fn enabled_disabled_and_warning_states() {
    let (model, _set, _p) = model_of(
        vec![
            TestPlugin::ok(manifest("plain")),
            TestPlugin::ok(manifest("off")),
            TestPlugin::warning(manifest("noisy")),
        ],
        vec![PluginEntry::new("off", false)],
        true,
    );

    assert_eq!(model.get("plain").unwrap().state, RowState::Enabled);
    assert_eq!(model.get("off").unwrap().state, RowState::Disabled);
    assert_eq!(
        model.get("noisy").unwrap().state,
        RowState::EnabledWithWarnings
    );
    assert!(!model.get("noisy").unwrap().warnings.is_empty());
    assert!(model.get("noisy").unwrap().state.is_problem());
    assert!(!model.get("plain").unwrap().state.is_problem());
}

#[test]
fn failed_state_carries_the_phase_for_the_progression() {
    let (model, _set, _p) = model_of(vec![TestPlugin::failing(manifest("bad"))], vec![], false);
    match &model.get("bad").unwrap().state {
        RowState::Failed { phase, message } => {
            assert_eq!(*phase, RegistrationPhase::Build);
            assert!(message.contains("deliberate test failure"));
        }
        other => panic!("expected Failed, got {other:?}"),
    }
}

/// A plugin whose *content hook* failed built fine — its systems are really
/// registered — but the manager must not keep calling it Enabled.
#[test]
fn world_loaded_failure_makes_a_built_plugin_read_as_failed() {
    let (model, mut set, project) = model_of(
        vec![TestPlugin {
            manifest: manifest("hooked"),
            build: Box::new(|ctx| {
                ctx.on_world_loaded(|_w, _r| Err(PluginError::build("hook blew up")));
                Ok(())
            }),
        }],
        vec![],
        false,
    );
    assert_eq!(model.get("hooked").unwrap().state, RowState::Enabled);

    let mut world = hecs::World::new();
    let mut resources = Resources::new();
    let _ = set.run_world_loaded(&mut world, &mut resources);

    let model = PluginManagerModel::build(&set, &project);
    let row = model.get("hooked").expect("row");
    match &row.state {
        RowState::Failed { phase, message } => {
            assert_eq!(*phase, RegistrationPhase::WorldLoaded);
            assert!(message.contains("hook blew up"), "{message}");
        }
        other => panic!("expected Failed, got {other:?}"),
    }
    assert!(row.state.is_problem());
    assert!(
        row.counts.is_some(),
        "the record survives — what it registered is still registered"
    );
}

#[test]
fn blocked_missing_dependency_names_the_dependency() {
    let (model, _set, _p) = model_of(
        vec![
            TestPlugin::ok(manifest("base")),
            TestPlugin::ok(manifest("needy").depends_on(["base"])),
        ],
        vec![PluginEntry::new("base", false)],
        false,
    );
    assert_eq!(
        model.get("needy").unwrap().state,
        RowState::BlockedMissingDependency {
            missing: "base".into()
        }
    );
    assert!(model.get("needy").unwrap().state.toggle_locked());
}

#[test]
fn orphan_manifest_entry_becomes_a_not_in_this_build_row() {
    let (model, _set, _p) = model_of(
        vec![TestPlugin::ok(manifest("present"))],
        vec![PluginEntry::new("from_another_build", true)],
        false,
    );
    let orphan = model.get("from_another_build").expect("orphan row");
    assert_eq!(orphan.state, RowState::NotInThisBuild);
    assert!(orphan.state.is_problem());
    assert!(orphan.state.toggle_locked(), "an absent plugin cannot be toggled on");
}

/// Pending is the disagreement between the edited manifest and the running
/// set — the whole reason the banner exists.
#[test]
fn editing_the_manifest_after_build_produces_pending_states() {
    let (_m, set, mut project) = model_of(
        vec![
            TestPlugin::ok(manifest("running")),
            TestPlugin::ok(manifest("stopped")),
        ],
        vec![PluginEntry::new("stopped", false)],
        false,
    );

    // The user flips both.
    set_manifest_enabled(&mut project, "running", false);
    set_manifest_enabled(&mut project, "stopped", true);
    let model = PluginManagerModel::build(&set, &project);

    assert_eq!(model.get("running").unwrap().state, RowState::PendingDisable);
    assert_eq!(model.get("stopped").unwrap().state, RowState::PendingEnable);
    assert_eq!(model.pending_count(), 2);
    // A pending-disable plugin is still live until the relaunch (§5.8).
    assert!(model.get("running").unwrap().state.is_live());
    assert!(!model.get("stopped").unwrap().state.is_live());
}

#[test]
fn internal_plugins_never_get_a_row() {
    let (model, _set, _p) = model_of(
        vec![
            TestPlugin::ok(manifest("visible")),
            TestPlugin::ok(manifest("game").internal()),
        ],
        vec![],
        false,
    );
    assert!(model.get("game").is_none(), "the game module is hidden");
    assert_eq!(model.rows.len(), 1);
}

#[test]
fn filters_and_counts_partition_the_rows() {
    let (model, _set, _p) = model_of(
        vec![
            TestPlugin::ok(manifest("on1")),
            TestPlugin::ok(manifest("on2")),
            TestPlugin::ok(manifest("off1")),
            TestPlugin::failing(manifest("bad")),
        ],
        vec![PluginEntry::new("off1", false)],
        false,
    );

    assert_eq!(model.count(ManagerFilter::All), 4);
    assert_eq!(model.count(ManagerFilter::Problems), 1);
    assert_eq!(
        model.count(ManagerFilter::Enabled) + model.count(ManagerFilter::Disabled),
        model.count(ManagerFilter::All),
        "Enabled and Disabled must partition All"
    );
}

#[test]
fn search_matches_name_author_and_description() {
    let (model, _set, _p) = model_of(
        vec![
            TestPlugin::ok(
                manifest("physics").with_author("Ada").with_description("rigid bodies"),
            ),
            TestPlugin::ok(manifest("audio").with_description("mixer buses")),
        ],
        vec![],
        false,
    );
    let hit = |q: &str| -> Vec<String> {
        model
            .visible(ManagerFilter::All, q)
            .iter()
            .map(|r| r.id.clone())
            .collect()
    };
    assert_eq!(hit("rigid"), vec!["physics"]);
    assert_eq!(hit("Ada"), vec!["physics"]);
    assert_eq!(hit("mixer"), vec!["audio"]);
    assert_eq!(hit("").len(), 2);
}

#[test]
fn rows_group_engine_before_project() {
    let (model, _set, _p) = model_of(
        vec![
            TestPlugin::ok(manifest("zeta").with_origin(PluginOrigin::Project)),
            TestPlugin::ok(manifest("alpha").with_origin(PluginOrigin::Engine)),
        ],
        vec![],
        false,
    );
    let ids: Vec<&str> = model.rows.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids, vec!["alpha", "zeta"]);
}

// ── cascade (§5.7) ───────────────────────────────────────────────────────

#[test]
fn cascade_names_visible_dependents() {
    let (_m, set, _p) = model_of(
        vec![
            TestPlugin::ok(manifest("physics_rapier")),
            TestPlugin::ok(manifest("ragdolls").depends_on(["physics_rapier"])),
        ],
        vec![],
        false,
    );
    let prompt = dependents_of(&set, "physics_rapier");
    assert_eq!(prompt.dependents, vec!["ragdolls".to_string()]);
    assert!(!prompt.hits_internal);
    assert!(prompt.message().contains("ragdolls"));
}

/// An internal dependent has no row to point at, so it is described rather
/// than named (§5.7).
#[test]
fn cascade_describes_internal_dependents_in_prose() {
    let (_m, set, _p) = model_of(
        vec![
            TestPlugin::ok(manifest("physics_rapier")),
            TestPlugin::ok(
                manifest("game_client")
                    .depends_on(["physics_rapier"])
                    .internal(),
            ),
        ],
        vec![],
        false,
    );
    let prompt = dependents_of(&set, "physics_rapier");
    assert!(prompt.dependents.is_empty(), "internal ones are not listed as rows");
    assert!(prompt.hits_internal);
    let msg = prompt.message();
    assert!(msg.contains("gameplay systems"), "{msg}");
    assert!(!msg.contains("game_client"), "internal ids stay out of the copy");
}

#[test]
fn cascade_is_empty_for_a_plugin_nothing_depends_on() {
    let (_m, set, _p) = model_of(vec![TestPlugin::ok(manifest("lonely"))], vec![], false);
    let prompt = dependents_of(&set, "lonely");
    assert!(prompt.is_empty());
    assert!(prompt.message().contains("Nothing else depends on"));
}

// ── manifest writes ──────────────────────────────────────────────────────

#[test]
fn toggling_materializes_an_entry_and_preserves_orphans() {
    let mut project = ProjectConfig::default();
    project.plugins = vec![PluginEntry::new("from_another_build", false)];

    set_manifest_enabled(&mut project, "dev_nodes", false);
    assert_eq!(
        project.plugins,
        vec![
            PluginEntry::new("from_another_build", false),
            PluginEntry::new("dev_nodes", false),
        ]
    );

    // Toggling again edits in place rather than appending a duplicate.
    set_manifest_enabled(&mut project, "dev_nodes", true);
    assert_eq!(project.plugins.len(), 2);
    assert!(project.plugins.iter().any(|e| e.id == "dev_nodes" && e.enabled));
    assert!(
        project.plugins.iter().any(|e| e.id == "from_another_build"),
        "an orphan entry must survive an unrelated toggle"
    );
}

#[test]
fn registered_summary_reads_as_prose() {
    let mut c = PluginCounts::default();
    c.systems = 1;
    c.node_types = 4;
    c.panels = 1;
    assert_eq!(registered_summary(&c), "1 system · 4 node types · 1 panel");
    assert_eq!(
        registered_summary(&PluginCounts::default()),
        "— nothing registered"
    );
}
