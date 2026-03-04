//! Thumbnail cache for asset browser
//!
//! Provides async thumbnail generation with memory and disk caching.
//! Thumbnails are generated in a background thread to avoid blocking the UI.

use crate::engine::assets::{AssetId, AssetMetadata, AssetType};
use egui::{ColorImage, Context, TextureHandle, TextureId, TextureOptions};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;

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
#[derive(Debug)]
struct ThumbnailResult {
    id: AssetId,
    image_data: Option<ColorImage>,
    _error: Option<String>,
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
    /// Create a new thumbnail cache
    pub fn new(assets_root: PathBuf) -> Self {
        let (request_tx, request_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();

        // Spawn worker thread
        let root = assets_root.clone();
        thread::spawn(move || {
            thumbnail_worker(request_rx, result_tx, root);
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
            // Create loading placeholder (gray with pattern)
            let placeholder_image = create_placeholder_image(THUMBNAIL_SIZE, [60, 60, 60, 255]);
            self.placeholder = Some(ctx.load_texture(
                "thumb_placeholder",
                placeholder_image,
                TextureOptions::default(),
            ));
        }

        if self.error_placeholder.is_none() {
            // Create error placeholder (red-ish)
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

    /// Request thumbnail generation
    fn request_thumbnail(&mut self, asset: &AssetMetadata) {
        // Only generate for types that support thumbnails
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
        // Process all available results
        while let Ok(result) = self.result_rx.try_recv() {
            self.pending.remove(&result.id);

            if let Some(image_data) = result.image_data {
                // Create texture from image data
                let texture = ctx.load_texture(
                    format!("thumb_{}", result.id.0),
                    image_data,
                    TextureOptions::default(),
                );

                self.cache.insert(
                    result.id,
                    ThumbnailEntry {
                        texture,
                        source_hash: 0, // TODO: Compute actual hash
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
        // Drop the sender to signal worker thread to exit
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
) {
    while let Ok(request) = rx.recv() {
        let result = generate_thumbnail(&request, &assets_root);
        if tx.send(result).is_err() {
            // Main thread has dropped, exit
            break;
        }
    }
}

/// Generate a thumbnail for an asset
fn generate_thumbnail(request: &ThumbnailRequest, assets_root: &Path) -> ThumbnailResult {
    let full_path = assets_root.join(&request.path);

    match request.asset_type {
        AssetType::Texture => generate_texture_thumbnail(request.id, &full_path),
        AssetType::Model => generate_model_thumbnail(request.id, &full_path),
        _ => ThumbnailResult {
            id: request.id,
            image_data: None,
            _error: Some("Unsupported asset type for thumbnails".to_string()),
        },
    }
}

/// Generate thumbnail for a texture asset
fn generate_texture_thumbnail(id: AssetId, path: &Path) -> ThumbnailResult {
    match image::open(path) {
        Ok(img) => {
            // Resize to thumbnail size
            let thumb = img.resize_exact(
                THUMBNAIL_SIZE,
                THUMBNAIL_SIZE,
                image::imageops::FilterType::Triangle,
            );

            // Convert to egui ColorImage
            let rgba = thumb.to_rgba8();
            let size = [THUMBNAIL_SIZE as usize, THUMBNAIL_SIZE as usize];
            let image_data = ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());

            ThumbnailResult {
                id,
                image_data: Some(image_data),
                _error: None,
            }
        }
        Err(e) => ThumbnailResult {
            id,
            image_data: None,
            _error: Some(format!("Failed to load image: {}", e)),
        },
    }
}

/// Generate thumbnail for a model asset
///
/// For now, attempts to extract the first texture from GLTF/GLB files.
/// Full 3D rendering could be added later.
fn generate_model_thumbnail(id: AssetId, path: &Path) -> ThumbnailResult {
    // Try to load GLTF and extract first texture
    match gltf::import(path) {
        Ok((document, buffers, _images)) => {
            // Try to find first image in the document
            for image in document.images() {
                match image.source() {
                    gltf::image::Source::View { view, mime_type: _ } => {
                        let buffer = &buffers[view.buffer().index()];
                        let offset = view.offset();
                        let length = view.length();
                        let data = &buffer[offset..offset + length];

                        // Try to decode the embedded image
                        if let Ok(img) = image::load_from_memory(data) {
                            let thumb = img.resize_exact(
                                THUMBNAIL_SIZE,
                                THUMBNAIL_SIZE,
                                image::imageops::FilterType::Triangle,
                            );

                            let rgba = thumb.to_rgba8();
                            let size = [THUMBNAIL_SIZE as usize, THUMBNAIL_SIZE as usize];
                            let image_data =
                                ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());

                            return ThumbnailResult {
                                id,
                                image_data: Some(image_data),
                                _error: None,
                            };
                        }
                    }
                    gltf::image::Source::Uri { uri, mime_type: _ } => {
                        // Try to load external image
                        if let Some(parent) = path.parent() {
                            let image_path = parent.join(uri);
                            if let Ok(img) = image::open(&image_path) {
                                let thumb = img.resize_exact(
                                    THUMBNAIL_SIZE,
                                    THUMBNAIL_SIZE,
                                    image::imageops::FilterType::Triangle,
                                );

                                let rgba = thumb.to_rgba8();
                                let size = [THUMBNAIL_SIZE as usize, THUMBNAIL_SIZE as usize];
                                let image_data =
                                    ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());

                                return ThumbnailResult {
                                    id,
                                    image_data: Some(image_data),
                                    _error: None,
                                };
                            }
                        }
                    }
                }
            }

            // No texture found, create a placeholder model icon
            ThumbnailResult {
                id,
                image_data: Some(create_model_icon_image()),
                _error: None,
            }
        }
        Err(e) => ThumbnailResult {
            id,
            image_data: None,
            _error: Some(format!("Failed to load model: {}", e)),
        },
    }
}

/// Create a placeholder image
fn create_placeholder_image(size: u32, color: [u8; 4]) -> ColorImage {
    // Create RGBA bytes for entire image
    let pixel_count = (size * size) as usize;
    let mut rgba_data = Vec::with_capacity(pixel_count * 4);
    for _ in 0..pixel_count {
        rgba_data.extend_from_slice(&color);
    }

    ColorImage::from_rgba_unmultiplied([size as usize, size as usize], &rgba_data)
}

/// Create a model icon placeholder image
fn create_model_icon_image() -> ColorImage {
    let size = THUMBNAIL_SIZE as usize;
    let mut pixels = vec![egui::Color32::from_gray(40); size * size];

    // Draw a simple cube outline
    let center = size / 2;
    let cube_size = size / 3;

    // Draw simple lines for cube edges
    let color = egui::Color32::from_rgb(100, 150, 220);

    // Front face (square)
    for i in 0..cube_size {
        // Top
        pixels[(center - cube_size / 2) * size + center - cube_size / 2 + i] = color;
        // Bottom
        pixels[(center + cube_size / 2) * size + center - cube_size / 2 + i] = color;
        // Left
        pixels[(center - cube_size / 2 + i) * size + center - cube_size / 2] = color;
        // Right
        pixels[(center - cube_size / 2 + i) * size + center + cube_size / 2] = color;
    }

    // Convert to RGBA bytes
    let mut rgba_data = Vec::with_capacity(pixels.len() * 4);
    for pixel in &pixels {
        rgba_data.push(pixel.r());
        rgba_data.push(pixel.g());
        rgba_data.push(pixel.b());
        rgba_data.push(pixel.a());
    }

    ColorImage::from_rgba_unmultiplied([size, size], &rgba_data)
}
