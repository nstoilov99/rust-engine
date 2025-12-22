//! ECS components for physics simulation

use glam::Vec3;
use nalgebra::Point3;
use rapier3d::prelude::*;
use serde::{Deserialize, Serialize};

/// Collision groups for filtering (which objects collide with which)
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct CollisionGroups {
    /// Collision groups this collider belongs to
    pub memberships: u32,
    /// Collision groups this collider can interact with
    pub filter: u32,
}

impl CollisionGroups {
    pub fn new(memberships: u32, filter: u32) -> Self {
        Self {
            memberships,
            filter,
        }
    }

    /// Convert to Rapier InteractionGroups
    pub fn to_rapier(&self) -> InteractionGroups {
        InteractionGroups::new(self.memberships.into(), self.filter.into())
    }
}

// Define common collision group constants
pub mod collision_groups {
    pub const PLAYER: u32 = 0b0001;
    pub const ENEMY: u32 = 0b0010;
    pub const PROJECTILE: u32 = 0b0100;
    pub const ENVIRONMENT: u32 = 0b1000;
    pub const SENSOR: u32 = 0b10000;
    pub const ALL: u32 = 0xFFFFFFFF;
    pub const NONE: u32 = 0b0;
}

/// Rigidbody component - makes entity dynamic/kinematic/static
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RigidBody {
    pub body_type: RigidBodyType,
    pub mass: f32,
    pub linear_damping: f32,
    pub angular_damping: f32,
    pub can_sleep: bool,

    // Constraints
    pub lock_translation_x: bool,
    pub lock_translation_y: bool,
    pub lock_translation_z: bool,
    pub lock_rotation_x: bool,
    pub lock_rotation_y: bool,
    pub lock_rotation_z: bool,

    // Runtime state (not serialized)
    #[serde(skip)]
    pub handle: Option<RigidBodyHandle>,
}

impl Default for RigidBody {
    fn default() -> Self {
        Self {
            body_type: RigidBodyType::Dynamic,
            mass: 1.0,
            linear_damping: 0.05, // Slight air resistance
            angular_damping: 0.05,
            can_sleep: true,
            lock_translation_x: false,
            lock_translation_y: false,
            lock_translation_z: false,
            lock_rotation_x: false,
            lock_rotation_y: false,
            lock_rotation_z: false,
            handle: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum RigidBodyType {
    /// Moved by forces (gravity, impulses) - most game objects
    Dynamic,
    /// Moved by code (kinematic characters, platforms)
    Kinematic,
    /// Never moves (walls, floors, static geometry)
    Static,
}

impl From<RigidBodyType> for rapier3d::dynamics::RigidBodyType {
    fn from(t: RigidBodyType) -> Self {
        match t {
            RigidBodyType::Dynamic => rapier3d::dynamics::RigidBodyType::Dynamic,
            RigidBodyType::Kinematic => rapier3d::dynamics::RigidBodyType::KinematicPositionBased,
            RigidBodyType::Static => rapier3d::dynamics::RigidBodyType::Fixed,
        }
    }
}

/// Collider component - defines collision shape
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Collider {
    pub shape: ColliderShape,
    pub friction: f32,
    pub restitution: f32, // Bounciness (0 = no bounce, 1 = perfect bounce)
    pub is_sensor: bool,  // Trigger volume (no collision response)

    // Collision filtering
    pub collision_groups: Option<CollisionGroups>,

    // Runtime state (not serialized)
    #[serde(skip)]
    pub handle: Option<ColliderHandle>,
}

impl Default for Collider {
    fn default() -> Self {
        Self {
            shape: ColliderShape::Cuboid {
                half_extents: Vec3::new(0.5, 0.5, 0.5),
            },
            friction: 0.5,
            restitution: 0.3,
            is_sensor: false,
            collision_groups: None, // Default: collides with everything
            handle: None,
        }
    }
}

/// Collision shapes supported
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ColliderShape {
    /// Box (half-extents from center)
    Cuboid { half_extents: Vec3 },
    /// Sphere (radius)
    Ball { radius: f32 },
    /// Capsule (height, radius)
    Capsule { half_height: f32, radius: f32 },
    /// Cylinder (height, radius)
    Cylinder { half_height: f32, radius: f32 },
    /// Triangle mesh (for complex static geometry)
    TriMesh {
        vertices: Vec<Vec3>,
        indices: Vec<[u32; 3]>,
    },
}

impl ColliderShape {
    /// Convert to Rapier SharedShape
    pub fn to_rapier_shape(&self) -> SharedShape {
        match self {
            ColliderShape::Cuboid { half_extents } => {
                SharedShape::cuboid(half_extents.x, half_extents.y, half_extents.z)
            }
            ColliderShape::Ball { radius } => SharedShape::ball(*radius),
            ColliderShape::Capsule {
                half_height,
                radius,
            } => SharedShape::capsule_y(*half_height, *radius),
            ColliderShape::Cylinder {
                half_height,
                radius,
            } => SharedShape::cylinder(*half_height, *radius),
            ColliderShape::TriMesh { vertices, indices } => {
                // Convert to Rapier format
                let rapier_vertices: Vec<_> = vertices
                    .iter()
                    .map(|v| Point3::new(v.x, v.y, v.z))
                    .collect();
                SharedShape::trimesh(rapier_vertices, indices.clone())
            }
        }
    }
}

/// Velocity component (for dynamic rigidbodies)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Velocity {
    pub linear: Vec3,
    pub angular: Vec3,
}
