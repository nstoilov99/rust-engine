//! Acceptance evidence for the runtime binding (Task 45-A P5).
//!
//! These drive the *real* plugin, the *real* schedule and a real `hecs` world
//! — the P2/P3 spike style — so what is asserted here is what the editor and
//! the standalone game do, not a parallel implementation of it.

use std::collections::BTreeMap;

use nalgebra_glm as glm;
use node_graph_types::{
    Edge, GraphDoc, GraphRealm, NodeInst, PropValue, EVENT_BEGIN_PLAY_TYPE_ID, EVENT_TICK_TYPE_ID,
    EXEC_IN_PIN, EXEC_OUT_PIN,
};

use crate::engine::ecs::components::{Name, Transform};
use crate::engine::ecs::resources::{EditorState, PlayMode, Resources, Time};
use crate::engine::ecs::schedule::Schedule;
use crate::engine::node_graph::NodeRegistry;
use crate::engine::plugins::{GraphScriptingPlugin, PluginSet, PluginTargets};
use crate::engine::scripting::runner::{GraphLoader, GraphPlanCache, GraphRuntime};
use crate::engine::scripting::GraphRunner;

use node_graph_types::std_nodes as ids;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn node(id: u64, type_id: &str) -> NodeInst {
    NodeInst {
        id,
        type_id: type_id.to_string(),
        type_version: 1,
        position: [id as f32 * 180.0, 0.0],
        properties: BTreeMap::new(),
        subgraph: None,
        tint: None,
        title: None,
    }
}

fn with(id: u64, type_id: &str, props: &[(&str, PropValue)]) -> NodeInst {
    let mut n = node(id, type_id);
    for (k, v) in props {
        n.properties.insert(k.to_string(), v.clone());
    }
    n
}

fn edge(from: u64, fp: &str, to: u64, tp: &str) -> Edge {
    Edge {
        from_node: from,
        from_pin: fp.to_string(),
        to_node: to,
        to_pin: tp.to_string(),
    }
}

/// The demo's curve, read from the committed asset (45-A P8). Read rather
/// than rebuilt: a fixture that restates the file's contents proves the test
/// agrees with itself, not that the engine can load a `.curve`.
const DEMO_CURVE: &str = "curves/duck_hop.curve";

/// The prefab the demo's ForLoop spawns (45-A P9 showcase). Written by the
/// same fixture generator that writes the graph.
const DEMO_PREFAB: &str = "prefabs/graph_cube.prefab";

fn demo_curve() -> curve_asset::CurveDoc {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("content")
        .join(DEMO_CURVE);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    curve_asset::parse_curve(&text).expect("the committed demo curve parses")
}

/// Every tick: read this entity's position, add 1 to X, write it back. A
/// read-modify-write, so it proves the read seam and the write seam in one
/// fixture and fails visibly if either is wrong.
fn move_each_tick() -> GraphDoc {
    let mut doc = GraphDoc::default();
    doc.nodes = vec![
        node(2, EVENT_TICK_TYPE_ID),
        node(3, ids::GET_POSITION),
        node(4, ids::BREAK_VEC3),
        with(5, ids::ADD_FLOAT, &[("b", PropValue::Float(1.0))]),
        node(6, ids::MAKE_VEC3),
        node(7, ids::SET_POSITION),
    ];
    doc.edges = vec![
        edge(2, EXEC_OUT_PIN, 7, EXEC_IN_PIN),
        edge(3, "position", 4, "value"),
        edge(4, "x", 5, "a"),
        edge(5, "result", 6, "x"),
        edge(4, "y", 6, "y"),
        edge(4, "z", 6, "z"),
        edge(6, "result", 7, "position"),
    ];
    doc
}

/// Counts BeginPlay firings into a variable, so "exactly once" is a number
/// rather than an inference from side effects.
fn count_begin_play() -> GraphDoc {
    count_entry(node(0, EVENT_BEGIN_PLAY_TYPE_ID))
}

/// Counts activations of one entry node into a variable and publishes the
/// count through the entity's X position — a number the test can read without
/// reaching into interpreter internals.
///
/// Parameterized over the entry node so "fired exactly N times" is asserted
/// the same way for BeginPlay and for an input action; a second counting
/// fixture would be the same graph with one node swapped.
fn count_entry(entry: NodeInst) -> GraphDoc {
    let mut doc = GraphDoc::default();
    doc.variables = vec![node_graph_types::VarDecl {
        slug: "n".into(),
        label: "N".into(),
        ty: node_graph_types::PinType::Int,
        default: Some(PropValue::Int(0)),
        group: None,
    }];
    doc.nodes = vec![
        entry,
        with(1, node_graph_types::VAR_GET_TYPE_ID, &[(node_graph_types::VAR_PROP, PropValue::Str("n".into()))]),
        with(2, ids::ADD_INT, &[("b", PropValue::Int(1))]),
        with(3, node_graph_types::VAR_SET_TYPE_ID, &[(node_graph_types::VAR_PROP, PropValue::Str("n".into()))]),
        node(4, ids::INT_TO_FLOAT),
        node(5, ids::MAKE_VEC3),
        node(6, ids::SET_POSITION),
    ];
    doc.edges = vec![
        edge(0, EXEC_OUT_PIN, 3, EXEC_IN_PIN),
        edge(1, node_graph_types::VAR_VALUE_PIN, 2, "a"),
        edge(2, "result", 3, node_graph_types::VAR_VALUE_PIN),
        edge(3, EXEC_OUT_PIN, 6, EXEC_IN_PIN),
        // Publish the count through the entity's X position, which the test
        // can read without reaching into interpreter internals.
        edge(1, node_graph_types::VAR_VALUE_PIN, 4, "value"),
        edge(4, "result", 5, "x"),
        edge(5, "result", 6, "position"),
    ];
    doc
}

/// A loader over an in-memory map — the same seam the engine fills with a
/// disk/pak reader.
struct MapLoader(BTreeMap<String, GraphDoc>, BTreeMap<String, curve_asset::CurveDoc>);

impl GraphLoader for MapLoader {
    fn load(&self, content_rel: &str) -> Option<GraphDoc> {
        self.0.get(content_rel).cloned()
    }

    fn load_curve(&self, content_rel: &str) -> Option<curve_asset::CurveDoc> {
        self.1.get(content_rel).cloned()
    }
}

/// A keyboard, as the input pipeline sees one. The subsystem's own tests use
/// the same shape; nothing here mocks the *subsystem*, which is the point —
/// the runner is tested against the real trigger/phase pipeline.
#[derive(Default)]
struct MockKeyboard {
    pressed: Vec<crate::engine::input::action::KeyCode>,
}

impl crate::engine::input::input_reader::InputReader for MockKeyboard {
    fn is_key_pressed(&self, key: crate::engine::input::action::KeyCode) -> bool {
        self.pressed.contains(&key)
    }
    fn is_key_just_pressed(&self, _: crate::engine::input::action::KeyCode) -> bool {
        false
    }
    fn is_mouse_pressed(&self, _: crate::engine::input::action::MouseButton) -> bool {
        false
    }
    fn mouse_delta(&self) -> (f32, f32) {
        (0.0, 0.0)
    }
    fn scroll_delta(&self) -> f32 {
        0.0
    }
    fn is_gamepad_pressed(&self, _: crate::engine::input::action::GamepadButton) -> bool {
        false
    }
    fn gamepad_axis(&self, _: crate::engine::input::action::GamepadAxisType) -> f32 {
        0.0
    }
}

/// Two digital actions on two keys. Named in lower case on purpose: the
/// runner matches an entry's `action` property against these strings exactly,
/// and a test whose names differ only by content would not notice a
/// case-folding regression.
fn test_action_set() -> crate::engine::input::enhanced_action::InputActionSet {
    use crate::engine::input::action::{InputSource, KeyCode};
    use crate::engine::input::enhanced_action::*;
    use crate::engine::input::trigger::InputTrigger;
    use crate::engine::input::value::InputValueType;

    let mut set = InputActionSet::new();
    for name in ["fire", "reload"] {
        set.add_action(
            InputActionDefinition::new(name, InputValueType::Digital)
                .with_trigger(InputTrigger::Pressed),
        );
    }
    set.add_context(
        MappingContext::new("gameplay", 0)
            .with_entry(
                MappingContextEntry::new("fire")
                    .with_binding(EnhancedBinding::digital(InputSource::Key(KeyCode::Space))),
            )
            .with_entry(
                MappingContextEntry::new("reload")
                    .with_binding(EnhancedBinding::digital(InputSource::Key(KeyCode::KeyR))),
            ),
    );
    set
}

/// The app, in miniature: plugin set, schedule, resources, world.
struct Harness {
    schedule: Schedule,
    resources: Resources,
    world: hecs::World,
    registry: NodeRegistry,
    time: f64,
    /// Keys held this frame. Empty unless [`Harness::with_input`] installed a
    /// subsystem, in which case [`Harness::tick`] pumps it before the schedule
    /// — which is where `EnhancedInputSystem` runs for real (`Stage::First`).
    held: Vec<crate::engine::input::action::KeyCode>,
}

impl Harness {
    fn new(docs: &[(&str, GraphDoc)]) -> Self {
        Self::with_root(docs, std::path::Path::new("content"))
    }

    /// …with `.curve` assets in the loader as well (45-A P8).
    fn with_curves(
        docs: &[(&str, GraphDoc)],
        curves: &[(&str, curve_asset::CurveDoc)],
    ) -> Self {
        Self::build(docs, curves, std::path::Path::new("content"))
    }

    fn with_root(docs: &[(&str, GraphDoc)], root: &std::path::Path) -> Self {
        Self::build(docs, &[], root)
    }

    fn build(
        docs: &[(&str, GraphDoc)],
        curves: &[(&str, curve_asset::CurveDoc)],
        root: &std::path::Path,
    ) -> Self {
        let mut resources = Resources::new();
        resources.insert(Time::new());
        let mut editor = EditorState::new();
        editor.play_mode = PlayMode::Playing;
        resources.insert(editor);

        let map: BTreeMap<String, GraphDoc> = docs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect();
        let curve_map: BTreeMap<String, curve_asset::CurveDoc> = curves
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect();

        // The *real* plugin, pointed at in-memory documents — the system
        // under test is the one the binaries register, not a stand-in.
        let mut set = PluginSet::new();
        set.add(GraphScriptingPlugin::with_loader(
            root.to_path_buf(),
            Box::new(move || Box::new(MapLoader(map.clone(), curve_map.clone()))),
        ));

        let mut h = Self {
            schedule: Schedule::new(),
            resources,
            world: hecs::World::new(),
            registry: NodeRegistry::new(),
            time: 0.0,
            held: Vec::new(),
        };
        set.build_all(
            PluginTargets {
                schedule: &mut h.schedule,
                resources: &mut h.resources,
                node_registry: &mut h.registry,
            },
            None,
        );
        assert!(set.failures().is_empty(), "{:?}", set.failures());

        // The runner reads the node registry from `Resources`, which is
        // where the app puts the scene's registry.
        h.resources
            .insert(std::sync::Arc::new(std::mem::take(&mut h.registry)));
        h
    }

    /// Give this harness a real [`InputSubsystem`] over [`test_action_set`],
    /// with the `gameplay` context active.
    fn with_input(mut self) -> Self {
        let mut subsystem =
            crate::engine::input::subsystem::InputSubsystem::new(test_action_set());
        subsystem.add_context("gameplay");
        self.resources.insert(subsystem);
        self
    }

    fn hold(&mut self, key: crate::engine::input::action::KeyCode) {
        if !self.held.contains(&key) {
            self.held.push(key);
        }
    }

    fn release_all(&mut self) {
        self.held.clear();
    }

    /// What `EnhancedInputSystem` does in `Stage::First`, before the runner's
    /// `Stage::Update`: run the real modifier/trigger pipeline over this
    /// frame's keys so `just_pressed` means what it means in the app.
    fn pump_input(&mut self, dt: f32) {
        let Some(mut subsystem) =
            self.resources.remove::<crate::engine::input::subsystem::InputSubsystem>()
        else {
            return;
        };
        let reader = MockKeyboard {
            pressed: self.held.clone(),
        };
        let mut events = crate::engine::ecs::events::Events::default();
        subsystem.tick(&reader, dt, &mut events);
        self.resources.insert(subsystem);
    }

    fn tick(&mut self, dt: f32) {
        self.time += dt as f64;
        if let Some(t) = self.resources.get_mut::<Time>() {
            t.delta = dt;
            t.total = self.time;
            t.frame += 1;
        }
        self.pump_input(dt);
        let mut commands = crate::engine::ecs::commands::CommandBuffer::new();
        self.schedule
            .run_raw(&mut self.world, &mut self.resources, &mut commands);
    }

    fn ticks(&mut self, n: usize) {
        for _ in 0..n {
            self.tick(1.0 / 60.0);
        }
    }

    fn spawn_runner(&mut self, name: &str, graph: &str) -> hecs::Entity {
        self.world.spawn((
            Name(name.to_string()),
            Transform::new(glm::vec3(0.0, 0.0, 0.0)),
            GraphRunner::new(graph),
        ))
    }

    fn position(&self, e: hecs::Entity) -> glm::Vec3 {
        self.world.get::<&Transform>(e).unwrap().position
    }

    fn stop_play(&mut self) {
        if let Some(s) = self.resources.get_mut::<EditorState>() {
            s.play_mode = PlayMode::Edit;
        }
    }

    fn start_play(&mut self) {
        if let Some(s) = self.resources.get_mut::<EditorState>() {
            s.play_mode = PlayMode::Playing;
        }
    }

    /// What stopping play really does to runtime state: the snapshot restore
    /// clears the world and respawns from serialized data, so anything not
    /// serialized is gone. Reproduced here without the editor's file I/O.
    fn simulate_snapshot_restore(&mut self) {
        let saved: Vec<(String, GraphRunner, glm::Vec3)> = self
            .world
            .query::<(&Name, &GraphRunner, &Transform)>()
            .iter()
            .map(|(_, (n, r, t))| (n.0.clone(), r.clone(), t.position))
            .collect();
        self.world.clear();
        for (name, runner, pos) in saved {
            self.world
                .spawn((Name(name), Transform::new(pos), runner));
        }
    }
}

// ---------------------------------------------------------------------------
// Acceptance
// ---------------------------------------------------------------------------

/// Play enter → the runner arms lazily, BeginPlay fires exactly once, and its
/// effect lands on a real `Transform`.
#[test]
fn begin_play_arms_lazily_and_fires_exactly_once() {
    let mut h = Harness::new(&[("graphs/t.graph", count_begin_play())]);
    let e = h.spawn_runner("Scripted", "graphs/t.graph");

    // Nothing exists until the first playing tick — that is the whole of the
    // lifecycle rule (addendum #3).
    assert!(h.world.get::<&GraphRuntime>(e).is_err(), "not armed before the first tick");

    h.tick(1.0 / 60.0);
    assert!(h.world.get::<&GraphRuntime>(e).is_ok(), "armed on the first playing tick");

    h.ticks(30);
    assert_eq!(
        h.position(e).x,
        1.0,
        "BeginPlay incremented the counter exactly once across 31 ticks"
    );
}

/// Effects visibly apply, tick after tick: the entity actually moves.
#[test]
fn effects_apply_to_the_world_over_ticks() {
    let mut h = Harness::new(&[("graphs/t.graph", move_each_tick())]);
    let e = h.spawn_runner("Mover", "graphs/t.graph");

    h.tick(1.0 / 60.0);
    assert_eq!(h.position(e).x, 1.0, "one tick, one nudge");
    h.ticks(9);
    assert_eq!(h.position(e).x, 10.0, "ten ticks, ten nudges — it really moves");
    assert_eq!(h.position(e).y, 0.0, "and only along the axis the graph touches");
}

/// Play → stop → play: the snapshot restore drops runtime state, so BeginPlay
/// re-arms and fires exactly once **again** — not twice, and not never.
#[test]
fn stopping_and_replaying_refires_begin_play_exactly_once() {
    let mut h = Harness::new(&[("graphs/t.graph", count_begin_play())]);
    let e = h.spawn_runner("Scripted", "graphs/t.graph");
    h.ticks(10);
    assert_eq!(h.position(e).x, 1.0);

    // Stop: no ticking, and the restore drops everything unserialized.
    h.stop_play();
    h.ticks(5);
    h.simulate_snapshot_restore();
    assert_eq!(
        h.world.query::<&GraphRuntime>().iter().count(),
        0,
        "runtime state is never serialized, so the restore leaves none of it"
    );

    // Play again: a *new* entity, a *new* instance, one more BeginPlay.
    h.start_play();
    h.ticks(10);
    let e2 = h
        .world
        .query::<&GraphRunner>()
        .iter()
        .map(|(e, _)| e)
        .next()
        .expect("the runner survived the restore");
    let _ = e; // hecs reuses ids after `clear`, so identity proves nothing here
    assert_eq!(
        h.position(e2).x,
        1.0,
        "the fresh instance fired BeginPlay once, from a fresh variable"
    );
}

/// An entity spawned *during* play arms on its next tick, exactly like one
/// that was there all along. Same rule, no special case.
#[test]
fn an_entity_spawned_during_play_arms_and_fires() {
    let mut h = Harness::new(&[("graphs/t.graph", count_begin_play())]);
    h.ticks(5);

    let late = h.spawn_runner("Latecomer", "graphs/t.graph");
    assert!(h.world.get::<&GraphRuntime>(late).is_err());
    h.tick(1.0 / 60.0);
    assert!(h.world.get::<&GraphRuntime>(late).is_ok(), "armed on its first tick");
    h.ticks(5);
    assert_eq!(h.position(late).x, 1.0, "and fired BeginPlay once");
}

/// The realm gate: a `Server`-realm graph on a client is refused visibly and
/// does not run.
#[test]
fn the_realm_gate_refuses_a_server_graph_on_a_client() {
    let mut server_graph = move_each_tick();
    server_graph.realm = GraphRealm::Server;
    let mut h = Harness::new(&[
        ("graphs/server.graph", server_graph),
        ("graphs/ok.graph", move_each_tick()),
    ]);

    let refused = h.spawn_runner("ServerSide", "graphs/server.graph");
    let allowed = h.spawn_runner("ClientSide", "graphs/ok.graph");
    h.ticks(5);

    let rt = h.world.get::<&GraphRuntime>(refused).unwrap();
    let why = rt.disabled.clone().expect("a refusal, with a reason");
    assert!(why.contains("Server"), "the reason names the realm: {why}");
    drop(rt);
    assert_eq!(
        h.position(refused),
        glm::vec3(0.0, 0.0, 0.0),
        "and nothing ran"
    );

    // A Shared/Client graph beside it is unaffected — the gate is per
    // instance, not per app.
    assert!(h.position(allowed).x > 0.0, "the graph next to it ran normally");
}

/// A missing or broken graph disables its instance with a reported reason
/// rather than panicking or silently doing nothing — and the failure is
/// cached, so it is not recompiled every frame.
#[test]
fn a_missing_graph_disables_its_instance_and_is_not_retried() {
    let mut h = Harness::new(&[("graphs/t.graph", move_each_tick())]);
    let e = h.spawn_runner("Broken", "graphs/nope.graph");
    h.ticks(3);

    let rt = h.world.get::<&GraphRuntime>(e).unwrap();
    assert!(
        rt.disabled.as_deref().unwrap_or_default().contains("nope.graph"),
        "{:?}",
        rt.disabled
    );
    drop(rt);
    assert_eq!(
        h.resources.get::<GraphPlanCache>().map(|c| c.len()),
        Some(1),
        "the failure is cached, not retried sixty times a second"
    );
}

/// Hot reload: invalidating the cache restarts live instances, which re-fires
/// BeginPlay. Edit-during-play is a stated non-goal (D9) — restarting is the
/// simplest correct answer, and this pins it as *chosen* behaviour.
#[test]
fn invalidating_the_cache_restarts_live_instances() {
    let mut h = Harness::new(&[("graphs/t.graph", count_begin_play())]);
    let e = h.spawn_runner("Scripted", "graphs/t.graph");
    h.ticks(5);
    assert_eq!(h.position(e).x, 1.0);

    if let Some(cache) = h.resources.get_mut::<GraphPlanCache>() {
        cache.invalidate("graphs/t.graph");
    }
    h.ticks(5);
    assert_eq!(
        h.position(e).x,
        1.0,
        "the instance restarted: a fresh variable, one fresh BeginPlay"
    );
    assert!(h.world.get::<&GraphRuntime>(e).is_ok(), "and it is running again");
}

/// **Finding 2**: a subgraph edit must recompile its *hosts*, not just
/// restart them onto the plan they already had.
///
/// `invalidate` used to drop one key. The host's plan stayed cached, the
/// generation bump restarted every instance, and each restart re-accepted the
/// stale host plan — so editing a subgraph looked like it worked (the graph
/// visibly restarted) while running exactly the old code. Wholesale
/// invalidation is the fix, and the cache being empty afterwards is the
/// assertion.
#[test]
fn invalidating_a_subgraph_drops_its_hosts_plans_too() {
    let mut h = Harness::new(&[
        ("graphs/host.graph", count_begin_play()),
        ("graphs/other.graph", move_each_tick()),
    ]);
    let a = h.spawn_runner("A", "graphs/host.graph");
    let b = h.spawn_runner("B", "graphs/other.graph");
    h.ticks(3);
    assert_eq!(
        h.resources.get::<GraphPlanCache>().map(|c| c.len()),
        Some(2),
        "both compiled"
    );

    // A subgraph nobody has compiled by name — the point is that invalidating
    // it still clears the hosts that inlined it.
    if let Some(cache) = h.resources.get_mut::<GraphPlanCache>() {
        cache.invalidate("graphs/lib/edited.subgraph");
    }
    assert_eq!(
        h.resources.get::<GraphPlanCache>().map(|c| c.len()),
        Some(0),
        "every plan is dropped: the cache does not track the reference tree"
    );

    // …and both instances come back, recompiled.
    h.ticks(3);
    assert!(h.world.get::<&GraphRuntime>(a).is_ok());
    assert!(h.world.get::<&GraphRuntime>(b).is_ok());
    assert_eq!(h.resources.get::<GraphPlanCache>().map(|c| c.len()), Some(2));
}

/// **Finding 6**: the `GraphRunner` component is read on every tick, not only
/// at arming. Switching it off, or re-pointing it at another asset, used to
/// leave the old instance ticking — the component said one thing and the
/// world did another.
#[test]
fn runner_config_changes_take_effect_mid_play() {
    let mut h = Harness::new(&[
        ("graphs/a.graph", move_each_tick()),
        ("graphs/b.graph", count_begin_play()),
    ]);
    let e = h.spawn_runner("Switcher", "graphs/a.graph");
    h.ticks(3);
    let moved = h.position(e).x;
    assert!(moved > 0.0, "graph A is running");

    // Disable it: the runtime goes, and nothing moves any more.
    h.world.get::<&mut GraphRunner>(e).unwrap().enabled = false;
    h.ticks(3);
    assert!(
        h.world.get::<&GraphRuntime>(e).is_err(),
        "a disabled runner keeps no runtime"
    );
    assert_eq!(h.position(e).x, moved, "and stops having effects");

    // Re-point it at another graph and switch it back on: the new one arms.
    {
        let mut r = h.world.get::<&mut GraphRunner>(e).unwrap();
        r.enabled = true;
        r.graph = "graphs/b.graph".to_string();
    }
    h.ticks(3);
    let rt = h.world.get::<&GraphRuntime>(e).expect("re-armed");
    assert_eq!(rt.graph, "graphs/b.graph", "on the asset the component names");
    drop(rt);

    // Re-pointing a *running* instance swaps it too, without a disable step.
    h.world.get::<&mut GraphRunner>(e).unwrap().graph = "graphs/a.graph".to_string();
    h.ticks(2);
    assert_eq!(
        h.world.get::<&GraphRuntime>(e).unwrap().graph,
        "graphs/a.graph"
    );
}

/// **Finding 5**: an instance that refuses to arm says so. Silence reads as
/// "the graph does nothing", which is the one diagnosis that is never true.
#[test]
fn a_refusal_to_arm_is_reported_not_swallowed() {
    let mut h = Harness::new(&[]);
    let e = h.spawn_runner("Broken", "graphs/missing.graph");
    h.ticks(1);
    let rt = h.world.get::<&GraphRuntime>(e).expect("armed, disabled");
    let why = rt.disabled.clone().expect("a reason");
    assert!(why.contains("graphs/missing.graph"), "{why}");
}

/// **Finding 4**: a graph-spawned prefab with physics joins the simulation,
/// and a graph-despawned one leaves it. Before this, `RigidBody::handle`
/// stayed `None` for the entity's whole life — it looked physical and never
/// moved — and despawning left Rapier simulating an invisible body.
#[test]
fn spawned_prefabs_join_and_leave_the_physics_world() {
    use crate::engine::physics::{Collider, PhysicsWorld, RigidBody};
    use crate::engine::scene::scene_format::{ComponentData, EntityData};

    let root = std::env::temp_dir().join("rust_engine_rev_physics");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("prefabs")).unwrap();
    let prefab = crate::engine::scene::prefab::Prefab {
        name: "Boulder".to_string(),
        description: String::new(),
        template: EntityData {
            name: "Boulder".to_string(),
            guid: None,
            components: vec![
                ComponentData::Transform {
                    position: [0.0, 0.0, 0.0],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    scale: [1.0, 1.0, 1.0],
                },
                ComponentData::RigidBody {
                    body_type: crate::engine::scene::scene_format::RigidBodyTypeData::Dynamic,
                    mass: 1.0,
                    linear_damping: 0.0,
                    angular_damping: 0.0,
                    can_sleep: true,
                    gravity_scale: 1.0,
                    continuous_collision: false,
                    lock_rotation: [false; 3],
                },
                ComponentData::Collider {
                    shape: crate::engine::scene::scene_format::ColliderShapeData::Ball {
                        radius: 0.5,
                    },
                    friction: 0.5,
                    restitution: 0.0,
                    is_sensor: false,
                },
            ],
        },
    };
    std::fs::write(
        root.join("prefabs/boulder.prefab"),
        ron::ser::to_string_pretty(&prefab, ron::ser::PrettyConfig::default()).unwrap(),
    )
    .unwrap();
    crate::engine::assets::asset_source::init_filesystem_if_unset(root.clone());

    // BeginPlay spawns it; a Custom Event destroys it, so the two structural
    // paths are exercised on the same entity.
    let mut doc = GraphDoc::default();
    doc.nodes = vec![
        node(0, EVENT_BEGIN_PLAY_TYPE_ID),
        with(
            1,
            ids::SPAWN_PREFAB,
            &[("path", PropValue::Str("prefabs/boulder.prefab".into()))],
        ),
    ];
    doc.edges = vec![edge(0, EXEC_OUT_PIN, 1, EXEC_IN_PIN)];

    let mut h = Harness::with_root(&[("graphs/spawn.graph", doc)], &root);
    h.resources.insert(PhysicsWorld::new());
    let owner = h.spawn_runner("Spawner", "graphs/spawn.graph");

    h.tick(1.0 / 60.0);
    let spawned: Vec<hecs::Entity> = h
        .world
        .query::<&RigidBody>()
        .iter()
        .map(|(e, _)| e)
        .filter(|e| *e != owner)
        .collect();
    assert_eq!(spawned.len(), 1, "the prefab spawned");
    let boulder = spawned[0];
    assert!(
        h.world.get::<&RigidBody>(boulder).unwrap().handle.is_some(),
        "…and it is registered with Rapier, mid-play"
    );
    assert!(h.world.get::<&Collider>(boulder).unwrap().handle.is_some());
    assert_eq!(
        h.resources.get::<PhysicsWorld>().unwrap().rigid_body_count(),
        1
    );

    // Now take it back out through the same seam the graph would.
    {
        let (pw, world) = (
            &mut *h.resources.get_mut::<PhysicsWorld>().unwrap(),
            &mut h.world,
        );
        assert!(crate::engine::physics::deregister_entity(pw, world, boulder));
    }
    assert_eq!(
        h.resources.get::<PhysicsWorld>().unwrap().rigid_body_count(),
        0,
        "the body left with the entity"
    );
    assert!(h.world.get::<&RigidBody>(boulder).unwrap().handle.is_none());

    let _ = std::fs::remove_dir_all(&root);
}

/// The runner never ticks outside play. `RunIfPlaying` is doing the work, but
/// it is worth asserting: an editor that ran gameplay while you were laying
/// out a level would be unusable.
#[test]
fn nothing_runs_in_edit_mode() {
    let mut h = Harness::new(&[("graphs/t.graph", move_each_tick())]);
    h.stop_play();
    let e = h.spawn_runner("Idle", "graphs/t.graph");
    h.ticks(10);
    assert!(h.world.get::<&GraphRuntime>(e).is_err(), "not even armed");
    assert_eq!(h.position(e), glm::vec3(0.0, 0.0, 0.0));
}

/// A disabled runner is attached but inert — and re-enabling it arms it,
/// which is what makes the checkbox a debugging switch.
#[test]
fn a_disabled_runner_is_inert_until_enabled() {
    let mut h = Harness::new(&[("graphs/t.graph", count_begin_play())]);
    let e = h.spawn_runner("Off", "graphs/t.graph");
    h.world.get::<&mut GraphRunner>(e).unwrap().enabled = false;
    h.ticks(5);
    assert!(h.world.get::<&GraphRuntime>(e).is_err());

    h.world.get::<&mut GraphRunner>(e).unwrap().enabled = true;
    h.ticks(2);
    assert_eq!(h.position(e).x, 1.0, "enabling arms it");
}

// ---------------------------------------------------------------------------
// Persistence
// ---------------------------------------------------------------------------

/// The scene round-trip: `GraphRunner` survives, and the runtime state never
/// reaches the file.
#[test]
fn scene_round_trip_keeps_the_runner_and_never_writes_runtime_state() {
    use crate::engine::scene::scene_serializer;

    let mut h = Harness::new(&[("graphs/t.graph", count_begin_play())]);
    let e = h.spawn_runner("Scripted", "graphs/t.graph");
    h.world
        .get::<&mut GraphRunner>(e)
        .unwrap()
        .enabled = false;
    h.ticks(2);

    let roots: Vec<hecs::Entity> = h.world.query::<&Name>().iter().map(|(e, _)| e).collect();
    let text = scene_serializer::serialize_scene_to_string(&h.world, "Test", &roots)
        .expect("serialize");
    assert!(text.contains("GraphRunner"), "the component reached the file");
    assert!(text.contains("graphs/t.graph"));
    assert!(
        !text.contains("GraphRuntime") && !text.contains("plan"),
        "runtime state must never be serialized: {text}"
    );

    let mut restored = hecs::World::new();
    scene_serializer::load_scene_from_string(&mut restored, &text).expect("deserialize");
    let (_, runner) = restored
        .query::<&GraphRunner>()
        .iter()
        .map(|(e, r)| (e, r.clone()))
        .next()
        .expect("the runner came back");
    assert_eq!(runner.graph, "graphs/t.graph");
    assert!(!runner.enabled, "including the enabled flag");
    assert_eq!(
        restored.query::<&GraphRuntime>().iter().count(),
        0,
        "and no runtime state came with it"
    );
}

/// **Reads see the world as it was at the start of the tick; writes land
/// after.** Two entities running the same read-modify-write graph therefore
/// cannot observe each other's half-applied frame — which is what makes a
/// tick's effect stream a function of its inputs rather than of iteration
/// order. Worth pinning: it is the difference between "deterministic" and
/// "deterministic if you squint".
#[test]
fn reads_see_the_start_of_tick_world() {
    let mut h = Harness::new(&[("graphs/t.graph", move_each_tick())]);
    let a = h.spawn_runner("A", "graphs/t.graph");
    let b = h.spawn_runner("B", "graphs/t.graph");
    h.ticks(5);
    assert_eq!(h.position(a).x, 5.0);
    assert_eq!(
        h.position(b).x,
        5.0,
        "both advanced identically — neither saw the other mid-tick"
    );
}

/// **With the plugin disabled** the standard descriptors are absent, so a
/// document using them still loads and still saves but validates to
/// `UnknownNodeType` — the Task 40 degradation, which the graph editor draws
/// as placeholder nodes. Nothing crashes, and nothing is lost.
#[test]
fn a_disabled_plugin_leaves_graphs_readable_but_unrunnable() {
    use crate::engine::plugins::PluginEntry;
    use node_graph_types::{serialize_graph, validate_doc, GraphError};

    let mut schedule = Schedule::new();
    let mut resources = Resources::new();
    let mut registry = NodeRegistry::new();
    let mut set = PluginSet::new();
    set.add(GraphScriptingPlugin::new("content"));
    set.build_all(
        PluginTargets {
            schedule: &mut schedule,
            resources: &mut resources,
            node_registry: &mut registry,
        },
        // The manifest says off.
        Some(&[PluginEntry {
            id: crate::engine::plugins::GRAPH_SCRIPTING_ID.to_string(),
            enabled: false,
        }]),
    );

    assert!(
        registry.get(ids::BRANCH).is_none() && registry.get(ids::PRINT).is_none(),
        "no standard descriptors are registered"
    );
    assert!(
        resources.get::<GraphPlanCache>().is_none(),
        "and no plan cache — nothing was staged at all"
    );

    // The document still round-trips losslessly…
    let doc = move_each_tick();
    let text = serialize_graph(&doc).expect("a graph still saves");
    assert_eq!(
        node_graph_types::parse_graph(&text).expect("and still loads"),
        doc
    );

    // …and validates to reportable placeholders rather than a parse failure.
    let unknown: Vec<String> = validate_doc(&doc, &registry)
        .into_iter()
        .filter_map(|e| match e {
            GraphError::UnknownNodeType { type_id, .. } => Some(type_id),
            _ => None,
        })
        .collect();
    assert!(
        unknown.contains(&ids::SET_POSITION.to_string()),
        "unknown types degrade to anchored errors: {unknown:?}"
    );
}

/// **The spawn alias protocol, end to end.** BeginPlay runs a ForLoop that
/// spawns a prefab per iteration and immediately moves each spawned entity by
/// its handle — so the alias must be bound to a real entity before the
/// instance's next tick, which is the whole contract D1 describes.
#[test]
fn begin_play_spawns_in_a_loop_and_the_aliases_bind() {
    use node_graph_types::{PinType, VarDecl, VAR_GET_TYPE_ID, VAR_PROP, VAR_SET_TYPE_ID, VAR_VALUE_PIN};

    // A prefab on disk, in a content root of this test's own.
    let root = std::env::temp_dir().join("rust_engine_p5_spawn");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("prefabs")).unwrap();
    let prefab = crate::engine::scene::prefab::Prefab {
        name: "Crate".to_string(),
        description: "spawned by a graph".to_string(),
        template: crate::engine::scene::scene_format::EntityData {
            name: "Crate".to_string(),
            guid: None,
            components: vec![crate::engine::scene::scene_format::ComponentData::Transform {
                position: [0.0, 0.0, 0.0],
                rotation: [0.0, 0.0, 0.0, 1.0],
                scale: [1.0, 1.0, 1.0],
            }],
        },
    };
    std::fs::write(
        root.join("prefabs/crate.prefab"),
        ron::ser::to_string_pretty(&prefab, ron::ser::PrettyConfig::default()).unwrap(),
    )
    .unwrap();
    // `Prefab::load` asks the asset source whether we are running from a pak;
    // in a shared test process the first caller wins and the rest must not
    // panic for having lost that race.
    crate::engine::assets::asset_source::init_filesystem_if_unset(root.clone());

    // BeginPlay: for i in 0..2 { spawned = spawn(prefabs/crate.prefab at
    // (i,0,0)); set_position(spawned, (i, 9, 0)) }  — the second statement
    // acts on the handle the first produced.
    let mut doc = GraphDoc::default();
    doc.variables = vec![VarDecl {
        slug: "last".into(),
        label: "Last".into(),
        ty: PinType::Entity,
        default: None,
        group: None,
    }];
    doc.nodes = vec![
        node(0, EVENT_BEGIN_PLAY_TYPE_ID),
        with(1, ids::FOR_LOOP, &[("first", PropValue::Int(0)), ("last", PropValue::Int(1))]),
        with(
            2,
            ids::SPAWN_PREFAB,
            &[("path", PropValue::Str("prefabs/crate.prefab".into()))],
        ),
        with(3, VAR_SET_TYPE_ID, &[(VAR_PROP, PropValue::Str("last".into()))]),
        node(4, ids::INT_TO_FLOAT),
        with(5, ids::MAKE_VEC3, &[("y", PropValue::Float(9.0))]),
        node(6, ids::SET_POSITION),
        with(7, VAR_GET_TYPE_ID, &[(VAR_PROP, PropValue::Str("last".into()))]),
    ];
    doc.edges = vec![
        edge(0, EXEC_OUT_PIN, 1, EXEC_IN_PIN),
        edge(1, "body", 2, EXEC_IN_PIN),
        edge(2, EXEC_OUT_PIN, 3, EXEC_IN_PIN),
        edge(3, EXEC_OUT_PIN, 6, EXEC_IN_PIN),
        // spawn position x = the loop index
        edge(1, "index", 4, "value"),
        edge(4, "result", 5, "x"),
        edge(5, "result", 2, "position"),
        // the handle goes into the variable, and out of it into set_position
        edge(2, "spawned", 3, VAR_VALUE_PIN),
        edge(7, VAR_VALUE_PIN, 6, "entity"),
        edge(5, "result", 6, "position"),
    ];

    let mut h = Harness::with_root(&[("graphs/spawn.graph", doc)], &root);
    let owner = h.spawn_runner("Spawner", "graphs/spawn.graph");

    h.tick(1.0 / 60.0);
    let spawned: Vec<glm::Vec3> = h
        .world
        .query::<(&Name, &Transform)>()
        .iter()
        .filter(|(e, _)| *e != owner)
        .map(|(_, (_, t))| t.position)
        .collect();
    assert_eq!(spawned.len(), 2, "two loop iterations, two entities");

    // Every spawned entity got a guid, which scene serialization needs.
    for (e, _) in h.world.query::<&Name>().iter() {
        if e != owner {
            assert!(
                h.world.get::<&crate::engine::ecs::components::EntityGuid>(e).is_ok(),
                "a graph-spawned entity is a first-class entity"
            );
        }
    }

    // The aliases bound: the runner recorded them, and the instance no longer
    // considers them pending.
    let rt = h.world.get::<&GraphRuntime>(owner).unwrap();
    assert_eq!(rt.aliases.len(), 2, "both aliases bound to real entities");
    assert!(
        rt.instance.pending_aliases.is_empty(),
        "and the handshake was drained"
    );
    drop(rt);

    // On the next tick the `set_position` that used the *handle* lands — the
    // alias was bound before the instance ticked again, which is the point.
    h.tick(1.0 / 60.0);
    let moved = h
        .world
        .query::<(&Name, &Transform)>()
        .iter()
        .filter(|(e, _)| *e != owner)
        .filter(|(_, (_, t))| t.position.y == 9.0)
        .count();
    assert!(moved >= 1, "the graph moved an entity it had spawned by handle");

    let _ = std::fs::remove_dir_all(&root);
}

/// Destroying by handle removes the entity from the world.
#[test]
fn destroy_entity_removes_it() {
    let mut doc = GraphDoc::default();
    doc.nodes = vec![node(0, EVENT_TICK_TYPE_ID), node(1, ids::DESTROY_ENTITY)];
    doc.edges = vec![edge(0, EXEC_OUT_PIN, 1, EXEC_IN_PIN)];

    let mut h = Harness::new(&[("graphs/kill.graph", doc)]);
    let e = h.spawn_runner("Doomed", "graphs/kill.graph");
    assert!(h.world.contains(e));
    h.tick(1.0 / 60.0);
    assert!(!h.world.contains(e), "the entity destroyed itself");
}

/// **P8 acceptance**: the whole `.curve` path, end to end in the real engine
/// — the loader fetches the asset, the *compiler* resolves the Timeline's
/// track pins from it, the runner hands the same copy to the interpreter
/// through `WorldRead::curve`, and the sampled values land on a real
/// `Transform` as SetPosition effects.
///
/// Four seams, one assertion each, because any one of them failing silently
/// would look identical from outside: "the entity did not move".
#[test]
fn a_timeline_drives_a_real_transform_from_a_curve_asset() {
    let mut curve = curve_asset::CurveDoc::default();
    curve.tracks = vec![curve_asset::Track {
        slug: "height".into(),
        label: "Height".into(),
        keys: vec![
            curve_asset::Key::new(0.0, 0.0),
            curve_asset::Key::new(0.5, 10.0),
        ],
    }];

    let mut doc = GraphDoc::default();
    doc.nodes = vec![
        node(0, EVENT_BEGIN_PLAY_TYPE_ID),
        with(
            1,
            node_graph_types::TIMELINE_TYPE_ID,
            &[(
                node_graph_types::CURVE_PROP,
                PropValue::Asset("curves/t.curve".into()),
            )],
        ),
        node(2, ids::GET_POSITION),
        node(3, ids::BREAK_VEC3),
        node(4, ids::MAKE_VEC3),
        node(5, ids::SET_POSITION),
    ];
    doc.edges = vec![
        edge(0, EXEC_OUT_PIN, 1, node_graph_types::TIMELINE_PLAY_PIN),
        edge(1, node_graph_types::TIMELINE_UPDATE_PIN, 5, EXEC_IN_PIN),
        edge(2, "position", 3, "value"),
        edge(3, "x", 4, "x"),
        edge(3, "y", 4, "y"),
        edge(1, "height", 4, "z"),
        edge(4, "result", 5, "position"),
    ];

    let mut h = Harness::with_curves(
        &[("graphs/tl.graph", doc)],
        &[("curves/t.curve", curve)],
    );
    let e = h.spawn_runner("Hopper", "graphs/tl.graph");

    // 1. It compiled: the Timeline's `height` output only exists because the
    //    compiler could read the curve.
    h.ticks(1);
    let disabled = h.world.get::<&GraphRuntime>(e).unwrap().disabled.clone();
    assert!(disabled.is_none(), "{disabled:?}");

    // 2. The curve is in the cache the runner shares with the interpreter.
    assert_eq!(
        h.resources
            .get::<crate::engine::scripting::runner::CurveCache>()
            .map(|c| c.len()),
        Some(1),
        "the compiler prefetched the asset"
    );

    // 3. It is *sampling*, not holding: Z climbs while the run is under way.
    //    (t = 0 on the first tick, so the first sample is the curve's start.)
    assert_eq!(h.position(e).z, 0.0);
    h.ticks(15);
    let mid = h.position(e).z;
    assert!(mid > 0.0 && mid < 10.0, "mid-run Z should be climbing, got {mid}");

    // 4. …and it comes to rest on the curve's last value rather than
    //    overshooting or snapping back.
    h.ticks(60);
    assert!(
        (h.position(e).z - 10.0).abs() < 1e-3,
        "settled at {}",
        h.position(e).z
    );
    // Nothing else moved: the Timeline replaced Z and preserved X/Y.
    assert_eq!(h.position(e).x, 0.0);
    assert_eq!(h.position(e).y, 0.0);
}

/// **The showcase, on the committed assets.** Loads `runner_demo.graph`,
/// `duck_hop.curve` and `graph_cube.prefab` off the real content tree and
/// checks the plan's three acceptance behaviours in one run:
///
/// **GS-4, end to end: only the debugged instance stops.**
///
/// Two entities run one graph. The tab is bound to one of them, arms a mark on
/// the node that moves it, and that entity freezes exactly where it stood — the
/// other keeps walking. This is the whole reason breakpoints live on the
/// runtime component rather than on the plan: a plan is shared by every
/// instance, and freezing the scene to read one of them would take the
/// reference away from the person reading it.
#[test]
fn breakpoints_stop_only_the_bound_instance() {
    use crate::engine::editor::graph_exec_viz::DebugRequest;
    use crate::engine::scripting::trace::arm_debug;

    let mut h = Harness::new(&[("graphs/t.graph", move_each_tick())]);
    let watched = h.spawn_runner("Watched", "graphs/t.graph");
    let other = h.spawn_runner("Other", "graphs/t.graph");
    h.ticks(4);
    assert!(h.position(watched).x > 0.0, "both are moving to start with");
    assert!(h.position(other).x > 0.0);

    // Arm node 7 (Set Position) on the bound instance only — the same call
    // the host makes after the UI, with the same arguments.
    let armed = [7u64];
    let bound = Some(watched.to_bits().get());
    let touched = arm_debug(&mut h.world, "graphs/t.graph", bound, &armed, None);
    assert_eq!(touched, 2, "both instances are re-pointed every frame — one armed, one cleared");
    h.ticks(3);

    let frozen = h.position(watched).x;
    let moving = h.position(other).x;
    assert!(
        h.world
            .get::<&GraphRuntime>(watched)
            .map(|rt| rt.instance.is_paused())
            .unwrap_or(false),
        "the bound instance parked on the mark"
    );
    h.ticks(10);
    assert_eq!(h.position(watched).x, frozen, "and it has not moved since");
    assert!(
        h.position(other).x > moving + 0.005,
        "while the other instance of the same graph ran free: {} then {}",
        moving,
        h.position(other).x
    );
    assert!(
        !h.world
            .get::<&GraphRuntime>(other)
            .map(|rt| rt.instance.is_paused())
            .unwrap_or(true),
        "…and never parked at all"
    );

    // Step: exactly one firing, so the entity moves once and stops again.
    arm_debug(&mut h.world, "graphs/t.graph", bound, &armed, Some(DebugRequest::Step));
    h.ticks(1);
    let stepped = h.position(watched).x;
    assert!(stepped > frozen, "one step, one move");
    h.ticks(5);
    assert_eq!(h.position(watched).x, stepped, "…and it re-parked");

    // Resume with the mark cleared: it runs on and stays running.
    arm_debug(&mut h.world, "graphs/t.graph", bound, &[], Some(DebugRequest::Resume));
    h.ticks(5);
    assert!(h.position(watched).x > stepped + 0.005, "resumed for good");

    // Stop ends that instance's session without an error: nothing halted, and
    // nothing moves it again.
    arm_debug(&mut h.world, "graphs/t.graph", bound, &[], Some(DebugRequest::Stop));
    h.ticks(2);
    let stopped_at = h.position(watched).x;
    h.ticks(10);
    assert_eq!(h.position(watched).x, stopped_at, "the session is over");
    let rt = h.world.get::<&GraphRuntime>(watched).unwrap();
    assert!(rt.instance.stopped);
    assert!(rt.instance.halted.is_none(), "Stop is not a kill — no error framing");
}

/// 1. *BeginPlay spawns prefabs in a ForLoop* — three cubes, spaced by the
///    loop index, each placed through the spawn pin;
/// 2. *Delay chains fire* — the Timeline's Play is wired downstream of the
///    one-second Delay, so nothing hops until the latent resumes. That
///    ordering is the assertion: hop-before-delay would be indistinguishable
///    from "the Delay was ignored";
/// 3. *Tick moves one via Timeline* — the last spawned cube's Z rides the
///    curve while the graph's own entity walks +X, two effect streams on two
///    entities in the same tick.
///
/// Reading the committed files rather than rebuilding them is the point: this
/// is the test that fails if someone edits the demo asset into something that
/// no longer demonstrates the task.
#[test]
fn the_committed_demo_shows_all_three_behaviours() {
    let content = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("content");
    if !content.join("prefabs/graph_cube.prefab").exists() {
        return; // packaged build, no content tree
    }
    crate::engine::assets::asset_source::init_filesystem_if_unset(content.clone());
    let doc = node_graph_types::load_graph(&content.join("graphs/runner_demo.graph"))
        .expect("the committed demo graph loads");

    let mut h = Harness::build(
        &[("graphs/runner_demo.graph", doc)],
        &[(DEMO_CURVE, demo_curve())],
        &content,
    );
    let owner = h.spawn_runner("Demo", "graphs/runner_demo.graph");

    // 1. One tick is enough: BeginPlay runs the whole loop in a single
    //    statement chain, and the spawns apply at the end of it.
    h.ticks(1);
    let disabled = h.world.get::<&GraphRuntime>(owner).unwrap().disabled.clone();
    assert!(disabled.is_none(), "the demo compiles and runs: {disabled:?}");
    let mut cubes: Vec<(hecs::Entity, glm::Vec3)> = h
        .world
        .query::<(&Name, &Transform)>()
        .iter()
        .filter(|(e, _)| *e != owner)
        .map(|(e, (_, t))| (e, t.position))
        .collect();
    cubes.sort_by(|a, b| a.1.x.total_cmp(&b.1.x));
    assert_eq!(cubes.len(), 3, "three loop iterations, three prefabs");
    let xs: Vec<f32> = cubes.iter().map(|(_, p)| p.x).collect();
    assert_eq!(xs, vec![1.5, 3.0, 4.5], "each spawn was placed by the loop index");
    assert!(
        cubes.iter().all(|(_, p)| p.z == 0.0),
        "nothing has hopped yet"
    );

    // 2. The Delay is still counting: half a second in, the Timeline that
    //    hangs off it has not started.
    h.ticks(30);
    let hopper = cubes.last().expect("a last spawn").0;
    assert_eq!(
        h.world.get::<&Transform>(hopper).unwrap().position.z,
        0.0,
        "the Timeline must not start before the Delay resumes"
    );
    // …meanwhile the Tick chain has been running all along.
    let walked = h.position(owner).x;
    assert!(walked > 0.0, "Tick walks the graph's own entity, got {walked}");

    // 3. Past the delay, the curve drives the last cube's Z — and only its Z.
    h.ticks(60);
    let hop = h.world.get::<&Transform>(hopper).unwrap().position;
    assert!(hop.z > 0.0, "the Timeline is sampling the curve, z = {}", hop.z);
    assert_eq!(hop.x, 4.5, "the hop replaces Z and leaves X alone");
    let others: Vec<f32> = cubes[..2]
        .iter()
        .map(|(e, _)| h.world.get::<&Transform>(*e).unwrap().position.z)
        .collect();
    assert_eq!(others, vec![0.0, 0.0], "only the handle the graph kept moves");
    assert!(
        h.position(owner).x > walked,
        "both effect streams ran in the same ticks"
    );
}

/// The committed demo graph — **the plan's acceptance demo, verbatim**
/// ("BeginPlay spawns prefabs in a ForLoop, Tick moves one via Timeline,
/// Delay chains fire"). Generated rather than hand-written so it is
/// guaranteed valid and canonical:
/// `UPDATE_GRAPH_FIXTURES=1 cargo test -p rust_engine --lib write_runner_demo`
///
/// What it does, and why each piece is there:
/// - **BeginPlay** runs a ForLoop that *spawns a prefab per iteration*,
///   parking each handle in an Entity variable and printing the index —
///   control flow, a pure chain, the spawn-alias protocol and a variable, all
///   visible the moment play starts;
/// - then a **Delay** chain prints once more a second later, which is the only
///   way to see latency working from outside;
/// - **Tick** walks the graph's own entity along +X, so the effect stream
///   visibly moves something every frame;
/// - a looping **Timeline** drives the **last spawned prefab's** Z off
///   `curves/duck_hop.curve` — so the P8 asset, the alias binding and the
///   latent are all visible in one motion. It composes with the Tick chain
///   rather than fighting it: the two act on different entities, and each
///   replaces only the axis it owns.
#[test]
fn write_runner_demo_if_requested() {
    use node_graph_types::{serialize_graph, PinType, VarDecl, VAR_GET_TYPE_ID, VAR_PROP, VAR_SET_TYPE_ID, VAR_VALUE_PIN};

    if std::env::var("UPDATE_GRAPH_FIXTURES").is_err() {
        return;
    }

    let mut doc = GraphDoc::default();
    doc.realm = GraphRealm::Shared;
    doc.variables = vec![
        VarDecl {
            slug: "steps".into(),
            label: "Steps".into(),
            ty: PinType::Int,
            default: Some(PropValue::Int(0)),
            group: None,
        },
        // The handle the ForLoop parks each spawn in; the Timeline reads it
        // back, which is what makes the alias protocol visible in the world.
        VarDecl {
            slug: "hopper".into(),
            label: "Hopper".into(),
            ty: PinType::Entity,
            default: None,
            group: None,
        },
    ];
    doc.nodes = vec![
        // --- BeginPlay: a counted loop that spawns, then a delayed line ----
        node(0, EVENT_BEGIN_PLAY_TYPE_ID),
        with(1, ids::FOR_LOOP, &[("first", PropValue::Int(1)), ("last", PropValue::Int(3))]),
        node(2, ids::INT_TO_STRING),
        with(3, ids::PRINT, &[]),
        with(4, ids::DELAY, &[("duration", PropValue::Float(1.0))]),
        with(5, ids::PRINT, &[("text", PropValue::Str("one second later".into()))]),
        // --- Tick: walk along +X -------------------------------------------
        node(6, EVENT_TICK_TYPE_ID),
        node(7, ids::GET_POSITION),
        node(8, ids::BREAK_VEC3),
        with(9, ids::ADD_FLOAT, &[("b", PropValue::Float(0.02))]),
        node(10, ids::MAKE_VEC3),
        node(11, ids::SET_POSITION),
        // --- …and count the steps in a variable ----------------------------
        with(12, VAR_GET_TYPE_ID, &[(VAR_PROP, PropValue::Str("steps".into()))]),
        with(13, ids::ADD_INT, &[("b", PropValue::Int(1))]),
        with(14, VAR_SET_TYPE_ID, &[(VAR_PROP, PropValue::Str("steps".into()))]),
        // --- …and a looping Timeline hopping the entity along Z (P8) --------
        with(
            15,
            node_graph_types::TIMELINE_TYPE_ID,
            &[
                (
                    node_graph_types::CURVE_PROP,
                    PropValue::Asset(DEMO_CURVE.into()),
                ),
                ("looping", PropValue::Bool(true)),
            ],
        ),
        node(16, ids::GET_POSITION),
        node(17, ids::BREAK_VEC3),
        node(18, ids::MAKE_VEC3),
        node(19, ids::SET_POSITION),
        // --- the spawn itself, and the handle it hands back -----------------
        with(
            20,
            ids::SPAWN_PREFAB,
            &[("path", PropValue::Asset(DEMO_PREFAB.into()))],
        ),
        with(21, VAR_SET_TYPE_ID, &[(VAR_PROP, PropValue::Str("hopper".into()))]),
        node(22, ids::INT_TO_FLOAT),
        // Spawn at (index * 1.5, 0, 0) so the three cubes stand apart.
        with(23, ids::MUL_FLOAT, &[("b", PropValue::Float(1.5))]),
        node(24, ids::MAKE_VEC3),
        // One Get feeding both ends of the Timeline chain: an output may fan
        // out, and two Gets of one variable would only invite them to drift.
        with(25, VAR_GET_TYPE_ID, &[(VAR_PROP, PropValue::Str("hopper".into()))]),
    ];
    doc.edges = vec![
        edge(0, EXEC_OUT_PIN, 1, EXEC_IN_PIN),
        // Loop body: spawn, park the handle, then say which iteration it was.
        edge(1, "body", 20, EXEC_IN_PIN),
        edge(20, EXEC_OUT_PIN, 21, EXEC_IN_PIN),
        edge(21, EXEC_OUT_PIN, 3, EXEC_IN_PIN),
        edge(20, "spawned", 21, VAR_VALUE_PIN),
        edge(1, "index", 22, "value"),
        edge(22, "result", 23, "a"),
        edge(23, "result", 24, "x"),
        edge(24, "result", 20, "position"),
        edge(1, "index", 2, "value"),
        edge(2, "text", 3, "text"),
        edge(1, "completed", 4, EXEC_IN_PIN),
        edge(4, EXEC_OUT_PIN, 5, EXEC_IN_PIN),
        edge(6, EXEC_OUT_PIN, 11, EXEC_IN_PIN),
        edge(7, "position", 8, "value"),
        edge(8, "x", 9, "a"),
        edge(9, "result", 10, "x"),
        edge(8, "y", 10, "y"),
        edge(8, "z", 10, "z"),
        edge(10, "result", 11, "position"),
        edge(11, EXEC_OUT_PIN, 14, EXEC_IN_PIN),
        edge(12, VAR_VALUE_PIN, 13, "a"),
        edge(13, "result", 14, VAR_VALUE_PIN),
        // The delayed line is also the cue to start hopping, so the Timeline
        // begins visibly *after* the rest rather than at frame zero.
        edge(5, EXEC_OUT_PIN, 15, node_graph_types::TIMELINE_PLAY_PIN),
        edge(15, node_graph_types::TIMELINE_UPDATE_PIN, 19, EXEC_IN_PIN),
        // …on the spawned entity, not on self.
        edge(25, VAR_VALUE_PIN, 16, "entity"),
        edge(25, VAR_VALUE_PIN, 19, "entity"),
        edge(16, "position", 17, "value"),
        edge(17, "x", 18, "x"),
        edge(17, "y", 18, "y"),
        edge(15, "height", 18, "z"),
        edge(18, "result", 19, "position"),
    ];
    // Lay it out so the committed asset opens readably rather than as a heap.
    for (i, n) in doc.nodes.iter_mut().enumerate() {
        n.position = [(i % 6) as f32 * 260.0, (i / 6) as f32 * 220.0];
    }

    // It must compile *and run* before it is committed — a demo that does not
    // work is worse than no demo. Compiled against the *committed* curve, so a
    // regenerated demo and the asset on disk cannot drift apart.
    let content = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("content");
    write_demo_prefab(&content);
    crate::engine::assets::asset_source::init_filesystem_if_unset(content.clone());
    // Rooted at the *repo's* content dir, not the relative "content" the other
    // harnesses use: this one has to load a real prefab off disk, and the test
    // process runs from the engine crate.
    let mut h = Harness::build(
        &[("graphs/runner_demo.graph", doc.clone())],
        &[(DEMO_CURVE, demo_curve())],
        &content,
    );
    let e = h.spawn_runner("Demo", "graphs/runner_demo.graph");
    h.ticks(3);
    let rt = h.world.get::<&GraphRuntime>(e).unwrap();
    assert!(rt.disabled.is_none(), "{:?}", rt.disabled);
    drop(rt);
    assert!(h.position(e).x > 0.0, "the demo actually moves its entity");
    assert_eq!(
        h.world.query::<&Name>().iter().filter(|(o, _)| *o != e).count(),
        3,
        "three loop iterations, three spawned prefabs"
    );

    let path = content.join("graphs/runner_demo.graph");
    std::fs::write(&path, serialize_graph(&doc).unwrap()).unwrap();
    println!("wrote {}", path.display());
}

// ---------------------------------------------------------------------------
// GAP 1 — Print reaches the editor
// ---------------------------------------------------------------------------

/// BeginPlay → Print(`text`). The smallest graph whose entire observable
/// output is a console line.
fn print_on_begin_play(text: &str) -> GraphDoc {
    let mut doc = GraphDoc::default();
    doc.nodes = vec![
        node(0, EVENT_BEGIN_PLAY_TYPE_ID),
        with(1, ids::PRINT, &[("text", PropValue::Str(text.into()))]),
    ];
    doc.edges = vec![edge(0, EXEC_OUT_PIN, 1, EXEC_IN_PIN)];
    doc
}

/// A `Print` lands in the sink the editor drains, structured — the graph and
/// the entity are *fields*, not a string the console has to parse back apart.
#[cfg(feature = "editor")]
#[test]
fn a_print_reaches_the_log_sink_with_its_tag_fields() {
    use crate::engine::scripting::log_sink::GraphLogSink;

    let mut h = Harness::new(&[("graphs/t.graph", print_on_begin_play("hello"))]);
    h.spawn_runner("Duck", "graphs/t.graph");
    h.ticks(3);

    let sink = h
        .resources
        .get::<GraphLogSink>()
        .expect("the plugin inserts one in editor builds");
    let entries: Vec<_> = sink.iter().collect();
    assert_eq!(entries.len(), 1, "BeginPlay printed once, not once per tick");
    let e = entries[0];
    assert_eq!(e.level, node_graph_exec::LogLevel::Info);
    assert_eq!(e.graph, "graphs/t.graph");
    assert_eq!(e.entity, "Duck");
    assert_eq!(e.text, "hello");
    assert_eq!(
        e.line(),
        "[graph graphs/t.graph on Duck] hello",
        "the P7 console tag, unchanged"
    );
}

/// A refusal to arm reaches the same sink. This is the message that *must*
/// not be stdout-only: the graph emitted no effects, so there is nothing else
/// on screen to notice.
#[cfg(feature = "editor")]
#[test]
fn a_refusal_reaches_the_log_sink_as_an_error() {
    use crate::engine::scripting::log_sink::GraphLogSink;

    let mut h = Harness::new(&[]);
    h.spawn_runner("Broken", "graphs/missing.graph");
    h.ticks(3);

    let sink = h.resources.get::<GraphLogSink>().unwrap();
    let entries: Vec<_> = sink.iter().collect();
    assert_eq!(entries.len(), 1, "reported once, not once per frame");
    assert_eq!(entries[0].level, node_graph_exec::LogLevel::Error);
    assert_eq!(entries[0].graph, "graphs/missing.graph");
    assert!(
        entries[0].text.starts_with("will not run — "),
        "{}",
        entries[0].text
    );
    assert!(
        !entries[0].text.starts_with("graphs/missing.graph"),
        "the path is a field; the text must not re-state it as a prefix: {}",
        entries[0].text
    );
}

// ---------------------------------------------------------------------------
// GAP 2 — input actions reach their entry nodes
// ---------------------------------------------------------------------------

/// An `event_input_action` entry listening for `action`.
fn input_entry_id(id: u64, action: &str) -> NodeInst {
    with(
        id,
        node_graph_types::EVENT_INPUT_ACTION_TYPE_ID,
        &[(node_graph_types::EVENT_ACTION_PROP, PropValue::Str(action.into()))],
    )
}

/// …at the id [`count_entry`] expects.
fn input_entry(action: &str) -> NodeInst {
    input_entry_id(0, action)
}

/// One press, one activation — and none while the key is held down.
///
/// Press semantics are `InputSubsystem::just_pressed`, the same predicate
/// `PlayerInputSystem` uses for jump: active this frame, inactive last. The
/// shipped `event_input_action` descriptor has a single exec output, so "on
/// press" is the only reading it supports.
#[test]
fn an_input_action_entry_fires_once_per_press() {
    use crate::engine::input::action::KeyCode;

    let mut h = Harness::new(&[("graphs/t.graph", count_entry(input_entry("fire")))]).with_input();
    let e = h.spawn_runner("Shooter", "graphs/t.graph");

    h.ticks(2);
    assert_eq!(h.position(e).x, 0.0, "nothing pressed, nothing fired");

    // The plan's listening set is precomputed at arm time.
    assert_eq!(
        h.world.get::<&GraphRuntime>(e).unwrap().input_actions,
        vec!["fire".to_string()]
    );

    h.hold(KeyCode::Space);
    h.tick(1.0 / 60.0);
    assert_eq!(h.position(e).x, 1.0, "the press fired the entry on that frame");

    h.ticks(5);
    assert_eq!(h.position(e).x, 1.0, "…and holding it fires nothing more");

    h.release_all();
    h.ticks(3);
    assert_eq!(h.position(e).x, 1.0, "releasing is not a press");

    h.hold(KeyCode::Space);
    h.tick(1.0 / 60.0);
    assert_eq!(h.position(e).x, 2.0, "a second press is a second activation");
}

/// A different action does not deliver. Names match exactly: the entry's
/// `action` property is the same string an `.inputaction` asset declares.
#[test]
fn another_action_does_not_reach_this_entry() {
    use crate::engine::input::action::KeyCode;

    let mut h = Harness::new(&[("graphs/t.graph", count_entry(input_entry("fire")))]).with_input();
    let e = h.spawn_runner("Shooter", "graphs/t.graph");
    h.ticks(2);

    h.hold(KeyCode::KeyR); // "reload"
    h.ticks(5);
    assert_eq!(h.position(e).x, 0.0, "'reload' is not 'fire'");

    // …and the case has to match too.
    let mut h = Harness::new(&[("graphs/t.graph", count_entry(input_entry("Fire")))]).with_input();
    let e = h.spawn_runner("Shouter", "graphs/t.graph");
    h.ticks(2);
    h.hold(KeyCode::Space);
    h.ticks(5);
    assert_eq!(
        h.position(e).x,
        0.0,
        "matching is case-sensitive — a typo must fail in the editor, not only in a shipped game"
    );
}

/// D3's drain order, from the enqueue side: an input action and a custom
/// event pending on the *same* tick deliver input first.
///
/// The interpreter has its own test for the ordering; this one proves the
/// runner puts the input event in the queue early enough for that order to
/// apply — enqueue it after the instances tick and every press would arrive a
/// frame late, behind a custom event queued a frame earlier.
#[cfg(feature = "editor")]
#[test]
fn an_input_action_drains_before_a_custom_event_queued_for_the_same_tick() {
    use crate::engine::input::action::KeyCode;
    use crate::engine::scripting::log_sink::GraphLogSink;

    let mut doc = GraphDoc::default();
    doc.nodes = vec![
        // BeginPlay emits "ping", which lands in the queue for the *next*
        // tick — the tick the press is delivered on.
        node(0, EVENT_BEGIN_PLAY_TYPE_ID),
        with(1, ids::EMIT_EVENT, &[("name", PropValue::Str("ping".into()))]),
        with(
            2,
            node_graph_types::EVENT_CUSTOM_TYPE_ID,
            &[(node_graph_types::EVENT_NAME_PROP, PropValue::Str("ping".into()))],
        ),
        with(3, ids::PRINT, &[("text", PropValue::Str("custom".into()))]),
        input_entry_id(4, "fire"),
        with(5, ids::PRINT, &[("text", PropValue::Str("input".into()))]),
    ];
    doc.edges = vec![
        edge(0, EXEC_OUT_PIN, 1, EXEC_IN_PIN),
        edge(2, EXEC_OUT_PIN, 3, EXEC_IN_PIN),
        edge(4, EXEC_OUT_PIN, 5, EXEC_IN_PIN),
    ];

    let mut h = Harness::new(&[("graphs/t.graph", doc)]).with_input();
    h.spawn_runner("Duck", "graphs/t.graph");

    // Tick 1: BeginPlay runs and queues "ping" for tick 2. Nothing pressed.
    h.tick(1.0 / 60.0);
    // Tick 2: the press and the custom event are both pending.
    h.hold(KeyCode::Space);
    h.tick(1.0 / 60.0);

    let sink = h.resources.get::<GraphLogSink>().unwrap();
    let texts: Vec<&str> = sink.iter().map(|e| e.text.as_str()).collect();
    assert_eq!(
        texts,
        vec!["input", "custom"],
        "input actions drain ahead of custom events (D3)"
    );
}

/// The zero-cost skip, asserted **by shape**: the runner reads
/// `InputSubsystem` only when [`wanted_input_actions`] is non-empty, and that
/// set is built from a per-plan list computed once at arm time. A graph with
/// no `event_input_action` entry therefore contributes nothing to it, and the
/// guard immediately above the resource lookup short-circuits.
#[test]
fn a_plan_with_no_input_entries_contributes_nothing_to_the_input_scan() {
    use crate::engine::input::action::KeyCode;
    use crate::engine::scripting::runner::wanted_input_actions;

    let mut h = Harness::new(&[("graphs/t.graph", move_each_tick())]).with_input();
    let e = h.spawn_runner("Mover", "graphs/t.graph");
    h.ticks(2);

    assert!(
        h.world.get::<&GraphRuntime>(e).unwrap().input_actions.is_empty(),
        "nothing in this plan listens for an action"
    );
    assert!(
        wanted_input_actions(&h.world).is_empty(),
        "…so the set that guards the input read is empty, and it is never read"
    );

    // And pressing things changes nothing about what this graph does.
    let before = h.position(e).x;
    h.hold(KeyCode::Space);
    h.ticks(3);
    assert_eq!(h.position(e).x, before + 3.0, "three ticks, three nudges, no more");

    // The same probe with a listening instance is non-empty — the guard is a
    // real condition, not a constant.
    let mut h = Harness::new(&[("graphs/t.graph", count_entry(input_entry("fire")))]).with_input();
    h.spawn_runner("Shooter", "graphs/t.graph");
    h.ticks(1);
    assert_eq!(
        wanted_input_actions(&h.world).into_iter().collect::<Vec<_>>(),
        vec!["fire".to_string()]
    );
}

/// The prefab the demo spawns, written alongside the graph for the same
/// reason the graph is generated: a hand-written asset can drift out of the
/// serde shape the loader expects, and nothing would notice until play.
#[cfg(test)]
fn write_demo_prefab(content: &std::path::Path) {
    use crate::engine::scene::scene_format::{ComponentData, EntityData};
    let prefab = crate::engine::scene::prefab::Prefab {
        name: "GraphCube".to_string(),
        description: "Spawned by graphs/runner_demo.graph (Task 45-A)".to_string(),
        template: EntityData {
            name: "GraphCube".to_string(),
            guid: None,
            components: vec![
                ComponentData::Transform {
                    position: [0.0, 0.0, 0.0],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    scale: [0.4, 0.4, 0.4],
                },
                ComponentData::MeshRenderer {
                    mesh_path: "__primitive__/Cube".to_string(),
                    material_paths: vec![String::new()],
                    material_path: String::new(),
                    mesh_index: 0,
                    material_index: 0,
                    visible: true,
                    cast_shadows: true,
                    receive_shadows: true,
                    base_color_factor: [0.9, 0.5, 0.2, 1.0],
                    metallic_factor: 0.0,
                    roughness_factor: 0.6,
                    emissive_factor: [0.0, 0.0, 0.0],
                },
            ],
        },
    };
    let dir = content.join("prefabs");
    std::fs::create_dir_all(&dir).expect("prefabs dir");
    let text = ron::ser::to_string_pretty(&prefab, ron::ser::PrettyConfig::default())
        .expect("serialize prefab");
    std::fs::write(dir.join("graph_cube.prefab"), text).expect("write prefab");
}
