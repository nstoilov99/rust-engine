use crate::engine::rendering::rendering_3d::material::*;
use crate::engine::rendering::rendering_3d::pipeline_3d::Vertex3D;
use glam::{Mat4, Quat, Vec3};
use gltf;
use std::collections::HashMap;
use std::sync::Arc;
use vulkano::command_buffer::allocator::StandardCommandBufferAllocator;
use vulkano::descriptor_set::allocator::StandardDescriptorSetAllocator;
use vulkano::device::{Device, Queue};
use vulkano::image::sampler::Sampler;
use vulkano::image::view::ImageView;
use vulkano::memory::allocator::StandardMemoryAllocator;
use vulkano::pipeline::PipelineLayout;

use super::model_loader::{
    calculate_tangents_safe, compute_bounding_sphere, AnimationChannel, BoneData, LoadedMesh,
    Model, RawAnimationClip, VertexBoneData,
};

/// Result type for GLTF loading operations.
type GltfResult = Result<
    (
        gltf::Document,
        Vec<gltf::buffer::Data>,
        Vec<gltf::image::Data>,
    ),
    Box<dyn std::error::Error>,
>;

/// Loads a GLTF/GLB file and returns the parsed document
pub fn load_gltf(path: &str) -> GltfResult {
    let (document, buffers, images) = gltf::import(path)?;
    Ok((document, buffers, images))
}

/// Loads a GLTF/GLB from in-memory bytes (for pak file loading).
pub fn load_gltf_from_bytes(data: &[u8]) -> GltfResult {
    let (document, buffers, images) = gltf::import_slice(data)?;
    Ok((document, buffers, images))
}

/// Prints detailed information about a GLTF file (for debugging)
pub fn print_gltf_info(document: &gltf::Document) {
    println!("\n=== GLTF Model Info ===");

    // Print scenes
    println!("\n📐 Scenes: {}", document.scenes().count());
    for (i, scene) in document.scenes().enumerate() {
        println!("  Scene {}: {:?}", i, scene.name());
        println!("    Nodes: {}", scene.nodes().count());
    }

    // Print meshes
    println!("\n🔷 Meshes: {}", document.meshes().count());
    for (i, mesh) in document.meshes().enumerate() {
        println!("  Mesh {}: {:?}", i, mesh.name());
        println!("    Primitives: {}", mesh.primitives().count());

        for (j, primitive) in mesh.primitives().enumerate() {
            println!("      Primitive {}: mode={:?}", j, primitive.mode());

            // Print attributes
            for (semantic, _accessor) in primitive.attributes() {
                println!("        - {:?}", semantic);
            }

            if let Some(_indices) = primitive.indices() {
                println!("        - Indexed (has indices)");
            }
        }
    }

    // Print materials
    println!("\n🎨 Materials: {}", document.materials().count());
    for (i, material) in document.materials().enumerate() {
        println!("  Material {}: {:?}", i, material.name());

        let pbr = material.pbr_metallic_roughness();
        println!("    Base color: {:?}", pbr.base_color_factor());

        if let Some(_texture) = pbr.base_color_texture() {
            println!("    Has base color texture");
        }
    }

    println!("\n========================\n");
}

/// Loads a complete model from GLTF file path (filesystem only).
pub fn load_model_gltf(path: &str) -> Result<Model, Box<dyn std::error::Error>> {
    crate::profile_function!();

    let (document, buffers, images) = {
        crate::profile_scope!("gltf_parse");
        load_gltf(path)?
    };

    let name = std::path::Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Unnamed")
        .to_string();

    build_model(name, document, buffers, images)
}

/// Loads a complete model from in-memory GLTF/GLB bytes.
pub fn load_model_gltf_from_bytes(
    data: &[u8],
    source_path: &str,
) -> Result<Model, Box<dyn std::error::Error>> {
    crate::profile_function!();

    let (document, buffers, images) = {
        crate::profile_scope!("gltf_parse");
        load_gltf_from_bytes(data)?
    };

    let name = std::path::Path::new(source_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Unnamed")
        .to_string();

    build_model(name, document, buffers, images)
}

fn build_model(
    name: String,
    document: gltf::Document,
    buffers: Vec<gltf::buffer::Data>,
    images: Vec<gltf::image::Data>,
) -> Result<Model, Box<dyn std::error::Error>> {
    let mut model = Model::new(name.clone());

    // Extract all meshes
    {
        crate::profile_scope!("vertex_processing");
        for mesh in document.meshes() {
            for primitive in mesh.primitives() {
                // Only handle triangle meshes
                if primitive.mode() != gltf::mesh::Mode::Triangles {
                    continue;
                }

                // Extract mesh data
                match extract_mesh_from_primitive(&primitive, &buffers) {
                    Ok(loaded_mesh) => {
                        model.meshes.push(loaded_mesh);
                    }
                    Err(e) => {
                        eprintln!("Failed to extract mesh primitive: {}", e);
                    }
                }
            }
        }
    }

    // Extract textures from images
    crate::profile_scope!("texture_extraction");
    for image_data in images.iter() {
        // Convert to RgbaImage
        let rgba_image = match image_data.format {
            gltf::image::Format::R8G8B8A8 => image::RgbaImage::from_raw(
                image_data.width,
                image_data.height,
                image_data.pixels.clone(),
            )
            .ok_or("Failed to create RGBA image")?,
            gltf::image::Format::R8G8B8 => {
                // Convert RGB to RGBA
                let mut rgba_pixels = Vec::with_capacity(image_data.pixels.len() * 4 / 3);
                for chunk in image_data.pixels.chunks(3) {
                    rgba_pixels.push(chunk[0]); // R
                    rgba_pixels.push(chunk[1]); // G
                    rgba_pixels.push(chunk[2]); // B
                    rgba_pixels.push(255); // A
                }
                image::RgbaImage::from_raw(image_data.width, image_data.height, rgba_pixels)
                    .ok_or("Failed to create RGBA image from RGB")?
            }
            _ => {
                // Unsupported format, use default white texture
                image::RgbaImage::from_pixel(1, 1, image::Rgba([255, 255, 255, 255]))
            }
        };

        model.textures.push(rgba_image);
    }

    // Extract skeleton from first skin
    if let Some(skin) = document.skins().next() {
        extract_skeleton(&skin, &buffers, &document, &mut model);
    }

    // Extract animations
    let has_bones = !model.bones.is_empty();
    if has_bones {
        extract_animations(&document, &buffers, &mut model);
    }

    println!(
        "✓ Model loaded: {} (meshes: {}, textures: {}, bones: {}, anims: {})",
        name,
        model.meshes.len(),
        model.textures.len(),
        model.bones.len(),
        model.animations.len(),
    );

    Ok(model)
}

/// Extract skeleton bones from a glTF skin.
fn extract_skeleton(
    skin: &gltf::Skin,
    buffers: &[gltf::buffer::Data],
    document: &gltf::Document,
    model: &mut Model,
) {
    let joints: Vec<gltf::Node> = skin.joints().collect();
    if joints.is_empty() {
        return;
    }

    // Map glTF node index → bone index for parent lookups
    let mut node_to_bone: HashMap<usize, usize> = HashMap::new();
    for (bone_idx, joint) in joints.iter().enumerate() {
        node_to_bone.insert(joint.index(), bone_idx);
    }

    // Read inverse bind matrices (one per joint)
    let ibms: Vec<Mat4> = if let Some(accessor) = skin.inverse_bind_matrices() {
        let reader = accessor.clone();
        let view = reader.view().unwrap();
        let buffer = &buffers[view.buffer().index()];
        let offset = view.offset() + accessor.offset();
        let count = accessor.count();

        let mut matrices = Vec::with_capacity(count);
        for i in 0..count {
            let start = offset + i * 64; // 16 floats * 4 bytes
            let floats: [f32; 16] = {
                let bytes = &buffer[start..start + 64];
                let mut arr = [0f32; 16];
                for (j, chunk) in bytes.chunks_exact(4).enumerate() {
                    arr[j] = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                }
                arr
            };
            matrices.push(Mat4::from_cols_array(&floats));
        }
        matrices
    } else {
        vec![Mat4::IDENTITY; joints.len()]
    };

    // Build node parent map from the scene hierarchy
    let mut node_parent: HashMap<usize, usize> = HashMap::new();
    fn walk_nodes(node: &gltf::Node, parent_map: &mut HashMap<usize, usize>) {
        for child in node.children() {
            parent_map.insert(child.index(), node.index());
            walk_nodes(&child, parent_map);
        }
    }
    for scene in document.scenes() {
        for node in scene.nodes() {
            walk_nodes(&node, &mut node_parent);
        }
    }

    // Build bone data
    for (bone_idx, joint) in joints.iter().enumerate() {
        let parent_index = node_parent
            .get(&joint.index())
            .and_then(|parent_node| node_to_bone.get(parent_node).copied());

        model.bones.push(BoneData {
            name: joint.name().unwrap_or("Unnamed").to_string(),
            parent_index,
            inverse_bind_matrix: ibms.get(bone_idx).copied().unwrap_or(Mat4::IDENTITY),
        });
    }
}

/// Extract animations from glTF, mapping channels to bone indices.
fn extract_animations(
    document: &gltf::Document,
    buffers: &[gltf::buffer::Data],
    model: &mut Model,
) {
    // Build node-index → bone-index map using the first skin's joints.
    let node_to_bone: HashMap<usize, usize> = if let Some(skin) = document.skins().next() {
        skin.joints()
            .enumerate()
            .map(|(bone_idx, joint)| (joint.index(), bone_idx))
            .collect()
    } else {
        return;
    };

    for anim in document.animations() {
        let name = anim
            .name()
            .unwrap_or("Unnamed")
            .to_string();

        // Group channels by target node
        let mut bone_channels: HashMap<usize, AnimationChannel> = HashMap::new();
        let mut max_time: f32 = 0.0;

        for channel in anim.channels() {
            let target_node = channel.target().node().index();
            let bone_idx = match node_to_bone.get(&target_node) {
                Some(&idx) => idx,
                None => continue, // Channel targets a non-bone node
            };

            let reader = channel.reader(|buffer| Some(&buffers[buffer.index()]));
            let timestamps: Vec<f32> = reader
                .read_inputs()
                .map(|iter| iter.collect())
                .unwrap_or_default();

            if let Some(&last) = timestamps.last() {
                max_time = max_time.max(last);
            }

            let entry = bone_channels.entry(bone_idx).or_insert(AnimationChannel {
                bone_index: bone_idx,
                position_keys: Vec::new(),
                rotation_keys: Vec::new(),
                scale_keys: Vec::new(),
            });

            if let Some(outputs) = reader.read_outputs() {
                match (channel.target().property(), outputs) {
                    (
                        gltf::animation::Property::Translation,
                        gltf::animation::util::ReadOutputs::Translations(translations),
                    ) => {
                        for (t, val) in timestamps.iter().zip(translations) {
                            entry
                                .position_keys
                                .push((*t, Vec3::new(val[0], val[1], val[2])));
                        }
                    }
                    (
                        gltf::animation::Property::Rotation,
                        gltf::animation::util::ReadOutputs::Rotations(rotations),
                    ) => {
                        for (t, val) in timestamps.iter().zip(rotations.into_f32()) {
                            entry.rotation_keys.push((
                                *t,
                                Quat::from_xyzw(val[0], val[1], val[2], val[3]),
                            ));
                        }
                    }
                    (
                        gltf::animation::Property::Scale,
                        gltf::animation::util::ReadOutputs::Scales(scales),
                    ) => {
                        for (t, val) in timestamps.iter().zip(scales) {
                            entry
                                .scale_keys
                                .push((*t, Vec3::new(val[0], val[1], val[2])));
                        }
                    }
                    _ => {} // Morph targets or mismatched output type
                }
            }
        }

        if !bone_channels.is_empty() {
            model.animations.push(RawAnimationClip {
                name,
                duration_seconds: max_time,
                channels: bone_channels.into_values().collect(),
            });
        }
    }
}

/// Extract per-vertex skinning data (joint indices + weights) from a glTF primitive.
fn extract_skinning(
    primitive: &gltf::Primitive,
    buffers: &[gltf::buffer::Data],
    vertex_count: usize,
) -> Option<Vec<VertexBoneData>> {
    let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));

    let joints: Vec<[u16; 4]> = reader
        .read_joints(0)?
        .into_u16()
        .collect();

    let weights: Vec<[f32; 4]> = reader
        .read_weights(0)?
        .into_f32()
        .collect();

    if joints.len() != vertex_count || weights.len() != vertex_count {
        return None;
    }

    Some(
        joints
            .into_iter()
            .zip(weights)
            .map(|(j, w)| VertexBoneData {
                joints: j,
                weights: w,
            })
            .collect(),
    )
}

/// Calculates tangent vectors for a mesh (used internally by glTF loader).
fn calculate_tangents(
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
    uvs: &[[f32; 2]],
    indices: &[u32],
) -> Vec<[f32; 4]> {
    calculate_tangents_safe(positions, normals, uvs, indices)
}

/// Extracts mesh data from a GLTF primitive
fn extract_mesh_from_primitive(
    primitive: &gltf::Primitive,
    buffers: &[gltf::buffer::Data],
) -> Result<LoadedMesh, Box<dyn std::error::Error>> {
    // Get accessors for vertex attributes
    let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));

    // Read positions (required)
    let positions = reader
        .read_positions()
        .ok_or("Missing position attribute")?
        .collect::<Vec<[f32; 3]>>();

    // Read normals (required for lighting)
    let normals = reader
        .read_normals()
        .ok_or("Missing normal attribute")?
        .collect::<Vec<[f32; 3]>>();

    // Read UVs (optional, default to [0, 0])
    let uvs = if let Some(uv_iter) = reader.read_tex_coords(0) {
        uv_iter.into_f32().collect::<Vec<[f32; 2]>>()
    } else {
        vec![[0.0, 0.0]; positions.len()]
    };

    // Ensure all arrays have same length
    let vertex_count = positions.len();
    if normals.len() != vertex_count || uvs.len() != vertex_count {
        return Err("Vertex attribute count mismatch".into());
    }

    // Read indices first (needed for tangent calculation)
    let indices = if let Some(indices_reader) = reader.read_indices() {
        indices_reader.into_u32().collect::<Vec<u32>>()
    } else {
        // No indices - generate them (0, 1, 2, 3, ...)
        (0..vertex_count as u32).collect()
    };

    // Read tangents (optional, will be calculated if missing)
    let tangents = if let Some(tangent_iter) = reader.read_tangents() {
        tangent_iter.collect::<Vec<[f32; 4]>>()
    } else {
        // Calculate tangents from geometry
        calculate_tangents(&positions, &normals, &uvs, &indices)
    };

    // Extract per-vertex skinning data (joints + weights)
    let skinning_data = extract_skinning(primitive, buffers, vertex_count);

    // Combine into Vertex3D format
    let mut vertices = Vec::with_capacity(vertex_count);
    for i in 0..vertex_count {
        let (joint_indices, joint_weights) = if let Some(ref skin) = skinning_data {
            let j = skin[i].joints;
            ([j[0] as u32, j[1] as u32, j[2] as u32, j[3] as u32], skin[i].weights)
        } else {
            ([0u32, 0, 0, 0], [1.0f32, 0.0, 0.0, 0.0])
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

    // Get material index
    let material_index = primitive.material().index();

    // Compute bounding sphere for frustum culling
    let (center, radius) = compute_bounding_sphere(&vertices);

    // Compute local-space AABB (once, at load time — never recomputed at runtime).
    let aabb = crate::engine::math::Aabb::from_points(
        vertices
            .iter()
            .map(|v| Vec3::new(v.position[0], v.position[1], v.position[2])),
    );

    Ok(LoadedMesh {
        vertices,
        indices,
        material_index,
        center,
        radius,
        aabb_min: aabb.min,
        aabb_max: aabb.max,
        skinning: skinning_data,
    })
}

/// Extracts texture image data from GLTF
pub fn extract_texture_from_gltf(
    document: &gltf::Document,
    buffers: &[gltf::buffer::Data],
    texture_index: usize,
) -> Result<image::RgbaImage, Box<dyn std::error::Error>> {
    let texture = document
        .textures()
        .nth(texture_index)
        .ok_or("Texture index out of bounds")?;

    let image_data = texture.source();
    let image_source = image_data.source();

    match image_source {
        gltf::image::Source::Uri { uri, .. } => {
            // External file reference
            Err(format!("External texture files not supported yet: {}", uri).into())
        }
        gltf::image::Source::View { view, .. } => {
            // Embedded in buffer
            let buffer = &buffers[view.buffer().index()];
            let start = view.offset();
            let end = start + view.length();
            let image_bytes = &buffer[start..end];

            // Decode image (PNG, JPEG, etc.)
            let img = image::load_from_memory(image_bytes)?;
            Ok(img.to_rgba8())
        }
    }
}

/// Extracts PBR material from GLTF
#[allow(clippy::too_many_arguments)]
pub fn extract_material_from_gltf(
    document: &gltf::Document,
    _buffers: &[gltf::buffer::Data],
    images: &[gltf::image::Data],
    material_index: usize,
    device: Arc<Device>,
    allocator: Arc<StandardMemoryAllocator>,
    command_buffer_allocator: Arc<StandardCommandBufferAllocator>,
    queue: Arc<Queue>,
    descriptor_set_allocator: Arc<StandardDescriptorSetAllocator>,
    geom_pipeline_layout: Arc<PipelineLayout>,
    sampler: Arc<Sampler>,
) -> Result<PbrMaterial, Box<dyn std::error::Error>> {
    let material = document
        .materials()
        .nth(material_index)
        .ok_or("Material not found")?;

    let pbr = material.pbr_metallic_roughness();

    // Extract base color (albedo) texture
    let albedo_view = if let Some(info) = pbr.base_color_texture() {
        let texture = info.texture();
        let image_index = texture.source().index();
        load_gltf_image(&images[image_index], device.clone(), allocator.clone(), command_buffer_allocator.clone(), queue.clone())?
    } else {
        create_default_texture(device.clone(), allocator.clone(), command_buffer_allocator.clone(), queue.clone(), DEFAULT_ALBEDO_RGBA)?
    };

    // Extract normal map
    let normal_view = if let Some(normal_texture) = material.normal_texture() {
        let texture = normal_texture.texture();
        let image_index = texture.source().index();
        load_gltf_image(&images[image_index], device.clone(), allocator.clone(), command_buffer_allocator.clone(), queue.clone())?
    } else {
        create_default_texture_with_format(allocator.clone(), command_buffer_allocator.clone(), queue.clone(), DEFAULT_NORMAL_RGBA, vulkano::format::Format::R8G8B8A8_UNORM)?
    };

    // Extract metallic-roughness map
    let metallic_roughness_view = if let Some(info) = pbr.metallic_roughness_texture() {
        let texture = info.texture();
        let image_index = texture.source().index();
        load_gltf_image(&images[image_index], device.clone(), allocator.clone(), command_buffer_allocator.clone(), queue.clone())?
    } else {
        create_default_texture_with_format(allocator.clone(), command_buffer_allocator.clone(), queue.clone(), DEFAULT_METALLIC_ROUGHNESS_RGBA, vulkano::format::Format::R8G8B8A8_UNORM)?
    };

    // Extract ambient occlusion map
    let ao_view = if let Some(ao_texture) = material.occlusion_texture() {
        let texture = ao_texture.texture();
        let image_index = texture.source().index();
        load_gltf_image(&images[image_index], device.clone(), allocator.clone(), command_buffer_allocator.clone(), queue.clone())?
    } else {
        create_default_texture_with_format(allocator.clone(), command_buffer_allocator.clone(), queue.clone(), DEFAULT_AO_RGBA, vulkano::format::Format::R8G8B8A8_UNORM)?
    };

    let base_color = pbr.base_color_factor();
    let metallic = pbr.metallic_factor();
    let roughness = pbr.roughness_factor();
    let emissive = material.emissive_factor();

    PbrMaterial::new(
        albedo_view,
        normal_view,
        metallic_roughness_view,
        ao_view,
        sampler,
        base_color,
        metallic,
        roughness,
        emissive,
        allocator,
        descriptor_set_allocator,
        geom_pipeline_layout,
    )
}

/// Helper: Load GLTF image data to Vulkan texture
fn load_gltf_image(
    _image_data: &gltf::image::Data,
    device: Arc<Device>,
    allocator: Arc<StandardMemoryAllocator>,
    command_buffer_allocator: Arc<StandardCommandBufferAllocator>,
    queue: Arc<Queue>,
) -> Result<Arc<ImageView>, Box<dyn std::error::Error>> {
    // Convert GLTF image format to Vulkan format
    // TODO: Implement texture upload from image_data.pixels
    // For now, create placeholder
    create_default_texture(
        device,
        allocator,
        command_buffer_allocator,
        queue,
        [255, 0, 255, 255], // Magenta = "texture not loaded"
    )
}
