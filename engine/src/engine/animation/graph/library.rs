//! The animation node library's registry (Task 41, editor authoring).
//!
//! The `.animgraph` compiler is registry-free by design — [`super::plan`]
//! matches type ids directly — but the *editor* needs descriptors: the
//! palette lists them, validation checks pins against them, and the theme
//! resolves the domain pin colors registered here. The library gets its own
//! [`NodeRegistry`] rather than a place in the script registry, which is what
//! "the node library filters by graph type" (DESIGN-nodegraph) means in
//! practice: a script palette never offers a State, an animation palette
//! never offers a Print, and neither needs a filter to stay that way.
//!
//! Only the **machine-level** nodes are registered — the five things placeable
//! on the state-machine canvas. The region libraries (rule nodes inside a
//! transition, blend-tree nodes inside a state) become placeable when the
//! region canvas ships (ticket 05); their *domain pins* are registered now so
//! Pose and Trigger already resolve to their reserved hues everywhere.

use node_graph_types::registry::{NodeDescriptor, NodeRegistry, PinDescriptor};
use node_graph_types::{Edge, GraphDoc, GraphRealm, NodeInst, NodeRealm, PinType};

use super::plan::{
    ANIM_ANY_STATE_TYPE_ID, ANIM_ENTRY_TYPE_ID, ANIM_PLAY_ONCE_TYPE_ID, ANIM_STATE_TYPE_ID,
    ANIM_TRANSITION_TYPE_ID, STATE_IN_PIN, STATE_OUT_PIN, TRANSITION_FROM_PIN, TRANSITION_TO_PIN,
    TRIGGER_PARAM_DOMAIN,
};

/// The machine-topology wire: state → transition → state. Not a Pose and not
/// an Exec — its own domain, keyed to **gold** (ramp 2), the Flow category's
/// slot: machine topology *is* this library's control flow, and the mockup
/// draws transitions in the Flow family.
pub const ANIM_FLOW_DOMAIN: &str = "anim_flow";

/// The Pose wire inside a state's blend tree. Keyed to **rose** (ramp 11),
/// pairing with the `animation` asset slot — same concept, same hue, the
/// mesh-pin/geometry-asset rule.
pub const ANIM_POSE_DOMAIN: &str = "anim_pose";

/// The category every machine node lists under (palette grouping + the 2px
/// node edge, which takes the same rose slot the asset kind owns).
pub const ANIM_CATEGORY: &str = "Animation";

/// The 9px mono header tag for an animation node — the mockup's vocabulary
/// (STATE / ENTRY / ANY / SLOT), overriding the derived PURE/EVENT tags,
/// which describe exec flow and mean nothing in a domain that has none.
/// Transitions return `None`: they render as chips, which have no header.
pub fn anim_node_tag(type_id: &str) -> Option<&'static str> {
    match type_id {
        ANIM_STATE_TYPE_ID => Some("STATE"),
        ANIM_ENTRY_TYPE_ID => Some("ENTRY"),
        ANIM_ANY_STATE_TYPE_ID => Some("ANY"),
        ANIM_PLAY_ONCE_TYPE_ID => Some("SLOT"),
        _ => None,
    }
}

fn flow_pin(slug: &str, label: &str) -> PinDescriptor {
    PinDescriptor::new(slug, label, PinType::Domain(ANIM_FLOW_DOMAIN.to_string()))
}

fn desc(
    id: &str,
    name: &str,
    doc: &str,
    inputs: Vec<PinDescriptor>,
    outputs: Vec<PinDescriptor>,
) -> NodeDescriptor {
    NodeDescriptor {
        id: id.to_string(),
        name: name.to_string(),
        category: ANIM_CATEGORY.to_string(),
        version: 1,
        inputs,
        outputs,
        // The whole library is pin-pure: no exec pins exist in this domain
        // (the registry invariant "impure requires exec" forces `true`), and
        // the PURE header tag is suppressed by [`anim_node_tag`].
        pure: true,
        realm: NodeRealm::Client,
        deterministic: true,
        doc: Some(doc.to_string()),
        preview: None,
    }
}

/// Build the animation node library. Machine nodes only — see module docs.
pub fn anim_node_registry() -> NodeRegistry {
    let mut reg = NodeRegistry::new();
    // Machine topology is *flow*, not a pulled value: several transitions may
    // target one state (fan-in) and Idle → Locomotion → Idle is a legal
    // cycle, so `anim_flow` registers flow-like.
    reg.register_domain_pin_flow(ANIM_FLOW_DOMAIN, 2); // gold — Flow's slot
    reg.register_domain_pin_keyed(ANIM_POSE_DOMAIN, 11); // rose — animation's slot
    reg.register_domain_pin_keyed(TRIGGER_PARAM_DOMAIN, 0); // ember — Bool/Event family

    let all = [
        desc(
            ANIM_ENTRY_TYPE_ID,
            "Entry",
            "Where the machine starts: exactly one per graph, wired to the starting state.",
            vec![],
            vec![flow_pin(STATE_OUT_PIN, "Start")],
        ),
        desc(
            ANIM_STATE_TYPE_ID,
            "State",
            "A character mode (Idle, Locomotion). Plays the clip its Clip names.",
            vec![flow_pin(STATE_IN_PIN, "In")],
            vec![flow_pin(STATE_OUT_PIN, "Out")],
        ),
        desc(
            ANIM_ANY_STATE_TYPE_ID,
            "Any State",
            "Its outgoing transitions apply from whatever state is active.",
            vec![],
            vec![flow_pin(STATE_OUT_PIN, "Out")],
        ),
        desc(
            ANIM_TRANSITION_TYPE_ID,
            "Transition",
            "A crossfade between two states; its rule decides when it fires.",
            vec![flow_pin(TRANSITION_FROM_PIN, "From")],
            vec![flow_pin(TRANSITION_TO_PIN, "To")],
        ),
        desc(
            ANIM_PLAY_ONCE_TYPE_ID,
            "Play-Once Slot",
            "Plays a clip over the base result when its Trigger fires, then returns.",
            vec![],
            vec![],
        ),
    ];
    for d in all {
        reg.register(d).expect("anim library descriptors are valid");
    }
    reg
}

/// The document a fresh `.animgraph` starts as: Client realm (the compiler's
/// authority requirement), an ENTRY already wired to an `Idle` state — the
/// same "teach the shape by seeding it" a fresh script graph gets from its
/// event + sink pair. The state names no clip yet, so the first thing the
/// author sees is the one anchored error that tells them what to do next.
pub fn new_animgraph_doc() -> GraphDoc {
    let node = |id: u64, type_id: &str, title: Option<&str>, position: [f32; 2]| NodeInst {
        id,
        type_id: type_id.to_string(),
        type_version: 1,
        position,
        properties: Default::default(),
        subgraph: None,
        tint: None,
        title: title.map(str::to_string),
    };
    GraphDoc {
        realm: GraphRealm::Client,
        nodes: vec![
            node(0, ANIM_ENTRY_TYPE_ID, None, [-220.0, 40.0]),
            node(1, ANIM_STATE_TYPE_ID, Some("Idle"), [60.0, 40.0]),
        ],
        edges: vec![Edge {
            from_node: 0,
            from_pin: STATE_OUT_PIN.to_string(),
            to_node: 1,
            to_pin: STATE_IN_PIN.to_string(),
        }],
        ..GraphDoc::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::animation::graph::compile_anim_graph;
    use node_graph_types::validate_doc;

    /// The five machine nodes register, the three domains resolve, and the
    /// palette grouping puts everything under one category.
    #[test]
    fn the_library_registers_the_machine_nodes() {
        let reg = anim_node_registry();
        for id in [
            ANIM_ENTRY_TYPE_ID,
            ANIM_STATE_TYPE_ID,
            ANIM_ANY_STATE_TYPE_ID,
            ANIM_TRANSITION_TYPE_ID,
            ANIM_PLAY_ONCE_TYPE_ID,
        ] {
            assert!(reg.get(id).is_some(), "{id}");
            assert_eq!(reg.get(id).unwrap().category, ANIM_CATEGORY);
        }
        assert_eq!(reg.domain_registration(ANIM_FLOW_DOMAIN), Some(Some(2)));
        assert_eq!(reg.domain_registration(ANIM_POSE_DOMAIN), Some(Some(11)));
        assert_eq!(reg.domain_registration(TRIGGER_PARAM_DOMAIN), Some(Some(0)));
    }

    /// A fresh document validates clean against the library — its one
    /// remaining problem (the seeded state names no clip) is the *compiler's*
    /// anchored refusal, by design, so the author is guided rather than
    /// blocked.
    #[test]
    fn a_fresh_animgraph_validates_and_refuses_on_the_missing_clip() {
        let doc = new_animgraph_doc();
        let reg = anim_node_registry();
        assert!(validate_doc(&doc, &reg).is_empty());
        let err = compile_anim_graph(&doc).unwrap_err();
        assert!(err.contains("state 'Idle'"), "{err}");
        assert!(err.contains("clip"), "{err}");
    }
}
