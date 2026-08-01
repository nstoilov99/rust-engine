//! Graph editor canvas panel (Task 40 P5; design-system Phase 2).
//!
//! Draws the pan/zoom node canvas — two-level world grid, nodes (flat header,
//! 2px category top edge, derived mono tag), shaped typed pins, inline
//! `PropValue` widgets, bezier wires, selection outlines and a validation
//! error overlay — and handles all mouse interaction (drag nodes, connect
//! pins, marquee select, right-click create menu). Keyboard editing
//! (undo/redo, delete, copy/paste/duplicate, save) is handled here only for
//! float windows (`handle_shortcuts`); docked graphs route through the main
//! window's menu / winit path.
//!
//! Every color comes from the theme's token module and every metric derives
//! from the design system's base values × `ui_scale` (see [`GraphMetrics`]) —
//! no raw hex, no bare pixel literals in the node path. Detail is governed by
//! one [`ZoomLod`] value derived once per frame, never by scattered zoom
//! comparisons.

use std::collections::{BTreeMap, BTreeSet};

use crusty_gui::context::{Direction, Ui, UiOptions};
use crusty_gui::id::Id;
use crusty_gui::input::{Key, Modifiers};
use crusty_gui::math::{Color, Pos2, Rect, Rounding, Vec2};
use crusty_gui::paint::Painter;
use crusty_gui::style::Style;
use crusty_gui::text::FontFamily;
use crusty_gui::widgets::{
    Button, Canvas, CanvasScope, CanvasView, Checkbox, DragValue, TextEdit,
};

use super::graph_editor::{
    anchored_comments, frame_view, nodes_captured_by_rect, prop_display, AlignMode, Annotation,
    AnnotationDrag, AnnotationEdit, AnnotationResize, ConnectDrag, GraphEdit, GraphEditorState,
    GraphFragment, MarqueeMode, NodeDrag, ResizeHandle, ANNOTATION_MIN_H, ANNOTATION_MIN_W,
    BOOKMARK_SLOTS,
};
use super::graph_prefs::{WirePrefs, WireStyle};
use super::graph_wire_router::{
    self as router, point_polyline_distance, RouteMeta,
};
use super::theme::{
    category_color, category_tag_color, grid_major, grid_minor, pin_color, ramp, wire_color,
    Palette, GRID_MAJOR_STEP, GRID_MINOR_MIN_ZOOM, GRID_MINOR_STEP,
};
use super::widgets::segmented_control;
use crate::engine::node_graph::{
    reroute_type, Edge, ErrorAnchor, GraphError, GraphResolver, NodeDescriptor, NodeRegistry,
    PinType, PropValue, REROUTE_IN, REROUTE_OUT, REROUTE_TYPE_ID, SUBGRAPH_TYPE_ID,
};

// ---------------------------------------------------------------------------
// Base metrics — the design system's node numbers at ui_scale 1.0, world units.
// ---------------------------------------------------------------------------

const BASE_ROW_H: f32 = 22.0;
const BASE_HEADER_H: f32 = 26.0;
const BASE_BODY_PAD: f32 = 6.0;
const BASE_MIN_W: f32 = 128.0;
const BASE_MAX_W: f32 = 320.0;
const BASE_RADIUS: f32 = 6.0;
/// Pins sit fully inside the border, Unreal-style.
const BASE_PIN_INSET: f32 = 6.0;
/// Draw radius 4.5 (9px across); the exec triangle is 11px across.
const BASE_PIN_R: f32 = 4.5;
const BASE_EXEC_W: f32 = 11.0;
const BASE_DIAMOND_W: f32 = 8.0;
const BASE_COLOR_SQ_W: f32 = 9.0;
const BASE_TAG_PX: f32 = 9.0;
const BASE_PAD_X: f32 = 8.0;
const BASE_COL_GAP: f32 = 12.0;
const BASE_LABEL_GAP: f32 = 4.0;
/// Reserved width for an inline value/widget cell.
const BASE_VALUE_W: f32 = 56.0;
const BASE_COMMENT_BAR: f32 = 18.0;
const BASE_GROUP_BAR: f32 = 20.0;
/// Pin hit target: never smaller than this in world units…
const BASE_HIT_W: f32 = 13.5;
/// …and never smaller than this in *screen* pixels (9px each side, the
/// "a pin is never harder to grab than a wire" rule).
const HIT_SCREEN_W: f32 = 18.0;
/// L4 collapses a node to a bar of its type color.
const L4_BAR_H: f32 = 4.0;
/// Annotation titles keep rendering below L2, down to this floor size.
const ANNOTATION_FLOOR_PX: f32 = 7.0;
/// Middle truncation keeps this fraction of the head (DESIGN-panels ▸ names).
const TRUNCATE_HEAD: f32 = 0.60;
/// Marquee fill alpha (1px accent border + 8% accent fill).
const MARQUEE_FILL_ALPHA: f32 = 0.08;
/// Non-primary members of a multi-selection draw their outline at 55%.
const SELECTION_REST_ALPHA: f32 = 0.55;
/// Hollow (unconnected) pin ring width, world units at ui_scale 1.0.
const BASE_RING_W: f32 = 1.5;

// --- Annotation tints. A group is a translucent region *containing* things
// (6% body wash + 45% border); a comment is an opaque card *next to* them, so
// its tint only reaches the NOTE bar and a 1px left edge. Distinguishable at
// any zoom, and both can carry color.
const GROUP_WASH_ALPHA: f32 = 0.06;
const GROUP_BORDER_ALPHA: f32 = 0.45;
/// An untinted group keeps its original neutral wash.
const GROUP_UNTINTED_WASH_ALPHA: f32 = 0.12;
/// Screen-space grab band for annotation resize handles.
const RESIZE_GRAB_PX: f32 = 8.0;

// --- Wire strokes. Screen pixels, deliberately zoom-INVARIANT: the routed
// geometry scales with the view, the stroke does not, so a wire is legible
// (and grabbable) at 15% and not a slab at 220%.
const WIRE_DATA: f32 = 1.9;
const WIRE_EXEC: f32 = 2.4;
const WIRE_DATA_SELECTED: f32 = 2.6;
const WIRE_EXEC_SELECTED: f32 = 3.0;
/// A wire must never be harder to grab than a pin: 9 screen px each side, at
/// every zoom (the pin's hit radius is also 9).
const WIRE_HOVER_PX: f32 = 9.0;

// ---------------------------------------------------------------------------
// Zoom LOD ladder — derived once, consumed everywhere.
// ---------------------------------------------------------------------------

/// DESIGN-nodegraph ▸ Zoom LOD. Ordered least-to-most detail so call sites
/// read as `lod >= ZoomLod::L2`, and derived at exactly one place
/// ([`ZoomLod::from_zoom`]) instead of scattered `if zoom > …` tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ZoomLod {
    /// < 15% — 4px type bars and 1px wires, nothing else.
    L4,
    /// 15–35% — header block + type edge only; no glyphs.
    L3,
    /// 35–60% — pin labels drop; the node title survives.
    L2,
    /// 60–90% — inline widgets fall back to plain values.
    L1,
    /// 90–220% — everything.
    L0,
}

impl ZoomLod {
    pub fn from_zoom(zoom: f32) -> Self {
        if zoom >= 0.90 {
            Self::L0
        } else if zoom >= 0.60 {
            Self::L1
        } else if zoom >= 0.35 {
            Self::L2
        } else if zoom >= 0.15 {
            Self::L3
        } else {
            Self::L4
        }
    }

    /// Editable widgets on the canvas (L0 only).
    fn inline_widgets(self) -> bool {
        self == Self::L0
    }
    /// Inline constants render as plain text (L1 and up).
    fn values(self) -> bool {
        self >= Self::L1
    }
    /// Pin labels (L1 and up — L2 is where they drop).
    fn pin_labels(self) -> bool {
        self >= Self::L1
    }
    /// Pin rows exist at all (L2 and up); below that a node is its header.
    fn rows(self) -> bool {
        self >= Self::L2
    }
    /// Any glyph may be submitted (L2 and up). Annotation titles are the one
    /// documented exception and ignore this.
    fn glyphs(self) -> bool {
        self >= Self::L2
    }
    /// L4: the node is a bar of its type color.
    fn bar_only(self) -> bool {
        self == Self::L4
    }
}

// ---------------------------------------------------------------------------
// Error anchoring
// ---------------------------------------------------------------------------

/// This frame's errors, resolved to the thing each one is *about*. Built once
/// from `state.errors` + `state.ref_errors` and read by both the geometry
/// pass (ghost rows change a node's shape) and the paint pass.
#[derive(Default)]
struct ErrorIndex {
    /// Nodes carrying a border + gutter badge.
    nodes: BTreeSet<u64>,
    /// Pins carrying an error ring, keyed `(node, slug, is_output)`.
    pins: BTreeSet<(u64, String, bool)>,
    /// Edges drawn in `status.error` with an x at their midpoint.
    edges: BTreeSet<Edge>,
    /// Ghost rows to append, `node -> [(slug, is_output)]`.
    ghosts: BTreeMap<u64, Vec<(String, bool)>>,
    /// Errors with nowhere on the canvas to live — the compiler rows.
    document: Vec<GraphError>,
    /// Every error, in a stable order, for the count chip's cycle.
    ordered: Vec<GraphError>,
}

impl ErrorIndex {
    fn build(doc_errors: &[GraphError], ref_errors: &[GraphError]) -> Self {
        let mut ix = ErrorIndex::default();
        for e in doc_errors.iter().chain(ref_errors.iter()) {
            match e.anchor() {
                ErrorAnchor::Node(id) => {
                    ix.nodes.insert(id);
                }
                ErrorAnchor::Pin { node, pin, output } => {
                    ix.pins.insert((node, pin, output));
                }
                ErrorAnchor::Edge(edge) => {
                    ix.edges.insert(edge);
                }
                ErrorAnchor::GhostPin { node, pin, output } => {
                    let rows = ix.ghosts.entry(node).or_default();
                    if !rows.iter().any(|(s, o)| *s == pin && *o == output) {
                        rows.push((pin, output));
                    }
                }
                ErrorAnchor::Document => ix.document.push(e.clone()),
            }
            ix.ordered.push(e.clone());
        }
        ix
    }

    fn total(&self) -> usize {
        self.ordered.len()
    }

    fn is_empty(&self) -> bool {
        self.ordered.is_empty()
    }

    fn ghosts_for(&self, node: u64) -> &[(String, bool)] {
        self.ghosts.get(&node).map(Vec::as_slice).unwrap_or(&[])
    }
}

// ---------------------------------------------------------------------------
// Resolved metrics
// ---------------------------------------------------------------------------

/// The node metrics for this frame: every design-system base value multiplied
/// by the active `ui_scale`, in world units (= pixels at zoom 1.0).
///
/// The canvas draws in world space and can only see crusty's [`Style`], whose
/// metrics arrive from the engine theme *already* scaled. `row_height` has a
/// spec-fixed base of 22, so it is the honest place to recover the factor.
struct GraphMetrics {
    scale: f32,
    header_h: f32,
    row_h: f32,
    body_pad: f32,
    min_w: f32,
    max_w: f32,
    radius: f32,
    /// 1px border, screen-space (a hairline must not vanish when zoomed out).
    border: f32,
    /// The reserved 2px edge — category top edge and selection outline.
    edge: f32,
    pin_inset: f32,
    pin_r: f32,
    exec_w: f32,
    diamond_w: f32,
    color_sq_w: f32,
    ring_w: f32,
    tag_px: f32,
    pad_x: f32,
    col_gap: f32,
    label_gap: f32,
    value_w: f32,
    comment_bar: f32,
    group_bar: f32,
}

impl GraphMetrics {
    fn new(st: &Style) -> Self {
        let s = (st.metrics.row_height / BASE_ROW_H).max(0.1);
        Self {
            scale: s,
            header_h: BASE_HEADER_H * s,
            row_h: st.metrics.row_height,
            body_pad: BASE_BODY_PAD * s,
            min_w: BASE_MIN_W * s,
            max_w: BASE_MAX_W * s,
            radius: BASE_RADIUS * s,
            border: st.metrics.border,
            edge: st.metrics.edge_accent,
            pin_inset: BASE_PIN_INSET * s,
            pin_r: BASE_PIN_R * s,
            exec_w: BASE_EXEC_W * s,
            diamond_w: BASE_DIAMOND_W * s,
            color_sq_w: BASE_COLOR_SQ_W * s,
            ring_w: BASE_RING_W * s,
            tag_px: BASE_TAG_PX * s,
            pad_x: BASE_PAD_X * s,
            col_gap: BASE_COL_GAP * s,
            label_gap: BASE_LABEL_GAP * s,
            value_w: BASE_VALUE_W * s,
            comment_bar: BASE_COMMENT_BAR * s,
            group_bar: BASE_GROUP_BAR * s,
        }
    }

    /// Diameter of a reroute's dot, world units.
    fn reroute_d(&self) -> f32 {
        self.pin_r * 3.2
    }

    /// Where a pin's label column starts, measured from the node border.
    fn label_inset(&self) -> f32 {
        self.pin_inset + self.pin_r + self.label_gap
    }

    /// World-space half-extent of a pin's hit target: at least 13.5 world
    /// units, and at least 18 screen px however far the view is zoomed out.
    fn pin_hit_w(&self, zoom: f32) -> f32 {
        (BASE_HIT_W * self.scale).max(HIT_SCREEN_W / zoom.max(0.01))
    }
}

/// Everything the panel needs, bundled so the signature stays small.
pub struct GraphEditorPanelCtx<'a> {
    pub state: &'a mut GraphEditorState,
    pub registry: &'a NodeRegistry,
    pub clipboard: &'a mut Option<GraphFragment>,
    /// Resolves subgraph references (open docs + disk) for pin derivation.
    pub resolver: &'a dyn GraphResolver,
    /// Content-relative paths of known `.subgraph` assets (create menu).
    pub subgraph_assets: &'a [String],
    /// Set to a content-relative path when a subgraph node is double-clicked;
    /// the host opens it as a tab (P6 open-in-tab navigation).
    pub open_subgraph: &'a mut Option<String>,
    /// `selection.outline` from the live theme. Passed in because crusty's
    /// `Style` has no counterpart and Graphite overrides the invariant.
    pub selection_outline: Color,
    /// Wire routing + appearance, the `graph.wires` prefs section.
    pub wire_prefs: WirePrefs,
    /// Set when the toolbar's segmented control picks a different wire style;
    /// the host writes it back to `prefs.graph.wires.style`, which is what
    /// makes it show the overridden dot in Preferences and autosave.
    pub wire_style_request: &'a mut Option<WireStyle>,
    /// Canvas zoom limits (EditorPrefs, P9).
    pub zoom_min: f32,
    pub zoom_max: f32,
    /// This tab is the focused tab of its dock (gates keyboard editing).
    pub focused: bool,
    /// True in float windows (no menu/winit edit path) — the panel handles
    /// keyboard editing itself. False when docked in the main window.
    pub handle_shortcuts: bool,
}

// ---------------------------------------------------------------------------
// Geometry
// ---------------------------------------------------------------------------

/// What an unconnected input renders in its value cell.
#[derive(Clone, Debug, PartialEq)]
enum InlineKind {
    /// Editable at L0 (`DragValue`).
    Float(f32),
    /// Editable at L0 (`Checkbox`).
    Bool(bool),
    /// Painted swatch + hex; not yet editable.
    Color([f32; 4]),
    /// Painted mono chip (asset path, enum variant, vector tuple).
    Chip(String),
    /// Forward-compat data: warning-dashed `preserved` chip, never a blank.
    Raw(String),
}

impl InlineKind {
    fn of(v: &PropValue) -> Self {
        match v {
            PropValue::Float(x) => InlineKind::Float(*x),
            PropValue::Bool(b) => InlineKind::Bool(*b),
            PropValue::Color(c) => InlineKind::Color(*c),
            PropValue::Raw(s) => InlineKind::Raw(s.clone()),
            other => InlineKind::Chip(prop_display(other)),
        }
    }
}

struct PinGeom {
    slug: String,
    label: String,
    ty: PinType,
    output: bool,
    /// Row index within its column — the Manhattan bundle stagger.
    row: usize,
    /// Row center on the node *border* — where a wire terminates.
    wire_anchor: Pos2,
    /// Drawn dot center, `pin_inset` inside the border.
    dot_center: Pos2,
    connected: bool,
    /// Inline constant on an unconnected input, if any.
    inline: Option<InlineKind>,
    /// A dashed placeholder for a pin the descriptor no longer declares, so
    /// the edge pointing at it has somewhere to land instead of vanishing.
    ghost: bool,
}

struct NodeGeom {
    id: u64,
    /// Full node box at L2 and up.
    rect: Rect,
    /// Title, already middle-truncated to fit the header.
    title: String,
    /// Derived 9px mono tag (SUB / PURE / EVENT / first-5-of-category).
    tag: String,
    category: Option<String>,
    /// Per-node ramp-index override for the 2px edge.
    tint: Option<u8>,
    missing: bool,
    /// Carries a validation error: error border + one gutter badge.
    errored: bool,
    /// A reroute: a bare typed dot, no header, no rows.
    reroute: bool,
    pins: Vec<PinGeom>,
}

impl NodeGeom {
    fn wire_anchor(&self, slug: &str, output: bool) -> Option<Pos2> {
        self.pins
            .iter()
            .find(|p| p.output == output && p.slug == slug)
            .map(|p| p.wire_anchor)
    }

    /// Row index of a pin, for the router's bundle stagger. Unknown pins
    /// answer 0 — the unstaggered lane, which is the safe default.
    fn pin_row(&self, slug: &str, output: bool) -> usize {
        self.pins
            .iter()
            .find(|p| p.output == output && p.slug == slug)
            .map_or(0, |p| p.row)
    }

    /// The box actually drawn (and hit-tested) at this detail level: below
    /// L2 a node collapses to its header, so rows never render as mush.
    fn body_rect(&self, lod: ZoomLod, m: &GraphMetrics) -> Rect {
        if lod.rows() {
            self.rect
        } else {
            Rect::from_min_size(self.rect.min, Vec2::new(self.rect.width(), m.header_h))
        }
    }

    /// The 2px top edge's color: the per-node tint if set, else the category
    /// slot's deep tone. Unregistered/missing types keep the error border and
    /// take a neutral edge from `category_color`'s Dev/unknown handling.
    fn edge_color(&self) -> Color {
        if let Some(i) = self.tint {
            return ramp()[(i % 12) as usize].deep;
        }
        category_color(self.category.as_deref().unwrap_or("Dev"))
    }

    fn tag_color(&self) -> Color {
        category_tag_color(self.category.as_deref().unwrap_or("Dev"))
    }
}

/// SUB > PURE > EVENT > category tag — the prototype's precedence. Tags are
/// derived per node, never stored.
fn derive_tag(
    is_subgraph: bool,
    desc: Option<&NodeDescriptor>,
    category: Option<&str>,
) -> String {
    if is_subgraph {
        return "SUB".to_string();
    }
    if let Some(d) = desc {
        if d.pure {
            return "PURE".to_string();
        }
        let exec_out = d.outputs.iter().any(|p| p.ty == PinType::Exec);
        let exec_in = d.inputs.iter().any(|p| p.ty == PinType::Exec);
        if exec_out && !exec_in {
            return "EVENT".to_string();
        }
    }
    category
        .unwrap_or("")
        .chars()
        .take(5)
        .collect::<String>()
        .to_uppercase()
}

/// Shorten `s` from the middle until it fits `max_w`, keeping ~60% of the
/// head and 40% of the tail — generated names differ at the end.
fn middle_truncate(p: &mut Painter, s: &str, px: f32, max_w: f32) -> String {
    if max_w <= 0.0 || p.measure_text(s, px, None).x <= max_w {
        return s.to_string();
    }
    let chars: Vec<char> = s.chars().collect();
    // Binary search the largest kept-character count that fits.
    let (mut lo, mut hi) = (0usize, chars.len());
    let mut best = String::from("\u{2026}");
    while lo <= hi {
        let keep = (lo + hi) / 2;
        if keep == 0 {
            break;
        }
        let head = ((keep as f32) * TRUNCATE_HEAD).round() as usize;
        let tail = keep - head;
        let mut candidate: String = chars[..head].iter().collect();
        candidate.push('\u{2026}');
        candidate.extend(chars[chars.len() - tail..].iter());
        if p.measure_text(&candidate, px, None).x <= max_w {
            best = candidate;
            lo = keep + 1;
        } else {
            if keep == 0 {
                break;
            }
            hi = keep - 1;
        }
    }
    best
}

#[allow(clippy::too_many_arguments)]
fn build_geoms(
    state: &GraphEditorState,
    registry: &NodeRegistry,
    resolver: &dyn GraphResolver,
    errors: &ErrorIndex,
    m: &GraphMetrics,
    st: &Style,
    p: &mut Painter,
) -> Vec<NodeGeom> {
    let title_px = st.fonts.body;
    let label_px = st.fonts.small;

    state
        .doc
        .nodes
        .iter()
        .map(|n| -> NodeGeom {
            let min = Pos2::new(n.position[0], n.position[1]);
            let is_sub = n.type_id == SUBGRAPH_TYPE_ID;
            let is_reroute = n.type_id == REROUTE_TYPE_ID;
            let desc = (!is_sub && !is_reroute)
                .then(|| registry.get(&n.type_id))
                .flatten();

            // A reroute is a bare pass-through: one in, one out, no header,
            // no rows, and a type inferred from whatever feeds it.
            if is_reroute {
                let d = m.reroute_d();
                let rect = Rect::from_min_size(min, Vec2::splat(d));
                let ty = reroute_type(&state.doc, registry, n.id)
                    .unwrap_or(PinType::Domain(String::new()));
                let c = Pos2::new(min.x + d * 0.5, min.y + d * 0.5);
                let pin = |slug: &str, output: bool, x: f32| PinGeom {
                    slug: slug.to_string(),
                    label: String::new(),
                    ty: ty.clone(),
                    output,
                    row: 0,
                    wire_anchor: Pos2::new(x, c.y),
                    dot_center: c,
                    connected: state.doc.edges.iter().any(|e| {
                        if output {
                            e.from_node == n.id && e.from_pin == slug
                        } else {
                            e.to_node == n.id && e.to_pin == slug
                        }
                    }),
                    inline: None,
                    ghost: false,
                };
                return NodeGeom {
                    id: n.id,
                    rect,
                    title: String::new(),
                    tag: String::new(),
                    category: None,
                    tint: n.tint,
                    missing: false,
                    errored: errors.nodes.contains(&n.id),
                    reroute: true,
                    pins: vec![
                        pin(REROUTE_IN, false, min.x),
                        pin(REROUTE_OUT, true, min.x + d),
                    ],
                };
            }
            #[allow(clippy::type_complexity)]
            let (title, category, missing, inputs, outputs): (
                String,
                Option<String>,
                bool,
                Vec<(String, String, PinType)>,
                Vec<(String, String, PinType)>,
            ) = if is_sub {
                let name = n
                    .subgraph
                    .as_deref()
                    .and_then(|q| std::path::Path::new(q).file_stem())
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| "Subgraph".to_string());
                // Pins derive from the referenced doc's declared interface;
                // an unresolvable reference renders in the missing-node style.
                match n.subgraph.as_deref().and_then(|q| resolver.resolve(q)) {
                    Some(sub) => {
                        let iface = |pins: &[crate::engine::node_graph::IfacePin]| {
                            pins.iter()
                                .map(|q| (q.slug.clone(), q.label.clone(), q.ty.clone()))
                                .collect::<Vec<_>>()
                        };
                        (
                            name,
                            Some("Subgraph".to_string()),
                            false,
                            iface(&sub.inputs),
                            iface(&sub.outputs),
                        )
                    }
                    None => (name, Some("Subgraph".to_string()), true, vec![], vec![]),
                }
            } else if let Some(d) = desc {
                let pins = |v: &[crate::engine::node_graph::PinDescriptor]| {
                    v.iter()
                        .map(|q| (q.slug.clone(), q.label.clone(), q.ty.clone()))
                        .collect::<Vec<_>>()
                };
                (
                    d.name.clone(),
                    Some(d.category.clone()),
                    false,
                    pins(&d.inputs),
                    pins(&d.outputs),
                )
            } else {
                (n.type_id.clone(), None, true, vec![], vec![])
            };

            let tag = derive_tag(is_sub, desc, category.as_deref());

            // --- auto width: widest of the header row and every pin row ---
            let tag_w = p
                .measure_text_family(&tag, m.tag_px, None, FontFamily::Mono)
                .x;
            let header_w = m.pad_x
                + p.measure_text(&title, title_px, None).x
                + m.col_gap
                + tag_w
                + m.pad_x;

            let inline_of = |slug: &str| -> Option<InlineKind> {
                let connected = state
                    .doc
                    .edges
                    .iter()
                    .any(|e| e.to_node == n.id && e.to_pin == slug);
                if connected {
                    return None;
                }
                n.properties.get(slug).map(InlineKind::of)
            };

            // Ghost rows for pins an edge names but the descriptor no longer
            // declares. They append after the real pins on their side, so an
            // otherwise-orphaned wire has somewhere to land.
            let ghosts = errors.ghosts_for(n.id);
            let ghost_in = ghosts.iter().filter(|(_, o)| !*o).count();
            let ghost_out = ghosts.iter().filter(|(_, o)| *o).count();

            let rows = (inputs.len() + ghost_in)
                .max(outputs.len() + ghost_out)
                .max(1);
            let mut content_w: f32 = header_w;
            for i in 0..rows.min(inputs.len().max(outputs.len()).max(1)) {
                let left = inputs
                    .get(i)
                    .map(|(slug, label, _)| {
                        let mut w = m.label_inset() + p.measure_text(label, label_px, None).x;
                        if inline_of(slug).is_some() {
                            w += m.label_gap + m.value_w;
                        }
                        w
                    })
                    .unwrap_or(0.0);
                let right = outputs
                    .get(i)
                    .map(|(_, label, _)| {
                        m.label_inset() + p.measure_text(label, label_px, None).x
                    })
                    .unwrap_or(0.0);
                let gap = if left > 0.0 && right > 0.0 { m.col_gap } else { 0.0 };
                content_w = content_w.max(left + gap + right);
            }
            let width = content_w.clamp(m.min_w, m.max_w);

            // Title fits whatever the (possibly capped) header leaves it.
            let title_avail = width - m.pad_x * 2.0 - tag_w - m.col_gap;
            let title = middle_truncate(p, &title, title_px, title_avail);

            let height = m.header_h + rows as f32 * m.row_h + m.body_pad;
            let rect = Rect::from_min_size(min, Vec2::new(width, height));
            let row_y = |i: usize| min.y + m.header_h + i as f32 * m.row_h + m.row_h * 0.5;

            let mut pins = Vec::new();
            let (in_count, out_count) = (inputs.len(), outputs.len());
            for (i, (slug, label, ty)) in inputs.into_iter().enumerate() {
                let y = row_y(i);
                let inline = inline_of(&slug);
                pins.push(PinGeom {
                    connected: state
                        .doc
                        .edges
                        .iter()
                        .any(|e| e.to_node == n.id && e.to_pin == slug),
                    wire_anchor: Pos2::new(min.x, y),
                    dot_center: Pos2::new(min.x + m.pin_inset, y),
                    slug,
                    label,
                    ty,
                    output: false,
                    row: i,
                    inline,
                    ghost: false,
                });
            }
            for (i, (slug, label, ty)) in outputs.into_iter().enumerate() {
                let y = row_y(i);
                pins.push(PinGeom {
                    connected: state
                        .doc
                        .edges
                        .iter()
                        .any(|e| e.from_node == n.id && e.from_pin == slug),
                    wire_anchor: Pos2::new(min.x + width, y),
                    dot_center: Pos2::new(min.x + width - m.pin_inset, y),
                    slug,
                    label,
                    ty,
                    output: true,
                    row: i,
                    inline: None,
                    ghost: false,
                });
            }
            // Ghost rows last, one per side, continuing that side's rows.
            let (mut gi, mut go) = (in_count, out_count);
            for (slug, output) in ghosts {
                let row = if *output { &mut go } else { &mut gi };
                let y = row_y(*row);
                let x = if *output { min.x + width } else { min.x };
                let dot = if *output {
                    min.x + width - m.pin_inset
                } else {
                    min.x + m.pin_inset
                };
                pins.push(PinGeom {
                    slug: slug.clone(),
                    label: slug.clone(),
                    // A ghost has no declared type — neutral, never a guess.
                    ty: PinType::Domain(String::new()),
                    output: *output,
                    row: *row,
                    wire_anchor: Pos2::new(x, y),
                    dot_center: Pos2::new(dot, y),
                    connected: true,
                    inline: None,
                    ghost: true,
                });
                *row += 1;
            }
            NodeGeom {
                id: n.id,
                rect,
                title,
                tag,
                category,
                tint: n.tint,
                missing,
                errored: errors.nodes.contains(&n.id),
                reroute: is_reroute,
                pins,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Pin shapes
// ---------------------------------------------------------------------------

/// Shape is deliberate redundancy for color-vision deficiency: twelve ramp
/// hues exceed comfortable deuteranopia separation, so shape + label carry
/// the difference where hue cannot.
fn draw_pin(p: &mut Painter, pin: &PinGeom, c: Pos2, zoom: f32, m: &GraphMetrics, st: &Style, reg: &NodeRegistry) {
    if pin.ghost {
        // A ghost has no declared type, so it draws neutral and dashed — the
        // wire lands somewhere, and the row says plainly that it should not.
        let r = m.pin_r * zoom;
        dashed_circle(p, c, r, (m.ring_w * zoom).max(1.0), st.palette.text_disabled);
        return;
    }
    let col = pin_color(Some(reg), &pin.ty);
    let filled = pin.connected;
    let ring = (m.ring_w * zoom).max(1.0);
    match &pin.ty {
        PinType::Exec => {
            // Triangle, 11px across, pointing along the flow direction. An
            // unconnected exec pin stays *filled* in a disabled grey (the
            // prototype's rule) — a hollow triangle reads as a broken glyph.
            let h = m.exec_w * zoom * 0.5;
            let w = h * 0.9;
            let (a, b, d) = (
                Pos2::new(c.x - w, c.y - h),
                Pos2::new(c.x - w, c.y + h),
                Pos2::new(c.x + w, c.y),
            );
            let color = if filled { col } else { st.palette.text_disabled };
            p.triangle(a, b, d, color);
        }
        PinType::Vec2 | PinType::Vec3 | PinType::Vec4 => {
            let r = m.diamond_w * zoom * 0.5;
            let pts = [
                Pos2::new(c.x, c.y - r),
                Pos2::new(c.x + r, c.y),
                Pos2::new(c.x, c.y + r),
                Pos2::new(c.x - r, c.y),
            ];
            if filled {
                p.convex_polygon_filled(pts.to_vec(), col);
            } else {
                p.polygon_stroke(&pts, ring, col);
            }
        }
        PinType::Color => {
            let r = m.color_sq_w * zoom * 0.5;
            let rect = Rect::from_center_size(c, Vec2::splat(r * 2.0));
            let round = Rounding::same((r * 0.45).max(1.0));
            if filled {
                p.rect_filled(rect, round, col);
            } else {
                p.rect_stroke(rect, round, ring, col);
            }
        }
        _ => {
            let r = m.pin_r * zoom;
            if filled {
                p.circle_filled(c, r, col);
            } else {
                p.circle_stroke(c, r - ring * 0.5, ring, col);
            }
        }
    }
}

/// A dashed ring — the ghost-row marker. crusty has no dash support, so the
/// segments are emitted directly.
fn dashed_circle(p: &mut Painter, c: Pos2, r: f32, w: f32, color: Color) {
    const SEGS: usize = 12;
    for i in (0..SEGS).step_by(2) {
        let a0 = i as f32 / SEGS as f32 * std::f32::consts::TAU;
        let a1 = (i + 1) as f32 / SEGS as f32 * std::f32::consts::TAU;
        p.line_segment(
            Pos2::new(c.x + r * a0.cos(), c.y + r * a0.sin()),
            Pos2::new(c.x + r * a1.cos(), c.y + r * a1.sin()),
            w,
            color,
        );
    }
}

/// A dashed horizontal rule under a ghost row.
fn dashed_line(p: &mut Painter, a: Pos2, b: Pos2, w: f32, color: Color) {
    let n = ((b.x - a.x).abs() / (w * 4.0).max(2.0)).clamp(2.0, 64.0) as usize;
    for i in (0..n).step_by(2) {
        let t0 = i as f32 / n as f32;
        let t1 = ((i + 1) as f32 / n as f32).min(1.0);
        p.line_segment(
            Pos2::new(a.x + (b.x - a.x) * t0, a.y + (b.y - a.y) * t0),
            Pos2::new(a.x + (b.x - a.x) * t1, a.y + (b.y - a.y) * t1),
            w,
            color,
        );
    }
}

/// Vector pins carry their arity in the label (`Position ·3`) — the pin
/// *shape* is shared across Vec2/3/4, so the label disambiguates.
fn pin_label(pin: &PinGeom) -> String {
    match pin.ty {
        PinType::Vec2 => format!("{} \u{b7}2", pin.label),
        PinType::Vec3 => format!("{} \u{b7}3", pin.label),
        PinType::Vec4 => format!("{} \u{b7}4", pin.label),
        _ => pin.label.clone(),
    }
}

// ---------------------------------------------------------------------------
// Connection validity
// ---------------------------------------------------------------------------

/// If a wire between two pins is legal, return the normalized edge
/// (output → input). No implicit conversions; exec only to exec (type
/// equality covers both); the input side must be currently unconnected.
#[allow(clippy::too_many_arguments)]
fn validate_connection(
    state: &GraphEditorState,
    a_node: u64,
    a_slug: &str,
    a_out: bool,
    a_ty: &PinType,
    b_node: u64,
    b_slug: &str,
    b_out: bool,
    b_ty: &PinType,
) -> Option<Edge> {
    if a_node == b_node || a_out == b_out || a_ty != b_ty {
        return None;
    }
    let (from_node, from_pin, to_node, to_pin) = if a_out {
        (a_node, a_slug, b_node, b_slug)
    } else {
        (b_node, b_slug, a_node, a_slug)
    };
    if state
        .doc
        .edges
        .iter()
        .any(|e| e.to_node == to_node && e.to_pin == to_pin)
    {
        return None;
    }
    Some(Edge {
        from_node,
        from_pin: from_pin.to_string(),
        to_node,
        to_pin: to_pin.to_string(),
    })
}

// ---------------------------------------------------------------------------
// Panel
// ---------------------------------------------------------------------------

pub fn graph_editor_panel(ui: &mut Ui, ctx: GraphEditorPanelCtx) {
    let GraphEditorPanelCtx {
        state,
        registry,
        clipboard,
        resolver,
        subgraph_assets,
        open_subgraph,
        selection_outline,
        wire_prefs,
        wire_style_request,
        zoom_min,
        zoom_max,
        focused,
        handle_shortcuts,
    } = ctx;

    if handle_shortcuts && focused {
        handle_panel_keys(ui, state, registry, clipboard);
    }

    // Finalize a gesture orphaned by a release that landed while this tab was
    // not being drawn (e.g. the user switched tabs mid-drag): the pointer is
    // already up on re-entry, so `draw_and_interact`'s in-body finish never
    // ran. Finalizing (vs reverting) keeps the edit the user made — the
    // simpler correct choice. No-op when nothing is in flight.
    if !ui.ctx().input.pointer_down {
        finish_node_drag(state, registry);
        finish_annotation_drag(state, registry);
        state.flush_prop_edit(registry);
    }

    // The graph toolbar sits above the canvas and takes its row out of the
    // available space, so the canvas shrinks by exactly the toolbar height and
    // everything measured off the canvas rect (F/A framing, the error overlay)
    // follows automatically.
    graph_toolbar(ui, state, wire_prefs.style, wire_style_request);

    // Canvas needs `&mut CanvasView`; `CanvasView` is Copy, so pass a local
    // copy and write it back — keeps `state` fully borrowable in the body.
    let mut view = state.view;
    let mut menu_open_at: Option<Pos2> = None;
    let mut annotation_menu_at: Option<Pos2> = None;
    let mut wire_menu_at: Option<Pos2> = None;
    let mut collapse_request = false;
    let mut frame_request: Option<CanvasView> = None;
    let out = Canvas::new().zoom_range(zoom_min, zoom_max).show(ui, &mut view, |ui, scope| {
        draw_and_interact(
            ui,
            scope,
            state,
            registry,
            resolver,
            &mut menu_open_at,
            &mut annotation_menu_at,
            &mut wire_menu_at,
            &mut collapse_request,
            open_subgraph,
            selection_outline,
            &wire_prefs,
            zoom_min,
            zoom_max,
            &mut frame_request,
        )
    });
    // F/A frame shortcuts re-fit the view (applied after the canvas ran, so it
    // replaces this frame's pan/zoom rather than fighting the live transform).
    if let Some(v) = frame_request {
        view = v;
    }
    state.view = view;

    create_menu(ui, state, registry, &out.inner, subgraph_assets, menu_open_at);
    annotation_menu(ui, state, registry, annotation_menu_at);
    wire_menu(ui, state, registry, wire_menu_at);
    edit_popup(ui, state, registry, out.rect);
    purge_confirm(ui, out.rect, state, registry);

    // Ctrl+G writes a new asset, so it runs outside the draw pass.
    if collapse_request {
        match state.collapse_to_subgraph(std::path::Path::new("content"), registry) {
            Ok(rel) => println!("graph: collapsed selection into {rel}"),
            Err(e) => println!("graph: collapse to subgraph failed: {e}"),
        }
    }
}

fn handle_panel_keys(
    ui: &Ui,
    state: &mut GraphEditorState,
    registry: &NodeRegistry,
    clipboard: &mut Option<GraphFragment>,
) {
    let (ctrl, shift, del, z, y, c, v, d, s) = {
        let input = &ui.ctx().input;
        let ctrl = input.modifiers.contains(Modifiers::CTRL);
        (
            ctrl,
            input.modifiers.contains(Modifiers::SHIFT),
            input.key_pressed(Key::Delete),
            input.key_pressed(Key::Char('z')),
            input.key_pressed(Key::Char('y')),
            input.key_pressed(Key::Char('c')),
            input.key_pressed(Key::Char('v')),
            input.key_pressed(Key::Char('d')),
            input.key_pressed(Key::Char('s')),
        )
    };
    if del {
        state.delete_selection(registry);
    }
    if ctrl && z {
        if shift {
            state.redo(registry);
        } else {
            state.undo(registry);
        }
    }
    if ctrl && y {
        state.redo(registry);
    }
    if ctrl && c {
        state.copy_selection(clipboard);
    }
    if ctrl && v {
        state.paste_clipboard(clipboard, registry);
    }
    if ctrl && d {
        state.duplicate_selection(registry);
    }
    if ctrl && s {
        let abs = std::path::Path::new("content").join(&state.path);
        if let Err(e) = state.save(&abs) {
            log::warn!("graph save failed: {e}");
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_and_interact(
    ui: &mut Ui,
    scope: &CanvasScope,
    state: &mut GraphEditorState,
    registry: &NodeRegistry,
    resolver: &dyn GraphResolver,
    menu_open_at: &mut Option<Pos2>,
    annotation_menu_at: &mut Option<Pos2>,
    wire_menu_at: &mut Option<Pos2>,
    collapse_request: &mut bool,
    open_subgraph: &mut Option<String>,
    selection_outline: Color,
    wire_prefs: &WirePrefs,
    zoom_min: f32,
    zoom_max: f32,
    frame_request: &mut Option<CanvasView>,
) -> Vec<NodeGeom> {
    let st = ui.style();
    let zoom = scope.zoom();
    let lod = ZoomLod::from_zoom(zoom);
    let m = GraphMetrics::new(&st);
    let vis = scope.visible_world_rect();
    // Errors are resolved to their anchors before geometry, because ghost
    // rows change a node's shape.
    let errors = ErrorIndex::build(&state.errors, &state.ref_errors);
    let geoms = {
        let mut p = ui.painter();
        build_geoms(state, registry, resolver, &errors, &m, &st, &mut p)
    };

    let node_rects: Vec<Rect> = geoms.iter().map(|g| g.rect).collect();
    let wires = build_wires(state, &geoms, &node_rects, &errors, wire_prefs, scope, vis);
    let hovered_wire = wire_under(&wires, ui.ctx().input.pointer_pos);

    draw_grid(ui, scope, &st, vis, zoom);
    draw_annotations(ui, scope, state, &st, &m, vis, zoom, lod);
    draw_wires(ui, scope, &wires, hovered_wire, wire_prefs, &st, lod, selection_outline);

    // Nodes, pins and inline widgets. `widget_rects` records the screen boxes
    // owned by embedded controls so the node-drag pass can yield to them.
    let mut widget_rects: Vec<Rect> = Vec::new();
    draw_nodes(
        ui,
        scope,
        state,
        registry,
        &geoms,
        &errors,
        &st,
        &m,
        vis,
        zoom,
        lod,
        selection_outline,
        &mut widget_rects,
    );

    // Interactions.
    let pointer_world = scope.pointer_world(ui);
    let pointer_screen = ui.ctx().input.pointer_pos;
    let pointer_down = ui.ctx().input.pointer_down;
    let pointer_pressed = ui.ctx().input.pointer_pressed;
    let released = ui.ctx().input.pointer_released;
    let right_pressed = ui.ctx().input.right_pressed;
    let mods = ui.ctx().input.modifiers;
    let shift = mods.contains(Modifiers::SHIFT);
    let widget_claimed = pointer_screen
        .is_some_and(|p| widget_rects.iter().any(|r| r.contains(p)));

    // Frame shortcuts (DCC F/A). Fire only while the pointer is over the
    // canvas, no inline text edit is active, and **no modifier is held** —
    // otherwise Ctrl+A / Ctrl+F over the canvas also framed the view.
    if pointer_world.is_some() && state.editing.is_none() && mods.is_empty() {
        let (frame_all, frame_sel) = {
            let input = &ui.ctx().input;
            (input.key_pressed(Key::Char('a')), input.key_pressed(Key::Char('f')))
        };
        if frame_all || frame_sel {
            // A frames everything. F frames the selection — nodes *or* the
            // selected comment/group (ruling: F applies to all canvas
            // widgets) — falling back to everything when nothing is selected.
            let bbox = if frame_all {
                content_bbox(state, &geoms)
            } else if let Some(b) = selection_bbox(state, &geoms) {
                Some(b)
            } else {
                content_bbox(state, &geoms)
            };
            if let Some((min, max)) = bbox {
                *frame_request =
                    Some(frame_view(min, max, scope.rect().size(), zoom_min, zoom_max));
            }
        }
    }

    // Advance / finish a live node drag. Snapshot the drag data first so the
    // `node_drag` borrow ends before mutating the doc.
    let drag_snapshot = state
        .node_drag
        .as_ref()
        .map(|d| (d.origin_world, d.originals.clone(), d.anchored.clone()));
    if let Some((origin, originals, anchored)) = drag_snapshot {
        if let Some(pw) = pointer_world {
            let (dx, dy) = (pw.x - origin[0], pw.y - origin[1]);
            for (id, start) in &originals {
                if let Some(n) = state.doc.node_mut(*id) {
                    n.position = [start[0] + dx, start[1] + dy];
                }
            }
            for (i, start) in &anchored {
                if let Some(c) = state.doc.comments.get_mut(*i) {
                    c.rect[0] = start[0] + dx;
                    c.rect[1] = start[1] + dy;
                }
            }
        }
        if !pointer_down {
            finish_node_drag(state, registry);
        }
    }

    // Advance / finish a live comment/group drag (P7).
    let ann_snapshot = state.annotation_drag.as_ref().map(|d| {
        (d.is_group, d.index, d.origin_world, d.rect_min0, d.captured.clone())
    });
    if let Some((is_group, index, origin, rect0, captured)) = ann_snapshot {
        if let Some(pw) = pointer_world {
            let (dx, dy) = (pw.x - origin[0], pw.y - origin[1]);
            if is_group {
                if let Some(g) = state.doc.groups.get_mut(index) {
                    g.rect[0] = rect0[0] + dx;
                    g.rect[1] = rect0[1] + dy;
                }
                for (id, start) in &captured {
                    if let Some(n) = state.doc.node_mut(*id) {
                        n.position = [start[0] + dx, start[1] + dy];
                    }
                }
            } else if let Some(c) = state.doc.comments.get_mut(index) {
                c.rect[0] = rect0[0] + dx;
                c.rect[1] = rect0[1] + dy;
            }
        }
        if !pointer_down {
            finish_annotation_drag(state, registry);
        }
    }

    // An inline widget gesture ends on release.
    if !pointer_down {
        state.flush_prop_edit(registry);
    }

    // Advance / finish a live annotation resize.
    let resize_snapshot = state
        .annotation_resize
        .as_ref()
        .map(|r| (r.target, r.handle, r.origin_world, r.rect0, r.min_h));
    let resizing = resize_snapshot.is_some();
    if let Some((target, handle, origin, rect0, min_h)) = resize_snapshot {
        if let Some(pw) = pointer_world {
            let next = apply_resize(rect0, handle, pw.x - origin[0], pw.y - origin[1], min_h);
            match target {
                Annotation::Comment(i) => {
                    if let Some(c) = state.doc.comments.get_mut(i) {
                        c.rect = next;
                    }
                }
                Annotation::Group(i) => {
                    if let Some(g) = state.doc.groups.get_mut(i) {
                        g.rect = next;
                    }
                }
            }
        }
        if !pointer_down {
            state.finish_annotation_resize(registry);
        }
    }

    // Wire selection. A wire is behind every node and pin, so it only claims
    // a press nothing in front of it wanted.
    let mut wire_claimed = false;
    if pointer_pressed && state.connect_drag.is_none() && state.node_drag.is_none() {
        if let (Some(i), Some(pw)) = (hovered_wire, pointer_world) {
            let free = node_under(&geoms, pw, lod, &m).is_none();
            if free {
                if let Some(edge) = state.doc.edges.get(i).cloned() {
                    if shift {
                        state.toggle_edge_selected(&edge);
                    } else {
                        state.select_only_edge(&edge);
                    }
                    wire_claimed = true;
                }
            }
        }
    }

    // Pins take precedence over the node body. Only where rows are drawn.
    let hit_w = m.pin_hit_w(zoom);
    let mut pin_claimed = false;
    if lod.rows() {
        for g in &geoms {
            for pin in &g.pins {
                let wr = Rect::from_center_size(pin.dot_center, Vec2::splat(hit_w));
                let id = ui.alloc_id(("graph_pin", g.id, &pin.slug, pin.output));
                let resp = scope.interact(ui, id, wr);
                if resp.pressed && state.connect_drag.is_none() && state.node_drag.is_none() {
                    state.connect_drag = Some(ConnectDrag {
                        from_node: g.id,
                        from_pin: pin.slug.clone(),
                        from_output: pin.output,
                    });
                    pin_claimed = true;
                }
            }
        }
    }

    // Node body: select + start drag.
    let mut begin_drag = false;
    let mut node_pressed = false;
    for g in &geoms {
        let id = ui.alloc_id(("graph_node", g.id));
        let resp = scope.interact(ui, id, g.body_rect(lod, &m));
        if resp.pressed {
            node_pressed = true;
        }
        if resp.pressed
            && !pin_claimed
            && !widget_claimed
            && !wire_claimed
            && state.node_drag.is_none()
            && state.connect_drag.is_none()
            && !begin_drag
            && !shift
        {
            if !state.selection.contains(&g.id) {
                state.select_only(g.id);
            } else {
                state.primary = Some(g.id);
            }
            begin_drag = true;
        }
        if resp.clicked && shift {
            state.toggle_selected(g.id);
        }
        // Double-click a subgraph node → open its referenced doc as a tab.
        if resp.double_clicked(ui) {
            if let Some(path) = state.doc.node(g.id).and_then(|n| n.subgraph.clone()) {
                *open_subgraph = Some(path);
            }
        }
    }
    if begin_drag {
        if let Some(pw) = pointer_world {
            let originals: Vec<(u64, [f32; 2])> = state
                .selection
                .iter()
                .filter_map(|id| state.doc.node(*id).map(|n| (*id, n.position)))
                .collect();
            // Anchored notes track their node live, not on release.
            let ids: std::collections::BTreeSet<u64> =
                originals.iter().map(|(id, _)| *id).collect();
            let anchored = anchored_comments(&state.doc, &ids)
                .into_iter()
                .map(|i| (i, [state.doc.comments[i].rect[0], state.doc.comments[i].rect[1]]))
                .collect();
            state.node_drag =
                Some(NodeDrag { origin_world: [pw.x, pw.y], originals, anchored });
        }
    }

    // Group title bars + comment headers: select, drag, double-click to edit.
    // Behind nodes, so only when no node/pin claimed the press this frame.
    let ann_free = !node_pressed
        && !pin_claimed
        && !widget_claimed
        && !wire_claimed
        && state.node_drag.is_none()
        && state.annotation_drag.is_none();
    // Resize handles come first: they sit on the border, which the bar and
    // body drags would otherwise swallow. Only on the selected annotation, so
    // a dense canvas does not become a minefield of invisible grab zones.
    if ann_free && !resizing && pointer_pressed {
        if let Some(pw) = pointer_world {
            let selected: Option<(Annotation, Rect, f32)> = if let Some(i) = state.sel_group {
                state
                    .doc
                    .groups
                    .get(i)
                    .map(|g| (Annotation::Group(i), group_rect(g, &m), 0.0))
            } else {
                state.sel_comment.and_then(|i| {
                    state.doc.comments.get(i).map(|c| {
                        (Annotation::Comment(i), comment_rect(c, &m), comment_min_h(c, &m))
                    })
                })
            };
            if let Some((target, rect, min_h)) = selected {
                if let Some(handle) = resize_handle_at(rect, pw, zoom) {
                    if let Some(rect0) = state.annotation_rect(target) {
                        state.annotation_resize = Some(AnnotationResize {
                            target,
                            handle,
                            origin_world: [pw.x, pw.y],
                            rect0,
                            min_h,
                        });
                    }
                }
            }
        }
    }
    let resize_claimed = state.annotation_resize.is_some();
    let ann_free = ann_free && !resize_claimed;

    // Groups first (front-most annotation is the last drawn = highest index).
    for i in (0..state.doc.groups.len()).rev() {
        let r = state.doc.groups[i].rect;
        let bar = Rect::from_min_size(Pos2::new(r[0], r[1]), Vec2::new(r[2], m.group_bar));
        let id = ui.alloc_id(("graph_group", i));
        let resp = scope.interact(ui, id, bar);
        // The fold caret shares the bar; it claims the press before the drag.
        let caret = caret_zone(bar, &m, zoom);
        if resp.pressed && ann_free && pointer_world.is_some_and(|p| caret.contains(p)) {
            state.toggle_annotation_collapsed(Annotation::Group(i), registry);
        } else if resp.double_clicked(ui) {
            begin_annotation_edit(state, true, i);
        } else if let (true, Some(pw)) = (resp.pressed && ann_free, pointer_world) {
            state.clear_selection();
            state.sel_group = Some(i);
            let captured = nodes_captured_by_rect(&node_centers(&geoms), r);
            let originals = captured
                .iter()
                .filter_map(|id| state.doc.node(*id).map(|n| (*id, n.position)))
                .collect();
            state.annotation_drag = Some(AnnotationDrag {
                is_group: true,
                index: i,
                origin_world: [pw.x, pw.y],
                rect_min0: [r[0], r[1]],
                captured: originals,
            });
        }
    }
    for i in (0..state.doc.comments.len()).rev() {
        let r = state.doc.comments[i].rect;
        let bar_h = m.comment_bar * state.doc.comments[i].clamped_font_scale();
        let bar = Rect::from_min_size(Pos2::new(r[0], r[1]), Vec2::new(r[2], bar_h));
        let id = ui.alloc_id(("graph_comment", i));
        let resp = scope.interact(ui, id, bar);
        let caret = caret_zone(bar, &m, zoom);
        if resp.pressed && ann_free && pointer_world.is_some_and(|p| caret.contains(p)) {
            state.toggle_annotation_collapsed(Annotation::Comment(i), registry);
        } else if resp.double_clicked(ui) {
            begin_annotation_edit(state, false, i);
        } else if let (true, Some(pw)) = (
            resp.pressed && ann_free && state.annotation_drag.is_none(),
            pointer_world,
        ) {
            state.clear_selection();
            state.sel_comment = Some(i);
            state.annotation_drag = Some(AnnotationDrag {
                is_group: false,
                index: i,
                origin_world: [pw.x, pw.y],
                rect_min0: [r[0], r[1]],
                captured: Vec::new(),
            });
        }
    }

    // Connection drag: live wire tinted by validity, resolved on release.
    // Snapshot first so the `connect_drag` borrow ends before any mutation.
    let status = Palette::invariant_status();
    let connect_snapshot = state
        .connect_drag
        .as_ref()
        .map(|d| (d.from_node, d.from_pin.clone(), d.from_output));
    if let Some((from_node, from_pin, from_output)) = connect_snapshot {
        let src = geoms
            .iter()
            .find(|g| g.id == from_node)
            .and_then(|g| g.wire_anchor(&from_pin, from_output));
        if let (Some(src), Some(pw)) = (src, pointer_world) {
            let src_ty = pin_ty(&geoms, from_node, &from_pin, from_output);
            let tint = match (pin_under(&geoms, pw, hit_w), src_ty.as_ref()) {
                (Some(h), Some(sty)) => {
                    if validate_connection(
                        state, from_node, &from_pin, from_output, sty, h.0, &h.1, h.3, &h.2,
                    )
                    .is_some()
                    {
                        status.success
                    } else {
                        status.error
                    }
                }
                _ => st.palette.accent_active,
            };
            // The ghost takes the same route as the finished wire will —
            // rect-less meta, so the backward lane uses its pin-relative
            // fallback (there is no target node yet).
            let ghost_meta = RouteMeta {
                src_rect: geoms.iter().find(|g| g.id == from_node).map(|g| g.rect),
                dst_rect: None,
                target_pin_index: 0,
                node_rects: &node_rects,
            };
            let width = if lod.bar_only() { 1.0 } else { WIRE_DATA };
            let mut p = ui.painter();
            let ghost = WireGeom {
                edge_index: usize::MAX,
                a: src,
                b: pw,
                ty: src_ty.clone().unwrap_or(PinType::Exec),
                screen: wire_screen_points(src, pw, wire_prefs, &ghost_meta, scope),
                selected: false,
                mismatched: false,
            };
            stroke_wire(&mut p, &ghost, wire_prefs, scope, width, tint);
        }
        if released {
            resolve_connection(state, &geoms, pointer_world, hit_w, registry);
            state.connect_drag = None;
        } else if !pointer_down {
            state.connect_drag = None;
        }
    }

    // Marquee box-select on empty canvas (suppressed while an overlay/node/
    // annotation/widget owns the gesture).
    handle_marquee(
        ui,
        scope,
        state,
        &geoms,
        MarqueeCtx {
            pointer_world,
            pointer_pressed,
            pointer_down,
            released,
            blocked: pin_claimed || node_pressed || widget_claimed || wire_claimed,
            mods,
            lod,
            hit_w,
        },
        &m,
        &st,
    );

    // `#node` chips in comment bodies: click selects and frames that node.
    // The reference is resolved at click time, so a renamed or deleted node
    // just stops resolving rather than corrupting the stored text.
    if pointer_pressed && !node_pressed && !pin_claimed && !wire_claimed && !resize_claimed {
        if let Some(pw) = pointer_world {
            if let Some(token) = comment_ref_at(state, &m, pw, &st, zoom, ui) {
                if let Some(id) = resolve_node_ref(state, &geoms, &token) {
                    state.select_only(id);
                    if let Some((mn, mx)) =
                        geoms_bbox(geoms.iter().filter(|g| g.id == id))
                    {
                        *frame_request =
                            Some(frame_view(mn, mx, scope.rect().size(), zoom_min, zoom_max));
                    }
                }
            }
        }
    }

    // Validation chrome last, so it paints over the canvas it describes.
    error_chip(
        ui,
        scope.rect(),
        state,
        &errors,
        &geoms,
        scope.rect().size(),
        zoom_min,
        zoom_max,
        frame_request,
        open_subgraph,
    );

    // Right-click: an annotation gets its own menu (tint / collapse / anchor
    // / delete); empty canvas gets the create menu.
    if right_pressed {
        if let Some(pw) = pointer_world {
            let on_annotation = annotation_at(state, &m, pw);
            if let Some(i) = hovered_wire {
                // Right-clicking a wire offers the one thing that belongs to
                // a wire: splitting it.
                state.wire_menu = state.doc.edges.get(i).cloned().map(|e| (e, [pw.x, pw.y]));
                *wire_menu_at = ui.ctx().input.pointer_pos;
            } else if let Some(target) = on_annotation {
                match target {
                    Annotation::Comment(i) => {
                        state.clear_selection();
                        state.sel_comment = Some(i);
                    }
                    Annotation::Group(i) => {
                        state.clear_selection();
                        state.sel_group = Some(i);
                    }
                }
                state.annotation_menu = Some(target);
                *annotation_menu_at = ui.ctx().input.pointer_pos;
            } else if pin_under(&geoms, pw, hit_w).is_none()
                && node_under(&geoms, pw, lod, &m).is_none()
            {
                state.create_menu_world = Some([pw.x, pw.y]);
                state.create_menu_search.clear();
                *menu_open_at = ui.ctx().input.pointer_pos;
            }
        }
    }

    // Organization keys (Windows mapping): C groups the selection, Shift+C
    // drops a note at the pointer. Canvas-hovered only, and never while an
    // inline editor owns the keyboard.
    if pointer_world.is_some() && state.editing.is_none() {
        let (c_key, shift_only, no_mods) = {
            let input = &ui.ctx().input;
            (
                input.key_pressed(Key::Char('c')),
                input.modifiers == Modifiers::SHIFT,
                input.modifiers.is_empty(),
            )
        };
        if c_key && no_mods {
            state.add_group_around_selection(registry);
        } else if c_key && shift_only {
            if let Some(pw) = pointer_world {
                state.add_comment([pw.x, pw.y], registry);
            }
        }

        let input = &ui.ctx().input;
        let ctrl_only = input.modifiers == Modifiers::CTRL;
        let ctrl_alt = input.modifiers == Modifiers::CTRL | Modifiers::ALT;
        let alt_only = input.modifiers == Modifiers::ALT;

        // Ctrl+G — collapse the selection into a new .subgraph asset.
        if ctrl_only && input.key_pressed(Key::Char('g')) {
            *collapse_request = true;
        }
        // Ctrl+B — store the view in the next bookmark slot (cycles 1..=5).
        if ctrl_only && input.key_pressed(Key::Char('b')) {
            let slot = state.store_bookmark();
            println!("graph: view bookmarked in slot {slot} (Shift+{slot} to recall)");
        }
        // Shift+1..5 — recall. Ctrl+1..5 belongs to the dock's tabs and bare
        // digits to the renderer's debug views, so Shift is the free row.
        if shift_only {
            for slot in 1..=BOOKMARK_SLOTS {
                let ch = char::from_digit(slot as u32, 10).unwrap_or('1');
                if input.key_pressed(Key::Char(ch)) && state.recall_bookmark(slot) {
                    *frame_request = Some(state.view);
                }
            }
        }
        // Ctrl+Alt+K — purge unused, via a confirm step.
        if ctrl_alt && input.key_pressed(Key::Char('k')) {
            let unused = state.unused_nodes(registry);
            state.purge_confirm = (!unused.is_empty()).then_some(unused);
            if state.purge_confirm.is_none() {
                println!("graph: purge unused found nothing to remove");
            }
        }
        // Alt+A — align & distribute, offered through the canvas menu.
        if alt_only && input.key_pressed(Key::Char('a')) && state.selection.len() >= 3 {
            if let Some(pw) = pointer_world {
                state.create_menu_world = Some([pw.x, pw.y]);
                state.create_menu_search.clear();
                *menu_open_at = ui.ctx().input.pointer_pos;
            }
        }
    }

    geoms
}

/// World rects of the selected nodes, for align & distribute — node sizes are
/// auto-fitted at draw time, so only the geometry pass knows them.
fn selected_rects(state: &GraphEditorState, geoms: &[NodeGeom]) -> Vec<(u64, [f32; 4])> {
    geoms
        .iter()
        .filter(|g| state.selection.contains(&g.id))
        .map(|g| {
            (
                g.id,
                [g.rect.min.x, g.rect.min.y, g.rect.width(), g.rect.height()],
            )
        })
        .collect()
}

/// The purge confirm step. Destructive actions confirm with a count; the
/// filled danger button lives in the dialog, per the design system.
fn purge_confirm(ui: &mut Ui, rect: Rect, state: &mut GraphEditorState, registry: &NodeRegistry) {
    let Some(ids) = state.purge_confirm.clone() else {
        return;
    };
    let st = ui.style();
    let pad = st.spacing.padding;
    let n = ids.len();
    let text = format!(
        "Purge {n} unused node{}?",
        if n == 1 { "" } else { "s" }
    );
    let font = st.fonts.body;
    let w = ui.painter().measure_text(&text, font, None).x + pad * 2.0;
    let panel = Rect::from_center_size(
        Pos2::new(rect.center().x, rect.min.y + rect.height() * 0.3),
        Vec2::new(w.max(240.0), st.metrics.control_height * 2.0 + pad * 3.0),
    );
    {
        let mut p = ui.painter();
        p.rect_filled(rect, Rounding::ZERO, Color::BLACK.with_alpha(st.palette.scrim_alpha));
        p.rect_filled(panel, st.rounding.panel, st.palette.elevated);
        p.rect_stroke(panel, st.rounding.panel, st.metrics.border, st.palette.stroke_strong);
        p.text(
            Pos2::new(panel.min.x + pad, panel.min.y + pad),
            &text,
            font,
            st.palette.text,
            None,
        );
    }
    let bw = 84.0;
    let by = panel.max.y - pad - st.metrics.control_height;
    let (mut purge, mut cancel) = (false, false);
    ui.run_at(
        Rect::from_min_size(
            Pos2::new(panel.max.x - pad - bw * 2.0 - st.spacing.item, by),
            Vec2::new(bw * 2.0 + st.spacing.item, st.metrics.control_height),
        ),
        Direction::LeftToRight,
        Id::new("graph_purge_confirm"),
        UiOptions { padding: Vec2::ZERO, spacing: st.spacing.item },
        |ui| {
            cancel = Button::new("Cancel")
                .exact_size(Vec2::new(bw, st.metrics.control_height))
                .show(ui)
                .clicked;
            purge = Button::new("Purge")
                .danger()
                .exact_size(Vec2::new(bw, st.metrics.control_height))
                .show(ui)
                .clicked;
        },
    );
    if purge {
        state.purge_nodes(&ids, registry);
        println!("graph: purged {n} unused node{}", if n == 1 { "" } else { "s" });
        state.purge_confirm = None;
    } else if cancel {
        state.purge_confirm = None;
    }
}

/// The front-most annotation under a world point — comments sit above groups,
/// so they are tested first.
fn annotation_at(state: &GraphEditorState, m: &GraphMetrics, pw: Pos2) -> Option<Annotation> {
    for (i, c) in state.doc.comments.iter().enumerate().rev() {
        if comment_rect(c, m).contains(pw) {
            return Some(Annotation::Comment(i));
        }
    }
    for (i, g) in state.doc.groups.iter().enumerate().rev() {
        // A group is a frame, not a surface: only its title bar takes the
        // click, or right-clicking anywhere inside one would shadow the
        // canvas menu over a large area.
        let r = group_rect(g, m);
        if Rect::from_min_size(r.min, Vec2::new(r.width(), m.group_bar)).contains(pw) {
            return Some(Annotation::Group(i));
        }
    }
    None
}

/// The wire context menu — a wire owns exactly one action: splitting it with
/// a reroute at the click point. Breaking is Delete (and, from Phase 6, the
/// ⌥-click and slash-cut gestures).
fn wire_menu(
    ui: &mut Ui,
    state: &mut GraphEditorState,
    registry: &NodeRegistry,
    open_at: Option<Pos2>,
) {
    let Some((edge, pos)) = state.wire_menu.clone() else {
        return;
    };
    let mut add = false;
    crusty_gui::widgets::context_menu_at(ui, "graph_wire_menu", open_at, |ui| {
        ui.menu_group_header("Wire");
        if ui.menu_item("Add reroute") {
            add = true;
        }
    });
    if add {
        // Center the dot on the click point rather than starting it there.
        let st = ui.style();
        let d = GraphMetrics::new(&st).reroute_d() * 0.5;
        state.insert_reroute(&edge, [pos[0] - d, pos[1] - d], registry);
        state.wire_menu = None;
    }
}

/// Context menu for the annotation under the pointer: the 12 deep-tone
/// swatches, collapse, anchoring, delete.
fn annotation_menu(
    ui: &mut Ui,
    state: &mut GraphEditorState,
    registry: &NodeRegistry,
    open_at: Option<Pos2>,
) {
    let Some(target) = state.annotation_menu else {
        return;
    };
    let is_comment = !target.is_group();
    let i = target.index();
    let current = match target {
        Annotation::Comment(i) => state.doc.comments.get(i).map(|c| c.tint),
        Annotation::Group(i) => state.doc.groups.get(i).map(|g| g.tint),
    };
    let Some(current) = current else {
        state.annotation_menu = None;
        return;
    };
    let collapsed = match target {
        Annotation::Comment(i) => state.doc.comments.get(i).map(|c| c.collapsed),
        Annotation::Group(i) => state.doc.groups.get(i).map(|g| g.collapsed),
    }
    .unwrap_or(false);
    let anchored = is_comment
        && state.doc.comments.get(i).and_then(|c| c.anchor).is_some();
    let one_node = state.selection.len() == 1;

    let mut picked: Option<Option<u8>> = None;
    let mut toggle = false;
    let mut anchor_to: Option<Option<u64>> = None;
    let mut delete = false;
    crusty_gui::widgets::context_menu_at(ui, "graph_annotation_menu", open_at, |ui| {
        ui.menu_group_header("Tint");
        picked = tint_menu_row(ui, current);
        ui.menu_group_header(if is_comment { "Comment" } else { "Group" });
        if ui.menu_item(if collapsed { "Expand" } else { "Collapse" }) {
            toggle = true;
        }
        if is_comment {
            if anchored {
                if ui.menu_item("Un-anchor") {
                    anchor_to = Some(None);
                }
            } else if ui.menu_item_enabled("Anchor to selected node", one_node) {
                anchor_to = Some(None); // resolved below from the selection
            }
        }
        if ui.menu_item("Delete") {
            delete = true;
        }
    });

    if let Some(t) = picked {
        state.set_annotation_tint(target, t, registry);
        state.annotation_menu = None;
    }
    if toggle {
        state.toggle_annotation_collapsed(target, registry);
        state.annotation_menu = None;
    }
    if let Some(a) = anchor_to {
        let node = if anchored { None } else { a.or_else(|| state.selection.iter().copied().next()) };
        state.set_comment_anchor(i, node, registry);
        state.annotation_menu = None;
    }
    if delete {
        state.delete_selection(registry);
        state.annotation_menu = None;
    }
}

// ---------------------------------------------------------------------------
// Annotation interaction
// ---------------------------------------------------------------------------

/// Which resize handle a world point grabs on `rect`, using a screen-space
/// band so a handle is equally grabbable at every zoom.
fn resize_handle_at(rect: Rect, pw: Pos2, zoom: f32) -> Option<ResizeHandle> {
    let band = RESIZE_GRAB_PX / zoom.max(0.01);
    let outer = Rect::from_min_max(
        Pos2::new(rect.min.x - band, rect.min.y - band),
        Pos2::new(rect.max.x + band, rect.max.y + band),
    );
    if !outer.contains(pw) {
        return None;
    }
    let (l, r) = ((pw.x - rect.min.x).abs() <= band, (pw.x - rect.max.x).abs() <= band);
    let (t, b) = ((pw.y - rect.min.y).abs() <= band, (pw.y - rect.max.y).abs() <= band);
    // Corners first: a corner overlaps two edges and must win.
    Some(match (l, r, t, b) {
        (true, _, true, _) => ResizeHandle::TopLeft,
        (_, true, true, _) => ResizeHandle::TopRight,
        (true, _, _, true) => ResizeHandle::BottomLeft,
        (_, true, _, true) => ResizeHandle::BottomRight,
        (true, ..) => ResizeHandle::Left,
        (_, true, ..) => ResizeHandle::Right,
        (_, _, true, _) => ResizeHandle::Top,
        (_, _, _, true) => ResizeHandle::Bottom,
        _ => return None,
    })
}

/// Apply a resize drag. Width is author-set in both directions; height has a
/// floor — a comment never auto-shrinks below its own wrapped text.
fn apply_resize(rect0: [f32; 4], h: ResizeHandle, dx: f32, dy: f32, min_h: f32) -> [f32; 4] {
    let (mut x, mut y, mut w, mut hh) = (rect0[0], rect0[1], rect0[2], rect0[3]);
    let floor_h = ANNOTATION_MIN_H.max(min_h);
    if h.moves_left() {
        let nw = (w - dx).max(ANNOTATION_MIN_W);
        x += w - nw;
        w = nw;
    }
    if h.moves_right() {
        w = (w + dx).max(ANNOTATION_MIN_W);
    }
    if h.moves_top() {
        let nh = (hh - dy).max(floor_h);
        y += hh - nh;
        hh = nh;
    }
    if h.moves_bottom() {
        hh = (hh + dy).max(floor_h);
    }
    [x, y, w, hh]
}

/// Handle marks on the selected annotation, so the affordance is visible
/// rather than something the user has to discover by hovering the border.
fn draw_resize_handles(p: &mut Painter, sr: Rect, st: &Style, m: &GraphMetrics) {
    let d = m.edge * 1.5;
    for c in [
        Pos2::new(sr.min.x, sr.min.y),
        Pos2::new(sr.max.x, sr.min.y),
        Pos2::new(sr.min.x, sr.max.y),
        Pos2::new(sr.max.x, sr.max.y),
    ] {
        p.rect_filled(
            Rect::from_center_size(c, Vec2::splat(d * 2.0)),
            Rounding::same(m.border),
            st.palette.focus_ring,
        );
    }
}

/// The 12 deep-tone swatches plus "None" — never a free picker. A hand-picked
/// color that reads on Steel vanishes on Graphite, breaks the no-hex lint and
/// survives no re-skin; the ramp index survives all three.
fn tint_menu_row(ui: &mut Ui, current: Option<u8>) -> Option<Option<u8>> {
    let st = ui.style();
    let sw = st.metrics.control_height * 0.7;
    let cols = 13.0;
    let rect = ui.allocate(Vec2::new(sw * cols + 8.0, sw + 6.0));
    let mut picked = None;
    for i in 0..13usize {
        let cell = Rect::from_min_size(
            Pos2::new(rect.min.x + 4.0 + sw * i as f32, rect.min.y + 3.0),
            Vec2::splat(sw - 2.0),
        );
        let slot = (i < 12).then_some(i as u8);
        let color = match slot {
            Some(t) => ramp()[t as usize].deep,
            None => st.palette.header,
        };
        let id = ui.alloc_id(("graph_tint", i));
        let resp = ui.interact(id, cell);
        ui.painter().rect_filled(cell, st.rounding.small, color);
        if slot.is_none() {
            // "None" reads as an empty well, not a 13th color.
            ui.painter().line_segment(
                cell.min,
                cell.max,
                st.metrics.border,
                st.palette.text_disabled,
            );
        }
        if slot == current || resp.hovered {
            ui.painter().rect_stroke(
                cell,
                st.rounding.small,
                st.metrics.border,
                if slot == current { st.palette.accent_active } else { st.palette.focus_ring },
            );
        }
        if resp.clicked {
            picked = Some(slot);
        }
    }
    picked
}

// ---------------------------------------------------------------------------
// Toolbar
// ---------------------------------------------------------------------------

/// Height of the graph toolbar row, base units at ui_scale 1.0.
const TOOLBAR_H: f32 = 30.0;
/// Width of the 3-way wire-style segmented control, base units.
const TOOLBAR_SEG_W: f32 = 168.0;

/// The graph tab's toolbar: a 3-way wire-style quick switch on the left and
/// the document's realm as a read-only mono chip on the right.
///
/// **Nothing else goes here.** A second control from the Preferences ▸ Graph
/// sections would become a competing settings surface — the panels doc's
/// explicit rule. The segmented control is the documented toggled-tool
/// treatment (`accent_soft` fill + accent border), one of the few approved
/// accent spends on this surface.
fn graph_toolbar(
    ui: &mut Ui,
    state: &GraphEditorState,
    style_now: WireStyle,
    request: &mut Option<WireStyle>,
) {
    let st = ui.style();
    let s = (st.metrics.row_height / BASE_ROW_H).max(0.1);
    let w = ui.available().width();
    let rect = ui.allocate(Vec2::new(w, TOOLBAR_H * s));
    ui.painter().rect_filled(rect, Rounding::ZERO, st.palette.window);
    ui.painter().line_segment(
        Pos2::new(rect.min.x, rect.max.y),
        Pos2::new(rect.max.x, rect.max.y),
        st.metrics.border,
        st.palette.stroke,
    );

    let pad = BASE_PAD_X * s;
    let seg_h = st.metrics.control_height.min(rect.height() - 6.0 * s);
    let seg = Rect::from_min_size(
        Pos2::new(rect.min.x + pad, rect.center().y - seg_h * 0.5),
        Vec2::new(TOOLBAR_SEG_W * s, seg_h),
    );
    let active = WireStyle::ALL
        .iter()
        .position(|x| *x == style_now)
        .unwrap_or(0);
    if let Some(i) = segmented_control(
        ui,
        "graph_toolbar_style",
        seg,
        &["Spline", "Manhattan", "Subway"],
        active,
        true,
    ) {
        if WireStyle::ALL[i] != style_now {
            *request = Some(WireStyle::ALL[i]);
        }
    }

    // Realm chip. A *node's* Shared realm prints nothing; the graph's realm
    // always shows — it is the document's authority statement, not a
    // per-node annotation.
    let label = state.doc.realm.label();
    let px = st.fonts.small;
    let tw = ui
        .painter()
        .measure_text_family(label, px, None, FontFamily::Mono)
        .x;
    let chip = Rect::from_min_size(
        Pos2::new(rect.max.x - pad - tw - pad * 2.0, rect.center().y - seg_h * 0.5),
        Vec2::new(tw + pad * 2.0, seg_h),
    );
    ui.painter()
        .rect_filled(chip, st.rounding.small, st.palette.header);
    ui.painter().rect_stroke(
        chip,
        st.rounding.small,
        st.metrics.border,
        st.palette.stroke,
    );
    ui.painter().text_family(
        Pos2::new(chip.min.x + pad, chip.center().y - px * 0.62),
        label,
        px,
        st.palette.text_secondary,
        None,
        FontFamily::Mono,
    );
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

/// Two-level world grid: minor 16px @ 5% white, major 128px @ 9%. The minor
/// grid drops below 40% zoom, where it would only add noise. Never accent.
fn draw_grid(ui: &mut Ui, scope: &CanvasScope, st: &Style, vis: Rect, zoom: f32) {
    let mut p = ui.painter();
    p.rect_filled(scope.rect(), Rounding::ZERO, st.palette.input);

    let mut level = |step: f32, color: Color| {
        // Guard against a pathological view producing millions of lines.
        let cols = (vis.width() / step).ceil() as i32;
        let rows = (vis.height() / step).ceil() as i32;
        if cols > 4096 || rows > 4096 {
            return;
        }
        for i in (vis.min.x / step).floor() as i32..=(vis.max.x / step).ceil() as i32 {
            let wx = i as f32 * step;
            p.line_segment(
                scope.world_to_screen(Pos2::new(wx, vis.min.y)),
                scope.world_to_screen(Pos2::new(wx, vis.max.y)),
                1.0,
                color,
            );
        }
        for i in (vis.min.y / step).floor() as i32..=(vis.max.y / step).ceil() as i32 {
            let wy = i as f32 * step;
            p.line_segment(
                scope.world_to_screen(Pos2::new(vis.min.x, wy)),
                scope.world_to_screen(Pos2::new(vis.max.x, wy)),
                1.0,
                color,
            );
        }
    };

    if zoom >= GRID_MINOR_MIN_ZOOM {
        level(GRID_MINOR_STEP, grid_minor());
    }
    level(GRID_MAJOR_STEP, grid_major());
}

/// Group frames + comment boxes, behind wires and nodes (P7). Their titles are
/// the one text exempt from the LOD glyph cut — on a graph too big to read,
/// annotations are the only wayfinding left.
#[allow(clippy::too_many_arguments)]
fn draw_annotations(
    ui: &mut Ui,
    scope: &CanvasScope,
    state: &GraphEditorState,
    st: &Style,
    m: &GraphMetrics,
    vis: Rect,
    zoom: f32,
    lod: ZoomLod,
) {
    let mut p = ui.painter();
    let round = Rounding::same(m.radius * zoom);
    // Annotation titles are the one text exempt from the LOD glyph cut: on a
    // graph too big to read, they are the only wayfinding left.
    let annotation_px = |base: f32| (base * zoom).max(ANNOTATION_FLOOR_PX);

    // Groups first — a group is a region that *contains* things, so it sits
    // below comments, which sit below nodes.
    for (i, g) in state.doc.groups.iter().enumerate() {
        let wr = group_rect(g, m);
        let clip = wr.intersect(vis);
        if clip.width() <= 0.0 || clip.height() <= 0.0 {
            continue;
        }
        let sr = scope.world_rect_to_screen(wr);
        let sel = state.sel_group == Some(i);
        // Tint paints a 6% body wash and a 45% border — translucent region.
        let (wash, border) = match g.tint {
            Some(t) => {
                let deep = ramp()[(t % 12) as usize].deep;
                (deep.with_alpha(GROUP_WASH_ALPHA), deep.with_alpha(GROUP_BORDER_ALPHA))
            }
            None => (
                st.palette.panel.with_alpha(GROUP_UNTINTED_WASH_ALPHA),
                if sel { st.palette.stroke_strong } else { st.palette.stroke },
            ),
        };
        p.rect_filled(sr, round, wash);
        p.rect_stroke(
            sr,
            round,
            m.border,
            if sel { st.palette.stroke_strong } else { border },
        );
        let bar_h = m.group_bar * zoom;
        let bar = Rect::from_min_size(sr.min, Vec2::new(sr.width(), bar_h));
        p.rect_filled(bar, round, st.palette.header);
        let px = annotation_px(st.fonts.body);
        let tx = collapse_caret(&mut p, bar, g.collapsed, st, zoom, m);
        p.text(
            Pos2::new(tx, sr.min.y + (bar_h - px) * 0.5),
            &g.title,
            px,
            st.palette.text,
            None,
        );
        if sel && !g.collapsed {
            draw_resize_handles(&mut p, sr, st, m);
        }
    }

    for (i, c) in state.doc.comments.iter().enumerate() {
        let wr = comment_rect(c, m);
        let clip = wr.intersect(vis);
        if clip.width() <= 0.0 || clip.height() <= 0.0 {
            continue;
        }
        let sr = scope.world_rect_to_screen(wr);
        let sel = state.sel_comment == Some(i);
        let fs = c.clamped_font_scale();
        let bar_h = m.comment_bar * fs * zoom;

        // The body is opaque `elevated` and is NEVER washed — that is exactly
        // what tells a comment from a group at any zoom.
        p.rect_filled(sr, round, st.palette.elevated);
        p.rect_stroke(
            sr,
            round,
            m.border,
            if sel { st.palette.focus_ring } else { st.palette.stroke_strong },
        );

        // Tint paints the NOTE bar's fill plus a 1px left edge, nothing else.
        let bar = Rect::from_min_size(sr.min, Vec2::new(sr.width(), bar_h));
        let (bar_fill, label_col) = match c.tint {
            Some(t) => (
                ramp()[(t % 12) as usize].deep,
                ramp()[(t % 12) as usize].bright,
            ),
            None => (st.palette.header, st.palette.text_secondary),
        };
        p.rect_filled(bar, round, bar_fill);
        if let Some(t) = c.tint {
            p.rect_filled(
                Rect::from_min_size(sr.min, Vec2::new(m.border, sr.height())),
                Rounding::ZERO,
                ramp()[(t % 12) as usize].deep,
            );
        }
        let bar_px = annotation_px(st.fonts.small * fs);
        let tx = collapse_caret(&mut p, bar, c.collapsed, st, zoom, m);
        p.text_family(
            Pos2::new(tx, sr.min.y + (bar_h - bar_px) * 0.5),
            "NOTE",
            bar_px,
            label_col,
            None,
            FontFamily::Mono,
        );
        // An anchored note says so: it is not free-floating, and that is the
        // difference between a live annotation and a stale one.
        if c.anchor.is_some() {
            let d = m.pin_r * zoom;
            p.circle_filled(
                Pos2::new(sr.max.x - m.pad_x * zoom, sr.min.y + bar_h * 0.5),
                d * 0.6,
                label_col,
            );
        }

        if sel && !c.collapsed {
            draw_resize_handles(&mut p, sr, st, m);
        }
        if c.collapsed || !lod.glyphs() {
            continue;
        }
        // Body text: plain, stored verbatim, with `#node` references drawn as
        // chips (see `draw_comment_body`).
        draw_comment_body(&mut p, st, m, sr, bar_h, c, fs, zoom);
    }
}

/// The `▾` / `▸` fold affordance at the left of an annotation bar. Returns the
/// x where the bar's label should start.
fn collapse_caret(
    p: &mut Painter,
    bar: Rect,
    collapsed: bool,
    st: &Style,
    zoom: f32,
    m: &GraphMetrics,
) -> f32 {
    let s = (m.pin_r * 0.8 * zoom).max(2.0);
    let c = Pos2::new(bar.min.x + m.pad_x * zoom, bar.center().y);
    let col = st.palette.text_secondary;
    if collapsed {
        // ▸
        p.triangle(
            Pos2::new(c.x - s * 0.5, c.y - s),
            Pos2::new(c.x - s * 0.5, c.y + s),
            Pos2::new(c.x + s * 0.7, c.y),
            col,
        );
    } else {
        // ▾
        p.triangle(
            Pos2::new(c.x - s, c.y - s * 0.5),
            Pos2::new(c.x + s, c.y - s * 0.5),
            Pos2::new(c.x, c.y + s * 0.7),
            col,
        );
    }
    bar.min.x + m.pad_x * zoom + s * 2.0
}

/// Comment body text with `#node` references rendered as inline chips.
///
/// The body is stored verbatim (the `Raw` philosophy — never reformat an
/// author's text on load); the chips are a *rendering* of a reference, so a
/// renamed or deleted node just stops resolving instead of corrupting text.
#[allow(clippy::too_many_arguments)]
fn draw_comment_body(
    p: &mut Painter,
    st: &Style,
    m: &GraphMetrics,
    sr: Rect,
    bar_h: f32,
    c: &crate::engine::node_graph::CommentBox,
    fs: f32,
    zoom: f32,
) {
    let px = st.fonts.small * fs * zoom;
    let pad = m.pad_x * zoom;
    let mut x = sr.min.x + pad;
    let mut y = sr.min.y + bar_h + m.label_gap * zoom;
    let line_h = px * 1.35;
    let max_x = sr.max.x - pad;

    for token in tokenize_comment(&c.text) {
        match token {
            CommentToken::Break => {
                x = sr.min.x + pad;
                y += line_h;
                continue;
            }
            CommentToken::Text(t) => {
                let w = p.measure_text(t, px, None).x;
                if x + w > max_x && x > sr.min.x + pad {
                    x = sr.min.x + pad;
                    y += line_h;
                }
                if y + line_h > sr.max.y {
                    return;
                }
                p.text(Pos2::new(x, y), t, px, st.palette.text_secondary, None);
                x += w + p.measure_text(" ", px, None).x;
            }
            CommentToken::NodeRef(t) => {
                let label = format!("#{t}");
                let w = p.measure_text(&label, px, None).x + pad;
                if x + w > max_x && x > sr.min.x + pad {
                    x = sr.min.x + pad;
                    y += line_h;
                }
                if y + line_h > sr.max.y {
                    return;
                }
                let chip = Rect::from_min_size(
                    Pos2::new(x, y - px * 0.15),
                    Vec2::new(w, px * 1.25),
                );
                p.rect_filled(chip, Rounding::same(px * 0.3), st.palette.accent_soft);
                p.text(
                    Pos2::new(x + pad * 0.5, y),
                    &label,
                    px,
                    st.palette.accent_active,
                    None,
                );
                x += w + p.measure_text(" ", px, None).x;
            }
        }
    }
}

/// A run of comment body text: plain words, `#node` references, and line
/// breaks. Splitting is presentation-only — the stored text never changes.
enum CommentToken<'a> {
    Text(&'a str),
    NodeRef(&'a str),
    Break,
}

fn tokenize_comment(text: &str) -> Vec<CommentToken<'_>> {
    let mut out = Vec::new();
    for (li, line) in text.split('\n').enumerate() {
        if li > 0 {
            out.push(CommentToken::Break);
        }
        for word in line.split_whitespace() {
            match word.strip_prefix('#') {
                // `#` alone, or `#` followed by nothing usable, stays text.
                Some(rest) if !rest.is_empty() => out.push(CommentToken::NodeRef(rest)),
                _ => out.push(CommentToken::Text(word)),
            }
        }
    }
    out
}

/// Resolve a `#node` reference: a node id, else the first node whose title
/// slugifies to it. Unresolvable references simply do nothing when clicked.
fn resolve_node_ref(state: &GraphEditorState, geoms: &[NodeGeom], token: &str) -> Option<u64> {
    if let Ok(id) = token.parse::<u64>() {
        if state.doc.node(id).is_some() {
            return Some(id);
        }
    }
    let want = token.to_lowercase();
    geoms
        .iter()
        .find(|g| slugify(&g.title) == want)
        .map(|g| g.id)
}

/// Lowercase, non-alphanumerics to `-` — the same shape a `#node-slug`
/// reference is written in.
fn slugify(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut dash = false;
    for ch in s.chars() {
        if ch.is_alphanumeric() {
            for c in ch.to_lowercase() {
                out.push(c);
            }
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
    }
    out.trim_end_matches('-').to_string()
}

/// The bar's fold-caret zone, in world space. It shares the bar with the drag
/// gesture, so it claims the press first.
fn caret_zone(bar: Rect, m: &GraphMetrics, zoom: f32) -> Rect {
    let w = (m.pad_x + m.pin_r * 2.4).max(RESIZE_GRAB_PX / zoom.max(0.01));
    Rect::from_min_size(bar.min, Vec2::new(w, bar.height()))
}

/// The height a comment's wrapped body needs. A comment auto-grows to its
/// text and never shrinks below it; the width stays author-set.
fn comment_min_h(c: &crate::engine::node_graph::CommentBox, m: &GraphMetrics) -> f32 {
    let fs = c.clamped_font_scale();
    let bar = m.comment_bar * fs;
    // Cheap estimate rather than a text measurement: this is a *floor*, and
    // the drawing pass clips to the box anyway. Roughly 1.9 chars per px of
    // body font at the wrap width.
    let px = BASE_TAG_PX * m.scale * fs * 1.2;
    let cols = ((c.rect[2] - m.pad_x * 2.0) / (px * 0.52)).max(1.0);
    let lines: f32 = c
        .text
        .split('\n')
        .map(|l| (l.chars().count() as f32 / cols).ceil().max(1.0))
        .sum();
    bar + lines * px * 1.35 + m.body_pad * 2.0
}

/// The `#node` token under a world point, if the pointer is over a comment's
/// body chip. Re-runs the body layout to find it, so the hit box can never
/// disagree with what was drawn.
fn comment_ref_at(
    state: &GraphEditorState,
    m: &GraphMetrics,
    pw: Pos2,
    st: &Style,
    zoom: f32,
    ui: &mut Ui,
) -> Option<String> {
    for c in state.doc.comments.iter().rev() {
        if c.collapsed {
            continue;
        }
        let wr = comment_rect(c, m);
        if !wr.contains(pw) {
            continue;
        }
        let fs = c.clamped_font_scale();
        let bar_h = m.comment_bar * fs;
        let px = st.fonts.small * fs;
        let pad = m.pad_x;
        let mut x = wr.min.x + pad;
        let mut y = wr.min.y + bar_h + m.label_gap;
        let line_h = px * 1.35;
        let max_x = wr.max.x - pad;
        let mut p = ui.painter();
        let space = p.measure_text(" ", px * zoom, None).x / zoom.max(0.01);
        for token in tokenize_comment(&c.text) {
            match token {
                CommentToken::Break => {
                    x = wr.min.x + pad;
                    y += line_h;
                }
                CommentToken::Text(t) => {
                    let w = p.measure_text(t, px * zoom, None).x / zoom.max(0.01);
                    if x + w > max_x && x > wr.min.x + pad {
                        x = wr.min.x + pad;
                        y += line_h;
                    }
                    x += w + space;
                }
                CommentToken::NodeRef(t) => {
                    let label = format!("#{t}");
                    let w = p.measure_text(&label, px * zoom, None).x / zoom.max(0.01) + pad;
                    if x + w > max_x && x > wr.min.x + pad {
                        x = wr.min.x + pad;
                        y += line_h;
                    }
                    let chip = Rect::from_min_size(
                        Pos2::new(x, y - px * 0.15),
                        Vec2::new(w, px * 1.25),
                    );
                    if chip.contains(pw) {
                        return Some(t.to_string());
                    }
                    x += w + space;
                }
            }
        }
        // The pointer was inside this comment; nothing behind it can match.
        return None;
    }
    None
}

/// A comment's drawn rect: collapsed folds it to its NOTE bar.
fn comment_rect(c: &crate::engine::node_graph::CommentBox, m: &GraphMetrics) -> Rect {
    let h = if c.collapsed {
        m.comment_bar * c.clamped_font_scale()
    } else {
        c.rect[3]
    };
    Rect::from_min_size(Pos2::new(c.rect[0], c.rect[1]), Vec2::new(c.rect[2], h))
}

/// A group's drawn rect: collapsed folds it to its title bar.
fn group_rect(g: &crate::engine::node_graph::GroupBox, m: &GraphMetrics) -> Rect {
    let h = if g.collapsed { m.group_bar } else { g.rect[3] };
    Rect::from_min_size(Pos2::new(g.rect[0], g.rect[1]), Vec2::new(g.rect[2], h))
}

/// One wire's resolved geometry for this frame: the endpoints the router
/// needs, the routed screen polyline, and everything the paint pass reads.
struct WireGeom {
    edge_index: usize,
    /// Graph-space endpoints (source pin border -> target pin border).
    a: Pos2,
    b: Pos2,
    /// Source pin type — a wire takes the color of what flows through it.
    ty: PinType,
    /// Routed + rounded polyline, already in screen space.
    screen: Vec<Pos2>,
    selected: bool,
    /// `TypeMismatch` — the only error that colors a wire.
    mismatched: bool,
}

impl WireGeom {
    fn is_exec(&self) -> bool {
        self.ty == PinType::Exec
    }

    fn width(&self, hovered: bool) -> f32 {
        match (self.is_exec(), self.selected || hovered) {
            (true, true) => WIRE_EXEC_SELECTED,
            (true, false) => WIRE_EXEC,
            (false, true) => WIRE_DATA_SELECTED,
            (false, false) => WIRE_DATA,
        }
    }
}

/// Route every visible edge once per frame. Routing happens in **graph
/// space** — the router never sees zoom — and only the finished polyline is
/// transformed, which is what keeps a wire's shape identical at 40% and 200%.
#[allow(clippy::too_many_arguments)]
fn build_wires(
    state: &GraphEditorState,
    geoms: &[NodeGeom],
    node_rects: &[Rect],
    errors: &ErrorIndex,
    prefs: &WirePrefs,
    scope: &CanvasScope,
    vis: Rect,
) -> Vec<WireGeom> {
    let mut out = Vec::with_capacity(state.doc.edges.len());
    for (edge_index, e) in state.doc.edges.iter().enumerate() {
        let src = geoms.iter().find(|g| g.id == e.from_node);
        let dst = geoms.iter().find(|g| g.id == e.to_node);
        let (Some(src), Some(dst)) = (src, dst) else {
            continue;
        };
        let (Some(a), Some(b)) = (
            src.wire_anchor(&e.from_pin, true),
            dst.wire_anchor(&e.to_pin, false),
        ) else {
            continue;
        };
        let meta = RouteMeta {
            src_rect: Some(src.rect),
            dst_rect: Some(dst.rect),
            target_pin_index: dst.pin_row(&e.to_pin, false),
            node_rects,
        };
        // Cull on the wire's own bounds, not the endpoints' box — a backward
        // lane or a spline bow reaches well outside it.
        if let Some(bounds) = router::wire_bounds(a, b, prefs, &meta) {
            let clip = bounds.intersect(vis);
            if clip.width() < 0.0 || clip.height() < 0.0 {
                continue;
            }
        }
        let ty = src
            .pins
            .iter()
            .find(|p| p.output && p.slug == e.from_pin)
            .map(|p| p.ty.clone())
            .unwrap_or(PinType::Exec);
        out.push(WireGeom {
            edge_index,
            a,
            b,
            ty,
            screen: wire_screen_points(a, b, prefs, &meta, scope),
            selected: state.selected_edges.contains(e),
            mismatched: errors.edges.contains(e),
        });
    }
    out
}

/// Route in graph space, round the corners there (so the radius scales with
/// the view like the rest of the geometry), then transform to screen.
fn wire_screen_points(
    a: Pos2,
    b: Pos2,
    prefs: &WirePrefs,
    meta: &RouteMeta,
    scope: &CanvasScope,
) -> Vec<Pos2> {
    // Spline is drawn as a real cubic; these samples exist so hover-testing
    // and culling see the same shape the user does.
    let pts = if prefs.style.is_orthogonal() {
        router::round_corners(&router::route(a, b, prefs, meta), prefs.corner_radius)
    } else {
        router::sample(a, b, prefs, meta)
    };
    pts.into_iter().map(|p| scope.world_to_screen(p)).collect()
}

#[allow(clippy::too_many_arguments)]
fn draw_wires(
    ui: &mut Ui,
    scope: &CanvasScope,
    wires: &[WireGeom],
    hovered: Option<usize>,
    prefs: &WirePrefs,
    st: &Style,
    lod: ZoomLod,
    selection_outline: Color,
) {
    let mut p = ui.painter();
    // Selected and hovered wires paint last so they are never buried under a
    // neighbour they cross.
    let order = wires
        .iter()
        .filter(|w| !w.selected && hovered != Some(w.edge_index))
        .chain(
            wires
                .iter()
                .filter(|w| w.selected || hovered == Some(w.edge_index)),
        );
    let status = Palette::invariant_status();
    for w in order {
        let is_hovered = hovered == Some(w.edge_index);
        // TypeMismatch is the ONLY error that colors a wire, and it outranks
        // hover — a broken wire should not look merely interesting.
        let color = if w.mismatched {
            status.error
        } else if w.selected {
            selection_outline
        } else if is_hovered {
            st.palette.focus_ring
        } else {
            wire_color(None, &w.ty)
        };
        // L4 collapses every wire to a hairline.
        let width = if lod.bar_only() { 1.0 } else { w.width(is_hovered) };
        stroke_wire(&mut p, w, prefs, scope, width, color);
        if w.mismatched && lod.rows() {
            if let Some(mid) = arc_length_midpoint(&w.screen) {
                let r = width * 2.2;
                p.line_segment(
                    Pos2::new(mid.x - r, mid.y - r),
                    Pos2::new(mid.x + r, mid.y + r),
                    width,
                    status.error,
                );
                p.line_segment(
                    Pos2::new(mid.x + r, mid.y - r),
                    Pos2::new(mid.x - r, mid.y + r),
                    width,
                    status.error,
                );
            }
        }
    }
}

/// The point halfway along a polyline **by arc length**, not the midpoint of
/// its endpoints — on an L-shaped route the latter is off the wire entirely.
fn arc_length_midpoint(pts: &[Pos2]) -> Option<Pos2> {
    if pts.len() < 2 {
        return pts.first().copied();
    }
    let total: f32 = pts.windows(2).map(|w| (w[1] - w[0]).length()).sum();
    if total <= f32::EPSILON {
        return pts.first().copied();
    }
    let half = total * 0.5;
    let mut walked = 0.0;
    for w in pts.windows(2) {
        let seg = (w[1] - w[0]).length();
        if walked + seg >= half {
            let t = if seg <= f32::EPSILON { 0.0 } else { (half - walked) / seg };
            return Some(Pos2::new(
                w[0].x + (w[1].x - w[0].x) * t,
                w[0].y + (w[1].y - w[0].y) * t,
            ));
        }
        walked += seg;
    }
    pts.last().copied()
}

/// Stroke one wire: a single cubic for Spline, a single polyline for the
/// orthogonal modes (one call, so the corner joins are continuous).
fn stroke_wire(
    p: &mut Painter,
    w: &WireGeom,
    prefs: &WirePrefs,
    scope: &CanvasScope,
    width: f32,
    color: Color,
) {
    if prefs.style.is_orthogonal() {
        if w.screen.len() >= 2 {
            p.polyline(&w.screen, width, color);
        }
        return;
    }
    let (c1, c2) = router::spline_controls(w.a, w.b, prefs.curve);
    p.bezier_cubic(
        scope.world_to_screen(w.a),
        scope.world_to_screen(c1),
        scope.world_to_screen(c2),
        scope.world_to_screen(w.b),
        width,
        color,
    );
}

/// The wire under the pointer, if any — per-segment distance in **screen**
/// space so a wire is exactly as easy to grab at 15% as at 220%.
fn wire_under(wires: &[WireGeom], pointer: Option<Pos2>) -> Option<usize> {
    let p = pointer?;
    wires
        .iter()
        .filter(|w| w.screen.len() >= 2)
        .map(|w| (w.edge_index, point_polyline_distance(p, &w.screen)))
        .filter(|(_, d)| *d <= WIRE_HOVER_PX)
        .min_by(|x, y| x.1.partial_cmp(&y.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(i, _)| i)
}

#[allow(clippy::too_many_arguments)]
fn draw_nodes(
    ui: &mut Ui,
    scope: &CanvasScope,
    state: &mut GraphEditorState,
    registry: &NodeRegistry,
    geoms: &[NodeGeom],
    errors: &ErrorIndex,
    st: &Style,
    m: &GraphMetrics,
    vis: Rect,
    zoom: f32,
    lod: ZoomLod,
    selection_outline: Color,
    widget_rects: &mut Vec<Rect>,
) {
    let status = Palette::invariant_status();
    // Collected during the paint pass, applied after (the widget pass needs
    // `&mut Ui`, which the painter borrow would otherwise hold).
    let mut pending_widgets: Vec<(u64, String, Rect, InlineKind)> = Vec::new();

    for g in geoms {
        let body = g.body_rect(lod, m);
        let clip = body.intersect(vis);
        if clip.width() <= 0.0 || clip.height() <= 0.0 {
            continue;
        }
        let srect = scope.world_rect_to_screen(body);
        let selected = state.selection.contains(&g.id);
        let round = Rounding::same(m.radius * zoom);
        let edge_col = if g.missing { status.error } else { g.edge_color() };

        let mut p = ui.painter();

        // A reroute is drawn as what it is: a typed dot on the wire.
        if g.reroute {
            let c = scope.world_to_screen(g.rect.center());
            let r = m.reroute_d() * 0.5 * zoom;
            let col = if g.errored {
                status.error
            } else {
                pin_color(Some(registry), &g.pins[0].ty)
            };
            p.circle_filled(c, r, col);
            p.circle_stroke(c, r, m.border, st.palette.stroke);
            if selected {
                p.circle_stroke(
                    c,
                    r + m.edge,
                    m.edge,
                    selection_outline.with_alpha(if state.primary == Some(g.id) {
                        1.0
                    } else {
                        SELECTION_REST_ALPHA
                    }),
                );
            }
            continue;
        }

        // L4: the node is a bar of its type color, nothing more.
        if lod.bar_only() {
            let bar = Rect::from_min_size(
                srect.min,
                Vec2::new(srect.width(), (L4_BAR_H * m.scale * zoom).max(1.0)),
            );
            p.rect_filled(bar, Rounding::ZERO, edge_col);
            continue;
        }

        // Body on `header`, header block on `elevated` — flat, no fill-tinted
        // title bar (the one thing both Blender and Unreal do that does not
        // survive this system's density).
        p.rect_filled(srect, round, st.palette.header);
        let header_rect =
            Rect::from_min_size(srect.min, Vec2::new(srect.width(), m.header_h * zoom));
        p.rect_filled(
            header_rect,
            Rounding { nw: round.nw, ne: round.ne, sw: 0.0, se: 0.0 },
            st.palette.elevated,
        );
        // Category identity: the reserved 2px top edge, deep tone.
        let edge_rect = Rect::from_min_size(
            srect.min,
            Vec2::new(srect.width(), (m.edge * zoom).max(1.0)),
        );
        p.rect_filled(
            edge_rect,
            Rounding { nw: round.nw, ne: round.ne, sw: 0.0, se: 0.0 },
            edge_col,
        );
        // 1px border — hairline, never scaled away. Status color reaches the
        // border on a node: there is no row to tint.
        p.rect_stroke(
            srect,
            round,
            m.border,
            if g.missing || g.errored { status.error } else { st.palette.stroke },
        );

        // Selection: the node keeps its fill and gains an offset outline in
        // `selection.outline`; last-clicked at 100%, the rest of the set 55%.
        if selected {
            let off = m.edge;
            let outer = Rect::from_min_max(
                Pos2::new(srect.min.x - off, srect.min.y - off),
                Pos2::new(srect.max.x + off, srect.max.y + off),
            );
            let alpha = if state.primary == Some(g.id) { 1.0 } else { SELECTION_REST_ALPHA };
            p.rect_stroke(
                outer,
                Rounding::same(round.nw + off),
                m.edge,
                selection_outline.with_alpha(alpha),
            );
        }

        if lod.glyphs() {
            let title_px = st.fonts.body * zoom;
            // Badges never stack: one glyph in the header's left gutter, by
            // precedence. Only errors exist so far — breakpoints and warnings
            // join the ladder when they land.
            let gutter = if g.errored || g.missing {
                let r = m.pin_r * zoom * 0.8;
                let c = Pos2::new(
                    srect.min.x + m.pad_x * zoom + r,
                    srect.min.y + m.header_h * zoom * 0.5,
                );
                p.circle_filled(c, r, status.error);
                p.text(
                    Pos2::new(c.x - title_px * 0.16, c.y - title_px * 0.52),
                    "!",
                    title_px * 0.9,
                    st.palette.elevated,
                    None,
                );
                r * 2.0 + m.label_gap * zoom
            } else {
                0.0
            };
            p.text(
                srect.min
                    + Vec2::new(m.pad_x * zoom + gutter, (m.header_h * zoom - title_px) * 0.5),
                &g.title,
                title_px,
                st.palette.text,
                None,
            );
            // 9px mono category tag, bright tone, right side of the header.
            let tag_px = m.tag_px * zoom;
            let tag_w = p
                .measure_text_family(&g.tag, tag_px, None, FontFamily::Mono)
                .x;
            p.text_family(
                Pos2::new(
                    srect.max.x - m.pad_x * zoom - tag_w,
                    srect.min.y + (m.header_h * zoom - tag_px) * 0.5,
                ),
                &g.tag,
                tag_px,
                g.tag_color(),
                None,
                FontFamily::Mono,
            );
        }

        if !lod.rows() {
            continue;
        }

        let label_px = st.fonts.small * zoom;
        for pin in &g.pins {
            let c = scope.world_to_screen(pin.dot_center);
            draw_pin(&mut p, pin, c, zoom, m, st, registry);
            // Pin-level errors ring the pin itself.
            if errors
                .pins
                .contains(&(g.id, pin.slug.clone(), pin.output))
            {
                p.circle_stroke(c, m.pin_r * zoom * 1.8, m.border, status.error);
            }
            // A ghost row gets a dashed rule so it reads as absent, not as a
            // pin someone forgot to label.
            if pin.ghost {
                let y = c.y + m.row_h * zoom * 0.45;
                dashed_line(
                    &mut p,
                    Pos2::new(srect.min.x + m.pad_x * zoom, y),
                    Pos2::new(srect.max.x - m.pad_x * zoom, y),
                    m.border,
                    st.palette.text_disabled,
                );
            }
            if !lod.pin_labels() {
                continue;
            }
            let label = pin_label(pin);
            if pin.output {
                let w = p.measure_text(&label, label_px, None).x;
                p.text(
                    Pos2::new(srect.max.x - m.label_inset() * zoom - w, c.y - label_px * 0.5),
                    &label,
                    label_px,
                    st.palette.text_secondary,
                    None,
                );
            } else {
                let x = srect.min.x + m.label_inset() * zoom;
                let lw = p
                    .text(
                        Pos2::new(x, c.y - label_px * 0.5),
                        &label,
                        label_px,
                        if pin.ghost {
                            st.palette.text_disabled
                        } else {
                            st.palette.text_secondary
                        },
                        None,
                    )
                    .x;
                // Inline value cell, right-aligned inside the row.
                if let Some(kind) = &pin.inline {
                    // Flows after the label, but never past the width the
                    // auto-sizer reserved for it.
                    let cell = Rect::from_min_size(
                        Pos2::new(
                            (x + lw + m.label_gap * zoom)
                                .min(srect.max.x - m.pad_x * zoom - m.value_w * zoom),
                            c.y - m.row_h * zoom * 0.4,
                        ),
                        Vec2::new(m.value_w * zoom, m.row_h * zoom * 0.8),
                    );
                    if lod.inline_widgets() && matches!(kind, InlineKind::Float(_) | InlineKind::Bool(_))
                    {
                        pending_widgets.push((g.id, pin.slug.clone(), cell, kind.clone()));
                    } else if lod.values() {
                        draw_inline_readonly(&mut p, cell, kind, label_px, st, zoom, m);
                    }
                }
            }
        }
    }

    // Editable widgets, after the painter borrow ends.
    for (node, slug, cell, kind) in pending_widgets {
        widget_rects.push(cell);
        inline_widget(ui, state, registry, node, &slug, cell, &kind, zoom);
    }
}

/// Non-editable inline value: a chip at L1, and the shapes L0 does not yet
/// edit (Color swatch, Enum/Asset/Vec chips, `Raw`).
fn draw_inline_readonly(
    p: &mut Painter,
    cell: Rect,
    kind: &InlineKind,
    px: f32,
    st: &Style,
    zoom: f32,
    m: &GraphMetrics,
) {
    let status = Palette::invariant_status();
    let round = Rounding::same((m.radius * 0.5 * zoom).max(1.0));
    match kind {
        InlineKind::Color(c) => {
            let sw = Rect::from_min_size(cell.min, Vec2::splat(cell.height()));
            let col = Color::rgba(c[0], c[1], c[2], c[3]);
            p.rect_filled(sw, round, col);
            p.rect_stroke(sw, round, m.border, st.palette.stroke_strong);
            p.text_family(
                Pos2::new(sw.max.x + m.label_gap * zoom, cell.center().y - px * 0.5),
                &format!("{:02X}{:02X}{:02X}", chan(c[0]), chan(c[1]), chan(c[2])),
                px,
                st.palette.text_mono,
                None,
                FontFamily::Mono,
            );
        }
        InlineKind::Raw(s) => {
            // Forward-compat data, not a blank: a warning-outlined chip.
            p.rect_stroke(cell, round, m.border, status.warning);
            let text = clip_text(p, s, px, cell.width() - m.label_gap * 2.0 * zoom);
            p.text_family(
                Pos2::new(cell.min.x + m.label_gap * zoom, cell.center().y - px * 0.5),
                &text,
                px,
                status.warning,
                None,
                FontFamily::Mono,
            );
        }
        other => {
            let text = match other {
                InlineKind::Float(x) => format!("{x}"),
                InlineKind::Bool(b) => b.to_string(),
                InlineKind::Chip(s) => s.clone(),
                _ => String::new(),
            };
            p.rect_filled(cell, round, st.palette.input);
            let w = cell.width() - m.label_gap * 2.0 * zoom;
            let text = clip_text(p, &text, px, w);
            p.text_family(
                Pos2::new(cell.min.x + m.label_gap * zoom, cell.center().y - px * 0.5),
                &text,
                px,
                st.palette.text_mono,
                None,
                FontFamily::Mono,
            );
        }
    }
}

fn chan(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Tail-trim `s` (with an ellipsis) until it fits `max_w` — value cells are
/// short and read left-to-right, unlike middle-truncated names.
fn clip_text(p: &mut Painter, s: &str, px: f32, max_w: f32) -> String {
    if max_w <= 0.0 || p.measure_text(s, px, None).x <= max_w {
        return s.to_string();
    }
    let chars: Vec<char> = s.chars().collect();
    for keep in (1..chars.len()).rev() {
        let mut c: String = chars[..keep].iter().collect();
        c.push('\u{2026}');
        if p.measure_text(&c, px, None).x <= max_w {
            return c;
        }
    }
    "\u{2026}".to_string()
}

/// Run a real crusty widget inside the canvas at `cell` (screen space), with
/// every style metric temporarily scaled by `zoom` so the control matches the
/// node it sits in. Any change opens (or continues) one coalesced
/// `SetProperty` gesture, flushed on pointer release.
#[allow(clippy::too_many_arguments)]
fn inline_widget(
    ui: &mut Ui,
    state: &mut GraphEditorState,
    registry: &NodeRegistry,
    node: u64,
    slug: &str,
    cell: Rect,
    kind: &InlineKind,
    zoom: f32,
) {
    let saved = ui.ctx().style;
    {
        let s = &mut ui.ctx_mut().style;
        s.fonts.body *= zoom;
        s.fonts.small *= zoom;
        s.fonts.mono *= zoom;
        s.fonts.title *= zoom;
        s.sizes.checkbox *= zoom;
        s.spacing.item *= zoom;
        s.spacing.button_padding =
            Vec2::new(s.spacing.button_padding.x * zoom, s.spacing.button_padding.y * zoom);
        s.metrics.control_height *= zoom;
        s.metrics.row_height *= zoom;
    }

    let id = Id::new(("graph_inline", node, slug));
    let mut changed: Option<PropValue> = None;
    ui.run_at(
        cell,
        Direction::TopDown,
        id,
        UiOptions { padding: Vec2::ZERO, spacing: 0.0 },
        |ui| match kind {
            InlineKind::Float(v) => {
                let mut x = *v;
                DragValue::new(&mut x)
                    .width(cell.width())
                    .height(cell.height())
                    .show(ui);
                if x != *v {
                    changed = Some(PropValue::Float(x));
                }
            }
            InlineKind::Bool(v) => {
                let mut b = *v;
                Checkbox::new(&mut b, "").show(ui);
                if b != *v {
                    changed = Some(PropValue::Bool(b));
                }
            }
            _ => {}
        },
    );
    ui.ctx_mut().style = saved;

    if let Some(v) = changed {
        state.begin_prop_edit(node, slug, registry);
        if let Some(n) = state.doc.node_mut(node) {
            n.properties.insert(slug.to_string(), v);
        }
    }
}

// ---------------------------------------------------------------------------
// Hit-testing helpers
// ---------------------------------------------------------------------------

/// Node centers (world) for group capture.
fn node_centers(geoms: &[NodeGeom]) -> Vec<(u64, [f32; 2])> {
    geoms
        .iter()
        .map(|g| (g.id, [g.rect.center().x, g.rect.center().y]))
        .collect()
}

fn pin_ty(geoms: &[NodeGeom], node: u64, slug: &str, output: bool) -> Option<PinType> {
    geoms
        .iter()
        .find(|g| g.id == node)?
        .pins
        .iter()
        .find(|p| p.output == output && p.slug == slug)
        .map(|p| p.ty.clone())
}

fn pin_under(
    geoms: &[NodeGeom],
    pw: Pos2,
    hit_w: f32,
) -> Option<(u64, String, PinType, bool)> {
    for g in geoms {
        for pin in &g.pins {
            if Rect::from_center_size(pin.dot_center, Vec2::splat(hit_w)).contains(pw) {
                return Some((g.id, pin.slug.clone(), pin.ty.clone(), pin.output));
            }
        }
    }
    None
}

fn node_under(geoms: &[NodeGeom], pw: Pos2, lod: ZoomLod, m: &GraphMetrics) -> Option<u64> {
    geoms
        .iter()
        .rev()
        .find(|g| g.body_rect(lod, m).contains(pw))
        .map(|g| g.id)
}

fn resolve_connection(
    state: &mut GraphEditorState,
    geoms: &[NodeGeom],
    pointer_world: Option<Pos2>,
    hit_w: f32,
    registry: &NodeRegistry,
) {
    let Some((from_node, from_pin, from_output)) = state
        .connect_drag
        .as_ref()
        .map(|d| (d.from_node, d.from_pin.clone(), d.from_output))
    else {
        return;
    };
    let (Some(pw), Some(src_ty)) =
        (pointer_world, pin_ty(geoms, from_node, &from_pin, from_output))
    else {
        return;
    };
    let Some((tn, ts, tty, to)) = pin_under(geoms, pw, hit_w) else {
        return;
    };
    if let Some(edge) = validate_connection(
        state, from_node, &from_pin, from_output, &src_ty, tn, &ts, to, &tty,
    ) {
        state.doc.edges.push(edge.clone());
        state.commit(GraphEdit::Connect(edge), registry);
    }
}

fn finish_node_drag(state: &mut GraphEditorState, registry: &NodeRegistry) {
    let Some(drag) = state.node_drag.take() else {
        return;
    };
    let ids: Vec<u64> = drag.originals.iter().map(|(id, _)| *id).collect();
    let delta = drag
        .originals
        .first()
        .and_then(|(id, start)| {
            state
                .doc
                .node(*id)
                .map(|n| [n.position[0] - start[0], n.position[1] - start[1]])
        })
        .unwrap_or([0.0, 0.0]);
    if delta[0].abs() > f32::EPSILON || delta[1].abs() > f32::EPSILON {
        state.commit(GraphEdit::MoveNodes { ids, delta }, registry);
    }
}

/// Finish a comment/group drag, recording one coalesced Move edit (P7).
fn finish_annotation_drag(state: &mut GraphEditorState, registry: &NodeRegistry) {
    let Some(d) = state.annotation_drag.take() else {
        return;
    };
    let min = if d.is_group {
        state.doc.groups.get(d.index).map(|g| [g.rect[0], g.rect[1]])
    } else {
        state.doc.comments.get(d.index).map(|c| [c.rect[0], c.rect[1]])
    };
    let Some(min) = min else {
        return;
    };
    let delta = [min[0] - d.rect_min0[0], min[1] - d.rect_min0[1]];
    if delta[0].abs() > f32::EPSILON || delta[1].abs() > f32::EPSILON {
        let edit = if d.is_group {
            GraphEdit::MoveGroup {
                index: d.index,
                node_ids: d.captured.iter().map(|(id, _)| *id).collect(),
                delta,
            }
        } else {
            GraphEdit::MoveComment { index: d.index, delta }
        };
        state.commit(edit, registry);
    }
}

/// Open the inline text editor for a comment/group (P7).
fn begin_annotation_edit(state: &mut GraphEditorState, is_group: bool, index: usize) {
    let (text, anchor) = if is_group {
        match state.doc.groups.get(index) {
            Some(g) => (g.title.clone(), [g.rect[0], g.rect[1]]),
            None => return,
        }
    } else {
        match state.doc.comments.get(index) {
            Some(c) => (c.text.clone(), [c.rect[0], c.rect[1]]),
            None => return,
        }
    };
    state.clear_selection();
    if is_group {
        state.sel_group = Some(index);
    } else {
        state.sel_comment = Some(index);
    }
    state.annotation_drag = None;
    state.editing = Some(AnnotationEdit {
        is_group,
        index,
        buffer: text.clone(),
        original: text,
        anchor_world: anchor,
        first_frame: true,
    });
}

/// World bbox `(min, max)` of the given node geoms, or `None` if empty (used
/// by the F/A frame shortcuts).
fn geoms_bbox<'a>(gs: impl Iterator<Item = &'a NodeGeom>) -> Option<(Vec2, Vec2)> {
    let (mut minx, mut miny, mut maxx, mut maxy) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    let mut any = false;
    for g in gs {
        minx = minx.min(g.rect.min.x);
        miny = miny.min(g.rect.min.y);
        maxx = maxx.max(g.rect.max.x);
        maxy = maxy.max(g.rect.max.y);
        any = true;
    }
    any.then_some((Vec2::new(minx, miny), Vec2::new(maxx, maxy)))
}

/// Bbox of whatever is currently selected — nodes, or the selected comment /
/// group frame. `None` when the canvas has no selection at all.
fn selection_bbox(state: &GraphEditorState, geoms: &[NodeGeom]) -> Option<(Vec2, Vec2)> {
    if !state.selection.is_empty() {
        return geoms_bbox(geoms.iter().filter(|g| state.selection.contains(&g.id)));
    }
    let r = if let Some(i) = state.sel_group {
        state.doc.groups.get(i).map(|g| g.rect)
    } else if let Some(i) = state.sel_comment {
        state.doc.comments.get(i).map(|c| c.rect)
    } else {
        None
    }?;
    Some((
        Vec2::new(r[0], r[1]),
        Vec2::new(r[0] + r[2], r[1] + r[3]),
    ))
}

/// World bbox `(min, max)` of the whole graph — nodes + groups + comments.
fn content_bbox(state: &GraphEditorState, geoms: &[NodeGeom]) -> Option<(Vec2, Vec2)> {
    let mut b = geoms_bbox(geoms.iter());
    let mut fold = |x0: f32, y0: f32, x1: f32, y1: f32| {
        b = Some(match b {
            Some((mn, mx)) => (
                Vec2::new(mn.x.min(x0), mn.y.min(y0)),
                Vec2::new(mx.x.max(x1), mx.y.max(y1)),
            ),
            None => (Vec2::new(x0, y0), Vec2::new(x1, y1)),
        });
    };
    for g in &state.doc.groups {
        fold(g.rect[0], g.rect[1], g.rect[0] + g.rect[2], g.rect[1] + g.rect[3]);
    }
    for c in &state.doc.comments {
        fold(c.rect[0], c.rect[1], c.rect[0] + c.rect[2], c.rect[1] + c.rect[3]);
    }
    b
}

/// Inline text-edit popup for the annotation in `state.editing` (P7).
fn edit_popup(
    ui: &mut Ui,
    state: &mut GraphEditorState,
    registry: &NodeRegistry,
    canvas_rect: Rect,
) {
    let Some((is_group, index, anchor_world, first_frame, original)) = state
        .editing
        .as_ref()
        .map(|e| (e.is_group, e.index, e.anchor_world, e.first_frame, e.original.clone()))
    else {
        return;
    };
    let view = state.view;
    let screen = Pos2::new(
        canvas_rect.min.x + (anchor_world[0] - view.pan.x) * view.zoom,
        canvas_rect.min.y + (anchor_world[1] - view.pan.y) * view.zoom,
    );
    let st = ui.style();
    let w = BASE_MAX_W * (st.metrics.row_height / BASE_ROW_H) * 0.625;
    let h = st.metrics.control_height;
    let rect = Rect::from_min_size(screen, Vec2::new(w, h));
    {
        let mut p = ui.painter();
        p.rect_filled(rect, st.rounding.small, st.palette.elevated);
        p.rect_stroke(rect, st.rounding.small, st.metrics.border, st.palette.accent_active);
    }
    let mut buffer = state.editing.as_ref().map(|e| e.buffer.clone()).unwrap_or_default();
    let (out, _) = ui.run_at(
        rect,
        Direction::TopDown,
        Id::new(("graph_annot_edit", is_group, index)),
        UiOptions { padding: Vec2::splat(st.metrics.border * 2.0), spacing: 0.0 },
        |ui| {
            TextEdit::new(&mut buffer)
                .width(w - st.metrics.border * 4.0)
                .request_focus(first_frame)
                .show_full(ui)
        },
    );
    if out.cancelled {
        state.editing = None;
        return;
    }
    let commit = out.submitted || (!out.focused && !first_frame);
    if commit {
        state.editing = None;
        // Only record the edit if the target still exists — the annotation may
        // have been deleted while the editor was open (guard the index so
        // undo/redo never indexes out of bounds).
        let applied = if buffer == original {
            false
        } else if is_group {
            match state.doc.groups.get_mut(index) {
                Some(g) => {
                    g.title = buffer.clone();
                    true
                }
                None => false,
            }
        } else {
            match state.doc.comments.get_mut(index) {
                Some(c) => {
                    c.text = buffer.clone();
                    true
                }
                None => false,
            }
        };
        if applied {
            let edit = if is_group {
                GraphEdit::SetGroupTitle { index, old: original, new: buffer }
            } else {
                GraphEdit::SetCommentText { index, old: original, new: buffer }
            };
            state.commit(edit, registry);
        }
    } else if let Some(e) = state.editing.as_mut() {
        e.buffer = buffer;
        e.first_frame = false;
    }
}

/// The per-frame inputs `handle_marquee` needs, bundled so the signature stays
/// readable.
struct MarqueeCtx {
    pointer_world: Option<Pos2>,
    pointer_pressed: bool,
    pointer_down: bool,
    released: bool,
    blocked: bool,
    mods: Modifiers,
    lod: ZoomLod,
    hit_w: f32,
}

fn handle_marquee(
    ui: &mut Ui,
    scope: &CanvasScope,
    state: &mut GraphEditorState,
    geoms: &[NodeGeom],
    c: MarqueeCtx,
    m: &GraphMetrics,
    st: &Style,
) {
    if c.pointer_pressed
        && state.node_drag.is_none()
        && state.connect_drag.is_none()
        && state.annotation_drag.is_none()
        && !c.blocked
    {
        if let Some(pw) = c.pointer_world {
            if node_under(geoms, pw, c.lod, m).is_none()
                && pin_under(geoms, pw, c.hit_w).is_none()
            {
                state.marquee = Some([pw.x, pw.y]);
                // Captured at press: releasing the modifier mid-drag must not
                // change what the gesture means. ⇧ adds, Alt subtracts.
                state.marquee_mode = if c.mods.contains(Modifiers::SHIFT) {
                    MarqueeMode::Add
                } else if c.mods.contains(Modifiers::ALT) {
                    MarqueeMode::Subtract
                } else {
                    MarqueeMode::Replace
                };
            }
        }
    }
    let Some(start) = state.marquee else {
        return;
    };
    let pw = c.pointer_world.unwrap_or(Pos2::new(start[0], start[1]));
    let world_rect = Rect::from_min_max(
        Pos2::new(start[0].min(pw.x), start[1].min(pw.y)),
        Pos2::new(start[0].max(pw.x), start[1].max(pw.y)),
    );
    {
        // Accent on the canvas is spent only on the compile chip, the marquee
        // and drag-time alignment guides — this is one of the three.
        let srect = scope.world_rect_to_screen(world_rect);
        let mut p = ui.painter();
        p.rect_filled(
            srect,
            Rounding::ZERO,
            st.palette.accent_active.with_alpha(MARQUEE_FILL_ALPHA),
        );
        p.rect_stroke(srect, Rounding::ZERO, m.border, st.palette.accent_active);
    }
    if c.released || !c.pointer_down {
        let hits: Vec<u64> = geoms
            .iter()
            .filter(|g| {
                let i = g.body_rect(c.lod, m).intersect(world_rect);
                i.width() > 0.0 && i.height() > 0.0
            })
            .map(|g| g.id)
            .collect();
        match state.marquee_mode {
            MarqueeMode::Replace => {
                state.selection = hits.into_iter().collect();
                state.primary = None;
                // A replace-marquee is "select exactly this", so it drops any
                // wire selection too — including the empty-canvas click that
                // deselects everything.
                state.selected_edges.clear();
            }
            MarqueeMode::Add => state.selection.extend(hits),
            MarqueeMode::Subtract => {
                for id in hits {
                    state.selection.remove(&id);
                }
                state.primary = state.primary.filter(|id| state.selection.contains(id));
            }
        }
        state.marquee = None;
        state.marquee_mode = MarqueeMode::Replace;
    }
}

fn create_menu(
    ui: &mut Ui,
    state: &mut GraphEditorState,
    registry: &NodeRegistry,
    geoms: &[NodeGeom],
    subgraph_assets: &[String],
    open_at: Option<Pos2>,
) {
    let world = state.create_menu_world;
    let has_selection = !state.selection.is_empty();
    let align_ready = state.selection.len() >= 3;
    let align_rects = selected_rects(state, geoms);
    let mut align: Option<AlignMode> = None;
    let search = &mut state.create_menu_search;
    let mut chosen: Option<String> = None;
    let mut chosen_subgraph: Option<String> = None;
    let mut add_comment = false;
    let mut add_group = false;
    crusty_gui::widgets::context_menu_at(ui, "graph_create_menu", open_at, |ui| {
        ui.menu_group_header("Add Node");
        TextEdit::new(search).hint("Search\u{2026}").width(170.0).show(ui);
        let needle = search.to_lowercase();
        if needle.is_empty() && align_ready {
            // Align & distribute only makes sense for 3+ nodes, so the
            // section is absent rather than dimmed below that.
            ui.menu_group_header("Align & Distribute");
            for mode in AlignMode::ALL {
                if ui.menu_item(mode.label()) {
                    align = Some(mode);
                }
            }
        }
        if needle.is_empty() {
            ui.menu_group_header("Annotate");
            if ui.menu_item("Add Comment") {
                add_comment = true;
            }
            if ui.menu_item_enabled("Add Group around selection", has_selection) {
                add_group = true;
            }
        }
        for (cat, descs) in registry.by_category() {
            let rows: Vec<_> = descs
                .iter()
                .filter(|d| {
                    needle.is_empty()
                        || d.name.to_lowercase().contains(&needle)
                        || d.id.to_lowercase().contains(&needle)
                })
                .collect();
            if rows.is_empty() {
                continue;
            }
            ui.menu_group_header(cat);
            for d in rows {
                if ui.menu_item(&d.name) {
                    chosen = Some(d.id.clone());
                }
            }
        }
        // Subgraph assets (`.subgraph`) as instance nodes.
        let subs: Vec<(&String, String)> = subgraph_assets
            .iter()
            .map(|p| {
                let stem = std::path::Path::new(p)
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| p.clone());
                (p, stem)
            })
            .filter(|(p, stem)| {
                needle.is_empty()
                    || stem.to_lowercase().contains(&needle)
                    || p.to_lowercase().contains(&needle)
            })
            .collect();
        if !subs.is_empty() {
            ui.menu_group_header("Subgraph");
            for (path, stem) in subs {
                if ui.menu_item(&stem) {
                    chosen_subgraph = Some(path.clone());
                }
            }
        }
    });
    if let Some(mode) = align {
        state.align_nodes(&align_rects, mode, registry);
        state.create_menu_world = None;
        return;
    }
    if let Some(pos) = world {
        if let Some(type_id) = chosen {
            state.add_node(&type_id, pos, registry);
            state.create_menu_world = None;
        } else if let Some(path) = chosen_subgraph {
            state.add_subgraph_node(&path, pos, registry);
            state.create_menu_world = None;
        } else if add_comment {
            state.add_comment(pos, registry);
            state.create_menu_world = None;
        } else if add_group {
            state.add_group_around_selection(registry);
            state.create_menu_world = None;
        }
    }
}

/// The validation **count chip**, pinned to the canvas' top-left corner.
///
/// Errors are anchored to the thing that is wrong — node border + badge, pin
/// ring, or the wire — so the corner is demoted to a count, per the spec.
/// Clicking the count cycles the anchored errors (framing and selecting each
/// anchor, which is the mechanism `F8` will reuse); the `n docs` segment
/// opens the compiler rows for the errors nothing on the canvas can own.
///
/// Returns a request to open another graph tab, if a cycle breadcrumb was
/// clicked in the popover.
#[allow(clippy::too_many_arguments)]
fn error_chip(
    ui: &mut Ui,
    rect: Rect,
    state: &mut GraphEditorState,
    errors: &ErrorIndex,
    geoms: &[NodeGeom],
    viewport: Vec2,
    zoom_min: f32,
    zoom_max: f32,
    frame_request: &mut Option<CanvasView>,
    open_subgraph: &mut Option<String>,
) {
    if errors.is_empty() {
        state.error_popover = false;
        return;
    }
    let st = ui.style();
    let status = Palette::invariant_status();
    let pad = st.spacing.padding * 0.75;
    let font = st.fonts.small;
    let h = st.metrics.control_height;

    let count = format!("{}", errors.total());
    let mut cw = ui.painter().measure_text_family(&count, font, None, FontFamily::Mono).x;
    cw += pad * 2.0 + font;
    let chip = Rect::from_min_size(
        rect.min + Vec2::splat(st.spacing.padding),
        Vec2::new(cw, h),
    );
    let id = ui.alloc_id("graph_error_chip");
    let resp = ui.interact(id, chip);
    {
        let mut p = ui.painter();
        // Count chips are filters: status tint fill, status border, a dot.
        p.rect_filled(chip, st.rounding.small, status.error.with_alpha(0.13));
        p.rect_stroke(chip, st.rounding.small, st.metrics.border, status.error);
        p.circle_filled(
            Pos2::new(chip.min.x + pad, chip.center().y),
            font * 0.22,
            status.error,
        );
        p.text_family(
            Pos2::new(chip.min.x + pad + font * 0.5, chip.center().y - font * 0.62),
            &count,
            font,
            status.error,
            None,
            FontFamily::Mono,
        );
    }
    if resp.hovered {
        ui.tooltip_for(chip, "Click to cycle validation errors");
    }
    if resp.clicked {
        cycle_error(state, errors, geoms, viewport, zoom_min, zoom_max, frame_request);
    }

    // Doc-level errors have no canvas anchor, so they get compiler rows.
    if errors.document.is_empty() {
        state.error_popover = false;
        return;
    }
    let label = format!("{} doc", errors.document.len());
    let lw = ui.painter().measure_text_family(&label, font, None, FontFamily::Mono).x
        + pad * 2.0;
    let more = Rect::from_min_size(
        Pos2::new(chip.max.x + st.spacing.item, chip.min.y),
        Vec2::new(lw, h),
    );
    let mid = ui.alloc_id("graph_error_docs");
    let mresp = ui.interact(mid, more);
    {
        let mut p = ui.painter();
        p.rect_filled(more, st.rounding.small, st.palette.elevated);
        p.rect_stroke(more, st.rounding.small, st.metrics.border, st.palette.stroke_strong);
        p.text_family(
            Pos2::new(more.min.x + pad, more.center().y - font * 0.62),
            &label,
            font,
            st.palette.text_secondary,
            None,
            FontFamily::Mono,
        );
    }
    if mresp.clicked {
        state.error_popover = !state.error_popover;
    }
    if !state.error_popover {
        return;
    }

    // Compiler rows. Document errors only — reference errors already drew on
    // the node that pulled them in, and the two stay visually separate.
    let mut rows: Vec<String> = Vec::new();
    for e in &errors.document {
        rows.push(format!("{e}"));
    }
    let mut w: f32 = 0.0;
    for r in &rows {
        w = w.max(ui.painter().measure_text_family(r, font, None, FontFamily::Mono).x);
    }
    let line_h = font * 1.7;
    let panel = Rect::from_min_size(
        Pos2::new(chip.min.x, more.max.y + st.spacing.item),
        Vec2::new(w + pad * 2.0, line_h * rows.len() as f32 + pad * 2.0),
    );
    {
        let mut p = ui.painter();
        p.rect_filled(panel, st.rounding.panel, st.palette.elevated);
        p.rect_stroke(panel, st.rounding.panel, st.metrics.border, st.palette.stroke_strong);
    }
    let mut y = panel.min.y + pad;
    for (i, e) in errors.document.iter().enumerate() {
        let row = Rect::from_min_size(
            Pos2::new(panel.min.x + pad, y),
            Vec2::new(panel.width() - pad * 2.0, line_h),
        );
        // A cycle is a clickable mono breadcrumb: each hop opens that graph.
        if let Some(chain) = e.cycle_chain() {
            let rid = ui.alloc_id(("graph_cycle_row", i));
            let r = ui.interact(rid, row);
            if r.hovered {
                ui.painter().rect_filled(row, st.rounding.small, st.palette.hover);
            }
            if r.clicked {
                if let Some(first) = chain.first() {
                    *open_subgraph = Some(first.clone());
                }
            }
        }
        ui.painter().text_family(
            Pos2::new(row.min.x, row.center().y - font * 0.62),
            &rows[i],
            font,
            if e.cycle_chain().is_some() {
                st.palette.accent_active
            } else {
                st.palette.text_secondary
            },
            None,
            FontFamily::Mono,
        );
        y += line_h;
    }
}

/// Step to the next anchored error, selecting and framing whatever it is
/// about. This is the mechanism `F8` / `⇧F8` will drive.
fn cycle_error(
    state: &mut GraphEditorState,
    errors: &ErrorIndex,
    geoms: &[NodeGeom],
    viewport: Vec2,
    zoom_min: f32,
    zoom_max: f32,
    frame_request: &mut Option<CanvasView>,
) {
    if errors.ordered.is_empty() {
        return;
    }
    // Skip the doc-level ones: there is nothing on the canvas to frame.
    let anchored: Vec<&GraphError> = errors
        .ordered
        .iter()
        .filter(|e| e.anchor() != ErrorAnchor::Document)
        .collect();
    if anchored.is_empty() {
        state.error_popover = true;
        return;
    }
    let i = state.error_cursor % anchored.len();
    state.error_cursor = (state.error_cursor + 1) % anchored.len();

    let node = match anchored[i].anchor() {
        ErrorAnchor::Node(id) => Some(id),
        ErrorAnchor::Pin { node, .. } | ErrorAnchor::GhostPin { node, .. } => Some(node),
        ErrorAnchor::Edge(edge) => {
            state.selected_edges.clear();
            state.selected_edges.insert(edge.clone());
            Some(edge.to_node)
        }
        ErrorAnchor::Document => None,
    };
    let Some(node) = node else { return };
    if !matches!(anchored[i].anchor(), ErrorAnchor::Edge(_)) {
        state.select_only(node);
    }
    if let Some((mn, mx)) = geoms_bbox(geoms.iter().filter(|g| g.id == node)) {
        *frame_request = Some(frame_view(mn, mx, viewport, zoom_min, zoom_max));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lod_thresholds_match_the_spec_ladder() {
        assert_eq!(ZoomLod::from_zoom(2.20), ZoomLod::L0);
        assert_eq!(ZoomLod::from_zoom(0.90), ZoomLod::L0);
        assert_eq!(ZoomLod::from_zoom(0.899), ZoomLod::L1);
        assert_eq!(ZoomLod::from_zoom(0.60), ZoomLod::L1);
        assert_eq!(ZoomLod::from_zoom(0.599), ZoomLod::L2);
        assert_eq!(ZoomLod::from_zoom(0.35), ZoomLod::L2);
        assert_eq!(ZoomLod::from_zoom(0.349), ZoomLod::L3);
        assert_eq!(ZoomLod::from_zoom(0.15), ZoomLod::L3);
        assert_eq!(ZoomLod::from_zoom(0.149), ZoomLod::L4);
    }

    #[test]
    fn lod_gates_are_monotonic() {
        use ZoomLod::*;
        // Detail only ever increases up the ladder — no gate turns back off.
        let gates: [(fn(ZoomLod) -> bool, &str); 5] = [
            (ZoomLod::inline_widgets, "inline_widgets"),
            (ZoomLod::values, "values"),
            (ZoomLod::pin_labels, "pin_labels"),
            (ZoomLod::rows, "rows"),
            (ZoomLod::glyphs, "glyphs"),
        ];
        for (g, name) in gates {
            let seq = [L4, L3, L2, L1, L0].map(g);
            for w in seq.windows(2) {
                assert!(w[0] <= w[1], "{name} is not monotonic across the ladder");
            }
        }
        // The documented cut points.
        assert!(!L1.inline_widgets() && L0.inline_widgets());
        assert!(!L2.pin_labels() && L1.pin_labels());
        assert!(!L3.rows() && L2.rows());
        assert!(!L3.glyphs() && L2.glyphs());
        assert!(L4.bar_only() && !L3.bar_only());
    }

    #[test]
    fn tag_precedence_is_sub_pure_event_category() {
        use crate::engine::node_graph::{NodeRealm, PinDescriptor};
        let desc = |pure: bool, ins: Vec<PinType>, outs: Vec<PinType>| NodeDescriptor {
            id: "t".into(),
            name: "T".into(),
            category: "Gameplay".into(),
            version: 1,
            inputs: ins
                .into_iter()
                .enumerate()
                .map(|(i, t)| PinDescriptor::new(&format!("i{i}"), "", t))
                .collect(),
            outputs: outs
                .into_iter()
                .enumerate()
                .map(|(i, t)| PinDescriptor::new(&format!("o{i}"), "", t))
                .collect(),
            pure,
            realm: NodeRealm::Shared,
            deterministic: true,
        };

        // SUB beats everything, even a pure descriptor.
        let pure = desc(true, vec![PinType::Float], vec![PinType::Float]);
        assert_eq!(derive_tag(true, Some(&pure), Some("Gameplay")), "SUB");
        assert_eq!(derive_tag(false, Some(&pure), Some("Gameplay")), "PURE");
        // Exec out and no exec in = an event entry point.
        let event = desc(false, vec![], vec![PinType::Exec]);
        assert_eq!(derive_tag(false, Some(&event), Some("Gameplay")), "EVENT");
        // Exec on both sides is an ordinary impure node — category tag.
        let flow = desc(false, vec![PinType::Exec], vec![PinType::Exec]);
        assert_eq!(derive_tag(false, Some(&flow), Some("Gameplay")), "GAMEP");
        // Unregistered type: no descriptor, still tagged by category.
        assert_eq!(derive_tag(false, None, Some("Dev")), "DEV");
        assert_eq!(derive_tag(false, None, None), "");
    }

    #[test]
    fn vector_pins_carry_their_arity_in_the_label() {
        let pin = |ty: PinType| PinGeom {
            slug: "p".into(),
            label: "Position".into(),
            ty,
            output: false,
            row: 0,
            ghost: false,
            wire_anchor: Pos2::ZERO,
            dot_center: Pos2::ZERO,
            connected: false,
            inline: None,
        };
        assert_eq!(pin_label(&pin(PinType::Vec2)), "Position \u{b7}2");
        assert_eq!(pin_label(&pin(PinType::Vec3)), "Position \u{b7}3");
        assert_eq!(pin_label(&pin(PinType::Vec4)), "Position \u{b7}4");
        assert_eq!(pin_label(&pin(PinType::Float)), "Position");
    }

    #[test]
    fn metrics_scale_off_the_row_height_token() {
        let mut st = Style::steel();
        let base = GraphMetrics::new(&st);
        assert!((base.scale - 1.0).abs() < 1e-6);
        assert!((base.header_h - BASE_HEADER_H).abs() < 1e-6);
        assert!((base.row_h - BASE_ROW_H).abs() < 1e-6);
        assert!((base.min_w - BASE_MIN_W).abs() < 1e-6);

        // Spacious (ui_scale 1.15) arrives as a pre-scaled row height.
        st.metrics.row_height = BASE_ROW_H * 1.15;
        let big = GraphMetrics::new(&st);
        assert!((big.scale - 1.15).abs() < 1e-4);
        assert!((big.header_h - BASE_HEADER_H * 1.15).abs() < 1e-3);
        assert!((big.max_w - BASE_MAX_W * 1.15).abs() < 1e-3);
    }

    #[test]
    fn pin_hit_target_never_shrinks_below_18_screen_px() {
        let m = GraphMetrics::new(&Style::steel());
        // At 1x the world floor wins; zoomed out, the screen floor does.
        assert!((m.pin_hit_w(1.0) - 18.0).abs() < 1e-4);
        assert!((m.pin_hit_w(2.0) - 13.5).abs() < 1e-4);
        for zoom in [0.15, 0.25, 0.5, 1.0, 2.2] {
            assert!(
                m.pin_hit_w(zoom) * zoom >= HIT_SCREEN_W - 1e-3,
                "hit target collapses below 18 screen px at zoom {zoom}"
            );
        }
    }

    #[test]
    fn node_collapses_to_its_header_below_l2() {
        let m = GraphMetrics::new(&Style::steel());
        let g = NodeGeom {
            id: 0,
            rect: Rect::from_min_size(Pos2::ZERO, Vec2::new(168.0, 120.0)),
            title: "T".into(),
            tag: "MATH".into(),
            category: Some("Math".into()),
            tint: None,
            missing: false,
            errored: false,
            reroute: false,
            pins: vec![],
        };
        assert_eq!(g.body_rect(ZoomLod::L2, &m).height(), 120.0);
        assert_eq!(g.body_rect(ZoomLod::L0, &m).height(), 120.0);
        assert!((g.body_rect(ZoomLod::L3, &m).height() - m.header_h).abs() < 1e-6);
        assert!((g.body_rect(ZoomLod::L4, &m).height() - m.header_h).abs() < 1e-6);
    }

    #[test]
    fn tint_overrides_the_category_edge_but_not_the_tag() {
        let mut g = NodeGeom {
            id: 0,
            rect: Rect::from_min_size(Pos2::ZERO, Vec2::new(168.0, 120.0)),
            title: "T".into(),
            tag: "MATH".into(),
            category: Some("Math".into()),
            tint: None,
            missing: false,
            errored: false,
            reroute: false,
            pins: vec![],
        };
        assert_eq!(g.edge_color(), category_color("Math"));
        g.tint = Some(9);
        assert_eq!(g.edge_color(), ramp()[9].deep);
        assert_eq!(g.tag_color(), category_tag_color("Math"), "tag keeps the category");
        // Out-of-range indices wrap instead of panicking on a hand-edited asset.
        g.tint = Some(200);
        assert_eq!(g.edge_color(), ramp()[(200 % 12) as usize].deep);
    }

    #[test]
    fn inline_kind_covers_every_prop_value() {
        assert!(matches!(InlineKind::of(&PropValue::Float(1.0)), InlineKind::Float(_)));
        assert!(matches!(InlineKind::of(&PropValue::Bool(true)), InlineKind::Bool(_)));
        assert!(matches!(
            InlineKind::of(&PropValue::Color([0.0; 4])),
            InlineKind::Color(_)
        ));
        assert!(matches!(
            InlineKind::of(&PropValue::Raw("(x:1)".into())),
            InlineKind::Raw(_)
        ));
        // Everything else falls back to a read-only chip.
        for v in [
            PropValue::Vec2([1.0, 2.0]),
            PropValue::Vec3([1.0, 2.0, 3.0]),
            PropValue::Vec4([1.0; 4]),
            PropValue::Enum("Variant".into()),
            PropValue::Asset("textures/a.png".into()),
        ] {
            assert!(matches!(InlineKind::of(&v), InlineKind::Chip(_)), "{v:?}");
        }
    }
}
