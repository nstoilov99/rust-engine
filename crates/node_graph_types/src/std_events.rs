//! Event entry nodes — the framework-owned seed of the `std_nodes` module
//! (Task 45-A D3/D5).
//!
//! These are ordinary registry descriptors, not reserved types: they are
//! staged by the `graph_scripting` plugin (45-A P5), which is why nothing
//! here registers itself. `event_custom` is the one doc-dependent member —
//! its payload pins come from the instance's properties, so the pins you see
//! on the canvas come from [`crate::descriptors::DocDescriptors`], not from
//! the descriptor below.
//!
//! Every event node is impure with exec outputs only: an entry point has
//! nothing to be continued *from*.

use crate::doc::{NodeRealm, PinType, PropValue};
use crate::registry::{NodeDescriptor, NodeRegistry, PinDescriptor, RegistryError, EXEC_OUT_PIN};

pub const EVENT_BEGIN_PLAY_TYPE_ID: &str = "event_begin_play";
pub const EVENT_TICK_TYPE_ID: &str = "event_tick";
pub const EVENT_CUSTOM_TYPE_ID: &str = "event_custom";
pub const EVENT_INPUT_ACTION_TYPE_ID: &str = "event_input_action";

/// Reserved property key on `event_custom`: the event's name, a
/// [`PropValue::Str`]. Custom events are same-entity scoped in v1, so the
/// name only has to be unique within one graph.
pub const EVENT_NAME_PROP: &str = "event_name";

/// Reserved property-key prefix on `event_custom`: `payload.<slug>` declares
/// one payload **output** pin named `<slug>`, whose value is a
/// [`PropValue::Enum`] holding the pin type's
/// [`PinType::type_slug`](crate::doc::PinType::type_slug).
///
/// A payload pin carries no constant — it is an output — so the property
/// stores the *type* rather than a default. The `Enum` spelling is deliberate:
/// the legal set is [`PAYLOAD_PIN_TYPES`], so the P6 property editor gets a
/// dropdown with no extra machinery.
pub const EVENT_PAYLOAD_PREFIX: &str = "payload.";

/// Reserved property key on `event_input_action`: the Task 33 input action
/// name, a [`PropValue::Str`].
pub const EVENT_ACTION_PROP: &str = "action";

/// The pin types a custom event payload may declare in v1. Deliberately the
/// scalar set: `Exec` is not data, and array payloads wait until arrays have
/// a literal editor (D9).
pub const PAYLOAD_PIN_TYPES: [&str; 9] = [
    "float", "int", "bool", "string", "vec2", "vec3", "vec4", "color", "entity",
];

/// One phase of a tick's event drain.
///
/// The order below is fixed by design (45-A D3), not implementation-defined,
/// and is stated here so the interpreter (P2/P4) and its tests consume one
/// spelling of it:
///
/// > Per-instance FIFO queue; emitting during a tick queues for the *next*
/// > tick (no reentrancy); within one tick the drain order is BeginPlay
/// > (exactly once per instance lifetime) → due latents (by due time, then
/// > activation id) → input-action events (input order) → custom events
/// > (FIFO) → Tick. Multiple entry nodes for the same event all fire, in doc
/// > order. Custom events are same-entity scoped in v1.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum EventPhase {
    /// Exactly once per instance lifetime.
    BeginPlay,
    /// Suspended activations whose due time has passed, ordered by due time
    /// then activation id.
    Latent,
    /// Task 33 input actions, in input order.
    InputAction,
    /// Custom events, FIFO within the tick.
    Custom,
    /// Every playing tick, last.
    Tick,
}

/// The drain order, in order. `EventPhase`'s `Ord` matches this array by
/// construction (the test below pins that), so a scheduler may sort by the
/// enum instead of scanning the array.
pub const EVENT_DRAIN_ORDER: [EventPhase; 5] = [
    EventPhase::BeginPlay,
    EventPhase::Latent,
    EventPhase::InputAction,
    EventPhase::Custom,
    EventPhase::Tick,
];

impl EventPhase {
    /// Which phase delivers a given event node type. `None` for anything
    /// that is not an entry node.
    pub fn of_type(type_id: &str) -> Option<EventPhase> {
        Some(match type_id {
            EVENT_BEGIN_PLAY_TYPE_ID => EventPhase::BeginPlay,
            EVENT_TICK_TYPE_ID => EventPhase::Tick,
            EVENT_CUSTOM_TYPE_ID => EventPhase::Custom,
            EVENT_INPUT_ACTION_TYPE_ID => EventPhase::InputAction,
            _ => return None,
        })
    }
}

/// The event entry descriptors, in registration order. Single source of truth
/// for both [`register_std_events`] and the 45-A P5 plugin's staged path.
pub fn std_event_descriptors() -> Vec<NodeDescriptor> {
    let entry = |id: &str, name: &str, doc: &str, outputs: Vec<PinDescriptor>| NodeDescriptor {
        id: id.to_string(),
        name: name.to_string(),
        category: "Event".to_string(),
        version: 1,
        inputs: vec![],
        outputs,
        pure: false,
        realm: NodeRealm::Shared,
        deterministic: true,
        doc: Some(doc.to_string()),
        preview: None,
    };
    let exec_out = || PinDescriptor::new(EXEC_OUT_PIN, "Exec", PinType::Exec);
    vec![
        entry(
            EVENT_BEGIN_PLAY_TYPE_ID,
            "Begin Play",
            "Fires once, the first playing tick after the instance is created",
            vec![exec_out()],
        ),
        entry(
            EVENT_TICK_TYPE_ID,
            "Tick",
            "Fires every playing tick, after every other event phase",
            vec![
                exec_out(),
                PinDescriptor::new("dt", "Delta Time", PinType::Float)
                    .with_doc("Seconds since the previous tick"),
            ],
        ),
        entry(
            EVENT_CUSTOM_TYPE_ID,
            "Custom Event",
            "Fires when this graph emits the named event — same entity only in v1",
            vec![exec_out()],
        ),
        entry(
            EVENT_INPUT_ACTION_TYPE_ID,
            "Input Action",
            "Fires when the named input action triggers",
            vec![exec_out()],
        ),
    ]
}

/// Register the event set directly. Used by the framework's own tests; the
/// engine reaches these through the 45-A P5 plugin instead, which stages the
/// same descriptors.
pub fn register_std_events(reg: &mut NodeRegistry) -> Result<(), RegistryError> {
    for desc in std_event_descriptors() {
        reg.register(desc)?;
    }
    Ok(())
}

/// The payload pins an `event_custom` instance declares, in property-key
/// order (which is slug order — `properties` is a `BTreeMap`, so the pin
/// order on the canvas is stable). A `payload.<slug>` whose value is not a
/// recognized type slug is skipped: stale data to fix, never a guessed pin.
pub fn payload_pins(properties: &std::collections::BTreeMap<String, PropValue>) -> Vec<PinDescriptor> {
    properties
        .iter()
        .filter_map(|(k, v)| {
            let slug = k.strip_prefix(EVENT_PAYLOAD_PREFIX)?;
            let ty = match v {
                PropValue::Enum(t) | PropValue::Str(t) => PinType::from_type_slug(t)?,
                _ => return None,
            };
            Some(PinDescriptor::new(slug, slug, ty))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// Entry nodes are impure with exec outputs only — an entry point has
    /// nothing to be continued from — and they register cleanly.
    #[test]
    fn event_descriptors_are_exec_output_only_entries() {
        let mut reg = NodeRegistry::new();
        register_std_events(&mut reg).unwrap();
        for d in std_event_descriptors() {
            assert!(!d.pure, "{} must be impure", d.id);
            assert!(
                d.inputs.iter().all(|p| p.ty != PinType::Exec),
                "{} must have no exec input",
                d.id
            );
            assert!(
                d.outputs.iter().any(|p| p.ty == PinType::Exec),
                "{} must have an exec output",
                d.id
            );
            assert!(EventPhase::of_type(&d.id).is_some(), "{} has no phase", d.id);
            assert!(reg.get(&d.id).is_some());
        }
        assert_eq!(
            reg.get(EVENT_TICK_TYPE_ID).unwrap().output("dt").unwrap().ty,
            PinType::Float
        );
    }

    /// The drain order is the documented one, and `Ord` agrees with it so a
    /// scheduler may sort by the enum.
    #[test]
    fn drain_order_is_the_documented_one() {
        assert_eq!(
            EVENT_DRAIN_ORDER,
            [
                EventPhase::BeginPlay,
                EventPhase::Latent,
                EventPhase::InputAction,
                EventPhase::Custom,
                EventPhase::Tick,
            ]
        );
        let mut sorted = EVENT_DRAIN_ORDER;
        sorted.reverse();
        sorted.sort();
        assert_eq!(sorted, EVENT_DRAIN_ORDER, "Ord must match the drain order");
    }

    #[test]
    fn payload_pins_come_from_properties_and_skip_stale_data() {
        let mut props = BTreeMap::new();
        props.insert("payload.amount".to_string(), PropValue::Enum("int".into()));
        props.insert("payload.who".to_string(), PropValue::Enum("entity".into()));
        props.insert("payload.bad".to_string(), PropValue::Enum("nope".into()));
        props.insert("event_name".to_string(), PropValue::Str("Hit".into()));
        let pins = payload_pins(&props);
        assert_eq!(
            pins.iter().map(|p| (p.slug.as_str(), p.ty.clone())).collect::<Vec<_>>(),
            vec![("amount", PinType::Int), ("who", PinType::Entity)],
            "slug order, and an unrecognized type slug is skipped not guessed"
        );
        // Every advertised payload type resolves.
        for t in PAYLOAD_PIN_TYPES {
            assert!(PinType::from_type_slug(t).is_some(), "{t}");
        }
    }
}
