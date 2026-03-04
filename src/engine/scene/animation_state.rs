use std::collections::HashMap;

/// Defines a transition between animations
#[derive(Debug, Clone)]
pub struct AnimationTransition {
    pub from_state: String,
    pub to_state: String,
    pub condition: TransitionCondition,
}

#[derive(Debug, Clone)]
pub enum TransitionCondition {
    Immediate,                // Transition immediately
    OnComplete,               // When current animation finishes
    OnInput(String),          // When specific input is pressed
    OnParameter(String, f32), // When parameter reaches value
}

/// Animation state machine
pub struct AnimationStateMachine {
    states: HashMap<String, String>, // state_name -> animation_name
    transitions: Vec<AnimationTransition>,
    current_state: String,
    parameters: HashMap<String, f32>,
}

impl AnimationStateMachine {
    pub fn new(initial_state: &str) -> Self {
        Self {
            states: HashMap::new(),
            transitions: Vec::new(),
            current_state: initial_state.to_string(),
            parameters: HashMap::new(),
        }
    }

    /// Add a state (maps state name to animation name)
    pub fn add_state(&mut self, state_name: &str, animation_name: &str) {
        self.states
            .insert(state_name.to_string(), animation_name.to_string());
    }

    /// Add a transition between states
    pub fn add_transition(&mut self, transition: AnimationTransition) {
        self.transitions.push(transition);
    }

    /// Set a parameter value
    pub fn set_parameter(&mut self, name: &str, value: f32) {
        self.parameters.insert(name.to_string(), value);
    }

    /// Get current animation name based on state
    pub fn get_current_animation(&self) -> Option<&str> {
        self.states.get(&self.current_state).map(|s| s.as_str())
    }

    /// Update state machine (checks transitions)
    pub fn update(&mut self, animation_finished: bool) {
        for transition in &self.transitions {
            if transition.from_state != self.current_state {
                continue;
            }

            let should_transition = match &transition.condition {
                TransitionCondition::Immediate => true,
                TransitionCondition::OnComplete => animation_finished,
                TransitionCondition::OnParameter(param, target) => self
                    .parameters
                    .get(param)
                    .map(|v| v >= target)
                    .unwrap_or(false),
                _ => false,
            };

            if should_transition {
                self.current_state = transition.to_state.clone();
                break;
            }
        }
    }
}
