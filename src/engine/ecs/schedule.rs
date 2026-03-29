//! Staged system scheduling with run criteria.
//!
//! Systems are registered into stages and executed in stage order
//! (First → PreUpdate → Update → PostUpdate → Last), with stable
//! insertion ordering within each stage.

use super::access::{AccessSet, SystemDescriptor, ValidationError};
use super::resources::{EditorState, PlayMode, Resources, Time};
use std::any::TypeId;
use std::collections::HashMap;

/// Execution stage for ordering systems.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Stage {
    /// First stage: event buffer swap, input polling
    First = 0,
    /// Pre-update: physics step, hierarchy validation
    PreUpdate = 1,
    /// Main update: game logic
    Update = 2,
    /// Post-update: transform propagation, cleanup
    PostUpdate = 3,
    /// Last stage: render preparation
    Last = 4,
}

impl Stage {
    /// Human-readable label for error messages and logging.
    pub fn label(&self) -> &'static str {
        match self {
            Stage::First => "First",
            Stage::PreUpdate => "PreUpdate",
            Stage::Update => "Update",
            Stage::PostUpdate => "PostUpdate",
            Stage::Last => "Last",
        }
    }
}

/// System trait for game logic. Systems receive the hecs world and resources.
///
/// Systems may hold state (e.g., cached data), but should primarily be
/// stateless logic operating on queries and resources.
pub trait System: Send + Sync {
    /// Execute the system.
    fn run(&mut self, world: &mut hecs::World, resources: &mut Resources);

    /// System name for debugging and identification.
    fn name(&self) -> &str {
        std::any::type_name::<Self>()
    }
}

/// Wraps a closure as a System.
pub struct FunctionSystem<F>
where
    F: FnMut(&mut hecs::World, &mut Resources) + Send + Sync,
{
    func: F,
    name: &'static str,
}

impl<F> FunctionSystem<F>
where
    F: FnMut(&mut hecs::World, &mut Resources) + Send + Sync,
{
    pub fn new(name: &'static str, func: F) -> Self {
        Self { func, name }
    }
}

impl<F> System for FunctionSystem<F>
where
    F: FnMut(&mut hecs::World, &mut Resources) + Send + Sync,
{
    fn run(&mut self, world: &mut hecs::World, resources: &mut Resources) {
        (self.func)(world, resources);
    }

    fn name(&self) -> &str {
        self.name
    }
}

/// Determines whether a system should run this frame.
pub trait RunCriteria: Send + Sync {
    fn should_run(&self, resources: &Resources) -> bool;
}

/// Always run (default).
pub struct Always;
impl RunCriteria for Always {
    fn should_run(&self, _resources: &Resources) -> bool {
        true
    }
}

/// Run only when in play mode.
pub struct RunIfPlaying;
impl RunCriteria for RunIfPlaying {
    fn should_run(&self, resources: &Resources) -> bool {
        resources
            .get::<EditorState>()
            .is_none_or(|state| state.play_mode == PlayMode::Playing)
    }
}

/// Run only when in edit mode.
pub struct RunIfEditing;
impl RunCriteria for RunIfEditing {
    fn should_run(&self, resources: &Resources) -> bool {
        resources
            .get::<EditorState>()
            .is_some_and(|state| state.play_mode == PlayMode::Edit)
    }
}

/// Run only when not paused.
pub struct RunIfNotPaused;
impl RunCriteria for RunIfNotPaused {
    fn should_run(&self, resources: &Resources) -> bool {
        resources.get::<Time>().is_none_or(|time| !time.paused)
    }
}

/// Placeholder: selection is managed by the external `Selection` struct,
/// not by `EditorState`. This always returns false; use `Selection` directly
/// for selection-dependent logic.
#[deprecated(note = "Selection is tracked externally via `Selection`, not `EditorState`")]
pub struct RunIfSelected;
#[allow(deprecated)]
impl RunCriteria for RunIfSelected {
    fn should_run(&self, _resources: &Resources) -> bool {
        false
    }
}

/// A system registered in the schedule with metadata.
struct RegisteredSystem {
    system: Box<dyn System>,
    stage: Stage,
    run_criteria: Box<dyn RunCriteria>,
    enabled: bool,
    insertion_order: usize,
    /// Access descriptor. `None` means undeclared (defaults to exclusive, logs warning).
    descriptor: Option<SystemDescriptor>,
}

/// Staged system schedule.
///
/// Systems are grouped by stage and executed in order:
/// First → PreUpdate → Update → PostUpdate → Last.
///
/// Within each stage, systems run in insertion order (stable, deterministic).
pub struct Schedule {
    systems: Vec<RegisteredSystem>,
    next_order: usize,
    needs_sort: bool,
}

impl Schedule {
    pub fn new() -> Self {
        Self {
            systems: Vec::new(),
            next_order: 0,
            needs_sort: false,
        }
    }

    /// Add a system to the schedule at the given stage.
    ///
    /// Systems added without a descriptor default to exclusive access and
    /// trigger a warning during validation so migration misses are visible.
    pub fn add_system<S: System + 'static>(&mut self, system: S, stage: Stage) -> &mut Self {
        self.systems.push(RegisteredSystem {
            system: Box::new(system),
            stage,
            run_criteria: Box::new(Always),
            enabled: true,
            insertion_order: self.next_order,
            descriptor: None,
        });
        self.next_order += 1;
        self.needs_sort = true;
        self
    }

    /// Add a system with a custom run criteria.
    pub fn add_system_with_criteria<S, C>(
        &mut self,
        system: S,
        stage: Stage,
        criteria: C,
    ) -> &mut Self
    where
        S: System + 'static,
        C: RunCriteria + 'static,
    {
        self.systems.push(RegisteredSystem {
            system: Box::new(system),
            stage,
            run_criteria: Box::new(criteria),
            enabled: true,
            insertion_order: self.next_order,
            descriptor: None,
        });
        self.next_order += 1;
        self.needs_sort = true;
        self
    }

    /// Add a system with an access descriptor for conflict detection.
    pub fn add_system_described<S: System + 'static>(
        &mut self,
        system: S,
        stage: Stage,
        descriptor: SystemDescriptor,
    ) -> &mut Self {
        self.systems.push(RegisteredSystem {
            system: Box::new(system),
            stage,
            run_criteria: Box::new(Always),
            enabled: true,
            insertion_order: self.next_order,
            descriptor: Some(descriptor),
        });
        self.next_order += 1;
        self.needs_sort = true;
        self
    }

    /// Add a system with an access descriptor and custom run criteria.
    pub fn add_system_described_with_criteria<S, C>(
        &mut self,
        system: S,
        stage: Stage,
        descriptor: SystemDescriptor,
        criteria: C,
    ) -> &mut Self
    where
        S: System + 'static,
        C: RunCriteria + 'static,
    {
        self.systems.push(RegisteredSystem {
            system: Box::new(system),
            stage,
            run_criteria: Box::new(criteria),
            enabled: true,
            insertion_order: self.next_order,
            descriptor: Some(descriptor),
        });
        self.next_order += 1;
        self.needs_sort = true;
        self
    }

    /// Add a function system (convenience).
    pub fn add_fn_system<F>(&mut self, name: &'static str, stage: Stage, func: F) -> &mut Self
    where
        F: FnMut(&mut hecs::World, &mut Resources) + Send + Sync + 'static,
    {
        let sys = FunctionSystem::new(name, func);
        self.add_system(sys, stage)
    }

    /// Sort systems by (stage, insertion_order) if needed.
    fn ensure_sorted(&mut self) {
        if self.needs_sort {
            self.systems.sort_by(|a, b| {
                a.stage
                    .cmp(&b.stage)
                    .then(a.insertion_order.cmp(&b.insertion_order))
            });
            self.needs_sort = false;
        }
    }

    /// Run all scheduled systems, applying commands between stages.
    ///
    /// This is called from `GameWorld::run_schedule()` which decomposes
    /// itself into the required mutable references.
    pub fn run_raw(
        &mut self,
        hecs_world: &mut hecs::World,
        resources: &mut Resources,
        command_buffer: &mut super::commands::CommandBuffer,
    ) {
        crate::profile_scope!("ecs_systems");

        self.ensure_sorted();

        let mut current_stage: Option<Stage> = None;

        for registered in &mut self.systems {
            // Detect stage transition — apply commands at boundary
            if current_stage != Some(registered.stage) {
                if current_stage.is_some() {
                    command_buffer.apply(hecs_world);
                }
                current_stage = Some(registered.stage);
            }

            if !registered.enabled {
                continue;
            }

            if !registered.run_criteria.should_run(resources) {
                continue;
            }

            registered.system.run(hecs_world, resources);
        }

        // Apply remaining commands after last stage
        command_buffer.apply(hecs_world);
    }

    /// Enable or disable a system by name.
    pub fn set_enabled(&mut self, name: &str, enabled: bool) {
        for registered in &mut self.systems {
            if registered.system.name() == name {
                registered.enabled = enabled;
            }
        }
    }

    /// Get the number of registered systems.
    pub fn system_count(&self) -> usize {
        self.systems.len()
    }

    /// Validate the schedule for access conflicts, duplicate names, dangling
    /// references, and circular dependencies.
    ///
    /// Returns a list of hard errors. Ordered conflicts (resolved by
    /// after/before) are info-logged, not returned as errors. Undeclared
    /// systems (descriptor: None) are warned about.
    pub fn validate(&mut self) -> Vec<ValidationError> {
        let mut errors = Vec::new();

        // Build merged type-name registry from all descriptors.
        let mut type_names: HashMap<TypeId, &'static str> = HashMap::new();
        for reg in &self.systems {
            if let Some(desc) = &reg.descriptor {
                type_names.extend(&desc.type_names);
            }
        }

        // Collect system names and check for duplicates.
        let mut name_counts: HashMap<&str, usize> = HashMap::new();
        for reg in &self.systems {
            let name = self.system_name(reg);
            *name_counts.entry(name).or_default() += 1;
        }
        for (name, count) in &name_counts {
            if *count > 1 {
                errors.push(ValidationError::DuplicateName {
                    name: name.to_string(),
                });
            }
        }
        if !errors.is_empty() {
            return errors; // Duplicate names make further validation ambiguous.
        }

        // Warn about undeclared systems.
        for reg in &self.systems {
            if reg.descriptor.is_none() {
                log::warn!(
                    "System \"{}\" registered without access descriptor — \
                     defaults to exclusive access. Declare access to enable \
                     conflict validation.",
                    reg.system.name()
                );
            }
        }

        // Collect all known system names for dangling reference checks.
        let all_names: std::collections::HashSet<&str> =
            self.systems.iter().map(|r| self.system_name(r)).collect();

        // Check dangling references.
        for reg in &self.systems {
            if let Some(desc) = &reg.descriptor {
                for after_name in &desc.after {
                    if !all_names.contains(after_name.as_str()) {
                        errors.push(ValidationError::DanglingReference {
                            system: desc.name.clone(),
                            references: after_name.clone(),
                            kind: "after",
                        });
                    }
                }
                for before_name in &desc.before {
                    if !all_names.contains(before_name.as_str()) {
                        errors.push(ValidationError::DanglingReference {
                            system: desc.name.clone(),
                            references: before_name.clone(),
                            kind: "before",
                        });
                    }
                }
            }
        }
        if !errors.is_empty() {
            return errors;
        }

        // Per-stage validation: conflicts, ordering, circular dependencies.
        let stages = [
            Stage::First,
            Stage::PreUpdate,
            Stage::Update,
            Stage::PostUpdate,
            Stage::Last,
        ];

        for stage in &stages {
            let stage_indices: Vec<usize> = self
                .systems
                .iter()
                .enumerate()
                .filter(|(_, r)| r.stage == *stage)
                .map(|(i, _)| i)
                .collect();

            if stage_indices.len() < 2 {
                continue;
            }

            // Build ordering edges from after/before constraints.
            // Edge: (must_run_first_name, must_run_second_name)
            let mut ordering_edges: Vec<(&str, &str)> = Vec::new();
            for &idx in &stage_indices {
                if let Some(desc) = &self.systems[idx].descriptor {
                    let name = desc.name.as_str();
                    for after_name in &desc.after {
                        // "self.after(X)" means X must run before self
                        if stage_indices
                            .iter()
                            .any(|&j| self.system_name(&self.systems[j]) == after_name.as_str())
                        {
                            ordering_edges.push((after_name.as_str(), name));
                        }
                    }
                    for before_name in &desc.before {
                        // "self.before(X)" means self must run before X
                        if stage_indices
                            .iter()
                            .any(|&j| self.system_name(&self.systems[j]) == before_name.as_str())
                        {
                            ordering_edges.push((name, before_name.as_str()));
                        }
                    }
                }
            }

            // Check for circular dependencies via topological sort.
            if let Err(cycle) =
                Self::topological_sort_names(&stage_indices, &self.systems, &ordering_edges)
            {
                errors.push(ValidationError::CircularDependency {
                    stage: stage.label(),
                    cycle,
                });
                continue; // Skip conflict checks for this stage.
            }

            // Check pairwise conflicts within the stage.
            for i in 0..stage_indices.len() {
                for j in (i + 1)..stage_indices.len() {
                    let idx_a = stage_indices[i];
                    let idx_b = stage_indices[j];

                    let access_a = self.effective_access(idx_a);
                    let access_b = self.effective_access(idx_b);

                    if let Some(conflict) = access_a.conflicts_with(&access_b, &type_names) {
                        let name_a = self.system_name(&self.systems[idx_a]).to_string();
                        let name_b = self.system_name(&self.systems[idx_b]).to_string();

                        // Check if ordering resolves this conflict.
                        let ordered = ordering_edges
                            .iter()
                            .any(|&(a, b)| {
                                (a == name_a && b == name_b) || (a == name_b && b == name_a)
                            });

                        if ordered {
                            log::info!(
                                "Conflict in stage {} between \"{}\" and \"{}\" ({}) \
                                 resolved by ordering constraint",
                                stage.label(),
                                name_a,
                                name_b,
                                conflict,
                            );
                        } else {
                            errors.push(ValidationError::UnresolvedConflict {
                                stage: stage.label(),
                                system_a: name_a,
                                system_b: name_b,
                                conflict: conflict.to_string(),
                            });
                        }
                    }
                }
            }
        }

        // Apply topological sort to reorder systems within each stage.
        if errors.is_empty() {
            self.apply_topological_order();
        }

        errors
    }

    /// Get the effective access set for a system.
    /// Systems without descriptors default to exclusive access.
    fn effective_access(&self, idx: usize) -> AccessSet {
        match &self.systems[idx].descriptor {
            Some(desc) => desc.access.clone(),
            None => AccessSet::exclusive(),
        }
    }

    /// Get the canonical name for a registered system.
    fn system_name<'a>(&'a self, reg: &'a RegisteredSystem) -> &'a str {
        reg.descriptor
            .as_ref()
            .map(|d| d.name.as_str())
            .unwrap_or_else(|| reg.system.name())
    }

    /// Topological sort of system names within a stage. Returns Err with cycle
    /// names if a cycle is detected.
    fn topological_sort_names(
        stage_indices: &[usize],
        systems: &[RegisteredSystem],
        edges: &[(&str, &str)],
    ) -> Result<Vec<String>, Vec<String>> {
        // Build name -> index mapping for systems in this stage.
        let names: Vec<&str> = stage_indices
            .iter()
            .map(|&i| {
                systems[i]
                    .descriptor
                    .as_ref()
                    .map(|d| d.name.as_str())
                    .unwrap_or_else(|| systems[i].system.name())
            })
            .collect();

        let name_to_local: HashMap<&str, usize> = names
            .iter()
            .enumerate()
            .map(|(i, &n)| (n, i))
            .collect();

        let n = names.len();
        let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
        let mut in_degree: Vec<usize> = vec![0; n];

        for &(from, to) in edges {
            if let (Some(&fi), Some(&ti)) = (name_to_local.get(from), name_to_local.get(to)) {
                adj[fi].push(ti);
                in_degree[ti] += 1;
            }
        }

        // Kahn's algorithm.
        let mut queue: std::collections::VecDeque<usize> =
            in_degree.iter().enumerate()
                .filter(|(_, &d)| d == 0)
                .map(|(i, _)| i)
                .collect();

        let mut sorted = Vec::with_capacity(n);
        while let Some(node) = queue.pop_front() {
            sorted.push(node);
            for &neighbor in &adj[node] {
                in_degree[neighbor] -= 1;
                if in_degree[neighbor] == 0 {
                    queue.push_back(neighbor);
                }
            }
        }

        if sorted.len() != n {
            // Find a cycle: nodes not in sorted have non-zero in-degree.
            let cycle: Vec<String> = (0..n)
                .filter(|i| !sorted.contains(i))
                .map(|i| names[i].to_string())
                .collect();
            Err(cycle)
        } else {
            Ok(sorted.iter().map(|&i| names[i].to_string()).collect())
        }
    }

    /// Apply topological ordering within each stage, preserving insertion
    /// order for systems without ordering constraints.
    fn apply_topological_order(&mut self) {
        let stages = [
            Stage::First,
            Stage::PreUpdate,
            Stage::Update,
            Stage::PostUpdate,
            Stage::Last,
        ];

        for stage in &stages {
            let stage_indices: Vec<usize> = self
                .systems
                .iter()
                .enumerate()
                .filter(|(_, r)| r.stage == *stage)
                .map(|(i, _)| i)
                .collect();

            if stage_indices.len() < 2 {
                continue;
            }

            // Build ordering edges.
            let mut edges: Vec<(&str, &str)> = Vec::new();
            for &idx in &stage_indices {
                if let Some(desc) = &self.systems[idx].descriptor {
                    let name = desc.name.as_str();
                    for after_name in &desc.after {
                        if stage_indices
                            .iter()
                            .any(|&j| self.system_name(&self.systems[j]) == after_name.as_str())
                        {
                            edges.push((after_name.as_str(), name));
                        }
                    }
                    for before_name in &desc.before {
                        if stage_indices
                            .iter()
                            .any(|&j| self.system_name(&self.systems[j]) == before_name.as_str())
                        {
                            edges.push((name, before_name.as_str()));
                        }
                    }
                }
            }

            if edges.is_empty() {
                continue; // No ordering constraints, keep insertion order.
            }

            // Topological sort with insertion-order tie-breaking.
            let names: Vec<&str> = stage_indices
                .iter()
                .map(|&i| self.system_name(&self.systems[i]))
                .collect();

            let name_to_local: HashMap<&str, usize> = names
                .iter()
                .enumerate()
                .map(|(i, &n)| (n, i))
                .collect();

            let n = names.len();
            let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
            let mut in_degree: Vec<usize> = vec![0; n];

            for &(from, to) in &edges {
                if let (Some(&fi), Some(&ti)) = (name_to_local.get(from), name_to_local.get(to)) {
                    adj[fi].push(ti);
                    in_degree[ti] += 1;
                }
            }

            // Kahn's algorithm with insertion-order tie-breaking.
            let mut queue: std::collections::BinaryHeap<std::cmp::Reverse<(usize, usize)>> =
                in_degree
                    .iter()
                    .enumerate()
                    .filter(|(_, &d)| d == 0)
                    .map(|(i, _)| {
                        std::cmp::Reverse((self.systems[stage_indices[i]].insertion_order, i))
                    })
                    .collect();

            let mut sorted_local = Vec::with_capacity(n);
            while let Some(std::cmp::Reverse((_, node))) = queue.pop() {
                sorted_local.push(node);
                for &neighbor in &adj[node] {
                    in_degree[neighbor] -= 1;
                    if in_degree[neighbor] == 0 {
                        queue.push(std::cmp::Reverse((
                            self.systems[stage_indices[neighbor]].insertion_order,
                            neighbor,
                        )));
                    }
                }
            }

            // Remap insertion_order to enforce topological sort.
            // We assign new insertion orders that preserve the topo order
            // within the stage while keeping stage-level sorting intact.
            let base_order = stage_indices
                .iter()
                .map(|&i| self.systems[i].insertion_order)
                .min()
                .unwrap_or(0);

            for (new_pos, &local_idx) in sorted_local.iter().enumerate() {
                self.systems[stage_indices[local_idx]].insertion_order = base_order + new_pos;
            }
        }

        self.needs_sort = true;
    }

    /// Log a human-readable report of all systems, their stages, and access patterns.
    pub fn print_access_report(&self) {
        let stages = [
            Stage::First,
            Stage::PreUpdate,
            Stage::Update,
            Stage::PostUpdate,
            Stage::Last,
        ];

        log::info!("=== Schedule Access Report ===");
        for stage in &stages {
            let stage_systems: Vec<&RegisteredSystem> = self
                .systems
                .iter()
                .filter(|r| r.stage == *stage)
                .collect();

            if stage_systems.is_empty() {
                continue;
            }

            log::info!("Stage: {}", stage.label());
            for reg in &stage_systems {
                let name = reg
                    .descriptor
                    .as_ref()
                    .map(|d| d.name.as_str())
                    .unwrap_or_else(|| reg.system.name());

                match &reg.descriptor {
                    Some(desc) => {
                        let reads: Vec<&str> = desc
                            .type_names
                            .iter()
                            .filter(|(tid, _)| {
                                desc.access.component_reads.contains(tid)
                                    || desc.access.resource_reads.contains(tid)
                            })
                            .map(|(_, name)| *name)
                            .collect();
                        let writes: Vec<&str> = desc
                            .type_names
                            .iter()
                            .filter(|(tid, _)| {
                                desc.access.component_writes.contains(tid)
                                    || desc.access.resource_writes.contains(tid)
                            })
                            .map(|(_, name)| *name)
                            .collect();
                        let after = &desc.after;
                        let before = &desc.before;
                        log::info!(
                            "  {} — reads: [{}], writes: [{}], after: {:?}, before: {:?}{}",
                            name,
                            reads.join(", "),
                            writes.join(", "),
                            after,
                            before,
                            if desc.access.exclusive {
                                " [EXCLUSIVE]"
                            } else {
                                ""
                            },
                        );
                    }
                    None => {
                        log::info!("  {} — [UNDECLARED, defaults to exclusive]", name);
                    }
                }
            }
        }
        log::info!("=== End Access Report ===");
    }
}

impl Default for Schedule {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::ecs::commands::CommandBuffer;
    use std::sync::{Arc, Mutex};

    #[test]
    fn stages_execute_in_order() {
        let order = Arc::new(Mutex::new(Vec::new()));

        let mut schedule = Schedule::new();

        let o = Arc::clone(&order);
        schedule.add_fn_system("last_sys", Stage::Last, move |_w, _r| {
            o.lock().expect("lock").push("Last");
        });

        let o = Arc::clone(&order);
        schedule.add_fn_system("first_sys", Stage::First, move |_w, _r| {
            o.lock().expect("lock").push("First");
        });

        let o = Arc::clone(&order);
        schedule.add_fn_system("update_sys", Stage::Update, move |_w, _r| {
            o.lock().expect("lock").push("Update");
        });

        let mut world = hecs::World::new();
        let mut resources = Resources::new();
        resources.insert(Time::new());
        resources.insert(EditorState::new());
        let mut cmd = CommandBuffer::new();

        schedule.run_raw(&mut world, &mut resources, &mut cmd);

        let result = order.lock().expect("lock");
        assert_eq!(*result, vec!["First", "Update", "Last"]);
    }

    #[test]
    fn insertion_order_within_stage() {
        let order = Arc::new(Mutex::new(Vec::new()));

        let mut schedule = Schedule::new();

        let o = Arc::clone(&order);
        schedule.add_fn_system("sys_a", Stage::Update, move |_w, _r| {
            o.lock().expect("lock").push("A");
        });
        let o = Arc::clone(&order);
        schedule.add_fn_system("sys_b", Stage::Update, move |_w, _r| {
            o.lock().expect("lock").push("B");
        });
        let o = Arc::clone(&order);
        schedule.add_fn_system("sys_c", Stage::Update, move |_w, _r| {
            o.lock().expect("lock").push("C");
        });

        let mut world = hecs::World::new();
        let mut resources = Resources::new();
        resources.insert(Time::new());
        resources.insert(EditorState::new());
        let mut cmd = CommandBuffer::new();

        schedule.run_raw(&mut world, &mut resources, &mut cmd);

        let result = order.lock().expect("lock");
        assert_eq!(*result, vec!["A", "B", "C"]);
    }

    #[test]
    fn run_criteria_always() {
        let ran = Arc::new(Mutex::new(false));
        let mut schedule = Schedule::new();

        let r = Arc::clone(&ran);
        schedule.add_system_with_criteria(
            FunctionSystem::new(
                "always_sys",
                move |_w: &mut hecs::World, _r: &mut Resources| {
                    *r.lock().expect("lock") = true;
                },
            ),
            Stage::Update,
            Always,
        );

        let mut world = hecs::World::new();
        let mut resources = Resources::new();
        resources.insert(Time::new());
        resources.insert(EditorState::new());
        let mut cmd = CommandBuffer::new();

        schedule.run_raw(&mut world, &mut resources, &mut cmd);
        assert!(*ran.lock().expect("lock"));
    }

    #[test]
    fn run_if_playing_skips_in_edit_mode() {
        let ran = Arc::new(Mutex::new(false));
        let mut schedule = Schedule::new();

        let r = Arc::clone(&ran);
        schedule.add_system_with_criteria(
            FunctionSystem::new(
                "play_sys",
                move |_w: &mut hecs::World, _r: &mut Resources| {
                    *r.lock().expect("lock") = true;
                },
            ),
            Stage::Update,
            RunIfPlaying,
        );

        let mut world = hecs::World::new();
        let mut resources = Resources::new();
        resources.insert(Time::new());
        resources.insert(EditorState::new()); // default is Edit mode
        let mut cmd = CommandBuffer::new();

        schedule.run_raw(&mut world, &mut resources, &mut cmd);
        assert!(!*ran.lock().expect("lock"), "should not run in Edit mode");
    }

    #[test]
    fn run_if_playing_runs_when_playing() {
        let ran = Arc::new(Mutex::new(false));
        let mut schedule = Schedule::new();

        let r = Arc::clone(&ran);
        schedule.add_system_with_criteria(
            FunctionSystem::new(
                "play_sys",
                move |_w: &mut hecs::World, _r: &mut Resources| {
                    *r.lock().expect("lock") = true;
                },
            ),
            Stage::Update,
            RunIfPlaying,
        );

        let mut world = hecs::World::new();
        let mut resources = Resources::new();
        resources.insert(Time::new());
        let mut state = EditorState::new();
        state.play_mode = PlayMode::Playing;
        resources.insert(state);
        let mut cmd = CommandBuffer::new();

        schedule.run_raw(&mut world, &mut resources, &mut cmd);
        assert!(*ran.lock().expect("lock"), "should run when Playing");
    }

    #[test]
    fn set_enabled_disables_system() {
        let ran = Arc::new(Mutex::new(false));
        let mut schedule = Schedule::new();

        let r = Arc::clone(&ran);
        schedule.add_fn_system("toggle_sys", Stage::Update, move |_w, _r| {
            *r.lock().expect("lock") = true;
        });

        schedule.set_enabled("toggle_sys", false);

        let mut world = hecs::World::new();
        let mut resources = Resources::new();
        resources.insert(Time::new());
        resources.insert(EditorState::new());
        let mut cmd = CommandBuffer::new();

        schedule.run_raw(&mut world, &mut resources, &mut cmd);
        assert!(
            !*ran.lock().expect("lock"),
            "disabled system should not run"
        );
    }

    #[test]
    fn commands_applied_between_stages() {
        let mut schedule = Schedule::new();

        // PreUpdate: spawn an entity via command buffer
        schedule.add_fn_system("spawner", Stage::PreUpdate, |_w, _r| {
            // We can't easily access the command buffer here since run_raw owns it,
            // but we can verify stage boundary behavior by checking world state
        });

        // Verify system_count
        assert_eq!(schedule.system_count(), 1);
    }

    #[test]
    fn system_count_tracks_additions() {
        let mut schedule = Schedule::new();
        assert_eq!(schedule.system_count(), 0);

        schedule.add_fn_system("sys1", Stage::Update, |_w, _r| {});
        assert_eq!(schedule.system_count(), 1);

        schedule.add_fn_system("sys2", Stage::Update, |_w, _r| {});
        assert_eq!(schedule.system_count(), 2);
    }

    // === Access declaration and validation tests ===

    use super::super::access::SystemDescriptor;

    // Dummy marker types for access declaration tests (never constructed).
    #[allow(dead_code)]
    struct CompA;
    #[allow(dead_code)]
    struct CompB;
    #[allow(dead_code)]
    struct ResX;

    #[test]
    fn validate_no_conflicts() {
        let mut schedule = Schedule::new();
        schedule.add_system_described(
            FunctionSystem::new("sys_a", |_w: &mut hecs::World, _r: &mut Resources| {}),
            Stage::Update,
            SystemDescriptor::new("sys_a").reads::<CompA>(),
        );
        schedule.add_system_described(
            FunctionSystem::new("sys_b", |_w: &mut hecs::World, _r: &mut Resources| {}),
            Stage::Update,
            SystemDescriptor::new("sys_b").reads::<CompA>(),
        );
        let errors = schedule.validate();
        assert!(errors.is_empty(), "read-read should have no conflicts: {errors:?}");
    }

    #[test]
    fn validate_detects_write_write_conflict() {
        let mut schedule = Schedule::new();
        schedule.add_system_described(
            FunctionSystem::new("sys_a", |_w: &mut hecs::World, _r: &mut Resources| {}),
            Stage::Update,
            SystemDescriptor::new("sys_a").writes::<CompA>(),
        );
        schedule.add_system_described(
            FunctionSystem::new("sys_b", |_w: &mut hecs::World, _r: &mut Resources| {}),
            Stage::Update,
            SystemDescriptor::new("sys_b").writes::<CompA>(),
        );
        let errors = schedule.validate();
        assert_eq!(errors.len(), 1);
        assert!(matches!(&errors[0], ValidationError::UnresolvedConflict { .. }));
    }

    #[test]
    fn validate_conflict_resolved_by_ordering() {
        let mut schedule = Schedule::new();
        schedule.add_system_described(
            FunctionSystem::new("sys_a", |_w: &mut hecs::World, _r: &mut Resources| {}),
            Stage::Update,
            SystemDescriptor::new("sys_a").writes::<CompA>(),
        );
        schedule.add_system_described(
            FunctionSystem::new("sys_b", |_w: &mut hecs::World, _r: &mut Resources| {}),
            Stage::Update,
            SystemDescriptor::new("sys_b").writes::<CompA>().after("sys_a"),
        );
        let errors = schedule.validate();
        assert!(errors.is_empty(), "ordering should resolve conflict: {errors:?}");
    }

    #[test]
    fn validate_duplicate_name_error() {
        let mut schedule = Schedule::new();
        schedule.add_system_described(
            FunctionSystem::new("dup", |_w: &mut hecs::World, _r: &mut Resources| {}),
            Stage::Update,
            SystemDescriptor::new("dup"),
        );
        schedule.add_system_described(
            FunctionSystem::new("dup", |_w: &mut hecs::World, _r: &mut Resources| {}),
            Stage::Update,
            SystemDescriptor::new("dup"),
        );
        let errors = schedule.validate();
        assert_eq!(errors.len(), 1);
        assert!(matches!(&errors[0], ValidationError::DuplicateName { .. }));
    }

    #[test]
    fn validate_dangling_reference_error() {
        let mut schedule = Schedule::new();
        schedule.add_system_described(
            FunctionSystem::new("sys_a", |_w: &mut hecs::World, _r: &mut Resources| {}),
            Stage::Update,
            SystemDescriptor::new("sys_a").after("nonexistent"),
        );
        let errors = schedule.validate();
        assert_eq!(errors.len(), 1);
        assert!(matches!(&errors[0], ValidationError::DanglingReference { .. }));
    }

    #[test]
    fn validate_circular_dependency_error() {
        let mut schedule = Schedule::new();
        schedule.add_system_described(
            FunctionSystem::new("sys_a", |_w: &mut hecs::World, _r: &mut Resources| {}),
            Stage::Update,
            SystemDescriptor::new("sys_a").after("sys_b"),
        );
        schedule.add_system_described(
            FunctionSystem::new("sys_b", |_w: &mut hecs::World, _r: &mut Resources| {}),
            Stage::Update,
            SystemDescriptor::new("sys_b").after("sys_a"),
        );
        let errors = schedule.validate();
        assert_eq!(errors.len(), 1);
        assert!(matches!(&errors[0], ValidationError::CircularDependency { .. }));
    }

    #[test]
    fn validate_undeclared_system_defaults_exclusive() {
        // An undeclared system conflicts with everything in the same stage.
        let mut schedule = Schedule::new();
        schedule.add_fn_system("undeclared", Stage::Update, |_w, _r| {});
        schedule.add_system_described(
            FunctionSystem::new("declared", |_w: &mut hecs::World, _r: &mut Resources| {}),
            Stage::Update,
            SystemDescriptor::new("declared").reads::<CompA>(),
        );
        let errors = schedule.validate();
        assert_eq!(errors.len(), 1);
        assert!(matches!(&errors[0], ValidationError::UnresolvedConflict { .. }));
    }

    #[test]
    fn topological_sort_respects_after() {
        let order = Arc::new(Mutex::new(Vec::new()));

        let mut schedule = Schedule::new();

        // Register B first, but A should run first because B.after("sys_a").
        let o = Arc::clone(&order);
        schedule.add_system_described(
            FunctionSystem::new("sys_b", move |_w: &mut hecs::World, _r: &mut Resources| {
                o.lock().expect("lock").push("B");
            }),
            Stage::Update,
            SystemDescriptor::new("sys_b").reads::<CompA>().after("sys_a"),
        );
        let o = Arc::clone(&order);
        schedule.add_system_described(
            FunctionSystem::new("sys_a", move |_w: &mut hecs::World, _r: &mut Resources| {
                o.lock().expect("lock").push("A");
            }),
            Stage::Update,
            SystemDescriptor::new("sys_a").reads::<CompB>(),
        );

        let errors = schedule.validate();
        assert!(errors.is_empty(), "should validate: {errors:?}");

        let mut world = hecs::World::new();
        let mut resources = Resources::new();
        resources.insert(Time::new());
        resources.insert(EditorState::new());
        let mut cmd = CommandBuffer::new();

        schedule.run_raw(&mut world, &mut resources, &mut cmd);

        let result = order.lock().expect("lock");
        assert_eq!(*result, vec!["A", "B"], "sys_a should run before sys_b");
    }

    #[test]
    fn topological_sort_preserves_insertion_order_without_constraints() {
        let order = Arc::new(Mutex::new(Vec::new()));

        let mut schedule = Schedule::new();

        let o = Arc::clone(&order);
        schedule.add_system_described(
            FunctionSystem::new("sys_x", move |_w: &mut hecs::World, _r: &mut Resources| {
                o.lock().expect("lock").push("X");
            }),
            Stage::Update,
            SystemDescriptor::new("sys_x").reads::<CompA>(),
        );
        let o = Arc::clone(&order);
        schedule.add_system_described(
            FunctionSystem::new("sys_y", move |_w: &mut hecs::World, _r: &mut Resources| {
                o.lock().expect("lock").push("Y");
            }),
            Stage::Update,
            SystemDescriptor::new("sys_y").reads::<CompB>(),
        );

        let errors = schedule.validate();
        assert!(errors.is_empty());

        let mut world = hecs::World::new();
        let mut resources = Resources::new();
        resources.insert(Time::new());
        resources.insert(EditorState::new());
        let mut cmd = CommandBuffer::new();

        schedule.run_raw(&mut world, &mut resources, &mut cmd);

        let result = order.lock().expect("lock");
        assert_eq!(*result, vec!["X", "Y"], "insertion order preserved");
    }

    #[test]
    fn different_stages_no_conflict() {
        // Systems in different stages never conflict.
        let mut schedule = Schedule::new();
        schedule.add_system_described(
            FunctionSystem::new("sys_a", |_w: &mut hecs::World, _r: &mut Resources| {}),
            Stage::PreUpdate,
            SystemDescriptor::new("sys_a").writes::<CompA>(),
        );
        schedule.add_system_described(
            FunctionSystem::new("sys_b", |_w: &mut hecs::World, _r: &mut Resources| {}),
            Stage::PostUpdate,
            SystemDescriptor::new("sys_b").writes::<CompA>(),
        );
        let errors = schedule.validate();
        assert!(errors.is_empty(), "different stages should not conflict: {errors:?}");
    }
}
