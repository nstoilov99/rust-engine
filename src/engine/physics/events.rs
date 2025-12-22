//! Physics collision events

use rapier3d::prelude::*;
use std::sync::Mutex;

pub struct CollisionEventHandler {
    pub events: Mutex<Vec<CollisionEvent>>,
}

impl Default for CollisionEventHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl CollisionEventHandler {
    pub fn new() -> Self {
        Self {
            events: Mutex::new(Vec::new()),
        }
    }
}

impl EventHandler for CollisionEventHandler {
    fn handle_collision_event(
        &self,
        _bodies: &RigidBodySet,
        _colliders: &ColliderSet,
        event: rapier3d::prelude::CollisionEvent,
        _contact_pair: Option<&ContactPair>,
    ) {
        let mut events = self.events.lock().unwrap();
        match event {
            rapier3d::prelude::CollisionEvent::Started(h1, h2, _) => {
                events.push(CollisionEvent::Started {
                    collider1: h1,
                    collider2: h2,
                });
            }
            rapier3d::prelude::CollisionEvent::Stopped(h1, h2, _) => {
                events.push(CollisionEvent::Stopped {
                    collider1: h1,
                    collider2: h2,
                });
            }
        }
    }

    fn handle_contact_force_event(
        &self,
        _dt: Real,
        _bodies: &RigidBodySet,
        _colliders: &ColliderSet,
        _contact_pair: &ContactPair,
        _total_force_magnitude: Real,
    ) {
        // Not handling contact force events for now
    }
}

#[derive(Debug, Clone)]
pub enum CollisionEvent {
    Started {
        collider1: ColliderHandle,
        collider2: ColliderHandle,
    },
    Stopped {
        collider1: ColliderHandle,
        collider2: ColliderHandle,
    },
}
