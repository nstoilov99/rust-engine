//! `DevNodesPlugin` — the first real plugin (Task 39.8 D5).
//!
//! Zero new functionality: it registers exactly the node set that
//! `node_graph::dev_nodes::register_dev_nodes` used to register from a direct
//! call at editor startup. Its job is to prove the whole chain — trait,
//! manifest, staged registration, activation toggle, and the Task 40 D3
//! promise that a disabled plugin degrades graphs instead of eating them.
//!
//! It is also the doc example for plugin authors, which is why it is this
//! short: a manifest and a loop.
//!
//! Compiled in only under the `dev_nodes` Cargo feature. That feature is
//! *inclusion*; the `project.ron` manifest is *activation*. The two are
//! separate on purpose (the tier-1 model): a shipped editor toggles the
//! manifest and restarts, never a rebuild.

use super::{
    EnginePlugin, PluginContext, PluginError, PluginKind, PluginManifest, PluginOrigin,
};
use crate::engine::node_graph::dev_nodes::dev_node_descriptors;

/// Fixture node types for exercising the graph editor.
pub struct DevNodesPlugin;

/// The manifest id. Permanent — renaming it would orphan every
/// `project.ron` entry that names it (see `PluginSet::with_alias`).
pub const DEV_NODES_ID: &str = "dev_nodes";

impl EnginePlugin for DevNodesPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new(DEV_NODES_ID, "Dev Nodes")
            .with_description(
                "Fixture node types (Test Event, Test Damage, Add, Editor Note) for \
                 exercising the node graph editor.",
            )
            .with_author("rust-engine")
            .with_origin(PluginOrigin::Engine)
            // Fixtures, not shipping content: they exist to populate the
            // create menu and keep `graphs/demo.graph` openable. `EditorOnly`
            // is what keeps them out of an export no matter how the manifest
            // is set (D9), which is the honest description of what they are.
            .with_kind(PluginKind::EditorOnly)
    }

    fn build(&self, ctx: &mut PluginContext) -> Result<(), PluginError> {
        for descriptor in dev_node_descriptors() {
            ctx.register_node(descriptor);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::ecs::resources::Resources;
    use crate::engine::ecs::schedule::Schedule;
    use crate::engine::node_graph::dev_nodes::register_dev_nodes;
    use crate::engine::node_graph::{NodeDescriptor, NodeRegistry};
    use crate::engine::plugins::{PluginSet, PluginTargets};

    fn contents(registry: &NodeRegistry) -> Vec<NodeDescriptor> {
        registry.iter().cloned().collect()
    }

    /// D5's acceptance: the plugin path must produce a registry indistinguishable
    /// from the pre-39.8 direct-call path. Same descriptors, same categories,
    /// same ordering — the create menu cannot tell the difference.
    #[test]
    fn plugin_path_produces_an_identical_registry_to_the_direct_call() {
        // The old way: `register_dev_nodes(&mut reg)` at editor startup.
        let mut direct = NodeRegistry::new();
        register_dev_nodes(&mut direct).expect("direct registration");

        // The new way: through PluginSet -> PluginContext -> merge_staged.
        let mut staged = NodeRegistry::new();
        let mut schedule = Schedule::new();
        let mut resources = Resources::new();
        let mut set = PluginSet::new();
        set.add(DevNodesPlugin);
        set.build_all(
            PluginTargets {
                schedule: &mut schedule,
                resources: &mut resources,
                node_registry: &mut staged,
            },
            None,
        );

        assert!(set.failures().is_empty(), "{:?}", set.failures());
        assert!(
            set.records()[0].warnings.is_empty(),
            "a clean build registers without warnings"
        );

        assert_eq!(
            contents(&staged),
            contents(&direct),
            "the plugin path must register exactly the direct-call node set"
        );

        // The create menu reads `by_category`; compare that projection too,
        // since it is what the user actually sees.
        let staged_menu: Vec<(String, Vec<String>)> = staged
            .by_category()
            .into_iter()
            .map(|(c, ds)| (c.to_string(), ds.iter().map(|d| d.id.clone()).collect()))
            .collect();
        let direct_menu: Vec<(String, Vec<String>)> = direct
            .by_category()
            .into_iter()
            .map(|(c, ds)| (c.to_string(), ds.iter().map(|d| d.id.clone()).collect()))
            .collect();
        assert_eq!(staged_menu, direct_menu, "create-menu inventory must match");

        // And the counts the Plugin Manager will show.
        assert_eq!(set.records()[0].counts.node_types, direct.iter().count());
    }

    /// Disabling it in the manifest registers nothing at all — the state the
    /// D3 placeholder degradation depends on.
    #[test]
    fn disabled_in_the_manifest_registers_nothing() {
        let mut registry = NodeRegistry::new();
        let mut schedule = Schedule::new();
        let mut resources = Resources::new();
        let mut set = PluginSet::new();
        set.add(DevNodesPlugin);
        set.build_all(
            PluginTargets {
                schedule: &mut schedule,
                resources: &mut resources,
                node_registry: &mut registry,
            },
            Some(&[crate::engine::plugins::PluginEntry::new(DEV_NODES_ID, false)]),
        );

        assert_eq!(set.disabled(), [DEV_NODES_ID.to_string()]);
        assert!(set.records().is_empty());
        assert_eq!(registry.iter().count(), 0);
        assert!(registry.get("test_add").is_none());
    }

    /// The manifest says what this plugin costs an export: nothing.
    #[test]
    fn manifest_marks_it_editor_only_and_manager_visible() {
        let m = DevNodesPlugin.manifest();
        assert_eq!(m.id, "dev_nodes");
        assert_eq!(m.kind, PluginKind::EditorOnly);
        assert_eq!(m.origin, PluginOrigin::Engine);
        assert!(!m.internal, "it must be visible in the Plugin Manager");
        assert!(m.depends_on.is_empty());
    }
}
