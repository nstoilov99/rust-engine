//! Physics ECS components
//!
//! These components mark entities for physics simulation.
//! The PhysicsWorld handles synchronization with Rapier.

use nalgebra_glm as glm;
use rapier3d::prelude::{ColliderHandle, RigidBodyHandle};
use serde::{Deserialize, Serialize};

/// Rigidbody component - marks entity for physics simulation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RigidBody {
    pub body_type: RigidBodyType,
    pub mass: f32,
    pub linear_damping: f32,
    pub angular_damping: f32,
    pub can_sleep: bool,
    /// Gravity multiplier (1.0 = normal, 0 = no gravity, -1 = anti-gravity)
    #[serde(default = "default_gravity_scale")]
    pub gravity_scale: f32,
    /// Lock rotation per axis [X, Y, Z]
    #[serde(default)]
    pub lock_rotation: [bool; 3],
    /// Continuous collision detection (prevents tunneling for fast objects)
    #[serde(default)]
    pub continuous_collision: bool,

    /// Rapier handle (runtime only, not serialized)
    #[serde(skip)]
    pub(crate) handle: Option<RigidBodyHandle>,
}

fn default_gravity_scale() -> f32 {
    1.0
}

impl Default for RigidBody {
    fn default() -> Self {
        Self {
            body_type: RigidBodyType::Dynamic,
            mass: 1.0,
            linear_damping: 0.05,
            angular_damping: 0.05,
            can_sleep: true,
            gravity_scale: 1.0,
            lock_rotation: [false; 3],
            continuous_collision: false,
            handle: None,
        }
    }
}

impl RigidBody {
    /// Create a dynamic rigidbody (moved by forces)
    pub fn dynamic() -> Self {
        Self::default()
    }

    /// Create a kinematic rigidbody (moved by code)
    pub fn kinematic() -> Self {
        Self {
            body_type: RigidBodyType::Kinematic,
            ..Default::default()
        }
    }

    /// Create a static rigidbody (never moves)
    pub fn fixed() -> Self {
        Self {
            body_type: RigidBodyType::Static,
            ..Default::default()
        }
    }

    /// Set mass (builder pattern)
    pub fn with_mass(mut self, mass: f32) -> Self {
        self.mass = mass;
        self
    }

    /// Set linear damping (builder pattern)
    pub fn with_linear_damping(mut self, damping: f32) -> Self {
        self.linear_damping = damping;
        self
    }

    /// Set angular damping (builder pattern)
    pub fn with_angular_damping(mut self, damping: f32) -> Self {
        self.angular_damping = damping;
        self
    }

    /// Set gravity scale (builder pattern)
    pub fn with_gravity_scale(mut self, scale: f32) -> Self {
        self.gravity_scale = scale;
        self
    }

    /// Lock rotation axes (builder pattern)
    pub fn with_locked_rotation(mut self, x: bool, y: bool, z: bool) -> Self {
        self.lock_rotation = [x, y, z];
        self
    }

    /// Enable continuous collision detection (builder pattern)
    pub fn with_ccd(mut self) -> Self {
        self.continuous_collision = true;
        self
    }

    /// Returns the Rapier rigid-body handle, if the body has been registered with the physics world.
    pub fn physics_handle(&self) -> Option<RigidBodyHandle> {
        self.handle
    }
}

/// Rigidbody types matching Rapier's types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum RigidBodyType {
    /// Moved by forces (gravity, impulses) - most game objects
    #[default]
    Dynamic,
    /// Moved by code (kinematic characters, platforms)
    Kinematic,
    /// Never moves (walls, floors, static geometry)
    Static,
}

/// Collider component - defines collision shape
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Collider {
    pub shape: ColliderShape,
    pub friction: f32,
    pub restitution: f32,
    pub is_sensor: bool,
    /// Show debug wireframe for this collider
    #[serde(default)]
    pub debug_draw_visible: bool,

    #[serde(skip)]
    pub(crate) handle: Option<ColliderHandle>,
}

impl Default for Collider {
    fn default() -> Self {
        Self {
            shape: ColliderShape::Cuboid {
                half_extents: glm::vec3(0.5, 0.5, 0.5),
            },
            friction: 0.5,
            restitution: 0.3,
            is_sensor: false,
            debug_draw_visible: false,
            handle: None,
        }
    }
}

impl Collider {
    /// Create a box collider
    pub fn cuboid(half_x: f32, half_y: f32, half_z: f32) -> Self {
        Self {
            shape: ColliderShape::Cuboid {
                half_extents: glm::vec3(half_x, half_y, half_z),
            },
            ..Default::default()
        }
    }

    /// Create a sphere collider
    pub fn ball(radius: f32) -> Self {
        Self {
            shape: ColliderShape::Ball { radius },
            ..Default::default()
        }
    }

    /// Create a capsule collider (Y-axis aligned)
    pub fn capsule(half_height: f32, radius: f32) -> Self {
        Self {
            shape: ColliderShape::Capsule {
                half_height,
                radius,
            },
            ..Default::default()
        }
    }

    /// Set friction (builder pattern)
    pub fn with_friction(mut self, friction: f32) -> Self {
        self.friction = friction;
        self
    }

    /// Set restitution/bounciness (builder pattern)
    pub fn with_restitution(mut self, restitution: f32) -> Self {
        self.restitution = restitution;
        self
    }

    /// Make this a sensor/trigger (no collision response)
    pub fn as_sensor(mut self) -> Self {
        self.is_sensor = true;
        self
    }
}

/// Collision shapes (KISS: only common shapes)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ColliderShape {
    /// Box (half-extents from center)
    Cuboid {
        #[serde(with = "super::vec3_serde")]
        half_extents: glm::Vec3,
    },
    /// Sphere
    Ball { radius: f32 },
    /// Capsule (Y-axis aligned)
    Capsule { half_height: f32, radius: f32 },
}

/// Velocity component (optional, for reading physics velocity)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Velocity {
    #[serde(with = "super::vec3_serde")]
    pub linear: glm::Vec3,
    #[serde(with = "super::vec3_serde")]
    pub angular: glm::Vec3,
}

impl Velocity {
    pub fn new(linear: glm::Vec3, angular: glm::Vec3) -> Self {
        Self { linear, angular }
    }

    pub fn linear(linear: glm::Vec3) -> Self {
        Self {
            linear,
            angular: glm::vec3(0.0, 0.0, 0.0),
        }
    }
}
