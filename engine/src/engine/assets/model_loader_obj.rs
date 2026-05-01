use crate::engine::rendering::rendering_3d::pipeline_3d::Vertex3D;
use glam::Vec3;
use std::path::Path;

use super::model_loader::{
    calculate_tangents_safe, compute_bounding_sphere, generate_flat_normals, ImportedMaterial,
    LoadedMesh, Model,
};

/// Load OBJ model from filesystem path.
pub fn load_model_obj(source_path: &str) -> Result<Model, Box<dyn std::error::Error>> {
    crate::profile_function!();

    let (models, materials_result) = tobj::load_obj(source_path, &tobj::GPU_LOAD_OPTIONS)
        .map_err(|e| format!("Failed to load OBJ '{}': {:?}", source_path, e))?;

    let materials = materials_result.unwrap_or_default();

    let name = Path::new(source_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Unnamed")
        .to_string();

    build_model_from_tobj(name, &models, &materials)
}

/// Load OBJ model from in-memory bytes.
pub fn load_model_obj_from_bytes(
    data: &[u8],
    source_path: &str,
) -> Result<Model, Box<dyn std::error::Error>> {
    crate::profile_function!();

    let mut reader = std::io::BufReader::new(data);
    let (models, materials_result) =
        tobj::load_obj_buf(&mut reader, &tobj::GPU_LOAD_OPTIONS, |mtl_path| {
            // No MTL file available when loading from bytes — return empty materials
            let _ = mtl_path;
            Ok((Vec::new(), Default::default()))
        })
        .map_err(|e| format!("Failed to load OBJ from bytes '{}': {:?}", source_path, e))?;

    let materials = materials_result.unwrap_or_default();

    let name = Path::new(source_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Unnamed")
        .to_string();

    build_model_from_tobj(name, &models, &materials)
}

fn build_model_from_tobj(
    name: String,
    models: &[tobj::Model],
    materials: &[tobj::Material],
) -> Result<Model, Box<dyn std::error::Error>> {
    let mut model = Model::new(name.clone());

    for tobj_model in models {
        let mesh = &tobj_model.mesh;
        if mesh.positions.is_empty() || mesh.indices.is_empty() {
            continue;
        }

        let loaded_mesh = convert_tobj_mesh(mesh)?;
        model.meshes.push(loaded_mesh);
    }

    // Convert tobj materials to ImportedMaterial
    for mat in materials {
        let diffuse = mat.diffuse.unwrap_or([0.8, 0.8, 0.8]);
        model.materials.push(ImportedMaterial {
            name: mat.name.clone(),
            albedo: None, // OBJ textures are file references — not embedded
            normal: None,
            metallic_roughness: None,
            ao: None,
            base_color_factor: [diffuse[0], diffuse[1], diffuse[2], 1.0],
            metallic_factor: 0.0,
            roughness_factor: 0.5,
            emissive_factor: [0.0, 0.0, 0.0],
        });
    }

    // Rebuild legacy textures from materials (will be empty for OBJ since no embedded textures)
    model.rebuild_legacy_textures();

    println!(
        "✓ Model loaded (OBJ): {} (meshes: {}, materials: {})",
        name,
        model.meshes.len(),
        model.materials.len()
    );

    Ok(model)
}

fn convert_tobj_mesh(mesh: &tobj::Mesh) -> Result<LoadedMesh, Box<dyn std::error::Error>> {
    let vertex_count = mesh.positions.len() / 3;

    // Extract positions as [f32; 3] arrays
    let positions: Vec<[f32; 3]> = (0..vertex_count)
        .map(|i| {
            [
                mesh.positions[i * 3],
                mesh.positions[i * 3 + 1],
                mesh.positions[i * 3 + 2],
            ]
        })
        .collect();

    let indices = mesh.indices.clone();

    // Extract or generate normals
    let normals: Vec<[f32; 3]> = if mesh.normals.is_empty() {
        // Generate flat normals from geometry
        generate_flat_normals(&positions, &indices)
    } else {
        (0..vertex_count)
            .map(|i| {
                [
                    mesh.normals[i * 3],
                    mesh.normals[i * 3 + 1],
                    mesh.normals[i * 3 + 2],
                ]
            })
            .collect()
    };

    // Extract UVs (default to [0,0] if missing)
    let uvs: Vec<[f32; 2]> = if mesh.texcoords.is_empty() {
        vec![[0.0, 0.0]; vertex_count]
    } else {
        (0..vertex_count)
            .map(|i| [mesh.texcoords[i * 2], mesh.texcoords[i * 2 + 1]])
            .collect()
    };

    // Calculate tangents safely (handles zero-UV case)
    let tangents = calculate_tangents_safe(&positions, &normals, &uvs, &indices);

    // Build Vertex3D array
    let mut vertices = Vec::with_capacity(vertex_count);
    for i in 0..vertex_count {
        vertices.push(Vertex3D {
            position: positions[i],
            normal: normals[i],
            uv: uvs[i],
            tangent: tangents[i],
            ..Default::default()
        });
    }

    // Get material index
    let material_index = mesh.material_id;

    // Compute bounding sphere
    let (center, radius) = compute_bounding_sphere(&vertices);

    // Compute AABB
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
        skinning: None,
    })
}
