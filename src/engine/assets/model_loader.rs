use std::path::Path;
use std::sync::Arc;
use crate::engine::rendering::rendering_3d::pipeline_3d::Vertex3D;
use vulkano::image::sampler::Sampler;
use vulkano::device::Device;
use vulkano::memory::allocator::StandardMemoryAllocator;
use vulkano::descriptor_set::allocator::StandardDescriptorSetAllocator;
use vulkano::pipeline::GraphicsPipeline;
use vulkano::image::view::ImageView;
use crate::engine::rendering::rendering_3d::pipeline_3d::create_pbr_material_descriptor_set;
use crate::engine::rendering::rendering_3d::material::*;
use glam::Vec3;
use gltf;

/// Represents a loaded mesh with vertex and index data
#[derive(Debug)]
pub struct LoadedMesh {
    pub vertices: Vec<Vertex3D>,
    pub indices: Vec<u32>,
    pub material_index: Option<usize>,
    /// Bounding sphere center in local/model space
    pub center: Vec3,
    /// Bounding sphere radius
    pub radius: f32,
    /// Local-space axis-aligned bounding box (computed once at load time).
    pub aabb_min: Vec3,
    pub aabb_max: Vec3,
}

/// Compute bounding sphere for a set of vertices
fn compute_bounding_sphere(vertices: &[Vertex3D]) -> (Vec3, f32) {
    if vertices.is_empty() {
        return (Vec3::ZERO, 0.0);
    }

    // Compute center as average of all positions
    let sum: Vec3 = vertices
        .iter()
        .map(|v| Vec3::new(v.position[0], v.position[1], v.position[2]))
        .sum();
    let center = sum / vertices.len() as f32;

    // Compute radius as max distance from center
    let radius = vertices
        .iter()
        .map(|v| {
            let pos = Vec3::new(v.position[0], v.position[1], v.position[2]);
            (pos - center).length()
        })
        .fold(0.0f32, f32::max);

    (center, radius)
}

/// Represents a complete 3D model with all meshes and textures
#[derive(Debug)]
pub struct Model {
    pub meshes: Vec<LoadedMesh>,
    pub name: String,
    pub textures: Vec<image::RgbaImage>, // Extracted textures from GLTF
}

impl Model {
    pub fn new(name: String) -> Self {
        Self {
            meshes: Vec::new(),
            name,
            textures: Vec::new(),
        }
    }
}

/// Loads a GLTF/GLB file and returns the parsed document
pub fn load_gltf(path: &str) -> Result<(gltf::Document, Vec<gltf::buffer::Data>, Vec<gltf::image::Data>), Box<dyn std::error::Error>> {
    let (document, buffers, images) = gltf::import(path)?;
    Ok((document, buffers, images))
}

/// Loads a GLTF/GLB from in-memory bytes (for pak file loading).
pub fn load_gltf_from_bytes(data: &[u8]) -> Result<(gltf::Document, Vec<gltf::buffer::Data>, Vec<gltf::image::Data>), Box<dyn std::error::Error>> {
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
pub fn load_model(path: &str) -> Result<Model, Box<dyn std::error::Error>> {
    crate::profile_function!();

    let (document, buffers, images) = {
        crate::profile_scope!("gltf_parse");
        load_gltf(path)?
    };

    let name = Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Unnamed")
        .to_string();

    build_model(name, document, buffers, images)
}

/// Loads a complete model from in-memory GLTF/GLB bytes.
pub fn load_model_from_bytes(data: &[u8], name: &str) -> Result<Model, Box<dyn std::error::Error>> {
    crate::profile_function!();

    let (document, buffers, images) = {
        crate::profile_scope!("gltf_parse");
        load_gltf_from_bytes(data)?
    };

    build_model(name.to_string(), document, buffers, images)
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
        for (_mesh_index, mesh) in document.meshes().enumerate() {
            for (_prim_index, primitive) in mesh.primitives().enumerate() {
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
    for (_i, image_data) in images.iter().enumerate() {
        // Convert to RgbaImage
        let rgba_image = match image_data.format {
            gltf::image::Format::R8G8B8A8 => {
                image::RgbaImage::from_raw(
                    image_data.width,
                    image_data.height,
                    image_data.pixels.clone(),
                ).ok_or("Failed to create RGBA image")?
            }
            gltf::image::Format::R8G8B8 => {
                // Convert RGB to RGBA
                let mut rgba_pixels = Vec::with_capacity(image_data.pixels.len() * 4 / 3);
                for chunk in image_data.pixels.chunks(3) {
                    rgba_pixels.push(chunk[0]); // R
                    rgba_pixels.push(chunk[1]); // G
                    rgba_pixels.push(chunk[2]); // B
                    rgba_pixels.push(255);      // A
                }
                image::RgbaImage::from_raw(
                    image_data.width,
                    image_data.height,
                    rgba_pixels,
                ).ok_or("Failed to create RGBA image from RGB")?
            }
            _ => {
                // Unsupported format, use default white texture
                image::RgbaImage::from_pixel(1, 1, image::Rgba([255, 255, 255, 255]))
            }
        };

        model.textures.push(rgba_image);
    }

    println!("✓ Model loaded: {} (meshes: {}, textures: {})", name, model.meshes.len(), model.textures.len());

    Ok(model)
}

/// Calculates tangent vectors for a mesh using vertex positions, normals, UVs, and indices
fn calculate_tangents(
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
    uvs: &[[f32; 2]],
    indices: &[u32],
) -> Vec<[f32; 4]> {
    let vertex_count = positions.len();
    let mut tangents = vec![[0.0f32; 3]; vertex_count];
    let mut bitangents = vec![[0.0f32; 3]; vertex_count];

    // Calculate tangents per triangle
    for i in (0..indices.len()).step_by(3) {
        let i0 = indices[i] as usize;
        let i1 = indices[i + 1] as usize;
        let i2 = indices[i + 2] as usize;

        let pos0 = positions[i0];
        let pos1 = positions[i1];
        let pos2 = positions[i2];

        let uv0 = uvs[i0];
        let uv1 = uvs[i1];
        let uv2 = uvs[i2];

        // Calculate edge vectors
        let edge1 = [pos1[0] - pos0[0], pos1[1] - pos0[1], pos1[2] - pos0[2]];
        let edge2 = [pos2[0] - pos0[0], pos2[1] - pos0[1], pos2[2] - pos0[2]];

        let delta_uv1 = [uv1[0] - uv0[0], uv1[1] - uv0[1]];
        let delta_uv2 = [uv2[0] - uv0[0], uv2[1] - uv0[1]];

        // Calculate tangent and bitangent
        let f = 1.0 / (delta_uv1[0] * delta_uv2[1] - delta_uv2[0] * delta_uv1[1]);

        let tangent = [
            f * (delta_uv2[1] * edge1[0] - delta_uv1[1] * edge2[0]),
            f * (delta_uv2[1] * edge1[1] - delta_uv1[1] * edge2[1]),
            f * (delta_uv2[1] * edge1[2] - delta_uv1[1] * edge2[2]),
        ];

        let bitangent = [
            f * (-delta_uv2[0] * edge1[0] + delta_uv1[0] * edge2[0]),
            f * (-delta_uv2[0] * edge1[1] + delta_uv1[0] * edge2[1]),
            f * (-delta_uv2[0] * edge1[2] + delta_uv1[0] * edge2[2]),
        ];

        // Accumulate tangents/bitangents for each vertex of the triangle
        for &idx in &[i0, i1, i2] {
            tangents[idx][0] += tangent[0];
            tangents[idx][1] += tangent[1];
            tangents[idx][2] += tangent[2];

            bitangents[idx][0] += bitangent[0];
            bitangents[idx][1] += bitangent[1];
            bitangents[idx][2] += bitangent[2];
        }
    }

    // Orthogonalize and normalize tangents using Gram-Schmidt
    let mut result = Vec::with_capacity(vertex_count);
    for i in 0..vertex_count {
        let n = normals[i];
        let t = tangents[i];
        let b = bitangents[i];

        // Gram-Schmidt orthogonalize: t' = normalize(t - n * dot(n, t))
        let dot_nt = n[0] * t[0] + n[1] * t[1] + n[2] * t[2];
        let t_orth = [
            t[0] - n[0] * dot_nt,
            t[1] - n[1] * dot_nt,
            t[2] - n[2] * dot_nt,
        ];

        // Normalize
        let len = (t_orth[0] * t_orth[0] + t_orth[1] * t_orth[1] + t_orth[2] * t_orth[2]).sqrt();
        let t_norm = if len > 0.0001 {
            [t_orth[0] / len, t_orth[1] / len, t_orth[2] / len]
        } else {
            [1.0, 0.0, 0.0] // Fallback tangent
        };

        // Calculate handedness (cross(n, t) dot b)
        let cross = [
            n[1] * t_norm[2] - n[2] * t_norm[1],
            n[2] * t_norm[0] - n[0] * t_norm[2],
            n[0] * t_norm[1] - n[1] * t_norm[0],
        ];
        let handedness = if cross[0] * b[0] + cross[1] * b[1] + cross[2] * b[2] < 0.0 {
            -1.0
        } else {
            1.0
        };

        result.push([t_norm[0], t_norm[1], t_norm[2], handedness]);
    }

    result
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

    // Combine into Vertex3D format
    let mut vertices = Vec::with_capacity(vertex_count);
    for i in 0..vertex_count {
        vertices.push(Vertex3D {
            position: positions[i],
            normal: normals[i],
            uv: uvs[i],
            tangent: tangents[i],
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
    })
}

/// Extracts texture image data from GLTF
pub fn extract_texture_from_gltf(
    document: &gltf::Document,
    buffers: &[gltf::buffer::Data],
    texture_index: usize,
) -> Result<image::RgbaImage, Box<dyn std::error::Error>> {
    let texture = document.textures().nth(texture_index)
        .ok_or("Texture index out of bounds")?;

    let image_data = texture.source();
    let image_source = image_data.source();

    match image_source {
        gltf::image::Source::Uri { uri, .. } => {
            // External file reference
            return Err(format!("External texture files not supported yet: {}", uri).into());
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
pub fn extract_material_from_gltf(
    document: &gltf::Document,
    _buffers: &[gltf::buffer::Data],
    images: &[gltf::image::Data],
    material_index: usize,
    device: Arc<Device>,
    allocator: Arc<StandardMemoryAllocator>,
    descriptor_set_allocator: Arc<StandardDescriptorSetAllocator>,
    pipeline: Arc<GraphicsPipeline>,
    sampler: Arc<Sampler>,
) -> Result<PbrMaterial, Box<dyn std::error::Error>> {
    let material = document.materials().nth(material_index)
        .ok_or("Material not found")?;

    let pbr = material.pbr_metallic_roughness();

    // Extract base color (albedo) texture
    let albedo_view = if let Some(info) = pbr.base_color_texture() {
        let texture = info.texture();
        let image_index = texture.source().index();
        load_gltf_image(&images[image_index], device.clone(), allocator.clone())?
    } else {
        // Default white texture
        create_default_texture(
            device.clone(),
            allocator.clone(),
            [255, 255, 255, 255],
        )?
    };

    // Extract normal map
    let normal_view = if let Some(normal_texture) = material.normal_texture() {
        let texture = normal_texture.texture();
        let image_index = texture.source().index();
        load_gltf_image(&images[image_index], device.clone(), allocator.clone())?
    } else {
        // Default normal map (pointing up: 128, 128, 255)
        create_default_texture(
            device.clone(),
            allocator.clone(),
            [128, 128, 255, 255],
        )?
    };

    // Extract metallic-roughness map
    let metallic_roughness_view = if let Some(info) = pbr.metallic_roughness_texture() {
        let texture = info.texture();
        let image_index = texture.source().index();
        load_gltf_image(&images[image_index], device.clone(), allocator.clone())?
    } else {
        // Default: non-metallic (B=0), half-rough (G=128)
        create_default_texture(
            device.clone(),
            allocator.clone(),
            [0, 128, 0, 255],
        )?
    };

    // Extract ambient occlusion map
    let ao_view = if let Some(ao_texture) = material.occlusion_texture() {
        let texture = ao_texture.texture();
        let image_index = texture.source().index();
        load_gltf_image(&images[image_index], device.clone(), allocator.clone())?
    } else {
        // Default: no occlusion (white = 255)
        create_default_texture(
            device.clone(),
            allocator.clone(),
            [255, 255, 255, 255],
        )?
    };

    // Create descriptor set
    let descriptor_set = create_pbr_material_descriptor_set(
        descriptor_set_allocator,
        pipeline,
        albedo_view.clone(),
        normal_view.clone(),
        metallic_roughness_view.clone(),
        ao_view.clone(),
        sampler,
    )?;

    Ok(PbrMaterial::new(
        albedo_view,
        normal_view,
        metallic_roughness_view,
        ao_view,
        descriptor_set,
    ))
}

/// Helper: Load GLTF image data to Vulkan texture
fn load_gltf_image(
    _image_data: &gltf::image::Data,
    device: Arc<Device>,
    allocator: Arc<StandardMemoryAllocator>,
) -> Result<Arc<ImageView>, Box<dyn std::error::Error>> {
    // Convert GLTF image format to Vulkan format
    // TODO: Implement texture upload from image_data.pixels
    // For now, create placeholder
    create_default_texture(
        device,
        allocator,
        [255, 0, 255, 255],  // Magenta = "texture not loaded"
    )
}