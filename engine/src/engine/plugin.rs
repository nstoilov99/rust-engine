//! Game plugin trait for engine-game boundary.
//!
//! The engine provides this trait; game code implements it to register
//! gameplay systems with the engine schedule.

use crate::engine::ecs::resources::Resources;
use crate::engine::ecs::schedule::Schedule;

/// Trait for game plugins that register gameplay systems.
///
/// The engine calls `build()` after registering its own systems
/// (animation, physics, input, transforms). The plugin registers
/// gameplay-specific systems (player input, character movement, etc.).
pub trait GamePlugin: Send + Sync {
    /// Register systems and resources with the engine schedule.
    fn build(&self, schedule: &mut Schedule, resources: &mut Resources);

    /// Human-readable name for logging.
    fn name(&self) -> &str;
}
