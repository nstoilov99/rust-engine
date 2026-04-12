//! Input events emitted by the enhanced input subsystem.

use super::trigger::ActionPhase;
use super::value::InputValue;

/// An input event emitted when an action's trigger state changes.
///
/// These are sent to the ECS double-buffered event system, readable
/// the frame after they are emitted.
#[derive(Debug, Clone)]
pub struct InputEvent {
    /// Name of the action that fired.
    pub action_name: String,
    /// Current phase of the action.
    pub phase: ActionPhase,
    /// The processed action value.
    pub value: InputValue,
    /// How long the action has been in the triggered state (seconds).
    pub elapsed: f32,
}

// Implement the engine's Event trait so InputEvent can be used with Events<InputEvent>.
impl crate::engine::ecs::events::Event for InputEvent {}

/// Convenience type alias for reading input events.
pub type InputEvents = crate::engine::ecs::events::Events<InputEvent>;

/// Action phase display names for debug/editor use.
impl ActionPhase {
    pub fn label(&self) -> &'static str {
        match self {
            ActionPhase::None => "None",
            ActionPhase::Started => "Started",
            ActionPhase::Triggered => "Triggered",
            ActionPhase::Ongoing => "Ongoing",
            ActionPhase::Completed => "Completed",
            ActionPhase::Canceled => "Canceled",
        }
    }
}

/// Compact summary for logging.
impl std::fmt::Display for InputEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.action_name, self.phase.label())
    }
}
