//! Enhanced Input Subsystem: the central runtime for processing input.
//!
//! Replaces `InputActionSystem`. Manages active contexts, runs the
//! modifier → trigger pipeline, and emits `InputEvent`s.

use std::collections::{HashMap, HashSet};

use super::enhanced_action::{InputActionDefinition, InputActionSet};
use super::event::InputEvent;
use super::input_reader::InputReader;
use super::modifier::InputModifier;
use super::pipeline;
use super::trigger::{ActionPhase, InputTrigger, TriggerState};
use super::value::{InputValue, InputValueType};
use crate::engine::ecs::events::Events;

/// Per-action runtime state tracked by the subsystem.
#[derive(Debug, Clone)]
struct PerActionState {
    current_value: InputValue,
    previous_value: InputValue,
    phase: ActionPhase,
    elapsed_triggered: f32,
    was_active: bool,
    /// Mutable copies of per-action modifiers (for stateful modifiers like Smooth).
    action_modifiers: Vec<InputModifier>,
    /// Mutable copies of per-action triggers.
    action_triggers: Vec<InputTrigger>,
}

impl PerActionState {
    fn new(action: &InputActionDefinition) -> Self {
        Self {
            current_value: InputValue::zero(action.value_type),
            previous_value: InputValue::zero(action.value_type),
            phase: ActionPhase::None,
            elapsed_triggered: 0.0,
            was_active: false,
            action_modifiers: action.modifiers.clone(),
            action_triggers: action.triggers.clone(),
        }
    }

    #[allow(dead_code)]
    fn reset(&mut self, action: &InputActionDefinition) {
        self.current_value = InputValue::zero(action.value_type);
        self.previous_value = InputValue::zero(action.value_type);
        self.phase = ActionPhase::None;
        self.elapsed_triggered = 0.0;
        self.was_active = false;
        self.action_modifiers = action.modifiers.clone();
        self.action_triggers = action.triggers.clone();
        for m in &mut self.action_modifiers {
            m.reset();
        }
        for t in &mut self.action_triggers {
            t.reset();
        }
    }
}

/// Per-binding runtime state (for stateful per-binding modifiers/triggers).
#[derive(Debug, Clone)]
struct PerBindingState {
    modifiers: Vec<InputModifier>,
    #[allow(dead_code)]
    triggers: Vec<InputTrigger>,
}

/// The Enhanced Input Subsystem.
///
/// This is inserted as an ECS resource and manages all input processing.
pub struct InputSubsystem {
    /// The complete action/context configuration.
    pub action_set: InputActionSet,
    /// Names of currently active contexts.
    active_contexts: Vec<String>,
    /// Per-action runtime state.
    action_states: HashMap<String, PerActionState>,
    /// Per-binding runtime state, keyed by (context_name, action_name, binding_index).
    binding_states: HashMap<(String, String, usize), PerBindingState>,
    /// Input sources consumed this frame (by higher-priority contexts).
    consumed_sources: HashSet<u64>,
    /// Sorted context processing order cache (highest priority first).
    sorted_contexts: Vec<String>,
    /// Whether the sorted order needs recalculation.
    contexts_dirty: bool,
}

impl InputSubsystem {
    /// Create a new subsystem from an action set.
    pub fn new(action_set: InputActionSet) -> Self {
        let mut action_states = HashMap::new();
        for (name, action) in &action_set.actions {
            action_states.insert(name.clone(), PerActionState::new(action));
        }

        Self {
            action_set,
            active_contexts: Vec::new(),
            action_states,
            binding_states: HashMap::new(),
            consumed_sources: HashSet::new(),
            sorted_contexts: Vec::new(),
            contexts_dirty: true,
        }
    }

    /// Activate a mapping context by name.
    pub fn add_context(&mut self, name: &str) {
        if !self.active_contexts.iter().any(|c| c == name) {
            self.active_contexts.push(name.to_string());
            self.contexts_dirty = true;
            self.init_binding_states_for_context(name);
        }
    }

    /// Deactivate a mapping context by name.
    pub fn remove_context(&mut self, name: &str) {
        if let Some(idx) = self.active_contexts.iter().position(|c| c == name) {
            self.active_contexts.remove(idx);
            self.contexts_dirty = true;
            // Clean up binding states for removed context
            self.binding_states
                .retain(|(ctx, _, _), _| ctx != name);
        }
    }

    /// Check if a context is active.
    pub fn has_context(&self, name: &str) -> bool {
        self.active_contexts.iter().any(|c| c == name)
    }

    /// Get the list of active context names.
    pub fn active_contexts(&self) -> &[String] {
        &self.active_contexts
    }

    /// Process all input for this frame.
    pub fn tick(
        &mut self,
        reader: &dyn InputReader,
        dt: f32,
        events: &mut Events<InputEvent>,
    ) {
        crate::profile_scope!("enhanced_input_tick");

        // Update sorted context order if needed
        if self.contexts_dirty {
            self.rebuild_sorted_contexts();
            self.contexts_dirty = false;
        }

        // Clear consumed sources
        self.consumed_sources.clear();

        // Shift current → previous for all actions
        for state in self.action_states.values_mut() {
            state.previous_value = state.current_value;
            state.was_active = state.current_value.is_active();
            state.current_value = InputValue::zero(state.current_value.value_type());
        }

        // Track which actions have been resolved (higher-priority context wins).
        let mut resolved_actions: HashSet<String> = HashSet::new();

        // Process each active context in priority order (highest first)
        let sorted = self.sorted_contexts.clone();
        for ctx_name in &sorted {
            let Some(context) = self.action_set.context(ctx_name) else {
                continue;
            };

            for entry in &context.entries {
                let action_name = &entry.action_name;

                // Skip if already resolved by a higher-priority context
                if resolved_actions.contains(action_name) {
                    continue;
                }

                let Some(action_def) = self.action_set.actions.get(action_name) else {
                    continue;
                };
                let value_type = action_def.value_type;

                // Accumulate value from all bindings
                let mut accumulated = InputValue::zero(value_type);

                for (bind_idx, binding) in entry.bindings.iter().enumerate() {
                    let source_id = pipeline::source_id(&binding.source);

                    // Skip consumed sources
                    if self.consumed_sources.contains(&source_id) {
                        continue;
                    }

                    // Collect raw value
                    let raw = pipeline::collect_raw_value(&binding.source, reader);

                    // Apply per-binding modifiers
                    let key = (ctx_name.clone(), action_name.clone(), bind_idx);
                    let modified = if let Some(bind_state) = self.binding_states.get_mut(&key) {
                        pipeline::apply_modifiers(raw, &mut bind_state.modifiers, dt)
                    } else {
                        raw
                    };

                    // Promote to action's value type
                    let promoted =
                        pipeline::promote_value(modified, value_type, binding.axis_contribution);

                    accumulated = accumulate(accumulated, promoted);
                }

                // Apply per-action modifiers and evaluate triggers.
                // We need to temporarily extract mutable parts to avoid borrow conflicts
                // with chord lookup needing to read other action_states.
                let action_name_owned = action_name.clone();
                let consumes = action_def.consumes_input;

                if let Some(action_state) = self.action_states.get_mut(&action_name_owned) {
                    // Apply per-action modifiers
                    let modified = pipeline::apply_modifiers(
                        accumulated,
                        &mut action_state.action_modifiers,
                        dt,
                    );
                    action_state.current_value = modified;
                }

                // Extract triggers temporarily for chord evaluation
                let (was_active, current_value, old_phase, mut triggers) = {
                    let Some(s) = self.action_states.get_mut(&action_name_owned) else {
                        continue;
                    };
                    (
                        s.was_active,
                        s.current_value,
                        s.phase,
                        std::mem::take(&mut s.action_triggers),
                    )
                };

                // Now self.action_states is not mutably borrowed, so chord lookup works
                let trigger_result = pipeline::evaluate_triggers(
                    &mut triggers,
                    &current_value,
                    was_active,
                    dt,
                    &|chord_name: &str| -> bool {
                        self.action_states
                            .get(chord_name)
                            .is_some_and(|s| s.current_value.is_active())
                    },
                );

                // Put triggers back and update state
                if let Some(action_state) = self.action_states.get_mut(&action_name_owned) {
                    action_state.action_triggers = triggers;

                    let new_phase = compute_phase(old_phase, trigger_result, &current_value);

                    // Emit event on phase change
                    if new_phase != ActionPhase::None && new_phase != old_phase
                        || new_phase == ActionPhase::Triggered
                    {
                        if new_phase == ActionPhase::Triggered {
                            action_state.elapsed_triggered += dt;
                        } else {
                            action_state.elapsed_triggered = 0.0;
                        }

                        events.send(InputEvent {
                            action_name: action_name_owned.clone(),
                            phase: new_phase,
                            value: current_value,
                            elapsed: action_state.elapsed_triggered,
                        });
                    }

                    action_state.phase = new_phase;

                    // Mark as resolved
                    resolved_actions.insert(action_name_owned);

                    // Consume sources if action is triggered and consumes_input
                    if consumes
                        && matches!(
                            new_phase,
                            ActionPhase::Triggered | ActionPhase::Started | ActionPhase::Ongoing
                        )
                    {
                        for binding in &entry.bindings {
                            self.consumed_sources
                                .insert(pipeline::source_id(&binding.source));
                        }
                    }
                }
            }
        }
    }

    /// Rebuild the action_set (e.g., after editor changes) and re-init states.
    pub fn set_action_set(&mut self, action_set: InputActionSet) {
        self.action_set = action_set;
        self.rebuild_action_states();
        self.rebuild_binding_states();
        self.contexts_dirty = true;
    }

    // --- Query methods (backward compat with ActionState API) ---

    /// Whether an action transitioned to active this frame.
    pub fn just_pressed(&self, name: &str) -> bool {
        self.action_states.get(name).is_some_and(|s| {
            s.current_value.is_active() && !s.was_active
        })
    }

    /// Whether an action transitioned to inactive this frame.
    pub fn just_released(&self, name: &str) -> bool {
        self.action_states.get(name).is_some_and(|s| {
            !s.current_value.is_active() && s.was_active
        })
    }

    /// Whether a digital action is currently active.
    pub fn digital(&self, name: &str) -> bool {
        self.action_states
            .get(name)
            .is_some_and(|s| s.current_value.is_active())
    }

    /// Get a 1D axis value.
    pub fn axis_1d(&self, name: &str) -> f32 {
        self.action_states
            .get(name)
            .map(|s| s.current_value.as_f32())
            .unwrap_or(0.0)
    }

    /// Get a 2D axis value as (x, y) tuple.
    pub fn axis_2d(&self, name: &str) -> (f32, f32) {
        self.action_states
            .get(name)
            .map(|s| {
                let v = s.current_value.as_vec2();
                (v.x, v.y)
            })
            .unwrap_or((0.0, 0.0))
    }

    /// Get the raw InputValue for an action.
    pub fn value(&self, name: &str) -> Option<InputValue> {
        self.action_states.get(name).map(|s| s.current_value)
    }

    /// Get the current phase of an action.
    pub fn phase(&self, name: &str) -> ActionPhase {
        self.action_states
            .get(name)
            .map(|s| s.phase)
            .unwrap_or(ActionPhase::None)
    }

    /// Get a snapshot of all action phases (for debug overlay).
    pub fn action_phases(&self) -> Vec<(String, ActionPhase, InputValue)> {
        self.action_states
            .iter()
            .map(|(name, state)| (name.clone(), state.phase, state.current_value))
            .collect()
    }

    /// Get the set of consumed source IDs this frame.
    pub fn consumed_sources(&self) -> &HashSet<u64> {
        &self.consumed_sources
    }

    // --- Internal helpers ---

    fn rebuild_sorted_contexts(&mut self) {
        self.sorted_contexts = self.active_contexts.clone();
        let action_set = &self.action_set;
        self.sorted_contexts.sort_by(|a, b| {
            let pa = action_set.context(a).map(|c| c.priority).unwrap_or(0);
            let pb = action_set.context(b).map(|c| c.priority).unwrap_or(0);
            pb.cmp(&pa) // descending (highest first)
        });
    }

    fn rebuild_action_states(&mut self) {
        let mut new_states = HashMap::new();
        for (name, action) in &self.action_set.actions {
            if let Some(existing) = self.action_states.get(name) {
                new_states.insert(name.clone(), existing.clone());
            } else {
                new_states.insert(name.clone(), PerActionState::new(action));
            }
        }
        self.action_states = new_states;
    }

    fn rebuild_binding_states(&mut self) {
        self.binding_states.clear();
        let ctx_names: Vec<String> = self.active_contexts.clone();
        for ctx_name in &ctx_names {
            self.init_binding_states_for_context(ctx_name);
        }
    }

    fn init_binding_states_for_context(&mut self, ctx_name: &str) {
        let Some(context) = self.action_set.context(ctx_name) else {
            return;
        };
        for entry in &context.entries {
            for (idx, binding) in entry.bindings.iter().enumerate() {
                let key = (ctx_name.to_string(), entry.action_name.clone(), idx);
                self.binding_states.entry(key).or_insert_with(|| {
                    PerBindingState {
                        modifiers: binding.modifiers.clone(),
                        triggers: binding.triggers.clone(),
                    }
                });
            }
        }
    }
}

/// Accumulate two InputValues of the same type (additive merge).
fn accumulate(a: InputValue, b: InputValue) -> InputValue {
    match (a, b) {
        (InputValue::Digital(a), InputValue::Digital(b)) => InputValue::Digital(a || b),
        (InputValue::Axis1D(a), InputValue::Axis1D(b)) => InputValue::Axis1D(a + b),
        (InputValue::Axis2D(a), InputValue::Axis2D(b)) => InputValue::Axis2D(a + b),
        (InputValue::Axis3D(a), InputValue::Axis3D(b)) => InputValue::Axis3D(a + b),
        // Mismatched types: prefer the second value
        (_, b) => b,
    }
}

/// Compute the new ActionPhase based on old phase and trigger result.
fn compute_phase(
    old: ActionPhase,
    trigger_state: TriggerState,
    _value: &InputValue,
) -> ActionPhase {
    match trigger_state {
        TriggerState::Triggered => match old {
            ActionPhase::None | ActionPhase::Completed | ActionPhase::Canceled => {
                ActionPhase::Started
            }
            ActionPhase::Started | ActionPhase::Ongoing => ActionPhase::Triggered,
            ActionPhase::Triggered => ActionPhase::Triggered,
        },
        TriggerState::Ongoing => match old {
            ActionPhase::None | ActionPhase::Completed | ActionPhase::Canceled => {
                ActionPhase::Started
            }
            _ => ActionPhase::Ongoing,
        },
        TriggerState::Idle => match old {
            ActionPhase::Triggered | ActionPhase::Ongoing | ActionPhase::Started => {
                ActionPhase::Completed
            }
            _ => ActionPhase::None,
        },
    }
}

/// ECS System wrapper for the Enhanced Input Subsystem.
pub struct EnhancedInputSystem;

impl crate::engine::ecs::schedule::System for EnhancedInputSystem {
    fn run(&mut self, _world: &mut hecs::World, resources: &mut crate::engine::ecs::resources::Resources) {
        crate::profile_scope!("enhanced_input_system");

        // Get dt from Time resource
        let dt = resources
            .get::<crate::engine::ecs::resources::Time>()
            .map(|t| t.delta)
            .unwrap_or(1.0 / 60.0);

        // Remove subsystem to get &mut access without borrow conflicts
        let Some(mut subsystem) = resources.remove::<InputSubsystem>() else {
            return;
        };

        // Remove input sources to avoid borrow conflicts
        let Some(input_manager) = resources.remove::<super::InputManager>() else {
            resources.insert(subsystem);
            return;
        };
        let gamepad_state = resources.remove::<super::gamepad::GamepadState>();
        let reader = super::input_reader::FullInputReader {
            input: &input_manager,
            gamepad: gamepad_state.as_ref(),
        };

        // Get or create events
        let mut events = resources
            .remove::<Events<InputEvent>>()
            .unwrap_or_default();

        // Tick the subsystem
        subsystem.tick(&reader, dt, &mut events);

        // Re-insert input sources
        resources.insert(input_manager);
        if let Some(gp) = gamepad_state {
            resources.insert(gp);
        }

        // Write back to legacy ActionState for backward compat
        if let Some(action_state) = resources.get_mut::<super::action_state::ActionState>() {
            action_state.begin_frame();
            for (name, state) in &subsystem.action_states {
                let legacy_value = match state.current_value {
                    InputValue::Digital(b) => super::action::ActionValue::Digital(b),
                    InputValue::Axis1D(f) => super::action::ActionValue::Axis1D(f),
                    InputValue::Axis2D(v) => super::action::ActionValue::Axis2D(v.x, v.y),
                    InputValue::Axis3D(v) => super::action::ActionValue::Axis2D(v.x, v.y),
                };
                let legacy_type = match state.current_value.value_type() {
                    InputValueType::Digital => super::action::ActionType::Digital,
                    InputValueType::Axis1D => super::action::ActionType::Axis1D,
                    InputValueType::Axis2D | InputValueType::Axis3D => {
                        super::action::ActionType::Axis2D
                    }
                };
                action_state.set_value(name, legacy_type, legacy_value);
            }
        }

        // Re-insert
        resources.insert(events);
        resources.insert(subsystem);
    }

    fn name(&self) -> &str {
        "EnhancedInputSystem"
    }
}

impl EnhancedInputSystem {
    pub fn descriptor() -> crate::engine::ecs::access::SystemDescriptor {
        crate::engine::ecs::access::SystemDescriptor::new("EnhancedInputSystem")
            .reads_resource::<super::InputManager>()
            .reads_resource::<super::gamepad::GamepadState>()
            .reads_resource::<crate::engine::ecs::resources::Time>()
            .writes_resource::<InputSubsystem>()
            .writes_resource::<super::action_state::ActionState>()
    }

    pub fn stage() -> crate::engine::ecs::schedule::Stage {
        crate::engine::ecs::schedule::Stage::First
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::input::action::*;
    use crate::engine::input::enhanced_action::*;
    use crate::engine::input::trigger::InputTrigger;
    use crate::engine::input::value::InputValueType;

    struct MockReader {
        pressed_keys: Vec<KeyCode>,
    }

    impl InputReader for MockReader {
        fn is_key_pressed(&self, key: KeyCode) -> bool {
            self.pressed_keys.contains(&key)
        }
        fn is_key_just_pressed(&self, _: KeyCode) -> bool { false }
        fn is_mouse_pressed(&self, _: MouseButton) -> bool { false }
        fn mouse_delta(&self) -> (f32, f32) { (0.0, 0.0) }
        fn scroll_delta(&self) -> f32 { 0.0 }
        fn is_gamepad_pressed(&self, _: GamepadButton) -> bool { false }
        fn gamepad_axis(&self, _: GamepadAxisType) -> f32 { 0.0 }
    }

    fn make_test_set() -> InputActionSet {
        let mut set = InputActionSet::new();
        set.add_action(
            InputActionDefinition::new("jump", InputValueType::Digital)
                .with_trigger(InputTrigger::Pressed),
        );
        set.add_action(InputActionDefinition::new("move", InputValueType::Axis2D));

        let ctx = MappingContext::new("gameplay", 0)
            .with_entry(
                MappingContextEntry::new("jump")
                    .with_binding(EnhancedBinding::digital(InputSource::Key(KeyCode::Space))),
            )
            .with_entry(
                MappingContextEntry::new("move")
                    .with_binding(EnhancedBinding::axis_2d(
                        InputSource::Key(KeyCode::KeyW),
                        0.0,
                        1.0,
                    ))
                    .with_binding(EnhancedBinding::axis_2d(
                        InputSource::Key(KeyCode::KeyS),
                        0.0,
                        -1.0,
                    ))
                    .with_binding(EnhancedBinding::axis_2d(
                        InputSource::Key(KeyCode::KeyD),
                        1.0,
                        0.0,
                    ))
                    .with_binding(EnhancedBinding::axis_2d(
                        InputSource::Key(KeyCode::KeyA),
                        -1.0,
                        0.0,
                    )),
            );
        set.add_context(ctx);
        set
    }

    #[test]
    fn basic_digital_action() {
        let set = make_test_set();
        let mut sub = InputSubsystem::new(set);
        sub.add_context("gameplay");
        let mut events = Events::<InputEvent>::new();

        // Frame 1: not pressed
        let reader = MockReader { pressed_keys: vec![] };
        sub.tick(&reader, 0.016, &mut events);
        assert!(!sub.digital("jump"));

        // Frame 2: pressed
        let reader = MockReader { pressed_keys: vec![KeyCode::Space] };
        sub.tick(&reader, 0.016, &mut events);
        assert!(sub.digital("jump"));
    }

    #[test]
    fn axis_2d_movement() {
        let set = make_test_set();
        let mut sub = InputSubsystem::new(set);
        sub.add_context("gameplay");
        let mut events = Events::<InputEvent>::new();

        let reader = MockReader {
            pressed_keys: vec![KeyCode::KeyW, KeyCode::KeyD],
        };
        sub.tick(&reader, 0.016, &mut events);
        let (x, y) = sub.axis_2d("move");
        assert!(x > 0.0, "expected positive x, got {x}");
        assert!(y > 0.0, "expected positive y, got {y}");
    }

    #[test]
    fn context_priority() {
        let mut set = InputActionSet::new();
        set.add_action(
            InputActionDefinition::new("action", InputValueType::Digital)
                .with_consumes(true),
        );

        // High priority context: binds Space
        let high = MappingContext::new("high", 100).with_entry(
            MappingContextEntry::new("action")
                .with_binding(EnhancedBinding::digital(InputSource::Key(KeyCode::Space))),
        );
        // Low priority context: also binds Space
        let low = MappingContext::new("low", 0).with_entry(
            MappingContextEntry::new("action")
                .with_binding(EnhancedBinding::digital(InputSource::Key(KeyCode::Space))),
        );
        set.add_context(high);
        set.add_context(low);

        let mut sub = InputSubsystem::new(set);
        sub.add_context("high");
        sub.add_context("low");
        let mut events = Events::<InputEvent>::new();

        let reader = MockReader { pressed_keys: vec![KeyCode::Space] };
        sub.tick(&reader, 0.016, &mut events);

        // High priority context should have consumed Space, so "action" resolves
        // from high context (resolved_actions prevents low from overriding)
        assert!(sub.digital("action"));
    }
}
