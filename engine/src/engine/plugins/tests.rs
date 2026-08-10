//! P1 acceptance tests: ordering, dependency failure modes, staged-commit
//! atomicity, collision policy, and manifest round-trip.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use super::*;
use crate::engine::ecs::access::SystemDescriptor;
use crate::engine::ecs::resources::Resources;
use crate::engine::ecs::schedule::{Schedule, Stage};
use crate::engine::node_graph::doc::{NodeRealm, PinType};
use crate::engine::node_graph::{NodeDescriptor, NodeRegistry, PinDescriptor};

// === Test doubles ===

/// A plugin whose whole behaviour is a closure over the staging context.
struct TestPlugin {
    manifest: PluginManifest,
    build: Box<dyn Fn(&mut PluginContext) -> Result<(), PluginError> + Send + Sync>,
}

impl TestPlugin {
    fn new(
        manifest: PluginManifest,
        build: impl Fn(&mut PluginContext) -> Result<(), PluginError> + Send + Sync + 'static,
    ) -> Self {
        Self {
            manifest,
            build: Box::new(build),
        }
    }

    /// A plugin that registers nothing.
    fn inert(id: &str) -> Self {
        Self::new(PluginManifest::new(id, id), |_| Ok(()))
    }

    fn depending(id: &str, deps: &[&str]) -> Self {
        Self::new(PluginManifest::new(id, id).depends_on(deps.to_vec()), |_| {
            Ok(())
        })
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

struct NoopSystem(&'static str);
impl crate::engine::ecs::schedule::System for NoopSystem {
    fn run(&mut self, _w: &mut hecs::World, _r: &mut Resources) {}
    fn name(&self) -> &str {
        self.0
    }
}

struct ResA(#[allow(dead_code)] u32);
struct ResB(#[allow(dead_code)] u32);

fn pure_node(id: &str) -> NodeDescriptor {
    NodeDescriptor {
        id: id.into(),
        name: id.into(),
        category: "Test".into(),
        version: 1,
        inputs: vec![PinDescriptor::new("a", "A", PinType::Float)],
        outputs: vec![PinDescriptor::new("out", "Out", PinType::Float)],
        pure: true,
        realm: NodeRealm::Shared,
        deterministic: true,
        doc: None,
        preview: None,
    }
}

/// Live registries plus a place to look at them afterwards.
struct Live {
    schedule: Schedule,
    resources: Resources,
    registry: NodeRegistry,
}

impl Live {
    fn new() -> Self {
        Self {
            schedule: Schedule::new(),
            resources: Resources::new(),
            registry: NodeRegistry::new(),
        }
    }

    fn build(&mut self, set: &mut PluginSet, filter: Option<&[PluginEntry]>) {
        set.build_all(
            PluginTargets {
                schedule: &mut self.schedule,
                resources: &mut self.resources,
                node_registry: &mut self.registry,
            },
            filter,
        );
    }
}

fn built_ids(set: &PluginSet) -> Vec<String> {
    set.records()
        .iter()
        .map(|r| r.manifest.id.clone())
        .collect()
}

// === Ordering ===

#[test]
fn topo_order_follows_depends_on_not_insertion_order() {
    let order = Arc::new(Mutex::new(Vec::new()));
    let mut set = PluginSet::new();
    for (id, deps) in [("c", vec!["b"]), ("b", vec!["a"]), ("a", vec![])] {
        let o = Arc::clone(&order);
        let id_owned = id.to_string();
        set.add(TestPlugin::new(
            PluginManifest::new(id, id).depends_on(deps),
            move |_ctx| {
                o.lock().expect("lock").push(id_owned.clone());
                Ok(())
            },
        ));
    }

    let mut live = Live::new();
    live.build(&mut set, None);

    assert!(set.failures().is_empty(), "{:?}", set.failures());
    assert_eq!(*order.lock().expect("lock"), vec!["a", "b", "c"]);
    assert_eq!(built_ids(&set), vec!["a", "b", "c"]);
}

#[test]
fn independent_plugins_keep_manifest_order() {
    let mut set = PluginSet::new();
    set.add(TestPlugin::inert("z"));
    set.add(TestPlugin::inert("m"));
    set.add(TestPlugin::inert("a"));

    let mut live = Live::new();
    live.build(&mut set, None);

    assert_eq!(built_ids(&set), vec!["z", "m", "a"]);
}

#[test]
fn dependency_cycle_fails_every_member() {
    let mut set = PluginSet::new();
    set.add(TestPlugin::depending("a", &["b"]));
    set.add(TestPlugin::depending("b", &["a"]));
    set.add(TestPlugin::inert("free"));

    let mut live = Live::new();
    live.build(&mut set, None);

    assert_eq!(built_ids(&set), vec!["free"], "the acyclic plugin still builds");
    assert_eq!(set.failures().len(), 2);
    for failure in set.failures() {
        assert_eq!(failure.phase, RegistrationPhase::Dependency);
        match &failure.error {
            PluginError::DependencyCycle { members } => {
                assert_eq!(members, &vec!["a".to_string(), "b".to_string()]);
            }
            other => panic!("expected DependencyCycle, got {other:?}"),
        }
    }
}

#[test]
fn missing_dependency_fails_the_dependent() {
    let mut set = PluginSet::new();
    set.add(TestPlugin::depending("needy", &["absent"]));

    let mut live = Live::new();
    live.build(&mut set, None);

    assert!(built_ids(&set).is_empty());
    assert_eq!(
        set.failures()[0].error,
        PluginError::MissingDependency {
            plugin: "needy".into(),
            missing: "absent".into(),
        }
    );
}

#[test]
fn dependency_disabled_in_manifest_is_a_missing_dependency() {
    let mut set = PluginSet::new();
    set.add(TestPlugin::inert("base"));
    set.add(TestPlugin::depending("needy", &["base"]));

    let mut live = Live::new();
    live.build(&mut set, Some(&[PluginEntry::new("base", false)]));

    assert!(built_ids(&set).is_empty());
    assert_eq!(set.disabled(), ["base".to_string()]);
    assert!(matches!(
        set.failures()[0].error,
        PluginError::MissingDependency { .. }
    ));
}

#[test]
fn failed_dependency_cascades_to_the_dependent() {
    let mut set = PluginSet::new();
    set.add(TestPlugin::new(
        PluginManifest::new("base", "base"),
        |_ctx| Err(PluginError::build("deliberate")),
    ));
    set.add(TestPlugin::depending("needy", &["base"]));

    let mut live = Live::new();
    live.build(&mut set, None);

    assert!(built_ids(&set).is_empty());
    assert_eq!(set.failures().len(), 2);
    assert_eq!(set.failures()[0].phase, RegistrationPhase::Build);
    assert!(matches!(
        set.failures()[1].error,
        PluginError::MissingDependency { .. }
    ));
}

// === Staged commit ===

#[test]
fn successful_build_commits_systems_resources_and_nodes() {
    let mut set = PluginSet::new();
    set.add(TestPlugin::new(
        PluginManifest::new("full", "Full"),
        |ctx| {
            ctx.add_system(
                NoopSystem("plugin_sys"),
                Stage::Update,
                SystemDescriptor::new("plugin_sys"),
            );
            ctx.insert_resource(ResA(7));
            ctx.register_node(pure_node("plug_node"));
            ctx.register_domain_pin("plug_domain");
            ctx.register_migration("plug_node", 1, |_| {});
            ctx.on_world_loaded(|_w, _r| Ok(()));
            Ok(())
        },
    ));

    let mut live = Live::new();
    live.build(&mut set, None);

    assert!(set.failures().is_empty(), "{:?}", set.failures());
    assert_eq!(live.schedule.system_count(), 1);
    assert!(live.resources.contains::<ResA>());
    assert!(live.registry.get("plug_node").is_some());
    assert!(live.registry.domain_pin_registered("plug_domain"));
    assert!(live.registry.migration("plug_node", 1).is_some());
    assert_eq!(set.world_loaded_count(), 1);

    let counts = set.records()[0].counts;
    assert_eq!(counts.systems, 1);
    assert_eq!(counts.resources, 1);
    assert_eq!(counts.node_types, 1);
    assert_eq!(counts.world_loaded_callbacks, 1);
}

#[test]
fn partial_failure_discards_the_whole_stage() {
    let mut set = PluginSet::new();
    set.add(TestPlugin::new(
        PluginManifest::new("half", "Half"),
        |ctx| {
            // Everything below is staged, then thrown away with `ctx`.
            ctx.add_system(
                NoopSystem("ghost_sys"),
                Stage::Update,
                SystemDescriptor::new("ghost_sys"),
            );
            ctx.insert_resource(ResA(1));
            ctx.register_node(pure_node("ghost_node"));
            ctx.on_world_loaded(|_w, _r| Ok(()));
            Err(PluginError::build("gave up after registering"))
        },
    ));

    let mut live = Live::new();
    live.build(&mut set, None);

    assert_eq!(set.failures().len(), 1);
    assert_eq!(set.failures()[0].phase, RegistrationPhase::Build);
    assert_eq!(live.schedule.system_count(), 0, "no half-registered system");
    assert!(!live.resources.contains::<ResA>(), "no half-registered resource");
    assert!(live.registry.get("ghost_node").is_none());
    assert_eq!(set.world_loaded_count(), 0);
}

#[test]
fn resource_collision_is_an_error_and_leaves_nothing_behind() {
    let mut live = Live::new();
    live.resources.insert(ResA(1)); // core already owns it

    let mut set = PluginSet::new();
    set.add(TestPlugin::new(
        PluginManifest::new("greedy", "Greedy"),
        |ctx| {
            ctx.add_system(
                NoopSystem("greedy_sys"),
                Stage::Update,
                SystemDescriptor::new("greedy_sys"),
            );
            ctx.insert_resource(ResA(2));
            ctx.insert_resource(ResB(3));
            Ok(())
        },
    ));

    live.build(&mut set, None);

    assert_eq!(set.failures()[0].phase, RegistrationPhase::Resources);
    assert_eq!(live.schedule.system_count(), 0);
    assert!(!live.resources.contains::<ResB>(), "sibling insert rolled back too");
}

#[test]
fn second_plugin_cannot_replace_the_first_plugins_resource() {
    let mut set = PluginSet::new();
    set.add(TestPlugin::new(PluginManifest::new("first", "First"), |ctx| {
        ctx.insert_resource(ResA(1));
        Ok(())
    }));
    set.add(TestPlugin::new(
        PluginManifest::new("second", "Second"),
        |ctx| {
            ctx.insert_resource(ResA(2));
            Ok(())
        },
    ));

    let mut live = Live::new();
    live.build(&mut set, None);

    assert_eq!(built_ids(&set), vec!["first"]);
    assert_eq!(set.failures()[0].id, "second");
}

#[test]
fn node_id_collision_skips_the_descriptor_with_a_warning() {
    let mut live = Live::new();
    live.registry.register(pure_node("shared")).expect("seed");

    let mut set = PluginSet::new();
    set.add(TestPlugin::new(
        PluginManifest::new("late", "Late"),
        |ctx| {
            ctx.register_node(pure_node("shared"));
            ctx.register_node(pure_node("unique"));
            Ok(())
        },
    ));

    live.build(&mut set, None);

    assert!(set.failures().is_empty(), "a duplicate id must not fail the plugin");
    let record = &set.records()[0];
    assert_eq!(record.warnings.len(), 1);
    assert!(record.warnings[0].contains("shared"));
    assert_eq!(record.counts.node_types, 1, "only the unique one counts");
    assert!(live.registry.get("unique").is_some());
    // The first registration wins.
    assert_eq!(live.registry.get("shared").expect("kept").category, "Test");
}

#[test]
fn migration_key_collision_fails_the_plugin_and_registers_nothing() {
    let mut live = Live::new();
    live.registry.register_migration("thing", 1, |_| {});

    let mut set = PluginSet::new();
    set.add(TestPlugin::new(
        PluginManifest::new("dup_mig", "Dup"),
        |ctx| {
            ctx.register_node(pure_node("mig_node"));
            ctx.register_migration("thing", 1, |_| {});
            Ok(())
        },
    ));

    live.build(&mut set, None);

    assert_eq!(set.failures()[0].phase, RegistrationPhase::NodeRegistry);
    assert!(
        live.registry.get("mig_node").is_none(),
        "the node staged alongside the bad migration must not land"
    );
}

#[test]
fn domain_pin_reregistration_is_identical_ok_else_first_wins() {
    let mut set = PluginSet::new();
    set.add(TestPlugin::new(PluginManifest::new("first", "First"), |ctx| {
        ctx.register_domain_pin_keyed("shader", 3);
        Ok(())
    }));
    set.add(TestPlugin::new(
        PluginManifest::new("same", "Same"),
        |ctx| {
            ctx.register_domain_pin_keyed("shader", 3);
            Ok(())
        },
    ));
    set.add(TestPlugin::new(
        PluginManifest::new("clash", "Clash"),
        |ctx| {
            ctx.register_domain_pin_keyed("shader", 9);
            Ok(())
        },
    ));

    let mut live = Live::new();
    live.build(&mut set, None);

    assert!(set.failures().is_empty());
    assert_eq!(live.registry.domain_registration("shader"), Some(Some(3)));
    assert!(set.records()[1].warnings.is_empty(), "identical is a no-op");
    let clash = &set.records()[2].warnings;
    assert_eq!(clash.len(), 1);
    assert!(clash[0].contains("clash") && clash[0].contains("first"));
}

// === Lifecycle ===

#[test]
fn world_loaded_callbacks_run_on_demand_and_only_for_built_plugins() {
    let runs = Arc::new(AtomicUsize::new(0));

    let mut set = PluginSet::new();
    let r = Arc::clone(&runs);
    set.add(TestPlugin::new(PluginManifest::new("live", "Live"), move |ctx| {
        let r = Arc::clone(&r);
        ctx.on_world_loaded(move |_w, _res| {
            r.fetch_add(1, Ordering::SeqCst);
            Ok(())
        });
        Ok(())
    }));
    let r = Arc::clone(&runs);
    set.add(TestPlugin::new(PluginManifest::new("dead", "Dead"), move |ctx| {
        let r = Arc::clone(&r);
        ctx.on_world_loaded(move |_w, _res| {
            r.fetch_add(100, Ordering::SeqCst);
            Ok(())
        });
        Err(PluginError::build("nope"))
    }));

    let mut live = Live::new();
    live.build(&mut set, None);

    let mut world = hecs::World::new();
    assert!(set.run_world_loaded(&mut world, &mut live.resources).is_empty());
    assert!(set.run_world_loaded(&mut world, &mut live.resources).is_empty());

    assert_eq!(runs.load(Ordering::SeqCst), 2, "runs per content moment");
}

#[test]
fn world_loaded_failure_is_reported_per_plugin_and_does_not_stop_the_others() {
    let later_ran = Arc::new(AtomicUsize::new(0));

    let mut set = PluginSet::new();
    set.add(TestPlugin::new(PluginManifest::new("bad", "Bad"), |ctx| {
        ctx.on_world_loaded(|_w, _r| Err(PluginError::build("content moment blew up")));
        Ok(())
    }));
    let r = Arc::clone(&later_ran);
    set.add(TestPlugin::new(PluginManifest::new("good", "Good"), move |ctx| {
        let r = Arc::clone(&r);
        ctx.on_world_loaded(move |_w, _res| {
            r.fetch_add(1, Ordering::SeqCst);
            Ok(())
        });
        Ok(())
    }));

    let mut live = Live::new();
    live.build(&mut set, None);

    let mut world = hecs::World::new();
    let failures = set.run_world_loaded(&mut world, &mut live.resources);

    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].id, "bad");
    assert_eq!(failures[0].phase, RegistrationPhase::WorldLoaded);
    assert_eq!(later_ran.load(Ordering::SeqCst), 1, "later callbacks still run");
}

/// The Plugin Manager reads `failures()`; a content-hook failure that only
/// came back as a return value left the plugin showing Enabled forever.
#[test]
fn world_loaded_failure_is_recorded_in_the_sets_failure_list() {
    let fail_next = Arc::new(AtomicUsize::new(1));

    let mut set = PluginSet::new();
    let f = Arc::clone(&fail_next);
    set.add(TestPlugin::new(PluginManifest::new("bad", "Bad"), move |ctx| {
        let f = Arc::clone(&f);
        ctx.on_world_loaded(move |_w, _r| {
            if f.load(Ordering::SeqCst) > 0 {
                Err(PluginError::build("content moment blew up"))
            } else {
                Ok(())
            }
        });
        Ok(())
    }));

    let mut live = Live::new();
    live.build(&mut set, None);
    assert!(set.failures().is_empty(), "build itself succeeded");

    let mut world = hecs::World::new();
    let _ = set.run_world_loaded(&mut world, &mut live.resources);

    assert_eq!(set.failures().len(), 1);
    assert_eq!(set.failures()[0].id, "bad");
    assert_eq!(set.failures()[0].phase, RegistrationPhase::WorldLoaded);
    assert!(
        set.is_active("bad"),
        "the record stays — its committed systems really are registered"
    );

    // A second content moment re-runs every callback, so its outcome replaces
    // the previous one instead of piling up.
    let _ = set.run_world_loaded(&mut world, &mut live.resources);
    assert_eq!(set.failures().len(), 1, "no duplicate per content moment");

    fail_next.store(0, Ordering::SeqCst);
    let _ = set.run_world_loaded(&mut world, &mut live.resources);
    assert!(
        set.failures().is_empty(),
        "a hook that succeeds again clears its own failure"
    );
}

// === Editor extension points (D6) ===

#[cfg(feature = "editor")]
mod editor_seams {
    use super::*;
    use crate::engine::editor::dock_crusty::{parse_tab, tab_id};
    use crate::engine::editor::EditorTab;
    use crate::engine::plugins::panel::{
        PluginPanel, PluginPanelCtx, PluginSettingsCtx, PluginSettingsPage,
    };

    struct Panel;
    impl PluginPanel for Panel {
        fn draw(
            &mut self,
            _ui: &mut crusty_gui::context::Ui,
            _rect: crusty_gui::math::Rect,
            _ctx: &mut PluginPanelCtx<'_>,
        ) {
        }
    }

    struct Page;
    impl PluginSettingsPage for Page {
        fn draw(&mut self, _ui: &mut crusty_gui::context::Ui, _ctx: &mut PluginSettingsCtx<'_>) {}
    }

    fn panel_plugin(id: &'static str, panel_id: &'static str) -> TestPlugin {
        TestPlugin::new(PluginManifest::new(id, id), move |ctx| {
            ctx.register_panel(panel_id, "Title", || Box::new(Panel));
            Ok(())
        })
    }

    /// The layout file stores `plugin:<id>`; it must survive the round trip
    /// **whether or not** anything registers that id, or a restored layout
    /// would silently reshape itself.
    #[test]
    fn plugin_tab_id_round_trips_even_for_an_unregistered_panel() {
        let tab = EditorTab::Plugin("never_registered".to_string());
        let id = tab_id(&tab);
        assert_eq!(id, "plugin:never_registered");
        assert_eq!(parse_tab(&id), Some(tab));
        assert_eq!(
            parse_tab("plugin:anything_at_all"),
            Some(EditorTab::Plugin("anything_at_all".to_string())),
            "parsing must not depend on the live plugin set"
        );
    }

    /// A registered panel's menu entry and its persisted tab id are the same
    /// string — that is what makes View ▸ Panels open the tab a layout restores.
    #[test]
    fn panel_menu_entry_matches_the_persisted_tab_id() {
        let mut set = PluginSet::new();
        set.add(panel_plugin("p", "my_panel"));
        let mut live = Live::new();
        live.build(&mut set, None);

        let entries = set.panel_menu_entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, tab_id(&EditorTab::Plugin("my_panel".into())));
        assert_eq!(
            parse_tab(&entries[0].0),
            Some(EditorTab::Plugin("my_panel".into()))
        );
    }

    /// Two plugins cannot share one dock tab id. Unlike a duplicate node type
    /// there is no user document to protect, so this fails the second plugin
    /// rather than silently letting one panel shadow the other.
    #[test]
    fn duplicate_panel_id_fails_the_second_plugin_and_keeps_the_first() {
        let mut set = PluginSet::new();
        set.add(panel_plugin("first", "shared"));
        set.add(panel_plugin("second", "shared"));

        let mut live = Live::new();
        live.build(&mut set, None);

        assert_eq!(built_ids(&set), vec!["first"]);
        assert_eq!(set.failures().len(), 1);
        assert_eq!(set.failures()[0].id, "second");
        assert_eq!(set.failures()[0].phase, RegistrationPhase::Panels);
        assert_eq!(set.panel_menu_entries().len(), 1);
    }

    /// A failing plugin's panel and settings page are discarded with the rest
    /// of its stage — the staged-commit guarantee extends to the new seams.
    #[test]
    fn failed_build_discards_staged_panels_and_pages() {
        let mut set = PluginSet::new();
        set.add(TestPlugin::new(
            PluginManifest::new("half", "Half"),
            |ctx| {
                ctx.register_panel("ghost_panel", "Ghost", || Box::new(Panel));
                ctx.register_settings_page("ghost_page", "Ghost", || Box::new(Page));
                Err(PluginError::build("gave up"))
            },
        ));

        let mut live = Live::new();
        live.build(&mut set, None);

        assert!(set.panel_menu_entries().is_empty());
        assert!(set.settings_pages().is_empty());
        assert!(set.panel_mut("ghost_panel").is_none());
    }

    /// Counts feed the Plugin Manager rows.
    #[test]
    fn panel_and_page_registrations_are_counted() {
        let mut set = PluginSet::new();
        set.add(TestPlugin::new(PluginManifest::new("p", "P"), |ctx| {
            ctx.register_panel("a", "A", || Box::new(Panel));
            ctx.register_panel("b", "B", || Box::new(Panel));
            ctx.register_settings_page("s", "S", || Box::new(Page));
            Ok(())
        }));

        let mut live = Live::new();
        live.build(&mut set, None);

        let counts = set.records()[0].counts;
        assert_eq!(counts.panels, 2);
        assert_eq!(counts.settings_pages, 1);
    }
}

// === Manifest ===

#[test]
fn manifest_absent_id_defaults_to_enabled_and_internal_ignores_the_manifest() {
    let mut set = PluginSet::new();
    set.add(TestPlugin::inert("listed"));
    set.add(TestPlugin::inert("unlisted"));
    set.add(TestPlugin::new(
        PluginManifest::new("game", "Game").internal(),
        |_| Ok(()),
    ));

    let mut live = Live::new();
    live.build(
        &mut set,
        Some(&[
            PluginEntry::new("listed", false),
            // An internal plugin is not manifest-toggleable.
            PluginEntry::new("game", false),
        ]),
    );

    assert_eq!(built_ids(&set), vec!["unlisted", "game"]);
    assert_eq!(set.disabled(), ["listed".to_string()]);
}

#[test]
fn manifest_alias_redirects_an_old_id_to_the_renamed_plugin() {
    let mut set = PluginSet::new();
    set.add(TestPlugin::inert("physics_rapier"));
    set.with_alias("physics", "physics_rapier");

    let mut live = Live::new();
    live.build(&mut set, Some(&[PluginEntry::new("physics", false)]));

    assert!(built_ids(&set).is_empty(), "the alias carries the disable");
    assert_eq!(set.disabled(), ["physics_rapier".to_string()]);
    assert!(set.orphans().is_empty(), "an aliased id is not an orphan");
}

#[test]
fn manifest_orphan_entries_are_reported_not_fatal() {
    let mut set = PluginSet::new();
    set.add(TestPlugin::inert("present"));

    let mut live = Live::new();
    live.build(
        &mut set,
        Some(&[
            PluginEntry::new("present", true),
            PluginEntry::new("from_another_build", false),
        ]),
    );

    assert_eq!(built_ids(&set), vec!["present"]);
    assert_eq!(set.orphans(), ["from_another_build".to_string()]);
}

#[cfg(feature = "editor")]
#[test]
fn project_config_round_trip_preserves_plugin_entries_including_orphans() {
    use crate::engine::editor::project_config::ProjectConfig;

    let mut config = ProjectConfig::default();
    config.plugins = vec![
        PluginEntry::new("physics_rapier", false),
        PluginEntry::new("from_another_build", true),
    ];

    let text = ron::ser::to_string_pretty(&config, Default::default()).expect("serialize");
    let back: ProjectConfig = ron::from_str(&text).expect("deserialize");

    assert_eq!(back.plugins, config.plugins);
    assert_eq!(back, config);
}

#[cfg(feature = "editor")]
#[test]
fn project_config_without_a_plugins_field_still_loads() {
    use crate::engine::editor::project_config::ProjectConfig;

    // A `project.ron` written before 39.8 — serde(default) fills the field.
    let legacy = "(name: \"legacy\", version: \"0.2.0\")";
    let config: ProjectConfig = ron::from_str(legacy).expect("deserialize legacy");

    assert_eq!(config.name, "legacy");
    assert!(config.plugins.is_empty(), "empty manifest = all enabled");
}
