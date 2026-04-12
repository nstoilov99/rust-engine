//! Analog input processing: dead zones and response curves.

use serde::{Deserialize, Serialize};

/// Response curve for analog input.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum ResponseCurve {
    #[default]
    Linear,
    Quadratic,
    Cubic,
}

/// Settings for processing analog input values.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalogSettings {
    pub dead_zone_inner: f32,
    pub dead_zone_outer: f32,
    pub response_curve: ResponseCurve,
}

impl Default for AnalogSettings {
    fn default() -> Self {
        Self {
            dead_zone_inner: 0.15,
            dead_zone_outer: 0.95,
            response_curve: ResponseCurve::Linear,
        }
    }
}

impl AnalogSettings {
    /// Apply dead zone and response curve to a 1D value in [-1, 1].
    pub fn apply(&self, raw: f32) -> f32 {
        let sign = raw.signum();
        let magnitude = raw.abs();
        sign * self.remap_magnitude(magnitude)
    }

    /// Apply radial dead zone and response curve to a 2D value.
    pub fn apply_2d(&self, raw_x: f32, raw_y: f32) -> (f32, f32) {
        let magnitude = (raw_x * raw_x + raw_y * raw_y).sqrt();
        if magnitude < f32::EPSILON {
            return (0.0, 0.0);
        }
        let remapped = self.remap_magnitude(magnitude);
        let scale = remapped / magnitude;
        (raw_x * scale, raw_y * scale)
    }

    fn remap_magnitude(&self, magnitude: f32) -> f32 {
        if magnitude < self.dead_zone_inner {
            return 0.0;
        }
        let range = self.dead_zone_outer - self.dead_zone_inner;
        if range <= f32::EPSILON {
            return if magnitude >= self.dead_zone_inner { 1.0 } else { 0.0 };
        }
        let normalized = ((magnitude - self.dead_zone_inner) / range).clamp(0.0, 1.0);
        match self.response_curve {
            ResponseCurve::Linear => normalized,
            ResponseCurve::Quadratic => normalized * normalized,
            ResponseCurve::Cubic => normalized * normalized * normalized,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dead_zone_inner_clamps_to_zero() {
        let s = AnalogSettings { dead_zone_inner: 0.2, dead_zone_outer: 0.9, response_curve: ResponseCurve::Linear };
        assert_eq!(s.apply(0.0), 0.0);
        assert_eq!(s.apply(0.1), 0.0);
        assert_eq!(s.apply(-0.1), 0.0);
    }

    #[test]
    fn dead_zone_outer_clamps_to_one() {
        let s = AnalogSettings { dead_zone_inner: 0.1, dead_zone_outer: 0.9, response_curve: ResponseCurve::Linear };
        assert_eq!(s.apply(1.0), 1.0);
        assert_eq!(s.apply(-1.0), -1.0);
    }

    #[test]
    fn linear_midpoint() {
        let s = AnalogSettings { dead_zone_inner: 0.0, dead_zone_outer: 1.0, response_curve: ResponseCurve::Linear };
        assert!((s.apply(0.5) - 0.5).abs() < 0.001);
    }

    #[test]
    fn quadratic_curve() {
        let s = AnalogSettings { dead_zone_inner: 0.0, dead_zone_outer: 1.0, response_curve: ResponseCurve::Quadratic };
        assert!((s.apply(0.5) - 0.25).abs() < 0.001);
    }

    #[test]
    fn radial_2d_dead_zone() {
        let s = AnalogSettings { dead_zone_inner: 0.2, dead_zone_outer: 0.9, response_curve: ResponseCurve::Linear };
        let (x, y) = s.apply_2d(0.1, 0.1);
        assert_eq!(x, 0.0);
        assert_eq!(y, 0.0);
    }

    #[test]
    fn preserves_sign() {
        let s = AnalogSettings::default();
        let pos = s.apply(0.8);
        let neg = s.apply(-0.8);
        assert!(pos > 0.0);
        assert!(neg < 0.0);
        assert!((pos + neg).abs() < 0.001);
    }
}
