//! Hierarchy panel icon set — SVG rasterization helpers used by the crusty
//! icon uploader on the render thread.
//!
//! The old `HierarchyIcons` texture map and `load_svg_texture`
//! helper were removed as part of the UI teardown. The crusty renderer
//! calls `rasterize_svg_rgba` + `icon_raster_px` + `icons_dir` directly.

use std::path::Path;

/// Pixel size each SVG is rasterized at. Big enough to look crisp on HiDPI
/// without bloating the texture cache.
const ICON_RASTER_PX: u32 = 32;

/// Subdirectory (relative to working dir) the loader scans.
const HIERARCHY_ICONS_DIR: &str = "engine/icons/hierarchy";

/// Rasterize one SVG to raw RGBA bytes at [`icon_raster_px`] square.
pub fn rasterize_svg_rgba(path: &Path) -> Result<Vec<u8>, String> {
    use resvg::tiny_skia;
    use resvg::usvg;

    let svg_bytes = std::fs::read(path).map_err(|e| format!("read failed: {e}"))?;

    // Parse with default options — fonts are needed only for `<text>` nodes
    // in the SVG (we have none), so an empty fontdb is fine.
    let opt = usvg::Options::default();
    let tree =
        usvg::Tree::from_data(&svg_bytes, &opt).map_err(|e| format!("usvg parse failed: {e}"))?;

    // Scale the SVG into a square pixmap at our target raster size.
    let svg_size = tree.size();
    let scale_x = ICON_RASTER_PX as f32 / svg_size.width().max(1.0);
    let scale_y = ICON_RASTER_PX as f32 / svg_size.height().max(1.0);
    let scale = scale_x.min(scale_y);
    let transform = tiny_skia::Transform::from_scale(scale, scale);

    let mut pixmap = tiny_skia::Pixmap::new(ICON_RASTER_PX, ICON_RASTER_PX)
        .ok_or_else(|| "tiny_skia::Pixmap allocation failed".to_string())?;
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    Ok(pixmap.take())
}

/// The square pixel size [`rasterize_svg_rgba`] renders at.
pub fn icon_raster_px() -> u32 {
    ICON_RASTER_PX
}

/// Directory the hierarchy icon SVGs are discovered in.
pub fn icons_dir() -> &'static Path {
    Path::new(HIERARCHY_ICONS_DIR)
}
