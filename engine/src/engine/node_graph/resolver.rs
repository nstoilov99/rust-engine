//! Cross-asset validation (plan D3 `validate_refs` + D8): subgraph
//! interface checks and reference-cycle detection through a [`GraphResolver`].
//!
//! The resolver abstracts "who can hand me a referenced doc": the editor's
//! implementation prefers open (possibly unsaved) documents over disk; tests
//! use a plain map. Paths are canonical content-relative with forward
//! slashes — the same key form used by editor tabs and hot-reload matching.

use std::collections::BTreeMap;

use super::doc::{GraphDoc, PinType};
use super::registry::{NodeRegistry, SUBGRAPH_TYPE_ID};
use super::validate::GraphError;

pub trait GraphResolver {
    fn resolve(&self, content_rel_path: &str) -> Option<&GraphDoc>;
}

impl GraphResolver for BTreeMap<String, GraphDoc> {
    fn resolve(&self, content_rel_path: &str) -> Option<&GraphDoc> {
        self.get(content_rel_path)
    }
}

/// Reverse index: which docs reference a given subgraph. Built by the caller
/// over whatever set of docs it knows (open editors + scanned assets); used
/// by hot-reload to refresh host graphs when a subgraph changes.
pub fn referencing_hosts<'a>(
    docs: impl Iterator<Item = (&'a str, &'a GraphDoc)>,
    subgraph_path: &str,
) -> Vec<String> {
    let mut hosts: Vec<String> = docs
        .filter(|(_, d)| d.subgraph_refs().contains(&subgraph_path))
        .map(|(p, _)| p.to_string())
        .collect();
    hosts.sort_unstable();
    hosts
}

/// Validate everything that needs other assets: subgraph references exist,
/// edges into/out of subgraph nodes match the referenced interface, and the
/// subgraph reference graph is a DAG. Run after `validate_doc`; the two
/// error lists concatenate.
pub fn validate_refs(
    doc: &GraphDoc,
    doc_path: &str,
    registry: &NodeRegistry,
    resolver: &dyn GraphResolver,
) -> Vec<GraphError> {
    let mut errors = Vec::new();

    // Resolve every subgraph node's interface.
    let mut ifaces: BTreeMap<u64, &GraphDoc> = BTreeMap::new();
    for n in &doc.nodes {
        let Some(path) = n.subgraph.as_deref() else { continue };
        match resolver.resolve(path) {
            Some(sub) => {
                ifaces.insert(n.id, sub);
            }
            None => errors.push(GraphError::MissingSubgraph {
                node: n.id,
                path: path.to_string(),
            }),
        }
    }

    // Edges touching subgraph nodes (validate_doc skipped them): pin
    // membership against the interface, then the same strict type equality.
    for e in &doc.edges {
        let from_node = doc.node(e.from_node);
        let to_node = doc.node(e.to_node);
        let (Some(from), Some(to)) = (from_node, to_node) else { continue };
        let from_is_sub = from.type_id == SUBGRAPH_TYPE_ID;
        let to_is_sub = to.type_id == SUBGRAPH_TYPE_ID;
        if !from_is_sub && !to_is_sub {
            continue; // fully covered by validate_doc
        }

        // A subgraph node's *outputs* come from the referenced doc's
        // `outputs`; its *inputs* from `inputs`.
        let from_ty: Option<PinType> = if from_is_sub {
            ifaces.get(&from.id).and_then(|sub| {
                match sub.outputs.iter().find(|p| p.slug == e.from_pin) {
                    Some(p) => Some(p.ty.clone()),
                    None => {
                        errors.push(GraphError::SubgraphPinUnknown {
                            node: from.id,
                            pin: e.from_pin.clone(),
                            output: true,
                        });
                        None
                    }
                }
            })
        } else {
            registry
                .get(&from.type_id)
                .and_then(|d| d.output(&e.from_pin))
                .map(|p| p.ty.clone())
        };
        let to_ty: Option<PinType> = if to_is_sub {
            ifaces.get(&to.id).and_then(|sub| {
                match sub.inputs.iter().find(|p| p.slug == e.to_pin) {
                    Some(p) => Some(p.ty.clone()),
                    None => {
                        errors.push(GraphError::SubgraphPinUnknown {
                            node: to.id,
                            pin: e.to_pin.clone(),
                            output: false,
                        });
                        None
                    }
                }
            })
        } else {
            registry
                .get(&to.type_id)
                .and_then(|d| d.input(&e.to_pin))
                .map(|p| p.ty.clone())
        };
        if let (Some(from_ty), Some(to_ty)) = (from_ty, to_ty) {
            if from_ty != to_ty {
                errors.push(GraphError::TypeMismatch { edge: e.clone(), from_ty, to_ty });
            }
        }
    }

    // Cycle detection: the subgraph reference graph must be a DAG.
    let mut stack = vec![doc_path.to_string()];
    detect_cycles(doc, resolver, &mut stack, &mut errors);

    errors
}

fn detect_cycles(
    doc: &GraphDoc,
    resolver: &dyn GraphResolver,
    stack: &mut Vec<String>,
    errors: &mut Vec<GraphError>,
) {
    for r in doc.subgraph_refs() {
        if let Some(pos) = stack.iter().position(|p| p == r) {
            let mut chain: Vec<String> = stack[pos..].to_vec();
            chain.push(r.to_string());
            // Report each cycle once, from its first-discovered entry point.
            if !errors
                .iter()
                .any(|e| matches!(e, GraphError::SubgraphCycle { chain: c } if c.contains(&r.to_string())))
            {
                errors.push(GraphError::SubgraphCycle { chain });
            }
            continue;
        }
        // Missing nested refs are the referenced asset's own problem —
        // surfaced when it is opened/validated; only traverse what resolves.
        if let Some(sub) = resolver.resolve(r) {
            stack.push(r.to_string());
            detect_cycles(sub, resolver, stack, errors);
            stack.pop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::node_graph::dev_nodes::register_dev_nodes;
    use crate::engine::node_graph::doc::{Edge, IfacePin, NodeInst};

    fn registry() -> NodeRegistry {
        let mut reg = NodeRegistry::new();
        register_dev_nodes(&mut reg).unwrap();
        reg
    }

    fn sub_node(id: u64, path: &str) -> NodeInst {
        NodeInst {
            id,
            type_id: SUBGRAPH_TYPE_ID.to_string(),
            type_version: 1,
            position: [0.0, 0.0],
            properties: Default::default(),
            subgraph: Some(path.to_string()),
        }
    }

    fn plain_node(id: u64, type_id: &str) -> NodeInst {
        NodeInst {
            id,
            type_id: type_id.to_string(),
            type_version: 1,
            position: [0.0, 0.0],
            properties: Default::default(),
            subgraph: None,
        }
    }

    fn calc_subgraph() -> GraphDoc {
        GraphDoc {
            inputs: vec![IfacePin {
                slug: "amount".into(),
                label: "Amount".into(),
                ty: PinType::Float,
            }],
            outputs: vec![IfacePin {
                slug: "result".into(),
                label: "Result".into(),
                ty: PinType::Float,
            }],
            ..GraphDoc::default()
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

    #[test]
    fn interface_match_and_missing_ref() {
        let mut host = GraphDoc::default();
        host.nodes = vec![plain_node(0, "test_add"), sub_node(1, "lib/calc.subgraph")];
        host.edges = vec![edge(0, "sum", 1, "amount")];

        let mut docs = BTreeMap::new();
        docs.insert("lib/calc.subgraph".to_string(), calc_subgraph());
        assert_eq!(
            validate_refs(&host, "main.graph", &registry(), &docs),
            vec![]
        );

        let empty: BTreeMap<String, GraphDoc> = BTreeMap::new();
        let errs = validate_refs(&host, "main.graph", &registry(), &empty);
        assert!(
            matches!(errs.as_slice(), [GraphError::MissingSubgraph { node: 1, .. }]),
            "{errs:?}"
        );
    }

    #[test]
    fn interface_drift_and_type_mismatch() {
        let mut host = GraphDoc::default();
        host.nodes = vec![plain_node(0, "test_add"), sub_node(1, "lib/calc.subgraph")];
        // "gone" no longer exists on the interface; "amount" exists but we
        // feed it an exec output in the second doc below.
        host.edges = vec![edge(0, "sum", 1, "gone")];
        let mut docs = BTreeMap::new();
        docs.insert("lib/calc.subgraph".to_string(), calc_subgraph());
        let errs = validate_refs(&host, "main.graph", &registry(), &docs);
        assert!(
            matches!(
                errs.as_slice(),
                [GraphError::SubgraphPinUnknown { node: 1, output: false, .. }]
            ),
            "{errs:?}"
        );

        let mut host2 = GraphDoc::default();
        host2.nodes = vec![plain_node(0, "test_event"), sub_node(1, "lib/calc.subgraph")];
        host2.edges = vec![edge(0, "exec_out", 1, "amount")];
        let errs = validate_refs(&host2, "main.graph", &registry(), &docs);
        assert!(
            matches!(errs.as_slice(), [GraphError::TypeMismatch { .. }]),
            "{errs:?}"
        );
    }

    #[test]
    fn cycles_detected() {
        // a -> b -> a, plus a direct self-reference c -> c.
        let mut a = GraphDoc::default();
        a.nodes = vec![sub_node(0, "b.subgraph")];
        let mut b = GraphDoc::default();
        b.nodes = vec![sub_node(0, "a.subgraph")];
        let mut c = GraphDoc::default();
        c.nodes = vec![sub_node(0, "c.subgraph")];

        let mut docs = BTreeMap::new();
        docs.insert("a.subgraph".to_string(), a.clone());
        docs.insert("b.subgraph".to_string(), b);
        docs.insert("c.subgraph".to_string(), c.clone());

        let errs = validate_refs(&a, "a.subgraph", &registry(), &docs);
        assert!(
            errs.iter().any(|e| matches!(e, GraphError::SubgraphCycle { .. })),
            "{errs:?}"
        );
        let errs = validate_refs(&c, "c.subgraph", &registry(), &docs);
        assert!(
            errs.iter().any(|e| matches!(e, GraphError::SubgraphCycle { chain } if chain.len() == 2)),
            "{errs:?}"
        );
    }

    #[test]
    fn reverse_index_finds_hosts() {
        let mut host = GraphDoc::default();
        host.nodes = vec![sub_node(0, "lib/calc.subgraph")];
        let other = GraphDoc::default();
        let docs = vec![
            ("main.graph", &host),
            ("other.graph", &other),
        ];
        assert_eq!(
            referencing_hosts(docs.into_iter(), "lib/calc.subgraph"),
            vec!["main.graph".to_string()]
        );
    }
}
