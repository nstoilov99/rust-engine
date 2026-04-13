//! Immediate-mode debug draw API.
//!
//! All coordinates are in Z-up game space. Conversion to Y-up render
//! space happens during `prepare_debug_draw_data()`.

use super::primitives::{self, DebugLineData};

/// A persistent debug line with a remaining lifetime.
#[derive(Clone)]
struct PersistentLine {
    line: DebugLineData,
    overlay: bool,
    remaining: f32,
}

/// Immediate-mode debug draw buffer. Accumulates lines per frame.
///
/// - `line()` / `line_overlay()` add one-frame lines (cleared each frame)
/// - `line_persistent()` adds lines that decay over `lifetime` seconds
/// - `update(dt)` ticks persistent lifetimes; `drain()` returns all lines for rendering
pub struct DebugDrawBuffer {
    depth_lines: Vec<DebugLineData>,
    overlay_lines: Vec<DebugLineData>,
    persistent: Vec<PersistentLine>,
}

impl DebugDrawBuffer {
    pub fn new() -> Self {
        Self {
            depth_lines: Vec::new(),
            overlay_lines: Vec::new(),
            persistent: Vec::new(),
        }
    }

    /// Add a depth-tested line (occluded by geometry).
    pub fn line(&mut self, start: [f32; 3], end: [f32; 3], color: [f32; 4]) {
        self.depth_lines.push(DebugLineData { start, end, color });
    }

    /// Add an overlay line (always visible, drawn on top).
    pub fn line_overlay(&mut self, start: [f32; 3], end: [f32; 3], color: [f32; 4]) {
        self.overlay_lines.push(DebugLineData { start, end, color });
    }

    /// Add a persistent depth-tested line with a lifetime in seconds.
    pub fn line_persistent(
        &mut self,
        start: [f32; 3],
        end: [f32; 3],
        color: [f32; 4],
        lifetime: f32,
    ) {
        self.persistent.push(PersistentLine {
            line: DebugLineData { start, end, color },
            overlay: false,
            remaining: lifetime,
        });
    }

    /// Add a depth-tested box wireframe (12 edges).
    pub fn box_wireframe(
        &mut self,
        center: [f32; 3],
        half_extents: [f32; 3],
        color: [f32; 4],
    ) {
        self.depth_lines
            .extend(primitives::box_lines(center, half_extents, color));
    }

    /// Add a depth-tested sphere wireframe (3 great circles).
    pub fn sphere_wireframe(&mut self, center: [f32; 3], radius: f32, color: [f32; 4]) {
        self.depth_lines
            .extend(primitives::sphere_lines(center, radius, color));
    }

    /// Add a depth-tested arrow.
    pub fn arrow(&mut self, start: [f32; 3], end: [f32; 3], color: [f32; 4]) {
        self.depth_lines
            .extend(primitives::arrow_lines(start, end, color));
    }

    /// Add a depth-tested capsule wireframe (Z-axis aligned).
    pub fn capsule_wireframe(
        &mut self,
        center: [f32; 3],
        half_height: f32,
        radius: f32,
        color: [f32; 4],
    ) {
        self.depth_lines
            .extend(primitives::capsule_lines(center, half_height, radius, color));
    }

    /// Add a depth-tested cross (3 axis-colored lines).
    pub fn cross(&mut self, center: [f32; 3], size: f32) {
        self.depth_lines.extend(primitives::cross_lines(center, size));
    }

    /// Tick persistent line lifetimes and remove expired ones.
    pub fn update(&mut self, dt: f32) {
        self.persistent.retain_mut(|p| {
            p.remaining -= dt;
            p.remaining > 0.0
        });
    }

    /// Drain all lines for this frame. Returns (depth_lines, overlay_lines).
    /// Clears per-frame lines but keeps persistent ones.
    pub fn drain(&mut self) -> (Vec<DebugLineData>, Vec<DebugLineData>) {
        // Start with per-frame lines
        let mut depth = std::mem::take(&mut self.depth_lines);
        let mut overlay = std::mem::take(&mut self.overlay_lines);

        // Add persistent lines (copied, since they persist across frames)
        for p in &self.persistent {
            if p.overlay {
                overlay.push(p.line);
            } else {
                depth.push(p.line);
            }
        }

        (depth, overlay)
    }
}

impl Default for DebugDrawBuffer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drain_clears_per_frame_lines() {
        let mut buf = DebugDrawBuffer::new();
        buf.line([0.0; 3], [1.0; 3], [1.0; 4]);
        buf.line_overlay([0.0; 3], [1.0; 3], [1.0; 4]);

        let (depth, overlay) = buf.drain();
        assert_eq!(depth.len(), 1);
        assert_eq!(overlay.len(), 1);

        // Second drain should be empty
        let (depth2, overlay2) = buf.drain();
        assert!(depth2.is_empty());
        assert!(overlay2.is_empty());
    }

    #[test]
    fn persistent_lines_survive_drain() {
        let mut buf = DebugDrawBuffer::new();
        buf.line_persistent([0.0; 3], [1.0; 3], [1.0; 4], 2.0);

        let (depth, _) = buf.drain();
        assert_eq!(depth.len(), 1);

        // Should still be there on next drain
        let (depth2, _) = buf.drain();
        assert_eq!(depth2.len(), 1);
    }

    #[test]
    fn persistent_lines_decay() {
        let mut buf = DebugDrawBuffer::new();
        buf.line_persistent([0.0; 3], [1.0; 3], [1.0; 4], 1.0);

        buf.update(0.5);
        let (depth, _) = buf.drain();
        assert_eq!(depth.len(), 1, "Should survive with 0.5s remaining");

        buf.update(0.6);
        let (depth2, _) = buf.drain();
        assert!(depth2.is_empty(), "Should be removed after lifetime expires");
    }
}
