//! Doc-local graph validation (plan D3, `validate_doc` layer): pure over
//! `(doc, registry)`, no I/O. Cross-asset checks (subgraph interfaces,
//! reference cycles) live in the resolver layer (`validate_refs`, P6).
//!
//! Unknown node types are errors but never fatal: the doc still loads and
//! re-saves without data loss, so a disabled 39.8 plugin can't eat graphs.

use std::collections::{BTreeMap, BTreeSet};

use super::doc::{Edge, GraphDoc, GraphRealm, NodeRealm, PinType};
use super::registry::{NodeRegistry, SUBGRAPH_TYPE_ID};

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
        if n.type_id == SUBGRAPH_TYPE_ID {
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
    use std::collections::BTreeMap;

    fn node(id: u64, type_id: &str) -> NodeInst {
        NodeInst {
            id,
            type_id: type_id.to_string(),
            type_version: 1,
            position: [0.0, 0.0],
            properties: BTreeMap::new(),
            subgraph: None,
        }
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
