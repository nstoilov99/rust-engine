//! Inventory-based auto-registration backend for derived node types (Task 40
//! P8, plan D9).
//!
//! This is *one optional backend*, not the primary path. Manual
//! [`NodeRegistry::register`](super::registry::NodeRegistry::register) stays
//! first-class (39.8 plugins, hot-reload) — there is no global registry
//! singleton here. A derive with `#[node(..., auto_register)]` emits an
//! `inventory::submit!` of a [`NodeFactory`]; a consumer that wants the
//! link-time-collected set calls [`register_inventory_nodes`] against its own
//! registry instance.

use super::registry::{NodeDescriptor, NodeRegistry, RegistryError};

/// A collected descriptor factory. The derive macro submits one of these per
/// `auto_register` node type; `inventory` gathers them across the linked
/// binary.
pub struct NodeFactory(pub fn() -> NodeDescriptor);

inventory::collect!(NodeFactory);

/// Register every `auto_register` node type collected by `inventory` into
/// `reg`. Fails fast on the first registry error (duplicate slug, invalid
/// descriptor). Iteration order follows `inventory`'s link order; the registry
/// itself keeps deterministic (BTree) storage.
pub fn register_inventory_nodes(reg: &mut NodeRegistry) -> Result<(), RegistryError> {
    for factory in inventory::iter::<NodeFactory> {
        reg.register((factory.0)())?;
    }
    Ok(())
}
