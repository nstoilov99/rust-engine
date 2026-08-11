//! Per-node-type schema migration (plan D4). Each node instance stores the
//! descriptor version it was saved against; on load, instances older than
//! the registered descriptor run the type's migration chain step by step
//! before validation.
//!
//! A step gets a [`MigrationCtx`], not just the property map: pin renames
//! are the most common migration, and edges store pin slugs *outside* the
//! node — [`MigrationCtx::rename_pin`] therefore rewrites the stored
//! constant key immediately and the node's incident edges after the step.

use std::collections::BTreeMap;

use crate::descriptors::NodeKind;
use crate::doc::{GraphDoc, PropValue};
use crate::registry::NodeRegistry;

/// What a migration step may do to a node instance.
pub struct MigrationCtx<'a> {
    /// The node's stored input constants, keyed by pin slug.
    pub props: &'a mut BTreeMap<String, PropValue>,
    pub(crate) renames: Vec<(String, String)>,
}

impl MigrationCtx<'_> {
    /// Rename a pin slug: moves the stored constant (if any) now, and
    /// rewrites the node's incident edges when the step returns.
    pub fn rename_pin(&mut self, old: &str, new: &str) {
        if let Some(v) = self.props.remove(old) {
            self.props.insert(new.to_string(), v);
        }
        self.renames.push((old.to_string(), new.to_string()));
    }
}

/// One migrated node, for logging.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationRecord {
    pub node: u64,
    pub type_id: String,
    pub from_version: u32,
    pub to_version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MigrationError {
    /// Registered type is at `current` but no step is registered from
    /// `from` — an authoring bug in the node library, reported loudly.
    MissingStep { type_id: String, from: u32, current: u32 },
    /// Instance was saved by a newer node library than this build.
    NewerInstance { node: u64, type_id: String, saved: u32, current: u32 },
}

impl std::fmt::Display for MigrationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MigrationError::MissingStep { type_id, from, current } => write!(
                f,
                "node type '{type_id}' is at v{current} but has no migration from v{from}"
            ),
            MigrationError::NewerInstance { node, type_id, saved, current } => write!(
                f,
                "node {node} ('{type_id}') was saved at v{saved}, newer than this build's v{current}"
            ),
        }
    }
}

impl std::error::Error for MigrationError {}

/// Migrate every stale node instance in `doc` up to its registered
/// descriptor version. Unknown types are left untouched (missing-node
/// tolerance — a disabled plugin must not corrupt or block the doc).
///
/// Document-dependent instances — subgraphs, reroutes, variable access,
/// interface binding — have no registered descriptor and therefore no version
/// chain: their pins move when the *document* moves, which is not a migration.
/// [`DocDescriptors::kind`] is what decides that, so the list of exceptions
/// lives in one place rather than growing a `type_id ==` here per reserved id.
pub fn migrate_doc(
    doc: &mut GraphDoc,
    registry: &NodeRegistry,
) -> Result<Vec<MigrationRecord>, MigrationError> {
    let mut records = Vec::new();
    for i in 0..doc.nodes.len() {
        let (node_id, type_id, start_version) = {
            let n = &doc.nodes[i];
            (n.id, n.type_id.clone(), n.type_version)
        };
        // `EventCustom` is deliberately *not* skipped: its base descriptor is
        // registered and versioned like any other, and only its payload pins
        // come from the document.
        if !matches!(
            NodeKind::of_type(&type_id),
            NodeKind::Registered | NodeKind::EventCustom
        ) {
            continue;
        }
        let Some(desc) = registry.get(&type_id) else {
            continue;
        };
        let current = desc.version;
        if start_version > current {
            return Err(MigrationError::NewerInstance {
                node: node_id,
                type_id,
                saved: start_version,
                current,
            });
        }
        let mut version = start_version;
        while version < current {
            let Some(step) = registry.migration(&type_id, version) else {
                return Err(MigrationError::MissingStep { type_id, from: version, current });
            };
            let renames = {
                let node = &mut doc.nodes[i];
                let mut ctx = MigrationCtx { props: &mut node.properties, renames: Vec::new() };
                step(&mut ctx);
                ctx.renames
            };
            for (old, new) in &renames {
                for e in doc.edges.iter_mut() {
                    if e.from_node == node_id && e.from_pin == *old {
                        e.from_pin = new.clone();
                    }
                    if e.to_node == node_id && e.to_pin == *old {
                        e.to_pin = new.clone();
                    }
                }
            }
            version += 1;
            doc.nodes[i].type_version = version;
        }
        if version != start_version {
            records.push(MigrationRecord {
                node: node_id,
                type_id,
                from_version: start_version,
                to_version: version,
            });
        }
    }
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dev_nodes::register_dev_nodes;
    use crate::doc::{NodeRealm, PinType};
    use crate::io::{parse_graph, serialize_graph};
    use crate::registry::{NodeDescriptor, PinDescriptor};
    use crate::validate::validate_doc;

    /// Registry with a v2 node type whose v1→v2 migration renames the
    /// "dps" pin to "damage_per_second".
    fn registry_v2() -> NodeRegistry {
        let mut reg = NodeRegistry::new();
        register_dev_nodes(&mut reg).unwrap();
        reg.register(NodeDescriptor {
            id: "mig_damage".into(),
            name: "Migrating Damage".into(),
            category: "Dev".into(),
            version: 2,
            inputs: vec![PinDescriptor::new(
                "damage_per_second",
                "Damage / s",
                PinType::Float,
            )
            .with_default(PropValue::Float(1.0))],
            outputs: vec![PinDescriptor::new("applied", "Applied", PinType::Float)],
            pure: true,
            realm: NodeRealm::Shared,
            deterministic: true,
            doc: None,
            preview: None,
        })
        .unwrap();
        reg.register_migration("mig_damage", 1, |ctx| {
            ctx.rename_pin("dps", "damage_per_second");
        });
        reg
    }

    #[test]
    fn golden_v1_doc_migrates_to_expected() {
        let mut doc = parse_graph(include_str!("fixtures/migration_v1.graph")).unwrap();
        let records = migrate_doc(&mut doc, &registry_v2()).unwrap();
        assert_eq!(
            records,
            vec![MigrationRecord {
                node: 1,
                type_id: "mig_damage".into(),
                from_version: 1,
                to_version: 2,
            }]
        );
        // Migrated doc must validate clean against the v2 registry…
        assert_eq!(validate_doc(&doc, &registry_v2()), vec![]);
        // …and match the committed expected output byte-for-byte.
        let actual = serialize_graph(&doc).unwrap();
        if std::env::var("UPDATE_GRAPH_FIXTURES").is_ok() {
            let path = concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/src/fixtures/migration_v1_expected.graph"
            );
            std::fs::write(path, &actual).unwrap();
            return; // freshly written; the compiled-in copy is stale
        }
        let expected = include_str!("fixtures/migration_v1_expected.graph");
        assert_eq!(actual, expected);
    }

    #[test]
    fn missing_step_is_loud() {
        let mut reg = NodeRegistry::new();
        // v3 type with only the 1→2 step registered.
        reg.register(NodeDescriptor {
            id: "gappy".into(),
            name: "Gappy".into(),
            category: "Dev".into(),
            version: 3,
            inputs: vec![PinDescriptor::new("x", "X", PinType::Float)],
            outputs: vec![PinDescriptor::new("y", "Y", PinType::Float)],
            pure: true,
            realm: NodeRealm::Shared,
            deterministic: true,
            doc: None,
            preview: None,
        })
        .unwrap();
        reg.register_migration("gappy", 1, |_| {});
        let mut doc = GraphDoc::default();
        doc.nodes.push(crate::doc::NodeInst {
            id: 0,
            type_id: "gappy".into(),
            type_version: 1,
            position: [0.0, 0.0],
            properties: BTreeMap::new(),
            subgraph: None,
        
        tint: None, title: None,});
        assert_eq!(
            migrate_doc(&mut doc, &reg),
            Err(MigrationError::MissingStep { type_id: "gappy".into(), from: 2, current: 3 })
        );
    }

    #[test]
    fn newer_instance_rejected_unknown_type_untouched() {
        let reg = registry_v2();
        let mut doc = GraphDoc::default();
        doc.nodes.push(crate::doc::NodeInst {
            id: 0,
            type_id: "totally_unknown".into(),
            type_version: 99,
            position: [0.0, 0.0],
            properties: BTreeMap::new(),
            subgraph: None,
        
        tint: None, title: None,});
        // Unknown type: not an error here (validate_doc reports it), and
        // the instance is untouched.
        assert_eq!(migrate_doc(&mut doc, &reg).unwrap(), vec![]);
        assert_eq!(doc.nodes[0].type_version, 99);

        doc.nodes[0].type_id = "mig_damage".into();
        assert!(matches!(
            migrate_doc(&mut doc, &reg),
            Err(MigrationError::NewerInstance { saved: 99, current: 2, .. })
        ));
    }
}
