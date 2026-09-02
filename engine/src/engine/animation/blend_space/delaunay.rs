//! Bowyer–Watson Delaunay triangulation for a handful of points.
//!
//! Pure geometry in `f64`: the caller hands over a point set that is already
//! canonically ordered, free of duplicates and not all collinear, and gets
//! back CCW triangles over the input indices. Blend spaces have tens of
//! samples at most, so the O(n²) insertion loop is the right size of tool.

type P = [f64; 2];

/// Twice the signed area of `abc`; positive when CCW.
pub fn orient(a: P, b: P, c: P) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

/// Strictly inside the circumcircle of the CCW triangle `abc`.
fn in_circumcircle(a: P, b: P, c: P, p: P) -> bool {
    let (ax, ay) = (a[0] - p[0], a[1] - p[1]);
    let (bx, by) = (b[0] - p[0], b[1] - p[1]);
    let (cx, cy) = (c[0] - p[0], c[1] - p[1]);
    let det = (ax * ax + ay * ay) * (bx * cy - cx * by) - (bx * bx + by * by) * (ax * cy - cx * ay)
        + (cx * cx + cy * cy) * (ax * by - bx * ay);
    det > 1e-12
}

/// CCW Delaunay triangles over `pts` (indices into `pts`). Deterministic for
/// a given point order; the caller sorts for order independence.
pub fn triangulate(pts: &[P]) -> Vec<[usize; 3]> {
    let n = pts.len();
    if n < 3 {
        return Vec::new();
    }
    // Super triangle far outside the bounding box so every hull edge survives.
    let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
    for p in pts {
        for k in 0..2 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    let span = (hi[0] - lo[0]).max(hi[1] - lo[1]).max(1e-6) * 1e4;
    let c = [(lo[0] + hi[0]) * 0.5, (lo[1] + hi[1]) * 0.5];
    let mut all: Vec<P> = pts.to_vec();
    all.push([c[0] - span, c[1] - span]);
    all.push([c[0] + span, c[1] - span]);
    all.push([c[0], c[1] + span]);
    let at = |i: usize| all[i];

    let mut tris: Vec<[usize; 3]> = vec![[n, n + 1, n + 2]];
    let mut bad: Vec<[usize; 3]> = Vec::new();
    let mut edges: Vec<(usize, usize)> = Vec::new();
    for p in 0..n {
        bad.clear();
        tris.retain(|t| {
            let hit = in_circumcircle(at(t[0]), at(t[1]), at(t[2]), at(p));
            if hit {
                bad.push(*t);
            }
            !hit
        });
        // Boundary of the cavity: edges used by exactly one bad triangle.
        edges.clear();
        for t in &bad {
            for k in 0..3 {
                edges.push((t[k], t[(k + 1) % 3]));
            }
        }
        for &(a, b) in &edges {
            let shared = edges.iter().any(|&(x, y)| x == b && y == a);
            if !shared {
                let t = if orient(at(a), at(b), at(p)) >= 0.0 { [a, b, p] } else { [b, a, p] };
                tris.push(t);
            }
        }
    }
    tris.retain(|t| t.iter().all(|&i| i < n));
    tris
}

/// The convex hull as a CCW loop of point indices, read off the triangle
/// boundary (edges without a reverse twin).
pub fn hull(tris: &[[usize; 3]]) -> Vec<usize> {
    let has_edge = |a: usize, b: usize| {
        tris.iter().any(|t| (0..3).any(|k| t[k] == a && t[(k + 1) % 3] == b))
    };
    let mut next: Vec<(usize, usize)> = Vec::new();
    for t in tris {
        for k in 0..3 {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            if !has_edge(b, a) {
                next.push((a, b));
            }
        }
    }
    let Some(start) = next.iter().map(|e| e.0).min() else {
        return Vec::new();
    };
    let mut out = vec![start];
    let mut cur = start;
    while let Some(&(_, b)) = next.iter().find(|e| e.0 == cur) {
        if b == start || out.len() > next.len() {
            break;
        }
        out.push(b);
        cur = b;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn square_gives_two_ccw_triangles_and_four_hull_points() {
        let pts = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]];
        let tris = triangulate(&pts);
        assert_eq!(tris.len(), 2);
        for t in &tris {
            assert!(orient(pts[t[0]], pts[t[1]], pts[t[2]]) > 0.0);
        }
        assert_eq!(hull(&tris).len(), 4);
    }

    #[test]
    fn interior_point_is_not_on_hull() {
        let pts = [[0.0, 0.0], [2.0, 0.0], [0.0, 2.0], [2.0, 2.0], [1.0, 1.0]];
        let tris = triangulate(&pts);
        assert_eq!(tris.len(), 4);
        let h = hull(&tris);
        assert_eq!(h.len(), 4);
        assert!(!h.contains(&4));
    }
}
