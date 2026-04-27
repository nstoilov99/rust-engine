//! Thumbnail cache for asset browser
//!
//! Provides async thumbnail generation with memory and disk caching.
//! Model thumbnails are rendered as 3D previews on a background thread
//! using an offscreen GPU render pass.

use crate::engine::assets::{AssetId, AssetMetadata, AssetType};
use egui::{ColorImage, Context, TextureHandle, TextureId, TextureOptions};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;

use super::thumbnail_renderer::GpuThumbnailContext;

/// Size of generated thumbnails (square)
pub const THUMBNAIL_SIZE: u32 = 128;

/// Request to generate a thumbnail
#[derive(Debug)]
struct ThumbnailRequest {
    id: AssetId,
    path: PathBuf,
    asset_type: AssetType,
}

/// Result of thumbnail generation
struct ThumbnailResult {
    id: AssetId,
    image_data: Option<ColorImage>,
}

/// Entry in the thumbnail cache
struct ThumbnailEntry {
    texture: TextureHandle,
    #[allow(dead_code)]
    source_hash: u64,
}

/// Cache for asset thumbnails
///
/// Manages generation and caching of thumbnails for the asset browser.
pub struct ThumbnailCache {
    /// In-memory texture cache
    cache: HashMap<AssetId, ThumbnailEntry>,
    /// Assets currently being generated
    pending: std::collections::HashSet<AssetId>,
    /// Channel to send requests to worker thread
    request_tx: Option<mpsc::Sender<ThumbnailRequest>>,
    /// Channel to receive results from worker thread
    result_rx: mpsc::Receiver<ThumbnailResult>,
    _assets_root: PathBuf,
    placeholder: Option<TextureHandle>,
    error_placeholder: Option<TextureHandle>,
    _type_icons: HashMap<AssetType, TextureHandle>,
}

impl ThumbnailCache {
    /// Create a new thumbnail cache with GPU context for 3D model rendering.
    pub fn new(assets_root: PathBuf, gpu_ctx: Option<GpuThumbnailContext>) -> Self {
        let (request_tx, request_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();

        // Spawn worker thread
        let root = assets_root.clone();
        thread::spawn(move || {
            thumbnail_worker(request_rx, result_tx, root, gpu_ctx);
        });

        Self {
            cache: HashMap::new(),
            pending: std::collections::HashSet::new(),
            request_tx: Some(request_tx),
            result_rx,
            _assets_root: assets_root,
            placeholder: None,
            error_placeholder: None,
            _type_icons: HashMap::new(),
        }
    }

    /// Initialize placeholder textures
    fn ensure_placeholders(&mut self, ctx: &Context) {
        if self.placeholder.is_none() {
            let placeholder_image = create_placeholder_image(THUMBNAIL_SIZE, [60, 60, 60, 255]);
            self.placeholder = Some(ctx.load_texture(
                "thumb_placeholder",
                placeholder_image,
                TextureOptions::default(),
            ));
        }

        if self.error_placeholder.is_none() {
            let error_image = create_placeholder_image(THUMBNAIL_SIZE, [80, 40, 40, 255]);
            self.error_placeholder =
                Some(ctx.load_texture("thumb_error", error_image, TextureOptions::default()));
        }
    }

    /// Get or request thumbnail for an asset
    ///
    /// Returns the texture ID if available, or requests generation and returns placeholder.
    pub fn get_texture_id(&mut self, ctx: &Context, asset: &AssetMetadata) -> Option<TextureId> {
        self.ensure_placeholders(ctx);

        // Check cache
        if let Some(entry) = self.cache.get(&asset.id) {
            return Some(entry.texture.id());
        }

        // Check if already pending
        if self.pending.contains(&asset.id) {
            return self.placeholder.as_ref().map(|t| t.id());
        }

        // Request thumbnail generation
        self.request_thumbnail(asset);

        // Return placeholder while generating
        self.placeholder.as_ref().map(|t| t.id())
    }

    /// Get or request thumbnail for a built-in primitive mesh (Cube, Sphere, Plane).
    ///
    /// Uses deterministic `AssetId` derived from the primitive path so thumbnails
    /// are generated once and cached for the session lifetime.
    pub fn get_primitive_texture_id(&mut self, ctx: &Context, prim_path: &str) -> Option<TextureId> {
        self.ensure_placeholders(ctx);

        let id = AssetId::from_path(prim_path);

        if let Some(entry) = self.cache.get(&id) {
            return Some(entry.texture.id());
        }

        if self.pending.contains(&id) {
            return self.placeholder.as_ref().map(|t| t.id());
        }

        if let Some(tx) = &self.request_tx {
            let request = ThumbnailRequest {
                id,
                path: PathBuf::from(prim_path),
                asset_type: AssetType::Mesh,
            };
            if tx.send(request).is_ok() {
                self.pending.insert(id);
            }
        }

        self.placeholder.as_ref().map(|t| t.id())
    }

    /// Request thumbnail generation
    fn request_thumbnail(&mut self, asset: &AssetMetadata) {
        if !asset.asset_type.has_thumbnail() {
            return;
        }

        if let Some(tx) = &self.request_tx {
            let request = ThumbnailRequest {
                id: asset.id,
                path: asset.path.clone(),
                asset_type: asset.asset_type,
            };

            if tx.send(request).is_ok() {
                self.pending.insert(asset.id);
            }
        }
    }

    /// Poll for completed thumbnails
    ///
    /// Call this each frame to process completed thumbnail generations.
    pub fn poll(&mut self, ctx: &Context) {
        while let Ok(result) = self.result_rx.try_recv() {
            self.pending.remove(&result.id);

            if let Some(image_data) = result.image_data {
                let texture = ctx.load_texture(
                    format!("thumb_{}", result.id.0),
                    image_data,
                    TextureOptions::default(),
                );

                self.cache.insert(
                    result.id,
                    ThumbnailEntry {
                        texture,
                        source_hash: 0,
                    },
                );
            }
        }
    }

    /// Invalidate a thumbnail (e.g., after file modification)
    pub fn invalidate(&mut self, id: AssetId) {
        self.cache.remove(&id);
        self.pending.remove(&id);
    }

    /// Clear all cached thumbnails
    pub fn clear(&mut self) {
        self.cache.clear();
        self.pending.clear();
    }

    /// Get cache statistics
    pub fn stats(&self) -> ThumbnailCacheStats {
        ThumbnailCacheStats {
            cached: self.cache.len(),
            pending: self.pending.len(),
        }
    }
}

impl Drop for ThumbnailCache {
    fn drop(&mut self) {
        self.request_tx = None;
    }
}

/// Statistics about the thumbnail cache
#[derive(Debug, Clone, Copy)]
pub struct ThumbnailCacheStats {
    pub cached: usize,
    pub pending: usize,
}

/// Worker thread for thumbnail generation
fn thumbnail_worker(
    rx: mpsc::Receiver<ThumbnailRequest>,
    tx: mpsc::Sender<ThumbnailResult>,
    assets_root: PathBuf,
    gpu_ctx: Option<GpuThumbnailContext>,
) {
    use super::thumbnail_renderer::ThumbnailRenderer;

    // Lazily initialize the GPU renderer on first model request
    let mut renderer: Option<ThumbnailRenderer> = None;

    while let Ok(request) = rx.recv() {
        let result = generate_thumbnail(&request, &assets_root, &gpu_ctx, &mut renderer);
        if tx.send(result).is_err() {
            break;
        }
    }
}

/// Generate a thumbnail for an asset
fn generate_thumbnail(
    request: &ThumbnailRequest,
    assets_root: &Path,
    gpu_ctx: &Option<super::thumbnail_renderer::GpuThumbnailContext>,
    renderer: &mut Option<super::thumbnail_renderer::ThumbnailRenderer>,
) -> ThumbnailResult {
    // Handle built-in primitives (paths like "__primitive__/Cube")
    let path_str = request.path.to_string_lossy();
    if path_str.starts_with("__primitive__/") {
        return generate_primitive_thumbnail(request.id, &path_str, gpu_ctx, renderer);
    }

    let full_path = assets_root.join(&request.path);

    match request.asset_type {
        AssetType::Texture => generate_texture_thumbnail(request.id, &full_path),
        AssetType::Model | AssetType::Mesh => {
            generate_model_thumbnail(request.id, &full_path, gpu_ctx, renderer)
        }
        AssetType::Animation => {
            generate_anim_thumbnail(request.id, &full_path, gpu_ctx, renderer)
        }
        AssetType::Material => generate_material_thumbnail(request.id, &full_path),
        _ => ThumbnailResult {
            id: request.id,
            image_data: None,
        },
    }
}

/// Generate a 3D thumbnail for a built-in primitive mesh.
fn generate_primitive_thumbnail(
    id: AssetId,
    prim_path: &str,
    gpu_ctx: &Option<super::thumbnail_renderer::GpuThumbnailContext>,
    renderer: &mut Option<super::thumbnail_renderer::ThumbnailRenderer>,
) -> ThumbnailResult {
    use crate::engine::assets::model_loader::{compute_bounding_sphere, LoadedMesh, Model};
    use crate::engine::rendering::rendering_3d::mesh::{
        create_cube, create_plane, create_sphere, PRIMITIVE_CUBE, PRIMITIVE_PLANE,
        PRIMITIVE_SPHERE,
    };

    let (vertices, indices) = match prim_path {
        PRIMITIVE_CUBE => create_cube(),
        PRIMITIVE_SPHERE => create_sphere(32, 16),
        PRIMITIVE_PLANE => create_plane(1.0),
        _ => {
            return ThumbnailResult {
                id,
                image_data: None,
            };
        }
    };

    let (center, radius) = compute_bounding_sphere(&vertices);
    let (mut aabb_min, mut aabb_max) = (glam::Vec3::splat(f32::MAX), glam::Vec3::splat(f32::MIN));
    for v in &vertices {
        let p = glam::Vec3::from(v.position);
        aabb_min = aabb_min.min(p);
        aabb_max = aabb_max.max(p);
    }

    let mesh = LoadedMesh {
        vertices,
        indices,
        material_index: None,
        center,
        radius,
        aabb_min,
        aabb_max,
        skinning: None,
    };

    let model = Model {
        meshes: vec![mesh],
        name: prim_path.to_string(),
        textures: Vec::new(),
        materials: Vec::new(),
        bones: Vec::new(),
        animations: Vec::new(),
    };

    generate_model_from_loaded(id, model, gpu_ctx, renderer)
}

/// Generate thumbnail for a texture asset
fn generate_texture_thumbnail(id: AssetId, path: &Path) -> ThumbnailResult {
    match image::open(path) {
        Ok(img) => {
            let thumb = img.resize_exact(
                THUMBNAIL_SIZE,
                THUMBNAIL_SIZE,
                image::imageops::FilterType::Triangle,
            );

            let rgba = thumb.to_rgba8();
            let size = [THUMBNAIL_SIZE as usize, THUMBNAIL_SIZE as usize];
            let image_data = ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());

            ThumbnailResult {
                id,
                image_data: Some(image_data),
            }
        }
        Err(e) => {
            log::warn!("Thumbnail: failed to load texture {:?}: {}", path, e);
            ThumbnailResult {
                id,
                image_data: None,
            }
        }
    }
}

/// Generate thumbnail for a model asset using 3D rendering.
///
/// Loads the model via load_model(), renders it offscreen with the GPU,
/// and falls back to a placeholder icon if rendering fails.
fn generate_model_thumbnail(
    id: AssetId,
    path: &Path,
    gpu_ctx: &Option<super::thumbnail_renderer::GpuThumbnailContext>,
    renderer: &mut Option<super::thumbnail_renderer::ThumbnailRenderer>,
) -> ThumbnailResult {
    use crate::engine::assets::model_loader::load_model;
    use super::thumbnail_renderer::ThumbnailRenderer;

    let path_str = path.to_string_lossy();

    // Load the model geometry
    let model = match load_model(&path_str) {
        Ok(m) => m,
        Err(e) => {
            log::warn!("Thumbnail: failed to load model {:?}: {}", path, e);
            return ThumbnailResult {
                id,
                image_data: Some(create_model_icon_image()),
            };
        }
    };

    // Lazily initialize the GPU renderer
    if renderer.is_none() {
        if let Some(ctx) = gpu_ctx {
            match ThumbnailRenderer::new(ctx.clone_context()) {
                Ok(r) => {
                    log::info!("Thumbnail renderer initialized");
                    *renderer = Some(r);
                }
                Err(e) => {
                    log::warn!("Thumbnail: failed to initialize GPU renderer: {}", e);
                }
            }
        }
    }

    // Try GPU rendering
    if let Some(r) = renderer {
        match r.render_model(&model, THUMBNAIL_SIZE) {
            Ok(image_data) => {
                return ThumbnailResult {
                    id,
                    image_data: Some(image_data),
                };
            }
            Err(e) => {
                log::warn!("Thumbnail: GPU render failed for {:?}: {}", path, e);
            }
        }
    }

    // Fallback: placeholder icon
    ThumbnailResult {
        id,
        image_data: Some(create_model_icon_image()),
    }
}

/// Generate thumbnail for an animation asset.
///
/// Finds the sibling `.mesh` file (same stem), loads it, applies the first
/// frame of the animation as a skeletal pose, and renders a 3D thumbnail.
fn generate_anim_thumbnail(
    id: AssetId,
    anim_path: &Path,
    gpu_ctx: &Option<super::thumbnail_renderer::GpuThumbnailContext>,
    renderer: &mut Option<super::thumbnail_renderer::ThumbnailRenderer>,
) -> ThumbnailResult {
    use crate::engine::assets::mesh_import::load_anim_binary;
    use crate::engine::assets::model_loader::load_model;

    // Find sibling .mesh file (same stem, different extension)
    let mesh_path = anim_path.with_extension("mesh");
    if !mesh_path.exists() {
        log::warn!(
            "Thumbnail: no sibling .mesh for animation {:?}",
            anim_path
        );
        return ThumbnailResult {
            id,
            image_data: Some(create_model_icon_image()),
        };
    }

    // Load the mesh model (geometry + bones + skinning)
    let mesh_str = mesh_path.to_string_lossy();
    let mut model = match load_model(&mesh_str) {
        Ok(m) => m,
        Err(e) => {
            log::warn!("Thumbnail: failed to load mesh for anim {:?}: {}", mesh_path, e);
            return ThumbnailResult {
                id,
                image_data: Some(create_model_icon_image()),
            };
        }
    };

    // Load the animation data
    let (_bone_names, mut clips) = match load_anim_binary(anim_path) {
        Ok(data) => data,
        Err(e) => {
            log::warn!("Thumbnail: failed to load anim {:?}: {}", anim_path, e);
            // Fall back to rendering the mesh in bind pose
            return generate_model_from_loaded(id, model, gpu_ctx, renderer);
        }
    };

    // Undo double axis conversion on animation keyframes if needed.
    // load_mesh_binary already corrected the mesh data; the .anim keyframes
    // need the same correction to stay consistent.
    {
        use crate::engine::assets::mesh_import::{MeshImportMeta, UpAxis};
        let sidecar = std::path::PathBuf::from(format!("{}.ron", mesh_path.display()));
        if let Ok(text) = std::fs::read_to_string(&sidecar) {
            if let Ok(meta) = ron::from_str::<MeshImportMeta>(&text) {
                let src_ext = std::path::Path::new(&meta.source)
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if meta.settings.up_axis == UpAxis::ZUp
                    && matches!(src_ext.as_str(), "fbx" | "gltf" | "glb")
                {
                    for clip in &mut clips {
                        for ch in &mut clip.channels {
                            for (_, pos) in &mut ch.position_keys {
                                let y = pos.y;
                                pos.y = -pos.z;
                                pos.z = y;
                            }
                            for (_, rot) in &mut ch.rotation_keys {
                                let y = rot.y;
                                *rot = glam::Quat::from_xyzw(rot.x, -rot.z, y, rot.w);
                            }
                            for (_, scl) in &mut ch.scale_keys {
                                std::mem::swap(&mut scl.y, &mut scl.z);
                            }
                        }
                    }
                }
            }
        }
    }

    // Apply the first frame of the first clip if we have bones and skinning
    if let Some(clip) = clips.first() {
        if !model.bones.is_empty() {
            apply_first_frame_pose(&mut model, clip);
        }
    }

    generate_model_from_loaded(id, model, gpu_ctx, renderer)
}

/// Render thumbnail from an already-loaded model.
fn generate_model_from_loaded(
    id: AssetId,
    model: crate::engine::assets::model_loader::Model,
    gpu_ctx: &Option<super::thumbnail_renderer::GpuThumbnailContext>,
    renderer: &mut Option<super::thumbnail_renderer::ThumbnailRenderer>,
) -> ThumbnailResult {
    use super::thumbnail_renderer::ThumbnailRenderer;

    // Lazily initialize the GPU renderer
    if renderer.is_none() {
        if let Some(ctx) = gpu_ctx {
            match ThumbnailRenderer::new(ctx.clone_context()) {
                Ok(r) => {
                    log::info!("Thumbnail renderer initialized");
                    *renderer = Some(r);
                }
                Err(e) => {
                    log::warn!("Thumbnail: failed to initialize GPU renderer: {}", e);
                }
            }
        }
    }

    if let Some(r) = renderer {
        match r.render_model(&model, THUMBNAIL_SIZE) {
            Ok(image_data) => {
                return ThumbnailResult {
                    id,
                    image_data: Some(image_data),
                };
            }
            Err(e) => {
                log::warn!("Thumbnail: GPU render failed: {}", e);
            }
        }
    }

    ThumbnailResult {
        id,
        image_data: Some(create_model_icon_image()),
    }
}

/// Apply the first frame of an animation clip to a model's vertices via CPU skinning.
fn apply_first_frame_pose(
    model: &mut crate::engine::assets::model_loader::Model,
    clip: &crate::engine::assets::model_loader::RawAnimationClip,
) {
    use glam::{Mat4, Quat, Vec3};

    let bone_count = model.bones.len();
    if bone_count == 0 {
        return;
    }

    // Derive rest-pose local transforms from inverse bind matrices.
    // Bones without animation channels keep their rest pose instead of
    // collapsing to identity (which would scatter the mesh).
    let mut local_transforms: Vec<Mat4> = (0..bone_count)
        .map(|i| {
            let bind_matrix = model.bones[i].inverse_bind_matrix.inverse();
            let parent_bind = model.bones[i]
                .parent_index
                .map(|p| model.bones[p].inverse_bind_matrix.inverse())
                .unwrap_or(Mat4::IDENTITY);
            parent_bind.inverse() * bind_matrix
        })
        .collect();

    // Override with the first keyframe for bones that have animation channels
    for channel in &clip.channels {
        if channel.bone_index >= bone_count {
            continue;
        }
        let t = channel
            .position_keys
            .first()
            .map(|k| k.1)
            .unwrap_or(Vec3::ZERO);
        let r = channel
            .rotation_keys
            .first()
            .map(|k| k.1)
            .unwrap_or(Quat::IDENTITY);
        let s = channel
            .scale_keys
            .first()
            .map(|k| k.1)
            .unwrap_or(Vec3::ONE);
        local_transforms[channel.bone_index] =
            Mat4::from_scale_rotation_translation(s, r, t);
    }

    // Compute world transforms by walking the parent hierarchy (bones are sorted parent-first)
    let mut world_transforms = vec![Mat4::IDENTITY; bone_count];
    for i in 0..bone_count {
        let local = local_transforms[i];
        world_transforms[i] = match model.bones[i].parent_index {
            Some(parent) => world_transforms[parent] * local,
            None => local,
        };
    }

    // Final skinning matrices: world_transform * inverse_bind_matrix
    let skin_matrices: Vec<Mat4> = (0..bone_count)
        .map(|i| world_transforms[i] * model.bones[i].inverse_bind_matrix)
        .collect();

    // Apply skinning to each mesh that has skinning data
    for mesh in &mut model.meshes {
        let skinning = match &mesh.skinning {
            Some(s) => s,
            None => continue,
        };

        for (vi, vertex) in mesh.vertices.iter_mut().enumerate() {
            let bone_data = &skinning[vi];
            let pos = Vec3::from(vertex.position);
            let norm = Vec3::from(vertex.normal);

            let mut skinned_pos = Vec3::ZERO;
            let mut skinned_norm = Vec3::ZERO;

            for j in 0..4 {
                let w = bone_data.weights[j];
                if w < 1e-6 {
                    continue;
                }
                let idx = bone_data.joints[j] as usize;
                if idx >= bone_count {
                    continue;
                }
                let m = skin_matrices[idx];
                skinned_pos += w * m.transform_point3(pos);
                skinned_norm += w * m.transform_vector3(norm);
            }

            vertex.position = skinned_pos.into();
            let len = skinned_norm.length();
            if len > 1e-6 {
                vertex.normal = (skinned_norm / len).into();
            }
        }

        // Recompute bounding sphere after skinning
        let (center, radius) =
            crate::engine::assets::model_loader::compute_bounding_sphere(&mesh.vertices);
        mesh.center = center;
        mesh.radius = radius;
    }
}

/// Generate thumbnail for a material asset.
///
/// If the material has an albedo texture, display it as the thumbnail.
/// Otherwise, render a colored swatch using the base_color_factor.
fn generate_material_thumbnail(id: AssetId, path: &Path) -> ThumbnailResult {
    use crate::engine::assets::mesh_import::load_material_ron;

    let mat = match load_material_ron(path) {
        Ok(m) => m,
        Err(e) => {
            log::warn!("Thumbnail: failed to load material {:?}: {}", path, e);
            return ThumbnailResult {
                id,
                image_data: None,
            };
        }
    };

    // Try albedo texture first (resolve relative to material file's directory)
    if !mat.albedo_texture.is_empty() {
        if let Some(parent) = path.parent() {
            let tex_path = parent.join(&mat.albedo_texture);
            if tex_path.exists() {
                if let Ok(img) = image::open(&tex_path) {
                    let thumb = img.resize_exact(
                        THUMBNAIL_SIZE,
                        THUMBNAIL_SIZE,
                        image::imageops::FilterType::Triangle,
                    );
                    let rgba = thumb.to_rgba8();
                    let size = [THUMBNAIL_SIZE as usize, THUMBNAIL_SIZE as usize];
                    return ThumbnailResult {
                        id,
                        image_data: Some(ColorImage::from_rgba_unmultiplied(size, rgba.as_raw())),
                    };
                }
            }
        }
    }

    // Fallback: solid color swatch from base_color_factor
    let [r, g, b, a] = mat.base_color_factor;
    let color = [
        (r * 255.0) as u8,
        (g * 255.0) as u8,
        (b * 255.0) as u8,
        (a * 255.0) as u8,
    ];
    ThumbnailResult {
        id,
        image_data: Some(create_placeholder_image(THUMBNAIL_SIZE, color)),
    }
}

/// Create a placeholder image
fn create_placeholder_image(size: u32, color: [u8; 4]) -> ColorImage {
    let pixel_count = (size * size) as usize;
    let mut rgba_data = Vec::with_capacity(pixel_count * 4);
    for _ in 0..pixel_count {
        rgba_data.extend_from_slice(&color);
    }

    ColorImage::from_rgba_unmultiplied([size as usize, size as usize], &rgba_data)
}

/// Create a model icon placeholder image (fallback when GPU rendering unavailable)
fn create_model_icon_image() -> ColorImage {
    let size = THUMBNAIL_SIZE as usize;
    let mut pixels = vec![egui::Color32::from_gray(40); size * size];

    let center = size / 2;
    let cube_size = size / 3;
    let color = egui::Color32::from_rgb(100, 150, 220);

    for i in 0..cube_size {
        pixels[(center - cube_size / 2) * size + center - cube_size / 2 + i] = color;
        pixels[(center + cube_size / 2) * size + center - cube_size / 2 + i] = color;
        pixels[(center - cube_size / 2 + i) * size + center - cube_size / 2] = color;
        pixels[(center - cube_size / 2 + i) * size + center + cube_size / 2] = color;
    }

    let mut rgba_data = Vec::with_capacity(pixels.len() * 4);
    for pixel in &pixels {
        rgba_data.push(pixel.r());
        rgba_data.push(pixel.g());
        rgba_data.push(pixel.b());
        rgba_data.push(pixel.a());
    }

    ColorImage::from_rgba_unmultiplied([size, size], &rgba_data)
}
