//! Icon management for the editor toolbar
//!
//! This module provides PNG icon loading and rendering for the viewport toolbar.
//! Icons are loaded from engine/icons/ directory (separate from game content).

use egui::{Color32, ColorImage, Context, TextureHandle, TextureOptions, Vec2};
use std::collections::HashMap;
use std::path::Path;

/// Toolbar icon identifiers
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolbarIcon {
    // Tool mode icons
    Select,
    Translate,
    Rotate,
    Scale,
    // Coordinate system icons
    World,
    Local,
    // Snap toggle icons
    GridSnap,
    RotationSnap,
    ScaleSnap,
    // Camera icons
    CameraSpeed,
}

/// Asset browser icon identifiers
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AssetBrowserIcon {
    // Folder icons
    Folder,
    FolderOpen,
    FolderPlus,
    // File type icons
    FileMesh,
    FileImage,
    FileDocument,
    FileCode,
    // Arrow icons for expand/collapse
    ArrowDown,
    ArrowRight,
}

impl AssetBrowserIcon {
    /// Get the filename for this icon (without extension)
    pub fn filename(&self) -> &'static str {
        match self {
            AssetBrowserIcon::Folder => "folder",
            AssetBrowserIcon::FolderOpen => "opened-folder",
            AssetBrowserIcon::FolderPlus => "folder-plus-fill",
            AssetBrowserIcon::FileMesh => "file-mesh",
            AssetBrowserIcon::FileImage => "image-file",
            AssetBrowserIcon::FileDocument => "file-document",
            AssetBrowserIcon::FileCode => "code-file",
            AssetBrowserIcon::ArrowDown => "arrow-down",
            AssetBrowserIcon::ArrowRight => "arrow-right",
        }
    }

    /// Get all asset browser icons
    pub fn all() -> &'static [AssetBrowserIcon] {
        &[
            AssetBrowserIcon::Folder,
            AssetBrowserIcon::FolderOpen,
            AssetBrowserIcon::FolderPlus,
            AssetBrowserIcon::FileMesh,
            AssetBrowserIcon::FileImage,
            AssetBrowserIcon::FileDocument,
            AssetBrowserIcon::FileCode,
            AssetBrowserIcon::ArrowDown,
            AssetBrowserIcon::ArrowRight,
        ]
    }
}

impl ToolbarIcon {
    /// Get the filename for this icon (without extension)
    pub fn filename(&self) -> &'static str {
        match self {
            ToolbarIcon::Select => "select",
            ToolbarIcon::Translate => "translate",
            ToolbarIcon::Rotate => "rotate",
            ToolbarIcon::Scale => "scale",
            ToolbarIcon::World => "world",
            ToolbarIcon::Local => "local",
            ToolbarIcon::GridSnap => "grid_snap",
            ToolbarIcon::RotationSnap => "rotation_snap",
            ToolbarIcon::ScaleSnap => "scale_snap",
            ToolbarIcon::CameraSpeed => "camera_speed",
        }
    }

    /// Get the tooltip text for this icon
    pub fn tooltip(&self) -> &'static str {
        match self {
            ToolbarIcon::Select => "Select (Q)",
            ToolbarIcon::Translate => "Translate (W)",
            ToolbarIcon::Rotate => "Rotate (E)",
            ToolbarIcon::Scale => "Scale (R)",
            ToolbarIcon::World => "World Space",
            ToolbarIcon::Local => "Local Space",
            ToolbarIcon::GridSnap => "Grid Snap",
            ToolbarIcon::RotationSnap => "Rotation Snap",
            ToolbarIcon::ScaleSnap => "Scale Snap",
            ToolbarIcon::CameraSpeed => "Camera Speed",
        }
    }
}

/// Manages toolbar icons loaded from PNG files
pub struct IconManager {
    /// Loaded texture handles indexed by icon type
    textures: HashMap<ToolbarIcon, TextureHandle>,
    /// Loaded asset browser icons
    asset_icons: HashMap<AssetBrowserIcon, TextureHandle>,
    /// Icon display size
    icon_size: u32,
    /// Icon tint color
    tint_color: Color32,
}

impl IconManager {
    /// Create a new icon manager with the specified icon size and tint color
    pub fn new(icon_size: u32, tint_color: Color32) -> Self {
        Self {
            textures: HashMap::new(),
            asset_icons: HashMap::new(),
            icon_size,
            tint_color,
        }
    }

    /// Load toolbar icons from the assets directory
    pub fn load_toolbar_icons(&mut self, ctx: &Context, assets_path: &Path) {
        let icons_dir = assets_path.join("icons");

        // List of all toolbar icons to load
        let icons = [
            ToolbarIcon::Select,
            ToolbarIcon::Translate,
            ToolbarIcon::Rotate,
            ToolbarIcon::Scale,
            ToolbarIcon::World,
            ToolbarIcon::Local,
            ToolbarIcon::GridSnap,
            ToolbarIcon::RotationSnap,
            ToolbarIcon::ScaleSnap,
            ToolbarIcon::CameraSpeed,
        ];

        for icon in icons {
            let filename = format!("{}.png", icon.filename());
            let path = icons_dir.join(&filename);

            if let Some(texture) = self.load_png_icon(ctx, &path, icon.filename()) {
                self.textures.insert(icon, texture);
            } else {
                eprintln!("Warning: Failed to load icon: {}", path.display());
            }
        }
    }

    /// Load asset browser icons from the assets directory
    pub fn load_asset_browser_icons(&mut self, ctx: &Context, assets_path: &Path) {
        let icons_dir = assets_path.join("icons");

        for icon in AssetBrowserIcon::all() {
            let filename = format!("{}.png", icon.filename());
            let path = icons_dir.join(&filename);

            if let Some(texture) = self.load_png_icon(ctx, &path, icon.filename()) {
                self.asset_icons.insert(*icon, texture);
            } else {
                eprintln!("Warning: Failed to load asset browser icon: {}", path.display());
            }
        }
    }

    /// Load a single PNG icon and return a texture handle
    fn load_png_icon(&self, ctx: &Context, path: &Path, name: &str) -> Option<TextureHandle> {
        // Read the file
        let data = std::fs::read(path).ok()?;

        // Decode the PNG
        let image = image::load_from_memory(&data).ok()?;
        let rgba = image.to_rgba8();
        let (width, height) = rgba.dimensions();

        // Apply tint and convert to egui ColorImage
        let raw = rgba.into_raw();
        let tinted: Vec<u8> = raw
            .chunks(4)
            .flat_map(|p| {
                // Apply tint by multiplying with tint color
                let r = (p[0] as f32 * self.tint_color.r() as f32 / 255.0) as u8;
                let g = (p[1] as f32 * self.tint_color.g() as f32 / 255.0) as u8;
                let b = (p[2] as f32 * self.tint_color.b() as f32 / 255.0) as u8;
                let a = p[3];
                [r, g, b, a]
            })
            .collect();

        let color_image = ColorImage::from_rgba_unmultiplied(
            [width as usize, height as usize],
            &tinted,
        );

        // Create texture
        let texture = ctx.load_texture(name, color_image, TextureOptions::LINEAR);
        Some(texture)
    }

    /// Get a texture handle for a toolbar icon
    pub fn get(&self, icon: ToolbarIcon) -> Option<&TextureHandle> {
        self.textures.get(&icon)
    }

    /// Get a texture handle for an asset browser icon
    pub fn get_asset_icon(&self, icon: AssetBrowserIcon) -> Option<&TextureHandle> {
        self.asset_icons.get(&icon)
    }

    /// Get the icon display size
    pub fn icon_size(&self) -> Vec2 {
        Vec2::splat(self.icon_size as f32)
    }

    /// Check if any icons have been loaded
    pub fn has_any_icons(&self) -> bool {
        !self.textures.is_empty()
    }

    /// Set the tint color for icons (will require reload)
    pub fn set_tint_color(&mut self, color: Color32) {
        self.tint_color = color;
    }
}

/// Render an icon button with optional selection state
pub fn icon_button(
    ui: &mut egui::Ui,
    icon_manager: &IconManager,
    icon: ToolbarIcon,
    selected: bool,
    tooltip: &str,
) -> egui::Response {
    let size = icon_manager.icon_size();

    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());

    if ui.is_rect_visible(rect) {
        let visuals = if selected {
            ui.visuals().widgets.active
        } else if response.hovered() {
            ui.visuals().widgets.hovered
        } else {
            ui.visuals().widgets.inactive
        };

        // Draw background
        ui.painter().rect_filled(rect, 2.0, visuals.bg_fill);

        // Draw icon if available
        if let Some(texture) = icon_manager.get(icon) {
            let image_rect = rect.shrink(2.0);
            let tint = if selected {
                Color32::WHITE
            } else if response.hovered() {
                Color32::from_gray(220)
            } else {
                Color32::from_gray(180)
            };
            ui.painter().image(
                texture.id(),
                image_rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                tint,
            );
        } else {
            // Fallback: draw text abbreviation
            let text = match icon {
                ToolbarIcon::Select => "Q",
                ToolbarIcon::Translate => "W",
                ToolbarIcon::Rotate => "E",
                ToolbarIcon::Scale => "R",
                ToolbarIcon::World => "G",
                ToolbarIcon::Local => "L",
                ToolbarIcon::GridSnap => "▦",
                ToolbarIcon::RotationSnap => "∠",
                ToolbarIcon::ScaleSnap => "⊞",
                ToolbarIcon::CameraSpeed => "🎥",
            };
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                text,
                egui::FontId::proportional(12.0),
                visuals.text_color(),
            );
        }

        // Draw selection indicator
        if selected {
            ui.painter().rect_stroke(
                rect,
                2.0,
                egui::Stroke::new(1.5, Color32::from_rgb(100, 150, 255)),
                egui::StrokeKind::Outside,
            );
        }
    }

    // Show tooltip on hover
    if response.hovered() {
        egui::show_tooltip_at_pointer(ui.ctx(), ui.layer_id(), egui::Id::new(tooltip), |ui| {
            ui.label(tooltip);
        });
    }

    response
}
