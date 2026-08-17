//! Task 41 tracer demo: gameplay writing an animation parameter.
//!
//! Three seconds into play, every animation-graph entity's `walk` Bool flips
//! true — enough to watch the demo machine crossfade Idle → Walk on the
//! `Human` in `main.scene`. This is the ADR 0002 posture in miniature:
//! gameplay writes a parameter on the blackboard, never a state.
//!
//! Demo-only, like the runner-demo graph on the Duck. The write is refused
//! harmlessly by any graph that declares no Bool `walk`, and the whole system
//! is one component scan per frame when no machines exist. The accumulator
//! survives play-stop, so a second play session starts already walking —
//! acceptable for a demo, wrong for anything that ships.

use rust_engine::engine::animation::graph::AnimGraphRuntime;
use rust_engine::engine::ecs::access::SystemDescriptor;
use rust_engine::engine::ecs::resources::{Resources, Time};
use rust_engine::engine::ecs::schedule::System;

/// Seconds of play before the demo flips `walk`.
const WALK_AFTER_SECONDS: f32 = 3.0;

#[derive(Default)]
pub struct AnimGraphDemoSystem {
    elapsed: f32,
}

impl AnimGraphDemoSystem {
    pub fn descriptor() -> SystemDescriptor {
        SystemDescriptor::new("AnimGraphDemoSystem")
            .reads_resource::<Time>()
            .writes::<AnimGraphRuntime>()
    }
}

impl System for AnimGraphDemoSystem {
    fn run(&mut self, world: &mut hecs::World, resources: &mut Resources) {
        let dt = resources
            .get::<Time>()
            .map(|t| t.scaled_delta())
            .unwrap_or(0.0);
        self.elapsed += dt;
        let walk = self.elapsed > WALK_AFTER_SECONDS;
        for (_entity, rt) in world.query_mut::<&mut AnimGraphRuntime>() {
            rt.params.set_bool("walk", walk);
        }
    }

    fn name(&self) -> &str {
        "AnimGraphDemoSystem"
    }
}
