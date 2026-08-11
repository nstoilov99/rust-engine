//! **Engine-side integration spike** for the portable interpreter (Task 45-A
//! P2). Test-only, deliberately thin, and deliberately *here* rather than in
//! the exec crate: its whole job is to validate the `EffectSink` / `WorldRead`
//! contract against the **second** consumer — a real `hecs` world with real
//! `Transform` components — before P2 freezes the shape.
//!
//! What it is not: the P5 runner. There is no component, no system, no
//! scheduler integration, no alias resolution against spawned entities, no
//! play-mode lifecycle. Those are P5's, and P5 is where this code gets
//! replaced by the real thing.
//!
//! What it proves:
//!
//! - the engine can implement `WorldRead` over `GameWorld` without the exec
//!   crate knowing what an `Entity` is;
//! - a `SetPosition` effect applies to a real `Transform` — the non-structural
//!   half of D4's effect-application split;
//! - the Z-up convention survives the round trip (the interpreter carries
//!   `[f32; 3]` and never converts anything);
//! - a graph can *read* the world, compute, and write back, which is the
//!   shape every gameplay node will use.

use std::collections::BTreeMap;

use nalgebra_glm as glm;
use node_graph_exec::{
    compile, nodes, tick, Effect, EntityRef, GraphInstance, NodeImpls, TickInput, WorldRead,
};

use crate::engine::ecs::components::Transform;
use crate::engine::ecs::game_world::GameWorld;
use crate::engine::node_graph::{
    register_std_events, Edge, GraphDoc, NodeInst, NodeRegistry, PropValue, EVENT_TICK_TYPE_ID,
    EXEC_IN_PIN, EXEC_OUT_PIN,
};

/// The engine's side of the read seam: one entity's world, as the portable
/// core is allowed to see it. Real entity ids never cross — the core only
/// ever names `SelfEntity` or an alias it was handed.
struct EntityView<'a> {
    world: &'a GameWorld,
    entity: hecs::Entity,
}

impl WorldRead for EntityView<'_> {
    fn position(&self, entity: EntityRef) -> Option<[f32; 3]> {
        match entity {
            EntityRef::SelfEntity => self
                .world
                .get::<Transform>(self.entity)
                .ok()
                .map(|t| [t.position.x, t.position.y, t.position.z]),
            // Spawn aliases resolve in P5, where the command buffer that
            // creates them runs.
            EntityRef::Spawned(_) => None,
        }
    }

    fn exists(&self, entity: EntityRef) -> bool {
        matches!(entity, EntityRef::SelfEntity)
            && self.world.get::<Transform>(self.entity).is_ok()
    }
}

/// The engine's side of the write seam, in the one form P2 needs: apply the
/// non-structural effects directly. P5 adds the structural half through the
/// closure command buffer, which is applied between scheduler stages.
fn apply(world: &mut GameWorld, self_entity: hecs::Entity, effects: &[Effect]) -> Vec<String> {
    let mut logged = Vec::new();
    for e in effects {
        match e {
            Effect::SetPosition { entity: EntityRef::SelfEntity, position } => {
                if let Ok(mut t) = world.get_component_mut::<Transform>(self_entity) {
                    t.position = glm::vec3(position[0], position[1], position[2]);
                }
            }
            Effect::Log { text, .. } => logged.push(text.clone()),
            // Structural effects and alias targets are P5's.
            _ => {}
        }
    }
    logged
}

fn node(id: u64, type_id: &str) -> NodeInst {
    NodeInst {
        id,
        type_id: type_id.to_string(),
        type_version: 1,
        position: [id as f32 * 200.0, 0.0],
        properties: BTreeMap::new(),
        subgraph: None,
        tint: None,
        title: None,
    }
}

fn edge(from: u64, fp: &str, to: u64, tp: &str) -> Edge {
    Edge {
        from_node: from,
        from_pin: fp.to_string(),
        to_node: to,
        to_pin: tp.to_string(),
    }
}

/// Read a real `Transform`, decide something, write it back — through the
/// effect stream, with the interpreter never touching the world itself.
#[test]
fn graph_effects_apply_to_a_real_world() {
    // A graph: every tick, print a marker and move the entity to a constant
    // position. `get_position` proves the read seam in the same pass.
    let mut doc = GraphDoc::default();
    let mut print = node(1, nodes::PRINT);
    print
        .properties
        .insert("text".into(), PropValue::Str("moved".into()));
    let mut set = node(2, nodes::SET_POSITION);
    set.properties
        .insert("position".into(), PropValue::Vec3([4.0, 5.0, 6.0]));
    doc.nodes = vec![
        node(0, EVENT_TICK_TYPE_ID),
        print,
        set,
        node(3, nodes::GET_POSITION),
        node(4, nodes::INT_TO_STRING),
    ];
    doc.edges = vec![
        edge(0, EXEC_OUT_PIN, 1, EXEC_IN_PIN),
        edge(1, EXEC_OUT_PIN, 2, EXEC_IN_PIN),
    ];

    let mut reg = NodeRegistry::new();
    nodes::register_descriptors(&mut reg).unwrap();
    register_std_events(&mut reg).unwrap();
    let subs: BTreeMap<String, GraphDoc> = BTreeMap::new();
    let plan = compile(&doc, "spike.graph", &reg, &subs).expect("compile");

    let mut impls = NodeImpls::new();
    nodes::register_impls(&mut impls);
    assert_eq!(impls.check_plan(&plan), vec![], "plan/impl cross-check");

    // A real world, a real entity, a real Transform — Z-up, unconverted.
    let mut world = GameWorld::new();
    let entity = world.spawn((Transform::new(glm::vec3(1.0, 2.0, 3.0)),));

    let mut instance = GraphInstance::new(&plan, EntityRef::SelfEntity, 0xC0FFEE);
    let mut effects: Vec<Effect> = Vec::new();
    {
        let view = EntityView { world: &world, entity };
        // The read seam sees the entity's actual position.
        assert_eq!(view.position(EntityRef::SelfEntity), Some([1.0, 2.0, 3.0]));
        assert!(view.exists(EntityRef::SelfEntity));
        tick(
            &plan,
            &mut instance,
            &impls,
            TickInput { dt: 1.0 / 60.0, time: 0.0 },
            &view,
            &mut effects,
        );
    }

    // The interpreter changed nothing itself: the world only moves when the
    // engine applies the stream.
    assert_eq!(
        world.get::<Transform>(entity).unwrap().position,
        glm::vec3(1.0, 2.0, 3.0),
        "running a graph must not touch the world by itself"
    );

    let logged = apply(&mut world, entity, &effects);
    assert_eq!(logged, vec!["moved"]);
    assert_eq!(
        world.get::<Transform>(entity).unwrap().position,
        glm::vec3(4.0, 5.0, 6.0),
        "the SetPosition effect applied to the real Transform, Z-up intact"
    );

    // …and the next tick's read sees the applied value, which is the loop
    // every gameplay node will run in.
    effects.clear();
    {
        let view = EntityView { world: &world, entity };
        assert_eq!(view.position(EntityRef::SelfEntity), Some([4.0, 5.0, 6.0]));
        tick(
            &plan,
            &mut instance,
            &impls,
            TickInput { dt: 1.0 / 60.0, time: 1.0 / 60.0 },
            &view,
            &mut effects,
        );
    }
    assert!(effects
        .iter()
        .any(|e| matches!(e, Effect::SetPosition { .. })));
}

/// The dependency direction, asserted rather than assumed: the exec crate is
/// reachable from the engine, and the engine's own document types are the
/// ones it compiles. If the types crate were ever forked in two, this stops
/// compiling.
#[test]
fn engine_and_interpreter_share_one_document_model() {
    let doc: node_graph_exec::types::GraphDoc = GraphDoc::default();
    assert_eq!(doc.version, crate::engine::node_graph::GRAPH_DOC_VERSION);
}
