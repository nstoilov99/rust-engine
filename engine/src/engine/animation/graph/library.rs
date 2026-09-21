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
//! Registries live here per canvas context — which is how placement is
//! gated without a filter in sight: [`anim_machine_registry`] holds the
//! **machine** family (the state-machine scope), [`anim_pipeline_registry`]
//! the **pipeline** family (the root scope, Task 41.7), and
//! [`anim_rule_registry`] the **rule-region** library (ticket 05: the peek
//! canvas inside a transition) — the std comparison/math/logic nodes plus the
//! RESULT sink. A palette lists exactly what its scope's registry holds, so a
//! State can never land inside a rule and a Compare can never land on the
//! machine. [`anim_node_registry`] is the union of the machine and pipeline
//! families: the *document* is one flat list holding both, and validation,
//! migration and pin theming resolve against it.
//! (`var_get`/`reroute` are reserved doc-dependent types with no descriptor;
//! they arrive via the variables strip and wire gestures, and the rule
//! compiler's whitelist is what vouches for them.)

use node_graph_types::registry::{NodeDescriptor, NodeRegistry, PinDescriptor};
use node_graph_types::std_nodes::{
    std_node_descriptors, ADD_FLOAT, AND, COMPARE_FLOAT, DIV_FLOAT, MUL_FLOAT, NOT, OR, SUB_FLOAT,
};
use node_graph_types::{Edge, GraphDoc, GraphRealm, NodeInst, NodeRealm, PinType};

use super::pipeline::{
    ANIM_PIPE_LAYER_TYPE_ID, ANIM_PIPE_MACHINE_TYPE_ID, ANIM_PIPE_OUTPUT_TYPE_ID, LAYER_BASE_PIN,
    LAYER_LAYER_PIN, PIPELINE_ROW_STEP, PIPE_IN_PIN,
};
use super::plan::{
    ANIM_CLIP_TYPE_ID, ANIM_ENTRY_TYPE_ID, ANIM_IK_CHAIN_TYPE_ID, ANIM_PLAY_ONCE_TYPE_ID,
    ANIM_RULE_RESULT_TYPE_ID, ANIM_STATE_ALIAS_TYPE_ID, ANIM_STATE_TYPE_ID,
    ANIM_TRANSITION_TYPE_ID, POSE_PIN, RULE_RESULT_PIN, STATE_IN_PIN, STATE_OUT_PIN,
    TRANSITION_FROM_PIN, TRANSITION_TO_PIN, TRIGGER_PARAM_DOMAIN,
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
/// (STATE / ENTRY / ALIAS / SLOT / TRANS, and the pipeline's MACHINE / CLIP
/// / LAYER / OUTPUT), overriding the derived PURE/EVENT tags, which describe
/// exec flow and mean nothing in a domain that has none. A transition's tag
/// only shows on its unfolded (selected) card — the at-rest chip has no
/// header.
pub fn anim_node_tag(type_id: &str) -> Option<&'static str> {
    match type_id {
        ANIM_STATE_TYPE_ID => Some("STATE"),
        ANIM_ENTRY_TYPE_ID => Some("ENTRY"),
        ANIM_STATE_ALIAS_TYPE_ID => Some("ALIAS"),
        ANIM_PLAY_ONCE_TYPE_ID => Some("SLOT"),
        ANIM_IK_CHAIN_TYPE_ID => Some("IK"),
        ANIM_TRANSITION_TYPE_ID => Some("TRANS"),
        ANIM_PIPE_MACHINE_TYPE_ID => Some("MACHINE"),
        ANIM_CLIP_TYPE_ID => Some("CLIP"),
        ANIM_PIPE_LAYER_TYPE_ID => Some("LAYER"),
        ANIM_PIPE_OUTPUT_TYPE_ID => Some("OUTPUT"),
        // The rule canvas's one sink (mockup: "RESULT ◉ Bool").
        ANIM_RULE_RESULT_TYPE_ID => Some("RESULT"),
        _ => None,
    }
}

fn flow_pin(slug: &str, label: &str) -> PinDescriptor {
    PinDescriptor::new(slug, label, PinType::Domain(ANIM_FLOW_DOMAIN.to_string()))
}

fn pose_pin(slug: &str, label: &str) -> PinDescriptor {
    PinDescriptor::new(slug, label, PinType::Domain(ANIM_POSE_DOMAIN.to_string()))
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

/// The three animation domains, registered on every animation registry so a
/// wire resolves to the same hue whichever scope draws it.
fn register_domains(reg: &mut NodeRegistry) {
    // Machine topology is *flow*, not a pulled value: several transitions may
    // target one state (fan-in) and Idle → Locomotion → Idle is a legal
    // cycle, so `anim_flow` registers flow-like.
    reg.register_domain_pin_flow(ANIM_FLOW_DOMAIN, 2); // gold — Flow's slot
    reg.register_domain_pin_keyed(ANIM_POSE_DOMAIN, 11); // rose — animation's slot
    reg.register_domain_pin_keyed(TRIGGER_PARAM_DOMAIN, 0); // ember — Bool/Event family
}

/// The whole animation node library: machine and pipeline families. What the
/// editor validates, migrates and themes a document against.
pub fn anim_node_registry() -> NodeRegistry {
    let mut reg = NodeRegistry::new();
    register_domains(&mut reg);
    for d in machine_descriptors().into_iter().chain(pipeline_descriptors()) {
        reg.register(d).expect("anim library descriptors are valid");
    }
    reg
}

/// The machine scope's palette: entry, state, alias, transition.
pub fn anim_machine_registry() -> NodeRegistry {
    let mut reg = NodeRegistry::new();
    register_domains(&mut reg);
    for d in machine_descriptors() {
        reg.register(d).expect("anim machine descriptors are valid");
    }
    reg
}

/// The pipeline scope's palette (Task 41.7): pose sources, layers, play-once
/// overlays, IK chains, Output Pose.
pub fn anim_pipeline_registry() -> NodeRegistry {
    let mut reg = NodeRegistry::new();
    register_domains(&mut reg);
    for d in pipeline_descriptors() {
        reg.register(d).expect("anim pipeline descriptors are valid");
    }
    reg
}

fn machine_descriptors() -> Vec<NodeDescriptor> {
    vec![
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
            ANIM_STATE_ALIAS_TYPE_ID,
            "State Alias",
            "Stands for the states it lists (or all of them); its transitions apply from each.",
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
    ]
}

fn pipeline_descriptors() -> Vec<NodeDescriptor> {
    vec![
        desc(
            ANIM_PIPE_MACHINE_TYPE_ID,
            "State Machine",
            "A pose source: this document's state machine (Graph empty), or a nested \
             .animgraph's machine run as its own instance.",
            vec![],
            vec![pose_pin(POSE_PIN, "Pose")],
        ),
        desc(
            ANIM_CLIP_TYPE_ID,
            "Clip",
            "A looping clip as a pose source, with its own clock and event track.",
            vec![],
            vec![pose_pin(POSE_PIN, "Pose")],
        ),
        desc(
            ANIM_PIPE_LAYER_TYPE_ID,
            "Layered Blend Per Bone",
            "Blends Layer over Base on the bones under the mask roots, weighted by a \
             Float parameter. Base is evaluated first.",
            vec![pose_pin(LAYER_BASE_PIN, "Base"), pose_pin(LAYER_LAYER_PIN, "Layer")],
            vec![pose_pin(POSE_PIN, "Pose")],
        ),
        desc(
            ANIM_PLAY_ONCE_TYPE_ID,
            "Play-Once Slot",
            "Plays a clip over its input when its Trigger fires, then passes the input \
             through again. Bones masks it; empty = whole body.",
            vec![pose_pin(PIPE_IN_PIN, "In")],
            vec![pose_pin(POSE_PIN, "Pose")],
        ),
        desc(
            ANIM_IK_CHAIN_TYPE_ID,
            "IK Chain",
            "A model-space IK pass — two-bone (foot, hand) or look-at — faded by a Float \
             parameter. Foot mode adds ground raycasts, plant locking and pelvis drop.",
            vec![pose_pin(PIPE_IN_PIN, "In")],
            vec![pose_pin(POSE_PIN, "Pose")],
        ),
        desc(
            ANIM_PIPE_OUTPUT_TYPE_ID,
            "Output Pose",
            "The pipeline's single sink: what the character shows.",
            vec![pose_pin(PIPE_IN_PIN, "In")],
            vec![],
        ),
    ]
}

/// The std node types a rule region may contain — the editor-side face of the
/// rule compiler's whitelist ([`super::plan`]'s `RULE_NODE_TYPES`, minus the
/// reserved doc-dependent types `var_get`/`reroute`, which no registry may
/// hold).
const RULE_STD_TYPES: [&str; 8] = [
    COMPARE_FLOAT,
    ADD_FLOAT,
    SUB_FLOAT,
    MUL_FLOAT,
    DIV_FLOAT,
    AND,
    OR,
    NOT,
];

/// Build the rule-region library (ticket 05): what the palette offers inside
/// a transition's rule canvas. Std descriptors are reused verbatim — same
/// pins, same docs, same categories (Logic/Math) — plus the RESULT sink under
/// the Animation category, and the Trigger domain pin so a Trigger read's
/// wire resolves to its reserved ember hue.
pub fn anim_rule_registry() -> NodeRegistry {
    let mut reg = NodeRegistry::new();
    reg.register_domain_pin_keyed(TRIGGER_PARAM_DOMAIN, 0); // ember — Bool/Event family
    for d in std_node_descriptors()
        .into_iter()
        .filter(|d| RULE_STD_TYPES.contains(&d.id.as_str()))
    {
        reg.register(d).expect("std rule descriptors are valid");
    }
    reg.register(desc(
        ANIM_RULE_RESULT_TYPE_ID,
        "Result",
        "The rule's single Bool sink. Unwired means always-true (hollow dot).",
        vec![PinDescriptor::new(RULE_RESULT_PIN, "Value", PinType::Bool)],
        vec![],
    ))
    .expect("rule RESULT descriptor is valid");
    reg
}

/// The document a fresh `.animgraph` starts as: Client realm (the compiler's
/// authority requirement), an inline State Machine wired to Output Pose on
/// the pipeline canvas (the upgrade row's layout, [`PIPELINE_ROW_STEP`]
/// apart) and, inside it, an ENTRY already wired to an `Idle` state — the
/// same "teach the shape by seeding it" a fresh script graph gets from its
/// event + sink pair. Already v4-shaped, so opening it neither upgrades nor
/// dirties. The state names no clip yet, so the first thing the author sees
/// is the one anchored error that tells them what to do next.
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
            node(2, ANIM_PIPE_MACHINE_TYPE_ID, None, [0.0, 0.0]),
            node(3, ANIM_PIPE_OUTPUT_TYPE_ID, None, [PIPELINE_ROW_STEP, 0.0]),
        ],
        edges: vec![
            Edge {
                from_node: 0,
                from_pin: STATE_OUT_PIN.to_string(),
                to_node: 1,
                to_pin: STATE_IN_PIN.to_string(),
            },
            Edge {
                from_node: 2,
                from_pin: POSE_PIN.to_string(),
                to_node: 3,
                to_pin: PIPE_IN_PIN.to_string(),
            },
        ],
        ..GraphDoc::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::animation::graph::compile_anim_graph;
    use node_graph_types::validate_doc;

    const MACHINE: [&str; 4] = [
        ANIM_ENTRY_TYPE_ID,
        ANIM_STATE_TYPE_ID,
        ANIM_STATE_ALIAS_TYPE_ID,
        ANIM_TRANSITION_TYPE_ID,
    ];
    const PIPELINE: [&str; 6] = [
        ANIM_PIPE_MACHINE_TYPE_ID,
        ANIM_CLIP_TYPE_ID,
        ANIM_PIPE_LAYER_TYPE_ID,
        ANIM_PLAY_ONCE_TYPE_ID,
        ANIM_IK_CHAIN_TYPE_ID,
        ANIM_PIPE_OUTPUT_TYPE_ID,
    ];

    /// Both families register, the three domains resolve, and the palette
    /// grouping puts everything under one category.
    #[test]
    fn the_library_registers_the_machine_nodes() {
        let reg = anim_node_registry();
        for id in MACHINE.iter().chain(PIPELINE.iter()) {
            assert!(reg.get(id).is_some(), "{id}");
            assert_eq!(reg.get(id).unwrap().category, ANIM_CATEGORY);
            assert!(anim_node_tag(id).is_some(), "{id} wears a header tag");
        }
        // The palette is registry-driven: the legacy Any State is not in it.
        assert!(reg.get(super::super::plan::ANIM_ANY_STATE_TYPE_ID).is_none());
        assert_eq!(reg.get(ANIM_STATE_ALIAS_TYPE_ID).unwrap().name, "State Alias");
        assert_eq!(reg.domain_registration(ANIM_FLOW_DOMAIN), Some(Some(2)));
        assert_eq!(reg.domain_registration(ANIM_POSE_DOMAIN), Some(Some(11)));
        assert_eq!(reg.domain_registration(TRIGGER_PARAM_DOMAIN), Some(Some(0)));
    }

    /// Placement gating is registry membership (ticket 05): the rule library
    /// offers the std rule nodes and the RESULT sink and none of the machine
    /// nodes; the machine library offers none of the rule nodes. No filter
    /// exists to get out of sync.
    #[test]
    fn the_rule_library_and_the_machine_library_never_overlap() {
        let rules = anim_rule_registry();
        let machine = anim_node_registry();
        for id in RULE_STD_TYPES.iter().chain([ANIM_RULE_RESULT_TYPE_ID].iter()) {
            assert!(rules.get(id).is_some(), "rule registry misses {id}");
            assert!(machine.get(id).is_none(), "machine registry must not offer {id}");
        }
        for id in MACHINE.iter().chain(PIPELINE.iter()) {
            assert!(rules.get(id).is_none(), "rule registry must not offer {id}");
        }
        assert_eq!(
            rules.get(ANIM_RULE_RESULT_TYPE_ID).unwrap().category,
            ANIM_CATEGORY
        );
        assert_eq!(rules.domain_registration(TRIGGER_PARAM_DOMAIN), Some(Some(0)));
    }

    /// The two scopes partition the library (Task 41.7): the machine palette
    /// offers no pipeline node and vice versa, and the union is exactly the
    /// document registry — so a slot or IK chain can only be placed on the
    /// pipeline canvas, and the pipeline pins (Play Once / IK Chain in+out,
    /// Layer base+layer, Output in) are as the compiler expects.
    #[test]
    fn the_machine_and_pipeline_scopes_partition_the_library() {
        let machine = anim_machine_registry();
        let pipeline = anim_pipeline_registry();
        let all = anim_node_registry();
        for id in MACHINE {
            assert!(machine.get(id).is_some(), "{id}");
            assert!(pipeline.get(id).is_none(), "{id}");
        }
        for id in PIPELINE {
            assert!(pipeline.get(id).is_some(), "{id}");
            assert!(machine.get(id).is_none(), "{id}");
        }
        assert_eq!(all.iter().count(), MACHINE.len() + PIPELINE.len());
        for reg in [&machine, &pipeline, &all] {
            assert_eq!(reg.domain_registration(ANIM_POSE_DOMAIN), Some(Some(11)));
        }
        let pins = |id: &str| {
            let d = pipeline.get(id).unwrap();
            (
                d.inputs.iter().map(|p| p.slug.clone()).collect::<Vec<_>>(),
                d.outputs.iter().map(|p| p.slug.clone()).collect::<Vec<_>>(),
            )
        };
        assert_eq!(pins(ANIM_PIPE_MACHINE_TYPE_ID), (vec![], vec![POSE_PIN.to_string()]));
        assert_eq!(pins(ANIM_CLIP_TYPE_ID), (vec![], vec![POSE_PIN.to_string()]));
        assert_eq!(
            pins(ANIM_PIPE_LAYER_TYPE_ID),
            (
                vec![LAYER_BASE_PIN.to_string(), LAYER_LAYER_PIN.to_string()],
                vec![POSE_PIN.to_string()]
            )
        );
        for id in [ANIM_PLAY_ONCE_TYPE_ID, ANIM_IK_CHAIN_TYPE_ID] {
            assert_eq!(pins(id), (vec![PIPE_IN_PIN.to_string()], vec![POSE_PIN.to_string()]));
        }
        assert_eq!(pins(ANIM_PIPE_OUTPUT_TYPE_ID), (vec![PIPE_IN_PIN.to_string()], vec![]));
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

    /// The template is v4-shaped (Task 41.7 D6): SM → Output already on the
    /// pipeline row, so opening it upgrades nothing, and once the seeded
    /// state names a clip it compiles clean — no warnings, the inline
    /// machine as the root.
    #[test]
    fn a_fresh_animgraph_is_already_a_pipeline_root_and_compiles_without_warnings() {
        use super::super::pipeline::{needs_pipeline_root, upgrade_pipeline_root};
        use super::super::plan::{PlanPose, CLIP_PROP};
        use node_graph_types::PropValue;
        let mut doc = new_animgraph_doc();
        assert!(!needs_pipeline_root(&doc));
        assert!(!upgrade_pipeline_root(&mut doc), "nothing to upgrade");
        assert_eq!(doc.version, node_graph_types::GRAPH_DOC_VERSION);
        let sm = doc.nodes.iter().find(|n| n.type_id == ANIM_PIPE_MACHINE_TYPE_ID).unwrap();
        let out = doc.nodes.iter().find(|n| n.type_id == ANIM_PIPE_OUTPUT_TYPE_ID).unwrap();
        assert_eq!(sm.position, [0.0, 0.0]);
        assert_eq!(out.position, [PIPELINE_ROW_STEP, 0.0]);
        doc.node_mut(1)
            .unwrap()
            .properties
            .insert(CLIP_PROP.to_string(), PropValue::Asset("anims/idle.anim".into()));
        let compiled = compile_anim_graph(&doc).expect("the seeded document compiles");
        assert!(compiled.warnings.is_empty(), "{:?}", compiled.warnings);
        assert_eq!(compiled.plan.pipeline.root, PlanPose::Machine(0));
        assert_eq!(compiled.plan.machines.len(), 1);
    }
}
