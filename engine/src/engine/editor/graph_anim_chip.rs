//! At-rest transition chips (Task 41 editor authoring).
//!
//! A transition renders as an edge chip summarizing its rule, duration and
//! priority — "Speed > 3.0 · 0.20s" — with a filled socket dot when the rule
//! is wired and a hollow one for always-true, and "⋯ n" elision at three or
//! more conditions (spec story 12; mockup 2d "chip collapse"). This module is
//! the pure half: document in, chip text out, so the summarization and
//! elision rules are unit-testable without a canvas.
//!
//! The summarizer is deliberately conservative: it prints exactly the shapes
//! it can print honestly (parameter reads, single comparisons, one uniform
//! ∧/∨ chain) and elides everything else to "⋯ n". A chip that guesses reads
//! worse than one that says "three conditions, descend to see them".

use crate::engine::animation::graph::plan::{
    ANIM_ENTRY_TYPE_ID, ANIM_RULE_RESULT_TYPE_ID, ANIM_STATE_ALIAS_TYPE_ID, DURATION_PROP,
    PRIORITY_PROP, RULE_RESULT_PIN, TRANSITION_FROM_PIN, TRANSITION_TO_PIN,
};
use crate::engine::node_graph::std_nodes::{AND, COMPARE_FLOAT, NOT, OR};
use crate::engine::node_graph::{
    GraphDoc, GraphRegion, NodeInst, PropValue, REROUTE_IN, REROUTE_TYPE_ID, VAR_GET_TYPE_ID,
    VAR_PROP,
};

/// What a transition's chip states, resolved from the document.
#[derive(Debug, Clone, PartialEq)]
pub struct TransitionChip {
    /// Rule summary text. `None` = always-true (no summary segment).
    pub summary: Option<String>,
    /// Filled socket dot = a wired rule; hollow = always-true.
    pub wired: bool,
    /// Crossfade seconds (the `duration` property, default 0).
    pub duration: f32,
    /// Priority (default 0). Non-zero shows as a `P{n}` tag.
    pub priority: i32,
    /// Source state's title ("State" untitled, "?" unwired).
    pub from: String,
    /// Target state's title, same fallbacks.
    pub to: String,
}

impl TransitionChip {
    /// The badge's hover text: `Idle → Jump` over the chip line.
    pub fn tooltip(&self) -> String {
        format!("{} \u{2192} {}\n{}", self.from, self.to, self.text())
    }

    /// The chip's one line: `[summary · ]duration[ · P{n}]`.
    ///
    /// Duration always prints — an instant transition saying "0.00s" is
    /// honest. `P0` (the default) does not: no tag *means* default, and a
    /// canvas of all-default transitions saying P0 six times is noise.
    pub fn text(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if let Some(s) = &self.summary {
            parts.push(s.clone());
        }
        parts.push(format!("{:.2}s", self.duration.max(0.0)));
        if self.priority != 0 {
            parts.push(format!("P{}", self.priority));
        }
        parts.join(" \u{b7} ")
    }
}

/// Resolve the chip for `transition` (a top-level node id). Callers pass any
/// node id; a missing node answers the all-default chip.
pub fn transition_chip(doc: &GraphDoc, transition: u64) -> TransitionChip {
    let (duration, priority) = doc
        .node(transition)
        .map(|n| {
            (
                match n.properties.get(DURATION_PROP) {
                    Some(PropValue::Float(f)) => *f,
                    _ => 0.0,
                },
                match n.properties.get(PRIORITY_PROP) {
                    Some(PropValue::Int(i)) => *i,
                    _ => 0,
                },
            )
        })
        .unwrap_or((0.0, 0));
    let (summary, wired) = match doc.regions.get(&transition) {
        Some(region) if !region.nodes.is_empty() => summarize_rule(doc, region),
        // No region, or an empty one: always-true, the hollow-dot chip.
        _ => (None, false),
    };
    let from = doc
        .edges
        .iter()
        .find(|e| e.to_node == transition && e.to_pin == TRANSITION_FROM_PIN)
        .map(|e| e.from_node);
    let to = doc
        .edges
        .iter()
        .find(|e| e.from_node == transition && e.from_pin == TRANSITION_TO_PIN)
        .map(|e| e.to_node);
    TransitionChip {
        summary,
        wired,
        duration,
        priority,
        from: state_title(doc, from),
        to: state_title(doc, to),
    }
}

/// A machine node's title for the badge tooltip: its own title, else what
/// kind of node it is; "?" for an endpoint nothing is wired to.
fn state_title(doc: &GraphDoc, id: Option<u64>) -> String {
    let Some(n) = id.and_then(|id| doc.node(id)) else {
        return "?".to_string();
    };
    match (&n.title, n.type_id.as_str()) {
        (Some(t), _) if !t.trim().is_empty() => t.clone(),
        (_, ANIM_STATE_ALIAS_TYPE_ID) => "Alias".to_string(),
        (_, ANIM_ENTRY_TYPE_ID) => "Entry".to_string(),
        _ => "State".to_string(),
    }
}

/// Summarize a non-empty rule region: `(summary segment, wired)`.
///
/// Mirrors `compile_rule`'s always-true reading — a RESULT with nothing wired
/// in is hollow — and degrades to hollow-no-summary on shapes the compiler
/// refuses anyway (no RESULT, a cycle): the anchored error tells that story,
/// the chip does not repeat it.
fn summarize_rule(doc: &GraphDoc, region: &GraphRegion) -> (Option<String>, bool) {
    let mut results = region
        .nodes
        .iter()
        .filter(|n| n.type_id == ANIM_RULE_RESULT_TYPE_ID);
    let (Some(result), None) = (results.next(), results.next()) else {
        return (None, false);
    };
    let Some(edge) = region
        .edges
        .iter()
        .find(|e| e.to_node == result.id && e.to_pin == RULE_RESULT_PIN)
    else {
        return (None, false); // unwired Bool input = always-true
    };

    let mut cx = Summarizer { doc, region, stack: Vec::new(), connective: None };
    let mut leaves = Vec::new();
    let uniform = cx.connective_walk(edge.from_node, &mut leaves);
    if !uniform || leaves.len() >= 3 || leaves.iter().any(Option::is_none) {
        // The condition count survives even when the chain is unprintable —
        // an aborted walk (mixed ∧/∨, a cycle) recounts from the top.
        let n = if uniform {
            leaves.len().max(1)
        } else {
            cx.stack.clear();
            cx.count_leaves(edge.from_node).max(1)
        };
        return (Some(format!("\u{22ef} {n}")), true);
    }
    let sep = match cx.connective {
        Some(Connective::And) => " \u{2227} ",
        Some(Connective::Or) => " \u{2228} ",
        None => "",
    };
    let text = leaves
        .into_iter()
        .map(|l| l.expect("checked above"))
        .collect::<Vec<_>>()
        .join(sep);
    (Some(text), true)
}

#[derive(Clone, Copy, PartialEq)]
enum Connective {
    And,
    Or,
}

struct Summarizer<'a> {
    doc: &'a GraphDoc,
    region: &'a GraphRegion,
    stack: Vec<u64>,
    /// The one connective this chain prints with — set by the first ∧/∨ met;
    /// a second *kind* makes the chain unprintable.
    connective: Option<Connective>,
}

impl Summarizer<'_> {
    /// Walk the connective tree under `node_id`, pushing one rendered
    /// condition per leaf (`None` = unprintable). Returns `false` when the
    /// chain mixes ∧ and ∨ or loops — the caller elides then.
    fn connective_walk(&mut self, node_id: u64, out: &mut Vec<Option<String>>) -> bool {
        if self.stack.contains(&node_id) {
            return false; // a cycle: the compiler refuses it, the chip elides
        }
        self.stack.push(node_id);
        let ok = (|| {
            let Some(node) = self.region.node(node_id) else {
                out.push(None);
                return true;
            };
            match node.type_id.as_str() {
                AND | OR => {
                    let here = if node.type_id == AND { Connective::And } else { Connective::Or };
                    match self.connective {
                        None => self.connective = Some(here),
                        Some(c) if c == here => {}
                        Some(_) => return false, // mixed ∧/∨: elide
                    }
                    for pin in ["a", "b"] {
                        match self.wired_source(node_id, pin) {
                            Some(src) => {
                                if !self.connective_walk(src, out) {
                                    return false;
                                }
                            }
                            // An unwired Bool input reads as its stored
                            // constant — not worth printing, count it.
                            None => out.push(None),
                        }
                    }
                    true
                }
                REROUTE_TYPE_ID => match self.wired_source(node_id, REROUTE_IN) {
                    Some(src) => self.connective_walk(src, out),
                    None => {
                        out.push(None);
                        true
                    }
                },
                _ => {
                    out.push(self.leaf(node));
                    true
                }
            }
        })();
        self.stack.pop();
        ok
    }

    /// One condition's text, or `None` when it has no honest short form.
    fn leaf(&mut self, node: &NodeInst) -> Option<String> {
        match node.type_id.as_str() {
            VAR_GET_TYPE_ID => self.param_label(node),
            NOT => {
                let src = self.wired_source(node.id, "a")?;
                if self.stack.contains(&src) {
                    return None;
                }
                let inner = self.region.node(src)?;
                // ¬ prints only over a bare parameter read; anything deeper
                // has no one-glance reading.
                if inner.type_id == VAR_GET_TYPE_ID {
                    Some(format!("\u{ac}{}", self.param_label(inner)?))
                } else {
                    None
                }
            }
            COMPARE_FLOAT => {
                let op = match node.properties.get("op") {
                    None => "=",
                    Some(PropValue::Enum(s)) => match s.as_str() {
                        "equal" => "=",
                        "not_equal" => "\u{2260}",
                        "less" => "<",
                        "less_equal" => "\u{2264}",
                        "greater" => ">",
                        "greater_equal" => "\u{2265}",
                        _ => return None,
                    },
                    Some(_) => return None,
                };
                let a = self.operand(node, "a")?;
                let b = self.operand(node, "b")?;
                Some(format!("{a} {op} {b}"))
            }
            _ => None,
        }
    }

    /// A comparison operand: a parameter read over the wire, or the pin's
    /// stored constant. Math chains and deeper shapes answer `None`.
    fn operand(&mut self, node: &NodeInst, pin: &str) -> Option<String> {
        match self.wired_source(node.id, pin) {
            Some(src) => {
                let n = self.region.node(src)?;
                if n.type_id == VAR_GET_TYPE_ID {
                    self.param_label(n)
                } else {
                    None
                }
            }
            None => Some(match node.properties.get(pin) {
                Some(PropValue::Float(f)) => fmt_float(*f),
                _ => fmt_float(0.0),
            }),
        }
    }

    /// The author-facing name of the parameter a `var_get` reads: the
    /// declaration's label, never the slug — the chip speaks the panel's
    /// language.
    fn param_label(&self, node: &NodeInst) -> Option<String> {
        let slug = match node.properties.get(VAR_PROP) {
            Some(PropValue::Str(s)) if !s.is_empty() => s.as_str(),
            _ => return None,
        };
        Some(
            self.doc
                .variable(slug)
                .map(|v| v.label.clone())
                .unwrap_or_else(|| slug.to_string()),
        )
    }

    /// How many conditions hang under `node_id`, connective discipline
    /// aside — what the "⋯ n" prints when the chain has no readable form.
    fn count_leaves(&mut self, node_id: u64) -> usize {
        if self.stack.contains(&node_id) {
            return 1; // a cycle: count the re-entry once and stop
        }
        self.stack.push(node_id);
        let n = match self.region.node(node_id).map(|n| n.type_id.as_str()) {
            Some(AND) | Some(OR) => ["a", "b"]
                .into_iter()
                .map(|pin| match self.wired_source(node_id, pin) {
                    Some(src) => self.count_leaves(src),
                    None => 1,
                })
                .sum(),
            Some(REROUTE_TYPE_ID) => match self.wired_source(node_id, REROUTE_IN) {
                Some(src) => self.count_leaves(src),
                None => 1,
            },
            _ => 1,
        };
        self.stack.pop();
        n
    }

    fn wired_source(&self, node: u64, pin: &str) -> Option<u64> {
        self.region
            .edges
            .iter()
            .find(|e| e.to_node == node && e.to_pin == pin)
            .map(|e| e.from_node)
    }
}

/// "3.0" for whole values (the mockup's spelling), the shortest honest form
/// otherwise ("0.5", "3.25").
fn fmt_float(v: f32) -> String {
    if v == v.trunc() && v.abs() < 1e6 {
        format!("{v:.1}")
    } else {
        format!("{v}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::animation::graph::plan::{
        ANIM_TRANSITION_TYPE_ID, DURATION_PROP, PRIORITY_PROP,
    };
    use crate::engine::node_graph::{Edge, VarDecl};
    use node_graph_types::PinType;

    fn node(id: u64, type_id: &str) -> NodeInst {
        NodeInst {
            id,
            type_id: type_id.to_string(),
            type_version: 1,
            position: [0.0, 0.0],
            properties: Default::default(),
            subgraph: None,
            tint: None,
            title: None,
        }
    }

    fn edge(from: u64, from_pin: &str, to: u64, to_pin: &str) -> Edge {
        Edge {
            from_node: from,
            from_pin: from_pin.to_string(),
            to_node: to,
            to_pin: to_pin.to_string(),
        }
    }

    fn var_get(id: u64, slug: &str) -> NodeInst {
        let mut n = node(id, VAR_GET_TYPE_ID);
        n.properties
            .insert(VAR_PROP.to_string(), PropValue::Str(slug.to_string()));
        n
    }

    fn compare(id: u64, op: &str, b: f32) -> NodeInst {
        let mut n = node(id, COMPARE_FLOAT);
        n.properties
            .insert("op".to_string(), PropValue::Enum(op.to_string()));
        n.properties.insert("b".to_string(), PropValue::Float(b));
        n
    }

    /// A doc holding one transition (id 1) whose region is `region`, plus the
    /// declared parameters the rules read.
    fn doc_with(region: Option<GraphRegion>, dur: f32, prio: i32) -> GraphDoc {
        let mut doc = GraphDoc::default();
        let mut t = node(1, ANIM_TRANSITION_TYPE_ID);
        t.properties
            .insert(DURATION_PROP.to_string(), PropValue::Float(dur));
        t.properties
            .insert(PRIORITY_PROP.to_string(), PropValue::Int(prio));
        doc.nodes.push(t);
        for (slug, label) in [("speed", "Speed"), ("died", "Died"), ("hp", "HP")] {
            doc.variables.push(VarDecl {
                slug: slug.to_string(),
                label: label.to_string(),
                ty: PinType::Float,
                default: None,
                group: None,
            });
        }
        if let Some(r) = region {
            doc.regions.insert(1, r);
        }
        doc
    }

    /// No region / empty region / unwired RESULT ⇒ always-true: hollow dot,
    /// duration only.
    #[test]
    fn always_true_is_a_hollow_duration_chip() {
        let chip = transition_chip(&doc_with(None, 0.2, 0), 1);
        assert_eq!(
            chip,
            TransitionChip {
                summary: None,
                wired: false,
                duration: 0.2,
                priority: 0,
                from: "?".to_string(),
                to: "?".to_string(),
            }
        );
        assert_eq!(chip.text(), "0.20s");

        let empty = doc_with(Some(GraphRegion::default()), 0.2, 0);
        assert!(!transition_chip(&empty, 1).wired);

        let unwired = doc_with(
            Some(GraphRegion {
                nodes: vec![node(0, ANIM_RULE_RESULT_TYPE_ID), var_get(2, "died")],
                edges: vec![],
            }),
            0.2,
            0,
        );
        let chip = transition_chip(&unwired, 1);
        assert!(!chip.wired, "a RESULT with nothing wired in is always-true");
        assert_eq!(chip.text(), "0.20s");
    }

    /// The ticket's own example: one comparison, wired, "Speed > 3.0 · 0.20s".
    #[test]
    fn a_single_comparison_prints_the_mockup_line() {
        let region = GraphRegion {
            nodes: vec![
                node(0, ANIM_RULE_RESULT_TYPE_ID),
                var_get(2, "speed"),
                compare(3, "greater", 3.0),
            ],
            edges: vec![
                edge(2, "value", 3, "a"),
                edge(3, "result", 0, RULE_RESULT_PIN),
            ],
        };
        let chip = transition_chip(&doc_with(Some(region), 0.2, 0), 1);
        assert!(chip.wired);
        assert_eq!(chip.text(), "Speed > 3.0 \u{b7} 0.20s");
    }

    /// Two conditions joined by ∧, with a Trigger read printing as its label
    /// — "Died ∧ HP ≤ 0.0 · 0.10s" — and the priority tag appearing only
    /// when non-default.
    #[test]
    fn an_and_pair_prints_both_sides_and_the_priority_tag() {
        let region = GraphRegion {
            nodes: vec![
                node(0, ANIM_RULE_RESULT_TYPE_ID),
                var_get(2, "died"),
                var_get(3, "hp"),
                compare(4, "less_equal", 0.0),
                node(5, AND),
            ],
            edges: vec![
                edge(3, "value", 4, "a"),
                edge(2, "value", 5, "a"),
                edge(4, "result", 5, "b"),
                edge(5, "result", 0, RULE_RESULT_PIN),
            ],
        };
        let chip = transition_chip(&doc_with(Some(region), 0.1, 1), 1);
        assert_eq!(chip.text(), "Died \u{2227} HP \u{2264} 0.0 \u{b7} 0.10s \u{b7} P1");
    }

    /// Three or more conditions elide to "⋯ n".
    #[test]
    fn three_conditions_elide() {
        let region = GraphRegion {
            nodes: vec![
                node(0, ANIM_RULE_RESULT_TYPE_ID),
                var_get(2, "died"),
                var_get(3, "died"),
                var_get(4, "died"),
                node(5, AND),
                node(6, AND),
            ],
            edges: vec![
                edge(2, "value", 5, "a"),
                edge(3, "value", 5, "b"),
                edge(5, "result", 6, "a"),
                edge(4, "value", 6, "b"),
                edge(6, "result", 0, RULE_RESULT_PIN),
            ],
        };
        let chip = transition_chip(&doc_with(Some(region), 0.15, 0), 1);
        assert_eq!(chip.text(), "\u{22ef} 3 \u{b7} 0.15s");
    }

    /// An ∨ chain prints with its own connective; a mixed ∧/∨ tree elides.
    #[test]
    fn or_prints_and_mixed_connectives_elide() {
        let or_pair = GraphRegion {
            nodes: vec![
                node(0, ANIM_RULE_RESULT_TYPE_ID),
                var_get(2, "died"),
                var_get(3, "died"),
                node(4, OR),
            ],
            edges: vec![
                edge(2, "value", 4, "a"),
                edge(3, "value", 4, "b"),
                edge(4, "result", 0, RULE_RESULT_PIN),
            ],
        };
        assert_eq!(
            transition_chip(&doc_with(Some(or_pair), 0.0, 0), 1).text(),
            "Died \u{2228} Died \u{b7} 0.00s"
        );

        let mixed = GraphRegion {
            nodes: vec![
                node(0, ANIM_RULE_RESULT_TYPE_ID),
                var_get(2, "died"),
                var_get(3, "died"),
                var_get(4, "died"),
                node(5, OR),
                node(6, AND),
            ],
            edges: vec![
                edge(2, "value", 5, "a"),
                edge(3, "value", 5, "b"),
                edge(5, "result", 6, "a"),
                edge(4, "value", 6, "b"),
                edge(6, "result", 0, RULE_RESULT_PIN),
            ],
        };
        assert_eq!(
            transition_chip(&doc_with(Some(mixed), 0.0, 0), 1).text(),
            "\u{22ef} 3 \u{b7} 0.00s"
        );
    }

    /// Shapes with no honest one-line form — math feeding a comparison —
    /// elide rather than guess, even below three conditions.
    #[test]
    fn unprintable_leaves_elide_instead_of_guessing() {
        use crate::engine::node_graph::std_nodes::ADD_FLOAT;
        let region = GraphRegion {
            nodes: vec![
                node(0, ANIM_RULE_RESULT_TYPE_ID),
                node(2, ADD_FLOAT),
                compare(3, "greater", 1.0),
            ],
            edges: vec![
                edge(2, "result", 3, "a"),
                edge(3, "result", 0, RULE_RESULT_PIN),
            ],
        };
        assert_eq!(
            transition_chip(&doc_with(Some(region), 0.0, 0), 1).text(),
            "\u{22ef} 1 \u{b7} 0.00s"
        );
    }

    /// A parameter nobody declared still prints (as its slug): the chip's job
    /// is a readable summary, the *error* about the undeclared parameter is
    /// the anchored refusal's job.
    #[test]
    fn an_undeclared_parameter_prints_its_slug() {
        let region = GraphRegion {
            nodes: vec![node(0, ANIM_RULE_RESULT_TYPE_ID), var_get(2, "ghost")],
            edges: vec![edge(2, "value", 0, RULE_RESULT_PIN)],
        };
        assert_eq!(
            transition_chip(&doc_with(Some(region), 0.0, 0), 1).text(),
            "ghost \u{b7} 0.00s"
        );
    }

    /// ¬ prints over a bare parameter read.
    #[test]
    fn not_prints_over_a_parameter_read() {
        let region = GraphRegion {
            nodes: vec![
                node(0, ANIM_RULE_RESULT_TYPE_ID),
                var_get(2, "died"),
                node(3, NOT),
            ],
            edges: vec![
                edge(2, "value", 3, "a"),
                edge(3, "result", 0, RULE_RESULT_PIN),
            ],
        };
        assert_eq!(
            transition_chip(&doc_with(Some(region), 0.0, 0), 1).text(),
            "\u{ac}Died \u{b7} 0.00s"
        );
    }

    /// The badge tooltip names both endpoints — the state's title, its kind
    /// when untitled, "?" when unwired — over the chip line.
    #[test]
    fn tooltip_names_both_states() {
        use crate::engine::animation::graph::plan::{
            ANIM_STATE_ALIAS_TYPE_ID, ANIM_STATE_TYPE_ID, STATE_IN_PIN, STATE_OUT_PIN,
        };
        let mut doc = doc_with(None, 0.2, 0);
        let mut idle = node(2, ANIM_STATE_TYPE_ID);
        idle.title = Some("Idle".to_string());
        doc.nodes.push(idle);
        doc.nodes.push(node(3, ANIM_STATE_TYPE_ID));
        doc.nodes.push(node(4, ANIM_STATE_ALIAS_TYPE_ID));
        doc.edges.push(edge(2, STATE_OUT_PIN, 1, TRANSITION_FROM_PIN));
        doc.edges.push(edge(1, TRANSITION_TO_PIN, 3, STATE_IN_PIN));
        assert_eq!(transition_chip(&doc, 1).tooltip(), "Idle \u{2192} State\n0.20s");

        doc.edges[0] = edge(4, STATE_OUT_PIN, 1, TRANSITION_FROM_PIN);
        doc.edges.pop();
        assert_eq!(transition_chip(&doc, 1).tooltip(), "Alias \u{2192} ?\n0.20s");
    }

    /// Float formatting: whole values take the mockup's ".0" spelling.
    #[test]
    fn float_formatting_matches_the_mockup() {
        assert_eq!(fmt_float(3.0), "3.0");
        assert_eq!(fmt_float(0.5), "0.5");
        assert_eq!(fmt_float(3.25), "3.25");
        assert_eq!(fmt_float(0.0), "0.0");
    }
}
