//! Pure functions that generate wireframe line segments for common shapes.
//!
//! All inputs and outputs are in Z-up game space. Coordinate conversion
//! to Y-up render space happens later during vertex buffer creation.

/// A single debug line in Z-up game space.
#[derive(Clone, Copy, Debug)]
pub struct DebugLineData {
    pub start: [f32; 3],
    pub end: [f32; 3],
    pub color: [f32; 4],
}

/// Generate 12 edges of an axis-aligned box.
pub fn box_lines(center: [f32; 3], half_extents: [f32; 3], color: [f32; 4]) -> Vec<DebugLineData> {
    let [cx, cy, cz] = center;
    let [hx, hy, hz] = half_extents;

    // 8 vertices
    let verts = [
        [cx - hx, cy - hy, cz - hz],
        [cx + hx, cy - hy, cz - hz],
        [cx + hx, cy + hy, cz - hz],
        [cx - hx, cy + hy, cz - hz],
        [cx - hx, cy - hy, cz + hz],
        [cx + hx, cy - hy, cz + hz],
        [cx + hx, cy + hy, cz + hz],
        [cx - hx, cy + hy, cz + hz],
    ];

    // 12 edges: bottom, top, vertical
    let edges: [(usize, usize); 12] = [
        (0, 1),
        (1, 2),
        (2, 3),
        (3, 0),
        (4, 5),
        (5, 6),
        (6, 7),
        (7, 4),
        (0, 4),
        (1, 5),
        (2, 6),
        (3, 7),
    ];

    edges
        .iter()
        .map(|&(a, b)| DebugLineData {
            start: verts[a],
            end: verts[b],
            color,
        })
        .collect()
}

/// Generate 3 great circles (XY, XZ, YZ planes) for a sphere wireframe.
pub fn sphere_lines(center: [f32; 3], radius: f32, color: [f32; 4]) -> Vec<DebugLineData> {
    let segments = 32;
    let mut lines = Vec::with_capacity(segments * 3);
    let [cx, cy, cz] = center;

    // XY circle (horizontal in Z-up)
    for i in 0..segments {
        let a1 = (i as f32 / segments as f32) * std::f32::consts::TAU;
        let a2 = ((i + 1) as f32 / segments as f32) * std::f32::consts::TAU;
        lines.push(DebugLineData {
            start: [cx + radius * a1.cos(), cy + radius * a1.sin(), cz],
            end: [cx + radius * a2.cos(), cy + radius * a2.sin(), cz],
            color,
        });
    }

    // XZ circle (vertical, facing Y)
    for i in 0..segments {
        let a1 = (i as f32 / segments as f32) * std::f32::consts::TAU;
        let a2 = ((i + 1) as f32 / segments as f32) * std::f32::consts::TAU;
        lines.push(DebugLineData {
            start: [cx + radius * a1.cos(), cy, cz + radius * a1.sin()],
            end: [cx + radius * a2.cos(), cy, cz + radius * a2.sin()],
            color,
        });
    }

    // YZ circle (vertical, facing X)
    for i in 0..segments {
        let a1 = (i as f32 / segments as f32) * std::f32::consts::TAU;
        let a2 = ((i + 1) as f32 / segments as f32) * std::f32::consts::TAU;
        lines.push(DebugLineData {
            start: [cx, cy + radius * a1.cos(), cz + radius * a1.sin()],
            end: [cx, cy + radius * a2.cos(), cz + radius * a2.sin()],
            color,
        });
    }

    lines
}

/// Generate an arrow: a line from `start` to `end` plus two arrowhead lines.
pub fn arrow_lines(start: [f32; 3], end: [f32; 3], color: [f32; 4]) -> Vec<DebugLineData> {
    let dx = end[0] - start[0];
    let dy = end[1] - start[1];
    let dz = end[2] - start[2];
    let len = (dx * dx + dy * dy + dz * dz).sqrt();

    if len < 1e-6 {
        return vec![];
    }

    let head_size = len * 0.15;

    // Normalized direction
    let dir = [dx / len, dy / len, dz / len];

    // Find a perpendicular vector
    let perp = if dir[2].abs() < 0.9 {
        // Cross with Z-up
        let cross_len =
            (dir[1] * dir[1] + dir[0] * dir[0]).sqrt();
        if cross_len < 1e-6 {
            [1.0, 0.0, 0.0]
        } else {
            [-dir[1] / cross_len, dir[0] / cross_len, 0.0]
        }
    } else {
        // Cross with X
        let cross_len =
            (dir[2] * dir[2] + dir[1] * dir[1]).sqrt();
        if cross_len < 1e-6 {
            [0.0, 1.0, 0.0]
        } else {
            [0.0, -dir[2] / cross_len, dir[1] / cross_len]
        }
    };

    let head_base = [
        end[0] - dir[0] * head_size,
        end[1] - dir[1] * head_size,
        end[2] - dir[2] * head_size,
    ];

    let wing1 = [
        head_base[0] + perp[0] * head_size * 0.4,
        head_base[1] + perp[1] * head_size * 0.4,
        head_base[2] + perp[2] * head_size * 0.4,
    ];

    let wing2 = [
        head_base[0] - perp[0] * head_size * 0.4,
        head_base[1] - perp[1] * head_size * 0.4,
        head_base[2] - perp[2] * head_size * 0.4,
    ];

    vec![
        DebugLineData {
            start,
            end,
            color,
        },
        DebugLineData {
            start: end,
            end: wing1,
            color,
        },
        DebugLineData {
            start: end,
            end: wing2,
            color,
        },
    ]
}

/// Generate a circle in a given plane. `normal_axis` is 0=X, 1=Y, 2=Z (Z-up).
pub fn circle_lines(
    center: [f32; 3],
    radius: f32,
    normal_axis: usize,
    color: [f32; 4],
) -> Vec<DebugLineData> {
    let segments = 32;
    let mut lines = Vec::with_capacity(segments);
    let [cx, cy, cz] = center;

    for i in 0..segments {
        let a1 = (i as f32 / segments as f32) * std::f32::consts::TAU;
        let a2 = ((i + 1) as f32 / segments as f32) * std::f32::consts::TAU;

        let (s1, e1) = match normal_axis {
            0 => {
                // Normal along X: circle in YZ plane
                (
                    [cx, cy + radius * a1.cos(), cz + radius * a1.sin()],
                    [cx, cy + radius * a2.cos(), cz + radius * a2.sin()],
                )
            }
            1 => {
                // Normal along Y: circle in XZ plane
                (
                    [cx + radius * a1.cos(), cy, cz + radius * a1.sin()],
                    [cx + radius * a2.cos(), cy, cz + radius * a2.sin()],
                )
            }
            _ => {
                // Normal along Z: circle in XY plane
                (
                    [cx + radius * a1.cos(), cy + radius * a1.sin(), cz],
                    [cx + radius * a2.cos(), cy + radius * a2.sin(), cz],
                )
            }
        };

        lines.push(DebugLineData {
            start: s1,
            end: e1,
            color,
        });
    }

    lines
}

/// Generate a capsule wireframe (Z-axis aligned in Z-up game space).
///
/// Two hemisphere caps at top/bottom + 4 vertical connecting lines + 2 ring circles.
pub fn capsule_lines(
    center: [f32; 3],
    half_height: f32,
    radius: f32,
    color: [f32; 4],
) -> Vec<DebugLineData> {
    let segments = 32;
    let half_segments = segments / 2;
    let [cx, cy, cz] = center;

    // 2 circles (at top/bottom of cylinder) + 2 half-circle arcs per cap (XZ, YZ) + 4 vertical lines
    let mut lines = Vec::with_capacity(segments * 2 + half_segments * 4 + 4);

    // Middle ring at top of cylinder portion
    for i in 0..segments {
        let a1 = (i as f32 / segments as f32) * std::f32::consts::TAU;
        let a2 = ((i + 1) as f32 / segments as f32) * std::f32::consts::TAU;
        lines.push(DebugLineData {
            start: [cx + radius * a1.cos(), cy + radius * a1.sin(), cz + half_height],
            end: [cx + radius * a2.cos(), cy + radius * a2.sin(), cz + half_height],
            color,
        });
    }

    // Middle ring at bottom of cylinder portion
    for i in 0..segments {
        let a1 = (i as f32 / segments as f32) * std::f32::consts::TAU;
        let a2 = ((i + 1) as f32 / segments as f32) * std::f32::consts::TAU;
        lines.push(DebugLineData {
            start: [cx + radius * a1.cos(), cy + radius * a1.sin(), cz - half_height],
            end: [cx + radius * a2.cos(), cy + radius * a2.sin(), cz - half_height],
            color,
        });
    }

    // Top hemisphere arcs (upper half of circle in XZ and YZ planes)
    for i in 0..half_segments {
        let a1 = (i as f32 / segments as f32) * std::f32::consts::TAU;
        let a2 = ((i + 1) as f32 / segments as f32) * std::f32::consts::TAU;
        // XZ arc
        lines.push(DebugLineData {
            start: [cx + radius * a1.cos(), cy, cz + half_height + radius * a1.sin()],
            end: [cx + radius * a2.cos(), cy, cz + half_height + radius * a2.sin()],
            color,
        });
        // YZ arc
        lines.push(DebugLineData {
            start: [cx, cy + radius * a1.cos(), cz + half_height + radius * a1.sin()],
            end: [cx, cy + radius * a2.cos(), cz + half_height + radius * a2.sin()],
            color,
        });
    }

    // Bottom hemisphere arcs (lower half of circle in XZ and YZ planes)
    for i in half_segments..segments {
        let a1 = (i as f32 / segments as f32) * std::f32::consts::TAU;
        let a2 = ((i + 1) as f32 / segments as f32) * std::f32::consts::TAU;
        // XZ arc
        lines.push(DebugLineData {
            start: [cx + radius * a1.cos(), cy, cz - half_height + radius * a1.sin()],
            end: [cx + radius * a2.cos(), cy, cz - half_height + radius * a2.sin()],
            color,
        });
        // YZ arc
        lines.push(DebugLineData {
            start: [cx, cy + radius * a1.cos(), cz - half_height + radius * a1.sin()],
            end: [cx, cy + radius * a2.cos(), cz - half_height + radius * a2.sin()],
            color,
        });
    }

    // 4 vertical lines connecting the caps
    lines.push(DebugLineData {
        start: [cx + radius, cy, cz + half_height],
        end: [cx + radius, cy, cz - half_height],
        color,
    });
    lines.push(DebugLineData {
        start: [cx - radius, cy, cz + half_height],
        end: [cx - radius, cy, cz - half_height],
        color,
    });
    lines.push(DebugLineData {
        start: [cx, cy + radius, cz + half_height],
        end: [cx, cy + radius, cz - half_height],
        color,
    });
    lines.push(DebugLineData {
        start: [cx, cy - radius, cz + half_height],
        end: [cx, cy - radius, cz - half_height],
        color,
    });

    lines
}

/// Generate 3 axis-colored lines forming a cross at `center`.
pub fn cross_lines(center: [f32; 3], size: f32) -> Vec<DebugLineData> {
    let [cx, cy, cz] = center;
    let half = size * 0.5;

    vec![
        // X axis (red)
        DebugLineData {
            start: [cx - half, cy, cz],
            end: [cx + half, cy, cz],
            color: [1.0, 0.0, 0.0, 1.0],
        },
        // Y axis (green)
        DebugLineData {
            start: [cx, cy - half, cz],
            end: [cx, cy + half, cz],
            color: [0.0, 1.0, 0.0, 1.0],
        },
        // Z axis (blue)
        DebugLineData {
            start: [cx, cy, cz - half],
            end: [cx, cy, cz + half],
            color: [0.0, 0.0, 1.0, 1.0],
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn box_lines_count() {
        let lines = box_lines([0.0, 0.0, 0.0], [1.0, 1.0, 1.0], [1.0; 4]);
        assert_eq!(lines.len(), 12, "A box wireframe has 12 edges");
    }

    #[test]
    fn sphere_lines_count() {
        let lines = sphere_lines([0.0, 0.0, 0.0], 1.0, [1.0; 4]);
        // 3 circles * 32 segments = 96 lines
        assert_eq!(lines.len(), 96, "Sphere wireframe has 3*32 = 96 line segments");
    }

    #[test]
    fn arrow_lines_count() {
        let lines = arrow_lines([0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0; 4]);
        assert_eq!(lines.len(), 3, "Arrow has shaft + 2 wing lines");
    }

    #[test]
    fn arrow_zero_length() {
        let lines = arrow_lines([1.0, 2.0, 3.0], [1.0, 2.0, 3.0], [1.0; 4]);
        assert!(lines.is_empty(), "Zero-length arrow produces no lines");
    }

    #[test]
    fn circle_lines_count() {
        let lines = circle_lines([0.0, 0.0, 0.0], 1.0, 2, [1.0; 4]);
        assert_eq!(lines.len(), 32, "Circle has 32 segments");
    }

    #[test]
    fn circle_closure() {
        let lines = circle_lines([0.0, 0.0, 0.0], 1.0, 2, [1.0; 4]);
        let first_start = lines[0].start;
        let last_end = lines.last().unwrap().end;
        let dx = first_start[0] - last_end[0];
        let dy = first_start[1] - last_end[1];
        let dz = first_start[2] - last_end[2];
        let dist = (dx * dx + dy * dy + dz * dz).sqrt();
        assert!(dist < 0.01, "Circle should close: gap = {}", dist);
    }

    #[test]
    fn cross_lines_count() {
        let lines = cross_lines([0.0, 0.0, 0.0], 1.0);
        assert_eq!(lines.len(), 3, "Cross has 3 axis lines");
    }

    #[test]
    fn cross_lines_colors() {
        let lines = cross_lines([0.0, 0.0, 0.0], 1.0);
        // X=red, Y=green, Z=blue
        assert_eq!(lines[0].color, [1.0, 0.0, 0.0, 1.0], "X axis is red");
        assert_eq!(lines[1].color, [0.0, 1.0, 0.0, 1.0], "Y axis is green");
        assert_eq!(lines[2].color, [0.0, 0.0, 1.0, 1.0], "Z axis is blue");
    }

    #[test]
    fn capsule_lines_count() {
        let lines = capsule_lines([0.0, 0.0, 0.0], 0.5, 0.25, [1.0; 4]);
        // 2 rings (32 each) + 2 arcs * 2 planes * 16 half-segments + 4 vertical = 64 + 64 + 4 = 132
        assert_eq!(lines.len(), 132, "Capsule wireframe has 132 segments");
    }

    #[test]
    fn box_symmetry() {
        let lines = box_lines([5.0, 3.0, 1.0], [2.0, 1.0, 0.5], [1.0; 4]);
        // All edges should start and end within the bounding box
        for line in &lines {
            for coord in [&line.start, &line.end] {
                assert!((coord[0] - 5.0).abs() <= 2.0 + 0.001);
                assert!((coord[1] - 3.0).abs() <= 1.0 + 0.001);
                assert!((coord[2] - 1.0).abs() <= 0.5 + 0.001);
            }
        }
    }
}
