//! Debug visualization for bone hierarchies.
//!
//! Draws parent→child bone lines and joint crosses using the
//! engine's immediate-mode debug draw buffer.

use crate::engine::animation::SkeletonInstance;
use crate::engine::debug_draw::DebugDrawBuffer;
use crate::engine::ecs::components::Transform;
use crate::engine::ecs::hierarchy::TransformCache;
use hecs::{Entity, World};

/// Bone line color (cyan).
const BONE_COLOR: [f32; 4] = [0.0, 0.9, 0.9, 1.0];
/// Joint cross size in world units.
const JOINT_CROSS_SIZE: f32 = 0.03;

/// Submit bone debug draw lines for all skeletons with `debug_draw_visible`.
///
/// Coordinates are output in Z-up game space (matching the debug draw API).
pub fn submit_skeleton_debug_draws(
    world: &World,
    buffer: &mut DebugDrawBuffer,
    transform_cache: &TransformCache,
) {
    for (entity, (_transform, skeleton)) in
        world.query::<(&Transform, &SkeletonInstance)>().iter()
    {
        if !skeleton.debug_draw_visible || skeleton.bones.is_empty() {
            continue;
        }
        submit_one(entity, skeleton, buffer, transform_cache);
    }
}

fn submit_one(
    entity: Entity,
    skeleton: &SkeletonInstance,
    buffer: &mut DebugDrawBuffer,
    transform_cache: &TransformCache,
) {
    let entity_render = transform_cache.get_render(entity);

    // entity_render is Y-up render space (nalgebra_glm::Mat4).
    // Convert to glam::Mat4 for bone math.
    let entity_mat = glam::Mat4::from_cols_array_2d(&unsafe {
        std::mem::transmute::<nalgebra_glm::Mat4, [[f32; 4]; 4]>(entity_render)
    });

    // Compute world-space bone positions (in Y-up render space).
    // palette[i] = world_bone[i] * inverse_bind[i]
    // world_bone[i] = palette[i] * inverse(inverse_bind[i])
    let bone_count = skeleton.bones.len();
    let world_positions: Vec<[f32; 3]> = (0..bone_count)
        .map(|i| {
            let world_bone =
                skeleton.palette[i] * skeleton.bones[i].inverse_bind_matrix.inverse();
            let scene_pos = entity_mat * world_bone.w_axis;
            // Convert Y-up render → Z-up game for debug draw API:
            // render (x, y, z) → game (x, z, -y)
            [scene_pos.x, scene_pos.z, -scene_pos.y]
        })
        .collect();

    // Draw parent→child bone lines
    for i in 0..bone_count {
        if let Some(parent) = skeleton.bones[i].parent_index {
            buffer.line(world_positions[parent], world_positions[i], BONE_COLOR);
        }
        // Draw a small cross at each joint
        let p = world_positions[i];
        let s = JOINT_CROSS_SIZE;
        buffer.line(
            [p[0] - s, p[1], p[2]],
            [p[0] + s, p[1], p[2]],
            [0.9, 0.2, 0.2, 1.0],
        );
        buffer.line(
            [p[0], p[1] - s, p[2]],
            [p[0], p[1] + s, p[2]],
            [0.2, 0.9, 0.2, 1.0],
        );
        buffer.line(
            [p[0], p[1], p[2] - s],
            [p[0], p[1], p[2] + s],
            [0.2, 0.2, 0.9, 1.0],
        );
    }
}
