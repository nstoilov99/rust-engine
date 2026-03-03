//! Play mode management: Edit <-> Playing <-> Paused, Stop restores snapshot.

use crate::engine::ecs::components::EntityGuid;
use crate::engine::ecs::game_world::GameWorld;
use crate::engine::editor::commands::CommandHistory;
use crate::engine::editor::hierarchy_panel::HierarchyPanel;
use crate::engine::editor::selection::Selection;
use crate::engine::physics::PhysicsWorld;
use crate::engine::scene::{load_scene_from_string, serialize_scene_to_string};
use hecs::Entity;
use std::collections::HashMap;
use uuid::Uuid;

/// Snapshot of the scene state taken when entering play mode.
pub struct PlayModeSnapshot {
    /// The entire scene serialized as a RON string.
    pub scene_data: String,
    /// Root entity ordering captured as GUIDs.
    pub root_order_guids: Vec<Uuid>,
    /// Primary selected entity GUID at time of snapshot.
    pub selected_guid: Option<Uuid>,
}

/// Build a mapping from UUID -> Entity for all entities with EntityGuid.
pub fn build_guid_map(world: &hecs::World) -> HashMap<Uuid, Entity> {
    let mut map = HashMap::new();
    for (entity, guid) in world.query::<&EntityGuid>().iter() {
        map.insert(guid.0, entity);
    }
    map
}

/// Get the GUID of the primary selected entity, if it has one.
pub fn selection_guid(world: &hecs::World, selection: &Selection) -> Option<Uuid> {
    selection.primary().and_then(|entity| {
        world
            .get::<&EntityGuid>(entity)
            .ok()
            .map(|g| g.0)
    })
}

/// Create a snapshot of the current scene state.
/// Syncs hierarchy root order with ECS before serializing to ensure no entities are missed.
pub fn create_snapshot(
    world: &hecs::World,
    hierarchy_panel: &mut HierarchyPanel,
    selection: &Selection,
) -> Result<PlayModeSnapshot, Box<dyn std::error::Error>> {
    hierarchy_panel.sync_root_order(world);

    let scene_data = serialize_scene_to_string(
        world,
        "PlayModeSnapshot",
        hierarchy_panel.root_order(),
    )?;

    let root_order_guids: Vec<Uuid> = hierarchy_panel
        .root_order()
        .iter()
        .filter_map(|&entity| {
            world.get::<&EntityGuid>(entity).ok().map(|g| g.0)
        })
        .collect();

    let selected_guid = selection_guid(world, selection);

    Ok(PlayModeSnapshot {
        scene_data,
        root_order_guids,
        selected_guid,
    })
}

/// Restore the scene from a snapshot.
/// Clears the world and rebuilds everything from the snapshot RON string.
pub fn restore_snapshot(
    snapshot: &PlayModeSnapshot,
    game_world: &mut GameWorld,
    hierarchy_panel: &mut HierarchyPanel,
    selection: &mut Selection,
    physics_world: &mut PhysicsWorld,
    command_history: &mut CommandHistory,
) -> Result<(), Box<dyn std::error::Error>> {
    // Load scene from snapshot RON string (clears world internally)
    let (_name, root_entities) = load_scene_from_string(
        game_world.hecs_mut(),
        &snapshot.scene_data,
    )?;

    // Build GUID -> Entity map for the restored world
    let guid_map = build_guid_map(game_world.hecs());

    // Restore root order from GUIDs (preserving original ordering)
    let restored_root_order: Vec<Entity> = snapshot
        .root_order_guids
        .iter()
        .filter_map(|guid| guid_map.get(guid).copied())
        .collect();

    if !restored_root_order.is_empty() {
        hierarchy_panel.set_root_order(restored_root_order);
    } else {
        // Fallback: use the root_entities from load if GUID mapping failed
        hierarchy_panel.set_root_order(root_entities);
    }

    // Restore selection from GUID
    selection.clear();
    if let Some(guid) = &snapshot.selected_guid {
        if let Some(&entity) = guid_map.get(guid) {
            selection.select(entity);
        }
    }

    // Rebuild physics world from restored ECS state
    rebuild_physics(physics_world, game_world.hecs_mut());

    // Clear undo/redo history (commands reference stale Entity handles)
    command_history.clear();

    Ok(())
}

/// Clear and rebuild the physics world from current ECS state.
pub fn rebuild_physics(physics_world: &mut PhysicsWorld, world: &mut hecs::World) {
    use crate::engine::ecs::components::Transform;
    use crate::engine::physics::{Collider as PhysCollider, RigidBody as PhysRigidBody};

    // Clear all Rapier state
    physics_world.rigid_body_set = rapier3d::prelude::RigidBodySet::new();
    physics_world.collider_set = rapier3d::prelude::ColliderSet::new();
    physics_world.island_manager = rapier3d::prelude::IslandManager::new();
    physics_world.broad_phase = rapier3d::prelude::DefaultBroadPhase::new();
    physics_world.narrow_phase = rapier3d::prelude::NarrowPhase::new();
    physics_world.impulse_joint_set = rapier3d::prelude::ImpulseJointSet::new();
    physics_world.multibody_joint_set = rapier3d::prelude::MultibodyJointSet::new();
    physics_world.ccd_solver = rapier3d::prelude::CCDSolver::new();
    physics_world.query_pipeline = rapier3d::prelude::QueryPipeline::new();
    physics_world.reset_accumulator();

    for (_, rigidbody) in world.query::<&mut PhysRigidBody>().iter() {
        rigidbody.handle = None;
    }

    for (_, collider) in world.query::<&mut PhysCollider>().iter() {
        collider.handle = None;
    }

    // Re-register all physics entities from ECS
    for (_, (transform, rigidbody, collider)) in world
        .query::<(&Transform, &mut PhysRigidBody, &mut PhysCollider)>()
        .iter()
    {
        physics_world.register_entity(transform, rigidbody, collider);
    }
}

#[cfg(test)]
mod tests {
    use super::rebuild_physics;
    use crate::engine::ecs::components::Transform;
    use crate::engine::physics::{Collider, PhysicsWorld, RigidBody};
    use hecs::World;
    use nalgebra_glm as glm;

    #[test]
    fn rebuild_physics_re_registers_bodies_with_stale_handles() {
        let mut world = World::new();
        let entity = world.spawn((
            Transform::new(glm::vec3(0.0, 0.0, 5.0)),
            RigidBody::dynamic(),
            Collider::cuboid(0.5, 0.5, 0.5),
        ));

        let mut physics_world = PhysicsWorld::new();
        {
            let mut query = world.query::<(&Transform, &mut RigidBody, &mut Collider)>();
            let (_, (transform, rigidbody, collider)) = query
                .iter()
                .next()
                .expect("spawned entity should have physics components");
            physics_world.register_entity(transform, rigidbody, collider);
            assert!(rigidbody.handle.is_some());
            assert!(collider.handle.is_some());
        }

        rebuild_physics(&mut physics_world, &mut world);

        assert_eq!(physics_world.rigid_body_set.len(), 1);
        assert_eq!(physics_world.collider_set.len(), 1);

        physics_world.step(0.5, &mut world);

        let transform = world
            .get::<&Transform>(entity)
            .expect("entity should still have a transform");
        assert!(transform.position.z < 5.0);
    }
}
