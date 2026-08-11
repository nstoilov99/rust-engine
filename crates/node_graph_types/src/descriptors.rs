//! `DocDescriptors` — the descriptor resolver layered over the registry
//! (Task 45-A D3, amended by the audit addendum's item 1).
//!
//! Task 40 answered "what are this node's pins" with `registry.get(type_id)`
//! and special-cased the exceptions at each call site: `validate.rs` knew
//! about subgraphs *and* dynamically-typed reroutes, `resolver.rs` knew about
//! subgraph interfaces again, and the editor knew about all of it a third
//! time. 45-A adds three more document-dependent types (variables, interface
//! binding, custom-event payloads), so the special-casing generalizes into
//! one place: **every instance-pin question goes through this resolver**, and
//! validation, the editor, migration and (from P2) compilation consume it.
//!
//! Two levels, deliberately:
//!
//! - [`DocDescriptors::descriptor`] — the node's *static shape*, as a
//!   [`NodeDescriptor`] (borrowed from the registry, or synthesized from the
//!   document). `None` means nothing can answer: an unknown type, a variable
//!   node naming a deleted variable, an unresolvable subgraph — **or a
//!   reroute, which has no descriptor by design**.
//! - [`DocDescriptors::pin_type`] — the *type* of one pin, which additionally
//!   knows how to infer a reroute's type from what feeds it. `None` here has
//!   the same meaning it always had: "cannot be determined", never "the types
//!   differ".
//!
//! A resolver is optional. Without one, subgraph nodes answer `None` exactly
//! as they did before (their pins are `validate_refs` territory); with one,
//! the same call answers from the referenced asset's interface.

use std::borrow::Cow;

use crate::doc::{GraphDoc, NodeRealm, PinType, PropValue, VarDecl};
use crate::registry::{
    NodeDescriptor, NodeRegistry, PinDescriptor, EXEC_IN_PIN, EXEC_OUT_PIN, GRAPH_INPUT_TYPE_ID,
    GRAPH_OUTPUT_TYPE_ID, REROUTE_IN, REROUTE_TYPE_ID, SUBGRAPH_TYPE_ID, VAR_GET_TYPE_ID,
    VAR_PROP, VAR_SET_TYPE_ID, VAR_VALUE_PIN,
};
use crate::resolver::GraphResolver;
use crate::std_events::{
    payload_pins, EVENT_ACTION_PROP, EVENT_CUSTOM_TYPE_ID, EVENT_INPUT_ACTION_TYPE_ID,
    EVENT_NAME_PROP,
};

/// Where a node instance's pins come from. One variant per answer path, so
/// adding a document-dependent type forces a decision here rather than a new
/// special case at some call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    /// A plain registry descriptor.
    Registered,
    /// Pins from the referenced `.subgraph` asset's declared interface.
    Subgraph,
    /// One in, one out; type inferred from whatever is wired to it. No
    /// descriptor exists, by design.
    Reroute,
    /// Outputs mirror the document's declared `inputs`.
    GraphInput,
    /// Inputs mirror the document's declared `outputs`.
    GraphOutput,
    /// Pure read of a document variable.
    VarGet,
    /// Impure write of a document variable.
    VarSet,
    /// Registry base descriptor plus payload pins from the instance's
    /// properties.
    EventCustom,
}

impl NodeKind {
    /// How this instance's pins are answered, from its type slug alone.
    pub fn of_type(type_id: &str) -> NodeKind {
        match type_id {
            SUBGRAPH_TYPE_ID => NodeKind::Subgraph,
            REROUTE_TYPE_ID => NodeKind::Reroute,
            GRAPH_INPUT_TYPE_ID => NodeKind::GraphInput,
            GRAPH_OUTPUT_TYPE_ID => NodeKind::GraphOutput,
            VAR_GET_TYPE_ID => NodeKind::VarGet,
            VAR_SET_TYPE_ID => NodeKind::VarSet,
            EVENT_CUSTOM_TYPE_ID => NodeKind::EventCustom,
            _ => NodeKind::Registered,
        }
    }

    /// Interface binding nodes are only meaningful inside a document that
    /// declares an interface.
    pub fn is_interface(self) -> bool {
        matches!(self, NodeKind::GraphInput | NodeKind::GraphOutput)
    }

    /// Variable access nodes name a `VarDecl` through
    /// [`VAR_PROP`](crate::registry::VAR_PROP).
    pub fn is_variable(self) -> bool {
        matches!(self, NodeKind::VarGet | NodeKind::VarSet)
    }
}

/// A descriptor resolver bound to one document.
pub struct DocDescriptors<'a> {
    doc: &'a GraphDoc,
    registry: &'a NodeRegistry,
    resolver: Option<&'a dyn GraphResolver>,
}

impl<'a> DocDescriptors<'a> {
    /// Doc-local answers only: subgraph nodes resolve to `None`, exactly as
    /// `validate_doc` has always treated them.
    pub fn new(doc: &'a GraphDoc, registry: &'a NodeRegistry) -> Self {
        Self { doc, registry, resolver: None }
    }

    /// …plus cross-asset answers: subgraph nodes resolve their interface.
    pub fn with_resolver(
        doc: &'a GraphDoc,
        registry: &'a NodeRegistry,
        resolver: &'a dyn GraphResolver,
    ) -> Self {
        Self { doc, registry, resolver: Some(resolver) }
    }

    pub fn doc(&self) -> &'a GraphDoc {
        self.doc
    }

    pub fn registry(&self) -> &'a NodeRegistry {
        self.registry
    }

    /// How this instance's pins are answered.
    pub fn kind(&self, node_id: u64) -> Option<NodeKind> {
        self.doc.node(node_id).map(|n| NodeKind::of_type(&n.type_id))
    }

    /// The variable a `var_get`/`var_set` instance names, if the document
    /// still declares it.
    pub fn variable_of(&self, node_id: u64) -> Option<&'a VarDecl> {
        let n = self.doc.node(node_id)?;
        let slug = match n.properties.get(VAR_PROP) {
            Some(PropValue::Str(s)) => s.as_str(),
            _ => return None,
        };
        self.doc.variable(slug)
    }

    /// The slug a `var_get`/`var_set` instance names, declared or not — so a
    /// dangling reference can be *reported* with the name the author typed.
    pub fn variable_slug(&self, node_id: u64) -> Option<&'a str> {
        match self.doc.node(node_id)?.properties.get(VAR_PROP) {
            Some(PropValue::Str(s)) => Some(s.as_str()),
            _ => None,
        }
    }

    /// This instance's static shape. See the module docs for what `None`
    /// means — in particular, a reroute always answers `None`.
    pub fn descriptor(&self, node_id: u64) -> Option<Cow<'a, NodeDescriptor>> {
        let n = self.doc.node(node_id)?;
        match NodeKind::of_type(&n.type_id) {
            NodeKind::Registered => self.registry.get(&n.type_id).map(Cow::Borrowed),
            NodeKind::Reroute => None,
            NodeKind::Subgraph => {
                let path = n.subgraph.as_deref()?;
                let sub = self.resolver?.resolve(path)?;
                Some(Cow::Owned(subgraph_descriptor(path, sub)))
            }
            NodeKind::GraphInput => Some(Cow::Owned(interface_descriptor(self.doc, true))),
            NodeKind::GraphOutput => Some(Cow::Owned(interface_descriptor(self.doc, false))),
            NodeKind::VarGet => Some(Cow::Owned(var_descriptor(self.variable_of(node_id)?, false))),
            NodeKind::VarSet => Some(Cow::Owned(var_descriptor(self.variable_of(node_id)?, true))),
            NodeKind::EventCustom => {
                let mut base = self.registry.get(&n.type_id)?.clone();
                base.outputs.extend(payload_pins(&n.properties));
                Some(Cow::Owned(base))
            }
        }
    }

    /// What this node instance calls itself on the canvas.
    ///
    /// Doc-dependent types name themselves from their *configuration*, which
    /// is what keeps a variable node readable without inventing new node
    /// anatomy: `var_get` reads "Get Score", `event_custom` reads
    /// "Event: Hit". Precedence is author over synthesis over descriptor —
    /// an explicit `NodeInst::title` always wins, because someone typed it.
    ///
    /// A dangling reference still gets a name (`Get <missing>`): a node that
    /// renders as the bare slug `var_get` tells the author nothing about what
    /// broke, and validation is already reporting the real error.
    pub fn display_name(&self, node_id: u64) -> Option<String> {
        let n = self.doc.node(node_id)?;
        if let Some(t) = n.title.as_ref().filter(|t| !t.trim().is_empty()) {
            return Some(t.clone());
        }
        let config = |key: &str| match n.properties.get(key) {
            Some(PropValue::Str(s)) | Some(PropValue::Enum(s)) if !s.is_empty() => {
                Some(s.clone())
            }
            _ => None,
        };
        let name = match NodeKind::of_type(&n.type_id) {
            NodeKind::VarGet | NodeKind::VarSet => {
                let verb = if n.type_id == VAR_SET_TYPE_ID { "Set" } else { "Get" };
                let what = self
                    .variable_of(node_id)
                    .map(|d| d.label.clone())
                    .or_else(|| self.variable_slug(node_id).map(|s| s.to_string()))
                    .unwrap_or_else(|| "<missing>".to_string());
                format!("{verb} {what}")
            }
            NodeKind::EventCustom => {
                format!("Event: {}", config(EVENT_NAME_PROP).unwrap_or_else(|| "<unnamed>".into()))
            }
            _ if n.type_id == EVENT_INPUT_ACTION_TYPE_ID => {
                format!("Input: {}", config(EVENT_ACTION_PROP).unwrap_or_else(|| "<unbound>".into()))
            }
            // Everything else is named by its descriptor, which the caller
            // already has.
            _ => return self.descriptor(node_id).map(|d| d.name.clone()),
        };
        Some(name)
    }

    /// The declared type of one pin, seeing through reroutes. Generalizes
    /// Task 40's `endpoint_type`, which this now backs.
    pub fn pin_type(&self, node_id: u64, pin: &str, output: bool) -> Option<PinType> {
        let n = self.doc.node(node_id)?;
        if n.type_id == REROUTE_TYPE_ID {
            return self.reroute_type(node_id);
        }
        let d = self.descriptor(node_id)?;
        if output { d.output(pin) } else { d.input(pin) }.map(|p| p.ty.clone())
    }

    /// The pin type flowing through a reroute chain, resolved by walking back
    /// to the first real descriptor pin that feeds it. `None` when nothing
    /// upstream is connected yet — an unwired reroute is untyped, not wrong.
    ///
    /// The walk is depth-capped rather than visited-set tracked: a reroute
    /// cycle is degenerate input, and bailing out is the right answer for a
    /// *type* query. `DanglingEdgeNode` still reports the structural problem.
    pub fn reroute_type(&self, node_id: u64) -> Option<PinType> {
        const MAX_HOPS: usize = 64;
        let mut at = node_id;
        for _ in 0..MAX_HOPS {
            let feed = self
                .doc
                .edges
                .iter()
                .find(|e| e.to_node == at && e.to_pin == REROUTE_IN)?;
            let src = self.doc.node(feed.from_node)?;
            if src.type_id == REROUTE_TYPE_ID {
                at = src.id;
                continue;
            }
            return self
                .descriptor(src.id)?
                .output(&feed.from_pin)
                .map(|p| p.ty.clone());
        }
        None
    }

    /// Does this instance pull its data outputs from its data inputs? Pure
    /// nodes and pass-throughs do; an impure node's outputs are values it
    /// *stored* when it fired, which is what breaks a data cycle.
    ///
    /// `None` = unresolvable, which the data-cycle rule treats as "do not
    /// guess" rather than as either answer.
    pub fn pulls_through(&self, node_id: u64) -> Option<bool> {
        match self.kind(node_id)? {
            // Transparent by definition.
            NodeKind::Reroute => Some(true),
            _ => Some(self.descriptor(node_id)?.pure),
        }
    }
}

/// A subgraph node's descriptor, synthesized from the referenced asset's
/// declared interface. Purity follows the interface: an interface carrying
/// exec pins is a statement that the subgraph has side effects.
fn subgraph_descriptor(path: &str, sub: &GraphDoc) -> NodeDescriptor {
    let pins = |v: &[crate::doc::IfacePin]| {
        v.iter()
            .map(|p| PinDescriptor::new(&p.slug, &p.label, p.ty.clone()))
            .collect::<Vec<_>>()
    };
    let inputs = pins(&sub.inputs);
    let outputs = pins(&sub.outputs);
    let has_exec = inputs
        .iter()
        .chain(outputs.iter())
        .any(|p| p.ty == PinType::Exec);
    NodeDescriptor {
        id: SUBGRAPH_TYPE_ID.to_string(),
        name: std::path::Path::new(path)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "Subgraph".to_string()),
        category: "Subgraph".to_string(),
        version: 1,
        inputs,
        outputs,
        pure: !has_exec,
        realm: sub.realm_as_node(),
        deterministic: true,
        doc: None,
        preview: None,
    }
}

/// A `graph_input` / `graph_output` node's descriptor: the document's own
/// interface, mirrored. The input node *produces* the declared inputs; the
/// output node *consumes* the declared outputs.
///
/// A document with no matching interface yields a pin-less descriptor rather
/// than `None`: the node exists, it is just not valid here, and validation
/// says so once (`InterfaceNodeInvalid`) instead of the edges saying it again.
fn interface_descriptor(doc: &GraphDoc, is_input: bool) -> NodeDescriptor {
    let iface = if is_input { &doc.inputs } else { &doc.outputs };
    let pins: Vec<PinDescriptor> = iface
        .iter()
        .map(|p| PinDescriptor::new(&p.slug, &p.label, p.ty.clone()))
        .collect();
    let has_exec = pins.iter().any(|p| p.ty == PinType::Exec);
    let (inputs, outputs) = if is_input {
        (Vec::new(), pins)
    } else {
        (pins, Vec::new())
    };
    NodeDescriptor {
        id: if is_input { GRAPH_INPUT_TYPE_ID } else { GRAPH_OUTPUT_TYPE_ID }.to_string(),
        name: if is_input { "Graph Input" } else { "Graph Output" }.to_string(),
        category: "Interface".to_string(),
        version: 1,
        inputs,
        outputs,
        pure: !has_exec,
        realm: NodeRealm::Shared,
        deterministic: true,
        doc: None,
        preview: None,
    }
}

/// A `var_get` / `var_set` descriptor synthesized from a [`VarDecl`].
fn var_descriptor(decl: &VarDecl, set: bool) -> NodeDescriptor {
    let value = || {
        let p = PinDescriptor::new(VAR_VALUE_PIN, &decl.label, decl.ty.clone());
        match &decl.default {
            Some(d) if set => p.with_default(d.clone()),
            _ => p,
        }
    };
    NodeDescriptor {
        id: if set { VAR_SET_TYPE_ID } else { VAR_GET_TYPE_ID }.to_string(),
        name: format!("{} {}", if set { "Set" } else { "Get" }, decl.label),
        category: "Data".to_string(),
        version: 1,
        inputs: if set {
            vec![
                PinDescriptor::new(EXEC_IN_PIN, "Exec", PinType::Exec),
                value(),
            ]
        } else {
            vec![]
        },
        outputs: if set {
            vec![PinDescriptor::new(EXEC_OUT_PIN, "Exec", PinType::Exec)]
        } else {
            vec![value()]
        },
        // A read is a pure pull; a write is a statement in the exec flow.
        pure: !set,
        realm: NodeRealm::Shared,
        deterministic: true,
        doc: None,
        preview: None,
    }
}

impl GraphDoc {
    /// The node realm a document's own realm implies when the document is
    /// used *as* a node (a subgraph instance). A `Shared` graph is usable
    /// anywhere; anything else admits only its own realm.
    fn realm_as_node(&self) -> NodeRealm {
        match self.realm {
            crate::doc::GraphRealm::Editor => NodeRealm::Editor,
            crate::doc::GraphRealm::Client => NodeRealm::Client,
            crate::doc::GraphRealm::Server => NodeRealm::ServerSafe,
            crate::doc::GraphRealm::Shared => NodeRealm::Shared,
        }
    }
}

#[cfg(test)]
// Tests build documents the way an author does: start from the default and
// fill in what matters. One giant struct literal per fixture would satisfy
// clippy and read markedly worse.
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use crate::dev_nodes::register_dev_nodes;
    use crate::doc::{Edge, IfacePin, NodeInst};
    use crate::std_events::{register_std_events, EVENT_PAYLOAD_PREFIX};
    use crate::validate::{endpoint_type, reroute_type};
    use std::collections::BTreeMap;

    fn registry() -> NodeRegistry {
        let mut reg = NodeRegistry::new();
        register_dev_nodes(&mut reg).unwrap();
        register_std_events(&mut reg).unwrap();
        reg
    }

    pub(crate) fn node(id: u64, type_id: &str) -> NodeInst {
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

    fn edge(from: u64, fp: &str, to: u64, tp: &str) -> Edge {
        Edge {
            from_node: from,
            from_pin: fp.to_string(),
            to_node: to,
            to_pin: tp.to_string(),
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

    /// **The parity test** (written against the pre-migration
    /// implementations, per the 45-A P1 sequence): the resolver's answers for
    /// the two special cases Task 40 hand-coded — subgraph nodes and
    /// dynamically-typed reroutes — are identical to what `endpoint_type` /
    /// `reroute_type` returned before validation was migrated onto it.
    #[test]
    fn doc_descriptors_match_the_old_special_cases() {
        let reg = registry();
        let mut doc = GraphDoc::default();
        doc.nodes = vec![
            node(0, "test_add"),
            node(1, REROUTE_TYPE_ID),
            node(2, REROUTE_TYPE_ID),
            node(3, "no_such_type"),
        ];
        let mut sub = node(4, SUBGRAPH_TYPE_ID);
        sub.subgraph = Some("lib/calc.subgraph".into());
        doc.nodes.push(sub);
        doc.edges = vec![
            edge(0, "sum", 1, REROUTE_IN),
            edge(1, "out", 2, REROUTE_IN),
        ];

        let d = DocDescriptors::new(&doc, &reg);
        let probes: &[(u64, &str, bool)] = &[
            (0, "sum", true),
            (0, "a", false),
            (0, "nope", true),
            (1, "out", true),
            (2, "out", true),
            (2, "in", false),
            (3, "whatever", true),
            (4, "amount", false),
            (4, "result", true),
            (9, "missing_node", true),
        ];
        for (n, pin, out) in probes.iter().copied() {
            assert_eq!(
                d.pin_type(n, pin, out),
                endpoint_type(&doc, &reg, n, pin, out),
                "node {n} pin '{pin}' (output={out})"
            );
        }
        for n in [1u64, 2, 3] {
            assert_eq!(d.reroute_type(n), reroute_type(&doc, &reg, n), "reroute {n}");
        }

        // An unwired reroute is untyped, not wrong — both agree.
        let bare = GraphDoc {
            nodes: vec![node(0, REROUTE_TYPE_ID)],
            ..GraphDoc::default()
        };
        assert_eq!(DocDescriptors::new(&bare, &reg).reroute_type(0), None);
        assert_eq!(reroute_type(&bare, &reg, 0), None);

        // With a resolver, the subgraph node answers from the referenced
        // asset's interface — the same answer `validate_refs` computes by
        // hand from `sub.inputs` / `sub.outputs`.
        let mut docs = BTreeMap::new();
        docs.insert("lib/calc.subgraph".to_string(), calc_subgraph());
        let d = DocDescriptors::with_resolver(&doc, &reg, &docs);
        assert_eq!(d.pin_type(4, "amount", false), Some(PinType::Float));
        assert_eq!(d.pin_type(4, "result", true), Some(PinType::Float));
        assert_eq!(d.pin_type(4, "gone", false), None);
        assert_eq!(d.descriptor(4).unwrap().name, "calc");
        // A pure interface makes a pure subgraph node.
        assert!(d.descriptor(4).unwrap().pure);
    }

    /// A reroute has no descriptor by design; every other resolvable kind
    /// has one.
    #[test]
    fn reroute_has_no_descriptor_but_still_has_a_type() {
        let reg = registry();
        let mut doc = GraphDoc::default();
        doc.nodes = vec![node(0, "test_add"), node(1, REROUTE_TYPE_ID)];
        doc.edges = vec![edge(0, "sum", 1, REROUTE_IN)];
        let d = DocDescriptors::new(&doc, &reg);
        assert_eq!(d.kind(1), Some(NodeKind::Reroute));
        assert!(d.descriptor(1).is_none());
        assert_eq!(d.pin_type(1, "out", true), Some(PinType::Float));
        assert_eq!(d.pulls_through(1), Some(true));
    }

    /// `graph_input` / `graph_output` mirror the document's interface: the
    /// input node produces the declared inputs, the output node consumes the
    /// declared outputs.
    #[test]
    fn interface_nodes_mirror_the_declared_interface() {
        let reg = registry();
        let mut doc = calc_subgraph();
        doc.nodes = vec![node(0, GRAPH_INPUT_TYPE_ID), node(1, GRAPH_OUTPUT_TYPE_ID)];
        let d = DocDescriptors::new(&doc, &reg);

        let gi = d.descriptor(0).unwrap();
        assert!(gi.inputs.is_empty(), "an interface input node has no inputs");
        assert_eq!(
            gi.outputs.iter().map(|p| p.slug.as_str()).collect::<Vec<_>>(),
            vec!["amount"]
        );
        assert_eq!(d.pin_type(0, "amount", true), Some(PinType::Float));

        let go = d.descriptor(1).unwrap();
        assert!(go.outputs.is_empty());
        assert_eq!(
            go.inputs.iter().map(|p| p.slug.as_str()).collect::<Vec<_>>(),
            vec!["result"]
        );
        assert_eq!(d.pin_type(1, "result", false), Some(PinType::Float));

        // Mirroring is live: retyping the interface retypes the node.
        doc.inputs[0].ty = PinType::Int;
        let d = DocDescriptors::new(&doc, &reg);
        assert_eq!(d.pin_type(0, "amount", true), Some(PinType::Int));

        // In a document with no interface the node exists but has no pins.
        let plain = GraphDoc {
            nodes: vec![node(0, GRAPH_INPUT_TYPE_ID)],
            ..GraphDoc::default()
        };
        let d = DocDescriptors::new(&plain, &reg);
        assert!(d.descriptor(0).unwrap().outputs.is_empty());
    }

    /// Variable nodes synthesize their pins from the `VarDecl` they name.
    #[test]
    fn var_get_set_pins_come_from_the_declaration() {
        let reg = registry();
        let mut doc = GraphDoc::default();
        doc.variables = vec![VarDecl {
            slug: "score".into(),
            label: "Score".into(),
            ty: PinType::Int,
            default: Some(PropValue::Int(3)),
        }];
        let mut get = node(0, VAR_GET_TYPE_ID);
        get.properties
            .insert(VAR_PROP.into(), PropValue::Str("score".into()));
        let mut set = node(1, VAR_SET_TYPE_ID);
        set.properties
            .insert(VAR_PROP.into(), PropValue::Str("score".into()));
        let mut dangling = node(2, VAR_GET_TYPE_ID);
        dangling
            .properties
            .insert(VAR_PROP.into(), PropValue::Str("deleted".into()));
        doc.nodes = vec![get, set, dangling, node(3, VAR_GET_TYPE_ID)];

        let d = DocDescriptors::new(&doc, &reg);
        let g = d.descriptor(0).unwrap();
        assert_eq!(g.name, "Get Score");
        assert!(g.pure && g.inputs.is_empty());
        assert_eq!(d.pin_type(0, VAR_VALUE_PIN, true), Some(PinType::Int));

        let s = d.descriptor(1).unwrap();
        assert!(!s.pure, "a write is a statement in the exec flow");
        assert_eq!(d.pin_type(1, VAR_VALUE_PIN, false), Some(PinType::Int));
        assert_eq!(d.pin_type(1, EXEC_IN_PIN, false), Some(PinType::Exec));
        assert_eq!(d.pin_type(1, EXEC_OUT_PIN, true), Some(PinType::Exec));
        assert_eq!(
            s.input(VAR_VALUE_PIN).unwrap().default,
            Some(PropValue::Int(3)),
            "the declaration's default seeds the unconnected write pin"
        );

        // Retyping the variable retypes both nodes.
        doc.variables[0].ty = PinType::Array(Box::new(PinType::String));
        let d = DocDescriptors::new(&doc, &reg);
        assert_eq!(
            d.pin_type(0, VAR_VALUE_PIN, true),
            Some(PinType::Array(Box::new(PinType::String)))
        );

        // A node naming a deleted variable resolves to nothing — and the
        // slug survives so the error can name it.
        let d = DocDescriptors::new(&doc, &reg);
        assert!(d.descriptor(2).is_none());
        assert_eq!(d.variable_slug(2), Some("deleted"));
        assert!(d.descriptor(3).is_none(), "no `var` property at all");
    }

    /// `event_custom` = the registered base descriptor plus payload pins
    /// declared by the instance's properties.
    #[test]
    fn event_custom_payload_pins_come_from_properties() {
        let reg = registry();
        let mut n = node(0, EVENT_CUSTOM_TYPE_ID);
        n.properties.insert(
            format!("{EVENT_PAYLOAD_PREFIX}amount"),
            PropValue::Enum("float".into()),
        );
        let doc = GraphDoc { nodes: vec![n], ..GraphDoc::default() };
        let d = DocDescriptors::new(&doc, &reg);
        assert_eq!(d.kind(0), Some(NodeKind::EventCustom));
        assert_eq!(d.pin_type(0, EXEC_OUT_PIN, true), Some(PinType::Exec));
        assert_eq!(d.pin_type(0, "amount", true), Some(PinType::Float));
        assert!(!d.descriptor(0).unwrap().pure);

        // With the plugin that owns the event set disabled, nothing resolves
        // — and that is an `UnknownNodeType`, not a crash.
        let empty = NodeRegistry::new();
        assert!(DocDescriptors::new(&doc, &empty).descriptor(0).is_none());
    }

    /// A registered node is borrowed, not cloned, for every lookup.
    #[test]
    fn registered_descriptors_are_borrowed() {
        let reg = registry();
        let doc = GraphDoc {
            nodes: vec![node(0, "test_add")],
            ..GraphDoc::default()
        };
        let d = DocDescriptors::new(&doc, &reg);
        assert!(matches!(d.descriptor(0), Some(Cow::Borrowed(_))));
        assert_eq!(d.pulls_through(0), Some(true));
    }
}
