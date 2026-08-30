//! Blend space asset (`.blendspace`) and its pure blend math (Task 41.5).
//!
//! A blend space places clips on one or two named parameter axes and blends
//! whichever samples surround the input. This module owns two things:
//!
//! - [`BlendSpaceDoc`] — the RON document (version 1) with load/save helpers,
//!   mirroring `curve_asset`'s shape. Both axes are always stored and
//!   `axis_count` says how many are live, so the editor's 1D/2D toggle never
//!   loses data.
//! - [`BlendSpace`] — the compiled value: samples canonically ordered, a
//!   Delaunay triangulation for two axes, and [`BlendSpace::weights`], which
//!   maps an input point to at most three `(sample index, weight)` pairs that
//!   sum to one without allocating.
//!
//! Rules (per the spec): one axis brackets and clamps exactly like the
//! `anim_blend1d` node (`pick_1d`); two axes use barycentric weights inside
//! the containing triangle and, outside the hull, the weights at the nearest
//! hull point (Unreal's clamp). Degenerate 2D sets fall back: one sample plays
//! pure, two samples or an all-collinear set blend 1D along their line. Exact
//! on a sample is weight 1 on that sample; weights under [`WEIGHT_EPSILON`]
//! are dropped and the rest renormalized.
//!
//! No runtime wiring lives here — the machine's `PlanTree::Space` and the
//! editor tab consume this module (later tickets).

mod delaunay;

use serde::{Deserialize, Serialize};

/// Container version of a `.blendspace` document.
pub const BLEND_SPACE_DOC_VERSION: u32 = 1;

/// Contributions below this are dropped, so at most three clips are sampled.
pub const WEIGHT_EPSILON: f32 = 1e-4;

/// One parameter axis of the space.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BlendAxis {
    pub name: String,
    /// The Float parameter driving this axis; empty ⇒ the axis name.
    pub param: String,
    pub min: f32,
    pub max: f32,
    /// Canvas grid divisions across `min..max` (snap target in the editor).
    pub grid_divisions: u32,
}

impl Default for BlendAxis {
    fn default() -> Self {
        Self::new("Speed", 0.0, 1.0)
    }
}

impl BlendAxis {
    pub fn new(name: &str, min: f32, max: f32) -> Self {
        Self { name: name.into(), param: String::new(), min, max, grid_divisions: 10 }
    }

    /// The parameter this axis reads: `param`, or the axis name when unset.
    pub fn param_name(&self) -> &str {
        if self.param.is_empty() {
            &self.name
        } else {
            &self.param
        }
    }
}

/// A clip placed in the space. `y` is ignored for a one-axis space.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BlendSample {
    pub x: f32,
    pub y: f32,
    /// Content-relative `.anim` path, forward slashes.
    pub clip: String,
    /// Clip inside the container; `None` = the first.
    pub clip_name: Option<String>,
    /// Playback rate multiplier for this sample.
    pub rate_scale: f32,
}

impl Default for BlendSample {
    fn default() -> Self {
        Self { x: 0.0, y: 0.0, clip: String::new(), clip_name: None, rate_scale: 1.0 }
    }
}

impl BlendSample {
    pub fn new(x: f32, y: f32, clip: &str) -> Self {
        Self { x, y, clip: clip.into(), ..Default::default() }
    }
}

/// The `.blendspace` document. Also the create-template via [`Default`]: one
/// "Speed" axis over `0..1`, no samples.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BlendSpaceDoc {
    pub version: u32,
    /// Live axes: 1 or 2. Both axes keep their data either way.
    pub axis_count: u32,
    pub axes: [BlendAxis; 2],
    pub samples: Vec<BlendSample>,
    /// Exponential input smoothing time in seconds; 0 = off.
    pub input_smoothing: f32,
}

impl Default for BlendSpaceDoc {
    fn default() -> Self {
        Self {
            version: BLEND_SPACE_DOC_VERSION,
            axis_count: 1,
            axes: [BlendAxis::default(), BlendAxis::new("Direction", -1.0, 1.0)],
            samples: Vec::new(),
            input_smoothing: 0.0,
        }
    }
}

impl BlendSpaceDoc {
    pub fn is_2d(&self) -> bool {
        self.axis_count >= 2
    }

    /// The live axes (one or two).
    pub fn active_axes(&self) -> &[BlendAxis] {
        &self.axes[..(self.axis_count.clamp(1, 2) as usize)]
    }
}

#[derive(Deserialize)]
struct Envelope {
    #[serde(default)]
    version: u32,
}

/// Parse a `.blendspace` RON text; refuses documents from a newer container.
pub fn parse_blend_space(text: &str) -> Result<BlendSpaceDoc, String> {
    let probe: Envelope = ron::from_str(text).map_err(|e| e.to_string())?;
    if probe.version > BLEND_SPACE_DOC_VERSION {
        return Err(format!(
            "blend space version {} is newer than supported {}",
            probe.version, BLEND_SPACE_DOC_VERSION
        ));
    }
    let mut doc: BlendSpaceDoc = ron::from_str(text).map_err(|e| e.to_string())?;
    doc.version = BLEND_SPACE_DOC_VERSION;
    Ok(doc)
}

/// Pretty RON for a `.blendspace` file (version stamped to the current one).
pub fn serialize_blend_space(doc: &BlendSpaceDoc) -> Result<String, String> {
    let mut out = doc.clone();
    out.version = BLEND_SPACE_DOC_VERSION;
    let cfg = ron::ser::PrettyConfig::new()
        .struct_names(false)
        .separate_tuple_members(false)
        .enumerate_arrays(false);
    ron::ser::to_string_pretty(&out, cfg).map_err(|e| e.to_string())
}

/// Up to three `(sample index, weight)` contributions summing to one.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BlendWeights {
    entries: [(usize, f32); 3],
    len: usize,
}

impl BlendWeights {
    fn one(i: usize) -> Self {
        Self { entries: [(i, 1.0), (0, 0.0), (0, 0.0)], len: 1 }
    }

    fn push(&mut self, i: usize, w: f32) {
        if w >= WEIGHT_EPSILON && self.len < 3 {
            self.entries[self.len] = (i, w);
            self.len += 1;
        }
    }

    /// Drop-below-epsilon happened in `push`; renormalize what is left.
    fn finish(mut self) -> Self {
        let total: f32 = self.as_slice().iter().map(|e| e.1).sum();
        if total > 0.0 {
            for e in &mut self.entries[..self.len] {
                e.1 /= total;
            }
        }
        self
    }

    pub fn as_slice(&self) -> &[(usize, f32)] {
        &self.entries[..self.len]
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// How the samples are laid out for evaluation.
#[derive(Debug, Clone, PartialEq)]
enum Shape {
    /// 1D, or a 2D set that is a single point / a line: samples ordered by
    /// their scalar position `t` along `dir` from `origin`.
    Line { origin: [f32; 2], dir: [f32; 2], order: Vec<(f32, usize)> },
    /// A proper 2D set: CCW triangles and the CCW hull loop.
    Mesh { triangles: Vec<[usize; 3]>, hull: Vec<usize> },
}

/// A compiled blend space: pure evaluation, no assets, no allocation per query.
#[derive(Debug, Clone, PartialEq)]
pub struct BlendSpace {
    axes: Vec<BlendAxis>,
    samples: Vec<BlendSample>,
    points: Vec<[f32; 2]>,
    shape: Shape,
}

impl BlendSpace {
    /// Compile a document. Refuses an empty sample set, an axis count outside
    /// 1..=2, and (two axes only) samples sharing a position.
    pub fn compile(doc: &BlendSpaceDoc) -> Result<Self, String> {
        if doc.samples.is_empty() {
            return Err("no samples".into());
        }
        if !(1..=2).contains(&doc.axis_count) {
            return Err(format!("axis_count must be 1 or 2, got {}", doc.axis_count));
        }
        let two_d = doc.is_2d();
        let points: Vec<[f32; 2]> =
            doc.samples.iter().map(|s| [s.x, if two_d { s.y } else { 0.0 }]).collect();
        // Canonical order: by x then y. Every downstream decision runs over
        // this order, which is what makes the result independent of how the
        // author listed the samples.
        let mut sorted: Vec<usize> = (0..points.len()).collect();
        sorted.sort_by(|&a, &b| {
            points[a][0].total_cmp(&points[b][0]).then(points[a][1].total_cmp(&points[b][1]))
        });
        if two_d {
            if let Some(w) = sorted.windows(2).find(|w| points[w[0]] == points[w[1]]) {
                return Err(format!("samples {} and {} share a position", w[0].min(w[1]), w[0].max(w[1])));
            }
        }
        let shape = if two_d {
            Self::shape_2d(&points, &sorted)
        } else {
            Shape::Line {
                origin: [0.0, 0.0],
                dir: [1.0, 0.0],
                order: sorted.iter().map(|&i| (points[i][0], i)).collect(),
            }
        };
        Ok(Self {
            axes: doc.active_axes().to_vec(),
            samples: doc.samples.clone(),
            points,
            shape,
        })
    }

    fn shape_2d(points: &[[f32; 2]], sorted: &[usize]) -> Shape {
        let (first, last) = (points[sorted[0]], points[sorted[sorted.len() - 1]]);
        let d = [last[0] - first[0], last[1] - first[1]];
        let len = (d[0] * d[0] + d[1] * d[1]).sqrt();
        let dir = if len > 0.0 { [d[0] / len, d[1] / len] } else { [0.0, 0.0] };
        let collinear = points.iter().all(|p| {
            let r = [p[0] - first[0], p[1] - first[1]];
            (dir[0] * r[1] - dir[1] * r[0]).abs() <= 1e-5 * len.max(1.0)
        });
        if collinear {
            let mut order: Vec<(f32, usize)> = sorted
                .iter()
                .map(|&i| {
                    let r = [points[i][0] - first[0], points[i][1] - first[1]];
                    (dir[0] * r[0] + dir[1] * r[1], i)
                })
                .collect();
            order.sort_by(|a, b| a.0.total_cmp(&b.0));
            return Shape::Line { origin: first, dir, order };
        }
        let pts: Vec<[f64; 2]> =
            sorted.iter().map(|&i| [points[i][0] as f64, points[i][1] as f64]).collect();
        let mut triangles: Vec<[usize; 3]> = delaunay::triangulate(&pts)
            .into_iter()
            .map(|t| {
                let t = [sorted[t[0]], sorted[t[1]], sorted[t[2]]];
                // Rotate so the smallest index leads; orientation is kept.
                let k = (0..3).min_by_key(|&k| t[k]).unwrap_or(0);
                [t[k], t[(k + 1) % 3], t[(k + 2) % 3]]
            })
            .collect();
        triangles.sort();
        let hull = delaunay::hull(&triangles);
        Shape::Mesh { triangles, hull }
    }

    pub fn axes(&self) -> &[BlendAxis] {
        &self.axes
    }

    pub fn is_2d(&self) -> bool {
        self.axes.len() == 2
    }

    pub fn samples(&self) -> &[BlendSample] {
        &self.samples
    }

    /// Sample positions in document order (`y` is 0 for one axis).
    pub fn points(&self) -> &[[f32; 2]] {
        &self.points
    }

    /// CCW Delaunay triangles over sample indices; empty unless the space is a
    /// proper 2D set.
    pub fn triangles(&self) -> &[[usize; 3]] {
        match &self.shape {
            Shape::Mesh { triangles, .. } => triangles,
            Shape::Line { .. } => &[],
        }
    }

    /// The convex hull as a CCW loop of sample indices; for a line-shaped set
    /// this is its two extreme samples, for a single sample just that one.
    pub fn hull(&self) -> Vec<usize> {
        match &self.shape {
            Shape::Mesh { hull, .. } => hull.clone(),
            Shape::Line { order, .. } => {
                let (a, b) = (order[0].1, order[order.len() - 1].1);
                if a == b { vec![a] } else { vec![a, b] }
            }
        }
    }

    /// Contributions for an input point (`input[1]` ignored for one axis).
    pub fn weights(&self, input: [f32; 2]) -> BlendWeights {
        match &self.shape {
            Shape::Line { origin, dir, order } => {
                let r = [input[0] - origin[0], input[1] - origin[1]];
                line_weights(order, dir[0] * r[0] + dir[1] * r[1])
            }
            Shape::Mesh { triangles, hull } => {
                if let Some(i) = self.points.iter().position(|p| *p == input) {
                    return BlendWeights::one(i);
                }
                for t in triangles {
                    if let Some(b) = self.barycentric(*t, input) {
                        let mut w = BlendWeights::default();
                        for k in 0..3 {
                            w.push(t[k], b[k]);
                        }
                        return w.finish();
                    }
                }
                self.hull_clamp(hull, input)
            }
        }
    }

    /// Barycentric weights of `p` in the CCW triangle, or `None` when outside.
    fn barycentric(&self, t: [usize; 3], p: [f32; 2]) -> Option<[f32; 3]> {
        let at = |i: usize| [self.points[i][0] as f64, self.points[i][1] as f64];
        let (a, b, c) = (at(t[0]), at(t[1]), at(t[2]));
        let q = [p[0] as f64, p[1] as f64];
        let area = delaunay::orient(a, b, c);
        if area <= 0.0 {
            return None;
        }
        let w = [
            delaunay::orient(b, c, q) / area,
            delaunay::orient(c, a, q) / area,
            delaunay::orient(a, b, q) / area,
        ];
        w.iter().all(|&x| x >= -1e-6).then(|| w.map(|x| x.max(0.0) as f32))
    }

    /// Weights at the nearest point on the hull loop to `p`.
    fn hull_clamp(&self, hull: &[usize], p: [f32; 2]) -> BlendWeights {
        let mut best = (f32::MAX, hull[0], hull[0], 0.0f32);
        for k in 0..hull.len() {
            let (ia, ib) = (hull[k], hull[(k + 1) % hull.len()]);
            let (a, b) = (self.points[ia], self.points[ib]);
            let e = [b[0] - a[0], b[1] - a[1]];
            let len2 = e[0] * e[0] + e[1] * e[1];
            let s = if len2 > 0.0 {
                (((p[0] - a[0]) * e[0] + (p[1] - a[1]) * e[1]) / len2).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let q = [a[0] + e[0] * s, a[1] + e[1] * s];
            let d2 = (p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2);
            if d2 < best.0 {
                best = (d2, ia, ib, s);
            }
        }
        let mut w = BlendWeights::default();
        w.push(best.1, 1.0 - best.3);
        w.push(best.2, best.3);
        w.finish()
    }
}

/// The `pick_1d` rule over samples sorted by `t`: the nearest endpoint plays
/// pure outside the range, a bracketing pair blends proportionally inside.
fn line_weights(order: &[(f32, usize)], v: f32) -> BlendWeights {
    let last = order.len() - 1;
    if v <= order[0].0 {
        return BlendWeights::one(order[0].1);
    }
    if v >= order[last].0 {
        return BlendWeights::one(order[last].1);
    }
    let i = order.iter().rposition(|(t, _)| *t <= v).unwrap_or(0);
    let (t0, t1) = (order[i].0, order[i + 1].0);
    let w = (v - t0) / (t1 - t0);
    if w <= 0.0 {
        return BlendWeights::one(order[i].1);
    }
    let mut out = BlendWeights::default();
    out.push(order[i].1, 1.0 - w);
    out.push(order[i + 1].1, w);
    out.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc_1d(xs: &[f32]) -> BlendSpaceDoc {
        let mut doc = BlendSpaceDoc::default();
        doc.samples = xs.iter().map(|&x| BlendSample::new(x, 0.0, "a.anim")).collect();
        doc
    }

    fn doc_2d(pts: &[(f32, f32)]) -> BlendSpaceDoc {
        let mut doc = BlendSpaceDoc::default();
        doc.axis_count = 2;
        doc.samples = pts.iter().map(|&(x, y)| BlendSample::new(x, y, "a.anim")).collect();
        doc
    }

    fn sorted(w: &BlendWeights) -> Vec<(usize, f32)> {
        let mut v = w.as_slice().to_vec();
        v.sort_by_key(|e| e.0);
        v
    }

    fn assert_sum_one(w: &BlendWeights) {
        let s: f32 = w.as_slice().iter().map(|e| e.1).sum();
        assert!((s - 1.0).abs() < 1e-5, "weights {:?} sum to {s}", w.as_slice());
    }

    fn assert_close(a: &[(usize, f32)], b: &[(usize, f32)]) {
        assert_eq!(a.len(), b.len(), "{a:?} vs {b:?}");
        for (x, y) in a.iter().zip(b) {
            assert_eq!(x.0, y.0, "{a:?} vs {b:?}");
            assert!((x.1 - y.1).abs() < 1e-4, "{a:?} vs {b:?}");
        }
    }

    // --- document ---------------------------------------------------------

    #[test]
    fn round_trips_1d_and_2d_documents() {
        let mut d1 = doc_1d(&[0.0, 3.0, 6.0]);
        d1.axes[0].param = "Velocity".into();
        d1.axes[0].max = 6.0;
        d1.samples[1].rate_scale = 1.5;
        d1.samples[2].clip_name = Some("Run".into());
        d1.input_smoothing = 0.2;
        let text = serialize_blend_space(&d1).expect("serialize");
        assert_eq!(parse_blend_space(&text).expect("parse"), d1);

        let mut d2 = doc_2d(&[(0.0, 0.0), (1.0, 0.0), (0.0, 1.0)]);
        d2.axes[1].name = "Strafe".into();
        d2.axes[1].grid_divisions = 4;
        let text = serialize_blend_space(&d2).expect("serialize");
        assert_eq!(parse_blend_space(&text).expect("parse"), d2);
    }

    #[test]
    fn default_is_the_create_template_and_parses_from_minimal_text() {
        let d = BlendSpaceDoc::default();
        assert_eq!(d.axis_count, 1);
        assert_eq!(d.axes[0].name, "Speed");
        assert_eq!((d.axes[0].min, d.axes[0].max), (0.0, 1.0));
        assert!(d.samples.is_empty());
        assert_eq!(d.axes[0].param_name(), "Speed");
        assert_eq!(parse_blend_space("()").expect("parse"), d);
        assert!(parse_blend_space("(version: 99)").is_err());
    }

    #[test]
    fn demo_asset_parses_and_compiles() {
        let text = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../content/blendspaces/locomotion.blendspace"
        ));
        let doc = parse_blend_space(text).expect("demo parses");
        assert_eq!(doc.samples.len(), 3);
        let space = BlendSpace::compile(&doc).expect("demo compiles");
        assert_close(&sorted(&space.weights([1.5, 0.0])), &[(0, 0.5), (1, 0.5)]);
    }

    // --- 1D ---------------------------------------------------------------

    #[test]
    fn one_axis_brackets_and_clamps_like_pick_1d() {
        let space = BlendSpace::compile(&doc_1d(&[0.0, 3.0, 6.0])).expect("compile");
        assert_eq!(space.weights([-5.0, 0.0]).as_slice(), &[(0, 1.0)]);
        assert_eq!(space.weights([0.0, 0.0]).as_slice(), &[(0, 1.0)]);
        assert_eq!(space.weights([3.0, 0.0]).as_slice(), &[(1, 1.0)]);
        assert_eq!(space.weights([6.0, 0.0]).as_slice(), &[(2, 1.0)]);
        assert_eq!(space.weights([60.0, 0.0]).as_slice(), &[(2, 1.0)]);
        assert_close(&sorted(&space.weights([1.0, 0.0])), &[(0, 2.0 / 3.0), (1, 1.0 / 3.0)]);
        assert_close(&sorted(&space.weights([4.5, 0.0])), &[(1, 0.5), (2, 0.5)]);
        // y is ignored for one axis.
        assert_close(&sorted(&space.weights([4.5, 99.0])), &[(1, 0.5), (2, 0.5)]);
    }

    #[test]
    fn one_axis_ignores_authoring_order() {
        let a = BlendSpace::compile(&doc_1d(&[0.0, 3.0, 6.0])).expect("compile");
        let b = BlendSpace::compile(&doc_1d(&[6.0, 0.0, 3.0])).expect("compile");
        // Doc index 1 in `b` is x=0, index 2 is x=3.
        assert_close(&sorted(&b.weights([1.0, 0.0])), &[(1, 2.0 / 3.0), (2, 1.0 / 3.0)]);
        assert_close(&sorted(&a.weights([1.0, 0.0])), &[(0, 2.0 / 3.0), (1, 1.0 / 3.0)]);
        assert_eq!(b.hull(), vec![1, 0]);
    }

    // --- 2D ---------------------------------------------------------------

    #[test]
    fn exact_on_sample_is_one_and_nothing_else() {
        let space = BlendSpace::compile(&doc_2d(&[(0.0, 0.0), (2.0, 0.0), (0.0, 2.0), (2.0, 2.0)]))
            .expect("compile");
        for (i, p) in space.points().iter().enumerate() {
            assert_eq!(space.weights(*p).as_slice(), &[(i, 1.0)]);
        }
    }

    #[test]
    fn inside_a_triangle_is_barycentric_and_sums_to_one() {
        let space =
            BlendSpace::compile(&doc_2d(&[(0.0, 0.0), (4.0, 0.0), (0.0, 4.0)])).expect("compile");
        assert_eq!(space.triangles(), &[[0, 1, 2]]);
        let w = space.weights([1.0, 1.0]);
        assert_sum_one(&w);
        assert_close(&sorted(&w), &[(0, 0.5), (1, 0.25), (2, 0.25)]);
        // On an edge: the far vertex drops out.
        assert_close(&sorted(&space.weights([2.0, 0.0])), &[(0, 0.5), (1, 0.5)]);
        // Never more than three contributors, always summing to one.
        let space = BlendSpace::compile(&doc_2d(&[
            (0.0, 0.0),
            (3.0, 0.0),
            (0.0, 3.0),
            (3.0, 3.0),
            (1.5, 1.5),
            (1.0, 2.5),
        ]))
        .expect("compile");
        for p in [[0.5, 0.5], [2.9, 0.1], [1.5, 2.0], [1.6, 1.4], [0.2, 2.8]] {
            let w = space.weights(p);
            assert!(w.len() <= 3 && !w.is_empty());
            assert_sum_one(&w);
        }
    }

    #[test]
    fn outside_the_hull_equals_the_nearest_hull_point() {
        let space =
            BlendSpace::compile(&doc_2d(&[(0.0, 0.0), (4.0, 0.0), (0.0, 4.0), (4.0, 4.0), (2.0, 2.0)]))
                .expect("compile");
        assert_eq!(space.hull().len(), 4);
        // Below the bottom edge: nearest hull point is (1, 0).
        assert_close(&sorted(&space.weights([1.0, -3.0])), &sorted(&space.weights([1.0, 0.0])));
        // Past a corner: the corner sample plays pure.
        assert_eq!(space.weights([9.0, 9.0]).as_slice(), &[(3, 1.0)]);
        // Right of the right edge, mid-height: nearest is (4, 3).
        assert_close(&sorted(&space.weights([7.0, 3.0])), &sorted(&space.weights([4.0, 3.0])));
        assert_sum_one(&space.weights([7.0, 3.0]));
    }

    #[test]
    fn degenerate_sets_fall_back_to_line_or_single() {
        // One sample: always that clip.
        let one = BlendSpace::compile(&doc_2d(&[(1.0, 1.0)])).expect("compile");
        assert_eq!(one.weights([5.0, -5.0]).as_slice(), &[(0, 1.0)]);
        assert_eq!(one.hull(), vec![0]);
        // Two samples: projected 1D along the segment, clamped.
        let two = BlendSpace::compile(&doc_2d(&[(0.0, 0.0), (2.0, 2.0)])).expect("compile");
        assert!(two.triangles().is_empty());
        assert_close(&sorted(&two.weights([1.0, 1.0])), &[(0, 0.5), (1, 0.5)]);
        assert_close(&sorted(&two.weights([2.0, 0.0])), &[(0, 0.5), (1, 0.5)]);
        assert_eq!(two.weights([-3.0, -1.0]).as_slice(), &[(0, 1.0)]);
        assert_eq!(two.weights([9.0, 9.0]).as_slice(), &[(1, 1.0)]);
        // All collinear: same rule along the line, in doc indices.
        let line =
            BlendSpace::compile(&doc_2d(&[(2.0, 2.0), (0.0, 0.0), (1.0, 1.0)])).expect("compile");
        assert!(line.triangles().is_empty());
        assert_eq!(line.hull(), vec![1, 0]);
        assert_close(&sorted(&line.weights([0.5, 0.5])), &[(1, 0.5), (2, 0.5)]);
        assert_eq!(line.weights([1.0, 1.0]).as_slice(), &[(2, 1.0)]);
    }

    #[test]
    fn refusals() {
        assert_eq!(BlendSpace::compile(&BlendSpaceDoc::default()).unwrap_err(), "no samples");
        let mut bad = doc_1d(&[0.0]);
        bad.axis_count = 3;
        assert!(BlendSpace::compile(&bad).is_err());
        let dup = doc_2d(&[(0.0, 0.0), (1.0, 1.0), (0.0, 0.0)]);
        assert_eq!(BlendSpace::compile(&dup).unwrap_err(), "samples 0 and 2 share a position");
        // Equal x in 1D is fine (pick_1d parity: the later one wins).
        assert!(BlendSpace::compile(&doc_1d(&[0.0, 0.0, 1.0])).is_ok());
    }

    #[test]
    fn triangulation_and_weights_are_independent_of_sample_order() {
        let pts = [
            (0.0, 0.0),
            (3.0, 0.0),
            (0.0, 3.0),
            (3.0, 3.0),
            (1.5, 1.5),
            (1.0, 2.5),
            (2.5, 0.5),
            (-1.0, 1.5),
        ];
        let base = BlendSpace::compile(&doc_2d(&pts)).expect("compile");
        let probes = [[0.5, 0.5], [2.0, 2.0], [1.2, 2.6], [-0.5, 1.0], [5.0, 1.0], [1.5, -2.0], [0.0, 3.0]];
        // A few permutations; each maps its doc indices back to the base ones.
        let perms: [Vec<usize>; 3] =
            [vec![7, 6, 5, 4, 3, 2, 1, 0], vec![4, 0, 7, 3, 1, 6, 2, 5], vec![2, 5, 1, 7, 0, 4, 6, 3]];
        for perm in &perms {
            let shuffled: Vec<(f32, f32)> = perm.iter().map(|&i| pts[i]).collect();
            let other = BlendSpace::compile(&doc_2d(&shuffled)).expect("compile");
            let mut tris: Vec<[usize; 3]> = other
                .triangles()
                .iter()
                .map(|t| {
                    let mut m = [perm[t[0]], perm[t[1]], perm[t[2]]];
                    m.sort();
                    m
                })
                .collect();
            tris.sort();
            let mut base_tris: Vec<[usize; 3]> = base
                .triangles()
                .iter()
                .map(|t| {
                    let mut m = *t;
                    m.sort();
                    m
                })
                .collect();
            base_tris.sort();
            assert_eq!(tris, base_tris);
            for p in probes {
                let mapped: Vec<(usize, f32)> =
                    other.weights(p).as_slice().iter().map(|&(i, w)| (perm[i], w)).collect();
                let mut mapped = mapped;
                mapped.sort_by_key(|e| e.0);
                assert_close(&mapped, &sorted(&base.weights(p)));
            }
        }
    }
}
