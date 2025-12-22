//! Physics system using Rapier 3D

pub mod debug_render;
pub mod events;

use crate::engine::ecs::components::{
    Collider as EcsCollider, RigidBody as EcsRigidBody, Transform, Velocity as EcsVelocity,
};
use hecs::World;
use nalgebra::{Isometry3, Translation3, UnitQuaternion, Vector3};
use nalgebra_glm as glm;
use rapier3d::prelude::*;

pub use debug_render::{PhysicsDebugRenderer, DebugLine};
pub use events::{CollisionEvent, CollisionEventHandler};

pub struct PhysicsSystem {
    // Rapier physics world
    pub rigid_body_set: RigidBodySet,
    pub collider_set: ColliderSet,
    pub integration_parameters: IntegrationParameters,
    pub physics_pipeline: PhysicsPipeline,
    pub island_manager: IslandManager,
    pub broad_phase: BroadPhase,
    pub narrow_phase: NarrowPhase,
    pub impulse_joint_set: ImpulseJointSet,
    pub multibody_joint_set: MultibodyJointSet,
    pub ccd_solver: CCDSolver,

    // Gravity
    pub gravity: Vector3<f32>,

    // Event handling
    pub collision_events: CollisionEventHandler,

    // Fixed timestep accumulator
    accumulator: f32,
    fixed_dt: f32,
}

impl PhysicsSystem {
    /// Create new physics system
    pub fn new() -> Self {
        let mut integration_parameters = IntegrationParameters::default();
        integration_parameters.dt = 1.0 / 60.0; // 60 Hz physics

        Self {
            rigid_body_set: RigidBodySet::new(),
            collider_set: ColliderSet::new(),
            integration_parameters,
            physics_pipeline: PhysicsPipeline::new(),
            island_manager: IslandManager::new(),
            broad_phase: BroadPhase::new(),
            narrow_phase: NarrowPhase::new(),
            impulse_joint_set: ImpulseJointSet::new(),
            multibody_joint_set: MultibodyJointSet::new(),
            ccd_solver: CCDSolver::new(),
            gravity: Vector3::new(0.0, -9.81, 0.0), // Earth gravity
            collision_events: CollisionEventHandler::new(),
            accumulator: 0.0,
            fixed_dt: 1.0 / 60.0,
        }
    }

    /// Step physics simulation with fixed timestep
    pub fn step(&mut self, delta_time: f32, ecs_world: &mut World) {
        // Accumulator pattern for fixed timestep
        self.accumulator += delta_time;

        while self.accumulator >= self.fixed_dt {
            // Sync ECS → Physics (read velocities, apply forces, etc.)
            self.sync_ecs_to_physics(ecs_world);

            // Run physics step
            self.physics_pipeline.step(
                &self.gravity.into(),
                &self.integration_parameters,
                &mut self.island_manager,
                &mut self.broad_phase,
                &mut self.narrow_phase,
                &mut self.rigid_body_set,
                &mut self.collider_set,
                &mut self.impulse_joint_set,
                &mut self.multibody_joint_set,
                &mut self.ccd_solver,
                None, // Query pipeline (for raycasts)
                &(),  // Hooks
                &mut self.collision_events,  // Events
            );

            // Sync Physics → ECS (update transforms)
            self.sync_physics_to_ecs(ecs_world);

            self.accumulator -= self.fixed_dt;
        }
    }

    /// Create rigidbody + collider from ECS components
    pub fn create_rigidbody(
        &mut self,
        transform: &Transform,
        rigidbody: &mut EcsRigidBody,
        collider: &EcsCollider,
    ) {
        // Build Rapier rigidbody
        // Convert from nalgebra_glm types to nalgebra types
        let translation = Vector3::new(
            transform.position.x,
            transform.position.y,
            transform.position.z,
        );
        let rotation = UnitQuaternion::from_quaternion(nalgebra::Quaternion::new(
            transform.rotation[3], // w
            transform.rotation[0], // i (x)
            transform.rotation[1], // j (y)
            transform.rotation[2], // k (z)
        ));

        let mut rb = RigidBodyBuilder::new(rigidbody.body_type.into())
            .translation(translation)
            .rotation(rotation.scaled_axis())
            .linear_damping(rigidbody.linear_damping)
            .angular_damping(rigidbody.angular_damping)
            .can_sleep(rigidbody.can_sleep)
            .build();

        // Apply locks
        rb.set_enabled_translations(
            !rigidbody.lock_translation_x,
            !rigidbody.lock_translation_y,
            !rigidbody.lock_translation_z,
            true, // Wake up
        );
        rb.set_enabled_rotations(
            !rigidbody.lock_rotation_x,
            !rigidbody.lock_rotation_y,
            !rigidbody.lock_rotation_z,
            true,
        );

        // Insert into physics world
        let rb_handle = self.rigid_body_set.insert(rb);
        rigidbody.handle = Some(rb_handle);

        // Build collider
        let mut collider_builder = ColliderBuilder::new(collider.shape.to_rapier_shape())
            .friction(collider.friction)
            .restitution(collider.restitution)
            .sensor(collider.is_sensor);

        // Apply collision groups if specified
        if let Some(groups) = collider.collision_groups {
            collider_builder = collider_builder.collision_groups(groups.to_rapier());
        }

        let collider_handle = self.collider_set.insert_with_parent(
            collider_builder.build(),
            rb_handle,
            &mut self.rigid_body_set,
        );

        // Store handle in component (for later lookup)
        // Note: This is a bit tricky - we need to update the component after creation
        // This handle will be stored when we return
    }

    /// Sync ECS transforms to physics world (for kinematic bodies)
    fn sync_ecs_to_physics(&mut self, ecs_world: &mut World) {
        for (_entity, (transform, rigidbody)) in
            ecs_world.query::<(&Transform, &EcsRigidBody)>().iter()
        {
            if let Some(handle) = rigidbody.handle {
                if let Some(rb) = self.rigid_body_set.get_mut(handle) {
                    // Only update kinematic bodies (dynamic bodies controlled by physics)
                    if rb.body_type() == rapier3d::dynamics::RigidBodyType::KinematicPositionBased {
                        let translation = Vector3::new(
                            transform.position.x,
                            transform.position.y,
                            transform.position.z,
                        );
                        let rotation = UnitQuaternion::from_quaternion(nalgebra::Quaternion::new(
                            transform.rotation[3], // w
                            transform.rotation[0], // i (x)
                            transform.rotation[1], // j (y)
                            transform.rotation[2], // k (z)
                        ));
                        let position =
                            Isometry3::from_parts(Translation3::from(translation), rotation);
                        rb.set_next_kinematic_position(position);
                    }
                }
            }
        }
    }

    /// Sync physics world to ECS transforms (for dynamic bodies)
    fn sync_physics_to_ecs(&mut self, ecs_world: &mut World) {
        for (_entity, (transform, rigidbody, velocity)) in ecs_world
            .query::<(&mut Transform, &EcsRigidBody, Option<&mut EcsVelocity>)>()
            .iter()
        {
            if let Some(handle) = rigidbody.handle {
                if let Some(rb) = self.rigid_body_set.get(handle) {
                    // Update transform from physics
                    let pos = rb.translation();
                    let rot = rb.rotation();

                    // Convert from nalgebra to nalgebra_glm types
                    transform.position = nalgebra_glm::vec3(pos.x, pos.y, pos.z);
                    transform.rotation = nalgebra_glm::quat(rot.w, rot.i, rot.j, rot.k);

                    // Update velocity if component exists
                    if let Some(vel) = velocity {
                        let linvel = rb.linvel();
                        let angvel = rb.angvel();
                        vel.linear = glam::Vec3::new(linvel.x, linvel.y, linvel.z);
                        vel.angular = glam::Vec3::new(angvel.x, angvel.y, angvel.z);
                    }
                }
            }
        }
    }

    /// Cast ray and return first hit
    pub fn raycast(
        &self,
        origin: Vector3<f32>,
        direction: Vector3<f32>,
        max_distance: f32,
        filter: QueryFilter,
    ) -> Option<(RigidBodyHandle, f32, Vector3<f32>)> {
        let ray = Ray::new(origin.into(), direction.into());

        let query_pipeline = QueryPipeline::new();

        if let Some((handle, toi)) = query_pipeline.cast_ray(
            &self.rigid_body_set,
            &self.collider_set,
            &ray,
            max_distance,
            true, // Solid
            filter,
        ) {
            let hit_point = ray.point_at(toi);
            Some((
                self.collider_set[handle].parent().unwrap(),
                toi,
                Vector3::new(hit_point.x, hit_point.y, hit_point.z),
            ))
        } else {
            None
        }
    }

    /// Check if point is inside any collider
    pub fn point_query(&self, point: Vector3<f32>) -> Vec<ColliderHandle> {
        let query_pipeline = QueryPipeline::new();
        let mut hits = Vec::new();

        let point_na = nalgebra::Point3::new(point.x, point.y, point.z);

        query_pipeline.intersections_with_point(
            &self.rigid_body_set,
            &self.collider_set,
            &point_na,
            QueryFilter::default(),
            |handle| {
                hits.push(handle);
                true // Continue search
            },
        );

        hits
    }
}

impl Default for PhysicsSystem {
    fn default() -> Self {
        Self::new()
    }
}
