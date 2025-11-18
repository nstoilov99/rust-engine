use std::path::Path;
use crate::engine::pipeline::Vertex3D;
use glam::Vec3;
use gltf;

/// Represents a loaded mesh with vertex and index data
pub struct LoadedMesh {
    pub vertices: Vec<Vertex3D>,
    pub indices: Vec<u32>,
    pub material_index: Option<usize>,
}

/// Represents a complete 3D model with all meshes
pub struct Model {
    pub meshes: Vec<LoadedMesh>,
    pub name: String,
}

impl Model {
    pub fn new(name: String) -> Self {
        Self {
            meshes: Vec::new(),
            name,
        }
    }
}

/// Loads a GLTF/GLB file and returns the parsed document
pub fn load_gltf(path: &str) -> Result<(gltf::Document, Vec<gltf::buffer::Data>), Box<dyn std::error::Error>> {
    println!("📦 Loading GLTF model: {}", path);

    // Load GLTF file (handles both .gltf and .glb)
    let (document, buffers, _images) = gltf::import(path)?;

    println!("✅ GLTF loaded successfully!");
    println!("   Meshes: {}", document.meshes().count());
    println!("   Materials: {}", document.materials().count());
    println!("   Textures: {}", document.textures().count());
    println!("   Nodes: {}", document.nodes().count());

    Ok((document, buffers))
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

/// Loads a complete model from GLTF file
pub fn load_model(path: &str) -> Result<Model, Box<dyn std::error::Error>> {
    let (document, buffers) = load_gltf(path)?;

    // Get model name from file path
    let name = Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Unnamed")
        .to_string();

    let mut model = Model::new(name.clone());

    println!("📥 Extracting meshes from '{}'...", name);

    // Extract all meshes
    for (mesh_index, mesh) in document.meshes().enumerate() {
        println!("\n  Mesh {}: {:?}", mesh_index, mesh.name());

        for (prim_index, primitive) in mesh.primitives().enumerate() {
            println!("    Primitive {}:", prim_index);

            // Only handle triangle meshes
            if primitive.mode() != gltf::mesh::Mode::Triangles {
                println!("      ⚠️  Skipping non-triangle primitive");
                continue;
            }

            // Extract mesh data
            match extract_mesh_from_primitive(&primitive, &buffers) {
                Ok(loaded_mesh) => {
                    model.meshes.push(loaded_mesh);
                }
                Err(e) => {
                    eprintln!("      ❌ Failed to extract primitive: {}", e);
                }
            }
        }
    }

    println!("\n✅ Model loaded: {} meshes extracted\n", model.meshes.len());

    Ok(model)
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

    // Combine into Vertex3D format
    let mut vertices = Vec::with_capacity(vertex_count);
    for i in 0..vertex_count {
        vertices.push(Vertex3D {
            position: positions[i],
            normal: normals[i],
            uv: uvs[i],
        });
    }

    // Read indices (convert to u32)
    let indices = if let Some(indices_reader) = reader.read_indices() {
        indices_reader.into_u32().collect::<Vec<u32>>()
    } else {
        // No indices - generate them (0, 1, 2, 3, ...)
        (0..vertex_count as u32).collect()
    };

    // Get material index
    let material_index = primitive.material().index();

    println!("  ✓ Extracted mesh: {} vertices, {} indices", vertices.len(), indices.len());

    Ok(LoadedMesh {
        vertices,
        indices,
        material_index,
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