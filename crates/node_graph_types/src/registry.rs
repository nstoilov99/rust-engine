//! `NodeRegistry` — the registration contract of the node graph framework.
//!
//! Runtime registration is the primary API: Task 39.8's plugins call
//! [`NodeRegistry::register`] from `Plugin::build`, the Phase B derive-macro
//! layer is just another caller, and nothing here assumes static linking or
//! a global singleton. Node types are registered by stable slug, looked up
//! by slug, displayed by name/category.

use std::collections::{BTreeMap, BTreeSet};

use crate::doc::{NodeRealm, PinType, PropValue};

/// Reserved node-type slug for subgraph instances — their pins derive from
/// the referenced `.subgraph` asset's interface, not from a descriptor.
pub const SUBGRAPH_TYPE_ID: &str = "subgraph";

/// Reserved node-type slug for reroute nodes — a one-in, many-out pass-through
/// whose pin type is *inferred* from whatever it is wired to, so it has no
/// descriptor and cannot be registered.
pub const REROUTE_TYPE_ID: &str = "reroute";

/// The two pin slugs every reroute has.
pub const REROUTE_IN: &str = "in";
pub const REROUTE_OUT: &str = "out";

/// Reserved node-type slug for a subgraph's **interface input** node (45-A
/// D3, Blender's Group Input pattern): its *outputs* mirror the document's
/// declared `inputs`, so compilation has somewhere to splice host edges. Only
/// meaningful inside a document that declares an interface.
pub const GRAPH_INPUT_TYPE_ID: &str = "graph_input";

/// Reserved node-type slug for a subgraph's **interface output** node: its
/// *inputs* mirror the document's declared `outputs`.
pub const GRAPH_OUTPUT_TYPE_ID: &str = "graph_output";

/// Reserved node-type slugs for variable access. Pins are synthesized from
/// the document's [`VarDecl`](crate::doc::VarDecl) named by the instance's
/// reserved [`VAR_PROP`] property, so neither can be registered.
///
/// `var_get` — pure, one output [`VAR_VALUE_PIN`] of the variable's type.
/// `var_set` — impure: inputs [`EXEC_IN_PIN`] + [`VAR_VALUE_PIN`], output
/// [`EXEC_OUT_PIN`].
pub const VAR_GET_TYPE_ID: &str = "var_get";
pub const VAR_SET_TYPE_ID: &str = "var_set";

/// Reserved *property* key naming the variable a `var_get`/`var_set` reads or
/// writes. Value is a [`PropValue::Str`](crate::doc::PropValue::Str) holding
/// the [`VarDecl::slug`](crate::doc::VarDecl::slug).
pub const VAR_PROP: &str = "var";

/// The value pin of both variable nodes.
pub const VAR_VALUE_PIN: &str = "value";

/// Node-type slug for the Timeline (45-A D7). **Registered, not reserved**:
/// its exec pins and configuration come from a normal descriptor, and only
/// its *track outputs* are synthesized from the referenced `.curve` — the
/// same "registry base plus document-dependent extras" shape `event_custom`
/// uses.
pub const TIMELINE_TYPE_ID: &str = "timeline";

/// Reserved *property* key naming the `.curve` asset a Timeline plays.
/// A property rather than an input pin on purpose: the track list — and
/// therefore the node's output pins — has to be knowable from the document
/// alone, and a wired pin's value is not. Same rule the `subgraph` reference
/// follows. Value is a [`PropValue::Asset`](crate::doc::PropValue::Asset).
pub const CURVE_PROP: &str = "curve";

/// Timeline exec pins.
pub const TIMELINE_PLAY_PIN: &str = "play";
pub const TIMELINE_REVERSE_PIN: &str = "reverse";
pub const TIMELINE_STOP_PIN: &str = "stop";
pub const TIMELINE_UPDATE_PIN: &str = "update";
pub const TIMELINE_FINISHED_PIN: &str = "finished";

/// The exec pin slugs the framework's own synthesized descriptors use. Node
/// libraries are free to pick their own; these exist so the doc-dependent
/// types spell them one way.
pub const EXEC_IN_PIN: &str = "exec_in";
pub const EXEC_OUT_PIN: &str = "exec_out";

/// Every reserved node-type slug, in one place: a descriptor may not claim
/// any of them, because their pins come from the *document*, not a registry
/// entry. Consulted by [`NodeRegistry::register`] and `merge_staged`.
pub const RESERVED_TYPE_IDS: [&str; 6] = [
    SUBGRAPH_TYPE_ID,
    REROUTE_TYPE_ID,
    GRAPH_INPUT_TYPE_ID,
    GRAPH_OUTPUT_TYPE_ID,
    VAR_GET_TYPE_ID,
    VAR_SET_TYPE_ID,
];

/// What a node type can render as a per-node preview. Opting in costs the
/// common case nothing: a node without one draws no slot at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewKind {
    /// A render target — material/shader nodes (Task 50).
    RenderTarget,
    /// A curve strip — animation nodes (Task 41).
    CurveStrip,
}

impl PreviewKind {
    /// Mono label for the placeholder well, until the real content lands.
    pub fn label(self) -> &'static str {
        match self {
            PreviewKind::RenderTarget => "RENDER",
            PreviewKind::CurveStrip => "CURVE",
        }
    }

    /// Parse the macro's `preview = "..."` spelling.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "render_target" => Some(PreviewKind::RenderTarget),
            "curve_strip" => Some(PreviewKind::CurveStrip),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PinDescriptor {
    /// Stable pin slug — serialized in edges and properties.
    pub slug: String,
    /// Display label, free to change.
    pub label: String,
    pub ty: PinType,
    /// Constant shown/stored when the pin is unconnected (input pins only).
    pub default: Option<PropValue>,
    /// One line explaining what the pin is for, shown in its hover tooltip.
    /// Descriptors are never serialized, so this needs no serde treatment —
    /// it lives with the code that registers the node.
    pub doc: Option<String>,
    /// Legal values for an `Enum` pin. Empty means "any string": the inline
    /// widget stays a free chip rather than becoming a dropdown over nothing.
    pub variants: Vec<String>,
}

impl PinDescriptor {
    pub fn new(slug: &str, label: &str, ty: PinType) -> Self {
        Self {
            slug: slug.to_string(),
            label: label.to_string(),
            ty,
            default: None,
            doc: None,
            variants: Vec::new(),
        }
    }

    pub fn with_default(mut self, v: PropValue) -> Self {
        self.default = Some(v);
        self
    }

    /// One line, shown on hover. Keep it to a line: a tooltip that needs
    /// scrolling is documentation in the wrong place.
    pub fn with_doc(mut self, doc: &str) -> Self {
        self.doc = Some(doc.to_string());
        self
    }

    /// Declare the legal values of an `Enum` pin. A stored value outside the
    /// list is *shown* as a warning rather than reported as an error — the
    /// `GraphError` set is closed (recorded ruling), and an out-of-list enum
    /// is stale data to fix, not a broken document.
    pub fn with_variants<I, S>(mut self, variants: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.variants = variants.into_iter().map(Into::into).collect();
        self
    }

    /// Is `value` legal for this pin? Always true when no list is declared.
    pub fn accepts_variant(&self, value: &str) -> bool {
        self.variants.is_empty() || self.variants.iter().any(|v| v == value)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct NodeDescriptor {
    /// Stable slug — the serialized identity (`"set_world_position"`).
    pub id: String,
    /// Display name (`"Set World Position"`), free to change.
    pub name: String,
    /// Search-menu category (`"Gameplay"`).
    pub category: String,
    /// Descriptor schema version; migrations registered per step (P3).
    pub version: u32,
    pub inputs: Vec<PinDescriptor>,
    pub outputs: Vec<PinDescriptor>,
    /// Pure nodes compute values (cacheable, no side effects, no exec pins);
    /// impure nodes emit commands and require exec flow.
    pub pure: bool,
    pub realm: NodeRealm,
    /// Same inputs always produce same outputs (networking/replay relevant).
    pub deterministic: bool,
    /// One line explaining what the node does, shown when hovering its header.
    pub doc: Option<String>,
    /// Opt-in per-node preview. `None` — the common case — costs nothing.
    pub preview: Option<PreviewKind>,
}

impl NodeDescriptor {
    pub fn input(&self, slug: &str) -> Option<&PinDescriptor> {
        self.inputs.iter().find(|p| p.slug == slug)
    }

    pub fn output(&self, slug: &str) -> Option<&PinDescriptor> {
        self.outputs.iter().find(|p| p.slug == slug)
    }

    fn has_exec_pin(&self) -> bool {
        self.inputs
            .iter()
            .chain(self.outputs.iter())
            .any(|p| p.ty == PinType::Exec)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum RegistryError {
    DuplicateId(String),
    InvalidDescriptor {
        id: String,
        reason: String,
    },
    /// Two migration steps claim the same `(type_id, from_version)`. Unlike a
    /// duplicate node id this is not survivable: silently picking one would
    /// change how documents upgrade.
    DuplicateMigration {
        type_id: String,
        from_version: u32,
    },
}

impl std::fmt::Display for RegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegistryError::DuplicateId(id) => write!(f, "node type '{id}' already registered"),
            RegistryError::InvalidDescriptor { id, reason } => {
                write!(f, "invalid node descriptor '{id}': {reason}")
            }
            RegistryError::DuplicateMigration {
                type_id,
                from_version,
            } => write!(
                f,
                "migration for '{type_id}' v{from_version} already registered"
            ),
        }
    }
}

impl std::error::Error for RegistryError {}

/// A single migration step for one node type, from one version to the next.
pub type MigrationFn = Box<dyn Fn(&mut crate::migrate::MigrationCtx) + Send + Sync>;

/// One plugin's pending registry contributions, collected by `PluginContext`
/// and merged in one shot by [`NodeRegistry::merge_staged`].
#[derive(Default)]
pub struct StagedRegistry {
    pub nodes: Vec<NodeDescriptor>,
    pub domain_pins: Vec<(String, Option<u8>)>,
    pub migrations: Vec<((String, u32), MigrationFn)>,
}

/// What a [`NodeRegistry::merge_staged`] call actually did. The counts are
/// what the Plugin Manager shows; the warnings are the "enabled with
/// warnings" state.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MergeReport {
    pub warnings: Vec<String>,
    /// Descriptors inserted (staged minus id-collision skips).
    pub nodes_registered: usize,
    /// Domain pins newly registered (identical re-registrations don't count).
    pub domain_pins_registered: usize,
    pub migrations_registered: usize,
}

/// Registry of node types, consumer domain pin types, and migration chains.
/// `BTreeMap` keeps iteration deterministic (stable search-menu ordering,
/// stable tests).
#[derive(Default)]
pub struct NodeRegistry {
    nodes: BTreeMap<String, NodeDescriptor>,
    /// slug -> optional explicit ramp index. `None` = registered but unkeyed
    /// (the theme hashes the slug); absent = not registered (theme renders
    /// `neutral`, the only unknown-color case).
    domain_pins: BTreeMap<String, Option<u8>>,
    /// Domain pins declared **flow-like** (Task 41): their wires carry
    /// topology, not pulled values, so — like Exec — their inputs may fan in
    /// and their wires may form cycles. Unlike Exec, their *outputs* may fan
    /// out (an animation state's `out` feeds several transitions). The
    /// animation machine's `anim_flow` is the first member.
    flow_domains: BTreeSet<String>,
    /// Which plugin registered a domain pin first, so a later conflicting
    /// registration can name both sides in its warning. Only populated via
    /// [`NodeRegistry::merge_staged`].
    domain_pin_owners: BTreeMap<String, String>,
    migrations: BTreeMap<(String, u32), MigrationFn>,
}

impl NodeRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a node type. Errors on slug collision and on descriptor-
    /// level invariant violations (the checks that don't need a document):
    /// pure nodes may not have exec pins, impure nodes require exec flow,
    /// pin slugs must be unique per side, [`RESERVED_TYPE_IDS`] are refused.
    pub fn register(&mut self, desc: NodeDescriptor) -> Result<(), RegistryError> {
        Self::validate_descriptor(&desc)?;
        if self.nodes.contains_key(&desc.id) {
            return Err(RegistryError::DuplicateId(desc.id));
        }
        self.nodes.insert(desc.id.clone(), desc);
        Ok(())
    }

    /// The descriptor-level invariants — the checks that need no document and
    /// no registry state. Shared by [`register`](Self::register) and
    /// [`merge_staged`](Self::merge_staged).
    fn validate_descriptor(desc: &NodeDescriptor) -> Result<(), RegistryError> {
        let invalid = |reason: &str| {
            Err(RegistryError::InvalidDescriptor {
                id: desc.id.clone(),
                reason: reason.to_string(),
            })
        };
        if RESERVED_TYPE_IDS.contains(&desc.id.as_str()) {
            return invalid("slug is reserved: its pins come from the document, not a descriptor");
        }
        if desc.pure && desc.has_exec_pin() {
            return invalid("pure nodes may not have exec pins");
        }
        if !desc.pure && !desc.has_exec_pin() {
            return invalid("impure nodes require at least one exec pin");
        }
        for pins in [&desc.inputs, &desc.outputs] {
            let mut seen = BTreeSet::new();
            for p in pins.iter() {
                if !seen.insert(p.slug.as_str()) {
                    return Err(RegistryError::InvalidDescriptor {
                        id: desc.id.clone(),
                        reason: format!("duplicate pin slug '{}'", p.slug),
                    });
                }
            }
        }
        Ok(())
    }

    /// Merge one plugin's staged registrations (Task 39.8).
    ///
    /// Every check runs before any mutation: on `Err` the registry is exactly
    /// as it was. Collision policy, per the 39.8 rulings:
    ///
    /// - **node id already taken** — the descriptor is skipped and a warning
    ///   returned; the plugin ends up "enabled with warnings", not failed.
    /// - **invalid descriptor** — hard error. That is a bug in the plugin, not
    ///   two plugins disagreeing.
    /// - **domain pin re-registered** — identical is a no-op; different is
    ///   first-wins plus a warning naming both registrants.
    /// - **migration key `(type_id, from_version)` taken** — hard error.
    pub fn merge_staged(
        &mut self,
        plugin_id: &str,
        staged: StagedRegistry,
    ) -> Result<MergeReport, RegistryError> {
        let mut warnings = Vec::new();

        // --- preflight: nodes ---
        let mut nodes = Vec::with_capacity(staged.nodes.len());
        let mut claimed: BTreeSet<String> = BTreeSet::new();
        for desc in staged.nodes {
            Self::validate_descriptor(&desc)?;
            if self.nodes.contains_key(&desc.id) || !claimed.insert(desc.id.clone()) {
                warnings.push(format!(
                    "node type '{}' is already registered — plugin '{plugin_id}' skipped it",
                    desc.id
                ));
                continue;
            }
            nodes.push(desc);
        }

        // --- preflight: domain pins ---
        let mut pins: Vec<(String, Option<u8>)> = Vec::new();
        for (slug, key) in staged.domain_pins {
            let existing = self
                .domain_pins
                .get(&slug)
                .copied()
                .or_else(|| pins.iter().find(|(s, _)| *s == slug).map(|(_, k)| *k));
            match existing {
                Some(existing_key) if existing_key == key => {}
                Some(_) => {
                    let owner = self
                        .domain_pin_owners
                        .get(&slug)
                        .cloned()
                        .unwrap_or_else(|| plugin_id.to_string());
                    warnings.push(format!(
                        "domain pin '{slug}' re-registered with a different ramp key by \
                         plugin '{plugin_id}' — keeping the registration from '{owner}'"
                    ));
                }
                None => pins.push((slug, key)),
            }
        }

        // --- preflight: migrations ---
        let mut migration_keys: BTreeSet<(String, u32)> = BTreeSet::new();
        for ((type_id, from_version), _) in &staged.migrations {
            let key = (type_id.clone(), *from_version);
            if self.migrations.contains_key(&key) || !migration_keys.insert(key) {
                return Err(RegistryError::DuplicateMigration {
                    type_id: type_id.clone(),
                    from_version: *from_version,
                });
            }
        }

        // --- commit (nothing below can fail) ---
        let report = MergeReport {
            nodes_registered: nodes.len(),
            domain_pins_registered: pins.len(),
            migrations_registered: staged.migrations.len(),
            warnings,
        };
        for desc in nodes {
            self.nodes.insert(desc.id.clone(), desc);
        }
        for (slug, key) in pins {
            self.domain_pin_owners
                .insert(slug.clone(), plugin_id.to_string());
            self.domain_pins.insert(slug, key);
        }
        for (key, step) in staged.migrations {
            self.migrations.insert(key, step);
        }

        Ok(report)
    }

    pub fn get(&self, id: &str) -> Option<&NodeDescriptor> {
        self.nodes.get(id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &NodeDescriptor> {
        self.nodes.values()
    }

    /// Descriptors grouped by category, both levels deterministically
    /// ordered — feeds the node-create search menu.
    pub fn by_category(&self) -> BTreeMap<&str, Vec<&NodeDescriptor>> {
        let mut out: BTreeMap<&str, Vec<&NodeDescriptor>> = BTreeMap::new();
        for d in self.nodes.values() {
            out.entry(d.category.as_str()).or_default().push(d);
        }
        out
    }

    /// Register the migration step for `type_id` from `from_version` to
    /// `from_version + 1`. Steps chain: an instance at v1 with the type at
    /// v3 runs the 1→2 then 2→3 steps in order.
    pub fn register_migration(
        &mut self,
        type_id: &str,
        from_version: u32,
        step: impl Fn(&mut crate::migrate::MigrationCtx) + Send + Sync + 'static,
    ) {
        self.migrations
            .insert((type_id.to_string(), from_version), Box::new(step));
    }

    pub fn migration(&self, type_id: &str, from_version: u32) -> Option<&MigrationFn> {
        self.migrations.get(&(type_id.to_string(), from_version))
    }

    /// Register a consumer domain pin type (e.g. `"shader"` for Task 50) so
    /// validation accepts `PinType::Domain("shader")`. Unkeyed: the theme
    /// derives a stable ramp hue by hashing the slug.
    pub fn register_domain_pin(&mut self, slug: &str) {
        self.domain_pins.insert(slug.to_string(), None);
    }

    /// Same, but pins the domain to an explicit ramp index (0..12) so a
    /// consumer can choose its own hue instead of taking the hash.
    pub fn register_domain_pin_keyed(&mut self, slug: &str, ramp_index: u8) {
        self.domain_pins.insert(slug.to_string(), Some(ramp_index));
    }

    /// Register a keyed domain pin whose wires are **flow, not values**: its
    /// inputs may fan in and its wires may loop (the Exec exemptions), while
    /// its outputs keep the data rule and may fan out. Direct-registration
    /// only for now — no plugin has staged one, and `StagedRegistry` grows
    /// the field when one does.
    pub fn register_domain_pin_flow(&mut self, slug: &str, ramp_index: u8) {
        self.domain_pins.insert(slug.to_string(), Some(ramp_index));
        self.flow_domains.insert(slug.to_string());
    }

    /// Is `slug` a registered flow-like domain?
    pub fn domain_is_flow(&self, slug: &str) -> bool {
        self.flow_domains.contains(slug)
    }

    /// Theme-side lookup: outer `Option` = registered?, inner = explicit key.
    /// The three cases drive `theme::tokens::domain_ramp_index`.
    pub fn domain_registration(&self, slug: &str) -> Option<Option<u8>> {
        self.domain_pins.get(slug).copied()
    }

    pub fn domain_pin_registered(&self, slug: &str) -> bool {
        self.domain_pins.contains_key(slug)
    }
}

#[cfg(test)]
// Tests build documents the way an author does: start from the default and
// fill in what matters. One giant struct literal per fixture would satisfy
// clippy and read markedly worse.
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;

    fn pure_add() -> NodeDescriptor {
        NodeDescriptor {
            id: "test_add".into(),
            name: "Add".into(),
            category: "Math".into(),
            version: 1,
            inputs: vec![
                PinDescriptor::new("a", "A", PinType::Float)
                    .with_default(PropValue::Float(0.0)),
                PinDescriptor::new("b", "B", PinType::Float)
                    .with_default(PropValue::Float(0.0)),
            ],
            outputs: vec![PinDescriptor::new("sum", "Sum", PinType::Float)],
            pure: true,
            realm: NodeRealm::Shared,
            deterministic: true,
            doc: None,
            preview: None,
        }
    }

    #[test]
    fn runtime_registration_round_trip() {
        // The 39.8 plugin call pattern: register at runtime, look up by
        // slug, appear in the category listing.
        let mut reg = NodeRegistry::new();
        reg.register(pure_add()).unwrap();
        assert_eq!(reg.get("test_add").unwrap().name, "Add");
        let cats = reg.by_category();
        assert_eq!(cats["Math"].len(), 1);
    }

    #[test]
    fn duplicate_id_rejected() {
        let mut reg = NodeRegistry::new();
        reg.register(pure_add()).unwrap();
        assert_eq!(
            reg.register(pure_add()),
            Err(RegistryError::DuplicateId("test_add".into()))
        );
    }

    #[test]
    fn descriptor_invariants_enforced() {
        let mut reg = NodeRegistry::new();

        let mut pure_with_exec = pure_add();
        pure_with_exec.id = "bad_pure".into();
        pure_with_exec
            .outputs
            .push(PinDescriptor::new("exec_out", "", PinType::Exec));
        assert!(reg.register(pure_with_exec).is_err());

        let mut impure_no_exec = pure_add();
        impure_no_exec.id = "bad_impure".into();
        impure_no_exec.pure = false;
        assert!(reg.register(impure_no_exec).is_err());

        let mut dup_pin = pure_add();
        dup_pin.id = "bad_pins".into();
        dup_pin.inputs.push(PinDescriptor::new("a", "A again", PinType::Float));
        assert!(reg.register(dup_pin).is_err());

        // Every reserved slug is refused: those types take their pins from
        // the document, so a descriptor claiming one would be shadowed.
        for id in RESERVED_TYPE_IDS {
            let mut reserved = pure_add();
            reserved.id = id.into();
            assert!(
                reg.register(reserved).is_err(),
                "'{id}' is reserved: it has no descriptor by design"
            );
            // …by the staged path too, where it is a hard error rather than a
            // skip-with-warning (a bug in the plugin, not two plugins
            // disagreeing).
            let mut staged = StagedRegistry::default();
            let mut d = pure_add();
            d.id = id.into();
            staged.nodes.push(d);
            assert!(reg.merge_staged("p", staged).is_err(), "staged '{id}'");
        }
    }

    #[test]
    fn domain_registration_three_cases() {
        let mut reg = NodeRegistry::new();
        reg.register_domain_pin("shader");
        reg.register_domain_pin_keyed("curve", 7);

        assert_eq!(reg.domain_registration("shader"), Some(None));
        assert_eq!(reg.domain_registration("curve"), Some(Some(7)));
        assert_eq!(reg.domain_registration("missing"), None);

        assert!(reg.domain_pin_registered("shader"));
        assert!(reg.domain_pin_registered("curve"));
        assert!(!reg.domain_pin_registered("missing"));

        // Re-registering unkeyed clears an explicit key (last write wins).
        reg.register_domain_pin("curve");
        assert_eq!(reg.domain_registration("curve"), Some(None));
    }
}
