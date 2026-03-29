//! System access declarations and conflict detection.
//!
//! Every system declares what components and resources it reads and writes.
//! The scheduler validates at startup that no two systems in the same stage
//! have unresolved conflicts (write-write or read-write on the same type).

use std::any::TypeId;
use std::collections::{HashMap, HashSet};
use std::fmt;

/// Tracks which component and resource types a system reads/writes.
#[derive(Debug, Clone)]
pub struct AccessSet {
    /// Component types read.
    pub component_reads: HashSet<TypeId>,
    /// Component types written.
    pub component_writes: HashSet<TypeId>,
    /// Resource types read.
    pub resource_reads: HashSet<TypeId>,
    /// Resource types written.
    pub resource_writes: HashSet<TypeId>,
    /// If true, system requires exclusive World access (no parallelism).
    pub exclusive: bool,
}

impl AccessSet {
    pub fn new() -> Self {
        Self {
            component_reads: HashSet::new(),
            component_writes: HashSet::new(),
            resource_reads: HashSet::new(),
            resource_writes: HashSet::new(),
            exclusive: false,
        }
    }

    /// Create an exclusive access set (conflicts with everything).
    pub fn exclusive() -> Self {
        Self {
            exclusive: true,
            ..Self::new()
        }
    }

    /// Returns `Some(Conflict)` if `self` and `other` cannot safely run in parallel.
    ///
    /// Conflict rules:
    /// - Either is exclusive → conflict
    /// - Write-write on the same type → conflict
    /// - Read-write on the same type → conflict
    /// - Read-read is always safe
    pub fn conflicts_with(
        &self,
        other: &AccessSet,
        type_names: &HashMap<TypeId, &'static str>,
    ) -> Option<Conflict> {
        if self.exclusive || other.exclusive {
            return Some(Conflict {
                kind: ConflictKind::Exclusive,
                type_name: None,
            });
        }

        // Check component write-write
        for tid in &self.component_writes {
            if other.component_writes.contains(tid) {
                return Some(Conflict {
                    kind: ConflictKind::WriteWrite,
                    type_name: type_names.get(tid).copied(),
                });
            }
        }
        // Check component read-write (both directions)
        for tid in &self.component_writes {
            if other.component_reads.contains(tid) {
                return Some(Conflict {
                    kind: ConflictKind::ReadWrite,
                    type_name: type_names.get(tid).copied(),
                });
            }
        }
        for tid in &self.component_reads {
            if other.component_writes.contains(tid) {
                return Some(Conflict {
                    kind: ConflictKind::ReadWrite,
                    type_name: type_names.get(tid).copied(),
                });
            }
        }

        // Check resource write-write
        for tid in &self.resource_writes {
            if other.resource_writes.contains(tid) {
                return Some(Conflict {
                    kind: ConflictKind::WriteWrite,
                    type_name: type_names.get(tid).copied(),
                });
            }
        }
        // Check resource read-write (both directions)
        for tid in &self.resource_writes {
            if other.resource_reads.contains(tid) {
                return Some(Conflict {
                    kind: ConflictKind::ReadWrite,
                    type_name: type_names.get(tid).copied(),
                });
            }
        }
        for tid in &self.resource_reads {
            if other.resource_writes.contains(tid) {
                return Some(Conflict {
                    kind: ConflictKind::ReadWrite,
                    type_name: type_names.get(tid).copied(),
                });
            }
        }

        None
    }
}

impl Default for AccessSet {
    fn default() -> Self {
        Self::new()
    }
}

/// Describes what kind of access conflict exists.
#[derive(Debug, Clone)]
pub struct Conflict {
    pub kind: ConflictKind,
    /// Human-readable type name of the conflicting type, if available.
    pub type_name: Option<&'static str>,
}

impl fmt::Display for Conflict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (&self.kind, self.type_name) {
            (ConflictKind::Exclusive, _) => write!(f, "exclusive access"),
            (ConflictKind::WriteWrite, Some(name)) => write!(f, "write-write on {name}"),
            (ConflictKind::WriteWrite, None) => write!(f, "write-write on unknown type"),
            (ConflictKind::ReadWrite, Some(name)) => write!(f, "read-write on {name}"),
            (ConflictKind::ReadWrite, None) => write!(f, "read-write on unknown type"),
        }
    }
}

/// The kind of access conflict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConflictKind {
    /// One or both systems are exclusive.
    Exclusive,
    /// Both systems write the same type.
    WriteWrite,
    /// One system reads and the other writes the same type.
    ReadWrite,
}

/// Describes a system's access pattern and ordering constraints.
///
/// Built via the builder pattern:
/// ```ignore
/// SystemDescriptor::new("PhysicsStep")
///     .writes::<Transform>()
///     .reads_resource::<Time>()
///     .writes_resource::<PhysicsWorld>()
///     .after("AnimationUpdate")
/// ```
#[derive(Debug, Clone)]
pub struct SystemDescriptor {
    /// Unique name for this system (used in ordering and error messages).
    pub name: String,
    /// The access set declaring reads/writes.
    pub access: AccessSet,
    /// Systems that must run before this one (within the same stage).
    pub after: Vec<String>,
    /// Systems that must run after this one (within the same stage).
    pub before: Vec<String>,
    /// Type names collected during builder calls for human-readable messages.
    pub type_names: HashMap<TypeId, &'static str>,
}

impl SystemDescriptor {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            access: AccessSet::new(),
            after: Vec::new(),
            before: Vec::new(),
            type_names: HashMap::new(),
        }
    }

    /// Declare that this system reads component type `T`.
    pub fn reads<T: 'static>(mut self) -> Self {
        let tid = TypeId::of::<T>();
        self.access.component_reads.insert(tid);
        self.type_names
            .entry(tid)
            .or_insert_with(std::any::type_name::<T>);
        self
    }

    /// Declare that this system writes component type `T`.
    pub fn writes<T: 'static>(mut self) -> Self {
        let tid = TypeId::of::<T>();
        self.access.component_writes.insert(tid);
        self.type_names
            .entry(tid)
            .or_insert_with(std::any::type_name::<T>);
        self
    }

    /// Declare that this system reads resource type `T`.
    pub fn reads_resource<T: 'static>(mut self) -> Self {
        let tid = TypeId::of::<T>();
        self.access.resource_reads.insert(tid);
        self.type_names
            .entry(tid)
            .or_insert_with(std::any::type_name::<T>);
        self
    }

    /// Declare that this system writes resource type `T`.
    pub fn writes_resource<T: 'static>(mut self) -> Self {
        let tid = TypeId::of::<T>();
        self.access.resource_writes.insert(tid);
        self.type_names
            .entry(tid)
            .or_insert_with(std::any::type_name::<T>);
        self
    }

    /// Mark this system as requiring exclusive World access.
    pub fn exclusive(mut self) -> Self {
        self.access.exclusive = true;
        self
    }

    /// This system must run after the named system (within the same stage).
    pub fn after(mut self, name: impl Into<String>) -> Self {
        self.after.push(name.into());
        self
    }

    /// This system must run before the named system (within the same stage).
    pub fn before(mut self, name: impl Into<String>) -> Self {
        self.before.push(name.into());
        self
    }
}

/// Errors detected during schedule validation.
#[derive(Debug, Clone)]
pub enum ValidationError {
    /// Two systems in the same stage have conflicting access with no ordering.
    UnresolvedConflict {
        stage: &'static str,
        system_a: String,
        system_b: String,
        conflict: String,
    },
    /// Two systems share the same name.
    DuplicateName {
        name: String,
    },
    /// A system references a nonexistent system in after/before.
    DanglingReference {
        system: String,
        references: String,
        kind: &'static str,
    },
    /// Circular dependency in ordering constraints.
    CircularDependency {
        stage: &'static str,
        cycle: Vec<String>,
    },
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ValidationError::UnresolvedConflict {
                stage,
                system_a,
                system_b,
                conflict,
            } => write!(
                f,
                "unresolved conflict in stage {stage}: \"{system_a}\" and \"{system_b}\" ({conflict})"
            ),
            ValidationError::DuplicateName { name } => {
                write!(f, "duplicate system name: \"{name}\"")
            }
            ValidationError::DanglingReference {
                system,
                references,
                kind,
            } => write!(
                f,
                "system \"{system}\" has {kind}(\"{references}\") but no system named \"{references}\" exists"
            ),
            ValidationError::CircularDependency { stage, cycle } => {
                write!(
                    f,
                    "circular dependency in stage {stage}: {}",
                    cycle.join(" -> ")
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Dummy types for testing.
    struct CompA;
    struct CompB;
    struct ResX;
    struct ResY;

    #[test]
    fn read_read_no_conflict() {
        let mut a = AccessSet::new();
        a.component_reads.insert(TypeId::of::<CompA>());
        let mut b = AccessSet::new();
        b.component_reads.insert(TypeId::of::<CompA>());
        assert!(a.conflicts_with(&b, &HashMap::new()).is_none());
    }

    #[test]
    fn write_write_conflict() {
        let mut a = AccessSet::new();
        a.component_writes.insert(TypeId::of::<CompA>());
        let mut b = AccessSet::new();
        b.component_writes.insert(TypeId::of::<CompA>());
        let conflict = a.conflicts_with(&b, &HashMap::new()).unwrap();
        assert_eq!(conflict.kind, ConflictKind::WriteWrite);
    }

    #[test]
    fn read_write_conflict() {
        let mut a = AccessSet::new();
        a.component_reads.insert(TypeId::of::<CompA>());
        let mut b = AccessSet::new();
        b.component_writes.insert(TypeId::of::<CompA>());
        let conflict = a.conflicts_with(&b, &HashMap::new()).unwrap();
        assert_eq!(conflict.kind, ConflictKind::ReadWrite);
    }

    #[test]
    fn exclusive_conflicts_with_anything() {
        let a = AccessSet::exclusive();
        let b = AccessSet::new();
        let conflict = a.conflicts_with(&b, &HashMap::new()).unwrap();
        assert_eq!(conflict.kind, ConflictKind::Exclusive);
    }

    #[test]
    fn resource_write_write_conflict() {
        let mut a = AccessSet::new();
        a.resource_writes.insert(TypeId::of::<ResX>());
        let mut b = AccessSet::new();
        b.resource_writes.insert(TypeId::of::<ResX>());
        let conflict = a.conflicts_with(&b, &HashMap::new()).unwrap();
        assert_eq!(conflict.kind, ConflictKind::WriteWrite);
    }

    #[test]
    fn resource_read_write_conflict() {
        let mut a = AccessSet::new();
        a.resource_reads.insert(TypeId::of::<ResX>());
        let mut b = AccessSet::new();
        b.resource_writes.insert(TypeId::of::<ResX>());
        let conflict = a.conflicts_with(&b, &HashMap::new()).unwrap();
        assert_eq!(conflict.kind, ConflictKind::ReadWrite);
    }

    #[test]
    fn disjoint_access_no_conflict() {
        let mut a = AccessSet::new();
        a.component_writes.insert(TypeId::of::<CompA>());
        a.resource_reads.insert(TypeId::of::<ResX>());
        let mut b = AccessSet::new();
        b.component_writes.insert(TypeId::of::<CompB>());
        b.resource_reads.insert(TypeId::of::<ResY>());
        assert!(a.conflicts_with(&b, &HashMap::new()).is_none());
    }

    #[test]
    fn conflict_includes_type_name() {
        let mut names = HashMap::new();
        names.insert(TypeId::of::<CompA>(), "CompA");
        let mut a = AccessSet::new();
        a.component_writes.insert(TypeId::of::<CompA>());
        let mut b = AccessSet::new();
        b.component_writes.insert(TypeId::of::<CompA>());
        let conflict = a.conflicts_with(&b, &names).unwrap();
        assert_eq!(conflict.type_name, Some("CompA"));
    }

    #[test]
    fn descriptor_builder_collects_type_names() {
        let desc = SystemDescriptor::new("test")
            .reads::<CompA>()
            .writes::<CompB>()
            .reads_resource::<ResX>()
            .writes_resource::<ResY>();

        assert!(desc.access.component_reads.contains(&TypeId::of::<CompA>()));
        assert!(desc.access.component_writes.contains(&TypeId::of::<CompB>()));
        assert!(desc.access.resource_reads.contains(&TypeId::of::<ResX>()));
        assert!(desc.access.resource_writes.contains(&TypeId::of::<ResY>()));
        assert_eq!(desc.type_names.len(), 4);
    }

    #[test]
    fn descriptor_ordering_constraints() {
        let desc = SystemDescriptor::new("test")
            .after("A")
            .before("B")
            .after("C");
        assert_eq!(desc.after, vec!["A", "C"]);
        assert_eq!(desc.before, vec!["B"]);
    }
}

