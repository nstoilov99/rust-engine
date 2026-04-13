//! Enhanced action and mapping context definitions.
//!
//! Replaces the legacy `ActionMap` with a richer model supporting
//! per-binding/per-action modifier chains and trigger state machines.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::action::InputSource;
use super::modifier::InputModifier;
use super::trigger::InputTrigger;
use super::value::InputValueType;

/// An enhanced input action definition.
///
/// Actions define what value type they produce, and carry optional
/// per-action modifiers and triggers applied after all bindings merge.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputActionDefinition {
    pub name: String,
    pub value_type: InputValueType,
    /// Modifiers applied to the merged action value (after per-binding modifiers).
    #[serde(default)]
    pub modifiers: Vec<InputModifier>,
    /// Triggers evaluated on the final value. If empty, an implicit `Down` trigger is used.
    #[serde(default)]
    pub triggers: Vec<InputTrigger>,
    /// Whether this action consumes its input sources (prevents lower-priority contexts).
    #[serde(default = "default_true")]
    pub consumes_input: bool,
}

fn default_true() -> bool {
    true
}

impl InputActionDefinition {
    pub fn new(name: impl Into<String>, value_type: InputValueType) -> Self {
        Self {
            name: name.into(),
            value_type,
            modifiers: Vec::new(),
            triggers: Vec::new(),
            consumes_input: true,
        }
    }

    pub fn with_modifier(mut self, modifier: InputModifier) -> Self {
        self.modifiers.push(modifier);
        self
    }

    pub fn with_trigger(mut self, trigger: InputTrigger) -> Self {
        self.triggers.push(trigger);
        self
    }

    pub fn with_consumes(mut self, consumes: bool) -> Self {
        self.consumes_input = consumes;
        self
    }
}

/// A single binding within a mapping context entry.
///
/// Each binding maps one input source to contribute to an action's value,
/// with optional per-binding modifiers and trigger overrides.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnhancedBinding {
    pub source: InputSource,
    /// Per-binding modifiers applied before merging into the action value.
    #[serde(default)]
    pub modifiers: Vec<InputModifier>,
    /// Per-binding trigger overrides. If non-empty, these replace the action's triggers
    /// for this specific binding.
    #[serde(default)]
    pub triggers: Vec<InputTrigger>,
    /// For digital-to-axis promotion: the axis contribution when the source is active.
    /// (x, y) — only relevant components are used based on the action's value_type.
    #[serde(default)]
    pub axis_contribution: (f32, f32),
}

impl EnhancedBinding {
    /// Create a digital binding (no axis contribution).
    pub fn digital(source: InputSource) -> Self {
        Self {
            source,
            modifiers: Vec::new(),
            triggers: Vec::new(),
            axis_contribution: (0.0, 0.0),
        }
    }

    /// Create a 1D axis binding.
    pub fn axis_1d(source: InputSource, value: f32) -> Self {
        Self {
            source,
            modifiers: Vec::new(),
            triggers: Vec::new(),
            axis_contribution: (value, 0.0),
        }
    }

    /// Create a 2D axis binding.
    pub fn axis_2d(source: InputSource, x: f32, y: f32) -> Self {
        Self {
            source,
            modifiers: Vec::new(),
            triggers: Vec::new(),
            axis_contribution: (x, y),
        }
    }

    pub fn with_modifier(mut self, modifier: InputModifier) -> Self {
        self.modifiers.push(modifier);
        self
    }

    pub fn with_trigger(mut self, trigger: InputTrigger) -> Self {
        self.triggers.push(trigger);
        self
    }
}

/// An entry in a mapping context: links an action name to its bindings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MappingContextEntry {
    pub action_name: String,
    pub bindings: Vec<EnhancedBinding>,
}

impl MappingContextEntry {
    pub fn new(action_name: impl Into<String>) -> Self {
        Self {
            action_name: action_name.into(),
            bindings: Vec::new(),
        }
    }

    pub fn with_binding(mut self, binding: EnhancedBinding) -> Self {
        self.bindings.push(binding);
        self
    }
}

/// A mapping context groups action bindings with a priority level.
///
/// Higher priority contexts are processed first and can consume input,
/// preventing lower-priority contexts from seeing those sources.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MappingContext {
    pub name: String,
    /// Higher priority = processed first. Default 0.
    #[serde(default)]
    pub priority: i32,
    pub entries: Vec<MappingContextEntry>,
}

impl MappingContext {
    pub fn new(name: impl Into<String>, priority: i32) -> Self {
        Self {
            name: name.into(),
            priority,
            entries: Vec::new(),
        }
    }

    pub fn with_entry(mut self, entry: MappingContextEntry) -> Self {
        self.entries.push(entry);
        self
    }
}

/// The complete input action configuration.
///
/// Contains all action definitions and all available mapping contexts.
/// Replaces the legacy `ActionMap`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputActionSet {
    pub actions: HashMap<String, InputActionDefinition>,
    pub contexts: Vec<MappingContext>,
}

impl InputActionSet {
    pub fn new() -> Self {
        Self {
            actions: HashMap::new(),
            contexts: Vec::new(),
        }
    }

    pub fn add_action(&mut self, action: InputActionDefinition) {
        self.actions.insert(action.name.clone(), action);
    }

    pub fn add_context(&mut self, context: MappingContext) {
        self.contexts.push(context);
    }

    pub fn context(&self, name: &str) -> Option<&MappingContext> {
        self.contexts.iter().find(|c| c.name == name)
    }

    pub fn context_mut(&mut self, name: &str) -> Option<&mut MappingContext> {
        self.contexts.iter_mut().find(|c| c.name == name)
    }
}

impl Default for InputActionSet {
    fn default() -> Self {
        Self::new()
    }
}
