//! Action map: defines actions and organizes them into input contexts.

use super::action::{ActionBinding, ActionType, GamepadStick2D};
use super::analog::AnalogSettings;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Definition of a single named action with its type and bindings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionDefinition {
    pub name: String,
    pub action_type: ActionType,
    pub bindings: Vec<ActionBinding>,
    pub gamepad_stick: Option<GamepadStick2D>,
    pub analog_settings: AnalogSettings,
}

impl ActionDefinition {
    pub fn new(name: impl Into<String>, action_type: ActionType) -> Self {
        Self {
            name: name.into(),
            action_type,
            bindings: Vec::new(),
            gamepad_stick: None,
            analog_settings: AnalogSettings::default(),
        }
    }

    pub fn with_binding(mut self, binding: ActionBinding) -> Self {
        self.bindings.push(binding);
        self
    }

    pub fn with_gamepad_stick(mut self, stick: GamepadStick2D) -> Self {
        self.gamepad_stick = Some(stick);
        self
    }
}

/// A named group of actions that can be activated/deactivated together.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputContext {
    pub name: String,
    pub actions: Vec<ActionDefinition>,
}

impl InputContext {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), actions: Vec::new() }
    }

    pub fn with_action(mut self, action: ActionDefinition) -> Self {
        self.actions.push(action);
        self
    }
}

/// Top-level action map containing all input contexts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionMap {
    pub contexts: HashMap<String, InputContext>,
}

impl ActionMap {
    pub fn new() -> Self {
        Self { contexts: HashMap::new() }
    }

    pub fn add_context(&mut self, context: InputContext) {
        self.contexts.insert(context.name.clone(), context);
    }

    pub fn context(&self, name: &str) -> Option<&InputContext> {
        self.contexts.get(name)
    }

    pub fn context_mut(&mut self, name: &str) -> Option<&mut InputContext> {
        self.contexts.get_mut(name)
    }
}

impl Default for ActionMap {
    fn default() -> Self { Self::new() }
}
