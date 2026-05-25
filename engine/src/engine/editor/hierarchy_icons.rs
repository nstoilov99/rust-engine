//! Hierarchy panel icon set — SVG, auto-discovered.
//!
//! Drop any `*.svg` into `engine/icons/hierarchy/` and it becomes available
//! to the hierarchy panel keyed by its file stem. The hierarchy panel chooses
//! which stem to show per entity (e.g. `camera`, `sun`, `object`); adding new
//! icons does **not** require code changes here — only at the call site if
//! you want a new entity kind to use it.
//!
//! SVGs are rasterized once at startup at a fixed size; the renderer simply
//! samples the resulting `TextureHandle` like any other icon. The file lives
//! alongside `widgets::IconRegistry` rather than inside it because the
//! hierarchy is the only consumer for now and we want this set to evolve
//! independently of the global icon palette / `IconKind` enum.

use std::collections::HashMap;
use std::path::Path;

/// Pixel size each SVG is rasterized at. Big enough to look crisp on HiDPI
/// without bloating the texture cache.
const ICON_RASTER_PX: u32 = 32;

/// Subdirectory (relative to working dir) the loader scans.
const HIERARCHY_ICONS_DIR: &str = "engine/icons/hierarchy";

/// Lookup table of hierarchy icon textures, keyed by SVG file stem
/// (e.g. `"camera"`, `"sun"`, `"object"`, `"visibility"`).
pub struct HierarchyIcons {
    textures: HashMap<String, egui::TextureHandle>,
}

impl HierarchyIcons {
    /// Empty placeholder used before icons are loaded (see
    /// `EditorServices::load_icons`).
    pub fn empty() -> Self {
        Self {
            textures: HashMap::new(),
        }
    }

    /// Load every `*.svg` under `engine/icons/hierarchy/`. Missing folder /
    /// unparseable files are logged once and skipped — the panel falls back
    /// to a text glyph in those cases.
    pub fn load(ctx: &egui::Context) -> Self {
        let mut textures = HashMap::new();

        let dir = Path::new(HIERARCHY_ICONS_DIR);
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) => {
                log::warn!("Hierarchy icons dir missing or unreadable ({}): {}", dir.display(), e);
                return Self { textures };
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("svg") {
                continue;
            }
            let stem = match path.file_stem().and_then(|s| s.to_str()) {
                Some(s) => s.to_string(),
                None => continue,
            };

            match load_svg_texture(ctx, &path, &stem) {
                Ok(tex) => {
                    textures.insert(stem, tex);
                }
                Err(e) => log::warn!("Failed to load hierarchy SVG {}: {}", path.display(), e),
            }
        }

        Self { textures }
    }

    /// Look up an icon by file stem.
    pub fn get(&self, name: &str) -> Option<&egui::TextureHandle> {
        self.textures.get(name)
    }

    pub fn is_empty(&self) -> bool {
        self.textures.is_empty()
    }
}

/// Rasterize one SVG to a `TextureHandle` in the given egui context.
///
/// Public so the Icon Inspector (which lives in a separate egui context inside
/// its own window) can populate its preview using the same code path. The
/// `name` argument is used as the texture's debug label only.
pub fn load_svg_texture(
    ctx: &egui::Context,
    path: &Path,
    name: &str,
) -> Result<egui::TextureHandle, String> {
    use resvg::tiny_skia;
    use resvg::usvg;

    let svg_bytes = std::fs::read(path).map_err(|e| format!("read failed: {e}"))?;

    // Parse with default options — fonts are needed only for `<text>` nodes
    // in the SVG (we have none), so an empty fontdb is fine.
    let opt = usvg::Options::default();
    let tree = usvg::Tree::from_data(&svg_bytes, &opt)
        .map_err(|e| format!("usvg parse failed: {e}"))?;

    // Scale the SVG into a square pixmap at our target raster size.
    let svg_size = tree.size();
    let scale_x = ICON_RASTER_PX as f32 / svg_size.width().max(1.0);
    let scale_y = ICON_RASTER_PX as f32 / svg_size.height().max(1.0);
    let scale = scale_x.min(scale_y);
    let transform = tiny_skia::Transform::from_scale(scale, scale);

    let mut pixmap = tiny_skia::Pixmap::new(ICON_RASTER_PX, ICON_RASTER_PX)
        .ok_or_else(|| "tiny_skia::Pixmap allocation failed".to_string())?;
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    let color_image = egui::ColorImage::from_rgba_unmultiplied(
        [ICON_RASTER_PX as usize, ICON_RASTER_PX as usize],
        pixmap.data(),
    );

    Ok(ctx.load_texture(
        format!("hierarchy_icon_{name}"),
        color_image,
        egui::TextureOptions::LINEAR,
    ))
}
