//! Add-node palette logic — search ranking, pin-type filtering and
//! auto-connect pin choice.
//!
//! Pure over `(registry, query, dragged pin type)`: no `Ui`, no state, so the
//! parts that are easy to get subtly wrong are the parts that are tested.
//! The palette *shell* (E3 popover, keyboard nav) lives in the panel.

use crate::engine::node_graph::{NodeDescriptor, NodeRegistry, PinType};

/// Why an entry matched, best tier first. Ranking is five tiers rather than a
/// fuzzy score because a node list is small and predictable beats clever: the
/// thing you typed the start of should be the thing on top, every time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MatchTier {
    /// The name starts with the query.
    NamePrefix,
    /// The type id starts with the query.
    IdPrefix,
    /// A whole word of the name (or id) equals the query.
    WholeWord,
    /// The name contains the query somewhere.
    NameSubstring,
    /// The id contains the query somewhere.
    IdSubstring,
}

/// How a candidate relates to the pin a drag came off.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PinFit {
    /// No drag in progress — the palette is unfiltered.
    Unfiltered,
    /// Has a pin of the dragged type on the receiving side; the slug is the
    /// one a pick would wire up.
    Compatible(String),
    /// No matching pin. **Still listed**, at 45%, with the type it does take
    /// on that side — hiding it would leave the user wondering where the node
    /// went and teach them nothing.
    Incompatible { type_tag: Option<String> },
}

impl PinFit {
    pub fn is_compatible(&self) -> bool {
        matches!(self, PinFit::Compatible(_) | PinFit::Unfiltered)
    }
}

/// One row in the palette.
#[derive(Debug, Clone, PartialEq)]
pub struct PaletteEntry {
    pub id: String,
    pub name: String,
    pub category: String,
    pub doc: Option<String>,
    pub fit: PinFit,
    /// `None` when the list is unfiltered (no query).
    pub tier: Option<MatchTier>,
}

/// What the palette is filtering against: the type dragged off a pin, and
/// which side of a candidate node has to accept it.
#[derive(Debug, Clone)]
pub struct PinFilter {
    pub ty: PinType,
    /// The drag came off an *output*, so a candidate needs a matching input.
    pub need_input: bool,
}

/// A short, human type name for the incompatible rows' tag.
pub fn type_tag(ty: &PinType) -> String {
    match ty {
        PinType::Domain(d) if d.is_empty() => "?".to_string(),
        PinType::Domain(d) => d.clone(),
        other => format!("{other:?}"),
    }
}

/// The pin a pick would wire, if the node can take the dragged type.
pub fn compatible_pin<'a>(desc: &'a NodeDescriptor, filter: &PinFilter) -> Option<&'a PinDescRef> {
    let side = if filter.need_input { &desc.inputs } else { &desc.outputs };
    side.iter().find(|p| p.ty == filter.ty)
}

/// Alias so the signature above reads without dragging the whole path in.
pub type PinDescRef = crate::engine::node_graph::PinDescriptor;

/// The pin an auto-connect should choose when a drag is released on a node
/// body: the right type first, then the closest name.
///
/// Name similarity is a cheap shared-prefix-and-substring score rather than
/// an edit distance — pin names are short and the tie is usually between two
/// or three candidates, so the expensive version buys nothing.
pub fn auto_connect_pin<'a>(
    desc: &'a NodeDescriptor,
    filter: &PinFilter,
    from_pin_name: &str,
) -> Option<&'a PinDescRef> {
    let side = if filter.need_input { &desc.inputs } else { &desc.outputs };
    let want = from_pin_name.to_lowercase();
    side.iter()
        .filter(|p| p.ty == filter.ty)
        .max_by_key(|p| name_similarity(&p.label.to_lowercase(), &want))
}

/// 0..=200-ish: exact name wins, then containment, then shared prefix.
fn name_similarity(a: &str, b: &str) -> u32 {
    if a == b {
        return 200;
    }
    let mut score = 0;
    if a.contains(b) || b.contains(a) {
        score += 100;
    }
    score += a
        .chars()
        .zip(b.chars())
        .take_while(|(x, y)| x == y)
        .count() as u32;
    score
}

/// Which tier a candidate matches the query at, if any.
fn tier_of(name: &str, id: &str, q: &str) -> Option<MatchTier> {
    let (name, id) = (name.to_lowercase(), id.to_lowercase());
    if name.starts_with(q) {
        return Some(MatchTier::NamePrefix);
    }
    if id.starts_with(q) {
        return Some(MatchTier::IdPrefix);
    }
    let word = |s: &str| {
        s.split(|c: char| !c.is_alphanumeric())
            .any(|w| w == q)
    };
    if word(&name) || word(&id) {
        return Some(MatchTier::WholeWord);
    }
    if name.contains(q) {
        return Some(MatchTier::NameSubstring);
    }
    if id.contains(q) {
        return Some(MatchTier::IdSubstring);
    }
    None
}

/// Build the palette's rows.
///
/// With an empty query every registered node is returned (the panel groups
/// them by category); with a query the list is flat and ranked. A pin filter
/// never removes rows — it only marks them, because "where did my node go" is
/// a worse experience than a dimmed row that explains itself.
pub fn build_entries(
    registry: &NodeRegistry,
    query: &str,
    filter: Option<&PinFilter>,
) -> Vec<PaletteEntry> {
    let q = query.trim().to_lowercase();
    let mut out: Vec<PaletteEntry> = registry
        .iter()
        .filter_map(|d| {
            let tier = if q.is_empty() {
                None
            } else {
                Some(tier_of(&d.name, &d.id, &q)?)
            };
            let fit = match filter {
                None => PinFit::Unfiltered,
                Some(f) => match compatible_pin(d, f) {
                    Some(p) => PinFit::Compatible(p.slug.clone()),
                    None => {
                        let side = if f.need_input { &d.inputs } else { &d.outputs };
                        PinFit::Incompatible {
                            type_tag: side.first().map(|p| type_tag(&p.ty)),
                        }
                    }
                },
            };
            Some(PaletteEntry {
                id: d.id.clone(),
                name: d.name.clone(),
                category: d.category.clone(),
                doc: d.doc.clone(),
                fit,
                tier,
            })
        })
        .collect();

    if q.is_empty() {
        // Unfiltered: registry order is already deterministic (BTreeMap), but
        // compatible nodes still float up so a drag has something to aim at.
        out.sort_by(|a, b| {
            b.fit
                .is_compatible()
                .cmp(&a.fit.is_compatible())
                .then_with(|| a.category.cmp(&b.category))
                .then_with(|| a.name.cmp(&b.name))
        });
        return out;
    }
    out.sort_by(|a, b| {
        b.fit
            .is_compatible()
            .cmp(&a.fit.is_compatible())
            .then_with(|| a.tier.cmp(&b.tier))
            .then_with(|| a.name.len().cmp(&b.name.len()))
            .then_with(|| a.name.cmp(&b.name))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::node_graph::{NodeRealm, PinDescriptor};

    fn desc(id: &str, name: &str, ins: &[(&str, PinType)], outs: &[(&str, PinType)]) -> NodeDescriptor {
        NodeDescriptor {
            id: id.into(),
            name: name.into(),
            category: "Math".into(),
            version: 1,
            inputs: ins
                .iter()
                .map(|(s, t)| PinDescriptor::new(s, s, t.clone()))
                .collect(),
            outputs: outs
                .iter()
                .map(|(s, t)| PinDescriptor::new(s, s, t.clone()))
                .collect(),
            pure: true,
            realm: NodeRealm::Shared,
            deterministic: true,
            doc: None,
        }
    }

    fn registry() -> NodeRegistry {
        let mut r = NodeRegistry::new();
        for d in [
            desc("math_add", "Add", &[("a", PinType::Float)], &[("sum", PinType::Float)]),
            desc("math_add_vec", "Add Vector", &[("a", PinType::Vec3)], &[("sum", PinType::Vec3)]),
            desc("gameplay_ladder", "Ladder", &[("x", PinType::Bool)], &[("y", PinType::Bool)]),
            desc("addendum", "Note Addendum", &[("t", PinType::Float)], &[]),
            desc("padded", "Padded Value", &[("v", PinType::Float)], &[]),
        ] {
            r.register(d).unwrap();
        }
        r
    }

    #[test]
    fn ranking_puts_prefix_matches_first() {
        let r = registry();
        let names: Vec<String> = build_entries(&r, "add", None)
            .into_iter()
            .map(|e| e.name)
            .collect();
        // "Add" (name prefix, shortest) then "Add Vector" (name prefix),
        // then the id-prefix match, then the substring matches — those last
        // two tie on tier, so the shorter name wins. ("Ladder" matches on
        // "l-add-er": substring search is literal, and that is the deal.)
        assert_eq!(names[0], "Add");
        assert_eq!(names[1], "Add Vector");
        assert_eq!(names[2], "Note Addendum", "id prefix outranks a substring");
        assert_eq!(names[3], "Ladder");
        assert_eq!(names[4], "Padded Value");

        let tiers: Vec<MatchTier> = build_entries(&r, "add", None)
            .into_iter()
            .filter_map(|e| e.tier)
            .collect();
        assert_eq!(tiers[0], MatchTier::NamePrefix);
        assert_eq!(tiers[2], MatchTier::IdPrefix);
        assert_eq!(tiers[3], MatchTier::NameSubstring);
        // The tier order itself is the documented one.
        assert!(MatchTier::NamePrefix < MatchTier::IdPrefix);
        assert!(MatchTier::IdPrefix < MatchTier::WholeWord);
        assert!(MatchTier::WholeWord < MatchTier::NameSubstring);
        assert!(MatchTier::NameSubstring < MatchTier::IdSubstring);
    }

    #[test]
    fn whole_word_outranks_substring() {
        let r = registry();
        // "vector" is a whole word of "Add Vector" and nothing else matches.
        let hits = build_entries(&r, "vector", None);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].tier, Some(MatchTier::WholeWord));
    }

    #[test]
    fn empty_query_lists_everything() {
        let r = registry();
        let all = build_entries(&r, "", None);
        assert_eq!(all.len(), 5);
        assert!(all.iter().all(|e| e.tier.is_none()));
        // Whitespace is not a query.
        assert_eq!(build_entries(&r, "   ", None).len(), 5);
    }

    /// The spec is explicit: incompatible nodes stay **listed**, dimmed, with
    /// their type tag. This is that rule at the data level.
    #[test]
    fn incompatible_nodes_are_listed_not_hidden() {
        let r = registry();
        let filter = PinFilter { ty: PinType::Float, need_input: true };
        let all = build_entries(&r, "", Some(&filter));
        assert_eq!(all.len(), 5, "nothing is filtered out of the list");

        let by_name = |n: &str| all.iter().find(|e| e.name == n).unwrap().clone();
        // Float-in nodes are compatible and name the pin a pick would wire.
        assert_eq!(by_name("Add").fit, PinFit::Compatible("a".into()));
        // The Bool one is listed, marked incompatible, and says what it takes.
        match by_name("Ladder").fit {
            PinFit::Incompatible { type_tag } => assert_eq!(type_tag.as_deref(), Some("Bool")),
            other => panic!("expected an incompatible entry, got {other:?}"),
        }
        // Compatible entries sort above incompatible ones.
        let first_incompatible = all.iter().position(|e| !e.fit.is_compatible()).unwrap();
        assert!(all[..first_incompatible].iter().all(|e| e.fit.is_compatible()));

        // Dragging off an input looks at the other side.
        let out_filter = PinFilter { ty: PinType::Vec3, need_input: false };
        let outs = build_entries(&r, "", Some(&out_filter));
        assert_eq!(
            outs.iter().find(|e| e.name == "Add Vector").unwrap().fit,
            PinFit::Compatible("sum".into())
        );
        // A node with no outputs at all is incompatible with no tag to show.
        match outs.iter().find(|e| e.name == "Padded Value").unwrap().fit.clone() {
            PinFit::Incompatible { type_tag } => assert_eq!(type_tag, None),
            other => panic!("expected incompatible, got {other:?}"),
        }
    }

    #[test]
    fn auto_connect_picks_the_type_then_the_closest_name() {
        let d = desc(
            "n",
            "N",
            &[
                ("value", PinType::Float),
                ("radius", PinType::Float),
                ("flag", PinType::Bool),
            ],
            &[],
        );
        let f = PinFilter { ty: PinType::Float, need_input: true };
        // Exact name wins outright.
        assert_eq!(auto_connect_pin(&d, &f, "radius").unwrap().slug, "radius");
        // Containment beats a bare shared prefix.
        assert_eq!(auto_connect_pin(&d, &f, "rad").unwrap().slug, "radius");
        // Nothing similar: still a correctly-typed pin, never the Bool.
        let picked = auto_connect_pin(&d, &f, "zzz").unwrap();
        assert_eq!(picked.ty, PinType::Float);
        // No pin of that type at all = no auto-connect.
        let g = PinFilter { ty: PinType::Vec3, need_input: true };
        assert!(auto_connect_pin(&d, &g, "anything").is_none());
    }

    #[test]
    fn type_tags_are_human() {
        assert_eq!(type_tag(&PinType::Float), "Float");
        assert_eq!(type_tag(&PinType::Exec), "Exec");
        assert_eq!(type_tag(&PinType::Domain("shader".into())), "shader");
        // A ghost pin's empty domain has no name to show.
        assert_eq!(type_tag(&PinType::Domain(String::new())), "?");
    }
}
