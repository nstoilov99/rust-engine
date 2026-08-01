//! Wire router — a port of the reference prototype's `polyPts` / `roundedPath`
//! / `path` / `samples` (Crusty Node Graph.dc.html, v1.2.2 router).
//!
//! Pure geometry: no `Ui`, no theme, no zoom. **All routing math happens in
//! graph space** and is transformed to screen by the caller — a screen-space
//! offset would change the route's shape as the user zooms and reads as a bug.
//!
//! The branch order is the prototype's, verbatim, because it is the part that
//! was actually verified against the acceptance tests:
//!
//! 0. near-horizontal shortcut — `|dy| < 2r` **and** `dx >= 6|dy|` (~9.46°)
//! 1. Subway forward (`dx >= 0`) — offsets compress continuously to a pure
//!    pin-to-pin 45° at `dx == |dy|`; when `|dy| > dx`, vertical-then-45°
//! 2. Manhattan forward (`dx >= 8`) — target-anchored, staggered by target
//!    pin row so parallel verticals never coincide
//! 3. the band `backward_lane_threshold < dx` — on-grid, with one residual
//!    straight stub for `|dx| < 24 && |dy| < 20`
//! 4. the backward lane — computed from **both nodes' exact bounding rects**
//!
//! Two deliberate deviations from the JS, both recorded rulings:
//! - the prototype's `hOf` over-reports node height by ~4-6px; this port takes
//!   exact rects from the caller instead;
//! - `HO`/`R` are `WirePrefs` fields here, and one `corner_radius` feeds both
//!   the `2r` straightness threshold and the actual corner rounding.

use crusty_gui::math::{Pos2, Rect};

use super::graph_prefs::{TurnAnchor, WirePrefs, WireStyle};

/// Manhattan's forward branch needs at least this much `dx` to be worth a
/// turn. Undocumented in the prose but load-bearing in the prototype: below
/// it, the band branch produces a cleaner route.
const MANHATTAN_MIN_DX: f32 = 8.0;
/// Floor of the Manhattan stagger cap. The cap `max(4, dx/2)` is what
/// collapses the stagger on short spans (see `bundle_offset`'s test).
const MIN_STAGGER_CAP: f32 = 4.0;
/// Clearance above/below the two nodes' rects when the backward lane is
/// computed from real geometry.
const LANE_MARGIN: f32 = 24.0;
/// Clearance used when node rects are unavailable (the live-drag ghost has no
/// target node yet), measured from the pin rows instead.
const LANE_FALLBACK: f32 = 34.0;
/// Vertical inflation applied to candidate node rects by the lane broadphase.
const LANE_PROBE_INFLATE: f32 = 6.0;
/// Straight-line subdivisions per polyline segment when sampling for
/// hit-tests and cuts (the prototype's value).
const SEGMENT_SUBDIVISIONS: usize = 4;
/// Cubic subdivisions when sampling a spline (17 points, as the prototype).
const SPLINE_SAMPLES: usize = 16;

/// What the router needs to know about the world beyond the two pins.
#[derive(Clone, Copy, Debug, Default)]
pub struct RouteMeta<'a> {
    /// Exact bounding rect of the source node, if it has one.
    pub src_rect: Option<Rect>,
    /// Exact bounding rect of the target node, if it has one. `None` during a
    /// live connection drag — the lane falls back to pin-relative clearance.
    pub dst_rect: Option<Rect>,
    /// Row index of the target pin — the Manhattan bundle stagger.
    pub target_pin_index: usize,
    /// Every node rect on the canvas, for the backward lane's broadphase.
    pub node_rects: &'a [Rect],
}

impl<'a> RouteMeta<'a> {
    /// The rect-less form used by the in-flight drag ghost.
    pub fn loose(node_rects: &'a [Rect]) -> Self {
        Self { src_rect: None, dst_rect: None, target_pin_index: 0, node_rects }
    }
}

/// The wire's **corner points** in graph space.
///
/// Manhattan/Subway return the branch polyline (2-6 points); Spline returns
/// just its endpoints, because a cubic's shape lives in its control points —
/// see [`spline_controls`]. Feed the result to [`round_corners`] to draw and
/// to [`sample`] to hit-test.
pub fn route(a: Pos2, b: Pos2, prefs: &WirePrefs, meta: &RouteMeta) -> Vec<Pos2> {
    if !prefs.style.is_orthogonal() {
        return vec![a, b];
    }

    let ho_base = prefs.offset();
    let r = prefs.corner_radius;
    let dx = b.x - a.x;
    let ady = (b.y - a.y).abs();
    let sgn = if b.y > a.y { 1.0 } else { -1.0 };
    let source_anchored = prefs.turn_anchor == TurnAnchor::Source;

    // 0. Near-horizontal shortcut — and only when it truly is one. Gating on
    //    dy alone let two side-by-side, slightly offset nodes (dx 25, dy 19 =
    //    37°) draw an arbitrary-angle line.
    if ady < 2.0 * r && dx >= 6.0 * ady {
        return vec![a, b];
    }

    // 1. Subway forward: one long horizontal run, one 45° diagonal covering
    //    the whole dy, one short run into the pin. `ho` compresses 16 -> 0 as
    //    the span tightens, so the handoff to the pure 45° never pops.
    if prefs.style == WireStyle::Subway && dx >= 0.0 {
        if dx >= ady {
            let ho = ho_base.min((dx - ady) * 0.5);
            return if source_anchored {
                vec![
                    a,
                    Pos2::new(a.x + ho, a.y),
                    Pos2::new(a.x + ho + ady, b.y),
                    b,
                ]
            } else {
                vec![
                    a,
                    Pos2::new(b.x - ho - ady, a.y),
                    Pos2::new(b.x - ho, b.y),
                    b,
                ]
            };
        }
        // |dy| > dx: no 45° covers the drop, so go vertical first. Still
        // on-grid — never an arbitrary-angle line.
        return if source_anchored {
            vec![a, Pos2::new(b.x, a.y + sgn * dx), b]
        } else {
            vec![a, Pos2::new(a.x, b.y - sgn * dx), b]
        };
    }

    // 2. Manhattan forward, target-anchored. All wires arriving at one node
    //    turn at the same x whatever their span — that is what makes a bundle
    //    parallel — offset per target pin row so they stay distinguishable.
    if prefs.style == WireStyle::Manhattan && dx >= MANHATTAN_MIN_DX {
        // The stagger wraps at `bundle_max`: without it a node with 20 input
        // rows pushes its high-index wires so far out that the `dx/2` cap
        // clamps them all to the same lane — the exact coincidence the
        // stagger exists to prevent. Wrapping reuses a lane instead, which is
        // what the spec's "above bundle_max, draw coincident" describes.
        let lanes = prefs.bundle_max.max(1) as usize;
        let bi = (meta.target_pin_index % lanes) as f32;
        let ho = (ho_base + bi * prefs.bundle_offset).min((dx * 0.5).max(MIN_STAGGER_CAP));
        return if source_anchored {
            vec![a, Pos2::new(a.x + ho, a.y), Pos2::new(a.x + ho, b.y), b]
        } else {
            vec![a, Pos2::new(b.x - ho, a.y), Pos2::new(b.x - ho, b.y), b]
        };
    }

    // 3. The band between "forward" and "backward". A bare line here leans up
    //    to 111° off true on a stacked pair, so it stays on-grid. No turn
    //    offset applies — there is no room for one, and both anchors agree.
    if dx > prefs.backward_lane_threshold {
        // The one residual: a stub between nearly-overlapping pins, worst
        // case 22° over <=18px. Straight is the right answer at that size.
        if ady < 2.0 * r {
            return vec![a, b];
        }
        if prefs.style == WireStyle::Subway {
            return vec![a, Pos2::new(a.x, b.y - sgn * dx.abs()), b];
        }
        return vec![a, Pos2::new(a.x, b.y), b];
    }

    // 4. Backward: a lane above or below *both* nodes. Pin-relative lanes land
    //    inside a tall node's own body and the wire runs under its own node,
    //    so this is the one place the router genuinely needs the geometry.
    let ho = ho_base;
    let (top, bot) = match (meta.src_rect, meta.dst_rect) {
        (Some(r1), Some(r2)) => (
            r1.min.y.min(r2.min.y) - LANE_MARGIN,
            r1.max.y.max(r2.max.y) + LANE_MARGIN,
        ),
        _ => (
            a.y.min(b.y) - LANE_FALLBACK,
            a.y.max(b.y) + LANE_FALLBACK,
        ),
    };
    let xa = (b.x - ho).min(a.x + ho);
    let xb = (a.x + ho).max(b.x - ho);
    let hits = |lane_y: f32| {
        meta.node_rects
            .iter()
            .filter(|r| {
                r.min.y - LANE_PROBE_INFLATE < lane_y
                    && r.max.y + LANE_PROBE_INFLATE > lane_y
                    && r.min.x < xb
                    && r.max.x > xa
            })
            .count()
    };
    let (ht, hb) = (hits(top), hits(bot));
    // Fewer intersected nodes wins; a tie goes to the side the target is on.
    let lane = if ht < hb {
        top
    } else if hb < ht {
        bot
    } else if b.y <= a.y {
        top
    } else {
        bot
    };
    vec![
        a,
        Pos2::new(a.x + ho, a.y),
        Pos2::new(a.x + ho, lane),
        Pos2::new(b.x - ho, lane),
        Pos2::new(b.x - ho, b.y),
        b,
    ]
}

/// The two cubic control points of a Spline wire. Tuned; the spec is explicit
/// that this tangent must not be retuned. Grown when the target is left of the
/// source, so a backward spline bows out instead of doubling back through its
/// own node.
pub fn spline_controls(a: Pos2, b: Pos2, curve: f32) -> (Pos2, Pos2) {
    let dx = b.x - a.x;
    let mut t = (dx.abs() * (0.35 + curve * 0.55)).clamp(34.0, 190.0);
    if dx < 0.0 {
        t = t.max(70.0 + dx.abs() * 0.35);
    }
    (Pos2::new(a.x + t, a.y), Pos2::new(b.x - t, b.y))
}

/// Drop consecutive duplicate points. The continuous-to-zero compression emits
/// zero-length segments at exact on-grid geometries (`dx == |dy|`), and a
/// corner arc on a zero-length segment divides by zero.
fn dedup(pts: &[Pos2]) -> Vec<Pos2> {
    let mut out: Vec<Pos2> = Vec::with_capacity(pts.len());
    for p in pts {
        match out.last() {
            Some(q) if (p.x - q.x).abs() <= 1e-6 && (p.y - q.y).abs() <= 1e-6 => {}
            _ => out.push(*p),
        }
    }
    out
}

/// Segments to tessellate one rounded corner into. Bigger radii get more.
fn corner_segments(rr: f32) -> usize {
    (rr * 0.6).clamp(2.0, 8.0) as usize
}

/// Round the corners of a routed polyline and return **one dense polyline**
/// ready for a single stroke.
///
/// The reference emits SVG `L`/`Q` pairs; crusty has no path primitive and
/// stroking each quadratic separately would leave visible seams at the joins,
/// so the quadratics are tessellated inline here and the whole wire is drawn
/// with one `Painter::polyline` call — same geometry, correct joins.
///
/// Per-corner radius clamp `min(radius, l1/2, l2/2)` so two adjacent corners
/// sharing a short segment can never overrun each other.
pub fn round_corners(pts: &[Pos2], radius: f32) -> Vec<Pos2> {
    let pts = dedup(pts);
    if pts.len() < 3 {
        return pts;
    }
    let mut out = Vec::with_capacity(pts.len() * 6);
    out.push(pts[0]);
    for i in 1..pts.len() - 1 {
        let (p, c, n) = (pts[i - 1], pts[i], pts[i + 1]);
        let l1 = (c - p).length();
        let l2 = (n - c).length();
        if l1 <= f32::EPSILON || l2 <= f32::EPSILON {
            out.push(c);
            continue;
        }
        let rr = radius.min(l1 * 0.5).min(l2 * 0.5);
        let start = c - (c - p) / l1 * rr;
        let end = c + (n - c) / l2 * rr;
        out.push(start);
        let segs = corner_segments(rr);
        for k in 1..=segs {
            let t = k as f32 / segs as f32;
            out.push(quadratic(start, c, end, t));
        }
    }
    out.push(pts[pts.len() - 1]);
    out
}

#[inline]
fn quadratic(a: Pos2, c: Pos2, b: Pos2, t: f32) -> Pos2 {
    let m = 1.0 - t;
    Pos2::new(
        m * m * a.x + 2.0 * m * t * c.x + t * t * b.x,
        m * m * a.y + 2.0 * m * t * c.y + t * t * b.y,
    )
}

#[inline]
fn cubic(a: Pos2, c1: Pos2, c2: Pos2, b: Pos2, t: f32) -> Pos2 {
    let m = 1.0 - t;
    Pos2::new(
        m * m * m * a.x + 3.0 * m * m * t * c1.x + 3.0 * m * t * t * c2.x + t * t * t * b.x,
        m * m * m * a.y + 3.0 * m * m * t * c1.y + 3.0 * m * t * t * c2.y + t * t * t * b.y,
    )
}

/// The polyline a wire is hit-tested and cut against, in graph space — dense
/// enough that per-segment distance is a faithful stand-in for the drawn
/// shape, for every style.
pub fn sample(a: Pos2, b: Pos2, prefs: &WirePrefs, meta: &RouteMeta) -> Vec<Pos2> {
    if !prefs.style.is_orthogonal() {
        let (c1, c2) = spline_controls(a, b, prefs.curve);
        return (0..=SPLINE_SAMPLES)
            .map(|i| cubic(a, c1, c2, b, i as f32 / SPLINE_SAMPLES as f32))
            .collect();
    }
    let pts = route(a, b, prefs, meta);
    subdivide(&pts, SEGMENT_SUBDIVISIONS)
}

/// Split each segment of `pts` into `n` equal parts.
fn subdivide(pts: &[Pos2], n: usize) -> Vec<Pos2> {
    if pts.is_empty() {
        return Vec::new();
    }
    let mut out = vec![pts[0]];
    for w in pts.windows(2) {
        let (p, q) = (w[0], w[1]);
        for k in 1..=n {
            let t = k as f32 / n as f32;
            out.push(Pos2::new(p.x + (q.x - p.x) * t, p.y + (q.y - p.y) * t));
        }
    }
    out
}

/// Do segments `a`-`b` and `c`-`d` cross? Exact parametric test, ported from
/// the prototype's `segHit` — the slash-cut must test every segment, not
/// bounding boxes, or a cut "through" an L-shaped wire misses the wire.
///
/// Parallel and collinear segments report `false`: a cut that runs exactly
/// along a wire is not a crossing, and treating it as one makes the gesture
/// fire on near-misses.
pub fn segments_intersect(a: Pos2, b: Pos2, c: Pos2, d: Pos2) -> bool {
    let den = (b.x - a.x) * (d.y - c.y) - (b.y - a.y) * (d.x - c.x);
    if den.abs() <= f32::EPSILON {
        return false;
    }
    let t = ((c.x - a.x) * (d.y - c.y) - (c.y - a.y) * (d.x - c.x)) / den;
    let u = ((c.x - a.x) * (b.y - a.y) - (c.y - a.y) * (b.x - a.x)) / den;
    (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u)
}

/// Does any segment of `path` cross any segment of `poly`?
pub fn path_crosses_polyline(path: &[Pos2], poly: &[Pos2]) -> bool {
    if path.len() < 2 || poly.len() < 2 {
        return false;
    }
    path.windows(2)
        .any(|p| poly.windows(2).any(|q| segments_intersect(p[0], p[1], q[0], q[1])))
}

/// Shortest distance from `p` to the segment `a`-`b`.
pub fn point_segment_distance(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let len_sq = ab.x * ab.x + ab.y * ab.y;
    if len_sq <= f32::EPSILON {
        return (p - a).length();
    }
    let ap = p - a;
    let t = ((ap.x * ab.x + ap.y * ab.y) / len_sq).clamp(0.0, 1.0);
    (p - (a + ab * t)).length()
}

/// Shortest distance from `p` to a polyline. `f32::MAX` for a degenerate one.
pub fn point_polyline_distance(p: Pos2, pts: &[Pos2]) -> f32 {
    if pts.len() < 2 {
        return pts.first().map_or(f32::MAX, |q| (p - *q).length());
    }
    pts.windows(2)
        .map(|w| point_segment_distance(p, w[0], w[1]))
        .fold(f32::MAX, f32::min)
}

/// Axis-aligned bounds of a polyline, for viewport culling.
pub fn polyline_bounds(pts: &[Pos2]) -> Option<Rect> {
    let first = *pts.first()?;
    let mut min = first;
    let mut max = first;
    for p in pts {
        min = Pos2::new(min.x.min(p.x), min.y.min(p.y));
        max = Pos2::new(max.x.max(p.x), max.y.max(p.y));
    }
    Some(Rect::from_min_max(min, max))
}

/// Bounds of a Spline wire including its control hull, so culling a curve
/// never clips a bow that reaches outside the endpoints' box.
pub fn wire_bounds(a: Pos2, b: Pos2, prefs: &WirePrefs, meta: &RouteMeta) -> Option<Rect> {
    if !prefs.style.is_orthogonal() {
        let (c1, c2) = spline_controls(a, b, prefs.curve);
        return polyline_bounds(&[a, c1, c2, b]);
    }
    polyline_bounds(&route(a, b, prefs, meta))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crusty_gui::math::Vec2;

    // ---------------------------------------------------------------
    // Fixtures
    // ---------------------------------------------------------------

    const NODE_W: f32 = 168.0;
    const NODE_H: f32 = 100.0;

    fn prefs(style: WireStyle) -> WirePrefs {
        WirePrefs { style, ..WirePrefs::default() }
    }

    /// The standard sweep pair: `a` is the source node's *output* pin, sitting
    /// on its right border; `b` is the target node's *input* pin, on its left
    /// border. Both nodes are 168x100 with the pin at mid-height.
    fn pair(dx: f32, dy: f32) -> (Pos2, Pos2, Rect, Rect) {
        let a = Pos2::new(0.0, 0.0);
        let b = Pos2::new(dx, dy);
        let src = Rect::from_min_max(
            Pos2::new(-NODE_W, -NODE_H * 0.5),
            Pos2::new(0.0, NODE_H * 0.5),
        );
        let dst = Rect::from_min_max(
            Pos2::new(dx, dy - NODE_H * 0.5),
            Pos2::new(dx + NODE_W, dy + NODE_H * 0.5),
        );
        (a, b, src, dst)
    }

    fn meta<'a>(src: &Rect, dst: &Rect, rects: &'a [Rect], bi: usize) -> RouteMeta<'a> {
        RouteMeta {
            src_rect: Some(*src),
            dst_rect: Some(*dst),
            target_pin_index: bi,
            node_rects: rects,
        }
    }

    /// Angle of a segment in degrees, normalized to [0, 360).
    fn angle_deg(p: Pos2, q: Pos2) -> f32 {
        let d = (q.y - p.y).atan2(q.x - p.x).to_degrees();
        if d < 0.0 {
            d + 360.0
        } else {
            d
        }
    }

    /// Distance from the nearest multiple of 45°.
    fn off_grid(p: Pos2, q: Pos2) -> f32 {
        let a = angle_deg(p, q);
        let nearest = (a / 45.0).round() * 45.0;
        (a - nearest).abs()
    }

    /// Is this (dx, dy) one of the two documented exceptions to on-grid?
    fn is_documented_exception(dx: f32, dy: f32, r: f32) -> bool {
        let ady = dy.abs();
        // (a) the near-horizontal shortcut, ~9.46° cap
        if ady < 2.0 * r && dx >= 6.0 * ady {
            return true;
        }
        // (b) the residual stub between nearly-overlapping pins
        dx.abs() < 24.0 && ady < 20.0
    }

    fn resample(pts: &[Pos2], n: usize) -> Vec<Pos2> {
        // Arc-length resampling, so two routes with different point counts
        // are still comparable shape-to-shape.
        let mut cum = vec![0.0f32];
        for w in pts.windows(2) {
            cum.push(cum.last().unwrap() + (w[1] - w[0]).length());
        }
        let total = *cum.last().unwrap();
        if total <= f32::EPSILON {
            return vec![pts[0]; n + 1];
        }
        (0..=n)
            .map(|i| {
                let want = total * i as f32 / n as f32;
                let mut seg = 1;
                while seg < cum.len() - 1 && cum[seg] < want {
                    seg += 1;
                }
                let (c0, c1) = (cum[seg - 1], cum[seg]);
                let t = if (c1 - c0).abs() <= f32::EPSILON {
                    0.0
                } else {
                    (want - c0) / (c1 - c0)
                };
                let (p, q) = (pts[seg - 1], pts[seg]);
                Pos2::new(p.x + (q.x - p.x) * t, p.y + (q.y - p.y) * t)
            })
            .collect()
    }

    // ---------------------------------------------------------------
    // Test 11 — the exhaustive sweep. Written first, per the brief.
    // ---------------------------------------------------------------

    /// Every segment of every orthogonal route across
    /// `dx ∈ [-40, 400] × dy ∈ [-200, 200]` sits within 0.5° of a 45°
    /// multiple, with exactly two documented exceptions: the near-horizontal
    /// shortcut and the residual stub. This is the test the whole branch
    /// structure exists to pass.
    #[test]
    fn sweep_every_segment_is_on_grid() {
        for style in [WireStyle::Manhattan, WireStyle::Subway] {
            let p = prefs(style);
            let mut checked = 0usize;
            let mut exceptions = 0usize;
            for dxi in -40..=400 {
                for dyi in -200..=200 {
                    let (dx, dy) = (dxi as f32, dyi as f32);
                    let (a, b, src, dst) = pair(dx, dy);
                    let rects = [src, dst];
                    let pts = route(a, b, &p, &meta(&src, &dst, &rects, 0));
                    assert!(pts.len() >= 2, "{style:?} dx={dx} dy={dy}: empty route");
                    assert!(
                        pts.iter().all(|q| q.x.is_finite() && q.y.is_finite()),
                        "{style:?} dx={dx} dy={dy}: non-finite point"
                    );
                    assert_eq!(pts[0], a, "{style:?}: route must start on the source pin");
                    assert_eq!(
                        *pts.last().unwrap(),
                        b,
                        "{style:?}: route must end on the target pin"
                    );

                    if is_documented_exception(dx, dy, p.corner_radius) {
                        exceptions += 1;
                        continue;
                    }
                    for w in pts.windows(2) {
                        if (w[1] - w[0]).length() <= 1e-4 {
                            continue; // zero-length, removed before drawing
                        }
                        let off = off_grid(w[0], w[1]);
                        assert!(
                            off <= 0.5,
                            "{style:?} dx={dx} dy={dy}: segment {:?}->{:?} is {off:.3}° \
                             off a 45° multiple",
                            w[0],
                            w[1]
                        );
                    }
                    checked += 1;
                }
            }
            assert!(checked > 100_000, "{style:?}: sweep did not run ({checked})");
            assert!(exceptions > 0, "{style:?}: the exception bands never fired");
        }
    }

    // ---------------------------------------------------------------
    // Acceptance tests 1-10
    // ---------------------------------------------------------------

    /// 1. Two wires arriving at the *same* pin row of one node from sources at
    ///    different distances turn at the same x — the turn belongs to the
    ///    node, not the span.
    #[test]
    fn t1_parallel_arrivals_turn_at_the_same_x() {
        for style in [WireStyle::Manhattan, WireStyle::Subway] {
            let p = prefs(style);
            let b = Pos2::new(600.0, 0.0);
            // Same target, same row, wildly different source distances.
            let turn_xs: Vec<f32> = [100.0f32, 300.0, 500.0]
                .iter()
                .map(|sx| {
                    let a = Pos2::new(*sx, -60.0);
                    let src = Rect::from_min_max(
                        Pos2::new(sx - NODE_W, -110.0),
                        Pos2::new(*sx, -10.0),
                    );
                    let dst = Rect::from_min_max(
                        Pos2::new(600.0, -50.0),
                        Pos2::new(600.0 + NODE_W, 50.0),
                    );
                    let rects = [src, dst];
                    let pts = route(a, b, &p, &meta(&src, &dst, &rects, 0));
                    // The last turn before the pin.
                    pts[pts.len() - 2].x
                })
                .collect();
            for x in &turn_xs {
                assert!(
                    (x - turn_xs[0]).abs() < 1e-3,
                    "{style:?}: turn x varies with span: {turn_xs:?}"
                );
            }
        }
    }

    /// 2. A long span is one long horizontal run plus a short angle at its
    ///    destination — never a diagonal marooned mid-canvas.
    #[test]
    fn t2_long_span_turns_only_near_the_target() {
        for style in [WireStyle::Manhattan, WireStyle::Subway] {
            let p = prefs(style);
            let (a, b, src, dst) = pair(900.0, 66.0);
            let rects = [src, dst];
            let pts = route(a, b, &p, &meta(&src, &dst, &rects, 0));
            // Longest segment is the horizontal run out of the source.
            let longest = pts
                .windows(2)
                .max_by(|x, y| {
                    (x[1] - x[0])
                        .length()
                        .partial_cmp(&(y[1] - y[0]).length())
                        .unwrap()
                })
                .unwrap();
            assert!(
                (longest[1].y - longest[0].y).abs() < 1e-3,
                "{style:?}: the long run is not horizontal"
            );
            assert!((longest[0].y - a.y).abs() < 1e-3, "{style:?}: run left the source row");
            // Everything that is not that run happens within
            // horizontal_offset + |dy| of the target.
            let near = p.horizontal_offset + (b.y - a.y).abs() + 1.0;
            assert!(
                pts[1].x >= b.x - near,
                "{style:?}: first turn at {} is not near the target {}",
                pts[1].x,
                b.x
            );
        }
    }

    /// 3. One row apart (22px) at a 60px span: a clean 45° diagonal, every
    ///    segment on-grid, offsets compressed to fit.
    #[test]
    fn t3_one_row_apart_at_60px_is_a_clean_45() {
        let p = prefs(WireStyle::Subway);
        let (a, b, src, dst) = pair(60.0, 22.0);
        let rects = [src, dst];
        let pts = route(a, b, &p, &meta(&src, &dst, &rects, 0));
        assert_eq!(pts.len(), 4);
        // horizontal, 45°, horizontal
        assert!((pts[1].y - pts[0].y).abs() < 1e-3);
        assert!(((pts[2].x - pts[1].x) - (pts[2].y - pts[1].y)).abs() < 1e-3);
        assert!((pts[3].y - pts[2].y).abs() < 1e-3);
        for w in pts.windows(2) {
            assert!(off_grid(w[0], w[1]) <= 0.5);
        }
        // Both offsets fit: the diagonal spans exactly |dy|.
        assert!(((pts[2].x - pts[1].x).abs() - 22.0).abs() < 1e-3);
    }

    /// 4. `|dy| > |dx|`, including the leftward band `-24 < dx < 0`, routes
    ///    on-grid — vertical-then-45° (Subway) or vertical-then-horizontal
    ///    (Manhattan), never an arbitrary-angle kink.
    #[test]
    fn t4_tall_spans_and_the_leftward_band_stay_on_grid() {
        for style in [WireStyle::Manhattan, WireStyle::Subway] {
            let p = prefs(style);
            for (dx, dy) in [
                (30.0f32, 200.0f32),
                (5.0, 120.0),
                (0.0, 90.0),
                (-10.0, 90.0),
                (-23.0, -140.0),
            ] {
                let (a, b, src, dst) = pair(dx, dy);
                let rects = [src, dst];
                let pts = route(a, b, &p, &meta(&src, &dst, &rects, 0));
                // Subway drops vertically then cuts 45°; Manhattan's forward
                // branch still fires at dx >= 8 (H/V/H, 4 points). Both are
                // on-grid, which is what this test is actually about.
                let expected = if style == WireStyle::Manhattan && dx >= MANHATTAN_MIN_DX {
                    4
                } else {
                    3
                };
                assert_eq!(
                    pts.len(),
                    expected,
                    "{style:?} ({dx},{dy}): unexpected point count"
                );
                for w in pts.windows(2) {
                    if (w[1] - w[0]).length() <= 1e-4 {
                        continue;
                    }
                    assert!(
                        off_grid(w[0], w[1]) <= 0.5,
                        "{style:?} ({dx},{dy}) went off-grid"
                    );
                }
            }
        }
    }

    /// 5. Degradation is continuous: sweeping `dx` down through the offset
    ///    compression never pops the shape. `dy = 60` keeps the whole sweep
    ///    clear of the near-horizontal shortcut (which needs `|dy| < 20`), so
    ///    this measures the compression handoff and nothing else.
    #[test]
    fn t5_compression_is_continuous_with_no_shape_pop() {
        for style in [WireStyle::Manhattan, WireStyle::Subway] {
            let p = prefs(style);
            let mut worst = 0.0f32;
            let mut worst_at = 0.0f32;
            let mut prev: Option<Vec<Pos2>> = None;
            for dxi in (0..=120).rev() {
                let dx = dxi as f32;
                let (a, b, src, dst) = pair(dx, 60.0);
                let rects = [src, dst];
                let pts = route(a, b, &p, &meta(&src, &dst, &rects, 0));
                let now = resample(&pts, 32);
                if let Some(before) = &prev {
                    // Both are anchored at b, which moves 1px per step, so the
                    // shape delta is measured relative to that.
                    let delta = before
                        .iter()
                        .zip(now.iter())
                        .map(|(u, v)| (*u - *v).length())
                        .fold(0.0f32, f32::max);
                    if delta > worst {
                        worst = delta;
                        worst_at = dx;
                    }
                }
                prev = Some(now);
            }
            assert!(
                worst < 6.0,
                "{style:?}: shape popped by {worst:.2}px at dx={worst_at} \
                 (1px of that is the endpoint itself moving)"
            );
        }
    }

    /// 6. Near-horizontal is a straight two-point line, with no micro-kink.
    #[test]
    fn t6_near_horizontal_is_straight() {
        for style in [WireStyle::Manhattan, WireStyle::Subway] {
            let p = prefs(style);
            for (dx, dy) in [(400.0f32, 4.0f32), (200.0, -10.0), (120.0, 19.0)] {
                let (a, b, src, dst) = pair(dx, dy);
                let rects = [src, dst];
                let pts = route(a, b, &p, &meta(&src, &dst, &rects, 0));
                assert_eq!(pts, vec![a, b], "{style:?} ({dx},{dy}) is not straight");
            }
            // …and the cap holds: dx = 25, dy = 19 is 37°, NOT a shortcut.
            let (a, b, src, dst) = pair(25.0, 19.0);
            let rects = [src, dst];
            let pts = route(a, b, &p, &meta(&src, &dst, &rects, 0));
            assert_ne!(pts, vec![a, b], "{style:?}: 37° took the near-horizontal path");
        }
    }

    /// 7. A backward wire takes a lane that clears *both* node rects — never
    ///    through the source's own body.
    #[test]
    fn t7_backward_lane_clears_both_nodes() {
        for style in [WireStyle::Manhattan, WireStyle::Subway] {
            let p = prefs(style);
            for dy in [-120.0f32, 0.0, 120.0] {
                let (a, b, src, dst) = pair(-400.0, dy);
                let rects = [src, dst];
                let pts = route(a, b, &p, &meta(&src, &dst, &rects, 0));
                assert_eq!(pts.len(), 6, "{style:?}: backward route is 6 points");
                let lane = pts[2].y;
                assert!((pts[3].y - lane).abs() < 1e-3, "the lane is not level");
                for r in &rects {
                    assert!(
                        lane < r.min.y || lane > r.max.y,
                        "{style:?} dy={dy}: lane {lane} runs through a node rect {r:?}"
                    );
                }
                // Clearance is the documented 24px, from exact rects.
                let clear = rects
                    .iter()
                    .map(|r| (lane - r.min.y).abs().min((lane - r.max.y).abs()))
                    .fold(f32::MAX, f32::min);
                assert!(
                    (clear - LANE_MARGIN).abs() < 1e-3,
                    "{style:?} dy={dy}: clearance {clear} != {LANE_MARGIN}"
                );
            }
        }
    }

    /// 7b. With no node rects (the live-drag ghost) the lane falls back to
    ///     pin-relative clearance instead of producing a degenerate route.
    #[test]
    fn t7b_backward_lane_falls_back_without_rects() {
        let p = prefs(WireStyle::Subway);
        let a = Pos2::new(0.0, 0.0);
        let b = Pos2::new(-400.0, 50.0);
        let pts = route(a, b, &p, &RouteMeta::loose(&[]));
        assert_eq!(pts.len(), 6);
        let lane = pts[2].y;
        assert!(
            (lane - (a.y.max(b.y) + LANE_FALLBACK)).abs() < 1e-3
                || (lane - (a.y.min(b.y) - LANE_FALLBACK)).abs() < 1e-3,
            "fallback lane {lane} is not ±{LANE_FALLBACK} from the pins"
        );
    }

    /// 8. Zoom invariance. The router never sees zoom — it is graph-space by
    ///    construction — so what this asserts is that the screen transform
    ///    preserves the route's shape: identical segment angles, identical
    ///    length ratios, at 40% and 200%.
    #[test]
    fn t8_shape_is_zoom_invariant() {
        let p = prefs(WireStyle::Subway);
        let (a, b, src, dst) = pair(240.0, 88.0);
        let rects = [src, dst];
        let pts = route(a, b, &p, &meta(&src, &dst, &rects, 0));
        let to_screen = |k: f32, pan: Vec2| -> Vec<Pos2> {
            pts.iter()
                .map(|q| Pos2::new((q.x - pan.x) * k, (q.y - pan.y) * k))
                .collect()
        };
        let lo = to_screen(0.4, Vec2::new(-30.0, 12.0));
        let hi = to_screen(2.0, Vec2::new(500.0, -80.0));
        assert_eq!(lo.len(), hi.len());
        let total_lo: f32 = lo.windows(2).map(|w| (w[1] - w[0]).length()).sum();
        let total_hi: f32 = hi.windows(2).map(|w| (w[1] - w[0]).length()).sum();
        for i in 0..lo.len() - 1 {
            assert!(
                (angle_deg(lo[i], lo[i + 1]) - angle_deg(hi[i], hi[i + 1])).abs() < 1e-2,
                "segment {i} changed angle with zoom"
            );
            let rl = (lo[i + 1] - lo[i]).length() / total_lo;
            let rh = (hi[i + 1] - hi[i]).length() / total_hi;
            assert!((rl - rh).abs() < 1e-4, "segment {i} changed length ratio with zoom");
        }
    }

    /// 9. Six wires into a column of six pins render as a parallel ribbon:
    ///    every wire turns in at the same target-anchored x, so the diagonals
    ///    stay parallel and evenly spaced by the pin pitch.
    #[test]
    fn t9_six_wires_form_an_even_ribbon() {
        let p = prefs(WireStyle::Subway);
        let row = 22.0;
        let dst = Rect::from_min_max(Pos2::new(600.0, 0.0), Pos2::new(600.0 + NODE_W, 6.0 * row));
        let mut turn_ins = Vec::new();
        let mut diag_starts = Vec::new();
        for i in 0..6 {
            let a = Pos2::new(0.0, 300.0 + i as f32 * 8.0);
            let b = Pos2::new(600.0, row * 0.5 + i as f32 * row);
            let src = Rect::from_min_max(Pos2::new(-NODE_W, a.y - 50.0), Pos2::new(0.0, a.y + 50.0));
            let rects = [src, dst];
            let pts = route(a, b, &p, &meta(&src, &dst, &rects, i));
            assert_eq!(pts.len(), 4);
            turn_ins.push(pts[2].x);
            diag_starts.push(pts[1].x);
        }
        // Target-anchored: every wire turns in at the same x.
        for x in &turn_ins {
            assert!((x - turn_ins[0]).abs() < 1e-3, "ribbon is not parallel: {turn_ins:?}");
        }
        // Diagonal onsets step by the pin pitch, evenly.
        let steps: Vec<f32> = diag_starts.windows(2).map(|w| w[1] - w[0]).collect();
        for s in &steps {
            assert!(
                (s - steps[0]).abs() < 1e-3,
                "ribbon spacing is uneven: {steps:?}"
            );
        }
    }

    /// 10. Six Manhattan wires into six adjacent pins of one node must be six
    ///     *distinguishable* verticals, spaced by `bundle_offset`.
    ///
    ///     Scoped to spans wide enough for the stagger to survive its cap
    ///     (recorded ruling): `ho = min(16 + bi*4, max(4, dx/2))` collapses
    ///     for `dx <= 32 + 8*bi`, so at short spans the verticals *do*
    ///     coincide. That is the prototype's own behaviour and this test is
    ///     deliberately scoped above it; perpendicular-offset bundling proper
    ///     is a later item.
    #[test]
    fn t10_manhattan_stagger_is_distinguishable() {
        let p = prefs(WireStyle::Manhattan);
        let dx = 400.0; // comfortably above 32 + 8*5
        let mut xs = Vec::new();
        for i in 0..6usize {
            // dy stays clear of the near-horizontal shortcut, which would
            // otherwise collapse the first wire to a straight line.
            let (a, b, src, dst) = pair(dx, 100.0 + i as f32 * 22.0);
            let rects = [src, dst];
            let pts = route(a, b, &p, &meta(&src, &dst, &rects, i));
            assert_eq!(pts.len(), 4);
            xs.push(pts[1].x);
        }
        for w in xs.windows(2) {
            assert!(
                (w[0] - w[1]).abs() >= p.bundle_offset - 1e-3,
                "verticals coincide: {xs:?}"
            );
        }

        // The documented collapse below the cap: at dx = 20 every wire's cap
        // is max(4, 10) = 10, so all six share one vertical.
        let short = 20.0;
        let mut short_xs = Vec::new();
        for i in 0..6usize {
            let (a, b, src, dst) = pair(short, 200.0 + i as f32 * 22.0);
            let rects = [src, dst];
            let pts = route(a, b, &p, &meta(&src, &dst, &rects, i));
            short_xs.push(pts[1].x);
        }
        assert!(
            short_xs.windows(2).all(|w| (w[0] - w[1]).abs() < 1e-3),
            "the cap is supposed to collapse the stagger at short spans: {short_xs:?}"
        );
    }

    /// The stagger wraps at `bundle_max` rather than climbing forever: past
    /// the cap the offsets would all clamp to `max(4, dx/2)` and coincide,
    /// which is precisely what staggering is for.
    #[test]
    fn stagger_wraps_at_bundle_max() {
        let p = prefs(WireStyle::Manhattan);
        let dx = 400.0;
        let turn = |bi: usize| {
            let (a, b, src, dst) = pair(dx, 100.0 + bi as f32 * 22.0);
            let rects = [src, dst];
            route(a, b, &p, &meta(&src, &dst, &rects, bi))[1].x
        };
        // Lane 0 and lane `bundle_max` share a turn x — the wrap.
        let lanes = p.bundle_max as usize;
        assert!(
            (turn(0) - turn(lanes)).abs() < 1e-3,
            "index {lanes} should reuse lane 0"
        );
        // Every lane inside one period is still distinguishable.
        let xs: Vec<f32> = (0..lanes).map(turn).collect();
        for w in xs.windows(2) {
            assert!(
                (w[0] - w[1]).abs() >= p.bundle_offset - 1e-3,
                "lanes coincide inside a period: {xs:?}"
            );
        }
        // A high index no longer collapses against the cap.
        assert!((turn(lanes + 1) - turn(1)).abs() < 1e-3);
    }

    // ---------------------------------------------------------------
    // Corner rounding + degeneracy
    // ---------------------------------------------------------------

    /// Two adjacent corners sharing a short segment clamp to half its length
    /// each, so neither overruns the other or the segment's endpoints.
    #[test]
    fn corner_radius_clamps_per_corner() {
        // Middle segment is 6px long; radius 10 must clamp to 3 on both sides.
        let pts = [
            Pos2::new(0.0, 0.0),
            Pos2::new(100.0, 0.0),
            Pos2::new(100.0, 6.0),
            Pos2::new(200.0, 6.0),
        ];
        let out = round_corners(&pts, 10.0);
        assert!(out.len() > pts.len(), "corners were not tessellated");
        // Everything stays inside the polyline's own bounding box.
        let b = polyline_bounds(&out).unwrap();
        assert!(b.min.x >= -1e-3 && b.max.x <= 200.0 + 1e-3);
        assert!(b.min.y >= -1e-3 && b.max.y <= 6.0 + 1e-3);
        // The two corner arcs meet on the short segment without overlapping:
        // each consumes exactly half of it.
        let on_short: Vec<&Pos2> = out
            .iter()
            .filter(|q| (q.x - 100.0).abs() < 3.0 + 1e-3 && q.y > -1e-3 && q.y < 6.0 + 1e-3)
            .collect();
        assert!(!on_short.is_empty());
        assert!(out.iter().all(|q| q.x.is_finite() && q.y.is_finite()));
    }

    /// `dx == |dy|` compresses the offset to exactly zero, which emits a
    /// zero-length segment. It must dedup away — a corner arc on a
    /// zero-length segment divides by zero and produces NaN.
    #[test]
    fn exact_45_degenerate_point_dedups_without_nan() {
        let p = prefs(WireStyle::Subway);
        for d in [40.0f32, 88.0, 150.0] {
            let (a, b, src, dst) = pair(d, d);
            let rects = [src, dst];
            let pts = route(a, b, &p, &meta(&src, &dst, &rects, 0));
            // Raw route carries the coincident point…
            assert_eq!(pts.len(), 4);
            assert!((pts[0] - pts[1]).length() < 1e-6, "expected a coincident point");
            // …and rounding removes it, cleanly.
            let rounded = round_corners(&pts, p.corner_radius);
            assert!(
                rounded.iter().all(|q| q.x.is_finite() && q.y.is_finite()),
                "NaN survived the dedup at d={d}"
            );
            for w in rounded.windows(2) {
                assert!((w[1] - w[0]).length() < 1e6);
            }
            assert_eq!(rounded[0], a);
            assert_eq!(*rounded.last().unwrap(), b);
        }
    }

    #[test]
    fn dedup_keeps_the_first_of_a_run() {
        let pts = [
            Pos2::new(0.0, 0.0),
            Pos2::new(0.0, 0.0),
            Pos2::new(10.0, 0.0),
            Pos2::new(10.0, 0.0),
            Pos2::new(10.0, 10.0),
        ];
        assert_eq!(
            dedup(&pts),
            vec![Pos2::new(0.0, 0.0), Pos2::new(10.0, 0.0), Pos2::new(10.0, 10.0)]
        );
        // A two-point route is passed through untouched (nothing to round).
        let two = [Pos2::new(0.0, 0.0), Pos2::new(5.0, 5.0)];
        assert_eq!(round_corners(&two, 10.0), two.to_vec());
    }

    // ---------------------------------------------------------------
    // Spline + shared helpers
    // ---------------------------------------------------------------

    #[test]
    fn spline_tangent_matches_the_reference() {
        let curve = 0.55;
        // Clamped low.
        let (c1, _) = spline_controls(Pos2::ZERO, Pos2::new(10.0, 0.0), curve);
        assert!((c1.x - 34.0).abs() < 1e-3);
        // Clamped high.
        let (c1, _) = spline_controls(Pos2::ZERO, Pos2::new(2000.0, 0.0), curve);
        assert!((c1.x - 190.0).abs() < 1e-3);
        // In range: |dx| * (0.35 + 0.55*0.55) = 200 * 0.6525 = 130.5
        let (c1, c2) = spline_controls(Pos2::ZERO, Pos2::new(200.0, 40.0), curve);
        assert!((c1.x - 130.5).abs() < 1e-3);
        assert!((c2.x - (200.0 - 130.5)).abs() < 1e-3);
        // Backward: grown to at least 70 + 0.35|dx| where that binds.
        // |dx| = 100 -> base 65.25, grown to 70 + 35 = 105.
        let (c1, _) = spline_controls(Pos2::ZERO, Pos2::new(-100.0, 0.0), curve);
        assert!((c1.x - 105.0).abs() < 1e-3, "backward tangent not grown");
        // Far backward: the 190 clamp already exceeds the growth floor, so it
        // wins — same as the reference, which clamps before growing.
        let (c1, _) = spline_controls(Pos2::ZERO, Pos2::new(-300.0, 0.0), curve);
        assert!((c1.x - 190.0).abs() < 1e-3);
        // Control points stay on their own pin rows.
        let (c1, c2) = spline_controls(Pos2::new(1.0, 2.0), Pos2::new(300.0, 90.0), curve);
        assert_eq!(c1.y, 2.0);
        assert_eq!(c2.y, 90.0);
    }

    #[test]
    fn sample_is_dense_and_endpoint_exact() {
        let (a, b, src, dst) = pair(300.0, 90.0);
        let rects = [src, dst];
        for style in WireStyle::ALL {
            let p = prefs(style);
            let s = sample(a, b, &p, &meta(&src, &dst, &rects, 0));
            assert!(s.len() >= 9, "{style:?}: only {} samples", s.len());
            assert!((s[0] - a).length() < 1e-3, "{style:?}: sample missed the source pin");
            assert!(
                (*s.last().unwrap() - b).length() < 1e-3,
                "{style:?}: sample missed the target pin"
            );
        }
    }

    #[test]
    fn segment_intersection_is_exact() {
        let a = Pos2::new(0.0, 0.0);
        let b = Pos2::new(10.0, 0.0);
        // Crossing.
        assert!(segments_intersect(a, b, Pos2::new(5.0, -5.0), Pos2::new(5.0, 5.0)));
        // Touching an endpoint counts.
        assert!(segments_intersect(a, b, Pos2::new(10.0, -5.0), Pos2::new(10.0, 5.0)));
        // Beyond the end does not.
        assert!(!segments_intersect(a, b, Pos2::new(11.0, -5.0), Pos2::new(11.0, 5.0)));
        // Parallel and collinear are not crossings.
        assert!(!segments_intersect(a, b, Pos2::new(0.0, 3.0), Pos2::new(10.0, 3.0)));
        assert!(!segments_intersect(a, b, Pos2::new(2.0, 0.0), Pos2::new(8.0, 0.0)));
    }

    /// The cut preview must test every segment, not a bounding box: a slash
    /// through the *hole* of an L-shaped route crosses its bbox but no wire.
    #[test]
    fn path_crossing_tests_segments_not_bounding_boxes() {
        // An L: right along the top, then down the right side.
        let poly = [
            Pos2::new(0.0, 0.0),
            Pos2::new(100.0, 0.0),
            Pos2::new(100.0, 100.0),
        ];
        // Inside the bbox, through the empty quadrant: no crossing.
        let miss = [Pos2::new(10.0, 40.0), Pos2::new(60.0, 90.0)];
        assert!(!path_crosses_polyline(&miss, &poly));
        // Across the vertical leg: a crossing.
        let hit = [Pos2::new(80.0, 50.0), Pos2::new(120.0, 50.0)];
        assert!(path_crosses_polyline(&hit, &poly));
        // Degenerate inputs are never a crossing.
        assert!(!path_crosses_polyline(&[Pos2::ZERO], &poly));
        assert!(!path_crosses_polyline(&hit, &[Pos2::ZERO]));
    }

    #[test]
    fn point_distance_helpers() {
        let a = Pos2::new(0.0, 0.0);
        let b = Pos2::new(10.0, 0.0);
        assert!((point_segment_distance(Pos2::new(5.0, 3.0), a, b) - 3.0).abs() < 1e-4);
        // Past the end: clamps to the endpoint.
        assert!((point_segment_distance(Pos2::new(14.0, 0.0), a, b) - 4.0).abs() < 1e-4);
        // Degenerate segment.
        assert!((point_segment_distance(Pos2::new(0.0, 2.0), a, a) - 2.0).abs() < 1e-4);
        let poly = [a, b, Pos2::new(10.0, 10.0)];
        assert!((point_polyline_distance(Pos2::new(12.0, 5.0), &poly) - 2.0).abs() < 1e-4);
        assert_eq!(point_polyline_distance(Pos2::ZERO, &[]), f32::MAX);
    }

    #[test]
    fn spline_bounds_include_the_control_hull() {
        let p = prefs(WireStyle::Spline);
        let a = Pos2::new(0.0, 0.0);
        let b = Pos2::new(-300.0, 0.0);
        let bounds = wire_bounds(a, b, &p, &RouteMeta::loose(&[])).unwrap();
        // A backward spline bows well outside the endpoints' box.
        assert!(bounds.max.x > a.x, "control hull not included in the bounds");
        assert!(bounds.min.x < b.x);
    }

    /// `turn_anchor = Source` is the documented mirror: the turn sits
    /// `horizontal_offset` *after* the source instead of before the target.
    #[test]
    fn source_anchor_mirrors_the_turn() {
        let p = WirePrefs {
            style: WireStyle::Manhattan,
            turn_anchor: TurnAnchor::Source,
            ..WirePrefs::default()
        };
        let (a, b, src, dst) = pair(400.0, 90.0);
        let rects = [src, dst];
        let pts = route(a, b, &p, &meta(&src, &dst, &rects, 0));
        assert_eq!(pts.len(), 4);
        assert!((pts[1].x - (a.x + p.horizontal_offset)).abs() < 1e-3);
        assert!((pts[2].x - (a.x + p.horizontal_offset)).abs() < 1e-3);

        // Now every wire *leaving* one node shares its turn x, whatever the
        // target distance — the mirror of test 1.
        let xs: Vec<f32> = [500.0f32, 800.0, 1200.0]
            .iter()
            .map(|tx| {
                let bb = Pos2::new(*tx, 90.0);
                let dd = Rect::from_min_max(Pos2::new(*tx, 40.0), Pos2::new(tx + NODE_W, 140.0));
                let rr = [src, dd];
                route(a, bb, &p, &meta(&src, &dd, &rr, 0))[1].x
            })
            .collect();
        for x in &xs {
            assert!((x - xs[0]).abs() < 1e-3, "source-anchored turn moved: {xs:?}");
        }
    }

    /// `disable_pin_offset` turns hard at the border.
    #[test]
    fn disable_pin_offset_removes_the_stub() {
        let p = WirePrefs {
            style: WireStyle::Manhattan,
            disable_pin_offset: true,
            ..WirePrefs::default()
        };
        let (a, b, src, dst) = pair(400.0, 90.0);
        let rects = [src, dst];
        let pts = route(a, b, &p, &meta(&src, &dst, &rects, 0));
        assert!((pts[1].x - b.x).abs() < 1e-3, "turn is not on the target border");
    }

    /// The lane picks the side crossing fewer nodes, and ties go to the side
    /// the target sits on.
    #[test]
    fn backward_lane_prefers_the_emptier_side() {
        let p = prefs(WireStyle::Manhattan);
        let (a, b, src, dst) = pair(-400.0, 0.0);
        // A blocker straddling the *top* lane inside the corridor pushes the
        // route to the bottom.
        let top_lane = src.min.y.min(dst.min.y) - LANE_MARGIN;
        let blocker = Rect::from_min_max(
            Pos2::new(-300.0, top_lane - 10.0),
            Pos2::new(-100.0, top_lane + 10.0),
        );
        let with_blocker = [src, dst, blocker];
        let pts = route(a, b, &p, &meta(&src, &dst, &with_blocker, 0));
        let bot_lane = src.max.y.max(dst.max.y) + LANE_MARGIN;
        assert!((pts[2].y - bot_lane).abs() < 1e-3, "lane did not avoid the blocker");

        // Untouched, the tie goes to the target's side: b.y <= a.y -> top.
        let clean = [src, dst];
        let pts = route(a, b, &p, &meta(&src, &dst, &clean, 0));
        assert!((pts[2].y - top_lane).abs() < 1e-3, "tie should break to the top");
    }

    #[test]
    fn spline_route_is_just_its_endpoints() {
        let p = prefs(WireStyle::Spline);
        let a = Pos2::new(3.0, 4.0);
        let b = Pos2::new(200.0, 40.0);
        assert_eq!(route(a, b, &p, &RouteMeta::loose(&[])), vec![a, b]);
    }
}
