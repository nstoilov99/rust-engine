//! **`.curve` — named float tracks of keyframes** (Task 45-A D7, resolved
//! question 4).
//!
//! # Why its own crate
//!
//! Three consumers, three different worlds:
//!
//! - the **interpreter** (`node_graph_exec`) samples tracks while a Timeline
//!   runs, and must stay wasm-clean with no engine dependency (D8);
//! - the **document layer** (`node_graph_types`) needs a curve's *track names*
//!   to answer "what pins does this Timeline node have" (D3's doc-dependent
//!   descriptor resolution);
//! - the **engine** owns file IO, `AssetType`, hot-reload and the editor.
//!
//! A curve is not a graph document, so it does not belong in
//! `node_graph_types`; it is not engine state, so it does not belong in the
//! engine. It is a leaf: serde + RON text + evaluation, no IO, no clock, no
//! allocation beyond the document itself. The engine reads and writes the
//! files; this crate defines what is in them and what they mean.
//!
//! The animation task (41) grows the same asset rather than forking one — that
//! is the whole point of shipping it here from day one.
//!
//! # Interpolation
//!
//! A key's `interp` governs the segment **leaving** it (the usual keyframe
//! convention: you set the mode on the key you are leaving, and the last key
//! has no segment to govern).
//!
//! `Cubic` is **Hermite with Catmull-Rom finite-difference tangents, scaled by
//! the segment's own duration**:
//!
//! ```text
//!   m_i = (v_{i+1} - v_{i-1}) / (t_{i+1} - t_{i-1})      (one-sided at the ends)
//!   segment [t_i, t_{i+1}], h = t_{i+1} - t_i, s = (t - t_i) / h
//!   v(s) = h00(s)·v_i + h10(s)·h·m_i + h01(s)·v_{i+1} + h11(s)·h·m_{i+1}
//! ```
//!
//! Chosen over uniform Catmull-Rom because keyframes are **not** uniformly
//! spaced in time, and the uniform form overshoots badly when they are not.
//! The time-scaled finite-difference form is C1, degenerates to a straight
//! line when the neighbours are collinear, and is the same construction a
//! curve editor's "auto" tangent gives — so what the editor draws is what the
//! interpreter samples.

use serde::{Deserialize, Serialize};

/// Container version of a `.curve` document. Bump when the *shape* changes;
/// [`migrate_container`] gains a step, and old files keep loading.
pub const CURVE_DOC_VERSION: u32 = 1;

/// How the segment leaving a key behaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Interp {
    /// Hold this key's value until the next key.
    Constant,
    #[default]
    Linear,
    /// See the module docs: time-scaled Catmull-Rom Hermite.
    Cubic,
}

impl Interp {
    pub const ALL: [Interp; 3] = [Interp::Constant, Interp::Linear, Interp::Cubic];

    pub fn label(&self) -> &'static str {
        match self {
            Interp::Constant => "Constant",
            Interp::Linear => "Linear",
            Interp::Cubic => "Cubic",
        }
    }

    /// The next mode in the cycle — what right-clicking a key does.
    pub fn next(&self) -> Interp {
        match self {
            Interp::Constant => Interp::Linear,
            Interp::Linear => Interp::Cubic,
            Interp::Cubic => Interp::Constant,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Key {
    /// Seconds from the start of the curve.
    pub t: f32,
    pub value: f32,
    pub interp: Interp,
}

impl Key {
    pub fn new(t: f32, value: f32) -> Self {
        Self { t, value, interp: Interp::Linear }
    }
}

/// One named float channel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Track {
    /// Identifier — and, for a Timeline node, the **output pin slug**. Stable
    /// across renames of `label`, which is why the two are separate.
    pub slug: String,
    pub label: String,
    /// Sorted by `t`. [`CurveDoc::canonicalize`] guarantees it; loading calls
    /// it, so nothing downstream has to check.
    pub keys: Vec<Key>,
}

impl Track {
    pub fn new(slug: &str, label: &str) -> Self {
        Self { slug: slug.to_string(), label: label.to_string(), keys: Vec::new() }
    }

    /// Last key time — this track's own length.
    pub fn duration(&self) -> f32 {
        self.keys.last().map(|k| k.t).unwrap_or(0.0)
    }

    /// Sample at `t`. Clamped at both ends: before the first key the first
    /// value, after the last key the last value. An empty track is 0.0 —
    /// absent data reads as neutral rather than as an error, because a track
    /// with no keys is a normal state in a half-authored curve.
    pub fn sample(&self, t: f32) -> f32 {
        let n = self.keys.len();
        if n == 0 {
            return 0.0;
        }
        // Strictly *before* the first key, not "at or before": landing exactly
        // on it has to fall through to the segment walk, or a pair of keys
        // stacked at t0 would report the earlier one and the step would run
        // backwards.
        if n == 1 || t < self.keys[0].t {
            return self.keys[0].value;
        }
        if t >= self.keys[n - 1].t {
            return self.keys[n - 1].value;
        }
        // The segment [i, i+1] containing t. Linear scan: a hand-authored
        // curve has a handful of keys, and a binary search here would be
        // more code than it saves.
        let i = self
            .keys
            .windows(2)
            .position(|w| t >= w[0].t && t < w[1].t)
            .unwrap_or(n - 2);
        let a = self.keys[i];
        let b = self.keys[i + 1];
        let h = b.t - a.t;
        if h <= f32::EPSILON {
            // Two keys at the same time: a step. The later one wins, which
            // is what stacking keys is *for*.
            return b.value;
        }
        let s = ((t - a.t) / h).clamp(0.0, 1.0);
        match a.interp {
            Interp::Constant => a.value,
            Interp::Linear => a.value + (b.value - a.value) * s,
            Interp::Cubic => {
                let ma = self.tangent(i);
                let mb = self.tangent(i + 1);
                hermite(a.value, ma * h, b.value, mb * h, s)
            }
        }
    }

    /// Catmull-Rom finite-difference tangent at key `i`, in value-per-second.
    /// One-sided at the ends, which is what makes the first and last segments
    /// behave rather than fly off.
    fn tangent(&self, i: usize) -> f32 {
        let n = self.keys.len();
        if n < 2 {
            return 0.0;
        }
        let (lo, hi) = (i.saturating_sub(1), (i + 1).min(n - 1));
        let dt = self.keys[hi].t - self.keys[lo].t;
        if dt <= f32::EPSILON {
            return 0.0;
        }
        (self.keys[hi].value - self.keys[lo].value) / dt
    }

    /// Sort by time. Stable, so two keys sharing a time keep document order
    /// and the later one wins the sample — a deliberate step.
    pub fn canonicalize(&mut self) {
        self.keys.sort_by(|a, b| a.t.total_cmp(&b.t));
    }
}

/// Cubic Hermite basis. `m0`/`m1` are already scaled by the segment duration.
fn hermite(v0: f32, m0: f32, v1: f32, m1: f32, s: f32) -> f32 {
    let s2 = s * s;
    let s3 = s2 * s;
    let h00 = 2.0 * s3 - 3.0 * s2 + 1.0;
    let h10 = s3 - 2.0 * s2 + s;
    let h01 = -2.0 * s3 + 3.0 * s2;
    let h11 = s3 - s2;
    h00 * v0 + h10 * m0 + h01 * v1 + h11 * m1
}

/// A `.curve` document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CurveDoc {
    pub version: u32,
    pub tracks: Vec<Track>,
}

impl Default for CurveDoc {
    fn default() -> Self {
        Self { version: CURVE_DOC_VERSION, tracks: Vec::new() }
    }
}

impl CurveDoc {
    /// The document's length: the longest track. `0.0` for an empty document,
    /// which a Timeline reads as "nothing to play".
    pub fn duration(&self) -> f32 {
        self.tracks.iter().map(|t| t.duration()).fold(0.0, f32::max)
    }

    pub fn track(&self, slug: &str) -> Option<&Track> {
        self.tracks.iter().find(|t| t.slug == slug)
    }

    pub fn track_mut(&mut self, slug: &str) -> Option<&mut Track> {
        self.tracks.iter_mut().find(|t| t.slug == slug)
    }

    /// Track slugs in document order — the Timeline node's output pins.
    pub fn slugs(&self) -> Vec<&str> {
        self.tracks.iter().map(|t| t.slug.as_str()).collect()
    }

    pub fn canonicalize(&mut self) {
        for t in &mut self.tracks {
            t.canonicalize();
        }
    }

    /// A slug not yet taken, derived from `base`. Same rule as the graph
    /// variables panel: snake_case, numeric suffix on collision.
    pub fn free_slug(&self, base: &str) -> String {
        let root = slugify(base);
        if self.track(&root).is_none() {
            return root;
        }
        for n in 2..1000 {
            let candidate = format!("{root}_{n}");
            if self.track(&candidate).is_none() {
                return candidate;
            }
        }
        format!("{root}_x")
    }
}

/// ASCII snake_case. Anything that is not alphanumeric becomes `_`; a leading
/// digit gets a `t` prefix, because a pin slug has to be an identifier.
pub fn slugify(s: &str) -> String {
    let mut out = String::new();
    let mut prev_us = false;
    for ch in s.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_us = false;
        } else if !prev_us && !out.is_empty() {
            out.push('_');
            prev_us = true;
        }
    }
    while out.ends_with('_') {
        out.pop();
    }
    if out.is_empty() {
        return "track".to_string();
    }
    if out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, 't');
    }
    out
}

// ---------------------------------------------------------------------------
// IO (text only — the engine owns files)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum CurveIoError {
    Parse(String),
    Serialize(String),
    /// Written by a newer build than this one. Refused rather than guessed at:
    /// the same rule `.graph` documents follow.
    FutureVersion { found: u32, supported: u32 },
}

impl std::fmt::Display for CurveIoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CurveIoError::Parse(e) => write!(f, "curve parse failed: {e}"),
            CurveIoError::Serialize(e) => write!(f, "curve serialize failed: {e}"),
            CurveIoError::FutureVersion { found, supported } => write!(
                f,
                "curve document version {found} is newer than supported {supported}"
            ),
        }
    }
}

impl std::error::Error for CurveIoError {}

/// Envelope probe: the version field only, unknown fields ignored — so a
/// document from a future build is *diagnosed* rather than failing to parse
/// with a field error that says nothing useful.
#[derive(Deserialize)]
struct Envelope {
    #[serde(default)]
    version: u32,
}

pub fn parse_curve(text: &str) -> Result<CurveDoc, CurveIoError> {
    let probe: Envelope =
        ron::from_str(text).map_err(|e| CurveIoError::Parse(e.to_string()))?;
    if probe.version > CURVE_DOC_VERSION {
        return Err(CurveIoError::FutureVersion {
            found: probe.version,
            supported: CURVE_DOC_VERSION,
        });
    }
    let mut doc: CurveDoc =
        ron::from_str(text).map_err(|e| CurveIoError::Parse(e.to_string()))?;
    migrate_container(&mut doc, probe.version);
    // Unsorted input is tolerated and fixed here, once, so nothing downstream
    // has to defend against it.
    doc.canonicalize();
    Ok(doc)
}

/// The container migration seam. There is exactly one version today, so this
/// is a no-op with a shape — the point is that v2 adds an arm here instead of
/// inventing a mechanism under time pressure.
fn migrate_container(doc: &mut CurveDoc, from: u32) {
    // Stated rather than looped: with one version there is nothing to step
    // through. Adding v2 turns this into the same `while v < VERSION` chain
    // `parse_graph` runs — one step per version, each leaving the document at
    // the next. `from == 0` means the file had no version field at all, which
    // is the earliest shape, i.e. v1.
    doc.version = from.clamp(1, CURVE_DOC_VERSION);
}

/// Pretty RON, canonicalized first. Curves live in version control, so a save
/// must not churn on key order the editor happened to produce.
pub fn serialize_curve(doc: &CurveDoc) -> Result<String, CurveIoError> {
    let mut out = doc.clone();
    out.version = CURVE_DOC_VERSION;
    out.canonicalize();
    let cfg = ron::ser::PrettyConfig::new()
        .struct_names(false)
        .separate_tuple_members(false)
        .enumerate_arrays(false);
    ron::ser::to_string_pretty(&out, cfg).map_err(|e| CurveIoError::Serialize(e.to_string()))
}

#[cfg(test)]
mod tests {
    // Tests build documents the way an author does — start from the default,
    // fill in what matters. One giant struct literal would satisfy clippy and
    // be markedly harder to read.
    #![allow(clippy::field_reassign_with_default)]

    use super::*;

    fn track(keys: &[(f32, f32, Interp)]) -> Track {
        Track {
            slug: "t".into(),
            label: "T".into(),
            keys: keys.iter().map(|(t, v, i)| Key { t: *t, value: *v, interp: *i }).collect(),
        }
    }

    #[test]
    fn empty_and_single_key_tracks_are_flat() {
        let empty = track(&[]);
        assert_eq!(empty.sample(0.0), 0.0);
        assert_eq!(empty.duration(), 0.0);

        let one = track(&[(1.0, 5.0, Interp::Linear)]);
        assert_eq!(one.sample(-9.0), 5.0);
        assert_eq!(one.sample(1.0), 5.0);
        assert_eq!(one.sample(99.0), 5.0);
    }

    #[test]
    fn sampling_clamps_outside_the_key_range() {
        let t = track(&[(1.0, 10.0, Interp::Linear), (3.0, 30.0, Interp::Linear)]);
        assert_eq!(t.sample(0.0), 10.0, "before the first key holds the first value");
        assert_eq!(t.sample(5.0), 30.0, "after the last key holds the last value");
        assert_eq!(t.duration(), 3.0);
    }

    #[test]
    fn linear_interpolates_and_constant_holds() {
        let lin = track(&[(0.0, 0.0, Interp::Linear), (2.0, 10.0, Interp::Linear)]);
        assert!((lin.sample(1.0) - 5.0).abs() < 1e-5);
        assert!((lin.sample(0.5) - 2.5).abs() < 1e-5);

        // The mode belongs to the key you are *leaving*.
        let step = track(&[(0.0, 0.0, Interp::Constant), (2.0, 10.0, Interp::Linear)]);
        assert_eq!(step.sample(1.9), 0.0);
        assert_eq!(step.sample(2.0), 10.0);
    }

    /// Cubic passes through its keys, stays monotone-ish on a ramp, and has no
    /// jump at a segment boundary (C1 by construction).
    #[test]
    fn cubic_passes_through_keys_and_is_continuous() {
        let c = track(&[
            (0.0, 0.0, Interp::Cubic),
            (1.0, 1.0, Interp::Cubic),
            (2.0, 0.0, Interp::Cubic),
        ]);
        for (t, v) in [(0.0, 0.0), (1.0, 1.0), (2.0, 0.0)] {
            assert!((c.sample(t) - v).abs() < 1e-5, "key at {t} must be exact");
        }
        // Continuity across the middle key.
        let l = c.sample(1.0 - 1e-3);
        let r = c.sample(1.0 + 1e-3);
        assert!((l - r).abs() < 1e-2, "{l} vs {r}");
        // The hump goes above the straight line between key 0 and key 1…
        assert!(c.sample(0.5) > 0.5, "cubic eases, {}", c.sample(0.5));
        // …and does not run away.
        assert!(c.sample(0.5) < 1.0);
    }

    /// A straight ramp with unevenly-spaced keys must stay a straight ramp —
    /// this is exactly what uniform Catmull-Rom gets wrong and why the
    /// tangents are time-scaled.
    #[test]
    fn cubic_does_not_overshoot_uneven_collinear_keys() {
        let c = track(&[
            (0.0, 0.0, Interp::Cubic),
            (0.1, 1.0, Interp::Cubic),
            (5.0, 50.0, Interp::Cubic),
        ]);
        for i in 0..=50 {
            let t = i as f32 * 0.1;
            let expected = t * 10.0;
            assert!(
                (c.sample(t) - expected).abs() < 1e-2,
                "t={t}: {} != {expected}",
                c.sample(t)
            );
        }
    }

    #[test]
    fn loading_sorts_unsorted_keys_and_stacked_keys_step() {
        let text = r#"(
            version: 1,
            tracks: [(
                slug: "z",
                label: "Z",
                keys: [
                    (t: 2.0, value: 20.0, interp: Linear),
                    (t: 0.0, value: 0.0, interp: Linear),
                    (t: 1.0, value: 10.0, interp: Linear),
                ],
            )],
        )"#;
        let doc = parse_curve(text).expect("parse");
        let ts: Vec<f32> = doc.tracks[0].keys.iter().map(|k| k.t).collect();
        assert_eq!(ts, vec![0.0, 1.0, 2.0], "unsorted input is canonicalized on load");
        assert_eq!(doc.duration(), 2.0);

        let mut stacked = track(&[
            (1.0, 1.0, Interp::Linear),
            (1.0, 9.0, Interp::Linear),
            (2.0, 9.0, Interp::Linear),
        ]);
        stacked.canonicalize();
        assert_eq!(stacked.sample(1.0), 9.0, "stacked keys are a step; the later one wins");
    }

    #[test]
    fn round_trip_is_a_fixed_point() {
        let mut doc = CurveDoc::default();
        doc.tracks = vec![
            track(&[(0.0, 0.0, Interp::Cubic), (1.0, 3.0, Interp::Constant)]),
            Track::new("wobble", "Wobble"),
        ];
        let text = serialize_curve(&doc).expect("serialize");
        let back = parse_curve(&text).expect("parse");
        assert_eq!(back, doc);
        assert_eq!(serialize_curve(&back).unwrap(), text, "saving twice does not churn");
    }

    #[test]
    fn a_future_version_is_refused_with_a_useful_error() {
        let text = format!("(version: {}, tracks: [])", CURVE_DOC_VERSION + 1);
        assert_eq!(
            parse_curve(&text),
            Err(CurveIoError::FutureVersion {
                found: CURVE_DOC_VERSION + 1,
                supported: CURVE_DOC_VERSION
            })
        );
    }

    #[test]
    fn slugs_are_identifiers_and_collisions_get_a_suffix() {
        assert_eq!(slugify("Duck Height"), "duck_height");
        assert_eq!(slugify("  2nd pass!! "), "t2nd_pass");
        assert_eq!(slugify("***"), "track");

        let mut doc = CurveDoc::default();
        doc.tracks = vec![Track::new("height", "Height")];
        assert_eq!(doc.free_slug("Height"), "height_2");
        assert_eq!(doc.free_slug("Other"), "other");
    }
}
