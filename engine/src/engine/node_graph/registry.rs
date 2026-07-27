//! `NodeRegistry` — the registration contract of the node graph framework.
//!
//! Runtime registration is the primary API: Task 39.8's plugins call
//! [`NodeRegistry::register`] from `Plugin::build`, the Phase B derive-macro
//! layer is just another caller, and nothing here assumes static linking or
//! a global singleton. Node types are registered by stable slug, looked up
//! by slug, displayed by name/category.

use std::collections::{BTreeMap, BTreeSet};

use super::doc::{NodeRealm, PinType, PropValue};

/// Reserved node-type slug for subgraph instances — their pins derive from
/// the referenced `.subgraph` asset's interface, not from a descriptor.
pub const SUBGRAPH_TYPE_ID: &str = "subgraph";

#[derive(Debug, Clone, PartialEq)]
pub struct PinDescriptor {
    /// Stable pin slug — serialized in edges and properties.
    pub slug: String,
    /// Display label, free to change.
    pub label: String,
    pub ty: PinType,
    /// Constant shown/stored when the pin is unconnected (input pins only).
    pub default: Option<PropValue>,
}

impl PinDescriptor {
    pub fn new(slug: &str, label: &str, ty: PinType) -> Self {
        Self { slug: slug.to_string(), label: label.to_string(), ty, default: None }
    }

    pub fn with_default(mut self, v: PropValue) -> Self {
        self.default = Some(v);
        self
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
    InvalidDescriptor { id: String, reason: String },
}

impl std::fmt::Display for RegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegistryError::DuplicateId(id) => write!(f, "node type '{id}' already registered"),
            RegistryError::InvalidDescriptor { id, reason } => {
                write!(f, "invalid node descriptor '{id}': {reason}")
            }
        }
    }
}

impl std::error::Error for RegistryError {}

/// A single migration step for one node type, from one version to the next.
pub type MigrationFn = Box<dyn Fn(&mut super::migrate::MigrationCtx) + Send + Sync>;

/// Registry of node types, consumer domain pin types, and migration chains.
/// `BTreeMap` keeps iteration deterministic (stable search-menu ordering,
/// stable tests).
#[derive(Default)]
pub struct NodeRegistry {
    nodes: BTreeMap<String, NodeDescriptor>,
    domain_pins: BTreeSet<String>,
    migrations: BTreeMap<(String, u32), MigrationFn>,
}

impl NodeRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a node type. Errors on slug collision and on descriptor-
    /// level invariant violations (the checks that don't need a document):
    /// pure nodes may not have exec pins, impure nodes require exec flow,
    /// pin slugs must be unique per side, the subgraph slug is reserved.
    pub fn register(&mut self, desc: NodeDescriptor) -> Result<(), RegistryError> {
        let invalid = |reason: &str| {
            Err(RegistryError::InvalidDescriptor {
                id: desc.id.clone(),
                reason: reason.to_string(),
            })
        };
        if desc.id == SUBGRAPH_TYPE_ID {
            return invalid("slug is reserved for subgraph instances");
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
        if self.nodes.contains_key(&desc.id) {
            return Err(RegistryError::DuplicateId(desc.id));
        }
        self.nodes.insert(desc.id.clone(), desc);
        Ok(())
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
        step: impl Fn(&mut super::migrate::MigrationCtx) + Send + Sync + 'static,
    ) {
        self.migrations
            .insert((type_id.to_string(), from_version), Box::new(step));
    }

    pub fn migration(&self, type_id: &str, from_version: u32) -> Option<&MigrationFn> {
        self.migrations.get(&(type_id.to_string(), from_version))
    }

    /// Register a consumer domain pin type (e.g. `"shader"` for Task 50) so
    /// validation accepts `PinType::Domain("shader")`.
    pub fn register_domain_pin(&mut self, slug: &str) {
        self.domain_pins.insert(slug.to_string());
    }

    pub fn domain_pin_registered(&self, slug: &str) -> bool {
        self.domain_pins.contains(slug)
    }
}

#[cfg(test)]
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

        let mut reserved = pure_add();
        reserved.id = SUBGRAPH_TYPE_ID.into();
        assert!(reg.register(reserved).is_err());
    }
}
