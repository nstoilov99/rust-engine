//! Compilation: document → flattened executable plan (D1).
//!
//! Five stages, in order:
//!
//! 1. **validate** — `validate_doc_with` + `validate_refs` over the whole
//!    reference tree. Any `ErrorSeverity::Error` refuses to compile; warnings
//!    pass (an unbound interface pin is a document that runs, just not the
//!    way its author probably meant).
//! 2. **flatten** — every node of the document *and of every subgraph it
//!    references*, each keyed by its instance path, gets a plan index.
//!    Reroutes and `graph_input`/`graph_output` nodes get none: they are
//!    transparent and are resolved away in stage 3.
//! 3. **splice** — every input and every exec pin is resolved to a *real*
//!    producing/consuming plan node by walking the transparent chain:
//!    through reroutes, and across subgraph boundaries via the P1 interface
//!    binding nodes. This is what makes a subgraph disappear at runtime.
//! 4. **prune** — nodes unreachable from any entry point are dropped, so a
//!    disconnected experiment costs nothing at run time.
//! 5. **index entries** — entry nodes bucketed by `EventPhase`, in document
//!    order within a phase (D3: "multiple entry nodes for the same event all
//!    fire, in doc order").

use std::collections::{BTreeMap, BTreeSet};

use node_graph_types::{
    validate_doc_with, validate_refs, DocDescriptors, ErrorSeverity, EventPhase, GraphDoc,
    GraphError, GraphResolver, NodeRegistry, PropValue, VarDecl, GRAPH_INPUT_TYPE_ID,
    GRAPH_OUTPUT_TYPE_ID, REROUTE_IN, REROUTE_OUT, REROUTE_TYPE_ID, SUBGRAPH_TYPE_ID,
};
use serde::{Deserialize, Serialize};

use crate::value::Value;

/// Where one input pin gets its value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum InputSource {
    /// The pin is unconnected: its stored constant, or the type's zero.
    Constant(Value),
    /// The pin is wired to another plan node's output.
    Pin { node: usize, pin: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanNode {
    pub type_id: String,
    pub pure: bool,
    /// `deterministic: false` in the descriptor — the node reads instance
    /// RNG or is otherwise volatile, so it is exempt from statement-scoped
    /// memoization (D1).
    pub volatile: bool,
    /// Human name for reports: `"lib/calc.subgraph#3 (test_add)"`.
    pub name: String,
    /// Input pin -> where its value comes from.
    pub inputs: BTreeMap<String, InputSource>,
    /// Exec output pin -> the plan node it continues to.
    pub exec: BTreeMap<String, usize>,
    /// The variable a `var_get`/`var_set` names, resolved at compile time.
    pub variable: Option<String>,
}

/// An entry point: an event node the runner can activate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub phase: EventPhase,
    pub node: usize,
    /// For `event_custom` / `event_input_action`: which name this entry
    /// answers to. `None` for the phases that have exactly one source.
    pub name: Option<String>,
}

/// A compiled graph. Plain data: cacheable per (asset, version), shareable
/// between instances, and serializable for a future precompiled-plan export.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Plan {
    pub nodes: Vec<PlanNode>,
    pub entries: Vec<Entry>,
    pub variables: Vec<VarSlot>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VarSlot {
    pub slug: String,
    pub initial: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CompileError {
    /// The document (or one it references) does not validate. Compilation is
    /// not the place to discover that a wire is mistyped.
    Invalid { path: String, errors: Vec<GraphError> },
    /// A subgraph reference that the resolver cannot produce. Distinct from
    /// `Invalid` because the fix is a missing *file*, not a wrong wire.
    MissingSubgraph { path: String },
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CompileError::Invalid { path, errors } => {
                write!(f, "{path} does not validate:")?;
                for e in errors {
                    write!(f, "\n  - {e}")?;
                }
                Ok(())
            }
            CompileError::MissingSubgraph { path } => write!(f, "subgraph '{path}' not found"),
        }
    }
}

impl std::error::Error for CompileError {}

/// A node's identity in the flattened plan: the chain of subgraph node ids
/// that led to it, then its own doc-local id. Two instances of the same
/// subgraph therefore get separate plan nodes, which is what inlining means.
type NodeKey = (Vec<u64>, u64);

/// Compile `doc` (loaded from `doc_path`) into an executable plan.
pub fn compile(
    doc: &GraphDoc,
    doc_path: &str,
    registry: &NodeRegistry,
    resolver: &dyn GraphResolver,
) -> Result<Plan, CompileError> {
    // --- 1. validate, through the whole reference tree ---------------------
    validate_tree(doc, doc_path, registry, resolver, &mut BTreeSet::new())?;

    // --- 2. flatten --------------------------------------------------------
    let mut f = Flattener {
        registry,
        resolver,
        nodes: Vec::new(),
        index: BTreeMap::new(),
        scopes: Vec::new(),
    };
    f.collect(doc, doc_path, &[])?;

    // --- 3. splice ---------------------------------------------------------
    let scopes = std::mem::take(&mut f.scopes);
    let mut nodes = std::mem::take(&mut f.nodes);
    let index = f.index.clone();
    let ctx = SpliceCtx { scopes: &scopes, index: &index, registry, resolver };
    for flat in nodes.iter_mut() {
        let scope = ctx.scope_of(&flat.key);
        flat.plan.inputs = ctx.resolve_inputs(&flat.key, scope);
        flat.plan.exec = ctx.resolve_exec(&flat.key, scope);
    }

    // --- 5. entries (indexed before pruning: entries are the roots) --------
    let mut entries: Vec<Entry> = Vec::new();
    for (i, n) in nodes.iter().enumerate() {
        // Only the top-level document's entry nodes are entry points. An
        // event node inside a subgraph would fire per *use* of that subgraph,
        // which is not what an entry point means.
        if !n.key.0.is_empty() {
            continue;
        }
        let Some(phase) = EventPhase::of_type(&n.plan.type_id) else { continue };
        entries.push(Entry {
            phase,
            node: i,
            name: n.event_name.clone(),
        });
    }
    // Doc order within a phase; phases in drain order.
    entries.sort_by_key(|e| (e.phase, e.node));

    // --- 4. prune ----------------------------------------------------------
    let keep = reachable(&nodes, &entries);
    let mut remap = vec![usize::MAX; nodes.len()];
    let mut out: Vec<PlanNode> = Vec::new();
    for (i, n) in nodes.iter().enumerate() {
        if keep.contains(&i) {
            remap[i] = out.len();
            out.push(n.plan.clone());
        }
    }
    for n in out.iter_mut() {
        for src in n.inputs.values_mut() {
            if let InputSource::Pin { node, .. } = src {
                *node = remap[*node];
            }
        }
        n.exec.retain(|_, target| remap[*target] != usize::MAX);
        for target in n.exec.values_mut() {
            *target = remap[*target];
        }
    }
    let entries: Vec<Entry> = entries
        .into_iter()
        .filter(|e| remap[e.node] != usize::MAX)
        .map(|e| Entry { node: remap[e.node], ..e })
        .collect();

    let variables = doc
        .variables
        .iter()
        .map(|v: &VarDecl| VarSlot {
            slug: v.slug.clone(),
            initial: v
                .default
                .as_ref()
                .map(Value::from_prop)
                .unwrap_or_else(|| Value::zero_of(&v.ty)),
        })
        .collect();

    Ok(Plan { nodes: out, entries, variables })
}

/// Validate the document and everything it references. A subgraph whose own
/// document is broken breaks its host, so the check is not doc-local.
fn validate_tree(
    doc: &GraphDoc,
    path: &str,
    registry: &NodeRegistry,
    resolver: &dyn GraphResolver,
    seen: &mut BTreeSet<String>,
) -> Result<(), CompileError> {
    if !seen.insert(path.to_string()) {
        return Ok(()); // already checked (and cycles are a validation error)
    }
    let d = DocDescriptors::with_resolver(doc, registry, resolver);
    let mut errors = validate_doc_with(&d);
    errors.extend(validate_refs(doc, path, registry, resolver));
    let hard: Vec<GraphError> = errors
        .into_iter()
        .filter(|e| e.severity() == ErrorSeverity::Error)
        .collect();
    if !hard.is_empty() {
        return Err(CompileError::Invalid { path: path.to_string(), errors: hard });
    }
    for r in doc.subgraph_refs() {
        let sub = resolver
            .resolve(r)
            .ok_or_else(|| CompileError::MissingSubgraph { path: r.to_string() })?;
        validate_tree(sub, r, registry, resolver, seen)?;
    }
    Ok(())
}

/// One flattened node, plus the bookkeeping the splice stage needs.
struct Flat {
    key: NodeKey,
    plan: PlanNode,
    event_name: Option<String>,
}

/// One inlined document instance: which doc it is, and (for a subgraph) the
/// host node whose pins its interface binds to.
struct Scope {
    path: Vec<u64>,
    doc: GraphDoc,
    /// The host scope + subgraph node this instance was expanded from.
    host: Option<(usize, u64)>,
}

struct Flattener<'a> {
    registry: &'a NodeRegistry,
    resolver: &'a dyn GraphResolver,
    nodes: Vec<Flat>,
    index: BTreeMap<NodeKey, usize>,
    scopes: Vec<Scope>,
}

impl Flattener<'_> {
    fn collect(
        &mut self,
        doc: &GraphDoc,
        path: &str,
        scope_path: &[u64],
    ) -> Result<usize, CompileError> {
        let host = self
            .scopes
            .iter()
            .position(|s| s.path.len() + 1 == scope_path.len() && scope_path.starts_with(&s.path))
            .map(|i| (i, *scope_path.last().unwrap()));
        let scope_ix = self.scopes.len();
        self.scopes.push(Scope {
            path: scope_path.to_vec(),
            doc: doc.clone(),
            host,
        });

        for n in &doc.nodes {
            // Transparent nodes never become plan nodes — stage 3 resolves
            // straight through them.
            if matches!(
                n.type_id.as_str(),
                REROUTE_TYPE_ID | GRAPH_INPUT_TYPE_ID | GRAPH_OUTPUT_TYPE_ID
            ) {
                continue;
            }
            if n.type_id == SUBGRAPH_TYPE_ID {
                let sub_path = n.subgraph.as_deref().unwrap_or_default();
                let sub = self
                    .resolver
                    .resolve(sub_path)
                    .ok_or_else(|| CompileError::MissingSubgraph { path: sub_path.to_string() })?
                    .clone();
                let mut inner = scope_path.to_vec();
                inner.push(n.id);
                self.collect(&sub, sub_path, &inner)?;
                continue;
            }

            let d = DocDescriptors::with_resolver(&self.scopes[scope_ix].doc, self.registry, self.resolver);
            let desc = d.descriptor(n.id);
            let (pure, volatile) = desc
                .as_ref()
                .map(|x| (x.pure, !x.deterministic))
                .unwrap_or((true, false));
            let key = (scope_path.to_vec(), n.id);
            let ix = self.nodes.len();
            self.index.insert(key.clone(), ix);
            self.nodes.push(Flat {
                key,
                plan: PlanNode {
                    type_id: n.type_id.clone(),
                    pure,
                    volatile,
                    name: format!("{path}#{} ({})", n.id, n.type_id),
                    inputs: BTreeMap::new(),
                    exec: BTreeMap::new(),
                    variable: d.variable_slug(n.id).map(|s| s.to_string()),
                },
                event_name: match n.properties.get(node_graph_types::EVENT_NAME_PROP) {
                    Some(PropValue::Str(s)) => Some(s.clone()),
                    _ => match n.properties.get(node_graph_types::EVENT_ACTION_PROP) {
                        Some(PropValue::Str(s)) => Some(s.clone()),
                        _ => None,
                    },
                },
            });
        }
        Ok(scope_ix)
    }
}

struct SpliceCtx<'a> {
    scopes: &'a [Scope],
    index: &'a BTreeMap<NodeKey, usize>,
    registry: &'a NodeRegistry,
    resolver: &'a dyn GraphResolver,
}

impl SpliceCtx<'_> {
    fn scope_of(&self, key: &NodeKey) -> usize {
        self.scopes
            .iter()
            .position(|s| s.path == key.0)
            .expect("every flattened node came from a scope")
    }

    /// Resolve every input pin of `flat` to a constant or a real producer.
    fn resolve_inputs(&self, key: &NodeKey, scope: usize) -> BTreeMap<String, InputSource> {
        let doc = &self.scopes[scope].doc;
        let d = DocDescriptors::with_resolver(doc, self.registry, self.resolver);
        let node = doc.node(key.1).expect("flattened node exists");
        let mut out = BTreeMap::new();

        // The descriptor's input list is the authority on which pins exist.
        // Without one (a doc-dependent type the registry cannot answer) fall
        // back to the stored properties, so a partially-known node still runs
        // its constants rather than nothing.
        let declared: Vec<String> = match d.descriptor(key.1) {
            Some(desc) => desc
                .inputs
                .iter()
                .filter(|p| p.ty != node_graph_types::PinType::Exec)
                .map(|p| p.slug.clone())
                .collect(),
            None => node.properties.keys().cloned().collect(),
        };

        for slug in declared {
            out.insert(slug.clone(), self.resolve_source(scope, key.1, &slug));
        }
        out
    }

    /// Resolve every exec output pin to the plan node it continues to.
    fn resolve_exec(&self, key: &NodeKey, scope: usize) -> BTreeMap<String, usize> {
        let doc = &self.scopes[scope].doc;
        let mut out = BTreeMap::new();
        for e in doc.edges.iter().filter(|e| e.from_node == key.1) {
            // Exec pins only: a data edge is resolved from the consumer side.
            if !self.is_exec_pin(scope, key.1, &e.from_pin, true) {
                continue;
            }
            if let Some(target) = self.consumer(scope, e.to_node, &e.to_pin) {
                out.insert(e.from_pin.clone(), target);
            }
        }
        out
    }

    fn is_exec_pin(&self, scope: usize, node: u64, pin: &str, output: bool) -> bool {
        let doc = &self.scopes[scope].doc;
        DocDescriptors::new(doc, self.registry).pin_type(node, pin, output)
            == Some(node_graph_types::PinType::Exec)
    }

    /// Walk *backward* from an input pin to whatever actually supplies it,
    /// stepping through reroutes and across subgraph boundaries.
    ///
    /// The walk can end two ways, and both matter. It ends at a **producing
    /// pin** — a real plan node's output — or it runs out of wire, in which
    /// case the answer is the **constant at wherever it stopped**. That
    /// second case is what carries a host's stored constant into an inlined
    /// subgraph: the chain steps out through `graph_input` to the host's
    /// subgraph-node pin, finds no wire there, and takes the property the
    /// author typed on the host node.
    fn resolve_source(&self, scope: usize, node: u64, pin: &str) -> InputSource {
        let mut scope = scope;
        let mut node = node;
        let mut pin = pin.to_string();
        for _ in 0..MAX_SPLICE_HOPS {
            let doc = &self.scopes[scope].doc;
            let Some(e) = doc
                .edges
                .iter()
                .find(|e| e.to_node == node && e.to_pin == pin)
            else {
                return InputSource::Constant(self.constant_at(scope, node, &pin));
            };
            let Some(src) = doc.node(e.from_node) else {
                return InputSource::Constant(self.constant_at(scope, node, &pin));
            };
            match src.type_id.as_str() {
                // A reroute passes its `in` straight through.
                REROUTE_TYPE_ID => {
                    node = src.id;
                    pin = REROUTE_IN.to_string();
                }
                // A `graph_input` output is the *host's* input pin of the
                // same slug: step out of this subgraph instance.
                GRAPH_INPUT_TYPE_ID => {
                    let Some((host_scope, host_node)) = self.scopes[scope].host else {
                        return InputSource::Constant(Value::Unit);
                    };
                    scope = host_scope;
                    node = host_node;
                    pin = e.from_pin.clone();
                }
                // A subgraph node's output is produced by whatever feeds the
                // matching `graph_output` pin inside it: step in.
                SUBGRAPH_TYPE_ID => {
                    let (Some(inner), ) = (self.inner_scope(scope, src.id), ) else {
                        return InputSource::Constant(Value::Unit);
                    };
                    let Some(go) = self.iface_node(inner, GRAPH_OUTPUT_TYPE_ID) else {
                        return InputSource::Constant(Value::Unit);
                    };
                    scope = inner;
                    node = go;
                    pin = e.from_pin.clone();
                }
                _ => {
                    return match self.index.get(&(self.scopes[scope].path.clone(), src.id)) {
                        Some(ix) => InputSource::Pin { node: *ix, pin: e.from_pin.clone() },
                        None => InputSource::Constant(Value::Unit),
                    }
                }
            }
        }
        InputSource::Constant(Value::Unit)
    }

    /// The constant an unwired input pin holds: the author's stored property,
    /// else the descriptor's declared default, else the type's zero.
    fn constant_at(&self, scope: usize, node: u64, pin: &str) -> Value {
        let doc = &self.scopes[scope].doc;
        let d = DocDescriptors::with_resolver(doc, self.registry, self.resolver);
        if let Some(n) = doc.node(node) {
            if let Some(v) = n.properties.get(pin) {
                return Value::from_prop(v);
            }
        }
        match d.descriptor(node).and_then(|desc| desc.input(pin).cloned()) {
            Some(p) => p
                .default
                .as_ref()
                .map(Value::from_prop)
                .unwrap_or_else(|| Value::zero_of(&p.ty)),
            None => Value::Unit,
        }
    }

    /// Walk *forward* from an exec edge's destination to the real plan node
    /// that executes, stepping through the same transparent nodes.
    fn consumer(&self, scope: usize, node: u64, pin: &str) -> Option<usize> {
        let mut scope = scope;
        let mut node = node;
        let mut pin = pin.to_string();
        for _ in 0..MAX_SPLICE_HOPS {
            let doc = &self.scopes[scope].doc;
            let n = doc.node(node)?;
            match n.type_id.as_str() {
                REROUTE_TYPE_ID => {
                    let e = doc
                        .edges
                        .iter()
                        .find(|e| e.from_node == n.id && e.from_pin == REROUTE_OUT)?;
                    node = e.to_node;
                    pin = e.to_pin.clone();
                }
                // Entering a subgraph: continue at whatever its `graph_input`
                // node's matching pin feeds.
                SUBGRAPH_TYPE_ID => {
                    let inner = self.inner_scope(scope, n.id)?;
                    let gi = self.iface_node(inner, GRAPH_INPUT_TYPE_ID)?;
                    let e = self.scopes[inner]
                        .doc
                        .edges
                        .iter()
                        .find(|e| e.from_node == gi && e.from_pin == pin)?;
                    scope = inner;
                    node = e.to_node;
                    pin = e.to_pin.clone();
                }
                // Leaving a subgraph through its `graph_output`: continue at
                // whatever the host's matching output pin feeds.
                GRAPH_OUTPUT_TYPE_ID => {
                    let (host_scope, host_node) = self.scopes[scope].host?;
                    let e = self.scopes[host_scope]
                        .doc
                        .edges
                        .iter()
                        .find(|e| e.from_node == host_node && e.from_pin == pin)?;
                    scope = host_scope;
                    node = e.to_node;
                    pin = e.to_pin.clone();
                }
                GRAPH_INPUT_TYPE_ID => return None,
                _ => return self.index.get(&(self.scopes[scope].path.clone(), n.id)).copied(),
            }
        }
        None
    }

    /// The scope a subgraph node in `scope` was expanded into.
    fn inner_scope(&self, scope: usize, sub_node: u64) -> Option<usize> {
        self.scopes
            .iter()
            .position(|s| s.host == Some((scope, sub_node)))
    }

    fn iface_node(&self, scope: usize, type_id: &str) -> Option<u64> {
        self.scopes[scope]
            .doc
            .nodes
            .iter()
            .find(|n| n.type_id == type_id)
            .map(|n| n.id)
    }
}

/// A splice chain longer than this is degenerate input, not a graph.
const MAX_SPLICE_HOPS: usize = 256;

/// Everything reachable from an entry point: the exec closure, plus the data
/// closure of every node in it.
fn reachable(nodes: &[Flat], entries: &[Entry]) -> BTreeSet<usize> {
    let mut keep: BTreeSet<usize> = BTreeSet::new();
    let mut frontier: Vec<usize> = entries.iter().map(|e| e.node).collect();
    while let Some(ix) = frontier.pop() {
        if !keep.insert(ix) {
            continue;
        }
        for target in nodes[ix].plan.exec.values() {
            frontier.push(*target);
        }
        for src in nodes[ix].plan.inputs.values() {
            if let InputSource::Pin { node, .. } = src {
                frontier.push(*node);
            }
        }
    }
    keep
}
