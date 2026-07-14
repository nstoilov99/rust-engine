//! Inspector Panel state.
//!
//! The old rendering fns (`show`, `show_contents`, `render_header`,
//! `render_empty_state`, `render_entity_info`, `component_section`,
//! `render_components`, `edit_*`, `render_add_component`, plus the local
//! `ComponentCategory` / `ComponentAction` enums) were removed as part of the
//! UI teardown; the crusty analog lives in `inspector_crusty`.

use crate::engine::animation::{AnimationPlayer, SkeletonInstance};
use crate::engine::audio::{AudioEmitter, AudioListener};
use crate::engine::ecs::{
    Camera, DirectionalLight, MeshRenderer, Name, ParticleEffect, PointLight, Transform,
};
use crate::engine::physics::{Collider, RigidBody};
use hecs::{Entity, World};
use nalgebra_glm as glm;
use std::collections::{HashMap, HashSet};

/// Bitflags for which inspectable components an entity has.
/// Computed once per selection change / component mutation, not per frame.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ComponentPresence {
    bits: u16,
}

impl ComponentPresence {
    pub(crate) const NAME: u16 = 1 << 0;
    pub(crate) const TRANSFORM: u16 = 1 << 1;
    pub(crate) const CAMERA: u16 = 1 << 2;
    pub(crate) const MESH_RENDERER: u16 = 1 << 3;
    pub(crate) const DIR_LIGHT: u16 = 1 << 4;
    pub(crate) const POINT_LIGHT: u16 = 1 << 5;
    pub(crate) const RIGID_BODY: u16 = 1 << 6;
    pub(crate) const COLLIDER: u16 = 1 << 7;
    pub(crate) const SKELETON: u16 = 1 << 8;
    pub(crate) const ANIM_PLAYER: u16 = 1 << 9;
    pub(crate) const AUDIO_EMITTER: u16 = 1 << 10;
    pub(crate) const AUDIO_LISTENER: u16 = 1 << 11;
    pub(crate) const PARTICLE_EFFECT: u16 = 1 << 12;

    pub(crate) fn probe(world: &World, entity: Entity) -> Self {
        let mut bits = 0u16;
        if world.get::<&Name>(entity).is_ok() {
            bits |= Self::NAME;
        }
        if world.get::<&Transform>(entity).is_ok() {
            bits |= Self::TRANSFORM;
        }
        if world.get::<&Camera>(entity).is_ok() {
            bits |= Self::CAMERA;
        }
        if world.get::<&MeshRenderer>(entity).is_ok() {
            bits |= Self::MESH_RENDERER;
        }
        if world.get::<&DirectionalLight>(entity).is_ok() {
            bits |= Self::DIR_LIGHT;
        }
        if world.get::<&PointLight>(entity).is_ok() {
            bits |= Self::POINT_LIGHT;
        }
        if world.get::<&RigidBody>(entity).is_ok() {
            bits |= Self::RIGID_BODY;
        }
        if world.get::<&Collider>(entity).is_ok() {
            bits |= Self::COLLIDER;
        }
        if world.get::<&SkeletonInstance>(entity).is_ok() {
            bits |= Self::SKELETON;
        }
        if world.get::<&AnimationPlayer>(entity).is_ok() {
            bits |= Self::ANIM_PLAYER;
        }
        if world.get::<&AudioEmitter>(entity).is_ok() {
            bits |= Self::AUDIO_EMITTER;
        }
        if world.get::<&AudioListener>(entity).is_ok() {
            bits |= Self::AUDIO_LISTENER;
        }
        if world.get::<&ParticleEffect>(entity).is_ok() {
            bits |= Self::PARTICLE_EFFECT;
        }
        Self { bits }
    }

    pub(crate) fn has(self, flag: u16) -> bool {
        self.bits & flag != 0
    }
}

/// Inspector Panel state
pub struct InspectorPanel {
    /// Cache for euler angles (quaternion -> euler conversion).
    pub(crate) euler_cache: HashMap<u64, (glm::Quat, [f32; 3])>,
    /// Last known entity for euler cache invalidation
    pub(crate) last_entity: Option<Entity>,
    _collapsed_components: HashSet<String>,
    /// Search/filter text for components
    pub(crate) search_filter: String,
    /// Cached component presence mask (recomputed on selection change or mutation).
    pub(crate) cached_presence: ComponentPresence,
    /// The entity for which `cached_presence` was computed.
    pub(crate) presence_entity: Option<Entity>,
    /// Unreal-style saved color swatches shared by all inspector color pickers.
    #[cfg(feature = "editor")]
    pub(crate) crusty_swatches: Vec<crusty_gui::math::Color>,
    /// Desktop eyedropper bridge shared by all inspector color pickers;
    /// the app loop fills its pixel patch while armed.
    #[cfg(feature = "editor")]
    pub crusty_eyedropper: crusty_gui::widgets::EyedropperState,
    #[cfg(all(feature = "editor", windows))]
    desktop_sampler: super::desktop_sampler::DesktopSampler,
}

impl Default for InspectorPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl InspectorPanel {
    pub fn new() -> Self {
        Self {
            euler_cache: HashMap::new(),
            last_entity: None,
            _collapsed_components: HashSet::new(),
            search_filter: String::new(),
            cached_presence: ComponentPresence::default(),
            presence_entity: None,
            #[cfg(feature = "editor")]
            crusty_swatches: Vec::new(),
            #[cfg(feature = "editor")]
            crusty_eyedropper: crusty_gui::widgets::EyedropperState::default(),
            #[cfg(all(feature = "editor", windows))]
            desktop_sampler: Default::default(),
        }
    }

    /// Drive the desktop eyedropper while armed: sample the pixels around
    /// the global cursor and detect pick/cancel clicks (which winit can't
    /// see outside the window). Call once per frame before the crusty
    /// layout pass.
    #[cfg(feature = "editor")]
    pub fn drive_eyedropper(&mut self) {
        #[cfg(windows)]
        self.desktop_sampler.update(&mut self.crusty_eyedropper);
    }

    /// Check if a component matches the search filter
    pub(crate) fn matches_filter(&self, component_name: &str) -> bool {
        self.search_filter.is_empty()
            || component_name
                .to_lowercase()
                .contains(&self.search_filter.to_lowercase())
    }
}

/// Convert quaternion to Euler angles (degrees, XYZ order).
///
/// Normalizes the quaternion first to prevent denormalization drift.
pub(crate) fn quaternion_to_euler_degrees(q: &glm::Quat) -> [f32; 3] {
    // Normalize quaternion to prevent NaN from denormalized quats
    let q_norm = glm::quat_normalize(q);
    let euler = glm::quat_euler_angles(&q_norm);
    [
        if euler.x.is_finite() {
            clean_display_degrees(euler.x.to_degrees())
        } else {
            0.0
        },
        if euler.y.is_finite() {
            clean_display_degrees(euler.y.to_degrees())
        } else {
            0.0
        },
        if euler.z.is_finite() {
            clean_display_degrees(euler.z.to_degrees())
        } else {
            0.0
        },
    ]
}

pub(crate) fn clean_display_degrees(value: f32) -> f32 {
    if value.abs() < 0.000_1 {
        0.0
    } else {
        value
    }
}

/// Convert Euler angles (degrees) to quaternion
pub(crate) fn euler_degrees_to_quaternion(euler: &[f32; 3]) -> glm::Quat {
    let rad_x = euler[0].to_radians();
    let rad_y = euler[1].to_radians();
    let rad_z = euler[2].to_radians();

    // Build quaternion from euler angles (XYZ order)
    let qx = glm::quat_angle_axis(rad_x, &glm::vec3(1.0, 0.0, 0.0));
    let qy = glm::quat_angle_axis(rad_y, &glm::vec3(0.0, 1.0, 0.0));
    let qz = glm::quat_angle_axis(rad_z, &glm::vec3(0.0, 0.0, 1.0));

    // Normalize to prevent drift accumulation
    glm::quat_normalize(&(qz * qy * qx))
}

/// Check if two quaternions represent approximately the same rotation.
/// Accounts for q and -q representing the same rotation.
pub(crate) fn quaternions_approximately_equal(a: &glm::Quat, b: &glm::Quat) -> bool {
    let dot = (a.coords.x * b.coords.x
        + a.coords.y * b.coords.y
        + a.coords.z * b.coords.z
        + a.coords.w * b.coords.w)
        .abs();
    dot > 0.9999
}
