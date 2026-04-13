//! Physics step system — wraps `PhysicsWorld::step()` as a scheduled system.

use super::PhysicsWorld;
use crate::engine::ecs::resources::{Resources, Time};
use crate::engine::ecs::schedule::System;

/// Runs the fixed-timestep physics simulation each frame.
///
/// Reads `Time` for delta, writes `PhysicsWorld` (the Rapier state),
/// and writes `Transform` / `TransformDirty` components for dynamic bodies.
pub struct PhysicsStepSystem;

impl System for PhysicsStepSystem {
    fn run(&mut self, world: &mut hecs::World, resources: &mut Resources) {
        crate::profile_scope!("physics_step");
        let delta_time = resources.get::<Time>().map(|t| t.delta).unwrap_or(0.0);
        if let Some(physics) = resources.get_mut::<PhysicsWorld>() {
            physics.step(delta_time, world);
        }
    }

    fn name(&self) -> &str {
        "PhysicsStepSystem"
    }
}
