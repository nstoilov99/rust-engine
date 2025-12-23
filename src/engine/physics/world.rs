//! Physics world management using Rapier 3D

use super::components::{
    Collider as EcsCollider, ColliderShape, RigidBody as EcsRigidBody,
    RigidBodyType as EcsRigidBodyType, Velocity as EcsVelocity,
};
use crate::engine::ecs::components::Transform;
use hecs::World;
use rapier3d::na::{Isometry3, Point3, Quaternion, UnitQuaternion, Vector3};
use rapier3d::prelude::{
    CCDSolver, ColliderBuilder, ColliderSet, DefaultBroadPhase, ImpulseJointSet,
    IntegrationParameters, IslandManager, MultibodyJointSet, NarrowPhase, PhysicsPipeline,
    QueryFilter, QueryPipeline, Ray, RigidBodyBuilder, RigidBodyHandle, RigidBodySet,
    SharedShape,
};

/// Manages Rapier physics simulation
///
/// # Example
/// ```ignore
/// let mut physics = PhysicsWorld::new();
///
/// // Register entities with physics components
/// for (_, (transform, rb, col)) in world.query::<(&Transform, &mut RigidBody, &mut Collider)>().iter() {
///     physics.register_entity(transform, rb, col);
/// }
///
/// // In game loop
/// physics.step(delta_time, &mut world);
/// ```
pub struct PhysicsWorld {
    // Rapier data structures
    pub rigid_body_set: RigidBodySet,
    pub collider_set: ColliderSet,
    pub integration_parameters: IntegrationParameters,
    pub physics_pipeline: PhysicsPipeline,
    pub island_manager: IslandManager,
    pub broad_phase: DefaultBroadPhase,
    pub narrow_phase: NarrowPhase,
    pub impulse_joint_set: ImpulseJointSet,
    pub multibody_joint_set: MultibodyJointSet,
    pub ccd_solver: CCDSolver,
    pub query_pipeline: QueryPipeline,

    // Configuration
    pub gravity: Vector3<f32>,

    // Fixed timestep accumulator
    accumulator: f32,
    fixed_dt: f32,
}

impl PhysicsWorld {
    /// Create a new physics world with default settings
    pub fn new() -> Self {
        Self {
            rigid_body_set: RigidBodySet::new(),
            collider_set: ColliderSet::new(),
            integration_parameters: IntegrationParameters::default(),
            physics_pipeline: PhysicsPipeline::new(),
            island_manager: IslandManager::new(),
            broad_phase: DefaultBroadPhase::new(),
            narrow_phase: NarrowPhase::new(),
            impulse_joint_set: ImpulseJointSet::new(),
            multibody_joint_set: MultibodyJointSet::new(),
            ccd_solver: CCDSolver::new(),
            query_pipeline: QueryPipeline::new(),
            gravity: Vector3::new(0.0, -9.81, 0.0),
            accumulator: 0.0,
            fixed_dt: 1.0 / 60.0,
        }
    }

    /// Set gravity vector
    pub fn set_gravity(&mut self, gravity: Vector3<f32>) {
        self.gravity = gravity;
    }

    /// Set fixed timestep for physics simulation (default: 1/60)
    pub fn set_timestep(&mut self, dt: f32) {
        self.fixed_dt = dt;
    }

    /// Step physics with fixed timestep accumulator
    ///
    /// This accumulates frame time and runs physics at a fixed rate
    /// to ensure deterministic simulation.
    pub fn step(&mut self, delta_time: f32, ecs_world: &mut World) {
        self.accumulator += delta_time;

        while self.accumulator >= self.fixed_dt {
            // Sync ECS -> Physics (kinematic bodies)
            self.sync_ecs_to_physics(ecs_world);

            // Run physics step
            self.physics_pipeline.step(
                &self.gravity,
                &self.integration_parameters,
                &mut self.island_manager,
                &mut self.broad_phase,
                &mut self.narrow_phase,
                &mut self.rigid_body_set,
                &mut self.collider_set,
                &mut self.impulse_joint_set,
                &mut self.multibody_joint_set,
                &mut self.ccd_solver,
                Some(&mut self.query_pipeline),
                &(),
                &(),
            );

            // Sync Physics -> ECS (dynamic bodies)
            self.sync_physics_to_ecs(ecs_world);

            self.accumulator -= self.fixed_dt;
        }
    }

    /// Register an entity with the physics world
    ///
    /// Creates Rapier rigidbody and collider from ECS components.
    /// Does nothing if already registered.
    pub fn register_entity(
        &mut self,
        transform: &Transform,
        rigidbody: &mut EcsRigidBody,
        collider: &mut EcsCollider,
    ) {
        // Skip if already registered
        if rigidbody.handle.is_some() {
            return;
        }

        // Build Rapier rigidbody
        let rb_builder = match rigidbody.body_type {
            EcsRigidBodyType::Dynamic => RigidBodyBuilder::dynamic(),
            EcsRigidBodyType::Kinematic => RigidBodyBuilder::kinematic_position_based(),
            EcsRigidBodyType::Static => RigidBodyBuilder::fixed(),
        };

        // Convert transform position to nalgebra Vector3
        let translation = Vector3::new(
            transform.position.x,
            transform.position.y,
            transform.position.z,
        );

        // Convert glm::Quat to UnitQuaternion
        // Note: nalgebra_glm::Quat stores as [x, y, z, w] in coords
        let rotation = UnitQuaternion::from_quaternion(Quaternion::new(
            transform.rotation.coords.w,
            transform.rotation.coords.x,
            transform.rotation.coords.y,
            transform.rotation.coords.z,
        ));

        let rb = rb_builder
            .translation(translation)
            .rotation(rotation.scaled_axis())
            .linear_damping(rigidbody.linear_damping)
            .angular_damping(rigidbody.angular_damping)
            .can_sleep(rigidbody.can_sleep)
            .build();

        let rb_handle = self.rigid_body_set.insert(rb);
        rigidbody.handle = Some(rb_handle);

        // Build collider shape
        let shape = match &collider.shape {
            ColliderShape::Cuboid { half_extents } => {
                SharedShape::cuboid(half_extents.x, half_extents.y, half_extents.z)
            }
            ColliderShape::Ball { radius } => SharedShape::ball(*radius),
            ColliderShape::Capsule {
                half_height,
                radius,
            } => SharedShape::capsule_y(*half_height, *radius),
        };

        let col = ColliderBuilder::new(shape)
            .friction(collider.friction)
            .restitution(collider.restitution)
            .sensor(collider.is_sensor)
            .build();

        let col_handle =
            self.collider_set
                .insert_with_parent(col, rb_handle, &mut self.rigid_body_set);
        collider.handle = Some(col_handle);
    }

    /// Apply an impulse to a rigidbody
    pub fn apply_impulse(&mut self, handle: RigidBodyHandle, impulse: Vector3<f32>) {
        if let Some(rb) = self.rigid_body_set.get_mut(handle) {
            rb.apply_impulse(impulse, true);
        }
    }

    /// Apply a force to a rigidbody (continuous, cleared each step)
    pub fn apply_force(&mut self, handle: RigidBodyHandle, force: Vector3<f32>) {
        if let Some(rb) = self.rigid_body_set.get_mut(handle) {
            rb.add_force(force, true);
        }
    }

    /// Cast a ray and return the first hit
    ///
    /// Returns (rigidbody handle, distance, hit point) if hit
    pub fn raycast(
        &self,
        origin: nalgebra_glm::Vec3,
        direction: nalgebra_glm::Vec3,
        max_distance: f32,
    ) -> Option<(RigidBodyHandle, f32, nalgebra_glm::Vec3)> {
        let ray = Ray::new(
            Point3::new(origin.x, origin.y, origin.z),
            Vector3::new(direction.x, direction.y, direction.z),
        );

        self.query_pipeline
            .cast_ray(
                &self.rigid_body_set,
                &self.collider_set,
                &ray,
                max_distance,
                true,
                QueryFilter::default(),
            )
            .map(|(handle, toi)| {
                let hit_point = ray.point_at(toi);
                let rb_handle = self.collider_set[handle].parent().unwrap();
                (
                    rb_handle,
                    toi,
                    nalgebra_glm::vec3(hit_point.x, hit_point.y, hit_point.z),
                )
            })
    }

    /// Sync ECS transforms to physics world (for kinematic bodies)
    fn sync_ecs_to_physics(&mut self, ecs_world: &World) {
        for (_, (transform, rigidbody)) in
            ecs_world.query::<(&Transform, &EcsRigidBody)>().iter()
        {
            // Only update kinematic bodies
            if rigidbody.body_type != EcsRigidBodyType::Kinematic {
                continue;
            }

            if let Some(handle) = rigidbody.handle {
                if let Some(rb) = self.rigid_body_set.get_mut(handle) {
                    let translation = Vector3::new(
                        transform.position.x,
                        transform.position.y,
                        transform.position.z,
                    );
                    let rotation = UnitQuaternion::from_quaternion(Quaternion::new(
                        transform.rotation.coords.w,
                        transform.rotation.coords.x,
                        transform.rotation.coords.y,
                        transform.rotation.coords.z,
                    ));
                    rb.set_next_kinematic_position(Isometry3::from_parts(
                        translation.into(),
                        rotation,
                    ));
                }
            }
        }
    }

    /// Sync physics world to ECS transforms (for dynamic bodies)
    fn sync_physics_to_ecs(&self, ecs_world: &mut World) {
        for (_, (transform, rigidbody, velocity)) in ecs_world
            .query::<(&mut Transform, &EcsRigidBody, Option<&mut EcsVelocity>)>()
            .iter()
        {
            // Static bodies don't need sync
            if rigidbody.body_type == EcsRigidBodyType::Static {
                continue;
            }

            if let Some(handle) = rigidbody.handle {
                if let Some(rb) = self.rigid_body_set.get(handle) {
                    // Update position
                    let pos = rb.translation();
                    transform.position.x = pos.x;
                    transform.position.y = pos.y;
                    transform.position.z = pos.z;

                    // Update rotation
                    let rot = rb.rotation();
                    transform.rotation =
                        nalgebra_glm::quat(rot.w, rot.coords.x, rot.coords.y, rot.coords.z);

                    // Update velocity if component exists
                    if let Some(vel) = velocity {
                        let linvel = rb.linvel();
                        let angvel = rb.angvel();
                        vel.linear = nalgebra_glm::vec3(linvel.x, linvel.y, linvel.z);
                        vel.angular = nalgebra_glm::vec3(angvel.x, angvel.y, angvel.z);
                    }
                }
            }
        }
    }
}

impl Default for PhysicsWorld {
    fn default() -> Self {
        Self::new()
    }
}
