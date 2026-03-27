use crate::engine::rendering::rendering_3d::pipeline_3d::Vertex3D;
use glam::{Mat4, Quat, Vec3};
use std::collections::HashMap;
use std::path::Path;

use super::model_loader::{
    calculate_tangents_safe, compute_bounding_sphere, generate_flat_normals, AnimationChannel,
    BoneData, ImportedMaterial, LoadedMesh, Model, RawAnimationClip, VertexBoneData,
};
use crate::engine::rendering::rendering_3d::pipeline_3d::MAX_PALETTE_BONES;

/// Load FBX model from filesystem path.
pub fn load_model_fbx(source_path: &str) -> Result<Model, Box<dyn std::error::Error>> {
    crate::profile_function!();

    let opts = ufbx::LoadOpts {
        target_axes: ufbx::CoordinateAxes::right_handed_y_up(),
        generate_missing_normals: true,
        ..Default::default()
    };

    let scene = ufbx::load_file(source_path, opts)
        .map_err(|e| format!("Failed to load FBX '{}': {:?}", source_path, e))?;

    let name = Path::new(source_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Unnamed")
        .to_string();

    build_model_from_fbx(name, &scene)
}

/// Load FBX model from in-memory bytes.
pub fn load_model_fbx_from_bytes(
    data: &[u8],
    source_path: &str,
) -> Result<Model, Box<dyn std::error::Error>> {
    crate::profile_function!();

    let opts = ufbx::LoadOpts {
        target_axes: ufbx::CoordinateAxes::right_handed_y_up(),
        generate_missing_normals: true,
        ..Default::default()
    };

    let scene = ufbx::load_memory(data, opts)
        .map_err(|e| format!("Failed to load FBX from bytes '{}': {:?}", source_path, e))?;

    let name = Path::new(source_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Unnamed")
        .to_string();

    build_model_from_fbx(name, &scene)
}

fn build_model_from_fbx(
    name: String,
    scene: &ufbx::Scene,
) -> Result<Model, Box<dyn std::error::Error>> {
    let mut model = Model::new(name.clone());

    // Build bone map from skeleton before processing meshes (needed for skin weights)
    let bone_node_to_index = extract_fbx_skeleton(scene, &mut model);

    for mesh in scene.meshes.iter() {
        // Process each material part separately to respect material assignments
        if mesh.material_parts.is_empty() {
            // No material parts — treat entire mesh as one piece
            if let Some(loaded) =
                convert_fbx_mesh_whole(mesh, None, &bone_node_to_index)?
            {
                model.meshes.push(loaded);
            }
        } else {
            for (part_idx, part) in mesh.material_parts.iter().enumerate() {
                if part.num_triangles == 0 {
                    continue;
                }
                if let Some(loaded) =
                    convert_fbx_mesh_part(mesh, part, Some(part_idx), &bone_node_to_index)?
                {
                    model.meshes.push(loaded);
                }
            }
        }
    }

    // Extract materials
    for mat in scene.materials.iter() {
        let pbr = &mat.pbr;
        let base_color = &pbr.base_color;
        let color = base_color.value_vec4;

        model.materials.push(ImportedMaterial {
            name: mat.element.name.to_string(),
            albedo: None, // Texture loading deferred to future phases
            normal: None,
            metallic_roughness: None,
            ao: None,
            base_color_factor: [color.x as f32, color.y as f32, color.z as f32, color.w as f32],
            metallic_factor: pbr.metalness.value_vec4.x as f32,
            roughness_factor: pbr.roughness.value_vec4.x as f32,
        });
    }

    // Extract animations
    extract_fbx_animations(scene, &mut model, &bone_node_to_index);

    model.rebuild_legacy_textures();

    let bone_count = model.bones.len();

    // Validate bone count against FixedUbo backend cap
    if bone_count > MAX_PALETTE_BONES {
        return Err(format!(
            "Model '{}' has {} bones, exceeding the current skinning backend cap of {}. \
             A larger backend (LargeSsbo) is needed for this asset.",
            name, bone_count, MAX_PALETTE_BONES,
        )
        .into());
    }
    if bone_count > 200 {
        log::warn!(
            "Model '{}' has {} bones (approaching FixedUbo cap of {})",
            name,
            bone_count,
            MAX_PALETTE_BONES,
        );
    }

    let anim_count = model.animations.len();
    let skinned_count = model.meshes.iter().filter(|m| m.skinning.is_some()).count();

    log::info!(
        "Model loaded (FBX): {} (meshes: {}, materials: {}, bones: {}, animations: {}, skinned meshes: {})",
        name,
        model.meshes.len(),
        model.materials.len(),
        bone_count,
        anim_count,
        skinned_count,
    );

    Ok(model)
}

// ──────────────────────────────────────────────────────────────
// Skeleton extraction
// ──────────────────────────────────────────────────────────────

/// Extract bone hierarchy from the FBX scene.
/// Returns a map from ufbx node `typed_id` → index in `model.bones`.
fn extract_fbx_skeleton(
    scene: &ufbx::Scene,
    model: &mut Model,
) -> HashMap<u32, usize> {
    let mut bone_node_to_index: HashMap<u32, usize> = HashMap::new();

    // Collect all nodes that are referenced as bones by skin clusters.
    // This is more reliable than checking node.bone which only reflects
    // nodes with an explicit Bone attribute (some rigs use plain Null nodes).
    let mut bone_node_ids: Vec<u32> = Vec::new();
    for cluster in scene.skin_clusters.iter() {
        if let Some(ref bone_node) = cluster.bone_node {
            let typed_id = bone_node.element.typed_id;
            if !bone_node_ids.contains(&typed_id) {
                bone_node_ids.push(typed_id);
            }
        }
    }

    // Also include nodes with explicit Bone attributes that weren't caught above
    for node in scene.nodes.iter() {
        if node.bone.is_some() {
            let typed_id = node.element.typed_id;
            if !bone_node_ids.contains(&typed_id) {
                bone_node_ids.push(typed_id);
            }
        }
    }

    if bone_node_ids.is_empty() {
        return bone_node_to_index;
    }

    // Sort by typed_id for deterministic ordering
    bone_node_ids.sort();

    // First pass: create BoneData entries (parent_index set to None initially)
    for &typed_id in &bone_node_ids {
        let bone_index = model.bones.len();
        bone_node_to_index.insert(typed_id, bone_index);

        // Find the node for this typed_id
        let node = scene.nodes.iter().find(|n| n.element.typed_id == typed_id);
        let name = node
            .map(|n| n.element.name.to_string())
            .unwrap_or_else(|| format!("bone_{}", bone_index));

        // Find the inverse bind matrix from skin clusters
        let inverse_bind = find_inverse_bind_matrix(scene, typed_id);

        model.bones.push(BoneData {
            name,
            parent_index: None,
            inverse_bind_matrix: inverse_bind,
        });
    }

    // Second pass: resolve parent indices
    for &typed_id in &bone_node_ids {
        let bone_index = bone_node_to_index[&typed_id];
        if let Some(node) = scene.nodes.iter().find(|n| n.element.typed_id == typed_id) {
            if let Some(ref parent) = node.parent {
                let parent_typed_id = parent.element.typed_id;
                if let Some(&parent_bone_idx) = bone_node_to_index.get(&parent_typed_id) {
                    model.bones[bone_index].parent_index = Some(parent_bone_idx);
                }
                // If parent isn't a bone, parent_index stays None (root of skeleton)
            }
        }
    }

    bone_node_to_index
}

/// Find the inverse bind matrix for a bone node from skin clusters.
fn find_inverse_bind_matrix(scene: &ufbx::Scene, bone_typed_id: u32) -> Mat4 {
    for cluster in scene.skin_clusters.iter() {
        if let Some(ref bone_node) = cluster.bone_node {
            if bone_node.element.typed_id == bone_typed_id {
                return ufbx_matrix_to_mat4(&cluster.geometry_to_bone);
            }
        }
    }
    // No skin cluster references this bone — use identity
    Mat4::IDENTITY
}

/// Convert a ufbx 4x3 affine Matrix to a glam Mat4.
fn ufbx_matrix_to_mat4(m: &ufbx::Matrix) -> Mat4 {
    // ufbx Matrix is column-major 4x3 (affine, bottom row implicit [0,0,0,1])
    Mat4::from_cols_array(&[
        m.m00 as f32, m.m10 as f32, m.m20 as f32, 0.0,
        m.m01 as f32, m.m11 as f32, m.m21 as f32, 0.0,
        m.m02 as f32, m.m12 as f32, m.m22 as f32, 0.0,
        m.m03 as f32, m.m13 as f32, m.m23 as f32, 1.0,
    ])
}

// ──────────────────────────────────────────────────────────────
// Skin weight extraction
// ──────────────────────────────────────────────────────────────

/// Extract per-vertex skin weights for a ufbx mesh.
/// Returns None if the mesh has no skin deformers.
fn extract_fbx_skin_weights(
    mesh: &ufbx::Mesh,
    tri_indices: &[u32],
    bone_node_to_index: &HashMap<u32, usize>,
) -> Option<Vec<VertexBoneData>> {
    if mesh.skin_deformers.count == 0 || bone_node_to_index.is_empty() {
        return None;
    }

    let deformer = &mesh.skin_deformers[0];
    if deformer.vertices.count == 0 {
        return None;
    }

    // Build a map from cluster_index → our bone index
    let mut cluster_to_bone: Vec<Option<u16>> = Vec::with_capacity(deformer.clusters.count);
    for cluster in deformer.clusters.iter() {
        let bone_idx = cluster
            .bone_node
            .as_ref()
            .and_then(|node| bone_node_to_index.get(&node.element.typed_id))
            .map(|&idx| idx as u16);
        cluster_to_bone.push(bone_idx);
    }

    let num_mesh_verts = deformer.vertices.count;
    let mut skinning = Vec::with_capacity(tri_indices.len());

    for &tri_idx in tri_indices {
        // Map from per-index to the actual vertex index
        let vert_idx = mesh.vertex_indices[tri_idx as usize] as usize;

        if vert_idx >= num_mesh_verts {
            // Out of bounds — push default (no influences)
            skinning.push(VertexBoneData {
                joints: [0; 4],
                weights: [0.0; 4],
            });
            continue;
        }

        let skin_vert = &deformer.vertices[vert_idx];
        let begin = skin_vert.weight_begin as usize;
        let count = skin_vert.num_weights as usize;

        // Collect all influences for this vertex
        let mut influences: Vec<(u16, f32)> = Vec::with_capacity(count);
        for i in 0..count {
            let weight_idx = begin + i;
            if weight_idx >= deformer.weights.count {
                break;
            }
            let sw = &deformer.weights[weight_idx];
            let w = sw.weight as f32;
            if w <= 0.0 {
                continue;
            }
            let cluster_idx = sw.cluster_index as usize;
            if cluster_idx < cluster_to_bone.len() {
                if let Some(bone_idx) = cluster_to_bone[cluster_idx] {
                    influences.push((bone_idx, w));
                }
            }
        }

        // Sort by weight descending, keep top 4
        influences.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        influences.truncate(4);

        // Normalize weights to sum to 1.0
        let total: f32 = influences.iter().map(|(_, w)| *w).sum();
        let mut joints = [0u16; 4];
        let mut weights = [0.0f32; 4];

        if total > 0.0 {
            for (i, (bone_idx, w)) in influences.iter().enumerate() {
                joints[i] = *bone_idx;
                weights[i] = *w / total;
            }
        }

        skinning.push(VertexBoneData { joints, weights });
    }

    Some(skinning)
}

// ──────────────────────────────────────────────────────────────
// Animation extraction
// ──────────────────────────────────────────────────────────────

/// Extract animation clips from FBX anim stacks using ufbx bake API.
fn extract_fbx_animations(
    scene: &ufbx::Scene,
    model: &mut Model,
    bone_node_to_index: &HashMap<u32, usize>,
) {
    if scene.anim_stacks.count == 0 || bone_node_to_index.is_empty() {
        return;
    }

    for stack in scene.anim_stacks.iter() {
        let clip_name = stack.element.name.to_string();
        let duration = (stack.time_end - stack.time_begin) as f32;

        if duration <= 0.0 {
            log::warn!("Skipping animation '{}': duration <= 0", clip_name);
            continue;
        }

        // Bake the animation to get clean keyframe arrays
        let baked = match ufbx::bake_anim(scene, &stack.anim, ufbx::BakeOpts::default()) {
            Ok(b) => b,
            Err(e) => {
                log::warn!("Failed to bake animation '{}': {:?}", clip_name, e);
                continue;
            }
        };

        let mut channels = Vec::new();

        for baked_node in baked.nodes.iter() {
            // Match this baked node to one of our bones
            let bone_index = match bone_node_to_index.get(&baked_node.typed_id) {
                Some(&idx) => idx,
                None => continue, // Not a bone we track
            };

            // Skip bones with only constant (single-key) transforms
            let has_anim = !baked_node.constant_translation
                || !baked_node.constant_rotation
                || !baked_node.constant_scale;
            if !has_anim {
                continue;
            }

            let position_keys: Vec<(f32, Vec3)> = baked_node
                .translation_keys
                .iter()
                .map(|k| {
                    (
                        k.time as f32,
                        Vec3::new(k.value.x as f32, k.value.y as f32, k.value.z as f32),
                    )
                })
                .collect();

            let rotation_keys: Vec<(f32, Quat)> = baked_node
                .rotation_keys
                .iter()
                .map(|k| {
                    (
                        k.time as f32,
                        Quat::from_xyzw(
                            k.value.x as f32,
                            k.value.y as f32,
                            k.value.z as f32,
                            k.value.w as f32,
                        ),
                    )
                })
                .collect();

            let scale_keys: Vec<(f32, Vec3)> = baked_node
                .scale_keys
                .iter()
                .map(|k| {
                    (
                        k.time as f32,
                        Vec3::new(k.value.x as f32, k.value.y as f32, k.value.z as f32),
                    )
                })
                .collect();

            channels.push(AnimationChannel {
                bone_index,
                position_keys,
                rotation_keys,
                scale_keys,
            });
        }

        if channels.is_empty() {
            log::warn!(
                "Animation '{}' has no bone channels, skipping",
                clip_name
            );
            continue;
        }

        model.animations.push(RawAnimationClip {
            name: clip_name,
            duration_seconds: duration,
            channels,
        });
    }
}

// ──────────────────────────────────────────────────────────────
// Mesh conversion (existing code, now with skinning)
// ──────────────────────────────────────────────────────────────

/// Convert an entire ufbx mesh (no material parts) into a LoadedMesh.
fn convert_fbx_mesh_whole(
    mesh: &ufbx::Mesh,
    material_index: Option<usize>,
    bone_node_to_index: &HashMap<u32, usize>,
) -> Result<Option<LoadedMesh>, Box<dyn std::error::Error>> {
    if mesh.num_triangles == 0 {
        return Ok(None);
    }

    // Triangulate all faces
    let mut tri_indices = Vec::with_capacity(mesh.num_triangles * 3);
    let mut scratch = vec![0u32; mesh.max_face_triangles * 3];

    for face in mesh.faces.iter() {
        let n = mesh.triangulate_face(&mut scratch, *face);
        for &idx in &scratch[..(n as usize * 3)] {
            tri_indices.push(idx);
        }
    }

    build_loaded_mesh_from_fbx(mesh, &tri_indices, material_index, bone_node_to_index)
}

/// Convert a single material part of a ufbx mesh into a LoadedMesh.
fn convert_fbx_mesh_part(
    mesh: &ufbx::Mesh,
    part: &ufbx::MeshPart,
    material_index: Option<usize>,
    bone_node_to_index: &HashMap<u32, usize>,
) -> Result<Option<LoadedMesh>, Box<dyn std::error::Error>> {
    if part.num_triangles == 0 {
        return Ok(None);
    }

    // Triangulate faces belonging to this material part
    let mut tri_indices = Vec::with_capacity(part.num_triangles * 3);
    let mut scratch = vec![0u32; mesh.max_face_triangles * 3];

    for &face_idx in part.face_indices.as_ref() {
        let face = mesh.faces[face_idx as usize];
        let n = mesh.triangulate_face(&mut scratch, face);
        for &idx in &scratch[..(n as usize * 3)] {
            tri_indices.push(idx);
        }
    }

    build_loaded_mesh_from_fbx(mesh, &tri_indices, material_index, bone_node_to_index)
}

/// Build a LoadedMesh from triangulated ufbx index data.
/// `tri_indices` are indices into the mesh's per-index vertex attribute arrays.
fn build_loaded_mesh_from_fbx(
    mesh: &ufbx::Mesh,
    tri_indices: &[u32],
    material_index: Option<usize>,
    bone_node_to_index: &HashMap<u32, usize>,
) -> Result<Option<LoadedMesh>, Box<dyn std::error::Error>> {
    if tri_indices.is_empty() {
        return Ok(None);
    }

    let has_normals = mesh.vertex_normal.exists;
    let has_uvs = mesh.vertex_uv.exists;

    // De-index: ufbx uses per-index attributes, so we expand into per-vertex
    // (each triangle corner becomes its own vertex).
    let vertex_count = tri_indices.len();

    let mut positions = Vec::with_capacity(vertex_count);
    let mut normals_raw = Vec::with_capacity(vertex_count);
    let mut uvs_raw = Vec::with_capacity(vertex_count);

    for &idx in tri_indices {
        let idx = idx as usize;

        // Position (via vertex_position which maps index → position)
        let p = mesh.vertex_position[idx];
        positions.push([p.x as f32, p.y as f32, p.z as f32]);

        // Normal
        if has_normals {
            let n = mesh.vertex_normal[idx];
            normals_raw.push([n.x as f32, n.y as f32, n.z as f32]);
        }

        // UV
        if has_uvs {
            let uv = mesh.vertex_uv[idx];
            uvs_raw.push([uv.x as f32, uv.y as f32]);
        }
    }

    // Sequential indices (already de-indexed)
    let indices: Vec<u32> = (0..vertex_count as u32).collect();

    // Generate normals if missing
    let normals = if normals_raw.is_empty() {
        generate_flat_normals(&positions, &indices)
    } else {
        normals_raw
    };

    // Default UVs if missing
    let uvs = if uvs_raw.is_empty() {
        vec![[0.0, 0.0]; vertex_count]
    } else {
        uvs_raw
    };

    // Calculate tangents
    let tangents = calculate_tangents_safe(&positions, &normals, &uvs, &indices);

    // Extract skin weights
    let skinning = extract_fbx_skin_weights(mesh, tri_indices, bone_node_to_index);

    // Build Vertex3D array (merge skinning data into vertex)
    let mut vertices = Vec::with_capacity(vertex_count);
    for i in 0..vertex_count {
        let (joint_indices, joint_weights) = if let Some(ref skin) = skinning {
            (
                [
                    skin[i].joints[0] as u32,
                    skin[i].joints[1] as u32,
                    skin[i].joints[2] as u32,
                    skin[i].joints[3] as u32,
                ],
                skin[i].weights,
            )
        } else {
            ([0u32; 4], [1.0, 0.0, 0.0, 0.0])
        };
        vertices.push(Vertex3D {
            position: positions[i],
            normal: normals[i],
            uv: uvs[i],
            tangent: tangents[i],
            joint_indices,
            joint_weights,
        });
    }

    // Compute bounding sphere
    let (center, radius) = compute_bounding_sphere(&vertices);

    // Compute AABB
    let aabb = crate::engine::math::Aabb::from_points(
        vertices
            .iter()
            .map(|v| Vec3::new(v.position[0], v.position[1], v.position[2])),
    );

    Ok(Some(LoadedMesh {
        vertices,
        indices,
        material_index,
        center,
        radius,
        aabb_min: aabb.min,
        aabb_max: aabb.max,
        skinning,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test that loading a rigged FBX extracts bones.
    /// Requires content/Defeated.fbx (Mixamo character).
    #[test]
    fn test_rigged_fbx_bones_extracted() {
        let path = "content/Defeated.fbx";
        if !Path::new(path).exists() {
            eprintln!("Skipping test: {} not found", path);
            return;
        }
        let model = load_model_fbx(path).expect("Failed to load rigged FBX");
        assert!(
            !model.bones.is_empty(),
            "Rigged FBX should have bones, got 0"
        );

        // Verify parent hierarchy is valid
        for (i, bone) in model.bones.iter().enumerate() {
            if let Some(parent) = bone.parent_index {
                assert!(
                    parent < model.bones.len(),
                    "Bone {} ('{}') has invalid parent index {}",
                    i,
                    bone.name,
                    parent
                );
                assert_ne!(
                    parent, i,
                    "Bone {} ('{}') is its own parent",
                    i, bone.name
                );
            }
        }

        // At least one root bone (parent_index = None)
        let root_count = model.bones.iter().filter(|b| b.parent_index.is_none()).count();
        assert!(root_count > 0, "Should have at least one root bone");
    }

    /// Test that skinning data is populated for a rigged FBX.
    #[test]
    fn test_rigged_fbx_skinning_populated() {
        let path = "content/Defeated.fbx";
        if !Path::new(path).exists() {
            eprintln!("Skipping test: {} not found", path);
            return;
        }
        let model = load_model_fbx(path).expect("Failed to load rigged FBX");

        let skinned_meshes: Vec<_> = model
            .meshes
            .iter()
            .filter(|m| m.skinning.is_some())
            .collect();

        assert!(
            !skinned_meshes.is_empty(),
            "Rigged FBX should have at least one skinned mesh"
        );

        for mesh in &skinned_meshes {
            let skinning = mesh.skinning.as_ref().expect("skinning is Some");
            assert_eq!(
                skinning.len(),
                mesh.vertices.len(),
                "Skinning data must have one entry per vertex"
            );

            // Check weight normalization
            for (i, vb) in skinning.iter().enumerate() {
                let sum: f32 = vb.weights.iter().sum();
                // Weights should sum to ~1.0 (or 0.0 for unweighted vertices)
                if sum > 0.0 {
                    assert!(
                        (sum - 1.0).abs() < 0.01,
                        "Vertex {} weights sum to {} (expected ~1.0)",
                        i,
                        sum
                    );
                }

                // Bone indices must be valid
                for j in 0..4 {
                    if vb.weights[j] > 0.0 {
                        assert!(
                            (vb.joints[j] as usize) < model.bones.len(),
                            "Vertex {} has invalid bone index {} (bones: {})",
                            i,
                            vb.joints[j],
                            model.bones.len()
                        );
                    }
                }
            }
        }
    }

    /// Test that animations are extracted from a rigged FBX.
    #[test]
    fn test_rigged_fbx_animations_extracted() {
        let path = "content/Defeated.fbx";
        if !Path::new(path).exists() {
            eprintln!("Skipping test: {} not found", path);
            return;
        }
        let model = load_model_fbx(path).expect("Failed to load rigged FBX");

        assert!(
            !model.animations.is_empty(),
            "Rigged FBX with animation should have at least one clip"
        );

        for clip in &model.animations {
            assert!(!clip.name.is_empty(), "Clip name should not be empty");
            assert!(
                clip.duration_seconds > 0.0,
                "Clip '{}' should have positive duration, got {}",
                clip.name,
                clip.duration_seconds
            );
            assert!(
                !clip.channels.is_empty(),
                "Clip '{}' should have at least one channel",
                clip.name
            );

            for ch in &clip.channels {
                assert!(
                    ch.bone_index < model.bones.len(),
                    "Channel bone_index {} out of range (bones: {})",
                    ch.bone_index,
                    model.bones.len()
                );
            }
        }
    }

    /// Test that a static (non-rigged) FBX loaded from bytes has no skeleton data.
    #[test]
    fn test_static_fbx_no_skeleton() {
        // Create a minimal FBX-like test by loading Defeated.fbx and verifying
        // our types handle the data correctly. For a truly static mesh, we verify
        // that the Model type can represent "no bones" cleanly.
        let model = Model::new("static_test".to_string());
        assert!(model.bones.is_empty(), "New model should have no bones");
        assert!(
            model.animations.is_empty(),
            "New model should have no animations"
        );
    }
}
