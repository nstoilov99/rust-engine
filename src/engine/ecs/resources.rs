//! Typed resource storage for global engine state.
//!
//! Resources are singleton values stored by type. Each type can have at most
//! one instance. Resources must be `Send + Sync + 'static`.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::ops::{Deref, DerefMut};

/// Type-erased storage for global resources.
///
/// Each resource type can have at most one instance stored.
/// Resources must be Send + Sync + 'static.
pub struct Resources {
    map: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

impl Resources {
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
        }
    }

    /// Insert a resource. Replaces any existing resource of the same type.
    pub fn insert<T: Send + Sync + 'static>(&mut self, resource: T) {
        self.map.insert(TypeId::of::<T>(), Box::new(resource));
    }

    /// Get an immutable reference to a resource.
    pub fn get<T: 'static>(&self) -> Option<&T> {
        self.map
            .get(&TypeId::of::<T>())
            .and_then(|boxed| boxed.downcast_ref::<T>())
    }

    /// Get a mutable reference to a resource.
    pub fn get_mut<T: 'static>(&mut self) -> Option<&mut T> {
        self.map
            .get_mut(&TypeId::of::<T>())
            .and_then(|boxed| boxed.downcast_mut::<T>())
    }

    /// Remove a resource, returning it if it existed.
    pub fn remove<T: 'static>(&mut self) -> Option<T> {
        self.map
            .remove(&TypeId::of::<T>())
            .and_then(|boxed| boxed.downcast::<T>().ok())
            .map(|boxed| *boxed)
    }

    /// Check if a resource of this type exists.
    pub fn contains<T: 'static>(&self) -> bool {
        self.map.contains_key(&TypeId::of::<T>())
    }
}

impl Default for Resources {
    fn default() -> Self {
        Self::new()
    }
}

/// Immutable resource wrapper (implements Deref to &T).
pub struct Res<'a, T: 'static> {
    value: &'a T,
}

impl<'a, T: 'static> Res<'a, T> {
    pub fn new(value: &'a T) -> Self {
        Self { value }
    }
}

impl<T: 'static> Deref for Res<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.value
    }
}

/// Mutable resource wrapper (implements Deref + DerefMut).
pub struct ResMut<'a, T: 'static> {
    value: &'a mut T,
}

impl<'a, T: 'static> ResMut<'a, T> {
    pub fn new(value: &'a mut T) -> Self {
        Self { value }
    }
}

impl<T: 'static> Deref for ResMut<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.value
    }
}

impl<T: 'static> DerefMut for ResMut<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        self.value
    }
}

// === Core Resources ===

/// Time resource providing frame timing information.
#[derive(Debug, Clone)]
pub struct Time {
    /// Time since last frame (seconds)
    pub delta: f32,
    /// Fixed timestep for physics (seconds, default 1/60)
    pub fixed_delta: f32,
    /// Total elapsed time since start (seconds)
    pub total: f64,
    /// Frame count since start
    pub frame: u64,
    /// Whether simulation is paused
    pub paused: bool,
    /// Time scale multiplier (1.0 = normal speed)
    pub scale: f32,
}

impl Time {
    pub fn new() -> Self {
        Self {
            delta: 0.0,
            fixed_delta: 1.0 / 60.0,
            total: 0.0,
            frame: 0,
            paused: false,
            scale: 1.0,
        }
    }

    /// Get scaled delta time (returns 0 when paused).
    pub fn scaled_delta(&self) -> f32 {
        if self.paused {
            0.0
        } else {
            self.delta * self.scale
        }
    }

    /// Advance time by one frame with the given raw delta.
    pub fn advance(&mut self, dt: f32) {
        self.delta = dt;
        self.total += dt as f64;
        self.frame += 1;
    }
}

impl Default for Time {
    fn default() -> Self {
        Self::new()
    }
}

/// Editor play/pause mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayMode {
    Edit,
    Playing,
    Paused,
}

impl Default for PlayMode {
    fn default() -> Self {
        Self::Edit
    }
}

/// Editor tool mode for viewport interaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolMode {
    Select,
    Translate,
    Rotate,
    Scale,
}

impl Default for ToolMode {
    fn default() -> Self {
        Self::Select
    }
}

/// Editor state resource tracking play mode and tool settings.
/// Selection is managed by the `Selection` struct (not stored here).
#[derive(Debug, Clone)]
pub struct EditorState {
    pub tool_mode: ToolMode,
    pub play_mode: PlayMode,
    pub grid_visible: bool,
    pub snap_enabled: bool,
    pub snap_increment: f32,
}

impl EditorState {
    pub fn new() -> Self {
        Self {
            tool_mode: ToolMode::default(),
            play_mode: PlayMode::default(),
            grid_visible: true,
            snap_enabled: false,
            snap_increment: 1.0,
        }
    }
}

impl Default for EditorState {
    fn default() -> Self {
        Self::new()
    }
}
