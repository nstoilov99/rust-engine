//! Graph asset serialization: RON save/load with a version-envelope pass.
//!
//! The container version is probed from the text *before* strict
//! deserialization, so future container migrations can rewrite old documents
//! that no longer parse as the current `GraphDoc` schema (none exist at v1;
//! the seam is `parse_graph`). Per-node-type migrations run after parse, in
//! the migration layer (P3).

use std::path::Path;

use super::doc::{GraphDoc, GRAPH_DOC_VERSION};

#[derive(Debug)]
pub enum GraphIoError {
    /// The document's container version is newer than this engine build.
    NewerVersion { found: u32, supported: u32 },
    Parse(String),
    Io(String),
}

impl std::fmt::Display for GraphIoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GraphIoError::NewerVersion { found, supported } => write!(
                f,
                "graph document version {found} is newer than supported {supported}"
            ),
            GraphIoError::Parse(e) => write!(f, "graph parse error: {e}"),
            GraphIoError::Io(e) => write!(f, "graph io error: {e}"),
        }
    }
}

impl std::error::Error for GraphIoError {}

/// Envelope probe: only the version field, unknown fields ignored — this
/// must keep parsing even when the full schema has moved on.
#[derive(serde::Deserialize)]
struct VersionProbe {
    version: u32,
}

/// Parse a graph document from RON text (envelope pass + typed parse).
pub fn parse_graph(text: &str) -> Result<GraphDoc, GraphIoError> {
    let probe: VersionProbe =
        ron::from_str(text).map_err(|e| GraphIoError::Parse(e.to_string()))?;
    if probe.version > GRAPH_DOC_VERSION {
        return Err(GraphIoError::NewerVersion {
            found: probe.version,
            supported: GRAPH_DOC_VERSION,
        });
    }
    // Container migrations slot in here when GRAPH_DOC_VERSION grows past 1:
    // rewrite `text` (or a ron::Value) from probe.version up, then parse.
    ron::from_str(text).map_err(|e| GraphIoError::Parse(e.to_string()))
}

/// Serialize a graph document to RON text. Deterministic (ordered maps,
/// fixed pretty config) so saves are byte-stable for diffing and tests.
/// Serialize a document in **canonical form**: nodes in id order, positions
/// and annotation rects rounded to whole pixels.
///
/// Graphs live in version control, so a save must not churn on things the
/// author did not change. Insertion order drifts with every paste and delete,
/// and sub-pixel positions come out of drag arithmetic — neither is
/// information, and both produce noisy diffs.
///
/// The canonicalization happens on a **clone**: the in-memory document keeps
/// its live order, because the undo stack addresses nodes and edges by index.
pub fn serialize_graph(doc: &GraphDoc) -> Result<String, GraphIoError> {
    let canonical = canonical_form(doc);
    let config = ron::ser::PrettyConfig::new().struct_names(false);
    let mut text = ron::ser::to_string_pretty(&canonical, config)
        .map_err(|e| GraphIoError::Parse(e.to_string()))?;
    // A text file ends with a newline. Without it every diff of an asset's
    // last line carries a spurious "\ No newline at end of file".
    text.push('\n');
    Ok(text)
}

/// The document as it goes to disk. Public so a caller can compare two docs
/// the way the file would see them.
pub fn canonical_form(doc: &GraphDoc) -> GraphDoc {
    let mut out = doc.clone();
    out.nodes.sort_by_key(|n| n.id);
    for n in out.nodes.iter_mut() {
        n.position = [round_px(n.position[0]), round_px(n.position[1])];
    }
    for c in out.comments.iter_mut() {
        c.rect = c.rect.map(round_px);
    }
    for g in out.groups.iter_mut() {
        g.rect = g.rect.map(round_px);
    }
    out
}

/// Round to a whole pixel, mapping a non-finite value to 0 rather than
/// writing `NaN` into an asset.
fn round_px(v: f32) -> f32 {
    if v.is_finite() {
        v.round()
    } else {
        0.0
    }
}

pub fn load_graph(path: &Path) -> Result<GraphDoc, GraphIoError> {
    let text =
        std::fs::read_to_string(path).map_err(|e| GraphIoError::Io(format!("{path:?}: {e}")))?;
    parse_graph(&text)
}

pub fn save_graph(path: &Path, doc: &GraphDoc) -> Result<(), GraphIoError> {
    let text = serialize_graph(doc)?;
    std::fs::write(path, text).map_err(|e| GraphIoError::Io(format!("{path:?}: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::node_graph::doc::*;
    use std::collections::BTreeMap;

    /// Saves are canonical: nodes in id order, positions whole-pixel — so a
    /// graph in version control does not churn on paste/delete history or on
    /// sub-pixel drag arithmetic.
    #[test]
    fn serialization_is_canonical_and_stable() {
        let mut a = GraphDoc::default();
        a.nodes = vec![
            NodeInst {
                id: 7,
                type_id: "b".into(),
                type_version: 1,
                position: [10.4, -3.6],
                properties: BTreeMap::new(),
                subgraph: None,
                tint: None,
            },
            NodeInst {
                id: 2,
                type_id: "a".into(),
                type_version: 1,
                position: [0.49, 100.5],
                properties: BTreeMap::new(),
                subgraph: None,
                tint: None,
            },
        ];
        a.comments = vec![CommentBox { rect: [1.2, 2.7, 100.4, 60.6], ..Default::default() }];
        a.groups = vec![GroupBox { rect: [-0.4, 9.5, 20.2, 30.8], ..Default::default() }];

        // The same doc with the nodes entered in the other order must produce
        // byte-identical output.
        let mut b = a.clone();
        b.nodes.reverse();
        let (ta, tb) = (serialize_graph(&a).unwrap(), serialize_graph(&b).unwrap());
        assert_eq!(ta, tb, "insertion order must not reach the file");

        // Node order on disk is id order.
        let reloaded = parse_graph(&ta).unwrap();
        assert_eq!(reloaded.nodes.iter().map(|n| n.id).collect::<Vec<_>>(), vec![2, 7]);
        // Positions and rects are whole pixels.
        assert_eq!(reloaded.nodes[0].position, [0.0, 101.0]);
        assert_eq!(reloaded.nodes[1].position, [10.0, -4.0]);
        assert_eq!(reloaded.comments[0].rect, [1.0, 3.0, 100.0, 61.0]);
        assert_eq!(reloaded.groups[0].rect, [0.0, 10.0, 20.0, 31.0]);

        // Canonical form is idempotent — saving a loaded file changes nothing.
        assert_eq!(serialize_graph(&reloaded).unwrap(), ta);
        assert!(ta.ends_with('\n'), "an asset must end with a newline");

        // …and the in-memory doc is untouched: undo addresses nodes by index.
        assert_eq!(a.nodes[0].id, 7, "canonicalization must not reorder in memory");
        assert_eq!(a.nodes[0].position, [10.4, -3.6]);
    }

    /// A non-finite position is written as 0 rather than `NaN`, which would
    /// make the asset unparseable.
    #[test]
    fn non_finite_positions_do_not_reach_the_file() {
        let mut d = GraphDoc::default();
        d.nodes = vec![NodeInst {
            id: 0,
            type_id: "a".into(),
            type_version: 1,
            position: [f32::NAN, f32::INFINITY],
            properties: BTreeMap::new(),
            subgraph: None,
            tint: None,
        }];
        let text = serialize_graph(&d).unwrap();
        assert!(!text.contains("NaN") && !text.contains("inf"), "{text}");
        assert_eq!(parse_graph(&text).unwrap().nodes[0].position, [0.0, 0.0]);
    }

    /// Rewrite the repository's committed graph assets in canonical form.
    /// Opt-in like the fixture writer: `UPDATE_GRAPH_FIXTURES=1`.
    #[test]
    fn canonicalize_content_assets_if_requested() {
        if std::env::var("UPDATE_GRAPH_FIXTURES").is_err() {
            return;
        }
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("content/graphs");
        let mut stack = vec![root];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                    continue;
                }
                let ext = p.extension().and_then(|x| x.to_str()).unwrap_or("");
                if ext != "graph" && ext != "subgraph" {
                    continue;
                }
                if let Ok(doc) = load_graph(&p) {
                    save_graph(&p, &doc).unwrap();
                }
            }
        }
    }

    fn demo_doc() -> GraphDoc {
        let mut props = BTreeMap::new();
        props.insert("dps".to_string(), PropValue::Float(12.5));
        props.insert("label".to_string(), PropValue::Enum("burn".to_string()));
        GraphDoc {
            realm: GraphRealm::Shared,
            nodes: vec![
                NodeInst {
                    id: 0,
                    type_id: "test_event".to_string(),
                    type_version: 1,
                    position: [40.0, 80.0],
                    properties: BTreeMap::new(),
                    subgraph: None,
                
                tint: None,},
                NodeInst {
                    id: 1,
                    type_id: "test_damage".to_string(),
                    type_version: 1,
                    position: [260.0, 80.0],
                    properties: props,
                    subgraph: None,
                
                tint: None,},
            ],
            edges: vec![Edge {
                from_node: 0,
                from_pin: "exec_out".to_string(),
                to_node: 1,
                to_pin: "exec_in".to_string(),
            }],
            ..GraphDoc::default()
        }
    }

    #[test]
    fn round_trip_is_byte_stable() {
        let doc = demo_doc();
        let a = serialize_graph(&doc).unwrap();
        let reparsed = parse_graph(&a).unwrap();
        assert_eq!(reparsed, doc);
        let b = serialize_graph(&reparsed).unwrap();
        assert_eq!(a, b, "save -> load -> save must be byte-identical");
    }

    #[test]
    fn newer_container_version_is_rejected() {
        let doc = GraphDoc { version: GRAPH_DOC_VERSION + 1, ..GraphDoc::default() };
        let text = serialize_graph(&doc).unwrap();
        match parse_graph(&text) {
            Err(GraphIoError::NewerVersion { found, .. }) => {
                assert_eq!(found, GRAPH_DOC_VERSION + 1)
            }
            other => panic!("expected NewerVersion, got {other:?}"),
        }
    }

    #[test]
    fn envelope_probe_survives_unknown_fields() {
        // A future schema with extra fields must still version-probe cleanly.
        let text = "(version: 1, realm: Shared, some_future_field: 42)";
        let doc = parse_graph(text);
        // Typed parse ignores unknown fields too (serde default behavior).
        assert!(doc.is_ok(), "{doc:?}");
    }

    #[test]
    fn subgraph_reference_round_trips() {
        // The dummy graph-references-subgraph readiness fixture: a host graph
        // referencing a subgraph asset by content-relative path.
        let sub = GraphDoc {
            inputs: vec![IfacePin {
                slug: "amount".to_string(),
                label: "Amount".to_string(),
                ty: PinType::Float,
            }],
            outputs: vec![IfacePin {
                slug: "result".to_string(),
                label: "Result".to_string(),
                ty: PinType::Float,
            }],
            ..GraphDoc::default()
        };
        let mut host = demo_doc();
        host.nodes.push(NodeInst {
            id: 2,
            type_id: "subgraph".to_string(),
            type_version: 1,
            position: [500.0, 80.0],
            properties: BTreeMap::new(),
            subgraph: Some("graphs/lib/calc.subgraph".to_string()),
        
        tint: None,});

        let dir = std::env::temp_dir().join("rust_engine_graph_io_test");
        std::fs::create_dir_all(&dir).unwrap();
        let host_path = dir.join("host.graph");
        let sub_path = dir.join("calc.subgraph");
        save_graph(&host_path, &host).unwrap();
        save_graph(&sub_path, &sub).unwrap();

        let host_back = load_graph(&host_path).unwrap();
        let sub_back = load_graph(&sub_path).unwrap();
        assert_eq!(host_back, host);
        assert_eq!(sub_back, sub);
        assert_eq!(host_back.subgraph_refs(), vec!["graphs/lib/calc.subgraph"]);
        assert_eq!(sub_back.inputs.len(), 1);
        assert_eq!(sub_back.outputs.len(), 1);
    }

    /// `UPDATE_GRAPH_FIXTURES=1 cargo test -p rust_engine write_fixture`
    /// regenerates the committed fixture (crusty-gui `UPDATE_SNAPSHOTS`
    /// convention).
    #[test]
    fn write_fixture_if_requested() {
        if std::env::var("UPDATE_GRAPH_FIXTURES").is_ok() {
            let path = concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/src/engine/node_graph/fixtures/demo.graph"
            );
            std::fs::write(path, serialize_graph(&demo_doc()).unwrap()).unwrap();
        }
    }

    #[test]
    fn checked_in_fixture_parses() {
        // Format-stability guard: this fixture is committed; if the schema
        // changes incompatibly, this fails before any user asset breaks.
        let text = include_str!("fixtures/demo.graph");
        let doc = parse_graph(text).unwrap();
        assert_eq!(doc.nodes.len(), 2);
        assert_eq!(doc.edges.len(), 1);
        assert_eq!(doc, demo_doc());
    }
}
