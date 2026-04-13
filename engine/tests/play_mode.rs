//! Integration tests for play mode snapshot/restore.
//!
//! These tests are gated behind `#[cfg(feature = "editor")]` because
//! play_mode.rs lives under the editor module which requires the editor feature.
//!
//! Run with: `cargo test --features editor --test play_mode`
#![cfg(feature = "editor")]

mod common;

use rust_engine::engine::ecs::components::*;
use rust_engine::engine::ecs::game_world::GameWorld;
use rust_engine::engine::ecs::hierarchy::set_parent;
use rust_engine::engine::editor::play_mode::{build_guid_map, create_snapshot, restore_snapshot};
use rust_engine::engine::editor::{CommandHistory, HierarchyPanel, Selection};
use rust_engine::engine::physics::{Collider, PhysicsWorld, RigidBody};

/// Helper: set up a game world with some entities for testing.
fn setup_test_world() -> (GameWorld, Vec<hecs::Entity>) {
    let mut game_world = GameWorld::new();

    let parent = game_world.spawn((
        Name::new("Parent"),
        Transform::new(nalgebra_glm::vec3(1.0, 2.0, 3.0)),
    ));

    let child = game_world.spawn((
        Name::new("Child"),
        Transform::new(nalgebra_glm::vec3(0.0, 1.0, 0.0)),
    ));
    set_parent(game_world.hecs_mut(), child, parent);

    let physics_entity = game_world.spawn((
        Name::new("PhysBox"),
        Transform::new(nalgebra_glm::vec3(0.0, 0.0, 5.0)),
        RigidBody::dynamic(),
        Collider::cuboid(0.5, 0.5, 0.5),
    ));

    (game_world, vec![parent, physics_entity])
}

#[test]
fn snapshot_preserves_entity_count() {
    let (mut game_world, roots) = setup_test_world();
    let mut hierarchy = HierarchyPanel::new();
    hierarchy.set_root_order(roots);

    let selection = Selection::new();
    let snapshot = create_snapshot(game_world.hecs(), &mut hierarchy, &selection)
        .expect("snapshot should succeed");

    // Snapshot data should not be empty
    assert!(!snapshot.scene_data.is_empty());

    let original_count = game_world.hecs().len();

    // Restore
    let mut physics = PhysicsWorld::new();
    let mut cmd_history = CommandHistory::new(100);
    let mut sel = Selection::new();

    restore_snapshot(
        &snapshot,
        &mut game_world,
        &mut hierarchy,
        &mut sel,
        &mut physics,
        &mut cmd_history,
    )
    .expect("restore should succeed");

    assert_eq!(
        game_world.hecs().len(),
        original_count,
        "entity count should be preserved"
    );
}

#[test]
fn snapshot_preserves_transforms() {
    let (mut game_world, roots) = setup_test_world();
    let mut hierarchy = HierarchyPanel::new();
    hierarchy.set_root_order(roots);

    let selection = Selection::new();
    let snapshot = create_snapshot(game_world.hecs(), &mut hierarchy, &selection)
        .expect("snapshot should succeed");

    let mut physics = PhysicsWorld::new();
    let mut cmd_history = CommandHistory::new(100);
    let mut sel = Selection::new();

    restore_snapshot(
        &snapshot,
        &mut game_world,
        &mut hierarchy,
        &mut sel,
        &mut physics,
        &mut cmd_history,
    )
    .expect("restore should succeed");

    // Verify parent transform was preserved
    let mut found_parent = false;
    for (_, (name, transform)) in game_world.hecs().query::<(&Name, &Transform)>().iter() {
        if name.0 == "Parent" {
            common::assert_approx_eq(transform.position.x, 1.0, 0.01);
            common::assert_approx_eq(transform.position.y, 2.0, 0.01);
            common::assert_approx_eq(transform.position.z, 3.0, 0.01);
            found_parent = true;
        }
    }
    assert!(found_parent, "Parent entity should exist after restore");
}

#[test]
fn snapshot_preserves_hierarchy() {
    let (mut game_world, roots) = setup_test_world();
    let mut hierarchy = HierarchyPanel::new();
    hierarchy.set_root_order(roots);

    let selection = Selection::new();
    let snapshot = create_snapshot(game_world.hecs(), &mut hierarchy, &selection)
        .expect("snapshot should succeed");

    let mut physics = PhysicsWorld::new();
    let mut cmd_history = CommandHistory::new(100);
    let mut sel = Selection::new();

    restore_snapshot(
        &snapshot,
        &mut game_world,
        &mut hierarchy,
        &mut sel,
        &mut physics,
        &mut cmd_history,
    )
    .expect("restore should succeed");

    // The child should still have a Parent component
    let mut child_has_parent = false;
    for (entity, name) in game_world.hecs().query::<&Name>().iter() {
        if name.0 == "Child" {
            child_has_parent = game_world
                .hecs()
                .get::<&rust_engine::engine::ecs::hierarchy::Parent>(entity)
                .is_ok();
        }
    }
    assert!(
        child_has_parent,
        "child should still be parented after restore"
    );
}

#[test]
fn snapshot_rebuilds_physics() {
    let (mut game_world, roots) = setup_test_world();
    let mut hierarchy = HierarchyPanel::new();
    hierarchy.set_root_order(roots);

    let selection = Selection::new();
    let snapshot = create_snapshot(game_world.hecs(), &mut hierarchy, &selection)
        .expect("snapshot should succeed");

    let mut physics = PhysicsWorld::new();
    let mut cmd_history = CommandHistory::new(100);
    let mut sel = Selection::new();

    restore_snapshot(
        &snapshot,
        &mut game_world,
        &mut hierarchy,
        &mut sel,
        &mut physics,
        &mut cmd_history,
    )
    .expect("restore should succeed");

    // Physics world should have a body for the PhysBox entity
    assert!(
        !physics.rigid_body_set.is_empty(),
        "physics should have at least 1 rigid body after restore"
    );
}

#[test]
fn build_guid_map_captures_all_guids() {
    let (game_world, _roots) = setup_test_world();
    let guid_map = build_guid_map(game_world.hecs());

    // All entities spawned via GameWorld::spawn get EntityGuid auto-assigned
    let guid_count = game_world.hecs().query::<&EntityGuid>().iter().count();
    assert_eq!(
        guid_map.len(),
        guid_count,
        "guid map should contain all entities with EntityGuid"
    );
}

#[test]
fn snapshot_preserves_selection() {
    let (mut game_world, roots) = setup_test_world();
    let mut hierarchy = HierarchyPanel::new();
    hierarchy.set_root_order(roots.clone());

    let mut selection = Selection::new();
    selection.select(roots[0]);

    let snapshot = create_snapshot(game_world.hecs(), &mut hierarchy, &selection)
        .expect("snapshot should succeed");

    assert!(
        snapshot.selected_guid.is_some(),
        "snapshot should capture selected entity GUID"
    );

    let mut physics = PhysicsWorld::new();
    let mut cmd_history = CommandHistory::new(100);
    let mut sel = Selection::new();

    restore_snapshot(
        &snapshot,
        &mut game_world,
        &mut hierarchy,
        &mut sel,
        &mut physics,
        &mut cmd_history,
    )
    .expect("restore should succeed");

    assert!(
        sel.primary().is_some(),
        "selection should be restored after snapshot"
    );
}

#[test]
fn snapshot_preserves_root_order() {
    let (mut game_world, roots) = setup_test_world();
    let mut hierarchy = HierarchyPanel::new();
    hierarchy.set_root_order(roots);

    let selection = Selection::new();
    let snapshot = create_snapshot(game_world.hecs(), &mut hierarchy, &selection)
        .expect("snapshot should succeed");

    assert!(
        !snapshot.root_order_guids.is_empty(),
        "root order GUIDs should not be empty"
    );

    let mut physics = PhysicsWorld::new();
    let mut cmd_history = CommandHistory::new(100);
    let mut sel = Selection::new();

    restore_snapshot(
        &snapshot,
        &mut game_world,
        &mut hierarchy,
        &mut sel,
        &mut physics,
        &mut cmd_history,
    )
    .expect("restore should succeed");

    assert!(
        !hierarchy.root_order().is_empty(),
        "root order should be restored"
    );
}
