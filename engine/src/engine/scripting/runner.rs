//! The runtime that ticks graphs (Task 45-A D4, as amended by the audit
//! addendum).
//!
//! Everything portable lives in `node_graph_exec`; this is the engine half of
//! the seam and nothing more — it supplies time, reads the world, applies the
//! effect stream, and owns the compiled-plan cache.
//!
//! ## Lifecycle: lazy, not hooked (addendum #3)
//!
//! There is no play-enter hook. Any entity carrying an enabled [`GraphRunner`]
//! *without* a [`GraphRuntime`] gets one created — and BeginPlay armed — on
//! its first playing tick. That one rule covers entities present at play
//! enter, entities spawned during play, and entities spawned by another
//! graph, uniformly. Stopping play restores the scene snapshot, which clears
//! the world and respawns from serialized data; `GraphRuntime` is not
//! serialized, so it simply ceases to exist and re-entry re-arms naturally.
//! Nothing has to remember to reset anything.
//!
//! ## Effect application (D4, with one correction)
//!
//! D4 says structural effects go through `world.commands()`. They cannot: a
//! scheduled `System` receives `(&mut hecs::World, &mut Resources)`, and the
//! command buffer belongs to the scheduler, which never hands it to a system.
//! Since this system holds `&mut hecs::World` exclusively for its whole run,
//! it applies both kinds directly and declares itself `exclusive()` — honest
//! about what it does rather than pretending to defer. Ordering within a tick
//! is still deterministic: effects apply in emission order, per instance, in
//! stable entity order.

use std::collections::BTreeMap;
use std::sync::Arc;

use nalgebra_glm as glm;
use node_graph_exec::{
    compile, nodes as std_impls, Effect, EntityRef, ExecError, GraphInstance, NodeImpls, Plan,
    TickInput, TransformSnapshot, WorldRead,
};
/// The traced entry point is the editor's; a shipped game takes the plain one,
/// which instantiates the interpreter against `NoTrace` (see [`super::trace`]).
#[cfg(feature = "editor")]
use node_graph_exec::{tick_traced, DEFAULT_BUDGET};
#[cfg(not(feature = "editor"))]
use node_graph_exec::tick;
use node_graph_types::{GraphDoc, GraphRealm, NodeRegistry};

use crate::engine::ecs::components::{EntityGuid, Name, Transform, TransformDirty};
use crate::engine::ecs::resources::{Resources, Time};
use crate::engine::ecs::schedule::System;

use super::GraphRunner;

/// The runtime half of a running graph. **Never serialized**: it holds a
/// compiled plan and live thread state, and its absence is precisely what
/// tells the runner to arm BeginPlay.
pub struct GraphRuntime {
    /// The asset this was compiled from, so a hot-reload can tell whether it
    /// is stale.
    pub graph: String,
    pub plan: Arc<Plan>,
    pub instance: GraphInstance,
    /// Instance-local spawn alias -> the entity the runner actually created.
    /// The portable core never sees the right-hand side (D1).
    pub aliases: BTreeMap<u32, hecs::Entity>,
    /// The cache generation this was compiled at. A mismatch means a graph
    /// asset changed under us and the instance must restart.
    pub generation: u64,
    /// Set when this instance refused to run — a realm violation, a compile
    /// error, or a budget kill. Reported once, then remembered so the console
    /// is not spammed every frame.
    pub disabled: Option<String>,
    /// What this instance has been doing, for the graph editor's execution
    /// visualization (P7). **Editor builds only** — in a shipped game the
    /// field does not exist and the interpreter is instantiated against
    /// `NoTrace`, so there is no recording path to strip. It rides on the
    /// runtime component because a trace describes one instance and should be
    /// born and die with it.
    #[cfg(feature = "editor")]
    pub trace: super::trace::GraphTrace,
}

/// Compiled plans, keyed by content-relative asset path.
///
/// A `Resource`, because the cache outlives any one instance and two entities
/// running the same graph must share the compilation. `Arc` because an
/// instance holds its plan across the frame in which the cache may be
/// invalidated.
#[derive(Default)]
pub struct GraphPlanCache {
    plans: BTreeMap<String, Result<Arc<Plan>, String>>,
    /// Bumped on every invalidation. A runtime remembers the generation it
    /// compiled at; a mismatch means "restart me".
    generation: u64,
}

impl GraphPlanCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Drop one asset's compilation (hot-reload). Also bumps the generation,
    /// so live instances of *any* graph re-check — a subgraph edit changes
    /// the plan of every host that inlined it, and the cache does not track
    /// that reference tree.
    pub fn invalidate(&mut self, content_rel: &str) {
        let key = super::normalize_graph_path(content_rel);
        self.plans.remove(&key);
        self.generation = self.generation.wrapping_add(1);
    }

    pub fn invalidate_all(&mut self) {
        self.plans.clear();
        self.generation = self.generation.wrapping_add(1);
    }

    pub fn len(&self) -> usize {
        self.plans.len()
    }

    pub fn is_empty(&self) -> bool {
        self.plans.is_empty()
    }

    /// A cached compilation, if there is one.
    pub fn peek(&self, content_rel: &str) -> Option<Result<Arc<Plan>, String>> {
        self.plans
            .get(&super::normalize_graph_path(content_rel))
            .cloned()
    }

    /// Record a compilation. **Failures are cached too**: a broken graph must
    /// not be recompiled sixty times a second just to fail again the same way.
    pub fn store(&mut self, content_rel: &str, result: Result<Arc<Plan>, String>) {
        self.plans
            .insert(super::normalize_graph_path(content_rel), result);
    }

    /// Compile on demand. Split into `peek` + `store` around the compile so
    /// the registry and the cache are never borrowed from `Resources` at the
    /// same time — the alternative was an `unsafe` aliasing trick for no gain.
    pub fn get_or_compile(
        &mut self,
        content_rel: &str,
        registry: &NodeRegistry,
        loader: &dyn GraphLoader,
    ) -> Result<Arc<Plan>, String> {
        if let Some(hit) = self.peek(content_rel) {
            return hit;
        }
        let result = compile_asset(content_rel, registry, loader);
        self.store(content_rel, result.clone());
        result
    }
}

/// How the runner gets documents. An indirection because the engine loads
/// from disk (or a pak) while tests hand over a map — and because the graph
/// editor will later want open-but-unsaved documents to win.
pub trait GraphLoader {
    fn load(&self, content_rel: &str) -> Option<GraphDoc>;
}

/// Loads through the engine's asset source, so paks work.
pub struct AssetGraphLoader {
    pub content_root: std::path::PathBuf,
}

impl GraphLoader for AssetGraphLoader {
    fn load(&self, content_rel: &str) -> Option<GraphDoc> {
        let path = self.content_root.join(content_rel);
        node_graph_types::load_graph(&path).ok()
    }
}

/// Load a graph and every subgraph it references, then compile.
fn compile_asset(
    content_rel: &str,
    registry: &NodeRegistry,
    loader: &dyn GraphLoader,
) -> Result<Arc<Plan>, String> {
    let root = loader
        .load(content_rel)
        .ok_or_else(|| format!("graph '{content_rel}' could not be loaded"))?;

    // Materialize the reference tree. Depth-capped: a cycle is a validation
    // error the compiler reports properly, but it must not be reached by
    // spinning here first.
    let mut docs: BTreeMap<String, GraphDoc> = BTreeMap::new();
    let mut frontier: Vec<String> = root.subgraph_refs().iter().map(|s| s.to_string()).collect();
    let mut guard = 0usize;
    while let Some(next) = frontier.pop() {
        guard += 1;
        if guard > 512 {
            break;
        }
        if docs.contains_key(&next) {
            continue;
        }
        let Some(doc) = loader.load(&next) else { continue };
        frontier.extend(doc.subgraph_refs().iter().map(|s| s.to_string()));
        docs.insert(next, doc);
    }

    compile(&root, content_rel, registry, &docs)
        .map(Arc::new)
        .map_err(|e| e.to_string())
}

/// Does this build admit a graph of this realm? The client refuses anything
/// it is not the authority for (D4's realm gate).
fn client_admits(realm: GraphRealm) -> bool {
    matches!(realm, GraphRealm::Shared | GraphRealm::Client)
}

// ---------------------------------------------------------------------------
// World read
// ---------------------------------------------------------------------------

/// The engine's side of the read seam. Z-up throughout: the graph sees game
/// world values, never render space — the conversion to Y-up happens in the
/// render adapter and nowhere near here.
struct LiveWorld<'a> {
    world: &'a hecs::World,
    self_entity: hecs::Entity,
    aliases: &'a BTreeMap<u32, hecs::Entity>,
}

impl LiveWorld<'_> {
    fn resolve(&self, e: EntityRef) -> Option<hecs::Entity> {
        match e {
            EntityRef::SelfEntity => Some(self.self_entity),
            // An alias the runner has not bound yet reads as absent rather
            // than as some other entity.
            EntityRef::Spawned(a) => self.aliases.get(&a).copied(),
        }
    }
}

impl WorldRead for LiveWorld<'_> {
    fn transform(&self, entity: EntityRef) -> Option<TransformSnapshot> {
        let e = self.resolve(entity)?;
        let t = self.world.get::<&Transform>(e).ok()?;
        Some(TransformSnapshot {
            position: [t.position.x, t.position.y, t.position.z],
            rotation: [
                t.rotation.coords.x,
                t.rotation.coords.y,
                t.rotation.coords.z,
                t.rotation.coords.w,
            ],
            scale: [t.scale.x, t.scale.y, t.scale.z],
        })
    }

    fn exists(&self, entity: EntityRef) -> bool {
        self.resolve(entity)
            .map(|e| self.world.contains(e))
            .unwrap_or(false)
    }
}

// ---------------------------------------------------------------------------
// The system
// ---------------------------------------------------------------------------

/// Ticks every enabled `GraphRunner` and applies what it emits.
pub struct GraphScriptRunnerSystem {
    impls: NodeImpls,
    /// Where `.graph` assets come from. Boxed so tests can hand over a map.
    loader: Box<dyn GraphLoader + Send + Sync>,
    /// Prefab paths are resolved against this, like every other asset.
    content_root: std::path::PathBuf,
}

impl GraphScriptRunnerSystem {
    pub fn new(
        loader: Box<dyn GraphLoader + Send + Sync>,
        content_root: impl Into<std::path::PathBuf>,
    ) -> Self {
        Self {
            impls: std_impls::std_impls(),
            loader,
            content_root: content_root.into(),
        }
    }
}

/// A spawn the runner still has to perform, collected while the world is
/// borrowed and applied after.
struct PendingSpawn {
    owner: hecs::Entity,
    alias: u32,
    path: String,
    transform: TransformSnapshot,
}

impl System for GraphScriptRunnerSystem {
    fn run(&mut self, world: &mut hecs::World, resources: &mut Resources) {
        crate::profile_scope!("graph_script_runner");

        // Frame dt, matching `PhysicsStepSystem` (`Time::delta`). Graphs are
        // gameplay logic and should move with the same clock physics does;
        // `Time::fixed_delta` exists but nothing steps on it today, and
        // inventing a second cadence here would be the wrong place to start.
        let (dt, now) = match resources.get::<Time>() {
            Some(t) => (t.delta, t.total),
            None => return,
        };
        let generation = resources
            .get::<GraphPlanCache>()
            .map(|c| c.generation())
            .unwrap_or(0);

        // 1. Arm anything that needs arming. Collected first: creating the
        //    runtime component is a structural change.
        let needs_runtime: Vec<(hecs::Entity, String)> = world
            .query::<&GraphRunner>()
            .iter()
            .filter(|(e, r)| r.is_runnable() && world.get::<&GraphRuntime>(*e).is_err())
            .map(|(e, r)| (e, r.graph.clone()))
            .collect();

        // …and anything whose plan went stale under it (hot reload).
        let stale: Vec<hecs::Entity> = world
            .query::<&GraphRuntime>()
            .iter()
            .filter(|(_, rt)| rt.generation != generation)
            .map(|(e, _)| e)
            .collect();
        for e in stale {
            // Edit-during-play is a stated non-goal (D9): the honest response
            // is to restart the instance, which re-fires BeginPlay. Dropping
            // the component is enough — the arming pass above picks it up on
            // the next tick.
            let _ = world.remove_one::<GraphRuntime>(e);
        }

        for (entity, graph) in needs_runtime {
            let runtime = self.arm(&graph, generation, resources);
            let _ = world.insert_one(entity, runtime);
        }

        // 2. Tick each instance and collect what it emitted. Entity order is
        //    stable (hecs iterates its archetypes deterministically), which is
        //    the iteration-order half of the determinism contract.
        let entities: Vec<hecs::Entity> = world
            .query::<&GraphRuntime>()
            .iter()
            .map(|(e, _)| e)
            .collect();

        let mut effects: Vec<(hecs::Entity, Vec<Effect>)> = Vec::new();
        for entity in entities {
            let Ok(mut guard) = world.get::<&mut GraphRuntime>(entity) else {
                continue;
            };
            // One `&mut` up front so the instance and (in editor builds) its
            // trace are two disjoint field borrows rather than two overlapping
            // guard derefs.
            let rt = &mut *guard;
            if rt.disabled.is_some() {
                continue;
            }
            let plan = rt.plan.clone();
            let mut out: Vec<Effect> = Vec::new();
            // The alias map moves out for the duration of the tick and back
            // afterwards: the world view reads it while the instance is
            // mutated, and splitting the borrow beats cloning a map every
            // frame for every instance.
            let aliases = std::mem::take(&mut rt.aliases);
            {
                let view = LiveWorld {
                    world,
                    self_entity: entity,
                    aliases: &aliases,
                };
                let input = TickInput { dt, time: now };
                #[cfg(feature = "editor")]
                {
                    // Stamp the tick before it runs, so every hit recorded
                    // during it carries this frame's time.
                    rt.trace.begin_tick(now);
                    tick_traced(
                        &plan,
                        &mut rt.instance,
                        &self.impls,
                        input,
                        &view,
                        &mut out,
                        DEFAULT_BUDGET,
                        &mut rt.trace,
                    );
                }
                #[cfg(not(feature = "editor"))]
                tick(&plan, &mut rt.instance, &self.impls, input, &view, &mut out);
            }
            rt.aliases = aliases;
            if let Some(err) = rt.instance.halted.clone() {
                report_once(rt, entity, world, &err);
            }
            if !out.is_empty() {
                effects.push((entity, out));
            }
        }

        // 3. Apply. Non-structural first (they only touch components that
        //    already exist), then structural, so a spawn cannot be moved by an
        //    effect emitted before it existed.
        let mut spawns: Vec<PendingSpawn> = Vec::new();
        let mut despawns: Vec<hecs::Entity> = Vec::new();
        for (owner, list) in &effects {
            for effect in list {
                self.apply(world, *owner, effect, &mut spawns, &mut despawns);
            }
        }
        self.apply_structural(world, spawns, despawns);
    }

    fn name(&self) -> &str {
        "GraphScriptRunnerSystem"
    }
}

impl GraphScriptRunnerSystem {
    /// Compile (or reuse) the plan and build a fresh instance with BeginPlay
    /// armed. Every refusal path lands in `disabled` rather than panicking or
    /// silently doing nothing.
    fn arm(&self, graph: &str, generation: u64, resources: &mut Resources) -> GraphRuntime {
        // Peek, compile, store — three short borrows rather than two
        // overlapping ones. `Resources` hands out one borrow at a time, and
        // the registry is only read while the cache is untouched.
        let cached = resources
            .get::<GraphPlanCache>()
            .and_then(|c| c.peek(graph));
        let plan = match cached {
            Some(hit) => hit,
            None => {
                let compiled = match resources.get::<std::sync::Arc<NodeRegistry>>() {
                    Some(reg) => compile_asset(graph, reg, &*self.loader),
                    None => Err(
                        "no node registry in Resources — the app shares one after                          building its plugins"
                            .to_string(),
                    ),
                };
                if let Some(cache) = resources.get_mut::<GraphPlanCache>() {
                    cache.store(graph, compiled.clone());
                }
                compiled
            }
        };

        match plan {
            Err(e) => {
                let empty = Plan::default();
                GraphRuntime {
                    graph: graph.to_string(),
                    instance: GraphInstance::new(&empty, EntityRef::SelfEntity, 0),
                    plan: Arc::new(empty),
                    aliases: BTreeMap::new(),
                    generation,
                    disabled: Some(format!("{graph}: {e}")),
                    #[cfg(feature = "editor")]
                    trace: Default::default(),
                }
            }
            Ok(plan) => {
                let realm = plan.realm;
                let disabled = (!client_admits(realm)).then(|| {
                    format!("{graph}: a {realm:?}-realm graph will not run on a client")
                });
                // Seeded from the asset path, so two entities running the same
                // graph do not share a random stream but a replay does.
                let seed = fnv(graph);
                let instance = GraphInstance::new(&plan, EntityRef::SelfEntity, seed);
                GraphRuntime {
                    graph: graph.to_string(),
                    plan,
                    instance,
                    aliases: BTreeMap::new(),
                    generation,
                    disabled,
                    #[cfg(feature = "editor")]
                    trace: Default::default(),
                }
            }
        }
    }

    fn apply(
        &self,
        world: &mut hecs::World,
        owner: hecs::Entity,
        effect: &Effect,
        spawns: &mut Vec<PendingSpawn>,
        despawns: &mut Vec<hecs::Entity>,
    ) {
        let resolve = |world: &hecs::World, e: EntityRef| -> Option<hecs::Entity> {
            match e {
                EntityRef::SelfEntity => Some(owner),
                EntityRef::Spawned(a) => world
                    .get::<&GraphRuntime>(owner)
                    .ok()
                    .and_then(|rt| rt.aliases.get(&a).copied()),
            }
        };
        match effect {
            Effect::Log { level, text } => {
                let who = world
                    .get::<&Name>(owner)
                    .map(|n| n.0.clone())
                    .unwrap_or_else(|_| format!("{owner:?}"));
                let graph = world
                    .get::<&GraphRunner>(owner)
                    .map(|r| r.graph.clone())
                    .unwrap_or_default();
                // Tagged with the graph, as D6 asks: a print with no idea
                // which asset produced it is a print you cannot act on.
                println!("[graph {graph} on {who}] {level:?}: {text}");
            }
            Effect::SetPosition { entity, position } => {
                if let Some(e) = resolve(world, *entity) {
                    if let Ok(mut t) = world.get::<&mut Transform>(e) {
                        t.position = glm::vec3(position[0], position[1], position[2]);
                    }
                    let _ = world.insert_one(e, TransformDirty);
                }
            }
            Effect::SetRotation { entity, rotation } => {
                if let Some(e) = resolve(world, *entity) {
                    if let Ok(mut t) = world.get::<&mut Transform>(e) {
                        t.rotation = glm::quat(rotation[0], rotation[1], rotation[2], rotation[3]);
                    }
                    let _ = world.insert_one(e, TransformDirty);
                }
            }
            Effect::SetScale { entity, scale } => {
                if let Some(e) = resolve(world, *entity) {
                    if let Ok(mut t) = world.get::<&mut Transform>(e) {
                        t.scale = glm::vec3(scale[0], scale[1], scale[2]);
                    }
                    let _ = world.insert_one(e, TransformDirty);
                }
            }
            Effect::SpawnPrefab { path, alias, transform } => spawns.push(PendingSpawn {
                owner,
                alias: *alias,
                path: path.clone(),
                transform: *transform,
            }),
            Effect::DestroyEntity { entity } => {
                if let Some(e) = resolve(world, *entity) {
                    despawns.push(e);
                }
            }
            // The core already queued this on the emitting instance
            // (same-entity scope, resolved question 3). Delivering it again
            // here would double-fire it.
            Effect::EmitEvent { .. } => {}
        }
    }

    /// Structural changes, after every non-structural one has landed.
    fn apply_structural(
        &self,
        world: &mut hecs::World,
        spawns: Vec<PendingSpawn>,
        despawns: Vec<hecs::Entity>,
    ) {
        for s in spawns {
            let full = self.content_root.join(&s.path);
            let Ok(prefab) = crate::engine::scene::prefab::Prefab::load(&full.to_string_lossy())
            else {
                println!("[graph] spawn_prefab: '{}' could not be loaded", s.path);
                continue;
            };
            let entity = prefab.instantiate(world);
            // Prefab instantiation may not carry a guid; every entity needs
            // one for scene serialization and hierarchy bookkeeping.
            if world.get::<&EntityGuid>(entity).is_err() {
                let _ = world.insert_one(entity, EntityGuid::new());
            }
            let position = glm::vec3(
                s.transform.position[0],
                s.transform.position[1],
                s.transform.position[2],
            );
            // The spawn position wins over whatever the prefab declared —
            // that is what the pin is for. A prefab with no Transform gets
            // one, since an entity a graph placed has a place.
            let placed = match world.get::<&mut Transform>(entity) {
                Ok(mut t) => {
                    t.position = position;
                    true
                }
                Err(_) => false,
            };
            if !placed {
                let _ = world.insert_one(entity, Transform::new(position));
            }
            // Bind the alias **before the owner's next tick**, which is what
            // lets a graph spawn something and act on it next frame.
            if let Ok(mut rt) = world.get::<&mut GraphRuntime>(s.owner) {
                rt.aliases.insert(s.alias, entity);
                // Drain the handshake: what remains is what is still unbound.
                rt.instance.pending_aliases.retain(|a| *a != s.alias);
            }
        }
        for e in despawns {
            crate::engine::ecs::hierarchy::despawn_recursive(world, e);
        }
    }
}

/// Report a halted instance once, then remember that we did.
fn report_once(
    rt: &mut GraphRuntime,
    entity: hecs::Entity,
    world: &hecs::World,
    err: &ExecError,
) {
    if rt.disabled.is_some() {
        return;
    }
    let who = world
        .get::<&Name>(entity)
        .map(|n| n.0.clone())
        .unwrap_or_else(|_| format!("{entity:?}"));
    let msg = format!("{}: {err}", rt.graph);
    println!("[graph] {who} stopped — {msg}");
    rt.disabled = Some(msg);
}

/// FNV-1a, so a graph path seeds a stable stream across runs and machines.
fn fnv(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}
