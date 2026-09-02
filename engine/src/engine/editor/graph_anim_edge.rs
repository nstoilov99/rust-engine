//! Derived state-machine edge geometry (Task 41 canvas rework).
//!
//! The document stores machine topology as pins and edges — `state.out →
//! transition.from`, `transition.to → state.in` — but the canvas draws what
//! modern state machines draw (mockup 2b, the Unreal reading): ONE direct
//! edge from source border to target border with an arrowhead, the
//! transition's chip sitting at its midpoint. The transition node's stored
//! position stops mattering while both endpoints are wired; it is kept in the
//! document untouched (format stability) and comes back the moment an
//! endpoint goes missing.
//!
//! This module is the pure half: document + displayed rects in, straight
//! segments + derived chip positions out, keyed by **doc edge index** so the
//! wire pass can substitute geometry without inventing edges. Anything this
//! pass cannot honestly place — a reroute spliced into a flow wire, states
//! stacked on top of each other, a self-loop — is simply *absent* from the
//! result, and the caller falls back to the stored-position rendering.
//!
//! Line rules (the Unreal pairing, 2026-09-01 ruling): **one straight line
//! per (from, to) direction**. A→B and B→A part to the two sides of the
//! centerline, each to the **right of its travel direction**; every further
//! same-direction transition rides the *same* line. Each transition is a
//! circular arrow badge in a row beside its line — centred on the midpoint,
//! laid along the line in ascending node-id order, offset right-of-travel by
//! radius + gap so the row never sits on the stroke. Rule text is not on the
//! canvas at all (tooltip + unfolded card).

use std::collections::BTreeMap;

use crate::engine::animation::graph::plan::{
    ANIM_ENTRY_TYPE_ID, ANIM_STATE_ALIAS_TYPE_ID, ANIM_STATE_TYPE_ID, ANIM_TRANSITION_TYPE_ID,
    STATE_IN_PIN, STATE_OUT_PIN, TRANSITION_FROM_PIN, TRANSITION_TO_PIN,
};
use crate::engine::node_graph::GraphDoc;

/// Distance between the two opposite-direction lines of a state pair, world
/// units. Wide enough that both arrowheads and a badge row on each outer
/// side read as two separate arrows at a glance.
pub const LANE_GAP: f32 = 36.0;

/// Badge sizing the layout needs from the canvas, world units: the badge
/// diameter (the chip row height) and the gap between badges — also the
/// air between the row and its line.
#[derive(Debug, Clone, Copy)]
pub struct BadgeMetrics {
    pub d: f32,
    pub gap: f32,
}

/// One straight display segment for a doc edge.
#[derive(Debug, Clone, PartialEq)]
pub struct EdgeSeg {
    pub a: [f32; 2],
    pub b: [f32; 2],
    /// Draw an arrowhead at `b` (the end that lands on a state or a chip).
    pub arrow: bool,
}

/// The derived layout: badge positions and per-edge segments.
#[derive(Debug, Default)]
pub struct AnimFlowLayout {
    /// Transition node id → derived rect **min** (world). Only transitions
    /// with both endpoints resolved appear; everything else keeps its stored
    /// position. The rect is the transition's *displayed* rect (a badge, or
    /// the unfolded card when selected) centred on its badge slot.
    pub chip_min: BTreeMap<u64, [f32; 2]>,
    /// Transition node id → unit flow direction its badge arrow points
    /// along. Present for every transition that has at least one placed
    /// half, so a half-wired badge still points with its one wire.
    pub chip_dir: BTreeMap<u64, [f32; 2]>,
    /// Doc edge index → straight segment replacing the routed wire.
    pub segs: BTreeMap<usize, EdgeSeg>,
}

/// Half-extent of `r` along direction `d` (the rect's support radius): how
/// far a lane may offset before its line misses the rect entirely.
fn half_extent(r: [f32; 4], d: [f32; 2]) -> f32 {
    d[0].abs() * (r[2] - r[0]) * 0.5 + d[1].abs() * (r[3] - r[1]) * 0.5
}

/// A machine node that can source flow (has `STATE_OUT_PIN`).
fn is_flow_source(type_id: &str) -> bool {
    matches!(
        type_id,
        ANIM_STATE_TYPE_ID | ANIM_STATE_ALIAS_TYPE_ID | ANIM_ENTRY_TYPE_ID
    )
}

fn center(r: [f32; 4]) -> [f32; 2] {
    [(r[0] + r[2]) * 0.5, (r[1] + r[3]) * 0.5]
}

fn size(r: [f32; 4]) -> [f32; 2] {
    [r[2] - r[0], r[3] - r[1]]
}

fn lerp(a: [f32; 2], b: [f32; 2], t: f32) -> [f32; 2] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

/// Liang-Barsky: the parameter interval of segment `a→b` inside `r`, or
/// `None` when the segment misses the rect entirely.
fn seg_rect_interval(a: [f32; 2], b: [f32; 2], r: [f32; 4]) -> Option<(f32, f32)> {
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    let d = [b[0] - a[0], b[1] - a[1]];
    for axis in 0..2 {
        let (lo, hi) = (r[axis], r[axis + 2]);
        if d[axis].abs() < f32::EPSILON {
            if a[axis] < lo || a[axis] > hi {
                return None;
            }
            continue;
        }
        let (mut ta, mut tb) = ((lo - a[axis]) / d[axis], (hi - a[axis]) / d[axis]);
        if ta > tb {
            std::mem::swap(&mut ta, &mut tb);
        }
        t0 = t0.max(ta);
        t1 = t1.min(tb);
        if t0 > t1 {
            return None;
        }
    }
    Some((t0, t1))
}

/// Where the line `a→b` leaves `r` (which nominally contains `a`). When the
/// offset line misses the rect — a lane wider than a small node — the anchor
/// clamps onto the rect instead of detaching from it.
fn exit_anchor(a: [f32; 2], b: [f32; 2], r: [f32; 4]) -> ([f32; 2], f32) {
    match seg_rect_interval(a, b, r) {
        Some((_, t_out)) => (lerp(a, b, t_out), t_out),
        None => ([a[0].clamp(r[0], r[2]), a[1].clamp(r[1], r[3])], 0.0),
    }
}

/// Where the ray from `r`'s center toward `to` leaves `r` — the start of a
/// transition being dragged out of a state. `None` while `to` is still
/// inside the rect: there is no border-to-pointer line to draw yet.
pub fn border_exit(r: [f32; 4], to: [f32; 2]) -> Option<[f32; 2]> {
    let c = center(r);
    let (a, t_out) = exit_anchor(c, to, r);
    (t_out < 1.0).then_some(a)
}

/// Where the line `a→b` enters `r` (which nominally contains `b`).
fn entry_anchor(a: [f32; 2], b: [f32; 2], r: [f32; 4]) -> ([f32; 2], f32) {
    match seg_rect_interval(a, b, r) {
        Some((t_in, _)) => (lerp(a, b, t_in), t_in),
        None => ([b[0].clamp(r[0], r[2]), b[1].clamp(r[1], r[3])], 1.0),
    }
}

/// A straight border-to-border segment between two rects along the line of
/// centers. `None` when the rects overlap or coincide — there is no honest
/// line to draw then.
fn border_segment(ra: [f32; 4], rb: [f32; 4]) -> Option<([f32; 2], [f32; 2])> {
    let (ca, cb) = (center(ra), center(rb));
    let d = [cb[0] - ca[0], cb[1] - ca[1]];
    if (d[0] * d[0] + d[1] * d[1]).sqrt() < 1.0 {
        return None;
    }
    let (a, t_out) = exit_anchor(ca, cb, ra);
    let (b, t_in) = entry_anchor(ca, cb, rb);
    (t_in > t_out).then_some((a, b))
}

/// The sub-segments of `a→b` that lie *outside* `r` — at most two. Used by
/// the dim-on-select edge redraw: the lit edge repaints above the scrim, but
/// must stop at the border of the open (unfolded) card it belongs to instead
/// of crossing it. Pure.
pub fn seg_outside(a: [f32; 2], b: [f32; 2], r: [f32; 4]) -> Vec<([f32; 2], [f32; 2])> {
    match seg_rect_interval(a, b, r) {
        None => vec![(a, b)],
        Some((t0, t1)) => {
            let mut out = Vec::new();
            if t0 > 0.0 {
                out.push((a, lerp(a, b, t0)));
            }
            if t1 < 1.0 {
                out.push((lerp(a, b, t1), b));
            }
            out
        }
    }
}

/// One resolved machine connection: a fully wired transition, or a direct
/// flow edge (ENTRY → state; legacy state → state).
struct Link {
    from: u64,
    to: u64,
    transition: Option<u64>,
    e_from: usize,
    e_to: Option<usize>,
}

/// Unit direction of `a→b`, or `None` for a degenerate segment.
fn unit(a: [f32; 2], b: [f32; 2]) -> Option<[f32; 2]> {
    let d = [b[0] - a[0], b[1] - a[1]];
    let len = (d[0] * d[0] + d[1] * d[1]).sqrt();
    (len >= 1.0).then(|| [d[0] / len, d[1] / len])
}

/// Compute the derived machine layout. `rects` are the *displayed* node
/// rects of the current frame — transitions included, for their sizes.
pub fn anim_flow_layout(
    doc: &GraphDoc,
    rects: &BTreeMap<u64, [f32; 4]>,
    badge: BadgeMetrics,
) -> AnimFlowLayout {
    let mut out = AnimFlowLayout::default();
    let ty_of = |id: u64| doc.node(id).map(|n| n.type_id.as_str());

    // Per transition: the doc edge feeding `from` and the one leaving `to`,
    // each resolved to a machine node (or not).
    let mut links: Vec<Link> = Vec::new();
    // (transition, half) pairs that could not join a full link.
    let mut partial: Vec<(u64, Option<(usize, u64)>, Option<(usize, u64)>)> = Vec::new();

    for t in doc
        .nodes
        .iter()
        .filter(|n| n.type_id == ANIM_TRANSITION_TYPE_ID)
    {
        let e_from = doc.edges.iter().enumerate().find_map(|(i, e)| {
            (e.to_node == t.id
                && e.to_pin == TRANSITION_FROM_PIN
                && e.from_pin == STATE_OUT_PIN
                && ty_of(e.from_node).is_some_and(is_flow_source))
            .then_some((i, e.from_node))
        });
        let e_to = doc.edges.iter().enumerate().find_map(|(i, e)| {
            (e.from_node == t.id
                && e.from_pin == TRANSITION_TO_PIN
                && e.to_pin == STATE_IN_PIN
                && ty_of(e.to_node) == Some(ANIM_STATE_TYPE_ID))
            .then_some((i, e.to_node))
        });
        match (e_from, e_to) {
            (Some((ei, from)), Some((eo, to)))
                if from != to
                    && rects.contains_key(&from)
                    && rects.contains_key(&to)
                    && rects.contains_key(&t.id) =>
            {
                links.push(Link {
                    from,
                    to,
                    transition: Some(t.id),
                    e_from: ei,
                    e_to: Some(eo),
                });
            }
            _ => partial.push((t.id, e_from, e_to)),
        }
    }

    // Direct machine edges: flow wires that touch no transition on either
    // end (the ENTRY seed wire; hand-authored state→state edges).
    for (i, e) in doc.edges.iter().enumerate() {
        if e.from_pin == STATE_OUT_PIN
            && e.to_pin == STATE_IN_PIN
            && ty_of(e.from_node).is_some_and(is_flow_source)
            && ty_of(e.to_node) == Some(ANIM_STATE_TYPE_ID)
            && e.from_node != e.to_node
            && rects.contains_key(&e.from_node)
            && rects.contains_key(&e.to_node)
        {
            links.push(Link {
                from: e.from_node,
                to: e.to_node,
                transition: None,
                e_from: i,
                e_to: None,
            });
        }
    }

    // One line per (from, to) direction; the two directions of an unordered
    // pair part to opposite sides of the centerline.
    let mut lines: BTreeMap<(u64, u64), Vec<usize>> = BTreeMap::new();
    for (i, l) in links.iter().enumerate() {
        lines.entry((l.from, l.to)).or_default().push(i);
    }
    for (&(from, to), members) in &lines {
        let has_opp = lines.contains_key(&(to, from));
        let (ra, rb) = (rects[&from], rects[&to]);
        let (ca, cb) = (center(ra), center(rb));
        let Some(u) = unit(ca, cb) else { continue };
        // Right of travel, screen coords (y down): east-bound offsets
        // south, so A→B and B→A part to opposite sides of the line.
        let right = [-u[1], u[0]];
        // A line offset past a node's half-extent would detach from its
        // border; the split compresses to what the smaller endpoint can carry.
        let allowed = (half_extent(ra, right).min(half_extent(rb, right)) - 2.0).max(0.0);
        let perp_off = (if has_opp { LANE_GAP * 0.5 } else { 0.0 }).min(allowed);
        let oa = [ca[0] + right[0] * perp_off, ca[1] + right[1] * perp_off];
        let ob = [cb[0] + right[0] * perp_off, cb[1] + right[1] * perp_off];
        let (a, t_out) = exit_anchor(oa, ob, ra);
        let (b, t_in) = entry_anchor(oa, ob, rb);
        if t_in <= t_out {
            continue; // rects overlap: no honest line, fall back
        }
        let mid = lerp(a, b, 0.5);

        // The badge row: ascending node id (explicit — doc order is not a
        // contract), centred on the midpoint, pushed right of travel so it
        // clears the stroke. With an opposite line present that side is
        // already the one facing away from it.
        let mut badges: Vec<(u64, usize)> = members
            .iter()
            .filter_map(|&li| links[li].transition.map(|t| (t, li)))
            .collect();
        badges.sort_by_key(|&(t, _)| t);
        let pitch = badge.d + badge.gap;
        let start = -(badges.len() as f32 - 1.0) * 0.5 * pitch;
        let side = badge.d * 0.5 + badge.gap;
        for (i, &(t, li)) in badges.iter().enumerate() {
            let s = start + i as f32 * pitch;
            let c = [
                mid[0] + u[0] * s + right[0] * side,
                mid[1] + u[1] * s + right[1] * side,
            ];
            let sz = size(rects[&t]);
            out.chip_min
                .insert(t, [c[0] - sz[0] * 0.5, c[1] - sz[1] * 0.5]);
            out.chip_dir.insert(t, u);
            // Both halves stay collinear and meet at the midpoint, so every
            // consumer of the halves (hit test, cut, dim redraw) still sees
            // one straight line.
            let link = &links[li];
            out.segs
                .insert(link.e_from, EdgeSeg { a, b: mid, arrow: false });
            if let Some(eo) = link.e_to {
                out.segs.insert(eo, EdgeSeg { a: mid, b, arrow: true });
            }
        }
        for &li in members {
            if links[li].transition.is_none() {
                out.segs
                    .insert(links[li].e_from, EdgeSeg { a, b, arrow: true });
            }
        }
    }

    // Half-wired (or self-looping) transitions: the badge keeps its stored
    // rect and each existing half draws straight between state border and
    // badge border, arrow pointing with the flow; the badge points with the
    // wire it has (the incoming one when both exist).
    for (t, from_half, to_half) in partial {
        let Some(&rt) = rects.get(&t) else { continue };
        if let Some((eo, to)) = to_half {
            if let Some(&rs) = rects.get(&to) {
                if let Some((a, b)) = border_segment(rt, rs) {
                    out.segs.insert(eo, EdgeSeg { a, b, arrow: true });
                    if let Some(u) = unit(a, b) {
                        out.chip_dir.insert(t, u);
                    }
                }
            }
        }
        if let Some((ei, from)) = from_half {
            if let Some(&rs) = rects.get(&from) {
                if let Some((a, b)) = border_segment(rs, rt) {
                    out.segs.insert(ei, EdgeSeg { a, b, arrow: true });
                    if let Some(u) = unit(a, b) {
                        out.chip_dir.insert(t, u);
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::node_graph::{Edge, NodeInst};

    fn node(id: u64, type_id: &str) -> NodeInst {
        NodeInst {
            id,
            type_id: type_id.to_string(),
            type_version: 1,
            position: [0.0, 0.0],
            properties: Default::default(),
            subgraph: None,
            tint: None,
            title: None,
        }
    }

    fn edge(from: u64, from_pin: &str, to: u64, to_pin: &str) -> Edge {
        Edge {
            from_node: from,
            from_pin: from_pin.to_string(),
            to_node: to,
            to_pin: to_pin.to_string(),
        }
    }

    /// Entry(0) → Idle(1) direct; Idle(1) → Loco(2) via transition 10.
    fn machine() -> (GraphDoc, BTreeMap<u64, [f32; 4]>) {
        let mut doc = GraphDoc::default();
        doc.nodes = vec![
            node(0, ANIM_ENTRY_TYPE_ID),
            node(1, ANIM_STATE_TYPE_ID),
            node(2, ANIM_STATE_TYPE_ID),
            node(10, ANIM_TRANSITION_TYPE_ID),
        ];
        doc.edges = vec![
            edge(0, STATE_OUT_PIN, 1, STATE_IN_PIN),
            edge(1, STATE_OUT_PIN, 10, TRANSITION_FROM_PIN),
            edge(10, TRANSITION_TO_PIN, 2, STATE_IN_PIN),
        ];
        let rects: BTreeMap<u64, [f32; 4]> = [
            (0, [-200.0, 40.0, -140.0, 70.0]),
            (1, [0.0, 0.0, 120.0, 60.0]),
            (2, [400.0, 0.0, 520.0, 60.0]),
            (10, [0.0, 300.0, 100.0, 326.0]), // stored position, off the line
        ]
        .into();
        (doc, rects)
    }

    /// The badge metrics every test lays out with: D = 22, gap = 4, so a
    /// badge centre sits 15 off its line and consecutive badges 26 apart.
    const BADGE: BadgeMetrics = BadgeMetrics { d: 22.0, gap: 4.0 };

    fn layout(doc: &GraphDoc, rects: &BTreeMap<u64, [f32; 4]>) -> AnimFlowLayout {
        anim_flow_layout(doc, rects, BADGE)
    }

    /// Centre of transition `t`'s derived rect.
    fn chip_center(l: &AnimFlowLayout, rects: &BTreeMap<u64, [f32; 4]>, t: u64) -> [f32; 2] {
        let sz = size(rects[&t]);
        let mn = l.chip_min[&t];
        [mn[0] + sz[0] * 0.5, mn[1] + sz[1] * 0.5]
    }

    /// A fully wired transition renders as two collinear halves — border to
    /// midpoint, midpoint to border — with the arrow on the landing half,
    /// and its badge beside the midpoint, right of travel, pointing with
    /// the flow.
    #[test]
    fn a_wired_transition_becomes_one_direct_edge_with_a_midpoint_chip() {
        let (doc, rects) = machine();
        let l = layout(&doc, &rects);

        let h1 = &l.segs[&1];
        let h2 = &l.segs[&2];
        assert!(!h1.arrow, "the half landing on the midpoint has no arrowhead");
        assert!(h2.arrow, "the half landing on the state carries the arrow");
        assert_eq!(h1.b, h2.a, "halves meet at the midpoint");
        // Straight horizontal run at the centers' height, border to border.
        assert_eq!(h1.a, [120.0, 30.0]);
        assert_eq!(h2.b, [400.0, 30.0]);
        assert_eq!(h1.b, [260.0, 30.0], "a lone line rides the centerline");
        // Badge beside the midpoint: east-bound, so right of travel is +y,
        // by radius + gap; its own rect size centres it.
        assert_eq!(chip_center(&l, &rects, 10), [260.0, 45.0]);
        assert_eq!(l.chip_dir[&10], [1.0, 0.0]);
    }

    /// The entry wire is a plain direct arrow.
    #[test]
    fn the_entry_wire_is_a_direct_arrow() {
        let (doc, rects) = machine();
        let l = layout(&doc, &rects);
        let s = &l.segs[&0];
        assert!(s.arrow);
        // Lands on Idle's border, leaves Entry's border.
        assert_eq!(s.a[0], -140.0);
        assert_eq!(s.b[0], 0.0);
    }

    /// A → B and B → A part to opposite sides of the centerline and both
    /// halves of each stay parallel to it (the Unreal pairing).
    #[test]
    fn bidirectional_transitions_take_opposite_lanes() {
        let (mut doc, mut rects) = machine();
        doc.nodes.push(node(11, ANIM_TRANSITION_TYPE_ID));
        doc.edges.push(edge(2, STATE_OUT_PIN, 11, TRANSITION_FROM_PIN));
        doc.edges.push(edge(11, TRANSITION_TO_PIN, 1, STATE_IN_PIN));
        rects.insert(11, [0.0, 400.0, 100.0, 426.0]);
        let l = layout(&doc, &rects);

        // Forward transition (1→2, east-bound): right of travel is +y.
        let f = &l.segs[&1];
        assert_eq!(f.a[1], 30.0 + LANE_GAP * 0.5);
        // Reverse (2→1, west-bound): right of travel is -y.
        let r = &l.segs[&3];
        assert_eq!(r.a[1], 30.0 - LANE_GAP * 0.5);
        // Each line is level (parallel to the horizontal centerline).
        assert_eq!(f.a[1], l.segs[&2].b[1]);
        assert_eq!(r.a[1], l.segs[&4].b[1]);
        // Each badge sits on the outer side of its own line — away from the
        // other line — and points with its own flow.
        assert_eq!(chip_center(&l, &rects, 10), [260.0, 48.0 + 15.0]);
        assert_eq!(chip_center(&l, &rects, 11), [260.0, 12.0 - 15.0]);
        assert_eq!(l.chip_dir[&10], [1.0, 0.0]);
        assert_eq!(l.chip_dir[&11], [-1.0, 0.0]);
    }

    /// Same-direction transitions share one line and stack their badges
    /// side by side along it.
    #[test]
    fn same_direction_transitions_share_one_line_and_stack_badges() {
        let (mut doc, mut rects) = machine();
        doc.nodes.push(node(11, ANIM_TRANSITION_TYPE_ID));
        doc.edges.push(edge(1, STATE_OUT_PIN, 11, TRANSITION_FROM_PIN));
        doc.edges.push(edge(11, TRANSITION_TO_PIN, 2, STATE_IN_PIN));
        rects.insert(11, [0.0, 400.0, 100.0, 426.0]);
        let l = layout(&doc, &rects);
        // One line: both transitions' halves are the same two segments.
        assert_eq!(l.segs[&1], l.segs[&3]);
        assert_eq!(l.segs[&2], l.segs[&4]);
        assert_eq!(l.segs[&1].a[1], 30.0, "no opposite line: the centerline");
        // Two badges, one pitch (D + gap) apart, centred on the midpoint.
        let c10 = chip_center(&l, &rects, 10);
        let c11 = chip_center(&l, &rects, 11);
        assert_eq!(c10, [260.0 - 13.0, 45.0]);
        assert_eq!(c11, [260.0 + 13.0, 45.0]);
    }

    /// Badge order along the line is ascending node id whatever the
    /// document's node order, and the row sits right of travel — so on a
    /// west-bound line it is above the line and runs east-to-west by id.
    #[test]
    fn badges_order_by_node_id_on_the_right_of_travel() {
        let (mut doc, mut rects) = machine();
        // Reverse the existing transition (2 → 10 → 1) and add two more of
        // the same direction with ids pushed out of order.
        doc.edges[1] = edge(2, STATE_OUT_PIN, 10, TRANSITION_FROM_PIN);
        doc.edges[2] = edge(10, TRANSITION_TO_PIN, 1, STATE_IN_PIN);
        for id in [12u64, 11] {
            doc.nodes.push(node(id, ANIM_TRANSITION_TYPE_ID));
            doc.edges.push(edge(2, STATE_OUT_PIN, id, TRANSITION_FROM_PIN));
            doc.edges.push(edge(id, TRANSITION_TO_PIN, 1, STATE_IN_PIN));
            rects.insert(id, [0.0, 400.0, 22.0, 422.0]);
        }
        let l = layout(&doc, &rects);
        let (c10, c11, c12) = (
            chip_center(&l, &rects, 10),
            chip_center(&l, &rects, 11),
            chip_center(&l, &rects, 12),
        );
        // West-bound: travel is -x, right of travel is -y (above the line).
        for c in [c10, c11, c12] {
            assert_eq!(c[1], 30.0 - 15.0);
        }
        // Ascending id runs along the flow: 10 first (east), 12 last (west).
        assert_eq!(c11[0], 260.0);
        assert_eq!(c10[0], 260.0 + 26.0);
        assert_eq!(c12[0], 260.0 - 26.0);
        for id in [10u64, 11, 12] {
            assert_eq!(l.chip_dir[&id], [-1.0, 0.0]);
        }
    }

    /// A split wider than the node allows compresses to the node's
    /// half-extent instead of detaching from its border.
    #[test]
    fn lanes_compress_to_what_a_small_node_can_carry() {
        let (mut doc, mut rects) = machine();
        doc.nodes.push(node(11, ANIM_TRANSITION_TYPE_ID));
        doc.edges.push(edge(2, STATE_OUT_PIN, 11, TRANSITION_FROM_PIN));
        doc.edges.push(edge(11, TRANSITION_TO_PIN, 1, STATE_IN_PIN));
        rects.insert(11, [0.0, 400.0, 100.0, 426.0]);
        // Short states (half-extent 10): the ±18 split caps at ±8, on-border.
        rects.insert(1, [0.0, 0.0, 120.0, 20.0]);
        rects.insert(2, [400.0, 0.0, 520.0, 20.0]);
        let l = layout(&doc, &rects);
        assert_eq!(l.segs[&1].a[1], 10.0 + 8.0);
        assert_eq!(l.segs[&3].a[1], 10.0 - 8.0);
        assert_eq!(l.segs[&1].a[0], 120.0, "anchor stays on the right border");
    }

    /// A transition with only its source wired keeps its stored badge and
    /// draws one straight arrow from the state into the badge; the badge
    /// points with that one wire.
    #[test]
    fn a_half_wired_transition_keeps_its_stored_position() {
        let (mut doc, rects) = machine();
        doc.edges.remove(2); // unwire transition → Loco
        let l = layout(&doc, &rects);
        assert!(
            !l.chip_min.contains_key(&10),
            "no derived position without both endpoints"
        );
        let s = &l.segs[&1];
        assert!(s.arrow, "the dangling half still points with the flow");
        // From Idle's border toward the stored chip rect.
        assert!(s.b[1] >= 300.0 && s.b[1] <= 326.0, "lands on the chip border");
        let d = l.chip_dir[&10];
        assert!(d[1] > 0.0, "points down toward the stored badge");
    }

    /// Overlapping states produce no segment — the caller falls back to the
    /// routed wire rather than drawing a lying arrow.
    #[test]
    fn overlapping_states_fall_back() {
        let (doc, mut rects) = machine();
        rects.insert(2, [10.0, 10.0, 130.0, 70.0]); // Loco on top of Idle
        let l = layout(&doc, &rects);
        assert!(!l.segs.contains_key(&1));
        assert!(!l.segs.contains_key(&2));
        assert!(!l.chip_min.contains_key(&10));
    }

    /// A reroute spliced into a flow wire is not a machine connection this
    /// pass can place; those edges are simply absent (router fallback).
    #[test]
    fn a_rerouted_flow_edge_is_left_to_the_router() {
        use crate::engine::node_graph::{REROUTE_IN, REROUTE_OUT, REROUTE_TYPE_ID};
        let (mut doc, mut rects) = machine();
        doc.nodes.push(node(20, REROUTE_TYPE_ID));
        rects.insert(20, [200.0, 100.0, 210.0, 110.0]);
        // Replace the entry wire with entry → reroute → Idle.
        doc.edges[0] = edge(0, STATE_OUT_PIN, 20, REROUTE_IN);
        doc.edges.push(edge(20, REROUTE_OUT, 1, STATE_IN_PIN));
        let l = layout(&doc, &rects);
        assert!(!l.segs.contains_key(&0));
        assert!(!l.segs.contains_key(&3));
        // The real transition is unaffected.
        assert!(l.segs.contains_key(&1) && l.segs.contains_key(&2));
    }

    /// `seg_outside`: a segment ending inside the rect trims to the border;
    /// one crossing straight through splits into two outside pieces; one
    /// that misses the rect passes through untouched.
    /// The drag ghost starts where the center→pointer ray leaves the state
    /// card, and does not exist while the pointer is still inside it.
    #[test]
    fn border_exit_leaves_the_card_toward_the_pointer() {
        let r = [0.0, 0.0, 100.0, 40.0];
        assert_eq!(border_exit(r, [300.0, 20.0]), Some([100.0, 20.0]));
        assert_eq!(border_exit(r, [50.0, -60.0]), Some([50.0, 0.0]));
        assert_eq!(border_exit(r, [80.0, 30.0]), None, "pointer inside");
        assert_eq!(border_exit(r, [50.0, 20.0]), None, "pointer at center");
    }

    #[test]
    fn seg_outside_trims_at_the_card_border() {
        let r = [100.0, 100.0, 200.0, 200.0];
        // Ends at the rect center: one piece, stopping on the left border.
        let cut = seg_outside([0.0, 150.0], [150.0, 150.0], r);
        assert_eq!(cut, vec![([0.0, 150.0], [100.0, 150.0])]);
        // Crosses through: two pieces, the gap exactly the rect's span.
        let through = seg_outside([0.0, 150.0], [300.0, 150.0], r);
        assert_eq!(
            through,
            vec![
                ([0.0, 150.0], [100.0, 150.0]),
                ([200.0, 150.0], [300.0, 150.0]),
            ]
        );
        // Misses entirely: untouched.
        let miss = seg_outside([0.0, 0.0], [300.0, 0.0], r);
        assert_eq!(miss, vec![([0.0, 0.0], [300.0, 0.0])]);
        // Fully inside: nothing to draw.
        assert!(seg_outside([110.0, 150.0], [190.0, 150.0], r).is_empty());
    }

    /// A self-loop keeps the stored chip and draws both halves against it.
    #[test]
    fn a_self_loop_uses_the_stored_chip_position() {
        let (mut doc, rects) = machine();
        doc.edges[2] = edge(10, TRANSITION_TO_PIN, 1, STATE_IN_PIN); // 1 → 10 → 1
        let l = layout(&doc, &rects);
        assert!(!l.chip_min.contains_key(&10));
        assert!(l.segs[&1].arrow && l.segs[&2].arrow);
    }
}
