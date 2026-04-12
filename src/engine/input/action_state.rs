//! Action state: runtime query interface for resolved action values.

use super::action::{ActionType, ActionValue};
use std::collections::HashMap;

#[derive(Debug, Clone)]
struct ActionEntry {
    action_type: ActionType,
    current: ActionValue,
    previous: ActionValue,
}

/// Runtime state for all resolved actions. Inserted as an ECS resource.
pub struct ActionState {
    actions: HashMap<String, ActionEntry>,
    context_stack: Vec<String>,
}

impl ActionState {
    pub fn new() -> Self {
        Self { actions: HashMap::new(), context_stack: Vec::new() }
    }

    pub fn push_context(&mut self, name: impl Into<String>) {
        self.context_stack.push(name.into());
    }

    pub fn pop_context(&mut self) -> Option<String> {
        self.context_stack.pop()
    }

    pub fn context_stack(&self) -> &[String] {
        &self.context_stack
    }

    pub fn has_context(&self, name: &str) -> bool {
        self.context_stack.iter().any(|c| c == name)
    }

    pub fn begin_frame(&mut self) {
        for entry in self.actions.values_mut() {
            entry.previous = entry.current;
        }
    }

    pub fn set_value(&mut self, name: &str, action_type: ActionType, value: ActionValue) {
        let entry = self.actions.entry(name.to_string()).or_insert_with(|| ActionEntry {
            action_type,
            current: ActionValue::zero(action_type),
            previous: ActionValue::zero(action_type),
        });
        entry.current = value;
        entry.action_type = action_type;
    }

    pub fn just_pressed(&self, name: &str) -> bool {
        self.actions.get(name).is_some_and(|e| {
            matches!(e.current, ActionValue::Digital(true))
                && matches!(e.previous, ActionValue::Digital(false))
        })
    }

    pub fn just_released(&self, name: &str) -> bool {
        self.actions.get(name).is_some_and(|e| {
            matches!(e.current, ActionValue::Digital(false))
                && matches!(e.previous, ActionValue::Digital(true))
        })
    }

    pub fn digital(&self, name: &str) -> bool {
        self.actions.get(name).is_some_and(|e| matches!(e.current, ActionValue::Digital(true)))
    }

    pub fn axis_1d(&self, name: &str) -> f32 {
        self.actions.get(name).map(|e| match e.current {
            ActionValue::Axis1D(v) => v,
            _ => 0.0,
        }).unwrap_or(0.0)
    }

    pub fn axis_2d(&self, name: &str) -> (f32, f32) {
        self.actions.get(name).map(|e| match e.current {
            ActionValue::Axis2D(x, y) => (x, y),
            _ => (0.0, 0.0),
        }).unwrap_or((0.0, 0.0))
    }

    pub fn value(&self, name: &str) -> Option<ActionValue> {
        self.actions.get(name).map(|e| e.current)
    }
}

impl Default for ActionState {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn just_pressed_transition() {
        let mut state = ActionState::new();
        state.set_value("jump", ActionType::Digital, ActionValue::Digital(false));
        state.begin_frame();
        state.set_value("jump", ActionType::Digital, ActionValue::Digital(true));
        assert!(state.just_pressed("jump"));
        assert!(!state.just_released("jump"));
        assert!(state.digital("jump"));
    }

    #[test]
    fn just_released_transition() {
        let mut state = ActionState::new();
        state.set_value("jump", ActionType::Digital, ActionValue::Digital(true));
        state.begin_frame();
        state.set_value("jump", ActionType::Digital, ActionValue::Digital(false));
        assert!(!state.just_pressed("jump"));
        assert!(state.just_released("jump"));
    }

    #[test]
    fn held_not_just_pressed() {
        let mut state = ActionState::new();
        state.set_value("jump", ActionType::Digital, ActionValue::Digital(true));
        state.begin_frame();
        state.set_value("jump", ActionType::Digital, ActionValue::Digital(true));
        assert!(!state.just_pressed("jump"));
        assert!(state.digital("jump"));
    }

    #[test]
    fn axis_2d_values() {
        let mut state = ActionState::new();
        state.set_value("move", ActionType::Axis2D, ActionValue::Axis2D(0.5, -0.3));
        let (x, y) = state.axis_2d("move");
        assert!((x - 0.5).abs() < 0.001);
        assert!((y + 0.3).abs() < 0.001);
    }

    #[test]
    fn context_stack() {
        let mut state = ActionState::new();
        state.push_context("gameplay");
        assert!(state.has_context("gameplay"));
        state.push_context("menu");
        assert_eq!(state.context_stack().len(), 2);
        let popped = state.pop_context();
        assert_eq!(popped, Some("menu".to_string()));
        assert!(!state.has_context("menu"));
    }

    #[test]
    fn unknown_action_returns_defaults() {
        let state = ActionState::new();
        assert!(!state.digital("nonexistent"));
        assert_eq!(state.axis_1d("nonexistent"), 0.0);
        assert_eq!(state.axis_2d("nonexistent"), (0.0, 0.0));
    }
}
