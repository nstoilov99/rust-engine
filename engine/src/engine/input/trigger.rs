//! Input triggers: state machines that determine when actions fire.
//!
//! Triggers evaluate processed input values and emit phase transitions
//! (Started, Triggered, Completed, Canceled). Inspired by Unreal's
//! Enhanced Input trigger system.

use serde::{Deserialize, Serialize};

use super::value::InputValue;

/// The phase of an action as determined by its triggers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ActionPhase {
    /// No trigger activity.
    None,
    /// Trigger evaluation has started (input is active, waiting to fire).
    Started,
    /// Trigger condition is met — action fires this frame.
    Triggered,
    /// Trigger is still evaluating (e.g., held but not yet long enough).
    Ongoing,
    /// Action was released / completed normally.
    Completed,
    /// Action was canceled (e.g., held released before duration threshold).
    Canceled,
}

/// Internal state for trigger evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum TriggerState {
    #[default]
    Idle,
    /// Input is active, trigger is evaluating.
    Ongoing,
    /// Trigger has fired.
    Triggered,
}

/// An input trigger that determines when an action transitions between phases.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum InputTrigger {
    /// Fires every frame the input is non-zero/active.
    Down,

    /// Fires once on the frame the input transitions from inactive to active.
    Pressed,

    /// Fires once on the frame the input transitions from active to inactive.
    Released,

    /// Fires once after the input has been held active for `duration` seconds.
    Held {
        duration: f32,
        #[serde(skip)]
        elapsed: f32,
        #[serde(skip)]
        fired: bool,
    },

    /// Fires if input is pressed and released within `max_duration` seconds.
    Tap {
        max_duration: f32,
        #[serde(skip)]
        elapsed: f32,
        #[serde(skip)]
        was_active: bool,
    },

    /// Fires repeatedly at `interval` seconds while input is held.
    /// `trigger_limit` of 0 means unlimited.
    Pulse {
        interval: f32,
        trigger_limit: u32,
        #[serde(skip)]
        elapsed: f32,
        #[serde(skip)]
        pulse_count: u32,
    },

    /// Requires another action to also be active (chord).
    /// The chord action is looked up by name at evaluation time.
    ChordAction {
        action_name: String,
    },
}

impl InputTrigger {
    /// Evaluate the trigger for this frame.
    ///
    /// `was_active` indicates whether the input was active in the previous frame.
    /// `chord_active` is a lookup function for ChordAction triggers.
    pub fn evaluate(
        &mut self,
        value: &InputValue,
        was_active: bool,
        dt: f32,
        chord_active: &dyn Fn(&str) -> bool,
    ) -> TriggerState {
        let active = value.is_active();

        match self {
            InputTrigger::Down => {
                if active {
                    TriggerState::Triggered
                } else {
                    TriggerState::Idle
                }
            }

            InputTrigger::Pressed => {
                if active && !was_active {
                    TriggerState::Triggered
                } else {
                    TriggerState::Idle
                }
            }

            InputTrigger::Released => {
                if !active && was_active {
                    TriggerState::Triggered
                } else {
                    TriggerState::Idle
                }
            }

            InputTrigger::Held {
                duration,
                elapsed,
                fired,
            } => {
                if active {
                    *elapsed += dt;
                    if *elapsed >= *duration && !*fired {
                        *fired = true;
                        TriggerState::Triggered
                    } else if !*fired {
                        TriggerState::Ongoing
                    } else {
                        // Already fired, stay triggered while held
                        TriggerState::Triggered
                    }
                } else {
                    *elapsed = 0.0;
                    *fired = false;
                    TriggerState::Idle
                }
            }

            InputTrigger::Tap {
                max_duration,
                elapsed,
                was_active: tap_was_active,
            } => {
                if active {
                    *elapsed += dt;
                    *tap_was_active = true;
                    if *elapsed > *max_duration {
                        // Held too long — cancel the tap
                        TriggerState::Idle
                    } else {
                        TriggerState::Ongoing
                    }
                } else if *tap_was_active {
                    // Just released
                    let result = if *elapsed <= *max_duration {
                        TriggerState::Triggered
                    } else {
                        TriggerState::Idle
                    };
                    *elapsed = 0.0;
                    *tap_was_active = false;
                    result
                } else {
                    TriggerState::Idle
                }
            }

            InputTrigger::Pulse {
                interval,
                trigger_limit,
                elapsed,
                pulse_count,
            } => {
                if active {
                    *elapsed += dt;
                    if *elapsed >= *interval {
                        *elapsed -= *interval;
                        if *trigger_limit == 0 || *pulse_count < *trigger_limit {
                            *pulse_count += 1;
                            TriggerState::Triggered
                        } else {
                            TriggerState::Ongoing
                        }
                    } else {
                        TriggerState::Ongoing
                    }
                } else {
                    *elapsed = 0.0;
                    *pulse_count = 0;
                    TriggerState::Idle
                }
            }

            InputTrigger::ChordAction { action_name } => {
                if active && chord_active(action_name) {
                    TriggerState::Triggered
                } else if active {
                    TriggerState::Ongoing
                } else {
                    TriggerState::Idle
                }
            }
        }
    }

    /// Reset any internal state.
    pub fn reset(&mut self) {
        match self {
            InputTrigger::Held {
                elapsed, fired, ..
            } => {
                *elapsed = 0.0;
                *fired = false;
            }
            InputTrigger::Tap {
                elapsed,
                was_active,
                ..
            } => {
                *elapsed = 0.0;
                *was_active = false;
            }
            InputTrigger::Pulse {
                elapsed,
                pulse_count,
                ..
            } => {
                *elapsed = 0.0;
                *pulse_count = 0;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec2;

    fn no_chord(_: &str) -> bool {
        false
    }

    #[test]
    fn down_trigger_fires_while_active() {
        let mut t = InputTrigger::Down;
        assert_eq!(
            t.evaluate(&InputValue::Digital(true), false, 0.016, &no_chord),
            TriggerState::Triggered
        );
        assert_eq!(
            t.evaluate(&InputValue::Digital(false), true, 0.016, &no_chord),
            TriggerState::Idle
        );
    }

    #[test]
    fn pressed_trigger_fires_on_transition() {
        let mut t = InputTrigger::Pressed;
        // Not active -> active = Triggered
        assert_eq!(
            t.evaluate(&InputValue::Digital(true), false, 0.016, &no_chord),
            TriggerState::Triggered
        );
        // Active -> active = Idle (already pressed)
        assert_eq!(
            t.evaluate(&InputValue::Digital(true), true, 0.016, &no_chord),
            TriggerState::Idle
        );
    }

    #[test]
    fn released_trigger_fires_on_transition() {
        let mut t = InputTrigger::Released;
        assert_eq!(
            t.evaluate(&InputValue::Digital(false), true, 0.016, &no_chord),
            TriggerState::Triggered
        );
        assert_eq!(
            t.evaluate(&InputValue::Digital(false), false, 0.016, &no_chord),
            TriggerState::Idle
        );
    }

    #[test]
    fn held_trigger_fires_after_duration() {
        let mut t = InputTrigger::Held {
            duration: 0.5,
            elapsed: 0.0,
            fired: false,
        };
        let active = InputValue::Digital(true);

        // Frame 1: ongoing (0.016s < 0.5s)
        assert_eq!(t.evaluate(&active, false, 0.016, &no_chord), TriggerState::Ongoing);

        // Simulate holding for 0.5s total
        for _ in 0..30 {
            t.evaluate(&active, true, 0.016, &no_chord);
        }
        // Should have triggered by now
        assert_eq!(t.evaluate(&active, true, 0.016, &no_chord), TriggerState::Triggered);
    }

    #[test]
    fn tap_trigger_fires_on_quick_release() {
        let mut t = InputTrigger::Tap {
            max_duration: 0.3,
            elapsed: 0.0,
            was_active: false,
        };
        let active = InputValue::Digital(true);
        let inactive = InputValue::Digital(false);

        // Press
        assert_eq!(t.evaluate(&active, false, 0.016, &no_chord), TriggerState::Ongoing);
        // Release quickly
        assert_eq!(t.evaluate(&inactive, true, 0.016, &no_chord), TriggerState::Triggered);
    }

    #[test]
    fn pulse_trigger_fires_at_interval() {
        let mut t = InputTrigger::Pulse {
            interval: 0.1,
            trigger_limit: 0,
            elapsed: 0.0,
            pulse_count: 0,
        };
        let active = InputValue::Digital(true);

        // First few frames: ongoing
        assert_eq!(t.evaluate(&active, false, 0.05, &no_chord), TriggerState::Ongoing);
        // After 0.1s total: triggered
        assert_eq!(t.evaluate(&active, true, 0.05, &no_chord), TriggerState::Triggered);
    }

    #[test]
    fn axis_2d_active() {
        let mut t = InputTrigger::Down;
        let v = InputValue::Axis2D(Vec2::new(0.5, 0.3));
        assert_eq!(t.evaluate(&v, false, 0.016, &no_chord), TriggerState::Triggered);
    }
}
