//! Doc-local graph validation (plan D3, `validate_doc` layer): pure over
//! `(doc, registry)`, no I/O. Cross-asset checks (subgraph interfaces,
//! reference cycles) live in the resolver layer (`validate_refs`, P6).
//!
//! Unknown node types are errors but never fatal: the doc still loads and
//! re-saves without data loss, so a disabled 39.8 plugin can't eat graphs.

use std::collections::{BTreeMap, BTreeSet};

use crate::descriptors::{DocDescriptors, NodeKind};
use crate::doc::{Edge, GraphDoc, GraphRealm, NodeRealm, PinType};
use crate::registry::{
    NodeRegistry, GRAPH_INPUT_TYPE_ID, GRAPH_OUTPUT_TYPE_ID, REROUTE_IN, REROUTE_OUT,
    REROUTE_TYPE_ID,
};

/// How loudly an error reads. The set is closed and every variant is
/// classified in [`GraphError::severity`] — a warning is a document that
/// *runs* but is probably not what the author meant (an interface pin nothing
/// binds), an error is one that cannot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorSeverity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq)]
pub enum GraphError {
    DuplicateNodeId(u64),
    UnknownNodeType { node: u64, type_id: String },
    /// Edge endpoint references a node id not present in the doc.
    DanglingEdgeNode { edge: Edge },
    /// Edge endpoint references a pin slug the node type doesn't have on
    /// that side (outputs feed `from`, inputs feed `to`).
    UnknownPin { node: u64, pin: String, output: bool },
    TypeMismatch { edge: Edge, from_ty: PinType, to_ty: PinType },
    /// An input pin has more than one incoming edge (outputs may fan out;
    /// inputs may not).
    InputMultiplyConnected { node: u64, pin: String },
    RealmViolation { node: u64, node_realm: NodeRealm, graph_realm: GraphRealm },
    /// A `Domain` pin type nobody registered with the registry.
    UnknownDomainPin { node: u64, pin: String, domain: String },
    /// Subgraph node references an asset the resolver can't find
    /// (`validate_refs`).
    MissingSubgraph { node: u64, path: String },
    /// Edge references a pin the referenced subgraph's interface no longer
    /// declares (`validate_refs`).
    SubgraphPinUnknown { node: u64, pin: String, output: bool },
    /// The subgraph reference graph has a cycle (`validate_refs`).
    SubgraphCycle { chain: Vec<String> },
    /// A cycle in *data* wires that no impure node breaks. Exec wires may
    /// loop — that is what a loop node is — but a pure pull chain that feeds
    /// itself has no fixed point, so the interpreter rejects it up front
    /// rather than discovering it at 100k firings (45-A D1).
    ///
    /// `nodes` is the cycle in traversal order, rotated to start at its
    /// lowest node id so the same cycle always reports the same way.
    DataCycle { nodes: Vec<u64> },
    /// A `var_get` / `var_set` node naming a variable the document does not
    /// declare (deleted, or renamed without updating the node).
    UnknownVariable { node: u64, slug: String },
    /// A `graph_input` / `graph_output` node in a document that declares no
    /// matching interface — the node has nothing to mirror.
    InterfaceNodeInvalid { node: u64, output: bool },
    /// A declared interface pin that no `graph_input`/`graph_output` node
    /// wires to anything. The subgraph still loads and still runs; that pin
    /// just goes nowhere, which is a warning, not a broken document.
    InterfacePinUnbound { pin: String, output: bool },
    /// A Timeline node naming a `.curve` the resolver cannot produce — an
    /// empty reference, or a file that is missing or unreadable. Its track
    /// outputs are unknowable, so every wire leaving it is dangling; the same
    /// statement `MissingSubgraph` makes about an interface (45-A P8).
    ///
    /// Only reported when curves were actually offered
    /// ([`DocDescriptors::has_curves`]): a doc-local validation pass has no
    /// business claiming a file is missing when it was never handed a way to
    /// look.
    MissingCurve { node: u64, path: String },
}

/// Where an error belongs on the canvas. The error UI is complete rather than
/// best-effort precisely because the set is closed: every variant has exactly
/// one place it can be anchored, and the corner overlay is demoted to a count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorAnchor {
    /// Node border + one gutter badge in the header.
    Node(u64),
    /// A 1px error ring around that pin.
    Pin { node: u64, pin: String, output: bool },
    /// The wire itself — `TypeMismatch` is the only error that colors one.
    Edge(Edge),
    /// A dashed ghost row appended to the node, so the wire has somewhere to
    /// land instead of vanishing.
    GhostPin { node: u64, pin: String, output: bool },
    /// Nothing on the canvas owns it — it goes in the compiler-row popover.
    Document,
}

impl GraphError {
    /// The one place this error anchors. Kept next to the variants so adding
    /// a twelfth error forces a decision about where it renders.
    pub fn anchor(&self) -> ErrorAnchor {
        match self {
            GraphError::DuplicateNodeId(id) => ErrorAnchor::Node(*id),
            GraphError::UnknownNodeType { node, .. } => ErrorAnchor::Node(*node),
            GraphError::RealmViolation { node, .. } => ErrorAnchor::Node(*node),
            // A missing subgraph draws on the node that pulled it in *and*
            // lists in the popover — reference errors stay visually separate
            // from doc errors, but the node still has to say something.
            GraphError::MissingSubgraph { node, .. } => ErrorAnchor::Node(*node),
            // Same reasoning for a missing curve: the node that named it is
            // the node whose pins are now unknowable.
            GraphError::MissingCurve { node, .. } => ErrorAnchor::Node(*node),
            GraphError::UnknownDomainPin { node, pin, .. } => ErrorAnchor::Pin {
                node: *node,
                pin: pin.clone(),
                output: false,
            },
            GraphError::InputMultiplyConnected { node, pin } => ErrorAnchor::Pin {
                node: *node,
                pin: pin.clone(),
                output: false,
            },
            GraphError::TypeMismatch { edge, .. } => ErrorAnchor::Edge(edge.clone()),
            // Both unknown-pin variants get the ghost-row treatment: the pin
            // the edge names does not exist, so one is drawn for it.
            GraphError::UnknownPin { node, pin, output } => ErrorAnchor::GhostPin {
                node: *node,
                pin: pin.clone(),
                output: *output,
            },
            GraphError::SubgraphPinUnknown { node, pin, output } => ErrorAnchor::GhostPin {
                node: *node,
                pin: pin.clone(),
                output: *output,
            },
            GraphError::UnknownVariable { node, .. } => ErrorAnchor::Node(*node),
            GraphError::InterfaceNodeInvalid { node, .. } => ErrorAnchor::Node(*node),
            // The cycle's entry node — the lowest id in it, so the badge
            // lands in the same place every validation pass.
            GraphError::DataCycle { nodes } => ErrorAnchor::Node(nodes.first().copied()
                .unwrap_or_default()),
            // An unbound interface pin is declared on the *document*, not on
            // any node, so the compiler-row popover owns it.
            GraphError::DanglingEdgeNode { .. }
            | GraphError::SubgraphCycle { .. }
            | GraphError::InterfacePinUnbound { .. } => ErrorAnchor::Document,
        }
    }

    /// How loudly this reads. Only the unbound-interface-pin case is a
    /// warning (45-A D3); everything else stops the document from running.
    pub fn severity(&self) -> ErrorSeverity {
        match self {
            GraphError::InterfacePinUnbound { .. } => ErrorSeverity::Warning,
            _ => ErrorSeverity::Error,
        }
    }

    /// The subgraph chain of a cycle, for the clickable mono breadcrumb.
    pub fn cycle_chain(&self) -> Option<&[String]> {
        match self {
            GraphError::SubgraphCycle { chain } => Some(chain),
            _ => None,
        }
    }
}

impl std::fmt::Display for GraphError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GraphError::DuplicateNodeId(id) => write!(f, "duplicate node id {id}"),
            GraphError::UnknownNodeType { node, type_id } => {
                write!(f, "node {node}: unknown type '{type_id}'")
            }
            GraphError::MissingCurve { node, path } if path.is_empty() => {
                write!(f, "node {node}: Timeline names no curve asset")
            }
            GraphError::MissingCurve { node, path } => {
                write!(f, "node {node}: curve '{path}' not found")
            }
            GraphError::DanglingEdgeNode { edge } => write!(
                f,
                "edge {}:{} -> {}:{} references a missing node",
                edge.from_node, edge.from_pin, edge.to_node, edge.to_pin
            ),
            GraphError::UnknownPin { node, pin, output } => write!(
                f,
                "node {node}: no {} pin '{pin}'",
                if *output { "output" } else { "input" }
            ),
            GraphError::TypeMismatch { edge, from_ty, to_ty } => write!(
                f,
                "edge {}:{} -> {}:{}: {from_ty:?} does not connect to {to_ty:?}",
                edge.from_node, edge.from_pin, edge.to_node, edge.to_pin
            ),
            GraphError::InputMultiplyConnected { node, pin } => {
                write!(f, "node {node}: input '{pin}' has multiple connections")
            }
            GraphError::RealmViolation { node, node_realm, graph_realm } => write!(
                f,
                "node {node}: realm {node_realm:?} not allowed in a {graph_realm:?} graph"
            ),
            GraphError::UnknownDomainPin { node, pin, domain } => {
                write!(f, "node {node}: pin '{pin}' uses unregistered domain type '{domain}'")
            }
            GraphError::MissingSubgraph { node, path } => {
                write!(f, "node {node}: subgraph '{path}' not found")
            }
            GraphError::SubgraphPinUnknown { node, pin, output } => write!(
                f,
                "node {node}: subgraph interface has no {} pin '{pin}'",
                if *output { "output" } else { "input" }
            ),
            GraphError::SubgraphCycle { chain } => {
                write!(f, "subgraph reference cycle: {}", chain.join(" -> "))
            }
            GraphError::DataCycle { nodes } => write!(
                f,
                "data cycle: {}",
                nodes
                    .iter()
                    .map(|n| n.to_string())
                    .collect::<Vec<_>>()
                    .join(" -> ")
            ),
            GraphError::UnknownVariable { node, slug } => {
                write!(f, "node {node}: no variable '{slug}' in this graph")
            }
            GraphError::InterfaceNodeInvalid { node, output } => write!(
                f,
                "node {node}: graph_{} needs a declared interface",
                if *output { "output" } else { "input" }
            ),
            GraphError::InterfacePinUnbound { pin, output } => write!(
                f,
                "interface {} '{pin}' is not bound by any graph_{} node",
                if *output { "output" } else { "input" },
                if *output { "output" } else { "input" }
            ),
        }
    }
}

/// The pin type flowing through a reroute chain. Thin wrapper over
/// [`DocDescriptors::reroute_type`], kept because call sites hold a
/// `(doc, registry)` pair rather than a resolver.
pub fn reroute_type(doc: &GraphDoc, registry: &NodeRegistry, node: u64) -> Option<PinType> {
    DocDescriptors::new(doc, registry).reroute_type(node)
}

/// The declared type of one pin, seeing through reroutes. `None` means
/// "cannot be determined here" — an unregistered node type, a subgraph
/// interface (the resolver layer owns those), or an unwired reroute — and is
/// never the same claim as "the types differ".
pub fn endpoint_type(
    doc: &GraphDoc,
    registry: &NodeRegistry,
    node: u64,
    pin: &str,
    output: bool,
) -> Option<PinType> {
    DocDescriptors::new(doc, registry).pin_type(node, pin, output)
}

/// Validate everything checkable without loading other assets. Subgraph
/// nodes (reserved type, pins derived from the referenced asset) are only
/// checked for duplicate ids here — their pins and cycles are `validate_refs`
/// territory, and edges touching them are skipped rather than misreported.
pub fn validate_doc(doc: &GraphDoc, registry: &NodeRegistry) -> Vec<GraphError> {
    validate_doc_with(&DocDescriptors::new(doc, registry))
}

/// The one curve rule, in one place: a Timeline whose `.curve` does not
/// resolve. Silent when the descriptor set was given no curves at all — "I
/// was not asked to look" is not "it is missing" (see
/// [`DocDescriptors::has_curves`]).
fn timeline_curve_error(d: &DocDescriptors<'_>, node: u64) -> Option<GraphError> {
    (d.has_curves() && d.curve_of(node).is_none()).then(|| GraphError::MissingCurve {
        node,
        path: d.curve_path(node).unwrap_or_default().to_string(),
    })
}

/// Just the curve rule, over every Timeline in the document.
///
/// The editor needs this on its own: `.curve` resolution is a *cross-asset*
/// question, so it belongs to the per-frame pass that already rebuilds the
/// subgraph resolver — not to the doc-local `validate_doc` that runs on every
/// keystroke. Sharing [`timeline_curve_error`] with [`validate_doc_with`] is
/// what keeps the editor's verdict and the compiler's identical (45-A P8b).
pub fn validate_curves(d: &DocDescriptors<'_>) -> Vec<GraphError> {
    d.doc()
        .nodes
        .iter()
        .filter(|n| NodeKind::of_type(&n.type_id) == NodeKind::Timeline)
        .filter_map(|n| timeline_curve_error(d, n.id))
        .collect()
}

/// The same rules against an explicit resolver. Only the data-cycle rule
/// differs in practice: with a resolver it can see whether a subgraph node
/// pulls its outputs through from its inputs, and therefore whether a cycle
/// crossing that boundary is real.
pub fn validate_doc_with(d: &DocDescriptors<'_>) -> Vec<GraphError> {
    let (doc, registry) = (d.doc(), d.registry());
    let mut errors = Vec::new();

    // Node ids + per-node checks.
    let mut ids = BTreeSet::new();
    for n in &doc.nodes {
        if !ids.insert(n.id) {
            errors.push(GraphError::DuplicateNodeId(n.id));
        }
        let kind = NodeKind::of_type(&n.type_id);
        match kind {
            // A subgraph's pins live in another asset (`validate_refs`); a
            // reroute has no descriptor by design.
            NodeKind::Subgraph | NodeKind::Reroute => continue,
            NodeKind::GraphInput | NodeKind::GraphOutput => {
                let iface = if kind == NodeKind::GraphInput { &doc.inputs } else { &doc.outputs };
                if iface.is_empty() {
                    errors.push(GraphError::InterfaceNodeInvalid {
                        node: n.id,
                        output: kind == NodeKind::GraphOutput,
                    });
                }
                continue;
            }
            NodeKind::VarGet | NodeKind::VarSet => {
                if d.variable_of(n.id).is_none() {
                    errors.push(GraphError::UnknownVariable {
                        node: n.id,
                        slug: d.variable_slug(n.id).unwrap_or_default().to_string(),
                    });
                }
                continue;
            }
            NodeKind::Timeline => errors.extend(timeline_curve_error(d, n.id)),
            NodeKind::Registered | NodeKind::EventCustom => {}
        }
        match d.descriptor(n.id) {
            None => {
                errors.push(GraphError::UnknownNodeType {
                    node: n.id,
                    type_id: n.type_id.clone(),
                });
            }
            Some(desc) => {
                if !desc.realm.admits(doc.realm) {
                    errors.push(GraphError::RealmViolation {
                        node: n.id,
                        node_realm: desc.realm,
                        graph_realm: doc.realm,
                    });
                }
                for p in desc.inputs.iter().chain(desc.outputs.iter()) {
                    // `Array(Domain(..))` is still a domain pin: the check
                    // sees through the container.
                    if let Some(slug) = p.ty.domain_slug() {
                        if !registry.domain_pin_registered(slug) {
                            errors.push(GraphError::UnknownDomainPin {
                                node: n.id,
                                pin: p.slug.clone(),
                                domain: slug.to_string(),
                            });
                        }
                    }
                }
            }
        }
    }

    // Interface pins nothing binds (warning-level): a subgraph that declares
    // an input no `graph_input` node feeds anywhere is almost certainly a
    // half-finished edit.
    errors.extend(unbound_interface_pins(doc));

    // Edge checks. Nodes that are unknown/subgraph get their edges skipped
    // instead of cascading noise.
    let mut input_use: BTreeMap<(u64, &str), u32> = BTreeMap::new();
    for e in &doc.edges {
        let (from, to) = (doc.node(e.from_node), doc.node(e.to_node));
        let (Some(from), Some(to)) = (from, to) else {
            errors.push(GraphError::DanglingEdgeNode { edge: e.clone() });
            continue;
        };
        *input_use.entry((to.id, e.to_pin.as_str())).or_default() += 1;

        // A reroute has no descriptor: its type is whatever reaches it, so
        // both sides resolve through `reroute_type` instead.
        if from.type_id == REROUTE_TYPE_ID || to.type_id == REROUTE_TYPE_ID {
            if from.type_id == REROUTE_TYPE_ID && e.from_pin != REROUTE_OUT {
                errors.push(GraphError::UnknownPin {
                    node: from.id,
                    pin: e.from_pin.clone(),
                    output: true,
                });
            }
            if to.type_id == REROUTE_TYPE_ID && e.to_pin != REROUTE_IN {
                errors.push(GraphError::UnknownPin {
                    node: to.id,
                    pin: e.to_pin.clone(),
                    output: false,
                });
            }
            let a = d.pin_type(e.from_node, &e.from_pin, true);
            let b = d.pin_type(e.to_node, &e.to_pin, false);
            if let (Some(a), Some(b)) = (a, b) {
                if a != b {
                    errors.push(GraphError::TypeMismatch {
                        edge: e.clone(),
                        from_ty: a,
                        to_ty: b,
                    });
                }
            }
            continue;
        }

        // Subgraph edges stay `validate_refs` territory even when a resolver
        // is in reach, so the two passes never report the same wire twice.
        //
        // A Timeline whose curve did not resolve is the same situation one
        // asset over: its track pins live in a file nobody handed us, so every
        // wire leaving it would report as `UnknownPin` and bury the one error
        // that matters. `MissingCurve` says it once, on the node, and only
        // when curves were actually offered.
        let opaque = |n: &crate::doc::NodeInst| {
            let kind = NodeKind::of_type(&n.type_id);
            kind == NodeKind::Subgraph || (kind == NodeKind::Timeline && d.curve_of(n.id).is_none())
        };
        let from_desc = (!opaque(from)).then(|| d.descriptor(from.id)).flatten();
        let to_desc = (!opaque(to)).then(|| d.descriptor(to.id)).flatten();

        let from_ty = match from_desc {
            Some(d) => match d.output(&e.from_pin) {
                Some(p) => Some(p.ty.clone()),
                None => {
                    errors.push(GraphError::UnknownPin {
                        node: from.id,
                        pin: e.from_pin.clone(),
                        output: true,
                    });
                    None
                }
            },
            None => None,
        };
        let to_ty = match to_desc {
            Some(d) => match d.input(&e.to_pin) {
                Some(p) => Some(p.ty.clone()),
                None => {
                    errors.push(GraphError::UnknownPin {
                        node: to.id,
                        pin: e.to_pin.clone(),
                        output: false,
                    });
                    None
                }
            },
            None => None,
        };
        if let (Some(from_ty), Some(to_ty)) = (from_ty, to_ty) {
            // No implicit conversions in v1; exec only connects to exec
            // (both are exactly this equality rule).
            if from_ty != to_ty {
                errors.push(GraphError::TypeMismatch { edge: e.clone(), from_ty, to_ty });
            }
        }
    }
    for ((node, pin), count) in input_use {
        if count <= 1 {
            continue;
        }
        // **Exec inputs may fan in** (ruling, 45-A P3). An exec input is a
        // *continuation target*, not a value source: three wires into it mean
        // three places execution can arrive from, which is the fundamental
        // convergence pattern — Branch's True and False rejoining a shared
        // tail. A *data* input still takes exactly one wire, because two
        // values arriving at one pin has no meaning.
        //
        // The type comes from the resolver, so doc-dependent nodes (variable
        // writes, interface binding, inlined subgraph pins) answer correctly.
        // An untyped reroute answers `None` and stays strict: refusing is the
        // honest answer when the type cannot be proven exec.
        if d.pin_type(node, pin, false) == Some(PinType::Exec) {
            continue;
        }
        errors.push(GraphError::InputMultiplyConnected {
            node,
            pin: pin.to_string(),
        });
    }

    errors.extend(data_cycles(d));

    errors
}

/// Declared interface pins that no `graph_input`/`graph_output` node wires to
/// anything — including the case where the binding node is missing entirely.
fn unbound_interface_pins(doc: &GraphDoc) -> Vec<GraphError> {
    let mut out = Vec::new();
    for (iface, output) in [(&doc.inputs, false), (&doc.outputs, true)] {
        if iface.is_empty() {
            continue;
        }
        let want = if output { GRAPH_OUTPUT_TYPE_ID } else { GRAPH_INPUT_TYPE_ID };
        let binders: Vec<u64> = doc
            .nodes
            .iter()
            .filter(|n| n.type_id == want)
            .map(|n| n.id)
            .collect();
        for p in iface {
            // The input node *produces* its pins (edges leave it); the
            // output node *consumes* its pins (edges arrive at it).
            let bound = binders.iter().any(|id| {
                doc.edges.iter().any(|e| {
                    if output {
                        e.to_node == *id && e.to_pin == p.slug
                    } else {
                        e.from_node == *id && e.from_pin == p.slug
                    }
                })
            });
            if !bound {
                out.push(GraphError::InterfacePinUnbound {
                    pin: p.slug.clone(),
                    output,
                });
            }
        }
    }
    out
}

/// Cycles in the *data* dependency graph (45-A D1: "the data-pin subgraph
/// must be a DAG; exec edges may loop").
///
/// An edge only creates a dependency when its source node **pulls through** —
/// a pure node computes its outputs from its inputs on demand, so it depends
/// on them; an impure node's outputs are values it stored when it fired, so
/// it breaks the chain. Reroutes are transparent. Anything unresolvable
/// (unknown type, unresolved subgraph, untyped reroute) contributes no edge:
/// the rule reports cycles it can prove, never ones it guesses.
fn data_cycles(d: &DocDescriptors<'_>) -> Vec<GraphError> {
    let doc = d.doc();
    let mut deps: BTreeMap<u64, BTreeSet<u64>> = BTreeMap::new();
    for e in &doc.edges {
        if doc.node(e.from_node).is_none() || doc.node(e.to_node).is_none() {
            continue;
        }
        // Exec wires may loop; unknown-typed wires are not claimed either way.
        match d.pin_type(e.from_node, &e.from_pin, true) {
            Some(PinType::Exec) | None => continue,
            Some(_) => {}
        }
        if d.pulls_through(e.from_node) != Some(true) {
            continue;
        }
        deps.entry(e.to_node).or_default().insert(e.from_node);
    }

    // Iterative DFS with an explicit path, so a 5,000-edge document cannot
    // blow the stack the way recursion would.
    let mut errors: Vec<GraphError> = Vec::new();
    let mut seen: BTreeSet<Vec<u64>> = BTreeSet::new();
    let mut done: BTreeSet<u64> = BTreeSet::new();
    for &start in deps.keys() {
        if done.contains(&start) {
            continue;
        }
        let mut path: Vec<u64> = Vec::new();
        let mut stack: Vec<(u64, Vec<u64>)> = vec![(
            start,
            deps.get(&start).map(|s| s.iter().copied().collect()).unwrap_or_default(),
        )];
        path.push(start);
        while let Some((node, todo)) = stack.last_mut() {
            let Some(next) = todo.pop() else {
                done.insert(*node);
                path.pop();
                stack.pop();
                continue;
            };
            if let Some(pos) = path.iter().position(|p| *p == next) {
                let mut cycle = path[pos..].to_vec();
                // Rotate to the lowest id: the same cycle must report the
                // same way whichever node the walk entered it from.
                if let Some(min_at) = cycle
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, v)| **v)
                    .map(|(i, _)| i)
                {
                    cycle.rotate_left(min_at);
                }
                if seen.insert(cycle.clone()) {
                    errors.push(GraphError::DataCycle { nodes: cycle });
                }
                continue;
            }
            if done.contains(&next) {
                continue;
            }
            path.push(next);
            stack.push((
                next,
                deps.get(&next).map(|s| s.iter().copied().collect()).unwrap_or_default(),
            ));
        }
    }
    errors
}

#[cfg(test)]
// Tests build documents the way an author does: start from the default and
// fill in what matters. One giant struct literal per fixture would satisfy
// clippy and read markedly worse.
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use crate::dev_nodes::register_dev_nodes;
    use crate::doc::NodeInst;
    use crate::registry::{NodeDescriptor, PinDescriptor};
    use crate::registry::{GRAPH_INPUT_TYPE_ID, GRAPH_OUTPUT_TYPE_ID, SUBGRAPH_TYPE_ID};
    use std::collections::BTreeMap;

    /// Every variant anchors somewhere, and the two unknown-pin variants both
    /// take the ghost-row treatment (recorded ruling). If a twelfth error is
    /// ever added, this fails until it is given a home.
    #[test]
    fn every_error_has_an_anchor() {
        let e = Edge {
            from_node: 0,
            from_pin: "sum".into(),
            to_node: 1,
            to_pin: "a".into(),
        };
        let all = [
            GraphError::DuplicateNodeId(1),
            GraphError::UnknownNodeType { node: 1, type_id: "x".into() },
            GraphError::DanglingEdgeNode { edge: e.clone() },
            GraphError::UnknownPin { node: 1, pin: "p".into(), output: false },
            GraphError::TypeMismatch {
                edge: e.clone(),
                from_ty: PinType::Float,
                to_ty: PinType::Bool,
            },
            GraphError::InputMultiplyConnected { node: 1, pin: "a".into() },
            GraphError::RealmViolation {
                node: 1,
                node_realm: NodeRealm::ServerSafe,
                graph_realm: GraphRealm::Client,
            },
            GraphError::UnknownDomainPin {
                node: 1,
                pin: "p".into(),
                domain: "shader".into(),
            },
            GraphError::MissingSubgraph { node: 1, path: "a.subgraph".into() },
            GraphError::SubgraphPinUnknown { node: 1, pin: "p".into(), output: true },
            GraphError::SubgraphCycle { chain: vec!["a".into(), "b".into()] },
            GraphError::DataCycle { nodes: vec![3, 7] },
            GraphError::UnknownVariable { node: 1, slug: "score".into() },
            GraphError::InterfaceNodeInvalid { node: 1, output: false },
            GraphError::InterfacePinUnbound { pin: "amount".into(), output: false },
        ];
        // The closed set is fifteen (eleven from Task 40, four from 45-A);
        // the UI is complete because the set is.
        assert_eq!(all.len(), 15);

        assert_eq!(all[0].anchor(), ErrorAnchor::Node(1));
        assert_eq!(all[1].anchor(), ErrorAnchor::Node(1));
        assert_eq!(all[2].anchor(), ErrorAnchor::Document);
        assert!(matches!(all[3].anchor(), ErrorAnchor::GhostPin { .. }));
        assert_eq!(all[4].anchor(), ErrorAnchor::Edge(e));
        assert!(matches!(all[5].anchor(), ErrorAnchor::Pin { .. }));
        assert_eq!(all[6].anchor(), ErrorAnchor::Node(1));
        assert!(matches!(all[7].anchor(), ErrorAnchor::Pin { .. }));
        assert_eq!(all[8].anchor(), ErrorAnchor::Node(1));
        assert!(matches!(all[9].anchor(), ErrorAnchor::GhostPin { .. }));
        assert_eq!(all[10].anchor(), ErrorAnchor::Document);

        assert_eq!(all[11].anchor(), ErrorAnchor::Node(3), "the cycle's lowest id");
        assert_eq!(all[12].anchor(), ErrorAnchor::Node(1));
        assert_eq!(all[13].anchor(), ErrorAnchor::Node(1));
        assert_eq!(all[14].anchor(), ErrorAnchor::Document);

        // Only the cycle carries a breadcrumb.
        assert_eq!(all[10].cycle_chain().map(|c| c.len()), Some(2));
        assert!(all[0].cycle_chain().is_none());

        // Exactly one variant is a warning: a document with an interface pin
        // nothing binds still runs.
        let warnings: Vec<&GraphError> = all
            .iter()
            .filter(|e| e.severity() == ErrorSeverity::Warning)
            .collect();
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(matches!(warnings[0], GraphError::InterfacePinUnbound { .. }));
    }

    /// A reroute takes the type of whatever feeds it, through a chain, and
    /// stays untyped (not wrong) while nothing is connected.
    #[test]
    fn reroute_infers_its_type_through_a_chain() {
        let reg = registry();
        let mut doc = GraphDoc::default();
        doc.nodes = vec![node(0, "test_add")];
        for id in 1..=3u64 {
            doc.nodes.push(NodeInst {
                id,
                type_id: REROUTE_TYPE_ID.to_string(),
                type_version: 1,
                position: [0.0, 0.0],
                properties: BTreeMap::new(),
                subgraph: None,
                tint: None, title: None,
            });
        }
        // Unwired: untyped, and that is not an error.
        assert_eq!(reroute_type(&doc, &reg, 1), None);
        assert!(validate_doc(&doc, &reg).is_empty());

        // sum -> r1 -> r2 -> r3
        doc.edges = vec![
            Edge { from_node: 0, from_pin: "sum".into(), to_node: 1, to_pin: REROUTE_IN.into() },
            Edge {
                from_node: 1,
                from_pin: REROUTE_OUT.into(),
                to_node: 2,
                to_pin: REROUTE_IN.into(),
            },
            Edge {
                from_node: 2,
                from_pin: REROUTE_OUT.into(),
                to_node: 3,
                to_pin: REROUTE_IN.into(),
            },
        ];
        assert_eq!(reroute_type(&doc, &reg, 3), Some(PinType::Float));
        assert!(
            validate_doc(&doc, &reg).is_empty(),
            "a well-typed reroute chain is clean: {:?}",
            validate_doc(&doc, &reg)
        );

        // A cycle bails out instead of spinning forever.
        doc.edges.push(Edge {
            from_node: 3,
            from_pin: REROUTE_OUT.into(),
            to_node: 1,
            to_pin: REROUTE_IN.into(),
        });
        let _ = reroute_type(&doc, &reg, 1);
    }

    /// Wiring a reroute to a pin of a different type is a `TypeMismatch`,
    /// same as any other wire — the reroute is transparent, not permissive.
    /// The reported bug's other half: an empty reroute could not be connected
    /// to. Once it can, the type it adopts has to flow through immediately.
    #[test]
    fn an_empty_reroute_adopts_the_type_of_the_first_thing_wired_to_it() {
        let mut reg = NodeRegistry::new();
        reg.register(NodeDescriptor {
            id: "src".into(),
            name: "Src".into(),
            category: "Math".into(),
            version: 1,
            inputs: vec![],
            outputs: vec![PinDescriptor::new("out", "", PinType::Float)],
            pure: true,
            realm: NodeRealm::Shared,
            deterministic: true,
            doc: None,
            preview: None,
        })
        .unwrap();

        let mut doc = GraphDoc::default();
        doc.nodes.push(NodeInst {
            id: 1,
            type_id: "src".into(),
            type_version: 1,
            position: [0.0, 0.0],
            properties: Default::default(),
            subgraph: None,
            tint: None, title: None,
        });
        doc.nodes.push(NodeInst {
            id: 2,
            type_id: REROUTE_TYPE_ID.into(),
            type_version: 1,
            position: [200.0, 0.0],
            properties: Default::default(),
            subgraph: None,
            tint: None, title: None,
        });

        // Unwired: untyped, and that is not an error.
        assert_eq!(reroute_type(&doc, &reg, 2), None);
        assert!(validate_doc(&doc, &reg).is_empty(), "an empty reroute is legal");

        // Wire the Float in; the reroute adopts it.
        doc.edges.push(Edge {
            from_node: 1,
            from_pin: "out".into(),
            to_node: 2,
            to_pin: REROUTE_IN.into(),
        });
        assert_eq!(
            reroute_type(&doc, &reg, 2),
            Some(PinType::Float),
            "the type flows through the moment the first wire lands"
        );
        assert!(validate_doc(&doc, &reg).is_empty(), "{:?}", validate_doc(&doc, &reg));
    }

    #[test]
    fn reroute_still_type_checks_its_output() {
        let reg = registry();
        let mut doc = GraphDoc::default();
        doc.nodes = vec![node(0, "test_add"), node(1, "test_damage")];
        doc.nodes.push(NodeInst {
            id: 2,
            type_id: REROUTE_TYPE_ID.to_string(),
            type_version: 1,
            position: [0.0, 0.0],
            properties: BTreeMap::new(),
            subgraph: None,
            tint: None, title: None,
        });
        doc.edges = vec![
            Edge { from_node: 0, from_pin: "sum".into(), to_node: 2, to_pin: REROUTE_IN.into() },
            // Float out of the reroute into an exec pin.
            Edge {
                from_node: 2,
                from_pin: REROUTE_OUT.into(),
                to_node: 1,
                to_pin: "exec_in".into(),
            },
        ];
        let errs = validate_doc(&doc, &reg);
        assert!(
            errs.iter().any(|e| matches!(e, GraphError::TypeMismatch { .. })),
            "expected a mismatch through the reroute, got {errs:?}"
        );
    }

    /// A pin slug a reroute does not have is an `UnknownPin`, which the
    /// canvas turns into a ghost row.
    #[test]
    fn reroute_rejects_unknown_pin_slugs() {
        let reg = registry();
        let mut doc = GraphDoc::default();
        doc.nodes = vec![node(0, "test_add")];
        doc.nodes.push(NodeInst {
            id: 1,
            type_id: REROUTE_TYPE_ID.to_string(),
            type_version: 1,
            position: [0.0, 0.0],
            properties: BTreeMap::new(),
            subgraph: None,
            tint: None, title: None,
        });
        doc.edges = vec![Edge {
            from_node: 0,
            from_pin: "sum".into(),
            to_node: 1,
            to_pin: "nope".into(),
        }];
        let errs = validate_doc(&doc, &reg);
        assert!(
            errs.iter().any(|e| matches!(
                e,
                GraphError::UnknownPin { node: 1, pin, output: false } if pin == "nope"
            )),
            "got {errs:?}"
        );
    }

    fn node(id: u64, type_id: &str) -> NodeInst {
        NodeInst {
            id,
            type_id: type_id.to_string(),
            type_version: 1,
            position: [0.0, 0.0],
            properties: BTreeMap::new(),
            subgraph: None,
        
        tint: None, title: None,}
    }

    fn edge(from: u64, fp: &str, to: u64, tp: &str) -> Edge {
        Edge {
            from_node: from,
            from_pin: fp.to_string(),
            to_node: to,
            to_pin: tp.to_string(),
        }
    }

    fn registry() -> NodeRegistry {
        let mut reg = NodeRegistry::new();
        register_dev_nodes(&mut reg).unwrap();
        reg
    }

    #[test]
    fn valid_graph_passes() {
        let mut doc = GraphDoc::default();
        doc.nodes = vec![node(0, "test_event"), node(1, "test_damage")];
        doc.edges = vec![edge(0, "exec_out", 1, "exec_in")];
        assert_eq!(validate_doc(&doc, &registry()), vec![]);
    }

    #[test]
    fn type_mismatch_and_exec_rules() {
        let mut doc = GraphDoc::default();
        doc.nodes = vec![node(0, "test_damage"), node(1, "test_add")];
        // Exec output into a float input: exactly the exec-to-data violation.
        doc.edges = vec![edge(0, "exec_out", 1, "a")];
        let errs = validate_doc(&doc, &registry());
        assert!(matches!(errs.as_slice(), [GraphError::TypeMismatch { .. }]), "{errs:?}");
    }

    #[test]
    fn realm_violation_detected() {
        let mut doc = GraphDoc { realm: GraphRealm::Shared, ..GraphDoc::default() };
        doc.nodes = vec![node(0, "test_editor_note")];
        let errs = validate_doc(&doc, &registry());
        assert!(
            matches!(errs.as_slice(), [GraphError::RealmViolation { .. }]),
            "{errs:?}"
        );
        // Same node in an editor graph is fine.
        let doc = GraphDoc {
            realm: GraphRealm::Editor,
            nodes: doc.nodes.clone(),
            ..GraphDoc::default()
        };
        assert_eq!(validate_doc(&doc, &registry()), vec![]);
    }

    #[test]
    fn unknown_type_duplicate_id_dangling_edge() {
        let mut doc = GraphDoc::default();
        doc.nodes = vec![node(0, "no_such_type"), node(0, "test_event")];
        doc.edges = vec![edge(5, "x", 6, "y")];
        let errs = validate_doc(&doc, &registry());
        assert!(errs.iter().any(|e| matches!(e, GraphError::DuplicateNodeId(0))));
        assert!(errs.iter().any(|e| matches!(e, GraphError::UnknownNodeType { .. })));
        assert!(errs.iter().any(|e| matches!(e, GraphError::DanglingEdgeNode { .. })));
    }

    #[test]
    fn input_fan_in_rejected_output_fan_out_allowed() {
        let mut doc = GraphDoc::default();
        doc.nodes = vec![
            node(0, "test_add"),
            node(1, "test_add"),
            node(2, "test_add"),
        ];
        // One output feeding two inputs: fine.
        doc.edges = vec![edge(0, "sum", 1, "a"), edge(0, "sum", 2, "a")];
        assert_eq!(validate_doc(&doc, &registry()), vec![]);
        // Two outputs feeding one input: rejected.
        doc.edges = vec![edge(0, "sum", 2, "a"), edge(1, "sum", 2, "a")];
        let errs = validate_doc(&doc, &registry());
        assert!(
            matches!(errs.as_slice(), [GraphError::InputMultiplyConnected { .. }]),
            "{errs:?}"
        );
    }

    // -----------------------------------------------------------------
    // 45-A: interface binding, variables, data cycles
    // -----------------------------------------------------------------

    fn iface_pin(slug: &str, ty: PinType) -> crate::IfacePin {
        crate::IfacePin {
            slug: slug.to_string(),
            label: slug.to_string(),
            ty,
        }
    }

    /// A subgraph document whose interface is bound by the pair validates
    /// clean; one whose pins go nowhere reports a *warning* per unbound pin;
    /// and a binding node in a document with no interface is an error.
    #[test]
    fn interface_binding_pins_and_warnings() {
        let reg = registry();
        let mut doc = GraphDoc::default();
        doc.inputs = vec![iface_pin("amount", PinType::Float)];
        doc.outputs = vec![iface_pin("total", PinType::Float)];
        doc.nodes = vec![
            node(0, GRAPH_INPUT_TYPE_ID),
            node(1, "test_add"),
            node(2, GRAPH_OUTPUT_TYPE_ID),
        ];

        // Declared but unbound: two warnings, nothing else.
        let errs = validate_doc(&doc, &reg);
        assert_eq!(
            errs,
            vec![
                GraphError::InterfacePinUnbound { pin: "amount".into(), output: false },
                GraphError::InterfacePinUnbound { pin: "total".into(), output: true },
            ],
            "{errs:?}"
        );
        assert!(errs.iter().all(|e| e.severity() == ErrorSeverity::Warning));

        // Wired through: the binding node's pins mirror the interface, so
        // these edges type-check like any other.
        doc.edges = vec![edge(0, "amount", 1, "a"), edge(1, "sum", 2, "total")];
        assert_eq!(validate_doc(&doc, &reg), vec![], "a bound interface is clean");

        // …and the mirroring is *typed*: retyping the interface breaks the
        // wire it no longer matches.
        doc.inputs[0].ty = PinType::Int;
        let errs = validate_doc(&doc, &reg);
        assert!(
            errs.iter().any(|e| matches!(
                e,
                GraphError::TypeMismatch { from_ty: PinType::Int, to_ty: PinType::Float, .. }
            )),
            "{errs:?}"
        );

        // A binding node in a document that declares no interface has
        // nothing to mirror.
        let plain = GraphDoc {
            nodes: vec![node(0, GRAPH_INPUT_TYPE_ID), node(1, GRAPH_OUTPUT_TYPE_ID)],
            ..GraphDoc::default()
        };
        let errs = validate_doc(&plain, &reg);
        assert_eq!(
            errs,
            vec![
                GraphError::InterfaceNodeInvalid { node: 0, output: false },
                GraphError::InterfaceNodeInvalid { node: 1, output: true },
            ],
            "{errs:?}"
        );
    }

    /// Variable nodes take their pins from the declaration; one naming a
    /// variable the document no longer declares reports that, by name.
    #[test]
    fn variable_nodes_validate_against_the_declaration() {
        use crate::{VAR_GET_TYPE_ID, VAR_PROP, VAR_VALUE_PIN};

        let reg = registry();
        let mut doc = GraphDoc::default();
        doc.variables = vec![crate::VarDecl {
            slug: "score".into(),
            label: "Score".into(),
            ty: PinType::Float,
            default: Some(crate::PropValue::Float(0.0)),
        }];
        let mut get = node(0, VAR_GET_TYPE_ID);
        get.properties.insert(
            VAR_PROP.into(),
            crate::PropValue::Str("score".into()),
        );
        doc.nodes = vec![get, node(1, "test_add")];
        doc.edges = vec![edge(0, VAR_VALUE_PIN, 1, "a")];
        assert_eq!(validate_doc(&doc, &reg), vec![], "a declared variable wires like any pin");

        // Retype the variable and the same wire stops type-checking.
        doc.variables[0].ty = PinType::String;
        let errs = validate_doc(&doc, &reg);
        assert!(
            errs.iter().any(|e| matches!(e, GraphError::TypeMismatch { .. })),
            "{errs:?}"
        );

        // Delete it and the node reports the name the author typed.
        doc.variables.clear();
        let errs = validate_doc(&doc, &reg);
        assert!(
            errs.iter().any(|e| matches!(
                e,
                GraphError::UnknownVariable { node: 0, slug } if slug == "score"
            )),
            "{errs:?}"
        );
    }

    /// Data cycles are errors; exec cycles are not (they are loops).
    #[test]
    fn data_cycles_detected_exec_cycles_allowed() {
        let reg = registry();

        // add0.sum -> add1.a -> add0.a: a pure pull chain feeding itself.
        let mut doc = GraphDoc::default();
        doc.nodes = vec![node(0, "test_add"), node(1, "test_add")];
        doc.edges = vec![edge(0, "sum", 1, "a"), edge(1, "sum", 0, "a")];
        let errs = validate_doc(&doc, &reg);
        assert_eq!(
            errs.iter()
                .filter_map(|e| match e {
                    GraphError::DataCycle { nodes } => Some(nodes.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            vec![vec![0u64, 1]],
            "one cycle, reported once, rotated to its lowest id: {errs:?}"
        );

        // A self-edge is the degenerate case of the same rule.
        let mut doc = GraphDoc::default();
        doc.nodes = vec![node(0, "test_add")];
        doc.edges = vec![edge(0, "sum", 0, "a")];
        assert!(validate_doc(&doc, &reg)
            .iter()
            .any(|e| matches!(e, GraphError::DataCycle { .. })));

        // Exec wires may loop — that is what a loop node is.
        let mut doc = GraphDoc::default();
        doc.nodes = vec![node(0, "test_damage"), node(1, "test_damage")];
        doc.edges = vec![
            edge(0, "exec_out", 1, "exec_in"),
            edge(1, "exec_out", 0, "exec_in"),
        ];
        assert!(
            !validate_doc(&doc, &reg)
                .iter()
                .any(|e| matches!(e, GraphError::DataCycle { .. })),
            "an exec loop is legal; the interpreter's budget handles runaway"
        );

        // An impure node breaks a data cycle: its `hit_count` is a value it
        // stored when it fired, not something pulled from its inputs.
        let mut doc = GraphDoc::default();
        doc.nodes = vec![node(0, "test_damage"), node(1, "test_add")];
        doc.edges = vec![edge(0, "hit_count", 1, "a"), edge(1, "sum", 0, "dps")];
        assert!(
            !validate_doc(&doc, &reg)
                .iter()
                .any(|e| matches!(e, GraphError::DataCycle { .. })),
            "{:?}",
            validate_doc(&doc, &reg)
        );
    }

    /// The two indirections that used to be special cases: a cycle that runs
    /// through a reroute, and one that closes across a subgraph boundary.
    #[test]
    fn data_cycles_through_a_reroute_and_a_subgraph() {
        let reg = registry();

        // add.sum -> reroute -> add.a
        let mut doc = GraphDoc::default();
        doc.nodes = vec![node(0, "test_add"), node(1, REROUTE_TYPE_ID)];
        doc.edges = vec![
            edge(0, "sum", 1, REROUTE_IN),
            edge(1, REROUTE_OUT, 0, "a"),
        ];
        let errs = validate_doc(&doc, &reg);
        assert!(
            errs.iter().any(|e| matches!(
                e,
                GraphError::DataCycle { nodes } if nodes.len() == 2 && nodes[0] == 0
            )),
            "a reroute is transparent, so the cycle runs through it: {errs:?}"
        );

        // sub.result -> add.a -> sub.amount, with the subgraph's pure
        // interface making it a pull-through node. Needs the resolver: with
        // no interface in reach the rule stays silent rather than guessing.
        let mut host = GraphDoc::default();
        host.nodes = vec![node(0, "test_add"), {
            let mut s = node(1, SUBGRAPH_TYPE_ID);
            s.subgraph = Some("lib/calc.subgraph".into());
            s
        }];
        host.edges = vec![edge(1, "result", 0, "a"), edge(0, "sum", 1, "amount")];
        assert!(
            !validate_doc(&host, &reg)
                .iter()
                .any(|e| matches!(e, GraphError::DataCycle { .. })),
            "without a resolver the boundary is opaque and nothing is claimed"
        );

        let mut docs = BTreeMap::new();
        docs.insert(
            "lib/calc.subgraph".to_string(),
            GraphDoc {
                inputs: vec![iface_pin("amount", PinType::Float)],
                outputs: vec![iface_pin("result", PinType::Float)],
                ..GraphDoc::default()
            },
        );
        let d = crate::DocDescriptors::with_resolver(&host, &reg, &docs);
        let errs = validate_doc_with(&d);
        assert!(
            errs.iter().any(|e| matches!(
                e,
                GraphError::DataCycle { nodes } if nodes == &vec![0u64, 1]
            )),
            "with the interface resolved the cycle closes across the boundary: {errs:?}"
        );

        // An interface carrying exec says the subgraph has side effects, so
        // its outputs are not pulled through and the cycle is broken.
        let mut docs = BTreeMap::new();
        docs.insert(
            "lib/calc.subgraph".to_string(),
            GraphDoc {
                inputs: vec![
                    iface_pin("amount", PinType::Float),
                    iface_pin("exec_in", PinType::Exec),
                ],
                outputs: vec![iface_pin("result", PinType::Float)],
                ..GraphDoc::default()
            },
        );
        let d = crate::DocDescriptors::with_resolver(&host, &reg, &docs);
        assert!(
            !validate_doc_with(&d)
                .iter()
                .any(|e| matches!(e, GraphError::DataCycle { .. })),
            "{:?}",
            validate_doc_with(&d)
        );
    }

    /// The curve rule has exactly one implementation: whatever
    /// `validate_doc_with` reports about a Timeline's `.curve`,
    /// [`validate_curves`] reports too — that is what lets the editor run the
    /// cross-asset half on its own frame budget without the two drifting.
    #[test]
    fn the_curve_rule_is_the_same_rule_both_ways() {
        use crate::doc::PropValue;
        use crate::registry::{CURVE_PROP, TIMELINE_TYPE_ID};
        use crate::std_nodes::register_std_nodes;

        let mut reg = NodeRegistry::new();
        register_std_nodes(&mut reg);
        let mut doc = GraphDoc::default();
        let mut n = NodeInst {
            id: 3,
            type_id: TIMELINE_TYPE_ID.into(),
            type_version: 1,
            position: [0.0, 0.0],
            properties: BTreeMap::new(),
            subgraph: None,
            tint: None,
            title: None,
        };
        n.properties
            .insert(CURVE_PROP.into(), PropValue::Asset("curves/gone.curve".into()));
        doc.nodes = vec![n];

        // No resolver: the question was not asked, so neither path answers it.
        let bare = DocDescriptors::new(&doc, &reg);
        assert!(validate_curves(&bare).is_empty());
        assert!(!validate_doc_with(&bare)
            .iter()
            .any(|e| matches!(e, GraphError::MissingCurve { .. })));

        // A resolver that does not hold it: both paths say so, identically,
        // and both quote the path the author typed.
        let curves: BTreeMap<String, curve_asset::CurveDoc> = BTreeMap::new();
        let d = DocDescriptors::new(&doc, &reg).with_curves(&curves);
        let only = validate_curves(&d);
        assert!(
            matches!(only.as_slice(), [GraphError::MissingCurve { node: 3, path }]
                if path == "curves/gone.curve"),
            "{only:?}"
        );
        let full: Vec<_> = validate_doc_with(&d)
            .into_iter()
            .filter(|e| matches!(e, GraphError::MissingCurve { .. }))
            .collect();
        assert_eq!(full, only);

        // Resolved: silence from both.
        let mut curves = BTreeMap::new();
        curves.insert("curves/gone.curve".to_string(), curve_asset::CurveDoc::default());
        let d = DocDescriptors::new(&doc, &reg).with_curves(&curves);
        assert!(validate_curves(&d).is_empty());
        assert!(!validate_doc_with(&d)
            .iter()
            .any(|e| matches!(e, GraphError::MissingCurve { .. })));
    }

    /// `Array(Domain(..))` is still a domain pin: the registration check sees
    /// through the container rather than silently accepting it.
    #[test]
    fn domain_check_sees_through_arrays() {
        let mut reg = NodeRegistry::new();
        reg.register(NodeDescriptor {
            id: "arr".into(),
            name: "Arr".into(),
            category: "Math".into(),
            version: 1,
            inputs: vec![PinDescriptor::new(
                "xs",
                "Xs",
                PinType::Array(Box::new(PinType::Domain("shader".into()))),
            )],
            outputs: vec![PinDescriptor::new("out", "", PinType::Float)],
            pure: true,
            realm: NodeRealm::Shared,
            deterministic: true,
            doc: None,
            preview: None,
        })
        .unwrap();
        let doc = GraphDoc {
            nodes: vec![node(0, "arr")],
            ..GraphDoc::default()
        };
        assert!(validate_doc(&doc, &reg)
            .iter()
            .any(|e| matches!(e, GraphError::UnknownDomainPin { domain, .. } if domain == "shader")));

        reg.register_domain_pin("shader");
        assert_eq!(validate_doc(&doc, &reg), vec![]);
    }

    /// **Exec inputs may fan in; data inputs may not** (45-A P3 ruling).
    ///
    /// Convergence is the fundamental Blueprint pattern — Branch's True and
    /// False rejoining a shared tail — and it is unauthorable without this.
    /// Two *values* arriving at one pin still has no meaning, including when
    /// they arrive through reroute chains.
    #[test]
    fn exec_inputs_fan_in_data_inputs_do_not() {
        let reg = registry();

        // Two exec outputs into one exec input: legal.
        let mut doc = GraphDoc::default();
        doc.nodes = vec![node(0, "test_event"), node(1, "test_event"), node(2, "test_damage")];
        doc.edges = vec![edge(0, "exec_out", 2, "exec_in"), edge(1, "exec_out", 2, "exec_in")];
        assert_eq!(validate_doc(&doc, &reg), vec![], "exec convergence is legal");

        // Three, for good measure — the rule is not "exactly two".
        doc.nodes.push(node(3, "test_event"));
        doc.edges.push(edge(3, "exec_out", 2, "exec_in"));
        assert_eq!(validate_doc(&doc, &reg), vec![]);

        // Two data outputs into one data input: still refused.
        let mut doc = GraphDoc::default();
        doc.nodes = vec![node(0, "test_add"), node(1, "test_add"), node(2, "test_add")];
        doc.edges = vec![edge(0, "sum", 2, "a"), edge(1, "sum", 2, "a")];
        assert!(
            validate_doc(&doc, &reg)
                .iter()
                .any(|e| matches!(e, GraphError::InputMultiplyConnected { node: 2, pin } if pin == "a")),
            "{:?}",
            validate_doc(&doc, &reg)
        );

        // …and refused when the two values converge *through reroute chains*,
        // which is where a naive type check would lose track of them.
        let mut doc = GraphDoc::default();
        doc.nodes = vec![
            node(0, "test_add"),
            node(1, "test_add"),
            node(2, REROUTE_TYPE_ID),
            node(3, REROUTE_TYPE_ID),
            node(4, "test_add"),
        ];
        doc.edges = vec![
            edge(0, "sum", 2, REROUTE_IN),
            edge(1, "sum", 3, REROUTE_IN),
            edge(2, REROUTE_OUT, 4, "a"),
            edge(3, REROUTE_OUT, 4, "a"),
        ];
        assert!(
            validate_doc(&doc, &reg)
                .iter()
                .any(|e| matches!(e, GraphError::InputMultiplyConnected { node: 4, pin } if pin == "a")),
            "two values through reroutes are still two values: {:?}",
            validate_doc(&doc, &reg)
        );

        // Two *data* wires into one reroute's `in` is the same violation one
        // step earlier.
        let mut doc = GraphDoc::default();
        doc.nodes = vec![node(0, "test_add"), node(1, "test_add"), node(2, REROUTE_TYPE_ID)];
        doc.edges = vec![edge(0, "sum", 2, REROUTE_IN), edge(1, "sum", 2, REROUTE_IN)];
        assert!(validate_doc(&doc, &reg)
            .iter()
            .any(|e| matches!(e, GraphError::InputMultiplyConnected { node: 2, .. })));

        // An *untyped* reroute stays strict: refusing is the honest answer
        // when the type cannot be proven exec.
        let mut doc = GraphDoc::default();
        doc.nodes = vec![node(0, REROUTE_TYPE_ID), node(1, REROUTE_TYPE_ID), node(2, REROUTE_TYPE_ID)];
        doc.edges = vec![edge(0, REROUTE_OUT, 2, REROUTE_IN), edge(1, REROUTE_OUT, 2, REROUTE_IN)];
        assert!(validate_doc(&doc, &reg)
            .iter()
            .any(|e| matches!(e, GraphError::InputMultiplyConnected { node: 2, .. })));

        // Exec fan-in through a reroute that *is* provably exec: legal.
        let mut doc = GraphDoc::default();
        doc.nodes = vec![
            node(0, "test_event"),
            node(1, "test_event"),
            node(2, REROUTE_TYPE_ID),
            node(3, "test_damage"),
        ];
        doc.edges = vec![
            edge(0, "exec_out", 2, REROUTE_IN),
            edge(1, "exec_out", 2, REROUTE_IN),
            edge(2, REROUTE_OUT, 3, "exec_in"),
        ];
        assert_eq!(validate_doc(&doc, &reg), vec![], "the reroute infers Exec");
    }

    /// A variable write's exec input is doc-dependent — the exemption has to
    /// resolve its type through `DocDescriptors`, not through the registry.
    #[test]
    fn exec_fan_in_works_on_doc_dependent_nodes() {
        use crate::{VAR_SET_TYPE_ID, VAR_PROP};
        let reg = registry();
        let mut doc = GraphDoc::default();
        doc.variables = vec![crate::VarDecl {
            slug: "n".into(),
            label: "N".into(),
            ty: PinType::Float,
            default: Some(crate::PropValue::Float(0.0)),
        }];
        let mut set = node(2, VAR_SET_TYPE_ID);
        set.properties
            .insert(VAR_PROP.into(), crate::PropValue::Str("n".into()));
        doc.nodes = vec![node(0, "test_event"), node(1, "test_event"), set];
        doc.edges = vec![edge(0, "exec_out", 2, "exec_in"), edge(1, "exec_out", 2, "exec_in")];
        assert_eq!(validate_doc(&doc, &reg), vec![]);

        // Its *value* input still takes one wire.
        doc.nodes.push(node(3, "test_add"));
        doc.nodes.push(node(4, "test_add"));
        doc.edges.push(edge(3, "sum", 2, "value"));
        doc.edges.push(edge(4, "sum", 2, "value"));
        assert!(validate_doc(&doc, &reg)
            .iter()
            .any(|e| matches!(e, GraphError::InputMultiplyConnected { node: 2, pin } if pin == "value")));
    }

    #[test]
    fn unknown_pin_reported() {
        let mut doc = GraphDoc::default();
        doc.nodes = vec![node(0, "test_add"), node(1, "test_add")];
        doc.edges = vec![edge(0, "nope", 1, "a")];
        let errs = validate_doc(&doc, &registry());
        assert!(
            matches!(
                errs.as_slice(),
                [GraphError::UnknownPin { output: true, .. }]
            ),
            "{errs:?}"
        );
    }
}
