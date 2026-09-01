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
//! Lane rules (the Unreal pairing): every connection between the same pair
//! of states offsets to the **right of its travel direction**, so an A→B /
//! B→A pair renders as two parallel arrows, one per side of the centerline.
//! Additional same-direction transitions stack further right. Chips stagger
//! along the edge so parallel lanes never pile their chips onto one point.

use std::collections::BTreeMap;

use crate::engine::animation::graph::plan::{
    ANIM_ANY_STATE_TYPE_ID, ANIM_ENTRY_TYPE_ID, ANIM_STATE_TYPE_ID, ANIM_TRANSITION_TYPE_ID,
    STATE_IN_PIN, STATE_OUT_PIN, TRANSITION_FROM_PIN, TRANSITION_TO_PIN,
};
use crate::engine::node_graph::GraphDoc;

/// Distance between parallel lanes, world units. Chosen to clear a chip's
/// height (one `row_h`, ~26) with air on both sides.
pub const LANE_GAP: f32 = 36.0;

/// How far apart chips stagger *along* a shared edge, as a fraction of the
/// border-to-border length per lane index. Keeps two wide chips on a vertical
/// pair from overlapping where the perpendicular gap alone could not.
const STAGGER_T: f32 = 0.16;

/// Chip stagger stays inside the middle of the edge — a chip pinned onto a
/// state border would read as part of the state.
const STAGGER_CLAMP: (f32, f32) = (0.3, 0.7);

/// One straight display segment for a doc edge.
#[derive(Debug, Clone, PartialEq)]
pub struct EdgeSeg {
    pub a: [f32; 2],
    pub b: [f32; 2],
    /// Draw an arrowhead at `b` (the end that lands on a state or a chip).
    pub arrow: bool,
}

/// The derived layout: chip positions and per-edge segments.
#[derive(Debug, Default)]
pub struct AnimFlowLayout {
    /// Transition node id → derived rect **min** (world). Only transitions
    /// with both endpoints resolved appear; everything else keeps its stored
    /// position.
    pub chip_min: BTreeMap<u64, [f32; 2]>,
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
        ANIM_STATE_TYPE_ID | ANIM_ANY_STATE_TYPE_ID | ANIM_ENTRY_TYPE_ID
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

/// Compute the derived machine layout. `rects` are the *displayed* node
/// rects of the current frame — transitions included, for their sizes.
pub fn anim_flow_layout(doc: &GraphDoc, rects: &BTreeMap<u64, [f32; 4]>) -> AnimFlowLayout {
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

    // Lane assignment per unordered endpoint pair.
    let mut groups: BTreeMap<(u64, u64), Vec<usize>> = BTreeMap::new();
    for (i, l) in links.iter().enumerate() {
        groups
            .entry((l.from.min(l.to), l.from.max(l.to)))
            .or_default()
            .push(i);
    }
    for group in groups.values() {
        let n = group.len();
        for (gi, &li) in group.iter().enumerate() {
            let link = &links[li];
            let has_opp = group.iter().any(|&j| links[j].from != link.from);
            let lane = group[..gi]
                .iter()
                .filter(|&&j| links[j].from == link.from)
                .count();
            let perp_off = (if has_opp { 0.5 } else { 0.0 } + lane as f32) * LANE_GAP;
            // Chip stagger in **canonical pair space** (min-id → max-id), so
            // an A→B / B→A pair cannot cancel into the same world point.
            let t_canon = (0.5 + (gi as f32 - (n as f32 - 1.0) * 0.5) * STAGGER_T)
                .clamp(STAGGER_CLAMP.0, STAGGER_CLAMP.1);
            let along_t = if link.from < link.to { t_canon } else { 1.0 - t_canon };

            let (ra, rb) = (rects[&link.from], rects[&link.to]);
            let (ca, cb) = (center(ra), center(rb));
            let d = [cb[0] - ca[0], cb[1] - ca[1]];
            let len = (d[0] * d[0] + d[1] * d[1]).sqrt();
            if len < 1.0 {
                continue;
            }
            // Right of travel, screen coords (y down): east-bound offsets
            // south, so A→B and B→A part to opposite sides of the line.
            let right = [-d[1] / len, d[0] / len];
            // A lane wider than a node's half-extent would detach from its
            // border; lanes compress to what the smaller endpoint can carry.
            let allowed = (half_extent(ra, right).min(half_extent(rb, right)) - 2.0).max(0.0);
            let perp_off = perp_off.min(allowed);
            let oa = [ca[0] + right[0] * perp_off, ca[1] + right[1] * perp_off];
            let ob = [cb[0] + right[0] * perp_off, cb[1] + right[1] * perp_off];
            let (a, t_out) = exit_anchor(oa, ob, ra);
            let (b, t_in) = entry_anchor(oa, ob, rb);
            if t_in <= t_out {
                continue; // rects overlap: no honest line, fall back
            }
            match link.transition {
                Some(t) => {
                    let sz = size(rects[&t]);
                    let c = lerp(a, b, along_t);
                    out.chip_min
                        .insert(t, [c[0] - sz[0] * 0.5, c[1] - sz[1] * 0.5]);
                    out.segs
                        .insert(link.e_from, EdgeSeg { a, b: c, arrow: false });
                    if let Some(eo) = link.e_to {
                        out.segs.insert(eo, EdgeSeg { a: c, b, arrow: true });
                    }
                }
                None => {
                    out.segs.insert(link.e_from, EdgeSeg { a, b, arrow: true });
                }
            }
        }
    }

    // Half-wired (or self-looping) transitions: the chip keeps its stored
    // rect and each existing half draws straight between state border and
    // chip border, arrow pointing with the flow.
    for (t, from_half, to_half) in partial {
        let Some(&rt) = rects.get(&t) else { continue };
        if let Some((ei, from)) = from_half {
            if let Some(&rs) = rects.get(&from) {
                if let Some((a, b)) = border_segment(rs, rt) {
                    out.segs.insert(ei, EdgeSeg { a, b, arrow: true });
                }
            }
        }
        if let Some((eo, to)) = to_half {
            if let Some(&rs) = rects.get(&to) {
                if let Some((a, b)) = border_segment(rt, rs) {
                    out.segs.insert(eo, EdgeSeg { a, b, arrow: true });
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

    /// A fully wired transition renders as two collinear halves — border to
    /// chip center, chip center to border — with the arrow on the landing
    /// half, and the chip's derived rect centered on the joint.
    #[test]
    fn a_wired_transition_becomes_one_direct_edge_with_a_midpoint_chip() {
        let (doc, rects) = machine();
        let l = anim_flow_layout(&doc, &rects);

        let h1 = &l.segs[&1];
        let h2 = &l.segs[&2];
        assert!(!h1.arrow, "the half landing on the chip has no arrowhead");
        assert!(h2.arrow, "the half landing on the state carries the arrow");
        assert_eq!(h1.b, h2.a, "halves meet at the chip center");
        // Straight horizontal run at the centers' height, border to border.
        assert_eq!(h1.a, [120.0, 30.0]);
        assert_eq!(h2.b, [400.0, 30.0]);
        assert_eq!(h1.b[1], 30.0, "single lane rides the centerline");
        // Chip centered on the joint, using its own size.
        let min = l.chip_min[&10];
        assert_eq!([min[0] + 50.0, min[1] + 13.0], [h1.b[0], h1.b[1]]);
    }

    /// The entry wire is a plain direct arrow.
    #[test]
    fn the_entry_wire_is_a_direct_arrow() {
        let (doc, rects) = machine();
        let l = anim_flow_layout(&doc, &rects);
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
        let l = anim_flow_layout(&doc, &rects);

        // Forward transition (1→2, east-bound): right of travel is +y.
        let f = &l.segs[&1];
        assert_eq!(f.a[1], 30.0 + LANE_GAP * 0.5);
        // Reverse (2→1, west-bound): right of travel is -y.
        let r = &l.segs[&3];
        assert_eq!(r.a[1], 30.0 - LANE_GAP * 0.5);
        // Each line is level (parallel to the horizontal centerline).
        assert_eq!(f.a[1], l.segs[&2].b[1]);
        assert_eq!(r.a[1], l.segs[&4].b[1]);
        // Chips stagger along the edge so they cannot overlap.
        assert_ne!(l.chip_min[&10][0], l.chip_min[&11][0]);
    }

    /// Two same-direction transitions stack lanes on the same side.
    #[test]
    fn parallel_same_direction_transitions_stack_lanes() {
        let (mut doc, mut rects) = machine();
        doc.nodes.push(node(11, ANIM_TRANSITION_TYPE_ID));
        doc.edges.push(edge(1, STATE_OUT_PIN, 11, TRANSITION_FROM_PIN));
        doc.edges.push(edge(11, TRANSITION_TO_PIN, 2, STATE_IN_PIN));
        rects.insert(11, [0.0, 400.0, 100.0, 426.0]);
        // Tall states so the second lane fits uncompressed.
        rects.insert(1, [0.0, 0.0, 120.0, 120.0]);
        rects.insert(2, [400.0, 0.0, 520.0, 120.0]);
        let l = anim_flow_layout(&doc, &rects);
        assert_eq!(l.segs[&1].a[1], 60.0);
        assert_eq!(l.segs[&3].a[1], 60.0 + LANE_GAP);
    }

    /// A lane wider than the node allows compresses to the node's half-extent
    /// instead of detaching from its border.
    #[test]
    fn lanes_compress_to_what_a_small_node_can_carry() {
        let (mut doc, mut rects) = machine();
        doc.nodes.push(node(11, ANIM_TRANSITION_TYPE_ID));
        doc.edges.push(edge(1, STATE_OUT_PIN, 11, TRANSITION_FROM_PIN));
        doc.edges.push(edge(11, TRANSITION_TO_PIN, 2, STATE_IN_PIN));
        rects.insert(11, [0.0, 400.0, 100.0, 426.0]);
        let l = anim_flow_layout(&doc, &rects);
        // States are 60 tall (half-extent 30): lane 1 caps at 28, on-border.
        assert_eq!(l.segs[&3].a[1], 30.0 + 28.0);
        assert_eq!(l.segs[&3].a[0], 120.0, "anchor stays on the right border");
    }

    /// A transition with only its source wired keeps its stored chip and
    /// draws one straight arrow from the state into the chip.
    #[test]
    fn a_half_wired_transition_keeps_its_stored_position() {
        let (mut doc, rects) = machine();
        doc.edges.remove(2); // unwire transition → Loco
        let l = anim_flow_layout(&doc, &rects);
        assert!(
            !l.chip_min.contains_key(&10),
            "no derived position without both endpoints"
        );
        let s = &l.segs[&1];
        assert!(s.arrow, "the dangling half still points with the flow");
        // From Idle's border toward the stored chip rect.
        assert!(s.b[1] >= 300.0 && s.b[1] <= 326.0, "lands on the chip border");
    }

    /// Overlapping states produce no segment — the caller falls back to the
    /// routed wire rather than drawing a lying arrow.
    #[test]
    fn overlapping_states_fall_back() {
        let (doc, mut rects) = machine();
        rects.insert(2, [10.0, 10.0, 130.0, 70.0]); // Loco on top of Idle
        let l = anim_flow_layout(&doc, &rects);
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
        let l = anim_flow_layout(&doc, &rects);
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
        let l = anim_flow_layout(&doc, &rects);
        assert!(!l.chip_min.contains_key(&10));
        assert!(l.segs[&1].arrow && l.segs[&2].arrow);
    }
}
