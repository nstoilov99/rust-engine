//! Shared test helpers for integration tests.
//!
//! Integration tests in `tests/` are compiled as separate crates and cannot
//! access `#[cfg(test)]`-gated modules from `src/`. This module provides
//! equivalent helpers for integration test use.

use rust_engine::engine::ecs::components::{EntityGuid, Name, Transform};
use rust_engine::engine::ecs::hierarchy::set_parent;

/// Assert two f32 values are approximately equal.
#[allow(dead_code)]
pub fn assert_approx_eq(a: f32, b: f32, epsilon: f32) {
    assert!(
        (a - b).abs() < epsilon,
        "assert_approx_eq failed: {} vs {} (epsilon {})",
        a,
        b,
        epsilon
    );
}

/// Spawn a named entity with Transform, Name, and EntityGuid.
#[allow(dead_code)]
pub fn spawn_named_entity(
    world: &mut hecs::World,
    name: &str,
    x: f32,
    y: f32,
    z: f32,
) -> hecs::Entity {
    world.spawn((
        Name::new(name),
        EntityGuid::new(),
        Transform::new(nalgebra_glm::vec3(x, y, z)),
    ))
}

/// Spawn a child entity with a parent relationship.
#[allow(dead_code)]
pub fn spawn_child_entity(
    world: &mut hecs::World,
    parent: hecs::Entity,
    name: &str,
    x: f32,
    y: f32,
    z: f32,
) -> hecs::Entity {
    let child = spawn_named_entity(world, name, x, y, z);
    set_parent(world, child, parent);
    child
}
