//! M5 Package 4: client clock sync (plan D5) and proxy interpolation
//! buffers (plan D6). Pure state machines — no ECS, unit-testable.

use std::collections::VecDeque;

/// Render this far behind estimated server time so there is normally a
/// sample pair to interpolate between (server ticks at 100ms).
pub const INTERP_DELAY_US: u64 = 150_000;

/// A gap between adjacent samples larger than this snaps instead of
/// interpolating a long glide through empty time.
const SNAP_GAP_US: u64 = 500_000;

const BUFFER_CAP: usize = 32;

/// EWMA weight for accepted clock offsets.
const CLOCK_ALPHA: f64 = 0.1;

/// Estimated server clock from NTP-style ping samples: outlier rejection on
/// RTT (vs. rolling minimum), then EWMA on the offset.
/// `server_time_us(t) = t + offset`, both on the backend's monotonic
/// `local_time_us()` timeline.
#[derive(Default)]
pub struct NetClock {
    offset_us: Option<f64>,
    min_rtt_us: Option<u64>,
    last_rtt_us: u64,
}

impl NetClock {
    pub fn add_sample(&mut self, offset_us: i64, rtt_us: u64) {
        let min = *self.min_rtt_us.get_or_insert(rtt_us);
        if rtt_us < min {
            self.min_rtt_us = Some(rtt_us);
        } else if rtt_us > min.saturating_mul(2) {
            return; // congestion spike: midpoint assumption is unreliable
        }
        self.last_rtt_us = rtt_us;
        match &mut self.offset_us {
            None => self.offset_us = Some(offset_us as f64),
            Some(o) => *o += CLOCK_ALPHA * (offset_us as f64 - *o),
        }
    }

    pub fn synced(&self) -> bool {
        self.offset_us.is_some()
    }

    pub fn server_time_us(&self, local_us: u64) -> Option<u64> {
        self.offset_us
            .map(|o| (local_us as f64 + o).max(0.0) as u64)
    }

    /// Last accepted RTT (overlay/status display).
    pub fn rtt_us(&self) -> u64 {
        self.last_rtt_us
    }
}

/// Per-proxy ring buffer of authoritative samples, evaluated at a delayed
/// render time. Out-of-order or duplicate timestamps are dropped, so the
/// same state arriving as both snapshot diff and `StateUpdate` is harmless.
#[derive(Default)]
pub struct InterpBuffer {
    samples: VecDeque<(u64, [f32; 3], f32)>,
}

impl InterpBuffer {
    pub fn push(&mut self, server_time_us: u64, pos: [f32; 3], yaw: f32) {
        if let Some(&(last, _, _)) = self.samples.back() {
            if server_time_us <= last {
                return;
            }
        }
        if self.samples.len() == BUFFER_CAP {
            self.samples.pop_front();
        }
        self.samples.push_back((server_time_us, pos, yaw));
    }

    /// Newest sample (fallback while the clock is not yet synced).
    pub fn latest(&self) -> Option<([f32; 3], f32)> {
        self.samples.back().map(|&(_, p, y)| (p, y))
    }

    /// Evaluate at `render_time_us`, discarding samples that can no longer
    /// be needed. Clamps at both ends; snaps across over-large gaps.
    pub fn sample(&mut self, render_time_us: u64) -> Option<([f32; 3], f32)> {
        while self.samples.len() >= 2 && self.samples[1].0 <= render_time_us {
            self.samples.pop_front();
        }
        let &(t0, p0, y0) = self.samples.front()?;
        if self.samples.len() < 2 || render_time_us <= t0 {
            return Some((p0, y0));
        }
        let (t1, p1, y1) = self.samples[1];
        if t1 - t0 > SNAP_GAP_US {
            return Some((p1, y1));
        }
        let f = (render_time_us - t0) as f32 / (t1 - t0) as f32;
        let pos = [
            p0[0] + (p1[0] - p0[0]) * f,
            p0[1] + (p1[1] - p0[1]) * f,
            p0[2] + (p1[2] - p0[2]) * f,
        ];
        Some((pos, lerp_yaw(y0, y1, f)))
    }
}

/// Shortest-arc yaw interpolation (radians).
fn lerp_yaw(a: f32, b: f32, f: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    let d = (b - a + PI).rem_euclid(TAU) - PI;
    a + d * f
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    #[test]
    fn clock_initializes_from_first_sample() {
        let mut c = NetClock::default();
        assert!(!c.synced());
        c.add_sample(10_000, 20_000);
        assert_eq!(c.server_time_us(100), Some(10_100));
    }

    #[test]
    fn clock_ewma_moves_toward_new_offset() {
        let mut c = NetClock::default();
        c.add_sample(0, 20_000);
        c.add_sample(10_000, 20_000);
        // 0 + 0.1 * (10_000 - 0) = 1000
        assert_eq!(c.server_time_us(0), Some(1000));
    }

    #[test]
    fn clock_rejects_rtt_outliers() {
        let mut c = NetClock::default();
        c.add_sample(0, 20_000);
        c.add_sample(1_000_000, 100_000); // >2× min RTT: rejected
        assert_eq!(c.server_time_us(0), Some(0));
        c.add_sample(10_000, 30_000); // within 2×: accepted
        assert_eq!(c.server_time_us(0), Some(1000));
    }

    #[test]
    fn clock_lowers_rolling_min() {
        let mut c = NetClock::default();
        c.add_sample(0, 40_000);
        c.add_sample(0, 15_000); // new min, accepted
        c.add_sample(5_000, 35_000); // >2×15k: now rejected
        assert_eq!(c.server_time_us(0), Some(0));
    }

    fn buf(times: &[u64]) -> InterpBuffer {
        let mut b = InterpBuffer::default();
        for &t in times {
            b.push(t, [t as f32, 0.0, 0.0], 0.0);
        }
        b
    }

    #[test]
    fn interpolates_between_samples() {
        let mut b = InterpBuffer::default();
        b.push(1000, [0.0, 0.0, 0.0], 0.0);
        b.push(2000, [10.0, 0.0, 0.0], 1.0);
        let (pos, yaw) = b.sample(1500).unwrap();
        assert_eq!(pos, [5.0, 0.0, 0.0]);
        assert!((yaw - 0.5).abs() < 1e-6);
    }

    #[test]
    fn clamps_before_first_and_after_last() {
        let mut b = buf(&[1000, 2000]);
        assert_eq!(b.sample(500).unwrap().0, [1000.0, 0.0, 0.0]);
        assert_eq!(b.sample(9000).unwrap().0, [2000.0, 0.0, 0.0]);
    }

    #[test]
    fn drops_stale_and_duplicate_samples() {
        let mut b = buf(&[1000, 2000, 1500, 2000]);
        // Only 1000 and 2000 were kept.
        assert_eq!(b.sample(1500).unwrap().0, [1500.0, 0.0, 0.0]);
    }

    #[test]
    fn snaps_across_large_gaps() {
        let mut b = InterpBuffer::default();
        b.push(1000, [0.0, 0.0, 0.0], 0.0);
        b.push(1000 + SNAP_GAP_US + 1, [10.0, 0.0, 0.0], 0.0);
        assert_eq!(b.sample(2000).unwrap().0, [10.0, 0.0, 0.0]);
    }

    #[test]
    fn discards_consumed_samples_and_caps() {
        let mut b = InterpBuffer::default();
        for i in 0..100u64 {
            b.push(i * 1000, [0.0; 3], 0.0);
        }
        assert!(b.samples.len() <= BUFFER_CAP);
        b.sample(98_500);
        assert_eq!(b.samples.len(), 2); // 98_000 kept as left edge, 99_000
    }

    #[test]
    fn yaw_takes_shortest_arc() {
        // From near +π to near −π should wrap through π, not through 0.
        let y = lerp_yaw(PI - 0.1, -PI + 0.1, 0.5);
        assert!((y.abs() - PI).abs() < 1e-5);
    }
}
