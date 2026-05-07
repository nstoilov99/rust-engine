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

use super::icon_classes::{ChromeState, IconClass, IconPalette, Severity, TintMode, TypeCategory};

/// Icon kind enum — covers chrome UI glyphs, typed asset/entity icons, and severity badges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum IconKind {
    // ── Chrome (~16) — monochrome UI affordances ──
    Folder,
    File,
    Save,
    Open,
    Add,
    Delete,
    Copy,
    Paste,
    Search,
    ChevronRight,
    ChevronDown,
    DragHandle,
    Close,
    Settings,
    Visibility,
    VisibilityOff,

    // ── Typed (~25) — category-colored asset/entity icons ──
    // Geometry
    Mesh,
    Prefab,
    Skeleton,
    // Lights
    Sun,
    PointLight,
    SpotLight,
    // Cameras
    Camera,
    // VFX
    Particle,
    ParticleEffect,
    // Audio
    AudioEmitter,
    AudioMixer,
    // Animation
    AnimationClip,
    AnimationGraph,
    // Materials
    Material,
    MaterialInstance,
    Texture,
    Image,
    // Scripting
    Script,
    BehaviorTree,
    // Physics
    RigidBody,
    Collider,
    Joint,
    // UI
    UiPanel,
    UiButton,
    UiText,

    // ── Severity (4) — semantic state badges ──
    SeverityError,
    SeverityWarning,
    SeveritySuccess,
    SeverityInfo,

    // ── Legacy / generic (kept for backward compat) ──
    /// Generic light (maps to Sun for classification)
    Light,
    /// Generic audio (maps to AudioEmitter for classification)
    Audio,
    /// Generic entity with no specific icon
    Empty,
    // Actions (chrome)
    Play,
    Pause,
    Stop,
    // Panels (chrome)
    Hierarchy,
    Inspector,
    AssetBrowser,
    Console,
    Profiler,
    // Legacy status (map to Severity variants)
    Error,
    Warning,
    Success,
    Info,
}

impl IconKind {
    /// The icon class determines which palette table provides the tint.
    pub fn class(self) -> IconClass {
        match self {
            // Chrome
            Self::Folder
            | Self::File
            | Self::Save
            | Self::Open
            | Self::Add
            | Self::Delete
            | Self::Copy
            | Self::Paste
            | Self::Search
            | Self::ChevronRight
            | Self::ChevronDown
            | Self::DragHandle
            | Self::Close
            | Self::Settings
            | Self::Visibility
            | Self::VisibilityOff
            | Self::Play
            | Self::Pause
            | Self::Stop
            | Self::Hierarchy
            | Self::Inspector
            | Self::AssetBrowser
            | Self::Console
            | Self::Profiler
            | Self::Empty => IconClass::Chrome,

            // Typed
            Self::Mesh
            | Self::Prefab
            | Self::Skeleton
            | Self::Sun
            | Self::PointLight
            | Self::SpotLight
            | Self::Light
            | Self::Camera
            | Self::Particle
            | Self::ParticleEffect
            | Self::AudioEmitter
            | Self::AudioMixer
            | Self::Audio
            | Self::AnimationClip
            | Self::AnimationGraph
            | Self::Material
            | Self::MaterialInstance
            | Self::Texture
            | Self::Image
            | Self::Script
            | Self::BehaviorTree
            | Self::RigidBody
            | Self::Collider
            | Self::Joint
            | Self::UiPanel
            | Self::UiButton
            | Self::UiText => IconClass::Typed,

            // Severity
            Self::SeverityError
            | Self::SeverityWarning
            | Self::SeveritySuccess
            | Self::SeverityInfo
            | Self::Error
            | Self::Warning
            | Self::Success
            | Self::Info => IconClass::Severity,
        }
    }

    /// The category for Typed icons. Returns `None` for Chrome and Severity icons.
    pub fn category(self) -> Option<TypeCategory> {
        match self {
            Self::Mesh | Self::Prefab | Self::Skeleton => Some(TypeCategory::Geometry),
            Self::Sun | Self::PointLight | Self::SpotLight | Self::Light => {
                Some(TypeCategory::Lights)
            }
            Self::Camera => Some(TypeCategory::Cameras),
            Self::Particle | Self::ParticleEffect => Some(TypeCategory::Vfx),
            Self::AudioEmitter | Self::AudioMixer | Self::Audio => Some(TypeCategory::Audio),
            Self::AnimationClip | Self::AnimationGraph => Some(TypeCategory::Animation),
            Self::Material | Self::MaterialInstance | Self::Texture | Self::Image => {
                Some(TypeCategory::Materials)
            }
            Self::Script | Self::BehaviorTree => Some(TypeCategory::Scripting),
            Self::RigidBody | Self::Collider | Self::Joint => Some(TypeCategory::Physics),
            Self::UiPanel | Self::UiButton | Self::UiText => Some(TypeCategory::Ui),
            _ => None,
        }
    }

    /// The severity for Severity icons. Returns `None` for Chrome and Typed icons.
    pub fn severity(self) -> Option<Severity> {
        match self {
            Self::SeverityError | Self::Error => Some(Severity::Error),
            Self::SeverityWarning | Self::Warning => Some(Severity::Warning),
            Self::SeveritySuccess | Self::Success => Some(Severity::Success),
            Self::SeverityInfo | Self::Info => Some(Severity::Info),
            _ => None,
        }
    }

    /// The default tint mode. Most icons are Tinted (white source + runtime tint).
    /// A few multi-color icons are Authored (drawn as-is).
    pub fn default_tint_mode(self) -> TintMode {
        match self {
            Self::Folder | Self::Sun => TintMode::Authored,
            _ => TintMode::Tinted,
        }
    }

    /// Human-readable display name.
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Folder => "Folder",
            Self::File => "File",
            Self::Save => "Save",
            Self::Open => "Open",
            Self::Add => "Add",
            Self::Delete => "Delete",
            Self::Copy => "Copy",
            Self::Paste => "Paste",
            Self::Search => "Search",
            Self::ChevronRight => "Chevron Right",
            Self::ChevronDown => "Chevron Down",
            Self::DragHandle => "Drag Handle",
            Self::Close => "Close",
            Self::Settings => "Settings",
            Self::Visibility => "Visibility",
            Self::VisibilityOff => "Visibility Off",
            Self::Play => "Play",
            Self::Pause => "Pause",
            Self::Stop => "Stop",
            Self::Hierarchy => "Hierarchy",
            Self::Inspector => "Inspector",
            Self::AssetBrowser => "Asset Browser",
            Self::Console => "Console",
            Self::Profiler => "Profiler",
            Self::Empty => "Empty",
            Self::Mesh => "Mesh",
            Self::Prefab => "Prefab",
            Self::Skeleton => "Skeleton",
            Self::Sun => "Sun",
            Self::PointLight => "Point Light",
            Self::SpotLight => "Spot Light",
            Self::Light => "Light",
            Self::Camera => "Camera",
            Self::Particle => "Particle",
            Self::ParticleEffect => "Particle Effect",
            Self::AudioEmitter => "Audio Emitter",
            Self::AudioMixer => "Audio Mixer",
            Self::Audio => "Audio",
            Self::AnimationClip => "Animation Clip",
            Self::AnimationGraph => "Animation Graph",
            Self::Material => "Material",
            Self::MaterialInstance => "Material Instance",
            Self::Texture => "Texture",
            Self::Image => "Image",
            Self::Script => "Script",
            Self::BehaviorTree => "Behavior Tree",
            Self::RigidBody => "Rigid Body",
            Self::Collider => "Collider",
            Self::Joint => "Joint",
            Self::UiPanel => "UI Panel",
            Self::UiButton => "UI Button",
            Self::UiText => "UI Text",
            Self::SeverityError | Self::Error => "Error",
            Self::SeverityWarning | Self::Warning => "Warning",
            Self::SeveritySuccess | Self::Success => "Success",
            Self::SeverityInfo | Self::Info => "Info",
        }
    }

    /// All icon kinds, in display order.
    pub const ALL: &'static [IconKind] = &[
        // Chrome
        Self::Folder,
        Self::File,
        Self::Save,
        Self::Open,
        Self::Add,
        Self::Delete,
        Self::Copy,
        Self::Paste,
        Self::Search,
        Self::ChevronRight,
        Self::ChevronDown,
        Self::DragHandle,
        Self::Close,
        Self::Settings,
        Self::Visibility,
        Self::VisibilityOff,
        Self::Play,
        Self::Pause,
        Self::Stop,
        Self::Hierarchy,
        Self::Inspector,
        Self::AssetBrowser,
        Self::Console,
        Self::Profiler,
        Self::Empty,
        // Typed
        Self::Mesh,
        Self::Prefab,
        Self::Skeleton,
        Self::Sun,
        Self::PointLight,
        Self::SpotLight,
        Self::Camera,
        Self::Particle,
        Self::ParticleEffect,
        Self::AudioEmitter,
        Self::AudioMixer,
        Self::AnimationClip,
        Self::AnimationGraph,
        Self::Material,
        Self::MaterialInstance,
        Self::Texture,
        Self::Image,
        Self::Script,
        Self::BehaviorTree,
        Self::RigidBody,
        Self::Collider,
        Self::Joint,
        Self::UiPanel,
        Self::UiButton,
        Self::UiText,
        // Severity
        Self::SeverityError,
        Self::SeverityWarning,
        Self::SeveritySuccess,
        Self::SeverityInfo,
    ];

    /// All Chrome icon kinds.
    pub fn chrome_icons() -> impl Iterator<Item = IconKind> {
        Self::ALL.iter().copied().filter(|k| k.class() == IconClass::Chrome)
    }

    /// All Typed icon kinds.
    pub fn typed_icons() -> impl Iterator<Item = IconKind> {
        Self::ALL.iter().copied().filter(|k| k.class() == IconClass::Typed)
    }

    /// All Severity icon kinds.
    pub fn severity_icons() -> impl Iterator<Item = IconKind> {
        Self::ALL.iter().copied().filter(|k| k.class() == IconClass::Severity)
    }

    /// Typed icons in a specific category.
    pub fn icons_in_category(cat: TypeCategory) -> impl Iterator<Item = IconKind> {
        Self::ALL
            .iter()
            .copied()
            .filter(move |k| k.category() == Some(cat))
    }
}

// ChromeState, IconPalette, TintMode imported at the top of the file

/// Icon registry — loads and caches editor icons with palette-based tinting.
///
/// Icons are loaded as individual PNG textures from the `engine/icons/` directory.
/// Each `IconKind` maps to a PNG file when available, with a text fallback for
/// missing icons. The `IconPalette` controls tint colors at draw time.
pub struct IconRegistry {
    textures: std::collections::HashMap<IconKind, egui::TextureHandle>,
    palette: IconPalette,
}

impl IconRegistry {
    /// Empty registry — all icons render as text fallback with default palette.
    pub fn empty() -> Self {
        Self {
            textures: std::collections::HashMap::new(),
            palette: IconPalette::default_dark(),
        }
    }

    /// Load icons from the engine icons directory. If a saved palette file
    /// exists at `IconPalette::default_path()`, it is loaded; otherwise the
    /// hardcoded `default_dark` palette is used.
    pub fn load(ctx: &egui::Context) -> Self {
        let palette = IconPalette::load_from_default().unwrap_or_else(IconPalette::default_dark);
        Self {
            textures: load_icon_textures(ctx),
            palette,
        }
    }

    /// Get the texture handle for an icon kind, if loaded.
    pub fn get(&self, kind: IconKind) -> Option<&egui::TextureHandle> {
        self.textures.get(&kind)
    }

    /// Read-only access to the palette.
    pub fn palette(&self) -> &IconPalette {
        &self.palette
    }

    /// Mutable access to the palette (for live editing in the icon inspector).
    pub fn palette_mut(&mut self) -> &mut IconPalette {
        &mut self.palette
    }

    /// Resolve the effective tint mode for an icon (per-icon override or default).
    pub fn tint_mode(&self, kind: IconKind) -> TintMode {
        self.palette
            .tint_modes
            .get(&kind)
            .copied()
            .unwrap_or_else(|| kind.default_tint_mode())
    }

    /// Resolve the tint color for a tinted icon.
    ///
    /// Lookup order:
    /// 1. `(kind, state)` per-icon, per-state override
    /// 2. `(kind, ChromeState::Default)` per-icon override (so an icon with
    ///    only its Default state customised stays that color across all states
    ///    unless a specific state is explicitly overridden)
    /// 3. Class default (chrome/category/severity)
    ///
    /// Authored icons skip this path; their `image()` renders without tint.
    pub fn tint(&self, kind: IconKind, state: ChromeState) -> egui::Color32 {
        if let Some(&c) = self.palette.overrides.get(&(kind, state)) {
            return c;
        }
        if state != ChromeState::Default {
            if let Some(&c) = self.palette.overrides.get(&(kind, ChromeState::Default)) {
                return c;
            }
        }

        match kind.class() {
            IconClass::Chrome => self
                .palette
                .chrome
                .get(&state)
                .copied()
                .unwrap_or(egui::Color32::from_gray(200)),
            IconClass::Typed => kind
                .category()
                .and_then(|cat| self.palette.category.get(&cat).copied())
                .unwrap_or(egui::Color32::from_gray(200)),
            IconClass::Severity => kind
                .severity()
                .and_then(|sev| self.palette.severity.get(&sev).copied())
                .unwrap_or(egui::Color32::from_gray(200)),
        }
    }

    /// Read the per-icon override for a specific state, if one is set.
    pub fn icon_override(&self, kind: IconKind, state: ChromeState) -> Option<egui::Color32> {
        self.palette.overrides.get(&(kind, state)).copied()
    }

    /// True if the icon has any per-state override set.
    pub fn icon_has_any_override(&self, kind: IconKind) -> bool {
        self.palette.overrides.keys().any(|(k, _)| *k == kind)
    }

    // --- Palette mutation methods (used by the icon inspector) ---

    /// Set a category tint color.
    pub fn set_category_color(&mut self, cat: TypeCategory, color: egui::Color32) {
        self.palette.category.insert(cat, color);
    }

    /// Set a severity tint color.
    pub fn set_severity_color(&mut self, sev: Severity, color: egui::Color32) {
        self.palette.severity.insert(sev, color);
    }

    /// Set a chrome state tint color.
    pub fn set_chrome_color(&mut self, state: ChromeState, color: egui::Color32) {
        self.palette.chrome.insert(state, color);
    }

    /// Set a per-icon, per-state tint override.
    pub fn set_icon_override(
        &mut self,
        kind: IconKind,
        state: ChromeState,
        color: egui::Color32,
    ) {
        self.palette.overrides.insert((kind, state), color);
    }

    /// Clear the override for a specific (icon, state) pair.
    pub fn clear_icon_override(&mut self, kind: IconKind, state: ChromeState) {
        self.palette.overrides.remove(&(kind, state));
    }

    /// Clear every per-state override for the given icon.
    pub fn clear_all_icon_overrides(&mut self, kind: IconKind) {
        self.palette.overrides.retain(|(k, _), _| *k != kind);
    }

    // ── Panel-icon overrides (engine/icons/<panel>/<stem>.svg) ────────────

    /// Resolve the tint to apply to an SVG drawn from
    /// `engine/icons/<panel>/<stem>.svg` at the given state.
    ///
    /// Lookup order:
    /// 1. `(panel, stem, state)` per-state override
    /// 2. `(panel, stem, ChromeState::Default)` override (fallback so an icon
    ///    customised only on Default keeps that colour across all states)
    /// 3. `default_tint` — typically `Color32::WHITE` so the SVG paints as
    ///    authored.
    pub fn panel_icon_tint(
        &self,
        panel: &str,
        stem: &str,
        state: ChromeState,
        default_tint: egui::Color32,
    ) -> egui::Color32 {
        if let Some(&c) = self.palette.panel_overrides.get(&(
            panel.to_string(),
            stem.to_string(),
            state,
        )) {
            return c;
        }
        if state != ChromeState::Default {
            if let Some(&c) = self.palette.panel_overrides.get(&(
                panel.to_string(),
                stem.to_string(),
                ChromeState::Default,
            )) {
                return c;
            }
        }
        default_tint
    }

    /// Read the per-state override for a panel icon, if one is set.
    pub fn panel_icon_override(
        &self,
        panel: &str,
        stem: &str,
        state: ChromeState,
    ) -> Option<egui::Color32> {
        self.palette
            .panel_overrides
            .get(&(panel.to_string(), stem.to_string(), state))
            .copied()
    }

    /// True if the panel icon has any per-state override set.
    pub fn panel_icon_has_any_override(&self, panel: &str, stem: &str) -> bool {
        self.palette
            .panel_overrides
            .keys()
            .any(|(p, s, _)| p == panel && s == stem)
    }

    /// Set / upsert a per-state override for a panel icon.
    pub fn set_panel_icon_override(
        &mut self,
        panel: &str,
        stem: &str,
        state: ChromeState,
        color: egui::Color32,
    ) {
        self.palette
            .panel_overrides
            .insert((panel.to_string(), stem.to_string(), state), color);
    }

    /// Clear the override for a specific (panel, stem, state) triple.
    pub fn clear_panel_icon_override(&mut self, panel: &str, stem: &str, state: ChromeState) {
        self.palette
            .panel_overrides
            .remove(&(panel.to_string(), stem.to_string(), state));
    }

    /// Clear every per-state override for the given panel icon.
    pub fn clear_all_panel_icon_overrides(&mut self, panel: &str, stem: &str) {
        self.palette
            .panel_overrides
            .retain(|(p, s, _), _| !(p == panel && s == stem));
    }

    /// Read the per-icon size override (px), if one is set.
    pub fn panel_icon_size(&self, panel: &str, stem: &str) -> Option<f32> {
        self.palette
            .panel_sizes
            .get(&(panel.to_string(), stem.to_string()))
            .copied()
    }

    /// Set / upsert the size for a panel icon.
    pub fn set_panel_icon_size(&mut self, panel: &str, stem: &str, size: f32) {
        self.palette
            .panel_sizes
            .insert((panel.to_string(), stem.to_string()), size);
    }

    /// Remove the size override for a panel icon (revert to caller default).
    pub fn clear_panel_icon_size(&mut self, panel: &str, stem: &str) {
        self.palette
            .panel_sizes
            .remove(&(panel.to_string(), stem.to_string()));
    }

    /// Set a per-icon tint mode override.
    pub fn set_icon_tint_mode(&mut self, kind: IconKind, mode: TintMode) {
        self.palette.tint_modes.insert(kind, mode);
    }

    /// Clear a per-icon tint mode override (revert to default_tint_mode()).
    pub fn clear_icon_tint_mode(&mut self, kind: IconKind) {
        self.palette.tint_modes.remove(&kind);
    }

    /// Reset the entire palette to defaults.
    pub fn reset_palette(&mut self) {
        self.palette.reset();
    }

    /// Get a text fallback character for an icon kind.
    pub fn fallback_text(kind: IconKind) -> &'static str {
        match kind {
            // Chrome
            IconKind::Folder => "\u{1F4C1}",
            IconKind::File => "\u{1F4C4}",
            IconKind::Save => "S",
            IconKind::Open => "O",
            IconKind::Add => "+",
            IconKind::Delete => "x",
            IconKind::Copy => "C",
            IconKind::Paste => "V",
            IconKind::Search => "?",
            IconKind::ChevronRight => "\u{25B8}",
            IconKind::ChevronDown => "\u{25BE}",
            IconKind::DragHandle => "\u{2261}",
            IconKind::Close => "X",
            IconKind::Settings => "\u{2699}",
            IconKind::Visibility => "\u{25C9}",
            IconKind::VisibilityOff => "\u{25CB}",
            IconKind::Play => "\u{25B6}",
            IconKind::Pause => "\u{23F8}",
            IconKind::Stop => "\u{23F9}",
            IconKind::Hierarchy => "H",
            IconKind::Inspector => "I",
            IconKind::AssetBrowser => "A",
            IconKind::Console => ">",
            IconKind::Profiler => "P",
            IconKind::Empty => "\u{00B7}",
            // Typed — Geometry
            IconKind::Mesh => "\u{25B3}",
            IconKind::Prefab => "\u{25A3}",
            IconKind::Skeleton => "\u{2620}",
            // Typed — Lights
            IconKind::Sun | IconKind::Light => "\u{2600}",
            IconKind::PointLight => "\u{25CF}",
            IconKind::SpotLight => "\u{25E3}",
            // Typed — Cameras
            IconKind::Camera => "\u{1F4F7}",
            // Typed — VFX
            IconKind::Particle | IconKind::ParticleEffect => "\u{2728}",
            // Typed — Audio
            IconKind::AudioEmitter | IconKind::Audio => "\u{266B}",
            IconKind::AudioMixer => "\u{266A}",
            // Typed — Animation
            IconKind::AnimationClip => "\u{25B6}",
            IconKind::AnimationGraph => "\u{2637}",
            // Typed — Materials
            IconKind::Material | IconKind::MaterialInstance => "\u{25C9}",
            IconKind::Texture | IconKind::Image => "\u{1F5BC}",
            // Typed — Scripting
            IconKind::Script => "\u{2699}",
            IconKind::BehaviorTree => "\u{2702}",
            // Typed — Physics
            IconKind::RigidBody => "\u{25A0}",
            IconKind::Collider => "\u{25A1}",
            IconKind::Joint => "\u{2318}",
            // Typed — UI
            IconKind::UiPanel => "\u{25A4}",
            IconKind::UiButton => "\u{25A3}",
            IconKind::UiText => "T",
            // Severity
            IconKind::SeverityError | IconKind::Error => "!",
            IconKind::SeverityWarning | IconKind::Warning => "\u{26A0}",
            IconKind::SeveritySuccess | IconKind::Success => "\u{2713}",
            IconKind::SeverityInfo | IconKind::Info => "i",
        }
    }
}

/// Load all editor icon PNGs from disk into the given egui context.
///
/// Returned `TextureHandle`s are bound to `ctx` — they cannot be sampled
/// from a different egui context (e.g. a secondary OS window). Callers that
/// need icons in a separate context must call this again with that context.
pub fn load_icon_textures(
    ctx: &egui::Context,
) -> std::collections::HashMap<IconKind, egui::TextureHandle> {
    let mut textures = std::collections::HashMap::new();

    let icons_dir = std::path::Path::new("engine/icons");
    if !icons_dir.exists() {
        log::warn!("Icons directory not found: {}", icons_dir.display());
        return textures;
    }

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
            textures.insert(*kind, texture);
        }
    }

    textures
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

    /// Render an icon from the `IconRegistry` using default chrome state.
    fn icon(&mut self, kind: IconKind) -> egui::Response;

    /// Render an icon with explicit chrome state and size.
    fn icon_tinted(
        &mut self,
        kind: IconKind,
        state: ChromeState,
        size: f32,
    ) -> egui::Response;
}

fn get_registry(ctx: &egui::Context) -> Arc<IconRegistry> {
    ctx.data(|d| d.get_temp::<Arc<IconRegistry>>(egui::Id::NULL))
        .unwrap_or_else(|| {
            log_once_warn("IconRegistry not yet installed; rendering text fallback");
            Arc::new(IconRegistry::empty())
        })
}

/// Render an icon at the given size with a specific tint color.
fn render_icon_impl(
    ui: &mut egui::Ui,
    kind: IconKind,
    registry: &IconRegistry,
    tint: egui::Color32,
    size: f32,
) -> egui::Response {
    let size_vec = egui::vec2(size, size);

    if let Some(texture) = registry.get(kind) {
        let (rect, response) = ui.allocate_exact_size(size_vec, egui::Sense::click());
        if ui.is_rect_visible(rect) {
            let draw_tint = match registry.tint_mode(kind) {
                TintMode::Tinted => tint,
                TintMode::Authored => egui::Color32::WHITE,
            };
            ui.painter().image(
                texture.id(),
                rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                draw_tint,
            );
        }
        response
    } else {
        // Text fallback for missing icons
        let text = IconRegistry::fallback_text(kind);
        let (rect, response) = ui.allocate_exact_size(size_vec, egui::Sense::click());
        if ui.is_rect_visible(rect) {
            let font_size = (size * 0.7).max(8.0);
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                text,
                egui::FontId::proportional(font_size),
                tint,
            );
        }
        response
    }
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
        let registry = get_registry(self.ctx());
        let tint = registry.tint(kind, ChromeState::Default);
        render_icon_impl(self, kind, &registry, tint, 16.0)
    }

    fn icon_tinted(
        &mut self,
        kind: IconKind,
        state: ChromeState,
        size: f32,
    ) -> egui::Response {
        let registry = get_registry(self.ctx());
        let tint = registry.tint(kind, state);
        render_icon_impl(self, kind, &registry, tint, size)
    }
}
