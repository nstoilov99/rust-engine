//! Debug visualization for bone hierarchies.
//!
//! Draws parent→child bone lines and joint crosses using the
//! engine's immediate-mode debug draw buffer. Lines are overlay
//! (not depth-tested): bones sit inside the skinned mesh and would
//! otherwise be hidden by the character's own skin.

use crate::engine::animation::SkeletonInstance;
use crate::engine::debug_draw::DebugDrawBuffer;
use crate::engine::ecs::components::Transform;
use crate::engine::ecs::hierarchy::TransformCache;
use crate::engine::utils::coords::convert_position_yup_to_zup;
use hecs::World;

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
    for (entity, (_transform, skeleton)) in world.query::<(&Transform, &SkeletonInstance)>().iter()
    {
        if !skeleton.debug_draw_visible || skeleton.bones.is_empty() {
            continue;
        }
        let entity_render = transform_cache.get_render(entity);
        let entity_mat = glam::Mat4::from_cols_slice(entity_render.as_slice());
        submit_one(skeleton, &joint_positions(skeleton, entity_mat), buffer);
    }
}

/// World-space joint positions in Z-up game space, indexed like `bones`.
///
/// `entity_render` is the entity's Y-up render matrix — the same `model`
/// the skinning shader multiplies by, so the joints land exactly where
/// the skinned mesh is drawn. `model_space[i]` is the retained FK phase-1
/// output (pre-inverse-bind), whose translation is the joint.
fn joint_positions(skeleton: &SkeletonInstance, entity_render: glam::Mat4) -> Vec<[f32; 3]> {
    skeleton
        .model_space
        .iter()
        .map(|model_bone| {
            let render_pos = entity_render * model_bone.w_axis;
            convert_position_yup_to_zup(render_pos.truncate()).to_array()
        })
        .collect()
}

fn submit_one(skeleton: &SkeletonInstance, positions: &[[f32; 3]], buffer: &mut DebugDrawBuffer) {
    for (i, bone) in skeleton.bones.iter().enumerate() {
        if let Some(parent) = bone.parent_index {
            buffer.line_overlay(positions[parent], positions[i], BONE_COLOR);
        }
    }
    // Small RGB cross at each joint (game-space X/Y/Z), on top of the lines.
    let s = JOINT_CROSS_SIZE;
    for p in positions {
        let [x, y, z] = *p;
        buffer.line_overlay([x - s, y, z], [x + s, y, z], [0.9, 0.2, 0.2, 1.0]);
        buffer.line_overlay([x, y - s, z], [x, y + s, z], [0.2, 0.9, 0.2, 1.0]);
        buffer.line_overlay([x, y, z - s], [x, y, z + s], [0.2, 0.2, 0.9, 1.0]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::adapters::render_adapter::world_matrix_to_render;
    use crate::engine::assets::model_loader::BoneData;
    use glam::{Mat4, Vec3};

    fn two_bone_skeleton() -> SkeletonInstance {
        // Y-up mesh space (what the cooked mesh and its bones live in):
        // root at the origin, child one unit *up* the mesh.
        let mut skeleton = SkeletonInstance::from_bones(vec![
            BoneData {
                name: "root".into(),
                parent_index: None,
                inverse_bind_matrix: Mat4::IDENTITY,
            },
            BoneData {
                name: "head".into(),
                parent_index: Some(0),
                inverse_bind_matrix: Mat4::from_translation(Vec3::new(0.0, 1.0, 0.0)).inverse(),
            },
        ]);
        // `from_bones` filled the retained `model_space` (the input joint
        // positions read); poison the palette to pin that they no longer
        // come from inverting it.
        skeleton.palette.fill(Mat4::ZERO);
        skeleton
    }

    #[test]
    fn joints_come_out_in_zup_game_space_where_the_mesh_is_drawn() {
        let skeleton = two_bone_skeleton();
        // Entity sits at game (2, 3, 0) — forward 2, right 3, on the floor.
        let world_zup = nalgebra_glm::translation(&nalgebra_glm::vec3(2.0, 3.0, 0.0));
        let render = world_matrix_to_render(&world_zup);
        let entity = Mat4::from_cols_slice(render.as_slice());

        let pos = joint_positions(&skeleton, entity);
        assert_eq!(pos[0], [2.0, 3.0, 0.0]);
        // The head is *above* the root in game space (+Z), not below it.
        assert_eq!(pos[1], [2.0, 3.0, 1.0]);
    }

    #[test]
    fn draws_one_bone_line_per_parented_bone_plus_crosses_as_overlay() {
        let skeleton = two_bone_skeleton();
        let mut buffer = DebugDrawBuffer::new();
        submit_one(
            &skeleton,
            &joint_positions(&skeleton, Mat4::IDENTITY),
            &mut buffer,
        );
        let (depth, overlay) = buffer.drain();
        assert!(depth.is_empty());
        assert_eq!(overlay.len(), 1 + 2 * 3);
        assert_eq!(overlay[0].color, BONE_COLOR);
    }
}
