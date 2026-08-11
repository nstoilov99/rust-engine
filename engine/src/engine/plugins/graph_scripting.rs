//! `GraphScriptingPlugin` — graph execution as a plugin (Task 45-A, audit
//! addendum #6).
//!
//! ## What this owns
//!
//! 1. The standard node library and event entry descriptors, staged into the
//!    `NodeRegistry`.
//! 2. The runner system, which ticks every `GraphRunner` and applies its
//!    effects.
//! 3. The compiled-plan cache resource.
//!
//! ## What stays in engine core
//!
//! The `GraphRunner` *component*, its scene/prefab serialization and its
//! inspector row — exactly the Rapier split, and for the same reason: a scene
//! naming a graph must round-trip losslessly whether or not anything runs it.
//!
//! ## The first real Cargo-feature gate (D9)
//!
//! Unlike Rapier this plugin carries `cargo_feature: Some("graph-scripting")`.
//! `node_graph_exec` is a leaf crate with no engine dependency, so the gate is
//! clean: disabling the plugin in `project.ron` genuinely strips the
//! interpreter from an export rather than shipping dead code behind a runtime
//! flag. It is default-on, so it is also in `EXPORT_BASE_FEATURES`' sibling
//! list — see `export.rs`.
//!
//! ## With the plugin disabled
//!
//! No std node descriptors are registered, so a `.graph` using them still
//! loads and still saves losslessly but validates to `UnknownNodeType` and
//! renders as placeholder nodes — the Task 40 degradation, exercised for real.
//! No runner system, so `GraphRunner` components are inert.

use std::path::PathBuf;

use crate::engine::ecs::access::SystemDescriptor;
use crate::engine::ecs::components::{Transform, TransformDirty};
use crate::engine::ecs::resources::Time;
use crate::engine::ecs::schedule::{RunIfPlaying, Stage};
use crate::engine::scripting::runner::{
    AssetGraphLoader, CurveCache, GraphLoader, GraphPlanCache, GraphRuntime,
    GraphScriptRunnerSystem,
};
use crate::engine::scripting::GraphRunner;

use super::{
    EnginePlugin, PluginContext, PluginError, PluginKind, PluginManifest, PluginOrigin,
    GRAPH_SCRIPTING_ID,
};

/// The Cargo feature that compiles this plugin and the interpreter in.
pub const GRAPH_SCRIPTING_FEATURE: &str = "graph-scripting";

/// Graph execution: the standard node library, the runner system and the
/// compiled-plan cache.
pub struct GraphScriptingPlugin {
    /// Where `.graph` and `.prefab` assets are read from.
    pub content_root: PathBuf,
    /// How the runner obtains documents. The default reads the content root
    /// through the asset source (pak-aware); overriding it is how tests run
    /// the *real* plugin against in-memory documents instead of stubbing the
    /// system out, and it is the seam the editor will later use to prefer an
    /// open-but-unsaved document over the one on disk.
    loader: LoaderFactory,
}

type LoaderFactory = Box<dyn Fn() -> Box<dyn GraphLoader + Send + Sync> + Send + Sync>;

impl GraphScriptingPlugin {
    pub fn new(content_root: impl Into<PathBuf>) -> Self {
        let content_root = content_root.into();
        let root = content_root.clone();
        Self {
            content_root,
            loader: Box::new(move || {
                Box::new(AssetGraphLoader {
                    content_root: root.clone(),
                })
            }),
        }
    }

    /// Build the plugin against a caller-supplied loader.
    pub fn with_loader(content_root: impl Into<PathBuf>, loader: LoaderFactory) -> Self {
        Self {
            content_root: content_root.into(),
            loader,
        }
    }
}

impl EnginePlugin for GraphScriptingPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new(GRAPH_SCRIPTING_ID, "Graph Scripting")
            .with_description(
                "Runs .graph assets attached to entities: the standard node library, the \
                 runner system and the compiled-plan cache. Disabling it makes GraphRunner \
                 components inert and leaves graph documents readable but unrunnable.",
            )
            .with_author("rust-engine")
            .with_origin(PluginOrigin::Engine)
            .with_kind(PluginKind::Runtime)
            .with_cargo_feature(GRAPH_SCRIPTING_FEATURE)
            .with_module_path("engine/src/engine/plugins/graph_scripting.rs")
    }

    fn build(&self, ctx: &mut PluginContext) -> Result<(), PluginError> {
        // 1. Descriptors. Registered one at a time through the staged
        //    context, which merges with whatever other plugins registered —
        //    `dev_nodes` keeps its own set, and only an id collision would be
        //    skipped (with a warning).
        for desc in node_graph_types::std_node_descriptors() {
            ctx.register_node(desc);
        }
        for desc in node_graph_types::std_event_descriptors() {
            ctx.register_node(desc);
        }

        // 2. The compiled-plan cache. One per app: two entities running the
        //    same graph share a compilation.
        ctx.insert_resource(GraphPlanCache::new());
        // …and the `.curve` assets those plans reference (45-A P8). Loaded by
        // the compiler, sampled by the interpreter — one copy, so a Timeline's
        // pins and its values cannot come from different files.
        ctx.insert_resource(CurveCache::new());

        // 3. The runner. `RunIfPlaying` for the same reason physics uses it —
        //    standalone forces `Playing` at startup, so a shipped game always
        //    passes.
        //
        //    `Stage::Update`, after physics has moved things: a graph reading
        //    a position should see this frame's, not last frame's.
        //
        //    Access is declared precisely rather than as `exclusive()`.
        //    Exclusive conflicts with *everything* in its stage, and the
        //    validator rejects an unresolved conflict — so claiming it would
        //    mean naming every other Update system in an `after` edge, which
        //    is a coupling this plugin has no business having.
        //
        //    **KNOWN UNDERSTATEMENT (45-A review, finding 8).** The declared
        //    set below is not the whole truth. The system also:
        //      - spawns and despawns entities (a scheduled `System` never
        //        receives the command buffer — that belongs to the scheduler),
        //        which touches every component of the affected archetypes;
        //      - writes `Name`, `EntityGuid`, `RigidBody`, `Collider` and the
        //        hierarchy components on spawn, none of which are declared;
        //      - writes `PhysicsWorld` when it registers or removes a
        //        graph-spawned body.
        //    The sequential executor makes this safe today — structural work
        //    between systems cannot race anything — and declaring it honestly
        //    would mean either `exclusive()` (rejected above) or a
        //    structural-access bit the descriptor does not have.
        //
        //    This is therefore a **prerequisite for the parallel executor**
        //    (Task 58), recorded in the 45-A deferred ledger: `SystemDescriptor`
        //    needs a way to say "structural" before this declaration can be
        //    made true, and until then no parallel schedule may include it.
        ctx.add_system_with_criteria(
            GraphScriptRunnerSystem::new((self.loader)(), self.content_root.clone()),
            Stage::Update,
            SystemDescriptor::new("GraphScriptRunnerSystem")
                .reads_resource::<Time>()
                .writes_resource::<GraphPlanCache>()
                .writes_resource::<CurveCache>()
                .reads::<GraphRunner>()
                .writes::<GraphRuntime>()
                .writes::<Transform>()
                .writes::<TransformDirty>()
                // Both client gameplay systems move things too, and the
                // ordering edge is meaningful rather than a formality: a graph
                // should see this frame's movement, not last frame's.
                .after("PlayerInputSystem")
                .after("CharacterMovementSystem"),
            RunIfPlaying,
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The plugin stages its descriptors, its resource and its system, and
    /// says the right things in its manifest — including the Cargo feature
    /// that makes the export gate real.
    #[test]
    fn the_plugin_stages_what_it_claims() {
        let m = GraphScriptingPlugin::new("content").manifest();
        assert_eq!(m.id, GRAPH_SCRIPTING_ID);
        assert_eq!(m.origin, PluginOrigin::Engine);
        assert_eq!(m.kind, PluginKind::Runtime);
        assert_eq!(
            m.cargo_feature.as_deref(),
            Some(GRAPH_SCRIPTING_FEATURE),
            "the export policy reads this"
        );
        assert!(!m.internal, "it must appear in the Plugin Manager");
    }

    /// Registrations **compose**: the standard library and the dev node set
    /// are separate plugins and both end up in one registry. This is the
    /// parity the 39.8 dev_nodes test asserts, seen from the other side.
    #[cfg(feature = "dev_nodes")]
    #[test]
    fn std_nodes_and_dev_nodes_compose_in_one_registry() {
        use crate::engine::plugins::PluginSet;
        use node_graph_types::NodeRegistry;
        let mut set = PluginSet::new();
        set.add(GraphScriptingPlugin::new("content"));
        set.add(crate::engine::plugins::DevNodesPlugin);

        let mut registry = NodeRegistry::new();
        let mut resources = crate::engine::ecs::resources::Resources::new();
        let mut schedule = crate::engine::ecs::schedule::Schedule::new();
        set.build_all(
            crate::engine::plugins::PluginTargets {
                schedule: &mut schedule,
                resources: &mut resources,
                node_registry: &mut registry,
            },
            None,
        );
        assert!(set.failures().is_empty(), "{:?}", set.failures());

        // The standard library is there…
        assert!(registry.get(node_graph_types::std_nodes::BRANCH).is_some());
        assert!(registry.get(node_graph_types::EVENT_TICK_TYPE_ID).is_some());
        // …and so is the dev set, untouched.
        assert!(registry.get("test_add").is_some());
        assert!(
            resources.get::<GraphPlanCache>().is_some(),
            "the cache resource is staged"
        );
    }
}
