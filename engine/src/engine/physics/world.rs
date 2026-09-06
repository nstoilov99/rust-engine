//! Physics world management using Rapier 3D
//!
//! ECS uses Z-up coordinates (Z=up, X=forward, Y=right).
//! Rapier uses Y-up internally. Conversion happens via physics_adapter.

use super::components::{
    Collider as EcsCollider, ColliderShape, RigidBody as EcsRigidBody,
    RigidBodyType as EcsRigidBodyType, Velocity as EcsVelocity,
};
use crate::engine::adapters::physics_adapter::{
    cuboid_half_extents_to_physics, position_from_physics, position_to_physics,
    rotation_from_physics, rotation_to_physics, velocity_from_physics, velocity_to_physics,
};
use crate::engine::ecs::components::Transform;
use hecs::{Entity, World};
use nalgebra_glm as glm;
use rapier3d::na::{Isometry3, Point3, Vector3};
use rapier3d::prelude::{
    CCDSolver, ColliderBuilder, ColliderHandle, ColliderSet, DefaultBroadPhase, ImpulseJointSet,
    IntegrationParameters, IslandManager, MultibodyJointSet, NarrowPhase, PhysicsPipeline,
    QueryFilter, QueryPipeline, Ray, RigidBodyBuilder, RigidBodyHandle, RigidBodySet, SharedShape,
};

/// A [`PhysicsWorld::raycast_filtered`] hit. Everything is ECS Z-up game
/// space (the adapter converts from Rapier's Y-up).
pub struct RayHit {
    pub collider: ColliderHandle,
    /// Distance along the ray (assuming a unit-length direction).
    pub distance: f32,
    pub point: glm::Vec3,
    /// Surface normal at the hit, unit length.
    pub normal: glm::Vec3,
}

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
    ///
    /// ECS uses Z-up coordinates. Rapier uses Y-up internally.
    /// Gravity in Z-up is (0, 0, -9.81) -> converts to Y-up (0, -9.81, 0).
    pub fn new() -> Self {
        // Gravity in Z-up space: down is -Z
        // Convert to Y-up for Rapier via physics_adapter
        let gravity_zup = glm::vec3(0.0, 0.0, -9.81);
        let gravity_yup = position_to_physics(&gravity_zup);

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
            gravity: gravity_yup,
            accumulator: 0.0,
            fixed_dt: 1.0 / 60.0,
        }
    }

    /// Set gravity vector (in ECS Z-up coordinates)
    pub fn set_gravity(&mut self, gravity: nalgebra_glm::Vec3) {
        self.gravity = crate::engine::adapters::physics_adapter::gravity_to_physics(&gravity);
    }

    /// Set fixed timestep for physics simulation (default: 1/60)
    pub fn set_timestep(&mut self, dt: f32) {
        self.fixed_dt = dt;
    }

    /// Reset the fixed-timestep accumulator to zero.
    /// Call after rebuilding physics to prevent stale time from triggering steps.
    pub fn reset_accumulator(&mut self) {
        self.accumulator = 0.0;
    }

    pub fn rigid_body_count(&self) -> u32 {
        self.rigid_body_set.len().min(u32::MAX as usize) as u32
    }

    /// Step physics with fixed timestep accumulator
    ///
    /// This accumulates frame time and runs physics at a fixed rate
    /// to ensure deterministic simulation.
    pub fn step(&mut self, delta_time: f32, ecs_world: &mut World) {
        crate::profile_function!();

        self.accumulator += delta_time;

        while self.accumulator >= self.fixed_dt {
            // Sync ECS -> Physics (kinematic bodies)
            {
                crate::profile_scope!("physics_sync_to_rapier");
                self.sync_ecs_to_physics(ecs_world);
            }

            // Run physics step
            {
                crate::profile_scope!("physics_pipeline_step");
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
            }

            // Sync Physics -> ECS (dynamic bodies)
            {
                crate::profile_scope!("physics_sync_from_rapier");
                self.sync_physics_to_ecs(ecs_world);
            }

            self.accumulator -= self.fixed_dt;
        }
    }

    /// Register an entity with the physics world
    ///
    /// Creates Rapier rigidbody and collider from ECS components.
    /// Does nothing if already registered.
    ///
    /// Uses physics_adapter for Z-up → Y-up coordinate conversion.
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

        // Convert Z-up ECS position to Y-up for Rapier via adapter
        let translation = position_to_physics(&transform.position);

        let rotation = rotation_to_physics(&transform.rotation);

        let rb = rb_builder
            .translation(translation)
            .rotation(rotation.scaled_axis())
            .linear_damping(rigidbody.linear_damping)
            .angular_damping(rigidbody.angular_damping)
            .can_sleep(rigidbody.can_sleep)
            .gravity_scale(rigidbody.gravity_scale)
            .ccd_enabled(rigidbody.continuous_collision)
            // `lock_rotation` is per Z-up axis [X, Y, Z]; Rapier's axes are
            // (x, y, z)_yup = (y, z, -x)_zup, and a lock ignores sign.
            .enabled_rotations(
                !rigidbody.lock_rotation[1],
                !rigidbody.lock_rotation[2],
                !rigidbody.lock_rotation[0],
            )
            .build();

        let rb_handle = self.rigid_body_set.insert(rb);
        rigidbody.handle = Some(rb_handle);

        // Build collider shape using adapter for dimension conversion
        let shape = match &collider.shape {
            ColliderShape::Cuboid { half_extents } => {
                let (hx, hy, hz) = cuboid_half_extents_to_physics(half_extents);
                SharedShape::cuboid(hx, hy, hz)
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

    /// Linear velocity of a body in Z-up game space; `None` for a stale handle.
    pub fn linear_velocity(&self, handle: RigidBodyHandle) -> Option<glm::Vec3> {
        self.rigid_body_set
            .get(handle)
            .map(|rb| velocity_from_physics(rb.linvel()))
    }

    /// Set a body's linear velocity (Z-up game space) and wake it. The
    /// character controller drives its capsule this way (Task 41.6 D1).
    pub fn set_linear_velocity(&mut self, handle: RigidBodyHandle, velocity: glm::Vec3) {
        if let Some(rb) = self.rigid_body_set.get_mut(handle) {
            rb.set_linvel(velocity_to_physics(&velocity), true);
        }
    }

    /// Set a body's rotation (Z-up game space) and wake it. A gameplay
    /// system that writes `Transform.rotation` on a dynamic body must write
    /// it here too: every fixed step copies the body's rotation back into
    /// the transform.
    pub fn set_rotation(&mut self, handle: RigidBodyHandle, rotation: &glm::Quat) {
        if let Some(rb) = self.rigid_body_set.get_mut(handle) {
            rb.set_rotation(rotation_to_physics(rotation), true);
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
        let physics_origin = position_to_physics(&origin);
        let physics_direction = position_to_physics(&direction);
        let ray = Ray::new(
            Point3::new(physics_origin.x, physics_origin.y, physics_origin.z),
            Vector3::new(
                physics_direction.x,
                physics_direction.y,
                physics_direction.z,
            ),
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
                    position_from_physics(&Vector3::new(hit_point.x, hit_point.y, hit_point.z)),
                )
            })
    }

    /// Cast a ray and return the first hit *with its surface normal*,
    /// optionally excluding one rigid body (and every collider attached to
    /// it) — a character probing the ground must not hit itself.
    ///
    /// `origin` / `direction` are Z-up game space, like [`Self::raycast`]
    /// (which stays as-is for its callers); this variant exists for foot
    /// placement (Task 41.5 P6, I-D4).
    pub fn raycast_filtered(
        &self,
        origin: nalgebra_glm::Vec3,
        direction: nalgebra_glm::Vec3,
        max_distance: f32,
        exclude: Option<RigidBodyHandle>,
    ) -> Option<RayHit> {
        let physics_origin = position_to_physics(&origin);
        let physics_direction = position_to_physics(&direction);
        let ray = Ray::new(
            Point3::new(physics_origin.x, physics_origin.y, physics_origin.z),
            Vector3::new(
                physics_direction.x,
                physics_direction.y,
                physics_direction.z,
            ),
        );
        let mut filter = QueryFilter::default();
        if let Some(rb) = exclude {
            filter = filter.exclude_rigid_body(rb);
        }

        self.query_pipeline
            .cast_ray_and_get_normal(
                &self.rigid_body_set,
                &self.collider_set,
                &ray,
                max_distance,
                true,
                filter,
            )
            .map(|(collider, hit)| {
                let point = ray.point_at(hit.time_of_impact);
                RayHit {
                    collider,
                    distance: hit.time_of_impact,
                    point: position_from_physics(&Vector3::new(point.x, point.y, point.z)),
                    // A normal is a free vector: the same pure-rotation
                    // conversion velocities use.
                    normal: velocity_from_physics(&hit.normal),
                }
            })
    }

    /// Sync ECS transforms to physics world (for kinematic bodies)
    ///
    /// Uses physics_adapter for Z-up → Y-up conversion.
    fn sync_ecs_to_physics(&mut self, ecs_world: &World) {
        for (_, (transform, rigidbody)) in ecs_world.query::<(&Transform, &EcsRigidBody)>().iter() {
            // Only update kinematic bodies
            if rigidbody.body_type != EcsRigidBodyType::Kinematic {
                continue;
            }

            if let Some(handle) = rigidbody.handle {
                if let Some(rb) = self.rigid_body_set.get_mut(handle) {
                    // Convert via physics_adapter
                    let translation = position_to_physics(&transform.position);

                    let rotation = rotation_to_physics(&transform.rotation);

                    rb.set_next_kinematic_position(Isometry3::from_parts(
                        translation.into(),
                        rotation,
                    ));
                }
            }
        }
    }

    /// Sync physics world to ECS transforms (for dynamic bodies)
    ///
    /// Uses physics_adapter for Y-up → Z-up conversion.
    fn sync_physics_to_ecs(&self, ecs_world: &mut World) {
        let mut dirty_entities: Vec<Entity> = Vec::new();
        for (entity, (transform, rigidbody, velocity)) in ecs_world
            .query::<(&mut Transform, &EcsRigidBody, Option<&mut EcsVelocity>)>()
            .iter()
        {
            // Static bodies don't need sync
            if rigidbody.body_type == EcsRigidBodyType::Static {
                continue;
            }

            if let Some(handle) = rigidbody.handle {
                if let Some(rb) = self.rigid_body_set.get(handle) {
                    // Convert via physics_adapter
                    let pos_zup = position_from_physics(rb.translation());
                    transform.position = pos_zup;

                    transform.rotation = rotation_from_physics(rb.rotation());

                    dirty_entities.push(entity);

                    // Update velocity if component exists (convert via adapter)
                    if let Some(vel) = velocity {
                        vel.linear = velocity_from_physics(rb.linvel());
                        vel.angular = velocity_from_physics(rb.angvel());
                    }
                }
            }
        }
        for entity in dirty_entities {
            crate::engine::ecs::hierarchy::mark_transform_dirty(ecs_world, entity);
        }
    }
}

impl Default for PhysicsWorld {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::ecs::components::Transform;
    use hecs::World;
    use nalgebra_glm as glm;

    /// Helper: spawn a physics entity and register it.
    fn spawn_and_register(
        world: &mut World,
        physics: &mut PhysicsWorld,
        pos: glm::Vec3,
        rb: EcsRigidBody,
        col: EcsCollider,
    ) -> Entity {
        let entity = world.spawn((Transform::new(pos), rb, col));
        {
            let mut query = world.query::<(&Transform, &mut EcsRigidBody, &mut EcsCollider)>();
            let (_, (transform, rigidbody, collider)) = query
                .iter()
                .find(|(e, _)| *e == entity)
                .expect("spawned entity should exist");
            physics.register_entity(transform, rigidbody, collider);
        }
        entity
    }

    #[test]
    fn register_dynamic_body() {
        let mut world = World::new();
        let mut physics = PhysicsWorld::new();

        let entity = spawn_and_register(
            &mut world,
            &mut physics,
            glm::vec3(0.0, 0.0, 5.0),
            EcsRigidBody::dynamic(),
            EcsCollider::cuboid(0.5, 0.5, 0.5),
        );

        assert_eq!(physics.rigid_body_set.len(), 1);
        assert_eq!(physics.collider_set.len(), 1);

        let rb = world.get::<&EcsRigidBody>(entity).expect("rb should exist");
        assert!(
            rb.handle.is_some(),
            "handle should be assigned after registration"
        );
    }

    #[test]
    fn velocity_and_rotation_roundtrip_in_zup() {
        let mut world = World::new();
        let mut physics = PhysicsWorld::new();
        let entity = spawn_and_register(
            &mut world,
            &mut physics,
            glm::vec3(0.0, 0.0, 1.0),
            EcsRigidBody::dynamic(),
            EcsCollider::capsule(0.5, 0.4),
        );
        let handle = world.get::<&EcsRigidBody>(entity).unwrap().handle.unwrap();

        physics.set_linear_velocity(handle, glm::vec3(1.0, 2.0, 3.0));
        let v = physics.linear_velocity(handle).expect("live handle");
        assert!((v - glm::vec3(1.0, 2.0, 3.0)).norm() < 1e-5, "{v:?}");

        let yaw = glm::quat_angle_axis(0.7, &glm::vec3(0.0, 0.0, 1.0));
        physics.set_rotation(handle, &yaw);
        let rb = &physics.rigid_body_set[handle];
        let back = rotation_from_physics(rb.rotation());
        let fwd = glm::quat_rotate_vec3(&back, &glm::vec3(1.0, 0.0, 0.0));
        assert!((fwd.y.atan2(fwd.x) - 0.7).abs() < 1e-5);
    }

    #[test]
    fn lock_rotation_maps_zup_axes_to_rapier() {
        let mut world = World::new();
        let mut physics = PhysicsWorld::new();
        let mut rb = EcsRigidBody::dynamic();
        rb.lock_rotation = [true, false, true]; // Z-up X and Z
        let entity = spawn_and_register(
            &mut world,
            &mut physics,
            glm::vec3(0.0, 0.0, 1.0),
            rb,
            EcsCollider::cuboid(0.5, 0.5, 0.5),
        );
        let handle = world.get::<&EcsRigidBody>(entity).unwrap().handle.unwrap();
        // Rapier (x, y, z) = Z-up (y, z, x): Y-up y (= Z-up Z) and z (= Z-up X) locked.
        assert_eq!(
            physics.rigid_body_set[handle].is_rotation_locked(),
            [false, true, true]
        );
    }

    #[test]
    fn register_static_body() {
        let mut world = World::new();
        let mut physics = PhysicsWorld::new();

        spawn_and_register(
            &mut world,
            &mut physics,
            glm::vec3(0.0, 0.0, 0.0),
            EcsRigidBody::fixed(),
            EcsCollider::cuboid(10.0, 10.0, 0.1),
        );

        assert_eq!(physics.rigid_body_set.len(), 1);
        assert_eq!(physics.collider_set.len(), 1);
    }

    #[test]
    fn register_kinematic_body() {
        let mut world = World::new();
        let mut physics = PhysicsWorld::new();

        spawn_and_register(
            &mut world,
            &mut physics,
            glm::vec3(1.0, 2.0, 3.0),
            EcsRigidBody::kinematic(),
            EcsCollider::ball(1.0),
        );

        assert_eq!(physics.rigid_body_set.len(), 1);
        assert_eq!(physics.collider_set.len(), 1);
    }

    #[test]
    fn skip_already_registered() {
        let mut world = World::new();
        let mut physics = PhysicsWorld::new();

        let entity = spawn_and_register(
            &mut world,
            &mut physics,
            glm::vec3(0.0, 0.0, 0.0),
            EcsRigidBody::dynamic(),
            EcsCollider::cuboid(1.0, 1.0, 1.0),
        );

        // Try to register again
        {
            let mut query = world.query::<(&Transform, &mut EcsRigidBody, &mut EcsCollider)>();
            let (_, (transform, rigidbody, collider)) = query
                .iter()
                .find(|(e, _)| *e == entity)
                .expect("entity should exist");
            physics.register_entity(transform, rigidbody, collider);
        }

        // Should still have only one body
        assert_eq!(physics.rigid_body_set.len(), 1);
    }

    #[test]
    fn dynamic_body_falls_under_gravity() {
        let mut world = World::new();
        let mut physics = PhysicsWorld::new();

        let entity = spawn_and_register(
            &mut world,
            &mut physics,
            glm::vec3(0.0, 0.0, 10.0),
            EcsRigidBody::dynamic(),
            EcsCollider::ball(0.5),
        );

        // Step physics for several frames
        for _ in 0..10 {
            physics.step(1.0 / 60.0, &mut world);
        }

        let transform = world
            .get::<&Transform>(entity)
            .expect("transform should exist");
        assert!(
            transform.position.z < 10.0,
            "dynamic body should have fallen: z = {}",
            transform.position.z
        );
    }

    #[test]
    fn static_body_does_not_move() {
        let mut world = World::new();
        let mut physics = PhysicsWorld::new();

        let entity = spawn_and_register(
            &mut world,
            &mut physics,
            glm::vec3(0.0, 0.0, 0.0),
            EcsRigidBody::fixed(),
            EcsCollider::cuboid(10.0, 10.0, 0.1),
        );

        for _ in 0..10 {
            physics.step(1.0 / 60.0, &mut world);
        }

        let transform = world
            .get::<&Transform>(entity)
            .expect("transform should exist");
        assert!(
            (transform.position.z).abs() < 0.001,
            "static body should not move: z = {}",
            transform.position.z
        );
    }

    #[test]
    fn collider_ball_shape() {
        let mut world = World::new();
        let mut physics = PhysicsWorld::new();

        spawn_and_register(
            &mut world,
            &mut physics,
            glm::vec3(0.0, 0.0, 0.0),
            EcsRigidBody::dynamic(),
            EcsCollider::ball(2.5),
        );

        assert_eq!(physics.collider_set.len(), 1);
        // Verify the collider was created (we can't easily inspect shape type
        // without Rapier internals, but creation success is the key test)
    }

    #[test]
    fn collider_capsule_shape() {
        let mut world = World::new();
        let mut physics = PhysicsWorld::new();

        spawn_and_register(
            &mut world,
            &mut physics,
            glm::vec3(0.0, 0.0, 0.0),
            EcsRigidBody::dynamic(),
            EcsCollider::capsule(1.0, 0.5),
        );

        assert_eq!(physics.collider_set.len(), 1);
    }

    #[test]
    fn gravity_direction_is_correct() {
        let physics = PhysicsWorld::new();
        // Default gravity in Z-up is (0, 0, -9.81)
        // After conversion to Y-up: (0, -9.81, 0)
        assert!((physics.gravity.x).abs() < 0.001);
        assert!((physics.gravity.y - (-9.81)).abs() < 0.01);
        assert!((physics.gravity.z).abs() < 0.001);
    }

    #[test]
    fn multiple_bodies_registered() {
        let mut world = World::new();
        let mut physics = PhysicsWorld::new();

        for i in 0..5 {
            spawn_and_register(
                &mut world,
                &mut physics,
                glm::vec3(i as f32 * 2.0, 0.0, 5.0),
                EcsRigidBody::dynamic(),
                EcsCollider::cuboid(0.5, 0.5, 0.5),
            );
        }

        assert_eq!(physics.rigid_body_set.len(), 5);
        assert_eq!(physics.collider_set.len(), 5);
    }
}
