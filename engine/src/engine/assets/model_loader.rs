use crate::engine::rendering::rendering_3d::pipeline_3d::Vertex3D;
use glam::{Mat4, Quat, Vec3};
use std::path::Path;

// Re-export glTF-specific functions for backward compatibility
pub use super::model_loader_gltf::{
    extract_material_from_gltf, extract_texture_from_gltf, load_gltf, load_gltf_from_bytes,
    print_gltf_info,
};

// ──────────────────────────────────────────────────────────────
// Shared data types
// ──────────────────────────────────────────────────────────────

/// Per-vertex bone weights for skeletal animation.
#[derive(Debug, Clone)]
pub struct VertexBoneData {
    pub joints: [u16; 4],
    pub weights: [f32; 4],
}

/// A single bone in a skeleton hierarchy.
#[derive(Debug, Clone)]
pub struct BoneData {
    /// Human-readable bone name (e.g. "mixamorig:Hips").
    pub name: String,
    /// Index of the parent bone in `Model::bones`, or `None` for root bones.
    pub parent_index: Option<usize>,
    /// Inverse bind matrix — transforms from model space to bone-local space.
    pub inverse_bind_matrix: Mat4,
}

/// A single channel of animation targeting one bone.
#[derive(Debug, Clone)]
pub struct AnimationChannel {
    /// Index into `Model::bones`.
    pub bone_index: usize,
    /// Position keyframes: (time_seconds, translation).
    pub position_keys: Vec<(f32, Vec3)>,
    /// Rotation keyframes: (time_seconds, rotation).
    pub rotation_keys: Vec<(f32, Quat)>,
    /// Scale keyframes: (time_seconds, scale).
    pub scale_keys: Vec<(f32, Vec3)>,
}

/// A raw animation clip extracted from a source file (one per anim stack).
#[derive(Debug, Clone)]
pub struct RawAnimationClip {
    /// Clip name (e.g. "Take 001", "Idle", "Walk").
    pub name: String,
    /// Total duration in seconds.
    pub duration_seconds: f32,
    /// Per-bone animation channels.
    pub channels: Vec<AnimationChannel>,
}

/// Format-agnostic imported material — source of truth for material data.
#[derive(Debug, Clone)]
pub struct ImportedMaterial {
    /// Human-readable name (from file or generated).
    pub name: String,
    /// Base color / albedo texture (RGBA).
    pub albedo: Option<image::RgbaImage>,
    /// Normal map texture (RGBA, tangent-space).
    pub normal: Option<image::RgbaImage>,
    /// Metallic-roughness packed texture (G=roughness, B=metallic).
    pub metallic_roughness: Option<image::RgbaImage>,
    /// Ambient occlusion texture (R channel).
    pub ao: Option<image::RgbaImage>,
    /// Base color factor (linear RGBA multiply).
    pub base_color_factor: [f32; 4],
    /// Metallic factor [0..1].
    pub metallic_factor: f32,
    /// Roughness factor [0..1].
    pub roughness_factor: f32,
}

impl Default for ImportedMaterial {
    fn default() -> Self {
        Self {
            name: String::from("Default"),
            albedo: None,
            normal: None,
            metallic_roughness: None,
            ao: None,
            base_color_factor: [1.0, 1.0, 1.0, 1.0],
            metallic_factor: 0.0,
            roughness_factor: 0.5,
        }
    }
}

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
    /// Per-vertex skinning data (None for static meshes).
    pub skinning: Option<Vec<VertexBoneData>>,
}

/// Represents a complete 3D model with all meshes and textures
#[derive(Debug)]
pub struct Model {
    pub meshes: Vec<LoadedMesh>,
    pub name: String,
    /// Legacy texture list — kept for backward compat with existing GPU upload path.
    /// Derived from `materials` via `rebuild_legacy_textures()`.
    pub textures: Vec<image::RgbaImage>,
    /// Format-agnostic materials (source of truth).
    pub materials: Vec<ImportedMaterial>,
    /// Skeleton bone hierarchy (empty for static meshes).
    pub bones: Vec<BoneData>,
    /// Animation clips extracted from the source file (empty for static meshes).
    pub animations: Vec<RawAnimationClip>,
}

impl Model {
    pub fn new(name: String) -> Self {
        Self {
            meshes: Vec::new(),
            name,
            textures: Vec::new(),
            materials: Vec::new(),
            bones: Vec::new(),
            animations: Vec::new(),
        }
    }

    /// Rebuild the legacy `textures` vec from `materials` by collecting albedo
    /// textures in material order. This keeps the existing GPU upload path
    /// working while `materials` is the canonical source of truth.
    pub fn rebuild_legacy_textures(&mut self) {
        self.textures = self
            .materials
            .iter()
            .filter_map(|m| m.albedo.clone())
            .collect();
    }
}

// ──────────────────────────────────────────────────────────────
// Shared utilities (used by all format loaders)
// ──────────────────────────────────────────────────────────────

/// Compute bounding sphere for a set of vertices
pub(crate) fn compute_bounding_sphere(vertices: &[Vertex3D]) -> (Vec3, f32) {
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

/// Calculates tangent vectors for a mesh using vertex positions, normals, UVs, and indices.
/// Includes zero-determinant guard for degenerate triangles.
pub(crate) fn calculate_tangents(
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
        let det = delta_uv1[0] * delta_uv2[1] - delta_uv2[0] * delta_uv1[1];
        if det.abs() < 1e-8 {
            continue; // skip degenerate triangle
        }
        let f = 1.0 / det;

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

/// Safe tangent calculation that returns fallback [1,0,0,1] tangents when all UVs are [0,0].
pub(crate) fn calculate_tangents_safe(
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
    uvs: &[[f32; 2]],
    indices: &[u32],
) -> Vec<[f32; 4]> {
    // If all UVs are zero, tangent calculation is meaningless — return fallback
    let all_zero = uvs.iter().all(|uv| uv[0] == 0.0 && uv[1] == 0.0);
    if all_zero {
        return vec![[1.0, 0.0, 0.0, 1.0]; positions.len()];
    }
    calculate_tangents(positions, normals, uvs, indices)
}

/// Generate flat normals from triangle positions and indices.
/// Each triangle gets a single face normal assigned to all three vertices.
pub(crate) fn generate_flat_normals(
    positions: &[[f32; 3]],
    indices: &[u32],
) -> Vec<[f32; 3]> {
    let mut normals = vec![[0.0f32; 3]; positions.len()];

    for i in (0..indices.len()).step_by(3) {
        let i0 = indices[i] as usize;
        let i1 = indices[i + 1] as usize;
        let i2 = indices[i + 2] as usize;

        let p0 = Vec3::from(positions[i0]);
        let p1 = Vec3::from(positions[i1]);
        let p2 = Vec3::from(positions[i2]);

        let edge1 = p1 - p0;
        let edge2 = p2 - p0;
        let face_normal = edge1.cross(edge2);
        let n = if face_normal.length_squared() > 1e-12 {
            face_normal.normalize()
        } else {
            Vec3::Y // degenerate triangle fallback
        };

        for &idx in &[i0, i1, i2] {
            normals[idx][0] += n.x;
            normals[idx][1] += n.y;
            normals[idx][2] += n.z;
        }
    }

    // Normalize accumulated normals
    for n in &mut normals {
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        if len > 1e-8 {
            n[0] /= len;
            n[1] /= len;
            n[2] /= len;
        } else {
            *n = [0.0, 1.0, 0.0];
        }
    }

    normals
}

// ──────────────────────────────────────────────────────────────
// Format dispatch — single source of truth
// ──────────────────────────────────────────────────────────────

/// Determine model format from file extension.
fn model_extension(source_path: &str) -> &str {
    Path::new(source_path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
}

/// Load a 3D model from a filesystem path, dispatching by file extension.
pub fn load_model(source_path: &str) -> Result<Model, Box<dyn std::error::Error>> {
    match model_extension(source_path).to_ascii_lowercase().as_str() {
        "gltf" | "glb" => super::model_loader_gltf::load_model_gltf(source_path),
        "obj" => super::model_loader_obj::load_model_obj(source_path),
        "fbx" => super::model_loader_fbx::load_model_fbx(source_path),
        "mesh" => super::mesh_import::load_mesh_binary(Path::new(source_path)),
        ext => Err(format!("Unsupported model format: .{}", ext).into()),
    }
}

/// Load a 3D model from in-memory bytes, dispatching by the extension in `source_path`.
pub fn load_model_from_bytes(
    data: &[u8],
    source_path: &str,
) -> Result<Model, Box<dyn std::error::Error>> {
    match model_extension(source_path).to_ascii_lowercase().as_str() {
        "gltf" | "glb" => {
            super::model_loader_gltf::load_model_gltf_from_bytes(data, source_path)
        }
        "obj" => super::model_loader_obj::load_model_obj_from_bytes(data, source_path),
        "fbx" => super::model_loader_fbx::load_model_fbx_from_bytes(data, source_path),
        "mesh" => super::mesh_import::load_mesh_binary_from_bytes(data, source_path),
        ext => Err(format!("Unsupported model format: .{}", ext).into()),
    }
}
