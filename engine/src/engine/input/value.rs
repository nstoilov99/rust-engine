//! Enhanced input value types.
//!
//! `InputValue` represents the processed output of an action, supporting
//! digital, 1D, 2D, and 3D value types.

use glam::{Vec2, Vec3};
use serde::{Deserialize, Serialize};

/// The kind of value an action produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum InputValueType {
    Digital,
    Axis1D,
    Axis2D,
    Axis3D,
}

/// Runtime value produced by an enhanced action.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum InputValue {
    Digital(bool),
    Axis1D(f32),
    Axis2D(Vec2),
    Axis3D(Vec3),
}

impl InputValue {
    /// Zero value for a given type.
    pub fn zero(value_type: InputValueType) -> Self {
        match value_type {
            InputValueType::Digital => InputValue::Digital(false),
            InputValueType::Axis1D => InputValue::Axis1D(0.0),
            InputValueType::Axis2D => InputValue::Axis2D(Vec2::ZERO),
            InputValueType::Axis3D => InputValue::Axis3D(Vec3::ZERO),
        }
    }

    /// Get the value type of this value.
    pub fn value_type(&self) -> InputValueType {
        match self {
            InputValue::Digital(_) => InputValueType::Digital,
            InputValue::Axis1D(_) => InputValueType::Axis1D,
            InputValue::Axis2D(_) => InputValueType::Axis2D,
            InputValue::Axis3D(_) => InputValueType::Axis3D,
        }
    }

    /// Whether this value is "active" (non-zero / true).
    pub fn is_active(&self) -> bool {
        match self {
            InputValue::Digital(v) => *v,
            InputValue::Axis1D(v) => v.abs() > f32::EPSILON,
            InputValue::Axis2D(v) => v.length_squared() > f32::EPSILON,
            InputValue::Axis3D(v) => v.length_squared() > f32::EPSILON,
        }
    }

    /// Get as a bool (Digital or magnitude > 0).
    pub fn as_bool(&self) -> bool {
        self.is_active()
    }

    /// Get as f32 (Digital: 0/1, Axis1D: value, Axis2D/3D: length).
    pub fn as_f32(&self) -> f32 {
        match self {
            InputValue::Digital(v) => if *v { 1.0 } else { 0.0 },
            InputValue::Axis1D(v) => *v,
            InputValue::Axis2D(v) => v.length(),
            InputValue::Axis3D(v) => v.length(),
        }
    }

    /// Get as Vec2 (Digital: (v,0), Axis1D: (v,0), Axis2D: value, Axis3D: xy).
    pub fn as_vec2(&self) -> Vec2 {
        match self {
            InputValue::Digital(v) => Vec2::new(if *v { 1.0 } else { 0.0 }, 0.0),
            InputValue::Axis1D(v) => Vec2::new(*v, 0.0),
            InputValue::Axis2D(v) => *v,
            InputValue::Axis3D(v) => v.truncate(),
        }
    }

    /// Get as Vec3.
    pub fn as_vec3(&self) -> Vec3 {
        match self {
            InputValue::Digital(v) => Vec3::new(if *v { 1.0 } else { 0.0 }, 0.0, 0.0),
            InputValue::Axis1D(v) => Vec3::new(*v, 0.0, 0.0),
            InputValue::Axis2D(v) => v.extend(0.0),
            InputValue::Axis3D(v) => *v,
        }
    }
}

impl Default for InputValue {
    fn default() -> Self {
        InputValue::Digital(false)
    }
}

/// Convert from legacy ActionValue.
impl From<super::action::ActionValue> for InputValue {
    fn from(v: super::action::ActionValue) -> Self {
        match v {
            super::action::ActionValue::Digital(b) => InputValue::Digital(b),
            super::action::ActionValue::Axis1D(f) => InputValue::Axis1D(f),
            super::action::ActionValue::Axis2D(x, y) => InputValue::Axis2D(Vec2::new(x, y)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_values() {
        assert_eq!(InputValue::zero(InputValueType::Digital), InputValue::Digital(false));
        assert_eq!(InputValue::zero(InputValueType::Axis1D), InputValue::Axis1D(0.0));
        assert_eq!(InputValue::zero(InputValueType::Axis2D), InputValue::Axis2D(Vec2::ZERO));
        assert_eq!(InputValue::zero(InputValueType::Axis3D), InputValue::Axis3D(Vec3::ZERO));
    }

    #[test]
    fn is_active() {
        assert!(!InputValue::Digital(false).is_active());
        assert!(InputValue::Digital(true).is_active());
        assert!(!InputValue::Axis1D(0.0).is_active());
        assert!(InputValue::Axis1D(0.5).is_active());
        assert!(!InputValue::Axis2D(Vec2::ZERO).is_active());
        assert!(InputValue::Axis2D(Vec2::new(1.0, 0.0)).is_active());
    }

    #[test]
    fn conversions() {
        let d = InputValue::Digital(true);
        assert_eq!(d.as_f32(), 1.0);
        assert_eq!(d.as_vec2(), Vec2::new(1.0, 0.0));

        let a = InputValue::Axis2D(Vec2::new(0.5, 0.3));
        assert!((a.as_f32() - Vec2::new(0.5, 0.3).length()).abs() < 0.001);
        assert!(a.as_bool());
    }
}
