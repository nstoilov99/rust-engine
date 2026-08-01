//! Tests for the node graph derive macros (Task 40 P8).
//!
//! These live in the engine crate (not the proc-macro crate) so the macro
//! output is checked against the real runtime types. Compile-failure cases
//! (unknown pin types, bad realm) are deliberately out of scope here — no
//! `trybuild`.

use super::auto_register::register_inventory_nodes;
use super::doc::{NodeRealm, PinType, PropValue};
use super::markers::ExecPin;
use super::registry::{NodeDescriptor, NodeRegistry, PinDescriptor};
use node_graph_macros::{AnimationNode, ScriptNode};

/// Representative node: exec pins, a defaulted input, custom + defaulted
/// labels, an impure classification.
#[derive(ScriptNode)]
#[node(
    id = "damage_zone",
    name = "Damage Zone",
    category = "Gameplay",
    pure = false,
    realm = "shared",
    deterministic = true,
    version = 1,
    doc = "Applies radial damage over time"
)]
#[allow(dead_code)]
pub struct DamageZone {
    #[input(pin = "exec")]
    exec_in: ExecPin,
    #[input(label = "DPS", default = 10.0, doc = "Damage per second")]
    dps: f32,
    #[input(label = "Radius")]
    radius: f32,
    #[output(pin = "exec")]
    exec_out: ExecPin,
    #[output(label = "Entities Hit", doc = "Entities the pulse reached")]
    hit_count: f32,
}

fn hand_written_damage_zone() -> NodeDescriptor {
    NodeDescriptor {
        id: "damage_zone".into(),
        name: "Damage Zone".into(),
        category: "Gameplay".into(),
        version: 1,
        inputs: vec![
            PinDescriptor::new("exec_in", "Exec In", PinType::Exec),
            PinDescriptor::new("dps", "DPS", PinType::Float)
                .with_default(PropValue::Float(10.0))
                .with_doc("Damage per second"),
            PinDescriptor::new("radius", "Radius", PinType::Float),
        ],
        outputs: vec![
            PinDescriptor::new("exec_out", "Exec Out", PinType::Exec),
            PinDescriptor::new("hit_count", "Entities Hit", PinType::Float)
                .with_doc("Entities the pulse reached"),
        ],
        pure: false,
        realm: NodeRealm::Shared,
        deterministic: true,
        doc: Some("Applies radial damage over time".into()),
        preview: None,
    }
}

#[test]
fn generated_descriptor_equals_hand_written() {
    assert_eq!(DamageZone::descriptor(), hand_written_damage_zone());
}

/// Exercises the type-mapping table: f32, bool, and fixed-size f32 arrays.
#[derive(ScriptNode)]
#[node(
    id = "type_map_probe",
    name = "Type Map Probe",
    category = "Test",
    pure = true,
    realm = "client",
    deterministic = true
)]
#[allow(dead_code)]
pub struct TypeMapProbe {
    #[input(label = "Scalar")]
    scalar: f32,
    #[input(label = "Flag")]
    flag: bool,
    #[input(label = "Point")]
    point: [f32; 2],
    #[output(label = "Direction")]
    direction: [f32; 3],
    #[output(label = "Rgba")]
    rgba: [f32; 4],
}

#[test]
fn type_mapping_and_default_version() {
    let d = TypeMapProbe::descriptor();
    assert_eq!(d.version, 1); // version omitted -> default 1
    assert_eq!(d.realm, NodeRealm::Client);
    assert_eq!(d.input("scalar").unwrap().ty, PinType::Float);
    assert_eq!(d.input("flag").unwrap().ty, PinType::Bool);
    assert_eq!(d.input("point").unwrap().ty, PinType::Vec2);
    assert_eq!(d.output("direction").unwrap().ty, PinType::Vec3);
    assert_eq!(d.output("rgba").unwrap().ty, PinType::Vec4);
}

/// The only `auto_register` node in this test binary, so
/// `register_inventory_nodes` yields exactly it. Pure + no exec pins so it
/// passes the registry's descriptor invariants.
#[derive(ScriptNode)]
#[node(
    id = "auto_probe",
    name = "Auto Probe",
    category = "Test",
    pure = true,
    realm = "shared",
    deterministic = true,
    auto_register
)]
#[allow(dead_code)]
pub struct AutoProbe {
    #[input(label = "In", default = 1.0)]
    value: f32,
    #[output(label = "Out")]
    result: f32,
}

#[test]
fn auto_register_collects_node() {
    let mut reg = NodeRegistry::new();
    register_inventory_nodes(&mut reg).unwrap();
    let d = reg.get("auto_probe").expect("auto_probe registered via inventory");
    assert_eq!(d.name, "Auto Probe");
    assert_eq!(d.input("value").unwrap().default, Some(PropValue::Float(1.0)));
}

/// Proves the domain-macro layering: `AnimationNode` shares the same core and
/// produces a descriptor. No animation nodes ship in Task 40.
#[derive(AnimationNode)]
#[node(
    id = "anim_probe",
    name = "Anim Probe",
    category = "Animation",
    pure = true,
    realm = "shared",
    deterministic = true
)]
#[allow(dead_code)]
pub struct AnimProbe {
    #[input(label = "Weight", default = 0.5)]
    weight: f32,
    #[output(label = "Pose")]
    pose: f32,
}

#[test]
fn animation_node_derive_produces_descriptor() {
    let d = AnimProbe::descriptor();
    assert_eq!(d.id, "anim_probe");
    assert_eq!(d.category, "Animation");
    assert_eq!(d.input("weight").unwrap().default, Some(PropValue::Float(0.5)));
}
