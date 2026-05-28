//! Editor asset preview registry.
//!
//! The registry owns stable preview identifiers and optional egui texture IDs.
//! GPU producers can attach or update textures when available; the editor also
//! has built-in preview producers for texture/material/material-instance assets.

use crate::engine::assets::mesh_import::{load_material_ron, MaterialDefinition};
use crate::engine::rendering::rendering_3d::MaterialInstanceDef;
use egui::{Color32, ColorImage, Pos2, Rect, TextureOptions, Vec2};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const PREVIEW_SIZE: usize = 256;
const CHECKER_SIZE: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PreviewId(u64);

impl PreviewId {
    pub fn get(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PreviewKind {
    Material,
    MaterialInstance,
    Texture,
}

#[derive(Debug, Clone)]
pub struct AssetPreview {
    pub id: PreviewId,
    pub kind: PreviewKind,
    pub key: String,
    pub source_path: PathBuf,
    pub texture_id: Option<egui::TextureId>,
    pub dirty: bool,
}

pub struct AssetPreviewRegistry {
    next_id: u64,
    previews: HashMap<PreviewId, AssetPreview>,
    by_key: HashMap<(PreviewKind, String), PreviewId>,
    local_textures: HashMap<PreviewId, egui::TextureHandle>,
}

impl std::fmt::Debug for AssetPreviewRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AssetPreviewRegistry")
            .field("next_id", &self.next_id)
            .field("previews", &self.previews)
            .field("by_key", &self.by_key)
            .field("local_texture_count", &self.local_textures.len())
            .finish()
    }
}

impl Default for AssetPreviewRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl AssetPreviewRegistry {
    pub fn new() -> Self {
        Self {
            next_id: 1,
            previews: HashMap::new(),
            by_key: HashMap::new(),
            local_textures: HashMap::new(),
        }
    }

    pub fn material_preview<P: AsRef<Path>>(&mut self, path: P) -> PreviewId {
        self.preview_for(PreviewKind::Material, path)
    }

    pub fn material_instance_preview<P: AsRef<Path>>(&mut self, path: P) -> PreviewId {
        self.preview_for(PreviewKind::MaterialInstance, path)
    }

    pub fn texture_preview<P: AsRef<Path>>(&mut self, path: P) -> PreviewId {
        self.preview_for(PreviewKind::Texture, path)
    }

    pub fn texture_for(&self, id: PreviewId) -> Option<egui::TextureId> {
        self.previews
            .get(&id)
            .and_then(|preview| preview.texture_id)
    }

    pub fn set_texture(&mut self, id: PreviewId, texture_id: egui::TextureId) {
        if let Some(preview) = self.previews.get_mut(&id) {
            self.local_textures.remove(&id);
            preview.texture_id = Some(texture_id);
            preview.dirty = false;
        }
    }

    pub fn invalidate(&mut self, id: PreviewId) {
        if let Some(preview) = self.previews.get_mut(&id) {
            preview.dirty = true;
        }
    }

    pub fn ensure_texture(
        &mut self,
        ctx: &egui::Context,
        id: PreviewId,
    ) -> Option<egui::TextureId> {
        let should_render = self
            .previews
            .get(&id)
            .map(|preview| preview.dirty || preview.texture_id.is_none())
            .unwrap_or(false);

        if should_render {
            self.render_local_preview(ctx, id);
        }

        self.texture_for(id)
    }

    pub fn render_preview(
        &mut self,
        ui: &mut egui::Ui,
        id: PreviewId,
        size: Vec2,
    ) -> egui::Response {
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::hover());
        if !ui.is_rect_visible(rect) {
            return response;
        }

        paint_checkerboard(ui.painter(), rect, CHECKER_SIZE as f32);

        if let Some(texture_id) = self.ensure_texture(ui.ctx(), id) {
            ui.painter().image(
                texture_id,
                rect,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        } else {
            ui.painter().rect_stroke(
                rect,
                4.0,
                egui::Stroke::new(1.0, Color32::from_gray(80)),
                egui::epaint::StrokeKind::Inside,
            );
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Preview unavailable",
                egui::FontId::proportional(13.0),
                Color32::from_gray(150),
            );
        }

        response
    }

    pub fn is_dirty(&self, id: PreviewId) -> bool {
        self.previews
            .get(&id)
            .map(|preview| preview.dirty)
            .unwrap_or(false)
    }

    pub fn get(&self, id: PreviewId) -> Option<&AssetPreview> {
        self.previews.get(&id)
    }

    pub fn remove(&mut self, id: PreviewId) -> Option<AssetPreview> {
        self.local_textures.remove(&id);
        let preview = self.previews.remove(&id)?;
        self.by_key.remove(&(preview.kind, preview.key.clone()));
        Some(preview)
    }

    fn render_local_preview(&mut self, ctx: &egui::Context, id: PreviewId) {
        let Some(preview) = self.previews.get(&id).cloned() else {
            return;
        };

        let image = match preview.kind {
            PreviewKind::Texture => render_texture_preview(&preview.source_path),
            PreviewKind::Material => render_material_preview(&preview.source_path),
            PreviewKind::MaterialInstance => render_material_instance_preview(&preview.source_path),
        };

        match image {
            Some(image) => {
                let texture = ctx.load_texture(
                    format!(
                        "asset_preview_{}_{}",
                        preview.kind.as_str(),
                        preview.id.get()
                    ),
                    image,
                    TextureOptions::default(),
                );
                let texture_id = texture.id();
                self.local_textures.insert(id, texture);
                if let Some(current) = self.previews.get_mut(&id) {
                    current.texture_id = Some(texture_id);
                    current.dirty = false;
                }
            }
            None => {
                if let Some(current) = self.previews.get_mut(&id) {
                    current.dirty = false;
                }
            }
        }
    }

    fn preview_for<P: AsRef<Path>>(&mut self, kind: PreviewKind, path: P) -> PreviewId {
        let source_path = path.as_ref().to_path_buf();
        let key = normalize_preview_key(&source_path);
        if let Some(id) = self.by_key.get(&(kind, key.clone())) {
            return *id;
        }

        let id = PreviewId(self.next_id);
        self.next_id = self.next_id.saturating_add(1);
        self.previews.insert(
            id,
            AssetPreview {
                id,
                kind,
                key: key.clone(),
                source_path,
                texture_id: None,
                dirty: true,
            },
        );
        self.by_key.insert((kind, key), id);
        id
    }
}

impl PreviewKind {
    fn as_str(self) -> &'static str {
        match self {
            PreviewKind::Material => "material",
            PreviewKind::MaterialInstance => "material_instance",
            PreviewKind::Texture => "texture",
        }
    }
}

fn normalize_preview_key(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/").to_lowercase()
}

fn render_texture_preview(path: &Path) -> Option<ColorImage> {
    let image = image::open(path).ok()?.to_rgba8();
    let resized = image::imageops::resize(
        &image,
        PREVIEW_SIZE as u32,
        PREVIEW_SIZE as u32,
        image::imageops::FilterType::Triangle,
    );

    Some(composite_on_checker(&resized, PREVIEW_SIZE, CHECKER_SIZE))
}

fn render_material_preview(path: &Path) -> Option<ColorImage> {
    let material = load_material_ron(path).ok()?;
    let albedo = load_material_albedo(path, &material);
    Some(render_material_sphere(
        material.base_color_factor,
        material.metallic_factor,
        material.roughness_factor,
        material.emissive_factor,
        albedo.as_ref(),
    ))
}

fn render_material_instance_preview(path: &Path) -> Option<ColorImage> {
    let instance = MaterialInstanceDef::load(path).ok()?;
    let base_path = resolve_material_reference(path, &instance.base_material);
    let base = load_material_ron(&base_path).ok();

    let base_color = base
        .as_ref()
        .map(|material| material.base_color_factor)
        .unwrap_or([1.0, 1.0, 1.0, 1.0]);
    let metallic = base
        .as_ref()
        .map(|material| material.metallic_factor)
        .unwrap_or(1.0)
        * instance.metallic_factor;
    let roughness = base
        .as_ref()
        .map(|material| material.roughness_factor)
        .unwrap_or(0.5)
        * instance.roughness_factor;
    let emissive = base
        .as_ref()
        .map(|material| {
            [
                material.emissive_factor[0] + instance.emissive_factor[0],
                material.emissive_factor[1] + instance.emissive_factor[1],
                material.emissive_factor[2] + instance.emissive_factor[2],
            ]
        })
        .unwrap_or(instance.emissive_factor);
    let color = [
        base_color[0] * instance.base_color_factor[0],
        base_color[1] * instance.base_color_factor[1],
        base_color[2] * instance.base_color_factor[2],
        base_color[3] * instance.base_color_factor[3],
    ];

    let albedo = base
        .as_ref()
        .and_then(|material| load_material_albedo(&base_path, material));
    Some(render_material_sphere(
        color,
        metallic,
        roughness,
        emissive,
        albedo.as_ref(),
    ))
}

fn load_material_albedo(path: &Path, material: &MaterialDefinition) -> Option<image::RgbaImage> {
    if material.albedo_texture.is_empty() {
        return None;
    }

    let texture_path = path.parent()?.join(&material.albedo_texture);
    image::open(texture_path).ok().map(|image| image.to_rgba8())
}

fn resolve_material_reference(instance_path: &Path, material_ref: &str) -> PathBuf {
    let referenced = PathBuf::from(material_ref);
    if referenced.is_absolute() && referenced.exists() {
        return referenced;
    }

    let content_path = PathBuf::from("content").join(&referenced);
    if content_path.exists() {
        return content_path;
    }

    instance_path
        .parent()
        .map(|parent| parent.join(&referenced))
        .unwrap_or(referenced)
}

fn render_material_sphere(
    base_color: [f32; 4],
    metallic: f32,
    roughness: f32,
    emissive: [f32; 3],
    albedo: Option<&image::RgbaImage>,
) -> ColorImage {
    let size = PREVIEW_SIZE;
    let mut rgba = Vec::with_capacity(size * size * 4);
    let roughness = roughness.clamp(0.04, 1.0);
    let metallic = metallic.clamp(0.0, 1.0);

    for y in 0..size {
        for x in 0..size {
            let checker = checker_color(x, y, CHECKER_SIZE);
            let nx = ((x as f32 + 0.5) / size as f32) * 2.0 - 1.0;
            let ny = ((y as f32 + 0.5) / size as f32) * 2.0 - 1.0;
            let radius2 = nx * nx + ny * ny;

            if radius2 > 0.86 {
                rgba.extend_from_slice(&checker);
                continue;
            }

            let nz = (1.0 - radius2).sqrt();
            let normal = [nx, -ny, nz];
            let mut color = sample_sphere_base_color(base_color, normal, albedo);

            let key = lambert(normal, normalize([-0.45, 0.55, 0.75])) * 0.75;
            let fill = lambert(normal, normalize([0.65, -0.25, 0.55])) * 0.25;
            let rim = (1.0 - nz).powf(2.2) * 0.22;
            let ambient = 0.18;
            let diffuse = ambient + key + fill + rim;

            let half_vec = normalize([-0.25, 0.35, 1.75]);
            let spec_power = 10.0 + (1.0 - roughness) * 120.0;
            let specular = lambert(normal, half_vec).powf(spec_power)
                * (0.18 + metallic * 0.45)
                * (1.15 - roughness);

            color[0] = color[0] * diffuse + specular + emissive[0];
            color[1] = color[1] * diffuse + specular + emissive[1];
            color[2] = color[2] * diffuse + specular + emissive[2];

            let alpha = base_color[3].clamp(0.0, 1.0);
            let shaded = [
                to_u8(color[0]),
                to_u8(color[1]),
                to_u8(color[2]),
                to_u8(alpha),
            ];
            rgba.extend_from_slice(&alpha_blend(checker, shaded));
        }
    }

    ColorImage::from_rgba_unmultiplied([size, size], &rgba)
}

fn sample_sphere_base_color(
    base_color: [f32; 4],
    normal: [f32; 3],
    albedo: Option<&image::RgbaImage>,
) -> [f32; 3] {
    let mut color = [base_color[0], base_color[1], base_color[2]];
    if let Some(albedo) = albedo {
        let u = 0.5 + normal[0].atan2(normal[2]) / std::f32::consts::TAU;
        let v = 0.5 - normal[1].asin() / std::f32::consts::PI;
        let x = ((u.fract() * albedo.width() as f32) as u32).min(albedo.width().saturating_sub(1));
        let y = ((v.clamp(0.0, 1.0) * albedo.height() as f32) as u32)
            .min(albedo.height().saturating_sub(1));
        let pixel = albedo.get_pixel(x, y).0;
        color[0] *= pixel[0] as f32 / 255.0;
        color[1] *= pixel[1] as f32 / 255.0;
        color[2] *= pixel[2] as f32 / 255.0;
    }
    color
}

fn composite_on_checker(image: &image::RgbaImage, size: usize, checker_size: usize) -> ColorImage {
    let mut rgba = Vec::with_capacity(size * size * 4);
    for y in 0..size {
        for x in 0..size {
            let background = checker_color(x, y, checker_size);
            let foreground = image.get_pixel(x as u32, y as u32).0;
            rgba.extend_from_slice(&alpha_blend(background, foreground));
        }
    }

    ColorImage::from_rgba_unmultiplied([size, size], &rgba)
}

fn paint_checkerboard(painter: &egui::Painter, rect: Rect, checker_size: f32) {
    let mut y = rect.top();
    let mut row = 0;
    while y < rect.bottom() {
        let mut x = rect.left();
        let mut col = 0;
        while x < rect.right() {
            let tile = Rect::from_min_max(
                Pos2::new(x, y),
                Pos2::new(
                    (x + checker_size).min(rect.right()),
                    (y + checker_size).min(rect.bottom()),
                ),
            );
            let color = if (row + col) % 2 == 0 {
                Color32::from_gray(42)
            } else {
                Color32::from_gray(58)
            };
            painter.rect_filled(tile, 0.0, color);
            x += checker_size;
            col += 1;
        }
        y += checker_size;
        row += 1;
    }
}

fn checker_color(x: usize, y: usize, checker_size: usize) -> [u8; 4] {
    if ((x / checker_size) + (y / checker_size)) & 1 == 0 {
        [42, 42, 42, 255]
    } else {
        [58, 58, 58, 255]
    }
}

fn alpha_blend(background: [u8; 4], foreground: [u8; 4]) -> [u8; 4] {
    let alpha = foreground[3] as f32 / 255.0;
    [
        to_u8(
            (foreground[0] as f32 / 255.0) * alpha + (background[0] as f32 / 255.0) * (1.0 - alpha),
        ),
        to_u8(
            (foreground[1] as f32 / 255.0) * alpha + (background[1] as f32 / 255.0) * (1.0 - alpha),
        ),
        to_u8(
            (foreground[2] as f32 / 255.0) * alpha + (background[2] as f32 / 255.0) * (1.0 - alpha),
        ),
        255,
    ]
}

fn lambert(a: [f32; 3], b: [f32; 3]) -> f32 {
    (a[0] * b[0] + a[1] * b[1] + a[2] * b[2]).max(0.0)
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len <= f32::EPSILON {
        [0.0, 0.0, 1.0]
    } else {
        [v[0] / len, v[1] / len, v[2] / len]
    }
}

fn to_u8(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}
