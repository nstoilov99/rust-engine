// Legacy modules (kept for backward compatibility)
pub mod action;
pub mod action_map;
pub mod action_state;
pub mod analog;
pub mod default_bindings;
pub mod gamepad;
mod input_manager;
pub mod input_reader;
pub mod key_convert;
pub mod serialization;
pub mod system;

// Enhanced input modules
pub mod enhanced_action;
pub mod enhanced_defaults;
pub mod enhanced_serialization;
pub mod event;
pub mod modifier;
pub mod pipeline;
pub mod subsystem;
pub mod trigger;
pub mod value;
#[cfg(feature = "editor")]
pub mod debug_overlay;

// Legacy re-exports (backward compat)
pub use action_map::ActionMap;
pub use action_state::ActionState;
pub use gamepad::GamepadState;
pub use input_manager::InputManager;
pub use system::InputActionSystem;

// Enhanced re-exports
pub use enhanced_action::{
    EnhancedBinding, InputActionDefinition, InputActionSet, MappingContext, MappingContextEntry,
};
pub use event::InputEvent;
pub use modifier::InputModifier;
pub use subsystem::{EnhancedInputSystem, InputSubsystem};
pub use trigger::{ActionPhase, InputTrigger};
pub use value::{InputValue, InputValueType};
