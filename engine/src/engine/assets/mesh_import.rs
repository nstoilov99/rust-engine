//! Native `.mesh` binary format — fast-loading preprocessed mesh data.
//!
//! Source model files (FBX, OBJ, glTF) are converted to `.mesh` at import time.
//! Runtime loads `.mesh` directly with near-zero parsing overhead.
//!
//! A `.mesh.ron` sidecar stores import settings for re-import.
//!
//! `.anim` binary files store extracted animation clips separately.

use super::model_loader::{
    AnimationChannel, BoneData, ImportedMaterial, LoadedMesh, Model, RawAnimationClip,
    VertexBoneData,
};
use crate::engine::rendering::rendering_3d::pipeline_3d::Vertex3D;
use glam::{Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};

// ──────────────────────────────────────────────────────────────
// Constants
// ──────────────────────────────────────────────────────────────

const MESH_MAGIC: &[u8; 4] = b"RMSH";
const MESH_VERSION: u32 = 2;

/// Size of the file header in bytes.
const HEADER_SIZE: usize = 24;
/// Size of one mesh descriptor in bytes.
const MESH_DESC_SIZE: usize = 68;

/// Flag bit: model contains skeleton (bones + optional per-mesh skinning).
const FLAG_HAS_BONES: u32 = 1;

const ANIM_MAGIC: &[u8; 4] = b"RANM";
const ANIM_VERSION: u32 = 1;

// ──────────────────────────────────────────────────────────────
// Import settings
// ──────────────────────────────────────────────────────────────

/// Axis convention for model coordinate system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UpAxis {
    YUp,
    ZUp,
}

/// Settings that control how a source model is converted to `.mesh`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeshImportSettings {
    /// Uniform scale multiplier applied to all vertex positions and bounds.
    pub scale: f32,
    /// Generate tangent vectors (required for normal mapping).
    pub generate_tangents: bool,
    /// Embed material data (textures + factors) in the `.mesh` binary.
    pub import_materials: bool,
    /// Flip V coordinate (1-v). Some formats need this.
    pub flip_uvs: bool,
    /// Source coordinate system up axis.
    pub up_axis: UpAxis,
    /// Import animation clips as separate `.anim` file.
    pub import_animations: bool,
}

impl Default for MeshImportSettings {
    fn default() -> Self {
        Self {
            scale: 1.0,
            generate_tangents: true,
            import_materials: true,
            flip_uvs: false,
            up_axis: UpAxis::YUp,
            import_animations: true,
        }
    }
}

/// A material slot in a mesh — maps a submesh to a `.material.ron` asset.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaterialSlot {
    /// Display name for this slot (e.g. "Skin", "Eyes").
    pub name: String,
    /// Content-relative path to the `.material.ron` file (empty = unassigned).
    #[serde(default)]
    pub material_path: String,
}

/// Sidecar metadata stored as `.mesh.ron` alongside the binary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeshImportMeta {
    /// Relative path to the original source file.
    pub source: String,
    /// Settings used for this import.
    pub settings: MeshImportSettings,
    /// CRC32 of the source file at import time (for change detection).
    pub source_hash: u32,
    /// Per-submesh material assignments.
    #[serde(default)]
    pub material_slots: Vec<MaterialSlot>,
}

/// Result of an import operation.
pub struct ImportResult {
    /// Whether a `.anim` file was written.
    pub anim_written: bool,
    /// Number of bones in the skeleton.
    pub bone_count: usize,
    /// Number of animation clips written.
    pub anim_clip_count: usize,
    /// Number of `.material.ron` files written.
    pub material_count: usize,
}

/// Standalone material definition, serialized to `.material.ron`.
///
/// Texture fields are relative paths (sibling files next to the `.material.ron`).
/// Empty string means no texture.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaterialDefinition {
    pub name: String,
    pub base_color_factor: [f32; 4],
    pub metallic_factor: f32,
    pub roughness_factor: f32,
    #[serde(default)]
    pub emissive_factor: [f32; 3],
    pub albedo_texture: String,
    pub normal_texture: String,
    pub metallic_roughness_texture: String,
    pub ao_texture: String,
}

/// Load a `MaterialDefinition` from a `.material.ron` file.
pub fn load_material_ron(
    path: &Path,
) -> Result<MaterialDefinition, Box<dyn std::error::Error>> {
    let text = std::fs::read_to_string(path)?;
    let def: MaterialDefinition = ron::from_str(&text)?;
    Ok(def)
}

// ──────────────────────────────────────────────────────────────
// Import pipeline (source → .mesh + .anim)
// ──────────────────────────────────────────────────────────────

/// Full import pipeline: load source model, apply settings, write `.mesh` + `.anim` + sidecar.
pub fn import_model_to_mesh(
    source_path: &Path,
    output_path: &Path,
    settings: &MeshImportSettings,
) -> Result<ImportResult, Box<dyn std::error::Error>> {
    // 1. Load source model via existing dispatch
    let mut model = super::model_loader::load_model(&source_path.to_string_lossy())?;

    // 2. Apply import settings (scale, flip UVs, axis conversion)
    //    FBX and glTF loaders already convert to Y-up internally, so skip
    //    the user's axis conversion to prevent double-conversion.
    let ext = source_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let mut effective_settings = settings.clone();
    if matches!(ext.as_str(), "fbx" | "gltf" | "glb") && settings.up_axis == UpAxis::ZUp {
        log::info!(
            "Import: overriding up_axis to YUp for .{} (loader already converts to Y-up)",
            ext
        );
        effective_settings.up_axis = UpAxis::YUp;
    }
    apply_import_settings(&mut model, &effective_settings);

    // 3. Strip materials if not requested
    if !settings.import_materials {
        model.materials.clear();
        for mesh in &mut model.meshes {
            mesh.material_index = None;
        }
    }

    // 4. Write .mesh binary (v2: includes bones + skinning)
    write_mesh_binary(output_path, &model)?;

    // 5. Write .anim file if model has animations and setting is enabled
    let anim_written = settings.import_animations && !model.animations.is_empty();
    let anim_clip_count = if anim_written {
        let anim_path = output_path.with_extension("anim");
        let bone_names: Vec<String> = model.bones.iter().map(|b| b.name.clone()).collect();
        write_anim_binary(&anim_path, &model.animations, &bone_names)?;
        model.animations.len()
    } else {
        0
    };

    // 6. Export .material.ron files for each material with textures
    let material_count = if settings.import_materials && !model.materials.is_empty() {
        export_materials(output_path, &model.materials)?
    } else {
        0
    };

    // 7. Build material slots from exported materials
    let material_slots = if settings.import_materials {
        let stem = output_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("model");
        model
            .materials
            .iter()
            .map(|mat| {
                let safe_name = sanitize_filename(&mat.name);
                MaterialSlot {
                    name: mat.name.clone(),
                    material_path: format!("{}_{}.material.ron", stem, safe_name),
                }
            })
            .collect()
    } else {
        vec![]
    };

    // 8. Write sidecar .mesh.ron
    let source_data = std::fs::read(source_path)?;
    let source_hash = crc32_hash(&source_data);

    let meta = MeshImportMeta {
        source: source_path.to_string_lossy().to_string(),
        settings: effective_settings.clone(),
        source_hash,
        material_slots,
    };
    write_mesh_sidecar(output_path, &meta)?;

    Ok(ImportResult {
        anim_written,
        bone_count: model.bones.len(),
        anim_clip_count,
        material_count,
    })
}

/// Apply import settings to a loaded Model in-place.
pub fn apply_import_settings(model: &mut Model, settings: &MeshImportSettings) {
    let need_scale = (settings.scale - 1.0).abs() > f32::EPSILON;
    let need_axis = settings.up_axis == UpAxis::ZUp;

    for mesh in &mut model.meshes {
        for v in &mut mesh.vertices {
            // Axis conversion: Z-up → Y-up (swap Y and Z, negate new Z)
            if need_axis {
                let y = v.position[1];
                v.position[1] = v.position[2];
                v.position[2] = -y;

                let ny = v.normal[1];
                v.normal[1] = v.normal[2];
                v.normal[2] = -ny;

                let ty = v.tangent[1];
                v.tangent[1] = v.tangent[2];
                v.tangent[2] = -ty;
            }

            // Scale
            if need_scale {
                v.position[0] *= settings.scale;
                v.position[1] *= settings.scale;
                v.position[2] *= settings.scale;
            }

            // Flip UVs
            if settings.flip_uvs {
                v.uv[1] = 1.0 - v.uv[1];
            }
        }

        // Recompute bounds after transform
        if need_scale || need_axis {
            let (center, radius) =
                super::model_loader::compute_bounding_sphere(&mesh.vertices);
            mesh.center = center;
            mesh.radius = radius;

            // Recompute AABB
            let mut aabb_min = Vec3::splat(f32::MAX);
            let mut aabb_max = Vec3::splat(f32::MIN);
            for v in &mesh.vertices {
                let p = Vec3::from(v.position);
                aabb_min = aabb_min.min(p);
                aabb_max = aabb_max.max(p);
            }
            if !mesh.vertices.is_empty() {
                mesh.aabb_min = aabb_min;
                mesh.aabb_max = aabb_max;
            }
        }
    }

    // Transform bone inverse bind matrices and animation keyframes
    if (need_scale || need_axis) && !model.bones.is_empty() {
        let axis_mat = if need_axis {
            // Z-up → Y-up: swap Y/Z, negate new Z
            Mat4::from_cols_array(&[
                1.0, 0.0, 0.0, 0.0, 0.0, 0.0, -1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0,
                1.0,
            ])
        } else {
            Mat4::IDENTITY
        };
        let scale_mat = if need_scale {
            Mat4::from_scale(Vec3::splat(settings.scale))
        } else {
            Mat4::IDENTITY
        };
        let transform = axis_mat * scale_mat;
        let inv_transform = transform.inverse();

        for bone in &mut model.bones {
            // Similarity transform: ibm' = transform * ibm * inv_transform
            // This ensures palette' * pos' = S * palette * pos (correctly scaled skinning).
            // A simple right-multiply (ibm *= inv_transform) breaks because uniform scale
            // doesn't commute with the translation component of affine transforms.
            bone.inverse_bind_matrix = transform * bone.inverse_bind_matrix * inv_transform;
        }
    }

    // Transform animation keyframes
    if need_scale || need_axis {
        for clip in &mut model.animations {
            for ch in &mut clip.channels {
                for (_, pos) in &mut ch.position_keys {
                    if need_axis {
                        let y = pos.y;
                        pos.y = pos.z;
                        pos.z = -y;
                    }
                    if need_scale {
                        *pos *= settings.scale;
                    }
                }
                if need_axis {
                    for (_, rot) in &mut ch.rotation_keys {
                        let y = rot.y;
                        *rot = Quat::from_xyzw(rot.x, rot.z, -y, rot.w);
                    }
                }
                for (_, scl) in &mut ch.scale_keys {
                    if need_axis {
                        std::mem::swap(&mut scl.y, &mut scl.z);
                    }
                }
            }
        }
    }
}

// ──────────────────────────────────────────────────────────────
// .mesh binary writer (v2: bones + skinning)
// ──────────────────────────────────────────────────────────────

/// Write a `Model` to a `.mesh` binary file (v2 format).
pub fn write_mesh_binary(path: &Path, model: &Model) -> io::Result<()> {
    let mut buf: Vec<u8> = Vec::new();

    let mesh_count = model.meshes.len() as u32;
    let mat_count = model.materials.len() as u32;
    let bone_count = model.bones.len() as u32;
    let has_bones = !model.bones.is_empty();
    let flags: u32 = if has_bones { FLAG_HAS_BONES } else { 0 };

    // ── Header (24 bytes) ──
    buf.extend_from_slice(MESH_MAGIC);
    buf.extend_from_slice(&MESH_VERSION.to_le_bytes());
    buf.extend_from_slice(&flags.to_le_bytes());
    buf.extend_from_slice(&mesh_count.to_le_bytes());
    buf.extend_from_slice(&mat_count.to_le_bytes());
    buf.extend_from_slice(&bone_count.to_le_bytes());

    // ── Mesh descriptors (filled with placeholder offsets, patched later) ──
    let desc_start = buf.len();
    buf.resize(desc_start + MESH_DESC_SIZE * model.meshes.len(), 0);

    // ── Material section ──
    for mat in &model.materials {
        write_material(&mut buf, mat);
    }

    // ── Bone section (v2) ──
    if has_bones {
        for bone in &model.bones {
            let name_bytes = bone.name.as_bytes();
            buf.extend_from_slice(&(name_bytes.len() as u32).to_le_bytes());
            buf.extend_from_slice(name_bytes);
            let parent: i32 = bone.parent_index.map(|i| i as i32).unwrap_or(-1);
            buf.extend_from_slice(&parent.to_le_bytes());
            // Inverse bind matrix: 16 f32s, column-major
            for &f in &bone.inverse_bind_matrix.to_cols_array() {
                buf.extend_from_slice(&f.to_le_bytes());
            }
        }
    }

    // ── Bulk data + patch descriptors ──
    for (i, mesh) in model.meshes.iter().enumerate() {
        let vertex_offset = buf.len() as u64;
        // Write vertex data (Vertex3D is repr(C), 48 bytes each)
        for v in &mesh.vertices {
            buf.extend_from_slice(&v.position[0].to_le_bytes());
            buf.extend_from_slice(&v.position[1].to_le_bytes());
            buf.extend_from_slice(&v.position[2].to_le_bytes());
            buf.extend_from_slice(&v.normal[0].to_le_bytes());
            buf.extend_from_slice(&v.normal[1].to_le_bytes());
            buf.extend_from_slice(&v.normal[2].to_le_bytes());
            buf.extend_from_slice(&v.uv[0].to_le_bytes());
            buf.extend_from_slice(&v.uv[1].to_le_bytes());
            buf.extend_from_slice(&v.tangent[0].to_le_bytes());
            buf.extend_from_slice(&v.tangent[1].to_le_bytes());
            buf.extend_from_slice(&v.tangent[2].to_le_bytes());
            buf.extend_from_slice(&v.tangent[3].to_le_bytes());
        }

        let index_offset = buf.len() as u64;
        for idx in &mesh.indices {
            buf.extend_from_slice(&idx.to_le_bytes());
        }

        // Write per-mesh skinning data (v2)
        if has_bones {
            if let Some(ref skinning) = mesh.skinning {
                buf.push(1u8); // has_skinning flag
                for vb in skinning {
                    for &j in &vb.joints {
                        buf.extend_from_slice(&j.to_le_bytes());
                    }
                    for &w in &vb.weights {
                        buf.extend_from_slice(&w.to_le_bytes());
                    }
                }
            } else {
                buf.push(0u8); // no skinning
            }
        }

        // Patch descriptor
        let desc_offset = desc_start + i * MESH_DESC_SIZE;
        let mat_idx: i32 = mesh.material_index.map(|i| i as i32).unwrap_or(-1);

        let mut desc = Vec::with_capacity(MESH_DESC_SIZE);
        desc.extend_from_slice(&(mesh.vertices.len() as u32).to_le_bytes());
        desc.extend_from_slice(&(mesh.indices.len() as u32).to_le_bytes());
        desc.extend_from_slice(&vertex_offset.to_le_bytes());
        desc.extend_from_slice(&index_offset.to_le_bytes());
        desc.extend_from_slice(&mat_idx.to_le_bytes());
        desc.extend_from_slice(&mesh.center.x.to_le_bytes());
        desc.extend_from_slice(&mesh.center.y.to_le_bytes());
        desc.extend_from_slice(&mesh.center.z.to_le_bytes());
        desc.extend_from_slice(&mesh.radius.to_le_bytes());
        desc.extend_from_slice(&mesh.aabb_min.x.to_le_bytes());
        desc.extend_from_slice(&mesh.aabb_min.y.to_le_bytes());
        desc.extend_from_slice(&mesh.aabb_min.z.to_le_bytes());
        desc.extend_from_slice(&mesh.aabb_max.x.to_le_bytes());
        desc.extend_from_slice(&mesh.aabb_max.y.to_le_bytes());
        desc.extend_from_slice(&mesh.aabb_max.z.to_le_bytes());

        buf[desc_offset..desc_offset + MESH_DESC_SIZE].copy_from_slice(&desc);
    }

    std::fs::write(path, &buf)
}

fn write_material(buf: &mut Vec<u8>, mat: &ImportedMaterial) {
    let name_bytes = mat.name.as_bytes();
    buf.extend_from_slice(&(name_bytes.len() as u32).to_le_bytes());
    buf.extend_from_slice(name_bytes);

    for f in &mat.base_color_factor {
        buf.extend_from_slice(&f.to_le_bytes());
    }
    buf.extend_from_slice(&mat.metallic_factor.to_le_bytes());
    buf.extend_from_slice(&mat.roughness_factor.to_le_bytes());

    buf.push(mat.albedo.is_some() as u8);
    buf.push(mat.normal.is_some() as u8);
    buf.push(mat.metallic_roughness.is_some() as u8);
    buf.push(mat.ao.is_some() as u8);

    for img in [&mat.albedo, &mat.normal, &mat.metallic_roughness, &mat.ao]
        .into_iter()
        .flatten()
    {
        buf.extend_from_slice(&img.width().to_le_bytes());
        buf.extend_from_slice(&img.height().to_le_bytes());
        buf.extend_from_slice(img.as_raw());
    }
}

// ──────────────────────────────────────────────────────────────
// .mesh binary reader (v1 + v2 backward compat)
// ──────────────────────────────────────────────────────────────

/// Load a `Model` from a `.mesh` binary file on disk.
///
/// The render pipeline expects vertex data in Y-up (render space), which
/// matches what FBX/glTF loaders produce.  The model matrix handles the
/// Z-up game-space → Y-up conversion via `C * M_zup * C_inv`.
///
/// The only case requiring correction is when an FBX/glTF source was
/// imported with `up_axis: ZUp` — the import applied a redundant Z-up
/// conversion on top of the loader's Y-up output.  We undo that here.
pub fn load_mesh_binary(path: &Path) -> Result<Model, Box<dyn std::error::Error>> {
    let data = std::fs::read(path)?;
    let mut model = load_mesh_binary_from_bytes(&data, path.to_string_lossy().as_ref())?;

    // Check sidecar for double axis conversion that needs undoing
    let sidecar_path = PathBuf::from(format!("{}.ron", path.display()));
    if let Ok(text) = std::fs::read_to_string(&sidecar_path) {
        if let Ok(meta) = ron::from_str::<MeshImportMeta>(&text) {
            let src_ext = Path::new(&meta.source)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if meta.settings.up_axis == UpAxis::ZUp
                && matches!(src_ext.as_str(), "fbx" | "gltf" | "glb")
            {
                log::info!(
                    "Undoing double axis conversion for {:?} (FBX/glTF + ZUp)",
                    path
                );
                undo_double_axis_conversion(&mut model);
            }
        }
    }

    Ok(model)
}

/// Reverse the Z-up→Y-up axis swap that was erroneously applied to data
/// which was already in Y-up (from ufbx / glTF loaders).
///
/// Forward was: `(x, y, z) → (x, z, -y)`.  Inverse: `(x, y, z) → (x, -z, y)`.
fn undo_double_axis_conversion(model: &mut Model) {
    // Undo vertex positions, normals, tangents
    for mesh in &mut model.meshes {
        for v in &mut mesh.vertices {
            let y = v.position[1];
            v.position[1] = -v.position[2];
            v.position[2] = y;

            let ny = v.normal[1];
            v.normal[1] = -v.normal[2];
            v.normal[2] = ny;

            let ty = v.tangent[1];
            v.tangent[1] = -v.tangent[2];
            v.tangent[2] = ty;
        }

        // Recompute bounds
        let (center, radius) =
            super::model_loader::compute_bounding_sphere(&mesh.vertices);
        mesh.center = center;
        mesh.radius = radius;
        let mut aabb_min = Vec3::splat(f32::MAX);
        let mut aabb_max = Vec3::splat(f32::MIN);
        for v in &mesh.vertices {
            let p = Vec3::from(v.position);
            aabb_min = aabb_min.min(p);
            aabb_max = aabb_max.max(p);
        }
        if !mesh.vertices.is_empty() {
            mesh.aabb_min = aabb_min;
            mesh.aabb_max = aabb_max;
        }
    }

    // Undo bone inverse bind matrices
    if !model.bones.is_empty() {
        // Forward applied similarity transform: ibm' = transform * ibm * inv_transform
        // Undo: ibm = inv_transform * ibm' * transform
        let axis_mat = Mat4::from_cols_array(&[
            1.0, 0.0, 0.0, 0.0, 0.0, 0.0, -1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0,
            1.0,
        ]);
        let inv_axis = axis_mat.inverse();
        for bone in &mut model.bones {
            bone.inverse_bind_matrix = inv_axis * bone.inverse_bind_matrix * axis_mat;
        }
    }

    // Undo animation keyframes (these were also double-converted during import)
    for clip in &mut model.animations {
        for ch in &mut clip.channels {
            for (_, pos) in &mut ch.position_keys {
                let y = pos.y;
                pos.y = -pos.z;
                pos.z = y;
            }
            for (_, rot) in &mut ch.rotation_keys {
                let y = rot.y;
                *rot = Quat::from_xyzw(rot.x, -rot.z, y, rot.w);
            }
            for (_, scl) in &mut ch.scale_keys {
                std::mem::swap(&mut scl.y, &mut scl.z);
            }
        }
    }
}

/// Load a `Model` from in-memory `.mesh` binary data.
pub fn load_mesh_binary_from_bytes(
    data: &[u8],
    name: &str,
) -> Result<Model, Box<dyn std::error::Error>> {
    if data.len() < HEADER_SIZE {
        return Err("Mesh file too small for header".into());
    }

    // ── Header ──
    if &data[0..4] != MESH_MAGIC {
        return Err("Invalid mesh file magic (expected RMSH)".into());
    }

    let version = u32::from_le_bytes(data[4..8].try_into().unwrap());
    if version > MESH_VERSION {
        return Err(format!(
            "Unsupported mesh version {} (max supported: {})",
            version, MESH_VERSION
        )
        .into());
    }

    let flags = u32::from_le_bytes(data[8..12].try_into().unwrap());
    let mesh_count = u32::from_le_bytes(data[12..16].try_into().unwrap()) as usize;
    let mat_count = u32::from_le_bytes(data[16..20].try_into().unwrap()) as usize;

    // In v2, byte [20..24] is bone_count. In v1, it was reserved (always 0).
    let bone_count = if version >= 2 {
        u32::from_le_bytes(data[20..24].try_into().unwrap()) as usize
    } else {
        0
    };
    let has_bones = version >= 2 && (flags & FLAG_HAS_BONES) != 0;

    let expected_desc_end = HEADER_SIZE + mesh_count * MESH_DESC_SIZE;
    if data.len() < expected_desc_end {
        return Err("Mesh file truncated in descriptor section".into());
    }

    // ── Parse mesh descriptors ──
    struct MeshDesc {
        vertex_count: u32,
        index_count: u32,
        vertex_offset: u64,
        index_offset: u64,
        material_index: i32,
        center: Vec3,
        radius: f32,
        aabb_min: Vec3,
        aabb_max: Vec3,
    }

    let mut descriptors = Vec::with_capacity(mesh_count);
    for i in 0..mesh_count {
        let off = HEADER_SIZE + i * MESH_DESC_SIZE;
        let d = &data[off..off + MESH_DESC_SIZE];
        descriptors.push(MeshDesc {
            vertex_count: read_u32(d, 0),
            index_count: read_u32(d, 4),
            vertex_offset: read_u64(d, 8),
            index_offset: read_u64(d, 16),
            material_index: read_i32(d, 24),
            center: Vec3::new(read_f32(d, 28), read_f32(d, 32), read_f32(d, 36)),
            radius: read_f32(d, 40),
            aabb_min: Vec3::new(read_f32(d, 44), read_f32(d, 48), read_f32(d, 52)),
            aabb_max: Vec3::new(read_f32(d, 56), read_f32(d, 60), read_f32(d, 64)),
        });
    }

    // ── Parse materials ──
    let mut cursor = expected_desc_end;
    let mut materials = Vec::with_capacity(mat_count);
    for _ in 0..mat_count {
        let (mat, new_cursor) = read_material(data, cursor)?;
        materials.push(mat);
        cursor = new_cursor;
    }

    // ── Parse bones (v2) ──
    let mut bones = Vec::with_capacity(bone_count);
    if has_bones {
        for _ in 0..bone_count {
            if cursor + 4 > data.len() {
                return Err("Mesh file truncated in bone section".into());
            }
            let name_len = read_u32(data, cursor) as usize;
            cursor += 4;
            if cursor + name_len > data.len() {
                return Err("Mesh file truncated in bone name".into());
            }
            let bone_name =
                String::from_utf8_lossy(&data[cursor..cursor + name_len]).to_string();
            cursor += name_len;

            if cursor + 4 + 64 > data.len() {
                return Err("Mesh file truncated in bone data".into());
            }
            let parent_raw = read_i32(data, cursor);
            cursor += 4;
            let parent_index = if parent_raw >= 0 {
                Some(parent_raw as usize)
            } else {
                None
            };

            let mut cols = [0.0f32; 16];
            for c in &mut cols {
                *c = read_f32(data, cursor);
                cursor += 4;
            }
            let inverse_bind_matrix = Mat4::from_cols_array(&cols);

            bones.push(BoneData {
                name: bone_name,
                parent_index,
                inverse_bind_matrix,
            });
        }
    }

    // ── Parse bulk vertex/index/skinning data ──
    let mut meshes = Vec::with_capacity(mesh_count);
    for desc in &descriptors {
        let v_start = desc.vertex_offset as usize;
        let v_bytes = desc.vertex_count as usize * 48;
        if v_start + v_bytes > data.len() {
            return Err("Mesh file truncated in vertex data".into());
        }

        let mut vertices = Vec::with_capacity(desc.vertex_count as usize);
        for vi in 0..desc.vertex_count as usize {
            let off = v_start + vi * 48;
            vertices.push(Vertex3D {
                position: [
                    read_f32(data, off),
                    read_f32(data, off + 4),
                    read_f32(data, off + 8),
                ],
                normal: [
                    read_f32(data, off + 12),
                    read_f32(data, off + 16),
                    read_f32(data, off + 20),
                ],
                uv: [read_f32(data, off + 24), read_f32(data, off + 28)],
                tangent: [
                    read_f32(data, off + 32),
                    read_f32(data, off + 36),
                    read_f32(data, off + 40),
                    read_f32(data, off + 44),
                ],
                ..Default::default()
            });
        }

        let i_start = desc.index_offset as usize;
        let i_bytes = desc.index_count as usize * 4;
        if i_start + i_bytes > data.len() {
            return Err("Mesh file truncated in index data".into());
        }

        let mut indices = Vec::with_capacity(desc.index_count as usize);
        for ii in 0..desc.index_count as usize {
            indices.push(read_u32(data, i_start + ii * 4));
        }

        // Read per-mesh skinning (v2)
        let skinning = if has_bones {
            let skin_start = i_start + i_bytes;
            if skin_start >= data.len() {
                None
            } else {
                let has_skinning = data[skin_start] != 0;
                if has_skinning {
                    let mut skin_data = Vec::with_capacity(desc.vertex_count as usize);
                    let mut off = skin_start + 1;
                    for _ in 0..desc.vertex_count {
                        let joints = [
                            read_u16(data, off),
                            read_u16(data, off + 2),
                            read_u16(data, off + 4),
                            read_u16(data, off + 6),
                        ];
                        off += 8;
                        let weights = [
                            read_f32(data, off),
                            read_f32(data, off + 4),
                            read_f32(data, off + 8),
                            read_f32(data, off + 12),
                        ];
                        off += 16;
                        skin_data.push(VertexBoneData { joints, weights });
                    }
                    Some(skin_data)
                } else {
                    None
                }
            }
        } else {
            None
        };

        // Merge skinning data into Vertex3D fields
        if let Some(ref skin) = skinning {
            for (vi, vb) in skin.iter().enumerate() {
                vertices[vi].joint_indices = [
                    vb.joints[0] as u32,
                    vb.joints[1] as u32,
                    vb.joints[2] as u32,
                    vb.joints[3] as u32,
                ];
                vertices[vi].joint_weights = vb.weights;
            }
        }

        meshes.push(LoadedMesh {
            vertices,
            indices,
            material_index: if desc.material_index >= 0 {
                Some(desc.material_index as usize)
            } else {
                None
            },
            center: desc.center,
            radius: desc.radius,
            aabb_min: desc.aabb_min,
            aabb_max: desc.aabb_max,
            skinning,
        });
    }

    // Validate bone count against FixedUbo backend cap
    use crate::engine::rendering::rendering_3d::pipeline_3d::MAX_PALETTE_BONES;
    if bones.len() > MAX_PALETTE_BONES {
        return Err(format!(
            "Mesh '{}' has {} bones, exceeding the current skinning backend cap of {}. \
             A larger backend (LargeSsbo) is needed for this asset.",
            name,
            bones.len(),
            MAX_PALETTE_BONES,
        )
        .into());
    }
    if bones.len() > 200 {
        log::warn!(
            "Mesh '{}' has {} bones (approaching FixedUbo cap of {})",
            name,
            bones.len(),
            MAX_PALETTE_BONES,
        );
    }

    let mut model = Model {
        meshes,
        name: Path::new(name)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("mesh")
            .to_string(),
        textures: Vec::new(),
        materials,
        bones,
        animations: Vec::new(),
    };
    model.rebuild_legacy_textures();

    Ok(model)
}

fn read_material(
    data: &[u8],
    mut cursor: usize,
) -> Result<(ImportedMaterial, usize), Box<dyn std::error::Error>> {
    if cursor + 4 > data.len() {
        return Err("Truncated material name length".into());
    }
    let name_len = read_u32(data, cursor) as usize;
    cursor += 4;
    if cursor + name_len > data.len() {
        return Err("Truncated material name".into());
    }
    let name = String::from_utf8_lossy(&data[cursor..cursor + name_len]).to_string();
    cursor += name_len;

    if cursor + 24 > data.len() {
        return Err("Truncated material factors".into());
    }
    let base_color_factor = [
        read_f32(data, cursor),
        read_f32(data, cursor + 4),
        read_f32(data, cursor + 8),
        read_f32(data, cursor + 12),
    ];
    let metallic_factor = read_f32(data, cursor + 16);
    let roughness_factor = read_f32(data, cursor + 20);
    cursor += 24;

    if cursor + 4 > data.len() {
        return Err("Truncated material texture flags".into());
    }
    let has_albedo = data[cursor] != 0;
    let has_normal = data[cursor + 1] != 0;
    let has_metallic_roughness = data[cursor + 2] != 0;
    let has_ao = data[cursor + 3] != 0;
    cursor += 4;

    let read_texture =
        |cursor: &mut usize| -> Result<Option<image::RgbaImage>, Box<dyn std::error::Error>> {
            if *cursor + 8 > data.len() {
                return Err("Truncated texture dimensions".into());
            }
            let w = read_u32(data, *cursor);
            let h = read_u32(data, *cursor + 4);
            *cursor += 8;
            let pixel_bytes = (w as usize) * (h as usize) * 4;
            if *cursor + pixel_bytes > data.len() {
                return Err("Truncated texture data".into());
            }
            let pixels = data[*cursor..*cursor + pixel_bytes].to_vec();
            *cursor += pixel_bytes;
            Ok(image::RgbaImage::from_raw(w, h, pixels))
        };

    let albedo = if has_albedo {
        read_texture(&mut cursor)?
    } else {
        None
    };
    let normal = if has_normal {
        read_texture(&mut cursor)?
    } else {
        None
    };
    let metallic_roughness = if has_metallic_roughness {
        read_texture(&mut cursor)?
    } else {
        None
    };
    let ao = if has_ao {
        read_texture(&mut cursor)?
    } else {
        None
    };

    Ok((
        ImportedMaterial {
            name,
            albedo,
            normal,
            metallic_roughness,
            ao,
            base_color_factor,
            metallic_factor,
            roughness_factor,
            emissive_factor: [0.0, 0.0, 0.0], // Not in binary format; default to zero
        },
        cursor,
    ))
}

// ──────────────────────────────────────────────────────────────
// .anim binary format (writer + reader)
// ──────────────────────────────────────────────────────────────

/// Write animation clips to a `.anim` binary file.
pub fn write_anim_binary(
    path: &Path,
    animations: &[RawAnimationClip],
    bone_names: &[String],
) -> io::Result<()> {
    let mut buf: Vec<u8> = Vec::new();

    // Header (16 bytes)
    buf.extend_from_slice(ANIM_MAGIC);
    buf.extend_from_slice(&ANIM_VERSION.to_le_bytes());
    buf.extend_from_slice(&(animations.len() as u32).to_le_bytes());
    buf.extend_from_slice(&(bone_names.len() as u32).to_le_bytes());

    // Bone name table
    for name in bone_names {
        let bytes = name.as_bytes();
        buf.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        buf.extend_from_slice(bytes);
    }

    // Clip data
    for clip in animations {
        let name_bytes = clip.name.as_bytes();
        buf.extend_from_slice(&(name_bytes.len() as u32).to_le_bytes());
        buf.extend_from_slice(name_bytes);
        buf.extend_from_slice(&clip.duration_seconds.to_le_bytes());
        buf.extend_from_slice(&(clip.channels.len() as u32).to_le_bytes());

        for ch in &clip.channels {
            buf.extend_from_slice(&(ch.bone_index as u32).to_le_bytes());

            // Position keys
            buf.extend_from_slice(&(ch.position_keys.len() as u32).to_le_bytes());
            for (time, pos) in &ch.position_keys {
                buf.extend_from_slice(&time.to_le_bytes());
                buf.extend_from_slice(&pos.x.to_le_bytes());
                buf.extend_from_slice(&pos.y.to_le_bytes());
                buf.extend_from_slice(&pos.z.to_le_bytes());
            }

            // Rotation keys
            buf.extend_from_slice(&(ch.rotation_keys.len() as u32).to_le_bytes());
            for (time, rot) in &ch.rotation_keys {
                buf.extend_from_slice(&time.to_le_bytes());
                buf.extend_from_slice(&rot.x.to_le_bytes());
                buf.extend_from_slice(&rot.y.to_le_bytes());
                buf.extend_from_slice(&rot.z.to_le_bytes());
                buf.extend_from_slice(&rot.w.to_le_bytes());
            }

            // Scale keys
            buf.extend_from_slice(&(ch.scale_keys.len() as u32).to_le_bytes());
            for (time, scl) in &ch.scale_keys {
                buf.extend_from_slice(&time.to_le_bytes());
                buf.extend_from_slice(&scl.x.to_le_bytes());
                buf.extend_from_slice(&scl.y.to_le_bytes());
                buf.extend_from_slice(&scl.z.to_le_bytes());
            }
        }
    }

    std::fs::write(path, &buf)
}

// ──────────────────────────────────────────────────────────────
// Material export (source → .material.ron + textures)
// ──────────────────────────────────────────────────────────────

/// Export each `ImportedMaterial` as a `.material.ron` file plus texture PNGs.
///
/// Files are written next to `mesh_path`:
///   `Foo.mesh` → `Foo_MatName.material.ron`, `Foo_MatName_albedo.png`, etc.
///
/// Returns the number of materials exported.
fn export_materials(
    mesh_path: &Path,
    materials: &[ImportedMaterial],
) -> Result<usize, Box<dyn std::error::Error>> {
    let stem = mesh_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("model");
    let parent = mesh_path.parent().unwrap_or(Path::new("."));

    let mut count = 0;
    for mat in materials {
        // Sanitize material name for use as a filename
        let safe_name = sanitize_filename(&mat.name);
        let prefix = format!("{}_{}", stem, safe_name);

        // Export texture images as PNGs
        let albedo_file = save_material_texture(parent, &prefix, "albedo", &mat.albedo)?;
        let normal_file = save_material_texture(parent, &prefix, "normal", &mat.normal)?;
        let mr_file = save_material_texture(
            parent,
            &prefix,
            "metallic_roughness",
            &mat.metallic_roughness,
        )?;
        let ao_file = save_material_texture(parent, &prefix, "ao", &mat.ao)?;

        // Write .material.ron
        let def = MaterialDefinition {
            name: mat.name.clone(),
            base_color_factor: mat.base_color_factor,
            metallic_factor: mat.metallic_factor,
            roughness_factor: mat.roughness_factor,
            emissive_factor: mat.emissive_factor,
            albedo_texture: albedo_file,
            normal_texture: normal_file,
            metallic_roughness_texture: mr_file,
            ao_texture: ao_file,
        };

        let ron_path = parent.join(format!("{}.material.ron", prefix));
        let ron_text = ron::ser::to_string_pretty(&def, ron::ser::PrettyConfig::default())?;
        std::fs::write(&ron_path, ron_text)?;
        count += 1;
    }

    Ok(count)
}

/// Save a material texture as a PNG file. Returns the filename (not full path)
/// or empty string if no texture.
fn save_material_texture(
    dir: &Path,
    prefix: &str,
    slot: &str,
    texture: &Option<image::RgbaImage>,
) -> Result<String, Box<dyn std::error::Error>> {
    match texture {
        Some(img) => {
            let filename = format!("{}_{}.png", prefix, slot);
            let path = dir.join(&filename);
            img.save(&path)?;
            Ok(filename)
        }
        None => Ok(String::new()),
    }
}

/// Replace characters that are invalid in filenames.
fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | ' ' => '_',
            _ => c,
        })
        .collect()
}

// ──────────────────────────────────────────────────────────────
// Animation binary format (.anim)
// ──────────────────────────────────────────────────────────────

/// Load animation clips from a `.anim` binary file on disk.
pub fn load_anim_binary(
    path: &Path,
) -> Result<(Vec<String>, Vec<RawAnimationClip>), Box<dyn std::error::Error>> {
    let data = std::fs::read(path)?;
    load_anim_binary_from_bytes(&data)
}

/// Load animation clips from in-memory `.anim` binary data.
pub fn load_anim_binary_from_bytes(
    data: &[u8],
) -> Result<(Vec<String>, Vec<RawAnimationClip>), Box<dyn std::error::Error>> {
    if data.len() < 16 {
        return Err("Anim file too small for header".into());
    }

    if &data[0..4] != ANIM_MAGIC {
        return Err("Invalid anim file magic (expected RANM)".into());
    }

    let version = read_u32(data, 4);
    if version > ANIM_VERSION {
        return Err(format!(
            "Unsupported anim version {} (max supported: {})",
            version, ANIM_VERSION
        )
        .into());
    }

    let clip_count = read_u32(data, 8) as usize;
    let bone_name_count = read_u32(data, 12) as usize;

    let mut cursor = 16usize;

    // Bone name table
    let mut bone_names = Vec::with_capacity(bone_name_count);
    for _ in 0..bone_name_count {
        if cursor + 4 > data.len() {
            return Err("Anim file truncated in bone name table".into());
        }
        let name_len = read_u32(data, cursor) as usize;
        cursor += 4;
        if cursor + name_len > data.len() {
            return Err("Anim file truncated in bone name".into());
        }
        bone_names.push(String::from_utf8_lossy(&data[cursor..cursor + name_len]).to_string());
        cursor += name_len;
    }

    // Clip data
    let mut clips = Vec::with_capacity(clip_count);
    for _ in 0..clip_count {
        if cursor + 4 > data.len() {
            return Err("Anim file truncated in clip name".into());
        }
        let name_len = read_u32(data, cursor) as usize;
        cursor += 4;
        if cursor + name_len > data.len() {
            return Err("Anim file truncated in clip name data".into());
        }
        let clip_name =
            String::from_utf8_lossy(&data[cursor..cursor + name_len]).to_string();
        cursor += name_len;

        if cursor + 8 > data.len() {
            return Err("Anim file truncated in clip header".into());
        }
        let duration = read_f32(data, cursor);
        cursor += 4;
        let channel_count = read_u32(data, cursor) as usize;
        cursor += 4;

        let mut channels = Vec::with_capacity(channel_count);
        for _ in 0..channel_count {
            if cursor + 4 > data.len() {
                return Err("Anim file truncated in channel header".into());
            }
            let bone_index = read_u32(data, cursor) as usize;
            cursor += 4;

            // Position keys
            if cursor + 4 > data.len() {
                return Err("Anim file truncated".into());
            }
            let pos_count = read_u32(data, cursor) as usize;
            cursor += 4;
            let mut position_keys = Vec::with_capacity(pos_count);
            for _ in 0..pos_count {
                let time = read_f32(data, cursor);
                let x = read_f32(data, cursor + 4);
                let y = read_f32(data, cursor + 8);
                let z = read_f32(data, cursor + 12);
                cursor += 16;
                position_keys.push((time, Vec3::new(x, y, z)));
            }

            // Rotation keys
            let rot_count = read_u32(data, cursor) as usize;
            cursor += 4;
            let mut rotation_keys = Vec::with_capacity(rot_count);
            for _ in 0..rot_count {
                let time = read_f32(data, cursor);
                let x = read_f32(data, cursor + 4);
                let y = read_f32(data, cursor + 8);
                let z = read_f32(data, cursor + 12);
                let w = read_f32(data, cursor + 16);
                cursor += 20;
                rotation_keys.push((time, Quat::from_xyzw(x, y, z, w)));
            }

            // Scale keys
            let scl_count = read_u32(data, cursor) as usize;
            cursor += 4;
            let mut scale_keys = Vec::with_capacity(scl_count);
            for _ in 0..scl_count {
                let time = read_f32(data, cursor);
                let x = read_f32(data, cursor + 4);
                let y = read_f32(data, cursor + 8);
                let z = read_f32(data, cursor + 12);
                cursor += 16;
                scale_keys.push((time, Vec3::new(x, y, z)));
            }

            channels.push(AnimationChannel {
                bone_index,
                position_keys,
                rotation_keys,
                scale_keys,
            });
        }

        clips.push(RawAnimationClip {
            name: clip_name,
            duration_seconds: duration,
            channels,
        });
    }

    Ok((bone_names, clips))
}

// ──────────────────────────────────────────────────────────────
// Sidecar (.mesh.ron)
// ──────────────────────────────────────────────────────────────

/// Write import metadata sidecar alongside the `.mesh` binary.
pub fn write_mesh_sidecar(mesh_path: &Path, meta: &MeshImportMeta) -> io::Result<()> {
    let sidecar_path = mesh_path.with_extension("mesh.ron");
    let ron_str = ron::ser::to_string_pretty(meta, ron::ser::PrettyConfig::default())
        .map_err(io::Error::other)?;
    std::fs::write(sidecar_path, ron_str)
}

/// Read import metadata sidecar for a `.mesh` file.
pub fn load_mesh_sidecar(mesh_path: &Path) -> Result<MeshImportMeta, Box<dyn std::error::Error>> {
    let sidecar_path = mesh_path.with_extension("mesh.ron");
    let text = std::fs::read_to_string(sidecar_path)?;
    let meta: MeshImportMeta = ron::from_str(&text)?;
    Ok(meta)
}

// ──────────────────────────────────────────────────────────────
// Helpers
// ──────────────────────────────────────────────────────────────

fn read_u16(data: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(data[offset..offset + 2].try_into().unwrap())
}

fn read_u32(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap())
}

fn read_i32(data: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(data[offset..offset + 4].try_into().unwrap())
}

fn read_u64(data: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(data[offset..offset + 8].try_into().unwrap())
}

fn read_f32(data: &[u8], offset: usize) -> f32 {
    f32::from_le_bytes(data[offset..offset + 4].try_into().unwrap())
}

/// Simple CRC32 hash for source change detection.
fn crc32_hash(data: &[u8]) -> u32 {
    let mut hash: u32 = 0xFFFFFFFF;
    for &byte in data {
        hash ^= byte as u32;
        for _ in 0..8 {
            if hash & 1 != 0 {
                hash = (hash >> 1) ^ 0xEDB88320;
            } else {
                hash >>= 1;
            }
        }
    }
    !hash
}

// ──────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_model() -> Model {
        let vertices = vec![
            Vertex3D {
                position: [0.0, 0.0, 0.0],
                normal: [0.0, 1.0, 0.0],
                uv: [0.0, 0.0],
                tangent: [1.0, 0.0, 0.0, 1.0],
                ..Default::default()
            },
            Vertex3D {
                position: [1.0, 0.0, 0.0],
                normal: [0.0, 1.0, 0.0],
                uv: [1.0, 0.0],
                tangent: [1.0, 0.0, 0.0, 1.0],
                ..Default::default()
            },
            Vertex3D {
                position: [0.0, 1.0, 0.0],
                normal: [0.0, 1.0, 0.0],
                uv: [0.0, 1.0],
                tangent: [1.0, 0.0, 0.0, 1.0],
                ..Default::default()
            },
        ];
        let indices = vec![0, 1, 2];

        let mesh = LoadedMesh {
            vertices,
            indices,
            material_index: Some(0),
            center: Vec3::new(0.333, 0.333, 0.0),
            radius: 0.8,
            aabb_min: Vec3::new(0.0, 0.0, 0.0),
            aabb_max: Vec3::new(1.0, 1.0, 0.0),
            skinning: None,
        };

        let material = ImportedMaterial {
            name: "TestMat".to_string(),
            base_color_factor: [1.0, 0.5, 0.25, 1.0],
            metallic_factor: 0.8,
            roughness_factor: 0.3,
            ..Default::default()
        };

        Model {
            meshes: vec![mesh],
            name: "TestModel".to_string(),
            textures: Vec::new(),
            materials: vec![material],
            bones: Vec::new(),
            animations: Vec::new(),
        }
    }

    fn make_test_model_with_skeleton() -> Model {
        let mut model = make_test_model();
        model.bones = vec![
            BoneData {
                name: "Root".to_string(),
                parent_index: None,
                inverse_bind_matrix: Mat4::IDENTITY,
            },
            BoneData {
                name: "Spine".to_string(),
                parent_index: Some(0),
                inverse_bind_matrix: Mat4::from_translation(Vec3::new(0.0, -1.0, 0.0)),
            },
        ];
        model.meshes[0].skinning = Some(vec![
            VertexBoneData {
                joints: [0, 1, 0, 0],
                weights: [0.8, 0.2, 0.0, 0.0],
            },
            VertexBoneData {
                joints: [0, 0, 0, 0],
                weights: [1.0, 0.0, 0.0, 0.0],
            },
            VertexBoneData {
                joints: [1, 0, 0, 0],
                weights: [0.6, 0.4, 0.0, 0.0],
            },
        ]);
        model
    }

    #[test]
    fn roundtrip_mesh_binary() {
        let original = make_test_model();

        let temp_dir = std::env::temp_dir().join("rust_engine_test_mesh");
        let _ = std::fs::create_dir_all(&temp_dir);
        let mesh_path = temp_dir.join("test.mesh");

        write_mesh_binary(&mesh_path, &original).expect("write failed");
        let loaded = load_mesh_binary(&mesh_path).expect("load failed");

        let _ = std::fs::remove_file(&mesh_path);
        let _ = std::fs::remove_dir(&temp_dir);

        assert_eq!(loaded.meshes.len(), 1);
        assert_eq!(loaded.materials.len(), 1);

        let orig_mesh = &original.meshes[0];
        let load_mesh = &loaded.meshes[0];
        assert_eq!(orig_mesh.vertices.len(), load_mesh.vertices.len());
        assert_eq!(orig_mesh.indices.len(), load_mesh.indices.len());

        for (ov, lv) in orig_mesh.vertices.iter().zip(load_mesh.vertices.iter()) {
            assert_eq!(ov.position, lv.position);
            assert_eq!(ov.normal, lv.normal);
            assert_eq!(ov.uv, lv.uv);
            assert_eq!(ov.tangent, lv.tangent);
        }
        assert_eq!(orig_mesh.indices, load_mesh.indices);
        assert_eq!(orig_mesh.material_index, load_mesh.material_index);
        assert!(load_mesh.skinning.is_none());
        assert!(loaded.bones.is_empty());
    }

    #[test]
    fn roundtrip_mesh_v2_with_bones() {
        let original = make_test_model_with_skeleton();

        let temp_dir = std::env::temp_dir().join("rust_engine_test_mesh_v2");
        let _ = std::fs::create_dir_all(&temp_dir);
        let mesh_path = temp_dir.join("test_v2.mesh");

        write_mesh_binary(&mesh_path, &original).expect("write failed");
        let loaded = load_mesh_binary(&mesh_path).expect("load failed");

        let _ = std::fs::remove_file(&mesh_path);
        let _ = std::fs::remove_dir(&temp_dir);

        // Bones
        assert_eq!(loaded.bones.len(), 2);
        assert_eq!(loaded.bones[0].name, "Root");
        assert!(loaded.bones[0].parent_index.is_none());
        assert_eq!(loaded.bones[1].name, "Spine");
        assert_eq!(loaded.bones[1].parent_index, Some(0));

        // Inverse bind matrices
        let diff = (loaded.bones[1].inverse_bind_matrix
            - Mat4::from_translation(Vec3::new(0.0, -1.0, 0.0)))
        .abs_diff_eq(Mat4::ZERO, 0.001);
        assert!(diff, "Inverse bind matrix mismatch");

        // Skinning
        let skinning = loaded.meshes[0].skinning.as_ref().expect("should have skinning");
        assert_eq!(skinning.len(), 3);
        assert_eq!(skinning[0].joints, [0, 1, 0, 0]);
        assert!((skinning[0].weights[0] - 0.8).abs() < 0.001);
        assert!((skinning[0].weights[1] - 0.2).abs() < 0.001);
    }

    #[test]
    fn roundtrip_mesh_from_bytes() {
        let original = make_test_model();

        let temp_dir = std::env::temp_dir().join("rust_engine_test_mesh2");
        let _ = std::fs::create_dir_all(&temp_dir);
        let mesh_path = temp_dir.join("test2.mesh");

        write_mesh_binary(&mesh_path, &original).expect("write failed");
        let data = std::fs::read(&mesh_path).expect("read failed");
        let loaded = load_mesh_binary_from_bytes(&data, "test2.mesh").expect("parse failed");

        let _ = std::fs::remove_file(&mesh_path);
        let _ = std::fs::remove_dir(&temp_dir);

        assert_eq!(loaded.meshes.len(), 1);
        assert_eq!(loaded.meshes[0].vertices.len(), 3);
        assert_eq!(loaded.name, "test2");
    }

    #[test]
    fn roundtrip_anim_binary() {
        let bone_names = vec!["Root".to_string(), "Spine".to_string()];
        let clips = vec![RawAnimationClip {
            name: "Walk".to_string(),
            duration_seconds: 1.5,
            channels: vec![AnimationChannel {
                bone_index: 0,
                position_keys: vec![
                    (0.0, Vec3::new(0.0, 0.0, 0.0)),
                    (1.5, Vec3::new(1.0, 0.0, 0.0)),
                ],
                rotation_keys: vec![(0.0, Quat::IDENTITY), (1.5, Quat::from_xyzw(0.0, 0.707, 0.0, 0.707))],
                scale_keys: vec![(0.0, Vec3::ONE)],
            }],
        }];

        let temp_dir = std::env::temp_dir().join("rust_engine_test_anim");
        let _ = std::fs::create_dir_all(&temp_dir);
        let anim_path = temp_dir.join("test.anim");

        write_anim_binary(&anim_path, &clips, &bone_names).expect("write failed");
        let (loaded_names, loaded_clips) = load_anim_binary(&anim_path).expect("load failed");

        let _ = std::fs::remove_file(&anim_path);
        let _ = std::fs::remove_dir(&temp_dir);

        assert_eq!(loaded_names, bone_names);
        assert_eq!(loaded_clips.len(), 1);
        assert_eq!(loaded_clips[0].name, "Walk");
        assert!((loaded_clips[0].duration_seconds - 1.5).abs() < 0.001);
        assert_eq!(loaded_clips[0].channels.len(), 1);
        assert_eq!(loaded_clips[0].channels[0].position_keys.len(), 2);
        assert_eq!(loaded_clips[0].channels[0].rotation_keys.len(), 2);
        assert_eq!(loaded_clips[0].channels[0].scale_keys.len(), 1);
    }

    #[test]
    fn apply_scale_setting() {
        let mut model = make_test_model();
        let settings = MeshImportSettings {
            scale: 2.0,
            ..Default::default()
        };
        apply_import_settings(&mut model, &settings);

        assert_eq!(model.meshes[0].vertices[1].position[0], 2.0);
    }

    #[test]
    fn apply_flip_uvs() {
        let mut model = make_test_model();
        let settings = MeshImportSettings {
            flip_uvs: true,
            ..Default::default()
        };
        apply_import_settings(&mut model, &settings);

        assert!((model.meshes[0].vertices[0].uv[1] - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn crc32_deterministic() {
        let data = b"hello world";
        let h1 = crc32_hash(data);
        let h2 = crc32_hash(data);
        assert_eq!(h1, h2);
        assert_ne!(h1, 0);
    }

    #[test]
    fn import_settings_default() {
        let s = MeshImportSettings::default();
        assert_eq!(s.scale, 1.0);
        assert!(s.generate_tangents);
        assert!(s.import_materials);
        assert!(!s.flip_uvs);
        assert_eq!(s.up_axis, UpAxis::YUp);
        assert!(s.import_animations);
    }

    #[test]
    fn material_definition_emissive_default() {
        // Legacy .material.ron without emissive_factor should deserialize with zeros
        let ron_str = r#"(
            name: "LegacyMat",
            base_color_factor: (1.0, 0.5, 0.25, 1.0),
            metallic_factor: 0.0,
            roughness_factor: 0.5,
            albedo_texture: "",
            normal_texture: "",
            metallic_roughness_texture: "",
            ao_texture: "",
        )"#;

        let def: MaterialDefinition = ron::from_str(ron_str).expect("deserialize legacy");
        assert_eq!(def.emissive_factor, [0.0, 0.0, 0.0]);

        // Re-serialize should include emissive_factor
        let serialized = ron::to_string(&def).expect("serialize");
        assert!(
            serialized.contains("emissive_factor"),
            "re-serialized output should contain emissive_factor"
        );
    }

    #[test]
    fn material_definition_emissive_roundtrip() {
        let def = MaterialDefinition {
            name: "Emissive".to_string(),
            base_color_factor: [1.0, 1.0, 1.0, 1.0],
            metallic_factor: 0.0,
            roughness_factor: 0.5,
            emissive_factor: [3.0, 0.0, 0.0],
            albedo_texture: String::new(),
            normal_texture: String::new(),
            metallic_roughness_texture: String::new(),
            ao_texture: String::new(),
        };

        let serialized = ron::to_string(&def).expect("serialize");
        let deserialized: MaterialDefinition = ron::from_str(&serialized).expect("deserialize");
        assert_eq!(deserialized.emissive_factor, [3.0, 0.0, 0.0]);
    }
}
