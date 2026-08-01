//! Test/demo node set (`dev_nodes` feature + unit tests). Not a domain
//! library — just enough surface to exercise the framework and the graph
//! editor before consumer tasks (41, 45, …) ship real nodes.

use super::doc::{NodeRealm, PinType, PropValue};
use super::registry::{NodeDescriptor, NodeRegistry, PinDescriptor, RegistryError};

pub fn register_dev_nodes(reg: &mut NodeRegistry) -> Result<(), RegistryError> {
    reg.register(NodeDescriptor {
        id: "test_event".into(),
        name: "Test Event".into(),
        category: "Dev".into(),
        version: 1,
        inputs: vec![],
        outputs: vec![PinDescriptor::new("exec_out", "Exec", PinType::Exec)
            .with_doc("Fires once when the graph starts")],
        pure: false,
        realm: NodeRealm::Shared,
        deterministic: false,
        doc: Some("Entry point for a fixture graph".into()),
        preview: None,
    })?;
    reg.register(NodeDescriptor {
        id: "test_damage".into(),
        name: "Test Damage".into(),
        category: "Dev".into(),
        version: 1,
        inputs: vec![
            PinDescriptor::new("exec_in", "Exec", PinType::Exec),
            PinDescriptor::new("dps", "DPS", PinType::Float)
                .with_default(PropValue::Float(10.0))
                .with_doc("Damage applied per second inside the radius"),
            PinDescriptor::new("radius", "Radius", PinType::Float)
                .with_default(PropValue::Float(1.0))
                .with_doc("Effect radius in world units"),
        ],
        outputs: vec![
            PinDescriptor::new("exec_out", "Exec", PinType::Exec),
            PinDescriptor::new("hit_count", "Entities Hit", PinType::Float)
                .with_doc("How many entities the pulse reached"),
        ],
        pure: false,
        realm: NodeRealm::Shared,
        deterministic: true,
        doc: Some("Applies radial damage — fixture node, not a shipping ability".into()),
        preview: None,
    })?;
    reg.register(NodeDescriptor {
        id: "test_add".into(),
        name: "Add".into(),
        category: "Math".into(),
        version: 1,
        inputs: vec![
            PinDescriptor::new("a", "A", PinType::Float)
                .with_default(PropValue::Float(0.0))
                .with_doc("First addend"),
            PinDescriptor::new("b", "B", PinType::Float)
                .with_default(PropValue::Float(0.0))
                .with_doc("Second addend"),
        ],
        outputs: vec![
            PinDescriptor::new("sum", "Sum", PinType::Float).with_doc("a + b"),
        ],
        pure: true,
        realm: NodeRealm::Shared,
        deterministic: true,
        doc: Some("Adds two floats".into()),
        preview: None,
    })?;
    reg.register(NodeDescriptor {
        id: "test_editor_note".into(),
        name: "Editor Note".into(),
        category: "Dev".into(),
        version: 1,
        inputs: vec![PinDescriptor::new("exec_in", "Exec", PinType::Exec)],
        outputs: vec![],
        pure: false,
        realm: NodeRealm::Editor,
        deterministic: true,
        doc: Some("Prints a note to the editor console — Editor realm only".into()),
        preview: None,
    })?;
    Ok(())
}
