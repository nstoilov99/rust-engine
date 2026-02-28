//! Staged system scheduling with run criteria.
//!
//! Systems are registered into stages and executed in stage order
//! (First → PreUpdate → Update → PostUpdate → Last), with stable
//! insertion ordering within each stage.

use super::resources::{EditorState, PlayMode, Resources, Time};

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
            .map_or(true, |state| state.play_mode == PlayMode::Playing)
    }
}

/// Run only when in edit mode.
pub struct RunIfEditing;
impl RunCriteria for RunIfEditing {
    fn should_run(&self, resources: &Resources) -> bool {
        resources
            .get::<EditorState>()
            .map_or(false, |state| state.play_mode == PlayMode::Edit)
    }
}

/// Run only when not paused.
pub struct RunIfNotPaused;
impl RunCriteria for RunIfNotPaused {
    fn should_run(&self, resources: &Resources) -> bool {
        resources
            .get::<Time>()
            .map_or(true, |time| !time.paused)
    }
}

/// Placeholder: selection is managed by the external `Selection` struct,
/// not by `EditorState`. This always returns false; use `Selection` directly
/// for selection-dependent logic.
#[deprecated(note = "Selection is tracked externally via `Selection`, not `EditorState`")]
pub struct RunIfSelected;
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
    pub fn add_system<S: System + 'static>(
        &mut self,
        system: S,
        stage: Stage,
    ) -> &mut Self {
        self.systems.push(RegisteredSystem {
            system: Box::new(system),
            stage,
            run_criteria: Box::new(Always),
            enabled: true,
            insertion_order: self.next_order,
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
        });
        self.next_order += 1;
        self.needs_sort = true;
        self
    }

    /// Add a function system (convenience).
    pub fn add_fn_system<F>(
        &mut self,
        name: &'static str,
        stage: Stage,
        func: F,
    ) -> &mut Self
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
}

impl Default for Schedule {
    fn default() -> Self {
        Self::new()
    }
}
