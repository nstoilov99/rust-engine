//! Reusable editor widgets and extension traits
//!
//! Provides the `UiExt` trait for accessing theme and icons from any `egui::Ui`,
//! plus all custom widgets used by the editor panels.

pub mod button;
pub mod toggle_switch;
pub mod field_row;
pub mod slider_with_input;
pub mod panel_header;
pub mod tab_bar;
pub mod tree_row;
pub mod asset_slot;
pub mod search_field;
pub mod search_popup;
pub mod empty_state;
// pub mod color_picker; // Step 13

use std::sync::Arc;

use super::theme::EditorTheme;

/// Icon kind enum — full set populated in Step 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IconKind {
    // Navigation / structure
    Folder,
    File,
    Image,
    Mesh,
    Material,
    Light,
    Camera,
    Audio,
    Script,
    // Actions
    Play,
    Pause,
    Stop,
    Save,
    Open,
    Add,
    Delete,
    Copy,
    Paste,
    Search,
    // Panels
    Hierarchy,
    Inspector,
    AssetBrowser,
    Console,
    Profiler,
    // Visibility
    Visibility,
    VisibilityOff,
    // Tree / navigation
    ChevronRight,
    ChevronDown,
    DragHandle,
    // Status
    Error,
    Warning,
    Success,
    Info,
    // Entity types
    Empty,
}

/// Icon registry — loads and caches editor icons.
///
/// Icons are loaded as individual PNG textures from the `engine/icons/` directory,
/// matching the existing `IconManager` pattern. Each `IconKind` maps to a PNG file
/// when available, with a text fallback for missing icons.
pub struct IconRegistry {
    textures: std::collections::HashMap<IconKind, egui::TextureHandle>,
}

impl IconRegistry {
    /// Empty registry — all icons render as text fallback.
    pub fn empty() -> Self {
        Self {
            textures: std::collections::HashMap::new(),
        }
    }

    /// Load icons from the engine icons directory.
    pub fn load(ctx: &egui::Context) -> Self {
        let mut registry = Self {
            textures: std::collections::HashMap::new(),
        };

        let icons_dir = std::path::Path::new("engine/icons");
        if !icons_dir.exists() {
            log::warn!("Icons directory not found: {}", icons_dir.display());
            return registry;
        }

        // Map IconKind to existing PNG filenames
        let mappings: &[(IconKind, &str)] = &[
            (IconKind::Folder, "folder"),
            (IconKind::File, "file-document"),
            (IconKind::Image, "image-file"),
            (IconKind::Mesh, "file-mesh"),
            (IconKind::Script, "code-file"),
            (IconKind::Play, "play-fill"),
            (IconKind::Pause, "pause-fill"),
            (IconKind::Stop, "stop-fill"),
            (IconKind::ChevronRight, "arrow-right"),
            (IconKind::ChevronDown, "arrow-down"),
        ];

        for (kind, filename) in mappings {
            let path = icons_dir.join(format!("{filename}.png"));
            if let Some(texture) = load_png_icon(ctx, &path, filename) {
                registry.textures.insert(*kind, texture);
            }
        }

        registry
    }

    /// Get the texture handle for an icon kind, if loaded.
    pub fn get(&self, kind: IconKind) -> Option<&egui::TextureHandle> {
        self.textures.get(&kind)
    }

    /// Get a text fallback character for an icon kind.
    pub fn fallback_text(kind: IconKind) -> &'static str {
        match kind {
            IconKind::Folder => "\u{1F4C1}",
            IconKind::File => "\u{1F4C4}",
            IconKind::Image => "\u{1F5BC}",
            IconKind::Mesh => "\u{25B3}",
            IconKind::Material => "\u{25C9}",
            IconKind::Light => "\u{2600}",
            IconKind::Camera => "\u{1F4F7}",
            IconKind::Audio => "\u{266B}",
            IconKind::Script => "\u{2699}",
            IconKind::Play => "\u{25B6}",
            IconKind::Pause => "\u{23F8}",
            IconKind::Stop => "\u{23F9}",
            IconKind::Save => "S",
            IconKind::Open => "O",
            IconKind::Add => "+",
            IconKind::Delete => "x",
            IconKind::Copy => "C",
            IconKind::Paste => "V",
            IconKind::Search => "?",
            IconKind::Hierarchy => "H",
            IconKind::Inspector => "I",
            IconKind::AssetBrowser => "A",
            IconKind::Console => ">",
            IconKind::Profiler => "P",
            IconKind::Visibility => "\u{25C9}",
            IconKind::VisibilityOff => "\u{25CB}",
            IconKind::ChevronRight => "\u{25B8}",
            IconKind::ChevronDown => "\u{25BE}",
            IconKind::DragHandle => "\u{2261}",
            IconKind::Error => "!",
            IconKind::Warning => "\u{26A0}",
            IconKind::Success => "\u{2713}",
            IconKind::Info => "i",
            IconKind::Empty => "\u{00B7}",
        }
    }
}

/// Load a single PNG icon from disk.
fn load_png_icon(
    ctx: &egui::Context,
    path: &std::path::Path,
    name: &str,
) -> Option<egui::TextureHandle> {
    let data = std::fs::read(path).ok()?;
    let image = image::load_from_memory(&data).ok()?;
    let rgba = image.to_rgba8();
    let (width, height) = rgba.dimensions();
    let color_image = egui::ColorImage::from_rgba_unmultiplied(
        [width as usize, height as usize],
        rgba.as_raw(),
    );
    Some(ctx.load_texture(
        format!("editor_icon_{name}"),
        color_image,
        egui::TextureOptions::LINEAR,
    ))
}

use std::collections::HashSet;
use std::sync::OnceLock;

/// Emit a warning log message at most once per unique message per session.
fn log_once_warn(msg: &str) {
    use std::sync::Mutex;
    static SEEN: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    let seen = SEEN.get_or_init(|| Mutex::new(HashSet::new()));
    let mut guard = seen.lock().unwrap();
    if guard.insert(msg.to_owned()) {
        log::warn!("{}", msg);
    }
}

/// Extension trait for `egui::Ui` — provides ergonomic access to the
/// editor theme and icon registry without passing `&EditorServices` everywhere.
pub trait UiExt {
    /// Read the current `EditorTheme` from egui context data.
    fn theme(&self) -> Arc<EditorTheme>;

    /// Render an icon from the `IconRegistry`.
    fn icon(&mut self, kind: IconKind) -> egui::Response;
}

impl UiExt for egui::Ui {
    fn theme(&self) -> Arc<EditorTheme> {
        self.ctx()
            .data(|d| d.get_temp::<Arc<EditorTheme>>(egui::Id::NULL))
            .unwrap_or_else(|| {
                log_once_warn("EditorTheme not yet installed; returning fallback");
                Arc::new(EditorTheme::fallback())
            })
    }

    fn icon(&mut self, kind: IconKind) -> egui::Response {
        let registry = self
            .ctx()
            .data(|d| d.get_temp::<Arc<IconRegistry>>(egui::Id::NULL))
            .unwrap_or_else(|| {
                log_once_warn("IconRegistry not yet installed; rendering text fallback");
                Arc::new(IconRegistry::empty())
            });

        let size = egui::vec2(16.0, 16.0);

        if let Some(texture) = registry.get(kind) {
            let (rect, response) = self.allocate_exact_size(size, egui::Sense::click());
            if self.is_rect_visible(rect) {
                self.painter().image(
                    texture.id(),
                    rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::from_gray(200),
                );
            }
            response
        } else {
            // Text fallback for missing icons
            let text = IconRegistry::fallback_text(kind);
            let (rect, response) = self.allocate_exact_size(size, egui::Sense::click());
            if self.is_rect_visible(rect) {
                self.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    text,
                    egui::FontId::proportional(11.0),
                    egui::Color32::from_gray(180),
                );
            }
            response
        }
    }
}
