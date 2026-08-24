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

use crusty_gui::context::{CursorIcon, Direction, Ui, UiOptions};
use crusty_gui::id::Id;
use crusty_gui::input::{Key, Modifiers};
use crusty_gui::math::{Color, Pos2, Rect, Rounding, Vec2};
use crusty_gui::paint::Painter;
use crusty_gui::style::Style;
use crusty_gui::text::FontFamily;
use crusty_gui::widgets::{
    Button, Canvas, CanvasScope, CanvasView, Checkbox, ComboBox, DragValue, ScrollArea,
    SelectableValue, Slider, TextEdit,
};

use super::keymap::{Action, ActionStatus, Context, Keymap};
use super::graph_editor::{
    anchored_comments, frame_view, nodes_captured_by_rect, prop_display, AlignMode, Annotation,
    AnnotationDrag, AnnotationEdit, AnnotationResize, ConnectDrag, DomainError, GraphDomain,
    GraphEdit, GraphEditorState, GraphOpenRequest,
    GraphFragment, MarqueeMode, NewVarDraft, NodeDrag, PayloadConfirm, PayloadDraft,
    PeekDrag, PeekDragKind,
    payload_reader_count, variable_matches, variable_mismatch, variable_node_ids, variable_slug,
    variables_view,
    retype_default_outcome, pin_type_label, ResizeHandle, VarConfirm, VarDrop, VarListRow,
    DEFAULT_PAYLOAD_TYPE, watch_chip_text, Watch, WATCH_STALE_SECS,
    ANNOTATION_MIN_H, ANNOTATION_MIN_W, FindState, PaletteDragSource, PaletteState,
    BOOKMARK_SLOTS, TOAST_MS,
    region_find_matches, rule_scope_registry,
};
use super::anim_preview::{AnimParamEdit, AnimPreview};
use super::graph_exec_viz::{DebugRequest, ExecInstance, GraphExecViz, STEADY_HOT_HZ};
use super::graph_palette::{self, PaletteEntry, PinFilter};
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
    std_events::{
        EVENT_ACTION_PROP, EVENT_CUSTOM_TYPE_ID, EVENT_INPUT_ACTION_TYPE_ID, EVENT_NAME_PROP,
        EVENT_PAYLOAD_PREFIX, PAYLOAD_PIN_TYPES,
    },
    CurveResolver, DocDescriptors, Edge, ErrorAnchor, GraphDoc, GraphError, GraphResolver,
    NodeDescriptor, NodeInst,
    NodeKind, NodeRegistry, PinType, PropValue, REROUTE_IN, REROUTE_OUT, REROUTE_TYPE_ID,
    SUBGRAPH_TYPE_ID, VAR_PROP,
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
const BASE_DIAMOND_W: f32 = 9.0;
const BASE_COLOR_SQ_W: f32 = 9.0;
const BASE_TAG_PX: f32 = 9.0;
const BASE_PAD_X: f32 = 8.0;
const BASE_COL_GAP: f32 = 12.0;
const BASE_LABEL_GAP: f32 = 4.0;
/// Reserved width for an inline value/widget cell.
const BASE_VALUE_W: f32 = 56.0;
/// …and the most one may grow to when its content is text. A number fits 56
/// units; a string constant ("one second later") does not, and a field that
/// paints past the node it belongs to is worse than a wide node. Past this the
/// text elides inside the cell — a node is not a text editor.
const BASE_VALUE_W_MAX: f32 = 150.0;
/// Reserved width for a **config** row's cell. Wider than a pin's value cell
/// because what it holds is a name — a variable slug, an event name, an input
/// action — and 56 units truncates every one of them.
const BASE_CONFIG_VALUE_W: f32 = 104.0;
/// The ✕ column on a payload row, world units. Reserved in the auto-width so
/// the remove affordance never eats into the type dropdown.
const BASE_CONFIG_REMOVE_W: f32 = 12.0;
/// The config band's own surface: `input` tone washed this far over the node
/// fill (DESIGN-graphscripting ▸ Surface 2). A colorless second channel — it
/// survives colour-vision deficiency and low zoom, where "no dot + secondary
/// label" does not.
const CONFIG_BAND_WASH: f32 = 0.70;
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
/// How close a dragged node's centre must come to a wire to splice into it.
const SPLICE_SNAP_PX: f32 = 24.0;
/// Wire midpoint handle radius, screen px.
const MIDPOINT_R: f32 = 5.0;
/// Slash-cut stroke, screen px.
const CUT_STROKE: f32 = 1.5;
// --- Auto-layout spacing, base units. Generous on purpose: a layered graph
// that touches itself is harder to read than one that needs a scroll.
const BASE_LAYOUT_COL_GAP: f32 = 48.0;
const BASE_LAYOUT_ROW_GAP: f32 = 24.0;

// --- Performance budget -------------------------------------------------
//
// The stated target is **60fps at 2,000 nodes / 5,000 edges**. Three things
// keep that honest, and every cap below is derived from it rather than picked:
//
//  * the LOD ladder governs detail *per node*, not node count;
//  * everything is culled against the viewport before it is routed, drawn or
//    registered for hit-testing — a node off-screen costs a rect intersection;
//  * the two unbounded-by-nature features are capped outright.
//
// At the budget, a full-screen view holds on the order of a hundred nodes; the
// rest is culled. The crossing broadphase is the one pass whose cost scales
// with *visible segments* rather than visible nodes, so it gets an explicit
// ceiling: ~4 segments per visible wire across ~500 visible wires.
/// Hard ceiling on segments the (future) crossing broadphase may consider.
pub const CROSSING_SEGMENT_CAP: usize = 2_000;
/// Preview slots actually rendered per frame, round-robin by frame index, so
/// 200 opted-in nodes cost a fixed budget rather than 200 render targets.
pub const PREVIEW_BUDGET_PER_FRAME: usize = 8;
/// Preview slot side, base units.
const BASE_PREVIEW_SIDE: f32 = 64.0;

/// Which of `count` preview slots this frame is allowed to render.
///
/// Round-robin by frame index: every slot is refreshed within
/// `ceil(count / budget)` frames, and no frame pays for more than the budget.
/// Pure so the rotation is testable without a canvas.
pub fn preview_slice(count: usize, frame: u64, budget: usize) -> (usize, usize) {
    if count == 0 || budget == 0 {
        return (0, 0);
    }
    if count <= budget {
        return (0, count);
    }
    let windows = count.div_ceil(budget);
    let start = (frame as usize % windows) * budget;
    (start, budget.min(count - start))
}

/// Non-matching nodes dim to this while a find — or a variables filter — is
/// active. One constant, because it is one idiom.
const FIND_DIM: f32 = 0.45;
/// How long the locate flash takes to fade, ms.
const FLASH_MS: f32 = 900.0;
/// Exec wires that have not fired this session drop to this during a live
/// session (DESIGN-graphscripting - S3: orientation by dimming the rest, not
/// by tinting the hot path a new colour). Data wires never tint.
const UNFIRED_EXEC_ALPHA: f32 = 0.5;
/// Flow bubbles are hidden below this zoom: under it they are noise on a wire
/// too thin to follow.
const BUBBLE_MIN_ZOOM: f32 = 0.5;
/// Bubble radius, screen px, and how many ride one wire.
const BUBBLE_R: f32 = 3.0;
const BUBBLES_PER_WIRE: usize = 2;
/// Waiting-node progress bar height, screen px (3px, the design's number).
const WAIT_BAR_PX: f32 = 3.0;
/// Marquee fill alpha (1px accent border + 8% accent fill).
const MARQUEE_FILL_ALPHA: f32 = 0.08;

/// A reroute's pin hit zones: squares of `d * REROUTE_PIN_HIT` centred
/// `d * REROUTE_PIN_OFF` either side of the disc centre. They deliberately do
/// **not** tile the disc — the middle band, and the margins above and below
/// them, stay body, so the reroute can still be grabbed and dragged. Sharing
/// one centre with the global 18-unit target (the old behaviour) blanketed the
/// whole node: every press read as a pin press, so it could neither be moved
/// nor have its `out` side reached.
/// Arrow-key nudge distances, world units.
const NUDGE_COARSE: f32 = 16.0;
const NUDGE_FINE: f32 = 1.0;

const REROUTE_PIN_OFF: f32 = 0.30;
const REROUTE_PIN_HIT: f32 = 0.40;
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
    /// 35–45% — pin labels drop; the node title survives.
    L2,
    /// 45–60% — inline widgets fall back to plain values.
    L1,
    /// 60–220% — everything, widgets live.
    L0,
}

impl ZoomLod {
    pub fn from_zoom(zoom: f32) -> Self {
        // L0 starts where rows are comfortably readable — editing is allowed
        // wherever a field can be read (user report 2026-08-14: an
        // uneditable-looking field at 83% zoom reads as a broken editor,
        // not as a zoom level).
        if zoom >= 0.60 {
            Self::L0
        } else if zoom >= 0.45 {
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
    /// The config band exists (DESIGN-graphscripting ▸ Surface 2 ▸ low zoom).
    ///
    /// Deliberately **the same gate as the pin rows**, not a threshold of its
    /// own: the band drops exactly where the node becomes title-only, and the
    /// synthesized titles ("Get Health", "Event: Hit") already name the
    /// configuration, so nothing is lost on the way down to the slab. Its two
    /// steps above that are the ladder's existing ones — widgets flatten to
    /// plain mono values with the inline pin widgets (L1), and the row labels
    /// drop with the pin labels (L2).
    fn config_band(self) -> bool {
        self.rows()
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

/// One entry of the anchored-error UI. `GraphError` is a closed set by
/// design ruling, so a *domain compiler's* refusal (Task 41: the animation
/// compiler's `String`, already anchored editor-side) joins the index as its
/// own arm rather than a new variant — same badge, same count chip, same F8.
#[derive(Debug, Clone, PartialEq)]
enum IndexedError {
    Graph(GraphError),
    Domain(DomainError),
}

impl IndexedError {
    fn anchor(&self) -> ErrorAnchor {
        match self {
            IndexedError::Graph(e) => e.anchor(),
            IndexedError::Domain(e) => match e.node {
                Some(id) => ErrorAnchor::Node(id),
                None => ErrorAnchor::Document,
            },
        }
    }

    fn text(&self) -> String {
        match self {
            IndexedError::Graph(e) => format!("{e}"),
            IndexedError::Domain(e) => e.message.clone(),
        }
    }

    fn cycle_chain(&self) -> Option<&[String]> {
        match self {
            IndexedError::Graph(e) => e.cycle_chain(),
            IndexedError::Domain(_) => None,
        }
    }
}

/// This frame's errors, resolved to the thing each one is *about*. Built once
/// from `state.errors` + `state.ref_errors` + `state.domain_errors` and read
/// by both the geometry pass (ghost rows change a node's shape) and the paint
/// pass.
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
    document: Vec<IndexedError>,
    /// Every error, in a stable order, for the count chip's cycle.
    ordered: Vec<IndexedError>,
}

impl ErrorIndex {
    fn build(
        doc_errors: &[GraphError],
        ref_errors: &[GraphError],
        domain_errors: &[DomainError],
    ) -> Self {
        let mut ix = ErrorIndex::default();
        let all = doc_errors
            .iter()
            .chain(ref_errors.iter())
            .cloned()
            .map(IndexedError::Graph)
            .chain(domain_errors.iter().cloned().map(IndexedError::Domain));
        for e in all {
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
            ix.ordered.push(e);
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

    /// Per-node preview slot side, world units.
    fn preview_side(&self) -> f32 {
        BASE_PREVIEW_SIDE * self.scale
    }

    /// Width of a config row's value cell, world units.
    fn config_value_w(&self) -> f32 {
        BASE_CONFIG_VALUE_W * self.scale
    }

    /// Width an inline pin cell needs for `text` — the reserved [`value_w`] for
    /// anything that fits, grown to hold longer text up to a cap. Both the
    /// auto-sizer and the drawing pass go through this, which is what keeps a
    /// widget inside the node the sizer measured for it.
    ///
    /// [`value_w`]: Self::value_w
    fn text_value_w(&self, text_w: f32) -> f32 {
        (text_w + self.label_gap * 2.0).clamp(self.value_w, BASE_VALUE_W_MAX * self.scale)
    }

    /// Width of a payload row's ✕ column, world units.
    fn config_remove_w(&self) -> f32 {
        BASE_CONFIG_REMOVE_W * self.scale
    }

    /// A config row's inner box: the row's height minus its breathing space,
    /// spanning `x0..x1` and centred on `y`. One helper so the value cell, the
    /// ✕ target and the "+ field" ghost row cannot disagree about a row's
    /// vertical extent.
    fn config_box(&self, x0: f32, x1: f32, y: f32) -> Rect {
        let h = self.row_h * 0.8;
        Rect::from_min_max(Pos2::new(x0, y - h * 0.5), Pos2::new(x1, y + h * 0.5))
    }

    /// Center of body row `i`, counting the config band and the pin rows as
    /// one sequence. **The only place a row index becomes a y.** Config rows
    /// occupy `0..config_n`; pin row `i` is therefore `config_n + i`, which is
    /// what keeps the pins, their wire anchors, the band separator and the
    /// node's height from disagreeing by a row.
    fn band_y(&self, min_y: f32, index: usize) -> f32 {
        min_y + self.header_h + index as f32 * self.row_h + self.row_h * 0.5
    }

    /// Node height for a body of `config_n` config rows plus `rows` pin rows.
    fn node_h(&self, config_n: usize, rows: usize, preview_h: f32) -> f32 {
        self.header_h + (config_n + rows) as f32 * self.row_h + preview_h + self.body_pad
    }

    /// Diameter of a reroute's dot, world units.
    fn reroute_d(&self) -> f32 {
        self.pin_r * 3.2
    }

    /// An untyped (unwired) reroute is provisional and reads that way: a
    /// smaller neutral dot than a typed one, which carries its type's colour.
    fn reroute_untyped_d(&self) -> f32 {
        self.pin_r * 2.6
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

/// The two document resolvers a node's pins can depend on, bundled so every
/// descriptor site in this file asks the same question the same way: subgraph
/// interfaces from other `.graph` documents, Timeline track pins from a
/// `.curve` (45-A D3/P8b). Passing one without the other is how a Timeline
/// ends up drawing pins the compiler does not agree with.
#[derive(Clone, Copy)]
pub struct DocResolvers<'a> {
    pub graphs: &'a dyn GraphResolver,
    pub curves: &'a dyn CurveResolver,
}

impl<'a> DocResolvers<'a> {
    /// Bind both to one document.
    pub fn bind<'b>(&self, doc: &'b GraphDoc, registry: &'b NodeRegistry) -> DocDescriptors<'b>
    where
        'a: 'b,
    {
        DocDescriptors::with_resolver(doc, registry, self.graphs).with_curves(self.curves)
    }
}

/// Everything the panel needs, bundled so the signature stays small.
pub struct GraphEditorPanelCtx<'a> {
    pub state: &'a mut GraphEditorState,
    pub registry: &'a NodeRegistry,
    pub clipboard: &'a mut Option<GraphFragment>,
    /// Resolves subgraph references (open docs + disk) for pin derivation.
    pub resolver: &'a dyn GraphResolver,
    /// Resolves `.curve` references the same way, so a Timeline node grows one
    /// Float output per track (45-A P8b). Without it a Timeline draws base
    /// pins only and its track wires have nowhere to land.
    pub curves: &'a dyn CurveResolver,
    /// Content-relative paths of known `.subgraph` assets (create menu).
    pub subgraph_assets: &'a [String],
    /// Set when a node that references another graph file is descended into
    /// (a subgraph node, or an animation state nesting a `.animgraph` —
    /// ticket 09); the host opens it as a tab (P6 open-in-tab navigation)
    /// and seeds the opened tab's breadcrumb chain from the request.
    pub open_subgraph: &'a mut Option<GraphOpenRequest>,
    /// `selection.outline` from the live theme. Passed in because crusty's
    /// `Style` has no counterpart and Graphite overrides the invariant.
    pub selection_outline: Color,
    /// Wire routing + appearance, the `graph.wires` prefs section.
    pub wire_prefs: WirePrefs,
    /// Set when the toolbar's segmented control picks a different wire style;
    /// the host writes it back to `prefs.graph.wires.style`, which is what
    /// makes it show the overridden dot in Preferences and autosave.
    pub wire_style_request: &'a mut Option<WireStyle>,
    /// Keyboard bindings. Every shortcut below resolves through this rather
    /// than matching a literal chord.
    pub keymap: &'a Keymap,
    /// Canvas zoom limits (EditorPrefs, P9).
    pub zoom_min: f32,
    pub zoom_max: f32,
    /// This tab is the focused tab of its dock (gates keyboard editing).
    pub focused: bool,
    /// True in float windows (no menu/winit edit path) — the panel handles
    /// keyboard editing itself. False when docked in the main window.
    pub handle_shortcuts: bool,
    /// Live execution, when a running instance of *this* document is bound
    /// (45-A P7). `None` — edit mode, nothing selected, a selection that runs
    /// a different graph, a build without the interpreter — costs nothing and
    /// draws nothing: every viz site below is inside an `if let`.
    pub exec: Option<&'a GraphExecViz>,
    /// Every instance running this document, for the LIVE chip's picker
    /// (GS-3). Present even when nothing is bound — that is exactly the
    /// "N RUNNING — select instance" state.
    pub exec_instances: &'a [ExecInstance],
    /// Set to this document's path when the toolbar's "Clear trace" was
    /// pressed; the host clears the recorders of the instances running it,
    /// which are the only things that own one.
    pub exec_clear: &'a mut Option<String>,
    /// The bound preview instance for an `.animgraph` tab (Task 41 ticket
    /// 06): current parameter values, active state, in-flight fade. `None`
    /// — a script tab, or nothing bound — draws no strip controls and no
    /// live highlight.
    pub anim: Option<&'a AnimPreview>,
    /// Every entity this `.animgraph` could preview on, for the PREVIEW
    /// chip's picker. Net rigs whose parameters gameplay owns are already
    /// excluded by the host.
    pub anim_instances: &'a [ExecInstance],
}

// ---------------------------------------------------------------------------
// Geometry
// ---------------------------------------------------------------------------

/// What an unconnected input renders in its value cell.
#[derive(Clone, Debug, PartialEq)]
enum InlineKind {
    /// Editable at L0 (`DragValue`).
    Float(f32),
    /// Editable at L0 (`DragValue` stepping whole numbers). Separate from
    /// `Float` because the *widget* differs — no decimals, and a scrub speed
    /// tuned so one unit is a deliberate movement rather than a twitch.
    Int(i32),
    /// Editable at L0 (`Checkbox`).
    Bool(bool),
    /// Editable at L0 (`TextEdit`). Focus is what keeps canvas shortcuts out
    /// of the field: `overlay_has_focus` already returns true whenever any
    /// crusty text field holds focus, so `Del` deletes a character rather
    /// than the selected nodes.
    Str(String),
    /// Painted swatch + hex; not yet editable.
    Color([f32; 4]),
    /// An `Enum` pin whose descriptor declares its legal values — a real
    /// dropdown at L0. `ok` is false when the stored value is not one of
    /// them: shown in `status.warning` rather than reported as an error,
    /// because the `GraphError` set is closed (recorded ruling) and stale
    /// enum data is something to fix, not a broken document.
    Enum { value: String, variants: Vec<String>, ok: bool },
    /// A dropdown over a list that is **not** an `Enum` on disk: the `var`
    /// config row picks a slug and stores it as [`PropValue::Str`], because
    /// that is what `DocDescriptors::variable_of` reads. Same widget as
    /// `Enum`, different write-back — which is exactly why it is a separate
    /// variant rather than a flag: the storage shape is not a rendering
    /// detail. `ok` is false for a dangling reference, which still *displays*
    /// (the author has to see the name that broke).
    Choice { value: String, variants: Vec<String>, ok: bool },
    /// Painted mono chip (asset path, free-string enum, vector tuple).
    Chip(String),
    /// Forward-compat data: warning-dashed `preserved` chip, never a blank.
    Raw(String),
}

impl InlineKind {
    /// `variants` comes from the pin's descriptor; empty means "free string",
    /// which stays a plain chip rather than a dropdown over nothing.
    fn of(v: &PropValue, variants: &[String]) -> Self {
        match v {
            PropValue::Float(x) => InlineKind::Float(*x),
            PropValue::Int(i) => InlineKind::Int(*i),
            PropValue::Bool(b) => InlineKind::Bool(*b),
            PropValue::Str(t) => InlineKind::Str(t.clone()),
            PropValue::Color(c) => InlineKind::Color(*c),
            PropValue::Raw(s) => InlineKind::Raw(s.clone()),
            PropValue::Enum(e) if !variants.is_empty() => InlineKind::Enum {
                value: e.clone(),
                variants: variants.to_vec(),
                ok: variants.iter().any(|v| v == e),
            },
            other => InlineKind::Chip(prop_display(other)),
        }
    }
}

/// A **config row**: one reserved property that *shapes* a node rather than
/// feeding it (45-A P6c ruling).
///
/// `var`, `event_name`, `action` and `payload.<slug>` are deliberately not
/// pins — they decide what the node's pins *are*, so a pin could not carry
/// them without a chicken-and-egg problem. They render as pin-less rows in a
/// band **above** the pin rows, with a secondary-text label and the same
/// inline widget vocabulary, and they write through the same coalesced
/// `SetProperty` path (`begin_prop_edit` / `flush_prop_edit`) as any inline
/// pin constant — so undo does not care which kind of row you edited.
struct ConfigGeom {
    /// Property key, which is also the widget id and the `SetProperty` key.
    key: String,
    label: String,
    kind: InlineKind,
    /// Row center, world space.
    y: f32,
    /// The value cell, world space.
    cell: Rect,
    /// The label's own box, world space — a payload slug is clickable (rename),
    /// a fixed label is not.
    label_box: Rect,
    /// The ✕ target, world space. `Some` only on payload rows: `Variable`,
    /// `Name` and `Action` are the node's shape, not a list you edit.
    remove: Option<Rect>,
}

impl ConfigGeom {
    /// The payload slug this row declares, if it is a payload row.
    fn payload_slug(&self) -> Option<&str> {
        self.key.strip_prefix(EVENT_PAYLOAD_PREFIX)
    }
    /// Payload slugs are identifiers and render mono; the fixed labels
    /// ("Variable", "Name", "Action") name a setting and stay sentence-case
    /// sans — the same mono-means-technical rule as everywhere else.
    fn mono_label(&self) -> bool {
        self.payload_slug().is_some()
    }
}

/// The reserved config rows a node instance shows, in render order.
///
/// Sourced from the type's `NodeKind` plus the reserved-key constants, so
/// adding a doc-dependent type is one arm here rather than a new special case
/// in the drawing code. Rows appear even when the property is missing — a
/// freshly placed `event_custom` has no `event_name` yet, and a row is the
/// only way to give it one.
fn config_rows(n: &NodeInst, docd: &DocDescriptors) -> Vec<(String, String, InlineKind)> {
    use crate::engine::animation::graph::plan::{
        ANIM_PLAY_ONCE_TYPE_ID, ANIM_STATE_TYPE_ID, ANIM_TRANSITION_TYPE_ID, CLIP_NAME_PROP,
        CLIP_PROP, DURATION_PROP, GRAPH_PROP, PRIORITY_PROP, SLOT_FADE_IN_PROP,
        SLOT_FADE_OUT_PROP, SLOT_TRIGGER_PROP, SPEED_PROP,
    };
    let text_of = |key: &str| match n.properties.get(key) {
        Some(PropValue::Str(s)) => s.clone(),
        Some(PropValue::Enum(s)) => s.clone(),
        // A clip reference is an `Asset` when the editor wrote it and a `Str`
        // when a hand went in — the compiler reads both, so both display.
        Some(PropValue::Asset(s)) => s.clone(),
        _ => String::new(),
    };
    let float_of = |key: &str, default: f32| match n.properties.get(key) {
        Some(PropValue::Float(f)) => *f,
        _ => default,
    };
    let int_of = |key: &str| match n.properties.get(key) {
        Some(PropValue::Int(i)) => *i,
        _ => 0,
    };
    let mut out = Vec::new();
    match NodeKind::of_type(&n.type_id) {
        NodeKind::VarGet | NodeKind::VarSet => {
            let value = text_of(VAR_PROP);
            let variants: Vec<String> =
                docd.doc().variables.iter().map(|v| v.slug.clone()).collect();
            let ok = variants.contains(&value);
            out.push((
                VAR_PROP.to_string(),
                "Variable".to_string(),
                InlineKind::Choice { value, variants, ok },
            ));
        }
        NodeKind::EventCustom => {
            out.push((
                EVENT_NAME_PROP.to_string(),
                "Name".to_string(),
                InlineKind::Str(text_of(EVENT_NAME_PROP)),
            ));
            // One row per declared payload pin, in slug order (`properties` is
            // a BTreeMap, which is also the order the pins come out in).
            let variants: Vec<String> =
                PAYLOAD_PIN_TYPES.iter().map(|s| s.to_string()).collect();
            for key in n.properties.keys() {
                let Some(slug) = key.strip_prefix(EVENT_PAYLOAD_PREFIX) else {
                    continue;
                };
                let value = text_of(key);
                let ok = variants.contains(&value);
                out.push((
                    key.clone(),
                    slug.to_string(),
                    InlineKind::Enum { value, variants: variants.clone(), ok },
                ));
            }
        }
        _ if n.type_id == EVENT_INPUT_ACTION_TYPE_ID => {
            out.push((
                EVENT_ACTION_PROP.to_string(),
                "Action".to_string(),
                InlineKind::Str(text_of(EVENT_ACTION_PROP)),
            ));
        }
        // Task 41 — the animation library's node data. Config rows, not pins:
        // a clip path or a speed is node configuration, and a pin dot would
        // promise a wire that no output type can ever legally feed.
        _ if n.type_id == ANIM_STATE_TYPE_ID => {
            if docd
                .doc()
                .regions
                .get(&n.id)
                .is_some_and(|r| !r.nodes.is_empty())
            {
                // A state with a blend tree ignores its clip and graph (the
                // compiler's rule); the row says what the state *is* instead
                // of showing fields that would do nothing.
                out.push((
                    CLIP_PROP.to_string(),
                    "Tree".to_string(),
                    InlineKind::Chip("blend tree".to_string()),
                ));
            } else if !text_of(GRAPH_PROP).trim().is_empty() {
                // A nested sub-state-machine (ticket 09): the referenced
                // `.animgraph` is the state's whole Pose source, so the clip
                // rows yield the same way they do to a tree.
                out.push((
                    GRAPH_PROP.to_string(),
                    "Graph".to_string(),
                    InlineKind::Str(text_of(GRAPH_PROP)),
                ));
            } else {
                out.push((
                    CLIP_PROP.to_string(),
                    "Clip".to_string(),
                    InlineKind::Str(text_of(CLIP_PROP)),
                ));
                // The in-container clip name only rows when the document
                // carries one — data is never hidden, and the common
                // one-clip-per-file case does not pay a row for it.
                if !text_of(CLIP_NAME_PROP).is_empty() {
                    out.push((
                        CLIP_NAME_PROP.to_string(),
                        "Clip Name".to_string(),
                        InlineKind::Str(text_of(CLIP_NAME_PROP)),
                    ));
                }
                // The nested-graph reference's front door: an empty row on
                // every leaf state (rows appear even when the property is
                // missing — the module rule above — because a row is the only
                // way to give it a value).
                out.push((
                    GRAPH_PROP.to_string(),
                    "Graph".to_string(),
                    InlineKind::Str(String::new()),
                ));
            }
            out.push((
                SPEED_PROP.to_string(),
                "Speed".to_string(),
                InlineKind::Float(float_of(SPEED_PROP, 1.0)),
            ));
        }
        _ if n.type_id == ANIM_TRANSITION_TYPE_ID => {
            out.push((
                DURATION_PROP.to_string(),
                "Duration".to_string(),
                InlineKind::Float(float_of(DURATION_PROP, 0.0)),
            ));
            out.push((
                PRIORITY_PROP.to_string(),
                "Priority".to_string(),
                InlineKind::Int(int_of(PRIORITY_PROP)),
            ));
        }
        _ if n.type_id == ANIM_PLAY_ONCE_TYPE_ID => {
            out.push((
                CLIP_PROP.to_string(),
                "Clip".to_string(),
                InlineKind::Str(text_of(CLIP_PROP)),
            ));
            // The starting Trigger: a dropdown over the declared Trigger
            // parameters, stored as `Str` — the `var` config row's shape.
            let value = text_of(SLOT_TRIGGER_PROP);
            let variants: Vec<String> = docd
                .doc()
                .variables
                .iter()
                .filter(|v| v.ty == crate::engine::animation::graph::trigger_pin_type())
                .map(|v| v.slug.clone())
                .collect();
            let ok = variants.contains(&value);
            out.push((
                SLOT_TRIGGER_PROP.to_string(),
                "Trigger".to_string(),
                InlineKind::Choice { value, variants, ok },
            ));
            out.push((
                SPEED_PROP.to_string(),
                "Speed".to_string(),
                InlineKind::Float(float_of(SPEED_PROP, 1.0)),
            ));
            out.push((
                SLOT_FADE_IN_PROP.to_string(),
                "Fade In".to_string(),
                InlineKind::Float(float_of(SLOT_FADE_IN_PROP, 0.0)),
            ));
            out.push((
                SLOT_FADE_OUT_PROP.to_string(),
                "Fade Out".to_string(),
                InlineKind::Float(float_of(SLOT_FADE_OUT_PROP, 0.0)),
            ));
        }
        _ => {}
    }
    out
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
    /// Width the inline cell was *sized for*, world units. Carried on the pin
    /// rather than recomputed at paint time so the widget cannot end up wider
    /// than the node the auto-sizer measured around it.
    value_w: f32,
    /// A dashed placeholder for a pin the descriptor no longer declares, so
    /// the edge pointing at it has somewhere to land instead of vanishing.
    ghost: bool,
    /// Hit-target side length in world units, overriding the global
    /// `pin_hit_w`. A reroute's two pins share a disc smaller than the 18-unit
    /// global target, which swallowed the whole node — body included, which is
    /// why an unwired reroute could not be dragged.
    hit_w: Option<f32>,
    /// This pin's type is not known yet (an unwired reroute), so it is a
    /// wildcard: it accepts any type and adopts the first one wired to it.
    /// Distinct from `PinType::Domain("")`, which merely *encodes* that state
    /// and compares equal only to itself, refusing every real type.
    untyped: bool,
    /// Never drawn (Task 41 rework): machine nodes show no pin dots — their
    /// flow wires land on the border. The pin still exists as a wire anchor
    /// and (unless its `hit_w` is zero) a hit target.
    hidden: bool,
}

/// The at-rest presentation of a transition node (Task 41): an edge chip —
/// "Speed > 3.0 · 0.20s" with a filled/hollow Bool socket dot — instead of
/// the standard node anatomy. The node is storage; the chip is the
/// presentation (mockup 2d). Selecting the transition unfolds it into a
/// small standard card whose config rows edit duration and priority.
struct ChipGeom {
    /// The summary line, from `graph_anim_chip`.
    text: String,
    /// Filled dot = a wired rule; hollow = always-true.
    wired: bool,
}

/// Which compact machine card a node draws as (Task 41 canvas rework,
/// mockup 2b/2d): states are name + tag + one mono subtitle, ENTRY and ANY
/// STATE are small pills. None of them show pins or fields at rest —
/// selecting a state swaps to the standard card (config rows) through
/// geometry, exactly like a transition's chip does.
#[derive(Clone, Copy, PartialEq)]
enum AnimCardKind {
    State,
    Entry,
    Any,
}

/// A compact machine card's extras beyond the shared title/tag.
struct AnimCard {
    kind: AnimCardKind,
    /// The one mono line under the name: `▷ clip`, `❐ file.animgraph` or
    /// `⧉ blend tree`. `None` — a state with nothing configured — draws
    /// name-only.
    subtitle: Option<String>,
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
    /// An at-rest transition chip (Task 41): drawn instead of node anatomy.
    chip: Option<ChipGeom>,
    /// A compact machine card (Task 41 rework): drawn instead of node
    /// anatomy for states / ENTRY / ANY STATE on the machine canvas.
    anim: Option<AnimCard>,
    /// This node's displayed position is derived (a chip riding its edge):
    /// its stored position is not what is on screen, so a direct grab must
    /// not start a move that would land somewhere else entirely.
    pinned_pos: bool,
    /// The breakpoint mark, if any: `Some(true)` armed, `Some(false)`
    /// disabled (GS-4). Whether it is *hit* or *invalid* is not geometry — it
    /// comes from the bound instance at draw time.
    breakpoint: Option<bool>,
    /// Opt-in preview slot; `None` — the common case — costs nothing.
    preview: Option<crate::engine::node_graph::PreviewKind>,
    /// Pin-less reserved-property rows, drawn above the pin band.
    config: Vec<ConfigGeom>,
    /// The dashed "+ field" ghost row closing the band, world space. Custom
    /// events only — it is the only node whose pins the author declares.
    add_field: Option<Rect>,
    pins: Vec<PinGeom>,
}

impl NodeGeom {
    /// Move every piece of this geometry by `d` — the derived-position pass
    /// (a transition riding its edge midpoint) after sizes are known.
    fn translate(&mut self, d: Vec2) {
        let mv = |r: &mut Rect| *r = Rect::from_min_max(r.min + d, r.max + d);
        mv(&mut self.rect);
        for pin in &mut self.pins {
            pin.wire_anchor += d;
            pin.dot_center += d;
        }
        for c in &mut self.config {
            c.y += d.y;
            mv(&mut c.cell);
            mv(&mut c.label_box);
            if let Some(r) = &mut c.remove {
                mv(r);
            }
        }
        if let Some(r) = &mut self.add_field {
            mv(r);
        }
        self.pinned_pos = true;
    }

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

    /// How many band rows this node has, the "+ field" ghost row included —
    /// the number the pin band is shifted down by.
    fn band_rows(&self) -> usize {
        self.config.len() + usize::from(self.add_field.is_some())
    }

    /// The band's closing rule / the top of the first pin row, world y.
    /// `None` when the node has no band at all.
    fn band_bottom(&self, m: &GraphMetrics) -> Option<f32> {
        let n = self.band_rows();
        (n > 0).then(|| m.band_y(self.rect.min.y, n - 1) + m.row_h * 0.5)
    }

    /// The box actually drawn (and hit-tested) at this detail level: below
    /// L2 a node collapses to its header, so rows never render as mush.
    fn body_rect(&self, lod: ZoomLod, m: &GraphMetrics) -> Rect {
        // A reroute has no header to collapse to; it is drawn as the same disc
        // at every zoom. Falling through to the header-height branch gave it a
        // hit box nearly twice as tall as the dot, hanging below it. A chip is
        // already smaller than a header, so it keeps its own box the same way.
        if self.reroute || self.chip.is_some() || self.anim.is_some() || lod.rows() {
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

/// Which pins of which nodes have an edge on them, indexed once per frame.
///
/// The naive form — asking the edge vector per pin per node — is O(nodes x
/// edges) every frame whether or not anything is on screen, which is the
/// first thing to fall over on a large graph. One pass builds this instead.
#[derive(Default)]
struct IncidentEdges<'a> {
    incoming: BTreeMap<u64, BTreeSet<&'a str>>,
    outgoing: BTreeMap<u64, BTreeSet<&'a str>>,
}

impl<'a> IncidentEdges<'a> {
    fn build(edges: &'a [Edge]) -> Self {
        let mut ix = Self::default();
        for e in edges {
            ix.incoming
                .entry(e.to_node)
                .or_default()
                .insert(e.to_pin.as_str());
            ix.outgoing
                .entry(e.from_node)
                .or_default()
                .insert(e.from_pin.as_str());
        }
        ix
    }
}

#[allow(clippy::too_many_arguments)]
fn build_geoms(
    state: &GraphEditorState,
    registry: &NodeRegistry,
    resolver: &DocResolvers<'_>,
    errors: &ErrorIndex,
    m: &GraphMetrics,
    st: &Style,
    p: &mut Painter,
) -> Vec<NodeGeom> {
    let title_px = st.fonts.body;
    let label_px = st.fonts.small;
    let incident = IncidentEdges::build(&state.doc.edges);
    // One resolver for the whole pass: "what are this instance's pins" is a
    // document question, not a registry lookup (45-A D3). Subgraph
    // interfaces, variable and interface-binding synthesis and custom-event
    // payloads all arrive through it; the reroute branch below is the one
    // shape it cannot answer with a descriptor, by design.
    let docd = resolver.bind(&state.doc, registry);

    state
        .doc
        .nodes
        .iter()
        .map(|n| -> NodeGeom {
            let min = Pos2::new(n.position[0], n.position[1]);
            let is_sub = n.type_id == SUBGRAPH_TYPE_ID;
            let is_reroute = n.type_id == REROUTE_TYPE_ID;
            let desc = docd.descriptor(n.id);
            let desc = desc.as_deref();

            // A reroute is a bare pass-through: one in, one out, no header,
            // no rows, and a type inferred from whatever feeds it.
            if is_reroute {
                let inferred = docd.reroute_type(n.id);
                let untyped = inferred.is_none();
                let d = if untyped {
                    m.reroute_untyped_d()
                } else {
                    m.reroute_d()
                };
                let rect = Rect::from_min_size(min, Vec2::splat(d));
                let ty = inferred.unwrap_or(PinType::Domain(String::new()));
                let c = Pos2::new(min.x + d * 0.5, min.y + d * 0.5);
                let band = d * REROUTE_PIN_OFF;
                let pin = |slug: &str, output: bool, x: f32| PinGeom {
                    slug: slug.to_string(),
                    label: String::new(),
                    ty: ty.clone(),
                    output,
                    row: 0,
                    wire_anchor: Pos2::new(x, c.y),
                    dot_center: Pos2::new(
                        if output { c.x + band } else { c.x - band },
                        c.y,
                    ),
                    connected: state.doc.edges.iter().any(|e| {
                        if output {
                            e.from_node == n.id && e.from_pin == slug
                        } else {
                            e.to_node == n.id && e.to_pin == slug
                        }
                    }),
                    inline: None,
                    value_w: m.value_w,
                    ghost: false,
                    hit_w: Some(d * REROUTE_PIN_HIT),
                    untyped,
                    hidden: false,
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
                    chip: None,
                    anim: None,
                    pinned_pos: false,
                    breakpoint: state.breakpoints.get(&n.id).copied(),
                    preview: None,
                    config: Vec::new(),
                    add_field: None,
                    pins: vec![
                        pin(REROUTE_IN, false, min.x),
                        pin(REROUTE_OUT, true, min.x + d),
                    ],
                };
            }

            // Task 41: an unselected transition renders as its at-rest chip —
            // rule summary · duration · priority tag, sized around that one
            // mono line. Selecting it unfolds the standard card (with its
            // Duration/Priority config rows) through the normal path below.
            if n.type_id == crate::engine::animation::graph::plan::ANIM_TRANSITION_TYPE_ID
                && !state.selection.contains(&n.id)
            {
                use crate::engine::animation::graph::plan::{
                    TRANSITION_FROM_PIN, TRANSITION_TO_PIN,
                };
                let resolved = super::graph_anim_chip::transition_chip(&state.doc, n.id);
                let text = resolved.text();
                let tw = p
                    .measure_text_family(&text, label_px, None, FontFamily::Mono)
                    .x;
                let dot_d = m.pin_r * 2.0;
                let w = m.pad_x + dot_d + m.label_gap + tw + m.pad_x;
                let h = m.row_h;
                let rect = Rect::from_min_size(min, Vec2::new(w, h));
                let cy = min.y + h * 0.5;
                let flow = PinType::Domain(
                    crate::engine::animation::graph::ANIM_FLOW_DOMAIN.to_string(),
                );
                let empty: BTreeSet<&str> = BTreeSet::new();
                let incoming = incident.incoming.get(&n.id).unwrap_or(&empty);
                let outgoing = incident.outgoing.get(&n.id).unwrap_or(&empty);
                let pin = |slug: &str, output: bool, x: f32, dot: f32| PinGeom {
                    slug: slug.to_string(),
                    label: String::new(),
                    ty: flow.clone(),
                    output,
                    row: 0,
                    wire_anchor: Pos2::new(x, cy),
                    dot_center: Pos2::new(dot, cy),
                    connected: if output {
                        outgoing.contains(slug)
                    } else {
                        incoming.contains(slug)
                    },
                    inline: None,
                    value_w: m.value_w,
                    ghost: false,
                    hit_w: None,
                    untyped: false,
                    hidden: true,
                };
                return NodeGeom {
                    id: n.id,
                    rect,
                    title: docd
                        .display_name(n.id)
                        .unwrap_or_else(|| "Transition".to_string()),
                    tag: String::new(),
                    category: Some(crate::engine::animation::graph::ANIM_CATEGORY.to_string()),
                    tint: n.tint,
                    missing: false,
                    errored: errors.nodes.contains(&n.id),
                    reroute: false,
                    chip: Some(ChipGeom { text, wired: resolved.wired }),
                    anim: None,
                    pinned_pos: false,
                    breakpoint: state.breakpoints.get(&n.id).copied(),
                    preview: None,
                    config: Vec::new(),
                    add_field: None,
                    pins: vec![
                        pin(TRANSITION_FROM_PIN, false, min.x, min.x + m.pin_inset),
                        pin(TRANSITION_TO_PIN, true, min.x + w, min.x + w - m.pin_inset),
                    ],
                };
            }

            // Task 41 canvas rework: the other machine nodes render as
            // compact cards — a state is its name, role tag and one mono
            // subtitle; ENTRY and ANY STATE are small pills (mockup 2b).
            // No pins, no fields. Selecting a *state* unfolds the standard
            // card (its Clip/Graph/Speed config rows) through the generic
            // path below — the transition-chip idiom exactly.
            if state.domain.is_animation() {
                use crate::engine::animation::graph::plan::{
                    ANIM_ANY_STATE_TYPE_ID, ANIM_ENTRY_TYPE_ID, ANIM_STATE_TYPE_ID,
                };
                let kind = match n.type_id.as_str() {
                    ANIM_STATE_TYPE_ID if !state.selection.contains(&n.id) => {
                        Some(AnimCardKind::State)
                    }
                    ANIM_ENTRY_TYPE_ID => Some(AnimCardKind::Entry),
                    ANIM_ANY_STATE_TYPE_ID => Some(AnimCardKind::Any),
                    _ => None,
                };
                if let Some(kind) = kind {
                    return anim_card_geom(n, kind, state, &docd, &incident, errors, m, st, p);
                }
            }
            #[allow(clippy::type_complexity)]
            let (title, category, missing, inputs, outputs): (
                String,
                Option<String>,
                bool,
                Vec<(String, String, PinType)>,
                Vec<(String, String, PinType)>,
            ) = if is_sub && desc.is_none() {
                // An unresolvable reference renders in the missing-node style
                // but keeps the name the path implies — the author still has
                // to recognize which subgraph went missing.
                let name = n
                    .subgraph
                    .as_deref()
                    .and_then(|q| std::path::Path::new(q).file_stem())
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| "Subgraph".to_string());
                (name, Some("Subgraph".to_string()), true, vec![], vec![])
            } else if let Some(d) = desc {
                // The three clones per pin are the one unavoidable copy: a
                // `PinGeom` outlives the registry borrow. Everything else in
                // this pass borrows.
                let pins = |v: &[crate::engine::node_graph::PinDescriptor]| {
                    v.iter()
                        .map(|q| (q.slug.clone(), q.label.clone(), q.ty.clone()))
                        .collect::<Vec<_>>()
                };
                (
                    // Doc-dependent types name themselves from their config
                    // ("Get Score", "Event: Hit"), and an explicit
                    // `NodeInst::title` beats both — see
                    // `DocDescriptors::display_name`.
                    docd.display_name(n.id).unwrap_or_else(|| d.name.clone()),
                    Some(d.category.clone()),
                    false,
                    pins(&d.inputs),
                    pins(&d.outputs),
                )
            } else {
                // Nothing resolved — an unknown type, or a variable node whose
                // declaration was deleted. The *name* is still knowable in the
                // second case (`Get health`), and `DocDescriptors::display_name`
                // exists precisely so a broken node does not render as the bare
                // slug `var_get`, which tells the author nothing about what
                // broke. Falls back to the type id when even that is unknown.
                (
                    docd.display_name(n.id).unwrap_or_else(|| n.type_id.clone()),
                    None,
                    true,
                    vec![],
                    vec![],
                )
            };

            // Animation nodes wear the mockup's role words (STATE / ENTRY /
            // ANY / SLOT) instead of the derived PURE/EVENT tags, which
            // describe exec flow and mean nothing in a domain without any.
            let tag = crate::engine::animation::graph::anim_node_tag(&n.type_id)
                .map(str::to_string)
                .unwrap_or_else(|| derive_tag(is_sub, desc, category.as_deref()));

            // Task 41 rework: machine nodes keep pins off the card even
            // unfolded — flow wires land on the border. A selected state
            // strips its pin rows before sizing (hidden border anchors are
            // pushed after the pins are built); a selected transition keeps
            // its two pins as hit targets (retargeting) but never draws them.
            let anim_state_unfold = state.domain.is_animation()
                && n.type_id == crate::engine::animation::graph::plan::ANIM_STATE_TYPE_ID;
            let anim_transition = state.domain.is_animation()
                && n.type_id
                    == crate::engine::animation::graph::plan::ANIM_TRANSITION_TYPE_ID;
            let (inputs, outputs) = if anim_state_unfold {
                (Vec::new(), Vec::new())
            } else {
                (inputs, outputs)
            };

            // --- auto width: widest of the header row and every pin row ---
            let mut tag_w = p
                .measure_text_family(&tag, m.tag_px, None, FontFamily::Mono)
                .x;
            // A node carrying a breakpoint reserves its debug header now
            // (GS-4): the badge gutter, and enough of the tag slot for the
            // PAUSED chip that replaces the category while execution is parked
            // on it. Sizing for it up front is what stops a hit from eliding
            // the node's own title at the moment you most need to read it —
            // and it costs only the marked nodes a few pixels, which is
            // exactly what the mockup's wider hit card does.
            let mut gutter_w = 0.0;
            if state.has_breakpoint(n.id) {
                tag_w = tag_w.max(
                    p.measure_text_family("PAUSED", m.tag_px, None, FontFamily::Mono).x,
                );
                gutter_w = m.pin_r * BREAK_BADGE_R * 2.0 + m.label_gap;
            }
            let header_w = m.pad_x
                + gutter_w
                + p.measure_text(&title, title_px, None).x
                + m.col_gap
                + tag_w
                + m.pad_x;

            // Looked up, not scanned: the index below is built in one pass
            // over the edge vector for the whole frame, so this is O(1) per
            // node instead of O(edges) — the difference between linear and
            // quadratic at the stated 2,000-node / 5,000-edge budget.
            let empty: BTreeSet<&str> = BTreeSet::new();
            let incoming = incident.incoming.get(&n.id).unwrap_or(&empty);
            let outgoing = incident.outgoing.get(&n.id).unwrap_or(&empty);
            let inline_of = |slug: &str| -> Option<InlineKind> {
                if incoming.contains(slug) {
                    return None;
                }
                let variants = desc
                    .and_then(|d| d.input(slug))
                    .map(|p| p.variants.as_slice())
                    .unwrap_or(&[]);
                n.properties.get(slug).map(|v| InlineKind::of(v, variants))
            };

            // Ghost rows for pins an edge names but the descriptor no longer
            // declares. They append after the real pins on their side, so an
            // otherwise-orphaned wire has somewhere to land.
            let ghosts = errors.ghosts_for(n.id);
            let ghost_in = ghosts.iter().filter(|(_, o)| !*o).count();
            let ghost_out = ghosts.iter().filter(|(_, o)| *o).count();

            // Config rows sit in a band **above** the pins, so they shift the
            // entire pin band down by `config.len()` rows. Every site that
            // derives a row position from an index goes through `row_y`, and
            // `row_y` is the only place the offset is applied — that is the
            // whole defence against a one-row error putting wires beside
            // their pins. (`PinGeom::row` is deliberately *not* offset: it is
            // the router's bundle-stagger lane index — "which pin of this
            // column" — not a geometric row.)
            let config = config_rows(n, &docd);
            // The "+ field" ghost row is part of the band's geometry, not a
            // decoration on top of it: it occupies a row, so it shifts the pin
            // band exactly like a declared field does and the node grows by
            // one row rather than overlapping its first pin.
            let has_add_field = n.type_id == EVENT_CUSTOM_TYPE_ID;
            let config_n = config.len() + usize::from(has_add_field);

            // A pin-less node with a config band (the play-once slot) does
            // not pay for an empty pin row; everything else keeps at least
            // one row so a bare node still has a body.
            let rows = (inputs.len() + ghost_in)
                .max(outputs.len() + ghost_out)
                .max(usize::from(config_n == 0));
            let mut content_w: f32 = header_w;
            for (key, label, kind) in &config {
                let remove_w = if key.starts_with(EVENT_PAYLOAD_PREFIX) {
                    m.config_remove_w() + m.label_gap
                } else {
                    0.0
                };
                content_w = content_w.max(
                    m.pad_x
                        + p.measure_text_family(
                            label,
                            label_px,
                            None,
                            if remove_w > 0.0 { FontFamily::Mono } else { FontFamily::Ui },
                        )
                        .x
                        + m.label_gap
                        // Same content-aware floor the cell itself uses: a
                        // config dropdown or long value widens its node
                        // rather than eliding (readable-at-rest bar).
                        + m.config_value_w().max(inline_cell_w(p, kind, m, st))
                        + remove_w
                        + m.pad_x,
                );
            }
            for i in 0..rows.min(inputs.len().max(outputs.len()).max(1)) {
                let left = inputs
                    .get(i)
                    .map(|(slug, label, _)| {
                        let mut w = m.label_inset() + p.measure_text(label, label_px, None).x;
                        // The cell's own width, plus the right padding the
                        // drawing pass reserves for it. Budgeting `value_w`
                        // flat (and forgetting `pad_x`) is what let a String
                        // constant paint past the node's edge.
                        if let Some(kind) = inline_of(slug) {
                            w = inline_row_w(w, inline_cell_w(p, &kind, m, st), m);
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
            let title_avail = width - m.pad_x * 2.0 - gutter_w - tag_w - m.col_gap;
            let title = middle_truncate(p, &title, title_px, title_avail);

            let preview_h = desc
                .and_then(|d| d.preview)
                .map_or(0.0, |_| m.preview_side() + m.body_pad);
            let height = m.node_h(config_n, rows, preview_h);
            let rect = Rect::from_min_size(min, Vec2::new(width, height));
            // `i` is a *pin* row index; the config band is added here, once.
            let row_y = |i: usize| m.band_y(min.y, config_n + i);
            let config: Vec<ConfigGeom> = config
                .into_iter()
                .enumerate()
                .map(|(i, (key, label, kind))| {
                    let y = m.band_y(min.y, i);
                    let payload = key.starts_with(EVENT_PAYLOAD_PREFIX);
                    let right = min.x + width - m.pad_x;
                    let remove = payload.then(|| {
                        m.config_box(right - m.config_remove_w(), right, y)
                    });
                    let cell_right = match &remove {
                        Some(r) => r.min.x - m.label_gap,
                        None => right,
                    };
                    let cell_w = m.config_value_w().max(inline_cell_w(p, &kind, m, st));
                    let cell = m.config_box(cell_right - cell_w, cell_right, y);
                    let label_box =
                        m.config_box(min.x + m.pad_x, cell.min.x - m.label_gap, y);
                    ConfigGeom { key, label, kind, y, cell, label_box, remove }
                })
                .collect();
            // …and the ghost row closes the band, one row below the last field.
            let add_field = has_add_field.then(|| {
                m.config_box(
                    min.x + m.pad_x,
                    min.x + width - m.pad_x,
                    m.band_y(min.y, config.len()),
                )
            });

            let mut pins = Vec::new();
            let (in_count, out_count) = (inputs.len(), outputs.len());
            for (i, (slug, label, ty)) in inputs.into_iter().enumerate() {
                let y = row_y(i);
                let inline = inline_of(&slug);
                let value_w = inline
                    .as_ref()
                    .map_or(m.value_w, |k| inline_cell_w(p, k, m, st));
                pins.push(PinGeom {
                    connected: incoming.contains(slug.as_str()),
                    wire_anchor: Pos2::new(min.x, y),
                    dot_center: Pos2::new(min.x + m.pin_inset, y),
                    slug,
                    label,
                    ty,
                    output: false,
                    row: i,
                    inline,
                    value_w,
                    ghost: false,
                    hit_w: None,
                    untyped: false,
                    hidden: anim_transition,
                });
            }
            for (i, (slug, label, ty)) in outputs.into_iter().enumerate() {
                let y = row_y(i);
                pins.push(PinGeom {
                    connected: outgoing.contains(slug.as_str()),
                    wire_anchor: Pos2::new(min.x + width, y),
                    dot_center: Pos2::new(min.x + width - m.pin_inset, y),
                    slug,
                    label,
                    ty,
                    output: true,
                    row: i,
                    inline: None,
                    value_w: m.value_w,
                    ghost: false,
                    hit_w: None,
                    untyped: false,
                    hidden: anim_transition,
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
                    value_w: m.value_w,
                    ghost: true,
                    hit_w: None,
                    untyped: false,
                    hidden: false,
                });
                *row += 1;
            }
            // The unfolded state's hidden border anchors: mid-border, no
            // dot, no hit — the same anchors its compact card carries, so
            // its flow wires do not move when it folds or unfolds.
            if anim_state_unfold {
                use crate::engine::animation::graph::plan::{STATE_IN_PIN, STATE_OUT_PIN};
                let flow = PinType::Domain(
                    crate::engine::animation::graph::ANIM_FLOW_DOMAIN.to_string(),
                );
                let cy = min.y + height * 0.5;
                for (slug, output, x) in [
                    (STATE_IN_PIN, false, min.x),
                    (STATE_OUT_PIN, true, min.x + width),
                ] {
                    pins.push(PinGeom {
                        slug: slug.to_string(),
                        label: String::new(),
                        ty: flow.clone(),
                        output,
                        row: 0,
                        wire_anchor: Pos2::new(x, cy),
                        dot_center: Pos2::new(x, cy),
                        connected: if output {
                            outgoing.contains(slug)
                        } else {
                            incoming.contains(slug)
                        },
                        inline: None,
                        value_w: m.value_w,
                        ghost: false,
                        hit_w: Some(0.0),
                        untyped: false,
                        hidden: true,
                    });
                }
            }
            NodeGeom {
                id: n.id,
                rect,
                breakpoint: state.breakpoints.get(&n.id).copied(),
                title,
                tag,
                category,
                tint: n.tint,
                missing,
                errored: errors.nodes.contains(&n.id),
                reroute: is_reroute,
                chip: None,
                anim: None,
                pinned_pos: false,
                preview: desc.and_then(|d| d.preview),
                config,
                add_field,
                pins,
            }
        })
        .collect()
}

/// Geometry for a compact machine card (Task 41 canvas rework, mockup 2b):
/// a state is name + STATE tag + one mono subtitle; ENTRY and ANY STATE are
/// small pills. No pin rows exist — the flow anchors are hidden mid-border
/// pins with a zero hit target, because a border press starts a wire and a
/// flow wire lands on the border, not on a dot.
#[allow(clippy::too_many_arguments)]
fn anim_card_geom(
    n: &NodeInst,
    kind: AnimCardKind,
    state: &GraphEditorState,
    docd: &DocDescriptors,
    incident: &IncidentEdges,
    errors: &ErrorIndex,
    m: &GraphMetrics,
    st: &Style,
    p: &mut Painter,
) -> NodeGeom {
    use crate::engine::animation::graph::plan::{
        CLIP_PROP, GRAPH_PROP, STATE_IN_PIN, STATE_OUT_PIN,
    };
    let text_of = |key: &str| match n.properties.get(key) {
        Some(PropValue::Str(s)) | Some(PropValue::Enum(s)) | Some(PropValue::Asset(s)) => {
            s.clone()
        }
        _ => String::new(),
    };
    let min = Pos2::new(n.position[0], n.position[1]);
    let errored = errors.nodes.contains(&n.id);
    let (title, tag, subtitle) = match kind {
        AnimCardKind::Entry => ("\u{25b6} ENTRY".to_string(), String::new(), None),
        AnimCardKind::Any => ("ANY STATE".to_string(), String::new(), None),
        AnimCardKind::State => {
            let has_tree = docd
                .doc()
                .regions
                .get(&n.id)
                .is_some_and(|r| !r.nodes.is_empty());
            let graph = text_of(GRAPH_PROP);
            let clip = text_of(CLIP_PROP);
            // The compiler's precedence, spoken back: tree > graph > clip.
            let subtitle = if has_tree {
                Some("\u{29c9} blend tree".to_string())
            } else if !graph.trim().is_empty() {
                Some(format!("\u{2750} {}", graph.trim()))
            } else if !clip.trim().is_empty() {
                Some(format!("\u{25b7} {}", clip.trim()))
            } else {
                None
            };
            (
                docd.display_name(n.id).unwrap_or_else(|| "State".to_string()),
                "STATE".to_string(),
                subtitle,
            )
        }
    };
    let title_px = st.fonts.body;
    let sub_px = st.fonts.small;
    let gutter_w = if errored {
        m.pin_r * 1.6 + m.label_gap
    } else {
        0.0
    };
    let tag_w = if tag.is_empty() {
        0.0
    } else {
        p.measure_text_family(&tag, m.tag_px, None, FontFamily::Mono).x + m.col_gap
    };
    let title_w = p.measure_text(&title, title_px, None).x;
    let sub_w = subtitle
        .as_deref()
        .map_or(0.0, |s| p.measure_text_family(s, sub_px, None, FontFamily::Mono).x);
    let content = (gutter_w + title_w + tag_w).max(sub_w);
    // States take the mockup's min card width; pills size to their text.
    let w = match kind {
        AnimCardKind::State => (m.pad_x * 2.0 + content).clamp(m.min_w, m.max_w),
        _ => (m.pad_x * 2.0 + content).min(m.max_w),
    };
    let h = match (kind, &subtitle) {
        (AnimCardKind::State, Some(_)) => m.header_h + m.row_h * 0.7,
        (AnimCardKind::State, None) => m.header_h + m.body_pad,
        _ => m.header_h,
    };
    let rect = Rect::from_min_size(min, Vec2::new(w, h));
    let title = middle_truncate(p, &title, title_px, w - m.pad_x * 2.0 - gutter_w - tag_w);

    let cy = min.y + h * 0.5;
    let empty: BTreeSet<&str> = BTreeSet::new();
    let incoming = incident.incoming.get(&n.id).unwrap_or(&empty);
    let outgoing = incident.outgoing.get(&n.id).unwrap_or(&empty);
    let flow = PinType::Domain(crate::engine::animation::graph::ANIM_FLOW_DOMAIN.to_string());
    let anchor = |slug: &str, output: bool, x: f32| PinGeom {
        slug: slug.to_string(),
        label: String::new(),
        ty: flow.clone(),
        output,
        row: 0,
        wire_anchor: Pos2::new(x, cy),
        dot_center: Pos2::new(x, cy),
        connected: if output {
            outgoing.contains(slug)
        } else {
            incoming.contains(slug)
        },
        inline: None,
        value_w: m.value_w,
        ghost: false,
        hit_w: Some(0.0),
        untyped: false,
        hidden: true,
    };
    let mut pins = vec![anchor(STATE_OUT_PIN, true, min.x + w)];
    if kind == AnimCardKind::State {
        pins.push(anchor(STATE_IN_PIN, false, min.x));
    }
    NodeGeom {
        id: n.id,
        rect,
        title,
        tag,
        category: Some(crate::engine::animation::graph::ANIM_CATEGORY.to_string()),
        tint: n.tint,
        missing: false,
        errored,
        reroute: false,
        chip: None,
        anim: Some(AnimCard { kind, subtitle }),
        pinned_pos: false,
        breakpoint: state.breakpoints.get(&n.id).copied(),
        preview: None,
        config: Vec::new(),
        add_field: None,
        pins,
    }
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

/// A dashed rectangle — the "not a thing yet" outline the "+ field" ghost row
/// wears, and the same dash language as the ghost pin rows it sits above.
///
/// Dashes are derived from each side's own length (rather than from x extent
/// like [`dashed_line`], which is enough for a horizontal rule but collapses a
/// vertical one to a single stroke).
fn dashed_rect(p: &mut Painter, r: Rect, w: f32, color: Color) {
    let dash = (w * 4.0).max(3.0);
    let mut side = |a: Pos2, b: Pos2| {
        let len = ((b.x - a.x).powi(2) + (b.y - a.y).powi(2)).sqrt();
        let n = (len / dash).clamp(2.0, 96.0) as usize;
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
    };
    let (a, b) = (r.min, r.max);
    side(a, Pos2::new(b.x, a.y));
    side(Pos2::new(b.x, a.y), b);
    side(b, Pos2::new(a.x, b.y));
    side(Pos2::new(a.x, b.y), a);
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
/// equality covers both).
///
/// The input side must be currently unconnected — **unless it is an exec
/// pin**, which may fan in (45-A P3 ruling): an exec input is a continuation
/// target, so a second wire into it is a second place execution can arrive
/// from, not a second value. Data inputs keep the single-wire rule.
#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_arguments)]
fn validate_connection(
    state: &GraphEditorState,
    registry: &NodeRegistry,
    a_node: u64,
    a_slug: &str,
    a_out: bool,
    a_ty: &PinType,
    a_wild: bool,
    b_node: u64,
    b_slug: &str,
    b_out: bool,
    b_ty: &PinType,
    b_wild: bool,
) -> Option<Edge> {
    if a_node == b_node || a_out == b_out {
        return None;
    }
    // Typing stays strict — no implicit conversions — but an *untyped* reroute
    // is not a type mismatch, it is an absence of one. It accepts any pin and
    // adopts that type; `reroute_type` then infers it through the chain from
    // the first real descriptor pin upstream.
    if !a_wild && !b_wild && a_ty != b_ty {
        return None;
    }
    let (from_node, from_pin, to_node, to_pin) = if a_out {
        (a_node, a_slug, b_node, b_slug)
    } else {
        (b_node, b_slug, a_node, a_slug)
    };
    let to_ty = if a_out { b_ty } else { a_ty };
    let occupied = state
        .doc
        .edges
        .iter()
        .any(|e| e.to_node == to_node && e.to_pin == to_pin);
    // Exec and flow-like domain inputs may fan in (Task 41: several
    // transitions arriving at one state); a data input takes one wire.
    let fan_in = *to_ty == PinType::Exec
        || matches!(to_ty, PinType::Domain(k) if registry.domain_is_flow(k));
    if occupied && !fan_in {
        return None;
    }
    // An identical wire is not a second arrival, it is the same one.
    if state.doc.edges.iter().any(|e| {
        e.to_node == to_node && e.to_pin == to_pin && e.from_node == from_node
            && e.from_pin == from_pin
    }) {
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
        curves,
        subgraph_assets,
        open_subgraph,
        selection_outline,
        wire_prefs,
        wire_style_request,
        zoom_min,
        zoom_max,
        focused,
        handle_shortcuts,
        keymap,
        exec,
        exec_instances,
        exec_clear,
        anim,
        anim_instances,
    } = ctx;
    let resolver = &DocResolvers { graphs: resolver, curves };
    // Subgraph instances are a script-library concept; the animation
    // library's file-backed nesting is a state referencing another
    // `.animgraph` through its Graph row (ticket 09), not a subgraph row.
    let subgraph_assets: &[String] = if state.domain.is_animation() {
        &[]
    } else {
        subgraph_assets
    };

    // Finalize a gesture orphaned by a release that landed while this tab was
    // not being drawn (e.g. the user switched tabs mid-drag): the pointer is
    // already up on re-entry, so `draw_and_interact`'s in-body finish never
    // ran. Finalizing (vs reverting) keeps the edit the user made — the
    // simpler correct choice. No-op when nothing is in flight.
    //
    // This runs **before** the shortcut handler: undo, redo and save must
    // never act across a half-finished gesture, or they would either skip an
    // untracked mutation or mark a save cursor over content the file does not
    // have.
    // Rule-scope upkeep (ticket 05): settle anything the peek recorded since
    // last frame onto the parent history before shortcuts or drawing read
    // either document.
    state.drain_rule_scope(registry);
    if !ui.ctx().input.pointer_down {
        finish_node_drag(state, registry);
        finish_annotation_drag(state, registry);
        // The peek's canvas gets the same orphaned-gesture finish.
        if let Some(mut scope) = state.rule_scope.take() {
            finish_node_drag(&mut scope.child, rule_scope_registry());
            finish_annotation_drag(&mut scope.child, rule_scope_registry());
            state.rule_scope = Some(scope);
        }
        state.flush_prop_edit(registry);
        // The variables panel's defaults coalesce on the same rule as node
        // properties — one drag, one entry — so they close on the same beat.
        state.flush_var_default_edit(registry);
    }

    if handle_shortcuts && focused {
        handle_panel_keys(ui, state, registry, clipboard, keymap, exec);
    }

    // The graph toolbar sits above the canvas and takes its row out of the
    // available space, so the canvas shrinks by exactly the toolbar height and
    // everything measured off the canvas rect (F/A framing, the error overlay)
    // follows automatically.
    let mut clear_trace = false;
    let mut live_chip_rect: Option<(Rect, bool)> = None;
    graph_toolbar(
        ui,
        state,
        wire_prefs.style,
        wire_style_request,
        exec,
        exec_instances,
        &mut clear_trace,
        &mut live_chip_rect,
    );
    if clear_trace {
        *exec_clear = Some(state.path.clone());
    }
    // The PAUSED banner takes its row the same way the toolbar does, so the
    // canvas below shrinks by exactly its height and every overlay measured
    // off the canvas rect follows without knowing it exists.
    paused_banner(ui, state, registry, resolver, keymap, exec);

    // The breadcrumb band (tickets 05/09): the file chain this tab was
    // descended into — "character.animgraph ▸ locomotion.animgraph" — and,
    // while a rule scope is open, the non-file scope target after it. A
    // chain crumb navigates back to that ancestor; the file crumb closes an
    // open rule scope.
    match breadcrumb_band(ui, state) {
        BreadcrumbClick::None => {}
        BreadcrumbClick::CloseRule => state.close_rule_scope(registry),
        BreadcrumbClick::Ancestor(i) => {
            if let Some(path) = state.nav_back.get(i).cloned() {
                *open_subgraph = Some(GraphOpenRequest {
                    path,
                    back: state.nav_back[..i].to_vec(),
                });
            }
        }
    }
    // Promoted (⤢): the rule takes the full canvas; the machine's canvas and
    // strips yield entirely until the breadcrumb (or Esc) climbs back out.
    if state.rule_scope.as_ref().is_some_and(|s| s.full) {
        rule_scope_full(
            ui,
            state,
            registry,
            clipboard,
            resolver,
            keymap,
            selection_outline,
            &wire_prefs,
            zoom_min,
            zoom_max,
        );
        return;
    }

    // Ticket 06: an animation document's preview strip reserves its band off
    // the bottom before anything else measures — the vars strip and the
    // canvas both stop above it (the curve editor's footer idiom, per the
    // spec's placement ruling).
    let preview_h = if state.domain.is_animation() {
        let st = ui.style();
        PREVIEW_H * (st.metrics.row_height / BASE_ROW_H).max(0.1)
    } else {
        0.0
    };

    // The variables strip takes its column out of the available space the same
    // way the toolbar takes its row: the cursor moves right by the strip's
    // width before the canvas allocates, so the canvas shrinks by exactly that
    // much and every overlay measured off `out.rect` follows. Painted after
    // the canvas (the two rects are disjoint, so paint order only decides who
    // draws over a shared edge, and the strip should).
    let strip = {
        let st = ui.style();
        let s = (st.metrics.row_height / BASE_ROW_H).max(0.1);
        let w = if state.vars.open { VARS_W * s } else { VARS_RAIL_W * s };
        let c = ui.cursor();
        let r = Rect::from_min_max(c, Pos2::new(c.x + w, ui.available().max.y - preview_h));
        ui.set_cursor(Pos2::new(c.x + w, c.y));
        r
    };

    // While a peek is open the machine is a backdrop: its view must not move
    // under the overlay (pan/zoom gestures belong to the peek's canvas), so
    // the pre-frame view is restored after the canvas ran (ticket 05).
    let peek_view_lock = state.rule_scope.is_some().then_some(state.view);
    // Canvas needs `&mut CanvasView`; `CanvasView` is Copy, so pass a local
    // copy and write it back — keeps `state` fully borrowable in the body.
    let mut view = state.view;
    let mut annotation_menu_at: Option<Pos2> = None;
    let mut wire_menu_at: Option<Pos2> = None;
    let mut node_menu_at: Option<Pos2> = None;
    let mut canvas_menu_at: Option<Pos2> = None;
    let mut collapse_request = false;
    let mut layout_request = false;
    let mut cycle_error_request: Option<bool> = None;
    let mut frame_request: Option<CanvasView> = None;
    // Rule 2's threshold is a crusty constant, scaled by the editor's UI scale
    // so it stays 4 logical points at any scale factor. One source, so the
    // canvas's right-drag decision and the graph's own hit tests agree.
    let mut canvas = Canvas::new()
        .zoom_range(zoom_min, zoom_max)
        .drag_threshold(crusty_gui::input::drag_threshold(
            (ui.style().metrics.row_height / BASE_ROW_H).max(0.1),
        ));
    if preview_h > 0.0 {
        // The canvas fills what remains *above* the preview band.
        let a = ui.available_size();
        canvas = canvas.size(Vec2::new(a.x.max(1.0), (a.y - preview_h).max(1.0)));
    }
    let out = canvas.show(ui, &mut view, |ui, scope| {
        draw_and_interact(
            ui,
            scope,
            state,
            registry,
            resolver,
            &mut annotation_menu_at,
            &mut wire_menu_at,
            &mut node_menu_at,
            &mut canvas_menu_at,
            &mut collapse_request,
            &mut layout_request,
            &mut cycle_error_request,
            open_subgraph,
            selection_outline,
            &wire_prefs,
            zoom_min,
            zoom_max,
            &mut frame_request,
            keymap,
            exec,
            None,
        )
    });
    // F/A frame shortcuts re-fit the view (applied after the canvas ran, so it
    // replaces this frame's pan/zoom rather than fighting the live transform).
    if let Some(v) = frame_request {
        view = v;
    }
    state.view = view;
    if let Some(v) = peek_view_lock {
        state.view = v;
    }
    // The machine dims under an open peek — its lit trio (transition, source
    // and target states) excepted (mockup 3b).
    rule_dim_scrim(ui, out.rect, state, &out.inner);
    // …and dims the same way while a transition is merely *selected*
    // (mockup 2d), its trio and its own edge staying lit.
    anim_select_dim(ui, out.rect, state, registry, &out.inner);
    // Live preview highlight (ticket 06): the active state, the outgoing
    // side of an in-flight fade, and the transition that fired. Above the
    // scrim, so it stays readable while a rule peek is open.
    anim_live_highlight(ui, out.rect, state, &out.inner, anim);

    // A row released over the canvas: remember where, and ask Get or Set.
    // Taken here rather than inside the canvas body because the body runs
    // before the strip is drawn, and the payload only has to be claimed once
    // per frame — the drag survives frames on the context, not on either side.
    // A drop headed for the open peek is the *peek's* — the claim is
    // first-come, and the overlay's own drop target runs later this frame.
    let drop_is_peeks = state.rule_scope.is_some()
        && ui
            .ctx()
            .input
            .pointer_pos
            .is_some_and(|sp| rule_peek_rect(ui, out.rect, state, &out.inner).contains(sp));
    if drop_is_peeks {
    } else if let Some(p) = ui.dnd_drop_target::<VarDragPayload>(out.rect) {
        if state.domain.is_animation() {
            // Parameters are read *inside* a transition's rule, not on the
            // machine canvas — a top-level Get would compile to nothing.
            // The rule canvas (the peek overlay) is where this drop lands.
            state.toast("Parameters are read inside transition rules");
        } else if let Some(sp) = ui.ctx().input.pointer_pos {
            let v = state.view;
            state.vars.drop = Some(VarDrop {
                slug: p.slug,
                label: p.label,
                world: [
                    v.pan.x + (sp.x - out.rect.min.x) / v.zoom,
                    v.pan.y + (sp.y - out.rect.min.y) / v.zoom,
                ],
                screen: [sp.x, sp.y],
            });
        }
    }
    // Locate (GS-2): the panel names a node, the host frames it — the panel
    // has no viewport and no geometry, and this is the same pan-only move
    // find-in-graph and error cycling already make.
    let mut locate: Option<u64> = None;
    variables_panel(ui, strip, state, registry, &mut locate);
    if let Some(id) = locate {
        state.select_only(id);
        state.flash = Some((id, std::time::Instant::now()));
        if let Some((mn, mx)) = geoms_bbox(out.inner.iter().filter(|g| g.id == id)) {
            let v = frame_view(mn, mx, out.rect.size(), zoom_min, zoom_max);
            state.view = CanvasView { pan: v.pan, zoom: state.view.zoom };
        }
    }
    // The preview strip (ticket 06) paints its band under the canvas, then
    // its entity picker floats with the other transient surfaces.
    let mut anim_chip_rect: Option<(Rect, bool)> = None;
    if preview_h > 0.0 {
        let band = Rect::from_min_max(
            Pos2::new(strip.min.x, strip.max.y),
            Pos2::new(ui.available().max.x, strip.max.y + preview_h),
        );
        anim_chip_rect = Some(anim_preview_strip(ui, band, state, registry, anim, anim_instances));
    }
    if let Some((chip, just_opened)) = anim_chip_rect.filter(|_| state.anim_picker) {
        let st = ui.style();
        let s = (st.metrics.row_height / BASE_ROW_H).max(0.1);
        if let Some(pick) =
            anim_preview_picker(ui, chip, anim_instances, anim, &st, s, just_opened)
        {
            state.anim_bind = pick;
            state.anim_picker = false;
        }
    }
    var_drop_popup(ui, state, registry);
    // The instance picker floats over the canvas like every other transient
    // surface, and therefore draws with them.
    if let Some((chip, just_opened)) = live_chip_rect.filter(|_| state.exec_picker) {
        let st = ui.style();
        let s = (st.metrics.row_height / BASE_ROW_H).max(0.1);
        if let Some(pick) =
            instance_picker(ui, chip, exec_instances, exec, &st, s, just_opened)
        {
            state.exec_bind = pick;
            state.exec_picker = false;
        }
    }
    var_confirm_dialog(ui, out.rect, state, registry);
    payload_confirm_dialog(ui, out.rect, state, registry);

    palette_popover(ui, state, registry, subgraph_assets);
    find_overlay(ui, out.rect, state, &out.inner, zoom_min, zoom_max, registry);
    annotation_menu(ui, state, registry, keymap, annotation_menu_at);
    wire_menu(ui, state, registry, wire_menu_at);
    node_menu(ui, state, registry, keymap, &out.inner, node_menu_at);
    if let Some((world, screen)) =
        canvas_menu(ui, state, registry, keymap, &out.inner, clipboard, canvas_menu_at)
    {
        // "Add Node…" opens the palette where the menu was anchored, so the
        // node lands where the user right-clicked rather than wherever the row
        // happened to be.
        open_palette(state, world, screen, None);
    }
    edit_popup(ui, state, registry, out.rect);
    purge_confirm(ui, out.rect, state, registry);
    cheat_sheet(ui, out.rect, state, keymap);
    // The rule peek (ticket 05) draws over everything the machine owns; its
    // own transient surfaces stack above it.
    rule_peek_overlay(
        ui,
        out.rect,
        &out.inner,
        state,
        registry,
        clipboard,
        resolver,
        keymap,
        selection_outline,
        &wire_prefs,
        zoom_min,
        zoom_max,
    );
    draw_toasts(ui, out.rect, state);

    // F8 / Shift+F8 walk the anchored errors. The chip's own cursor drives
    // it, so clicking and keying stay in step.
    if let Some(forward) = cycle_error_request {
        let errors = ErrorIndex::build(&state.errors, &state.ref_errors, &state.domain_errors);
        if !forward {
            // Backwards = step back two, then forward one.
            let n = errors
                .ordered
                .iter()
                .filter(|e| e.anchor() != ErrorAnchor::Document)
                .count();
            if n > 0 {
                state.error_cursor = (state.error_cursor + n.saturating_sub(2)) % n;
            }
        }
        let mut req = None;
        cycle_error(
            state,
            &errors,
            &out.inner,
            out.rect.size(),
            zoom_min,
            zoom_max,
            &mut req,
            registry,
        );
        if let Some(v) = req {
            state.view = v;
        }
    }

    if layout_request {
        let rects = all_rects(&out.inner);
        let sp = layout_spacing(&ui.style());
        state.auto_layout(&rects, sp, registry);
    }

    // Ctrl+G writes a new asset, so it runs outside the draw pass.
    if collapse_request {
        if state.domain.is_animation() {
            // Subgraphs are the script library's factoring tool; a machine
            // factors by pointing a state's Graph row at another `.animgraph`
            // (ticket 09), which is an authoring act, not a collapse gesture.
            state.toast("An animation graph has no subgraphs");
        } else {
            match state.collapse_to_subgraph(std::path::Path::new("content"), registry) {
                Ok(rel) => println!("graph: collapsed selection into {rel}"),
                Err(e) => println!("graph: collapse to subgraph failed: {e}"),
            }
        }
    }
}

/// Is a text field or overlay holding the keyboard right now?
///
/// One predicate, consumed by every canvas key gate, so a shortcut can never
/// be added that forgets one of them: typing `c` in the palette's search must
/// add a letter, not a group.
/// Stack ids for the graph's own transient surfaces. They are drawn by the
/// engine rather than by a crusty widget, so they register themselves.
fn find_modal_id() -> crusty_gui::id::Id {
    crusty_gui::id::Id::ROOT.with("graph_find_overlay")
}

fn palette_modal_id() -> crusty_gui::id::Id {
    crusty_gui::id::Id::ROOT.with("graph_palette_overlay")
}

fn overlay_has_focus(ui: &Ui, state: &GraphEditorState) -> bool {
    overlay_has_focus_excl(ui, state, None)
}

/// [`overlay_has_focus`] for a canvas living *inside* a modal surface (the
/// rule peek, ticket 05): the surface registers itself on the modal stack to
/// shield the machine underneath, but must not count as "an open modal" for
/// its own body — only things stacked above it do.
fn overlay_has_focus_excl(ui: &Ui, state: &GraphEditorState, own_modal: Option<Id>) -> bool {
    state.palette.is_some()
        || state.find.is_some()
        || state.editing.is_some()
        // The variables strip's two transient surfaces: the Get/Set choice and
        // a confirmation. Both own Escape while they are up.
        || state.vars.drop.is_some()
        || state.vars.confirm.is_some()
        // The payload band's own confirmation and its in-flight name entry.
        || state.payload.confirm.is_some()
        || state.payload.draft.is_some()
        // Any crusty text field holding focus counts, wherever it lives — an
        // inspector name box, a search field in another panel. Single-key
        // shortcuts must never fire mid-typing.
        || ui.ctx().text_focused()
        // An open menu/dropdown owns the keyboard too.
        || match own_modal {
            Some(id) => ui.ctx().modal_any_open_above(id),
            None => ui.ctx().modal_any_open(),
        }
}

/// Tab-level shortcuts: clipboard, history, delete, save.
///
/// Chords no longer appear here — they live in the keymap, and this resolves
/// whatever the user bound to each action. The Pass A gating still runs first:
/// an open modal surface or a focused text field swallows everything, so `Del`
/// deletes a character rather than the selection while you are renaming.
fn handle_panel_keys(
    ui: &Ui,
    state: &mut GraphEditorState,
    registry: &NodeRegistry,
    clipboard: &mut Option<GraphFragment>,
    keymap: &Keymap,
    exec: Option<&GraphExecViz>,
) {
    // With a rule scope open the gate consults the *peek's* surfaces — the
    // scope's own modal must not swallow the keys it exists to serve
    // (ticket 05). The actions below route into the scope at the state level.
    let focus_taken = match &state.rule_scope {
        Some(scope) if !scope.full => {
            overlay_has_focus_excl(ui, &scope.child, Some(rule_peek_modal_id()))
        }
        Some(scope) => overlay_has_focus_excl(ui, &scope.child, None),
        None => overlay_has_focus(ui, state),
    };
    if focus_taken {
        return;
    }
    for action in keymap.dispatch(&ui.ctx().input, Context::GraphTab) {
        match action {
            Action::DELETE_SELECTION => state.delete_selection(registry),
            Action::UNDO => state.undo(registry),
            Action::REDO => state.redo(registry),
            Action::COPY => state.copy_selection(clipboard),
            Action::CUT => {
                state.copy_selection(clipboard);
                state.delete_selection(registry);
            }
            Action::PASTE => {
                // Paste lands at the cursor when the pointer is over this
                // canvas; otherwise (menu-driven) it falls back to the view.
                let at = ui.ctx().input.pointer_pos.map(|p| {
                    let v = state.view;
                    [v.pan.x + p.x / v.zoom, v.pan.y + p.y / v.zoom]
                });
                state.paste_clipboard(clipboard, at, registry);
            }
            Action::DUPLICATE => state.duplicate_selection(registry),
            Action::TOGGLE_VARIABLES => state.vars.open = !state.vars.open,
            // The debugger transport again, for the tab that has focus with
            // the pointer somewhere else — the canvas dispatch below only runs
            // while the pointer is over the graph.
            Action::DEBUG_RESUME | Action::DEBUG_STEP | Action::DEBUG_STOP
                if exec.is_some_and(|v| v.paused.is_some()) =>
            {
                state.debug_request = Some(match action {
                    Action::DEBUG_STEP => DebugRequest::Step,
                    Action::DEBUG_STOP => DebugRequest::Stop,
                    _ => DebugRequest::Resume,
                });
            }
            _ => {}
        }
    }
    // Save is the editor's, not the graph's — it is not in the graph keymap
    // and stays on its literal chord until a Global "file" action exists.
    if ui.ctx().input.modifiers == Modifiers::CTRL
        && ui.ctx().input.key_pressed(Key::Char('s'))
    {
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
    resolver: &DocResolvers<'_>,
    annotation_menu_at: &mut Option<Pos2>,
    wire_menu_at: &mut Option<Pos2>,
    node_menu_at: &mut Option<Pos2>,
    canvas_menu_at: &mut Option<Pos2>,
    collapse_request: &mut bool,
    layout_request: &mut bool,
    cycle_error_request: &mut Option<bool>,
    open_subgraph: &mut Option<GraphOpenRequest>,
    selection_outline: Color,
    wire_prefs: &WirePrefs,
    zoom_min: f32,
    zoom_max: f32,
    frame_request: &mut Option<CanvasView>,
    keymap: &Keymap,
    exec: Option<&GraphExecViz>,
    // The modal surface this canvas lives inside, when it lives inside one
    // (the rule peek, ticket 05): excluded from the modal gates so the
    // surface does not shield its own body.
    own_modal: Option<Id>,
) -> Vec<NodeGeom> {
    let st = ui.style();
    let zoom = scope.zoom();
    let lod = ZoomLod::from_zoom(zoom);
    let m = GraphMetrics::new(&st);
    let vis = scope.visible_world_rect();
    // Errors are resolved to their anchors before geometry, because ghost
    // rows change a node's shape.
    let errors = ErrorIndex::build(&state.errors, &state.ref_errors, &state.domain_errors);
    let mut geoms = {
        let mut p = ui.painter();
        build_geoms(state, registry, resolver, &errors, &m, &st, &mut p)
    };
    // Task 41 rework: the machine canvas derives its topology geometry —
    // straight border-to-border edges with the transition chips riding their
    // midpoints. Chips with both endpoints wired move to the derived spot
    // (their stored position stays untouched in the document); every
    // downstream consumer — hit tests, error badges, F8 framing, the live
    // highlight — reads the translated geoms and follows for free.
    let anim_flow = state.domain.is_animation().then(|| {
        let rects: BTreeMap<u64, [f32; 4]> = geoms
            .iter()
            .map(|g| (g.id, [g.rect.min.x, g.rect.min.y, g.rect.max.x, g.rect.max.y]))
            .collect();
        super::graph_anim_edge::anim_flow_layout(&state.doc, &rects)
    });
    if let Some(l) = &anim_flow {
        for g in &mut geoms {
            if let Some(mn) = l.chip_min.get(&g.id) {
                g.translate(Vec2::new(mn[0] - g.rect.min.x, mn[1] - g.rect.min.y));
            }
        }
        // An unfolded (selected) card is the thing being edited: nothing may
        // cross it. Selected nodes move to the end of the paint order — which
        // is also the hit order, `node_under` taking the last hit — so open
        // cards sit above every other chip and state. Stable, so unselected
        // nodes keep their relative document order.
        geoms.sort_by_key(|g| state.selection.contains(&g.id));
    }
    let geoms = geoms;

    state.frame = state.frame.wrapping_add(1);
    let frame = state.frame;
    // A graph opened with no remembered view frames its content once, on the
    // first draw — that is the only point the geometry is known.
    if state.frame_all_on_open {
        state.frame_all_on_open = false;
        if let Some((mn, mx)) = content_bbox(state, &geoms) {
            *frame_request = Some(frame_view(mn, mx, scope.rect().size(), zoom_min, zoom_max));
        }
    }
    let node_rects: Vec<Rect> = geoms.iter().map(|g| g.rect).collect();
    let wires = build_wires(
        state,
        &geoms,
        &node_rects,
        &errors,
        wire_prefs,
        scope,
        vis,
        exec,
        anim_flow.as_ref().map(|l| &l.segs),
    );
    let hovered_wire = wire_under(&wires, ui.ctx().input.pointer_pos);
    // Set when Alt+click removed a wire this frame, so nothing downstream
    // treats the same press as a reroute grab or a selection change.
    let mut alt_broke_wire = false;
    // What the in-flight cut would take, tested against the drawn polylines
    // so the preview and the release can never disagree.
    let mut cut_preview: BTreeSet<usize> = crossed_indices(state, &wires, scope);
    // Rule 5 — Alt is a *precursor*: holding it shows what the click would
    // take, before the click. A hovered wire joins the cut preview, so the
    // break reuses the red-dashed language the slash-cut already speaks
    // rather than inventing a second one.
    if ui.ctx().input.modifiers.contains(Modifiers::ALT) {
        if let Some(i) = hovered_wire {
            cut_preview.insert(i);
        }
    }

    // A single node dragged over a wire it could sit on: the wire highlights
    // and a drop splices it in.
    let splice_target: Option<usize> = state
        .node_drag
        .as_ref()
        .filter(|d| d.originals.len() == 1)
        .and_then(|d| {
            let id = d.originals[0].0;
            let g = geoms.iter().find(|g| g.id == id)?;
            let centre = scope.world_to_screen(g.rect.center());
            wires
                .iter()
                .filter(|w| w.edge_index != usize::MAX)
                .find(|w| {
                    point_polyline_distance(centre, &w.screen) <= SPLICE_SNAP_PX
                        && state
                            .doc
                            .edges
                            .get(w.edge_index)
                            .is_some_and(|e| state.splice_pins(e, id, registry).is_some())
                })
                .map(|w| w.edge_index)
        });


    draw_grid(ui, scope, &st, vis, zoom);
    draw_annotations(ui, scope, state, &st, &m, vis, zoom, lod);
    draw_wires(
        ui,
        scope,
        &wires,
        hovered_wire.or(splice_target),
        &cut_preview,
        Some(registry),
        wire_prefs,
        &st,
        lod,
        selection_outline,
        exec.is_some_and(|v| v.has_session()),
        zoom,
        (frame % 60) as f32 / 60.0,
    );

    // The variables filter dims the canvas on the **same rule** find-in-graph
    // does (GS-2): what a search does not match drops to 45% rather than
    // hiding, so the context that makes a result mean something survives. The
    // set is the Get/Set nodes of the variables the filter matched; everything
    // else — including unrelated nodes — dims, exactly as the design shows.
    let var_dim: Option<BTreeSet<u64>> = (!state.vars.filter.trim().is_empty()).then(|| {
        state
            .doc
            .variables
            .iter()
            .filter(|v| variable_matches(v, &state.vars.filter))
            .flat_map(|v| variable_node_ids(&state.doc, &v.slug))
            .collect()
    });

    // Watches take this frame's values before anything draws, so the chip and
    // the tooltip cannot disagree about what the pin holds. Off-session this
    // does nothing and the chips keep the last run's value — the layer's only
    // edit-mode residue, and the reason a watch is an editor annotation.
    if let Some(viz) = exec {
        for i in 0..state.watches.len() {
            let (node, pin, output) = {
                let w = &state.watches[i];
                (w.node, w.pin.clone(), w.output)
            };
            if let Some(v) = pin_value(viz, state, node, &pin, output).map(str::to_string) {
                state.watches[i].observe(&v);
            }
        }
    }

    // Nodes, pins and inline widgets. `widget_rects` records the screen boxes
    // owned by embedded controls so the node-drag pass can yield to them.
    let mut widget_rects: Vec<Rect> = Vec::new();
    let mut watch_rects: Vec<(u64, String, bool, Rect)> = Vec::new();
    let mut badge_rects: Vec<(u64, Rect)> = Vec::new();
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
        frame,
        selection_outline,
        &mut widget_rects,
        exec,
        var_dim.as_ref(),
        &mut watch_rects,
        &mut badge_rects,
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
    let alt = mods.contains(Modifiers::ALT);
    let ctrl = mods.contains(Modifiers::CTRL);

    // ── Rule 3: every drag is abortable ──────────────────────────────────────
    // Escape abandons whatever gesture is running; so does losing the window
    // (the OS drops pointer capture, so resuming against a pointer that moved
    // while we were away is worse than starting over), and so does the right
    // button interrupting a left drag. All three revert to the pre-drag state
    // and record nothing — `cancel_interactions` is the single revert path.
    //
    // Note the ordering: this runs before any gesture-start handling below, so
    // the aborting press cannot also arm a fresh gesture on the same frame.
    if state.interaction_in_flight() {
        let escaped = ui.ctx().input.key_pressed(Key::Escape);
        let interrupted = right_pressed && pointer_down;
        if escaped || interrupted || ui.ctx().focus_lost() {
            // The slash-cut is the one gesture with no visible revert (the
            // path simply vanishes), so it says so.
            if state.cut_path.is_some() {
                state.toast("Cut cancelled");
            }
            state.cancel_interactions();
            if escaped {
                // Consume it, or the same Escape also closes the panel's
                // overlays on its way past.
                ui.ctx_mut().input.consume_key(Key::Escape);
            }
            // Nothing else this frame: an aborting press must not arm a new
            // gesture, and the geometry is already drawn.
            return geoms;
        }
    }

    // A press landing on any open transient surface — context menu, combo
    // dropdown, the palette, the find bar — was already consumed by the modal
    // stack before we got here (Rule 1). Asking the stack directly also keeps
    // *hover* interactions off the canvas underneath one, which consumption
    // alone would not.
    let widget_claimed = pointer_screen.is_some_and(|p| {
        (match own_modal {
            Some(id) => ui.ctx().modal_contains_above(id, p),
            None => ui.ctx().modal_contains(p),
        }) || widget_owns(p, &widget_rects)
    });

    // Frame shortcuts. F frames the selection, Home fits the whole graph.
    // (Bare `A` used to mean fit-graph; the ratified table gives that job to
    // Home and frees `A` for the Shift+W/A/S/D align family.) The keymap
    // matches modifiers exactly, which replaces the old blanket
    // `mods.is_empty()` guard — that existed only to stop Ctrl+A and Ctrl+F
    // over the canvas also framing the view.
    if pointer_world.is_some() && !overlay_has_focus_excl(ui, state, own_modal) {
        let framing = keymap.dispatch(&ui.ctx().input, Context::Canvas);
        let frame_all = framing.contains(&Action::FIT_GRAPH);
        let frame_sel = framing.contains(&Action::FRAME_SELECTION);
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
            // Dropped on a wire, with a node that can sit on it? Splice.
            let dragged = (originals.len() == 1).then(|| originals[0].0);
            let spliced = dragged.zip(splice_target).is_some_and(
                |(id, ei)| {
                    let Some(edge) = state.doc.edges.get(ei).cloned() else {
                        return false;
                    };
                    match state.splice_pins(&edge, id, registry) {
                        Some((i, o)) => {
                            finish_node_drag(state, registry);
                            state.splice_node_into(&edge, id, &i, &o, registry)
                        }
                        None => false,
                    }
                },
            );
            if !spliced {
                finish_node_drag(state, registry);
            }
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

    // Midpoint handle: hovering a wire offers a grab at its arc-length
    // midpoint. Taking it inserts a reroute *and* hands it straight to a
    // drag, so grabbing the wire and moving it is one gesture rather than
    // insert-then-find-the-thing-you-just-made.
    let mut midpoint_grab: Option<(Edge, [f32; 2])> = None;
    // Rule 4 — Alt is the break modifier, and a wire is the most direct thing
    // it can break. Handled before the midpoint handle so an Alt press over
    // the middle of a wire deletes it instead of grabbing a reroute.
    // (A pin under the pointer wins — the pin loop runs later and would break
    // its own links; `wire_under` already excludes the pin end zones.)
    if alt && pointer_pressed {
        if let Some(i) = hovered_wire {
            if let Some(edge) = state.doc.edges.get(i).cloned() {
                state.break_links(&[edge], "Delete", registry);
                alt_broke_wire = true;
            }
        }
    }
    // Set when the reroute-insert handle owns the pointer, so the value
    // tooltip below does not talk over it.
    let mut midpoint_hovered = false;
    if let Some(i) = hovered_wire.filter(|_| !alt_broke_wire) {
        // A derived machine edge (Task 41 rework) offers no reroute handle:
        // its midpoint belongs to the transition chip, and a reroute spliced
        // into a flow wire would knock the edge off its straight rendering.
        if let Some(w) = wires
            .iter()
            .find(|w| w.edge_index == i)
            .filter(|w| !w.direct)
        {
            if let Some(mid) = arc_length_midpoint(&w.screen) {
                let r = MIDPOINT_R;
                let handle = Rect::from_center_size(mid, Vec2::splat(r * 2.0));
                let id = ui.alloc_id(("graph_wire_mid", i));
                let resp = ui.interact(id, handle);
                midpoint_hovered = resp.hovered;
                {
                    let mut p = ui.painter();
                    p.circle_filled(
                        mid,
                        if resp.hovered { r } else { r * 0.7 },
                        st.palette.focus_ring,
                    );
                    p.circle_stroke(mid, r, m.border, st.palette.elevated);
                }
                if resp.hovered {
                    ui.tooltip_for(handle, "Drag to insert a reroute");
                }
                if resp.pressed {
                    if let (Some(edge), Some(pw)) =
                        (state.doc.edges.get(i).cloned(), pointer_world)
                    {
                        midpoint_grab = Some((edge, [pw.x, pw.y]));
                    }
                }
            }
        }
    }
    // Value hover on a wire (45-A P7). Only with a bound instance, only on
    // data wires — an exec wire carries control, not a value, and saying "-"
    // for it would be noise. Anchored at the pointer rather than at the wire's
    // midpoint: the wire is what is under the cursor, not the handle.
    if let (Some(viz), Some(i), Some(sp)) = (
        exec,
        hovered_wire.filter(|_| !alt_broke_wire && !midpoint_hovered),
        ui.ctx().input.pointer_pos,
    ) {
        if let Some(e) = state.doc.edges.get(i) {
            let is_exec = wires
                .iter()
                .find(|w| w.edge_index == i)
                .is_some_and(|w| w.is_exec());
            if !is_exec {
                if let Some(v) = pin_value(viz, state, e.to_node, &e.to_pin, false) {
                    ui.tooltip_for(
                        Rect::from_center_size(sp, Vec2::splat(MIDPOINT_R)),
                        &format!("Value  {v}"),
                    );
                }
            }
        }
    }
    if let Some((edge, at)) = midpoint_grab {
        let d = m.reroute_d() * 0.5;
        state.grab_wire_midpoint(&edge, [at[0] - d, at[1] - d], registry);
    }
    let midpoint_claimed = state.node_drag.is_some() && hovered_wire.is_some();

    // Wire selection. A wire is behind every node and pin, so it only claims
    // a press nothing in front of it wanted.
    let mut wire_claimed = midpoint_claimed;
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
    let mut break_pin: Option<(u64, String, bool)> = None;
    let mut watch_toggle: Option<(u64, String, bool)> = None;
    if lod.rows() {
        // Register interaction only for what is on screen: an off-canvas node
        // costs one rect intersection instead of a widget-memory entry and a
        // hit-test per pin, which is what keeps the stated budget reachable.
        for g in geoms.iter().filter(|g| visible(g, lod, &m, vis)) {
            for pin in &g.pins {
                // A zero hit target (a machine card's hidden border anchor,
                // Task 41 rework) takes no interaction at all — the border
                // gesture below owns that surface.
                if pin.hit_w == Some(0.0) {
                    continue;
                }
                let wr = Rect::from_center_size(
                    pin.dot_center,
                    Vec2::splat(pin.hit_w.unwrap_or(hit_w)),
                );
                let id = ui.alloc_id(("graph_pin", g.id, &pin.slug, pin.output));
                let resp = scope.interact(ui, id, wr);
                let watched = state
                    .watches
                    .iter()
                    .any(|w| w.is(g.id, &pin.slug, pin.output));
                // Right-click a data pin pins (or unpins) its value as a chip.
                // Exec carries control, not a value, and has nothing to watch.
                if resp.hovered && right_pressed && pin.ty != PinType::Exec {
                    watch_toggle = Some((g.id, pin.slug.clone(), pin.output));
                }
                // Pin hover docs: type name always, descriptor line when the
                // node type bothered to write one. Removes an inspector
                // round-trip exactly when the user is wiring.
                if resp.hovered && state.connect_drag.is_none() {
                    let mut tip = format!(
                        "{}  {}",
                        pin.label,
                        graph_palette::type_tag(&pin.ty)
                    );
                    if let Some(doc) =
                        pin_doc(registry, resolver, state, g.id, &pin.slug, pin.output)
                    {
                        tip.push('\n');
                        tip.push_str(&doc);
                    }
                    // The live block (GS-3): the last value that crossed this
                    // pin, how long ago, and the way to pin it. Same spelling
                    // `Print` uses, so the tooltip and the console agree.
                    if let Some(v) = exec
                        .filter(|_| pin.ty != PinType::Exec)
                        .and_then(|viz| pin_value(viz, state, g.id, &pin.slug, pin.output))
                    {
                        tip.push_str("\nLAST  ");
                        tip.push_str(v);
                        if let Some(age) = exec.and_then(|viz| {
                            real_producer(state, g.id, &pin.slug)
                                .filter(|_| pin.output)
                                .and_then(|(n, p)| viz.value_age(n, &p))
                        }) {
                            tip.push_str(&format!("  \u{b7} {age:.1} s ago"));
                        }
                    }
                    if pin.ty != PinType::Exec {
                        // The tooltip cannot be clicked, so it names the
                        // gesture instead of pretending to be a button.
                        tip.push_str(if watched {
                            "\n\u{2715} Watch  \u{b7} right-click to unpin"
                        } else {
                            "\n+ Watch  \u{b7} right-click to pin"
                        });
                    }
                    ui.tooltip_for(scope.world_rect_to_screen(wr), &tip);
                }
                // The pin's own precursor: a danger ring around the dot while
                // Alt is held, so a connected pin reads as breakable before
                // the press rather than after it.
                if resp.hovered && alt && pin.connected {
                    let c = scope.world_to_screen(pin.dot_center);
                    let r = m.pin_r * zoom * 1.6;
                    ui.painter()
                        .circle_stroke(c, r, m.border.max(1.0), Palette::invariant_status().error);
                }
                if !resp.pressed {
                    continue;
                }
                // Alt-click breaks instead of connecting (Unreal's gesture).
                // It claims the press so no drag starts and selection is left
                // exactly as it was.
                if alt {
                    break_pin = Some((g.id, pin.slug.clone(), pin.output));
                    pin_claimed = true;
                } else if state.connect_drag.is_none() && state.node_drag.is_none() {
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
    if let Some((node, pin, output)) = break_pin {
        state.break_pin_links(node, &pin, output, registry);
    }
    if let Some((node, pin, output)) = watch_toggle {
        let before = state.watches.len();
        state.watches.retain(|w| !w.is(node, &pin, output));
        if state.watches.len() == before {
            state.watches.push(Watch::new(node, &pin, output));
            state.toast(format!("Watching {pin}"));
        } else {
            state.toast(format!("Unpinned {pin}"));
        }
    }

    // A watch chip's ✕: hover-revealed like every other quiet remove in the
    // system, and it un-pins rather than deleting anything.
    let mut drop_watch: Option<(u64, String, bool)> = None;
    for (node, pin, output, chip) in &watch_rects {
        let id = ui.alloc_id(("graph_watch_chip", *node, pin.as_str(), *output));
        let resp = ui.interact(id, *chip);
        if resp.hovered {
            let mut p = ui.painter();
            let px = st.fonts.small * zoom;
            let x = chip.max.x - px * 0.9;
            p.rect_filled(
                Rect::from_min_max(Pos2::new(x - px * 0.3, chip.min.y), chip.max),
                st.rounding.small,
                st.palette.input,
            );
            p.text(
                Pos2::new(x, chip.center().y - px * 0.62),
                "\u{2715}",
                px,
                st.palette.text,
                None,
            );
            let full = state
                .watches
                .iter()
                .find(|w| w.is(*node, pin, *output))
                .and_then(|w| w.last.clone())
                .unwrap_or_else(|| "no value yet".to_string());
            ui.tooltip_for(*chip, &format!("{pin}  {full}\nClick to unpin"));
        }
        if resp.clicked {
            drop_watch = Some((*node, pin.clone(), *output));
        }
    }
    if let Some((node, pin, output)) = drop_watch {
        state.watches.retain(|w| !w.is(node, &pin, output));
    }

    // The badge gutter (GS-4): click a mark to arm or disarm it, Alt+click to
    // remove it. Alt is the destroy verb everywhere else in this canvas — on a
    // node header it breaks the links — so the slot claims the press to keep
    // the two apart, and a mark is *created* with F9 or the node menu rather
    // than by clicking an empty gutter that draws nothing.
    let mut badge_claimed = false;
    let mut badge_edit: Option<(u64, bool)> = None;
    for (id, rect) in &badge_rects {
        let iid = ui.alloc_id(("graph_break_badge", *id));
        let resp = ui.interact(iid, *rect);
        if resp.hovered {
            let armed = state.breakpoint_armed(*id);
            ui.tooltip_for(
                *rect,
                if armed {
                    "Breakpoint armed\nClick to disable · Alt+click to remove"
                } else {
                    "Breakpoint disabled\nClick to arm · Alt+click to remove"
                },
            );
        }
        if resp.pressed {
            badge_claimed = true;
        }
        if resp.clicked {
            badge_edit = Some((*id, alt));
        }
    }
    if let Some((id, remove)) = badge_edit {
        if remove {
            state.remove_breakpoint(id);
            state.toast("Breakpoint removed");
        } else {
            state.cycle_breakpoint(id);
        }
    }

    // Config band: the ✕ on a payload row, the slug (rename) and the "+ field"
    // ghost row. Before the node body so none of the three starts a drag, and
    // only where the band is drawn — an affordance you cannot see must not be
    // clickable.
    let mut config_claimed = false;
    // `(node, slug, new name)` — `None` is a removal. Collected here and
    // resolved below, where the cross-document reader count can be asked.
    let mut payload_request: Option<(u64, String, Option<String>)> = None;
    // `widget_claimed` covers the embedded controls, the in-flight name entry
    // among them: without it, clicking into the draft field to move the caret
    // would land on the ghost row underneath and restart the draft empty.
    if lod.config_band() && !alt && !widget_claimed {
        for g in geoms.iter().filter(|g| visible(g, lod, &m, vis)) {
            for cfg in &g.config {
                let Some(slug) = cfg.payload_slug() else { continue };
                if let Some(r) = cfg.remove {
                    let id = ui.alloc_id(("graph_payload_x", g.id, slug));
                    let resp = scope.interact(ui, id, r);
                    if resp.hovered {
                        ui.tooltip_for(scope.world_rect_to_screen(r), "Remove field");
                    }
                    if resp.pressed {
                        payload_request = Some((g.id, slug.to_string(), None));
                        config_claimed = true;
                    }
                }
                // The slug itself opens a rename, pre-filled. **Double**-click,
                // deliberately: a single press over the band still belongs to
                // the node, so a custom event can be dragged by its own rows.
                let id = ui.alloc_id(("graph_payload_name", g.id, slug));
                let resp = scope.interact(ui, id, cfg.label_box);
                if resp.hovered {
                    ui.tooltip_for(
                        scope.world_rect_to_screen(cfg.label_box),
                        "Double-click to rename",
                    );
                }
                if resp.double_clicked(ui) && lod.inline_widgets() {
                    state.payload.draft = Some(PayloadDraft {
                        node: g.id,
                        slug: Some(slug.to_string()),
                        name: slug.to_string(),
                        first_frame: true,
                        seen_focus: false,
                        submitted: false,
                    });
                    config_claimed = true;
                }
            }
            if let Some(r) = g.add_field {
                let id = ui.alloc_id(("graph_payload_add", g.id));
                let resp = scope.interact(ui, id, r);
                if resp.hovered {
                    ui.tooltip_for(
                        scope.world_rect_to_screen(r),
                        "Add a payload field \u{2014} Enter commits",
                    );
                }
                if resp.pressed && lod.inline_widgets() {
                    state.payload.draft = Some(PayloadDraft {
                        node: g.id,
                        slug: None,
                        name: String::new(),
                        first_frame: true,
                        seen_focus: false,
                        submitted: false,
                    });
                    config_claimed = true;
                }
            }
        }
    }

    // A committed name entry (the field widget ran during the draw pass and
    // only reported). Adding is never breaking, so it goes straight through;
    // a rename joins the remove path, which asks about readers first.
    if state.payload.draft.as_ref().is_some_and(|d| d.submitted) {
        if let Some(d) = state.payload.draft.take() {
            let name = d.name.trim().to_string();
            match d.slug {
                _ if name.is_empty() => {}
                None => {
                    state.add_payload_field(d.node, &name, registry);
                }
                Some(slug) if variable_slug(&name) == slug => {}
                Some(slug) => payload_request = Some((d.node, slug, Some(name))),
            }
        }
    }
    if let Some((node, slug, rename)) = payload_request {
        let event = state
            .doc
            .node(node)
            .and_then(|n| match n.properties.get(EVENT_NAME_PROP) {
                Some(PropValue::Str(s)) | Some(PropValue::Enum(s)) => Some(s.clone()),
                _ => None,
            })
            .unwrap_or_default();
        let (readers, graphs) =
            payload_reader_count(&state.doc, &state.path, &event, &slug, resolver.graphs);
        match rename {
            // Zero readers is no ceremony: nothing downstream can notice.
            Some(name) if readers == 0 => {
                state.rename_payload_field(node, &slug, &name, registry);
            }
            Some(name) => {
                state.payload.confirm =
                    Some(PayloadConfirm::Rename { node, slug, name, readers, graphs });
            }
            None if readers == 0 => {
                state.remove_payload_field(node, &slug, registry);
            }
            None => {
                state.payload.confirm =
                    Some(PayloadConfirm::Remove { node, slug, readers, graphs });
            }
        }
    }

    // Node body: select + start drag.
    //
    // Skipped entirely while an embedded control owns the pointer. `interact`
    // does not merely *ask* whether a rect is pressed — it claims
    // `active_widget` for the id it is given, and the node body is drawn under
    // every inline widget. Asking here therefore took the press away from the
    // `DragValue`/`Checkbox` that ran a moment ago in `draw_nodes`, and those
    // widgets live entirely on `active_widget` (scrub *and* click-to-type read
    // it on the frames after the press). That is why an inline value could be
    // hovered but never edited — and why letting the pointer wander off the
    // field mid-press handed the leftover claim to the node as a drag. Guarding
    // the *gestures* is not enough; the question itself has to go unasked. The
    // config band has followed this rule since GS-1.
    let mut begin_drag = false;
    let mut node_pressed = false;
    let mut break_node: Option<u64> = None;
    for g in geoms.iter().filter(|g| visible(g, lod, &m, vis)) {
        if widget_claimed {
            break;
        }
        let id = ui.alloc_id(("graph_node", g.id));
        let body = g.body_rect(lod, &m);
        let resp = scope.interact(ui, id, body);
        if resp.pressed {
            node_pressed = true;
        }
        // Node header hover: the node's own doc line.
        if resp.hovered && !g.reroute && state.node_drag.is_none() {
            let header = Rect::from_min_size(
                g.rect.min,
                Vec2::new(g.rect.width(), m.header_h),
            );
            if pointer_world.is_some_and(|p| header.contains(p)) {
                if let Some(doc) = node_doc(registry, resolver, state, g.id) {
                    ui.tooltip_for(scope.world_rect_to_screen(header), &doc);
                }
            }
        }
        // Alt-click the header breaks every link the node has — the pin
        // gesture, extended to the whole node.
        if resp.pressed && alt && !pin_claimed && !badge_claimed {
            let header = Rect::from_min_size(
                g.rect.min,
                Vec2::new(g.rect.width(), m.header_h),
            );
            if pointer_world.is_some_and(|p| header.contains(p)) || g.reroute {
                break_node = Some(g.id);
            }
        }
        // Task 41 rework: a press on a machine card's border rim starts a
        // wire from its Out — the modern engines' "drag from the state
        // edge", with no pin dot to aim at. Release over another state
        // auto-inserts a Transition through the existing auto-connect path.
        if resp.pressed
            && !alt
            && !shift
            && !pin_claimed
            && !badge_claimed
            && !config_claimed
            && !widget_claimed
            && !wire_claimed
            && state.node_drag.is_none()
            && state.connect_drag.is_none()
            // Compact cards, and the unfolded (selected) state — which lost
            // its `anim` marker by taking the standard-card path.
            && (g.anim.is_some()
                || (state.domain.is_animation()
                    && state.doc.node(g.id).is_some_and(|n| {
                        n.type_id
                            == crate::engine::animation::graph::plan::ANIM_STATE_TYPE_ID
                    })))
        {
            if let Some(pw) = pointer_world {
                let b = g.body_rect(lod, &m);
                let grab = (BORDER_GRAB_PX / zoom).min(b.width().min(b.height()) * 0.33);
                if b.contains(pw) && !b.shrink(grab).contains(pw) {
                    use crate::engine::animation::graph::plan::STATE_OUT_PIN;
                    state.connect_drag = Some(ConnectDrag {
                        from_node: g.id,
                        from_pin: STATE_OUT_PIN.to_string(),
                        from_output: true,
                    });
                    pin_claimed = true;
                }
            }
        }
        if resp.pressed && !alt
            && !pin_claimed
            && !badge_claimed
            && !config_claimed
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
            // A chip riding its edge (Task 41 rework) has no position of its
            // own to drag — a grab selects it and nothing more.
            if !g.pinned_pos {
                begin_drag = true;
            }
        }
        if resp.clicked && shift && !alt {
            state.toggle_selected(g.id);
        }
        // Double-click always means "descend" (spec): a subgraph node or a
        // nested-`.animgraph` state opens its referenced file as a tab, a
        // transition descends into its embedded rule as a peek (ticket 05) —
        // same gesture, same meaning, the breadcrumb tells files and rules
        // apart.
        if resp.double_clicked(ui) {
            if let Some(path) = state.file_descend_target(g.id) {
                *open_subgraph = Some(state.open_request(path));
            } else {
                state.open_rule_scope(g.id, registry);
            }
        }
    }
    if let Some(id) = break_node {
        state.break_node_links(id, registry);
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
                Some(NodeDrag { origin_world: [pw.x, pw.y], originals, anchored, pending: None });
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
    // A group bar runs under the nodes it holds, so it asks the same question
    // the node body does and has to hold its tongue on the same condition.
    for i in (0..state.doc.groups.len()).rev() {
        if widget_claimed {
            break;
        }
        let r = state.doc.groups[i].rect;
        let bar = Rect::from_min_size(Pos2::new(r[0], r[1]), Vec2::new(r[2], m.group_bar));
        if !overlaps(bar, vis) {
            continue;
        }
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
        if widget_claimed {
            break;
        }
        let r = state.doc.comments[i].rect;
        let bar_h = m.comment_bar * state.doc.comments[i].clamped_font_scale();
        let bar = Rect::from_min_size(Pos2::new(r[0], r[1]), Vec2::new(r[2], bar_h));
        if !overlaps(bar, vis) {
            continue;
        }
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
                        state,
                        registry,
                        from_node,
                        &from_pin,
                        from_output,
                        sty,
                        pin_is_untyped(&geoms, from_node, &from_pin, from_output),
                        h.0,
                        &h.1,
                        h.3,
                        &h.2,
                        pin_is_untyped(&geoms, h.0, &h.1, h.3),
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
                converge_index: 0,
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
                // A wire being dragged is not running: it does not exist yet.
                pulse: 0.0,
                taken: true,
                rate: 0.0,
                direct: false,
                arrow: false,
            };
            stroke_wire(&mut p, &ghost, wire_prefs, scope, width, tint);
        }
        if released {
            let landed = resolve_connection(state, &geoms, pointer_world, hit_w, registry);
            if !landed {
                if let Some(pw) = pointer_world {
                    let src_ty = pin_ty(&geoms, from_node, &from_pin, from_output);
                    let label = geoms
                        .iter()
                        .find(|g| g.id == from_node)
                        .and_then(|g| {
                            g.pins
                                .iter()
                                .find(|q| q.output == from_output && q.slug == from_pin)
                        })
                        .map(|q| q.label.clone())
                        .unwrap_or_default();
                    if let Some(ty) = src_ty {
                        let src = PaletteDragSource {
                            node: from_node,
                            pin: from_pin.clone(),
                            output: from_output,
                            ty: ty.clone(),
                            label: label.clone(),
                        };
                        // Released on a node body: auto-connect to its best
                        // compatible pin, no palette. Released on empty
                        // canvas: the type-filtered palette.
                        match node_under(&geoms, pw, lod, &m) {
                            Some(target) if target != from_node => {
                                auto_connect(state, registry, resolver, &src, target);
                            }
                            _ => {
                                if let Some(sp) = ui.ctx().input.pointer_pos {
                                    open_palette(state, [pw.x, pw.y], [sp.x, sp.y], Some(src));
                                }
                            }
                        }
                    }
                }
            }
            state.connect_drag = None;
        } else if !pointer_down {
            state.connect_drag = None;
        }
    }

    // Ctrl-drag slash cut. Arms only on an empty-canvas press, so Ctrl-drag
    // over a node keeps whatever meaning that has; the wheel's Ctrl+scroll
    // zoom is a different input entirely and never conflicts.
    let mut cut_now: Vec<Edge> = Vec::new();
    {
        if pointer_pressed
            && ctrl
            && state.cut_path.is_none()
            && !pin_claimed
            && !node_pressed
            && !wire_claimed
            && !widget_claimed
            && state.annotation_resize.is_none()
        {
            if let Some(pw) = pointer_world {
                if node_under(&geoms, pw, lod, &m).is_none()
                    && pin_under(&geoms, pw, hit_w).is_none()
                    && annotation_at(state, &m, pw).is_none()
                {
                    state.cut_path = Some(vec![[pw.x, pw.y]]);
                }
            }
        }
        // Escape no longer needs handling here: the Rule 3 abort at the top
        // of this pass reverts every gesture through one path, this one
        // included, and toasts on the way out.
        if state.cut_path.is_some() {
            if let Some(pw) = pointer_world {
                state.push_cut_point([pw.x, pw.y]);
            }
            if !pointer_down {
                cut_now = crossed_by_cut(state, &wires, scope);
                state.cut_path = None;
            }
        }
    }
    if !cut_now.is_empty() {
        state.break_links(&cut_now, "Cut", registry);
    }
    // Marquee is plain-LMB, so it must yield while a cut is armed.
    let cutting = state.cut_path.is_some();

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
            blocked: pin_claimed
                || node_pressed
                || widget_claimed
                || wire_claimed
                || cutting
                || ctrl,
            mods,
            lod,
            hit_w,
        },
        &m,
        &st,
    );

    // Double-click empty canvas also opens the palette — the same gesture
    // that opens an asset picker anywhere else in the editor. `!widget_claimed`
    // is load-bearing rather than decorative: this rect is the whole visible
    // canvas, so without it the background would claim every press an inline
    // widget just took (see the node-body loop).
    if !node_pressed && !pin_claimed && !wire_claimed && !widget_claimed && state.palette.is_none()
    {
        let id = ui.alloc_id("graph_canvas_bg");
        let resp = scope.interact(ui, id, vis);
        if resp.double_clicked(ui) {
            if let (Some(pw), Some(sp)) = (pointer_world, ui.ctx().input.pointer_pos) {
                if node_under(&geoms, pw, lod, &m).is_none()
                    && annotation_at(state, &m, pw).is_none()
                {
                    open_palette(state, [pw.x, pw.y], [sp.x, sp.y], None);
                }
            }
        }
    }

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

    // The cut path itself: dashed error-red, thin, drawn over everything it
    // is about to sever.
    if let Some(path) = state.cut_path.clone() {
        let status = Palette::invariant_status();
        let mut p = ui.painter();
        let pts: Vec<Pos2> = path
            .iter()
            .map(|q| scope.world_to_screen(Pos2::new(q[0], q[1])))
            .collect();
        for w in pts.windows(2) {
            dashed_line(&mut p, w[0], w[1], CUT_STROKE, status.error);
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
        keymap,
        registry,
    );

    // Right-click: an annotation gets its own menu (tint / collapse / anchor
    // / delete); empty canvas gets the create menu.
    //
    // Rule 2 — this fires on the *release* of a right press that never became
    // a pan, not on the press. The canvas owns that decision (it also owns
    // right-drag panning), so both readings of the button come from one place
    // and can never both fire for one gesture.
    if let Some(rc) = scope.right_clicked() {
        let menu_at = Some(rc);
        {
            let pw = scope.screen_to_world(rc);
            let on_annotation = annotation_at(state, &m, pw);
            let on_node = node_under(&geoms, pw, lod, &m);
            if let Some(i) = hovered_wire {
                // Right-clicking a wire offers the one thing that belongs to
                // a wire: splitting it.
                state.wire_menu = state.doc.edges.get(i).cloned().map(|e| (e, [pw.x, pw.y]));
                *wire_menu_at = menu_at;
            } else if let Some(id) = on_node.filter(|_| on_annotation.is_none()) {
                if !state.selection.contains(&id) {
                    state.select_only(id);
                }
                state.node_menu = Some(id);
                *node_menu_at = menu_at;
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
                *annotation_menu_at = menu_at;
            } else if pin_under(&geoms, pw, hit_w).is_none()
                && node_under(&geoms, pw, lod, &m).is_none()
            {
                // Ratified: empty canvas gets a proper context menu whose
                // first row *is* "Add Node…". Right-click used to open the
                // palette directly, which made the canvas the one surface in
                // the editor where right-click meant something else — and left
                // Paste and Select All with no mouse route at all.
                state.canvas_menu = Some(([pw.x, pw.y], [rc.x, rc.y]));
                *canvas_menu_at = menu_at;
            }
        }
    }

    // A nudge closes when the last arrow key comes up: that is the moment the
    // whole hold becomes one undo entry. Checking *held* state rather than a
    // key-up event means a key released while the window was unfocused (whose
    // key-up went to another window) still closes the transaction.
    if state.nudging() {
        let input = &ui.ctx().input;
        let any_arrow_held = [Key::ArrowUp, Key::ArrowDown, Key::ArrowLeft, Key::ArrowRight]
            .iter()
            .any(|k| input.key_down(*k));
        if !any_arrow_held {
            state.commit_nudge(registry);
        }
    }

    // Rule 5 — Ctrl over empty canvas is the slash-cut's precursor. A
    // crosshair is the cheapest legible hint and the OS already owns drawing
    // it, so it costs nothing and cannot drift out of sync with a glyph.
    if ctrl && !widget_claimed && pointer_world.is_some() && hovered_wire.is_none() {
        ui.ctx_mut().set_cursor_icon(crusty_gui::context::CursorIcon::Crosshair);
    }

    // Canvas-context shortcuts. Pointer over the canvas, and never while an
    // inline editor, a modal surface or a text field owns the keyboard. Chords
    // come from the keymap: `Canvas` shadows `GraphTab` and `Global`, which is
    // what lets a bare `C` group here without disturbing `Ctrl+C` elsewhere.
    // `?` toggles the cheat sheet. Deliberately *not* a keymap action: the
    // sheet documents the keymap, so binding it through the keymap would let a
    // user rebind away the only in-app reference for getting it back.
    if pointer_world.is_some() && !ui.ctx().text_focused() {
        let input = &ui.ctx().input;
        let q = input.key_pressed(Key::Char('?'))
            || (input.modifiers == Modifiers::SHIFT && input.key_pressed(Key::Char('/')));
        if q {
            state.cheat_sheet = !state.cheat_sheet;
        }
    }

    if pointer_world.is_some() && !overlay_has_focus_excl(ui, state, own_modal) {
        for action in keymap.dispatch(&ui.ctx().input, Context::Canvas) {
            match action {
                Action::GROUP => state.add_group_around_selection(registry),
                Action::COMMENT => {
                    if let Some(pw) = pointer_world {
                        state.add_comment([pw.x, pw.y], registry);
                    }
                }
                Action::COLLAPSE => *collapse_request = true,
                Action::BOOKMARK_STORE => {
                    let slot = state.store_bookmark();
                    println!("graph: view bookmarked in slot {slot} (Shift+{slot} to recall)");
                }
                Action::BOOKMARK_RECALL_1
                | Action::BOOKMARK_RECALL_2
                | Action::BOOKMARK_RECALL_3
                | Action::BOOKMARK_RECALL_4
                | Action::BOOKMARK_RECALL_5 => {
                    let slot = bookmark_slot(action);
                    if slot <= BOOKMARK_SLOTS && state.recall_bookmark(slot) {
                        *frame_request = Some(state.view);
                    }
                }
                Action::PURGE_UNUSED => {
                    let unused = state.unused_nodes(registry);
                    state.purge_confirm = (!unused.is_empty()).then_some(unused);
                }
                // Align & distribute is a selection operation, so it lives in
                // the node menu; the key opens that rather than duplicating
                // the rows somewhere else.
                Action::ALIGN_STRIP => {
                    if state.selection.len() >= 2 {
                        state.node_menu =
                            state.primary.or_else(|| state.selection.iter().copied().next());
                        *node_menu_at = ui.ctx().input.pointer_pos;
                    }
                }
                Action::ALIGN_TOP => {
                    let rects = selected_rects(state, &geoms);
                    state.align_nodes(&rects, AlignMode::Top, registry);
                }
                Action::ALIGN_LEFT => {
                    let rects = selected_rects(state, &geoms);
                    state.align_nodes(&rects, AlignMode::Left, registry);
                }
                Action::ALIGN_BOTTOM => {
                    let rects = selected_rects(state, &geoms);
                    state.align_nodes(&rects, AlignMode::Bottom, registry);
                }
                Action::ALIGN_RIGHT => {
                    let rects = selected_rects(state, &geoms);
                    state.align_nodes(&rects, AlignMode::Right, registry);
                }
                Action::ALIGN_CENTER_H => {
                    let rects = selected_rects(state, &geoms);
                    state.align_nodes(&rects, AlignMode::CenterHorizontally, registry);
                }
                Action::ALIGN_CENTER_V => {
                    let rects = selected_rects(state, &geoms);
                    state.align_nodes(&rects, AlignMode::CenterVertically, registry);
                }
                Action::AUTO_LAYOUT => *layout_request = true,
                Action::COMPILE => state.compile(registry),
                Action::TOGGLE_BREAKPOINT => state.toggle_breakpoint(),
                Action::CLEAR_BREAKPOINTS => state.clear_breakpoints(),
                // Debugger transport (GS-4). Gated on there being a paused
                // session: outside one these keys mean nothing, and silence is
                // the right nothing — a toast on every stray F10 would be
                // worse than no answer.
                Action::DEBUG_RESUME | Action::DEBUG_STEP | Action::DEBUG_STOP
                    if exec.is_some_and(|v| v.paused.is_some()) =>
                {
                    state.debug_request = Some(match action {
                        Action::DEBUG_STEP => DebugRequest::Step,
                        Action::DEBUG_STOP => DebugRequest::Stop,
                        _ => DebugRequest::Resume,
                    });
                }
                Action::RENAME => {
                    state.begin_rename();
                }
                Action::CHILD_GRAPH => {
                    // Rules first (ticket 05): descending a selected
                    // transition opens its rule peek — no file involved.
                    if let Some(t) = state.rule_descend_target() {
                        state.open_rule_scope(t, registry);
                    } else if let Some(path) = state.descend_target() {
                        *open_subgraph = Some(state.open_request(path));
                    }
                }
                Action::PARENT_GRAPH => {
                    // Inside a rule, "up" means back to the machine.
                    if state.rule_scope.is_some() {
                        state.close_rule_scope(registry);
                    } else if let Some(path) = state.ascend_target() {
                        // The popped remainder is the parent's own chain.
                        *open_subgraph = Some(GraphOpenRequest {
                            path,
                            back: state.nav_back.clone(),
                        });
                    }
                }
                // Arrow nudge. Each repeat extends the open transaction
                // instead of recording its own entry — see `nudge_selection`.
                Action::NUDGE_UP => state.nudge_selection([0.0, -NUDGE_COARSE]),
                Action::NUDGE_DOWN => state.nudge_selection([0.0, NUDGE_COARSE]),
                Action::NUDGE_LEFT => state.nudge_selection([-NUDGE_COARSE, 0.0]),
                Action::NUDGE_RIGHT => state.nudge_selection([NUDGE_COARSE, 0.0]),
                Action::NUDGE_UP_FINE => state.nudge_selection([0.0, -NUDGE_FINE]),
                Action::NUDGE_DOWN_FINE => state.nudge_selection([0.0, NUDGE_FINE]),
                Action::NUDGE_LEFT_FINE => state.nudge_selection([-NUDGE_FINE, 0.0]),
                Action::NUDGE_RIGHT_FINE => state.nudge_selection([NUDGE_FINE, 0.0]),
                // The discoverable route to the palette is right-click; this
                // is the one you keep.
                Action::ADD_NODE_PALETTE => {
                    if state.palette.is_none() {
                        if let (Some(pw), Some(sp)) =
                            (pointer_world, ui.ctx().input.pointer_pos)
                        {
                            open_palette(state, [pw.x, pw.y], [sp.x, sp.y], None);
                        }
                    }
                }
                Action::FIND => {
                    state.find = Some(FindState { first_frame: true, ..Default::default() });
                }
                // `Canvas` walks out to `GraphTab`, so the strip toggles from
                // the canvas too — which is where the pointer is when an
                // author decides they want the list back.
                Action::TOGGLE_VARIABLES => state.vars.open = !state.vars.open,
                // Same cursor the count chip drives, so the two never disagree.
                Action::NEXT_ERROR => *cycle_error_request = Some(true),
                Action::PREV_ERROR => *cycle_error_request = Some(false),
                _ => {}
            }
        }
    }

    geoms
}

/// Slot number behind a `BOOKMARK_RECALL_n` action.
fn bookmark_slot(action: Action) -> usize {
    match action {
        Action::BOOKMARK_RECALL_1 => 1,
        Action::BOOKMARK_RECALL_2 => 2,
        Action::BOOKMARK_RECALL_3 => 3,
        Action::BOOKMARK_RECALL_4 => 4,
        Action::BOOKMARK_RECALL_5 => 5,
        _ => 0,
    }
}


/// The `?` cheat sheet: every binding, two columns, grouped by context.
///
/// An E3 surface on the modal stack, so Rule 1 dismisses it exactly like a
/// context menu — a press outside closes it *and* is consumed, and Escape pops
/// it without also aborting whatever was underneath.
fn cheat_sheet(ui: &mut Ui, rect: Rect, state: &mut GraphEditorState, keymap: &Keymap) {
    if !state.cheat_sheet {
        return;
    }
    let id = cheat_sheet_modal_id();
    if ui.ctx().modal_dismissed(id).is_some() {
        state.cheat_sheet = false;
        ui.ctx_mut().modal_dismiss(id);
        return;
    }

    let st = ui.style();
    let pal = st.palette;
    let rows = keymap.rows();
    let bound: Vec<_> = rows.iter().filter(|r| !r.chords.is_empty()).collect();

    // Two columns, split at the halfway mark by row count so both read as a
    // single continuous list rather than a table with a hole in it.
    let line_h = st.fonts.body * 1.55;
    let head_h = st.fonts.small * 1.9;
    let per_col = bound.len().div_ceil(2) + 6;
    let w = (rect.width() * 0.8).clamp(520.0, 900.0);
    let h = (per_col as f32 * line_h + head_h * 4.0 + 56.0).min(rect.height() * 0.9);
    let panel = Rect::from_min_size(
        Pos2::new(rect.center().x - w * 0.5, rect.center().y - h * 0.5),
        Vec2::new(w, h),
    );

    {
        let mut p = ui.overlay_painter();
        // A scrim that still shows the graph: you are reading the sheet
        // *about* what is behind it.
        p.rect_filled_translucent(rect, Rounding::ZERO, Color::BLACK.with_alpha(pal.scrim_alpha));
        p.rect_filled(panel, st.rounding.panel, pal.elevated);
        p.rect_stroke(panel, st.rounding.panel, st.metrics.border, pal.stroke_strong);
        p.text(
            Pos2::new(panel.min.x + 20.0, panel.min.y + 16.0),
            "Keyboard Shortcuts",
            st.fonts.body * 1.15,
            pal.text,
            None,
        );
        p.text_family(
            Pos2::new(panel.max.x - 92.0, panel.min.y + 18.0),
            "Esc or ?",
            st.fonts.small,
            pal.text_disabled,
            None,
            FontFamily::Mono,
        );

        let col_w = (panel.width() - 56.0) * 0.5;
        let mut x = panel.min.x + 20.0;
        let mut y = panel.min.y + 52.0;
        let mut col = 0usize;
        let mut last_ctx: Option<Context> = None;

        // Fixed actions are the input model's, not the keymap's, so they get
        // their own heading rather than sitting among rebindable rows.
        let mut ordered: Vec<_> = bound.clone();
        ordered.sort_by_key(|r| {
            (
                r.action.status() == ActionStatus::Fixed,
                Context::ALL
                    .iter()
                    .position(|c| *c == r.action.context())
                    .unwrap_or(usize::MAX),
            )
        });

        for (placed, row) in ordered.into_iter().enumerate() {
            let fixed = row.action.status() == ActionStatus::Fixed;
            let ctx = row.action.context();
            let heading = if fixed { None } else { Some(ctx) };
            if heading != last_ctx || (fixed && last_ctx.is_some()) {
                if placed > 0 && placed >= per_col && col == 0 {
                    col = 1;
                    x = panel.min.x + 20.0 + col_w + 16.0;
                    y = panel.min.y + 52.0;
                }
                last_ctx = heading;
                let label = if fixed { "Input model".to_string() } else { ctx.label().to_string() };
                y += 6.0;
                p.text_family(
                    Pos2::new(x, y),
                    &label.to_uppercase(),
                    st.fonts.small,
                    pal.text_secondary,
                    None,
                    FontFamily::Mono,
                );
                y += head_h;
            }
            // Unimplemented rows are listed and dimmed: the key exists, its
            // behaviour does not yet.
            let live = row.action.status() == ActionStatus::Live;
            let name_col = if live { pal.text } else { pal.text_disabled };
            let chord_col = if live { pal.text_mono } else { pal.text_disabled };
            p.text_family(
                Pos2::new(x, y),
                &row.chords[0].label(),
                st.fonts.body,
                chord_col,
                Some(col_w * 0.42),
                FontFamily::Mono,
            );
            p.text(
                Pos2::new(x + col_w * 0.44, y),
                row.action.name(),
                st.fonts.body,
                name_col,
                Some(col_w * 0.54),
            );
            y += line_h;
            if y > panel.max.y - line_h && col == 0 {
                col = 1;
                x = panel.min.x + 20.0 + col_w + 16.0;
                y = panel.min.y + 52.0;
                last_ctx = None;
            }
        }
    }

    ui.ctx_mut().modal_push(id, panel);
}

fn cheat_sheet_modal_id() -> crusty_gui::id::Id {
    crusty_gui::id::Id::ROOT.with("graph_cheat_sheet")
}

/// Does this node's drawn box reach the viewport?
fn visible(g: &NodeGeom, lod: ZoomLod, m: &GraphMetrics, vis: Rect) -> bool {
    overlaps(g.body_rect(lod, m), vis)
}

/// Rect overlap that treats a shared edge as visible (`Rect::intersect`
/// returns a degenerate rect there, and clipping those out makes a node
/// scrolling into view pop rather than slide).
fn overlaps(a: Rect, b: Rect) -> bool {
    let i = a.intersect(b);
    i.width() >= 0.0 && i.height() >= 0.0
}

/// Layout spacing from the theme, so a denser UI lays out denser too.
fn layout_spacing(st: &Style) -> super::graph_layout::LayoutSpacing {
    let s = (st.metrics.row_height / BASE_ROW_H).max(0.1);
    super::graph_layout::LayoutSpacing {
        column_gap: BASE_LAYOUT_COL_GAP * s,
        row_gap: BASE_LAYOUT_ROW_GAP * s,
    }
}

/// World rects of every node, for auto-layout.
fn all_rects(geoms: &[NodeGeom]) -> Vec<(u64, [f32; 4])> {
    geoms
        .iter()
        .map(|g| {
            (
                g.id,
                [g.rect.min.x, g.rect.min.y, g.rect.width(), g.rect.height()],
            )
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Rule scope drawing (Task 41 ticket 05): peek overlay, promoted scope,
// breadcrumb, and the dim-with-lit-holes scrim. Mockup 3b is the reference.
// ---------------------------------------------------------------------------

fn rule_peek_modal_id() -> Id {
    Id::ROOT.with("graph_rule_peek")
}

/// Source and target of a transition, resolved through its pins.
fn rule_endpoints(state: &GraphEditorState, owner: u64) -> (Option<u64>, Option<u64>) {
    use crate::engine::animation::graph::plan::{TRANSITION_FROM_PIN, TRANSITION_TO_PIN};
    let from = state
        .doc
        .edges
        .iter()
        .find(|e| e.to_node == owner && e.to_pin == TRANSITION_FROM_PIN)
        .map(|e| e.from_node);
    let to = state
        .doc
        .edges
        .iter()
        .find(|e| e.from_node == owner && e.from_pin == TRANSITION_TO_PIN)
        .map(|e| e.to_node);
    (from, to)
}

/// A state's display name for the scope labels — the same fallbacks the
/// compiler's messages use, plus "?" for an unwired endpoint.
fn rule_state_name(state: &GraphEditorState, id: Option<u64>) -> String {
    use crate::engine::animation::graph::plan::{ANIM_ANY_STATE_TYPE_ID, ANIM_STATE_TYPE_ID};
    let Some(n) = id.and_then(|id| state.doc.node(id)) else {
        return "?".to_string();
    };
    if let Some(t) = &n.title {
        return t.clone();
    }
    match n.type_id.as_str() {
        ANIM_ANY_STATE_TYPE_ID => "Any State".to_string(),
        ANIM_STATE_TYPE_ID => format!("State {}", n.id),
        _ => format!("Node {}", n.id),
    }
}

/// "rule: Idle → Locomotion" — the breadcrumb's non-file scope target.
fn rule_scope_label(state: &GraphEditorState, owner: u64) -> String {
    let (from, to) = rule_endpoints(state, owner);
    format!(
        "rule: {} \u{2192} {}",
        rule_state_name(state, from),
        rule_state_name(state, to)
    )
}

/// "0.20s" / "0.20s · P1" — sourced from the same chip resolver the edge
/// draws with, so the header and the chip never disagree.
fn rule_duration_tag(state: &GraphEditorState, owner: u64) -> String {
    let chip = super::graph_anim_chip::transition_chip(&state.doc, owner);
    let mut t = format!("{:.2}s", chip.duration.max(0.0));
    if chip.priority != 0 {
        t.push_str(&format!(" \u{b7} P{}", chip.priority));
    }
    t
}

/// The smallest the peek can be resized to — the header plus a usable slice
/// of rule canvas.
fn peek_min_size(s: f32) -> Vec2 {
    Vec2::new(280.0 * s, 170.0 * s)
}

/// A user-placed peek rect: canvas-relative min offset + size, clamped so
/// the panel stays fully on the canvas at a workable size. Pure, tested.
fn peek_user_rect(canvas: Rect, off: [f32; 2], size: [f32; 2], s: f32) -> Rect {
    let min_sz = peek_min_size(s);
    let w = size[0].max(min_sz.x).min(canvas.width().max(min_sz.x));
    let h = size[1].max(min_sz.y).min(canvas.height().max(min_sz.y));
    let x = (canvas.min.x + off[0]).clamp(canvas.min.x, (canvas.max.x - w).max(canvas.min.x));
    let y = (canvas.min.y + off[1]).clamp(canvas.min.y, (canvas.max.y - h).max(canvas.min.y));
    Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, h))
}

/// Where the peek panel sits: where the user last put it (header drag /
/// border resize — session-remembered per tab), else anchored under the
/// transition's chip when it is on screen (the rule opens where the eye
/// already is), clamped into the canvas, sized to leave the machine visible
/// around it.
fn rule_peek_rect(
    ui: &Ui,
    canvas: Rect,
    state: &GraphEditorState,
    geoms: &[NodeGeom],
) -> Rect {
    let st = ui.style();
    let s = (st.metrics.row_height / BASE_ROW_H).max(0.1);
    if let Some((off, size)) = state.peek_panel {
        return peek_user_rect(canvas, off, size, s);
    }
    let margin = 12.0 * s;
    let w = (canvas.width() * 0.62)
        .clamp(340.0 * s, 780.0 * s)
        .min((canvas.width() - margin * 2.0).max(80.0));
    let h = (canvas.height() * 0.58)
        .clamp(260.0 * s, 500.0 * s)
        .min((canvas.height() - margin * 2.0).max(60.0));
    let v = state.view;
    let anchor = state
        .rule_scope
        .as_ref()
        .and_then(|sc| geoms.iter().find(|g| g.id == sc.owner))
        .map(|g| {
            Pos2::new(
                canvas.min.x + (g.rect.center().x - v.pan.x) * v.zoom,
                canvas.min.y + (g.rect.max.y - v.pan.y) * v.zoom,
            )
        })
        .unwrap_or_else(|| canvas.center());
    let mut min = Pos2::new(anchor.x - w * 0.5, anchor.y + 14.0 * s);
    min.x = min
        .x
        .clamp(canvas.min.x + margin, (canvas.max.x - margin - w).max(canvas.min.x + margin));
    if min.y + h > canvas.max.y - margin {
        min.y = (anchor.y - 28.0 * s - h).max(canvas.min.y + margin);
    }
    min.y = min
        .y
        .clamp(canvas.min.y + margin, (canvas.max.y - margin - h).max(canvas.min.y + margin));
    Rect::from_min_size(min, Vec2::new(w, h))
}

/// `outer` minus the union of `holes`, as horizontal bands: cut at every hole
/// edge, then walk each band's uncovered x-intervals. Pure, and tested.
fn subtract_rects(outer: Rect, holes: &[Rect]) -> Vec<Rect> {
    let mut ys: Vec<f32> = vec![outer.min.y, outer.max.y];
    for h in holes {
        ys.push(h.min.y.clamp(outer.min.y, outer.max.y));
        ys.push(h.max.y.clamp(outer.min.y, outer.max.y));
    }
    ys.sort_by(f32::total_cmp);
    ys.dedup();
    let mut out = Vec::new();
    for w in ys.windows(2) {
        let (y0, y1) = (w[0], w[1]);
        if y1 <= y0 {
            continue;
        }
        let mid = (y0 + y1) * 0.5;
        let mut spans: Vec<(f32, f32)> = holes
            .iter()
            .filter(|h| h.min.y <= mid && mid < h.max.y)
            .map(|h| (h.min.x.max(outer.min.x), h.max.x.min(outer.max.x)))
            .filter(|(a, b)| b > a)
            .collect();
        spans.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut x = outer.min.x;
        for (hx0, hx1) in spans {
            if hx0 > x {
                out.push(Rect::from_min_max(Pos2::new(x, y0), Pos2::new(hx0, y1)));
            }
            x = x.max(hx1);
        }
        if outer.max.x > x {
            out.push(Rect::from_min_max(Pos2::new(x, y0), Pos2::new(outer.max.x, y1)));
        }
    }
    out
}

/// Dim the machine under an open peek, leaving the transition and its two
/// states lit — holes in the scrim, not repaints, so the lit trio is exactly
/// what the canvas already drew (mockup 3b: "source + target states stay
/// lit; rest dims").
fn rule_dim_scrim(ui: &mut Ui, canvas: Rect, state: &GraphEditorState, geoms: &[NodeGeom]) {
    let Some(scope) = &state.rule_scope else { return };
    let (from, to) = rule_endpoints(state, scope.owner);
    let v = state.view;
    let mut lit: Vec<Rect> = Vec::new();
    for g in geoms {
        if g.id == scope.owner || Some(g.id) == from || Some(g.id) == to {
            let r = Rect::from_min_max(
                Pos2::new(
                    canvas.min.x + (g.rect.min.x - v.pan.x) * v.zoom,
                    canvas.min.y + (g.rect.min.y - v.pan.y) * v.zoom,
                ),
                Pos2::new(
                    canvas.min.x + (g.rect.max.x - v.pan.x) * v.zoom,
                    canvas.min.y + (g.rect.max.y - v.pan.y) * v.zoom,
                ),
            )
            .intersect(canvas);
            if r.width() > 0.0 && r.height() > 0.0 {
                lit.push(r);
            }
        }
    }
    let st = ui.style();
    let color = Color::BLACK.with_alpha(st.palette.scrim_alpha);
    let mut p = ui.painter();
    for band in subtract_rects(canvas, &lit) {
        p.rect_filled_translucent(band, Rounding::ZERO, color);
    }
}

/// Mockup 2d's dim-on-select (Task 41 rework): exactly one transition
/// selected (no rule peek open) dims the rest of the machine behind the
/// scrim while the transition's card, its source and target states and its
/// own edge stay lit — the rule-peek scrim's idiom, applied at selection.
fn anim_select_dim(
    ui: &mut Ui,
    canvas: Rect,
    state: &GraphEditorState,
    registry: &NodeRegistry,
    geoms: &[NodeGeom],
) {
    use crate::engine::animation::graph::plan::ANIM_TRANSITION_TYPE_ID;
    if !state.domain.is_animation() || state.rule_scope.is_some() {
        return;
    }
    if state.selection.len() != 1 {
        return;
    }
    let Some(&owner) = state.selection.iter().next() else {
        return;
    };
    if !state
        .doc
        .node(owner)
        .is_some_and(|n| n.type_id == ANIM_TRANSITION_TYPE_ID)
    {
        return;
    }
    let (from, to) = rule_endpoints(state, owner);
    let v = state.view;
    let screen = |q: Pos2| {
        Pos2::new(
            canvas.min.x + (q.x - v.pan.x) * v.zoom,
            canvas.min.y + (q.y - v.pan.y) * v.zoom,
        )
    };
    let mut lit: Vec<Rect> = Vec::new();
    for g in geoms {
        if g.id == owner || Some(g.id) == from || Some(g.id) == to {
            let r = Rect::from_min_max(screen(g.rect.min), screen(g.rect.max))
                .intersect(canvas);
            if r.width() > 0.0 && r.height() > 0.0 {
                lit.push(r);
            }
        }
    }
    let st = ui.style();
    let scrim = Color::BLACK.with_alpha(st.palette.scrim_alpha);
    let flow = PinType::Domain(crate::engine::animation::graph::ANIM_FLOW_DOMAIN.to_string());
    let bright = wire_color(Some(registry), &flow);
    let mut p = ui.painter();
    for band in subtract_rects(canvas, &lit) {
        p.rect_filled_translucent(band, Rounding::ZERO, scrim);
    }
    // The selected transition's own edge stays lit: its segments re-draw
    // above the scrim, arrowhead included — but they stop at the border of
    // the unfolded card itself (its halves meet at the card's center), so
    // nothing ever crosses the card being edited.
    let rects: BTreeMap<u64, [f32; 4]> = geoms
        .iter()
        .map(|g| (g.id, [g.rect.min.x, g.rect.min.y, g.rect.max.x, g.rect.max.y]))
        .collect();
    let layout = super::graph_anim_edge::anim_flow_layout(&state.doc, &rects);
    let card = rects.get(&owner).copied();
    for (i, e) in state.doc.edges.iter().enumerate() {
        if e.to_node != owner && e.from_node != owner {
            continue;
        }
        if let Some(seg) = layout.segs.get(&i) {
            let pieces = match card {
                Some(r) => super::graph_anim_edge::seg_outside(seg.a, seg.b, r),
                None => vec![(seg.a, seg.b)],
            };
            for (pa, pb) in pieces {
                let a = screen(Pos2::new(pa[0], pa[1]));
                let b = screen(Pos2::new(pb[0], pb[1]));
                p.line_segment(a, b, WIRE_DATA_SELECTED, bright);
                if seg.arrow && pb == seg.b {
                    draw_arrow_head(&mut p, b, a, (ARROW_L * v.zoom).clamp(5.0, 16.0), bright);
                }
            }
        }
    }
}

/// What the breadcrumb band's one click this frame asked for.
enum BreadcrumbClick {
    None,
    /// The current file's crumb, while a rule scope is open — climb out.
    CloseRule,
    /// An ancestor crumb (index into `nav_back`) — navigate back to it.
    Ancestor(usize),
}

/// The tab's breadcrumb: the file chain the tab was descended into (ticket
/// 09 — every ancestor crumb is a link back), the current file, and — while
/// a rule scope is open (ticket 05) — the non-file scope target:
/// `character.animgraph ▸ locomotion.animgraph ▸ rule: Idle → Walk · 0.20s`.
/// Drawn when either has something to say; the two scopes read as one chain,
/// which is exactly the spec's "breadcrumb distinguishes the two".
fn breadcrumb_band(ui: &mut Ui, state: &GraphEditorState) -> BreadcrumbClick {
    let chain: &[String] = if state.domain.is_animation() { &state.nav_back } else { &[] };
    let rule = state.rule_scope.as_ref().map(|s| s.owner);
    if chain.is_empty() && rule.is_none() {
        return BreadcrumbClick::None;
    }
    let st = ui.style();
    let pad = st.spacing.padding;
    let h = st.metrics.control_height;
    let font = st.fonts.body;
    let rect = ui.allocate(Vec2::new(ui.available_size().x, h));
    let file_name =
        |path: &str| path.rsplit('/').next().unwrap_or(path).to_string();
    let sep = " \u{25b8} ";
    let sep_w = ui.painter().measure_text(sep, font, None).x;
    let ty = rect.center().y - font * 0.62;

    // Interact with every link crumb first (ancestors, plus the file crumb
    // when a rule scope makes it a way back out), then paint — the band's
    // layout is one left-to-right cursor either way.
    let mut clicked = BreadcrumbClick::None;
    let mut p = ui.painter();
    p.rect_filled(rect, Rounding::ZERO, st.palette.header);
    p.rect_filled(
        Rect::from_min_max(Pos2::new(rect.min.x, rect.max.y - st.metrics.border), rect.max),
        Rounding::ZERO,
        st.palette.stroke_strong,
    );
    drop(p);

    let mut x = rect.min.x + pad;
    for (i, ancestor) in chain.iter().enumerate() {
        let text = file_name(ancestor);
        let w = ui.painter().measure_text(&text, font, None).x;
        let crumb = Rect::from_min_size(Pos2::new(x, rect.min.y), Vec2::new(w, h));
        let id = ui.alloc_id(("graph_breadcrumb", i));
        let resp = ui.interact(id, crumb);
        if resp.clicked {
            clicked = BreadcrumbClick::Ancestor(i);
        }
        let mut p = ui.painter();
        p.text(
            Pos2::new(x, ty),
            &text,
            font,
            if resp.hovered { st.palette.accent_active } else { st.palette.text_secondary },
            None,
        );
        x += w;
        p.text(Pos2::new(x, ty), sep, font, st.palette.text_disabled, None);
        x += sep_w;
    }

    let file = file_name(&state.path);
    let file_w = ui.painter().measure_text(&file, font, None).x;
    let crumb = Rect::from_min_size(Pos2::new(x, rect.min.y), Vec2::new(file_w, h));
    let file_hovered = if rule.is_some() {
        let id = ui.alloc_id("rule_breadcrumb_file");
        let resp = ui.interact(id, crumb);
        if resp.clicked {
            clicked = BreadcrumbClick::CloseRule;
        }
        resp.hovered
    } else {
        false
    };
    let mut p = ui.painter();
    p.text(
        Pos2::new(x, ty),
        &file,
        font,
        if file_hovered { st.palette.accent_active } else { st.palette.text },
        None,
    );
    x += file_w;

    if let Some(owner) = rule {
        let scope_text = rule_scope_label(state, owner);
        let tag = rule_duration_tag(state, owner);
        p.text(Pos2::new(x, ty), sep, font, st.palette.text_disabled, None);
        x += sep_w;
        p.text(Pos2::new(x, ty), &scope_text, font, st.palette.text, None);
        x += p.measure_text(&scope_text, font, None).x + pad;
        p.text_family(
            Pos2::new(x, rect.center().y - st.fonts.small * 0.62),
            &format!("\u{b7} {tag}"),
            st.fonts.small,
            st.palette.text_disabled,
            None,
            FontFamily::Mono,
        );
    }
    clicked
}

/// What one child-canvas pass hands back to its host surface.
struct RuleCanvasOut {
    rect: Rect,
    geoms: Vec<NodeGeom>,
    /// `PageUp` fired over the child canvas — the caller climbs out.
    ascend: bool,
}

/// One frame of the rule projection's canvas: the same `draw_and_interact`
/// the machine runs, over the scope's child state and the rule registry —
/// which *is* the placement gate — plus the child's own transient surfaces.
/// Ends by draining the child's recorded edits onto the parent history.
#[allow(clippy::too_many_arguments)]
fn rule_canvas_pass(
    ui: &mut Ui,
    body: Rect,
    state: &mut GraphEditorState,
    registry: &NodeRegistry,
    clipboard: &mut Option<GraphFragment>,
    resolver: &DocResolvers<'_>,
    keymap: &Keymap,
    selection_outline: Color,
    wire_prefs: &WirePrefs,
    zoom_min: f32,
    zoom_max: f32,
    own_modal: Option<Id>,
) -> Option<RuleCanvasOut> {
    let Some(mut scope) = state.rule_scope.take() else { return None };
    let rreg = rule_scope_registry();
    let owner = scope.owner;
    let child = &mut *scope.child;

    let mut view = child.view;
    let mut annotation_menu_at: Option<Pos2> = None;
    let mut wire_menu_at: Option<Pos2> = None;
    let mut node_menu_at: Option<Pos2> = None;
    let mut canvas_menu_at: Option<Pos2> = None;
    let mut collapse_request = false;
    let mut layout_request = false;
    let mut cycle_error_request: Option<bool> = None;
    let mut frame_request: Option<CanvasView> = None;
    // Rules reference no files; nothing ever lands here.
    let mut open_subgraph = None;
    let (out, _) = ui.run_at(
        body,
        Direction::TopDown,
        Id::ROOT.with(("rule_canvas", owner)),
        UiOptions { padding: Vec2::ZERO, spacing: 0.0 },
        |ui| {
            Canvas::new()
                .size(body.size())
                .zoom_range(zoom_min, zoom_max)
                .drag_threshold(crusty_gui::input::drag_threshold(
                    (ui.style().metrics.row_height / BASE_ROW_H).max(0.1),
                ))
                .show(ui, &mut view, |ui, cscope| {
                    draw_and_interact(
                        ui,
                        cscope,
                        child,
                        rreg,
                        resolver,
                        &mut annotation_menu_at,
                        &mut wire_menu_at,
                        &mut node_menu_at,
                        &mut canvas_menu_at,
                        &mut collapse_request,
                        &mut layout_request,
                        &mut cycle_error_request,
                        &mut open_subgraph,
                        selection_outline,
                        wire_prefs,
                        zoom_min,
                        zoom_max,
                        &mut frame_request,
                        keymap,
                        None,
                        own_modal,
                    )
                })
        },
    );
    let _ = (annotation_menu_at, collapse_request, open_subgraph);
    if let Some(v) = frame_request {
        view = v;
    }
    child.view = view;

    // A parameter dropped on the rule canvas places a Get outright — a rule
    // only reads, so there is no Get/Set question to ask.
    if let Some(p) = ui.dnd_drop_target::<VarDragPayload>(out.rect) {
        if let Some(sp) = ui.ctx().input.pointer_pos {
            let v = child.view;
            let world = [
                v.pan.x + (sp.x - out.rect.min.x) / v.zoom,
                v.pan.y + (sp.y - out.rect.min.y) / v.zoom,
            ];
            let id = child.add_variable_node(&p.slug, false, world, rreg);
            child.select_only(id);
        }
    }

    palette_popover(ui, child, rreg, &[]);
    find_overlay(ui, out.rect, child, &out.inner, zoom_min, zoom_max, rreg);
    wire_menu(ui, child, rreg, wire_menu_at);
    node_menu(ui, child, rreg, keymap, &out.inner, node_menu_at);
    if let Some((world, screen)) =
        canvas_menu(ui, child, rreg, keymap, &out.inner, clipboard, canvas_menu_at)
    {
        open_palette(child, world, screen, None);
    }
    purge_confirm(ui, out.rect, child, rreg);
    var_confirm_dialog(ui, out.rect, child, rreg);

    // F8 inside the scope cycles the rule's own anchored errors.
    if let Some(forward) = cycle_error_request {
        let errors = ErrorIndex::build(&child.errors, &child.ref_errors, &child.domain_errors);
        if !forward {
            let n = errors
                .ordered
                .iter()
                .filter(|e| e.anchor() != ErrorAnchor::Document)
                .count();
            if n > 0 {
                child.error_cursor = (child.error_cursor + n.saturating_sub(2)) % n;
            }
        }
        let mut req = None;
        cycle_error(
            child,
            &errors,
            &out.inner,
            out.rect.size(),
            zoom_min,
            zoom_max,
            &mut req,
            rreg,
        );
        if let Some(v) = req {
            child.view = v;
        }
    }
    if layout_request {
        let rects = all_rects(&out.inner);
        let sp = layout_spacing(&ui.style());
        child.auto_layout(&rects, sp, rreg);
    }
    draw_toasts(ui, out.rect, child);

    // PageUp over the child canvas climbs out — the child cannot reach the
    // parent, so the ascend is answered here for the caller to take.
    let ascend = out.hovered
        && !overlay_has_focus_excl(ui, child, own_modal)
        && keymap
            .dispatch(&ui.ctx().input, Context::Canvas)
            .contains(&Action::PARENT_GRAPH);

    state.rule_scope = Some(scope);
    state.drain_rule_scope(registry);
    Some(RuleCanvasOut { rect: out.rect, geoms: out.inner, ascend })
}

/// Which panel borders a point inside the grip zones would resize — corners
/// widen the diagonal grab. The float window's border language (6px edges,
/// 14px corners), kept *inside* the panel because a press outside the modal
/// rect dismisses the peek. Pure, tested.
fn peek_resize_zone(panel: Rect, pt: Pos2, s: f32) -> Option<(bool, bool, bool, bool)> {
    if !panel.contains(pt) {
        return None;
    }
    let grip = 6.0 * s;
    let corner = 14.0 * s;
    let l = pt.x < panel.min.x + grip;
    let r = pt.x > panel.max.x - grip;
    let t = pt.y < panel.min.y + grip;
    let b = pt.y > panel.max.y - grip;
    if !(l || r || t || b) {
        return None;
    }
    let cl = pt.x < panel.min.x + corner;
    let cr = pt.x > panel.max.x - corner;
    let ct = pt.y < panel.min.y + corner;
    let cb = pt.y > panel.max.y - corner;
    Some((
        l || ((t || b) && cl),
        r || ((t || b) && cr),
        t || ((l || r) && ct),
        b || ((l || r) && cb),
    ))
}

/// The peek overlay (mockup 3b): a modal panel over the dimmed machine —
/// header ("RULE · IDLE → LOCOMOTION", duration tag, ⤢ promote, ✕ close),
/// body a fully editable rule canvas. Esc, an outside press or a wheel
/// outside dismiss it, per the modal stack's standing rules.
///
/// The panel is the user's window onto the rule: its header drags it, its
/// borders resize it (both session-remembered per tab via `peek_panel`), and
/// releasing a header drag on the band at the canvas top lands on the
/// existing ⤢ promote path — deliberately *not* a second document tab
/// (ticket 05's ruling: two copies of one document break the one-history
/// invariant).
#[allow(clippy::too_many_arguments)]
fn rule_peek_overlay(
    ui: &mut Ui,
    canvas: Rect,
    geoms: &[NodeGeom],
    state: &mut GraphEditorState,
    registry: &NodeRegistry,
    clipboard: &mut Option<GraphFragment>,
    resolver: &DocResolvers<'_>,
    keymap: &Keymap,
    selection_outline: Color,
    wire_prefs: &WirePrefs,
    zoom_min: f32,
    zoom_max: f32,
) {
    let Some(scope) = &state.rule_scope else { return };
    if scope.full {
        return;
    }
    let owner = scope.owner;
    let st = ui.style();
    let s = (st.metrics.row_height / BASE_ROW_H).max(0.1);
    let pad = st.spacing.padding;
    let header_h = st.metrics.control_height + pad * 0.5;
    let mut panel = rule_peek_rect(ui, canvas, state, geoms);

    // ── Move / resize / dock ─────────────────────────────────────────────
    // Continue an in-flight header drag or border resize before anything
    // draws, so the frame shows the placement the pointer is at.
    let pointer = ui.ctx().input.pointer_pos;
    let pressed = ui.ctx().input.pointer_pressed;
    let down = ui.ctx().input.pointer_down;
    let released = ui.ctx().input.pointer_released;
    let dock_band =
        Rect::from_min_max(canvas.min, Pos2::new(canvas.max.x, canvas.min.y + 34.0 * s));
    let mut dock_hot = false;
    let mut promote_drop = false;
    let mut drag_aborted = false;
    if let Some(drag) = state.peek_drag {
        if ui.ctx().input.key_pressed(Key::Escape) || ui.ctx().focus_lost() || pointer.is_none()
        {
            // Rule 3: the drag reverts and records nothing — and the Esc
            // that aborted it must not also dismiss the peek.
            state.peek_panel = drag.prev;
            state.peek_drag = None;
            drag_aborted = true;
        } else if let Some(pt) = pointer {
            let store = |r: Rect| {
                (
                    [r.min.x - canvas.min.x, r.min.y - canvas.min.y],
                    [r.width(), r.height()],
                )
            };
            match drag.kind {
                PeekDragKind::Move { grab } => {
                    dock_hot = pt.y < dock_band.max.y;
                    let min = Pos2::new(pt.x - grab[0], pt.y - grab[1]);
                    state.peek_panel = Some(store(Rect::from_min_size(min, panel.size())));
                }
                PeekDragKind::Resize { left, right, top, bottom } => {
                    let min_sz = peek_min_size(s);
                    let mut r = panel;
                    if left {
                        r.min.x = pt.x.clamp(canvas.min.x, r.max.x - min_sz.x);
                    }
                    if right {
                        r.max.x = pt.x.clamp(r.min.x + min_sz.x, canvas.max.x);
                    }
                    if top {
                        r.min.y = pt.y.clamp(canvas.min.y, r.max.y - min_sz.y);
                    }
                    if bottom {
                        r.max.y = pt.y.clamp(r.min.y + min_sz.y, canvas.max.y);
                    }
                    state.peek_panel = Some(store(r));
                }
            }
            if released || !down {
                if matches!(drag.kind, PeekDragKind::Move { .. }) && dock_hot {
                    // Dropped on the dock band: this was a promote, not a
                    // move — the placement reverts and ⤢ takes it from here.
                    state.peek_panel = drag.prev;
                    promote_drop = true;
                    dock_hot = false;
                }
                state.peek_drag = None;
            }
            panel = rule_peek_rect(ui, canvas, state, geoms);
        }
    }
    ui.ctx_mut().modal_push(rule_peek_modal_id(), panel);

    // Header controls: measured right-to-left, interacted before painting so
    // hover states land in the same frame.
    let font = st.fonts.small;
    let close_text = "\u{2715} esc";
    let promote_text = "\u{2922} tab";
    let (close_w, promote_w) = {
        let mut p = ui.painter();
        (
            p.measure_text(close_text, font, None).x,
            p.measure_text(promote_text, font, None).x,
        )
    };
    let cy = panel.min.y + header_h * 0.5;
    let close_r = Rect::from_center_size(
        Pos2::new(panel.max.x - pad - close_w * 0.5, cy),
        Vec2::new(close_w + pad * 0.5, header_h * 0.8),
    );
    let promote_r = Rect::from_center_size(
        Pos2::new(close_r.min.x - pad - promote_w * 0.5, cy),
        Vec2::new(promote_w + pad * 0.5, header_h * 0.8),
    );
    let close_id = ui.alloc_id("rule_peek_close");
    let promote_id = ui.alloc_id("rule_peek_promote");
    let close_resp = ui.interact(close_id, close_r);
    let promote_resp = ui.interact(promote_id, promote_r);

    // Starting a grab: border zones first, then the header as the move
    // handle (its buttons excepted). The zones sit inside the panel, so the
    // press can never read as an outside-press dismissing the modal.
    if state.peek_drag.is_none() && pressed {
        if let Some(pt) = pointer
            .filter(|pt| panel.contains(*pt) && !close_r.contains(*pt) && !promote_r.contains(*pt))
        {
            let kind = match peek_resize_zone(panel, pt, s) {
                Some((left, right, top, bottom)) => {
                    Some(PeekDragKind::Resize { left, right, top, bottom })
                }
                None if pt.y < panel.min.y + header_h => Some(PeekDragKind::Move {
                    grab: [pt.x - panel.min.x, pt.y - panel.min.y],
                }),
                None => None,
            };
            if let Some(kind) = kind {
                state.peek_drag = Some(PeekDrag { kind, prev: state.peek_panel });
                // The grab pins today's concrete placement, so a Move keeps
                // this size and a Resize moves only the grabbed edges even
                // when the panel was still on its anchored default.
                state.peek_panel = Some((
                    [panel.min.x - canvas.min.x, panel.min.y - canvas.min.y],
                    [panel.width(), panel.height()],
                ));
            }
        }
    }

    // Cursor language: borders advertise the resize, an active move grabs.
    let zone = match state.peek_drag {
        Some(PeekDrag { kind: PeekDragKind::Resize { left, right, top, bottom }, .. }) => {
            Some((left, right, top, bottom))
        }
        Some(PeekDrag { kind: PeekDragKind::Move { .. }, .. }) => None,
        None => pointer.and_then(|pt| peek_resize_zone(panel, pt, s)),
    };
    if let Some((l, r, t, b)) = zone {
        ui.ctx_mut().set_cursor_icon(match (l, r, t, b) {
            (true, _, true, _) | (_, true, _, true) => CursorIcon::ResizeNwSe,
            (true, _, _, true) | (_, true, true, _) => CursorIcon::ResizeNeSw,
            (true, _, _, _) | (_, true, _, _) => CursorIcon::ResizeEw,
            _ => CursorIcon::ResizeNs,
        });
    } else if matches!(
        state.peek_drag,
        Some(PeekDrag { kind: PeekDragKind::Move { .. }, .. })
    ) {
        ui.ctx_mut().set_cursor_icon(CursorIcon::Grabbing);
    }

    {
        let (from, to) = rule_endpoints(state, owner);
        let title = format!(
            "RULE \u{b7} {} \u{2192} {}",
            rule_state_name(state, from).to_uppercase(),
            rule_state_name(state, to).to_uppercase()
        );
        let tag = rule_duration_tag(state, owner);
        let mut p = ui.painter();
        p.rect_filled(panel, st.rounding.panel, st.palette.elevated);
        p.rect_filled(
            Rect::from_min_max(panel.min, Pos2::new(panel.max.x, panel.min.y + header_h)),
            st.rounding.panel,
            st.palette.header,
        );
        p.rect_stroke(panel, st.rounding.panel, st.metrics.border, st.palette.stroke_strong);
        p.text_family(
            Pos2::new(panel.min.x + pad, cy - font * 0.62),
            &title,
            font,
            st.palette.text,
            None,
            FontFamily::Mono,
        );
        let title_w = p.measure_text(&title, font, None).x;
        p.text_family(
            Pos2::new(panel.min.x + pad * 2.0 + title_w, cy - font * 0.62),
            &format!("\u{b7} {tag}"),
            font,
            st.palette.text_disabled,
            None,
            FontFamily::Mono,
        );
        for (r, text, resp) in [
            (close_r, close_text, &close_resp),
            (promote_r, promote_text, &promote_resp),
        ] {
            if resp.hovered {
                p.rect_filled(r, st.rounding.small, st.palette.selection_fill);
            }
            p.text_family(
                Pos2::new(r.min.x + pad * 0.25, cy - font * 0.62),
                text,
                font,
                if resp.hovered { st.palette.text } else { st.palette.text_disabled },
                None,
                FontFamily::Mono,
            );
        }
    }

    // The body insets by the grip width, so the resize borders own their
    // strip outright — a border press can never double as a canvas gesture.
    let grip = 6.0 * s;
    let body = Rect::from_min_max(
        Pos2::new(panel.min.x + grip, panel.min.y + header_h),
        Pos2::new(panel.max.x - grip, panel.max.y - grip),
    );
    let pass = rule_canvas_pass(
        ui,
        body,
        state,
        registry,
        clipboard,
        resolver,
        keymap,
        selection_outline,
        wire_prefs,
        zoom_min,
        zoom_max,
        Some(rule_peek_modal_id()),
    );

    // The dock band lights while a header drag hovers it — releasing there
    // promotes. Drawn last, above the panel it may overlap.
    if dock_hot {
        let mut p = ui.painter();
        p.rect_filled_translucent(
            dock_band,
            st.rounding.panel,
            st.palette.selection_fill,
        );
        p.rect_stroke(dock_band, st.rounding.panel, st.metrics.border, st.palette.accent_active);
        let label = "\u{2922} drop to open as full canvas";
        let w = p.measure_text(label, st.fonts.small, None).x;
        p.text_family(
            Pos2::new(
                dock_band.center().x - w * 0.5,
                dock_band.center().y - st.fonts.small * 0.62,
            ),
            label,
            st.fonts.small,
            st.palette.text,
            None,
            FontFamily::Mono,
        );
    }

    // Dismissal: the modal stack's standing rules (Esc, press/wheel outside),
    // the ✕, or PageUp over the canvas. An Esc that aborted a peek drag this
    // frame is spent — it must not also close the peek.
    if (ui.ctx().modal_dismissed(rule_peek_modal_id()).is_some() && !drag_aborted)
        || close_resp.clicked
        || pass.as_ref().is_some_and(|p| p.ascend)
    {
        state.close_rule_scope(registry);
        ui.ctx_mut().modal_dismiss(rule_peek_modal_id());
        return;
    }
    if promote_resp.clicked || promote_drop {
        if let Some(sc) = state.rule_scope.as_mut() {
            sc.full = true;
        }
        ui.ctx_mut().modal_dismiss(rule_peek_modal_id());
    }
}

/// The promoted scope (⤢): the rule takes the machine's whole canvas area,
/// with the variables strip rebound to the projection — a Get dragged from it
/// lands in the rule. Esc or the breadcrumb's file crumb climbs out.
#[allow(clippy::too_many_arguments)]
fn rule_scope_full(
    ui: &mut Ui,
    state: &mut GraphEditorState,
    registry: &NodeRegistry,
    clipboard: &mut Option<GraphFragment>,
    resolver: &DocResolvers<'_>,
    keymap: &Keymap,
    selection_outline: Color,
    wire_prefs: &WirePrefs,
    zoom_min: f32,
    zoom_max: f32,
) {
    let strip = {
        let st = ui.style();
        let s = (st.metrics.row_height / BASE_ROW_H).max(0.1);
        let open = state.rule_scope.as_ref().is_some_and(|sc| sc.child.vars.open);
        let w = if open { VARS_W * s } else { VARS_RAIL_W * s };
        let c = ui.cursor();
        let r = Rect::from_min_max(c, Pos2::new(c.x + w, ui.available().max.y));
        ui.set_cursor(Pos2::new(c.x + w, c.y));
        r
    };
    let body = Rect::from_min_max(ui.cursor(), ui.available().max);
    // Whether Escape already had a job this frame — a gesture to abort, a
    // popup to pop — decided before the pass runs, because the pass consumes
    // exactly those.
    let esc_busy = state.rule_scope.as_ref().is_some_and(|sc| {
        sc.child.interaction_in_flight() || overlay_has_focus_excl(ui, &sc.child, None)
    });
    let pass = rule_canvas_pass(
        ui,
        body,
        state,
        registry,
        clipboard,
        resolver,
        keymap,
        selection_outline,
        wire_prefs,
        zoom_min,
        zoom_max,
        None,
    );

    if let Some(mut scope) = state.rule_scope.take() {
        let mut locate = None;
        variables_panel(ui, strip, &mut scope.child, rule_scope_registry(), &mut locate);
        if let (Some(id), Some(pass)) = (locate, &pass) {
            scope.child.select_only(id);
            scope.child.flash = Some((id, std::time::Instant::now()));
            if let Some((mn, mx)) = geoms_bbox(pass.geoms.iter().filter(|g| g.id == id)) {
                let v = frame_view(mn, mx, pass.rect.size(), zoom_min, zoom_max);
                scope.child.view = CanvasView { pan: v.pan, zoom: scope.child.view.zoom };
            }
        }
        state.rule_scope = Some(scope);
        state.drain_rule_scope(registry);
    }

    if pass.is_some_and(|p| p.ascend)
        || (ui.ctx().input.key_pressed(Key::Escape) && !esc_busy)
    {
        state.close_rule_scope(registry);
    }
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

/// Transient canvas messages, stacked bottom-centre and fading out. Every
/// gesture that changes something off-screen — a break, a cut, a paste that
/// dropped links — says what it did here rather than in a log the user is not
/// reading.
fn draw_toasts(ui: &mut Ui, rect: Rect, state: &mut GraphEditorState) {
    state
        .toasts
        .retain(|t| t.at.elapsed().as_millis() < TOAST_MS);
    if state.toasts.is_empty() {
        return;
    }
    let st = ui.style();
    let font = st.fonts.small;
    let pad = st.spacing.padding;
    let h = st.metrics.control_height;
    let mut y = rect.max.y - pad - h;
    // Newest nearest the canvas edge, older ones stacked above it.
    let toasts: Vec<(String, f32)> = state
        .toasts
        .iter()
        .rev()
        .map(|t| {
            let age = t.at.elapsed().as_millis() as f32 / TOAST_MS as f32;
            // Hold, then fade over the last third.
            (t.text.clone(), (1.0 - (age - 0.66) / 0.34).clamp(0.0, 1.0))
        })
        .collect();
    let mut p = ui.painter();
    for (text, alpha) in toasts {
        let w = p.measure_text(&text, font, None).x + pad * 2.0;
        let chip = Rect::from_min_size(
            Pos2::new(rect.center().x - w * 0.5, y),
            Vec2::new(w, h),
        );
        p.rect_filled(chip, st.rounding.small, st.palette.elevated.with_alpha(alpha));
        p.rect_stroke(
            chip,
            st.rounding.small,
            st.metrics.border,
            st.palette.stroke_strong.with_alpha(alpha),
        );
        p.text(
            Pos2::new(chip.min.x + pad, chip.center().y - font * 0.62),
            &text,
            font,
            st.palette.text.with_alpha(alpha),
            None,
        );
        y -= h + st.spacing.item;
    }
}

/// Find-in-graph: a small field at the canvas' top edge. Non-matching nodes
/// dim to 45%; `Enter` cycles the matches, framing each; `Esc` closes and
/// clears the dim. Session-only — a search is a thought, not a document.
fn find_overlay(
    ui: &mut Ui,
    rect: Rect,
    state: &mut GraphEditorState,
    geoms: &[NodeGeom],
    zoom_min: f32,
    zoom_max: f32,
    registry: &NodeRegistry,
) {
    let Some(find) = state.find.clone() else {
        return;
    };
    let st = ui.style();
    let pad = st.spacing.padding;
    let w = (rect.width() * 0.32).clamp(180.0, 360.0);
    let panel = Rect::from_min_size(
        Pos2::new(rect.center().x - w * 0.5, rect.min.y + pad),
        Vec2::new(w, st.metrics.control_height + pad),
    );
    // Rule 1: the find bar is a transient surface like any other. Registering
    // it means a press outside dismisses it *and* is consumed, so the same
    // click can't also select a node — which the old next-frame `overlay_rect`
    // could only approximate, with no consumption at all.
    ui.ctx_mut().modal_push(find_modal_id(), panel);
    {
        let mut p = ui.painter();
        p.rect_filled(panel, st.rounding.widget, st.palette.elevated);
        p.rect_stroke(panel, st.rounding.widget, st.metrics.border, st.palette.stroke_strong);
    }
    let mut query = find.query.clone();
    let (mut submitted, mut cancelled) = (false, false);
    ui.run_at(
        Rect::from_min_size(
            Pos2::new(panel.min.x + pad * 0.5, panel.min.y + pad * 0.5),
            Vec2::new(w - pad, st.metrics.control_height),
        ),
        Direction::TopDown,
        Id::new("graph_find_field"),
        UiOptions { padding: Vec2::ZERO, spacing: 0.0 },
        |ui| {
            let out = TextEdit::new(&mut query)
                .hint("Find in graph\u{2026}")
                .width(w - pad)
                .request_focus(find.first_frame)
                .show_full(ui);
            submitted = out.submitted;
            cancelled = out.cancelled;
        },
    );
    if cancelled
        || ui.ctx().input.key_pressed(Key::Escape)
        || ui.ctx().modal_dismissed(find_modal_id()).is_some()
    {
        state.find = None;
        ui.ctx_mut().modal_dismiss(find_modal_id());
        return;
    }

    let matches: Vec<u64> = geoms
        .iter()
        .filter(|g| find.matches(&g.title, &g.title))
        .map(|g| g.id)
        .collect();
    // Nodes inside embedded rules count too (ticket 05, spec story 22):
    // a search must never silently skip a rule. They cycle after the
    // canvas's own hits; landing on one opens the peek.
    let rule_hits: Vec<(u64, u64)> = if state.domain.is_animation() {
        region_find_matches(&state.doc, &find)
    } else {
        Vec::new()
    };
    let total = matches.len() + rule_hits.len();
    // Mono count, the same convention the palette footer uses.
    if find.active() {
        ui.painter().text_family(
            Pos2::new(panel.max.x - pad * 4.0, panel.center().y - st.fonts.small * 0.62),
            &format!("{total}"),
            st.fonts.small,
            st.palette.text_disabled,
            None,
            FontFamily::Mono,
        );
    }

    let mut cursor = find.cursor;
    if submitted && total > 0 {
        let i = cursor % total;
        cursor = (cursor + 1) % total;
        let frame_on = |state: &mut GraphEditorState, id: u64| {
            if let Some((mn, mx)) = geoms_bbox(geoms.iter().filter(|g| g.id == id)) {
                // Pan only, like error cycling — a find should not also
                // rescale the canvas out from under the reader.
                let v = frame_view(mn, mx, rect.size(), zoom_min, zoom_max);
                state.view = CanvasView { pan: v.pan, zoom: state.view.zoom };
            }
        };
        if let Some(&id) = matches.get(i) {
            state.select_only(id);
            frame_on(state, id);
        } else {
            let (owner, inner) = rule_hits[i - matches.len()];
            frame_on(state, owner);
            state.open_rule_scope_at(owner, inner, registry);
        }
    }
    if let Some(fs) = state.find.as_mut() {
        fs.query = query;
        fs.cursor = cursor;
        fs.first_frame = false;
    }
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
        // A scrim dims what is behind it and still shows it — that is the
        // whole point. Glass mode would blur the graph away entirely, so the
        // user could not see what they are about to purge.
        p.rect_filled_translucent(
            rect,
            Rounding::ZERO,
            Color::BLACK.with_alpha(st.palette.scrim_alpha),
        );
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

/// The empty-canvas context menu. `Add Node…` first, then the operations that
/// act on the canvas or the selection.
fn canvas_menu(
    ui: &mut Ui,
    state: &mut GraphEditorState,
    registry: &NodeRegistry,
    keymap: &Keymap,
    geoms: &[NodeGeom],
    clipboard: &mut Option<GraphFragment>,
    open_at: Option<Pos2>,
) -> Option<([f32; 2], [f32; 2])> {
    let at = state.canvas_menu?;
    let mut palette_at = None;
    let mut align: Option<AlignMode> = None;
    let mut do_layout = false;
    let mut do_paste = false;
    let mut select_all = false;
    let selected = state.selection.len();
    let has_clip = clipboard.is_some();

    crusty_gui::widgets::context_menu_at(ui, "graph_canvas_menu", open_at, |ui| {
        ui.menu_group_header("Canvas");
        if menu_row_for(ui, keymap, Action::ADD_NODE_PALETTE, "Add Node\u{2026}", true) {
            palette_at = Some(at);
        }
        if menu_row_for(ui, keymap, Action::PASTE, "Paste", has_clip) {
            do_paste = true;
        }
        if ui.menu_item("Select All") {
            select_all = true;
        }
        ui.separator();
        // The align strip appears once aligning is possible at all; distribute
        // rows disable at exactly two, same rule as the node menu.
        if selected >= 2 {
            ui.submenu("Align", |ui| {
                for mode in AlignMode::ALL {
                    let enabled = selected >= mode.min_nodes();
                    let row = match align_action(mode) {
                        Some(a) => menu_row_for(ui, keymap, a, mode.label(), enabled),
                        None => ui.menu_item_enabled(mode.label(), enabled),
                    };
                    if row {
                        align = Some(mode);
                    }
                }
            });
        }
        if menu_row_for(ui, keymap, Action::AUTO_LAYOUT, "Auto Layout", true) {
            do_layout = true;
        }
    });

    if let Some(mode) = align {
        let rects = selected_rects(state, geoms);
        state.align_nodes(&rects, mode, registry);
        state.canvas_menu = None;
    }
    if select_all {
        state.selection = state.doc.nodes.iter().map(|n| n.id).collect();
        state.canvas_menu = None;
    }
    if do_paste {
        // Paste lands where the menu was opened, which is the point the user
        // aimed at — not the view centre and not the live pointer, which has
        // since travelled to the menu row.
        state.paste_clipboard(clipboard, Some(at.0), registry);
        state.canvas_menu = None;
    }
    if do_layout || palette_at.is_some() {
        state.canvas_menu = None;
    }
    if do_layout {
        return None;
    }
    palette_at
}

/// The direct-align action behind an `AlignMode`, if one exists.
fn align_action(mode: AlignMode) -> Option<Action> {
    Some(match mode {
        AlignMode::Top => Action::ALIGN_TOP,
        AlignMode::Left => Action::ALIGN_LEFT,
        AlignMode::Bottom => Action::ALIGN_BOTTOM,
        AlignMode::Right => Action::ALIGN_RIGHT,
        AlignMode::CenterHorizontally => Action::ALIGN_CENTER_H,
        AlignMode::CenterVertically => Action::ALIGN_CENTER_V,
        AlignMode::DistributeHorizontally | AlignMode::DistributeVertically => return None,
    })
}

/// A context-menu row that shows its keyboard chord, right-aligned in mono,
/// whenever the action has one bound.
///
/// The chord is looked up rather than written into the label, so rebinding in
/// Preferences updates every menu that offers the action — a row and its key
/// cannot drift apart. An unbound action (the user cleared it, or the preset
/// never bound it) simply shows no chord.
fn menu_row_for(
    ui: &mut Ui,
    keymap: &Keymap,
    action: Action,
    label: &str,
    enabled: bool,
) -> bool {
    match keymap.chord_label(action) {
        Some(chord) => ui.menu_item_shortcut(label, chord, enabled),
        None => ui.menu_item_enabled(label, enabled),
    }
}

/// The node context menu. Every break path is also reachable here — the
/// gesture is the fast route, the menu is the discoverable one.
fn node_menu(
    ui: &mut Ui,
    state: &mut GraphEditorState,
    registry: &NodeRegistry,
    keymap: &Keymap,
    geoms: &[NodeGeom],
    open_at: Option<Pos2>,
) {
    let Some(id) = state.node_menu else {
        return;
    };
    let links = state.edges_on_node(id).len();
    // The strip appears as soon as *any* align is possible; distribute rows
    // are disabled rather than hidden at exactly two, so the operation stays
    // discoverable and its requirement is legible (the settings rule
    // generalises: disable, do not hide).
    let selected = state.selection.len();
    let align_ready = selected >= 2;
    let mut brk = false;
    let mut del = false;
    let mut align: Option<AlignMode> = None;
    let mut do_layout = false;
    crusty_gui::widgets::context_menu_at(ui, "graph_node_menu", open_at, |ui| {
        ui.menu_group_header("Node");
        if ui.menu_item_enabled(format!("Break all links ({links})"), links > 0) {
            brk = true;
        }
        if menu_row_for(ui, keymap, Action::DELETE_SELECTION, "Delete", true) {
            del = true;
        }
        // Selection operations: the node menu is where they belong, since
        // both act on what is selected rather than on where you clicked.
        ui.menu_group_header("Arrange");
        if menu_row_for(ui, keymap, Action::AUTO_LAYOUT, "Auto Layout", true) {
            do_layout = true;
        }
        if align_ready {
            for mode in AlignMode::ALL {
                let enabled = selected >= mode.min_nodes();
                let row = match align_action(mode) {
                    Some(a) => menu_row_for(ui, keymap, a, mode.label(), enabled),
                    // Distribute has no direct key — it is only ever reached
                    // from this strip.
                    None => ui.menu_item_enabled(mode.label(), enabled),
                };
                if row {
                    align = Some(mode);
                }
            }
        }
    });
    if let Some(mode) = align {
        let rects = selected_rects(state, geoms);
        state.align_nodes(&rects, mode, registry);
        state.node_menu = None;
        return;
    }
    if do_layout {
        let rects = all_rects(geoms);
        state.auto_layout(&rects, layout_spacing(&ui.style()), registry);
        state.node_menu = None;
        return;
    }
    if brk {
        state.break_node_links(id, registry);
        state.node_menu = None;
    } else if del {
        state.delete_selection(registry);
        state.node_menu = None;
    }
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
    let mut brk = false;
    crusty_gui::widgets::context_menu_at(ui, "graph_wire_menu", open_at, |ui| {
        ui.menu_group_header("Wire");
        if ui.menu_item("Add reroute") {
            add = true;
        }
        if ui.menu_item("Break link") {
            brk = true;
        }
    });
    if brk {
        state.break_links(std::slice::from_ref(&edge), "Broke", registry);
        state.wire_menu = None;
        return;
    }
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
    keymap: &Keymap,
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
        if menu_row_for(ui, keymap, Action::DELETE_SELECTION, "Delete", true) {
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
// Add-node palette
// ---------------------------------------------------------------------------

/// Palette popover metrics, base units.
const PALETTE_W: f32 = 260.0;
const PALETTE_ROWS: usize = 9;
/// Incompatible rows stay listed at 45% — never hidden.
const PALETTE_DIM: f32 = 0.45;

/// Open the palette at a screen/world point, optionally filtered by the pin a
/// drag came off.
fn open_palette(
    state: &mut GraphEditorState,
    world: [f32; 2],
    screen: [f32; 2],
    from: Option<PaletteDragSource>,
) {
    state.palette = Some(PaletteState {
        world,
        screen,
        search: String::new(),
        cursor: 0,
        from,
        first_frame: true,
    });
}

/// The add-node palette — the asset-picker shell at E3: translucent
/// `elevated` fill, `stroke_strong` border, auto-focused search, mono count in
/// the footer.
///
/// With no query the rows are grouped by category (plus Annotate and Subgraph
/// sections); typing switches to a flat ranked list. A pin drag filters by
/// type but **never hides** a row: incompatible nodes stay at 45% carrying the
/// type they do take, because "where did my node go" teaches nothing.
fn palette_popover(
    ui: &mut Ui,
    state: &mut GraphEditorState,
    registry: &NodeRegistry,
    subgraph_assets: &[String],
) {
    let Some(p) = state.palette.clone() else {
        ui.ctx_mut().modal_dismiss(palette_modal_id());
        return;
    };
    if ui.ctx().modal_dismissed(palette_modal_id()).is_some() {
        state.palette = None;
        return;
    }
    let st = ui.style();
    let s = (st.metrics.row_height / BASE_ROW_H).max(0.1);
    let w = PALETTE_W * s;
    let row_h = st.metrics.row_height;
    let pad = st.spacing.padding;

    let filter = p.from.as_ref().map(|f| PinFilter {
        ty: f.ty.clone(),
        need_input: f.output,
    });
    let entries = graph_palette::build_entries(registry, &p.search, filter.as_ref());
    let searching = !p.search.trim().is_empty();

    // Extra rows the unfiltered palette offers beyond node types.
    let extras: Vec<(&str, PaletteExtra)> = if searching || p.from.is_some() {
        Vec::new()
    } else {
        let mut v = vec![("Add Comment", PaletteExtra::Comment)];
        if !state.selection.is_empty() {
            v.push(("Add Group around selection", PaletteExtra::Group));
        }
        v.extend(
            subgraph_assets
                .iter()
                .map(|path| (path.as_str(), PaletteExtra::Subgraph(path.clone()))),
        );
        v
    };

    let total = entries.len() + extras.len();
    let shown = total.min(PALETTE_ROWS);
    let list_h = row_h * shown.max(1) as f32;
    let head_h = st.metrics.control_height + pad;
    let foot_h = st.fonts.small * 2.0;
    let rect = Rect::from_min_size(
        Pos2::new(p.screen[0], p.screen[1]),
        Vec2::new(w, head_h + list_h + foot_h + pad * 2.0),
    );

    ui.ctx_mut().modal_push(palette_modal_id(), rect);

    // E3 surface: `popover_alpha` (0.96) is the design system's standard for a
    // floating panel — near-opaque by design, composited through the glass path
    // like every other popover. (This comment used to claim "simple alpha, no
    // blur", contradicting both the constant and the renderer; ruling 2026-08:
    // the constant wins.)
    {
        let mut pt = ui.painter();
        pt.rect_filled(
            rect,
            st.rounding.panel,
            st.palette.elevated.with_alpha(st.palette.popover_alpha),
        );
        pt.rect_stroke(rect, st.rounding.panel, st.metrics.border, st.palette.stroke_strong);
    }

    // Search field, focused on open.
    let mut search = p.search.clone();
    let mut submitted = false;
    let mut cancelled = false;
    ui.run_at(
        Rect::from_min_size(
            Pos2::new(rect.min.x + pad, rect.min.y + pad),
            Vec2::new(w - pad * 2.0, st.metrics.control_height),
        ),
        Direction::TopDown,
        Id::new("graph_palette_search"),
        UiOptions { padding: Vec2::ZERO, spacing: 0.0 },
        |ui| {
            let out = TextEdit::new(&mut search)
                .hint("Search nodes\u{2026}")
                .width(w - pad * 2.0)
                .request_focus(p.first_frame)
                .show_full(ui);
            submitted = out.submitted;
            cancelled = out.cancelled;
        },
    );

    // Keyboard navigation. Up/Down move the highlight, Enter takes it, Esc
    // closes — the picker idiom, not a new pattern.
    let (up, down, esc) = {
        let input = &ui.ctx().input;
        (
            input.key_pressed(Key::ArrowUp),
            input.key_pressed(Key::ArrowDown),
            input.key_pressed(Key::Escape),
        )
    };
    let mut cursor = p.cursor.min(total.saturating_sub(1));
    if down && total > 0 {
        cursor = (cursor + 1) % total;
    }
    if up && total > 0 {
        cursor = (cursor + total - 1) % total;
    }
    if esc || cancelled {
        state.palette = None;
        return;
    }

    // Rows: a window of PALETTE_ROWS around the cursor.
    let first = cursor.saturating_sub(PALETTE_ROWS - 1);
    let mut picked: Option<usize> = None;
    let mut y = rect.min.y + head_h + pad;
    {
        let mut pt = ui.painter();
        for i in first..total.min(first + PALETTE_ROWS) {
            let row = Rect::from_min_size(
                Pos2::new(rect.min.x + pad, y),
                Vec2::new(w - pad * 2.0, row_h),
            );
            let hot = i == cursor;
            if hot {
                pt.rect_filled(row, st.rounding.small, st.palette.selection_fill);
            }
            let (label, tag, dim) = if i < entries.len() {
                let e = &entries[i];
                let dim = !e.fit.is_compatible();
                let tag = match &e.fit {
                    graph_palette::PinFit::Incompatible { type_tag } => type_tag.clone(),
                    _ => searching.then(|| e.category.clone()),
                };
                (e.name.clone(), tag, dim)
            } else {
                (extras[i - entries.len()].0.to_string(), None, false)
            };
            let alpha = if dim { PALETTE_DIM } else { 1.0 };
            let color = if hot {
                st.palette.selection_text
            } else {
                st.palette.text
            };
            pt.text(
                Pos2::new(row.min.x + pad * 0.5, row.center().y - st.fonts.body * 0.62),
                &label,
                st.fonts.body,
                color.with_alpha(alpha),
                None,
            );
            if let Some(tag) = tag {
                let tw = pt
                    .measure_text_family(&tag, st.fonts.small, None, FontFamily::Mono)
                    .x;
                pt.text_family(
                    Pos2::new(row.max.x - pad * 0.5 - tw, row.center().y - st.fonts.small * 0.62),
                    &tag,
                    st.fonts.small,
                    st.palette.text_secondary.with_alpha(alpha),
                    None,
                    FontFamily::Mono,
                );
            }
            y += row_h;
        }
        // Footer: mono count, the picker shell's convention.
        pt.text_family(
            Pos2::new(rect.min.x + pad, rect.max.y - pad - st.fonts.small),
            &format!("{total} node{}", if total == 1 { "" } else { "s" }),
            st.fonts.small,
            st.palette.text_disabled,
            None,
            FontFamily::Mono,
        );
    }

    // Mouse picking over the same window.
    let mut yy = rect.min.y + head_h + pad;
    for i in first..total.min(first + PALETTE_ROWS) {
        let row = Rect::from_min_size(
            Pos2::new(rect.min.x + pad, yy),
            Vec2::new(w - pad * 2.0, row_h),
        );
        let id = ui.alloc_id(("graph_palette_row", i));
        let resp = ui.interact(id, row);
        if resp.hovered {
            cursor = i;
        }
        if resp.clicked {
            picked = Some(i);
        }
        yy += row_h;
    }
    if submitted && total > 0 {
        picked = Some(cursor);
    }

    // Write the (possibly moved) cursor and search back.
    if let Some(ps) = state.palette.as_mut() {
        ps.search = search;
        ps.cursor = cursor;
        ps.first_frame = false;
    }

    let Some(i) = picked else {
        return;
    };
    state.palette = None;
    if i >= entries.len() {
        match &extras[i - entries.len()].1 {
            PaletteExtra::Comment => state.add_comment(p.world, registry),
            PaletteExtra::Group => state.add_group_around_selection(registry),
            PaletteExtra::Subgraph(path) => state.add_subgraph_node(path, p.world, registry),
        }
        return;
    }
    let entry = &entries[i];
    place_palette_pick(state, registry, entry, &p);
}

/// The non-node rows the unfiltered palette also offers.
#[derive(Clone)]
enum PaletteExtra {
    Comment,
    Group,
    Subgraph(String),
}

/// Spawn a picked node and, when the palette was type-filtered, wire it up.
///
/// A compatible pick lands with its *wired pin* on the drop point rather than
/// its corner: the wire then ends where the user let go, which is the whole
/// point of dragging off a pin. An incompatible pick just lands there
/// unconnected.
fn place_palette_pick(
    state: &mut GraphEditorState,
    registry: &NodeRegistry,
    entry: &PaletteEntry,
    p: &PaletteState,
) {
    let slug = match (&entry.fit, &p.from) {
        (graph_palette::PinFit::Compatible(slug), Some(_)) => Some(slug.clone()),
        _ => None,
    };
    let mut pos = p.world;
    if let (Some(slug), Some(from)) = (slug.as_ref(), p.from.as_ref()) {
        // Offset so the pin we are about to wire lands on the drop point.
        if let Some(desc) = registry.get(&entry.id) {
            let side = if from.output { &desc.inputs } else { &desc.outputs };
            let row = side.iter().position(|q| &q.slug == slug).unwrap_or(0);
            pos[1] -= BASE_HEADER_H + BASE_ROW_H * (row as f32 + 0.5);
            if !from.output {
                // Wiring our *output* back to their input: the pin is on the
                // node's right edge, so shift the body left of the drop.
                pos[0] -= BASE_MIN_W;
            }
        }
    }
    // Nudge clear of anything already there — the palette's drop rule.
    for _ in 0..32 {
        let clash = state
            .doc
            .nodes
            .iter()
            .any(|n| (n.position[0] - pos[0]).abs() < 1.0 && (n.position[1] - pos[1]).abs() < 1.0);
        if !clash {
            break;
        }
        pos[0] += 8.0;
        pos[1] += 8.0;
    }

    state.add_node(&entry.id, pos, registry);
    let Some(new_id) = state.doc.nodes.last().map(|n| n.id) else {
        return;
    };
    let (Some(slug), Some(from)) = (slug, p.from.as_ref()) else {
        return;
    };
    let edge = if from.output {
        Edge {
            from_node: from.node,
            from_pin: from.pin.clone(),
            to_node: new_id,
            to_pin: slug,
        }
    } else {
        Edge {
            from_node: new_id,
            from_pin: slug,
            to_node: from.node,
            to_pin: from.pin.clone(),
        }
    };
    state.doc.edges.push(edge.clone());
    state.commit(GraphEdit::Connect(edge), registry);
}

// ---------------------------------------------------------------------------
// Toolbar
// ---------------------------------------------------------------------------

/// Height of the graph toolbar row, base units at ui_scale 1.0.
const TOOLBAR_H: f32 = 30.0;
/// Width of the 3-way wire-style segmented control, base units.
const TOOLBAR_SEG_W: f32 = 168.0;
/// Height of the PAUSED banner's row (GS-4), base units — the mockup's 32px
/// bar plus the half-pad of breathing room that keeps it off the toolbar rule.
const BANNER_H: f32 = 40.0;
/// Breakpoint octagon radius, as a multiple of the pin radius (GS-4).
const BREAK_BADGE_R: f32 = 1.15;

/// The graph tab's toolbar: a 3-way wire-style quick switch on the left and
/// the document's realm as a read-only mono chip on the right.
///
/// **Nothing else goes here.** A second control from the Preferences ▸ Graph
/// sections would become a competing settings surface — the panels doc's
/// explicit rule. The segmented control is the documented toggled-tool
/// treatment (`accent_soft` fill + accent border), one of the few approved
/// accent spends on this surface.
#[allow(clippy::too_many_arguments)]
fn graph_toolbar(
    ui: &mut Ui,
    state: &mut GraphEditorState,
    style_now: WireStyle,
    request: &mut Option<WireStyle>,
    exec: Option<&GraphExecViz>,
    instances: &[ExecInstance],
    clear_trace: &mut bool,
    chip_rect: &mut Option<(Rect, bool)>,
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

    // Live chip (45-A P7, extended in GS-3): which running instance the canvas
    // is showing, and the picker for choosing another. It answers "whose
    // execution am I looking at" — without it a pulsing wire is a mystery,
    // since two entities can run the same document.
    //
    // Three states, per the design: LIVE (bound, alive) - KILLED (bound, dead;
    // the canvas keeps its last trace for the post-mortem) - "N RUNNING"
    // (instances exist, none picked, and the canvas stays a pure editor).
    // Nothing running at all leaves no residue whatsoever.
    let status = Palette::invariant_status();
    let killed = exec.is_some_and(|v| {
        v.killed.is_some() || instances.iter().any(|i| i.id == v.instance_id && i.killed)
    });
    let (word, word_col, chip_fill, chip_stroke) = match chip_state(exec.is_some(), killed, instances.len()) {
        ChipState::Absent => {
            state.exec_picker = false;
            return;
        }
        ChipState::Killed => (
            "\u{2715} KILLED".to_string(),
            status.error,
            status.error.with_alpha(0.10),
            status.error,
        ),
        ChipState::Live => (
            "LIVE".to_string(),
            st.palette.accent_active,
            st.palette.accent_soft,
            st.palette.accent_active,
        ),
        ChipState::Unbound(n) => (
            format!("{n} RUNNING"),
            st.palette.text_secondary,
            st.palette.panel,
            st.palette.stroke,
        ),
    };
    let who = match exec {
        Some(v) => v.instance.clone(),
        None => "select instance".to_string(),
    };
    // Two runs, not one string: "LIVE Duck" as a single accent label reads as
    // a mode *called* Duck. The state is the accent word; the entity is a
    // separate, plainly-colored noun after a thin separator.
    let who = who.as_str();
    let sep = if who.is_empty() { "" } else { " · " };
    let state_w = ui
        .painter()
        .measure_text_family(&word, px, None, FontFamily::Mono)
        .x;
    let who_w = ui
        .painter()
        .measure_text_family(&format!("{sep}{who}"), px, None, FontFamily::Mono)
        .x;
    let dot_r = px * 0.28;
    let inner = pad + dot_r * 2.0 + pad * 0.5 + state_w + who_w + pad;
    // Two pads from the realm chip: they are different statements (what is
    // running vs. what authority the document declares) and should not read as
    // one segmented control.
    let live_chip = Rect::from_min_size(
        Pos2::new(chip.min.x - pad * 2.0 - inner, chip.min.y),
        Vec2::new(inner, seg_h),
    );
    ui.painter()
        .rect_filled(live_chip, st.rounding.small, chip_fill);
    ui.painter()
        .rect_stroke(live_chip, st.rounding.small, st.metrics.border, chip_stroke);
    ui.painter().circle_filled(
        Pos2::new(live_chip.min.x + pad + dot_r, live_chip.center().y),
        dot_r,
        if exec.is_some() { word_col } else { status.success },
    );
    let text_x = live_chip.min.x + pad + dot_r * 2.0 + pad * 0.5;
    let text_y = live_chip.center().y - px * 0.62;
    ui.painter().text_family(
        Pos2::new(text_x, text_y),
        &word,
        px,
        word_col,
        None,
        FontFamily::Mono,
    );
    if !who.is_empty() {
        ui.painter().text_family(
            Pos2::new(text_x + state_w, text_y),
            &format!("{sep}{who}"),
            px,
            st.palette.text,
            None,
            FontFamily::Mono,
        );
    }
    // The chip is the picker's button.
    let chip_id = ui.alloc_id("graph_live_chip");
    let resp = ui.interact(chip_id, live_chip);
    let just_opened = resp.clicked && !state.exec_picker;
    if resp.clicked {
        state.exec_picker = !state.exec_picker;
    }
    if resp.hovered && !state.exec_picker {
        ui.tooltip_for(
            live_chip,
            "Which running instance this canvas shows.\n\
             Click to pick another; hover a wire or pin for its last value.",
        );
    }

    // Clear trace: the taken-path tint and the pulse history are one session's
    // statement, and sometimes the session should start *here*.
    let clear_w = ui.painter().measure_text("Clear trace", px, None).x + pad * 2.0;
    let clear = Rect::from_min_size(
        Pos2::new(live_chip.min.x - pad - clear_w, live_chip.min.y),
        Vec2::new(clear_w, seg_h),
    );
    if strip_button(ui, Id::new("graph_clear_trace"), clear, "Clear trace", 0) {
        *clear_trace = true;
    }

    // The dropdown itself is drawn by the panel *after* the canvas: the
    // toolbar paints before it, so a surface opened here would be buried
    // under the grid it is supposed to float over.
    *chip_rect = Some((live_chip, just_opened));
}

/// The PAUSED banner (GS-4) — **inside the tab**, docked under the graph
/// toolbar, never global chrome.
///
/// A pause belongs to one instance of one document; a window-level bar would
/// claim the whole editor is stopped, and with two graph tabs open it would
/// have to name which one. Docking it here also means it takes its row out of
/// the canvas exactly like the toolbar does, so nothing measured off the
/// canvas has to know it exists.
///
/// Anatomy is the mockup's: PAUSED · node · instance and hit count · Resume /
/// Step / Stop, each with its chord in mono.
#[allow(clippy::too_many_arguments)]
fn paused_banner(
    ui: &mut Ui,
    state: &mut GraphEditorState,
    registry: &NodeRegistry,
    resolver: &DocResolvers<'_>,
    keymap: &Keymap,
    exec: Option<&GraphExecViz>,
) {
    let (Some(pause), Some(viz)) = (exec.and_then(|v| v.paused), exec) else {
        return;
    };
    let st = ui.style();
    let s = (st.metrics.row_height / BASE_ROW_H).max(0.1);
    let status = Palette::invariant_status();
    let pad = BASE_PAD_X * s;
    let w = ui.available().width();
    let row = ui.allocate(Vec2::new(w, BANNER_H * s));
    let rect = Rect::from_min_max(
        Pos2::new(row.min.x + pad, row.min.y + pad * 0.5),
        Pos2::new(row.max.x - pad, row.max.y - pad * 0.5),
    );
    ui.painter()
        .rect_filled(row, Rounding::ZERO, st.palette.window);
    ui.painter()
        // Half a chip's tint: the wash is the same token, but a full-width bar
        // covers twenty times a chip's area and would otherwise outshout the
        // canvas it is reporting on. The 1px border carries the identity.
        .rect_filled(rect, st.rounding.small, status.warning.with_alpha(0.05));
    ui.painter()
        .rect_stroke(rect, st.rounding.small, st.metrics.border, status.warning);

    let px = st.fonts.small;
    let body = st.fonts.body;
    let mut x = rect.min.x + pad;
    let word_w = ui
        .painter()
        .measure_text_family("PAUSED", px, None, FontFamily::Mono)
        .x;
    ui.painter().text_family(
        Pos2::new(x, rect.center().y - px * 0.62),
        "PAUSED",
        px,
        status.warning,
        None,
        FontFamily::Mono,
    );
    x += word_w + pad;

    // The node, by the name the canvas gives it — a synthesized title
    // ("Branch", "Get Health") is what the author is looking at, so the banner
    // must not fall back to a type id.
    let title = resolver
        .bind(&state.doc, registry)
        .display_name(pause.node)
        .unwrap_or_else(|| "node".to_string());
    let title_w = ui.painter().measure_text(&title, body, None).x;
    ui.painter().text(
        Pos2::new(x, rect.center().y - body * 0.62),
        &title,
        body,
        st.palette.text,
        None,
    );
    x += title_w + pad * 1.6;

    // Instance and hit count in one mono run: which Duck, and how many times
    // this session has stopped here.
    let who = if viz.instance.is_empty() { "instance" } else { viz.instance.as_str() };
    let sub = format!("{who} \u{00B7} hit {}\u{00D7}", pause.hits.max(1));
    ui.painter().text_family(
        Pos2::new(x, rect.center().y - px * 0.62),
        &sub,
        px,
        st.palette.text_disabled,
        None,
        FontFamily::Mono,
    );

    // Buttons, right to left so the primary lands where the eye already is.
    let mut req: Option<DebugRequest> = None;
    let mut right = rect.max.x - pad;
    for (label, action, kind) in [
        ("Stop", None, 0u8),
        ("Step", Some(Action::DEBUG_STEP), 0),
        ("Resume", Some(Action::DEBUG_RESUME), 1),
    ] {
        let chord = action.and_then(|a| keymap.chord_label(a)).unwrap_or_default();
        let lw = ui.painter().measure_text(label, body, None).x;
        let cw = if chord.is_empty() {
            0.0
        } else {
            ui.painter().measure_text_family(&chord, px * 0.9, None, FontFamily::Mono).x
                + pad * 0.5
        };
        let bw = lw + cw + pad * 1.6;
        let b = Rect::from_min_max(
            Pos2::new(right - bw, rect.min.y + pad * 0.4),
            Pos2::new(right, rect.max.y - pad * 0.4),
        );
        right = b.min.x - pad * 0.6;
        let bid = ui.alloc_id(("graph_paused_btn", label));
        let resp = ui.interact(bid, b);
        let fill = match (kind, resp.hovered) {
            (1, false) => st.palette.active,
            (1, true) => st.palette.hover,
            (_, true) => st.palette.hover,
            _ => Color::TRANSPARENT,
        };
        let mut p = ui.painter();
        p.rect_filled(b, st.rounding.small, fill);
        p.rect_stroke(
            b,
            st.rounding.small,
            st.metrics.border,
            if kind == 1 { st.palette.stroke_strong } else { st.palette.stroke },
        );
        p.text(
            Pos2::new(b.min.x + pad * 0.8, b.center().y - body * 0.62),
            label,
            body,
            if kind == 1 { st.palette.text } else { st.palette.text_secondary },
            None,
        );
        if !chord.is_empty() {
            p.text_family(
                Pos2::new(b.min.x + pad * 0.8 + lw + pad * 0.5, b.center().y - px * 0.56),
                &chord,
                px * 0.9,
                st.palette.text_disabled,
                None,
                FontFamily::Mono,
            );
        }
        if resp.clicked {
            req = Some(match label {
                "Resume" => DebugRequest::Resume,
                "Step" => DebugRequest::Step,
                _ => DebugRequest::Stop,
            });
        }
    }
    if let Some(r) = req {
        state.debug_request = Some(r);
    }
}

/// What the LIVE chip is saying (GS-3). Pure, so the state machine the design
/// specifies — bound-and-alive, bound-and-dead, running-but-unbound, nothing
/// at all — is checkable without a canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChipState {
    /// Bound to a live instance.
    Live,
    /// Bound to one that died. The canvas keeps its last trace: a post-mortem
    /// is exactly when you want to look at it.
    Killed,
    /// `n` instances exist and none is picked. The canvas stays a pure editor
    /// and never aggregates them.
    Unbound(usize),
    /// Nothing is running: edit mode leaves no residue at all.
    Absent,
}

fn chip_state(bound: bool, killed: bool, instances: usize) -> ChipState {
    match (bound, killed) {
        (true, true) => ChipState::Killed,
        (true, false) => ChipState::Live,
        (false, _) if instances > 0 => ChipState::Unbound(instances),
        (false, _) => ChipState::Absent,
    }
}

/// The LIVE chip's dropdown: every instance running this graph, nearest first.
///
/// Returns `Some(binding)` when a row was chosen - `Some(None)` for "follow
/// the selection", which is the baseline rule and therefore the way back out
/// of an explicit pick.
fn instance_picker(
    ui: &mut Ui,
    anchor: Rect,
    instances: &[ExecInstance],
    exec: Option<&GraphExecViz>,
    st: &Style,
    s: f32,
    // The press that opened the picker lands *outside* it — without this the
    // modal stack would read its own opening click as a dismissal and the
    // dropdown would close on the frame it appeared.
    just_opened: bool,
) -> Option<Option<u64>> {
    let pad = BASE_PAD_X * s;
    let px = st.fonts.small;
    let row_h = st.metrics.control_height;
    let w = (VARS_W * s).max(anchor.width());
    let h = row_h * (instances.len() as f32 + 1.0) + row_h * 0.9;
    let panel = Rect::from_min_size(
        Pos2::new(anchor.max.x - w, anchor.max.y + pad * 0.25),
        Vec2::new(w, h),
    );
    // Rule 1: a transient surface registers, so the press that dismisses it is
    // consumed instead of also landing on the canvas.
    ui.ctx_mut().modal_push(instance_picker_modal_id(), panel);
    {
        let mut p = ui.painter();
        p.rect_filled(panel, st.rounding.widget, st.palette.elevated);
        p.rect_stroke(panel, st.rounding.widget, st.metrics.border, st.palette.stroke_strong);
    }
    let mut picked: Option<Option<u64>> = None;
    let bound = exec.map(|v| v.instance_id);
    let mut y = panel.min.y;
    for inst in instances {
        let row = Rect::from_min_size(Pos2::new(panel.min.x, y), Vec2::new(w, row_h));
        y += row_h;
        let id = ui.alloc_id(("graph_instance_row", inst.id));
        let resp = ui.interact(id, row);
        let is_bound = bound == Some(inst.id);
        let mut p = ui.painter();
        if is_bound {
            p.rect_filled(row, Rounding::ZERO, st.palette.selection_fill);
        } else if resp.hovered {
            p.rect_filled(row, Rounding::ZERO, st.palette.hover);
        }
        let dot_r = px * 0.28;
        p.circle_filled(
            Pos2::new(row.min.x + pad + dot_r, row.center().y),
            dot_r,
            if inst.killed {
                Palette::invariant_status().error
            } else if is_bound {
                st.palette.accent_active
            } else {
                Palette::invariant_status().success
            },
        );
        // Distance and recency, mono: two "Duck"s are told apart by where they
        // are and when they last did something, not by their name.
        let meta = if is_bound {
            "selected".to_string()
        } else {
            let recency = match inst.last_active {
                Some(a) if a < 60.0 => format!("{a:.0} s ago"),
                _ => "idle".to_string(),
            };
            format!("{:.0} m \u{b7} {recency}", inst.distance)
        };
        let mw = p.measure_text_family(&meta, px, None, FontFamily::Mono).x;
        p.text_family(
            Pos2::new(row.max.x - pad - mw, row.center().y - px * 0.62),
            &meta,
            px,
            if is_bound { st.palette.text_mono } else { st.palette.text_disabled },
            None,
            FontFamily::Mono,
        );
        let name = if inst.name.is_empty() { "(unnamed)" } else { inst.name.as_str() };
        let avail = row.max.x - pad * 2.0 - mw - (row.min.x + pad * 2.0 + dot_r * 2.0);
        let name = clip_text(&mut p, name, st.fonts.body, avail);
        p.text(
            Pos2::new(
                row.min.x + pad * 2.0 + dot_r * 2.0,
                row.center().y - st.fonts.body * 0.62,
            ),
            &name,
            st.fonts.body,
            if is_bound { st.palette.selection_text } else { st.palette.text },
            None,
        );
        if resp.clicked {
            picked = Some(Some(inst.id));
        }
    }
    // The way back to following the selection, then the count as the footer's
    // last line — the mockup's order, and the one that reads as a summary
    // rather than as another option.
    let follow = Rect::from_min_size(Pos2::new(panel.min.x, y), Vec2::new(w, row_h));
    let fid = ui.alloc_id("graph_instance_follow");
    let fresp = ui.interact(fid, follow);
    {
        let mut p = ui.painter();
        if fresp.hovered {
            p.rect_filled(follow, Rounding::ZERO, st.palette.hover);
        }
        p.text(
            Pos2::new(follow.min.x + pad, follow.center().y - px * 0.62),
            "Follow selection",
            px,
            st.palette.text_secondary,
            None,
        );
    }
    if fresp.clicked {
        picked = Some(None);
    }
    let footer = Rect::from_min_size(Pos2::new(panel.min.x, follow.max.y), Vec2::new(w, row_h * 0.9));
    {
        let mut p = ui.painter();
        p.line_segment(
            Pos2::new(footer.min.x, footer.min.y),
            Pos2::new(footer.max.x, footer.min.y),
            st.metrics.border,
            st.palette.stroke,
        );
        p.text(
            Pos2::new(footer.min.x + pad, footer.center().y - px * 0.62),
            &format!(
                "{} instance{} running this graph",
                instances.len(),
                if instances.len() == 1 { "" } else { "s" }
            ),
            px,
            st.palette.text_disabled,
            None,
        );
    }
    // Light dismiss, decided here rather than read off the modal stack: the
    // press that *opens* this panel necessarily lands outside it, and asking
    // the stack "was there a press outside" cannot tell that press from the
    // one that should close it. Escape, or a press anywhere else, ends it.
    let pressed_outside = ui.ctx().input.pointer_pressed
        && ui
            .ctx()
            .input
            .pointer_pos
            .is_some_and(|p| !panel.contains(p) && !anchor.contains(p));
    if !just_opened && (ui.ctx().input.key_pressed(Key::Escape) || pressed_outside) {
        ui.ctx_mut().modal_dismiss(instance_picker_modal_id());
        // Keep the binding, close the surface: "put it away" is not "unbind".
        return Some(exec.map(|v| v.instance_id));
    }
    if picked.is_some() {
        ui.ctx_mut().modal_dismiss(instance_picker_modal_id());
    }
    picked
}

fn instance_picker_modal_id() -> crusty_gui::id::Id {
    crusty_gui::id::Id::ROOT.with("graph_instance_picker")
}

// ---------------------------------------------------------------------------
// Preview strip (Task 41 ticket 06)
// ---------------------------------------------------------------------------

/// Footer band height at ui_scale 1.0 — one control row, the curve editor's
/// footer weight.
const PREVIEW_H: f32 = 34.0;
/// A Float parameter's slider width at ui_scale 1.0.
const PREVIEW_SLIDER_W: f32 = 110.0;

/// What the PREVIEW chip says. The LIVE chip's ladder in preview vocabulary,
/// pure so the states are checkable without a canvas: bound (driving),
/// bound-but-refused (the runtime would not arm), candidates-but-unbound,
/// nothing at all — which, unlike the LIVE chip, still draws: the strip is
/// where an author learns preview exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PreviewChipState {
    Bound,
    Refused,
    Unbound(usize),
    Empty,
}

fn preview_chip_state(bound: bool, refused: bool, instances: usize) -> PreviewChipState {
    match (bound, refused) {
        (true, true) => PreviewChipState::Refused,
        (true, false) => PreviewChipState::Bound,
        (false, _) if instances > 0 => PreviewChipState::Unbound(instances),
        (false, _) => PreviewChipState::Empty,
    }
}

/// The preview parameter strip (mockup 2g): `PREVIEW · entity` chip, then a
/// control per declared parameter — Float slider, Bool checkbox, Trigger
/// FIRE. Values are read off the bound runtime every frame, so whatever else
/// writes a parameter shows here, and a buffered Trigger stays lit because
/// the machine still holds it — not because the strip remembers the click.
/// Edits land in `state.anim_edits`, applied by the host after the UI:
/// runtime-only writes, never document state, never undo entries.
///
/// Returns the chip's rect + whether its picker just opened, so the panel
/// can float the entity picker after the band drew.
fn anim_preview_strip(
    ui: &mut Ui,
    band: Rect,
    state: &mut GraphEditorState,
    registry: &NodeRegistry,
    anim: Option<&AnimPreview>,
    instances: &[ExecInstance],
) -> (Rect, bool) {
    let st = ui.style();
    let s = (st.metrics.row_height / BASE_ROW_H).max(0.1);
    let pad = BASE_PAD_X * s;
    let px = st.fonts.small;
    let status = Palette::invariant_status();
    {
        let mut p = ui.painter();
        p.rect_filled(band, Rounding::ZERO, st.palette.window);
        p.line_segment(
            Pos2::new(band.min.x, band.min.y),
            Pos2::new(band.max.x, band.min.y),
            st.metrics.border,
            st.palette.stroke_strong,
        );
    }
    let seg_h = st.metrics.control_height.min(band.height() - 6.0 * s);
    let cy = band.center().y;

    let refused = anim.is_some_and(|a| a.disabled.is_some());
    let chip_state = preview_chip_state(anim.is_some(), refused, instances.len());
    let (word, word_col, chip_fill, chip_stroke) = match chip_state {
        PreviewChipState::Refused => (
            "\u{2715} REFUSED".to_string(),
            status.error,
            status.error.with_alpha(0.10),
            status.error,
        ),
        PreviewChipState::Bound => (
            "PREVIEW".to_string(),
            st.palette.accent_active,
            st.palette.accent_soft,
            st.palette.accent_active,
        ),
        PreviewChipState::Unbound(n) => (
            format!("{n} RUNNING"),
            st.palette.text_secondary,
            st.palette.panel,
            st.palette.stroke,
        ),
        PreviewChipState::Empty => (
            "PREVIEW".to_string(),
            st.palette.text_disabled,
            st.palette.panel,
            st.palette.stroke,
        ),
    };
    let who = match (anim, chip_state) {
        (Some(a), _) if a.instance.is_empty() => "(unnamed)".to_string(),
        (Some(a), _) => a.instance.clone(),
        (None, PreviewChipState::Unbound(_)) => "select entity".to_string(),
        _ => "nothing runs this graph".to_string(),
    };
    // Two runs, like the LIVE chip: the state is the accent word, the entity
    // a plainly-colored noun after a thin separator.
    let sep = " \u{00B7} ";
    let (state_w, who_w) = {
        let mut p = ui.painter();
        (
            p.measure_text_family(&word, px, None, FontFamily::Mono).x,
            p.measure_text_family(&format!("{sep}{who}"), px, None, FontFamily::Mono).x,
        )
    };
    let dot_r = px * 0.28;
    let inner = pad + dot_r * 2.0 + pad * 0.5 + state_w + who_w + pad;
    let chip = Rect::from_min_size(
        Pos2::new(band.min.x + pad, cy - seg_h * 0.5),
        Vec2::new(inner, seg_h),
    );
    {
        let mut p = ui.painter();
        p.rect_filled(chip, st.rounding.small, chip_fill);
        p.rect_stroke(chip, st.rounding.small, st.metrics.border, chip_stroke);
        p.circle_filled(
            Pos2::new(chip.min.x + pad + dot_r, chip.center().y),
            dot_r,
            match chip_state {
                PreviewChipState::Unbound(_) => status.success,
                PreviewChipState::Empty => st.palette.text_disabled,
                _ => word_col,
            },
        );
        let text_x = chip.min.x + pad + dot_r * 2.0 + pad * 0.5;
        let text_y = chip.center().y - px * 0.62;
        p.text_family(Pos2::new(text_x, text_y), &word, px, word_col, None, FontFamily::Mono);
        p.text_family(
            Pos2::new(text_x + state_w, text_y),
            &format!("{sep}{who}"),
            px,
            if chip_state == PreviewChipState::Empty {
                st.palette.text_disabled
            } else {
                st.palette.text
            },
            None,
            FontFamily::Mono,
        );
    }
    let mut just_opened = false;
    if chip_state == PreviewChipState::Empty {
        // Nothing to pick; a dead chip must not hold a picker open either.
        state.anim_picker = false;
    } else {
        let chip_id = ui.alloc_id("anim_preview_chip");
        let resp = ui.interact(chip_id, chip);
        just_opened = resp.clicked && !state.anim_picker;
        if resp.clicked {
            state.anim_picker = !state.anim_picker;
        }
        if resp.hovered && !state.anim_picker {
            let tip = match (chip_state, anim.and_then(|a| a.disabled.as_deref())) {
                (PreviewChipState::Refused, Some(why)) => {
                    format!("This instance refused to run:\n{why}")
                }
                _ => "Which entity this graph previews on.\n\
                      Click to pick another; the controls drive its parameters live."
                    .to_string(),
            };
            ui.tooltip_for(chip, &tip);
        }
    }

    // Controls, only for a live binding: sliders against a refused runtime
    // (or nothing) would be dead weight pretending to work.
    if let Some(a) = anim.filter(|a| a.disabled.is_none()) {
        anim_param_controls(ui, band, chip.max.x + pad * 2.0, state, registry, a);
    }
    (chip, just_opened)
}

/// One control per declared parameter, flowing left to right from `x0`;
/// parameters past the band's width elide to a mono `+n` — the chip
/// summarizer's discipline, not a scroll surface in a footer.
fn anim_param_controls(
    ui: &mut Ui,
    band: Rect,
    x0: f32,
    state: &mut GraphEditorState,
    registry: &NodeRegistry,
    a: &AnimPreview,
) {
    use crate::engine::animation::graph::{trigger_pin_type, ParamValue};
    let st = ui.style();
    let s = (st.metrics.row_height / BASE_ROW_H).max(0.1);
    let pad = BASE_PAD_X * s;
    let px = st.fonts.small;
    let cy = band.center().y;
    let seg_h = st.metrics.control_height.min(band.height() - 6.0 * s);
    let slider_w = PREVIEW_SLIDER_W * s;
    let fire_label = "FIRE";
    let trigger_col = pin_color(Some(registry), &trigger_pin_type());
    let mut x = x0;

    for (i, p) in a.params.iter().enumerate() {
        // The declaration's label, by slug — the strip lists the *runtime's*
        // parameters (what edits can actually drive), the document supplies
        // the author-facing name while the two agree.
        let label = state
            .doc
            .variables
            .iter()
            .find(|v| v.slug == p.slug)
            .map(|v| v.label.clone())
            .unwrap_or_else(|| p.slug.clone());
        let (label_w, value_w, fire_w) = {
            let mut painter = ui.painter();
            (
                painter.measure_text(&label, px, None).x,
                painter.measure_text_family("00.00", px, None, FontFamily::Mono).x,
                painter
                    .measure_text_family(fire_label, px, None, FontFamily::Mono)
                    .x,
            )
        };
        let w = match p.value {
            ParamValue::Float(_) => label_w + pad + slider_w + pad * 0.5 + value_w,
            ParamValue::Bool(_) => label_w + pad * 0.75 + st.sizes.checkbox,
            ParamValue::Trigger(_) => label_w + pad * 0.75 + fire_w + pad * 1.6,
        };
        // Out of room: say how many the band is not showing, and stop.
        if x + w > band.max.x - pad {
            let n = a.params.len() - i;
            ui.painter().text_family(
                Pos2::new(x, cy - px * 0.62),
                &format!("+{n}"),
                px,
                st.palette.text_disabled,
                None,
                FontFamily::Mono,
            );
            break;
        }

        match p.value {
            ParamValue::Float(v) => {
                ui.painter().text(
                    Pos2::new(x, cy - px * 0.62),
                    &label,
                    px,
                    st.palette.text_secondary,
                    None,
                );
                x += label_w + pad;
                let mut val = v;
                let slider = Rect::from_min_size(
                    Pos2::new(x, cy - st.sizes.slider_height * 0.5),
                    Vec2::new(slider_w, st.sizes.slider_height),
                );
                ui.run_at(
                    slider,
                    Direction::LeftToRight,
                    Id::new(("anim_preview_float", p.slug.as_str())),
                    UiOptions { padding: Vec2::ZERO, spacing: 0.0 },
                    |ui| {
                        Slider::new(&mut val, p.range.0..=p.range.1)
                            .width(slider_w)
                            .show(ui);
                    },
                );
                if val != v {
                    state
                        .anim_edits
                        .push(AnimParamEdit::SetFloat(p.slug.clone(), val));
                }
                x += slider_w + pad * 0.5;
                ui.painter().text_family(
                    Pos2::new(x, cy - px * 0.62),
                    &format!("{val:.2}"),
                    px,
                    st.palette.text_mono,
                    None,
                    FontFamily::Mono,
                );
                x += value_w;
            }
            ParamValue::Bool(v) => {
                // The label is painted like every other param's, so the four
                // control families share one typography; the box alone is the
                // widget.
                ui.painter().text(
                    Pos2::new(x, cy - px * 0.62),
                    &label,
                    px,
                    st.palette.text_secondary,
                    None,
                );
                x += label_w + pad * 0.75;
                let mut val = v;
                let row = Rect::from_min_size(
                    Pos2::new(x, cy - st.sizes.checkbox * 0.5),
                    Vec2::new(st.sizes.checkbox, st.sizes.checkbox),
                );
                ui.run_at(
                    row,
                    Direction::LeftToRight,
                    Id::new(("anim_preview_bool", p.slug.as_str())),
                    UiOptions { padding: Vec2::ZERO, spacing: 0.0 },
                    |ui| {
                        Checkbox::new(&mut val, "").show(ui);
                    },
                );
                if val != v {
                    state
                        .anim_edits
                        .push(AnimParamEdit::SetBool(p.slug.clone(), val));
                }
                x += st.sizes.checkbox;
            }
            ParamValue::Trigger(lit) => {
                ui.painter().text(
                    Pos2::new(x, cy - px * 0.62),
                    &label,
                    px,
                    st.palette.text_secondary,
                    None,
                );
                x += label_w + pad * 0.75;
                // FIRE holds the trigger's ember while the shot is buffered
                // — lit until a transition consumes it, read, not
                // remembered.
                let b = Rect::from_min_size(
                    Pos2::new(x, cy - seg_h * 0.5),
                    Vec2::new(fire_w + pad * 1.6, seg_h),
                );
                let id = ui.alloc_id(("anim_preview_fire", p.slug.as_str()));
                let resp = ui.interact(id, b);
                let (fill, stroke, text_col) = if lit {
                    (trigger_col.with_alpha(0.18), trigger_col, trigger_col)
                } else if resp.hovered {
                    (st.palette.hover, st.palette.stroke_strong, st.palette.text)
                } else {
                    (Color::TRANSPARENT, st.palette.stroke, st.palette.text_secondary)
                };
                {
                    let mut painter = ui.painter();
                    painter.rect_filled(b, st.rounding.small, fill);
                    painter.rect_stroke(b, st.rounding.small, st.metrics.border, stroke);
                    painter.text_family(
                        Pos2::new(b.min.x + pad * 0.8, b.center().y - px * 0.62),
                        fire_label,
                        px,
                        text_col,
                        None,
                        FontFamily::Mono,
                    );
                }
                if resp.hovered {
                    ui.tooltip_for(
                        b,
                        if lit {
                            "Buffered — stays set until a transition consumes it"
                        } else {
                            "Fire the trigger (buffered until consumed)"
                        },
                    );
                }
                if resp.clicked {
                    state
                        .anim_edits
                        .push(AnimParamEdit::FireTrigger(p.slug.clone()));
                }
                x += b.width();
            }
        }
        x += pad * 1.6;
    }
}

fn anim_picker_modal_id() -> crusty_gui::id::Id {
    crusty_gui::id::Id::ROOT.with("anim_preview_picker")
}

/// The PREVIEW chip's dropdown: every entity this graph could preview on,
/// nearest first, opening **upward** — the chip lives in the footer band.
/// Returns `Some(binding)` when a row was chosen; `Some(None)` is "follow
/// the selection", the baseline rule and the way back out of an explicit
/// pick.
fn anim_preview_picker(
    ui: &mut Ui,
    anchor: Rect,
    instances: &[ExecInstance],
    anim: Option<&AnimPreview>,
    st: &Style,
    s: f32,
    // The press that opened the picker lands *outside* it — without this the
    // opening click would read as a dismissal.
    just_opened: bool,
) -> Option<Option<u64>> {
    let pad = BASE_PAD_X * s;
    let px = st.fonts.small;
    let row_h = st.metrics.control_height;
    let w = (VARS_W * s).max(anchor.width());
    let h = row_h * (instances.len() as f32 + 1.0) + row_h * 0.9;
    let panel = Rect::from_min_size(
        Pos2::new(anchor.min.x, anchor.min.y - pad * 0.25 - h),
        Vec2::new(w, h),
    );
    ui.ctx_mut().modal_push(anim_picker_modal_id(), panel);
    {
        let mut p = ui.painter();
        p.rect_filled(panel, st.rounding.widget, st.palette.elevated);
        p.rect_stroke(panel, st.rounding.widget, st.metrics.border, st.palette.stroke_strong);
    }
    let mut picked: Option<Option<u64>> = None;
    let bound = anim.map(|a| a.instance_id);
    let mut y = panel.min.y;
    for inst in instances {
        let row = Rect::from_min_size(Pos2::new(panel.min.x, y), Vec2::new(w, row_h));
        y += row_h;
        let id = ui.alloc_id(("anim_preview_row", inst.id));
        let resp = ui.interact(id, row);
        let is_bound = bound == Some(inst.id);
        let mut p = ui.painter();
        if is_bound {
            p.rect_filled(row, Rounding::ZERO, st.palette.selection_fill);
        } else if resp.hovered {
            p.rect_filled(row, Rounding::ZERO, st.palette.hover);
        }
        let dot_r = px * 0.28;
        p.circle_filled(
            Pos2::new(row.min.x + pad + dot_r, row.center().y),
            dot_r,
            if inst.killed {
                Palette::invariant_status().error
            } else if is_bound {
                st.palette.accent_active
            } else {
                Palette::invariant_status().success
            },
        );
        // Distance only — machines tick every frame, so recency says
        // nothing here; "refused" is the one state worth words.
        let meta = if is_bound {
            "selected".to_string()
        } else if inst.killed {
            format!("{:.0} m \u{b7} refused", inst.distance)
        } else {
            format!("{:.0} m", inst.distance)
        };
        let mw = p.measure_text_family(&meta, px, None, FontFamily::Mono).x;
        p.text_family(
            Pos2::new(row.max.x - pad - mw, row.center().y - px * 0.62),
            &meta,
            px,
            if is_bound { st.palette.text_mono } else { st.palette.text_disabled },
            None,
            FontFamily::Mono,
        );
        let name = if inst.name.is_empty() { "(unnamed)" } else { inst.name.as_str() };
        let avail = row.max.x - pad * 2.0 - mw - (row.min.x + pad * 2.0 + dot_r * 2.0);
        let name = clip_text(&mut p, name, st.fonts.body, avail);
        p.text(
            Pos2::new(
                row.min.x + pad * 2.0 + dot_r * 2.0,
                row.center().y - st.fonts.body * 0.62,
            ),
            &name,
            st.fonts.body,
            if is_bound { st.palette.selection_text } else { st.palette.text },
            None,
        );
        if resp.clicked {
            picked = Some(Some(inst.id));
        }
    }
    let follow = Rect::from_min_size(Pos2::new(panel.min.x, y), Vec2::new(w, row_h));
    let fid = ui.alloc_id("anim_preview_follow");
    let fresp = ui.interact(fid, follow);
    {
        let mut p = ui.painter();
        if fresp.hovered {
            p.rect_filled(follow, Rounding::ZERO, st.palette.hover);
        }
        p.text(
            Pos2::new(follow.min.x + pad, follow.center().y - px * 0.62),
            "Follow selection",
            px,
            st.palette.text_secondary,
            None,
        );
    }
    if fresp.clicked {
        picked = Some(None);
    }
    let footer = Rect::from_min_size(Pos2::new(panel.min.x, follow.max.y), Vec2::new(w, row_h * 0.9));
    {
        let mut p = ui.painter();
        p.line_segment(
            Pos2::new(footer.min.x, footer.min.y),
            Pos2::new(footer.max.x, footer.min.y),
            st.metrics.border,
            st.palette.stroke,
        );
        p.text(
            Pos2::new(footer.min.x + pad, footer.center().y - px * 0.62),
            &format!(
                "{} entit{} running this graph",
                instances.len(),
                if instances.len() == 1 { "y" } else { "ies" }
            ),
            px,
            st.palette.text_disabled,
            None,
        );
    }
    let pressed_outside = ui.ctx().input.pointer_pressed
        && ui
            .ctx()
            .input
            .pointer_pos
            .is_some_and(|p| !panel.contains(p) && !anchor.contains(p));
    if !just_opened && (ui.ctx().input.key_pressed(Key::Escape) || pressed_outside) {
        ui.ctx_mut().modal_dismiss(anim_picker_modal_id());
        // Keep the binding, close the surface: "put it away" is not "unbind".
        return Some(anim.map(|a| a.instance_id));
    }
    if picked.is_some() {
        ui.ctx_mut().modal_dismiss(anim_picker_modal_id());
    }
    picked
}

/// The live preview highlight (ticket 06): while a preview is bound, the
/// active state carries an accent outline, the outgoing side of an in-flight
/// crossfade fades out with the fade itself, and the transition that fired
/// flashes and decays. Painted over the canvas (and over the rule peek's
/// scrim) from the node geoms — the canvas itself never learns about it.
fn anim_live_highlight(
    ui: &mut Ui,
    canvas: Rect,
    state: &GraphEditorState,
    geoms: &[NodeGeom],
    anim: Option<&AnimPreview>,
) {
    let Some(a) = anim.filter(|a| a.disabled.is_none()) else {
        return;
    };
    let v = state.view;
    let st = ui.style();
    let s = (st.metrics.row_height / BASE_ROW_H).max(0.1);
    let accent = st.palette.accent_active;
    let screen = |r: Rect| {
        Rect::from_min_max(
            Pos2::new(
                canvas.min.x + (r.min.x - v.pan.x) * v.zoom,
                canvas.min.y + (r.min.y - v.pan.y) * v.zoom,
            ),
            Pos2::new(
                canvas.min.x + (r.max.x - v.pan.x) * v.zoom,
                canvas.min.y + (r.max.y - v.pan.y) * v.zoom,
            ),
        )
    };
    // Seconds an instant (zero-duration) fire stays lit — long enough to
    // see, short enough to read as an event rather than a state.
    const FLASH_SECS: f32 = 0.6;
    let mut lit: Vec<(u64, f32)> = Vec::new();
    if let Some(id) = a.active_state {
        lit.push((id, 1.0));
    }
    if let Some((from, w)) = a.fade {
        lit.push((from, (1.0 - w).clamp(0.0, 1.0)));
    }
    if let Some((t, age)) = a.fired {
        // The firing transition holds while its fade runs, else decays.
        let alpha = match a.fade {
            Some(_) => 1.0,
            None => 1.0 - (age / FLASH_SECS).clamp(0.0, 1.0),
        };
        if alpha > 0.0 {
            lit.push((t, alpha));
        }
    }
    for (id, alpha) in lit {
        let Some(g) = geoms.iter().find(|g| g.id == id) else {
            continue;
        };
        let r = screen(g.rect).expand(3.0 * s);
        let visible = r.intersect(canvas);
        if visible.width() <= 0.0 || visible.height() <= 0.0 || alpha <= 0.01 {
            continue;
        }
        let mut p = ui.painter();
        p.rect_stroke(
            r,
            st.rounding.widget,
            (st.metrics.border * 2.0).max(2.0),
            accent.with_alpha(alpha),
        );
        // A soft outer breath, so "live" reads at a glance without a second
        // color: the same accent, wider and quieter.
        p.rect_stroke(
            r.expand(3.0 * s),
            st.rounding.widget,
            st.metrics.border,
            accent.with_alpha(alpha * 0.35),
        );
    }
}

// ---------------------------------------------------------------------------
// Variables strip (45-A P6c)
// ---------------------------------------------------------------------------

/// Expanded width of the variables strip, base units. Wide enough that a
/// two-word name, a `Vec3[]` tag and the usage count sit in three columns
/// without crowding the right edge (review finding, 2026-08-11).
const VARS_W: f32 = 240.0;
/// Collapsed rail width: the caret that brings the strip back, and the count.
const VARS_RAIL_W: f32 = 22.0;
/// Type chip side, base units.
const VARS_CHIP: f32 = 9.0;

/// What a dragged variable row carries onto the canvas.
#[derive(Clone)]
struct VarDragPayload {
    slug: String,
    label: String,
}

/// The element types the add / retype pickers offer, in order — per domain.
///
/// Script graphs: the scalar set plus `Entity` — `Exec` is not data, `Enum`
/// has no variant list to declare here, and textures/meshes/domain types
/// belong to the editors that own them. The `Array` checkbox wraps whichever
/// one is picked, which is how `Float[]` is authored without a second list.
///
/// Animation graphs: exactly the parameter contract (Task 41) — Float, Bool
/// and Trigger, nothing else. The compiler refuses every other type, so the
/// picker offering one would be a lie; Trigger is the domain-typed one-shot
/// (`plan::trigger_pin_type`).
fn var_types(domain: GraphDomain) -> &'static [(&'static str, PinType)] {
    use std::sync::OnceLock;
    match domain {
        GraphDomain::Script => {
            static TYPES: OnceLock<Vec<(&'static str, PinType)>> = OnceLock::new();
            TYPES.get_or_init(|| {
                vec![
                    ("Float", PinType::Float),
                    ("Int", PinType::Int),
                    ("String", PinType::String),
                    ("Bool", PinType::Bool),
                    ("Vec3", PinType::Vec3),
                    ("Entity", PinType::Entity),
                ]
            })
        }
        // The rule canvas declares against the same parameter contract as
        // the machine — a parameter added mid-rule is a machine parameter.
        GraphDomain::Animation | GraphDomain::AnimationRule { .. } => {
            static TYPES: OnceLock<Vec<(&'static str, PinType)>> = OnceLock::new();
            TYPES.get_or_init(|| {
                vec![
                    ("Float", PinType::Float),
                    ("Bool", PinType::Bool),
                    ("Trigger", crate::engine::animation::graph::trigger_pin_type()),
                ]
            })
        }
    }
}

/// The tag a variable row shows: `Float`, `Float[]`, and for anything the
/// picker cannot make, its honest type slug. One spelling, shared with the
/// view-model's reason lines — a row and the warning about it must not
/// disagree about what a type is called.
fn type_label(ty: &PinType) -> String {
    pin_type_label(ty)
}

/// Split a declared type into what the two controls hold: (picker index,
/// array). A type the picker cannot express answers index 0 — the row's tag
/// still shows the truth, and touching the picker is then an explicit retype.
fn type_pick(domain: GraphDomain, ty: &PinType) -> (usize, bool) {
    let (elem, array) = match ty {
        PinType::Array(inner) => (inner.as_ref().clone(), true),
        other => (other.clone(), false),
    };
    let i = var_types(domain)
        .iter()
        .position(|(_, t)| *t == elem)
        .unwrap_or(0);
    (i, array)
}

/// Rebuild a `PinType` from the two controls.
fn type_from_pick(domain: GraphDomain, index: usize, array: bool) -> PinType {
    let ty = var_types(domain)
        .get(index)
        .map(|(_, t)| t.clone())
        .unwrap_or(PinType::Float);
    if array {
        PinType::Array(Box::new(ty))
    } else {
        ty
    }
}

/// A button drawn at an exact rect (the strip lays itself out by hand, so the
/// buttons have to as well).
fn strip_button(ui: &mut Ui, id: Id, rect: Rect, label: &str, variant: u8) -> bool {
    let mut clicked = false;
    ui.run_at(
        rect,
        Direction::TopDown,
        id,
        UiOptions { padding: Vec2::ZERO, spacing: 0.0 },
        |ui| {
            let b = Button::new(label).exact_size(rect.size());
            let b = match variant {
                1 => b.primary(),
                // Outline, not filled: a filled danger button is the design
                // system's dialog treatment, and a row-level control that
                // shouts red before anything is at stake is noise. The weight
                // belongs on the confirmation, which is where the loss is.
                2 => b.danger_outline(),
                _ => b.ghost(),
            };
            clicked = b.show(ui).clicked;
        },
    );
    clicked
}

/// One edit the strip asks for. Collected during the draw and applied after,
/// so the drawing pass never holds a borrow across a document mutation — and
/// so every mutation goes through one `match` that is easy to read against the
/// P6b model ops.
enum VarRequest {
    Add(String, PinType),
    Rename(String, String),
    /// Retype straight through (no uses) or after a confirmation.
    Retype(String, PinType),
    /// Ask first: the count is the reason to ask.
    ConfirmRetype(String, PinType, usize),
    ConfirmDelete(String, usize),
    SetDefault(String, PropValue),
    /// Assign (or clear) the panel group — display metadata (GS-2).
    SetGroup(String, Option<String>),
    /// Move a declaration to a new index. The only order-changing gesture.
    Reorder(usize, usize),
    AddEntry(String),
    RemoveEntry(String, usize),
    MoveEntry(String, usize, usize),
    SetEntry(String, usize, PropValue),
}

/// Where a row dragged inside the strip would land.
#[derive(Debug, Clone, PartialEq)]
enum VarDropTarget {
    /// Between rows: insert the dragged declaration at this index.
    Before(usize),
    /// Onto a header: assign that group (`None` = the ungrouped section).
    Group(Option<String>),
}

/// Row height for a declaration that carries a mismatch reason line — one
/// caption line taller, per the design's reason-at-rest rule.
const VARS_MISMATCH_ROW: f32 = 30.0 / 22.0;
/// The usage/locate gutter's width, base units.
const VARS_GUTTER: f32 = 26.0;
/// Array literal editor: rows visible before it scrolls.
const VARS_ARRAY_ROWS: usize = 6;

/// The variables side strip: a column beside the canvas listing the document's
/// declarations, with the whole authoring loop on it — add, rename, retype,
/// edit the default, group, reorder, delete, locate, and drag a row onto the
/// canvas.
///
/// **It contains no edit logic of its own.** Every mutation is one of the
/// `GraphEditorState` variable operations, which is what keeps undo,
/// validation and the no-coercion retype rule identical whether an edit came
/// from here, from a config row, or from a test. Layout is by hand because the
/// strip is a fixed column with a pinned footer: the list gets whatever the
/// header, the filter and the footer inspector leave it.
fn variables_panel(
    ui: &mut Ui,
    rect: Rect,
    state: &mut GraphEditorState,
    registry: &NodeRegistry,
    locate: &mut Option<u64>,
) {
    let st = ui.style();
    let pad = st.spacing.padding;
    let s = (st.metrics.row_height / BASE_ROW_H).max(0.1);
    let row_h = st.metrics.row_height;
    let small = st.fonts.small;
    let head_h = st.metrics.control_height;
    {
        let mut p = ui.painter();
        p.rect_filled(rect, Rounding::ZERO, st.palette.panel);
        p.line_segment(
            Pos2::new(rect.max.x, rect.min.y),
            Pos2::new(rect.max.x, rect.max.y),
            st.metrics.border,
            st.palette.stroke,
        );
    }

    if !state.vars.open {
        // Collapsed: a rail carrying the caret back to the list and the count.
        // 22px that hides "this graph has 10 variables" is 22px wasted.
        let btn = Rect::from_min_size(
            Pos2::new(rect.min.x + st.metrics.border, rect.min.y + pad * 0.5),
            Vec2::new(rect.width() - st.metrics.border * 2.0, head_h),
        );
        if strip_button(ui, Id::new("graph_vars_expand"), btn, "\u{203a}", 0) {
            state.vars.open = true;
        }
        let n = state.doc.variables.len().to_string();
        let mut p = ui.painter();
        let w = p.measure_text_family(&n, small, None, FontFamily::Mono).x;
        p.text_family(
            Pos2::new(rect.center().x - w * 0.5, btn.max.y + pad * 0.5),
            &n,
            small,
            st.palette.text_disabled,
            None,
            FontFamily::Mono,
        );
        return;
    }

    // ── The frame's derived state, computed once ─────────────────────────
    let vars: Vec<crate::engine::node_graph::VarDecl> = state.doc.variables.clone();
    let uses: Vec<usize> = vars
        .iter()
        .map(|v| state.variable_usage_count(&v.slug))
        .collect();
    let mismatch: Vec<Option<String>> = vars
        .iter()
        .map(|v| variable_mismatch(&state.doc, &v.slug, &state.errors))
        .collect();
    let view = variables_view(&state.doc, &state.vars.filter, &state.vars.collapsed);
    let selected = state.vars.selected.clone();
    let sel_index = selected
        .as_ref()
        .and_then(|slug| vars.iter().position(|v| v.slug == *slug));
    let mut request: Option<VarRequest> = None;
    let mut collapse_strip = false;
    let mut select: Option<Option<String>> = None;
    let mut toggle_group: Option<String> = None;
    let mut new_var = state.vars.new_var.clone();
    let mut rename_buf = state.vars.rename_buf.clone();
    let mut filter = state.vars.filter.clone();
    let mut new_group = state.vars.new_group.clone();
    let mut drop_target: Option<VarDropTarget> = None;

    // ── Header ───────────────────────────────────────────────────────────
    let header = Rect::from_min_size(rect.min, Vec2::new(rect.width(), head_h));
    {
        let mut p = ui.painter();
        p.rect_filled(header, Rounding::ZERO, st.palette.header);
        p.line_segment(
            Pos2::new(header.min.x, header.max.y),
            Pos2::new(header.max.x, header.max.y),
            st.metrics.border,
            st.palette.stroke,
        );
        p.text_family(
            Pos2::new(header.min.x + pad * 1.5, header.center().y - small * 0.62),
            "VARIABLES",
            small,
            st.palette.text_secondary,
            None,
            FontFamily::Mono,
        );
        let label_w = p
            .measure_text_family("VARIABLES", small, None, FontFamily::Mono)
            .x;
        p.text_family(
            Pos2::new(
                header.min.x + pad * 1.5 + label_w + pad * 0.5,
                header.center().y - small * 0.62,
            ),
            &vars.len().to_string(),
            small,
            st.palette.text_disabled,
            None,
            FontFamily::Mono,
        );
    }
    let caret = Rect::from_min_size(
        Pos2::new(header.min.x, header.min.y),
        Vec2::new(pad * 1.5, head_h),
    );
    if strip_button(ui, Id::new("graph_vars_collapse"), caret, "\u{2039}", 0) {
        collapse_strip = true;
    }
    // The `+` is a menu: a variable is one thing to add, a group is the other,
    // and a group is only ever a label on a declaration — so it needs one.
    let plus = Rect::from_min_size(
        Pos2::new(header.max.x - head_h - pad * 0.25, header.min.y),
        Vec2::splat(head_h),
    );
    let mut want_group = false;
    ui.run_at(
        plus,
        Direction::TopDown,
        Id::new("graph_vars_add_menu"),
        UiOptions { padding: Vec2::ZERO, spacing: 0.0 },
        |ui| {
            // The width is the *dropdown's*, not the button's — a menu as
            // narrow as the + glyph would be unreadable.
            ui.menu_button_width("+", VARS_W * s * 0.75, |ui| {
                if ui.menu_item("Add Variable") {
                    new_var = match new_var {
                        Some(_) => None,
                        None => Some(NewVarDraft { first_frame: true, ..Default::default() }),
                    };
                }
                // A group with no members cannot exist — it is metadata on a
                // declaration — so creating one needs a row to put in it.
                if ui.menu_item_enabled("New Group\u{2026}", selected.is_some()) {
                    want_group = true;
                }
            });
        },
    );
    if want_group {
        new_group = Some(String::new());
    }

    // ── Filter ───────────────────────────────────────────────────────────
    let filter_row = Rect::from_min_size(
        Pos2::new(rect.min.x, header.max.y),
        Vec2::new(rect.width(), head_h + pad),
    );
    {
        let mut p = ui.painter();
        p.line_segment(
            Pos2::new(filter_row.min.x, filter_row.max.y),
            Pos2::new(filter_row.max.x, filter_row.max.y),
            st.metrics.border,
            st.palette.stroke,
        );
    }
    let field = Rect::from_min_size(
        Pos2::new(filter_row.min.x + pad * 0.5, filter_row.min.y + pad * 0.5),
        Vec2::new(rect.width() - pad, head_h),
    );
    let mut filter_cleared = false;
    ui.run_at(
        field,
        Direction::TopDown,
        Id::new("graph_vars_filter"),
        UiOptions { padding: Vec2::ZERO, spacing: 0.0 },
        |ui| {
            let out = TextEdit::new(&mut filter)
                .hint("\u{2315} Filter variables")
                .width(field.width())
                .show_full(ui);
            filter_cleared = out.cancelled;
        },
    );
    if !filter.trim().is_empty() {
        // The count belongs *in* the field — the find-in-graph idiom.
        let text = format!("{}/{}", view.matches, view.total);
        let mut p = ui.painter();
        let w = p.measure_text_family(&text, small, None, FontFamily::Mono).x;
        p.text_family(
            Pos2::new(field.max.x - w - pad * 0.5, field.center().y - small * 0.62),
            &text,
            small,
            st.palette.text_disabled,
            None,
            FontFamily::Mono,
        );
    }

    // ── Footer inspector: measured first, because it is pinned ───────────
    let footer_h = sel_index
        .map(|i| footer_height(&vars[i], &st, s))
        .unwrap_or(0.0);
    let footer = Rect::from_min_max(
        Pos2::new(rect.min.x, rect.max.y - footer_h),
        rect.max,
    );
    let list = Rect::from_min_max(
        Pos2::new(rect.min.x, filter_row.max.y),
        Pos2::new(rect.max.x, rect.max.y - footer_h),
    );

    // ── The list ─────────────────────────────────────────────────────────
    // Row rects, collected as they are drawn, so the drop target can be
    // resolved from the pointer without laying anything out twice.
    let mut row_rects: Vec<(usize, Rect)> = Vec::new();
    let mut header_rects: Vec<(Option<String>, Rect)> = Vec::new();
    let dragging_row = ui.dnd_hovering::<VarDragPayload>(list);
    ui.run_at(
        list,
        Direction::TopDown,
        Id::new("graph_vars_list"),
        UiOptions { padding: Vec2::new(pad * 0.5, pad * 0.5), spacing: 0.0 },
        |ui| {
            if vars.is_empty() && new_var.is_none() {
                empty_state(ui, list, &st);
                return;
            }
            ScrollArea::new(list.height() - pad)
                .inset(0.0)
                .spacing(0.0)
                .show(ui, |ui| {
                // The width the scroll area actually offers: it reserves a
                // scrollbar gutter the moment the content overflows, and a row
                // laid out to the panel width would slide its usage count
                // underneath it.
                let w = ui.available().width();
                // The add draft sits at the top of the list, unchanged from
                // the baseline: name, type, array, Cancel/Add.
                if let Some(draft) = new_var.as_mut() {
                    if let Some(req) = add_draft_block(ui, w, &st, pad, state.domain, draft) {
                        request = Some(req);
                    }
                }
                if new_var.as_ref().is_some_and(|d| d.done) {
                    new_var = None;
                }
                for row in &view.rows {
                    match row {
                        VarListRow::Group { name, count, collapsed } => {
                            let r = ui.allocate(Vec2::new(w, st.metrics.control_height * 0.8));
                            header_rects.push((name.clone(), r));
                            let hot = dragging_row && ui.dnd_hovering::<VarDragPayload>(r);
                            if hot {
                                drop_target = Some(VarDropTarget::Group(name.clone()));
                            }
                            group_header(ui, r, name.as_deref(), *count, *collapsed, hot, &st);
                            let id = ui.alloc_id(("graph_var_group", name.clone()));
                            if ui.interact(id, r).clicked {
                                toggle_group =
                                    Some(name.clone().unwrap_or_default());
                            }
                        }
                        VarListRow::Var(i) => {
                            let i = *i;
                            let v = &vars[i];
                            let tall = mismatch[i].is_some();
                            let h = if tall { row_h * VARS_MISMATCH_ROW } else { row_h };
                            let r = ui.allocate(Vec2::new(w, h));
                            row_rects.push((i, r));
                            let is_sel = selected.as_deref() == Some(v.slug.as_str());
                            let id = ui.alloc_id(("graph_var_row", v.slug.as_str()));
                            let resp = ui.interact(id, r);
                            let dragged = ui.dnd_drag_source::<VarDragPayload>(
                                id,
                                r,
                                v.label.clone(),
                                || VarDragPayload {
                                    slug: v.slug.clone(),
                                    label: v.label.clone(),
                                },
                            );
                            // The usage gutter is the locate affordance: hover
                            // swaps the count for ◎, click cycles the uses.
                            let gutter = Rect::from_min_max(
                                Pos2::new(r.max.x - VARS_GUTTER * s, r.min.y),
                                Pos2::new(r.max.x, r.min.y + row_h),
                            );
                            let over_gutter = ui
                                .ctx()
                                .input
                                .pointer_pos
                                .is_some_and(|p| gutter.contains(p));
                            if resp.clicked && over_gutter && uses[i] > 0 {
                                *locate = state.next_locate(&v.slug);
                            } else if resp.clicked {
                                select = Some(if is_sel { None } else { Some(v.slug.clone()) });
                            }
                            var_row(
                                ui,
                                r,
                                v,
                                uses[i],
                                mismatch[i].as_deref(),
                                is_sel,
                                resp.hovered && !dragged,
                                over_gutter,
                                &filter,
                                registry,
                                &st,
                                s,
                            );
                        }
                    }
                }
                if view.hidden > 0 {
                    let r = ui.allocate(Vec2::new(w, row_h));
                    ui.painter().text(
                        Pos2::new(r.min.x + pad * 0.5, r.center().y - small * 0.62),
                        &format!("{} hidden by filter \u{2014} Esc to clear", view.hidden),
                        small,
                        st.palette.text_disabled,
                        None,
                    );
                }
            });
        },
    );

    // Between-rows insertion: the 2px accent line the design asks for, and
    // the index the drop will use. Resolved from the pointer against the row
    // rects collected above — one source for the preview and the commit.
    if dragging_row {
        if let Some(p) = ui.ctx().input.pointer_pos {
            if drop_target.is_none() {
                if let Some(t) = insertion_target(&row_rects, p) {
                    drop_target = Some(VarDropTarget::Before(t));
                }
            }
        }
    }
    if let Some(target) = &drop_target {
        match target {
            VarDropTarget::Before(i) => {
                let y = row_rects
                    .iter()
                    .find(|(idx, _)| idx == i)
                    .map(|(_, r)| r.min.y)
                    .or_else(|| row_rects.last().map(|(_, r)| r.max.y));
                if let Some(y) = y {
                    ui.painter().line_segment(
                        Pos2::new(list.min.x + pad * 0.5, y),
                        Pos2::new(list.max.x - pad * 0.5, y),
                        st.metrics.edge_accent,
                        st.palette.accent_active,
                    );
                }
            }
            VarDropTarget::Group(_) => {}
        }
    }
    // The drop itself. A row dropped on the strip never reaches the canvas —
    // the two targets are disjoint rects, so only one of them claims it.
    if let Some(p) = ui.dnd_drop_target::<VarDragPayload>(list) {
        match drop_target.clone() {
            Some(VarDropTarget::Group(g)) => {
                request = Some(VarRequest::SetGroup(p.slug, g));
            }
            Some(VarDropTarget::Before(to)) => {
                if let Some(from) = vars.iter().position(|v| v.slug == p.slug) {
                    // Dropping into another group's run assigns as well as
                    // moves: the row lands where it was dropped, and a row
                    // that reads under a header belongs to it.
                    let group = vars.get(to.min(vars.len() - 1)).and_then(|v| v.group.clone());
                    let to = if from < to { to.saturating_sub(1) } else { to };
                    if vars[from].group != group {
                        request = Some(VarRequest::SetGroup(p.slug.clone(), group));
                    } else if from != to {
                        request = Some(VarRequest::Reorder(from, to));
                    }
                }
            }
            None => {}
        }
    }

    // ── Footer inspector ─────────────────────────────────────────────────
    if let Some(i) = sel_index {
        let v = &vars[i];
        if rename_buf.is_none() {
            rename_buf = Some(v.label.clone());
        }
        let buf = rename_buf.get_or_insert_with(|| v.label.clone());
        if let Some(req) = footer_inspector(
            ui, footer, v, uses[i], buf, &st, pad, s, registry, locate, state,
        ) {
            request = Some(req);
        }
    }

    // ── New-group name entry (from the + menu) ───────────────────────────
    if let (Some(name), Some(slug)) = (new_group.as_mut(), selected.as_ref()) {
        let entry = Rect::from_min_size(
            Pos2::new(rect.min.x + pad * 0.5, filter_row.max.y + pad * 0.5),
            Vec2::new(rect.width() - pad, head_h),
        );
        let (mut done, mut cancelled) = (false, false);
        ui.run_at(
            entry,
            Direction::TopDown,
            Id::new("graph_vars_new_group"),
            UiOptions { padding: Vec2::ZERO, spacing: 0.0 },
            |ui| {
                let out = TextEdit::new(name)
                    .hint("New group\u{2026}")
                    .width(entry.width())
                    .request_focus(true)
                    .show_full(ui);
                done = out.submitted;
                cancelled = out.cancelled;
            },
        );
        if done {
            let text = name.trim().to_string();
            if !text.is_empty() {
                request = Some(VarRequest::SetGroup(slug.clone(), Some(text)));
            }
            new_group = None;
        } else if cancelled {
            new_group = None;
        }
    } else if new_group.is_some() {
        new_group = None;
    }

    // ── Write session state back, then apply the one request ─────────────
    state.vars.new_var = new_var.filter(|d| !d.done);
    state.vars.rename_buf = rename_buf;
    state.vars.new_group = new_group;
    if filter_cleared {
        state.vars.filter.clear();
    } else {
        state.vars.filter = filter;
    }
    if collapse_strip {
        state.vars.open = false;
    }
    if let Some(name) = toggle_group {
        if !state.vars.collapsed.remove(&name) {
            state.vars.collapsed.insert(name);
        }
    }
    if let Some(sel) = select {
        state.vars.selected = sel;
        state.vars.rename_buf = None;
    }
    match request {
        Some(VarRequest::Add(name, ty)) => {
            let slug = state.add_variable(&name, ty, registry);
            state.vars.selected = Some(slug);
            state.vars.rename_buf = None;
        }
        Some(VarRequest::Rename(slug, label)) => {
            state.rename_variable(&slug, &label, registry);
        }
        Some(VarRequest::Retype(slug, ty)) => {
            state.retype_variable(&slug, ty, registry);
        }
        Some(VarRequest::ConfirmRetype(slug, ty, uses)) => {
            state.vars.confirm = Some(VarConfirm::Retype { slug, ty, uses });
        }
        Some(VarRequest::ConfirmDelete(slug, uses)) => {
            state.vars.confirm = Some(VarConfirm::Delete { slug, uses });
        }
        Some(VarRequest::SetDefault(slug, value)) => {
            state.set_variable_default(&slug, Some(value), registry);
        }
        Some(VarRequest::SetGroup(slug, group)) => {
            state.set_variable_group(&slug, group, registry);
        }
        Some(VarRequest::Reorder(from, to)) => {
            state.reorder_variable(from, to, registry);
        }
        Some(VarRequest::AddEntry(slug)) => {
            state.add_array_entry(&slug, registry);
        }
        Some(VarRequest::RemoveEntry(slug, i)) => {
            state.remove_array_entry(&slug, i, registry);
        }
        Some(VarRequest::MoveEntry(slug, from, to)) => {
            state.move_array_entry(&slug, from, to, registry);
        }
        Some(VarRequest::SetEntry(slug, i, value)) => {
            state.set_array_entry(&slug, i, value, registry);
        }
        None => {}
    }
}

/// Which insertion index a pointer at `p` names: the row it is over, or the one
/// after it when the pointer is past that row's midpoint.
fn insertion_target(rows: &[(usize, Rect)], p: Pos2) -> Option<usize> {
    let (_, first) = rows.first()?;
    if p.x < first.min.x || p.x > first.max.x {
        return None;
    }
    for (i, r) in rows {
        if p.y < r.center().y {
            return Some(*i);
        }
        if p.y <= r.max.y {
            return Some(i + 1);
        }
    }
    rows.last().map(|(i, _)| i + 1)
}

/// The empty state — it names both gestures rather than saying "nothing here".
fn empty_state(ui: &mut Ui, rect: Rect, st: &Style) {
    let mut p = ui.painter();
    let line1 = "No variables yet";
    let line2 = "Press + to add one, or drag a";
    let line3 = "value pin here to promote it";
    let cy = rect.center().y - st.fonts.body;
    for (i, (text, px, col)) in [
        (line1, st.fonts.body, st.palette.text_secondary),
        (line2, st.fonts.small, st.palette.text_disabled),
        (line3, st.fonts.small, st.palette.text_disabled),
    ]
    .into_iter()
    .enumerate()
    {
        let w = p.measure_text(text, px, None).x;
        p.text(
            Pos2::new(rect.center().x - w * 0.5, cy + i as f32 * px * 1.5),
            text,
            px,
            col,
            None,
        );
    }
}

/// A group header: caret, mono name, count, and a hairline out to the edge.
/// `hot` is a drag hovering it — the drop-into affordance, a 1px accent border.
fn group_header(
    ui: &mut Ui,
    rect: Rect,
    name: Option<&str>,
    count: usize,
    collapsed: bool,
    hot: bool,
    st: &Style,
) {
    let mut p = ui.painter();
    let small = st.fonts.small;
    p.rect_filled(rect, Rounding::ZERO, st.palette.window);
    if hot {
        p.rect_stroke(rect, st.rounding.small, st.metrics.border, st.palette.accent_active);
    }
    let caret = if collapsed { "\u{25b8}" } else { "\u{25be}" };
    let x = rect.min.x + st.spacing.padding * 0.25;
    let cw = p.measure_text(caret, small, None).x;
    p.text(
        Pos2::new(x, rect.center().y - small * 0.62),
        caret,
        small,
        st.palette.text_disabled,
        None,
    );
    let label = name.map(str::to_uppercase).unwrap_or_else(|| "UNGROUPED".to_string());
    let lx = x + cw + st.spacing.item * 0.5;
    let lw = p
        .text_family(
            Pos2::new(lx, rect.center().y - small * 0.62),
            &label,
            small,
            st.palette.text_secondary,
            None,
            FontFamily::Mono,
        )
        .x;
    let count_x = lx + lw + st.spacing.item * 0.5;
    let cx = p
        .text_family(
            Pos2::new(count_x, rect.center().y - small * 0.62),
            &count.to_string(),
            small,
            st.palette.text_disabled,
            None,
            FontFamily::Mono,
        )
        .x;
    p.line_segment(
        Pos2::new(count_x + cx + st.spacing.item * 0.5, rect.center().y),
        Pos2::new(rect.max.x, rect.center().y),
        st.metrics.border,
        st.palette.stroke,
    );
}

/// One declaration row: type dot, name, type tag (or the mismatch warning),
/// usage gutter. The name is the only body-text element — the list reads as a
/// column of names first.
#[allow(clippy::too_many_arguments)]
fn var_row(
    ui: &mut Ui,
    rect: Rect,
    decl: &crate::engine::node_graph::VarDecl,
    uses: usize,
    mismatch: Option<&str>,
    selected: bool,
    hovered: bool,
    over_gutter: bool,
    filter: &str,
    registry: &NodeRegistry,
    st: &Style,
    s: f32,
) {
    let status = Palette::invariant_status();
    let pad = st.spacing.padding;
    let small = st.fonts.small;
    let mut p = ui.painter();
    let top = Rect::from_min_max(
        rect.min,
        Pos2::new(rect.max.x, rect.min.y + st.metrics.row_height),
    );
    if selected {
        p.rect_filled(rect, st.rounding.small, st.palette.selection_fill);
    } else if mismatch.is_some() {
        // A problem state is visible at rest: the faintest possible wash, so
        // the row reads as flagged without competing with selection.
        //
        // An opaque blend rather than a translucent fill, and a very small
        // factor: channel values reach the GPU as **linear** light, so mixing
        // 8% of a bright warning yellow into a near-black panel lands around
        // 25% once it is displayed. 2% is what reads as the design's hint —
        // measured on a capture, not guessed.
        p.rect_filled(rect, st.rounding.small, fade(status.warning, 0.02, st.palette.panel));
    } else if hovered {
        p.rect_filled(rect, st.rounding.small, st.palette.hover);
    }
    // Type dot: the pin's own colour, so a row and the pin it drops as are
    // recognisably the same thing.
    let c = VARS_CHIP * s * 0.8;
    p.rect_filled(
        Rect::from_center_size(
            Pos2::new(rect.min.x + pad * 0.75 + c * 0.5, top.center().y),
            Vec2::splat(c),
        ),
        Rounding::same(c * 0.5),
        pin_color(Some(registry), &decl.ty),
    );
    // Usage gutter — hover swaps the count for the locate mark.
    let gutter_w = VARS_GUTTER * s;
    let count = format!("{}\u{d7}", uses);
    let count_col = if uses == 0 {
        st.palette.text_disabled
    } else if selected {
        st.palette.selection_text
    } else {
        st.palette.text_secondary
    };
    let cw = p.measure_text_family(&count, small, None, FontFamily::Mono).x;
    p.text_family(
        Pos2::new(rect.max.x - cw - pad * 0.25, top.center().y - small * 0.62),
        &count,
        small,
        if over_gutter && uses > 0 { st.palette.text } else { count_col },
        None,
        FontFamily::Mono,
    );
    if hovered && uses > 0 {
        p.text(
            Pos2::new(rect.max.x - gutter_w, top.center().y - small * 0.62),
            "\u{25ce}",
            small,
            if over_gutter { st.palette.accent_active } else { st.palette.text_secondary },
            None,
        );
    }
    // Type tag, or the warning glyph that replaces it when uses disagree.
    let tag_x = rect.max.x - gutter_w - pad * 0.25;
    let tag_w = if let Some(_reason) = mismatch {
        let g = "\u{25b2}";
        let w = p.measure_text_family(g, small, None, FontFamily::Mono).x;
        p.text_family(
            Pos2::new(tag_x - w, top.center().y - small * 0.62),
            g,
            small,
            status.warning,
            None,
            FontFamily::Mono,
        );
        w
    } else {
        let tag = type_label(&decl.ty);
        let w = p.measure_text_family(&tag, small, None, FontFamily::Mono).x;
        p.text_family(
            Pos2::new(tag_x - w, top.center().y - small * 0.62),
            &tag,
            small,
            st.palette.text_mono,
            None,
            FontFamily::Mono,
        );
        w
    };
    let lx = rect.min.x + pad * 0.75 + c + pad * 0.5;
    let avail = tag_x - tag_w - pad * 0.5 - lx;
    let label = clip_text(&mut p, &decl.label, st.fonts.body, avail);
    // What the filter matched is marked in the name itself — the row says
    // *why* it survived the filter rather than leaving the reader to guess.
    if let Some((a, b)) = filter_hit(&label, filter) {
        let px = st.fonts.body;
        let x0 = lx + p.measure_text(&label[..a], px, None).x;
        let x1 = lx + p.measure_text(&label[..b], px, None).x;
        p.rect_filled(
            Rect::from_min_max(
                Pos2::new(x0 - 1.0, top.center().y - px * 0.62),
                Pos2::new(x1 + 1.0, top.center().y + px * 0.5),
            ),
            st.rounding.small,
            st.palette.accent_soft,
        );
    }
    p.text(
        Pos2::new(lx, top.center().y - st.fonts.body * 0.62),
        &label,
        st.fonts.body,
        if selected { st.palette.selection_text } else { st.palette.text },
        None,
    );
    // The reason line, at rest, in place of nothing — plugin-manager rule.
    if let Some(reason) = mismatch {
        p.text(
            Pos2::new(lx, top.max.y - small * 0.2),
            reason,
            small,
            status.warning,
            None,
        );
    }
}

/// Byte range of the filter's match inside `label`, case-insensitively — what
/// the row highlights. `None` when the filter is empty, or when the match is
/// in the slug rather than in the displayed label.
fn filter_hit(label: &str, filter: &str) -> Option<(usize, usize)> {
    let q = filter.trim().to_lowercase();
    if q.is_empty() {
        return None;
    }
    let at = label.to_lowercase().find(&q)?;
    let end = at + q.len();
    // Byte offsets from a lowercased haystack are only usable as indices into
    // the original when they land on char boundaries — bail rather than slice
    // a name apart mid-glyph.
    (label.is_char_boundary(at) && label.is_char_boundary(end)).then_some((at, end))
}

/// The add-variable draft block — the baseline's three rows (name, type +
/// array, actions), unchanged behaviour: collisions take numeric suffixes,
/// never a dialog.
fn add_draft_block(
    ui: &mut Ui,
    w: f32,
    st: &Style,
    pad: f32,
    domain: GraphDomain,
    draft: &mut NewVarDraft,
) -> Option<VarRequest> {
    let block = ui.allocate(Vec2::new(w, st.metrics.control_height * 3.0 + pad * 3.0));
    {
        let mut p = ui.painter();
        p.rect_filled(block, st.rounding.small, st.palette.window);
        p.rect_stroke(block, st.rounding.small, st.metrics.border, st.palette.stroke_strong);
    }
    let (mut commit, mut cancel) = (false, false);
    ui.run_at(
        Rect::from_min_max(
            block.min + Vec2::splat(pad * 0.5),
            block.max - Vec2::splat(pad * 0.5),
        ),
        Direction::TopDown,
        Id::new("graph_vars_new"),
        UiOptions { padding: Vec2::ZERO, spacing: pad * 0.75 },
        |ui| {
            let fw = w - pad;
            let out = TextEdit::new(&mut draft.name)
                .hint("New variable\u{2026}")
                .width(fw)
                .request_focus(draft.first_frame)
                .show_full(ui);
            commit |= out.submitted;
            cancel |= out.cancelled;
            draft.first_frame = false;
            let row = ui.allocate(Vec2::new(fw, st.metrics.control_height));
            ui.run_at(
                row,
                Direction::LeftToRight,
                Id::new("graph_vars_new_row"),
                UiOptions { padding: Vec2::ZERO, spacing: pad },
                |ui| {
                    let types = var_types(domain);
                    let picked = draft.ty.min(types.len() - 1);
                    ComboBox::new("graph_vars_new_ty")
                        .selected_text(types[picked].0)
                        .width(fw * 0.46)
                        .show_ui(ui, |ui| {
                            for (i, (name, _)) in types.iter().enumerate() {
                                SelectableValue::new(&mut draft.ty, i, *name).show(ui);
                            }
                        });
                    // Animation parameters have no array form — the compiler
                    // refuses one, so the checkbox does not exist there.
                    if domain == GraphDomain::Script {
                        Checkbox::new(&mut draft.array, "Array").show(ui);
                    }
                },
            );
            let bw = (fw - pad) * 0.5;
            let actions = ui.allocate(Vec2::new(fw, st.metrics.control_height));
            ui.run_at(
                actions,
                Direction::LeftToRight,
                Id::new("graph_vars_new_actions"),
                UiOptions { padding: Vec2::ZERO, spacing: pad },
                |ui| {
                    cancel |= Button::new("Cancel")
                        .exact_size(Vec2::new(bw, st.metrics.control_height))
                        .show(ui)
                        .clicked;
                    commit |= Button::new("Add")
                        .primary()
                        .exact_size(Vec2::new(bw, st.metrics.control_height))
                        .show(ui)
                        .clicked;
                },
            );
        },
    );
    if commit && !draft.name.trim().is_empty() {
        draft.done = true;
        return Some(VarRequest::Add(
            draft.name.clone(),
            type_from_pick(domain, draft.ty, draft.array),
        ));
    }
    if cancel {
        draft.done = true;
    }
    None
}

/// How tall the pinned footer is for this declaration: the fixed blocks plus,
/// for an array, its bounded entry list.
fn footer_height(decl: &crate::engine::node_graph::VarDecl, st: &Style, s: f32) -> f32 {
    let pad = st.spacing.padding;
    let base = st.metrics.control_height * 4.0 + pad * 5.0;
    match (&decl.ty, &decl.default) {
        (PinType::Array(inner), _) if PropValue::zero_of(inner).is_some() => {
            let n = match &decl.default {
                Some(PropValue::Array(v)) => v.len(),
                _ => 0,
            };
            let rows = n.clamp(1, VARS_ARRAY_ROWS) as f32;
            base + rows * st.metrics.row_height * 0.9 + st.metrics.control_height + pad * 2.0 * s
        }
        _ => base,
    }
}

/// The pinned footer inspector: the selected declaration's detail, in the node
/// inspector's block order (identity, then type, then value), with the list
/// above it left exactly where it was.
#[allow(clippy::too_many_arguments)]
fn footer_inspector(
    ui: &mut Ui,
    rect: Rect,
    decl: &crate::engine::node_graph::VarDecl,
    uses: usize,
    rename_buf: &mut String,
    st: &Style,
    pad: f32,
    s: f32,
    _registry: &NodeRegistry,
    locate: &mut Option<u64>,
    state: &mut GraphEditorState,
) -> Option<VarRequest> {
    let small = st.fonts.small;
    {
        let mut p = ui.painter();
        p.rect_filled(rect, Rounding::ZERO, st.palette.window);
        p.line_segment(
            Pos2::new(rect.min.x, rect.min.y),
            Pos2::new(rect.max.x, rect.min.y),
            st.metrics.border,
            st.palette.stroke_strong,
        );
    }
    let mut request: Option<VarRequest> = None;
    let inner = Rect::from_min_max(
        rect.min + Vec2::splat(pad * 0.5),
        Pos2::new(rect.max.x - pad * 0.5, rect.max.y - pad * 0.5),
    );
    let label_w = 52.0 * s;
    let ch = st.metrics.control_height;

    // Header line: the name in small caps, the usage + locate link.
    let head = Rect::from_min_size(inner.min, Vec2::new(inner.width(), ch * 0.8));
    {
        let mut p = ui.painter();
        p.text_family(
            Pos2::new(head.min.x, head.center().y - small * 0.62),
            &decl.label.to_uppercase(),
            small,
            st.palette.text_secondary,
            None,
            FontFamily::Mono,
        );
    }
    let usage = if uses == 0 {
        "0\u{d7} \u{b7} unused".to_string()
    } else {
        format!("{uses}\u{d7} \u{b7} locate \u{203a}")
    };
    let (uw, ux) = {
        let mut p = ui.painter();
        let w = p.measure_text_family(&usage, small, None, FontFamily::Mono).x;
        p.text_family(
            Pos2::new(head.max.x - w, head.center().y - small * 0.62),
            &usage,
            small,
            if uses == 0 { st.palette.text_disabled } else { st.palette.accent_active },
            None,
            FontFamily::Mono,
        );
        (w, head.max.x - w)
    };
    if uses > 0 {
        let hit = Rect::from_min_max(
            Pos2::new(ux, head.min.y),
            Pos2::new(head.max.x, head.max.y),
        );
        let id = ui.alloc_id(("graph_var_locate", decl.slug.as_str()));
        let resp = ui.interact(id, hit);
        if resp.hovered {
            ui.tooltip_for(hit, "Frame the next node that uses this variable");
        }
        if resp.clicked {
            *locate = state.next_locate(&decl.slug);
        }
        let _ = uw;
    }

    // Name.
    let mut y = head.max.y + pad * 0.5;
    let name_row = Rect::from_min_size(Pos2::new(inner.min.x, y), Vec2::new(inner.width(), ch));
    field_label(ui, name_row, "Name", label_w, st);
    let mut rename_done = false;
    ui.run_at(
        Rect::from_min_max(Pos2::new(inner.min.x + label_w, y), name_row.max),
        Direction::TopDown,
        Id::new(("graph_var_name", decl.slug.as_str())),
        UiOptions { padding: Vec2::ZERO, spacing: 0.0 },
        |ui| {
            let out = TextEdit::new(rename_buf)
                .width(name_row.width() - label_w)
                .show_full(ui);
            // Commit on Enter or when the field gives the keyboard back —
            // never per keystroke, which would be one undo entry per letter.
            rename_done = out.submitted || !out.focused;
        },
    );
    if rename_done {
        let text = rename_buf.trim().to_string();
        if !text.is_empty() && text != decl.label {
            request = Some(VarRequest::Rename(decl.slug.clone(), text));
        }
    }

    // Type + array.
    y = name_row.max.y + pad * 0.5;
    let ty_row = Rect::from_min_size(Pos2::new(inner.min.x, y), Vec2::new(inner.width(), ch));
    field_label(ui, ty_row, "Type", label_w, st);
    let (mut pick, mut array) = type_pick(state.domain, &decl.ty);
    let (pick0, array0) = (pick, array);
    let domain = state.domain;
    ui.run_at(
        Rect::from_min_max(Pos2::new(inner.min.x + label_w, y), ty_row.max),
        Direction::LeftToRight,
        Id::new(("graph_var_ty", decl.slug.as_str())),
        UiOptions { padding: Vec2::ZERO, spacing: pad * 0.5 },
        |ui| {
            let types = var_types(domain);
            ComboBox::new(format!("graph_var_ty_{}", decl.slug))
                .selected_text(types[pick.min(types.len() - 1)].0)
                .width((ty_row.width() - label_w) * 0.56)
                .show_ui(ui, |ui| {
                    for (i, (name, _)) in types.iter().enumerate() {
                        SelectableValue::new(&mut pick, i, *name).show(ui);
                    }
                });
            if domain == GraphDomain::Script {
                Checkbox::new(&mut array, "Array").show(ui);
            }
        },
    );
    let want = type_from_pick(domain, pick, array);
    if (pick, array) != (pick0, array0) && want != decl.ty && request.is_none() {
        request = Some(if uses > 0 {
            VarRequest::ConfirmRetype(decl.slug.clone(), want, uses)
        } else {
            VarRequest::Retype(decl.slug.clone(), want)
        });
    }

    // Default + delete.
    y = ty_row.max.y + pad * 0.5;
    let d_row = Rect::from_min_size(Pos2::new(inner.min.x, y), Vec2::new(inner.width(), ch));
    field_label(ui, d_row, "Default", label_w, st);
    let del = Rect::from_min_size(Pos2::new(d_row.max.x - ch, d_row.min.y), Vec2::splat(ch));
    let cell = Rect::from_min_max(
        Pos2::new(inner.min.x + label_w, d_row.min.y),
        Pos2::new(del.min.x - pad * 0.5, d_row.max.y),
    );
    let is_array = matches!(decl.ty, PinType::Array(_));
    if is_array {
        let n = match &decl.default {
            Some(PropValue::Array(v)) => v.len(),
            _ => 0,
        };
        ui.painter().text_family(
            Pos2::new(cell.min.x, cell.center().y - small * 0.62),
            &format!("{n} entr{}", if n == 1 { "y" } else { "ies" }),
            small,
            st.palette.text_secondary,
            None,
            FontFamily::Mono,
        );
    } else if PropValue::zero_of(&decl.ty).is_none() {
        // No literal form: the chip says why, instead of a broken "—".
        let text = if state.domain.is_animation_family() {
            "fired by gameplay"
        } else {
            "bound at runtime"
        };
        runtime_chip(ui, cell, st, text);
    } else if let Some(v) = var_default_widget(ui, cell, decl) {
        if request.is_none() {
            request = Some(VarRequest::SetDefault(decl.slug.clone(), v));
        }
    }
    // The one unlabelled control in the block says what it is on hover —
    // tested by containment rather than a second `interact`, which would
    // fight the button for the same press.
    if ui.ctx().input.pointer_pos.is_some_and(|p| del.contains(p)) {
        ui.tooltip_for(del, "Delete variable");
    }
    if strip_button(ui, Id::new(("graph_var_del", decl.slug.as_str())), del, "\u{2715}", 2)
        && request.is_none()
    {
        request = Some(VarRequest::ConfirmDelete(decl.slug.clone(), uses));
    }

    // The array literal editor, bounded and scrolling.
    if is_array {
        let editor = Rect::from_min_max(
            Pos2::new(inner.min.x, d_row.max.y + pad * 0.5),
            inner.max,
        );
        if let Some(req) = array_editor(ui, editor, decl, st, pad, s) {
            if request.is_none() {
                request = Some(req);
            }
        }
    }
    request
}

fn field_label(ui: &mut Ui, row: Rect, text: &str, _label_w: f32, st: &Style) {
    ui.painter().text(
        Pos2::new(row.min.x, row.center().y - st.fonts.small * 0.62),
        text,
        st.fonts.small,
        st.palette.text_secondary,
        None,
    );
}

/// The dashed "bound at runtime" chip an `Entity` default wears. An entity has
/// no literal, and saying so teaches more than an em dash.
fn runtime_chip(ui: &mut Ui, cell: Rect, st: &Style, text: &str) {
    let small = st.fonts.small;
    let mut p = ui.painter();
    let w = p.measure_text(text, small, None).x + st.spacing.padding;
    let chip = Rect::from_min_size(
        Pos2::new(cell.min.x, cell.center().y - st.metrics.control_height * 0.4),
        Vec2::new(w.min(cell.width()), st.metrics.control_height * 0.8),
    );
    dashed_rect(&mut p, chip, st.metrics.border, st.palette.stroke_strong);
    let text = clip_text(&mut p, text, small, chip.width() - st.spacing.padding * 0.5);
    p.text(
        Pos2::new(chip.min.x + st.spacing.padding * 0.25, chip.center().y - small * 0.62),
        &text,
        small,
        st.palette.text_secondary,
        None,
    );
}

/// The array literal editor: one typed row per entry (index, per-component
/// fields, drag handle, ✕), a "+ Entry" action, and a hard bound of six
/// visible rows before it scrolls — an unbounded list would push the rest of
/// the inspector off a 240px strip.
fn array_editor(
    ui: &mut Ui,
    rect: Rect,
    decl: &crate::engine::node_graph::VarDecl,
    st: &Style,
    pad: f32,
    s: f32,
) -> Option<VarRequest> {
    let PinType::Array(elem) = &decl.ty else {
        return None;
    };
    let small = st.fonts.small;
    let entries: Vec<PropValue> = match &decl.default {
        Some(PropValue::Array(v)) => v.clone(),
        _ => Vec::new(),
    };
    // An element type with no literal (Entity[]) has nothing to edit; the
    // chip already said so on the Default row.
    PropValue::zero_of(elem)?;
    let mut request: Option<VarRequest> = None;
    let row_h = st.metrics.row_height * 0.9;
    let visible = entries.len().clamp(1, VARS_ARRAY_ROWS) as f32;
    let box_rect = Rect::from_min_size(
        rect.min,
        Vec2::new(rect.width(), visible * row_h + st.metrics.border * 2.0),
    );
    {
        let mut p = ui.painter();
        p.rect_filled(box_rect, st.rounding.small, st.palette.input);
        p.rect_stroke(box_rect, st.rounding.small, st.metrics.border, st.palette.stroke);
    }
    if entries.is_empty() {
        ui.painter().text(
            Pos2::new(box_rect.min.x + pad * 0.5, box_rect.center().y - small * 0.62),
            "no entries",
            small,
            st.palette.text_disabled,
            None,
        );
    }
    ui.run_at(
        box_rect.shrink(st.metrics.border),
        Direction::TopDown,
        Id::new(("graph_var_array", decl.slug.as_str())),
        UiOptions { padding: Vec2::ZERO, spacing: 0.0 },
        |ui| {
            ScrollArea::new(visible * row_h)
                .inset(0.0)
                .spacing(0.0)
                .show(ui, |ui| {
                let ew = ui.available().width();
                for (i, entry) in entries.iter().enumerate() {
                    let r = ui.allocate(Vec2::new(ew, row_h));
                    {
                        let mut p = ui.painter();
                        if i + 1 < entries.len() {
                            p.line_segment(
                                Pos2::new(r.min.x, r.max.y),
                                Pos2::new(r.max.x, r.max.y),
                                st.metrics.border,
                                st.palette.stroke,
                            );
                        }
                        p.text_family(
                            Pos2::new(r.min.x + pad * 0.25, r.center().y - small * 0.62),
                            &i.to_string(),
                            small,
                            st.palette.text_disabled,
                            None,
                            FontFamily::Mono,
                        );
                    }
                    let idx_w = 12.0 * s;
                    let handle = Rect::from_min_size(
                        Pos2::new(r.max.x - 30.0 * s, r.min.y),
                        Vec2::new(15.0 * s, r.height()),
                    );
                    let kill = Rect::from_min_size(
                        Pos2::new(r.max.x - 15.0 * s, r.min.y),
                        Vec2::new(15.0 * s, r.height()),
                    );
                    let cell = Rect::from_min_max(
                        Pos2::new(r.min.x + idx_w + pad * 0.25, r.min.y + 1.0),
                        Pos2::new(handle.min.x - pad * 0.25, r.max.y - 1.0),
                    );
                    if let Some(v) = array_entry_widget(ui, cell, elem, entry, &decl.slug, i, st) {
                        request = Some(VarRequest::SetEntry(decl.slug.clone(), i, v));
                    }
                    // ⋮⋮ reorders by one step per click — a drag inside a
                    // scrolling six-row box is a worse gesture than a nudge,
                    // and both land as one "Reorder Array Entry" entry.
                    let hid = ui.alloc_id(("graph_var_entry_move", decl.slug.as_str(), i));
                    let hresp = ui.interact(hid, handle);
                    if hresp.hovered {
                        ui.tooltip_for(handle, "Move down (\u{21e7} for up)");
                    }
                    if hresp.clicked {
                        let up = ui.ctx().input.modifiers.contains(Modifiers::SHIFT);
                        let to = if up { i.saturating_sub(1) } else { i + 1 };
                        if to < entries.len() {
                            request = Some(VarRequest::MoveEntry(decl.slug.clone(), i, to));
                        }
                    }
                    let kid = ui.alloc_id(("graph_var_entry_del", decl.slug.as_str(), i));
                    let kresp = ui.interact(kid, kill);
                    ui.painter().text(
                        Pos2::new(handle.center().x - small * 0.3, r.center().y - small * 0.62),
                        "\u{22ee}\u{22ee}",
                        small,
                        if hresp.hovered { st.palette.text } else { st.palette.text_secondary },
                        None,
                    );
                    ui.painter().text(
                        Pos2::new(kill.center().x - small * 0.3, r.center().y - small * 0.62),
                        "\u{2715}",
                        small,
                        if kresp.hovered { st.palette.text } else { st.palette.text_secondary },
                        None,
                    );
                    if kresp.clicked {
                        request = Some(VarRequest::RemoveEntry(decl.slug.clone(), i));
                    }
                }
            });
        },
    );
    let actions = Rect::from_min_size(
        Pos2::new(rect.min.x, box_rect.max.y + pad * 0.6),
        Vec2::new(rect.width(), st.metrics.control_height),
    );
    let add = Rect::from_min_size(actions.min, Vec2::new(64.0 * s, actions.height()));
    if strip_button(ui, Id::new(("graph_var_entry_add", decl.slug.as_str())), add, "+ Entry", 0) {
        request = Some(VarRequest::AddEntry(decl.slug.clone()));
    }
    if entries.len() > VARS_ARRAY_ROWS {
        ui.painter().text(
            Pos2::new(add.max.x + pad * 0.5, actions.center().y - small * 0.62),
            &format!("{} entries \u{2014} scrolls", entries.len()),
            small,
            st.palette.text_disabled,
            None,
        );
    }
    request
}

/// One array entry's value editor, keyed by the element type — the same widget
/// vocabulary the scalar default uses, laid out across the row's components.
fn array_entry_widget(
    ui: &mut Ui,
    cell: Rect,
    elem: &PinType,
    value: &PropValue,
    slug: &str,
    index: usize,
    st: &Style,
) -> Option<PropValue> {
    let mut changed: Option<PropValue> = None;
    let v = value.clone();
    ui.run_at(
        cell,
        Direction::LeftToRight,
        Id::new(("graph_var_entry", slug, index)),
        UiOptions { padding: Vec2::ZERO, spacing: st.spacing.item * 0.25 },
        |ui| match (elem, v) {
            (PinType::Float, PropValue::Float(x)) => {
                let mut n = x;
                DragValue::new(&mut n).width(cell.width()).show(ui);
                if n != x {
                    changed = Some(PropValue::Float(n));
                }
            }
            (PinType::Int, PropValue::Int(x)) => {
                let mut n = x as f32;
                DragValue::new(&mut n)
                    .speed(0.05)
                    .decimals(0)
                    .min_decimals(0)
                    .width(cell.width())
                    .show(ui);
                if n.round() as i32 != x {
                    changed = Some(PropValue::Int(n.round() as i32));
                }
            }
            (PinType::Bool, PropValue::Bool(b)) => {
                let mut n = b;
                Checkbox::new(&mut n, "").show(ui);
                if n != b {
                    changed = Some(PropValue::Bool(n));
                }
            }
            (PinType::String, PropValue::Str(t)) => {
                let mut n = t.clone();
                TextEdit::new(&mut n).width(cell.width()).show(ui);
                if n != t {
                    changed = Some(PropValue::Str(n));
                }
            }
            (PinType::Vec3, PropValue::Vec3(v3)) => {
                let mut out = v3;
                let w = (cell.width() - st.spacing.item * 0.5) / 3.0;
                for x in out.iter_mut() {
                    let mut n = *x;
                    DragValue::new(&mut n).width(w).show(ui);
                    *x = n;
                }
                if out != v3 {
                    changed = Some(PropValue::Vec3(out));
                }
            }
            // Stale data (a hand-edited mixed array): shown, never silently
            // rewritten — the wire's type rule is what reports it.
            (_, other) => {
                ui.painter().text_family(
                    Pos2::new(cell.min.x, cell.center().y - st.fonts.small * 0.62),
                    &prop_display(&other),
                    st.fonts.small,
                    st.palette.text_disabled,
                    None,
                    FontFamily::Mono,
                );
            }
        },
    );
    changed
}

/// The default-value editor for one declaration: the P6a field widgets, keyed
/// by the declared type. The types with no constant form say so rather than
/// showing a zero they do not have.
fn var_default_widget(
    ui: &mut Ui,
    cell: Rect,
    decl: &crate::engine::node_graph::VarDecl,
) -> Option<PropValue> {
    let st = ui.style();
    let mut changed: Option<PropValue> = None;
    let d = decl.default.clone();
    ui.run_at(
        cell,
        Direction::LeftToRight,
        Id::new(("graph_var_default", decl.slug.as_str())),
        UiOptions { padding: Vec2::ZERO, spacing: st.spacing.item * 0.5 },
        |ui| match (&decl.ty, d) {
            (PinType::Float, Some(PropValue::Float(v))) => {
                let mut x = v;
                DragValue::new(&mut x).width(cell.width()).show(ui);
                if x != v {
                    changed = Some(PropValue::Float(x));
                }
            }
            (PinType::Int, Some(PropValue::Int(v))) => {
                let mut x = v as f32;
                DragValue::new(&mut x)
                    .speed(0.05)
                    .decimals(0)
                    .min_decimals(0)
                    .width(cell.width())
                    .show(ui);
                if x.round() as i32 != v {
                    changed = Some(PropValue::Int(x.round() as i32));
                }
            }
            (PinType::Bool, Some(PropValue::Bool(v))) => {
                let mut b = v;
                Checkbox::new(&mut b, "").show(ui);
                if b != v {
                    changed = Some(PropValue::Bool(b));
                }
            }
            (PinType::String, Some(PropValue::Str(v))) => {
                let mut t = v.clone();
                TextEdit::new(&mut t).width(cell.width()).show(ui);
                if t != v {
                    changed = Some(PropValue::Str(t));
                }
            }
            (PinType::Vec3, Some(PropValue::Vec3(v))) => {
                let mut out = v;
                let w = (cell.width() - st.spacing.item) / 3.0;
                for (i, axis) in ["X", "Y", "Z"].iter().enumerate() {
                    let mut x = out[i];
                    DragValue::new(&mut x)
                        .prefix(format!("{axis} "))
                        .width(w)
                        .show(ui);
                    out[i] = x;
                }
                if out != v {
                    changed = Some(PropValue::Vec3(out));
                }
            }
            (_, value) => {
                let text = match value {
                    Some(v) => prop_display(&v),
                    None => "\u{2014}".to_string(),
                };
                ui.painter().text_family(
                    Pos2::new(cell.min.x, cell.center().y - st.fonts.small * 0.62),
                    &text,
                    st.fonts.small,
                    st.palette.text_disabled,
                    None,
                    FontFamily::Mono,
                );
            }
        },
    );
    changed
}

/// The two-choice popup a variable drop opens: Get or Set, at the cursor.
///
/// Rule 1 surface — it registers on the modal stack, so the press that
/// dismisses it is consumed rather than also landing on the canvas.
fn var_drop_popup(
    ui: &mut Ui,
    state: &mut GraphEditorState,
    registry: &NodeRegistry,
) {
    let Some(drop) = state.vars.drop.clone() else {
        ui.ctx_mut().modal_dismiss(var_drop_modal_id());
        return;
    };
    let st = ui.style();
    let pad = st.spacing.padding;
    let row_h = st.metrics.row_height;
    let rows = [
        (format!("Get {}", drop.label), false),
        (format!("Set {}", drop.label), true),
    ];
    let mut w: f32 = 0.0;
    {
        let mut p = ui.painter();
        for (text, _) in &rows {
            w = w.max(p.measure_text(text, st.fonts.body, None).x);
        }
    }
    let w = w + pad * 3.0;
    let rect = Rect::from_min_size(
        Pos2::new(drop.screen[0], drop.screen[1]),
        Vec2::new(w, row_h * 2.0 + pad),
    );
    ui.ctx_mut().modal_push(var_drop_modal_id(), rect);
    {
        let mut p = ui.painter();
        p.rect_filled(
            rect,
            st.rounding.panel,
            st.palette.elevated.with_alpha(st.palette.popover_alpha),
        );
        p.rect_stroke(rect, st.rounding.panel, st.metrics.border, st.palette.stroke_strong);
    }
    let mut picked: Option<bool> = None;
    for (i, (text, set)) in rows.iter().enumerate() {
        let row = Rect::from_min_size(
            Pos2::new(rect.min.x + pad * 0.5, rect.min.y + pad * 0.5 + i as f32 * row_h),
            Vec2::new(w - pad, row_h),
        );
        let id = ui.alloc_id(("graph_var_drop", i));
        let resp = ui.interact(id, row);
        let mut p = ui.painter();
        if resp.hovered {
            p.rect_filled(row, st.rounding.small, st.palette.selection_fill);
        }
        p.text(
            Pos2::new(row.min.x + pad * 0.5, row.center().y - st.fonts.body * 0.62),
            text,
            st.fonts.body,
            st.palette.text,
            None,
        );
        if resp.clicked {
            picked = Some(*set);
        }
    }
    if let Some(set) = picked {
        state.add_variable_node(&drop.slug, set, drop.world, registry);
        state.vars.drop = None;
        ui.ctx_mut().modal_dismiss(var_drop_modal_id());
        return;
    }
    if ui.ctx().input.key_pressed(Key::Escape)
        || ui.ctx().modal_dismissed(var_drop_modal_id()).is_some()
    {
        state.vars.drop = None;
        ui.ctx_mut().modal_dismiss(var_drop_modal_id());
    }
}

fn var_drop_modal_id() -> crusty_gui::id::Id {
    crusty_gui::id::Id::ROOT.with("graph_var_drop")
}

/// Retype / delete confirmation. Both name the usage count, because the count
/// is the consequence: a retype leaves stale wires for validation to flag, and
/// a delete leaves `UnknownVariable` placeholders on the nodes that named it.
fn var_confirm_dialog(
    ui: &mut Ui,
    rect: Rect,
    state: &mut GraphEditorState,
    registry: &NodeRegistry,
) {
    let Some(confirm) = state.vars.confirm.clone() else {
        return;
    };
    let st = ui.style();
    let pad = st.spacing.padding;
    let label = |slug: &str| {
        state
            .doc
            .variable(slug)
            .map(|v| v.label.clone())
            .unwrap_or_else(|| slug.to_string())
    };
    let plural = |n: usize| if n == 1 { "node" } else { "nodes" };
    // The copy **states the outcome** rather than gesturing at it: how many
    // nodes are involved, what happens to their wires, and — for a retype —
    // which of the two things the no-coercion rule will do to the default.
    let (title, detail, verb, danger) = match &confirm {
        VarConfirm::Retype { slug, ty, uses } => {
            let mut detail = format!(
                "{} is used by {} {}. Wires that expect {} will flag until re-wired.",
                label(slug),
                uses,
                plural(*uses),
                state
                    .doc
                    .variable(slug)
                    .map(|v| type_label(&v.ty))
                    .unwrap_or_default(),
            );
            if let Some(outcome) = state
                .doc
                .variable(slug)
                .and_then(|v| retype_default_outcome(v, ty))
            {
                detail.push(' ');
                detail.push_str(&outcome);
            }
            (
                format!("Change type to {}?", type_label(ty)),
                detail,
                "Change type",
                false,
            )
        }
        VarConfirm::Delete { slug, uses } if *uses > 0 => (
            format!("Delete \u{201c}{}\u{201d}?", label(slug)),
            format!(
                "{} Get/Set {} stay on canvas as flagged placeholders (Get <missing> + error \
                 badge). Your nodes are never deleted for you.",
                uses,
                plural(*uses)
            ),
            "Delete Variable",
            true,
        ),
        VarConfirm::Delete { slug, .. } => (
            format!("Delete \u{201c}{}\u{201d}?", label(slug)),
            "Nothing uses it.".to_string(),
            "Delete Variable",
            true,
        ),
    };
    let font = st.fonts.body;
    // The detail is a sentence or three of consequence, so it wraps to a fixed
    // measure instead of stretching the dialog across the canvas.
    let text_w = (rect.width() * 0.32).clamp(240.0, 360.0);
    let detail_h = {
        let mut p = ui.painter();
        p.measure_text(&detail, st.fonts.small, Some(text_w)).y
    };
    let panel = Rect::from_center_size(
        Pos2::new(rect.center().x, rect.min.y + rect.height() * 0.3),
        Vec2::new(
            text_w + pad * 2.0,
            font * 1.6 + detail_h + st.metrics.control_height + pad * 3.0,
        ),
    );
    {
        let mut p = ui.painter();
        p.rect_filled_translucent(
            rect,
            Rounding::ZERO,
            Color::BLACK.with_alpha(st.palette.scrim_alpha),
        );
        p.rect_filled(panel, st.rounding.panel, st.palette.elevated);
        p.rect_stroke(panel, st.rounding.panel, st.metrics.border, st.palette.stroke_strong);
        p.text(
            Pos2::new(panel.min.x + pad, panel.min.y + pad),
            &title,
            font,
            st.palette.text,
            None,
        );
        p.text(
            Pos2::new(panel.min.x + pad, panel.min.y + pad + font * 1.6),
            &detail,
            st.fonts.small,
            st.palette.text_secondary,
            Some(text_w),
        );
    }
    let bw = 104.0;
    let by = panel.max.y - pad - st.metrics.control_height;
    let (mut go, mut cancel) = (false, false);
    ui.run_at(
        Rect::from_min_size(
            Pos2::new(panel.max.x - pad - bw * 2.0 - st.spacing.item, by),
            Vec2::new(bw * 2.0 + st.spacing.item, st.metrics.control_height),
        ),
        Direction::LeftToRight,
        Id::new("graph_var_confirm"),
        UiOptions { padding: Vec2::ZERO, spacing: st.spacing.item },
        |ui| {
            cancel = Button::new("Cancel")
                .exact_size(Vec2::new(bw, st.metrics.control_height))
                .show(ui)
                .clicked;
            let go_b =
                Button::new(verb).exact_size(Vec2::new(bw, st.metrics.control_height));
            let go_b = if danger { go_b.danger() } else { go_b.primary() };
            go = go_b.show(ui).clicked;
        },
    );
    if cancel || ui.ctx().input.key_pressed(Key::Escape) {
        state.vars.confirm = None;
        return;
    }
    if go {
        match confirm {
            VarConfirm::Retype { slug, ty, .. } => {
                state.retype_variable(&slug, ty, registry);
            }
            VarConfirm::Delete { slug, .. } => {
                if state.remove_variable(&slug, registry) && state.vars.selected.as_deref() == Some(slug.as_str()) {
                    state.vars.selected = None;
                    state.vars.rename_buf = None;
                }
            }
        }
        state.vars.confirm = None;
    }
}

/// Remove / rename confirmation for a payload field with readers (GS-1).
///
/// Removing or renaming a field is a **breaking change and it says so**: the
/// count is the consequence, and the copy names what happens to the wires that
/// read it. A field with zero readers never reaches here — no ceremony for a
/// change nothing can notice.
fn payload_confirm_dialog(
    ui: &mut Ui,
    rect: Rect,
    state: &mut GraphEditorState,
    registry: &NodeRegistry,
) {
    let Some(confirm) = state.payload.confirm.clone() else {
        return;
    };
    let st = ui.style();
    let pad = st.spacing.padding;
    let slug = confirm.slug().to_string();
    let (readers, graphs) = confirm.counts();
    let plural = |n: usize, one: &'static str, many: &'static str| if n == 1 { one } else { many };
    let (title, detail, verb, danger) = match &confirm {
        PayloadConfirm::Remove { .. } => (
            format!("Remove field \u{201c}{slug}\u{201d}?"),
            format!(
                "{readers} {} in {graphs} {} read this field. Their {slug} pins become ghost \
                 rows until re-wired.",
                plural(readers, "listener", "listeners"),
                plural(graphs, "graph", "graphs"),
            ),
            "Remove Field",
            true,
        ),
        PayloadConfirm::Rename { name, .. } => (
            format!(
                "Rename field \u{201c}{slug}\u{201d} to \u{201c}{}\u{201d}?",
                variable_slug(name)
            ),
            format!(
                "{readers} {} in {graphs} {} read this field. Wires in this graph follow the \
                 rename; wires in other graphs cannot, and their {slug} pins become ghost rows \
                 until re-wired.",
                plural(readers, "listener", "listeners"),
                plural(graphs, "graph", "graphs"),
            ),
            "Rename Field",
            false,
        ),
    };
    // The detail is two or three lines of consequence, so it wraps to a fixed
    // measure instead of stretching the dialog off the canvas.
    let font = st.fonts.body;
    let text_w = (rect.width() * 0.32).clamp(240.0, 360.0);
    let detail_h = {
        let mut p = ui.painter();
        p.measure_text(&detail, st.fonts.small, Some(text_w)).y
    };
    let panel = Rect::from_center_size(
        Pos2::new(rect.center().x, rect.min.y + rect.height() * 0.3),
        Vec2::new(
            text_w + pad * 2.0,
            font * 1.6 + detail_h + st.metrics.control_height + pad * 3.0,
        ),
    );
    {
        let mut p = ui.painter();
        p.rect_filled_translucent(
            rect,
            Rounding::ZERO,
            Color::BLACK.with_alpha(st.palette.scrim_alpha),
        );
        p.rect_filled(panel, st.rounding.panel, st.palette.elevated);
        p.rect_stroke(panel, st.rounding.panel, st.metrics.border, st.palette.stroke_strong);
        p.text(
            Pos2::new(panel.min.x + pad, panel.min.y + pad),
            &title,
            font,
            st.palette.text,
            None,
        );
        p.text(
            Pos2::new(panel.min.x + pad, panel.min.y + pad + font * 1.6),
            &detail,
            st.fonts.small,
            st.palette.text_secondary,
            Some(text_w),
        );
    }
    let bw = 92.0;
    let by = panel.max.y - pad - st.metrics.control_height;
    let (mut go, mut cancel) = (false, false);
    ui.run_at(
        Rect::from_min_size(
            Pos2::new(panel.max.x - pad - bw * 2.0 - st.spacing.item, by),
            Vec2::new(bw * 2.0 + st.spacing.item, st.metrics.control_height),
        ),
        Direction::LeftToRight,
        Id::new("graph_payload_confirm"),
        UiOptions { padding: Vec2::ZERO, spacing: st.spacing.item },
        |ui| {
            cancel = Button::new("Cancel")
                .exact_size(Vec2::new(bw, st.metrics.control_height))
                .show(ui)
                .clicked;
            let b = Button::new(verb).exact_size(Vec2::new(bw, st.metrics.control_height));
            let b = if danger { b.danger() } else { b.primary() };
            go = b.show(ui).clicked;
        },
    );
    if cancel || ui.ctx().input.key_pressed(Key::Escape) {
        state.payload.confirm = None;
        return;
    }
    if go {
        match confirm {
            PayloadConfirm::Remove { node, slug, .. } => {
                state.remove_payload_field(node, &slug, registry);
            }
            PayloadConfirm::Rename { node, slug, name, .. } => {
                state.rename_payload_field(node, &slug, &name, registry);
            }
        }
        state.payload.confirm = None;
    }
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

/// Two-level world grid: minor 16px @ 5% white, major 128px @ 9%. The minor
/// grid drops below 40% zoom, where it would only add noise (and the surviving
/// major grid lifts to 7%, per the design). Never accent.
///
/// Both levels are painted as **opaque** colors mixed in gamma space
/// ([`grid_minor`] / [`grid_major`]), not as translucent white: crusty
/// composites in linear light, where a 5% white lands ~5× brighter than the
/// design's CSS gradient of the same number. Each line is also snapped to a
/// whole pixel — a 1px stroke straddling two pixel columns spreads its ink and
/// reads as a fat smudge instead of the design's hairline.
fn draw_grid(ui: &mut Ui, scope: &CanvasScope, st: &Style, vis: Rect, zoom: f32) {
    let canvas = st.palette.input;
    let mut p = ui.painter();
    p.rect_filled(scope.rect(), Rounding::ZERO, canvas);

    let mut level = |step: f32, color: Color| {
        // Guard against a pathological view producing millions of lines.
        let cols = (vis.width() / step).ceil() as i32;
        let rows = (vis.height() / step).ceil() as i32;
        if cols > 4096 || rows > 4096 {
            return;
        }
        for i in (vis.min.x / step).floor() as i32..=(vis.max.x / step).ceil() as i32 {
            let wx = i as f32 * step;
            let x = pixel_center(scope.world_to_screen(Pos2::new(wx, vis.min.y)).x);
            p.line_segment(
                Pos2::new(x, scope.world_to_screen(Pos2::new(wx, vis.min.y)).y),
                Pos2::new(x, scope.world_to_screen(Pos2::new(wx, vis.max.y)).y),
                1.0,
                color,
            );
        }
        for i in (vis.min.y / step).floor() as i32..=(vis.max.y / step).ceil() as i32 {
            let wy = i as f32 * step;
            let y = pixel_center(scope.world_to_screen(Pos2::new(vis.min.x, wy)).y);
            p.line_segment(
                Pos2::new(scope.world_to_screen(Pos2::new(vis.min.x, wy)).x, y),
                Pos2::new(scope.world_to_screen(Pos2::new(vis.max.x, wy)).x, y),
                1.0,
                color,
            );
        }
    };

    let minor = zoom >= GRID_MINOR_MIN_ZOOM;
    if minor {
        level(GRID_MINOR_STEP, grid_minor(canvas));
    }
    level(GRID_MAJOR_STEP, grid_major(canvas, !minor));
}

/// Centre of the pixel `v` falls in — a 1px stroke drawn here covers exactly
/// that pixel instead of half-covering two.
fn pixel_center(v: f32) -> f32 {
    v.floor() + 0.5
}

/// Group frames + comment boxes, behind wires and nodes (P7). Their titles are
/// the one text exempt from the LOD glyph cut — on a graph too big to read,
/// annotations are the only wayfinding left.
#[allow(clippy::too_many_arguments)]
/// Find-in-graph dims a non-match by blending its fills *toward the canvas*
/// rather than by lowering their alpha. A node is painted in layers now (the
/// category-edge underlay, then the body, then the header block); translucent
/// layers would let the edge color bleed up through the whole card instead of
/// showing only in its 2px reveal.
fn fade(col: Color, t: f32, bg: Color) -> Color {
    if t >= 1.0 {
        return col;
    }
    Color::rgba(
        bg.r + (col.r - bg.r) * t,
        bg.g + (col.g - bg.g) * t,
        bg.b + (col.b - bg.b) * t,
        col.a,
    )
}

/// The title bar of an annotation card rounds its top corners into the card's
/// outline and squares off into the body below — rounding all four made the
/// bar's bottom corners curve into open body. A collapsed card *is* its bar, so
/// there it keeps all four.
fn bar_rounding(round: Rounding, bar_h: f32, card_h: f32) -> Rounding {
    if bar_h >= card_h - 0.5 {
        round
    } else {
        Rounding { nw: round.nw, ne: round.ne, sw: 0.0, se: 0.0 }
    }
}

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
        // A 6% body wash is a tint over the canvas and the nodes inside the
        // group, not a surface that covers them.
        p.rect_filled_translucent(sr, round, wash);
        p.rect_stroke(
            sr,
            round,
            m.border,
            if sel { st.palette.stroke_strong } else { border },
        );
        let bar_h = m.group_bar * zoom;
        let bar = Rect::from_min_size(sr.min, Vec2::new(sr.width(), bar_h));
        p.rect_filled(bar, bar_rounding(round, bar_h, sr.height()), st.palette.header);
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
        p.rect_filled(bar, bar_rounding(round, bar_h, sr.height()), bar_fill);
        if let Some(t) = c.tint {
            // The 1px left edge lives on the card's *straight* left side; run it
            // corner-to-corner and its square ends poke outside the radius.
            let top = sr.min.y + round.nw;
            let bot = (sr.max.y - round.sw).max(top);
            p.rect_filled(
                Rect::from_min_max(Pos2::new(sr.min.x, top), Pos2::new(sr.min.x + m.border, bot)),
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
    /// This exec wire has fired at some point this session (GS-3). `true` for
    /// every wire when no session is running, so nothing dims in edit mode.
    taken: bool,
    /// Firings per second on the producing pin, for the bubble rate cap.
    rate: f32,
    /// Live execution pulse, `0.0` (the only value in edit mode) to `1.0`
    /// (fired this instant). Layered onto the color and the width below —
    /// never a re-route: the pulse travels the polyline the router already
    /// produced, so a wire does not move when it runs.
    pulse: f32,
    /// A derived machine edge (Task 41 rework): drawn as its straight
    /// segment regardless of the wire-style preference — a state machine's
    /// arrows do not take the subway.
    direct: bool,
    /// Draw an arrowhead at `b` — the machine edge's direction marker.
    arrow: bool,
}

/// Machine-edge arrowhead length, world units (Task 41 rework).
const ARROW_L: f32 = 9.0;

/// Screen-space width of the machine card's border rim that starts a wire
/// drag (Task 41 rework), capped to a third of the card's smaller side so a
/// small pill keeps a grabbable center.
const BORDER_GRAB_PX: f32 = 10.0;

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
    exec: Option<&GraphExecViz>,
    // Task 41 rework: doc-edge indices the machine layout replaced with
    // straight border-to-chip segments. Edges it could not place honestly
    // (reroutes, overlapping states) fall through to the router.
    anim_segs: Option<&BTreeMap<usize, super::graph_anim_edge::EdgeSeg>>,
) -> Vec<WireGeom> {
    let mut out = Vec::with_capacity(state.doc.edges.len());
    let ranks = converge_ranks(&state.doc.edges);
    for (edge_index, e) in state.doc.edges.iter().enumerate() {
        let src = geoms.iter().find(|g| g.id == e.from_node);
        let dst = geoms.iter().find(|g| g.id == e.to_node);
        let (Some(src), Some(dst)) = (src, dst) else {
            continue;
        };
        if let Some(seg) = anim_segs.and_then(|s| s.get(&edge_index)) {
            let a = Pos2::new(seg.a[0], seg.a[1]);
            let b = Pos2::new(seg.b[0], seg.b[1]);
            let bounds = Rect::from_min_max(
                Pos2::new(a.x.min(b.x) - 20.0, a.y.min(b.y) - 20.0),
                Pos2::new(a.x.max(b.x) + 20.0, a.y.max(b.y) + 20.0),
            );
            let clip = bounds.intersect(vis);
            if clip.width() < 0.0 || clip.height() < 0.0 {
                continue;
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
                screen: vec![scope.world_to_screen(a), scope.world_to_screen(b)],
                selected: state.selected_edges.contains(e),
                mismatched: errors.edges.contains(e),
                // Machine flow never pulses: it is not exec.
                pulse: 0.0,
                taken: true,
                rate: 0.0,
                direct: true,
                arrow: seg.arrow,
            });
            continue;
        }
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
            // P3's converging-exec finding, closed here: several edges into
            // one exec input share a target pin row, so the row-keyed stagger
            // put them in the same lane and their final approach coincided.
            // Ranking each edge within its converging set gives them separate
            // lanes. Data pins are single-wire by construction, so this is
            // always 0 for them and their geometry is bit-identical to before.
            converge_index: ranks[edge_index],
            node_rects,
        };
        // Cull **before** routing: `wire_bounds` is cheap (a branch decision
        // plus a min/max over at most six points, or the spline's control
        // hull) where the full route plus corner tessellation plus the
        // world→screen transform is not. Bounds, not the endpoints' box — a
        // backward lane or a spline bow reaches well outside it.
        let Some(bounds) = router::wire_bounds(a, b, prefs, &meta) else {
            continue;
        };
        let clip = bounds.intersect(vis);
        if clip.width() < 0.0 || clip.height() < 0.0 {
            continue;
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
            pulse: exec.map_or(0.0, |v| wire_pulse(v, state, e)),
            taken: exec.is_none_or(|v| !v.has_session() || wire_taken(v, state, e)),
            rate: exec.map_or(0.0, |v| wire_rate(v, state, e)),
            direct: false,
            arrow: false,
        });
    }
    out
}

/// Each edge's rank among the edges landing on the *same* input pin, indexed
/// by edge index.
///
/// Only exec inputs ever have more than one (data pins are single-wire), so
/// this is 0 for every data wire and for every unshared exec wire. Computed in
/// one pass for the whole document rather than per edge: the per-edge form is
/// quadratic, and this runs every frame.
fn converge_ranks(edges: &[Edge]) -> Vec<usize> {
    let mut seen: BTreeMap<(u64, &str), usize> = BTreeMap::new();
    edges
        .iter()
        .map(|e| {
            let slot = seen.entry((e.to_node, e.to_pin.as_str())).or_insert(0);
            let rank = *slot;
            *slot += 1;
            rank
        })
        .collect()
}

/// How brightly this wire is running. Resolved against the **real producer**:
/// a reroute is transparent at run time, so the interpreter recorded the edge
/// against whatever feeds the reroute's input, and both halves of a rerouted
/// wire have to light from that one record.
fn wire_pulse(viz: &GraphExecViz, state: &GraphEditorState, e: &Edge) -> f32 {
    match real_producer(state, e.from_node, &e.from_pin) {
        Some((node, pin)) => viz.pulse(node, &pin),
        None => 0.0,
    }
}

/// Has this exec edge been travelled this session? Resolved against the real
/// producer, exactly like the pulse: a reroute is transparent at run time, so
/// both halves of a rerouted wire answer from the one record.
fn wire_taken(viz: &GraphExecViz, state: &GraphEditorState, e: &Edge) -> bool {
    match real_producer(state, e.from_node, &e.from_pin) {
        Some((node, pin)) => viz.is_taken(node, &pin, e.to_node),
        None => false,
    }
}

/// Firings per second on this wire's producing pin.
fn wire_rate(viz: &GraphExecViz, state: &GraphEditorState, e: &Edge) -> f32 {
    match real_producer(state, e.from_node, &e.from_pin) {
        Some((node, pin)) => viz.rate(node, &pin),
        None => 0.0,
    }
}

/// The last value that crossed this pin, when a running instance is bound.
///
/// An **input** pin borrows its wire's value: what arrived there is whatever
/// its producer sent, and only the producer side is recorded (one output
/// feeding three inputs is one wire's worth of truth). An unwired input has no
/// wire and answers `None` — its constant is already drawn in its field.
fn pin_value<'a>(
    viz: &'a GraphExecViz,
    state: &GraphEditorState,
    node: u64,
    slug: &str,
    output: bool,
) -> Option<&'a str> {
    let (n, p) = if output {
        real_producer(state, node, slug)?
    } else {
        let e = state
            .doc
            .edges
            .iter()
            .find(|e| e.to_node == node && e.to_pin == slug)?;
        real_producer(state, e.from_node, &e.from_pin)?
    };
    viz.value(n, &p)
}

/// A chain of reroutes is longer than this only if the document is degenerate.
const MAX_REROUTE_HOPS: usize = 64;

/// Walk back through reroutes to the pin that actually produces this value.
/// Returns `None` when the chain runs out of wire (an unwired reroute produces
/// nothing) or loops.
fn real_producer(state: &GraphEditorState, node: u64, pin: &str) -> Option<(u64, String)> {
    let mut node = node;
    let mut pin = pin.to_string();
    for _ in 0..MAX_REROUTE_HOPS {
        let n = state.doc.node(node)?;
        if n.type_id != REROUTE_TYPE_ID {
            return Some((node, pin));
        }
        let up = state
            .doc
            .edges
            .iter()
            .find(|e| e.to_node == node && e.to_pin == REROUTE_IN)?;
        node = up.from_node;
        pin = up.from_pin.clone();
    }
    None
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
        // Tessellated for the size it will be drawn at: these points are
        // about to be scaled to screen, and a corner arc built for graph
        // space reads as a chamfer once zoomed in.
        router::round_corners(
            &router::route(a, b, prefs, meta),
            prefs.corner_radius,
            scope.zoom(),
        )
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
    cut_preview: &BTreeSet<usize>,
    registry: Option<&NodeRegistry>,
    prefs: &WirePrefs,
    st: &Style,
    lod: ZoomLod,
    selection_outline: Color,
    // A live session is bound: the switch for every debug-only layer.
    live: bool,
    zoom: f32,
    // Bubble animation phase in `[0, 1)`, derived from the frame counter so
    // the flow moves without the panel needing a clock of its own.
    phase: f32,
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
            wire_color(registry, &w.ty)
        };
        // Execution pulse (45-A P7): a *layer* over whatever the wire already
        // is, never a replacement — a mistyped wire that happens to be running
        // still reads as broken, and a selected wire still reads as selected.
        // `accent_active` is the live/active token by job (it is what a
        // toggled tool and a running profiler use), which keeps the pulse
        // distinct from selection's amber and from focus.
        let color = if w.pulse > 0.0 {
            mix(color, st.palette.accent_active, PULSE_TINT * w.pulse)
        } else {
            color
        };
        // Taken-path inversion (GS-3): during a live session an exec wire that
        // has not fired drops to 50%, and one that has holds its normal 92%.
        // Only exec — a data wire carries a value, not a path, and tinting it
        // would say something untrue about control flow.
        let color = if !w.taken && w.is_exec() {
            color.with_alpha(color.a * UNFIRED_EXEC_ALPHA)
        } else {
            color
        };
        // L4 collapses every wire to a hairline.
        let width = if lod.bar_only() { 1.0 } else { w.width(is_hovered) };
        // Flow bubbles ride the wire that is firing right now.
        // Above the cap the wire holds steady-hot (the pulse tint above
        // already does that) instead of strobing dots nobody can follow.
        if live
            && w.is_exec()
            && w.pulse > 0.0
            && zoom >= BUBBLE_MIN_ZOOM
            && w.rate > 0.0
            && w.rate < STEADY_HOT_HZ
        {
            draw_bubbles(&mut p, w, phase, st.palette.accent_active);
        }
        // Weight carries the pulse where color cannot: an exec wire is already
        // white, so brightness alone would barely read.
        let width = width * (1.0 + PULSE_WEIGHT * w.pulse);
        // A wire the cut is about to take goes red-dashed *during* the drag,
        // so the gesture is previewed and Esc-abortable.
        if cut_preview.contains(&w.edge_index) {
            for seg in w.screen.windows(2) {
                dashed_line(&mut p, seg[0], seg[1], width.max(CUT_STROKE), status.error);
            }
            continue;
        }
        stroke_wire(&mut p, w, prefs, scope, width, color);
        // The machine edge's direction marker (Task 41 rework): a filled
        // arrowhead at the landing border — the Unreal reading of a state
        // machine. World-sized, so it zooms with the graph.
        if w.arrow && !lod.bar_only() && w.screen.len() >= 2 {
            draw_arrow_head(
                &mut p,
                w.screen[w.screen.len() - 1],
                w.screen[w.screen.len() - 2],
                (ARROW_L * zoom).clamp(5.0, 16.0),
                color,
            );
        }
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

/// Filled arrowhead at `tip`, oriented away from `prev` (screen space) —
/// the machine edge's direction marker (Task 41 rework).
fn draw_arrow_head(p: &mut Painter, tip: Pos2, prev: Pos2, l: f32, color: Color) {
    let v = tip - prev;
    let len = v.length();
    if len <= 1.0 {
        return;
    }
    let d = Vec2::new(v.x / len * l, v.y / len * l);
    let n = Vec2::new(-d.y * 0.45, d.x * 0.45);
    p.triangle(
        tip,
        Pos2::new(tip.x - d.x + n.x, tip.y - d.y + n.y),
        Pos2::new(tip.x - d.x - n.x, tip.y - d.y - n.y),
        color,
    );
}

/// Flow bubbles: dots riding an exec wire in the direction control took
/// (GS-3). Debug sessions only, exec only, above [`BUBBLE_MIN_ZOOM`], and only
/// while the wire is actually firing.
///
/// **The rate cap is the honest part.** Above [`STEADY_HOT_HZ`] firings a
/// second the dots are a flicker rather than a flow, so the wire holds
/// steady-hot instead — one bright statement rather than twenty lies about
/// individual firings.
fn draw_bubbles(p: &mut Painter, w: &WireGeom, phase: f32, color: Color) {
    if w.screen.len() < 2 {
        return;
    }
    // Arc-length table, so the dots move at a constant speed along a polyline
    // whose segments are not equal (every router style produces those).
    let mut lens: Vec<f32> = Vec::with_capacity(w.screen.len());
    let mut total = 0.0;
    lens.push(0.0);
    for pair in w.screen.windows(2) {
        total += ((pair[1].x - pair[0].x).powi(2) + (pair[1].y - pair[0].y).powi(2)).sqrt();
        lens.push(total);
    }
    if total <= 1.0 {
        return;
    }
    for i in 0..BUBBLES_PER_WIRE {
        let t = (phase + i as f32 / BUBBLES_PER_WIRE as f32).fract();
        let want = t * total;
        let Some(seg) = lens.windows(2).position(|l| want >= l[0] && want <= l[1]) else {
            continue;
        };
        let span = (lens[seg + 1] - lens[seg]).max(1e-3);
        let f = (want - lens[seg]) / span;
        let (a, b) = (w.screen[seg], w.screen[seg + 1]);
        let at = Pos2::new(a.x + (b.x - a.x) * f, a.y + (b.y - a.y) * f);
        // Fading tail: the leading dot is brightest, so the direction reads.
        let alpha = 0.9 - 0.2 * i as f32;
        p.circle_filled(at, BUBBLE_R, color.with_alpha(alpha));
    }
}

/// How far a full-strength pulse pulls a wire's color toward `accent_active`.
/// Short of 1.0 on purpose: at full replacement every running wire is the same
/// color and the type language the palette spent twelve hues on disappears.
const PULSE_TINT: f32 = 0.75;
/// How much a full-strength pulse thickens a wire, as a fraction of its normal
/// width. Enough to read at a glance, not enough to change the canvas's rhythm.
const PULSE_WEIGHT: f32 = 0.75;

/// Linear blend, `t = 0` -> `a`. Straight lerp in crusty's linear-space color,
/// which is where a blend is meant to happen.
fn mix(a: Color, b: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    Color::rgba(
        a.r + (b.r - a.r) * t,
        a.g + (b.g - a.g) * t,
        a.b + (b.b - a.b) * t,
        a.a + (b.a - a.a) * t,
    )
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
    // A machine edge is a straight arrow whatever the wire-style pref says.
    if w.direct {
        if w.screen.len() >= 2 {
            p.polyline(&w.screen, width, color);
        }
        return;
    }
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

/// Edge indices whose routed polyline the in-flight cut path crosses. Exact
/// segment-segment tests against the *drawn* polyline, so what the preview
/// highlights is exactly what the release cuts.
fn crossed_indices(
    state: &GraphEditorState,
    wires: &[WireGeom],
    scope: &CanvasScope,
) -> BTreeSet<usize> {
    let Some(path) = state.cut_path.as_ref() else {
        return BTreeSet::new();
    };
    if path.len() < 2 {
        return BTreeSet::new();
    }
    let screen: Vec<Pos2> = path
        .iter()
        .map(|q| scope.world_to_screen(Pos2::new(q[0], q[1])))
        .collect();
    wires
        .iter()
        .filter(|w| router::path_crosses_polyline(&screen, &w.screen))
        .map(|w| w.edge_index)
        .collect()
}

/// The edges the in-flight cut would take, resolved to real `Edge` values.
fn crossed_by_cut(
    state: &GraphEditorState,
    wires: &[WireGeom],
    scope: &CanvasScope,
) -> Vec<Edge> {
    crossed_indices(state, wires, scope)
        .into_iter()
        .filter_map(|i| state.doc.edges.get(i).cloned())
        .collect()
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
/// The breakpoint ladder's four states (GS-4), in precedence order below the
/// error disc. Kept as an enum so the drawing is one match and the ladder is
/// one testable function rather than a chain of conditions at the paint site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BreakBadge {
    /// Execution is parked here: solid warning octagon.
    Hit,
    /// Armed and resolvable: solid error octagon.
    Armed,
    /// Armed but resolves to nothing that can fire: hollow warning + `!`.
    Invalid,
    /// Kept, not armed: hollow grey.
    Disabled,
}

/// Which badge a node's mark draws, given what the bound instance is doing.
///
/// `None` here means "not a breakpoint badge" — the caller falls through to
/// the error disc, which is the rung above everything but a hit.
fn break_badge(
    mark: Option<bool>,
    errored: bool,
    paused_here: bool,
    invalid: bool,
) -> Option<BreakBadge> {
    match () {
        _ if paused_here && mark.is_some() => Some(BreakBadge::Hit),
        _ if errored => None,
        _ if mark == Some(true) && invalid => Some(BreakBadge::Invalid),
        _ if mark == Some(true) => Some(BreakBadge::Armed),
        _ if mark == Some(false) => Some(BreakBadge::Disabled),
        _ => None,
    }
}

/// Paint one breakpoint octagon. Filled for the two "this will stop"
/// states, a hollow ring for the two that will not — the same fill-means-live
/// convention the pin dots use.
fn draw_break_badge(
    p: &mut Painter,
    st: &Style,
    c: Pos2,
    r: f32,
    badge: BreakBadge,
    text_px: f32,
) {
    let status = Palette::invariant_status();
    let pts: Vec<Pos2> = (0..8)
        .map(|i| {
            let a = std::f32::consts::TAU * (i as f32 + 0.5) / 8.0;
            Pos2::new(c.x + r * a.cos(), c.y + r * a.sin())
        })
        .collect();
    let (col, filled) = match badge {
        BreakBadge::Hit => (status.warning, true),
        BreakBadge::Armed => (status.error, true),
        BreakBadge::Invalid => (status.warning, false),
        BreakBadge::Disabled => (st.palette.text_disabled, false),
    };
    if filled {
        p.convex_polygon_filled(pts, col);
    } else {
        // A true stroke rather than a donut: the header fill behind a badge is
        // not always the same colour (the category band reaches it), so
        // punching a hole with a second fill would show a seam.
        let w = (r * 0.24).max(1.0);
        for i in 0..8 {
            p.line_segment(pts[i], pts[(i + 1) % 8], w, col);
        }
    }
    if badge == BreakBadge::Invalid {
        let px = text_px * 0.8;
        p.text(
            Pos2::new(c.x - px * 0.16, c.y - px * 0.56),
            "!",
            px,
            col,
            None,
        );
    }
}

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
    frame: u64,
    selection_outline: Color,
    widget_rects: &mut Vec<Rect>,
    exec: Option<&GraphExecViz>,
    // Nodes the variables filter matched. `Some` means a filter is active:
    // everything outside the set dims, on the find-in-graph rule.
    var_dim: Option<&BTreeSet<u64>>,
    // Watch chip boxes, collected for the interaction pass (the ✕ is
    // hover-revealed and lives where the chip was drawn).
    watch_rects: &mut Vec<(u64, String, bool, Rect)>,
    // Badge gutter slots of nodes carrying a breakpoint mark, for the click
    // that arms/disarms it (GS-4).
    badge_rects: &mut Vec<(u64, Rect)>,
) {
    let status = Palette::invariant_status();
    // Collected during the paint pass, applied after (the widget pass needs
    // `&mut Ui`, which the painter borrow would otherwise hold).
    let mut pending_widgets: Vec<(u64, String, Rect, InlineKind)> = Vec::new();
    // Where the in-flight payload name entry draws, once we reach its node.
    let mut draft_cell: Option<Rect> = None;
    let pointer_world = scope.pointer_world(ui);

    for g in geoms {
        let body = g.body_rect(lod, m);
        // Node-level, not row-level: the band's ✕ marks reveal together, so
        // moving down a list of fields does not strobe one glyph at a time.
        let band_hovered = pointer_world.is_some_and(|pw| g.rect.contains(pw));
        let clip = body.intersect(vis);
        if clip.width() <= 0.0 || clip.height() <= 0.0 {
            continue;
        }
        let srect = scope.world_rect_to_screen(body);
        let selected = state.selection.contains(&g.id);
        let round = Rounding::same(m.radius * zoom);
        // A runtime kill flags its node like a validation error does: the
        // border and the badge gutter are the existing precedence slot, and
        // the reason line goes underneath (GS-3).
        let killed_here = exec
            .and_then(|v| v.killed.as_ref())
            .is_some_and(|k| k.node == Some(g.id));
        let errored = g.errored || killed_here;
        // GS-4. `paused_here` does not require a mark: Step re-parks on
        // whatever node comes next, and "you are here" is exactly what the
        // border and the chip are for. The octagon does need one — drawing a
        // breakpoint glyph where there is no breakpoint would be a lie.
        let paused_here = exec.is_some_and(|v| v.paused_on(g.id));
        let invalid_break = exec.is_some_and(|v| v.break_invalid(g.id));
        let edge_col = if g.missing { status.error } else { g.edge_color() };
        // Find-in-graph dims what does not match, rather than hiding it —
        // context is what makes a search result mean anything.
        let dim = match (
            state.find.as_ref().filter(|f| f.active()),
            var_dim,
        ) {
            (Some(f), _) => {
                if f.matches(&g.title, &g.title) { 1.0 } else { FIND_DIM }
            }
            (None, Some(keep)) => {
                if keep.contains(&g.id) { 1.0 } else { FIND_DIM }
            }
            (None, None) => 1.0,
        };

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

        // Task 41: the at-rest transition chip. One mono line over a small
        // rounded card, with the Bool socket dot at its left — filled when a
        // rule is wired, hollow for always-true (mockup 2b's dot on 2d's
        // collapse). Selection swaps to the standard card via geometry, so
        // this branch only ever draws the rest state.
        if let Some(chip) = &g.chip {
            let bg = st.palette.input;
            if lod.bar_only() {
                let bar = Rect::from_min_size(
                    srect.min,
                    Vec2::new(srect.width(), (L4_BAR_H * m.scale * zoom).max(1.0)),
                );
                p.rect_filled(bar, Rounding::ZERO, edge_col);
                continue;
            }
            let round = Rounding::same((m.radius * 0.7 * zoom).min(srect.height() * 0.5));
            p.rect_filled(srect, round, fade(st.palette.elevated, dim, bg));
            p.rect_stroke(
                srect,
                round,
                m.border,
                if errored { status.error } else { st.palette.stroke_strong },
            );
            // The socket dot: ember — the Bool family — per the mockup.
            let dot_c = Pos2::new(
                srect.min.x + (m.pad_x + m.pin_r) * zoom,
                srect.center().y,
            );
            let dot_r = m.pin_r * zoom;
            let ember = fade(pin_color(Some(registry), &PinType::Bool), dim, bg);
            if chip.wired {
                p.circle_filled(dot_c, dot_r, ember);
            } else {
                p.circle_stroke(dot_c, dot_r, (m.ring_w * zoom).max(1.0), ember);
            }
            if lod.glyphs() {
                let px = st.fonts.small * zoom;
                p.text_family(
                    Pos2::new(
                        dot_c.x + dot_r + m.label_gap * zoom,
                        srect.center().y - px * 0.62,
                    ),
                    &chip.text,
                    px,
                    fade(st.palette.text, dim, bg),
                    None,
                    FontFamily::Mono,
                );
            }
            continue;
        }

        // Task 41 rework: compact machine cards (mockup 2b). A state is its
        // name, role tag and one mono subtitle; ENTRY (Logic-green text) and
        // ANY STATE (dashed border) are small pills of the same card family.
        // No pins, no fields — selection swaps a state to the standard card
        // via geometry, so this branch only ever draws the rest form.
        if let Some(card) = &g.anim {
            let bg = st.palette.input;
            if lod.bar_only() {
                let bar = Rect::from_min_size(
                    srect.min,
                    Vec2::new(srect.width(), (L4_BAR_H * m.scale * zoom).max(1.0)),
                );
                p.rect_filled(bar, Rounding::ZERO, edge_col);
                continue;
            }
            p.rect_filled(srect, round, fade(st.palette.header, dim, bg));
            if card.kind == AnimCardKind::State {
                // The generic card's underlay-reveal: a rounded band in the
                // category color, the fills inset below its top 2px.
                let band_h = (round.nw * 2.0).min(srect.height());
                p.rect_filled(
                    Rect::from_min_size(srect.min, Vec2::new(srect.width(), band_h)),
                    Rounding::same(round.nw.min(band_h * 0.5)),
                    edge_col,
                );
                let edge_h = (m.edge * zoom).max(1.0).min(band_h);
                let inner_r = (round.nw - edge_h).max(0.0);
                p.rect_filled(
                    Rect::from_min_max(
                        Pos2::new(srect.min.x, srect.min.y + edge_h),
                        srect.max,
                    ),
                    Rounding { nw: inner_r, ne: inner_r, sw: round.sw, se: round.se },
                    fade(st.palette.elevated, dim, bg),
                );
            }
            let border_col = if errored {
                status.error
            } else {
                st.palette.stroke_strong
            };
            if card.kind == AnimCardKind::Any && !errored {
                dashed_rect(&mut p, srect, m.border, border_col);
            } else {
                p.rect_stroke(srect, round, m.border, border_col);
            }
            if selected {
                let off = m.edge;
                let outer = Rect::from_min_max(
                    Pos2::new(srect.min.x - off, srect.min.y - off),
                    Pos2::new(srect.max.x + off, srect.max.y + off),
                );
                let alpha =
                    if state.primary == Some(g.id) { 1.0 } else { SELECTION_REST_ALPHA };
                p.rect_stroke(
                    outer,
                    Rounding::same(round.nw + off),
                    m.edge,
                    selection_outline.with_alpha(alpha),
                );
            }
            if lod.glyphs() {
                let header_c = srect.min.y + m.header_h * zoom * 0.5;
                let mut x = srect.min.x + m.pad_x * zoom;
                if errored {
                    let r = m.pin_r * zoom * 0.8;
                    p.circle_filled(Pos2::new(x + r, header_c), r, status.error);
                    x += r * 2.0 + m.label_gap * zoom;
                }
                let (title_col, px) = match card.kind {
                    AnimCardKind::Entry => {
                        (fade(category_tag_color("Logic"), dim, bg), st.fonts.small * zoom)
                    }
                    AnimCardKind::Any => {
                        (fade(st.palette.text_secondary, dim, bg), st.fonts.small * zoom)
                    }
                    AnimCardKind::State => {
                        (fade(st.palette.text, dim, bg), st.fonts.body * zoom)
                    }
                };
                p.text(Pos2::new(x, header_c - px * 0.62), &g.title, px, title_col, None);
                if !g.tag.is_empty() {
                    let tag_px = m.tag_px * zoom;
                    let tw = p
                        .measure_text_family(&g.tag, tag_px, None, FontFamily::Mono)
                        .x;
                    p.text_family(
                        Pos2::new(
                            srect.max.x - m.pad_x * zoom - tw,
                            header_c - tag_px * 0.62,
                        ),
                        &g.tag,
                        tag_px,
                        fade(g.tag_color(), dim, bg),
                        None,
                        FontFamily::Mono,
                    );
                }
                if let Some(sub) = &card.subtitle {
                    let spx = st.fonts.small * zoom;
                    let sub_c = srect.min.y
                        + (m.header_h + (g.rect.height() - m.header_h) * 0.5) * zoom;
                    p.text_family(
                        Pos2::new(srect.min.x + m.pad_x * zoom, sub_c - spx * 0.62),
                        sub,
                        spx,
                        fade(st.palette.text_secondary, dim, bg),
                        None,
                        FontFamily::Mono,
                    );
                }
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
        //
        // The category edge is an *underlay-reveal*, not a strip drawn on top:
        // a 2px-tall rrect cannot round itself (the tessellator clamps the
        // radius to half the height, so radius 6 comes out as 1 and the strip's
        // square ends poke past the node's corners). The prototype got the
        // clipped look from CSS `overflow: hidden`; with no rounded clipping the
        // equivalent is to paint a band tall enough to round properly in the
        // edge color and then inset the fills 2px at the top. The exposed 2px
        // *is* the edge, and it follows the corner curve exactly.
        //
        // Total height is unchanged: the edge already lived inside `header_h`'s
        // top 2px, this only changes paint order and shape.
        let bg = st.palette.input;
        p.rect_filled(srect, round, fade(st.palette.header, dim, bg));
        // 2·radius tall with all four corners rounded, so below the top curve
        // the band tucks back inside the fills and leaves no colored fringe
        // down the node's sides.
        let band_h = (round.nw * 2.0).min(srect.height());
        p.rect_filled(
            Rect::from_min_size(srect.min, Vec2::new(srect.width(), band_h)),
            Rounding::same(round.nw.min(band_h * 0.5)),
            edge_col,
        );
        let edge_h = (m.edge * zoom).max(1.0).min(band_h);
        let inner_r = (round.nw - edge_h).max(0.0);
        let header_rect = Rect::from_min_max(
            Pos2::new(srect.min.x, srect.min.y + edge_h),
            Pos2::new(srect.max.x, srect.min.y + m.header_h * zoom),
        );
        p.rect_filled(
            header_rect,
            Rounding { nw: inner_r, ne: inner_r, sw: 0.0, se: 0.0 },
            fade(st.palette.elevated, dim, bg),
        );
        // 1px border — hairline, never scaled away. Status color reaches the
        // border on a node: there is no row to tint.
        p.rect_stroke(
            srect,
            round,
            m.border,
            match () {
                // Paused outranks errored on the border for the same reason it
                // does in the gutter: the stop is now.
                _ if paused_here => status.warning,
                _ if g.missing || errored => status.error,
                _ => st.palette.stroke,
            },
        );

        // Running (45-A P7): a node that fired recently gains an accent ring
        // at the pulse's own strength. Under the selection outline on purpose
        // — what you picked outranks what is happening. This is also the only
        // thing a **subgraph host** can show: nothing inside it is an edge of
        // the document being drawn, so its inner wires never pulse and the
        // node has to carry the whole statement.
        if let Some(a) = exec.map(|v| v.node_active(g.id)).filter(|a| *a > 0.0) {
            // **Outside** the selection outline's offset, not on it: the two
            // rings answer different questions ("what did I pick" vs "what is
            // running"), and drawn at the same offset the selection simply
            // painted over the pulse — a selected node stopped reporting that
            // it was executing, which is the one time you most want to know.
            let off = m.edge * 2.0;
            let outer = Rect::from_min_max(
                Pos2::new(srect.min.x - off, srect.min.y - off),
                Pos2::new(srect.max.x + off, srect.max.y + off),
            );
            p.rect_stroke(
                outer,
                Rounding::same(round.nw + off),
                m.edge,
                st.palette.accent_active.with_alpha(a),
            );
        }

        // Locate flash (GS-2): the node the panel just framed pulses once in
        // `focus_ring` — the token that means "this is what you asked for" —
        // and fades. Outside the selection outline like the running ring, so
        // it never overpaints what the selection is saying.
        if let Some(a) = state
            .flash
            .filter(|(id, _)| *id == g.id)
            .map(|(_, at)| 1.0 - (at.elapsed().as_millis() as f32 / FLASH_MS).clamp(0.0, 1.0))
            .filter(|a| *a > 0.0)
        {
            let off = m.edge * 3.0;
            let outer = Rect::from_min_max(
                Pos2::new(srect.min.x - off, srect.min.y - off),
                Pos2::new(srect.max.x + off, srect.max.y + off),
            );
            p.rect_stroke(
                outer,
                Rounding::same(round.nw + off),
                m.edge,
                st.palette.focus_ring.with_alpha(a),
            );
        }

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
            // precedence — **hit > error > invalid > armed > disabled**
            // (GS-4). A node stopped at outranks a node that cannot run,
            // because the stop is happening now and the error has been true
            // since before you pressed play; everything below error is a mark
            // you left for yourself.
            let badge = break_badge(
                g.breakpoint,
                errored || g.missing,
                paused_here,
                invalid_break,
            );
            let gutter = if let Some(badge) = badge {
                // Larger than the error disc on purpose: an octagon of the
                // same radius reads smaller (its corners are cut away), and at
                // the disc's size the facets vanish into a dot. ~40% of the
                // header's height, which is the mockup's proportion.
                let r = m.pin_r * zoom * BREAK_BADGE_R;
                let c = Pos2::new(
                    srect.min.x + m.pad_x * zoom + r,
                    srect.min.y + m.header_h * zoom * 0.5,
                );
                // An octagon, per the Badges spec: a stop sign reads as "halt"
                // at a glance and cannot be confused with the error disc.
                draw_break_badge(&mut p, st, c, r, badge, title_px);
                // The click target is the slot, not the glyph: an octagon of
                // six screen pixels is a dot to hit at 60% zoom.
                badge_rects.push((
                    g.id,
                    Rect::from_min_max(
                        Pos2::new(srect.min.x, srect.min.y),
                        Pos2::new(
                            c.x + r + m.label_gap * zoom * 0.5,
                            srect.min.y + m.header_h * zoom,
                        ),
                    ),
                ));
                r * 2.0 + m.label_gap * zoom
            } else if errored || g.missing {
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
            // 9px mono category tag, bright tone, right side of the header —
            // or, while this is the node execution is parked on, the mono
            // PAUSED chip in its place (GS-4). One slot, one occupant: the
            // category is a permanent fact and can wait a moment, and squeezing
            // both in overflows the header on any node with a short title.
            let tag_px = m.tag_px * zoom;
            let (tag, tag_col) = if paused_here {
                ("PAUSED", status.warning)
            } else {
                (g.tag.as_str(), fade(g.tag_color(), dim, bg))
            };
            let tag_w = p.measure_text_family(tag, tag_px, None, FontFamily::Mono).x;
            // The title is truncated *here*, against what the badge gutter and
            // this particular tag actually leave — the geometry pass sized the
            // node for the category tag and no badge, and both change under a
            // live session (a wider PAUSED chip, a gutter that appears when a
            // mark is set). Measuring at paint is the only place both are known.
            let title_avail =
                srect.width() - m.pad_x * zoom * 2.0 - gutter - tag_w - m.label_gap * zoom;
            let title = middle_truncate(&mut p, &g.title, title_px, title_avail.max(0.0));
            // Dimming reaches the *text*, not only the fills: a card whose
            // title still reads at full strength does not look excluded from
            // a search, which is the whole job of the 45% state.
            p.text(
                srect.min
                    + Vec2::new(m.pad_x * zoom + gutter, (m.header_h * zoom - title_px) * 0.5),
                &title,
                title_px,
                fade(st.palette.text, dim, bg),
                None,
            );
            p.text_family(
                Pos2::new(
                    srect.max.x - m.pad_x * zoom - tag_w,
                    srect.min.y + (m.header_h * zoom - tag_px) * 0.5,
                ),
                tag,
                tag_px,
                tag_col,
                None,
                FontFamily::Mono,
            );
        }

        // Waiting (GS-3): one indicator per node, never per activation. A 3px
        // accent bar under the title carries elapsed, mono carries remaining,
        // and a ×n chip names concurrent activations — two bars would imply
        // two nodes. Drawn while the rows are, so the LOD ladder takes it away
        // with them.
        if let Some(wait) = exec.filter(|_| lod.rows()).and_then(|v| v.wait(g.id)) {
            let bar = Rect::from_min_max(
                Pos2::new(srect.min.x, srect.min.y + m.header_h * zoom),
                Pos2::new(
                    srect.max.x,
                    srect.min.y + m.header_h * zoom + (WAIT_BAR_PX * m.scale * zoom).max(1.0),
                ),
            );
            p.rect_filled(bar, Rounding::ZERO, st.palette.input);
            p.rect_filled(
                Rect::from_min_max(
                    bar.min,
                    Pos2::new(bar.min.x + bar.width() * wait.fraction, bar.max.y),
                ),
                Rounding::ZERO,
                st.palette.accent_active,
            );
            if lod.glyphs() {
                let px = st.fonts.small * zoom;
                let text = format!("{:.1}s", wait.remaining);
                let tw = p.measure_text_family(&text, px, None, FontFamily::Mono).x;
                // Right of the title, left of the category tag: the countdown
                // belongs to the node's identity line, not to its body.
                let tag_w = p
                    .measure_text_family(&g.tag, m.tag_px * zoom, None, FontFamily::Mono)
                    .x;
                p.text_family(
                    Pos2::new(
                        srect.max.x - m.pad_x * zoom - tag_w - m.label_gap * zoom * 2.0 - tw,
                        srect.min.y + (m.header_h * zoom - px) * 0.5,
                    ),
                    &text,
                    px,
                    st.palette.focus_ring,
                    None,
                    FontFamily::Mono,
                );
                if wait.count > 1 {
                    let chip_text = format!("\u{d7}{}", wait.count);
                    let cw = p.measure_text_family(&chip_text, px, None, FontFamily::Mono).x;
                    let title_w = p.measure_text(&g.title, st.fonts.body * zoom, None).x;
                    let chip = Rect::from_min_size(
                        Pos2::new(
                            srect.min.x + m.pad_x * zoom + title_w + m.label_gap * zoom,
                            srect.min.y + (m.header_h * zoom - px * 1.6) * 0.5,
                        ),
                        Vec2::new(cw + m.label_gap * zoom * 2.0, px * 1.6),
                    );
                    p.rect_filled(chip, Rounding::same(m.radius * 0.4 * zoom), st.palette.accent_soft);
                    p.text_family(
                        Pos2::new(chip.min.x + m.label_gap * zoom, chip.center().y - px * 0.62),
                        &chip_text,
                        px,
                        st.palette.focus_ring,
                        None,
                        FontFamily::Mono,
                    );
                }
            }
        }

        // Runtime kill (GS-3): the error anchors to the node that raised it —
        // border and badge come from the existing error path (the host sets
        // `errored`), and the reason goes *under* the node as one mono line,
        // the same reason-at-rest rule the variables panel follows.
        if let Some(kill) = exec.and_then(|v| v.killed.as_ref()).filter(|k| k.node == Some(g.id)) {
            let px = st.fonts.small * zoom;
            if lod.glyphs() {
                p.text_family(
                    Pos2::new(srect.min.x, srect.max.y + m.label_gap * zoom * 1.5),
                    &kill.reason,
                    px,
                    status.error,
                    None,
                    FontFamily::Mono,
                );
            }
        }

        if !lod.rows() {
            continue;
        }

        let label_px = st.fonts.small * zoom;

        // Config band: pin-less rows for the reserved properties that shape
        // the node. It gets **its own surface** — an `input`-tone wash over the
        // node fill, closed by a hairline rule — so config reads as a form area
        // and the pin rows below it as connection rows, at rest, at low zoom
        // and under colour-vision deficiency (DESIGN-graphscripting ▸ S2).
        // Labels are secondary text; payload slugs are identifiers and go mono.
        if let Some(bottom) = g.band_bottom(m).filter(|_| lod.config_band()) {
            let band = Rect::from_min_max(
                Pos2::new(srect.min.x, srect.min.y + m.header_h * zoom),
                Pos2::new(
                    srect.max.x,
                    scope.world_to_screen(Pos2::new(g.rect.min.x, bottom)).y,
                ),
            );
            p.rect_filled(
                band,
                Rounding::ZERO,
                fade(fade(bg, CONFIG_BAND_WASH, st.palette.header), dim, bg),
            );
            for cfg in &g.config {
                let cy = scope.world_to_screen(Pos2::new(g.rect.min.x, cfg.y)).y;
                let cell = scope.world_rect_to_screen(cfg.cell);
                // The row label follows the **band**, not the pin labels: a
                // config row is label + value, and the mockup's own low-zoom
                // node keeps "Variable  Score" in a body whose pin labels have
                // already gone. It leaves with the band, one rung later.
                let at = Pos2::new(srect.min.x + m.pad_x * zoom, cy - label_px * 0.5);
                if cfg.mono_label() {
                    p.text_family(
                        at,
                        &cfg.label,
                        label_px,
                        st.palette.text_disabled,
                        None,
                        FontFamily::Mono,
                    );
                } else {
                    p.text(at, &cfg.label, label_px, st.palette.text_secondary, None);
                }
                // A dangling reference flags on the widget itself: the value is
                // still shown (the author has to see the name that broke) with
                // a warning outline the live L0 control does not draw for
                // itself. `draw_inline_readonly` already does this below L0.
                if lod.inline_widgets()
                    && matches!(
                        &cfg.kind,
                        InlineKind::Choice { ok: false, .. } | InlineKind::Enum { ok: false, .. }
                    )
                {
                    p.rect_stroke(
                        cell.expand(m.border),
                        Rounding::same((m.radius * 0.5 * zoom).max(1.0)),
                        m.border,
                        status.warning,
                    );
                }
                if lod.inline_widgets() {
                    pending_widgets.push((g.id, cfg.key.clone(), cell, cfg.kind.clone()));
                } else {
                    // Down to the last rung the band has: a config *value* is
                    // the row's whole content, so it outlives the row label
                    // (which drops with the pin labels) and goes only when the
                    // band itself goes. An empty band would be worse than none.
                    draw_inline_readonly(&mut p, cell, &cfg.kind, label_px, st, zoom, m);
                }
                // The ✕ is quiet at rest and revealed by hovering the band —
                // the system's hover-precedes rule. Removing a field is a
                // structural edit, so it never sits there inviting a misclick.
                if let Some(r) = cfg.remove.filter(|_| band_hovered) {
                    let sr = scope.world_rect_to_screen(r);
                    let hot = pointer_world.is_some_and(|pw| r.contains(pw));
                    p.text(
                        Pos2::new(sr.center().x - label_px * 0.35, sr.center().y - label_px * 0.5),
                        "\u{2715}",
                        label_px,
                        if hot { st.palette.text } else { st.palette.text_disabled },
                        None,
                    );
                }
            }
            // The "+ field" ghost row: dashed, inside the band, custom events
            // only. Adding a field is never breaking, so it is the one band
            // affordance that stays visible at rest.
            if let Some(r) = g.add_field {
                let sr = scope.world_rect_to_screen(r);
                let hot = pointer_world.is_some_and(|pw| r.contains(pw));
                let drafting = state
                    .payload
                    .draft
                    .as_ref()
                    .is_some_and(|d| d.node == g.id && d.slug.is_none());
                dashed_rect(
                    &mut p,
                    sr,
                    m.border,
                    if drafting {
                        st.palette.focus_ring
                    } else if hot {
                        st.palette.stroke_strong
                    } else {
                        st.palette.stroke
                    },
                );
                if drafting {
                    // Mid-add the row already wears the shape it is about to
                    // take: a compact slug field where the slug will live, and
                    // the type it will default to where the dropdown will be.
                    // Only the ✕ is missing — there is nothing to remove yet.
                    let cell = m.config_box(
                        r.max.x - m.config_remove_w() - m.label_gap - m.config_value_w(),
                        r.max.x - m.config_remove_w() - m.label_gap,
                        r.center().y,
                    );
                    draft_cell = Some(scope.world_rect_to_screen(m.config_box(
                        r.min.x + m.label_gap,
                        (r.min.x + m.label_gap + m.config_value_w() * 0.5).min(cell.min.x - m.label_gap),
                        r.center().y,
                    )));
                    draw_inline_readonly(
                        &mut p,
                        scope.world_rect_to_screen(cell),
                        &InlineKind::Chip(DEFAULT_PAYLOAD_TYPE.to_string()),
                        label_px,
                        st,
                        zoom,
                        m,
                    );
                } else {
                    let text = "+ field";
                    let w = p.measure_text(text, label_px, None).x;
                    p.text(
                        Pos2::new(sr.center().x - w * 0.5, sr.center().y - label_px * 0.5),
                        text,
                        label_px,
                        if hot { st.palette.text_secondary } else { st.palette.text_disabled },
                        None,
                    );
                }
            }
            // A rename in flight replaces the slug in place.
            if let Some(slug) = state
                .payload
                .draft
                .as_ref()
                .filter(|d| d.node == g.id)
                .and_then(|d| d.slug.clone())
            {
                if let Some(cfg) = g
                    .config
                    .iter()
                    .find(|c| c.payload_slug() == Some(slug.as_str()))
                {
                    draft_cell = Some(scope.world_rect_to_screen(cfg.label_box));
                }
            }
            p.line_segment(
                Pos2::new(srect.min.x, band.max.y),
                Pos2::new(srect.max.x, band.max.y),
                m.border,
                st.palette.stroke,
            );
        }

        for pin in &g.pins {
            // A hidden pin (Task 41 rework) is an anchor, not an affordance:
            // no dot, no label — nothing to draw.
            if pin.hidden {
                continue;
            }
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
                    fade(st.palette.text_secondary, dim, bg),
                    None,
                );
            } else {
                let x = srect.min.x + m.label_inset() * zoom;
                let lw = p
                    .text(
                        Pos2::new(x, c.y - label_px * 0.5),
                        &label,
                        label_px,
                        fade(
                            if pin.ghost {
                                st.palette.text_disabled
                            } else {
                                st.palette.text_secondary
                            },
                            dim,
                            bg,
                        ),
                        None,
                    )
                    .x;
                // Inline value cell, right-aligned inside the row.
                if let Some(kind) = &pin.inline {
                    // Flows after the label, but never past the width the
                    // auto-sizer reserved for it — `pin.value_w` is the very
                    // number the node was sized around, so the cell lands
                    // inside the node by construction.
                    let cell =
                        inline_cell_rect(srect, x + lw, pin.value_w, c.y, m, zoom);
                    if lod.inline_widgets()
                        && matches!(
                            kind,
                            InlineKind::Float(_)
                                | InlineKind::Int(_)
                                | InlineKind::Bool(_)
                                | InlineKind::Str(_)
                                | InlineKind::Enum { .. }
                        )
                    {
                        pending_widgets.push((g.id, pin.slug.clone(), cell, kind.clone()));
                    } else if lod.values() {
                        draw_inline_readonly(&mut p, cell, kind, label_px, st, zoom, m);
                    }
                }
            }
        }
    }

    // Watch chips (GS-3). Drawn after the nodes so a chip is never buried by
    // the node it hangs off, and only where its pin actually is: the chip is a
    // label on a wire's source, not a floating annotation.
    if lod.pin_labels() {
        let mut p = ui.painter();
        for w in &state.watches {
            let Some(g) = geoms.iter().find(|g| g.id == w.node) else {
                continue;
            };
            let Some(pin) = g.pins.iter().find(|q| q.output == w.output && q.slug == w.pin) else {
                continue;
            };
            if !overlaps(g.rect, vis) {
                continue;
            }
            let c = scope.world_to_screen(pin.dot_center);
            let text = watch_chip_text(w.last.as_deref());
            let px = st.fonts.small * zoom;
            let tw = p.measure_text_family(&text, px, None, FontFamily::Mono).x;
            let h = m.row_h * zoom * 0.82;
            let gap = m.label_gap * zoom * 2.0;
            let chip = if w.output {
                Rect::from_min_size(
                    Pos2::new(scope.world_rect_to_screen(g.rect).max.x + gap, c.y - h * 0.5),
                    Vec2::new(tw + gap * 2.0, h),
                )
            } else {
                Rect::from_min_size(
                    Pos2::new(
                        scope.world_rect_to_screen(g.rect).min.x - gap - (tw + gap * 2.0),
                        c.y - h * 0.5,
                    ),
                    Vec2::new(tw + gap * 2.0, h),
                )
            };
            // States, per the design: dashed until a value has ever arrived
            // (and in edit mode, where the last run's value is residue rather
            // than truth); dimmed with an age tag once it has sat unchanged.
            let live_value = exec.is_some() && w.last.is_some();
            // …but a paused session cannot go stale (GS-4): the value stopped
            // changing because the graph stopped running, and dimming the one
            // frame you deliberately froze would be the opposite of the help.
            let stale = if exec.is_some_and(|v| v.paused.is_some()) {
                None
            } else {
                w.stale_for(WATCH_STALE_SECS)
            };
            let alpha = if stale.is_some() || !live_value { 0.55 } else { 1.0 };
            // A stale chip dims *as a chip* — fill, border, type edge and text
            // together. Dimming only the number left a full-strength frame
            // around a faded value, which reads as a rendering fault rather
            // than as "this is old" (DESIGN-graphscripting ▸ Surface 3).
            //
            // `input` at 94%, translucent rather than glass: the chip floats
            // over the canvas and has to admit what is behind it.
            p.rect_filled_translucent(
                chip,
                st.rounding.small,
                st.palette.input.with_alpha(0.94 * alpha),
            );
            if live_value {
                p.rect_stroke(
                    chip,
                    st.rounding.small,
                    m.border,
                    st.palette.stroke_strong.with_alpha(alpha),
                );
            } else {
                dashed_rect(&mut p, chip, m.border, st.palette.stroke_strong.with_alpha(alpha));
            }
            // 2px type-coloured left edge — which wire this value came off.
            p.rect_filled(
                Rect::from_min_size(chip.min, Vec2::new((m.edge * zoom).max(1.0), chip.height())),
                Rounding::ZERO,
                pin_color(Some(registry), &pin.ty).with_alpha(alpha),
            );
            p.text_family(
                Pos2::new(chip.min.x + gap, chip.center().y - px * 0.62),
                &text,
                px,
                st.palette.text_mono.with_alpha(alpha),
                None,
                FontFamily::Mono,
            );
            if let Some(secs) = stale {
                let age = format!("{secs:.0}s");
                let aw = p.measure_text_family(&age, px * 0.9, None, FontFamily::Mono).x;
                p.text_family(
                    Pos2::new(chip.max.x - gap * 0.5 - aw, chip.center().y - px * 0.62),
                    &age,
                    px * 0.9,
                    st.palette.text_disabled,
                    None,
                    FontFamily::Mono,
                );
            }
            watch_rects.push((w.node, w.pin.clone(), w.output, chip));
        }
    }

    // Per-node previews: a 64x64 well below the rows, L0 only, and only for
    // this frame's slice of the budget. The well is a labeled placeholder —
    // real render-target / curve content arrives with Tasks 50 and 41; the
    // geometry, the LOD gate and the budget land now so they do not have to
    // be retrofitted around live content.
    if lod.inline_widgets() {
        let slots: Vec<&NodeGeom> = geoms
            .iter()
            .filter(|g| g.preview.is_some() && g.rect.intersect(vis).width() > 0.0)
            .collect();
        let (start, take) =
            preview_slice(slots.len(), frame, PREVIEW_BUDGET_PER_FRAME);
        let mut p = ui.painter();
        for g in slots.iter().skip(start).take(take) {
            let Some(kind) = g.preview else { continue };
            let srect = scope.world_rect_to_screen(g.rect);
            let side = m.preview_side() * zoom;
            let well = Rect::from_min_size(
                Pos2::new(
                    srect.min.x + m.pad_x * zoom,
                    srect.max.y - m.body_pad * zoom - side,
                ),
                Vec2::splat(side),
            );
            p.rect_filled(well, Rounding::same(m.radius * zoom), st.palette.input);
            p.rect_stroke(
                well,
                Rounding::same(m.radius * zoom),
                m.border,
                st.palette.stroke,
            );
            let px = m.tag_px * zoom;
            let label = kind.label();
            let tw = p.measure_text_family(label, px, None, FontFamily::Mono).x;
            p.text_family(
                Pos2::new(well.center().x - tw * 0.5, well.center().y - px * 0.62),
                label,
                px,
                st.palette.text_disabled,
                None,
                FontFamily::Mono,
            );
        }
    }

    // Editable widgets, after the painter borrow ends.
    for (node, slug, cell, kind) in pending_widgets {
        widget_rects.push(cell);
        inline_widget(ui, state, registry, node, &slug, cell, &kind, zoom);
    }
    // The payload name entry is a widget like any other — it just lives in a
    // row that is not a property yet (add) or is about to change key (rename).
    if let Some(cell) = draft_cell {
        widget_rects.push(cell);
        payload_draft_widget(ui, state, cell, zoom);
    }
}

/// The in-flight payload-field name entry: one text field drawn in the band
/// row it belongs to.
///
/// It only ever **reports** — Enter sets `submitted` and the panel decides what
/// that means, because "does this need a confirmation" is a cross-document
/// question the drawing pass has no resolver for. Escape and losing focus both
/// cancel, which is the canvas rule for every other in-flight gesture.
fn payload_draft_widget(ui: &mut Ui, state: &mut GraphEditorState, cell: Rect, zoom: f32) {
    let saved = ui.ctx().style;
    {
        let s = &mut ui.ctx_mut().style;
        s.fonts.body *= zoom;
        s.fonts.small *= zoom;
        s.fonts.mono *= zoom;
        s.spacing.item *= zoom;
        s.metrics.control_height *= zoom;
        s.metrics.row_height *= zoom;
    }
    let Some(draft) = state.payload.draft.as_mut() else {
        ui.ctx_mut().style = saved;
        return;
    };
    let first = draft.first_frame;
    let (mut submitted, mut cancelled, mut focused) = (false, false, false);
    let field_bg = ui.style().palette.input;
    ui.run_at(
        cell,
        Direction::TopDown,
        Id::new(("graph_payload_draft", draft.node)),
        UiOptions { padding: Vec2::ZERO, spacing: 0.0 },
        |ui| {
            let out = TextEdit::new(&mut draft.name)
                .hint("field")
                .width(cell.width())
                .fill(field_bg)
                .request_focus(first)
                .show_full(ui);
            submitted = out.submitted;
            cancelled = out.cancelled;
            focused = out.focused;
        },
    );
    ui.ctx_mut().style = saved;
    draft.first_frame = false;
    draft.seen_focus |= focused;
    let gave_up_focus = draft.seen_focus && !focused;
    if submitted {
        draft.submitted = true;
    } else if cancelled || gave_up_focus {
        state.payload.draft = None;
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
        InlineKind::Enum { value, ok, .. } | InlineKind::Choice { value, ok, .. } => {
            // Out-of-list values read as a warning, not an error.
            let col = if *ok { st.palette.text_mono } else { status.warning };
            p.rect_filled(cell, round, st.palette.input);
            if !*ok {
                p.rect_stroke(cell, round, m.border, status.warning);
            }
            let w = cell.width() - m.label_gap * 2.0 * zoom;
            let shown = flagged_value(value, *ok);
            let text = clip_text(p, &shown, px, w);
            p.text_family(
                Pos2::new(cell.min.x + m.label_gap * zoom, cell.center().y - px * 0.5),
                &text,
                px,
                col,
                None,
                FontFamily::Mono,
            );
        }
        other => {
            let text = match other {
                InlineKind::Float(x) => format!("{x}"),
                InlineKind::Int(i) => i.to_string(),
                InlineKind::Bool(b) => b.to_string(),
                InlineKind::Str(s) | InlineKind::Chip(s) => s.clone(),
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

/// What a dropdown shows for a value that is not one of its variants — a
/// deleted variable, a stale enum: **the name that broke, marked**. The name
/// has to stay (it is the only clue to what the author meant) and the mark has
/// to be readable without colour, which is why the `?` is in the text rather
/// than only in the tint (DESIGN-graphscripting ▸ S2, dangling reference).
fn flagged_value(value: &str, ok: bool) -> String {
    if ok || value.is_empty() {
        value.to_string()
    } else {
        format!("{value} ?")
    }
}

fn chan(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Does an embedded control own the pointer? `rects` are the screen boxes the
/// drawing pass handed to real crusty widgets this frame.
///
/// The canvas is drawn back-to-front but *interacts* front-to-back, and the
/// embedded widgets run first (inside `draw_nodes`). Every canvas-level
/// gesture below them therefore has to ask this before it so much as calls
/// `interact` — that call claims `active_widget` for whatever id it is given,
/// which is how a node body used to swallow the press a `DragValue` had just
/// taken, leaving the field hoverable but not editable.
fn widget_owns(pointer: Pos2, rects: &[Rect]) -> bool {
    rects.iter().any(|r| r.contains(pointer))
}

/// Width a pin row needs once an inline cell of `value_w` is added to a label
/// row already `label_row_w` wide (label inset + glyphs). The trailing `pad_x`
/// is the same one [`inline_cell_rect`] keeps clear on the right — the two
/// have to agree or the cell gets clamped over its own label, or off the node.
fn inline_row_w(label_row_w: f32, value_w: f32, m: &GraphMetrics) -> f32 {
    label_row_w + m.label_gap + value_w + m.pad_x
}

/// Screen rect of an inline value cell. It flows after the row label, but is
/// clamped so its right edge never passes the node's padding — with the width
/// [`inline_row_w`] reserved, the clamp is what guarantees "inside the node"
/// even when the label ran long.
fn inline_cell_rect(
    node: Rect,
    label_end_x: f32,
    value_w: f32,
    row_center_y: f32,
    m: &GraphMetrics,
    zoom: f32,
) -> Rect {
    let w = value_w * zoom;
    let x = (label_end_x + m.label_gap * zoom).min(node.max.x - m.pad_x * zoom - w);
    Rect::from_min_size(
        Pos2::new(x, row_center_y - m.row_h * zoom * 0.4),
        Vec2::new(w, m.row_h * zoom * 0.8),
    )
}

/// Width the inline cell of `kind` needs, world units.
///
/// Content-shaped: text, numbers and dropdowns all size to what they hold
/// (with the reserved [`GraphMetrics::value_w`] as the floor), because the
/// bar is *every field readable at 100% zoom without interaction* — "le…"
/// for `less_equal` teaches nothing. Measured at **both** fonts the cell can
/// be painted in — the live widget at L0 draws body-size, the L1 fallback
/// draws small mono — because the sizer has to hold whichever the zoom picks.
fn inline_cell_w(p: &mut Painter, kind: &InlineKind, m: &GraphMetrics, st: &Style) -> f32 {
    fn both(p: &mut Painter, st: &Style, s: &str) -> f32 {
        let body = p.measure_text(s, st.fonts.body, None).x;
        let mono = p
            .measure_text_family(s, st.fonts.small, None, FontFamily::Mono)
            .x;
        body.max(mono)
    }
    match kind {
        InlineKind::Str(s) => m.text_value_w(both(p, st, s)),
        // Numbers get air around the digits, not a tight sleeve.
        InlineKind::Float(x) => {
            let t = format!("{x}");
            m.text_value_w(both(p, st, &t) + m.label_gap * 2.0)
        }
        InlineKind::Int(i) => {
            let t = i.to_string();
            m.text_value_w(both(p, st, &t) + m.label_gap * 2.0)
        }
        // A dropdown sizes its closed box to the longest option. The chrome
        // mirrors the `ComboBox` trigger: 16 of text insets, 4 text→arrow
        // gap, the arrow itself, a little slack. The usual cap applies, but
        // yields whenever even the *current* value would not fit whole —
        // the selected value never truncates.
        InlineKind::Enum { value, variants, ok }
        | InlineKind::Choice { value, variants, ok } => {
            let arrow = p.measure_text("\u{25BC}", st.fonts.body, None).x;
            let chrome = 20.0 + arrow + 2.0;
            let cur = both(p, st, &flagged_value(value, *ok));
            let longest = variants.iter().fold(cur, |w, v| w.max(both(p, st, v)));
            (longest + chrome).clamp(m.value_w, (BASE_VALUE_W_MAX * m.scale).max(cur + chrome))
        }
        _ => m.value_w,
    }
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
            InlineKind::Int(v) => {
                // `DragValue` is f32-only, so an Int rides through one: no
                // decimals on display, rounded on the way back, and a slower
                // scrub than Float because every pixel of travel is a whole
                // unit of meaning.
                let mut x = *v as f32;
                DragValue::new(&mut x)
                    .speed(0.05)
                    .decimals(0)
                    .min_decimals(0)
                    .width(cell.width())
                    .height(cell.height())
                    .show(ui);
                let n = x.round() as i32;
                if n != *v {
                    changed = Some(PropValue::Int(n));
                }
            }
            InlineKind::Bool(v) => {
                let mut b = *v;
                Checkbox::new(&mut b, "").show(ui);
                if b != *v {
                    changed = Some(PropValue::Bool(b));
                }
            }
            InlineKind::Str(v) => {
                let mut t = v.clone();
                let field_bg = ui.style().palette.input;
                TextEdit::new(&mut t)
                    .width(cell.width())
                    .fill(field_bg)
                    .show_full(ui);
                if t != *v {
                    changed = Some(PropValue::Str(t));
                }
            }
            InlineKind::Enum { value, variants, .. }
            | InlineKind::Choice { value, variants, .. } => {
                // `SelectableValue` wants a `Copy` value, so the selection is
                // carried as an index and mapped back to the string.
                let now = variants.iter().position(|v| v == value);
                let mut picked = now.unwrap_or(usize::MAX);
                let shown = flagged_value(value, now.is_some());
                // The open list renders every item fully: each row paints 8px
                // insets plus the right-aligned ✓, so the popup grows to its
                // widest row instead of clipping it (fonts here are already
                // zoom-scaled, so the measure matches what the rows draw).
                let popup_w = {
                    let font = ui.style().fonts.body;
                    let mut p = ui.painter();
                    let check = p.measure_text("\u{2713}", font, None).x;
                    let longest = variants
                        .iter()
                        .fold(0.0f32, |w, v| w.max(p.measure_text(v, font, None).x));
                    (longest + check + 28.0).max(cell.width())
                };
                ComboBox::new("graph_enum")
                    .selected_text(shown.as_str())
                    .width(cell.width())
                    .popup_width(popup_w)
                    .show_ui(ui, |ui| {
                        for (i, v) in variants.iter().enumerate() {
                            SelectableValue::new(&mut picked, i, v.as_str()).show(ui);
                        }
                    });
                if picked != now.unwrap_or(usize::MAX) {
                    if let Some(v) = variants.get(picked) {
                        // The write-back shape follows the *variant*, not the
                        // widget: `var` is a `Str` on disk and must stay one,
                        // or `DocDescriptors` stops resolving the node.
                        changed = Some(match kind {
                            InlineKind::Choice { .. } => PropValue::Str(v.clone()),
                            _ => PropValue::Enum(v.clone()),
                        });
                    }
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

/// A pin's descriptor doc line, if this node *instance*'s pin declares one.
/// Through the resolver, so a subgraph or variable node answers from the
/// document instead of silently having no pins.
fn pin_doc(
    registry: &NodeRegistry,
    resolver: &DocResolvers<'_>,
    state: &GraphEditorState,
    node: u64,
    slug: &str,
    output: bool,
) -> Option<String> {
    let d = resolver.bind(&state.doc, registry).descriptor(node)?;
    if output { d.output(slug) } else { d.input(slug) }?.doc.clone()
}

/// A node's doc line, if it declares one.
fn node_doc(
    registry: &NodeRegistry,
    resolver: &DocResolvers<'_>,
    state: &GraphEditorState,
    node: u64,
) -> Option<String> {
    resolver
        .bind(&state.doc, registry)
        .descriptor(node)?
        .doc
        .clone()
}

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
            let w = pin.hit_w.unwrap_or(hit_w);
            if Rect::from_center_size(pin.dot_center, Vec2::splat(w)).contains(pw) {
                return Some((g.id, pin.slug.clone(), pin.ty.clone(), pin.output));
            }
        }
    }
    None
}

/// Is this pin a wildcard — an unwired reroute that has not adopted a type
/// yet? Such an endpoint accepts anything; refusing it is what made an empty
/// reroute impossible to connect in either direction.
fn pin_is_untyped(geoms: &[NodeGeom], node: u64, slug: &str, output: bool) -> bool {
    geoms
        .iter()
        .find(|g| g.id == node)
        .and_then(|g| {
            g.pins
                .iter()
                .find(|p| p.output == output && p.slug == slug)
        })
        .is_some_and(|p| p.untyped)
}

fn node_under(geoms: &[NodeGeom], pw: Pos2, lod: ZoomLod, m: &GraphMetrics) -> Option<u64> {
    geoms
        .iter()
        .rev()
        .find(|g| g.body_rect(lod, m).contains(pw))
        .map(|g| g.id)
}

/// Resolve a released pin drag onto another pin. Returns whether it landed —
/// `false` hands the release to the palette / auto-connect path.
fn resolve_connection(
    state: &mut GraphEditorState,
    geoms: &[NodeGeom],
    pointer_world: Option<Pos2>,
    hit_w: f32,
    registry: &NodeRegistry,
) -> bool {
    let Some((from_node, from_pin, from_output)) = state
        .connect_drag
        .as_ref()
        .map(|d| (d.from_node, d.from_pin.clone(), d.from_output))
    else {
        return false;
    };
    let (Some(pw), Some(src_ty)) =
        (pointer_world, pin_ty(geoms, from_node, &from_pin, from_output))
    else {
        return false;
    };
    let Some((tn, ts, tty, to)) = pin_under(geoms, pw, hit_w) else {
        return false;
    };
    if let Some(edge) = validate_connection(
        state,
        registry,
        from_node,
        &from_pin,
        from_output,
        &src_ty,
        pin_is_untyped(geoms, from_node, &from_pin, from_output),
        tn,
        &ts,
        to,
        &tty,
        pin_is_untyped(geoms, tn, &ts, to),
    ) {
        // The state-machine drag (Task 41): a flow wire dropped state → state
        // means "make a transition here", never a bare edge the compiler
        // would silently ignore.
        if state.domain.is_animation() {
            if let Some((a, b)) = super::graph_editor::transition_shortcut(&state.doc, &edge) {
                state.insert_transition_between(a, b, registry);
                return true;
            }
        }
        state.doc.edges.push(edge.clone());
        state.commit(GraphEdit::Connect(edge), registry);
        return true;
    }
    // A refused drop on a real pin is still a landing: opening the palette
    // over it would read as the editor ignoring what the user aimed at.
    true
}

/// Release on a node body wires to that node's best compatible pin — the
/// right type first, then the closest name.
fn auto_connect(
    state: &mut GraphEditorState,
    registry: &NodeRegistry,
    resolver: &DocResolvers<'_>,
    src: &PaletteDragSource,
    target: u64,
) {
    // Instance pins, not type pins: dropping a wire on a subgraph or a
    // variable node has to see the pins the canvas is drawing.
    let desc = {
        let d = resolver.bind(&state.doc, registry);
        match d.descriptor(target) {
            Some(d) => d.into_owned(),
            None => return,
        }
    };
    let filter = PinFilter { ty: src.ty.clone(), need_input: src.output };
    let Some(pin) = graph_palette::auto_connect_pin(&desc, &filter, &src.label) else {
        state.toast("No compatible pin");
        return;
    };
    let edge = if src.output {
        Edge {
            from_node: src.node,
            from_pin: src.pin.clone(),
            to_node: target,
            to_pin: pin.slug.clone(),
        }
    } else {
        Edge {
            from_node: target,
            from_pin: pin.slug.clone(),
            to_node: src.node,
            to_pin: src.pin.clone(),
        }
    };
    // What a second wire replaces depends on which end is which:
    //
    // - a *data input* takes one edge, so a second drop replaces it;
    // - an *exec input* may fan in, so a second drop is an addition —
    //   silently breaking a converging wire would delete visible work;
    // - an *exec output* takes one edge (review ruling: `PlanNode::exec` is
    //   one target per pin, and `Sequence` is how you run two things), so
    //   dragging a second continuation off it replaces the first rather than
    //   authoring a document validation is about to refuse.
    let is_exec = pin.ty == PinType::Exec;
    // A flow-like domain pin (Task 41) fans in and out freely, so a new wire
    // never breaks an existing one — and a state-to-state drop means "make a
    // transition here" (the state-machine drag), same as a pin-to-pin drop.
    if matches!(&pin.ty, PinType::Domain(k) if registry.domain_is_flow(k)) {
        if state.domain.is_animation() {
            if let Some((a, b)) = super::graph_editor::transition_shortcut(&state.doc, &edge) {
                state.insert_transition_between(a, b, registry);
                return;
            }
        }
        let edit = GraphEdit::Connect(edge);
        edit.apply(&mut state.doc);
        state.commit(edit, registry);
        return;
    }
    let existing: Vec<Edge> = if is_exec {
        state
            .doc
            .edges
            .iter()
            .filter(|e| e.from_node == edge.from_node && e.from_pin == edge.from_pin)
            .cloned()
            .collect()
    } else {
        state
            .doc
            .edges
            .iter()
            .filter(|e| e.to_node == edge.to_node && e.to_pin == edge.to_pin)
            .cloned()
            .collect()
    };
    let mut edits: Vec<GraphEdit> = Vec::new();
    if !existing.is_empty() {
        let indexed: Vec<(usize, Edge)> = state
            .doc
            .edges
            .iter()
            .enumerate()
            .filter(|(_, e)| existing.contains(e))
            .map(|(i, e)| (i, e.clone()))
            .collect();
        edits.push(GraphEdit::Disconnect { edges: indexed });
    }
    edits.push(GraphEdit::Connect(edge));
    let edit = GraphEdit::Composite { label: "Connect".to_string(), edits };
    edit.apply(&mut state.doc);
    state.commit(edit, registry);
}

/// `finish_node_drag` for state-level tests, which have no `Ui` to drive.
#[cfg(test)]
pub(super) fn finish_node_drag_for_test(
    state: &mut GraphEditorState,
    registry: &NodeRegistry,
) {
    finish_node_drag(state, registry)
}

fn finish_node_drag(state: &mut GraphEditorState, registry: &NodeRegistry) {
    let Some(mut drag) = state.node_drag.take() else {
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
    let moved = delta[0].abs() > f32::EPSILON || delta[1].abs() > f32::EPSILON;
    let mv = moved.then_some(GraphEdit::MoveNodes { ids, delta });

    // A midpoint grab applied its reroute insert without recording it, so
    // that the whole gesture — insert *and* move — is one undo entry.
    match (drag.pending.take(), mv) {
        (Some(pending), Some(mv)) => {
            let label = pending.description();
            state.commit(
                GraphEdit::Composite { label, edits: vec![pending, mv] },
                registry,
            );
        }
        (Some(pending), None) => state.commit(pending, registry),
        (None, Some(mv)) => state.commit(mv, registry),
        (None, None) => {}
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
        // Translucent, not glass: the marquee has to reveal the nodes it is
        // sweeping over. `rect_filled` treats alpha as a glass tint strength
        // over a *blurred* backdrop and emits an opaque interior, which turned
        // this into a solid slab that hid everything under it.
        p.rect_filled_translucent(
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
    open_subgraph: &mut Option<GraphOpenRequest>,
    keymap: &Keymap,
    registry: &NodeRegistry,
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
        // Tooltips that describe a bound action name its chord, so the
        // keyboard route is learnable from the mouse route.
        let tip = match keymap.chord_label(Action::NEXT_ERROR) {
            Some(c) => format!("Click to cycle validation errors  ({c})"),
            None => "Click to cycle validation errors".to_string(),
        };
        ui.tooltip_for(chip, &tip);
    }
    if resp.clicked {
        cycle_error(state, errors, geoms, viewport, zoom_min, zoom_max, frame_request, registry);
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
        rows.push(e.text());
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
                    // A jump between peers, not a descent — no chain.
                    *open_subgraph = Some(GraphOpenRequest::jump(first.clone()));
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
    registry: &NodeRegistry,
) {
    if errors.ordered.is_empty() {
        return;
    }
    // Skip the doc-level ones: there is nothing on the canvas to frame.
    let anchored: Vec<&IndexedError> = errors
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
    // A refusal naming a node *inside* the transition's rule descends
    // (ticket 05): the peek opens on the transition with the culprit
    // selected and flashing, so F8 never strands the eye at the chip.
    if let IndexedError::Domain(e) = anchored[i] {
        if let (Some(owner), Some(inner)) = (e.node, e.region_node) {
            state.open_rule_scope_at(owner, inner, registry);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A bare node instance, for the geometry/config tests.
    fn test_node(id: u64, type_id: &str) -> NodeInst {
        NodeInst {
            id,
            type_id: type_id.to_string(),
            type_version: 1,
            position: [0.0, 0.0],
            properties: BTreeMap::new(),
            subgraph: None,
            tint: None,
            title: None,
        }
    }

    /// Ticket 05: the peek's scrim is `canvas − lit nodes` as bands. The
    /// bands must tile exactly — full coverage minus the holes, no overlap —
    /// or the dim reads blotchy over the machine.
    #[test]
    fn scrim_bands_tile_the_complement_of_the_lit_rects() {
        let outer = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(100.0, 100.0));
        let holes = [
            Rect::from_min_max(Pos2::new(10.0, 10.0), Pos2::new(30.0, 30.0)),
            Rect::from_min_max(Pos2::new(20.0, 20.0), Pos2::new(50.0, 40.0)), // overlaps
            Rect::from_min_max(Pos2::new(90.0, 95.0), Pos2::new(120.0, 130.0)), // clipped
        ];
        let bands = subtract_rects(outer, &holes);
        // Sampled coverage: a point is under exactly one band iff it is in
        // the outer rect and in no hole.
        for xi in 0..40 {
            for yi in 0..40 {
                let p = Pos2::new(xi as f32 * 2.5 + 1.2, yi as f32 * 2.5 + 1.2);
                let in_hole = holes.iter().any(|h| h.contains(p));
                let covered = bands.iter().filter(|b| b.contains(p)).count();
                assert_eq!(covered, usize::from(!in_hole), "at {p:?}");
            }
        }
        // No holes: one band, the whole canvas.
        assert_eq!(subtract_rects(outer, &[]), vec![outer]);
    }

    /// Task 41 polish: a user-placed peek stays fully on the canvas at a
    /// workable size — the offset is canvas-relative, the min size is the
    /// floor, and an off-canvas placement clamps back in.
    #[test]
    fn a_user_placed_peek_clamps_onto_the_canvas() {
        let canvas = Rect::from_min_max(Pos2::new(100.0, 50.0), Pos2::new(1100.0, 750.0));
        // A placement well inside passes through untouched.
        let r = peek_user_rect(canvas, [40.0, 60.0], [400.0, 300.0], 1.0);
        assert_eq!(r, Rect::from_min_size(Pos2::new(140.0, 110.0), Vec2::new(400.0, 300.0)));
        // Sizes below the minimum grow to it.
        let min = peek_min_size(1.0);
        let r = peek_user_rect(canvas, [40.0, 60.0], [10.0, 10.0], 1.0);
        assert_eq!(r.size(), min);
        // Dragged past the bottom-right corner: pulled back inside.
        let r = peek_user_rect(canvas, [5000.0, 5000.0], [400.0, 300.0], 1.0);
        assert!(r.max.x <= canvas.max.x && r.max.y <= canvas.max.y);
        // …and past the top-left.
        let r = peek_user_rect(canvas, [-5000.0, -5000.0], [400.0, 300.0], 1.0);
        assert!(r.min.x >= canvas.min.x && r.min.y >= canvas.min.y);
        // A size larger than the canvas caps at the canvas.
        let r = peek_user_rect(canvas, [0.0, 0.0], [9000.0, 9000.0], 1.0);
        assert_eq!(r.size(), canvas.size());
    }

    /// The peek's border zones: edges resize one side, corners two, the
    /// interior none — and a point outside the panel is nobody's grip.
    #[test]
    fn peek_resize_zones_read_edges_and_corners() {
        let panel = Rect::from_min_max(Pos2::new(100.0, 100.0), Pos2::new(500.0, 400.0));
        // Left edge, clear of the corners.
        assert_eq!(
            peek_resize_zone(panel, Pos2::new(103.0, 250.0), 1.0),
            Some((true, false, false, false))
        );
        // Bottom edge.
        assert_eq!(
            peek_resize_zone(panel, Pos2::new(300.0, 398.0), 1.0),
            Some((false, false, false, true))
        );
        // Bottom-right corner: the 14px corner widens the diagonal grab.
        assert_eq!(
            peek_resize_zone(panel, Pos2::new(497.0, 390.0), 1.0),
            Some((false, true, false, true))
        );
        // Top-left corner via the top strip.
        assert_eq!(
            peek_resize_zone(panel, Pos2::new(105.0, 103.0), 1.0),
            Some((true, false, true, false))
        );
        // Interior: no grip (the header's move handle takes it instead).
        assert_eq!(peek_resize_zone(panel, Pos2::new(300.0, 250.0), 1.0), None);
        // Outside the panel entirely.
        assert_eq!(peek_resize_zone(panel, Pos2::new(50.0, 250.0), 1.0), None);
    }

    /// 45-A P7. A reroute is transparent at run time, so the interpreter
    /// records against the real producer — and *both* halves of a rerouted
    /// wire have to light from that one record, or a wire visibly stops
    /// mid-air at the dot.
    #[test]
    fn a_pulse_resolves_through_reroutes() {
        let mut state = crate::engine::editor::graph_editor::test_state("graphs/t.graph");
        state.doc.nodes = vec![
            test_node(1, "add_int"),
            test_node(2, REROUTE_TYPE_ID),
            test_node(3, "print"),
        ];
        state.doc.edges = vec![
            Edge {
                from_node: 1,
                from_pin: "result".into(),
                to_node: 2,
                to_pin: REROUTE_IN.into(),
            },
            Edge {
                from_node: 2,
                from_pin: REROUTE_OUT.into(),
                to_node: 3,
                to_pin: "text".into(),
            },
        ];

        let mut viz = GraphExecViz::new("Duck");
        viz.add_pulse(1, "result", 0.8);
        viz.set_value(1, "result", "7".into());

        // Both halves of the chain light from the one record…
        assert_eq!(wire_pulse(&viz, &state, &state.doc.edges[0]), 0.8);
        assert_eq!(wire_pulse(&viz, &state, &state.doc.edges[1]), 0.8);
        // …and the value reaches the far end's input pin.
        assert_eq!(pin_value(&viz, &state, 3, "text", false), Some("7"));
        assert_eq!(pin_value(&viz, &state, 1, "result", true), Some("7"));
        // An unwired input has no wire, so no value — its constant is already
        // drawn in its own field.
        assert_eq!(pin_value(&viz, &state, 1, "a", false), None);
    }

    /// The converge rank the router's second stagger key reads: a wire's
    /// position within the set landing on the same input pin.
    #[test]
    fn converge_ranks_count_only_the_same_target_pin() {
        let to = |from: u64, node: u64, pin: &str| Edge {
            from_node: from,
            from_pin: "exec_out".into(),
            to_node: node,
            to_pin: pin.into(),
        };
        // Three into 9.exec_in, one into a different pin, one into another
        // node's identically-named pin.
        let edges = vec![
            to(1, 9, "exec_in"),
            to(2, 9, "exec_in"),
            to(3, 9, "other"),
            to(4, 9, "exec_in"),
            to(5, 8, "exec_in"),
        ];
        assert_eq!(converge_ranks(&edges), vec![0, 1, 0, 2, 0]);
        assert!(converge_ranks(&[]).is_empty());
    }

    #[test]
    fn lod_thresholds_match_the_spec_ladder() {
        assert_eq!(ZoomLod::from_zoom(2.20), ZoomLod::L0);
        assert_eq!(ZoomLod::from_zoom(0.90), ZoomLod::L0);
        assert_eq!(ZoomLod::from_zoom(0.83), ZoomLod::L0); // user-report zoom: editable
        assert_eq!(ZoomLod::from_zoom(0.599), ZoomLod::L1);
        assert_eq!(ZoomLod::from_zoom(0.60), ZoomLod::L0);
        assert_eq!(ZoomLod::from_zoom(0.45), ZoomLod::L1);
        assert_eq!(ZoomLod::from_zoom(0.449), ZoomLod::L2);
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
            doc: None,
            preview: None,
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
            value_w: 0.0,
            hit_w: None,
            untyped: false,
            hidden: false,
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


    /// Build the two pins of a reroute exactly as `build_geoms` does, so the
    /// hit-zone tests exercise the shipped layout rather than a copy of it.
    fn reroute_geom(m: &GraphMetrics, min: Pos2, untyped: bool) -> NodeGeom {
        let d = if untyped { m.reroute_untyped_d() } else { m.reroute_d() };
        let c = Pos2::new(min.x + d * 0.5, min.y + d * 0.5);
        let band = d * REROUTE_PIN_OFF;
        let pin = |slug: &str, output: bool| PinGeom {
            slug: slug.into(),
            label: String::new(),
            ty: PinType::Float,
            output,
            row: 0,
            wire_anchor: c,
            dot_center: Pos2::new(if output { c.x + band } else { c.x - band }, c.y),
            connected: false,
            inline: None,
            value_w: 0.0,
            ghost: false,
            hit_w: Some(d * REROUTE_PIN_HIT),
            untyped,
            hidden: false,
        };
        NodeGeom {
            id: 7,
            rect: Rect::from_min_size(min, Vec2::splat(d)),
            title: String::new(),
            tag: String::new(),
            category: None,
            tint: None,
            missing: false,
            errored: false,
            reroute: true,
            chip: None,
            anim: None,
            pinned_pos: false,
            breakpoint: None,
            preview: None,
            config: Vec::new(),
            add_field: None,
            pins: vec![pin(REROUTE_IN, false), pin(REROUTE_OUT, true)],
        }
    }

    #[test]
    fn an_untyped_reroute_is_smaller_than_a_typed_one() {
        let m = GraphMetrics::new(&Style::steel());
        assert!(
            m.reroute_untyped_d() < m.reroute_d(),
            "an unadopted type reads as provisional"
        );
        assert!(
            m.reroute_untyped_d() > m.pin_r * 2.0,
            "but still bigger than a plain pin dot, or it stops looking like a node"
        );
    }

    #[test]
    fn a_reroute_keeps_its_disc_hit_box_at_every_zoom() {
        let m = GraphMetrics::new(&Style::steel());
        let g = reroute_geom(&m, Pos2::ZERO, false);
        for lod in [ZoomLod::L0, ZoomLod::L2, ZoomLod::L3, ZoomLod::L4] {
            let r = g.body_rect(lod, &m);
            assert_eq!(r, g.rect, "a reroute has no header to collapse to ({lod:?})");
        }
    }

    /// The reported bug: an empty reroute could not be moved. Its two pins sat
    /// on the same centre with the global 18-unit hit target, which blanketed
    /// the whole disc — every press was a pin press, so `pin_claimed` was set
    /// and the node-drag arm (gated on `!pin_claimed`) never fired.
    #[test]
    fn a_reroutes_centre_is_body_so_it_can_be_grabbed_and_dragged() {
        let m = GraphMetrics::new(&Style::steel());
        let g = reroute_geom(&m, Pos2::ZERO, true);
        let geoms = [g];
        let centre = geoms[0].rect.center();
        let global_hit = m.pin_hit_w(1.0);

        assert!(
            global_hit > geoms[0].rect.width(),
            "the global pin target really is wider than the disc — that was the bug"
        );
        assert!(
            pin_under(&geoms, centre, global_hit).is_none(),
            "the middle of the disc must not be claimed by a pin"
        );
        assert_eq!(
            node_under(&geoms, centre, ZoomLod::L0, &m),
            Some(7),
            "so the body takes the press and the reroute drags like any node"
        );
    }

    /// Bug: the Print node's `Text` field painted past the node's right edge.
    /// The sizer budgeted a flat `value_w` for every inline cell (and forgot
    /// the trailing padding), so a String constant that needed more simply
    /// overflowed. Sizer and painter now share `inline_row_w`/`inline_cell_rect`
    /// — this pins them together for cells from "fits easily" to "over the cap".
    #[test]
    fn a_long_string_constant_keeps_its_widget_inside_the_node() {
        let m = GraphMetrics::new(&Style::steel());
        let label_w = 26.0; // "Text"
        let row_center = 40.0;
        for text_w in [10.0, 56.0, 120.0, 400.0, 4000.0] {
            let value_w = m.text_value_w(text_w);
            let label_row = m.label_inset() + label_w;
            let node_w = inline_row_w(label_row, value_w, &m).clamp(m.min_w, m.max_w);
            for zoom in [0.9, 1.0, 2.2] {
                let node = Rect::from_min_size(
                    Pos2::new(100.0, 20.0),
                    Vec2::new(node_w * zoom, 80.0 * zoom),
                );
                let cell = inline_cell_rect(
                    node,
                    node.min.x + label_row * zoom,
                    value_w,
                    row_center,
                    &m,
                    zoom,
                );
                assert!(
                    cell.max.x <= node.max.x - m.pad_x * zoom + 0.01,
                    "text_w {text_w} @ {zoom}: cell {:?} ran past node {:?}",
                    cell.max.x,
                    node.max.x
                );
                assert!(cell.min.x >= node.min.x, "cell must start inside the node");
                assert!(cell.width() > 0.0);
            }
        }
    }

    /// The cap is what stops a paragraph-length constant from stretching a node
    /// across the canvas; the floor is what keeps a numeric cell the size the
    /// design draws it.
    #[test]
    fn an_inline_cell_grows_for_text_but_only_up_to_the_cap() {
        let m = GraphMetrics::new(&Style::steel());
        assert_eq!(m.text_value_w(0.0), m.value_w, "short text keeps the reserved width");
        assert!(m.text_value_w(90.0) > m.value_w, "a wider string widens the cell");
        assert_eq!(
            m.text_value_w(10_000.0),
            BASE_VALUE_W_MAX * m.scale,
            "past the cap the text elides instead of growing the node"
        );
    }

    /// Bug: an inline `DragValue`/`Checkbox` could be hovered but never edited.
    /// The canvas asks `interact` about the node body *after* the widgets have
    /// run, and asking claims the press — so the widget lost it on the very
    /// frame it was taken. Every canvas gesture now yields where a widget rect
    /// covers the pointer; this is that precedence, stated once.
    #[test]
    fn a_press_inside_an_inline_widget_belongs_to_the_widget() {
        let cell = Rect::from_min_size(Pos2::new(50.0, 20.0), Vec2::new(56.0, 14.0));
        let rects = [cell];
        assert!(
            widget_owns(cell.center(), &rects),
            "the field's own box is the field's press — the node body under it may not ask"
        );
        assert!(
            widget_owns(Pos2::new(50.5, 20.5), &rects),
            "including its edges, where a slow click lands"
        );
        assert!(
            !widget_owns(Pos2::new(49.0, 27.0), &rects),
            "a pixel outside is the node's again, or box-select would never start"
        );
        assert!(!widget_owns(Pos2::new(80.0, 60.0), &[]), "no widgets, no claim");
    }

    #[test]
    fn a_reroutes_two_pins_have_distinct_hit_zones() {
        let m = GraphMetrics::new(&Style::steel());
        let g = reroute_geom(&m, Pos2::ZERO, true);
        let d = g.rect.width();
        let geoms = [g];
        let hit = m.pin_hit_w(1.0);
        let y = d * 0.5;

        let left = pin_under(&geoms, Pos2::new(d * 0.1, y), hit).expect("left band is a pin");
        assert_eq!(left.1, REROUTE_IN);
        assert!(!left.3, "the left one is the input");

        let right = pin_under(&geoms, Pos2::new(d * 0.9, y), hit).expect("right band is a pin");
        assert_eq!(
            right.1, REROUTE_OUT,
            "sharing one centre made `out` unreachable — pin_under matched `in` first, always"
        );
        assert!(right.3, "the right one is the output");
    }

    #[test]
    fn an_untyped_reroute_accepts_any_type_from_either_direction() {
        let st = crate::engine::editor::graph_editor::tests_support::empty_state();
        // Dragging a Float output onto the empty reroute's input...
        assert!(
            validate_connection(
                &st, &NodeRegistry::new(), 1, "o", true, &PinType::Float, false,
                7, REROUTE_IN, false, &PinType::Domain(String::new()), true,
            )
            .is_some(),
            "an untyped reroute is an absence of a type, not a mismatch"
        );
        // ...and dragging out of the empty reroute onto a Float input.
        assert!(
            validate_connection(
                &st, &NodeRegistry::new(), 7, REROUTE_OUT, true, &PinType::Domain(String::new()), true,
                1, "i", false, &PinType::Float, false,
            )
            .is_some(),
            "and it works in the other direction too"
        );
        // Strictness is untouched where both sides really are typed.
        assert!(
            validate_connection(
                &st, &NodeRegistry::new(), 1, "o", true, &PinType::Float, false,
                2, "i", false, &PinType::Exec, false,
            )
            .is_none(),
            "no implicit conversions; typing stays strict"
        );
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
            chip: None,
            anim: None,
            pinned_pos: false,
            breakpoint: None,
            preview: None,
            config: Vec::new(),
            add_field: None,
            pins: vec![],
        };
        assert_eq!(g.body_rect(ZoomLod::L2, &m).height(), 120.0);
        assert_eq!(g.body_rect(ZoomLod::L0, &m).height(), 120.0);
        assert!((g.body_rect(ZoomLod::L3, &m).height() - m.header_h).abs() < 1e-6);
        assert!((g.body_rect(ZoomLod::L4, &m).height() - m.header_h).abs() < 1e-6);
    }

    /// Task 41 rework: the derived-position pass moves a chip's whole
    /// geometry — rect, pins, config cells — as one piece, and marks it so a
    /// grab cannot start a move to nowhere.
    #[test]
    fn translate_moves_every_piece_of_a_geom() {
        let m = GraphMetrics::new(&Style::steel());
        let mut g = band_geom(&m, 1, false);
        let pin = PinGeom {
            slug: "from".into(),
            label: String::new(),
            ty: PinType::Float,
            output: false,
            row: 0,
            wire_anchor: Pos2::new(0.0, 52.0),
            dot_center: Pos2::new(4.0, 52.0),
            connected: false,
            inline: None,
            value_w: 0.0,
            ghost: false,
            hit_w: None,
            untyped: false,
            hidden: false,
        };
        g.pins.push(pin);
        let (r0, p0, c0) = (g.rect, g.pins[0].wire_anchor, g.config[0].cell);
        g.translate(Vec2::new(30.0, -12.0));
        assert_eq!(g.rect.min, r0.min + Vec2::new(30.0, -12.0));
        assert_eq!(g.rect.size(), r0.size());
        assert_eq!(g.pins[0].wire_anchor, p0 + Vec2::new(30.0, -12.0));
        assert_eq!(g.config[0].cell.min, c0.min + Vec2::new(30.0, -12.0));
        assert!(g.pinned_pos, "a translated geom knows its position is derived");
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
            chip: None,
            anim: None,
            pinned_pos: false,
            breakpoint: None,
            preview: None,
            config: Vec::new(),
            add_field: None,
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
    fn title_bars_square_off_into_the_body_but_a_collapsed_card_stays_round() {
        let round = Rounding::same(6.0);
        // Bar over an open card: top corners follow the outline, bottom squares.
        let open = bar_rounding(round, 20.0, 140.0);
        assert_eq!((open.nw, open.ne, open.sw, open.se), (6.0, 6.0, 0.0, 0.0));
        // Collapsed: the bar *is* the card, so it keeps all four.
        assert_eq!(bar_rounding(round, 20.0, 20.0), round);
    }

    #[test]
    fn the_category_edge_reveal_rounds_with_the_node() {
        // A 2px strip cannot round itself — the tessellator clamps the radius to
        // half the height. The underlay band must clear 2·radius so its corners
        // match the node's, and the inset fill's radius must shrink by the edge.
        let radius = BASE_RADIUS;
        let band_h = radius * 2.0;
        assert!(band_h * 0.5 >= radius, "band must not clamp its own rounding");
        let edge_h = crate::engine::editor::theme::tokens::Metrics::default().edge_accent;
        assert!(edge_h < band_h);
        assert_eq!((radius - edge_h).max(0.0), 4.0);
        // The reveal is a *top* edge: 2px at the crown, tapering to nothing where
        // the outline turns vertical. Outer corner center (6,6) r6 vs inner (4,6)
        // r4 — the gap is edge-wide on the axis and closes by the side.
        let gap_at = |y: f32| {
            let outer = radius - (radius * radius - (radius - y).powi(2)).max(0.0).sqrt();
            let ir = radius - edge_h;
            let inner = if y < edge_h {
                f32::INFINITY
            } else {
                ir - (ir * ir - (radius - y).powi(2)).max(0.0).sqrt()
            };
            inner - outer
        };
        assert!(gap_at(0.0).is_infinite(), "full width across the crown");
        assert!(gap_at(3.0) > 0.0 && gap_at(3.0) < edge_h, "tapering round the curve");
        assert!(gap_at(radius).abs() < 1e-5, "closed where the side goes vertical");
    }

    #[test]
    fn fade_blends_toward_the_canvas_and_is_a_no_op_undimmed() {
        let bg = Color::rgb(0.0, 0.0, 0.0);
        let col = Color::rgb(1.0, 0.5, 0.25);
        assert_eq!(fade(col, 1.0, bg), col);
        let half = fade(col, 0.5, bg);
        assert!((half.r - 0.5).abs() < 1e-6 && (half.g - 0.25).abs() < 1e-6);
        // Opacity is preserved: the layers must stay opaque or the underlay bleeds.
        assert_eq!(half.a, 1.0);
    }

    #[test]
    fn inline_kind_covers_every_prop_value() {
        let none: &[String] = &[];
        assert!(matches!(InlineKind::of(&PropValue::Float(1.0), none), InlineKind::Float(_)));
        assert!(matches!(InlineKind::of(&PropValue::Bool(true), none), InlineKind::Bool(_)));
        assert!(matches!(
            InlineKind::of(&PropValue::Color([0.0; 4]), none),
            InlineKind::Color(_)
        ));
        assert!(matches!(
            InlineKind::of(&PropValue::Raw("(x:1)".into()), none),
            InlineKind::Raw(_)
        ));
        // Int and String became editable in 45-A P6.
        assert!(matches!(InlineKind::of(&PropValue::Int(-7), none), InlineKind::Int(-7)));
        assert!(matches!(
            InlineKind::of(&PropValue::Str("hi".into()), none),
            InlineKind::Str(_)
        ));

        // Everything else still falls back to a read-only chip. **Arrays stay
        // read-only on purpose**: array literal editing is a stated non-goal
        // (D9), and arrays reach a graph through wires and variables.
        for v in [
            PropValue::Vec2([1.0, 2.0]),
            PropValue::Vec3([1.0, 2.0, 3.0]),
            PropValue::Vec4([1.0; 4]),
            PropValue::Enum("Variant".into()),
            PropValue::Asset("textures/a.png".into()),
            PropValue::Array(vec![PropValue::Int(1), PropValue::Int(2)]),
        ] {
            assert!(matches!(InlineKind::of(&v, none), InlineKind::Chip(_)), "{v:?}");
        }
        // …and the chips say something useful rather than `Debug` spew.
        assert_eq!(
            InlineKind::of(
                &PropValue::Array(vec![PropValue::Int(1), PropValue::Str("a".into())]),
                none
            ),
            InlineKind::Chip("[1, a]".to_string())
        );
    }

    /// The 45-A P6 field widgets: which `PropValue`s are *interactive* at L0,
    /// and which stay painted. The list is asserted rather than described
    /// because "editable" is the whole feature.
    #[test]
    fn int_and_string_are_editable_arrays_are_not() {
        let none: &[String] = &[];
        let editable = |v: PropValue| {
            matches!(
                InlineKind::of(&v, none),
                InlineKind::Float(_)
                    | InlineKind::Int(_)
                    | InlineKind::Bool(_)
                    | InlineKind::Str(_)
                    | InlineKind::Enum { .. }
            )
        };
        assert!(editable(PropValue::Float(1.0)));
        assert!(editable(PropValue::Int(1)));
        assert!(editable(PropValue::Bool(true)));
        assert!(editable(PropValue::Str("x".into())));
        assert!(!editable(PropValue::Array(vec![])), "array literals are a non-goal");
        assert!(!editable(PropValue::Vec3([0.0; 3])), "vectors are P7+ work");
        assert!(!editable(PropValue::Asset("a/b.png".into())), "asset picker is not here yet");
        assert!(!editable(PropValue::Raw("(x:1)".into())), "forward-compat data is never edited");
    }

    /// The standard library's comparison nodes declare their operator
    /// variants, so the inline editor gives them a real dropdown rather than a
    /// free-text chip — the reason 45-A P3 collapsed twelve comparison nodes
    /// into two.
    #[test]
    fn compare_nodes_get_a_real_operator_dropdown() {
        let mut reg = NodeRegistry::new();
        node_graph_types::register_std_nodes(&mut reg).unwrap();

        for id in [
            node_graph_types::std_nodes::COMPARE_INT,
            node_graph_types::std_nodes::COMPARE_FLOAT,
        ] {
            let desc = reg.get(id).expect("registered");
            let variants = &desc.input("op").expect("an operator pin").variants;
            match InlineKind::of(&PropValue::Enum("less".into()), variants) {
                InlineKind::Enum { value, variants: v, ok } => {
                    assert_eq!(value, "less");
                    assert!(ok);
                    assert_eq!(v.len(), 6, "all six operators are offered on {id}");
                }
                other => panic!("{id}: expected a dropdown, got {other:?}"),
            }
            // A stale operator still edits, flagged rather than rejected.
            assert!(matches!(
                InlineKind::of(&PropValue::Enum("approximately".into()), variants),
                InlineKind::Enum { ok: false, .. }
            ));
        }
    }

    /// **The gap P6a pinned, now closed.** Reserved config properties
    /// (`var`, `event_name`, `action`, `payload.*`) are still deliberately not
    /// pins — they decide what the pins *are* — so P6c gave them their own
    /// anatomy: pin-less rows in a band above the pin rows. This asserts the
    /// rows exist, carry the right widget, and (for `var`) write back the
    /// storage shape `DocDescriptors` reads.
    #[test]
    fn reserved_config_properties_have_rows() {
        use crate::engine::node_graph::{
            std_events::{EVENT_CUSTOM_TYPE_ID, EVENT_PAYLOAD_PREFIX},
            GraphDoc, VarDecl, VAR_GET_TYPE_ID,
        };
        let mut reg = NodeRegistry::new();
        node_graph_types::register_std_events(&mut reg).unwrap();

        let mut doc = GraphDoc {
            variables: vec![VarDecl {
                slug: "score".into(),
                label: "Score".into(),
                ty: PinType::Int,
                default: Some(PropValue::Int(0)),
                group: None,
            }],
            ..GraphDoc::default()
        };
        let mut get = test_node(0, VAR_GET_TYPE_ID);
        get.properties
            .insert(VAR_PROP.into(), PropValue::Str("score".into()));
        let mut dangling = test_node(1, VAR_GET_TYPE_ID);
        dangling
            .properties
            .insert(VAR_PROP.into(), PropValue::Str("deleted".into()));
        let mut custom = test_node(2, EVENT_CUSTOM_TYPE_ID);
        custom
            .properties
            .insert(EVENT_NAME_PROP.into(), PropValue::Str("Hit".into()));
        custom.properties.insert(
            format!("{EVENT_PAYLOAD_PREFIX}amount"),
            PropValue::Enum("int".into()),
        );
        let action = test_node(3, EVENT_INPUT_ACTION_TYPE_ID);
        doc.nodes = vec![get, dangling, custom, action];
        let docd = DocDescriptors::new(&doc, &reg);

        // A variable node: one dropdown over the document's declarations,
        // stored as `Str` — an `Enum` here would stop `variable_of` resolving.
        let rows = config_rows(&doc.nodes[0], &docd);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0, VAR_PROP);
        match &rows[0].2 {
            InlineKind::Choice { value, variants, ok } => {
                assert_eq!(value, "score");
                assert_eq!(variants, &vec!["score".to_string()]);
                assert!(ok);
            }
            other => panic!("expected a Choice, got {other:?}"),
        }
        // A dangling reference still shows the name that broke, flagged.
        match &config_rows(&doc.nodes[1], &docd)[0].2 {
            InlineKind::Choice { value, ok, .. } => {
                assert_eq!(value, "deleted");
                assert!(!ok, "a dangling slug is flagged, not hidden");
            }
            other => panic!("expected a Choice, got {other:?}"),
        }
        // A custom event: the name, then one row per declared payload pin.
        let rows = config_rows(&doc.nodes[2], &docd);
        assert_eq!(
            rows.iter().map(|(k, _, _)| k.as_str()).collect::<Vec<_>>(),
            vec![EVENT_NAME_PROP, "payload.amount"]
        );
        assert!(matches!(rows[0].2, InlineKind::Str(_)));
        match &rows[1].2 {
            InlineKind::Enum { value, variants, ok } => {
                assert_eq!(value, "int");
                assert_eq!(variants.len(), PAYLOAD_PIN_TYPES.len());
                assert!(ok);
            }
            other => panic!("expected a payload-type dropdown, got {other:?}"),
        }
        // An input-action event gets its row even with nothing stored yet —
        // the row is how the property comes to exist.
        let rows = config_rows(&doc.nodes[3], &docd);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0, EVENT_ACTION_PROP);
        assert!(matches!(&rows[0].2, InlineKind::Str(s) if s.is_empty()));

        // A plain registered node has no config band at all.
        assert!(config_rows(&test_node(9, "event_tick"), &docd).is_empty());
    }

    /// Ticket 09: a state's config band routes between its three sources.
    /// A leaf shows Clip and an (empty) Graph row — the nesting feature's
    /// front door; a set Graph row takes over and the clip rows yield; a
    /// blend-tree region beats both.
    #[test]
    fn a_states_config_band_routes_clip_graph_and_tree() {
        use crate::engine::animation::graph::plan::{
            ANIM_STATE_TYPE_ID, CLIP_PROP, GRAPH_PROP, SPEED_PROP,
        };
        use crate::engine::node_graph::{GraphDoc, GraphRegion};
        let reg = NodeRegistry::new();
        let mut doc = GraphDoc::default();
        let mut leaf = test_node(0, ANIM_STATE_TYPE_ID);
        leaf.properties
            .insert(CLIP_PROP.into(), PropValue::Asset("anims/idle.anim".into()));
        let mut nested = test_node(1, ANIM_STATE_TYPE_ID);
        nested
            .properties
            .insert(CLIP_PROP.into(), PropValue::Asset("anims/idle.anim".into()));
        nested
            .properties
            .insert(GRAPH_PROP.into(), PropValue::Asset("graphs/loco.animgraph".into()));
        let mut tree = test_node(2, ANIM_STATE_TYPE_ID);
        tree.properties
            .insert(GRAPH_PROP.into(), PropValue::Asset("graphs/loco.animgraph".into()));
        doc.nodes = vec![leaf, nested, tree];
        doc.regions.insert(
            2,
            GraphRegion {
                nodes: vec![test_node(0, "anim_pose_result")],
                edges: vec![],
            },
        );
        let docd = DocDescriptors::new(&doc, &reg);

        let keys = |i: usize| -> Vec<String> {
            config_rows(&doc.nodes[i], &docd).iter().map(|(k, _, _)| k.clone()).collect()
        };
        assert_eq!(keys(0), vec![CLIP_PROP, GRAPH_PROP, SPEED_PROP]);
        let rows = config_rows(&doc.nodes[0], &docd);
        assert!(
            matches!(&rows[1].2, InlineKind::Str(s) if s.is_empty()),
            "the Graph row exists empty — it is how the property comes to be"
        );

        assert_eq!(keys(1), vec![GRAPH_PROP, SPEED_PROP], "clip rows yield to the graph");
        let rows = config_rows(&doc.nodes[1], &docd);
        assert!(matches!(&rows[0].2, InlineKind::Str(s) if s == "graphs/loco.animgraph"));

        assert_eq!(
            keys(2),
            vec![CLIP_PROP, SPEED_PROP],
            "a tree region beats the graph reference — the chip row"
        );
        assert!(matches!(&config_rows(&doc.nodes[2], &docd)[0].2, InlineKind::Chip(_)));
    }

    /// **The five-site invariant.** Config rows shift the whole pin band down,
    /// and every site that turns a row index into geometry has to agree or
    /// wires land beside their pins. All of them go through `band_y` /
    /// `node_h`, so the agreement is checkable without a canvas: pin row `i`
    /// sits exactly `config_n` rows below where it would with no band, the
    /// band's closing rule is the top of the first pin row, and the node grew
    /// by exactly the band.
    #[test]
    fn a_config_band_shifts_the_pin_band_by_whole_rows() {
        let m = GraphMetrics::new(&Style::steel());
        let y0 = 40.0;
        for config_n in 0..4usize {
            for i in 0..3usize {
                assert!(
                    (m.band_y(y0, config_n + i) - (m.band_y(y0, i) + config_n as f32 * m.row_h))
                        .abs()
                        < 1e-4,
                    "pin row {i} with {config_n} config rows"
                );
            }
            // The rule under the last config row is the top of the first pin
            // row — one shared boundary, not two that drift.
            if config_n > 0 {
                let rule = m.band_y(y0, config_n - 1) + m.row_h * 0.5;
                let first_pin_top = m.band_y(y0, config_n) - m.row_h * 0.5;
                assert!((rule - first_pin_top).abs() < 1e-4);
            }
            // Height grows by exactly the band.
            assert!(
                (m.node_h(config_n, 2, 0.0) - (m.node_h(0, 2, 0.0) + config_n as f32 * m.row_h))
                    .abs()
                    < 1e-4
            );
            // …and the last pin row still fits inside the body.
            let last = m.band_y(y0, config_n + 1) + m.row_h * 0.5;
            assert!(last <= y0 + m.node_h(config_n, 2, 0.0));
        }
    }

    /// A `NodeGeom` with `fields` payload rows and, optionally, the "+ field"
    /// ghost row — enough to answer the band's geometry questions without a
    /// canvas.
    fn band_geom(m: &GraphMetrics, fields: usize, add_field: bool) -> NodeGeom {
        let width = 200.0;
        let rows = fields + usize::from(add_field);
        let rect = Rect::from_min_size(
            Pos2::new(0.0, 40.0),
            Vec2::new(width, m.node_h(rows, 2, 0.0)),
        );
        let config = (0..fields)
            .map(|i| {
                let y = m.band_y(rect.min.y, i);
                let right = rect.max.x - m.pad_x;
                let remove = m.config_box(right - m.config_remove_w(), right, y);
                let cell = m.config_box(
                    remove.min.x - m.label_gap - m.config_value_w(),
                    remove.min.x - m.label_gap,
                    y,
                );
                ConfigGeom {
                    key: format!("{EVENT_PAYLOAD_PREFIX}f{i}"),
                    label: format!("f{i}"),
                    kind: InlineKind::Enum {
                        value: "float".into(),
                        variants: vec!["float".into()],
                        ok: true,
                    },
                    y,
                    cell,
                    label_box: m.config_box(rect.min.x + m.pad_x, cell.min.x - m.label_gap, y),
                    remove: Some(remove),
                }
            })
            .collect::<Vec<_>>();
        let add = add_field.then(|| {
            m.config_box(
                rect.min.x + m.pad_x,
                rect.max.x - m.pad_x,
                m.band_y(rect.min.y, fields),
            )
        });
        NodeGeom {
            id: 0,
            rect,
            title: "Event: Hit".into(),
            tag: "EVENT".into(),
            category: Some("Event".into()),
            tint: None,
            missing: false,
            errored: false,
            reroute: false,
            chip: None,
            anim: None,
            pinned_pos: false,
            breakpoint: None,
            preview: None,
            config,
            add_field: add,
            pins: vec![],
        }
    }

    /// **GS-1: the ghost row is band geometry, not decoration.** "+ field"
    /// occupies a row, so it shifts the pin band and grows the node exactly
    /// like a declared field does — otherwise it would draw over the first pin.
    /// The band's fill and its closing rule share one boundary with the top of
    /// that pin row, which is what keeps the surface from covering a wire
    /// anchor.
    #[test]
    fn the_add_field_ghost_row_takes_a_band_row() {
        let m = GraphMetrics::new(&Style::steel());
        let plain = band_geom(&m, 2, false);
        let with_add = band_geom(&m, 2, true);
        assert_eq!(plain.band_rows(), 2);
        assert_eq!(with_add.band_rows(), 3, "the ghost row counts");
        assert!(
            (with_add.rect.height() - (plain.rect.height() + m.row_h)).abs() < 1e-4,
            "the node grows by exactly one row"
        );
        // The band closes one row lower, and that boundary is the top of the
        // first pin row — the five-site invariant, with the ghost row in it.
        let bottom = with_add.band_bottom(&m).unwrap();
        assert!((bottom - (plain.band_bottom(&m).unwrap() + m.row_h)).abs() < 1e-4);
        assert!(
            (bottom - (m.band_y(with_add.rect.min.y, 3) - m.row_h * 0.5)).abs() < 1e-4,
            "the rule is the top of the first pin row"
        );
        // Every band box lives strictly inside the band, so nothing the author
        // can click sits at wire height.
        for r in with_add
            .config
            .iter()
            .flat_map(|c| [c.cell, c.label_box, c.remove.unwrap()])
            .chain(with_add.add_field)
        {
            assert!(r.min.y >= with_add.rect.min.y + m.header_h - 1e-4);
            assert!(r.max.y <= bottom + 1e-4, "{r:?} escapes the band");
            assert!(r.min.x >= with_add.rect.min.x && r.max.x <= with_add.rect.max.x);
        }
        // The ✕ column never overlaps the value cell it sits beside.
        let c = &with_add.config[0];
        assert!(c.remove.unwrap().min.x >= c.cell.max.x);
        assert!(c.cell.min.x >= c.label_box.max.x);
        // A node with no band has no rule to draw.
        assert_eq!(band_geom(&m, 0, false).band_bottom(&m), None);
    }

    /// Payload slugs are identifiers (mono); the fixed rows name a setting and
    /// stay sentence-case sans. The ✕ belongs to the payload rows alone — the
    /// `Variable` / `Name` / `Action` rows are the node's shape, not a list.
    #[test]
    fn payload_rows_render_mono_and_own_the_remove_affordance() {
        let m = GraphMetrics::new(&Style::steel());
        let g = band_geom(&m, 1, true);
        assert_eq!(g.config[0].payload_slug(), Some("f0"));
        assert!(g.config[0].mono_label());
        let fixed = ConfigGeom {
            key: VAR_PROP.into(),
            label: "Variable".into(),
            kind: InlineKind::Str(String::new()),
            y: 0.0,
            cell: Rect::from_min_size(Pos2::ZERO, Vec2::ZERO),
            label_box: Rect::from_min_size(Pos2::ZERO, Vec2::ZERO),
            remove: None,
        };
        assert_eq!(fixed.payload_slug(), None);
        assert!(!fixed.mono_label(), "a fixed label is sentence-case sans");
        assert!(fixed.remove.is_none());
    }

    /// **GS-3: the chip's state machine.** Bound-and-alive, bound-and-dead,
    /// running-but-unbound, and nothing at all — the fourth is what keeps edit
    /// mode free of debug residue.
    #[test]
    fn the_live_chip_names_all_four_states() {
        assert_eq!(chip_state(true, false, 3), ChipState::Live);
        assert_eq!(chip_state(true, true, 3), ChipState::Killed);
        assert_eq!(chip_state(false, false, 3), ChipState::Unbound(3));
        assert_eq!(chip_state(false, false, 0), ChipState::Absent);
        // A dead instance is still bound: the canvas holds its last trace.
        assert_eq!(chip_state(true, true, 1), ChipState::Killed);
        // Killed-but-unbound is not a state the chip can be in; the instances
        // exist, so it says how many and waits to be pointed at one.
        assert_eq!(chip_state(false, true, 2), ChipState::Unbound(2));
    }

    /// **GS-4: the badge gutter's precedence ladder.**
    /// hit > error > invalid > armed > disabled, one slot, never stacked.
    #[test]
    fn the_breakpoint_badge_ladder_never_stacks() {
        use BreakBadge::*;
        // A hit outranks everything, including an error on the same node: the
        // stop is happening now.
        assert_eq!(break_badge(Some(true), true, true, false), Some(Hit));
        assert_eq!(break_badge(Some(false), false, true, false), Some(Hit));
        // …but a hit with no mark draws no octagon — Step parks wherever it
        // lands, and a breakpoint glyph there would claim a mark that is not
        // set. The border and the PAUSED chip still say where execution is.
        assert_eq!(break_badge(None, false, true, false), None);
        // Error outranks every resting mark.
        assert_eq!(break_badge(Some(true), true, false, false), None);
        assert_eq!(break_badge(Some(false), true, false, false), None);
        // Then the marks themselves.
        assert_eq!(break_badge(Some(true), false, false, true), Some(Invalid));
        assert_eq!(break_badge(Some(true), false, false, false), Some(Armed));
        assert_eq!(break_badge(Some(false), false, false, false), Some(Disabled));
        // A *disabled* mark is never invalid: it arms nothing, so there is
        // nothing to fail to resolve.
        assert_eq!(break_badge(Some(false), false, false, true), Some(Disabled));
        assert_eq!(break_badge(None, false, false, false), None);
    }

    /// **GS-2: where a dragged row would land.** The insertion index follows
    /// the row midpoints, so the 2px line the drag shows and the reorder the
    /// drop performs are read off one function — a preview that disagrees with
    /// its commit is the worst kind of drag.
    #[test]
    fn a_row_drag_lands_where_the_insertion_line_says() {
        let rows: Vec<(usize, Rect)> = (0..3)
            .map(|i| {
                (
                    i,
                    Rect::from_min_size(
                        Pos2::new(0.0, 100.0 + i as f32 * 20.0),
                        Vec2::new(200.0, 20.0),
                    ),
                )
            })
            .collect();
        // Above a row's midpoint inserts before it, below inserts after.
        assert_eq!(insertion_target(&rows, Pos2::new(50.0, 104.0)), Some(0));
        assert_eq!(insertion_target(&rows, Pos2::new(50.0, 116.0)), Some(1));
        assert_eq!(insertion_target(&rows, Pos2::new(50.0, 124.0)), Some(1));
        assert_eq!(insertion_target(&rows, Pos2::new(50.0, 156.0)), Some(3), "past the end");
        // Outside the column entirely: not a reorder at all.
        assert_eq!(insertion_target(&rows, Pos2::new(400.0, 116.0)), None);
        assert_eq!(insertion_target(&[], Pos2::new(50.0, 116.0)), None);
    }

    /// A dangling reference keeps the name that broke and marks it in the
    /// *text*, so the flag survives greyscale, deuteranopia and a screenshot.
    #[test]
    fn a_dangling_reference_is_marked_in_the_value_not_only_in_the_tint() {
        assert_eq!(flagged_value("mana", false), "mana ?");
        assert_eq!(flagged_value("mana", true), "mana");
        assert_eq!(flagged_value("", false), "", "an empty cell is not \"?\"");
    }

    /// **The band's rung of the LOD ladder.** No new thresholds: config
    /// widgets flatten with the inline pin widgets, the row labels drop with
    /// the pin labels, and the band drops exactly where the node becomes
    /// title-only — the synthesized title carries the configuration down to
    /// the slab.
    #[test]
    fn the_config_band_follows_the_existing_lod_ladder() {
        use ZoomLod::*;
        for lod in [L4, L3, L2, L1, L0] {
            assert_eq!(lod.config_band(), lod.rows(), "{lod:?}");
        }
        assert!(L0.inline_widgets(), "L0 — widgets live");
        assert!(!L1.inline_widgets() && L1.values(), "L1 — plain mono values");
        assert!(L2.config_band() && !L2.pin_labels(), "L2 — band, no row labels");
        assert!(!L3.config_band(), "L3 — title only, the band drops");
        // The value outlives the row label: a band with nothing in it would be
        // worse than no band, so the flattened value is drawn wherever the
        // band is drawn and the live widget is not.
        for lod in [L2, L1] {
            assert!(lod.config_band() && !lod.inline_widgets());
        }
    }

    /// Removing a payload field leaves the wires that read it pointing at a
    /// pin the descriptor no longer declares — which is exactly the existing
    /// `UnknownPin` → ghost-row treatment, so the wire keeps a landing spot
    /// and the node says what broke. Nothing new was needed for that; this
    /// pins it, because the confirmation dialog promises it.
    #[test]
    fn a_removed_payload_field_degrades_to_the_ghost_row_treatment() {
        use crate::engine::node_graph::{validate_doc, GraphDoc};
        let mut reg = NodeRegistry::new();
        node_graph_types::register_std_events(&mut reg).unwrap();
        let mut state = crate::engine::editor::graph_editor::test_state("graphs/t.graph");
        let mut ev = test_node(0, EVENT_CUSTOM_TYPE_ID);
        ev.properties
            .insert(EVENT_NAME_PROP.into(), PropValue::Str("Hit".into()));
        ev.properties.insert(
            format!("{EVENT_PAYLOAD_PREFIX}damage"),
            PropValue::Enum("float".into()),
        );
        let mut sink = test_node(1, EVENT_CUSTOM_TYPE_ID);
        sink.properties
            .insert(EVENT_NAME_PROP.into(), PropValue::Str("Other".into()));
        state.doc = GraphDoc { nodes: vec![ev, sink], ..GraphDoc::default() };
        state.doc.edges = vec![Edge {
            from_node: 0,
            from_pin: "damage".into(),
            to_node: 1,
            to_pin: "gone".into(),
        }];

        // With the field declared, the pin exists and only the (already
        // unknown) target pin ghosts.
        let ix = ErrorIndex::build(&validate_doc(&state.doc, &reg), &[], &[]);
        assert!(!ix
            .ghosts_for(0)
            .iter()
            .any(|(s, o)| s == "damage" && *o));

        assert!(state.remove_payload_field(0, "damage", &reg));
        let ix = ErrorIndex::build(&validate_doc(&state.doc, &reg), &[], &[]);
        assert!(
            ix.ghosts_for(0).iter().any(|(s, o)| s == "damage" && *o),
            "the wire's source pin becomes a ghost row"
        );
        assert_eq!(state.doc.edges.len(), 1, "and the wire itself survives");
    }

    /// The strip's two type controls and the declared type are one round trip:
    /// what the picker shows is what the declaration says, and rebuilding from
    /// the controls gives the declaration back.
    #[test]
    fn the_type_picker_round_trips_a_declared_type() {
        for ty in [
            PinType::Float,
            PinType::Int,
            PinType::String,
            PinType::Bool,
            PinType::Vec3,
            PinType::Entity,
            PinType::Array(Box::new(PinType::Float)),
            PinType::Array(Box::new(PinType::Entity)),
        ] {
            let (i, array) = type_pick(GraphDomain::Script, &ty);
            assert_eq!(type_from_pick(GraphDomain::Script, i, array), ty, "{ty:?}");
        }
        assert_eq!(type_label(&PinType::Array(Box::new(PinType::Float))), "Float[]");
        // A type the picker cannot express still reads honestly in the row and
        // does not silently become Float on the way past.
        let domain = PinType::Domain("shader".into());
        assert_eq!(type_label(&domain), "domain:shader");
        assert_eq!(type_pick(GraphDomain::Script, &domain), (0, false));
    }

    /// An `Enum` pin becomes a dropdown only when its descriptor says what
    /// the legal values are; an out-of-list value is flagged, not rejected.
    #[test]
    fn enum_pins_become_dropdowns_only_with_declared_variants() {
        let variants = vec!["Idle".to_string(), "Run".to_string()];
        match InlineKind::of(&PropValue::Enum("Run".into()), &variants) {
            InlineKind::Enum { value, ok, variants: v } => {
                assert_eq!(value, "Run");
                assert!(ok);
                assert_eq!(v.len(), 2);
            }
            other => panic!("expected a dropdown, got {other:?}"),
        }
        // Stale data: still editable, shown as a warning.
        match InlineKind::of(&PropValue::Enum("Sprint".into()), &variants) {
            InlineKind::Enum { ok, .. } => assert!(!ok),
            other => panic!("expected a dropdown, got {other:?}"),
        }
        // No declared variants = a free string, so a plain chip.
        assert!(matches!(
            InlineKind::of(&PropValue::Enum("anything".into()), &[]),
            InlineKind::Chip(_)
        ));
    }

    /// The preview budget rotates: every slot gets refreshed within
    /// `ceil(count / budget)` frames and no frame pays for more than the cap.
    #[test]
    fn preview_budget_rotates_and_caps() {
        // Under budget: everything, every frame.
        assert_eq!(preview_slice(5, 0, 8), (0, 5));
        assert_eq!(preview_slice(5, 99, 8), (0, 5));

        // Over budget: a moving window that covers everything in 3 frames.
        let mut seen = vec![false; 20];
        for frame in 0..3u64 {
            let (start, take) = preview_slice(20, frame, 8);
            assert!(take <= 8, "the cap is never exceeded");
            for i in start..start + take {
                seen[i] = true;
            }
        }
        assert!(seen.iter().all(|s| *s), "every slot refreshed within 3 frames");
        // …and it keeps cycling.
        assert_eq!(preview_slice(20, 3, 8), preview_slice(20, 0, 8));

        // Degenerate inputs are not a panic.
        assert_eq!(preview_slice(0, 0, 8), (0, 0));
        assert_eq!(preview_slice(10, 0, 0), (0, 0));
    }
}
