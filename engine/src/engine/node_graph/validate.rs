//! Doc-local graph validation (plan D3, `validate_doc` layer): pure over
//! `(doc, registry)`, no I/O. Cross-asset checks (subgraph interfaces,
//! reference cycles) live in the resolver layer (`validate_refs`, P6).
//!
//! Unknown node types are errors but never fatal: the doc still loads and
//! re-saves without data loss, so a disabled 39.8 plugin can't eat graphs.

use std::collections::{BTreeMap, BTreeSet};

use super::doc::{Edge, GraphDoc, GraphRealm, NodeRealm, PinType};
use super::registry::{NodeRegistry, REROUTE_IN, REROUTE_OUT, REROUTE_TYPE_ID, SUBGRAPH_TYPE_ID};

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
            GraphError::DanglingEdgeNode { .. } | GraphError::SubgraphCycle { .. } => {
                ErrorAnchor::Document
            }
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
        }
    }
}

/// The pin type flowing through a reroute chain, resolved by walking back to
/// the first real descriptor pin that feeds it. `None` when nothing upstream
/// is connected yet — an unwired reroute is untyped, not wrong.
///
/// The walk is depth-capped rather than visited-set tracked: a reroute cycle
/// is degenerate input, and bailing out is the right answer for a *type*
/// query. `DanglingEdgeNode` still reports the structural problem.
pub fn reroute_type(doc: &GraphDoc, registry: &NodeRegistry, node: u64) -> Option<PinType> {
    const MAX_HOPS: usize = 64;
    let mut at = node;
    for _ in 0..MAX_HOPS {
        let feed = doc
            .edges
            .iter()
            .find(|e| e.to_node == at && e.to_pin == REROUTE_IN)?;
        let src = doc.node(feed.from_node)?;
        if src.type_id == REROUTE_TYPE_ID {
            at = src.id;
            continue;
        }
        if src.type_id == SUBGRAPH_TYPE_ID {
            // The interface lives in another asset; the resolver layer owns it.
            return None;
        }
        return registry
            .get(&src.type_id)
            .and_then(|d| d.output(&feed.from_pin))
            .map(|p| p.ty.clone());
    }
    None
}

/// The type on one end of an edge, seeing through reroutes.
fn reroute_edge_type(
    doc: &GraphDoc,
    registry: &NodeRegistry,
    e: &Edge,
    source_side: bool,
) -> Option<PinType> {
    let (id, pin) = if source_side {
        (e.from_node, &e.from_pin)
    } else {
        (e.to_node, &e.to_pin)
    };
    endpoint_type(doc, registry, id, pin, source_side)
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
    let n = doc.node(node)?;
    if n.type_id == REROUTE_TYPE_ID {
        return reroute_type(doc, registry, node);
    }
    if n.type_id == SUBGRAPH_TYPE_ID {
        return None;
    }
    let d = registry.get(&n.type_id)?;
    if output { d.output(pin) } else { d.input(pin) }.map(|p| p.ty.clone())
}

/// Validate everything checkable without loading other assets. Subgraph
/// nodes (reserved type, pins derived from the referenced asset) are only
/// checked for duplicate ids here — their pins and cycles are `validate_refs`
/// territory, and edges touching them are skipped rather than misreported.
pub fn validate_doc(doc: &GraphDoc, registry: &NodeRegistry) -> Vec<GraphError> {
    let mut errors = Vec::new();

    // Node ids + per-node checks.
    let mut ids = BTreeSet::new();
    for n in &doc.nodes {
        if !ids.insert(n.id) {
            errors.push(GraphError::DuplicateNodeId(n.id));
        }
        if n.type_id == SUBGRAPH_TYPE_ID || n.type_id == REROUTE_TYPE_ID {
            continue;
        }
        match registry.get(&n.type_id) {
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
                    if let PinType::Domain(d) = &p.ty {
                        if !registry.domain_pin_registered(d) {
                            errors.push(GraphError::UnknownDomainPin {
                                node: n.id,
                                pin: p.slug.clone(),
                                domain: d.clone(),
                            });
                        }
                    }
                }
            }
        }
    }

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
            let a = reroute_edge_type(doc, registry, e, true);
            let b = reroute_edge_type(doc, registry, e, false);
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

        let from_desc = (from.type_id != SUBGRAPH_TYPE_ID)
            .then(|| registry.get(&from.type_id))
            .flatten();
        let to_desc = (to.type_id != SUBGRAPH_TYPE_ID)
            .then(|| registry.get(&to.type_id))
            .flatten();

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
        // A reroute is explicitly one-in, many-out; its `in` still takes one.

        if count > 1 {
            errors.push(GraphError::InputMultiplyConnected {
                node,
                pin: pin.to_string(),
            });
        }
    }

    errors
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::node_graph::dev_nodes::register_dev_nodes;
    use crate::engine::node_graph::doc::NodeInst;
    use crate::engine::node_graph::registry::{NodeDescriptor, PinDescriptor};
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
        ];
        // The closed set is eleven (recorded ruling); the UI is complete
        // because the set is.
        assert_eq!(all.len(), 11);

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

        // Only the cycle carries a breadcrumb.
        assert_eq!(all[10].cycle_chain().map(|c| c.len()), Some(2));
        assert!(all[0].cycle_chain().is_none());
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
                tint: None,
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
            tint: None,
        });
        doc.nodes.push(NodeInst {
            id: 2,
            type_id: REROUTE_TYPE_ID.into(),
            type_version: 1,
            position: [200.0, 0.0],
            properties: Default::default(),
            subgraph: None,
            tint: None,
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
            tint: None,
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
            tint: None,
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
        
        tint: None,}
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
