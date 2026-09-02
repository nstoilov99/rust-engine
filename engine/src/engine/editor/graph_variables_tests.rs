//! The variables model (Task 45-A P6b): declarations, their edits, and above
//! all their **undo behavior**.
//!
//! Gesture × undo interleaving is the high-yield bug surface here, so the
//! tests lean on one invariant rather than on inspection: after any sequence
//! of edits, undoing everything must produce a document that serializes
//! byte-identically to the one before the sequence, and redoing everything
//! must reproduce the after-state byte-identically. Canonical serialization
//! makes that a real check — it catches order drift a field-by-field
//! comparison would miss.

use std::collections::BTreeMap;

use crate::engine::editor::graph_editor::{test_state, variable_slug, GraphEditorState};
use crate::engine::node_graph::{
    serialize_graph, validate_doc, Edge, GraphError, NodeInst, NodeRegistry, PinType, PropValue,
    VAR_GET_TYPE_ID, VAR_PROP, VAR_SET_TYPE_ID, VAR_VALUE_PIN,
};

fn registry() -> NodeRegistry {
    let mut reg = NodeRegistry::new();
    node_graph_types::register_std_nodes(&mut reg).unwrap();
    node_graph_types::register_std_events(&mut reg).unwrap();
    reg
}

fn state() -> GraphEditorState {
    test_state("graphs/test.graph")
}

fn var_node(id: u64, type_id: &str, slug: &str) -> NodeInst {
    let mut props = BTreeMap::new();
    props.insert(VAR_PROP.to_string(), PropValue::Str(slug.to_string()));
    NodeInst {
        id,
        type_id: type_id.to_string(),
        type_version: 1,
        position: [id as f32 * 200.0, 0.0],
        properties: props,
        subgraph: None,
        tint: None,
        title: None,
    }
}

fn ron(st: &GraphEditorState) -> String {
    serialize_graph(&st.doc).expect("serialize")
}

// ---------------------------------------------------------------------------
// Slugs
// ---------------------------------------------------------------------------

/// Slugs are snake_case and forever; collisions take a numeric suffix, the
/// same disambiguation interface pins and generated subgraph paths use.
#[test]
fn slugs_are_snake_case_and_unique() {
    assert_eq!(variable_slug("Player Health"), "player_health");
    assert_eq!(variable_slug("  spaced  out  "), "spaced_out");
    assert_eq!(variable_slug("Ammo (9mm)!"), "ammo_9mm");
    assert_eq!(variable_slug("HP"), "hp");
    assert_eq!(variable_slug("!!!"), "var", "an unnameable name still gets a name");
    assert_eq!(variable_slug(""), "var");

    let reg = registry();
    let mut st = state();
    assert_eq!(st.add_variable("Score", PinType::Int, &reg), "score");
    assert_eq!(st.add_variable("Score", PinType::Float, &reg), "score_2");
    assert_eq!(st.add_variable("score", PinType::Bool, &reg), "score_3");
    assert_eq!(st.doc.variables.len(), 3);
    // The label keeps what the author typed; only the slug is normalized.
    assert_eq!(st.doc.variable("score_3").unwrap().label, "score");
}

/// A new declaration starts at its type's zero, and the types with no
/// constant form start at nothing rather than at a lie.
#[test]
fn a_new_variable_starts_at_its_types_zero() {
    let reg = registry();
    let mut st = state();
    for (ty, want) in [
        (PinType::Int, Some(PropValue::Int(0))),
        (PinType::Float, Some(PropValue::Float(0.0))),
        (PinType::Bool, Some(PropValue::Bool(false))),
        (PinType::String, Some(PropValue::Str(String::new()))),
        (PinType::Vec3, Some(PropValue::Vec3([0.0; 3]))),
        (PinType::Array(Box::new(PinType::Int)), Some(PropValue::Array(Vec::new()))),
        (PinType::Entity, None),
    ] {
        let slug = st.add_variable("v", ty.clone(), &reg);
        assert_eq!(st.doc.variable(&slug).unwrap().default, want, "{ty:?}");
    }
}

// ---------------------------------------------------------------------------
// Retype
// ---------------------------------------------------------------------------

/// **The retype-default rule**: a default survives only if it already holds
/// the new type's shape. No coercion — turning 2.7 into 2 would change the
/// author's value without saying so.
#[test]
fn retype_keeps_a_compatible_default_and_resets_the_rest() {
    let reg = registry();
    let mut st = state();

    let slug = st.add_variable("N", PinType::Float, &reg);
    st.set_variable_default(&slug, Some(PropValue::Float(2.7)), &reg);
    st.flush_var_default_edit(&reg);
    assert_eq!(st.doc.variable(&slug).unwrap().default, Some(PropValue::Float(2.7)));

    // Float -> Int: incompatible, so it resets rather than truncating.
    assert!(st.retype_variable(&slug, PinType::Int, &reg));
    assert_eq!(
        st.doc.variable(&slug).unwrap().default,
        Some(PropValue::Int(0)),
        "no silent truncation"
    );

    // Int -> Int is not a change at all.
    assert!(!st.retype_variable(&slug, PinType::Int, &reg));

    // A compatible retype keeps the value: Array(Int) -> Array(Float) is the
    // same stored shape, so the (empty) list survives.
    let arr = st.add_variable("Xs", PinType::Array(Box::new(PinType::Int)), &reg);
    st.set_variable_default(&arr, Some(PropValue::Array(vec![PropValue::Int(1)])), &reg);
    st.flush_var_default_edit(&reg);
    assert!(st.retype_variable(&arr, PinType::Array(Box::new(PinType::Float)), &reg));
    assert_eq!(
        st.doc.variable(&arr).unwrap().default,
        Some(PropValue::Array(vec![PropValue::Int(1)])),
        "the stored shape still fits, so it is kept"
    );

    // …and undo puts the original type *and* the original default back.
    st.undo(&reg);
    assert_eq!(st.doc.variable(&arr).unwrap().ty, PinType::Array(Box::new(PinType::Int)));
}

/// Retyping a variable with live nodes re-types their pins through
/// `DocDescriptors`, so a wire that no longer fits surfaces as a
/// `TypeMismatch` — and undo clears it.
#[test]
fn retype_breaks_a_wired_edge_and_undo_restores_it() {
    let reg = registry();
    let mut st = state();
    let slug = st.add_variable("Score", PinType::Int, &reg);

    // var_get(score) -> add_int.a, which only accepts Int.
    st.doc.nodes = vec![
        var_node(0, VAR_GET_TYPE_ID, &slug),
        NodeInst {
            id: 1,
            type_id: node_graph_types::std_nodes::ADD_INT.to_string(),
            type_version: 1,
            position: [200.0, 0.0],
            properties: BTreeMap::new(),
            subgraph: None,
            tint: None,
            title: None,
        },
    ];
    st.doc.edges = vec![Edge {
        from_node: 0,
        from_pin: VAR_VALUE_PIN.to_string(),
        to_node: 1,
        to_pin: "a".to_string(),
    }];
    assert_eq!(validate_doc(&st.doc, &reg), vec![], "Int into an Int pin is clean");

    let before = ron(&st);
    assert!(st.retype_variable(&slug, PinType::String, &reg));
    let errs = validate_doc(&st.doc, &reg);
    assert!(
        errs.iter().any(|e| matches!(
            e,
            GraphError::TypeMismatch { from_ty: PinType::String, to_ty: PinType::Int, .. }
        )),
        "the pin re-typed under the wire: {errs:?}"
    );

    st.undo(&reg);
    assert_eq!(validate_doc(&st.doc, &reg), vec![], "undo clears the mismatch");
    assert_eq!(ron(&st), before, "and restores the document byte-for-byte");
}

// ---------------------------------------------------------------------------
// Delete
// ---------------------------------------------------------------------------

/// Deleting a used variable leaves its nodes in place as `UnknownVariable`
/// errors — the honest degradation — and undo restores both the declaration
/// and the document's exact bytes.
#[test]
fn deleting_a_used_variable_degrades_then_undoes_cleanly() {
    let reg = registry();
    let mut st = state();
    let slug = st.add_variable("Score", PinType::Int, &reg);
    st.doc.nodes = vec![
        var_node(0, VAR_GET_TYPE_ID, &slug),
        var_node(1, VAR_SET_TYPE_ID, &slug),
        var_node(2, VAR_GET_TYPE_ID, "other"),
    ];

    assert_eq!(st.variable_usage_count(&slug), 2, "what the confirmation counts");
    assert_eq!(st.variable_usage_count("nobody"), 0);

    let before = ron(&st);
    assert!(st.remove_variable(&slug, &reg));

    let unknown: Vec<u64> = validate_doc(&st.doc, &reg)
        .into_iter()
        .filter_map(|e| match e {
            GraphError::UnknownVariable { node, slug: s } if s == slug => Some(node),
            _ => None,
        })
        .collect();
    assert_eq!(unknown, vec![0, 1], "both nodes report, and neither was deleted");
    assert_eq!(st.doc.nodes.len(), 3, "an author's nodes are not silently removed");

    st.undo(&reg);
    assert!(!validate_doc(&st.doc, &reg)
        .iter()
        .any(|e| matches!(e, GraphError::UnknownVariable { slug: s, .. } if *s == slug)));
    assert_eq!(ron(&st), before, "byte-identical after undo");
}

/// Removal restores to its **original index**, so a delete in the middle of
/// the list does not quietly reorder the document.
#[test]
fn undoing_a_delete_restores_declaration_order() {
    let reg = registry();
    let mut st = state();
    for n in ["A", "B", "C"] {
        st.add_variable(n, PinType::Int, &reg);
    }
    let before = ron(&st);
    st.remove_variable("b", &reg);
    assert_eq!(
        st.doc.variables.iter().map(|v| v.slug.as_str()).collect::<Vec<_>>(),
        vec!["a", "c"]
    );
    st.undo(&reg);
    assert_eq!(
        st.doc.variables.iter().map(|v| v.slug.as_str()).collect::<Vec<_>>(),
        vec!["a", "b", "c"],
        "back in the middle, not appended"
    );
    assert_eq!(ron(&st), before);
}

// ---------------------------------------------------------------------------
// Coalescing
// ---------------------------------------------------------------------------

/// A drag is one undo entry. Variable defaults coalesce exactly like node
/// properties — and switching targets flushes, so two drags never merge.
#[test]
fn variable_default_edits_coalesce_per_gesture() {
    let reg = registry();
    let mut st = state();
    let a = st.add_variable("A", PinType::Int, &reg);
    let b = st.add_variable("B", PinType::Int, &reg);
    let depth = st.stack.undo_len();

    // One "drag": many values, one entry.
    for v in 1..=5 {
        st.set_variable_default(&a, Some(PropValue::Int(v)), &reg);
    }
    st.flush_var_default_edit(&reg);
    assert_eq!(st.stack.undo_len(), depth + 1, "five steps, one entry");
    assert_eq!(st.doc.variable(&a).unwrap().default, Some(PropValue::Int(5)));
    assert_eq!(st.stack.undo_description().as_deref(), Some("Set Variable Default"));

    // Touching a *different* variable flushes the first gesture.
    st.set_variable_default(&b, Some(PropValue::Int(9)), &reg);
    st.flush_var_default_edit(&reg);
    assert_eq!(st.stack.undo_len(), depth + 2);

    // Undo peels them one gesture at a time, not one value at a time.
    st.undo(&reg);
    assert_eq!(st.doc.variable(&b).unwrap().default, Some(PropValue::Int(0)));
    st.undo(&reg);
    assert_eq!(st.doc.variable(&a).unwrap().default, Some(PropValue::Int(0)));

    // A gesture that ends where it started records nothing.
    let depth = st.stack.undo_len();
    st.set_variable_default(&a, Some(PropValue::Int(3)), &reg);
    st.set_variable_default(&a, Some(PropValue::Int(0)), &reg);
    st.flush_var_default_edit(&reg);
    assert_eq!(st.stack.undo_len(), depth, "no net change, no entry");
}

// ---------------------------------------------------------------------------
// The interleaving invariant
// ---------------------------------------------------------------------------

/// **The whole-sequence invariant.** Add, rename, retype, default-edit and
/// delete, interleaved with node edits; undo everything and the document is
/// byte-identical to the start; redo everything and it is byte-identical to
/// the end.
#[test]
fn a_full_variable_sequence_undoes_and_redoes_byte_identically() {
    let reg = registry();
    let mut st = state();

    // A starting document with a node in it, so variable edits interleave
    // with the ordinary edit stream rather than living in a vacuum.
    st.doc.nodes = vec![NodeInst {
        id: 0,
        type_id: node_graph_types::std_nodes::ADD_INT.to_string(),
        type_version: 1,
        position: [0.0, 0.0],
        properties: BTreeMap::new(),
        subgraph: None,
        tint: None,
        title: None,
    }];
    let start = ron(&st);
    let depth = st.stack.undo_len();

    // The sequence.
    let score = st.add_variable("Score", PinType::Int, &reg);
    st.rename_variable(&score, "Player Score", &reg);
    st.set_variable_default(&score, Some(PropValue::Int(7)), &reg);
    st.flush_var_default_edit(&reg);
    let health = st.add_variable("Health", PinType::Float, &reg);
    st.doc.nodes.push(var_node(1, VAR_GET_TYPE_ID, &score));
    st.commit(
        crate::engine::editor::graph_editor::GraphEdit::AddNode(
            st.doc.nodes.last().unwrap().clone(),
        ),
        &reg,
    );
    st.retype_variable(&score, PinType::String, &reg);
    st.remove_variable(&health, &reg);
    let end = ron(&st);
    assert_ne!(start, end);

    let steps = st.stack.undo_len() - depth;
    assert_eq!(steps, 7, "seven gestures, seven entries");

    for _ in 0..steps {
        st.undo(&reg);
    }
    assert_eq!(ron(&st), start, "undo returns the exact starting bytes");

    for _ in 0..steps {
        st.redo(&reg);
    }
    assert_eq!(ron(&st), end, "redo returns the exact ending bytes");

    // …and again, to catch state that only survives one round trip.
    for _ in 0..steps {
        st.undo(&reg);
    }
    assert_eq!(ron(&st), start);
}

// ---------------------------------------------------------------------------
// Display names
// ---------------------------------------------------------------------------

/// Doc-dependent nodes name themselves from their configuration, so a
/// variable node reads as "Get Player Score" with no new node anatomy.
#[test]
fn display_names_come_from_configuration() {
    use crate::engine::node_graph::DocDescriptors;
    use node_graph_types::{EVENT_CUSTOM_TYPE_ID, EVENT_INPUT_ACTION_TYPE_ID, EVENT_NAME_PROP};

    let reg = registry();
    let mut st = state();
    let slug = st.add_variable("Player Score", PinType::Int, &reg);
    st.doc.nodes = vec![
        var_node(0, VAR_GET_TYPE_ID, &slug),
        var_node(1, VAR_SET_TYPE_ID, &slug),
        var_node(2, VAR_GET_TYPE_ID, "deleted"),
        NodeInst {
            id: 3,
            type_id: EVENT_CUSTOM_TYPE_ID.to_string(),
            type_version: 1,
            position: [0.0, 0.0],
            properties: [(EVENT_NAME_PROP.to_string(), PropValue::Str("Hit".into()))]
                .into_iter()
                .collect(),
            subgraph: None,
            tint: None,
            title: None,
        },
        NodeInst {
            id: 4,
            type_id: EVENT_INPUT_ACTION_TYPE_ID.to_string(),
            type_version: 1,
            position: [0.0, 0.0],
            properties: [(
                node_graph_types::std_events::EVENT_ACTION_PROP.to_string(),
                PropValue::Str("Jump".into()),
            )]
            .into_iter()
            .collect(),
            subgraph: None,
            tint: None,
            title: None,
        },
        NodeInst {
            id: 5,
            type_id: EVENT_CUSTOM_TYPE_ID.to_string(),
            type_version: 1,
            position: [0.0, 0.0],
            properties: BTreeMap::new(),
            subgraph: None,
            tint: None,
            title: None,
        },
    ];

    let d = DocDescriptors::new(&st.doc, &reg);
    assert_eq!(d.display_name(0).as_deref(), Some("Get Player Score"));
    assert_eq!(d.display_name(1).as_deref(), Some("Set Player Score"));
    assert_eq!(
        d.display_name(2).as_deref(),
        Some("Get deleted"),
        "a dangling reference still names what it was looking for"
    );
    assert_eq!(d.display_name(3).as_deref(), Some("Event: Hit"));
    assert_eq!(d.display_name(4).as_deref(), Some("Input: Jump"));
    assert_eq!(d.display_name(5).as_deref(), Some("Event: <unnamed>"));

    // Renaming the variable renames every node that reads it, with no edit
    // to the nodes at all — the point of a slug.
    st.rename_variable(&slug, "Score", &reg);
    let d = DocDescriptors::new(&st.doc, &reg);
    assert_eq!(d.display_name(0).as_deref(), Some("Get Score"));

    // An explicit title beats the synthesis (the P1 schema field, which 45.5
    // gives a rename UI).
    st.doc.node_mut(0).unwrap().title = Some("Current Score".into());
    let d = DocDescriptors::new(&st.doc, &reg);
    assert_eq!(d.display_name(0).as_deref(), Some("Current Score"));

    // An ordinary node still answers with its descriptor name.
    let d = DocDescriptors::new(&st.doc, &reg);
    assert_eq!(d.display_name(99), None, "no such node");
}

/// Per-document layouts ticket 02: the Details panel's Name row renames a
/// node through `set_node_title` — one undo entry, byte-identical undo, and
/// no entry at all for a no-op.
#[test]
fn a_node_title_edit_is_one_undo_entry() {
    let reg = registry();
    let mut st = state();
    st.doc.nodes.push(var_node(1, VAR_GET_TYPE_ID, "score"));
    let before = ron(&st);
    let depth = st.stack.undo_len();
    assert!(st.set_node_title(1, Some("Score".into()), &reg));
    assert_eq!(st.doc.node(1).unwrap().title.as_deref(), Some("Score"));
    assert_eq!(st.stack.undo_len(), depth + 1);
    assert_eq!(st.stack.undo_description().as_deref(), Some("Rename Node"));
    // Same title again, or a node that does not exist: nothing recorded.
    assert!(!st.set_node_title(1, Some("Score".into()), &reg));
    assert!(!st.set_node_title(99, None, &reg));
    assert_eq!(st.stack.undo_len(), depth + 1);
    let after = ron(&st);
    st.undo(&reg);
    assert_eq!(ron(&st), before, "undo returns the exact starting bytes");
    st.redo(&reg);
    assert_eq!(ron(&st), after, "redo returns the exact ending bytes");
}
