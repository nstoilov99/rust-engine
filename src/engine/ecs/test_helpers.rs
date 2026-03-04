//! Shared test helpers for ECS unit tests.
//!
//! This module is gated behind `#[cfg(test)]` so it's only compiled for tests.
//! Integration tests in `tests/` cannot access this module — they use
//! `tests/common/mod.rs` instead.

use hecs::World;
use nalgebra_glm as glm;

use super::components::{EntityGuid, Name, Transform};
use super::hierarchy::set_parent;
use super::resources::{EditorState, PlayMode, Resources, Time};

/// Assert two f32 values are approximately equal.
pub fn assert_approx_eq(a: f32, b: f32, epsilon: f32) {
    assert!(
        (a - b).abs() < epsilon,
        "assert_approx_eq failed: {} vs {} (epsilon {})",
        a,
        b,
        epsilon
    );
}

/// Assert two Vec3 values are approximately equal.
pub fn assert_vec3_approx_eq(a: &glm::Vec3, b: &glm::Vec3, epsilon: f32) {
    assert_approx_eq(a.x, b.x, epsilon);
    assert_approx_eq(a.y, b.y, epsilon);
    assert_approx_eq(a.z, b.z, epsilon);
}

/// Assert two quaternions represent the same rotation (accounting for sign flip).
pub fn assert_quat_approx_eq(a: &glm::Quat, b: &glm::Quat, epsilon: f32) {
    let direct = (a.coords.x - b.coords.x).abs()
        + (a.coords.y - b.coords.y).abs()
        + (a.coords.z - b.coords.z).abs()
        + (a.coords.w - b.coords.w).abs();
    let negated = (a.coords.x + b.coords.x).abs()
        + (a.coords.y + b.coords.y).abs()
        + (a.coords.z + b.coords.z).abs()
        + (a.coords.w + b.coords.w).abs();
    assert!(
        direct < epsilon || negated < epsilon,
        "assert_quat_approx_eq failed: {:?} vs {:?}",
        a,
        b
    );
}

/// Spawn a named entity with a transform at the given position.
pub fn spawn_named_at(world: &mut World, name: &str, x: f32, y: f32, z: f32) -> hecs::Entity {
    world.spawn((
        Name::new(name),
        EntityGuid::new(),
        Transform::new(glm::vec3(x, y, z)),
    ))
}

/// Spawn a child entity parented to `parent` with a local offset.
pub fn spawn_child_at(
    world: &mut World,
    parent: hecs::Entity,
    name: &str,
    x: f32,
    y: f32,
    z: f32,
) -> hecs::Entity {
    let child = spawn_named_at(world, name, x, y, z);
    set_parent(world, child, parent);
    child
}

/// Create a minimal Resources with Time and EditorState in Edit mode.
pub fn test_resources() -> Resources {
    let mut resources = Resources::new();
    resources.insert(Time::new());
    resources.insert(EditorState::new());
    resources
}

/// Create Resources with EditorState set to Playing.
pub fn test_resources_playing() -> Resources {
    let mut resources = test_resources();
    if let Some(state) = resources.get_mut::<EditorState>() {
        state.play_mode = PlayMode::Playing;
    }
    resources
}
