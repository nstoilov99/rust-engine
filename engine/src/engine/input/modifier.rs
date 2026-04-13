//! Input modifiers: composable transformations applied to input values.
//!
//! Modifiers form a chain that processes raw input values before they reach
//! trigger evaluation. Inspired by Unreal's Enhanced Input modifiers.

use super::value::InputValue;
use glam::{Vec2, Vec3};
use serde::{Deserialize, Serialize};

/// Axis swizzle ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SwizzleOrder {
    YXZ,
    ZYX,
    XZY,
    YZX,
    ZXY,
}

/// Dead zone calculation mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum DeadZoneKind {
    /// Apply dead zone per-axis independently.
    PerAxis,
    /// Apply radial dead zone (magnitude-based).
    #[default]
    Radial,
}

/// Response curve type for analog processing.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub enum CurveType {
    #[default]
    Linear,
    Quadratic,
    Cubic,
    /// Custom curve defined by control points (input, output) in [0,1].
    Custom(Vec<(f32, f32)>),
}

/// A composable input modifier that transforms an `InputValue`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum InputModifier {
    /// Negate (flip sign) on selected axes.
    Negate {
        x: bool,
        y: bool,
        z: bool,
    },
    /// Remap axis order (e.g., swap X and Y).
    Swizzle {
        order: SwizzleOrder,
    },
    /// Apply dead zone processing.
    DeadZone {
        lower: f32,
        upper: f32,
        kind: DeadZoneKind,
    },
    /// Scale by a per-axis factor.
    Scale {
        factor: Vec3,
    },
    /// Frame-rate-independent smoothing (lerp toward target).
    Smooth {
        speed: f32,
        #[serde(skip)]
        previous: Option<Vec3>,
    },
    /// Apply a response curve to the magnitude.
    ResponseCurve {
        curve: CurveType,
    },
    /// Clamp each axis to a range.
    Clamp {
        min: Vec3,
        max: Vec3,
    },
}

impl InputModifier {
    /// Apply this modifier to an input value.
    pub fn apply(&mut self, value: InputValue, dt: f32) -> InputValue {
        match self {
            InputModifier::Negate { x, y, z } => apply_negate(value, *x, *y, *z),
            InputModifier::Swizzle { order } => apply_swizzle(value, *order),
            InputModifier::DeadZone { lower, upper, kind } => {
                apply_dead_zone(value, *lower, *upper, *kind)
            }
            InputModifier::Scale { factor } => apply_scale(value, *factor),
            InputModifier::Smooth { speed, previous } => {
                apply_smooth(value, *speed, previous, dt)
            }
            InputModifier::ResponseCurve { curve } => apply_response_curve(value, curve),
            InputModifier::Clamp { min, max } => apply_clamp(value, *min, *max),
        }
    }

    /// Reset any internal state (e.g., smoothing history).
    pub fn reset(&mut self) {
        if let InputModifier::Smooth { previous, .. } = self {
            *previous = None;
        }
    }
}

fn apply_negate(value: InputValue, nx: bool, ny: bool, nz: bool) -> InputValue {
    let v = value.as_vec3();
    let result = Vec3::new(
        if nx { -v.x } else { v.x },
        if ny { -v.y } else { v.y },
        if nz { -v.z } else { v.z },
    );
    reconstruct(value, result)
}

fn apply_swizzle(value: InputValue, order: SwizzleOrder) -> InputValue {
    let v = value.as_vec3();
    let result = match order {
        SwizzleOrder::YXZ => Vec3::new(v.y, v.x, v.z),
        SwizzleOrder::ZYX => Vec3::new(v.z, v.y, v.x),
        SwizzleOrder::XZY => Vec3::new(v.x, v.z, v.y),
        SwizzleOrder::YZX => Vec3::new(v.y, v.z, v.x),
        SwizzleOrder::ZXY => Vec3::new(v.z, v.x, v.y),
    };
    reconstruct(value, result)
}

fn apply_dead_zone(value: InputValue, lower: f32, upper: f32, kind: DeadZoneKind) -> InputValue {
    match kind {
        DeadZoneKind::PerAxis => {
            let v = value.as_vec3();
            let result = Vec3::new(
                remap_axis(v.x, lower, upper),
                remap_axis(v.y, lower, upper),
                remap_axis(v.z, lower, upper),
            );
            reconstruct(value, result)
        }
        DeadZoneKind::Radial => {
            let v = value.as_vec3();
            let magnitude = v.length();
            if magnitude < f32::EPSILON {
                return InputValue::zero(value.value_type());
            }
            let remapped = remap_magnitude(magnitude, lower, upper);
            let scale = remapped / magnitude;
            reconstruct(value, v * scale)
        }
    }
}

fn apply_scale(value: InputValue, factor: Vec3) -> InputValue {
    let v = value.as_vec3();
    reconstruct(value, v * factor)
}

fn apply_smooth(value: InputValue, speed: f32, previous: &mut Option<Vec3>, dt: f32) -> InputValue {
    let target = value.as_vec3();
    let prev = previous.unwrap_or(target);
    let alpha = if speed > 0.0 { (speed * dt).min(1.0) } else { 1.0 };
    let smoothed = prev.lerp(target, alpha);
    *previous = Some(smoothed);
    reconstruct(value, smoothed)
}

fn apply_response_curve(value: InputValue, curve: &CurveType) -> InputValue {
    let v = value.as_vec3();
    let result = Vec3::new(
        apply_curve_1d(v.x, curve),
        apply_curve_1d(v.y, curve),
        apply_curve_1d(v.z, curve),
    );
    reconstruct(value, result)
}

fn apply_curve_1d(raw: f32, curve: &CurveType) -> f32 {
    let sign = raw.signum();
    let mag = raw.abs();
    let curved = match curve {
        CurveType::Linear => mag,
        CurveType::Quadratic => mag * mag,
        CurveType::Cubic => mag * mag * mag,
        CurveType::Custom(points) => interpolate_custom(mag, points),
    };
    sign * curved
}

fn interpolate_custom(t: f32, points: &[(f32, f32)]) -> f32 {
    if points.is_empty() {
        return t;
    }
    if t <= points[0].0 {
        return points[0].1;
    }
    for window in points.windows(2) {
        let (x0, y0) = window[0];
        let (x1, y1) = window[1];
        if t >= x0 && t <= x1 {
            let frac = if (x1 - x0).abs() < f32::EPSILON {
                0.0
            } else {
                (t - x0) / (x1 - x0)
            };
            return y0 + frac * (y1 - y0);
        }
    }
    points.last().map(|p| p.1).unwrap_or(t)
}

fn apply_clamp(value: InputValue, min: Vec3, max: Vec3) -> InputValue {
    let v = value.as_vec3();
    reconstruct(value, v.clamp(min, max))
}

fn remap_axis(raw: f32, lower: f32, upper: f32) -> f32 {
    let sign = raw.signum();
    let mag = raw.abs();
    sign * remap_magnitude(mag, lower, upper)
}

fn remap_magnitude(magnitude: f32, lower: f32, upper: f32) -> f32 {
    if magnitude < lower {
        return 0.0;
    }
    let range = upper - lower;
    if range <= f32::EPSILON {
        return if magnitude >= lower { 1.0 } else { 0.0 };
    }
    ((magnitude - lower) / range).clamp(0.0, 1.0)
}

/// Reconstruct an InputValue of the same variant from a Vec3.
fn reconstruct(original: InputValue, v: Vec3) -> InputValue {
    match original {
        InputValue::Digital(_) => InputValue::Digital(v.x.abs() > 0.5),
        InputValue::Axis1D(_) => InputValue::Axis1D(v.x),
        InputValue::Axis2D(_) => InputValue::Axis2D(Vec2::new(v.x, v.y)),
        InputValue::Axis3D(_) => InputValue::Axis3D(v),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negate_x() {
        let mut m = InputModifier::Negate { x: true, y: false, z: false };
        let result = m.apply(InputValue::Axis2D(Vec2::new(1.0, 0.5)), 0.016);
        assert_eq!(result, InputValue::Axis2D(Vec2::new(-1.0, 0.5)));
    }

    #[test]
    fn dead_zone_below_threshold() {
        let mut m = InputModifier::DeadZone {
            lower: 0.2,
            upper: 0.9,
            kind: DeadZoneKind::Radial,
        };
        let result = m.apply(InputValue::Axis2D(Vec2::new(0.1, 0.1)), 0.016);
        assert_eq!(result, InputValue::Axis2D(Vec2::ZERO));
    }

    #[test]
    fn scale_factor() {
        let mut m = InputModifier::Scale { factor: Vec3::new(2.0, 3.0, 1.0) };
        let result = m.apply(InputValue::Axis2D(Vec2::new(0.5, 0.5)), 0.016);
        assert_eq!(result, InputValue::Axis2D(Vec2::new(1.0, 1.5)));
    }

    #[test]
    fn response_curve_quadratic() {
        let mut m = InputModifier::ResponseCurve { curve: CurveType::Quadratic };
        let result = m.apply(InputValue::Axis1D(0.5), 0.016);
        match result {
            InputValue::Axis1D(v) => assert!((v - 0.25).abs() < 0.001),
            _ => panic!("expected Axis1D"),
        }
    }

    #[test]
    fn swizzle_yx() {
        let mut m = InputModifier::Swizzle { order: SwizzleOrder::YXZ };
        let result = m.apply(InputValue::Axis2D(Vec2::new(1.0, 2.0)), 0.016);
        assert_eq!(result, InputValue::Axis2D(Vec2::new(2.0, 1.0)));
    }

    #[test]
    fn clamp_values() {
        let mut m = InputModifier::Clamp {
            min: Vec3::new(-0.5, -0.5, -0.5),
            max: Vec3::new(0.5, 0.5, 0.5),
        };
        let result = m.apply(InputValue::Axis1D(1.0), 0.016);
        assert_eq!(result, InputValue::Axis1D(0.5));
    }
}
