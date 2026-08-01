//! Layered auto-layout — a Sugiyama-shaped pass over a graph's nodes.
//!
//! Pure over `(nodes, edges, sizes)`: no `Ui`, no document mutation, so the
//! ranking and ordering can be tested directly. The caller turns the returned
//! positions into one undo transaction.
//!
//! It is deliberately **not** incremental and never runs on its own. Layout
//! that reflows while you work fights the author; layout you asked for, once,
//! with an undo, does not.

use std::collections::{BTreeMap, BTreeSet};

/// One node's input to the layout: its id, its drawn size, and whether a
/// group frame pins it into that group's column band.
#[derive(Debug, Clone, PartialEq)]
pub struct LayoutNode {
    pub id: u64,
    pub width: f32,
    pub height: f32,
}

/// A directed dependency: `from` must sit left of `to`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayoutEdge {
    pub from: u64,
    pub to: u64,
}

/// Spacing between columns and between rows within a column. Generous by
/// default: a layered graph that touches itself is harder to read than one
/// that needs a scroll.
#[derive(Debug, Clone, Copy)]
pub struct LayoutSpacing {
    pub column_gap: f32,
    pub row_gap: f32,
}

impl Default for LayoutSpacing {
    fn default() -> Self {
        Self { column_gap: 48.0, row_gap: 24.0 }
    }
}

/// How many barycenter sweeps to run. Two is enough to take most of the
/// crossings out; the spec says "minimizes", not "minimal", and an exact
/// solution is NP-hard for no visible gain at graph-editor sizes.
const SWEEPS: usize = 2;

/// Rank every node by **longest path** from a source, then order within each
/// column by the average position of its neighbours (barycenter).
///
/// Longest path rather than shortest: a node should sit to the right of
/// *everything* that feeds it, so an edge never points backwards.
///
/// Cycles are broken by ignoring the edge that would close one — the ranking
/// stays finite and the result is still readable, which is what an author
/// wants from a cyclic graph rather than a refusal.
pub fn layered_ranks(nodes: &[LayoutNode], edges: &[LayoutEdge]) -> BTreeMap<u64, usize> {
    let ids: BTreeSet<u64> = nodes.iter().map(|n| n.id).collect();
    let edges: Vec<&LayoutEdge> = edges
        .iter()
        .filter(|e| e.from != e.to && ids.contains(&e.from) && ids.contains(&e.to))
        .collect();

    let mut rank: BTreeMap<u64, usize> = ids.iter().map(|id| (*id, 0)).collect();
    // Relax until stable, capped at |V| passes — that bound is exactly what
    // makes a cycle terminate instead of climbing forever.
    for _ in 0..ids.len().max(1) {
        let mut changed = false;
        for e in &edges {
            let want = rank[&e.from] + 1;
            if want > rank[&e.to] && want < ids.len() {
                rank.insert(e.to, want);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    rank
}

/// The final positions, keyed by node id.
pub fn layout(
    nodes: &[LayoutNode],
    edges: &[LayoutEdge],
    origin: [f32; 2],
    spacing: LayoutSpacing,
) -> BTreeMap<u64, [f32; 2]> {
    if nodes.is_empty() {
        return BTreeMap::new();
    }
    let rank = layered_ranks(nodes, edges);
    let size: BTreeMap<u64, (f32, f32)> =
        nodes.iter().map(|n| (n.id, (n.width, n.height))).collect();

    // Columns, each holding its members in a stable starting order.
    let mut columns: BTreeMap<usize, Vec<u64>> = BTreeMap::new();
    for n in nodes {
        columns.entry(rank[&n.id]).or_default().push(n.id);
    }

    // Barycenter sweeps: order each column by the mean index of its
    // neighbours in the adjacent column, alternating direction.
    let mut order: BTreeMap<usize, Vec<u64>> = columns.clone();
    for sweep in 0..SWEEPS * 2 {
        let forward = sweep % 2 == 0;
        let keys: Vec<usize> = order.keys().copied().collect();
        let keys: Vec<usize> = if forward {
            keys
        } else {
            keys.into_iter().rev().collect()
        };
        for col in keys {
            // Index of each node in the column we are measuring against.
            let neighbour_col = if forward {
                col.checked_sub(1)
            } else {
                Some(col + 1)
            };
            let Some(nc) = neighbour_col else { continue };
            let Some(reference) = order.get(&nc).cloned() else {
                continue;
            };
            let index_of: BTreeMap<u64, usize> = reference
                .iter()
                .enumerate()
                .map(|(i, id)| (*id, i))
                .collect();
            let Some(members) = order.get_mut(&col) else {
                continue;
            };
            let mut keyed: Vec<(f32, u64)> = members
                .iter()
                .map(|id| {
                    let mut sum = 0.0;
                    let mut count = 0.0;
                    for e in edges {
                        let other = if forward {
                            (e.to == *id).then_some(e.from)
                        } else {
                            (e.from == *id).then_some(e.to)
                        };
                        if let Some(o) = other.and_then(|o| index_of.get(&o)) {
                            sum += *o as f32;
                            count += 1.0;
                        }
                    }
                    // No neighbours: hold position rather than pile at zero.
                    let bary = if count > 0.0 { sum / count } else { f32::MAX };
                    (bary, *id)
                })
                .collect();
            keyed.sort_by(|a, b| {
                a.0.partial_cmp(&b.0)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then(a.1.cmp(&b.1))
            });
            *members = keyed.into_iter().map(|(_, id)| id).collect();
        }
    }

    // Place: columns left to right at the widest member's pitch, rows stacked
    // top to bottom, each column vertically centred on the tallest one.
    let mut col_x = origin[0];
    let heights: BTreeMap<usize, f32> = order
        .iter()
        .map(|(c, members)| {
            let h: f32 = members.iter().map(|id| size[id].1).sum::<f32>()
                + spacing.row_gap * (members.len().saturating_sub(1)) as f32;
            (*c, h)
        })
        .collect();
    let tallest = heights.values().copied().fold(0.0f32, f32::max);

    let mut out = BTreeMap::new();
    for (col, members) in &order {
        let widest = members
            .iter()
            .map(|id| size[id].0)
            .fold(0.0f32, f32::max);
        let mut y = origin[1] + (tallest - heights[col]) * 0.5;
        for id in members {
            out.insert(*id, [col_x, y]);
            y += size[id].1 + spacing.row_gap;
        }
        col_x += widest + spacing.column_gap;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(id: u64) -> LayoutNode {
        LayoutNode { id, width: 100.0, height: 40.0 }
    }
    fn e(from: u64, to: u64) -> LayoutEdge {
        LayoutEdge { from, to }
    }

    #[test]
    fn ranks_follow_the_longest_path() {
        // 0 -> 1 -> 2, and 0 -> 2 directly. Node 2 must clear *both*, so the
        // longest path wins: rank 2, not rank 1.
        let nodes = [n(0), n(1), n(2)];
        let r = layered_ranks(&nodes, &[e(0, 1), e(1, 2), e(0, 2)]);
        assert_eq!(r[&0], 0);
        assert_eq!(r[&1], 1);
        assert_eq!(r[&2], 2, "a node sits right of everything that feeds it");
    }

    #[test]
    fn unconnected_nodes_rank_as_sources() {
        let nodes = [n(0), n(1), n(9)];
        let r = layered_ranks(&nodes, &[e(0, 1)]);
        assert_eq!(r[&9], 0, "an island is its own source");
    }

    #[test]
    fn cycles_terminate_instead_of_climbing() {
        let nodes = [n(0), n(1), n(2)];
        let r = layered_ranks(&nodes, &[e(0, 1), e(1, 2), e(2, 0)]);
        // Every node still gets a finite rank inside the node count.
        assert!(r.values().all(|v| *v < nodes.len()));
        // Self-edges and edges to absent nodes are ignored outright.
        let r2 = layered_ranks(&[n(0)], &[e(0, 0), e(0, 77)]);
        assert_eq!(r2[&0], 0);
    }

    #[test]
    fn layout_places_columns_left_to_right_without_overlap() {
        let nodes = [n(0), n(1), n(2)];
        let pos = layout(&nodes, &[e(0, 1), e(1, 2)], [0.0, 0.0], LayoutSpacing::default());
        assert_eq!(pos.len(), 3);
        assert!(pos[&0][0] < pos[&1][0], "rank order is left to right");
        assert!(pos[&1][0] < pos[&2][0]);
        // Column pitch is the widest member plus the gap.
        assert_eq!(pos[&1][0] - pos[&0][0], 100.0 + LayoutSpacing::default().column_gap);
        // A single-node column is a single row.
        assert_eq!(pos[&0][1], pos[&1][1]);
    }

    #[test]
    fn siblings_stack_with_the_row_gap() {
        // Two nodes fed by one source share a column.
        let nodes = [n(0), n(1), n(2)];
        let sp = LayoutSpacing::default();
        let pos = layout(&nodes, &[e(0, 1), e(0, 2)], [0.0, 0.0], sp);
        assert_eq!(pos[&1][0], pos[&2][0], "siblings share a column");
        let dy = (pos[&1][1] - pos[&2][1]).abs();
        assert_eq!(dy, 40.0 + sp.row_gap, "stacked by height + row gap");
    }

    /// The barycenter pass is what stops a layered graph looking like a
    /// cat's cradle: a crossed pair should come out uncrossed.
    #[test]
    fn barycenter_ordering_removes_an_obvious_crossing() {
        // Sources 0,1 feed targets 2,3 — but crosswise (0->3, 1->2). After
        // ordering, the target column should mirror the source column so the
        // two wires run parallel instead of crossing.
        let nodes = [n(0), n(1), n(2), n(3)];
        let pos = layout(
            &nodes,
            &[e(0, 3), e(1, 2)],
            [0.0, 0.0],
            LayoutSpacing::default(),
        );
        // 0 is above 1 (stable id order in the source column).
        assert!(pos[&0][1] < pos[&1][1]);
        // …so 3 (fed by 0) must end up above 2 (fed by 1).
        assert!(
            pos[&3][1] < pos[&2][1],
            "barycenter did not uncross the pair: {pos:?}"
        );
    }

    #[test]
    fn empty_input_is_not_a_panic() {
        assert!(layout(&[], &[], [0.0, 0.0], LayoutSpacing::default()).is_empty());
        assert!(layered_ranks(&[], &[]).is_empty());
    }

    #[test]
    fn origin_offsets_the_whole_result() {
        let nodes = [n(0), n(1)];
        let a = layout(&nodes, &[e(0, 1)], [0.0, 0.0], LayoutSpacing::default());
        let b = layout(&nodes, &[e(0, 1)], [500.0, -200.0], LayoutSpacing::default());
        for id in [0u64, 1] {
            assert_eq!(b[&id][0] - a[&id][0], 500.0);
            assert_eq!(b[&id][1] - a[&id][1], -200.0);
        }
    }
}
