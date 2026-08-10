//! `RapierPhysicsPlugin` — the physics stepping lifecycle as a plugin
//! (Task 39.8 D7).
//!
//! **Module first, not a workspace crate** (resolved 2026-08-10): it registers
//! only through `PluginContext`, so promoting it to a crate later is a move,
//! not a redesign. Premature `pub` API surface is not.
//!
//! ## What this owns — what *advances* physics
//!
//! 1. `PhysicsStepSystem` registration (stage + run criteria unchanged).
//! 2. Body/collider registration at every content moment, via
//!    `on_world_loaded`.
//! 3. The collider debug overlay, via `register_debug_draw`.
//!
//! ## What stays in engine core
//!
//! The `PhysicsWorld` resource itself (always inserted, inert when nothing
//! steps it), the play-mode rebuild plumbing, the component definitions, scene
//! serialization and the inspector UI. The review pass killed the
//! "no `PhysicsWorld` resource when disabled" version: the resource is
//! load-bearing far outside this boundary (profiler counters, play-mode
//! enter/restore, the benchmark loader, standalone `load_world`), and an
//! unstepped `PhysicsWorld` is already inert — edit mode proves that daily.
//!
//! ## Honesty clause
//!
//! This is a stepping-lifecycle extraction, not a full componentized
//! decoupling. It proves registration, the dependency cascade, lifecycle
//! callbacks and the manifest/restart loop on the most-coupled subsystem.
//! Plugin-provided *components* remain out of scope (D2) — they need the
//! reflection-lite arc.
//!
//! ## With the plugin disabled
//!
//! No step system, no bodies created anywhere (play-mode enter included — the
//! editor gates its resync on [`PluginSet::is_active`]), no collider overlay.
//! Scenes with physics components still load, save and display losslessly;
//! the inspector says the components are inert.
//!
//! [`PluginSet::is_active`]: super::PluginSet::is_active

use crate::engine::ecs::access::SystemDescriptor;
use crate::engine::ecs::components::{Transform, TransformDirty};
use crate::engine::ecs::resources::Time;
use crate::engine::ecs::schedule::{RunIfPlaying, Stage};
use crate::engine::physics::{
    rebuild_bodies_from_world, submit_collider_debug_draws, Collider, PhysicsStepSystem,
    PhysicsWorld, RigidBody, Velocity,
};

use super::{
    EnginePlugin, PluginContext, PluginError, PluginKind, PluginManifest, PluginOrigin,
};

/// The manifest id. Permanent: `project.ron` entries name it, and
/// `ClientGamePlugin` depends on it.
pub const PHYSICS_RAPIER_ID: &str = "physics_rapier";

/// Rapier 3D stepping, body registration and the collider overlay.
pub struct RapierPhysicsPlugin;

impl EnginePlugin for RapierPhysicsPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new(PHYSICS_RAPIER_ID, "Rapier Physics")
            .with_description(
                "Steps Rapier 3D, registers rigid bodies and colliders at every content \
                 moment, and draws the collider overlay. Disabling it makes physics \
                 components inert — a scene-editing configuration, not a playable one.",
            )
            .with_author("rust-engine")
            .with_origin(PluginOrigin::Engine)
            .with_kind(PluginKind::Runtime)
    }

    fn build(&self, ctx: &mut PluginContext) -> Result<(), PluginError> {
        // 1. Stepping. `RunIfPlaying` in both binaries: standalone forces
        //    `PlayMode::Playing` at startup precisely so this criteria is
        //    always true there (see `StandaloneApp::new`), which is how every
        //    other gameplay system already behaves in a shipped game.
        ctx.add_system_with_criteria(
            PhysicsStepSystem,
            Stage::PreUpdate,
            SystemDescriptor::new("PhysicsStepSystem")
                .reads_resource::<Time>()
                .writes_resource::<PhysicsWorld>()
                .writes::<Transform>()
                .writes::<TransformDirty>()
                .reads::<RigidBody>()
                .reads::<Collider>()
                .reads::<Velocity>()
                .after("AnimationUpdateSystem"),
            RunIfPlaying,
        );

        // 2. Bodies, at every content moment. Registration at startup is not
        //    enough — under registries-then-content the world is empty when
        //    this runs, and every later load replaces its contents.
        ctx.on_world_loaded(|world, resources| {
            if let Some(physics_world) = resources.get_mut::<PhysicsWorld>() {
                rebuild_bodies_from_world(physics_world, world);
            }
            Ok(())
        });

        // 3. Collider overlay. The render path no longer knows about physics.
        ctx.register_debug_draw(|world, buffer| {
            submit_collider_debug_draws(world, buffer);
        });

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::debug_draw::DebugDrawBuffer;
    use crate::engine::ecs::resources::{EditorState, PlayMode, Resources};
    use crate::engine::ecs::schedule::Schedule;
    use crate::engine::node_graph::NodeRegistry;
    use crate::engine::plugins::{PluginEntry, PluginSet, PluginTargets};

    /// A world with one dynamic body, the shape every content moment produces.
    fn world_with_a_body() -> hecs::World {
        let mut world = hecs::World::new();
        world.spawn((
            Transform::new(nalgebra_glm::vec3(0.0, 0.0, 5.0)),
            RigidBody::dynamic(),
            Collider::cuboid(0.5, 0.5, 0.5),
        ));
        world
    }

    struct Harness {
        schedule: Schedule,
        resources: Resources,
        registry: NodeRegistry,
    }

    impl Harness {
        fn new() -> Self {
            let mut resources = Resources::new();
            // Core always inserts PhysicsWorld, plugin or no plugin (D7).
            resources.insert(PhysicsWorld::new());
            resources.insert(EditorState::new());
            Self {
                schedule: Schedule::new(),
                resources,
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

        /// Core registers `AnimationUpdateSystem` before any plugin builds;
        /// `PhysicsStepSystem` orders itself after it, so a harness that omits
        /// it would see a dangling edge that the real binaries never have.
        fn with_core_systems(mut self) -> Self {
            struct Anim;
            impl crate::engine::ecs::schedule::System for Anim {
                fn run(&mut self, _w: &mut hecs::World, _r: &mut Resources) {}
                fn name(&self) -> &str {
                    "AnimationUpdateSystem"
                }
            }
            self.schedule.add_system_described(
                Anim,
                Stage::PreUpdate,
                SystemDescriptor::new("AnimationUpdateSystem"),
            );
            self
        }

        fn bodies(&self) -> usize {
            self.resources
                .get::<PhysicsWorld>()
                .map(|pw| pw.rigid_body_set.len())
                .unwrap_or(0)
        }
    }

    #[test]
    fn enabled_registers_the_step_system_bodies_and_the_overlay() {
        let mut h = Harness::new();
        let mut set = PluginSet::new();
        set.add(RapierPhysicsPlugin);
        h.build(&mut set, None);

        assert!(set.failures().is_empty(), "{:?}", set.failures());
        assert!(set.is_active(PHYSICS_RAPIER_ID));
        assert_eq!(h.schedule.system_count(), 1, "the step system is registered");

        let counts = set.records()[0].counts;
        assert_eq!(counts.systems, 1);
        assert_eq!(counts.world_loaded_callbacks, 1);
        assert_eq!(counts.debug_draw_hooks, 1);
        assert_eq!(counts.resources, 0, "the PhysicsWorld stays core-owned");

        // The content moment creates the bodies.
        let mut world = world_with_a_body();
        assert_eq!(h.bodies(), 0, "nothing is registered before a content moment");
        assert!(set.run_world_loaded(&mut world, &mut h.resources).is_empty());
        assert_eq!(h.bodies(), 1);

        // And re-running it (a second load) does not double them.
        assert!(set.run_world_loaded(&mut world, &mut h.resources).is_empty());
        assert_eq!(h.bodies(), 1);

        let mut buffer = DebugDrawBuffer::new();
        set.run_debug_draw(&world, &mut buffer);
    }

    /// The D7 acceptance: disabled means nothing steps and no handle is ever
    /// created, while `PhysicsWorld` itself still exists.
    #[test]
    fn disabled_steps_nothing_and_creates_no_handles() {
        let mut h = Harness::new();
        let mut set = PluginSet::new();
        set.add(RapierPhysicsPlugin);
        h.build(&mut set, Some(&[PluginEntry::new(PHYSICS_RAPIER_ID, false)]));

        assert!(!set.is_active(PHYSICS_RAPIER_ID));
        assert_eq!(set.disabled(), [PHYSICS_RAPIER_ID.to_string()]);
        assert_eq!(h.schedule.system_count(), 0, "no step system");
        assert_eq!(set.world_loaded_count(), 0, "no body registration hook");

        let mut world = world_with_a_body();
        assert!(set.run_world_loaded(&mut world, &mut h.resources).is_empty());
        assert_eq!(h.bodies(), 0, "a content moment creates no handles");

        // The resource is still there and still inert — every site that
        // `expect()`s it keeps working.
        assert!(h.resources.get::<PhysicsWorld>().is_some());

        // The overlay draws nothing.
        let mut buffer = DebugDrawBuffer::new();
        set.run_debug_draw(&world, &mut buffer);

        // The components survive untouched: no handles were stamped into them.
        for (_, rb) in world.query::<&RigidBody>().iter() {
            assert!(rb.handle.is_none());
        }
        for (_, c) in world.query::<&Collider>().iter() {
            assert!(c.handle.is_none());
        }
    }

    /// The D7 cascade, and the specific hazard it creates: `PlayerInputSystem`
    /// declares `.after("PhysicsStepSystem")`, and schedule validation rejects
    /// ordering edges to absent systems. Disabling physics must therefore take
    /// its dependents with it — if the dependent survived, its edge would
    /// dangle and validation would fail the whole editor.
    #[test]
    fn disabling_physics_cascades_to_dependents_and_leaves_no_dangling_edge() {
        struct Gameplay;
        impl crate::engine::ecs::schedule::System for Gameplay {
            fn run(&mut self, _w: &mut hecs::World, _r: &mut Resources) {}
            fn name(&self) -> &str {
                "PlayerInputSystem"
            }
        }
        struct GameplayPlugin;
        impl EnginePlugin for GameplayPlugin {
            fn manifest(&self) -> PluginManifest {
                PluginManifest::new("game_client", "Game")
                    .depends_on([PHYSICS_RAPIER_ID])
                    .internal()
            }
            fn build(&self, ctx: &mut PluginContext) -> Result<(), PluginError> {
                ctx.add_system(
                    Gameplay,
                    Stage::Update,
                    SystemDescriptor::new("PlayerInputSystem").after("PhysicsStepSystem"),
                );
                Ok(())
            }
        }

        // Physics on: both build, the edge resolves, validation is clean.
        let mut h = Harness::new().with_core_systems();
        let mut set = PluginSet::new();
        set.add(RapierPhysicsPlugin);
        set.add(GameplayPlugin);
        h.build(&mut set, None);
        assert!(set.failures().is_empty(), "{:?}", set.failures());
        assert_eq!(h.schedule.system_count(), 3, "core anim + physics + gameplay");
        let on_errors = h.schedule.validate();
        assert!(
            on_errors.is_empty(),
            "validation must pass with physics on: {on_errors:?}"
        );

        // Physics off: gameplay goes with it, and nothing is left referring
        // to the absent step system.
        let mut h = Harness::new().with_core_systems();
        let mut set = PluginSet::new();
        set.add(RapierPhysicsPlugin);
        set.add(GameplayPlugin);
        h.build(&mut set, Some(&[PluginEntry::new(PHYSICS_RAPIER_ID, false)]));

        assert!(set.records().is_empty(), "neither plugin builds");
        assert_eq!(set.failures().len(), 1);
        assert_eq!(set.failures()[0].id, "game_client");
        assert!(matches!(
            set.failures()[0].error,
            crate::engine::plugins::PluginError::MissingDependency { .. }
        ));
        assert!(!set.is_active("game_client"), "drives the toolbar hint");

        assert_eq!(h.schedule.system_count(), 1, "only the core system remains");
        let errors = h.schedule.validate();
        assert!(
            errors.is_empty(),
            "the after(PhysicsStepSystem) edge must not dangle: {errors:?}"
        );
    }

    /// The step system carries `RunIfPlaying`, so an editor in edit mode never
    /// advances physics even with the plugin enabled.
    #[test]
    fn step_system_runs_only_in_play_mode() {
        let mut h = Harness::new();
        let mut set = PluginSet::new();
        set.add(RapierPhysicsPlugin);
        h.build(&mut set, None);

        let mut world = world_with_a_body();
        let _ = set.run_world_loaded(&mut world, &mut h.resources);
        let start_z = world
            .query::<&Transform>()
            .iter()
            .next()
            .map(|(_, t)| t.position.z)
            .expect("one body");

        let mut commands = crate::engine::ecs::commands::CommandBuffer::new();
        if let Some(t) = h.resources.get_mut::<Time>() {
            t.delta = 1.0 / 60.0;
        } else {
            h.resources.insert(Time::new());
        }

        // Edit mode: nothing moves.
        for _ in 0..30 {
            h.schedule
                .run_raw(&mut world, &mut h.resources, &mut commands);
        }
        let edit_z = world
            .query::<&Transform>()
            .iter()
            .next()
            .map(|(_, t)| t.position.z)
            .expect("one body");
        assert_eq!(edit_z, start_z, "physics must not step in edit mode");

        // Playing: it falls.
        if let Some(state) = h.resources.get_mut::<EditorState>() {
            state.play_mode = PlayMode::Playing;
        }
        if let Some(t) = h.resources.get_mut::<Time>() {
            t.delta = 1.0 / 60.0;
        }
        for _ in 0..60 {
            h.schedule
                .run_raw(&mut world, &mut h.resources, &mut commands);
        }
        let played_z = world
            .query::<&Transform>()
            .iter()
            .next()
            .map(|(_, t)| t.position.z)
            .expect("one body");
        assert!(
            played_z < start_z,
            "gravity should pull the body down once stepping ({played_z} vs {start_z})"
        );
    }
}
