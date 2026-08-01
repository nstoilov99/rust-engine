//! Graph-canvas preferences — the `graph` section of `editor_prefs.ron`.
//!
//! These are **author taste, not theme**: no color lives here, nothing swaps
//! with a preset, and there is exactly one preferences file (never a second
//! `graph_prefs.ron`). Every field is `serde(default)` so an older prefs file
//! keeps parsing as the set grows.
//!
//! Not all of it is consumed yet. Phase 3 (the router port) reads
//! `style`, `horizontal_offset`, `turn_anchor`, `corner_radius`,
//! `backward_lane_threshold`, `bundle_offset`, `disable_pin_offset` and
//! `curve`. The rest — crossings, exec override, flow bubbles,
//! `bundle_merge_offset`/`bundle_max`, and the `Node`/`Pin` turn priorities —
//! is stored and serialized now so the settings surface (Phase 4) and the
//! later routing work land against a stable schema.
//!
//! `min_dist` / `min_dist_style` are deliberately absent: continuous
//! degradation plus vertical-then-45° covers every `dx >= 0`, the backward
//! lane owns `dx <= backward_lane_threshold`, and the band between them is
//! on-grid — nothing could ever reach a distance floor, and a documented
//! preference nothing can trigger must not ship (spec v1.2.3).

use serde::{Deserialize, Serialize};

/// How a wire gets from one pin to the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum WireStyle {
    /// Symmetric cubic. The default; its tangent is tuned, do not retune it.
    #[default]
    Spline,
    /// Horizontal/vertical runs, 90° corners rounded by `corner_radius`.
    Manhattan,
    /// Horizontal runs joined by 45° diagonals — the preferred orthogonal
    /// mode; the diagonal is what keeps parallel wires from coinciding.
    Subway,
}

impl WireStyle {
    pub const ALL: [WireStyle; 3] = [WireStyle::Spline, WireStyle::Manhattan, WireStyle::Subway];

    pub fn label(self) -> &'static str {
        match self {
            WireStyle::Spline => "Spline",
            WireStyle::Manhattan => "Manhattan",
            WireStyle::Subway => "Subway",
        }
    }

    /// Spline is a curve; the other two are polylines through corners.
    pub fn is_orthogonal(self) -> bool {
        !matches!(self, WireStyle::Spline)
    }
}

/// Which node the turn is anchored to. The turn belongs to a *node*, not to
/// the span: every wire sharing the anchored node turns at the same x
/// whatever its length, which is what keeps bundles parallel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TurnAnchor {
    /// Turn `horizontal_offset` before the target — suits many-converge-on-one.
    #[default]
    Target,
    /// Turn `horizontal_offset` after the source — suits one-fans-to-many.
    Source,
}

impl TurnAnchor {
    pub const ALL: [TurnAnchor; 2] = [TurnAnchor::Target, TurnAnchor::Source];

    pub fn label(self) -> &'static str {
        match self {
            TurnAnchor::Target => "Target",
            TurnAnchor::Source => "Source",
        }
    }
}

/// Where the turn lands within the anchor's lane. `None` is the only one that
/// preserves span-independence, and it is the cheapest; the other two
/// reintroduce per-wire variation and are opt-in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TurnPriority {
    /// Turn at exactly `horizontal_offset`.
    #[default]
    None,
    /// Nudge to a lane clearing both bounding boxes. Not consumed yet.
    Node,
    /// Align to pin positions. Not consumed yet.
    Pin,
}

impl TurnPriority {
    pub const ALL: [TurnPriority; 3] =
        [TurnPriority::None, TurnPriority::Node, TurnPriority::Pin];

    pub fn label(self) -> &'static str {
        match self {
            TurnPriority::None => "None",
            TurnPriority::Node => "Node",
            TurnPriority::Pin => "Pin",
        }
    }
}

/// How a lower-priority wire is interrupted where wires cross. Not consumed
/// yet (the broadphase lands with the crossing pass).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum CrossingStyle {
    #[default]
    None,
    Gap,
    Arc,
    Circle,
}

impl CrossingStyle {
    pub const ALL: [CrossingStyle; 4] = [
        CrossingStyle::None,
        CrossingStyle::Gap,
        CrossingStyle::Arc,
        CrossingStyle::Circle,
    ];

    pub fn label(self) -> &'static str {
        match self {
            CrossingStyle::None => "None",
            CrossingStyle::Gap => "Gap",
            CrossingStyle::Arc => "Arc",
            CrossingStyle::Circle => "Circle",
        }
    }
}

/// Exec wires may route with their own style/anchor/priority, separating
/// control flow from data flow by *shape* as well as color and width.
/// Suggested when enabled: Manhattan · Target · Node. Not consumed yet.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExecWirePrefs {
    pub style: WireStyle,
    pub turn_anchor: TurnAnchor,
    pub priority: TurnPriority,
}

impl Default for ExecWirePrefs {
    fn default() -> Self {
        Self {
            style: WireStyle::Manhattan,
            turn_anchor: TurnAnchor::Target,
            priority: TurnPriority::Node,
        }
    }
}

/// Flow bubbles — the one motion signal on an executing wire (dashes are
/// retired from execution). Debug-session only, never on an idle graph.
/// Not consumed yet.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BubblePrefs {
    pub enabled: bool,
    /// Bubble radius, px.
    pub size: f32,
    /// Travel speed, px/s.
    pub speed: f32,
    /// Spacing between bubbles along the wire, px.
    pub spacing: f32,
    /// Exec wires only (the default) vs. every wire.
    pub exec_only: bool,
}

impl Default for BubblePrefs {
    fn default() -> Self {
        Self {
            enabled: true,
            size: 4.0,
            speed: 150.0,
            spacing: 40.0,
            exec_only: true,
        }
    }
}

/// Wire routing + appearance preferences.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WirePrefs {
    pub style: WireStyle,
    /// Distance from the anchored node's *border* at which the wire turns.
    /// Fixed and span-independent — this is what makes bundles parallel.
    pub horizontal_offset: f32,
    pub turn_anchor: TurnAnchor,
    /// One radius, two jobs (recorded ruling): the `2 * corner_radius`
    /// near-horizontal straightness threshold and the actual corner rounding.
    /// They were tuned equal; a second field would let them drift.
    pub corner_radius: f32,
    pub priority: TurnPriority,
    /// Turn hard at the border instead of `horizontal_offset` away from it.
    pub disable_pin_offset: bool,
    /// Below this `dx` the wire takes the backward lane. Negative, and named
    /// rather than a magic `-24` buried in the router.
    pub backward_lane_threshold: f32,
    /// Manhattan stagger per target pin row, so N wires into one node render
    /// as N distinguishable verticals instead of one thick bus.
    pub bundle_offset: f32,
    /// Extra offset where ribbons join. Not consumed yet.
    pub bundle_merge_offset: f32,
    /// Above this many shared-lane wires, draw coincident. Not consumed yet.
    pub bundle_max: u32,
    pub crossing: CrossingStyle,
    pub exec_overwrite: Option<ExecWirePrefs>,
    pub bubbles: BubblePrefs,
    /// Spline tangent shape. Tuned; the spec says do not retune it.
    pub curve: f32,
}

impl Default for WirePrefs {
    fn default() -> Self {
        Self {
            style: WireStyle::Spline,
            horizontal_offset: 16.0,
            turn_anchor: TurnAnchor::Target,
            corner_radius: 10.0,
            priority: TurnPriority::None,
            disable_pin_offset: false,
            backward_lane_threshold: -24.0,
            bundle_offset: 4.0,
            bundle_merge_offset: 20.0,
            bundle_max: 8,
            crossing: CrossingStyle::None,
            exec_overwrite: None,
            bubbles: BubblePrefs::default(),
            curve: 0.55,
        }
    }
}

impl WirePrefs {
    /// The turn offset actually used, honoring `disable_pin_offset`.
    #[inline]
    pub fn offset(&self) -> f32 {
        if self.disable_pin_offset {
            0.0
        } else {
            self.horizontal_offset
        }
    }
}

/// The `graph` section of `editor_prefs.ron`: canvas prefs and their natural
/// neighbours, one struct so the settings sidebar has one place to point at.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GraphPrefs {
    pub zoom_min: f32,
    pub zoom_max: f32,
    pub wires: WirePrefs,
}

impl Default for GraphPrefs {
    fn default() -> Self {
        Self {
            zoom_min: 0.15,
            zoom_max: 2.2,
            wires: WirePrefs::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_spec() {
        let w = WirePrefs::default();
        assert_eq!(w.style, WireStyle::Spline);
        assert_eq!(w.turn_anchor, TurnAnchor::Target);
        assert_eq!(w.priority, TurnPriority::None);
        assert_eq!(w.horizontal_offset, 16.0);
        assert_eq!(w.corner_radius, 10.0);
        assert_eq!(w.backward_lane_threshold, -24.0);
        assert_eq!(w.bundle_offset, 4.0);
        assert_eq!(w.bundle_merge_offset, 20.0);
        assert_eq!(w.bundle_max, 8);
        assert_eq!(w.crossing, CrossingStyle::None);
        assert_eq!(w.exec_overwrite, None);
        assert_eq!(w.curve, 0.55);
        assert!(!w.disable_pin_offset);
        assert_eq!(w.offset(), 16.0);

        let b = w.bubbles;
        assert_eq!((b.size, b.speed, b.spacing), (4.0, 150.0, 40.0));
        assert!(b.exec_only);
    }

    #[test]
    fn disable_pin_offset_turns_at_the_border() {
        let w = WirePrefs { disable_pin_offset: true, ..WirePrefs::default() };
        assert_eq!(w.offset(), 0.0);
    }

    /// Every field is `serde(default)`, so a prefs file written before a
    /// field existed still parses — and a full round trip is lossless.
    #[test]
    fn partial_and_full_ron_round_trip() {
        let sparse: GraphPrefs = ron::from_str("(zoom_max: 3.0)").expect("sparse parse");
        assert_eq!(sparse.zoom_max, 3.0);
        assert_eq!(sparse.zoom_min, GraphPrefs::default().zoom_min);
        assert_eq!(sparse.wires, WirePrefs::default());

        // An empty section is the default section.
        let empty: GraphPrefs = ron::from_str("()").expect("empty parse");
        assert_eq!(empty, GraphPrefs::default());

        let mut full = GraphPrefs::default();
        full.wires.style = WireStyle::Subway;
        full.wires.exec_overwrite = Some(ExecWirePrefs::default());
        full.wires.crossing = CrossingStyle::Arc;
        let text = ron::ser::to_string_pretty(&full, Default::default()).unwrap();
        assert_eq!(ron::from_str::<GraphPrefs>(&text).unwrap(), full);
    }
}
